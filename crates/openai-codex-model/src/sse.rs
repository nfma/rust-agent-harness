use std::io::{self, Read};
use std::time::{Duration, Instant};

use serde_json::Value;

pub(super) const MAX_EVENT_BYTES: usize = 1024 * 1024;
pub(super) const MAX_RESPONSE_BYTES: usize = 4 * 1024 * 1024;
pub(super) const MAX_TEXT_BYTES: usize = 256 * 1024;

#[derive(Clone, Copy)]
pub(super) struct Limits {
    pub event_bytes: usize,
    pub response_bytes: usize,
    pub text_bytes: usize,
    pub total_budget: Duration,
}

impl Limits {
    pub(super) fn production() -> Self {
        Self {
            event_bytes: MAX_EVENT_BYTES,
            response_bytes: MAX_RESPONSE_BYTES,
            text_bytes: MAX_TEXT_BYTES,
            total_budget: Duration::from_secs(2 * 60),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum DecodeError {
    Invalid,
    LimitExceeded,
    TimedOut,
    ModelFailed,
    EmptyText,
}

#[derive(Default)]
struct TextState {
    deltas: String,
    output_item: Option<String>,
    done: Option<String>,
}

enum EventOutcome {
    Continue,
    Completed(String),
}

pub(super) fn decode(
    reader: &mut dyn Read,
    limits: Limits,
    started: Instant,
) -> Result<String, DecodeError> {
    let mut pending = Vec::new();
    let mut event_data = String::new();
    let mut event_bytes = 0usize;
    let mut response_bytes = 0usize;
    let mut state = TextState::default();
    let mut buffer = [0u8; 8 * 1024];

    loop {
        check_budget(started, limits.total_budget)?;
        let read = reader.read(&mut buffer).map_err(map_read_error)?;
        check_budget(started, limits.total_budget)?;
        if read == 0 {
            return Err(DecodeError::Invalid);
        }
        response_bytes = response_bytes
            .checked_add(read)
            .ok_or(DecodeError::LimitExceeded)?;
        if response_bytes > limits.response_bytes {
            return Err(DecodeError::LimitExceeded);
        }
        if let Some(text) = process_chunk(
            &buffer[..read],
            &mut pending,
            &mut event_data,
            &mut event_bytes,
            &mut state,
            limits,
            started,
        )? {
            return Ok(text);
        }
    }
}

fn process_chunk(
    chunk: &[u8],
    pending: &mut Vec<u8>,
    event_data: &mut String,
    event_bytes: &mut usize,
    state: &mut TextState,
    limits: Limits,
    started: Instant,
) -> Result<Option<String>, DecodeError> {
    for byte in chunk {
        if *byte == b'\n' {
            if let Some(text) = process_line(
                std::mem::take(pending),
                event_data,
                event_bytes,
                state,
                limits,
                started,
            )? {
                return Ok(Some(text));
            }
            continue;
        }
        if event_bytes
            .checked_add(pending.len())
            .and_then(|length| length.checked_add(1))
            .is_none_or(|length| length > limits.event_bytes)
        {
            return Err(DecodeError::LimitExceeded);
        }
        pending.push(*byte);
    }
    Ok(None)
}

fn process_line(
    mut line: Vec<u8>,
    event_data: &mut String,
    event_bytes: &mut usize,
    state: &mut TextState,
    limits: Limits,
    started: Instant,
) -> Result<Option<String>, DecodeError> {
    if line.last() == Some(&b'\r') {
        line.pop();
    }
    *event_bytes = event_bytes
        .checked_add(line.len() + 1)
        .ok_or(DecodeError::LimitExceeded)?;
    if *event_bytes > limits.event_bytes {
        return Err(DecodeError::LimitExceeded);
    }
    let line = std::str::from_utf8(&line).map_err(|_| DecodeError::Invalid)?;

    if line.is_empty() {
        check_budget(started, limits.total_budget)?;
        let completed = if event_data.is_empty() {
            None
        } else {
            match dispatch(event_data, state, limits.text_bytes)? {
                EventOutcome::Continue => None,
                EventOutcome::Completed(text) => Some(text),
            }
        };
        event_data.clear();
        *event_bytes = 0;
        return Ok(completed);
    }
    if line.starts_with(':') {
        return Ok(None);
    }
    let Some((field, value)) = line.split_once(':') else {
        return Ok(None);
    };
    if field != "data" {
        return Ok(None);
    }
    let value = value.strip_prefix(' ').unwrap_or(value);
    let additional = value.len() + usize::from(!event_data.is_empty());
    if event_data
        .len()
        .checked_add(additional)
        .is_none_or(|length| length > limits.event_bytes)
    {
        return Err(DecodeError::LimitExceeded);
    }
    if !event_data.is_empty() {
        event_data.push('\n');
    }
    event_data.push_str(value);
    Ok(None)
}

fn map_read_error(error: io::Error) -> DecodeError {
    let timed_out = matches!(
        error.kind(),
        io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
    ) || error
        .get_ref()
        .and_then(|inner| inner.downcast_ref::<reqwest::Error>())
        .is_some_and(reqwest::Error::is_timeout);
    if timed_out {
        DecodeError::TimedOut
    } else {
        DecodeError::Invalid
    }
}

fn check_budget(started: Instant, total_budget: Duration) -> Result<(), DecodeError> {
    if started.elapsed() >= total_budget {
        Err(DecodeError::TimedOut)
    } else {
        Ok(())
    }
}

fn dispatch(
    data: &str,
    state: &mut TextState,
    maximum_text_bytes: usize,
) -> Result<EventOutcome, DecodeError> {
    let event: Value = serde_json::from_str(data).map_err(|_| DecodeError::Invalid)?;
    let event_type = event
        .get("type")
        .and_then(Value::as_str)
        .ok_or(DecodeError::Invalid)?;

    match event_type {
        "response.output_text.delta" => {
            let delta = event
                .get("delta")
                .and_then(Value::as_str)
                .ok_or(DecodeError::Invalid)?;
            append_bounded(&mut state.deltas, delta, maximum_text_bytes)?;
        }
        "response.output_item.done" => {
            if let Some(text) = assistant_item_text(&event, maximum_text_bytes)? {
                state.output_item = Some(text);
            }
        }
        "response.output_text.done" => {
            let text = event
                .get("text")
                .and_then(Value::as_str)
                .ok_or(DecodeError::Invalid)?;
            ensure_text_bound(text, maximum_text_bytes)?;
            state.done = Some(text.to_owned());
        }
        "response.completed" => {
            let text = state
                .output_item
                .clone()
                .or_else(|| (!state.deltas.is_empty()).then(|| state.deltas.clone()))
                .or_else(|| state.done.clone())
                .ok_or(DecodeError::EmptyText)?;
            if text.is_empty() {
                return Err(DecodeError::EmptyText);
            }
            return Ok(EventOutcome::Completed(text));
        }
        "response.failed" | "response.incomplete" | "error" => {
            return Err(DecodeError::ModelFailed);
        }
        _ => {}
    }

    Ok(EventOutcome::Continue)
}

fn assistant_item_text(
    event: &Value,
    maximum_text_bytes: usize,
) -> Result<Option<String>, DecodeError> {
    let item = event.get("item").ok_or(DecodeError::Invalid)?;
    if item.get("type").and_then(Value::as_str) != Some("message")
        || item.get("role").and_then(Value::as_str) != Some("assistant")
    {
        return Ok(None);
    }
    let Some(content) = item.get("content").and_then(Value::as_array) else {
        return Ok(None);
    };
    let mut text = String::new();
    for part in content {
        if part.get("type").and_then(Value::as_str) != Some("output_text") {
            continue;
        }
        let Some(part) = part.get("text").and_then(Value::as_str) else {
            continue;
        };
        append_bounded(&mut text, part, maximum_text_bytes)?;
    }
    Ok((!text.is_empty()).then_some(text))
}

fn append_bounded(
    target: &mut String,
    text: &str,
    maximum_text_bytes: usize,
) -> Result<(), DecodeError> {
    if target
        .len()
        .checked_add(text.len())
        .is_none_or(|length| length > maximum_text_bytes)
    {
        return Err(DecodeError::LimitExceeded);
    }
    target.push_str(text);
    Ok(())
}

fn ensure_text_bound(text: &str, maximum_text_bytes: usize) -> Result<(), DecodeError> {
    if text.len() > maximum_text_bytes {
        Err(DecodeError::LimitExceeded)
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;
    use std::io::Cursor;

    use super::*;

    struct ChunkedReader {
        bytes: Cursor<Vec<u8>>,
        chunk_size: usize,
    }

    impl Read for ChunkedReader {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            let limit = buffer.len().min(self.chunk_size);
            self.bytes.read(&mut buffer[..limit])
        }
    }

    fn limits() -> Limits {
        Limits {
            event_bytes: 4 * 1024,
            response_bytes: 16 * 1024,
            text_bytes: 1024,
            total_budget: Duration::from_secs(1),
        }
    }

    fn decode_text(stream: impl Into<Vec<u8>>, chunk_size: usize) -> Result<String, DecodeError> {
        decode(
            &mut ChunkedReader {
                bytes: Cursor::new(stream.into()),
                chunk_size,
            },
            limits(),
            Instant::now(),
        )
    }

    proptest! {
        #[test]
        fn fuzz_sse_decoder_is_bounded_and_panic_free(
            stream in prop::collection::vec(any::<u8>(), 0..8 * 1024),
            chunk_size in 1usize..128,
        ) {
            let result = decode_text(stream, chunk_size);
            prop_assert!(result.is_err() || result.as_ref().is_ok_and(|text| text.len() <= 1024));
        }
    }

    #[test]
    fn framing_supports_fragmented_utf8_lf_crlf_comments_and_multiline_data() {
        let stream = concat!(
            ": keepalive\r\n\r\n",
            "unknown: ignored\n",
            "data: {\"type\":\"response.unknown\"}\n\n",
            "data: {\"type\":\"response.output_text.delta\"\n",
            "data: ,\"delta\":\"Olá \"}\n\n",
            "data: {\"type\":\"response.output_text.delta\",\"delta\":\"mundo\"}\r\n\r\n",
            "data: {\"type\":\"response.completed\"}\n\n"
        );

        assert_eq!(decode_text(stream, 1).unwrap(), "Olá mundo");
    }

    #[test]
    fn authoritative_item_replaces_deltas_and_done_without_duplication() {
        let stream = concat!(
            "data: {\"type\":\"response.output_text.delta\",\"delta\":\"partial\"}\n\n",
            "data: {\"type\":\"response.output_text.done\",\"text\":\"done fallback\"}\n\n",
            "data: {\"type\":\"response.output_item.done\",\"item\":{\"type\":\"message\",\"role\":\"assistant\",\"content\":[{\"type\":\"output_text\",\"text\":\"authoritative\"}]}}\n\n",
            "data: {\"type\":\"response.completed\"}\n\n"
        );

        assert_eq!(decode_text(stream, 7).unwrap(), "authoritative");
    }

    #[test]
    fn assistant_item_without_output_text_preserves_deltas() {
        let stream = concat!(
            "data: {\"type\":\"response.output_text.delta\",\"delta\":\"complete delta text\"}\n\n",
            "data: {\"type\":\"response.output_item.done\",\"item\":{\"type\":\"message\",\"role\":\"assistant\",\"content\":[{\"type\":\"refusal\",\"refusal\":\"declined\"}]}}\n\n",
            "data: {\"type\":\"response.completed\"}\n\n"
        );

        assert_eq!(decode_text(stream, 7).unwrap(), "complete delta text");
    }

    #[test]
    fn unexpected_assistant_item_shapes_preserve_deltas() {
        let without_content = concat!(
            "data: {\"type\":\"response.output_text.delta\",\"delta\":\"complete delta text\"}\n\n",
            "data: {\"type\":\"response.output_item.done\",\"item\":{\"type\":\"message\",\"role\":\"assistant\"}}\n\n",
            "data: {\"type\":\"response.completed\"}\n\n"
        );
        let null_text = concat!(
            "data: {\"type\":\"response.output_text.delta\",\"delta\":\"complete delta text\"}\n\n",
            "data: {\"type\":\"response.output_item.done\",\"item\":{\"type\":\"message\",\"role\":\"assistant\",\"content\":[{\"type\":\"output_text\",\"text\":null}]}}\n\n",
            "data: {\"type\":\"response.completed\"}\n\n"
        );

        assert_eq!(
            decode_text(without_content, 7).unwrap(),
            "complete delta text"
        );
        assert_eq!(decode_text(null_text, 7).unwrap(), "complete delta text");
    }

    #[test]
    fn delta_precedes_done_and_done_is_the_final_fallback() {
        let with_delta = concat!(
            "data: {\"type\":\"response.output_text.delta\",\"delta\":\"delta\"}\n\n",
            "data: {\"type\":\"response.output_text.done\",\"text\":\"done\"}\n\n",
            "data: {\"type\":\"response.completed\"}\n\n"
        );
        let done_only = concat!(
            "data: {\"type\":\"response.output_text.done\",\"text\":\"done\"}\n\n",
            "data: {\"type\":\"response.completed\"}\n\n"
        );

        assert_eq!(decode_text(with_delta, 8192).unwrap(), "delta");
        assert_eq!(decode_text(done_only, 8192).unwrap(), "done");
    }

    #[test]
    fn terminal_failure_empty_text_and_eof_are_rejected() {
        for event_type in ["response.failed", "response.incomplete", "error"] {
            let stream = format!("data: {{\"type\":\"{event_type}\"}}\n\n");
            assert_eq!(decode_text(stream, 8192), Err(DecodeError::ModelFailed));
        }

        assert_eq!(
            decode_text("data: {\"type\":\"response.completed\"}\n\n", 8192),
            Err(DecodeError::EmptyText)
        );
        assert_eq!(
            decode_text(
                "data: {\"type\":\"response.output_text.delta\",\"delta\":\"partial\"}\n\n",
                8192
            ),
            Err(DecodeError::Invalid)
        );
    }

    #[test]
    fn malformed_relevant_events_and_invalid_utf8_are_rejected() {
        for stream in [
            b"data: not-json\n\n".to_vec(),
            b"data: {\"type\":\"response.output_text.delta\"}\n\n".to_vec(),
            b"data: {\"type\":\"response.output_item.done\"}\n\n".to_vec(),
            b"data: \xff\n\n".to_vec(),
        ] {
            assert_eq!(decode_text(stream, 1), Err(DecodeError::Invalid));
        }
    }

    #[test]
    fn response_event_text_and_total_budget_limits_are_enforced() {
        let mut response_limited = limits();
        response_limited.response_bytes = 10;
        assert_eq!(
            decode(
                &mut Cursor::new(b"data: too much\n\n"),
                response_limited,
                Instant::now()
            ),
            Err(DecodeError::LimitExceeded)
        );

        let mut event_limited = limits();
        event_limited.event_bytes = 8;
        assert_eq!(
            decode(
                &mut Cursor::new(b"data: too much\n\n"),
                event_limited,
                Instant::now()
            ),
            Err(DecodeError::LimitExceeded)
        );

        let mut text_limited = limits();
        text_limited.text_bytes = 3;
        assert_eq!(
            decode(
                &mut Cursor::new(
                    b"data: {\"type\":\"response.output_text.delta\",\"delta\":\"four\"}\n\n"
                ),
                text_limited,
                Instant::now()
            ),
            Err(DecodeError::LimitExceeded)
        );

        let mut timed_out = limits();
        timed_out.total_budget = Duration::ZERO;
        assert_eq!(
            decode(&mut Cursor::new(Vec::new()), timed_out, Instant::now()),
            Err(DecodeError::TimedOut)
        );
    }

    #[test]
    fn production_limits_match_the_transport_contract() {
        let limits = Limits::production();

        assert_eq!(limits.event_bytes, 1024 * 1024);
        assert_eq!(limits.response_bytes, 4 * 1024 * 1024);
        assert_eq!(limits.text_bytes, 256 * 1024);
        assert_eq!(limits.total_budget, Duration::from_secs(120));
    }
}

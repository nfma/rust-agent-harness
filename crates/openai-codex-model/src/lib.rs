mod sse;

use std::fmt;
use std::fmt::Write as _;
use std::time::{Duration, Instant};

use harness_openai_codex_auth::{AuthorizedCredential, CredentialReadError};
use rand::{RngCore, rngs::OsRng};
use reqwest::StatusCode;
use reqwest::blocking::{Client, RequestBuilder};
use reqwest::header::{ACCEPT, CONTENT_TYPE, USER_AGENT};
use reqwest::redirect::Policy;
use serde::Serialize;

const ENDPOINT: &str = "https://chatgpt.com/backend-api/codex/responses";
const MODEL: &str = "gpt-5.5";
const INSTRUCTIONS: &str =
    "Return only the requested plain-text verification response. Do not call tools.";
const PROMPT: &str = "Reply with exactly: OpenAI Codex connection verified.";
const ORIGINATOR: &str = "rust-agent-harness";
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ModelTestError {
    UnsupportedPlatform,
    NotConnected,
    CredentialStoreUnavailable,
    InvalidCredential,
    ExpiredCredential,
    Unavailable,
    UnexpectedProviderResponse,
    ConnectionRejected,
    AccessDenied,
    RateLimited,
    TemporarilyUnavailable,
    Rejected,
    InvalidProviderResponse,
    ModelCallFailed,
    EmptyResponse,
}

impl fmt::Display for ModelTestError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::UnsupportedPlatform => {
                "OpenAI Codex model testing is supported only on macOS"
            }
            Self::NotConnected => {
                "OpenAI Codex is not connected; run 'harness auth login openai-codex'"
            }
            Self::CredentialStoreUnavailable => {
                "the OpenAI Codex credential store is unavailable"
            }
            Self::InvalidCredential => {
                "the stored OpenAI Codex connection is invalid; run 'harness auth login openai-codex'"
            }
            Self::ExpiredCredential => {
                "the OpenAI Codex connection expired; run 'harness auth login openai-codex'"
            }
            Self::Unavailable => "OpenAI Codex is unavailable; try again",
            Self::UnexpectedProviderResponse => "OpenAI Codex returned an unexpected response",
            Self::ConnectionRejected => {
                "OpenAI Codex rejected the connection; run 'harness auth login openai-codex'"
            }
            Self::AccessDenied => "this account does not have access to the Codex model request",
            Self::RateLimited => "OpenAI Codex usage or rate limit reached",
            Self::TemporarilyUnavailable => "OpenAI Codex is temporarily unavailable; try again",
            Self::Rejected => "OpenAI Codex rejected the model request",
            Self::InvalidProviderResponse => "OpenAI Codex returned an invalid response",
            Self::ModelCallFailed => "the OpenAI Codex model call did not complete",
            Self::EmptyResponse => "OpenAI Codex returned no text",
        })
    }
}

impl std::error::Error for ModelTestError {}

pub fn test_connection() -> Result<String, ModelTestError> {
    match harness_openai_codex_auth::with_authorized_credential(|credential| {
        let mut correlation_ids = RandomCorrelationIds;
        test_connection_with(
            credential,
            &TransportConfig::production(),
            &mut correlation_ids,
        )
    }) {
        Ok(result) => result,
        Err(error) => Err(map_credential_error(error)),
    }
}

trait RequestAuthorizer {
    fn authorize(&self, request: RequestBuilder) -> RequestBuilder;
}

impl RequestAuthorizer for AuthorizedCredential {
    fn authorize(&self, request: RequestBuilder) -> RequestBuilder {
        self.authorize(request)
    }
}

trait CorrelationIds {
    fn next(&mut self) -> String;
}

struct RandomCorrelationIds;

impl CorrelationIds for RandomCorrelationIds {
    fn next(&mut self) -> String {
        let mut random = [0u8; 16];
        OsRng.fill_bytes(&mut random);
        let mut id = String::with_capacity(random.len() * 2);
        for byte in random {
            write!(id, "{byte:02x}").expect("writing to a String cannot fail");
        }
        id
    }
}

struct TransportConfig {
    endpoint: reqwest::Url,
    connect_timeout: Duration,
    request_timeout: Duration,
    limits: sse::Limits,
}

impl TransportConfig {
    fn production() -> Self {
        Self {
            endpoint: reqwest::Url::parse(ENDPOINT)
                .expect("production OpenAI Codex endpoint must be valid"),
            connect_timeout: CONNECT_TIMEOUT,
            request_timeout: REQUEST_TIMEOUT,
            limits: sse::Limits::production(),
        }
    }
}

#[derive(Serialize)]
struct RequestBody {
    model: &'static str,
    instructions: &'static str,
    input: [InputMessage; 1],
    tool_choice: &'static str,
    parallel_tool_calls: bool,
    reasoning: Reasoning,
    store: bool,
    stream: bool,
}

#[derive(Serialize)]
struct InputMessage {
    role: &'static str,
    content: [InputContent; 1],
}

#[derive(Serialize)]
struct InputContent {
    #[serde(rename = "type")]
    kind: &'static str,
    text: &'static str,
}

#[derive(Serialize)]
struct Reasoning {
    effort: &'static str,
}

fn request_body() -> RequestBody {
    RequestBody {
        model: MODEL,
        instructions: INSTRUCTIONS,
        input: [InputMessage {
            role: "user",
            content: [InputContent {
                kind: "input_text",
                text: PROMPT,
            }],
        }],
        tool_choice: "auto",
        parallel_tool_calls: false,
        reasoning: Reasoning { effort: "low" },
        store: false,
        stream: true,
    }
}

fn test_connection_with(
    authorizer: &dyn RequestAuthorizer,
    config: &TransportConfig,
    correlation_ids: &mut dyn CorrelationIds,
) -> Result<String, ModelTestError> {
    let started = Instant::now();
    let client = Client::builder()
        .connect_timeout(config.connect_timeout)
        .timeout(config.request_timeout)
        .redirect(Policy::none())
        .build()
        .map_err(|_| ModelTestError::Unavailable)?;
    let correlation_id = correlation_ids.next();
    let request = client
        .post(config.endpoint.clone())
        .header(ACCEPT, "text/event-stream")
        .header("originator", ORIGINATOR)
        .header(
            USER_AGENT,
            concat!("rust-agent-harness/", env!("CARGO_PKG_VERSION")),
        )
        .header("version", env!("CARGO_PKG_VERSION"))
        .header("session-id", &correlation_id)
        .header("x-client-request-id", &correlation_id)
        .json(&request_body());
    let mut response = authorizer
        .authorize(request)
        .send()
        .map_err(|_| ModelTestError::Unavailable)?;

    validate_status(response.status())?;
    validate_content_type(response.headers().get(CONTENT_TYPE))?;
    sse::decode(&mut response, config.limits, started).map_err(map_decode_error)
}

fn validate_status(status: StatusCode) -> Result<(), ModelTestError> {
    if status.is_success() {
        return Ok(());
    }
    let error = match status {
        StatusCode::UNAUTHORIZED => ModelTestError::ConnectionRejected,
        StatusCode::FORBIDDEN => ModelTestError::AccessDenied,
        StatusCode::REQUEST_TIMEOUT => ModelTestError::TemporarilyUnavailable,
        StatusCode::TOO_MANY_REQUESTS => ModelTestError::RateLimited,
        status if status.is_server_error() => ModelTestError::TemporarilyUnavailable,
        status if status.is_redirection() => ModelTestError::UnexpectedProviderResponse,
        status if status.is_client_error() => ModelTestError::Rejected,
        _ => ModelTestError::UnexpectedProviderResponse,
    };
    Err(error)
}

fn validate_content_type(
    content_type: Option<&reqwest::header::HeaderValue>,
) -> Result<(), ModelTestError> {
    let valid = content_type
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .is_some_and(|value| value.trim().eq_ignore_ascii_case("text/event-stream"));
    if valid {
        Ok(())
    } else {
        Err(ModelTestError::InvalidProviderResponse)
    }
}

fn map_credential_error(error: CredentialReadError) -> ModelTestError {
    match error {
        CredentialReadError::UnsupportedPlatform => ModelTestError::UnsupportedPlatform,
        CredentialReadError::NotConnected => ModelTestError::NotConnected,
        CredentialReadError::StoreUnavailable => ModelTestError::CredentialStoreUnavailable,
        CredentialReadError::UnsupportedVersion | CredentialReadError::InvalidCredential => {
            ModelTestError::InvalidCredential
        }
        CredentialReadError::Expired => ModelTestError::ExpiredCredential,
    }
}

fn map_decode_error(error: sse::DecodeError) -> ModelTestError {
    match error {
        sse::DecodeError::Invalid | sse::DecodeError::LimitExceeded => {
            ModelTestError::InvalidProviderResponse
        }
        sse::DecodeError::TimedOut => ModelTestError::Unavailable,
        sse::DecodeError::ModelFailed => ModelTestError::ModelCallFailed,
        sse::DecodeError::EmptyText => ModelTestError::EmptyResponse,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{HashMap, VecDeque};
    use std::io::{Read, Write};
    use std::net::{Ipv4Addr, TcpListener, TcpStream};
    use std::sync::mpsc;
    use std::thread;
    use std::time::Instant;

    use serde_json::{Value, json};
    use tiny_http::{Header, Response, Server};

    use super::*;

    const ACCESS_TOKEN: &str = "model-access-token-sentinel";
    const ACCOUNT_ID: &str = "model-account-id-sentinel";
    const REFRESH_TOKEN: &str = "model-refresh-token-sentinel";
    const ID_TOKEN: &str = "model-id-token-sentinel";
    const KEYCHAIN_BYTES: &str = "model-keychain-bytes-sentinel";

    struct FakeAuthorizer;

    impl RequestAuthorizer for FakeAuthorizer {
        fn authorize(&self, request: RequestBuilder) -> RequestBuilder {
            request
                .bearer_auth(ACCESS_TOKEN)
                .header("ChatGPT-Account-ID", ACCOUNT_ID)
        }
    }

    struct FixedCorrelationIds {
        values: VecDeque<String>,
    }

    impl CorrelationIds for FixedCorrelationIds {
        fn next(&mut self) -> String {
            self.values.pop_front().expect("a correlation id")
        }
    }

    #[derive(Debug)]
    struct CapturedRequest {
        method: String,
        target: String,
        headers: HashMap<String, String>,
        body: Vec<u8>,
    }

    struct FakeServer {
        endpoint: reqwest::Url,
        thread: thread::JoinHandle<Vec<CapturedRequest>>,
    }

    impl FakeServer {
        fn responses(responses: Vec<(u16, Option<&'static str>, Vec<u8>)>) -> Self {
            let server = Server::http((Ipv4Addr::LOCALHOST, 0)).unwrap();
            let port = server.server_addr().to_ip().unwrap().port();
            let thread = thread::spawn(move || {
                let mut captured = Vec::new();
                for (status, content_type, body) in responses {
                    let mut request = server.recv().unwrap();
                    let mut request_body = Vec::new();
                    request.as_reader().read_to_end(&mut request_body).unwrap();
                    let headers = request
                        .headers()
                        .iter()
                        .map(|header| {
                            (
                                header.field.to_string().to_ascii_lowercase(),
                                header.value.as_str().to_owned(),
                            )
                        })
                        .collect();
                    captured.push(CapturedRequest {
                        method: request.method().as_str().to_owned(),
                        target: request.url().to_owned(),
                        headers,
                        body: request_body,
                    });
                    let mut response = Response::from_data(body).with_status_code(status);
                    if let Some(content_type) = content_type {
                        response
                            .add_header(Header::from_bytes("Content-Type", content_type).unwrap());
                    }
                    request.respond(response).unwrap();
                }
                captured
            });
            Self {
                endpoint: reqwest::Url::parse(&format!(
                    "http://127.0.0.1:{port}/backend-api/codex/responses"
                ))
                .unwrap(),
                thread,
            }
        }

        fn finish(self) -> Vec<CapturedRequest> {
            self.thread.join().unwrap()
        }
    }

    fn completed(text: &str) -> Vec<u8> {
        format!(
            "data: {{\"type\":\"response.output_text.delta\",\"delta\":{}}}\n\ndata: {{\"type\":\"response.completed\"}}\n\n",
            serde_json::to_string(text).unwrap()
        )
        .into_bytes()
    }

    fn config(endpoint: reqwest::Url) -> TransportConfig {
        TransportConfig {
            endpoint,
            connect_timeout: Duration::from_millis(100),
            request_timeout: Duration::from_secs(1),
            limits: sse::Limits {
                event_bytes: 8 * 1024,
                response_bytes: 32 * 1024,
                text_bytes: 4 * 1024,
                total_budget: Duration::from_secs(1),
            },
        }
    }

    fn ids(values: &[&str]) -> FixedCorrelationIds {
        FixedCorrelationIds {
            values: values.iter().map(|value| (*value).to_owned()).collect(),
        }
    }

    #[test]
    fn request_contract_is_exact_and_excludes_deferred_headers_and_fields() {
        let server = FakeServer::responses(vec![(
            200,
            Some("text/event-stream; charset=utf-8"),
            completed("verified"),
        )]);
        let mut correlation_ids = ids(&["correlation-one"]);

        let result = test_connection_with(
            &FakeAuthorizer,
            &config(server.endpoint.clone()),
            &mut correlation_ids,
        );
        let requests = server.finish();

        assert_eq!(result.as_deref(), Ok("verified"));
        assert_eq!(requests.len(), 1);
        let request = &requests[0];
        assert_eq!(request.method, "POST");
        assert_eq!(request.target, "/backend-api/codex/responses");
        assert_eq!(
            request.headers["authorization"],
            format!("Bearer {ACCESS_TOKEN}")
        );
        assert_eq!(request.headers["chatgpt-account-id"], ACCOUNT_ID);
        assert_eq!(request.headers["originator"], "rust-agent-harness");
        assert_eq!(
            request.headers["user-agent"],
            concat!("rust-agent-harness/", env!("CARGO_PKG_VERSION"))
        );
        assert_eq!(request.headers["version"], env!("CARGO_PKG_VERSION"));
        assert_eq!(request.headers["accept"], "text/event-stream");
        assert_eq!(request.headers["content-type"], "application/json");
        assert_eq!(request.headers["session-id"], "correlation-one");
        assert_eq!(
            request.headers["x-client-request-id"],
            request.headers["session-id"]
        );
        for absent in [
            "cookie",
            "openai-beta",
            "openai-organization",
            "openai-project",
            "thread-id",
            "x-codex-beta-features",
        ] {
            assert!(!request.headers.contains_key(absent), "unexpected {absent}");
        }

        let body: Value = serde_json::from_slice(&request.body).unwrap();
        assert_eq!(
            body,
            json!({
                "model": "gpt-5.5",
                "instructions": "Return only the requested plain-text verification response. Do not call tools.",
                "input": [{
                    "role": "user",
                    "content": [{
                        "type": "input_text",
                        "text": "Reply with exactly: OpenAI Codex connection verified."
                    }]
                }],
                "tool_choice": "auto",
                "parallel_tool_calls": false,
                "reasoning": {"effort": "low"},
                "store": false,
                "stream": true
            })
        );
        for absent in ["tools", "include", "text", "thread_id", "session_id"] {
            assert!(body.get(absent).is_none(), "unexpected body field {absent}");
        }
    }

    #[test]
    fn each_call_uses_one_distinct_correlation_id_for_both_headers() {
        let server = FakeServer::responses(vec![
            (200, Some("text/event-stream"), completed("one")),
            (200, Some("text/event-stream"), completed("two")),
        ]);
        let transport = config(server.endpoint.clone());
        let mut correlation_ids = ids(&["correlation-one", "correlation-two"]);

        assert_eq!(
            test_connection_with(&FakeAuthorizer, &transport, &mut correlation_ids).as_deref(),
            Ok("one")
        );
        assert_eq!(
            test_connection_with(&FakeAuthorizer, &transport, &mut correlation_ids).as_deref(),
            Ok("two")
        );
        let requests = server.finish();

        assert_eq!(requests[0].headers["session-id"], "correlation-one");
        assert_eq!(requests[1].headers["session-id"], "correlation-two");
        for request in requests {
            assert_eq!(
                request.headers["session-id"],
                request.headers["x-client-request-id"]
            );
        }
    }

    #[test]
    fn http_statuses_are_categorized_without_exposing_provider_bodies() {
        for (status, expected) in [
            (301, ModelTestError::UnexpectedProviderResponse),
            (401, ModelTestError::ConnectionRejected),
            (403, ModelTestError::AccessDenied),
            (408, ModelTestError::TemporarilyUnavailable),
            (418, ModelTestError::Rejected),
            (429, ModelTestError::RateLimited),
            (500, ModelTestError::TemporarilyUnavailable),
            (503, ModelTestError::TemporarilyUnavailable),
        ] {
            let provider_body = format!(
                "provider {ACCESS_TOKEN} {ACCOUNT_ID} {REFRESH_TOKEN} {ID_TOKEN} {KEYCHAIN_BYTES}"
            );
            let server = FakeServer::responses(vec![(
                status,
                Some("text/plain"),
                provider_body.into_bytes(),
            )]);
            let mut correlation_ids = ids(&["status-correlation"]);

            let error = test_connection_with(
                &FakeAuthorizer,
                &config(server.endpoint.clone()),
                &mut correlation_ids,
            )
            .unwrap_err();
            server.finish();

            assert_eq!(error, expected);
            let rendered = format!("{error:?} {error}");
            for sentinel in [
                ACCESS_TOKEN,
                ACCOUNT_ID,
                REFRESH_TOKEN,
                ID_TOKEN,
                KEYCHAIN_BYTES,
            ] {
                assert!(!rendered.contains(sentinel));
            }
        }
    }

    #[test]
    fn redirects_are_rejected_without_contacting_the_location() {
        let target = Server::http((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let target_port = target.server_addr().to_ip().unwrap().port();
        let target_thread = thread::spawn(move || {
            target
                .recv_timeout(Duration::from_millis(200))
                .unwrap()
                .is_some()
        });
        let redirect = Server::http((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let redirect_port = redirect.server_addr().to_ip().unwrap().port();
        let redirect_thread = thread::spawn(move || {
            let request = redirect.recv().unwrap();
            let response = Response::empty(302).with_header(
                Header::from_bytes(
                    "Location",
                    format!("http://127.0.0.1:{target_port}/must-not-be-contacted"),
                )
                .unwrap(),
            );
            request.respond(response).unwrap();
        });
        let endpoint = reqwest::Url::parse(&format!(
            "http://127.0.0.1:{redirect_port}/backend-api/codex/responses"
        ))
        .unwrap();
        let mut correlation_ids = ids(&["redirect-correlation"]);

        let result = test_connection_with(&FakeAuthorizer, &config(endpoint), &mut correlation_ids);
        redirect_thread.join().unwrap();
        let target_contacted = target_thread.join().unwrap();

        assert_eq!(result, Err(ModelTestError::UnexpectedProviderResponse));
        assert!(!target_contacted);
    }

    #[test]
    fn wrong_content_type_and_bounded_response_fail_without_provider_data() {
        let cases = [
            (Some("application/json"), completed("hidden"), 32 * 1024),
            (None, completed("hidden"), 32 * 1024),
            (Some("text/event-stream"), vec![b'x'; 1024], 100),
        ];

        for (content_type, body, response_bytes) in cases {
            let server = FakeServer::responses(vec![(200, content_type, body)]);
            let mut transport = config(server.endpoint.clone());
            transport.limits.response_bytes = response_bytes;
            let mut correlation_ids = ids(&["invalid-response"]);

            let result = test_connection_with(&FakeAuthorizer, &transport, &mut correlation_ids);
            server.finish();

            assert_eq!(result, Err(ModelTestError::InvalidProviderResponse));
        }
    }

    fn raw_server(
        response: &'static [u8],
        hold_open: Option<mpsc::Receiver<()>>,
    ) -> (reqwest::Url, thread::JoinHandle<()>) {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let thread = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            read_request(&mut stream);
            stream.write_all(response).unwrap();
            stream.flush().unwrap();
            if let Some(release) = hold_open {
                let _ = release.recv_timeout(Duration::from_secs(1));
            }
        });
        (
            reqwest::Url::parse(&format!(
                "http://127.0.0.1:{port}/backend-api/codex/responses"
            ))
            .unwrap(),
            thread,
        )
    }

    fn read_request(stream: &mut TcpStream) {
        let mut bytes = Vec::new();
        let mut buffer = [0u8; 1024];
        while !bytes.windows(4).any(|window| window == b"\r\n\r\n") {
            let read = stream.read(&mut buffer).unwrap();
            if read == 0 {
                break;
            }
            bytes.extend_from_slice(&buffer[..read]);
        }
    }

    #[test]
    fn completed_event_returns_before_the_server_closes_the_socket() {
        let (release_tx, release_rx) = mpsc::channel();
        let response = b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\ndata: {\"type\":\"response.output_text.done\",\"text\":\"complete\"}\n\ndata: {\"type\":\"response.completed\"}\n\n";
        let (endpoint, server) = raw_server(response, Some(release_rx));
        let mut correlation_ids = ids(&["early-terminal"]);
        let started = Instant::now();

        let result = test_connection_with(&FakeAuthorizer, &config(endpoint), &mut correlation_ids);
        let elapsed = started.elapsed();
        release_tx.send(()).unwrap();
        server.join().unwrap();

        assert_eq!(result.as_deref(), Ok("complete"));
        assert!(elapsed < Duration::from_millis(500));
    }

    #[test]
    fn header_wait_read_idle_and_total_budget_timeouts_are_unavailable() {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let header_server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            read_request(&mut stream);
            thread::sleep(Duration::from_millis(100));
        });
        let endpoint = reqwest::Url::parse(&format!("http://127.0.0.1:{port}/responses")).unwrap();
        let mut transport = config(endpoint);
        transport.request_timeout = Duration::from_millis(20);
        let mut correlation_ids = ids(&["header-timeout"]);
        assert_eq!(
            test_connection_with(&FakeAuthorizer, &transport, &mut correlation_ids),
            Err(ModelTestError::Unavailable)
        );
        header_server.join().unwrap();

        let response =
            b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n";
        let (_release_tx, release_rx) = mpsc::channel();
        let (endpoint, read_server) = raw_server(response, Some(release_rx));
        let mut transport = config(endpoint);
        transport.request_timeout = Duration::from_millis(20);
        let mut correlation_ids = ids(&["read-timeout"]);
        assert_eq!(
            test_connection_with(&FakeAuthorizer, &transport, &mut correlation_ids),
            Err(ModelTestError::Unavailable)
        );
        read_server.join().unwrap();

        let server = FakeServer::responses(vec![(
            200,
            Some("text/event-stream"),
            completed("too late"),
        )]);
        let mut transport = config(server.endpoint.clone());
        transport.limits.total_budget = Duration::ZERO;
        let mut correlation_ids = ids(&["total-timeout"]);
        assert_eq!(
            test_connection_with(&FakeAuthorizer, &transport, &mut correlation_ids),
            Err(ModelTestError::Unavailable)
        );
        server.finish();
    }

    #[test]
    fn unavailable_endpoint_and_decode_failures_are_categorized() {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        let endpoint = reqwest::Url::parse(&format!("http://127.0.0.1:{port}/responses")).unwrap();
        let mut correlation_ids = ids(&["connect-failure"]);
        assert_eq!(
            test_connection_with(&FakeAuthorizer, &config(endpoint), &mut correlation_ids),
            Err(ModelTestError::Unavailable)
        );

        for (body, expected) in [
            (
                b"data: {\"type\":\"response.failed\"}\n\n".to_vec(),
                ModelTestError::ModelCallFailed,
            ),
            (
                b"data: {\"type\":\"response.completed\"}\n\n".to_vec(),
                ModelTestError::EmptyResponse,
            ),
            (
                b"data: not-json\n\n".to_vec(),
                ModelTestError::InvalidProviderResponse,
            ),
            (
                b"data: {\"type\":\"response.output_text.delta\",\"delta\":\"partial\"}\n\n"
                    .to_vec(),
                ModelTestError::InvalidProviderResponse,
            ),
        ] {
            let server = FakeServer::responses(vec![(200, Some("text/event-stream"), body)]);
            let mut correlation_ids = ids(&["decode-failure"]);
            let result = test_connection_with(
                &FakeAuthorizer,
                &config(server.endpoint.clone()),
                &mut correlation_ids,
            );
            server.finish();
            assert_eq!(result, Err(expected));
        }
    }

    #[test]
    fn error_variants_and_configuration_are_secret_free_and_fixed() {
        for error in [
            ModelTestError::UnsupportedPlatform,
            ModelTestError::NotConnected,
            ModelTestError::CredentialStoreUnavailable,
            ModelTestError::InvalidCredential,
            ModelTestError::ExpiredCredential,
            ModelTestError::Unavailable,
            ModelTestError::UnexpectedProviderResponse,
            ModelTestError::ConnectionRejected,
            ModelTestError::AccessDenied,
            ModelTestError::RateLimited,
            ModelTestError::TemporarilyUnavailable,
            ModelTestError::Rejected,
            ModelTestError::InvalidProviderResponse,
            ModelTestError::ModelCallFailed,
            ModelTestError::EmptyResponse,
        ] {
            let rendered = format!("{error:?} {error}");
            for sentinel in [
                ACCESS_TOKEN,
                ACCOUNT_ID,
                REFRESH_TOKEN,
                ID_TOKEN,
                KEYCHAIN_BYTES,
            ] {
                assert!(!rendered.contains(sentinel));
            }
        }

        let production = TransportConfig::production();
        assert_eq!(production.endpoint.as_str(), ENDPOINT);
        assert_eq!(production.connect_timeout, Duration::from_secs(10));
        assert_eq!(production.request_timeout, Duration::from_secs(60));
    }

    #[test]
    fn random_correlation_ids_have_a_header_safe_fixed_width_shape() {
        let mut generator = RandomCorrelationIds;
        let first = generator.next();

        assert_eq!(first.len(), 32);
        assert!(first.bytes().all(|byte| byte.is_ascii_hexdigit()));
    }
}

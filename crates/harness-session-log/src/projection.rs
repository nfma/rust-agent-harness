use std::collections::HashSet;

use crate::format::{
    FORMAT_VERSION, FailureCode, MAX_DIAGNOSTIC_BYTES, MAX_RECORD_BYTES, MAX_SESSION_BYTES,
    Payload, Record,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SessionStatus {
    Completed,
    Failed,
    Interrupted,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProjectedTerminal {
    Assistant(String),
    Failure(FailureCode),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SessionProjection {
    session_id: String,
    prompt: String,
    status: SessionStatus,
    terminal: Option<ProjectedTerminal>,
}

impl SessionProjection {
    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    pub fn prompt(&self) -> &str {
        &self.prompt
    }

    pub fn status(&self) -> &SessionStatus {
        &self.status
    }

    pub fn terminal(&self) -> Option<&ProjectedTerminal> {
        self.terminal.as_ref()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ShowResult {
    projection: SessionProjection,
    trailing_fragment: bool,
}

impl ShowResult {
    pub fn projection(&self) -> &SessionProjection {
        &self.projection
    }

    pub fn has_trailing_fragment(&self) -> bool {
        self.trailing_fragment
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ProjectionError {
    UnsupportedVersion,
    Corrupt,
}

pub(crate) fn project(
    bytes: &[u8],
    expected_session_id: &str,
) -> Result<ShowResult, ProjectionError> {
    let (records, trailing_fragment) = parse_records(bytes)?;

    let Some(Record::Header {
        format_version,
        session_id,
        ..
    }) = records.first()
    else {
        return Err(ProjectionError::Corrupt);
    };
    if *format_version != FORMAT_VERSION {
        return Err(ProjectionError::UnsupportedVersion);
    }
    if session_id != expected_session_id || !valid_id(session_id) {
        return Err(ProjectionError::Corrupt);
    }
    if records.len() < 2 {
        return Err(ProjectionError::Corrupt);
    }

    let Record::Entry {
        entry_id: user_entry_id,
        parent_entry_id,
        sequence,
        operation_id,
        payload,
        ..
    } = &records[1]
    else {
        return Err(ProjectionError::Corrupt);
    };
    if !valid_id(user_entry_id)
        || !valid_id(operation_id)
        || parent_entry_id.is_some()
        || *sequence != 1
    {
        return Err(ProjectionError::Corrupt);
    }
    let Payload::UserMessage { text: prompt, .. } = payload else {
        return Err(ProjectionError::Corrupt);
    };
    if HashSet::from([session_id.as_str(), user_entry_id.as_str(), operation_id]).len() != 3 {
        return Err(ProjectionError::Corrupt);
    }

    let terminal = project_terminal(records.get(2), session_id, user_entry_id, operation_id)?;
    let status = match terminal {
        Some(ProjectedTerminal::Assistant(_)) => SessionStatus::Completed,
        Some(ProjectedTerminal::Failure(_)) => SessionStatus::Failed,
        None => SessionStatus::Interrupted,
    };

    Ok(ShowResult {
        projection: SessionProjection {
            session_id: session_id.clone(),
            prompt: prompt.clone(),
            status,
            terminal,
        },
        trailing_fragment,
    })
}

fn parse_records(bytes: &[u8]) -> Result<(Vec<Record>, bool), ProjectionError> {
    if bytes.len() > MAX_SESSION_BYTES {
        return Err(ProjectionError::Corrupt);
    }
    let Some(complete_end) = bytes
        .iter()
        .rposition(|byte| *byte == b'\n')
        .map(|index| index + 1)
    else {
        return Err(ProjectionError::Corrupt);
    };
    let trailing_fragment = complete_end != bytes.len();
    let complete = &bytes[..complete_end - 1];
    if complete.is_empty() {
        return Err(ProjectionError::Corrupt);
    }
    let mut records = Vec::with_capacity(3);
    for line in complete.split(|byte| *byte == b'\n') {
        if line.is_empty() {
            return Err(ProjectionError::Corrupt);
        }
        if line.len() > MAX_RECORD_BYTES || records.len() == 3 {
            return Err(ProjectionError::Corrupt);
        }
        records.push(serde_json::from_slice::<Record>(line).map_err(|_| ProjectionError::Corrupt)?);
    }
    Ok((records, trailing_fragment))
}

fn project_terminal(
    record: Option<&Record>,
    session_id: &str,
    user_entry_id: &str,
    operation_id: &str,
) -> Result<Option<ProjectedTerminal>, ProjectionError> {
    match record {
        None => Ok(None),
        Some(Record::Entry {
            entry_id,
            parent_entry_id,
            sequence,
            operation_id: terminal_operation_id,
            payload,
            ..
        }) => {
            if !valid_id(entry_id)
                || entry_id == session_id
                || entry_id == user_entry_id
                || entry_id == operation_id
                || parent_entry_id.as_deref() != Some(user_entry_id)
                || *sequence != 2
                || terminal_operation_id != operation_id
            {
                return Err(ProjectionError::Corrupt);
            }
            match payload {
                Payload::AssistantMessage { text, .. } => {
                    Ok(Some(ProjectedTerminal::Assistant(text.clone())))
                }
                Payload::TerminalFailure {
                    failure_code,
                    diagnostic,
                    ..
                } if diagnostic.len() <= MAX_DIAGNOSTIC_BYTES
                    && diagnostic == failure_code.diagnostic() =>
                {
                    Ok(Some(ProjectedTerminal::Failure(*failure_code)))
                }
                _ => Err(ProjectionError::Corrupt),
            }
        }
        Some(_) => Err(ProjectionError::Corrupt),
    }
}

pub(crate) fn valid_id(value: &str) -> bool {
    value.len() == 32
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

use serde::{Deserialize, Serialize};

pub(crate) const FORMAT_VERSION: u64 = 1;
pub(crate) const MAX_RECORD_BYTES: usize = 2 * 1024 * 1024;
pub(crate) const MAX_SESSION_BYTES: usize = 3 * 1024 * 1024;
pub(crate) const MAX_DIAGNOSTIC_BYTES: usize = 4 * 1024;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureCode {
    UnsupportedPlatform,
    NotConnected,
    CredentialStoreUnavailable,
    InvalidCredential,
    ExpiredCredential,
    CredentialRefreshBusy,
    CredentialRefreshUnavailable,
    Unavailable,
    UnexpectedProviderResponse,
    ConnectionRejected,
    AccessDenied,
    RateLimited,
    TemporarilyUnavailable,
    Rejected,
    InvalidProviderResponse,
    CallTimedOut,
    ResponseTooLarge,
    ModelCallFailed,
    EmptyResponse,
}

impl FailureCode {
    pub fn diagnostic(self) -> &'static str {
        match self {
            Self::UnsupportedPlatform => "OpenAI Codex model calls are supported only on macOS",
            Self::NotConnected => {
                "OpenAI Codex is not connected; run 'harness auth login openai-codex'"
            }
            Self::CredentialStoreUnavailable => "the OpenAI Codex credential store is unavailable",
            Self::InvalidCredential => {
                "the stored OpenAI Codex connection is invalid; run 'harness auth login openai-codex'"
            }
            Self::ExpiredCredential => {
                "the OpenAI Codex connection expired; run 'harness auth login openai-codex'"
            }
            Self::CredentialRefreshBusy => {
                "OpenAI Codex credential refresh is already in progress; try again"
            }
            Self::CredentialRefreshUnavailable => {
                "OpenAI Codex credential refresh is temporarily unavailable; try again"
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
            Self::CallTimedOut => "the OpenAI Codex model call timed out",
            Self::ResponseTooLarge => {
                "OpenAI Codex returned a response that exceeded harness limits"
            }
            Self::ModelCallFailed => "the OpenAI Codex model call did not complete",
            Self::EmptyResponse => "OpenAI Codex returned no text",
        }
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields, tag = "record_type")]
pub(crate) enum Record {
    #[serde(rename = "session_header")]
    Header {
        format_version: u64,
        session_id: String,
        created_at_unix_ms: u64,
        working_directory: Option<String>,
        harness_version: String,
    },
    #[serde(rename = "entry")]
    Entry {
        entry_id: String,
        parent_entry_id: Option<String>,
        sequence: u64,
        timestamp_unix_ms: u64,
        operation_id: String,
        payload: Payload,
    },
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields, tag = "type")]
pub(crate) enum Payload {
    #[serde(rename = "user_message")]
    UserMessage { provider: Provider, text: String },
    #[serde(rename = "assistant_message")]
    AssistantMessage {
        provider: Provider,
        text: String,
        operation_outcome: CompletedOutcome,
    },
    #[serde(rename = "terminal_failure")]
    TerminalFailure {
        provider: Provider,
        failure_code: FailureCode,
        diagnostic: String,
        operation_outcome: FailedOutcome,
    },
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) enum Provider {
    #[serde(rename = "openai-codex")]
    OpenAiCodex,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
pub(crate) enum CompletedOutcome {
    #[serde(rename = "completed")]
    Completed,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
pub(crate) enum FailedOutcome {
    #[serde(rename = "failed")]
    Failed,
}

pub(crate) fn header(
    session_id: String,
    created_at_unix_ms: u64,
    working_directory: Option<String>,
    harness_version: String,
) -> Record {
    Record::Header {
        format_version: FORMAT_VERSION,
        session_id,
        created_at_unix_ms,
        working_directory,
        harness_version,
    }
}

pub(crate) fn user_entry(
    entry_id: String,
    timestamp_unix_ms: u64,
    operation_id: String,
    text: String,
) -> Record {
    Record::Entry {
        entry_id,
        parent_entry_id: None,
        sequence: 1,
        timestamp_unix_ms,
        operation_id,
        payload: Payload::UserMessage {
            provider: Provider::OpenAiCodex,
            text,
        },
    }
}

pub(crate) fn assistant_entry(
    entry_id: String,
    user_entry_id: String,
    timestamp_unix_ms: u64,
    operation_id: String,
    text: String,
) -> Record {
    Record::Entry {
        entry_id,
        parent_entry_id: Some(user_entry_id),
        sequence: 2,
        timestamp_unix_ms,
        operation_id,
        payload: Payload::AssistantMessage {
            provider: Provider::OpenAiCodex,
            text,
            operation_outcome: CompletedOutcome::Completed,
        },
    }
}

pub(crate) fn failure_entry(
    entry_id: String,
    user_entry_id: String,
    timestamp_unix_ms: u64,
    operation_id: String,
    failure_code: FailureCode,
) -> Record {
    Record::Entry {
        entry_id,
        parent_entry_id: Some(user_entry_id),
        sequence: 2,
        timestamp_unix_ms,
        operation_id,
        payload: Payload::TerminalFailure {
            provider: Provider::OpenAiCodex,
            failure_code,
            diagnostic: failure_code.diagnostic().to_owned(),
            operation_outcome: FailedOutcome::Failed,
        },
    }
}

pub(crate) fn serialize_line(record: &Record) -> Result<Vec<u8>, ()> {
    let mut bytes = serde_json::to_vec(record).map_err(|_| ())?;
    if bytes.len() > MAX_RECORD_BYTES {
        return Err(());
    }
    bytes.push(b'\n');
    Ok(bytes)
}

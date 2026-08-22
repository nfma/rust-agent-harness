mod file;
mod format;
mod projection;

use std::fmt;

pub use file::{SessionLog, SessionWriter};
pub use format::FailureCode;
pub use projection::{ProjectedTerminal, SessionProjection, SessionStatus, ShowResult};

pub const TRAILING_FRAGMENT_WARNING: &str = "ignored incomplete trailing session data";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CreateError {
    StoreUnavailable,
}

impl fmt::Display for CreateError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("unable to create the local session")
    }
}

impl std::error::Error for CreateError {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AppendError {
    StoreUnavailable,
}

impl fmt::Display for AppendError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("unable to persist the local session result")
    }
}

impl std::error::Error for AppendError {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RollbackError {
    StoreUnavailable,
}

impl fmt::Display for RollbackError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("unable to roll back the invalid local session")
    }
}

impl std::error::Error for RollbackError {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ShowError {
    InvalidSessionId,
    Missing,
    Busy,
    UnsupportedVersion,
    Corrupt,
    StoreUnavailable,
}

impl fmt::Display for ShowError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidSessionId => "the session identifier is invalid",
            Self::Missing => "the session was not found",
            Self::Busy => "the session is still being written; try again",
            Self::UnsupportedVersion => "the session format version is not supported",
            Self::Corrupt => "the session data is corrupt",
            Self::StoreUnavailable => "the local session store is unavailable",
        })
    }
}

impl std::error::Error for ShowError {}

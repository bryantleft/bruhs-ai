use std::io;
use thiserror::Error;

/// Why a query could not be started or configured.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum Error {
    /// The `codex` process could not be spawned.
    #[error("failed to spawn the codex CLI: {0}")]
    Spawn(#[source] io::Error),

    /// `codex` is not on `PATH`.
    #[error("codex CLI not found on PATH; is Codex installed?")]
    CliNotFound,

    /// A resume thread id started with `-` and could be read as a flag.
    #[error("invalid session id {0:?}: must not start with '-'")]
    InvalidSessionId(String),

    /// A [`ConfigOverride`](crate::ConfigOverride) key was empty or
    /// contained `=`.
    #[error("invalid config key {0:?}: must be non-empty and contain no '='")]
    InvalidConfigKey(String),
}

impl Error {
    pub(crate) fn spawn(error: io::Error) -> Self {
        match error.kind() {
            io::ErrorKind::NotFound => Self::CliNotFound,
            _ => Self::Spawn(error),
        }
    }
}

/// `Result` with this crate's [`enum@Error`].
pub type Result<T> = std::result::Result<T, Error>;

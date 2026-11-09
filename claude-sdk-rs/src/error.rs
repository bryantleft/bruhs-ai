use std::io;
use thiserror::Error;

/// Why a query could not be started or answered.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum Error {
    /// The `claude` process could not be spawned.
    #[error("failed to spawn the claude CLI: {0}")]
    Spawn(#[source] io::Error),

    /// Spawning failed with "not found": `claude` is not on `PATH`, or the
    /// working directory does not exist (the OS reports both the same way).
    #[error(
        "could not start claude: the CLI is not on PATH or the working directory does not exist"
    )]
    CliNotFound,

    /// A permission answer was sent after the CLI's stdin closed (the turn
    /// ended), or to a query without `permission_prompts`.
    #[error("the claude CLI's stdin is closed")]
    StdinClosed,

    /// Writing to the CLI's stdin failed.
    #[error("writing to the claude CLI's stdin failed: {0}")]
    Stdin(#[source] io::Error),

    /// A resume session id started with `-` and could be read as a flag.
    #[error("invalid session id {0:?}: must not start with '-'")]
    InvalidSessionId(String),
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

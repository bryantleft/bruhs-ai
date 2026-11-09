//! A small async wrapper around the [Codex](https://github.com/openai/codex)
//! CLI. [`query()`] spawns the local `codex` binary as `codex exec --json`,
//! writes the prompt to its stdin, and returns a [`QueryStream`] of typed
//! [`QueryEvent`]s. Authentication is whatever the local Codex CLI already
//! uses.
//!
//! - The prompt never reaches argv, so a prompt such as `--help` or `-` is
//!   always text.
//! - Dropping the stream kills and reaps the CLI.
//! - A CLI that exits unsuccessfully ends the stream with
//!   [`QueryEvent::ProcessFailed`], carrying the exit code and the tail of its
//!   stderr.
//! - New threads are saved so a later query can [resume](QueryOptions::resume)
//!   them; [`Session::Ephemeral`] opts out.
//!
//! ```no_run
//! use codex_sdk_rs::{query, QueryEvent, QueryOptions};
//! use tokio_stream::StreamExt;
//!
//! # async fn run() -> Result<(), codex_sdk_rs::Error> {
//! let mut stream = query(QueryOptions::new("List the files").cwd("."))?;
//! while let Some(event) = stream.next().await {
//!     if let QueryEvent::Text(text) = event {
//!         println!("{text}");
//!     }
//! }
//! # Ok(())
//! # }
//! ```
//!
//! This crate is internal to the bruhs-ai workspace and is not published.
#![warn(missing_docs)]

mod error;
// Shared with claude-sdk-rs, which owns and publishes this file; see its
// module docs for why it is not a separate crate.
#[path = "../../claude-sdk-rs/src/process.rs"]
mod process;
mod query;
mod rollout;
mod types;

pub use error::{Error, Result};
pub use process::ProcessFailure;
pub use query::{query, QueryStream};
pub use types::{
    ApprovalPolicy, CodexAccess, ConfigOverride, QueryEvent, QueryOptions, SandboxMode, Session,
    UsageInfo,
};

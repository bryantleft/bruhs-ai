#![doc = include_str!("../README.md")]
#![warn(missing_docs)]

mod error;
mod process;
mod query;
mod types;

pub use error::{Error, Result};
pub use process::ProcessFailure;
pub use query::{query, PermissionResponder, QueryStream};
pub use types::{
    McpServerInfo, PermissionDecision, PermissionMode, QueryEvent, QueryOptions, Tools, UsageInfo,
};

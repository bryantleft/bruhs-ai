//! Pure data types for bruhs-ai. No async, no I/O, no platform deps —
//! safe to compile to any Rust target including `wasm32-unknown-unknown`.
//!
//! Consumed by `bruhs-core` (server-side orchestration), `bruhs-web` (Leptos
//! WASM SPA), `bruhs-desktop` (GPUI native), and `bruhs-mcp` (MCP server).
//!
//! - [`api`]: request/response bodies and the agent stream protocol shared by
//!   the server, web and desktop.
//! - [`agent`]: permission modes, providers, roles and the model catalog.
//! - [`ids`]: validated id newtypes; [`time`]: [`EpochMs`] and date helpers.
//! - [`num`]: named lossy numeric conversions (rounding, saturating).
//! - [`updates`]: provider CLI and app update reports and their rules.
//! - [`logic`]: pure logic the web and desktop both run (diffs, WAV, tool
//!   labels, git parsing, …).

pub mod agent;
pub mod api;
pub mod calendar;
pub mod capture_auth;
pub mod capture_recovery;
pub mod connections;
pub mod ids;
pub mod logic;
pub mod models;
pub mod num;
pub mod roles;
pub mod secrets;
pub mod thread_link;
pub mod time;
mod unknown_variant;
pub mod updates;

pub use agent::PermissionMode;
pub use capture_recovery::*;
pub use models::*;
pub use time::EpochMs;
pub use unknown_variant::UnknownVariant;

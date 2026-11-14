//! Portable, local whiteboards. Geometry and edits are shared by both renderers.
mod agent;
mod assets;
pub use agent::*;
mod commands;
mod diff;
pub use diff::*;
mod mermaid;
mod proposal;
mod svg;
pub use commands::*;
pub use mermaid::*;
pub use proposal::*;
pub use svg::*;
mod authoring;
mod import;
mod input;
pub use assets::*;
pub use import::*;
pub use input::*;
mod editor;
mod fixture;
mod geometry;
mod types;
pub use authoring::*;
pub use editor::*;
pub use fixture::benchmark_scene;
pub use geometry::*;
pub use types::*;
#[cfg(test)]
mod review_tests;

pub const BOARD_FORMAT_VERSION: u32 = 1;

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum BoardError {
    #[error("{0}")]
    Invalid(String),
    #[error("This board changed. Review its current revision before applying the edit.")]
    Conflict,
    #[error("The board item no longer exists. Reload the current board.")]
    Missing,
}
pub type BoardResult<T> = Result<T, BoardError>;

#[cfg(test)]
mod authority_tests;

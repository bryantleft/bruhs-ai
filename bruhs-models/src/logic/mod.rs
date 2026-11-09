//! Pure logic the web and desktop surfaces (and bruhs-core) used to carry
//! their own copies of. Each module notes where it was moved from.
//!
//! Calendar-driven auto-record rules live with their settings in
//! [`crate::models::auto_record`]; time helpers in [`crate::time`]; the model
//! catalog and provider routing in [`crate::agent`].

pub mod attachments;
pub mod branch_pull;
pub mod commands;
pub mod compact;
pub mod composer_text;
pub mod context_chip;
pub mod diff;
pub mod dither;
pub mod editor;
pub mod find;
pub mod follow_up;
pub mod git;
pub mod handoff;
pub mod keybindings;
pub mod pages;
pub mod portable;
pub mod preview;
pub mod queue;
pub mod recall;
pub mod review;
pub mod rewind;
pub mod run_place;
pub mod sidebar;
pub mod terminal;
pub mod text;
pub mod tools;
pub mod wav;

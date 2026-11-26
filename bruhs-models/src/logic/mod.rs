//! Pure logic the web and desktop surfaces (and bruhs-core) used to carry
//! their own copies of. Each module notes where it was moved from.
//!
//! Calendar-driven auto-record rules live with their settings in
//! [`crate::models::auto_record`]; time helpers in [`crate::time`]; the model
//! catalog and provider routing in [`crate::agent`].

pub mod app_link;
pub mod attachments;
pub mod booking;
pub mod branch_pull;
pub mod cli_login;
pub mod commands;
pub mod compact;
pub mod composer_text;
pub mod context_chip;
pub mod context_meter;
pub mod diff;
pub mod dither;
pub mod editor;
pub mod find;
pub mod follow_up;
pub mod git;
pub mod handoff;
pub mod keybindings;
pub mod knowledge_library;
pub mod layout;
pub mod link_scheme;
pub mod menu;
pub mod pages;
pub mod portable;
pub mod preview;
pub mod prompt_suggestion;
pub mod queue;
pub mod recall;
pub mod record_picker;
pub mod review;
pub mod rewind;
pub mod rich_response;
pub mod run_place;
pub mod sidebar;
pub mod task_detail;
pub mod terminal;
pub mod text;
pub mod tools;
pub mod user_question;
pub mod wav;
pub mod workbench_draft;
pub mod workflow_history;
pub mod workflow_review;

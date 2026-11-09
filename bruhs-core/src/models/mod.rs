//! Core's background-task registry types. The pure data types (threads,
//! meetings, people, …) live in `bruhs-models`; import them from there.

pub mod task;

pub use task::*;

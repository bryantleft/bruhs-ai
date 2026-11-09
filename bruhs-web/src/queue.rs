//! Prompts typed while a turn is running. The queue itself lives in
//! `bruhs_models::logic::queue` so desktop uses the same rules. Web persists
//! it as `bruhs.promptQueue`.

pub use bruhs_models::logic::queue::PromptQueue;

//! Hooks for `BRUHS_DESKTOP_COMMANDS_SMOKE` and
//! `BRUHS_DESKTOP_THREADS_SMOKE` (see `src/smoke.rs`). Debug
//! builds only: a release build has no way to auto-submit prompts.

use super::*;

impl ChatView {
    /// Put `text` in the composer and press Enter, through the same path a
    /// key press takes.
    pub(crate) fn smoke_enter(
        &mut self,
        text: &str,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.write_composer(text, window, cx);
        self.send_follow_up(false, window, cx);
    }

    /// The composer's text.
    pub(crate) fn smoke_composer(&self, cx: &App) -> String {
        self.input_state.read(cx).value().to_string()
    }

    /// The latest prompt's text.
    pub(crate) fn smoke_last_prompt(&self) -> String {
        self.messages
            .iter()
            .rev()
            .find(|message| matches!(message.role, MessageRole::User))
            .map(|message| message.content.clone())
            .unwrap_or_default()
    }

    /// The last assistant reply's text.
    pub(crate) fn smoke_last_reply(&self) -> String {
        self.messages
            .iter()
            .rev()
            .find(|message| matches!(message.role, MessageRole::Assistant))
            .map(|message| message.content.clone())
            .unwrap_or_default()
    }
}

//! The header's inline thread rename, as on the web (`chat_header.rs`):
//! double-click the title or use the pen button. Enter or leaving the field
//! saves the new title, and Escape keeps the old one.

use super::*;
use bruhs_models::logic::text::resolve_rename;

/// An open rename: the title field and the subscription to its events.
pub(super) struct TitleRename {
    pub(super) input: Entity<InputState>,
    _subscription: Subscription,
}

impl ChatView {
    pub(super) fn start_title_rename(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        if self.title_rename.is_some() {
            return;
        }
        let title = self.thread_title.clone();
        let input = cx.new(|cx| InputState::new(window, cx).default_value(title.clone()));
        let subscription = cx.subscribe_in(
            &input,
            window,
            |this: &mut Self, _input, event: &InputEvent, window, cx| match event {
                InputEvent::PressEnter { .. } => {
                    this.commit_title_rename(cx);
                    this.focus_composer(window, cx);
                }
                InputEvent::Blur => this.commit_title_rename(cx),
                _ => {}
            },
        );
        // Focused with the whole title selected, so typing replaces it.
        input.update(cx, |input, cx| {
            input.focus(window, cx);
            input.set_selected_range(0..title.len(), cx);
        });
        self.title_rename = Some(TitleRename {
            input,
            _subscription: subscription,
        });
        cx.notify();
    }

    /// Save the draft unless it is blank or unchanged; saving persists the
    /// thread like any other change to it.
    pub(super) fn commit_title_rename(&mut self, cx: &mut Context<'_, Self>) {
        let Some(rename) = self.title_rename.take() else {
            return;
        };
        let draft = rename.input.read(cx).value().to_string();
        if let Some(title) = resolve_rename(&draft, &self.thread_title) {
            self.thread_title = title;
            cx.emit(ChatViewEvent::TitleRenamed);
        }
        cx.notify();
    }

    pub(super) fn cancel_title_rename(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        if self.title_rename.take().is_some() {
            self.focus_composer(window, cx);
            cx.notify();
        }
    }
}

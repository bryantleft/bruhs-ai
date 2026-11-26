use super::*;
use bruhs_models::logic::app_link::LinkedPrompt;
use bruhs_ui::banner::{banner, BannerTone};
use bruhs_ui::press::on_press_view;

pub(super) enum LinkedDraft {
    Offered(LinkedPrompt),
    Placed { removed_hidden_characters: bool },
}

impl ChatView {
    pub(crate) fn prompt_link_text(&self, cx: &App) -> SharedString {
        self.input_state.read(cx).value()
    }

    pub(crate) fn receive_linked_prompt(
        &mut self,
        prompt: LinkedPrompt,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let existing = self.prompt_link_text(cx);
        if self.linked_prompt_locked(cx)
            || self.has_link_draft_attachments()
            || !self.context_chips.is_empty()
            || (!existing.trim().is_empty() && &*existing != prompt.text.as_str())
        {
            self.linked_draft = Some(LinkedDraft::Offered(prompt));
            cx.notify();
        } else {
            self.place_linked_prompt(prompt, window, cx);
        }
    }

    pub(super) fn clear_placed_prompt_notice(&mut self, cx: &App) {
        if matches!(self.linked_draft, Some(LinkedDraft::Placed { .. }))
            && self.prompt_link_text(cx).trim().is_empty()
        {
            self.linked_draft = None;
        }
    }

    fn has_link_draft_attachments(&self) -> bool {
        !self.file_offers.is_empty() || !self.text_attachments.is_empty()
    }

    fn linked_prompt_locked(&self, cx: &App) -> bool {
        self.queue_edit.is_some()
            || self.pending_composer.is_some()
            || self.dictation.active()
            || self.input_state.read(cx).is_composing()
    }

    fn place_linked_prompt(
        &mut self,
        prompt: LinkedPrompt,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.linked_draft = Some(LinkedDraft::Placed {
            removed_hidden_characters: prompt.removed_hidden_characters,
        });
        self.install_chips(&prompt.text, Vec::new(), prompt.text.len(), window, cx);
    }

    fn replace_draft_from_link(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        if self.linked_prompt_locked(cx) {
            return;
        }
        match self.linked_draft.take() {
            Some(LinkedDraft::Offered(prompt)) => self.place_linked_prompt(prompt, window, cx),
            other => self.linked_draft = other,
        }
    }

    pub(super) fn render_linked_prompt_banner(&self, cx: &Context<'_, Self>) -> Option<AnyElement> {
        let draft = self.linked_draft.as_ref()?;
        let offered = matches!(draft, LinkedDraft::Offered(_));
        let hidden = match draft {
            LinkedDraft::Offered(prompt) => prompt.removed_hidden_characters,
            LinkedDraft::Placed {
                removed_hidden_characters,
            } => *removed_hidden_characters,
        };
        let locked = offered && self.linked_prompt_locked(cx);
        let detail = match (offered, locked, self.has_link_draft_attachments()) {
            (true, true, _) => "Finish your current edit to use the linked prompt.",
            (true, false, true) => "Replace the draft text and keep its attachments?",
            (true, false, false) => "Replace the existing draft with the linked prompt?",
            _ => "Review the draft before sending.",
        };
        let detail = if hidden {
            format!("{detail} Hidden characters were removed.")
        } else {
            detail.to_owned()
        };
        let dismiss_label = if offered { "Keep draft" } else { "Dismiss" };
        let dismiss = on_press_view(
            web_button(
                "linked-prompt-dismiss",
                WebButtonVariant::Ghost,
                WebButtonSize::Small,
                ButtonState::Enabled,
            )
            .debug_selector(|| "desktop-linked-prompt-dismiss".into())
            .aria_label(dismiss_label)
            .child(dismiss_label),
            cx,
            |chat, window, cx| {
                chat.linked_draft = None;
                chat.focus_composer(window, cx);
                cx.notify();
            },
        );
        let mut notice = banner(
            "linked-prompt-banner",
            if hidden {
                BannerTone::Warning
            } else {
                BannerTone::Info
            },
            if offered {
                "Linked prompt ready"
            } else {
                "Prompt added from a link"
            },
        )
        .detail(detail)
        .debug_selector("desktop-linked-prompt-banner")
        .action(dismiss);
        if offered && !locked {
            notice = notice.action(on_press_view(
                web_button(
                    "linked-prompt-replace",
                    WebButtonVariant::Outline,
                    WebButtonSize::Small,
                    ButtonState::Enabled,
                )
                .debug_selector(|| "desktop-linked-prompt-replace".into())
                .aria_label("Replace draft")
                .child("Replace draft"),
                cx,
                Self::replace_draft_from_link,
            ));
        }
        Some(div().w_full().mb_2().child(notice).into_any_element())
    }
}

//! UI acknowledgment for the ordered whole-workspace writer.

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) enum SaveStatus {
    #[default]
    Saved,
    Saving,
    Failed(String),
    Conflict,
    /// A corrupt/unreadable original must not be replaced by a default or backup.
    RecoveryRequired(String),
}

impl SaveStatus {
    pub(super) fn is_saved(&self) -> bool {
        matches!(self, Self::Saved)
    }

    pub(super) fn begin(&mut self) -> bool {
        if matches!(self, Self::RecoveryRequired(_) | Self::Conflict) {
            return false;
        }
        *self = Self::Saving;
        true
    }

    pub(super) fn finish(&mut self, result: Result<(), SaveError>) {
        *self = match result {
            Ok(()) => Self::Saved,
            Err(SaveError::Conflict) => Self::Conflict,
            Err(error) => Self::Failed(error.to_string()),
        };
    }

    /// The warning a save needs the user to act on. Routine saves stay
    /// silent, so `Saved` and `Saving` have none.
    pub(super) fn alert(&self) -> Option<&'static str> {
        match self {
            Self::Saved | Self::Saving => None,
            Self::Failed(_) => Some("Local save failed. Keep this window open; copy a recovery snapshot or retry. Workflows cannot advance yet."),
            Self::Conflict => Some("Another instance changed this workspace. Your draft is kept here. Copy recovery before reloading; retry cannot overwrite the newer saved version."),
            Self::RecoveryRequired(_) => Some("Saved data needs recovery. The original file is preserved and saving is paused. Copy this window's recovery snapshot before repairing or reloading."),
        }
    }
}

use super::*;

impl Workspace {
    /// The strip appears only when a save failed; routine saves show nothing.
    pub(super) fn render_save_status(&self, cx: &Context<'_, Self>) -> Option<AnyElement> {
        let message = self.save_status.alert()?;
        Some(
            div()
                .id("workspace-save-status")
                .debug_selector(|| "desktop-save-status".into())
                .role(Role::Alert)
                .aria_label(message)
                .w_full()
                .flex_shrink_0()
                .flex()
                .flex_wrap()
                .items_center()
                .gap_2()
                .px_3()
                .py_2()
                .bg(theme::longan_900())
                .text_size(px(12.0))
                .text_color(theme::durian_300())
                .child(div().flex_1().min_w_0().child(message))
                .when(matches!(self.save_status, SaveStatus::Failed(_)), |el| {
                    el.child(web_button("retry-workspace-save", WebButtonVariant::Outline, WebButtonSize::Small, ButtonState::Enabled)
                        .debug_selector(|| "desktop-retry-save".into())
                        .aria_label("Retry local save")
                        .child("Retry save")
                        .on_click(cx.listener(|this, _, _, cx| this.persist_and_notify(cx))))
                })
                .child(web_button("copy-workspace-recovery", WebButtonVariant::Ghost, WebButtonSize::Small, ButtonState::Enabled)
                    .debug_selector(|| "desktop-copy-recovery".into())
                    .aria_label("Copy private recovery snapshot")
                    .child("Copy recovery")
                    .on_click(cx.listener(|this, _, _, cx| {
                        // Explicit local clipboard action; recovery is never sent to the agent.
                        match serde_json::to_string_pretty(&this.snapshot(cx)) {
                            Ok(json) => cx.write_to_clipboard(ClipboardItem::new_string(json)),
                            Err(_) => this.persistence_error = Some("The recovery snapshot could not be encoded. Keep this window open.".into()),
                        }
                        cx.notify();
                    })))
                .into_any_element(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::{theme, DesktopState, SaveStatus, Workspace};
    use gpui::AppContext;

    #[test]
    fn workflow_may_advance_only_after_successful_save_acknowledgment() {
        let mut state = SaveStatus::Saved;
        assert!(state.begin());
        assert!(!state.is_saved());
        state.finish(Err("disk full".into()));
        assert!(!state.is_saved());
        assert!(state.begin());
        assert!(!state.is_saved());
        state.finish(Ok(()));
        assert!(state.is_saved());
    }

    #[test]
    fn unreadable_original_cannot_be_overwritten_by_a_new_save() {
        let mut state = SaveStatus::RecoveryRequired("bad file".into());
        assert!(!state.begin());
        assert!(matches!(state, SaveStatus::RecoveryRequired(_)));
    }

    #[test]
    fn conflict_disables_blind_retry_without_discarding_the_draft() {
        let mut state = SaveStatus::Saving;
        state.finish(Err(super::SaveError::Conflict));
        assert_eq!(state, SaveStatus::Conflict);
        assert!(!state.begin());
        assert!(!state.is_saved());
    }

    #[test]
    fn global_message_does_not_expose_private_error_details() {
        let state = SaveStatus::Failed("private path and payload".into());
        assert!(!state.alert().unwrap().contains("private path"));
    }

    #[gpui::test]
    fn only_a_failed_save_shows_the_status_strip(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            gpui_component::init(cx);
            theme::apply_bruhs_theme(cx);
            cx.set_global(DesktopState::new().unwrap());
        });
        let (root, cx) = cx.add_window_view(|window, cx| {
            gpui_component::Root::new(cx.new(|cx| Workspace::new(window, cx)), window, cx)
        });
        for (status, shown) in [
            (SaveStatus::Saving, false),
            (SaveStatus::Saved, false),
            (SaveStatus::Failed("disk full".into()), true),
            (SaveStatus::Conflict, true),
            (SaveStatus::RecoveryRequired("bad file".into()), true),
        ] {
            let label = format!("{status:?}");
            root.update(cx, |root, cx| {
                root.view()
                    .clone()
                    .downcast::<Workspace>()
                    .unwrap()
                    .update(cx, |workspace, cx| {
                        workspace.save_status = status;
                        cx.notify();
                    });
            });
            cx.run_until_parked();
            assert_eq!(
                cx.debug_bounds("desktop-save-status").is_some(),
                shown,
                "{label}"
            );
        }
    }
}

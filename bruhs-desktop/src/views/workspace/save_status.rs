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
            // The web's words for the same failures (`storage::metadata::SaveError`),
            // with this app's window and local storage.
            Self::Failed(_) => Some("Local storage could not save this change. Keep this window open and retry."),
            Self::Conflict => Some("Another window saved newer data. Your unsaved changes are kept here; copy them before reloading to review the latest version."),
            Self::RecoveryRequired(_) => Some("Saved data could not be decoded. The original copy was preserved; no replacement was saved. Copy this window's recovery data before repairing or reloading."),
        }
    }
}

use super::*;
use bruhs_ui::zoom::px;

impl Workspace {
    /// Web `MetadataStatus`: nothing while saves succeed; after a failure,
    /// "Not saved", the reason, Retry save and the private recovery data.
    /// The desktop saves the whole workspace in one file, so its strip
    /// stands for every page, under the navigation.
    pub(super) fn render_save_status(&self, cx: &Context<'_, Self>) -> Option<AnyElement> {
        let message = self.save_status.alert()?;
        Some(
            div()
                .id("workspace-save-status")
                .debug_selector(|| "desktop-save-status".into())
                .w_full()
                .flex_shrink_0()
                .border_b_1()
                .border_color(theme::longan_800().opacity(0.4))
                // Web `px-4 sm:px-6`; the window is never narrower than `sm`.
                .px(px(24.0))
                .py_2()
                .child(super::automations_ui::label_sm("Not saved", theme::rambutan_400()))
                .child(
                    super::automations_ui::label_sm(message, theme::lychee_300())
                        .id("workspace-save-reason")
                        .mt_1()
                        .role(Role::Alert)
                        .aria_label(message),
                )
                .child(
                    div()
                        .mt_2()
                        .flex()
                        .flex_wrap()
                        .items_center()
                        .gap_3()
                        .when(matches!(self.save_status, SaveStatus::Failed(_)), |row| {
                            row.child(
                                web_button(
                                    "retry-workspace-save",
                                    WebButtonVariant::Ghost,
                                    WebButtonSize::Small,
                                    ButtonState::Enabled,
                                )
                                .debug_selector(|| "desktop-retry-save".into())
                                .aria_label("Retry save")
                                // Web `bg-longan-800 px-3 py-2 text-rambutan-200`.
                                .bg(theme::longan_800())
                                .px_3()
                                .py_2()
                                .text_color(theme::rambutan_200())
                                .child("Retry save")
                                .on_click(cx.listener(|this, _, _, cx| this.persist_and_notify(cx))),
                            )
                        })
                        .child(
                            web_button(
                                "copy-workspace-recovery",
                                WebButtonVariant::Ghost,
                                WebButtonSize::Small,
                                ButtonState::Enabled,
                            )
                            .debug_selector(|| "desktop-copy-recovery".into())
                            .aria_label("Copy recovery data (private)")
                            .px_0()
                            .text_color(theme::rambutan_400())
                            // The web shows the data to copy; the desktop copies it.
                            .child("Copy recovery data (private)")
                            .on_click(cx.listener(|this, _, _, cx| {
                                // Explicit local clipboard action; recovery is never sent to the agent.
                                match serde_json::to_string_pretty(&this.recovery_snapshot(cx)) {
                                    Ok(json) => cx.write_to_clipboard(ClipboardItem::new_string(json)),
                                    Err(_) => this.persistence_error = Some("The recovery snapshot could not be encoded. Keep this window open.".into()),
                                }
                                cx.notify();
                            })),
                        ),
                )
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
            bruhs_ui::init(cx);
            theme::apply_bruhs_theme(cx);
            cx.set_global(DesktopState::new().unwrap());
        });
        let (root, cx) = cx.add_window_view(|window, cx| {
            bruhs_ui::Root::new(cx.new(|cx| Workspace::new(window, cx)), window, cx)
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

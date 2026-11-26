use super::*;
use bruhs_core::workspace_trust;
use bruhs_models::workspace_trust::{SETTINGS_DESCRIPTION, SETTINGS_TITLE};

#[derive(Default)]
pub(super) struct TrustedFolders {
    paths: Vec<String>,
    busy: bool,
    notice: &'static str,
}

impl Workspace {
    pub(super) fn update_trusted_folders(
        &mut self,
        revoke: Option<String>,
        cx: &mut Context<'_, Self>,
    ) {
        let state = &mut self.trusted_folders;
        if state.busy {
            return;
        }
        let revoking = revoke.is_some();
        state.busy = true;
        state.notice = if revoking {
            "Revoking trust…"
        } else {
            "Loading trusted folders…"
        };
        let task = cx.background_spawn(async move {
            match revoke {
                Some(path) => workspace_trust::revoke_trusted(&path),
                None => workspace_trust::list_trusted(),
            }
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                let state = &mut this.trusted_folders;
                state.busy = false;
                match result {
                    Ok(paths) => {
                        state.notice = match (revoking, paths.is_empty()) {
                            (true, true) => "Trust revoked. No trusted folders.",
                            (true, false) => "Trust revoked.",
                            (false, true) => "No trusted folders.",
                            (false, false) => "",
                        };
                        state.paths = paths;
                        if revoking {
                            for thread in &this.threads {
                                thread
                                    .chat
                                    .update(cx, |chat, cx| chat.invalidate_workspace_trust(cx));
                            }
                        }
                    }
                    Err(error) => {
                        tracing::warn!(%error, "Workspace trust settings update failed");
                        state.notice = if revoking {
                            "Could not revoke trust. Try again."
                        } else {
                            "Could not load trusted folders. Refresh to try again."
                        };
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub(super) fn render_trusted_folders(&self, cx: &Context<'_, Self>) -> AnyElement {
        let state = &self.trusted_folders;
        let refresh = web_button(
            "settings-refresh-trusted-folders",
            WebButtonVariant::Outline,
            WebButtonSize::Small,
            ButtonState::disabled_if(state.busy),
        )
        .aria_label("Refresh trusted folders")
        .child("Refresh")
        .when(!state.busy, |button| {
            on_press_view(button, cx, |this, _, cx| {
                this.update_trusted_folders(None, cx)
            })
        });
        let mut rows = vec![self.settings_row_opts(
            "desktop-settings-row-trusted-folders",
            SETTINGS_TITLE,
            SETTINGS_DESCRIPTION,
            (!state.notice.is_empty()).then(|| state.notice.into()),
            false,
            refresh.into_any_element(),
        )];
        rows.extend(state.paths.iter().map(|path| {
            let entry = path.clone();
            let row_id = SharedString::from(format!("desktop-settings-trusted-folder-{path}"));
            let revoke = web_button(
                SharedString::from(format!("settings-revoke-trust-{path}")),
                WebButtonVariant::Outline,
                WebButtonSize::Small,
                ButtonState::disabled_if(state.busy),
            )
            .aria_label(format!("Revoke trust for {path}"))
            .child("Revoke trust")
            .when(!state.busy, |button| {
                on_press_view(button, cx, move |this, _, cx| {
                    this.update_trusted_folders(Some(entry.clone()), cx);
                })
            });
            self.settings_row(row_id, path.clone(), "", revoke.into_any_element())
        }));
        self.settings_card("", "", rows)
    }
}

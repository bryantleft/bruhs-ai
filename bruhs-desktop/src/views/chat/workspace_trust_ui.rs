//! Workspace Trust banner for Desktop chat: passively inspects the active
//! project folder off the UI thread and lets the user trust it in one click.

use super::*;
use bruhs_models::workspace_trust::{banner_summary, WorkspaceTrustStatus};
use bruhs_ui::banner::{banner, BannerTone};
use bruhs_ui::press::on_press_view;

impl ChatView {
    pub(in crate::views) fn invalidate_workspace_trust(&mut self, cx: &mut Context<'_, Self>) {
        if self.project_scoped {
            self.workspace_trust_stale = true;
            cx.notify();
        }
    }

    pub(super) fn clear_workspace_trust(&mut self) {
        self.workspace_trust_stale = false;
        self.workspace_trust = None;
        self.workspace_trust_saving = false;
        self.workspace_trust_error = None;
        self.workspace_trust_task = None;
    }

    pub(super) fn refresh_workspace_trust(&mut self, cx: &mut Context<'_, Self>) {
        if !self.project_scoped || self.project_path.is_empty() {
            self.clear_workspace_trust();
            return;
        }
        self.workspace_trust_stale = false;
        self.workspace_trust_error = None;
        let path = PathBuf::from(&self.project_path);
        let scope = self.project_path.clone();
        let read = cx.background_spawn(async move { bruhs_core::workspace_trust::inspect(&path) });
        self.workspace_trust_task = Some(cx.spawn(async move |this, cx| {
            let status = read.await;
            let _closed = this.update(cx, |this, cx| {
                if this.project_scoped && this.project_path == scope {
                    this.workspace_trust = Some(status);
                    cx.notify();
                }
            });
        }));
    }

    pub(super) fn trust_active_workspace(&mut self, cx: &mut Context<'_, Self>) {
        if !self.project_scoped || self.project_path.is_empty() || self.workspace_trust_saving {
            return;
        }
        self.workspace_trust_saving = true;
        self.workspace_trust_error = None;
        let path = PathBuf::from(&self.project_path);
        let scope = self.project_path.clone();
        let save = cx
            .background_spawn(async move { bruhs_core::workspace_trust::set_trusted(&path, true) });
        self.workspace_trust_task = Some(cx.spawn(async move |this, cx| {
            let result = save.await;
            let _closed = this.update(cx, |this, cx| {
                this.workspace_trust_saving = false;
                if this.project_scoped && this.project_path == scope {
                    match result {
                        Ok(status) => {
                            this.workspace_trust = Some(status);
                            this.workspace_trust_error = None;
                        }
                        Err(error) => {
                            this.workspace_trust_error =
                                Some(format!("Could not trust folder: {error}"));
                        }
                    }
                    cx.notify();
                }
            });
        }));
        cx.notify();
    }

    pub(super) fn render_workspace_trust_banner(
        &self,
        cx: &Context<'_, Self>,
    ) -> Option<AnyElement> {
        let status: &WorkspaceTrustStatus = self.workspace_trust.as_ref()?;
        if !status.should_warn() {
            return None;
        }
        let saving = self.workspace_trust_saving;
        let label = if saving {
            "Trusting…"
        } else {
            "Trust folder"
        };
        let trust_btn = web_button(
            "desktop-workspace-trust-action",
            WebButtonVariant::Primary,
            WebButtonSize::Small,
            ButtonState::disabled_if(saving),
        )
        .aria_label(label)
        .child(label)
        .when(!saving, |el| {
            el.tab_index(0)
                .role(Role::Button)
                .focus_visible(|el| el.border_color(theme::persimmon_400()))
                .map(|el| {
                    on_press_view(el, cx, |this, _window, cx| {
                        this.trust_active_workspace(cx);
                    })
                })
        });
        Some(
            div()
                .w_full()
                .mb_2()
                .child(
                    banner(
                        "desktop-workspace-trust-banner",
                        BannerTone::Warning,
                        banner_summary(&status.risks),
                    )
                    .debug_selector("desktop-workspace-trust-banner")
                    .detail_opt(self.workspace_trust_error.clone())
                    .action(trust_btn),
                )
                .into_any_element(),
        )
    }
}

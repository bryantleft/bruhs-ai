//! A return point for setting up this device from Settings → About.
use super::*;
use bruhs_ui::press::on_press_view;

impl Workspace {
    pub(super) fn render_welcome(&self, window: &Window, cx: &Context<'_, Self>) -> AnyElement {
        let providers = self.updates.read(cx).report.as_ref().map(|report| {
            report
                .providers
                .iter()
                .filter_map(|p| {
                    p.installed
                        .as_ref()
                        .map(|v| format!("{} {v}", p.provider.label()))
                })
                .collect::<Vec<_>>()
                .join(" · ")
        });
        let description = match providers.as_deref() {
            Some("") => "Choose a coding agent for this machine.",
            None => "Check installed coding agents and available updates.",
            Some(installed) => installed,
        };
        let action = |id, label| {
            web_button(
                id,
                WebButtonVariant::Outline,
                WebButtonSize::Small,
                ButtonState::Enabled,
            )
            .aria_label(label)
            .child(label)
        };
        let content = div()
            .p_4()
            .flex()
            .flex_col()
            .gap_3()
            .child(crate::views::braille::braille_bruh(64.))
            .child(
                self.settings_row(
                    "welcome-providers",
                    "Coding agents",
                    description.to_owned(),
                    on_press_view(
                        action("welcome-open-agents", "View agents"),
                        cx,
                        |this, window, cx| {
                            this.open_settings(&OpenSettings, window, cx);
                            this.set_settings_section(SettingsSection::Updates, cx);
                        },
                    )
                    .into_any_element(),
                ),
            )
            .child(
                self.settings_row(
                    "welcome-projects",
                    "Projects",
                    "Open a local folder or clone a Git repository.",
                    on_press_view(
                        action("welcome-open-project", "Add project"),
                        cx,
                        |this, window, cx| {
                            this.open_project_picker(&OpenProjectPicker, window, cx);
                            cx.notify();
                        },
                    )
                    .into_any_element(),
                ),
            )
            .child(
                self.settings_row(
                    "welcome-phone",
                    "Mobile app",
                    "Pair your phone with this host.",
                    on_press_view(
                        action("welcome-open-phone", "Set up phone"),
                        cx,
                        |this, window, cx| {
                            this.open_settings(&OpenSettings, window, cx);
                            this.set_settings_section(SettingsSection::Access, cx);
                        },
                    )
                    .into_any_element(),
                ),
            );
        self.render_modal_shell("Welcome to bruhs", content, None, window, cx)
    }
}

fn display_command_path(path: &std::path::Path) -> String {
    if let Some(home) = directories::BaseDirs::new() {
        if let Ok(relative) = path.strip_prefix(home.home_dir()) {
            return format!("~/{}", relative.display());
        }
    }
    path.display().to_string()
}
impl Workspace {
    pub(super) fn refresh_launcher(&mut self, install: bool, cx: &mut Context<'_, Self>) {
        if self.command_launcher_busy {
            return;
        }
        self.command_launcher_busy = true;
        let task = cx.background_spawn(async move {
            crate::launcher::configure(install).map_err(|e| e.to_string())
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                this.command_launcher_busy = false;
                this.command_launcher = Some(result);
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    pub(super) fn launcher_row(&self, cx: &Context<'_, Self>) -> AnyElement {
        use crate::launcher::Status;
        let (message, label, action) = match self.command_launcher.as_ref() {
            Some(Ok(Status::Available(path))) => (
                format!(
                    "Run bruhs from your terminal. Install to {}.",
                    display_command_path(path)
                ),
                "Install",
                Some(true),
            ),
            Some(Ok(Status::Installed(path, true))) => (
                format!(
                    "Run bruhs from your terminal using {}.",
                    display_command_path(path)
                ),
                "Installed",
                None,
            ),
            Some(Ok(Status::Installed(path, false))) => (
                format!(
                    "Add {} to your shell PATH to run bruhs.",
                    display_command_path(path.parent().unwrap_or(path))
                ),
                "Refresh",
                Some(false),
            ),
            Some(Ok(Status::Occupied(path))) => (
                format!(
                    "Another command already exists at {}.",
                    display_command_path(path)
                ),
                "Refresh",
                Some(false),
            ),
            Some(Err(e)) => (e.clone(), "Retry", Some(false)),
            None => (
                "Checking the terminal command…".to_owned(),
                "Checking",
                None,
            ),
        };
        let button = web_button(
            "about-cli-install",
            WebButtonVariant::Outline,
            WebButtonSize::Small,
            ButtonState::disabled_if(self.command_launcher_busy || action.is_none()),
        )
        .aria_label(format!("{label} bruhs command"))
        .child(label);
        let control = if let Some(install) = action {
            on_press_view(button, cx, move |this, _, cx| {
                this.refresh_launcher(install, cx)
            })
            .into_any_element()
        } else {
            button.into_any_element()
        };
        self.settings_row("settings-about-command", "bruhs command", message, control)
    }
}

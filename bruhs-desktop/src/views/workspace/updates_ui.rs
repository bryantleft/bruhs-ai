//! Update checks on the desktop (web `updates.rs` + `components/updates.rs`):
//! the sidebar update pills, the "Update N providers?" confirmation and
//! Settings → Updates. Checks and updates run in process through `bruhs_core::updates`.

use super::*;
use crate::app_state::spawn_tokio;
use bruhs_core::updates::{BuildInfo, UpdateChecker};
use bruhs_models::agent::AgentProvider;
use bruhs_models::time::{format_relative_ms, EpochMs, RelativeTimeStyle};
use bruhs_models::updates::{
    confirm_title, provider_cli_label, running_label, sidebar_label, AppUpdateState, ProviderRun,
    ProviderUpdate, ProviderUpdateState, UpdatesReport, POLL_INTERVAL_MS, RUN_POLL_INTERVAL_MS,
    SUCCESS_VISIBLE_MS,
};
use std::collections::HashSet;

const UPDATE_CONFIRM_LABEL: &str = "Update providers";
const RESTART_CONFIRM_LABEL: &str = "Restart to update";

/// A packaged build's own update (T3: download in the background, then
/// "Restart to update").
#[derive(Clone, Debug, PartialEq)]
pub(super) enum AppInstall {
    Idle,
    Downloading(String),
    /// Verified and unpacked beside the app; restart swaps it in.
    Ready {
        version: String,
        staged: std::path::PathBuf,
    },
    /// Retried after [`APP_RETRY_AFTER`] (or on the next launch).
    Failed {
        version: String,
        error: String,
        at: std::time::Instant,
    },
}

/// How long a failed app update waits before the next check tries again.
const APP_RETRY_AFTER: std::time::Duration = std::time::Duration::from_secs(60 * 60);

/// The sidebar's provider pill (web `SidebarRuns`).
#[derive(Clone, Debug, PartialEq)]
pub(super) enum SidebarRuns {
    None,
    Active(Vec<ProviderRun>),
    Finished(ProviderRun),
}

pub(super) struct DesktopUpdates {
    checker: UpdateChecker,
    /// Online checks (registries, the remote) run only while this is on.
    pub enabled: bool,
    pub report: Option<UpdatesReport>,
    pub checking: bool,
    /// Why the last check failed.
    pub error: Option<String>,
    /// Why the last update did not start; kept until the next start or
    /// Check now (web `start_error`).
    pub start_error: Option<String>,
    /// Runs dismissed from the sidebar (kept in memory, like T3).
    pub dismissed: HashSet<u64>,
    /// This app's own update, when it is a packaged release.
    pub install: AppInstall,
    /// Runs that finished before this window opened stay out of the sidebar.
    loaded_at: EpochMs,
    _poll: Option<Task<()>>,
}

impl DesktopUpdates {
    pub(super) fn new(enabled: bool, cx: &mut Context<'_, Self>) -> Self {
        let mut this = Self {
            checker: UpdateChecker::new(
                BuildInfo::new(
                    env!("BRUHS_BUILD_COMMIT"),
                    env!("BRUHS_REPO_ROOT"),
                    "cargo run --release -p bruhs-desktop",
                )
                .with_release(env!("BRUHS_RELEASE_VERSION")),
            ),
            enabled,
            report: None,
            checking: false,
            error: None,
            start_error: None,
            dismissed: HashSet::new(),
            install: AppInstall::Idle,
            loaded_at: EpochMs::now(),
            _poll: None,
        };
        // Tests drive checks by hand; a real check spawns CLIs and dials out.
        if cfg!(not(test)) {
            this.start_polling(cx);
        }
        this
    }

    /// Check on open and every five minutes; every second while a run is
    /// active or a success is on screen (it leaves after three seconds).
    fn start_polling(&mut self, cx: &mut Context<'_, Self>) {
        let tick = std::time::Duration::from_millis(u64::from(RUN_POLL_INTERVAL_MS));
        let every = POLL_INTERVAL_MS / RUN_POLL_INTERVAL_MS;
        self._poll = Some(cx.spawn(async move |this, cx| {
            let mut ticks: u32 = 0;
            loop {
                let Ok(()) = this.update(cx, |this, cx| {
                    let running = this.report.as_ref().is_some_and(UpdatesReport::any_running);
                    if ticks.is_multiple_of(every) || running {
                        this.check(false, cx);
                    } else if matches!(
                        this.sidebar_runs(EpochMs::now()),
                        SidebarRuns::Finished(run) if !run.needs_attention()
                    ) {
                        // A success leaves after three seconds; a failure
                        // stays until dismissed and needs no redraws.
                        cx.notify();
                    }
                }) else {
                    return;
                };
                ticks = ticks.wrapping_add(1);
                cx.background_executor().timer(tick).await;
            }
        }));
    }

    pub(super) fn set_enabled(&mut self, on: bool, cx: &mut Context<'_, Self>) {
        self.enabled = on;
        self.check(false, cx);
        cx.notify();
    }

    pub(super) fn check(&mut self, force: bool, cx: &mut Context<'_, Self>) {
        if self.checking || cx.try_global::<crate::app_state::DesktopState>().is_none() {
            return;
        }
        self.checking = true;
        cx.notify();
        let checker = self.checker.clone();
        let online = self.enabled;
        let task = spawn_tokio(cx, async move { checker.report(force, online).await });
        cx.spawn(async move |this, cx| {
            let report = task.await;
            let _ = this.update(cx, |this, cx| {
                this.checking = false;
                this.error = None;
                if report.online == this.enabled {
                    this.report = Some(report);
                    this.stage_app_update(cx);
                } else {
                    // The switch flipped while this ran: ask again for the
                    // side it is on now.
                    this.check(false, cx);
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Start updating `provider`. The installer runs in its own process
    /// group, so it finishes even if bruhs quits.
    pub(super) fn update(&mut self, provider: AgentProvider, cx: &mut Context<'_, Self>) {
        if cx.try_global::<crate::app_state::DesktopState>().is_none() {
            return;
        }
        let checker = self.checker.clone();
        let task = spawn_tokio(cx, async move { checker.start_update(provider) });
        cx.spawn(async move |this, cx| {
            let started = task.await;
            let _ = this.update(cx, |this, cx| {
                match started {
                    Ok(run) => {
                        this.start_error = None;
                        if let Some(report) = &mut this.report {
                            report.runs.retain(|r| r.id != run.id);
                            report.runs.push(run);
                        }
                    }
                    Err(error) => {
                        this.start_error = Some(error.to_string());
                        this.check(false, cx);
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Download and verify a newer release in the background, once per
    /// version (a failure is not retried until another version appears).
    fn stage_app_update(&mut self, cx: &mut Context<'_, Self>) {
        let Some(release) = self
            .report
            .as_ref()
            .and_then(|r| r.app.installable_release())
            .cloned()
        else {
            return;
        };
        let Some(version) = release.latest_version.clone() else {
            return;
        };
        let current = match &self.install {
            AppInstall::Idle => None,
            AppInstall::Failed { at, .. } if at.elapsed() >= APP_RETRY_AFTER => None,
            AppInstall::Downloading(v)
            | AppInstall::Ready { version: v, .. }
            | AppInstall::Failed { version: v, .. } => Some(v),
        };
        if current == Some(&version) {
            return;
        }
        let Some(app) = bruhs_core::updates::release::current_app_bundle() else {
            return;
        };
        self.install = AppInstall::Downloading(version.clone());
        cx.notify();
        let task = spawn_tokio(cx, async move {
            bruhs_core::updates::release::stage_update(&release, &app).await
        });
        cx.spawn(async move |this, cx| {
            let staged = task.await;
            let _ = this.update(cx, |this, cx| {
                this.install = match staged {
                    Ok(staged) => AppInstall::Ready { version, staged },
                    Err(error) => {
                        tracing::warn!("app update v{version}: {error}");
                        AppInstall::Failed {
                            version,
                            error,
                            at: std::time::Instant::now(),
                        }
                    }
                };
                cx.notify();
            });
        })
        .detach();
    }

    /// Start every provider that can update here; the checker queues the
    /// ones that share an installer.
    pub(super) fn update_all(&mut self, cx: &mut Context<'_, Self>) {
        let providers = self
            .report
            .as_ref()
            .map(UpdatesReport::updatable_providers)
            .unwrap_or_default();
        for provider in providers {
            self.update(provider, cx);
        }
    }

    pub(super) fn dismiss(&mut self, id: u64, cx: &mut Context<'_, Self>) {
        self.dismissed.insert(id);
        cx.notify();
    }

    /// Web `Updates::sidebar_runs`.
    pub(super) fn sidebar_runs(&self, now: EpochMs) -> SidebarRuns {
        let Some(report) = &self.report else {
            return SidebarRuns::None;
        };
        let active: Vec<ProviderRun> = report
            .runs
            .iter()
            .filter(|r| r.is_active())
            .cloned()
            .collect();
        if !active.is_empty() {
            return SidebarRuns::Active(active);
        }
        report
            .runs
            .iter()
            .filter(|r| !self.dismissed.contains(&r.id))
            .filter(|r| r.finished_at.is_some_and(|at| at >= self.loaded_at))
            .filter(|r| {
                r.needs_attention()
                    || r.finished_at
                        .is_some_and(|at| now.millis_since(at) < SUCCESS_VISIBLE_MS)
            })
            .max_by_key(|r| r.id)
            .cloned()
            .map_or(SidebarRuns::None, SidebarRuns::Finished)
    }

    #[cfg(test)]
    pub(super) fn set_report_for_test(
        &mut self,
        report: UpdatesReport,
        cx: &mut Context<'_, Self>,
    ) {
        self.report = Some(report);
        cx.notify();
    }
}

type Tone = (gpui::Hsla, gpui::Hsla, gpui::Hsla);

fn pill(id: &'static str, (bg, border, text): Tone) -> Stateful<Div> {
    div()
        .id(id)
        .debug_selector(move || id.to_string())
        .h(px(32.0))
        .w_full()
        .px(px(8.0))
        .rounded(px(8.0))
        .border_1()
        .border_color(border)
        .bg(bg)
        .text_color(text)
        .text_size(px(14.0))
        .flex()
        .items_center()
        .gap(px(8.0))
}

fn tone_action() -> Tone {
    (
        theme::persimmon_500().opacity(0.10),
        theme::persimmon_500().opacity(0.25),
        theme::persimmon_200(),
    )
}

fn tone_info() -> Tone {
    (
        theme::longan_800().opacity(0.6),
        gpui::white().opacity(0.10),
        theme::rambutan_200(),
    )
}

fn tone_failed() -> Tone {
    (
        theme::lychee_500().opacity(0.10),
        theme::lychee_500().opacity(0.25),
        theme::lychee_200(),
    )
}

fn icon_svg(path: &'static str, size: f32, color: gpui::Hsla) -> Svg {
    svg()
        .path(path)
        .size(px(size))
        .flex_shrink_0()
        .text_color(color)
}

fn meta(text: impl Into<SharedString>) -> Div {
    div()
        .text_size(px(SETTINGS_META_TEXT_SIZE_PX))
        .text_color(theme::rambutan_400())
        .child(text.into())
}

impl Workspace {
    pub(super) fn toggle_update_checks(&mut self, cx: &mut Context<'_, Self>) {
        let on = !self.updates.read(cx).enabled;
        self.updates
            .update(cx, |updates, cx| updates.set_enabled(on, cx));
        self.persist_and_notify(cx);
    }

    fn open_updates_settings(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        self.open_settings(&OpenSettings, window, cx);
        self.set_settings_section(SettingsSection::Updates, cx);
    }

    /// Web `SidebarUpdateButton`: the provider pill while runs are active or
    /// just finished, else Update while something is behind.
    pub(super) fn render_sidebar_update_button(
        &self,
        cx: &Context<'_, Self>,
    ) -> Option<AnyElement> {
        let updates = self.updates.read(cx);
        let element = match updates.sidebar_runs(EpochMs::now()) {
            SidebarRuns::Active(active) => {
                let tooltip: SharedString = active
                    .iter()
                    .map(|r| format!("{}: {}", provider_cli_label(r.provider), r.phase_line()))
                    .collect::<Vec<_>>()
                    .join("\n")
                    .into();
                pill("desktop-sidebar-update", tone_info())
                    .role(Role::Button)
                    .aria_label(running_label(&active).unwrap_or_default())
                    .tab_index(0)
                    .cursor_pointer()
                    .focus_visible(|el| el.border_2().border_color(theme::persimmon_400()))
                    .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                        if is_web_button_activation_key(event.keystroke.key.as_str()) {
                            window.prevent_default();
                            cx.stop_propagation();
                            this.open_updates_settings(window, cx);
                        }
                    }))
                    .tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx))
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.open_updates_settings(window, cx);
                    }))
                    .child(icon_svg(
                        assets::REFRESH_ICON_PATH,
                        14.0,
                        theme::rambutan_300(),
                    ))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .child(running_label(&active).unwrap_or_default()),
                    )
                    .into_any_element()
            }
            SidebarRuns::Finished(run) => {
                let failed = run.needs_attention();
                let id = run.id;
                let tooltip: SharedString = if failed {
                    run.message.clone().unwrap_or_default().into()
                } else {
                    "New sessions will use the updated provider.".into()
                };
                pill(
                    "desktop-sidebar-update",
                    if failed { tone_failed() } else { tone_info() },
                )
                .pr(px(4.0))
                .tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx))
                .child(
                    div()
                        .id("desktop-sidebar-update-open")
                        .role(Role::Button)
                        .aria_label(run.pill_label())
                        .tab_index(0)
                        .focus_visible(|el| el.border_2().border_color(theme::persimmon_400()))
                        .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                            if is_web_button_activation_key(event.keystroke.key.as_str()) {
                                window.prevent_default();
                                cx.stop_propagation();
                                this.open_updates_settings(window, cx);
                            }
                        }))
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .items_center()
                        .gap(px(8.0))
                        .cursor_pointer()
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.open_updates_settings(window, cx);
                        }))
                        .child(if failed {
                            icon_svg(assets::ALERT_TRIANGLE_ICON_PATH, 14.0, theme::lychee_200())
                        } else {
                            icon_svg(assets::CHECK_ICON_PATH, 14.0, theme::pandan_400())
                        })
                        .child(div().flex_1().min_w_0().truncate().child(run.pill_label())),
                )
                .when(failed, |el| {
                    el.child(
                        div()
                            .id("desktop-sidebar-update-dismiss")
                            .debug_selector(|| "desktop-sidebar-update-dismiss".to_string())
                            .role(Role::Button)
                            .aria_label("Dismiss")
                            .tab_index(0)
                            .focus_visible(|el| el.border_2().border_color(theme::persimmon_400()))
                            .on_key_down(cx.listener(
                                move |this, event: &KeyDownEvent, window, cx| {
                                    if is_web_button_activation_key(event.keystroke.key.as_str()) {
                                        window.prevent_default();
                                        cx.stop_propagation();
                                        this.updates
                                            .update(cx, |updates, cx| updates.dismiss(id, cx));
                                    }
                                },
                            ))
                            .size(px(24.0))
                            .rounded(px(6.0))
                            .flex()
                            .items_center()
                            .justify_center()
                            .cursor_pointer()
                            .hover(|el| el.bg(theme::longan_800().opacity(0.6)))
                            .tooltip(|window, cx| {
                                Tooltip::new("Dismiss until provider status changes")
                                    .build(window, cx)
                            })
                            .on_click(cx.listener(move |this, _, _, cx| {
                                cx.stop_propagation();
                                this.updates
                                    .update(cx, |updates, cx| updates.dismiss(id, cx));
                            }))
                            .child(icon_svg(assets::CLOSE_ICON_PATH, 12.0, theme::lychee_200())),
                    )
                })
                .into_any_element()
            }
            SidebarRuns::None if matches!(updates.install, AppInstall::Ready { .. }) => {
                let AppInstall::Ready { version, .. } = &updates.install else {
                    return None;
                };
                let tooltip: SharedString =
                    format!("Bruhs v{version} is ready. Restart to finish updating.").into();
                pill("desktop-sidebar-update", tone_action())
                    .role(Role::Button)
                    .aria_label(RESTART_CONFIRM_LABEL)
                    .tab_index(0)
                    .cursor_pointer()
                    .hover(|el| {
                        el.bg(theme::persimmon_500().opacity(0.15))
                            .text_color(theme::persimmon_100())
                    })
                    .focus_visible(|el| el.border_2().border_color(theme::persimmon_400()))
                    .tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx))
                    .on_click(cx.listener(|this, _, window, cx| {
                        cx.stop_propagation();
                        this.open_restart_confirm(window, cx);
                    }))
                    .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                        if is_web_button_activation_key(event.keystroke.key.as_str()) {
                            window.prevent_default();
                            cx.stop_propagation();
                            this.open_restart_confirm(window, cx);
                        }
                    }))
                    .child(icon_svg(
                        assets::REFRESH_ICON_PATH,
                        14.0,
                        theme::persimmon_200(),
                    ))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .child(RESTART_CONFIRM_LABEL),
                    )
                    .into_any_element()
            }
            SidebarRuns::None => {
                let report = updates.report.as_ref().filter(|r| r.has_updates())?;
                let can_update = !report.updatable_providers().is_empty();
                let label = sidebar_label(report);
                let tooltip: SharedString = report.summary().into();
                let count = report.providers.iter().filter(|p| p.is_outdated()).count()
                    + usize::from(report.app.needs_action());
                pill("desktop-sidebar-update", tone_action())
                    .role(Role::Button)
                    .aria_label(label)
                    .tab_index(0)
                    .cursor_pointer()
                    .hover(|el| {
                        el.bg(theme::persimmon_500().opacity(0.15))
                            .text_color(theme::persimmon_100())
                    })
                    .focus_visible(|el| el.border_2().border_color(theme::persimmon_400()))
                    .tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        cx.stop_propagation();
                        this.activate_sidebar_update(can_update, window, cx);
                    }))
                    .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
                        if is_web_button_activation_key(event.keystroke.key.as_str()) {
                            window.prevent_default();
                            cx.stop_propagation();
                            this.activate_sidebar_update(can_update, window, cx);
                        }
                    }))
                    .child(icon_svg(
                        assets::DOWNLOAD_ICON_PATH,
                        14.0,
                        theme::persimmon_200(),
                    ))
                    .child(div().flex_1().min_w_0().truncate().child(label))
                    .when(count > 1, |el| {
                        el.child(
                            div()
                                .text_size(px(12.0))
                                .text_color(theme::persimmon_300().opacity(0.8))
                                .child(count.to_string()),
                        )
                    })
                    .into_any_element()
            }
        };
        Some(
            div()
                .px(px(8.0))
                .pt(px(4.0))
                .flex_shrink_0()
                .child(element)
                .into_any_element(),
        )
    }

    fn activate_sidebar_update(
        &mut self,
        can_update: bool,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if can_update {
            self.open_overlay(Overlay::UpdateConfirm);
            self.focus_first_modal_tab_stop(window, cx);
            cx.notify();
        } else {
            self.open_updates_settings(window, cx);
        }
    }

    pub(super) fn open_restart_confirm(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        self.open_overlay(Overlay::RestartConfirm);
        self.focus_first_modal_tab_stop(window, cx);
        cx.notify();
    }

    /// Swap in the staged update once this process exits, then quit; the
    /// bundled helpers stop with it and the new app opens.
    fn restart_to_update(&mut self, cx: &mut Context<'_, Self>) {
        let AppInstall::Ready { version, staged } = self.updates.read(cx).install.clone() else {
            return;
        };
        let Some(app) = bruhs_core::updates::release::current_app_bundle() else {
            return;
        };
        match bruhs_core::updates::release::install_on_exit(&app, &staged) {
            Ok(()) => {
                self.persist(cx);
                cx.quit();
            }
            Err(error) => self.updates.update(cx, |updates, cx| {
                updates.install = AppInstall::Failed {
                    version,
                    error,
                    at: std::time::Instant::now(),
                };
                cx.notify();
            }),
        }
    }

    /// T3's restart step: Later keeps working; the pill stays.
    pub(super) fn render_restart_confirm(&self, cx: &Context<'_, Self>) -> AnyElement {
        let version = match &self.updates.read(cx).install {
            AppInstall::Ready { version, .. } => version.clone(),
            _ => String::new(),
        };
        let content = div()
            .debug_selector(|| "desktop-restart-confirm".to_string())
            .p(px(16.0))
            .flex()
            .flex_col()
            .gap(px(12.0))
            .child(
                div()
                    .child(
                        div()
                            .text_size(px(18.0))
                            .text_color(theme::rambutan_100())
                            .child(format!("Restart to update to v{version}?")),
                    )
                    .child(meta(
                        "Bruhs quits, swaps in the new version and opens again. Running chats stop; their threads stay.",
                    )),
            )
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap(px(8.0))
                    .child(
                        web_button(
                            "restart-confirm-later",
                            WebButtonVariant::Ghost,
                            WebButtonSize::Small,
                            ButtonState::Enabled,
                        )
                        .debug_selector(|| "desktop-restart-confirm-later".to_string())
                        .tab_index(0)
                        .role(Role::Button)
                        .child("Later")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.dismiss_overlay(|o| matches!(o, Overlay::RestartConfirm));
                            cx.notify();
                        })),
                    )
                    .child(
                        web_button(
                            "restart-confirm-run",
                            WebButtonVariant::Primary,
                            WebButtonSize::Small,
                            ButtonState::Enabled,
                        )
                        .debug_selector(|| "desktop-restart-confirm-run".to_string())
                        .tab_index(0)
                        .role(Role::Button)
                        .child("Restart")
                        .on_click(cx.listener(|this, _, _, cx| this.restart_to_update(cx))),
                    ),
            );
        self.render_modal_shell(RESTART_CONFIRM_LABEL, content, cx)
    }

    pub(super) fn close_update_confirm(&mut self, cx: &mut Context<'_, Self>) {
        self.dismiss_overlay(|overlay| matches!(overlay, Overlay::UpdateConfirm));
        cx.notify();
    }

    /// Web `UpdateConfirmDialog`.
    pub(super) fn render_update_confirm(&self, cx: &Context<'_, Self>) -> AnyElement {
        let report = self.updates.read(cx).report.clone();
        let targets = report
            .as_ref()
            .map(UpdatesReport::updatable_providers)
            .unwrap_or_default();
        let skipped = report
            .as_ref()
            .map(UpdatesReport::skipped_providers)
            .unwrap_or_default();
        let app_behind = report.as_ref().is_some_and(|r| r.app.needs_action());
        let skipped_count = skipped.len() + usize::from(app_behind);
        let rows = targets
            .iter()
            .filter_map(|p| report.as_ref().and_then(|r| r.provider(*p)).cloned())
            .map(|p| {
                div()
                    .rounded(px(8.0))
                    .px(px(12.0))
                    .py(px(8.0))
                    .bg(theme::longan_900())
                    .border_1()
                    .border_color(gpui::white().opacity(0.05))
                    .flex()
                    .flex_col()
                    .gap(px(2.0))
                    .child(
                        div()
                            .flex()
                            .justify_between()
                            .gap(px(8.0))
                            .text_size(px(14.0))
                            .text_color(theme::rambutan_100())
                            .child(provider_cli_label(p.provider))
                            .child(meta(format!(
                                "{} → {}",
                                p.installed.clone().unwrap_or_default(),
                                p.latest.clone().unwrap_or_default()
                            ))),
                    )
                    .child(
                        div()
                            .truncate()
                            .font_family(SETTINGS_CODE_FONT_FAMILY)
                            .text_size(px(SETTINGS_META_TEXT_SIZE_PX))
                            .text_color(theme::rambutan_500())
                            .child(p.command_line().unwrap_or_default()),
                    )
            });
        let mut skipped_lines: Vec<String> = skipped
            .iter()
            .map(|(p, why)| format!("{} · {why}", provider_cli_label(*p)))
            .collect();
        if app_behind {
            skipped_lines
                .push("bruhs · needs a manual update (Settings → Updates has the command)".into());
        }
        let content = div()
            .debug_selector(|| "desktop-update-confirm".to_string())
            .p(px(16.0))
            .flex()
            .flex_col()
            .gap(px(12.0))
            .child(
                div()
                    .child(
                        div()
                            .text_size(px(18.0))
                            .text_color(theme::rambutan_100())
                            .child(confirm_title(targets.len())),
                    )
                    .child(meta(
                        "Running chats keep the CLI they started with. New sessions use the update.",
                    )),
            )
            .child(div().flex().flex_col().gap(px(6.0)).children(rows))
            .when(skipped_count > 0, |el| {
                el.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(4.0))
                        .child(meta(format!("{skipped_count} skipped")))
                        .children(skipped_lines.into_iter().map(|line| meta(line).pl(px(16.0)))),
                )
            })
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap(px(8.0))
                    .child(
                        web_button(
                            "update-confirm-cancel",
                            WebButtonVariant::Ghost,
                            WebButtonSize::Small,
                            ButtonState::Enabled,
                        )
                        .debug_selector(|| "desktop-update-confirm-cancel".to_string())
                        .tab_index(0)
                        .role(Role::Button)
                        .child("Cancel")
                        .on_click(cx.listener(|this, _, _, cx| this.close_update_confirm(cx))),
                    )
                    .child(
                        web_button(
                            "update-confirm-run",
                            WebButtonVariant::Primary,
                            WebButtonSize::Small,
                            ButtonState::disabled_if(targets.is_empty()),
                        )
                        .debug_selector(|| "desktop-update-confirm-run".to_string())
                        .tab_index(0)
                        .role(Role::Button)
                        .child("Update")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.close_update_confirm(cx);
                            this.updates
                                .update(cx, |updates, cx| updates.update_all(cx));
                        })),
                    ),
            );
        self.render_modal_shell(UPDATE_CONFIRM_LABEL, content, cx)
    }

    pub(super) fn render_settings_updates_rows(&self, cx: &Context<'_, Self>) -> Vec<AnyElement> {
        let updates = self.updates.read(cx);
        let toggle = settings_ui::switch(
            "desktop-settings-update-checks",
            updates.enabled,
            false,
            "Check for updates",
            cx,
            |this, _window, cx| this.toggle_update_checks(cx),
        );
        let mut rows = vec![self.settings_row(
            "desktop-settings-row-update-checks",
            "Check for updates",
            "Check installed provider CLIs for newer versions, and this checkout's origin for new commits. When off, bruhs still reads the installed versions but asks nothing online. This device only.",
            settings_ui::control_column(toggle),
        )];
        let status: SharedString = if let Some(error) = &updates.start_error {
            error.clone().into()
        } else if updates.checking {
            "Checking…".into()
        } else if let Some(error) = &updates.error {
            error.clone().into()
        } else {
            match &updates.report {
                Some(report) if !report.online => {
                    "Update checks are off: installed versions only.".into()
                }
                Some(report) => format!(
                    "Last checked {}.",
                    format_relative_ms(report.checked_at, EpochMs::now(), RelativeTimeStyle::Ago)
                )
                .into(),
                None => "Not checked yet.".into(),
            }
        };
        let updatable = updates
            .report
            .as_ref()
            .map_or(0, |r| r.updatable_providers().len());
        let check_now = web_button(
            "desktop-settings-updates-check-now",
            WebButtonVariant::Outline,
            WebButtonSize::Small,
            ButtonState::disabled_if(updates.checking),
        )
        .debug_selector(|| "desktop-settings-updates-check-now".to_string())
        .gap(px(6.0))
        .child(icon_svg(
            assets::REFRESH_ICON_PATH,
            12.0,
            theme::rambutan_200(),
        ))
        .child("Check now")
        .on_click(cx.listener(|this, _, _, cx| {
            this.updates.update(cx, |updates, cx| {
                updates.start_error = None;
                updates.check(true, cx);
            });
        }));
        let status_controls = div()
            .flex()
            .items_center()
            .gap(px(8.0))
            .when(updatable > 0, |el| {
                el.child(
                    web_button(
                        "desktop-settings-updates-update-all",
                        WebButtonVariant::Primary,
                        WebButtonSize::Small,
                        ButtonState::Enabled,
                    )
                    .debug_selector(|| "desktop-settings-updates-update-all".to_string())
                    .child(if updatable > 1 {
                        "Update all"
                    } else {
                        "Update"
                    })
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.updates
                            .update(cx, |updates, cx| updates.update_all(cx));
                    })),
                )
            })
            .child(check_now)
            .into_any_element();
        rows.push(self.settings_row_opts(
            "desktop-settings-row-updates-status",
            "Status",
            "",
            Some(status),
            false,
            status_controls,
        ));
        for provider in [AgentProvider::Claude, AgentProvider::Codex] {
            rows.extend(self.render_provider_update_row(provider, cx));
        }
        rows.extend(self.render_app_update_row(cx));
        rows
    }

    fn render_provider_update_row(
        &self,
        provider: AgentProvider,
        cx: &Context<'_, Self>,
    ) -> Vec<AnyElement> {
        let updates = self.updates.read(cx);
        let entry = updates
            .report
            .as_ref()
            .and_then(|report| report.provider(provider).cloned());
        let run = updates
            .report
            .as_ref()
            .and_then(|report| report.run(provider).cloned());
        let line: SharedString = match &entry {
            Some(entry) => entry.status_line().into(),
            None if updates.checking => "Checking…".into(),
            None => "Not checked yet.".into(),
        };
        let path = entry
            .as_ref()
            .and_then(|entry| entry.binary_path.clone())
            .map(SharedString::from);
        let running = run.as_ref().is_some_and(ProviderRun::is_active);
        let control = entry.clone().map_or_else(
            || div().into_any_element(),
            |entry| self.provider_update_control(entry, running, cx),
        );
        let (row, failure) = match provider {
            AgentProvider::Claude => (
                "desktop-settings-row-update-claude",
                "desktop-settings-update-failure-claude",
            ),
            AgentProvider::Codex => (
                "desktop-settings-row-update-codex",
                "desktop-settings-update-failure-codex",
            ),
        };
        // One block, so the card's dividers fall between providers rather
        // than between a provider and its warning, hint or progress line.
        let mut rows = vec![self.settings_row_opts(
            row,
            provider_cli_label(provider),
            line,
            path,
            false,
            control,
        )];
        let body = || {
            div()
                .px(px(16.0))
                .pb(px(12.0))
                .text_size(px(SETTINGS_META_TEXT_SIZE_PX))
        };
        if let Some((title, message)) = entry.as_ref().and_then(ProviderUpdate::compat_warning) {
            rows.push(
                body()
                    .child(
                        div()
                            .debug_selector(|| "desktop-settings-update-compat".to_string())
                            .rounded(px(8.0))
                            .p(px(12.0))
                            .bg(theme::durian_500().opacity(0.10))
                            .border_1()
                            .border_color(theme::durian_500().opacity(0.20))
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(6.0))
                                    .text_color(theme::durian_200())
                                    .child(icon_svg(
                                        assets::ALERT_TRIANGLE_ICON_PATH,
                                        12.0,
                                        theme::durian_200(),
                                    ))
                                    .child(title),
                            )
                            .child(
                                div()
                                    .mt(px(4.0))
                                    .text_color(theme::rambutan_300())
                                    .child(message),
                            ),
                    )
                    .into_any_element(),
            );
        }
        if let Some(hint) = entry
            .as_ref()
            .filter(|entry| entry.is_outdated() && entry.update_command.is_none())
            .and_then(|entry| entry.manual_hint.clone())
        {
            rows.push(
                body()
                    .text_color(theme::rambutan_400())
                    .child(hint)
                    .into_any_element(),
            );
        }
        match run {
            Some(run) if run.is_active() => rows.push(
                body()
                    .debug_selector(|| "desktop-settings-update-progress".to_string())
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .text_color(theme::rambutan_300())
                    .child(icon_svg(
                        assets::REFRESH_ICON_PATH,
                        12.0,
                        theme::rambutan_300(),
                    ))
                    .child(div().min_w_0().truncate().child(run.phase_line()))
                    .into_any_element(),
            ),
            Some(run) if run.needs_attention() => rows.push(
                body()
                    .child(
                        div()
                            .debug_selector(move || failure.to_string())
                            .rounded(px(8.0))
                            .p(px(12.0))
                            .bg(theme::lychee_500().opacity(0.10))
                            .border_1()
                            .border_color(theme::lychee_500().opacity(0.20))
                            .child(
                                div()
                                    .text_color(theme::lychee_200())
                                    .child(run.message.clone().unwrap_or_default()),
                            )
                            .when(!run.output.is_empty(), |el| {
                                // Web: `max-h-40 overflow-auto`.
                                el.child(
                                    div()
                                        .id(failure)
                                        .mt(px(8.0))
                                        .max_h(px(160.0))
                                        .overflow_y_scroll()
                                        .font_family(SETTINGS_CODE_FONT_FAMILY)
                                        .text_color(theme::rambutan_300())
                                        .child(run.output.clone()),
                                )
                            }),
                    )
                    .into_any_element(),
            ),
            _ => {}
        }
        vec![div().flex().flex_col().children(rows).into_any_element()]
    }

    fn provider_update_control(
        &self,
        entry: ProviderUpdate,
        running: bool,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        let provider = entry.provider;
        if running {
            return meta("Updating…")
                .text_color(theme::rambutan_300())
                .into_any_element();
        }
        match (entry.state, entry.command_line()) {
            (ProviderUpdateState::Outdated, Some(command)) => {
                let (id, copy_id) = match provider {
                    AgentProvider::Claude => (
                        "desktop-settings-update-claude",
                        "desktop-settings-copy-claude",
                    ),
                    AgentProvider::Codex => (
                        "desktop-settings-update-codex",
                        "desktop-settings-copy-codex",
                    ),
                };
                let tooltip: SharedString = format!("Runs `{command}`").into();
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .child(
                        web_button(
                            id,
                            WebButtonVariant::Primary,
                            WebButtonSize::Small,
                            ButtonState::Enabled,
                        )
                        .debug_selector(move || id.to_string())
                        .tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx))
                        .child("Update now")
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.updates
                                .update(cx, |updates, cx| updates.update(provider, cx));
                        })),
                    )
                    .child(copy_button(
                        copy_id,
                        command,
                        "Copy update command. Run it in a terminal when you are ready to update.",
                        cx,
                    ))
                    .into_any_element()
            }
            (ProviderUpdateState::Outdated, None) => meta("Update by hand").into_any_element(),
            (ProviderUpdateState::UpToDate, _) => up_to_date_badge(),
            _ => div().into_any_element(),
        }
    }

    fn render_app_update_row(&self, cx: &Context<'_, Self>) -> Vec<AnyElement> {
        let app = self
            .updates
            .read(cx)
            .report
            .as_ref()
            .map(|report| report.app.clone());
        let line: SharedString = app
            .as_ref()
            .map_or_else(|| "Not checked yet.".to_string(), |app| app.status_line())
            .into();
        let install = self.updates.read(cx).install.clone();
        let release_url = app
            .as_ref()
            .and_then(|app| app.release.as_ref())
            .and_then(|release| release.release_url.clone());
        let control = match (app.as_ref().map(|app| app.state), &install) {
            (_, AppInstall::Ready { .. }) => web_button(
                "desktop-settings-restart-update",
                WebButtonVariant::Primary,
                WebButtonSize::Small,
                ButtonState::Enabled,
            )
            .debug_selector(|| "desktop-settings-restart-update".to_string())
            .child(RESTART_CONFIRM_LABEL)
            .on_click(cx.listener(|this, _, window, cx| this.open_restart_confirm(window, cx)))
            .into_any_element(),
            (_, AppInstall::Downloading(version)) => meta(format!("Downloading v{version}…"))
                .text_color(theme::rambutan_300())
                .into_any_element(),
            (Some(AppUpdateState::Behind), _) if release_url.is_some() => {
                let url = release_url.unwrap_or_default();
                web_button(
                    "desktop-settings-release-download",
                    WebButtonVariant::Outline,
                    WebButtonSize::Small,
                    ButtonState::Enabled,
                )
                .debug_selector(|| "desktop-settings-release-download".to_string())
                .child("Download")
                .on_click(cx.listener(move |_, _, _, cx| cx.open_url(&url)))
                .into_any_element()
            }
            (Some(AppUpdateState::UpToDate), _) => up_to_date_badge(),
            _ => div().into_any_element(),
        };
        let mut rows = vec![self.settings_row_opts(
            "desktop-settings-row-update-app",
            "bruhs",
            line,
            None,
            false,
            control,
        )];
        if let AppInstall::Failed { error, .. } = &install {
            rows.push(
                div()
                    .px(px(16.0))
                    .pb(px(12.0))
                    .text_size(px(SETTINGS_META_TEXT_SIZE_PX))
                    .text_color(theme::lychee_200())
                    .child(error.clone())
                    .into_any_element(),
            );
        }
        if let Some(command) = app.and_then(|app| app.command) {
            rows.push(
                div()
                    .px(px(16.0))
                    .pb(px(12.0))
                    .text_size(px(SETTINGS_META_TEXT_SIZE_PX))
                    .child(
                        div()
                            .mb(px(6.0))
                            .text_color(theme::rambutan_500())
                            .child("bruhs runs from this checkout, so it does not pull for you. Run this in a terminal, then relaunch:"),
                    )
                    .child(
                        div()
                            .debug_selector(|| "desktop-settings-update-command".to_string())
                            .flex()
                            .items_center()
                            .gap(px(8.0))
                            .rounded(px(8.0))
                            .px(px(12.0))
                            .py(px(8.0))
                            .bg(theme::longan_950())
                            .border_1()
                            .border_color(gpui::white().opacity(0.10))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .font_family(SETTINGS_CODE_FONT_FAMILY)
                                    .text_color(theme::rambutan_200())
                                    .child(command.clone()),
                            )
                            .child(copy_button(
                                "desktop-settings-update-copy",
                                command,
                                "Copy update command",
                                cx,
                            )),
                    )
                    .into_any_element(),
            );
        }
        vec![div().flex().flex_col().children(rows).into_any_element()]
    }
}

fn copy_button(
    id: &'static str,
    text: String,
    label: &'static str,
    cx: &Context<'_, Workspace>,
) -> Stateful<Div> {
    web_button(
        id,
        WebButtonVariant::Ghost,
        WebButtonSize::IconSmall,
        ButtonState::Enabled,
    )
    .debug_selector(move || id.to_string())
    .aria_label(label)
    .tooltip(move |window, cx| Tooltip::new(label).build(window, cx))
    .child(icon_svg(
        assets::COPY_ICON_PATH,
        12.0,
        theme::rambutan_500(),
    ))
    .on_click(cx.listener(move |_, _, _, cx| {
        cx.write_to_clipboard(gpui::ClipboardItem::new_string(text.clone()));
    }))
}

fn up_to_date_badge() -> AnyElement {
    div()
        .flex()
        .items_center()
        .gap(px(4.0))
        .text_size(px(SETTINGS_META_TEXT_SIZE_PX))
        .text_color(theme::pandan_400())
        .child(icon_svg(assets::CHECK_ICON_PATH, 12.0, theme::pandan_400()))
        .child("Up to date")
        .into_any_element()
}

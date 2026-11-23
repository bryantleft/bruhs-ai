//! Update checks on the desktop (web `updates.rs` + `components/updates.rs`):
//! the sidebar update pills, the "Update N providers?" confirmation and
//! Settings → Updates. Checks and updates run in process through `bruhs_core::updates`.

mod combined;

use super::*;
use crate::app_state::spawn_tokio;
use bruhs_core::agent_extensions::registry::updates as registry;
use bruhs_core::updates::{release::StageProgress, BuildInfo, UpdateChecker};
use bruhs_models::agent::AgentProvider;
use bruhs_models::time::{format_relative_ms, EpochMs, RelativeTimeStyle};
use bruhs_models::updates::{
    provider_cli_label, running_label, sidebar_label, AppUpdateState, ProviderRun, ProviderUpdate,
    ProviderUpdateState, ReleaseUpdate, UpdatesReport, POLL_INTERVAL_MS, RUN_POLL_INTERVAL_MS,
    SUCCESS_VISIBLE_MS,
};
use bruhs_ui::press::on_press_view;
use bruhs_ui::zoom::px;
use combined::CombinedRun;
use std::collections::{HashMap, HashSet};

pub(super) enum UpdateRow<'a> {
    Confirmation,
    Settings,
    About(&'a str),
}

const RESTART_CONFIRM_LABEL: &str = "Restart to update";

/// A packaged build's own update (T3: download in the background, then
/// "Restart to update").
#[derive(Clone, Debug, PartialEq)]
pub(super) enum AppInstall {
    Idle,
    Staging {
        version: String,
        progress: StageProgress,
    },
    /// Verified and unpacked beside the app; restart swaps it in.
    Ready {
        version: String,
        staged: std::path::PathBuf,
        restart_error: Option<String>,
    },
    /// Retried after [`APP_RETRY_AFTER`] (or on the next launch).
    Failed {
        version: String,
        error: String,
        at: std::time::Instant,
    },
}

impl AppInstall {
    fn progress_label(&self) -> Option<String> {
        let Self::Staging { version, progress } = self else {
            return None;
        };
        Some(match progress {
            StageProgress::Downloading(Some(percent)) => {
                format!("Downloading v{version} · {percent}%")
            }
            StageProgress::Downloading(None) => format!("Downloading v{version}…"),
            StageProgress::Verifying => format!("Verifying v{version}…"),
        })
    }
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

pub(super) use bruhs_core::updates::release::UpdateTrack;

pub(super) struct DesktopUpdates {
    pub track: UpdateTrack,
    checker: UpdateChecker,
    pub(super) bundle: Option<std::path::PathBuf>,
    pub(super) translated: bool,
    /// Online checks (registries, the remote) run only while this is on.
    pub enabled: bool,
    pub provider_checks: bool,
    pub report: Option<UpdatesReport>,
    registry: Vec<registry::Installed>,
    registry_notice: Option<String>,
    registry_errors: HashMap<String, String>,
    registry_busy: HashSet<String>,
    pub(super) registry_revision: u64,
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
    starting: HashSet<AgentProvider>,
    combined: Option<CombinedRun>,
    combined_timer: Option<Task<()>>,
    /// Runs that finished before this window opened stay out of the sidebar.
    loaded_at: EpochMs,
    /// The copy button that shows its check, like web's.
    pub copied: crate::views::copy_feedback::CopyFeedback,
    _poll: Option<Task<()>>,
}

impl DesktopUpdates {
    pub(super) fn new(
        enabled: bool,
        provider_checks: bool,
        track: UpdateTrack,
        cx: &mut Context<'_, Self>,
    ) -> Self {
        let mut this = Self {
            bundle: bruhs_core::updates::release::current_app_bundle(),
            translated: bruhs_core::updates::release::running_under_rosetta(),
            checker: UpdateChecker::new(
                BuildInfo::new(
                    env!("BRUHS_BUILD_COMMIT"),
                    env!("BRUHS_REPO_ROOT"),
                    "cargo run --release -p bruhs-desktop",
                )
                .with_release(env!("BRUHS_RELEASE_VERSION")),
            ),
            enabled,
            provider_checks,
            track,
            report: None,
            checking: false,
            registry: Vec::new(),
            registry_notice: None,
            registry_errors: HashMap::new(),
            registry_busy: HashSet::new(),
            registry_revision: 0,
            error: None,
            start_error: None,
            dismissed: HashSet::new(),
            install: AppInstall::Idle,
            starting: HashSet::new(),
            combined: None,
            combined_timer: None,
            loaded_at: EpochMs::now(),
            copied: Default::default(),
            _poll: None,
        };
        // Tests drive checks by hand; a real check spawns CLIs and dials out.
        if cfg!(not(test)) {
            #[cfg(feature = "native-performance")]
            if crate::native_performance::block_service(
                cx,
                crate::native_performance::Service::Updates,
            ) {
                return this;
            }
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

    pub(super) fn set_track(&mut self, track: UpdateTrack, cx: &mut Context<'_, Self>) {
        if self.track == track {
            return;
        }
        self.combined = None;
        self.combined_timer = None;
        self.track = track;
        self.report = None;
        // Let an active writer finish before reusing the shared staging directory.
        if !matches!(self.install, AppInstall::Staging { .. }) {
            self.install = AppInstall::Idle;
        }
        self.check(true, cx);
        cx.notify();
    }

    pub(super) fn set_enabled(&mut self, on: bool, cx: &mut Context<'_, Self>) {
        self.enabled = on;
        self.check(false, cx);
        cx.notify();
    }

    pub(super) fn check(&mut self, force: bool, cx: &mut Context<'_, Self>) {
        self.run_check(force, false, cx);
    }

    fn check_now(&mut self, cx: &mut Context<'_, Self>) {
        self.start_error = None;
        self.run_check(true, true, cx);
    }

    fn run_check(&mut self, force: bool, manual: bool, cx: &mut Context<'_, Self>) {
        if self.checking || cx.try_global::<crate::app_state::DesktopState>().is_none() {
            return;
        }
        self.checking = true;
        cx.notify();
        let checker = self.checker.clone();
        let enabled = self.enabled;
        let online = manual || enabled;
        let providers = self.provider_checks;
        let track = self.track;
        let revision = self.registry_revision;
        let task = spawn_tokio(cx, async move {
            tokio::join!(
                checker.report_on_track(force, online, providers, track),
                registry::check(online && providers, manual)
            )
        });
        cx.spawn(async move |this, cx| {
            let (report, registry) = task.await;
            let _ = this.update(cx, |this, cx| {
                this.checking = false;
                this.error = None;
                if track == this.track
                    && enabled == this.enabled
                    && providers == this.provider_checks
                {
                    if this.registry_revision == revision {
                        match registry {
                            Ok(checked) => {
                                this.registry = checked.installed;
                                this.registry_notice = checked.notice;
                                this.registry_errors.retain(|id, _| {
                                    this.registry.iter().any(|r| {
                                        r.id() == id
                                            && !matches!(
                                                r.state,
                                                registry::State::Current | registry::State::Ahead
                                            )
                                    })
                                });
                            }
                            Err(e) => {
                                this.registry.clear();
                                this.registry_notice =
                                    Some(format!("Could not read installed registry agents. {e}"));
                            }
                        }
                    } else {
                        this.check(false, cx);
                    }
                    this.report = Some(report);
                    this.stage_app_update(false, cx);
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
        if cx.try_global::<crate::app_state::DesktopState>().is_none()
            || !self.starting.insert(provider)
        {
            return;
        }
        let checker = self.checker.clone();
        let task = spawn_tokio(cx, async move { checker.start_update(provider) });
        cx.spawn(async move |this, cx| {
            let started = task.await;
            let _ = this.update(cx, |this, cx| {
                this.starting.remove(&provider);
                if let Some(batch) = &mut this.combined {
                    batch.provider_started(
                        provider,
                        started
                            .as_ref()
                            .map(|run| run.id)
                            .map_err(ToString::to_string),
                    );
                }
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
    /// version. A user-initiated retry bypasses the automatic failure backoff.
    fn stage_app_update(&mut self, retry: bool, cx: &mut Context<'_, Self>) {
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
            AppInstall::Failed { at, .. } if retry || at.elapsed() >= APP_RETRY_AFTER => None,
            // Releases share a staging directory. Finish this download before
            // starting a newer release discovered by another poll.
            AppInstall::Staging { .. } => return,
            AppInstall::Ready { version: v, .. } | AppInstall::Failed { version: v, .. } => Some(v),
        };
        if current == Some(&version) {
            return;
        }
        let Some(app) = self.bundle.clone() else {
            return;
        };
        let track = self.track;
        let initial = StageProgress::Downloading(None);
        self.install = AppInstall::Staging {
            version: version.clone(),
            progress: initial,
        };
        let (sender, mut progress) = tokio::sync::watch::channel(initial);
        cx.notify();
        let runtime = &cx.global::<crate::app_state::DesktopState>().runtime;
        let task = runtime.spawn(async move {
            bruhs_core::updates::release::stage_update_with_progress(&release, &app, move |state| {
                sender.send_replace(state);
            })
            .await
        });
        cx.spawn(async move |this, cx| {
            while progress.changed().await.is_ok() {
                let next = *progress.borrow_and_update();
                let _ = this.update(cx, |this, cx| {
                    if let AppInstall::Staging { progress, .. } = &mut this.install {
                        *progress = next;
                        cx.notify();
                    }
                });
            }
            let staged = task
                .await
                .unwrap_or_else(|error| Err(format!("The update download stopped: {error}")));
            let _ = this.update(cx, |this, cx| {
                if this.track != track {
                    this.install = AppInstall::Idle;
                    this.stage_app_update(false, cx);
                    cx.notify();
                    return;
                }
                this.install = match staged {
                    Ok(staged) => AppInstall::Ready {
                        version,
                        staged,
                        restart_error: None,
                    },
                    Err(error) => {
                        tracing::warn!("app update v{version}: {error}");
                        AppInstall::Failed {
                            version,
                            error,
                            at: std::time::Instant::now(),
                        }
                    }
                };
                this.stage_app_update(false, cx);
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn registry_config_changed(&mut self, cx: &mut Context<'_, Self>) {
        self.registry_revision += 1;
        self.registry.clear();
        self.check(false, cx);
        cx.notify();
    }

    fn registry_targets(&self) -> impl Iterator<Item = &registry::Installed> {
        self.registry
            .iter()
            .filter(|r| r.target().is_some() && !self.registry_busy.contains(r.id()))
    }

    fn update_registry(&mut self, id: &str, cx: &mut Context<'_, Self>) {
        let Some(row) = self.registry_targets().find(|r| r.id() == id).cloned() else {
            return;
        };
        let id = id.to_owned();
        self.registry_busy.insert(id.clone());
        self.registry_errors.remove(&id);
        let job = spawn_tokio(cx, async move { row.apply().await });
        cx.spawn(async move |this, cx| {
            let result = job.await;
            let _ = this.update(cx, |this, cx| {
                this.registry_busy.remove(&id);
                if let Some(batch) = &mut this.combined {
                    let error = result.as_ref().err().map(ToString::to_string);
                    batch.registry_finished(&id, error.as_deref());
                }
                match result {
                    Ok(updated) => {
                        if let Some(row) = this.registry.iter_mut().find(|r| r.id() == id) {
                            *row = updated;
                        }
                        this.registry_revision += 1;
                    }
                    Err(err) => {
                        this.registry_errors.insert(id, err.to_string());
                        this.check(false, cx);
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// Start each available provider update. The built-in checker queues shared installers.
    pub(super) fn update_all(&mut self, cx: &mut Context<'_, Self>) {
        let providers = self
            .report
            .as_ref()
            .map(UpdatesReport::updatable_providers)
            .unwrap_or_default();
        let agents: Vec<_> = self.registry_targets().map(|r| r.id().to_owned()).collect();
        for id in agents {
            self.update_registry(&id, cx);
        }
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
        .text_size(px(theme::APP_UI_TEXT_SIZE_PX))
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

/// Web `animate-spin`: one turn a second.
const SPINNER_TURN_MS: u64 = 1000;

/// The running pill's loader. Like web's `motion-safe:animate-spin`, it
/// turns unless the system asks for reduced motion.
fn update_spinner(cx: &App) -> AnyElement {
    let loader = icon_svg(
        assets::LOADER_ICON_PATH,
        theme::ICON_PX,
        theme::rambutan_300(),
    );
    if crate::motion::reduce_motion(cx) {
        return loader
            .debug_selector(|| "desktop-sidebar-update-spinner-static".into())
            .into_any_element();
    }
    loader
        .debug_selector(|| "desktop-sidebar-update-spinner-animated".into())
        .with_animation(
            "desktop-sidebar-update-spinner",
            Animation::new(std::time::Duration::from_millis(SPINNER_TURN_MS)).repeat(),
            |loader, delta| loader.with_transformation(Transformation::rotate(percentage(delta))),
        )
        .into_any_element()
}

pub(super) fn icon_svg(path: &'static str, size: f32, color: gpui::Hsla) -> Svg {
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

    pub(super) fn sidebar_check_button(&self, cx: &Context<'_, Self>) -> AnyElement {
        let checking = self.updates.read(cx).checking;
        let label = self.ui_language().text(if checking {
            "Checking for updates…"
        } else {
            "Check for updates"
        });
        let button = div()
            .id("desktop-sidebar-check-updates")
            .debug_selector(|| "desktop-sidebar-check-updates".into())
            .size(px(theme::CONTROL_PX))
            .flex_shrink_0()
            .rounded_full()
            .flex()
            .items_center()
            .justify_center()
            .track_focus(&self.modal_focus.restart_sidebar)
            .tab_index(0)
            .aria_label(label)
            .border_1()
            .border_color(gpui::transparent_black())
            .focus_visible(|el| el.border_color(theme::persimmon_400()))
            .tooltip(move |window, cx| Tooltip::new(label).build(window, cx));
        if checking {
            return button
                .role(Role::Status)
                .child(update_spinner(cx))
                .into_any_element();
        }
        let target = cx.weak_entity();
        let button = button
            .role(Role::Button)
            .on_a11y_action(gpui::AccessibleAction::Click, move |_, _, cx| {
                let _ = target.update(cx, |ws, cx| {
                    ws.check_linked_providers(cx);
                    ws.updates.update(cx, |u, cx| u.check_now(cx));
                });
            })
            .cursor_pointer()
            .hover(|el| el.bg(theme::longan_800().opacity(0.4)))
            .child(icon_svg(
                assets::REFRESH_ICON_PATH,
                theme::ICON_PX,
                theme::rambutan_500(),
            ));
        on_press_view(button, cx, |this, _, cx| {
            this.check_linked_providers(cx);
            this.updates.update(cx, |updates, cx| updates.check_now(cx));
        })
        .into_any_element()
    }

    /// Web `SidebarUpdateButton`: the provider pill while runs are active or
    /// just finished, else Update while something is behind.
    pub(super) fn render_sidebar_update_button(
        &self,
        cx: &Context<'_, Self>,
    ) -> Option<AnyElement> {
        let updates = self.updates.read(cx);
        let element = if let Some(run) = &updates.combined {
            self.combined_sidebar(run, cx)
        } else {
            self.provider_or_desktop_pill(updates, cx)?
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

    fn provider_or_desktop_pill(
        &self,
        updates: &DesktopUpdates,
        cx: &Context<'_, Self>,
    ) -> Option<AnyElement> {
        Some(match updates.sidebar_runs(EpochMs::now()) {
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
                    .track_focus(&self.modal_focus.restart_sidebar)
                    .tab_index(0)
                    .cursor_pointer()
                    .focus_visible(|el| el.border_color(theme::persimmon_400()))
                    .map(|el| {
                        on_press_view(el, cx, |this, window, cx| {
                            this.open_updates_settings(window, cx);
                        })
                    })
                    .tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx))
                    .child(update_spinner(cx))
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
                        .track_focus(&self.modal_focus.restart_sidebar)
                        .role(Role::Button)
                        .aria_label(run.pill_label())
                        .tab_index(0)
                        .border_1()
                        .border_color(gpui::transparent_black())
                        .focus_visible(|el| el.border_color(theme::persimmon_400()))
                        .map(|el| {
                            on_press_view(el, cx, |this, window, cx| {
                                this.open_updates_settings(window, cx);
                            })
                        })
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .items_center()
                        .gap(px(8.0))
                        .cursor_pointer()
                        .child(if failed {
                            icon_svg(
                                assets::ALERT_TRIANGLE_ICON_PATH,
                                theme::ICON_PX,
                                theme::lychee_200(),
                            )
                        } else {
                            icon_svg(assets::CHECK_ICON_PATH, theme::ICON_PX, theme::pandan_400())
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
                            .border_1()
                            .border_color(gpui::transparent_black())
                            .focus_visible(|el| el.border_color(theme::persimmon_400()))
                            .map(|el| {
                                on_press_view(el, cx, move |this, _window, cx| {
                                    this.updates
                                        .update(cx, |updates, cx| updates.dismiss(id, cx));
                                })
                            })
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
                            .child(icon_svg(
                                assets::CLOSE_ICON_PATH,
                                theme::ICON_PX,
                                theme::lychee_200(),
                            )),
                    )
                })
                .into_any_element()
            }
            SidebarRuns::None
                if self.linked_projects.peer_updates().is_empty()
                    && updates.registry_targets().next().is_none()
                    && updates.registry_busy.is_empty()
                    && matches!(updates.install, AppInstall::Ready { .. })
                    && !updates
                        .report
                        .as_ref()
                        .is_some_and(|report| !report.updatable_providers().is_empty()) =>
            {
                let AppInstall::Ready { version, .. } = &updates.install else {
                    return None;
                };
                let tooltip: SharedString =
                    format!("Bruhs v{version} is ready. Restart to finish updating.").into();
                pill("desktop-sidebar-update", tone_action())
                    .role(Role::Button)
                    .aria_label(RESTART_CONFIRM_LABEL)
                    .track_focus(&self.modal_focus.restart_sidebar)
                    .tab_index(0)
                    .cursor_pointer()
                    .hover(|el| {
                        el.bg(theme::persimmon_500().opacity(0.15))
                            .text_color(theme::persimmon_100())
                    })
                    .focus_visible(|el| el.border_color(theme::persimmon_400()))
                    .tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx))
                    .map(|el| {
                        on_press_view(el, cx, |this, window, cx| {
                            this.open_restart_confirm(window, cx);
                        })
                    })
                    .child(icon_svg(
                        assets::REFRESH_ICON_PATH,
                        theme::ICON_PX,
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
                let registry_count = updates.registry_targets().count();
                let peers = self.linked_projects.peer_updates();
                let busy = !updates.registry_busy.is_empty();
                let report = updates.report.as_ref();
                let progress = updates.install.progress_label();
                if progress.is_none()
                    && !busy
                    && peers.is_empty()
                    && registry_count == 0
                    && !report.is_some_and(UpdatesReport::has_updates)
                {
                    return None;
                }
                let can_update = !peers.is_empty()
                    || registry_count > 0
                    || progress.is_some()
                    || report.is_some_and(|r| {
                        !r.updatable_providers().is_empty() || r.app.state == AppUpdateState::Behind
                    });
                let label = progress.clone().unwrap_or_else(|| {
                    if busy {
                        "Updating providers…"
                    } else if registry_count > 0 || !peers.is_empty() {
                        "Update"
                    } else {
                        report.map_or("Update", sidebar_label)
                    }
                    .to_owned()
                });
                let tooltip: SharedString = if busy {
                    "Updating installed registry agents".into()
                } else if registry_count > 0 {
                    updates
                        .registry_targets()
                        .map(|r| r.name())
                        .collect::<Vec<_>>()
                        .join(", ")
                        .into()
                } else if let Some(progress) = progress {
                    progress.into()
                } else {
                    report
                        .map(UpdatesReport::summary)
                        .unwrap_or_default()
                        .into()
                };
                let count = peers.len()
                    + registry_count
                    + report.map_or(0, |r| {
                        r.providers.iter().filter(|p| p.is_outdated()).count()
                            + usize::from(r.app.needs_action())
                    });
                pill("desktop-sidebar-update", tone_action())
                    .role(Role::Button)
                    .aria_label(label.clone())
                    .track_focus(&self.modal_focus.restart_sidebar)
                    .tab_index(0)
                    .cursor_pointer()
                    .hover(|el| {
                        el.bg(theme::persimmon_500().opacity(0.15))
                            .text_color(theme::persimmon_100())
                    })
                    .focus_visible(|el| el.border_color(theme::persimmon_400()))
                    .tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx))
                    .map(|el| {
                        on_press_view(el, cx, move |this, window, cx| {
                            this.activate_sidebar_update(can_update, window, cx);
                        })
                    })
                    .child(icon_svg(
                        assets::DOWNLOAD_ICON_PATH,
                        theme::ICON_PX,
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
        })
    }

    pub(super) fn activate_sidebar_update(
        &mut self,
        can_update: bool,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if can_update {
            self.open_overlay(Overlay::UpdateConfirm {
                from_settings: self.settings_visible(),
                details_open: false,
            });
            if self
                .updates
                .read(cx)
                .combined_targets(self.linked_projects.peer_updates())
                .is_some()
            {
                let cancel = self.modal_focus.update_cancel.clone();
                cx.defer_in(window, move |_, window, cx| cancel.focus(window, cx));
            } else {
                self.focus_first_modal_tab_stop(window, cx);
            }
            cx.notify();
        } else {
            self.open_updates_settings(window, cx);
        }
    }

    pub(super) fn open_restart_confirm(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let previous = self.overlay.take().map(Box::new);
        self.open_overlay(Overlay::RestartConfirm { previous });
        self.focus_first_modal_tab_stop(window, cx);
        cx.notify();
    }

    pub(super) fn dismiss_restart_confirm(
        &mut self,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> bool {
        let Some(Overlay::RestartConfirm { previous }) = &mut self.overlay else {
            return false;
        };
        self.overlay = previous.take().map(|overlay| *overlay);
        let focus = if self.overlay.is_some() {
            self.modal_focus.restart_row.clone()
        } else {
            self.modal_focus.restart_sidebar.clone()
        };
        cx.defer_in(window, move |_, window, cx| focus.focus(window, cx));
        cx.notify();
        true
    }

    /// Swap in the staged update once this process exits, then quit; the
    /// bundled helpers stop with it and the new app opens.
    fn restart_to_update(&mut self, cx: &mut Context<'_, Self>) -> bool {
        if self.updates.read(cx).combined.is_some() {
            return false;
        }
        let AppInstall::Ready { staged, .. } = self.updates.read(cx).install.clone() else {
            return false;
        };
        let Some(app) = self.updates.read(cx).bundle.clone() else {
            return false;
        };
        match bruhs_core::updates::release::install_on_exit(&app, &staged) {
            Ok(()) => {
                self.persist(cx);
                cx.quit();
                true
            }
            Err(error) => self.updates.update(cx, |updates, cx| {
                if let AppInstall::Ready { restart_error, .. } = &mut updates.install {
                    *restart_error = Some(error);
                }
                cx.notify();
                false
            }),
        }
    }

    /// T3's restart step: Later keeps working; the pill stays.
    pub(super) fn render_restart_confirm(
        &self,
        window: &Window,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        let (version, error) = match &self.updates.read(cx).install {
            AppInstall::Ready {
                version,
                restart_error,
                ..
            } => (version.clone(), restart_error.clone()),
            _ => (String::new(), None),
        };
        let title = if version.is_empty() {
            "Restart to update?".to_string()
        } else {
            format!("Restart to update to v{version}?")
        };
        let content = div()
            .debug_selector(|| "desktop-restart-confirm".to_string())
            .p(px(16.0))
            .flex()
            .flex_col()
            .gap(px(12.0))
            .when_some(error, |el, error| el.child(meta(error).id("restart-error").role(Role::Alert).text_color(theme::lychee_200())))
            .child(meta(
                "Bruhs quits, swaps in the new version and opens again. Running chats stop; their threads stay.",
            ))
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
                        .map(|el| on_press_view(el, cx, |this, window, cx| {
                            this.dismiss_restart_confirm(window, cx);
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
                        .map(|el| on_press_view(el, cx, |this, _, cx| { this.restart_to_update(cx); })),
                    ),
            );
        self.render_modal_shell(title, content, None, window, cx)
    }

    pub(super) fn dismiss_update_confirm(
        &mut self,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> bool {
        let Some(Overlay::UpdateConfirm { from_settings, .. }) = self.overlay else {
            return false;
        };
        self.overlay = from_settings.then_some(Overlay::Settings {
            open_dropdown: None,
        });
        let focus = if from_settings && self.updates.read(cx).combined.is_some() {
            self.modal_start_focus_handle.clone()
        } else if from_settings {
            self.modal_focus.update_all.clone()
        } else {
            self.modal_focus.restart_sidebar.clone()
        };
        cx.defer_in(window, move |_, window, cx| focus.focus(window, cx));
        cx.notify();
        true
    }

    /// Web `UpdateConfirmDialog`.
    pub(super) fn render_update_confirm(
        &self,
        window: &Window,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        let report = self.updates.read(cx).report.clone();
        let targets = report
            .as_ref()
            .map(UpdatesReport::updatable_providers)
            .unwrap_or_default();
        let skipped = report
            .as_ref()
            .map(UpdatesReport::skipped_providers)
            .unwrap_or_default();
        let updates = self.updates.read(cx);
        let registry: Vec<_> = updates.registry_targets().collect();
        let has_targets = !targets.is_empty()
            || !registry.is_empty()
            || !self.linked_projects.peer_updates().is_empty();
        let combined = updates.combined_targets(self.linked_projects.peer_updates());
        let combined_dialog = combined.is_some();
        let details_open = matches!(
            self.overlay,
            Some(Overlay::UpdateConfirm {
                details_open: true,
                ..
            })
        );
        let show_details = !combined_dialog || details_open;
        let restarting = combined.as_ref().is_some_and(|p| p.version.is_some());
        let remote = combined.as_ref().is_some_and(|p| !p.peers.is_empty());
        let action = if restarting {
            "Update all and restart"
        } else {
            "Update providers"
        };
        let desktop_update = report.as_ref().is_some_and(|r| r.app.needs_action())
            || matches!(
                updates.install,
                AppInstall::Staging { .. } | AppInstall::Ready { .. }
            );
        let skipped_hosts = self.linked_projects.skipped_update_hosts();
        let skipped_count = skipped.len();
        let rows = targets
            .iter()
            .filter_map(|p| report.as_ref().and_then(|r| r.provider(*p)).cloned())
            .map(|p| {
                confirm_provider_row(
                    provider_cli_label(p.provider),
                    format!(
                        "{} → {}",
                        p.installed.as_deref().unwrap_or_default(),
                        p.latest.as_deref().unwrap_or_default()
                    ),
                    p.command_line().unwrap_or_default(),
                )
            })
            .chain(registry.iter().map(|r| {
                let detail = updates
                    .registry_errors
                    .get(r.id())
                    .cloned()
                    .unwrap_or_else(|| "ACP Registry".into());
                confirm_provider_row(r.name().to_owned(), registry_status(r), detail)
            }));
        let skipped_lines: Vec<String> = skipped
            .iter()
            .map(|(p, why)| format!("{}, {why}", provider_cli_label(*p)))
            .collect();
        let compact = bruhs_ui::zoom::css_px(window.viewport_size().width) < 640.0;
        let content = div()
            .debug_selector(|| "desktop-update-confirm".to_string())
            .px(px(24.0))
            .py(px(16.0))
            .flex()
            .flex_col()
            .gap(px(12.0))
            .when_some(combined.as_ref(), |el, plan| {
                el.child(plan.render_summary(&registry))
            })
            .when(!skipped_hosts.is_empty(), |el| {
                el.child(meta("Skipped machines"))
                    .children(skipped_hosts.into_iter().map(meta))
            })
            .when(show_details, |el| {
                el.children(self.render_app_update_row(UpdateRow::Confirmation, cx))
            })
            .when(has_targets && show_details, |el| {
                el.child(meta(
                    "Running chats keep their current CLI. New sessions use updated providers.",
                ))
                .child(div().flex().flex_col().gap(px(6.0)).children(rows))
            })
            .when(restarting, |el| {
                let working = self
                    .threads
                    .iter()
                    .filter(|t| t.chat.read(cx).is_working())
                    .count();
                let detail = match working {
                    0 => "Bruhs restarts last.".to_owned(),
                    1 => "Bruhs restarts last and interrupts 1 running thread.".to_owned(),
                    n => format!("Bruhs restarts last and interrupts {n} running threads."),
                };
                el.child(meta(format!(
                    "{detail} Choose Later during the countdown to keep working."
                )))
            })
            .when(combined_dialog, |el| {
                let text = if details_open {
                    "Hide update details"
                } else {
                    "Show update details"
                };
                let button = web_button(
                    "combined-update-details",
                    WebButtonVariant::Ghost,
                    WebButtonSize::Small,
                    ButtonState::Enabled,
                )
                .debug_selector(|| "desktop-combined-update-details".into())
                .role(Role::Button)
                .tab_index(0)
                .aria_expanded(details_open)
                .aria_label(text)
                .child(text);
                el.child(div().flex().child(on_press_view(button, cx, |ws, _, cx| {
                    if let Some(Overlay::UpdateConfirm { details_open, .. }) = &mut ws.overlay {
                        *details_open = !*details_open;
                        cx.notify();
                    }
                })))
            })
            .when(skipped_count > 0, |el| {
                el.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(4.0))
                        .child(meta(format!("{skipped_count} skipped")))
                        .children(
                            skipped_lines
                                .into_iter()
                                .map(|line| meta(line).pl(px(16.0))),
                        ),
                )
            });
        let machine_title = combined
            .as_ref()
            .filter(|_| remote)
            .map(|p| match p.machine_count() {
                1 => "Update 1 machine?".into(),
                n => format!("Update {n} machines?"),
            });
        let button_height = if compact {
            px(36.0)
        } else {
            px(theme::CONTROL_PX)
        };
        let cancel_btn = web_button(
            "update-confirm-cancel",
            WebButtonVariant::Ghost,
            WebButtonSize::Default,
            ButtonState::Enabled,
        )
        .h(button_height)
        .when(compact, |el| el.w_full().justify_center())
        .debug_selector(|| "desktop-update-confirm-cancel".to_string())
        .when(combined_dialog, |el| {
            el.track_focus(&self.modal_focus.update_cancel)
        })
        .aria_label(if combined_dialog {
            "Cancel updates"
        } else if has_targets {
            "Cancel provider updates"
        } else {
            "Close update details"
        })
        .tab_index(0)
        .role(Role::Button)
        .child(if has_targets { "Cancel" } else { "Close" })
        .map(|el| {
            on_press_view(el, cx, |this, window, cx| {
                this.dismiss_update_confirm(window, cx);
            })
        });
        let footer = div()
            .flex()
            .when(compact, |el| el.flex_col().gap(px(8.0)))
            .when(!compact, |el| el.justify_end().gap(px(8.0)))
            .child(cancel_btn)
            .when(has_targets, |el| {
                el.child(
                    web_button(
                        "update-confirm-run",
                        WebButtonVariant::Primary,
                        WebButtonSize::Default,
                        ButtonState::Enabled,
                    )
                    .h(button_height)
                    .when(compact, |el| el.w_full().justify_center())
                    .debug_selector(|| "desktop-update-confirm-run".to_string())
                    .aria_label(action)
                    .tab_index(0)
                    .role(Role::Button)
                    .child(action)
                    .map(|el| {
                        on_press_view(el, cx, move |this, window, cx| {
                            if let Some(expected) = &combined {
                                if this.linked_projects.peer_updates()!=expected.peers || !this.updates.update(cx, |updates, cx| updates.start_combined(expected, cx)) {
                                    this.meetings_hub.update(cx, |hub, cx| hub.show_notice(
                                        "The available updates changed. Review the updated list.", false, cx));
                                    cx.notify();
                                    return;
                                }
                                for peer in &expected.peers { this.start_peer_update(peer.clone(),cx); }
                            } else {
                                this.updates.update(cx, |updates, cx| updates.update_all(cx));
                            }
                            this.dismiss_update_confirm(window, cx);
                        })
                    }),
                )
            });

        let title: SharedString = if let Some(title) = machine_title {
            title.into()
        } else if combined_dialog {
            "Update this Mac?".into()
        } else if !has_targets && desktop_update {
            "Desktop update".into()
        } else {
            "Provider and desktop updates".into()
        };
        bruhs_ui::dialog::dialog_backdrop(
            "desktop-modal-overlay",
            bruhs_ui::dialog::DialogPlacement::Centered,
        )
        .when(compact, |el| el.items_end().py_0())
        .on_click(cx.listener(|this, _ev, window, cx| {
            if this.dismiss_update_confirm(window, cx) {
                return;
            }
            this.close_transient_surfaces();
            cx.notify();
        }))
        .child(
            bruhs_ui::dialog::dialog_card("desktop-modal-dialog", title.clone(), Role::Dialog, cx)
                .debug_selector(|| "desktop-modal-dialog".to_string())
                .child(self.blur_scrim.backdrop())
                .when(compact, |el| {
                    el.w_full().max_w_full().rounded_none().rounded_t(px(16.0))
                })
                .when(!compact, |el| {
                    el.w(px(576.0)).max_w(relative(0.9)).rounded(px(16.0))
                })
                .max_h(dialog_max_height(window))
                .track_focus(&self.modal_start_focus_handle)
                .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                    this.trap_modal_tab_focus(event, window, cx);
                }))
                .on_click(cx.listener(|_this, _ev, _window, cx| {
                    cx.stop_propagation();
                }))
                .child(
                    div()
                        .flex_shrink_0()
                        .px(px(24.0))
                        .pt(px(24.0))
                        .pb(px(if compact { 16.0 } else { 24.0 }))
                        .border_b_1()
                        .border_color(theme::longan_800())
                        .flex()
                        .items_center()
                        .justify_between()
                        .child(
                            div()
                                .font_family(theme::HEADING_FONT_FAMILY)
                                .text_size(px(DESKTOP_MODAL_TITLE_TEXT_SIZE_PX))
                                .font_weight(FontWeight::MEDIUM)
                                .line_height(relative(1.5))
                                .text_color(theme::rambutan_100())
                                .child(title),
                        )
                        .child(
                            web_button(
                                "close-modal",
                                WebButtonVariant::Ghost,
                                WebButtonSize::IconSmall,
                                ButtonState::Enabled,
                            )
                            .debug_selector(|| "desktop-modal-close".to_string())
                            .aria_label("Close dialog")
                            .child(
                                svg()
                                    .path(assets::CLOSE_ICON_PATH)
                                    .size(px(theme::ICON_PX))
                                    .text_color(theme::rambutan_400()),
                            )
                            .track_focus(&self.modal_focus.close)
                            .tab_index(0)
                            .role(Role::Button)
                            .focus_visible(|el| el.border_color(theme::persimmon_400()))
                            .map(|el| {
                                on_press_view(el, cx, |this, window, cx| {
                                    if this.dismiss_update_confirm(window, cx) {
                                        return;
                                    }
                                    this.close_transient_surfaces();
                                    cx.notify();
                                })
                            }),
                        ),
                )
                .child(self.render_modal_content_start())
                .child(
                    div()
                        .id("desktop-modal-body")
                        .min_h_0()
                        .overflow_y_scroll()
                        .track_scroll(&self.modal_scroll)
                        .child(content)
                        .vertical_scrollbar(&self.modal_scroll),
                )
                .child(
                    div()
                        .id("desktop-modal-footer")
                        .debug_selector(|| "desktop-modal-footer".into())
                        .flex_shrink_0()
                        .px(px(24.0))
                        .py(px(16.0))
                        .child(footer),
                )
                .child(self.render_modal_focus_sentinel()),
        )
        .into_any_element()
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
            "Check for desktop and provider updates automatically.",
            toggle,
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
        let updatable = self.linked_projects.peer_updates().len()
            + updates.registry_targets().count()
            + updates
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
        .aria_label("Check now")
        .gap(px(6.0))
        .child(icon_svg(
            assets::REFRESH_ICON_PATH,
            12.0,
            theme::rambutan_200(),
        ))
        .child("Check now")
        .on_click(cx.listener(|this, _, _, cx| {
            this.check_linked_providers(cx);
            this.updates.update(cx, |updates, cx| {
                updates.check_now(cx);
            });
        }));
        let includes_desktop = updates
            .combined_targets(self.linked_projects.peer_updates())
            .is_some();
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
                        ButtonState::disabled_if(updates.combined.is_some()),
                    )
                    .debug_selector(|| "desktop-settings-updates-update-all".to_string())
                    .track_focus(&self.modal_focus.update_all)
                    .aria_label(if includes_desktop {
                        "Review provider and desktop updates"
                    } else {
                        "Update providers"
                    })
                    .child(if includes_desktop || updatable > 1 {
                        "Update all"
                    } else {
                        "Update"
                    })
                    .map(|el| {
                        on_press_view(el, cx, |this, window, cx| {
                            if this.updates.read(cx).combined.is_some() {
                                return;
                            }
                            if this
                                .updates
                                .read(cx)
                                .combined_targets(this.linked_projects.peer_updates())
                                .is_some()
                            {
                                this.activate_sidebar_update(true, window, cx);
                            } else {
                                this.updates
                                    .update(cx, |updates, cx| updates.update_all(cx));
                            }
                        })
                    }),
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
        rows.extend(self.linked_provider_rows(cx));
        for provider in [AgentProvider::Claude, AgentProvider::Codex] {
            rows.extend(self.render_provider_update_row(provider, cx));
        }
        if let Some(notice) = &updates.registry_notice {
            rows.push(meta(notice.clone()).into_any_element());
        }
        for row in &updates.registry {
            let id = row.id().to_owned();
            let busy = updates.registry_busy.contains(&id);
            let control = if busy {
                meta("Installing…").into_any_element()
            } else if row.target().is_some() {
                let label = if matches!(row.state, registry::State::Repair(_)) {
                    "Repair"
                } else {
                    "Update"
                };
                web_button(
                    SharedString::from(format!("registry-update-{id}")),
                    WebButtonVariant::Outline,
                    WebButtonSize::Small,
                    ButtonState::Enabled,
                )
                .aria_label(format!("{label} {}", row.name()))
                .child(label)
                .map(|el| {
                    on_press_view(el, cx, move |ws, _, cx| {
                        ws.updates.update(cx, |u, cx| u.update_registry(&id, cx))
                    })
                })
                .into_any_element()
            } else {
                meta(registry_status(row)).into_any_element()
            };
            let description = updates
                .registry_errors
                .get(row.id())
                .cloned()
                .unwrap_or_else(|| {
                    if row.target().is_some() {
                        registry_status(row)
                    } else {
                        format!("ACP Registry · {}", row.version())
                    }
                });
            rows.push(self.settings_row(
                format!("desktop-settings-registry-update-{}", row.id()),
                row.name().to_owned(),
                description,
                control,
            ));
        }
        rows.extend(self.render_app_update_row(UpdateRow::Settings, cx));
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
            _ => return Vec::new(),
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
                                        .font_family(
                                            crate::theme::FontFace::Code
                                                .with_default(SETTINGS_CODE_FONT_FAMILY),
                                        )
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
                    _ => return meta("Manage updates in this provider's CLI.").into_any_element(),
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
                        .aria_label("Update now")
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
                        self.updates.read(cx).copied.is_shown(copy_id),
                        cx,
                    ))
                    .into_any_element()
            }
            (ProviderUpdateState::Outdated, None) => meta("Update by hand").into_any_element(),
            (ProviderUpdateState::UpToDate, _) => up_to_date_badge(),
            _ => div().into_any_element(),
        }
    }

    pub(super) fn render_app_update_row(
        &self,
        location: UpdateRow<'_>,
        cx: &Context<'_, Self>,
    ) -> Vec<AnyElement> {
        let version = match location {
            UpdateRow::About(version) => Some(version),
            _ => None,
        };
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
        let notes_url = app
            .as_ref()
            .filter(|app| app.state == AppUpdateState::Behind)
            .and_then(|app| app.release.as_ref())
            .and_then(ReleaseUpdate::notes_url);
        let control = match (app.as_ref().map(|app| app.state), &install) {
            (_, AppInstall::Ready { .. }) if self.updates.read(cx).combined.is_some() => {
                meta("Desktop update ready")
                    .id("desktop-combined-release-ready")
                    .role(Role::Status)
                    .into_any_element()
            }
            (_, AppInstall::Ready { .. }) => web_button(
                "desktop-settings-restart-update",
                WebButtonVariant::Primary,
                WebButtonSize::Small,
                ButtonState::Enabled,
            )
            .debug_selector(|| "desktop-settings-restart-update".to_string())
            .aria_label("Restart to update")
            .track_focus(&self.modal_focus.restart_row)
            .child(RESTART_CONFIRM_LABEL)
            .map(|el| {
                on_press_view(el, cx, |this, window, cx| {
                    this.open_restart_confirm(window, cx)
                })
            })
            .into_any_element(),
            (_, AppInstall::Staging { .. }) => meta(install.progress_label().unwrap_or_default())
                .id("desktop-settings-update-progress")
                .debug_selector(|| "desktop-settings-update-progress".into())
                .role(Role::Status)
                .text_color(theme::rambutan_300())
                .into_any_element(),
            (Some(AppUpdateState::Behind), _)
                if self.updates.read(cx).bundle.is_some()
                    && app.as_ref().and_then(|a| a.installable_release()).is_some() =>
            {
                let label = if matches!(install, AppInstall::Failed { .. }) {
                    "Retry download"
                } else {
                    "Download"
                };
                let button = web_button(
                    "desktop-settings-stage-update",
                    WebButtonVariant::Outline,
                    WebButtonSize::Small,
                    ButtonState::Enabled,
                )
                .debug_selector(|| "desktop-settings-stage-update".into())
                .aria_label(label)
                .child(label);
                on_press_view(button, cx, |this, _, cx| {
                    this.updates
                        .update(cx, |updates, cx| updates.stage_app_update(true, cx));
                })
                .into_any_element()
            }
            (Some(AppUpdateState::Behind), _) if release_url.is_some() => {
                let url = release_url.clone().unwrap_or_default();
                web_button(
                    "desktop-settings-release-download",
                    WebButtonVariant::Outline,
                    WebButtonSize::Small,
                    ButtonState::Enabled,
                )
                .debug_selector(|| "desktop-settings-release-download".to_string())
                .aria_label("Download update")
                .child("Download")
                .map(|el| on_press_view(el, cx, move |_, _, cx| cx.open_url(&url)))
                .into_any_element()
            }
            _ if version.is_some() => {
                let checking = self.updates.read(cx).checking;
                let label = if checking {
                    "Checking…"
                } else {
                    "Check for updates"
                };
                let button = web_button(
                    "desktop-about-check-update",
                    WebButtonVariant::Outline,
                    WebButtonSize::Small,
                    ButtonState::disabled_if(checking),
                )
                .debug_selector(|| "desktop-about-check-update".into())
                .aria_label(label)
                .child(label);
                on_press_view(button, cx, |this, _, cx| {
                    this.updates.update(cx, |updates, cx| updates.check_now(cx));
                })
                .into_any_element()
            }
            (Some(AppUpdateState::UpToDate), _) => up_to_date_badge(),
            _ => div().into_any_element(),
        };
        // "Release notes" opens this version's entry on bruhs.ai/changelog.
        let control = match notes_url {
            Some(url) => div()
                .flex()
                .flex_wrap()
                .max_w_full()
                .items_center()
                .gap(px(8.0))
                .child(
                    web_button(
                        "desktop-settings-release-notes",
                        WebButtonVariant::Ghost,
                        WebButtonSize::Small,
                        ButtonState::Enabled,
                    )
                    .debug_selector(|| "desktop-settings-release-notes".to_string())
                    .aria_label("Release notes")
                    .child("Release notes")
                    .map(|el| on_press_view(el, cx, move |_, _, cx| cx.open_url(&url))),
                )
                .child(control)
                .into_any_element(),
            None => control,
        };
        let control = if let Some(version) = version {
            div()
                .flex()
                .flex_wrap()
                .items_center()
                .gap_2()
                .child(control)
                .child(copy_button(
                    "desktop-settings-copy-version",
                    version.to_owned(),
                    "Copy version",
                    self.updates
                        .read(cx)
                        .copied
                        .is_shown("desktop-settings-copy-version"),
                    cx,
                ))
                .into_any_element()
        } else {
            control
        };
        let row = match location {
            UpdateRow::Confirmation => {
                self.stacked_settings_row("desktop-settings-row-update-app", "bruhs", line, control)
            }
            UpdateRow::Settings => {
                self.settings_row("desktop-settings-row-update-app", "bruhs", line, control)
            }
            UpdateRow::About(version) => self.settings_row_opts(
                "desktop-settings-about-version",
                format!("Version  {version}"),
                line,
                None,
                false,
                control,
            ),
        };
        let mut rows = vec![row];
        if self.updates.read(cx).translated && self.updates.read(cx).bundle.is_some() {
            let detail = match &install {
                AppInstall::Ready { .. } => "Restart to switch to the downloaded Apple silicon build.",
                AppInstall::Staging { .. } => "Preparing the native Apple silicon update.",
                _ if app.as_ref().is_some_and(|app| app.state == AppUpdateState::Behind) =>
                    "Update to run natively on this Mac.",
                _ => "The next desktop update will switch this Mac to the native Apple silicon build.",
            };
            rows.push(
                div()
                    .id("desktop-update-architecture")
                    .debug_selector(|| "desktop-update-architecture".into())
                    .role(Role::Status)
                    .px(px(16.0))
                    .py_2()
                    .child(
                        div()
                            .text_size(px(SETTINGS_ROW_TITLE_TEXT_SIZE_PX))
                            .text_color(theme::persimmon_200())
                            .child("Intel build on Apple silicon"),
                    )
                    .child(meta(detail))
                    .into_any_element(),
            );
        }
        if let AppInstall::Failed { error, .. }
        | AppInstall::Ready {
            restart_error: Some(error),
            ..
        } = &install
        {
            rows.push(
                div()
                    .px(px(16.0))
                    .pb(px(12.0))
                    .text_size(px(SETTINGS_META_TEXT_SIZE_PX))
                    .text_color(theme::lychee_200())
                    .id("desktop-settings-update-error")
                    .debug_selector(|| "desktop-settings-update-error".into())
                    .role(Role::Alert)
                    .child(error.clone())
                    .when_some(
                        release_url.filter(|_| matches!(install, AppInstall::Failed { .. })),
                        |el, url| {
                            let button = web_button(
                                "desktop-settings-manual-download",
                                WebButtonVariant::Ghost,
                                WebButtonSize::Small,
                                ButtonState::Enabled,
                            )
                            .aria_label("Open release page")
                            .child("Open release page");
                            el.child(on_press_view(button, cx, move |_, _, cx| cx.open_url(&url)))
                        },
                    )
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
                                    .font_family(crate::theme::FontFace::Code.with_default(SETTINGS_CODE_FONT_FAMILY))
                                    .text_color(theme::rambutan_200())
                                    .child(command.clone()),
                            )
                            .child(copy_button(
                                "desktop-settings-update-copy",
                                command,
                                "Copy update command",
                                self.updates
                                    .read(cx)
                                    .copied
                                    .is_shown("desktop-settings-update-copy"),
                                cx,
                            )),
                    )
                    .into_any_element(),
            );
        }
        vec![div().flex().flex_col().children(rows).into_any_element()]
    }
}

pub(super) fn copy_button(
    id: &'static str,
    text: String,
    label: &'static str,
    copied: bool,
    cx: &Context<'_, Workspace>,
) -> Stateful<Div> {
    web_button(
        id,
        WebButtonVariant::Ghost,
        WebButtonSize::IconSmall,
        ButtonState::Enabled,
    )
    // A row action, like the web's copy button (`size-control-xs`).
    .min_w(px(theme::CONTROL_XS_PX))
    .min_h(px(theme::CONTROL_XS_PX))
    .size(px(theme::CONTROL_XS_PX))
    .debug_selector(move || id.to_string())
    .aria_label(label)
    .tooltip(move |window, cx| Tooltip::new(label).build(window, cx))
    // Web: a green check for a moment after copying.
    .child(if copied {
        icon_svg(assets::CHECK_ICON_PATH, theme::ICON_PX, theme::pandan_400())
    } else {
        icon_svg(
            assets::COPY_ICON_PATH,
            theme::ICON_PX,
            theme::rambutan_500(),
        )
    })
    .on_click(cx.listener(move |this, _, _, cx| {
        cx.write_to_clipboard(gpui::ClipboardItem::new_string(text.clone()));
        this.updates.update(cx, |updates, cx| {
            crate::views::copy_feedback::CopyFeedback::show(
                updates,
                |updates| &mut updates.copied,
                id,
                cx,
            );
        });
    }))
}

fn confirm_provider_row(name: impl Into<SharedString>, version: String, detail: String) -> Div {
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
                .child(div().min_w_0().truncate().child(name.into()))
                .child(meta(version)),
        )
        .child(
            div()
                .truncate()
                .font_family(crate::theme::FontFace::Code.with_default(SETTINGS_CODE_FONT_FAMILY))
                .text_size(px(SETTINGS_META_TEXT_SIZE_PX))
                .text_color(theme::rambutan_500())
                .child(detail),
        )
}

fn registry_status(row: &registry::Installed) -> String {
    match &row.state {
        registry::State::Update(agent) => format!("{} → {}", row.version(), agent.version),
        registry::State::Repair(_) => {
            format!("{} · Launcher missing or unavailable", row.version())
        }
        registry::State::Current => "Up to date".into(),
        registry::State::Installed => "Installed".into(),
        registry::State::Unavailable => "Unavailable on this device".into(),
        registry::State::Ahead => "Ahead of registry".into(),
        registry::State::UnknownVersion => "Versions could not be compared".into(),
    }
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

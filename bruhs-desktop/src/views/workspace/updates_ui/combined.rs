//! A confirmed local desktop release and its provider installs. Each provider
//! result belongs to the exact run that this confirmation started.
use super::super::linked_keybindings::PeerUpdate;
use super::*;
use sha2::{Digest, Sha256};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

const RESTART_SECONDS: u8 = 5;

#[derive(Clone, PartialEq, Eq)]
pub(super) struct Targets {
    pub(super) version: Option<String>,
    pub(super) peers: Vec<PeerUpdate>,
    providers: Vec<(AgentProvider, String)>,
    registry: Vec<(String, String)>,
}

impl Targets {
    pub(super) fn machine_count(&self) -> usize {
        self.peers
            .iter()
            .map(|p| p.host)
            .collect::<HashSet<_>>()
            .len()
            + usize::from(
                self.version.is_some() || !self.providers.is_empty() || !self.registry.is_empty(),
            )
    }

    pub(super) fn render_summary(&self, registry: &[&registry::Installed]) -> AnyElement {
        let providers = self
            .providers
            .iter()
            .map(|(p, _)| provider_cli_label(*p))
            .chain(registry.iter().map(|r| r.name()))
            .collect::<Vec<_>>();
        let count = providers.len() + self.peers.len();
        let mut machines = std::collections::BTreeMap::new();
        for plan in &self.peers {
            let (_, names) = machines
                .entry(plan.host)
                .or_insert_with(|| (plan.machine.clone(), Vec::new()));
            names.push(provider_cli_label(plan.provider));
        }
        let include_local = self.version.is_some() || !providers.is_empty();
        let noun = if count == 1 {
            "provider update"
        } else {
            "provider updates"
        };
        let row = |id: SharedString, header: bool, labels: [String; 3]| {
            let role = if header {
                Role::ColumnHeader
            } else {
                Role::Cell
            };
            div()
                .id(id.clone())
                .role(Role::Row)
                .flex()
                .gap_3()
                .py_1()
                .when(header, |el| {
                    el.border_b_1()
                        .border_color(theme::ring_panel())
                        .text_color(theme::rambutan_400())
                })
                .children(labels.into_iter().enumerate().map(move |(i, label)| {
                    div()
                        .id((id.clone(), i))
                        .role(role)
                        .min_w_0()
                        .when(i < 2, |cell| cell.w(px(86.)).flex_shrink_0())
                        .when(i == 2, |cell| cell.flex_1())
                        .child(label)
                }))
        };
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(meta(match &self.version {
                Some(v) => format!("Bruhs v{v} · {count} {noun}"),
                None => format!("{count} {noun}"),
            }))
            .child(
                div()
                    .id("combined-update-table")
                    .debug_selector(|| "desktop-combined-update-table".into())
                    .role(Role::Table)
                    .aria_label("Updates by machine")
                    .text_size(px(SETTINGS_META_TEXT_SIZE_PX))
                    .child(row(
                        "combined-update-headings".into(),
                        true,
                        ["Machine".into(), "Desktop app".into(), "Providers".into()],
                    ))
                    .when(include_local, |table| {
                        table.child(row(
                            "combined-update-machine".into(),
                            false,
                            [
                                if cfg!(target_os = "macos") {
                                    "This Mac".into()
                                } else {
                                    "This computer".into()
                                },
                                if self.version.is_some() {
                                    "Bruhs".into()
                                } else {
                                    "—".into()
                                },
                                providers.join(", "),
                            ],
                        ))
                    })
                    .children(machines.into_iter().map(|(id, (name, providers))| {
                        row(
                            format!("peer-update-machine-{id}").into(),
                            false,
                            [name, "—".into(), providers.join(", ")],
                        )
                    })),
            )
            .into_any_element()
    }
}

pub(super) struct CombinedRun {
    version: Option<String>,
    peers: HashSet<(uuid::Uuid, AgentProvider)>,
    providers: HashMap<AgentProvider, Option<u64>>,
    registry: HashSet<String>,
    total: usize,
    failures: Vec<String>,
    seconds: Option<u8>,
}

impl CombinedRun {
    fn new(targets: &Targets) -> Self {
        Self {
            version: targets.version.clone(),
            peers: targets.peers.iter().map(|p| (p.host, p.provider)).collect(),
            providers: targets.providers.iter().map(|(p, _)| (*p, None)).collect(),
            registry: targets.registry.iter().map(|(id, _)| id.clone()).collect(),
            total: targets.providers.len() + targets.registry.len() + targets.peers.len(),
            failures: Vec::new(),
            seconds: None,
        }
    }

    pub(in crate::views::workspace) fn peer_finished(
        &mut self,
        key: (uuid::Uuid, AgentProvider),
        machine: &str,
        result: &Result<(), String>,
    ) {
        if self.peers.remove(&key) {
            if let Err(message) = result {
                self.failures.push(format!(
                    "{} on {}: {message}",
                    provider_cli_label(key.1),
                    machine
                ));
            }
        }
    }

    pub(super) fn provider_started(
        &mut self,
        provider: AgentProvider,
        result: Result<u64, String>,
    ) {
        if let Some(slot) = self.providers.get_mut(&provider) {
            match result {
                Ok(id) => *slot = Some(id),
                Err(message) => {
                    self.failures
                        .push(format!("{}: {message}", provider_cli_label(provider)));
                    self.providers.remove(&provider);
                }
            }
        }
    }

    pub(super) fn registry_finished(&mut self, id: &str, error: Option<&str>) {
        if self.registry.remove(id) {
            if let Some(message) = error {
                self.failures.push(format!("{id}: {message}"));
            }
        }
    }

    fn completed(&self) -> usize {
        self.total - self.providers.len() - self.registry.len() - self.peers.len()
    }

    fn tick(&mut self, install: &AppInstall, runs: &[ProviderRun]) -> Option<CombinedEvent> {
        self.providers.retain(|provider, id| {
            let Some(id) = id else { return true };
            match runs
                .iter()
                .find(|run| run.id == *id && run.provider == *provider)
            {
                Some(run) if run.is_active() => true,
                Some(run) => {
                    if run.needs_attention() {
                        self.failures.push(format!(
                            "{}: {}",
                            provider_cli_label(*provider),
                            run.phase_line()
                        ));
                    }
                    false
                }
                None => {
                    self.failures.push(format!(
                        "{}: update result unavailable",
                        provider_cli_label(*provider)
                    ));
                    false
                }
            }
        });
        if self.completed() < self.total {
            return None;
        }
        let Some(expected_version) = self.version.as_ref() else {
            return Some(CombinedEvent::Finished(
                self.summary(),
                !self.failures.is_empty(),
            ));
        };
        match install {
            AppInstall::Staging { version, .. } if version == expected_version => return None,
            AppInstall::Ready { version, .. } if version == expected_version => {}
            AppInstall::Failed { version, error, .. } if version == expected_version => {
                return Some(CombinedEvent::Finished(
                    format!("{}\nDesktop download: {error}", self.summary()),
                    true,
                ));
            }
            _ => {
                return Some(CombinedEvent::Finished(
                    "The desktop release changed. Review the available update before restarting."
                        .into(),
                    true,
                ));
            }
        }
        let seconds = self
            .seconds
            .map_or(RESTART_SECONDS, |s| s.saturating_sub(1));
        self.seconds = Some(seconds);
        (seconds == 0).then(|| {
            CombinedEvent::Restart(UpdateReceipt {
                version: expected_version.clone(),
                at: EpochMs::now(),
                summary: self.summary(),
                warning: !self.failures.is_empty(),
            })
        })
    }

    fn summary(&self) -> String {
        let failed = self.failures.len();
        let mut text = if failed == 0 {
            format!(
                "{} of {} provider updates finished.",
                self.completed(),
                self.total
            )
        } else {
            format!(
                "Provider updates completed. {} succeeded, {failed} failed.",
                self.completed() - failed
            )
        };
        for failure in &self.failures {
            text.push('\n');
            text.push_str(failure);
        }
        text
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
pub(in crate::views::workspace) struct UpdateReceipt {
    version: String,
    at: EpochMs,
    summary: String,
    warning: bool,
}

fn receipt_path(bundle: &Path) -> Option<PathBuf> {
    let bundle = std::fs::canonicalize(bundle).unwrap_or_else(|_| bundle.to_owned());
    let identity = Sha256::digest(bundle.as_os_str().as_encoded_bytes());
    crate::paths::desktop_state_file()
        .map(|state| state.with_added_extension(format!("update-{identity:x}.json")))
}

impl UpdateReceipt {
    fn save(&self, path: &Path) -> std::io::Result<()> {
        let parent = path.parent().expect("desktop state directory");
        std::fs::create_dir_all(parent)?;
        let mut file = tempfile::NamedTempFile::new_in(parent)?;
        serde_json::to_writer(&mut file, self)?;
        file.flush()?;
        file.persist(path)?;
        Ok(())
    }

    fn take(path: &Path, installed: &str) -> Option<(String, bool)> {
        if !path.is_file() {
            return None;
        }
        let claimed = tempfile::NamedTempFile::new_in(path.parent()?)
            .ok()?
            .into_temp_path();
        std::fs::rename(path, &claimed).ok()?;
        let data = std::fs::read(&claimed).ok()?;
        let receipt: Self = serde_json::from_slice(&data).ok()?;
        if !(0..=600_000).contains(&EpochMs::now().millis_since(receipt.at)) {
            return None;
        }
        if installed.trim_start_matches('v') == receipt.version {
            Some((
                format!("Bruhs updated to v{}. {}", receipt.version, receipt.summary),
                receipt.warning,
            ))
        } else {
            Some((
                format!(
                    "The update to v{} did not apply. {}",
                    receipt.version, receipt.summary
                ),
                true,
            ))
        }
    }
}

pub(in crate::views::workspace) enum CombinedEvent {
    Restart(UpdateReceipt),
    Finished(String, bool),
}

impl gpui::EventEmitter<CombinedEvent> for DesktopUpdates {}

impl DesktopUpdates {
    pub(in crate::views::workspace) fn peer_update_finished(
        &mut self,
        key: (uuid::Uuid, AgentProvider),
        machine: &str,
        result: &Result<(), String>,
        cx: &mut Context<'_, Self>,
    ) {
        if let Some(batch) = &mut self.combined {
            batch.peer_finished(key, machine, result);
            cx.notify();
        }
    }

    pub(super) fn combined_targets(&self, peers: Vec<PeerUpdate>) -> Option<Targets> {
        if self.combined.is_some() {
            return None;
        }
        let version = self.bundle.as_ref().and_then(|_| match &self.install {
            AppInstall::Ready { version, .. } | AppInstall::Staging { version, .. } => {
                Some(version.clone())
            }
            _ => self
                .report
                .as_ref()?
                .app
                .installable_release()?
                .latest_version
                .clone(),
        });
        if version.is_none() && peers.is_empty() {
            return None;
        }
        let report = self.report.as_ref();
        let providers: Vec<_> = report
            .map(UpdatesReport::updatable_providers)
            .unwrap_or_default()
            .into_iter()
            .filter(|p| !self.starting.contains(p))
            .filter_map(|p| Some((p, report?.provider(p)?.latest.clone()?)))
            .collect();
        let registry: Vec<_> = self
            .registry_targets()
            .filter_map(|r| Some((r.id().to_owned(), r.target()?.version.clone())))
            .collect();
        (!providers.is_empty() || !registry.is_empty() || !peers.is_empty()).then_some(Targets {
            peers,
            version,
            providers,
            registry,
        })
    }

    pub(super) fn start_combined(&mut self, targets: &Targets, cx: &mut Context<'_, Self>) -> bool {
        if self.combined_targets(targets.peers.clone()).as_ref() != Some(targets) {
            return false;
        }
        self.combined = Some(CombinedRun::new(targets));
        if targets.version.is_some()
            && matches!(self.install, AppInstall::Idle | AppInstall::Failed { .. })
        {
            self.stage_app_update(true, cx);
        }
        for (id, _) in &targets.registry {
            self.update_registry(id, cx);
        }
        for (provider, _) in &targets.providers {
            self.update(*provider, cx);
        }
        self.combined_timer = Some(cx.spawn(async move |this, cx| loop {
            cx.background_executor().timer(Duration::from_secs(1)).await;
            if !this
                .update(cx, |this, cx| {
                    let Some(batch) = &mut this.combined else {
                        return false;
                    };
                    if let Some(event) = batch.tick(&this.install, &this.checker.runs()) {
                        this.combined = None;
                        cx.emit(event);
                    }
                    cx.notify();
                    this.combined.is_some()
                })
                .unwrap_or(false)
            {
                break;
            }
        }));
        cx.notify();
        true
    }

    fn defer_combined_restart(&mut self, cx: &mut Context<'_, Self>) {
        if self.combined.as_ref().is_some_and(|r| r.seconds.is_some()) {
            let run = self.combined.take().unwrap();
            self.combined_timer = None;
            cx.emit(CombinedEvent::Finished(
                format!("Restart deferred. {}", run.summary()),
                !run.failures.is_empty(),
            ));
            cx.notify();
        }
    }
}

impl Workspace {
    pub(in crate::views::workspace) fn combined_update_event(
        &mut self,
        _: Entity<DesktopUpdates>,
        event: &CombinedEvent,
        cx: &mut Context<'_, Self>,
    ) {
        let (message, warning) = match event {
            CombinedEvent::Finished(message, warning) => (message.clone(), *warning),
            CombinedEvent::Restart(receipt) => {
                if !matches!(&self.updates.read(cx).install, AppInstall::Ready { version, .. } if version == &receipt.version)
                {
                    return;
                }
                let path = self
                    .updates
                    .read(cx)
                    .bundle
                    .as_deref()
                    .and_then(receipt_path);
                if let Some(path) = &path {
                    if let Err(error) = receipt.save(path) {
                        tracing::warn!("Could not save update result: {error}");
                    }
                }
                if self.restart_to_update(cx) {
                    return;
                }
                if let Some(path) = path {
                    let _ = std::fs::remove_file(path);
                }
                let detail = match &self.updates.read(cx).install {
                    AppInstall::Ready {
                        restart_error: Some(message),
                        ..
                    } => message.clone(),
                    _ => "Restart could not start.".into(),
                };
                (format!("{detail} {}", receipt.summary), true)
            }
        };
        self.meetings_hub
            .update(cx, |hub, cx| hub.show_update_notice(message, warning, cx));
    }

    pub(in crate::views::workspace) fn show_update_receipt(&self, cx: &mut Context<'_, Self>) {
        let installed = env!("BRUHS_RELEASE_VERSION");
        if installed.is_empty() {
            return;
        }
        if let Some((message, warning)) = self
            .updates
            .read(cx)
            .bundle
            .as_deref()
            .and_then(receipt_path)
            .and_then(|p| UpdateReceipt::take(&p, installed))
        {
            self.meetings_hub
                .update(cx, |hub, cx| hub.show_update_notice(message, warning, cx));
        }
    }

    pub(in crate::views::workspace) fn floating_update_progress(
        &self,
        cx: &Context<'_, Self>,
    ) -> Option<AnyElement> {
        let run = self.updates.read(cx).combined.as_ref()?;
        Some(
            div()
                .absolute()
                .bottom_3()
                .right_3()
                .w(px(240.0))
                .occlude()
                .bg(theme::longan_900())
                .rounded(px(8.0))
                .child(self.combined_sidebar(run, cx))
                .into_any_element(),
        )
    }

    pub(super) fn combined_sidebar(&self, run: &CombinedRun, cx: &Context<'_, Self>) -> AnyElement {
        let label = if let Some(seconds) = run.seconds {
            format!("Restart in {seconds}s")
        } else if run.completed() == run.total {
            self.updates
                .read(cx)
                .install
                .progress_label()
                .unwrap_or_else(|| "Preparing restart…".into())
        } else {
            format!("Providers · {} of {}", run.completed(), run.total)
        };
        let later = run.seconds.is_some();
        pill("desktop-sidebar-update", tone_info())
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .id("combined-update-status")
                    .role(Role::Status)
                    .debug_selector(|| "desktop-combined-update-status".into())
                    .child(label),
            )
            .when(later, |el| {
                let updates = self.updates.downgrade();
                el.child(on_press_view(
                    web_button(
                        "combined-update-later",
                        WebButtonVariant::Ghost,
                        WebButtonSize::Small,
                        ButtonState::Enabled,
                    )
                    .role(Role::Button)
                    .tab_index(0)
                    .debug_selector(|| "desktop-combined-update-later".into())
                    .aria_label("Restart later")
                    .on_a11y_action(gpui::AccessibleAction::Click, move |_, _, cx| {
                        let _ = updates.update(cx, |u, cx| u.defer_combined_restart(cx));
                    })
                    .child("Later"),
                    cx,
                    |ws, _, cx| {
                        ws.updates.update(cx, |u, cx| u.defer_combined_restart(cx));
                    },
                ))
            })
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::{AppInstall, CombinedEvent, CombinedRun, Targets, UpdateReceipt, RESTART_SECONDS};
    use bruhs_core::updates::release::StageProgress;
    use bruhs_models::{
        agent::AgentProvider,
        time::EpochMs,
        updates::{ProviderRun, RunOutcome, RunPhase},
    };

    fn batch() -> CombinedRun {
        CombinedRun::new(&Targets {
            version: Some("0.2.0".into()),
            peers: vec![],
            providers: vec![(AgentProvider::Codex, "1.0".into())],
            registry: vec![("fixture".into(), "1.0".into())],
        })
    }

    fn ready() -> AppInstall {
        AppInstall::Ready {
            version: "0.2.0".into(),
            staged: "fixture.app".into(),
            restart_error: None,
        }
    }

    fn installed() -> ProviderRun {
        ProviderRun {
            id: 42,
            provider: AgentProvider::Codex,
            phase: RunPhase::Done,
            started_at: EpochMs::now(),
            finished_at: Some(EpochMs::now()),
            command: None,
            last_line: None,
            from_version: None,
            to_version: None,
            outcome: Some(RunOutcome::Updated),
            message: None,
            output: String::new(),
        }
    }

    #[test]
    fn countdown_waits_for_each_selected_provider_and_the_exact_verified_release() {
        let mut r = batch();
        let mut provider = installed();
        assert!(r.tick(&ready(), std::slice::from_ref(&provider)).is_none());
        assert_eq!(r.completed(), 0);
        r.provider_started(AgentProvider::Codex, Ok(provider.id));
        r.registry_finished("another", None);
        provider.phase = RunPhase::Running;
        assert!(r.tick(&ready(), std::slice::from_ref(&provider)).is_none());
        assert_eq!(r.completed(), 0);
        provider.phase = RunPhase::Done;
        assert!(r.tick(&ready(), &[provider]).is_none());
        assert_eq!(r.completed(), 1);
        r.registry_finished("fixture", Some("offline"));
        let download = AppInstall::Staging {
            version: r.version.clone().unwrap(),
            progress: StageProgress::Verifying,
        };
        assert!(r.tick(&download, &[]).is_none());
        assert_eq!(r.seconds, None);
        for remaining in (1..=RESTART_SECONDS).rev() {
            assert!(r.tick(&ready(), &[]).is_none());
            assert_eq!(r.seconds, Some(remaining));
        }
        let Some(CombinedEvent::Restart(receipt)) = r.tick(&ready(), &[]) else {
            panic!("expected restart");
        };
        assert!(receipt.warning);
        assert!(receipt.summary.contains("1 succeeded, 1 failed."));
        assert!(receipt.summary.contains("fixture: offline"));
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("receipt");
        receipt.save(&path).unwrap();
        let (message, warning) = UpdateReceipt::take(&path, "v0.2.0").unwrap();
        assert!(warning && message.contains("updated to v0.2.0"));
        assert!(UpdateReceipt::take(&path, "v0.2.0").is_none());
        receipt.save(&path).unwrap();
        assert!(UpdateReceipt::take(&path, "0.1.0")
            .unwrap()
            .0
            .contains("did not apply"));
    }

    #[test]
    fn a_failed_download_or_replaced_release_finishes_without_a_restart() {
        let mut r = batch();
        r.provider_started(AgentProvider::Codex, Err("start failed".into()));
        r.registry_finished("fixture", None);
        let failed = AppInstall::Failed {
            version: "0.2.0".into(),
            error: "checksum mismatch".into(),
            at: std::time::Instant::now(),
        };
        let Some(CombinedEvent::Finished(message, true)) = r.tick(&failed, &[]) else {
            panic!("expected failure");
        };
        assert!(message.contains("checksum mismatch") && message.contains("start failed"));
        let another = AppInstall::Ready {
            version: "0.3.0".into(),
            staged: "other.app".into(),
            restart_error: None,
        };
        assert!(matches!(
            r.tick(&another, &[]),
            Some(CombinedEvent::Finished(_, true))
        ));
    }
}

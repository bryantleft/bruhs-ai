//! Update checks on the web: the report from `/api/updates`, the switch that
//! turns online checks off, and the provider update runs the sidebar and
//! Settings → Updates start. The server
//! checks and caches; this asks every five minutes, and every second while
//! an update runs.

use std::collections::HashSet;

use bruhs_models::agent::AgentProvider;
use bruhs_models::time::EpochMs;
use bruhs_models::updates::{
    ProviderRun, UpdatesReport, POLL_INTERVAL_MS, RUN_POLL_INTERVAL_MS, SUCCESS_VISIBLE_MS,
};
use leptos::prelude::*;

use crate::state::{load_pref, save_pref};

const ENABLED_KEY: &str = "bruhs.updateChecks";

#[derive(Clone, Copy)]
pub struct Updates {
    /// Online checks (registries, the remote) run only while this is on.
    /// This browser only.
    pub enabled: RwSignal<bool>,
    pub report: RwSignal<Option<UpdatesReport>>,
    pub checking: RwSignal<bool>,
    /// Why the last check failed.
    pub error: RwSignal<Option<String>>,
    /// Why the last update did not start (a 409, the server down). Kept apart
    /// from `error`, which every successful check clears.
    pub start_error: RwSignal<Option<String>>,
    /// Runs the user dismissed from the sidebar (T3 keeps these in memory).
    pub dismissed: RwSignal<HashSet<u64>>,
    /// The "Update N providers?" confirmation.
    pub confirm_open: RwSignal<bool>,
    /// Set to scroll Settings to the Updates section when it opens.
    pub reveal_in_settings: RwSignal<bool>,
    /// Runs that finished before this page loaded stay out of the sidebar.
    pub loaded_at: EpochMs,
}

impl Updates {
    /// Provide the context and start checking.
    pub fn provide() -> Self {
        let updates = Self {
            enabled: RwSignal::new(load_pref(ENABLED_KEY).as_deref() != Some("0")),
            report: RwSignal::new(None),
            checking: RwSignal::new(false),
            error: RwSignal::new(None),
            start_error: RwSignal::new(None),
            dismissed: RwSignal::new(HashSet::new()),
            confirm_open: RwSignal::new(false),
            reveal_in_settings: RwSignal::new(false),
            loaded_at: now(),
        };
        provide_context(updates);
        let enabled = updates.enabled;
        Effect::new(move |_| {
            let on = enabled.get();
            save_pref(ENABLED_KEY, if on { "1" } else { "0" });
            updates.check(false);
        });
        // The app root lives as long as the page, so both timers do too.
        gloo_timers::callback::Interval::new(POLL_INTERVAL_MS, move || updates.check(false))
            .forget();
        gloo_timers::callback::Interval::new(RUN_POLL_INTERVAL_MS, move || {
            let running = updates
                .report
                .with_untracked(|r| r.as_ref().is_some_and(UpdatesReport::any_running));
            if running {
                updates.check(false);
            }
        })
        .forget();
        updates
    }

    pub fn get() -> Self {
        expect_context::<Self>()
    }

    /// Ask the server; `force` checks now instead of reading its cache.
    pub fn check(self, force: bool) {
        if self.checking.get_untracked() {
            return;
        }
        self.checking.set(true);
        let online = self.enabled.get_untracked();
        wasm_bindgen_futures::spawn_local(async move {
            let mut stale = false;
            match crate::api::fetch_updates(force, online).await {
                Ok(report) if report.online == self.enabled.get_untracked() => {
                    self.report.set(Some(report));
                    self.error.set(None);
                }
                // The switch flipped while this ran: ask again for the side
                // it is on now.
                Ok(_) => stale = true,
                Err(error) => {
                    log::warn!("update check: {error}");
                    self.error
                        .set(Some(format!("The update check failed: {error}")));
                }
            }
            self.checking.set(false);
            if stale {
                self.check(false);
            }
        });
    }

    /// Start updating `provider` on the server.
    pub fn update(self, provider: AgentProvider) {
        wasm_bindgen_futures::spawn_local(async move {
            match crate::api::update_provider(provider).await {
                Ok(run) => {
                    self.start_error.set(None);
                    self.report.update(|report| {
                        if let Some(report) = report {
                            report.runs.retain(|r| r.id != run.id);
                            report.runs.push(run);
                        }
                    });
                }
                Err(error) => {
                    self.start_error.set(Some(error.to_string()));
                    self.check(false);
                }
            }
        });
    }

    /// Start every provider that can update here. The server queues the ones
    /// that share an installer.
    pub fn update_all(self) {
        let providers = self
            .report
            .get_untracked()
            .map(|report| report.updatable_providers())
            .unwrap_or_default();
        for provider in providers {
            self.update(provider);
        }
    }

    /// What the sidebar's provider pill shows: active runs first, then the
    /// newest run that finished since this page loaded and is not dismissed
    /// (a success only for its first three seconds).
    pub fn sidebar_runs(self, now: EpochMs) -> SidebarRuns {
        let Some(report) = self.report.get() else {
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
        let dismissed = self.dismissed.get();
        report
            .runs
            .iter()
            .filter(|r| !dismissed.contains(&r.id))
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

    pub fn dismiss(self, id: u64) {
        self.dismissed.update(|set| {
            set.insert(id);
        });
    }
}

/// The sidebar's provider pill.
#[derive(Clone, Debug, PartialEq)]
pub enum SidebarRuns {
    None,
    Active(Vec<ProviderRun>),
    Finished(ProviderRun),
}

/// The browser clock as a timestamp.
pub fn now() -> EpochMs {
    EpochMs::saturating_from_f64(js_sys::Date::now())
}

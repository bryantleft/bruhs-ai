//! App-wide awareness of what always-on agents told the owner, and the
//! reverse path: the owner's decisions on workflows made from agent drafts.
//!
//! The `reactive` frames of the live stream (`bruhs_models::api::live_events`)
//! carry an [`OutboxSignal`]. It drives the unread badge on the Agent workflows
//! navigation entry and a banner when a new unread update arrives while the
//! app is open. Items already unread when the page loads never raise a banner.
//!
//! Workflows created from a run's draft carry their origin. Whenever the
//! automation state changes, every settled decision the server has not yet
//! acknowledged is sent as `ReactiveCommand::DraftOutcome`; a failed send is
//! retried on the next change and never blocks the UI.

use bruhs_models::api::live_events::REACTIVE;
use bruhs_models::crew::BruhState;
use bruhs_models::reactive::{DraftOutcomeKind, OutboxItem, OutboxSignal, ReactiveCommand};
use bruhs_models::{DraftOutcomeReport, EpochMs, WorkflowStatus};
use leptos::prelude::*;

use crate::components::banner::{BannerAction, BannerKind, BannerStackSignal};
use crate::live_events::LiveEvents;
use crate::state::{AppView, AutomationStateSignal, SetActiveView};

/// How long a new-update banner stays before it dismisses itself.
const BANNER_TTL_MS: u32 = 15_000;

/// Unread agent updates, for badges outside the automations view, and the
/// crew as the service last reported it.
#[derive(Clone, Copy)]
pub struct AgentUpdates {
    pub unread: RwSignal<usize>,
    /// Every bruh's state from the latest `reactive` frame.
    pub bruhs: RwSignal<Vec<BruhState>>,
    /// The service's crew protocol; `None` until the first frame arrives.
    pub protocol: RwSignal<Option<u32>>,
}

impl AgentUpdates {
    /// Provide the context and start reporting draft decisions back.
    pub fn install() {
        provide_context(Self {
            unread: RwSignal::new(0),
            bruhs: RwSignal::new(Vec::new()),
            protocol: RwSignal::new(None),
        });
        if let Some(AutomationStateSignal(automation)) = use_context::<AutomationStateSignal>() {
            reconcile_draft_outcomes(automation);
        }
    }

    /// Accessible label for the navigation entry that carries the badge.
    pub fn nav_label(unread: usize) -> String {
        match unread {
            0 => "Agent workflows".into(),
            1 => "Agent workflows, 1 unread update".into(),
            count => format!("Agent workflows, {count} unread updates"),
        }
    }
}

/// Decides which `reactive` frame announces a new update. The first frame
/// seen is the baseline: whatever was unread at load stays quiet.
#[derive(Debug, Default)]
pub(crate) struct Announcer {
    started: bool,
    newest: Option<EpochMs>,
}

impl Announcer {
    pub(crate) fn observe(&mut self, signal: &OutboxSignal) -> Option<OutboxItem> {
        let latest = signal.latest_unread.as_ref();
        if !self.started {
            self.started = true;
            self.newest = latest.map(|item| item.created_at);
            return None;
        }
        let item = latest?;
        if self.newest.is_some_and(|newest| item.created_at <= newest) {
            return None;
        }
        self.newest = Some(item.created_at);
        Some(item.clone())
    }
}

/// Banner text for a new update.
pub(crate) fn banner_message(agent_name: Option<&str>, item: &OutboxItem) -> String {
    let name = agent_name
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .unwrap_or("Agent");
    format!("{name}: {}", item.title)
}

/// Follow the `reactive` frames: keep the unread count and announce new updates.
pub fn subscribe(live: &mut LiveEvents) {
    let Some(updates) = use_context::<AgentUpdates>() else {
        return;
    };
    let banners = use_context::<BannerStackSignal>();
    let set_view = use_context::<SetActiveView>().map(|set| set.0);
    let announcer = StoredValue::new(Announcer::default());
    live.on(REACTIVE, move |data| {
        let signal = match serde_json::from_str::<OutboxSignal>(&data) {
            Ok(signal) => signal,
            Err(error) => {
                log::warn!("reactive stream: could not read the update signal: {error}");
                return;
            }
        };
        updates.unread.set(signal.unread);
        if updates.bruhs.with_untracked(|bruhs| *bruhs != signal.bruhs) {
            updates.bruhs.set(signal.bruhs.clone());
        }
        if updates.protocol.get_untracked() != Some(signal.protocol) {
            updates.protocol.set(Some(signal.protocol));
        }
        let Some(item) = announcer
            .try_update_value(|announcer| announcer.observe(&signal))
            .flatten()
        else {
            return;
        };
        let Some(banners) = banners else {
            return;
        };
        wasm_bindgen_futures::spawn_local(async move {
            // The signal names the agent by id; a snapshot resolves its name.
            let name = crate::api::reactive_snapshot()
                .await
                .ok()
                .and_then(|snapshot| {
                    snapshot
                        .agents
                        .into_iter()
                        .find(|agent| agent.id == item.agent_id)
                        .map(|agent| agent.name)
                });
            let message = banner_message(name.as_deref(), &item);
            match set_view {
                Some(set_view) => banners.push_with_action(
                    BannerKind::Info,
                    message,
                    Some(BANNER_TTL_MS),
                    BannerAction {
                        label: "Open",
                        run: Callback::new(move |_| set_view.set(AppView::Automations)),
                    },
                ),
                None => banners.push(BannerKind::Info, message, Some(BANNER_TTL_MS)),
            }
        });
    });
}

/// The command that reports one draft decision, when the status is a decision.
pub(crate) fn draft_outcome_command(report: &DraftOutcomeReport) -> Option<ReactiveCommand> {
    Some(ReactiveCommand::DraftOutcome {
        run_id: report.run_id.clone(),
        workflow_id: report.workflow_id.clone(),
        outcome: DraftOutcomeKind::from_workflow(report.status)?,
        applied_steps: report.applied_steps,
        total_steps: report.total_steps,
    })
}

/// A rejection that retrying cannot fix: the run the draft came from is gone.
fn final_rejection(error: &crate::api::ApiError) -> bool {
    matches!(error, crate::api::ApiError::Http { status: 404, .. })
}

fn reconcile_draft_outcomes(
    automation: crate::durable_metadata::DurableMetadata<bruhs_models::AutomationState>,
) {
    // `WorkflowStatus` is not `Hash`; the set holds a handful of entries.
    let in_flight = StoredValue::new(Vec::<(String, WorkflowStatus)>::new());
    Effect::new(move |_| {
        // Tracks loading, transactions and save conflicts, so a report whose
        // acknowledgement could not be stored is retried once edits reopen.
        if !automation.editable() {
            return;
        }
        let reports = automation.with(|state| state.draft_outcome_reports());
        for report in reports {
            let key = (report.workflow_id.clone(), report.status);
            if in_flight.with_value(|sent| sent.contains(&key)) {
                continue;
            }
            let Some(command) = draft_outcome_command(&report) else {
                continue;
            };
            in_flight.update_value(|sent| {
                sent.push(key.clone());
            });
            wasm_bindgen_futures::spawn_local(async move {
                let result = crate::api::reactive_command(&command).await;
                let acknowledged = match &result {
                    Ok(_) => true,
                    Err(error) if final_rejection(error) => true,
                    Err(error) => {
                        log::warn!("draft outcome not reported yet: {error}");
                        false
                    }
                };
                if acknowledged {
                    automation.update(|state| {
                        state.mark_draft_outcome_reported(&key.0, key.1);
                    });
                }
                in_flight.update_value(|sent| {
                    sent.retain(|entry| *entry != key);
                });
            });
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use bruhs_models::reactive::{OutboxKind, SystemDelivery};

    fn item(id: &str, at: i64) -> OutboxItem {
        OutboxItem {
            id: id.into(),
            agent_id: "agent".into(),
            run_id: "run".into(),
            kind: OutboxKind::Notify,
            title: format!("Title {id}"),
            message: String::new(),
            created_at: EpochMs::saturating_from_i64(at),
            read_at: None,
            resolved_at: None,
            answer: None,
            system: SystemDelivery::NotRequested,
            approval: None,
        }
    }

    fn signal(latest: Option<OutboxItem>) -> OutboxSignal {
        OutboxSignal {
            unread: usize::from(latest.is_some()),
            open_questions: 0,
            latest_unread: latest,
            bruhs: Vec::new(),
            rooms: Vec::new(),
            protocol: 0,
        }
    }

    #[test]
    fn updates_present_at_load_stay_quiet_and_only_newer_ones_announce() {
        let mut announcer = Announcer::default();
        assert_eq!(announcer.observe(&signal(Some(item("old", 10)))), None);
        // The same item again, for example after a reconnect.
        assert_eq!(announcer.observe(&signal(Some(item("old", 10)))), None);
        let fresh = announcer.observe(&signal(Some(item("new", 20))));
        assert_eq!(fresh.map(|item| item.id), Some("new".to_string()));
        // Reading the newest makes an older unread item the latest again.
        assert_eq!(announcer.observe(&signal(Some(item("old", 10)))), None);
        assert_eq!(announcer.observe(&signal(None)), None);
    }

    #[test]
    fn an_empty_outbox_at_load_announces_the_first_update() {
        let mut announcer = Announcer::default();
        assert_eq!(announcer.observe(&signal(None)), None);
        assert!(announcer.observe(&signal(Some(item("a", 5)))).is_some());
    }

    #[test]
    fn banner_names_the_agent_or_falls_back() {
        let update = item("a", 1);
        assert_eq!(banner_message(Some("Bills"), &update), "Bills: Title a");
        assert_eq!(banner_message(Some("  "), &update), "Agent: Title a");
        assert_eq!(banner_message(None, &update), "Agent: Title a");
    }

    #[test]
    fn nav_label_counts_unread_updates() {
        assert_eq!(AgentUpdates::nav_label(0), "Agent workflows");
        assert_eq!(
            AgentUpdates::nav_label(1),
            "Agent workflows, 1 unread update"
        );
        assert_eq!(
            AgentUpdates::nav_label(4),
            "Agent workflows, 4 unread updates"
        );
    }

    #[test]
    fn draft_decisions_map_to_outcome_commands() {
        let report = |status| DraftOutcomeReport {
            workflow_id: "wf".into(),
            run_id: "run".into(),
            status,
            applied_steps: 2,
            total_steps: 3,
        };
        let wire = serde_json::to_value(draft_outcome_command(&report(WorkflowStatus::Completed)))
            .unwrap();
        assert_eq!(wire["action"], "draft_outcome");
        assert_eq!(wire["outcome"], "applied");
        assert_eq!(wire["run_id"], "run");
        assert_eq!(wire["workflow_id"], "wf");
        assert_eq!(wire["applied_steps"], 2);
        assert_eq!(wire["total_steps"], 3);
        assert!(matches!(
            draft_outcome_command(&report(WorkflowStatus::RolledBack)),
            Some(ReactiveCommand::DraftOutcome {
                outcome: DraftOutcomeKind::RolledBack,
                ..
            })
        ));
        assert!(draft_outcome_command(&report(WorkflowStatus::Previewed)).is_none());
    }

    #[test]
    fn only_a_missing_run_stops_retries() {
        assert!(final_rejection(&crate::api::ApiError::Http {
            status: 404,
            detail: None
        }));
        assert!(!final_rejection(&crate::api::ApiError::Http {
            status: 409,
            detail: None
        }));
        assert!(!final_rejection(&crate::api::ApiError::Network(
            "down".into()
        )));
    }
}

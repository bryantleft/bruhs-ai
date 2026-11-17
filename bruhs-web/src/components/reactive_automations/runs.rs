//! What a run card says about why a run exists and how it ended.
use bruhs_models::reactive::{ReactiveRun, RunOutcome, RunStatus};

pub(super) fn run_label(status: RunStatus) -> &'static str {
    match status {
        RunStatus::AwaitingReview => "Awaiting review",
        RunStatus::Queued => "Queued",
        RunStatus::Running => "Running",
        RunStatus::Completed => "Draft ready",
        RunStatus::Failed => "Failed",
        RunStatus::Interrupted => "Interrupted",
        RunStatus::Cancelled => "Cancelled",
        RunStatus::Merged => "Handled with another run",
    }
}

/// The first eight characters of a run id.
pub(super) fn short_id(id: &str) -> String {
    id.chars().take(8).collect()
}

pub(super) fn trigger_label(run: &ReactiveRun) -> &'static str {
    run.trigger
        .as_ref()
        .map_or("Event", |trigger| trigger.label())
}

/// "Notified" or "Asked"; a silent run shows no badge.
pub(super) fn outcome_badge(run: &ReactiveRun) -> Option<&'static str> {
    match run.report.as_ref()?.outcome {
        RunOutcome::Notify => Some("Notified"),
        RunOutcome::Ask => Some("Asked"),
        RunOutcome::Silent => None,
    }
}

/// Plain notes about batching and session continuity, in display order.
pub(super) fn run_notes(run: &ReactiveRun) -> Vec<String> {
    let mut notes = Vec::new();
    if let Some(into) = run.merged_into.as_deref() {
        notes.push(format!("Handled with run {}", short_id(into)));
    }
    if !run.merged_runs.is_empty() {
        notes.push(match run.merged_runs.len() {
            1 => "Handled 1 more item in this run".to_owned(),
            count => format!("Handled {count} more items in this run"),
        });
    }
    if run.fresh_session {
        notes.push("Started a new conversation; memory carried over.".into());
    }
    // A run handled with another one keeps an explanation, not a failure.
    if run.status == RunStatus::Merged {
        notes.extend(run.error.clone());
    }
    notes
}

/// The run's error, if it is one. Text on a merged run is informational
/// and appears among its notes instead.
pub(super) fn run_error(run: &ReactiveRun) -> Option<String> {
    run.error
        .clone()
        .filter(|_| run.status != RunStatus::Merged)
}

pub(super) fn report_warning(run: &ReactiveRun) -> Option<String> {
    run.report_warning
        .as_deref()
        .map(|warning| format!("Report note: {warning}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use bruhs_models::reactive::{RunReport, RunTrigger};
    use bruhs_models::EpochMs;

    fn run() -> ReactiveRun {
        ReactiveRun {
            id: "run-1".into(),
            agent_id: "agent".into(),
            source_id: None,
            subscription_id: None,
            event_id: None,
            status: RunStatus::Completed,
            prompt: String::new(),
            thread_id: bruhs_models::ids::ThreadId::new_v4(),
            session_id: None,
            created_at: EpochMs::UNIX_EPOCH,
            updated_at: EpochMs::UNIX_EPOCH,
            not_before: EpochMs::UNIX_EPOCH,
            result: None,
            error: None,
            trigger: None,
            merged_runs: Vec::new(),
            merged_into: None,
            report: None,
            report_warning: None,
            fresh_session: false,
            context_tokens: None,
            owner_message: None,
        }
    }

    #[test]
    fn runs_without_a_trigger_read_as_events() {
        let mut item = run();
        assert_eq!(trigger_label(&item), "Event");
        item.trigger = Some(RunTrigger::FollowUp {
            follow_up_id: "f".into(),
        });
        assert_eq!(trigger_label(&item), "Follow-up");
    }

    #[test]
    fn merged_runs_and_fresh_sessions_are_explained() {
        let mut item = run();
        assert!(run_notes(&item).is_empty());
        item.status = RunStatus::Merged;
        item.merged_into = Some("abcdef0123456789".into());
        item.merged_runs = vec!["a".into(), "b".into()];
        item.fresh_session = true;
        assert_eq!(run_label(item.status), "Handled with another run");
        assert_eq!(
            run_notes(&item),
            vec![
                "Handled with run abcdef01".to_string(),
                "Handled 2 more items in this run".to_string(),
                "Started a new conversation; memory carried over.".to_string(),
            ]
        );
    }

    #[test]
    fn a_merged_runs_explanation_is_a_note_and_real_errors_stay_errors() {
        let mut item = run();
        item.status = RunStatus::Merged;
        item.merged_into = Some("abcdef0123456789".into());
        item.error = Some("Handled together with an earlier queued item.".into());
        assert_eq!(run_error(&item), None);
        assert_eq!(
            run_notes(&item),
            vec![
                "Handled with run abcdef01".to_string(),
                "Handled together with an earlier queued item.".to_string(),
            ]
        );
        item.status = RunStatus::Failed;
        item.merged_into = None;
        item.error = Some("The provider exited.".into());
        assert_eq!(run_error(&item).as_deref(), Some("The provider exited."));
        assert!(run_notes(&item).is_empty());
    }

    #[test]
    fn outcome_badges_skip_silent_runs_and_warnings_are_labeled() {
        let mut item = run();
        assert_eq!(outcome_badge(&item), None);
        item.report = Some(RunReport::default());
        assert_eq!(outcome_badge(&item), None);
        item.report = Some(RunReport {
            outcome: RunOutcome::Ask,
            ..RunReport::default()
        });
        assert_eq!(outcome_badge(&item), Some("Asked"));
        item.report_warning = Some("Key Bad was rejected.".into());
        assert_eq!(
            report_warning(&item).as_deref(),
            Some("Report note: Key Bad was rejected.")
        );
    }
}

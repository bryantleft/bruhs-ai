//! What a run card says about why it ran and how it ended.
use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Tone {
    Muted,
    Warning,
}

/// The first eight characters of an id, as the web shows it.
pub(super) fn short_id(id: &str) -> &str {
    id.char_indices().nth(8).map_or(id, |(end, _)| &id[..end])
}

pub(super) fn trigger_label(run: &ReactiveRun) -> &'static str {
    run.trigger.as_ref().map_or("Event", RunTrigger::label)
}

pub(super) fn outcome_label(run: &ReactiveRun) -> Option<&'static str> {
    match run.report.as_ref()?.outcome {
        RunOutcome::Notify => Some("Notified"),
        RunOutcome::Ask => Some("Asked"),
        RunOutcome::Silent => None,
    }
}

/// Batching, session and report notes, in display order.
pub(super) fn run_notes(run: &ReactiveRun) -> Vec<(String, Tone)> {
    let mut notes = Vec::new();
    if let Some(into) = run.merged_into.as_deref() {
        notes.push((format!("Handled with run {}", short_id(into)), Tone::Muted));
    }
    match run.merged_runs.len() {
        0 => {}
        1 => notes.push(("Handled 1 more item in this run".into(), Tone::Muted)),
        count => notes.push((
            format!("Handled {count} more items in this run"),
            Tone::Muted,
        )),
    }
    if run.fresh_session {
        notes.push((
            "Started a new conversation; memory carried over.".into(),
            Tone::Muted,
        ));
    }
    // A run handled with another one keeps an explanation, not a failure.
    if run.status == RunStatus::Merged {
        notes.extend(run.error.clone().map(|text| (text, Tone::Muted)));
    }
    if let Some(warning) = &run.report_warning {
        notes.push((format!("Report note: {warning}"), Tone::Warning));
    }
    notes
}

/// The run's error, if it is one. Text on a merged run is informational
/// and appears among its notes instead.
pub(super) fn run_error(run: &ReactiveRun) -> Option<&str> {
    run.error
        .as_deref()
        .filter(|_| run.status != RunStatus::Merged)
}

pub(super) fn note((text, tone): (String, Tone)) -> Div {
    match tone {
        Tone::Muted => muted(text),
        Tone::Warning => div()
            .min_w_0()
            .text_size(px(12.0))
            .line_height(relative(1.5))
            .text_color(theme::durian_300())
            .child(text),
    }
}

#[cfg(test)]
mod tests {
    use super::{outcome_label, run_error, run_notes, short_id, trigger_label, Tone};
    use bruhs_models::{
        ids::ThreadId,
        reactive::{ReactiveRun, RunOutcome, RunReport, RunStatus, RunTrigger},
        EpochMs,
    };

    fn run() -> ReactiveRun {
        ReactiveRun {
            id: "run-1".into(),
            agent_id: "agent".into(),
            source_id: None,
            subscription_id: None,
            event_id: None,
            status: RunStatus::Completed,
            prompt: String::new(),
            thread_id: ThreadId::new_v4(),
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
    fn run_cards_explain_triggers_batches_sessions_and_report_outcomes() {
        let mut quiet = run();
        assert_eq!(trigger_label(&quiet), "Event");
        assert_eq!(outcome_label(&quiet), None);
        assert!(run_notes(&quiet).is_empty());
        quiet.report = Some(RunReport::default());
        assert_eq!(outcome_label(&quiet), None, "silent runs show no badge");

        let mut merged = run();
        merged.status = RunStatus::Merged;
        merged.merged_into = Some("abcdef0123456789".into());
        merged.trigger = Some(RunTrigger::FollowUp {
            follow_up_id: "f".into(),
        });
        assert_eq!(trigger_label(&merged), "Follow-up");
        assert_eq!(
            run_notes(&merged),
            vec![("Handled with run abcdef01".to_string(), Tone::Muted)]
        );
        // The merge explanation is a muted note, never an error.
        merged.error = Some("Handled together with an earlier queued item.".into());
        assert_eq!(run_error(&merged), None);
        assert_eq!(
            run_notes(&merged).last(),
            Some(&(
                "Handled together with an earlier queued item.".to_string(),
                Tone::Muted
            ))
        );
        let mut failed = run();
        failed.status = RunStatus::Failed;
        failed.error = Some("The provider exited.".into());
        assert_eq!(run_error(&failed), Some("The provider exited."));
        assert!(run_notes(&failed).is_empty());

        let mut batch = run();
        batch.merged_runs = vec!["a".into(), "b".into()];
        batch.fresh_session = true;
        batch.report_warning = Some("A follow-up key was invalid.".into());
        batch.report = Some(RunReport {
            outcome: RunOutcome::Ask,
            ..RunReport::default()
        });
        assert_eq!(outcome_label(&batch), Some("Asked"));
        assert_eq!(
            run_notes(&batch),
            vec![
                ("Handled 2 more items in this run".to_string(), Tone::Muted),
                (
                    "Started a new conversation; memory carried over.".to_string(),
                    Tone::Muted
                ),
                (
                    "Report note: A follow-up key was invalid.".to_string(),
                    Tone::Warning
                ),
            ]
        );
        assert_eq!(short_id("short"), "short");
    }
}

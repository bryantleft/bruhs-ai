//! The rules and copy a bruh's page shares on every surface: its job line,
//! how its chat pages runs and when the composer is off, how a decided
//! approval reads, and the access levels a model can run.

use super::TranscriptItem;
use super::{level_unavailable, AccessLevel, ApprovalState, BruhSettings, BruhStatus};
use crate::agent::AgentProvider;
use crate::reactive::{OutboxItem, ReactiveRun, ReactiveSnapshot, HISTORY_LIMIT};

/// The job line under a bruh's name: the first line of its instructions.
#[must_use]
pub fn job_line(instructions: &str) -> &str {
    instructions
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or_default()
}

/// Puts a page of runs into those already shown, replacing runs it updates.
pub fn merge_runs(shown: &mut Vec<ReactiveRun>, page: Vec<ReactiveRun>) {
    for run in page {
        match shown.iter_mut().find(|known| known.id == run.id) {
            Some(known) => *known = run,
            None => shown.push(run),
        }
    }
}

/// Whether a page this long may have older runs behind it.
#[must_use]
pub fn may_have_more(page_len: usize) -> bool {
    page_len >= HISTORY_LIMIT
}

/// Why the composer is off for a bruh in this state.
#[must_use]
pub fn composer_block(status: Option<BruhStatus>) -> Option<&'static str> {
    match status? {
        BruhStatus::Paused => Some("This bruh is paused. Resume it to chat."),
        BruhStatus::Off => Some("This bruh is off. Turn it on in Agent workflows to chat."),
        _ => None,
    }
}

/// How a decided approval ended; `None` while it still waits. An expired
/// approval stays open: a late approval still counts.
#[must_use]
pub fn approval_outcome(item: &OutboxItem) -> Option<&'static str> {
    item.resolved_at?;
    Some(
        match item.approval.as_ref().map(|approval| approval.state) {
            Some(ApprovalState::Approved) => "Approved",
            Some(ApprovalState::Denied) => "Denied",
            Some(ApprovalState::Expired) => "Expired",
            Some(ApprovalState::Waiting) | None => "Withdrawn",
        },
    )
}

/// What identifies a line as drawn: a line is redrawn only when this changes,
/// so a reply typed into a card survives unrelated updates, and a new last
/// line scrolls the conversation.
#[must_use]
pub fn line_key(item: &TranscriptItem) -> String {
    match item {
        TranscriptItem::Owner {
            run_id, pending, ..
        } => format!("owner:{run_id}:{pending}"),
        TranscriptItem::Bruh {
            run_id,
            text,
            failed,
            ..
        } => format!("bruh:{run_id}:{failed}:{text}"),
        TranscriptItem::Review { run_id, .. } => format!("review:{run_id}"),
        TranscriptItem::Card { item } => format!(
            "card:{}:{}:{:?}",
            item.id,
            item.resolved_at.is_some(),
            item.approval.as_ref().map(|approval| approval.state)
        ),
    }
}

/// What a bruh's chat follows, as rooms do (`room::room_follow_key`): the
/// newest run's line and the last line's key. Open cards come from the
/// crew and can stay last while the runs page loads above them; Load
/// earlier puts runs above and changes neither.
#[must_use]
pub fn chat_follow_key(lines: &[TranscriptItem]) -> (Option<String>, Option<String>) {
    let newest = lines
        .iter()
        .rev()
        .find(|line| !matches!(line, TranscriptItem::Card { .. }))
        .map(line_key);
    (newest, lines.last().map(line_key))
}

/// Each access level in display order, with why `model` can't run it.
#[must_use]
pub fn access_options(model: &str) -> Vec<(AccessLevel, Option<&'static str>)> {
    let provider = AgentProvider::for_model(model);
    [AccessLevel::ReadOnly, AccessLevel::Ask, AccessLevel::Full]
        .into_iter()
        .map(|level| (level, level_unavailable(level, provider)))
        .collect()
}

/// `level`, or Read only when `model` can't run it, which the service would
/// otherwise refuse.
#[must_use]
pub fn clamp_level(level: AccessLevel, model: &str) -> AccessLevel {
    if level_unavailable(level, AgentProvider::for_model(model)).is_some() {
        AccessLevel::ReadOnly
    } else {
        level
    }
}

/// The settings a bruh has now, from the snapshot.
#[must_use]
pub fn current_settings(snapshot: &ReactiveSnapshot, agent_id: &str) -> Option<BruhSettings> {
    let agent = snapshot.agents.iter().find(|agent| agent.id == agent_id)?;
    let state = snapshot
        .bruhs
        .iter()
        .find(|bruh| bruh.agent_id == agent_id)?;
    Some(BruhSettings {
        name: agent.name.clone(),
        instructions: agent.instructions.clone(),
        model: agent.model.clone(),
        effort: state.effort.clone(),
        look: state.look.clone(),
        access: state.access.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::{
        access_options, approval_outcome, chat_follow_key, clamp_level, composer_block,
        current_settings, job_line, line_key, may_have_more, merge_runs,
    };
    use crate::agent::AgentProvider;
    use crate::crew::{
        level_unavailable, AccessLevel, ApprovalState, BruhAccess, BruhLook, BruhState, BruhStatus,
        TranscriptItem,
    };
    use crate::reactive::{
        OutboxItem, ReactiveAgent, ReactiveRun, ReactiveSnapshot, RunStatus, HISTORY_LIMIT,
    };
    use crate::EpochMs;

    fn run(id: &str, status: &str) -> ReactiveRun {
        serde_json::from_value(serde_json::json!({
            "id": id, "agent_id": "a", "source_id": null, "subscription_id": null,
            "event_id": null, "status": status, "prompt": id, "thread_id": "t",
            "session_id": null, "created_at": 1, "updated_at": 1,
            "not_before": 1, "result": null, "error": null,
        }))
        .unwrap()
    }

    #[test]
    fn the_job_is_the_first_line_with_words() {
        assert_eq!(job_line("\n  Watch bills.\nPay nothing."), "Watch bills.");
        assert_eq!(job_line(""), "");
    }

    #[test]
    fn a_newer_page_updates_runs_in_place_and_adds_the_rest() {
        let mut shown = vec![run("r2", "running"), run("r1", "completed")];
        merge_runs(
            &mut shown,
            vec![run("r3", "queued"), run("r2", "completed")],
        );
        let got: Vec<(&str, RunStatus)> = shown
            .iter()
            .map(|run| (run.id.as_str(), run.status))
            .collect();
        assert_eq!(
            got,
            vec![
                ("r2", RunStatus::Completed),
                ("r1", RunStatus::Completed),
                ("r3", RunStatus::Queued),
            ]
        );
        assert!(may_have_more(HISTORY_LIMIT) && !may_have_more(HISTORY_LIMIT - 1));
    }

    #[test]
    fn the_composer_says_why_a_paused_or_off_bruh_cannot_chat() {
        assert!(composer_block(Some(BruhStatus::Paused))
            .is_some_and(|reason| reason.contains("paused")));
        assert!(composer_block(Some(BruhStatus::Off)).is_some_and(|reason| reason.contains("off")));
        for status in [
            BruhStatus::Asleep,
            BruhStatus::Working,
            BruhStatus::NeedsYou,
        ] {
            assert_eq!(composer_block(Some(status)), None);
        }
        assert_eq!(composer_block(None), None);
    }

    #[test]
    fn a_decided_approval_says_how_and_an_open_one_waits() {
        let mut item: OutboxItem = serde_json::from_value(serde_json::json!({
            "id": "i", "agent_id": "a", "run_id": "r", "kind": "ask", "title": "t",
            "message": "m", "created_at": 1,
            "approval": {"tool": "SandboxNetworkAccess", "summary": "Reach example.com", "fingerprint": "f", "state": "expired"},
        }))
        .unwrap();
        // Expired stays open: a late approval still counts.
        assert_eq!(approval_outcome(&item), None);
        item.resolved_at = Some(EpochMs::saturating_from_i64(2));
        assert_eq!(approval_outcome(&item), Some("Expired"));
        if let Some(approval) = item.approval.as_mut() {
            approval.state = ApprovalState::Waiting;
        }
        assert_eq!(approval_outcome(&item), Some("Withdrawn"));
        if let Some(approval) = item.approval.as_mut() {
            approval.state = ApprovalState::Approved;
        }
        assert_eq!(approval_outcome(&item), Some("Approved"));
        let open = line_key(&TranscriptItem::Card { item: item.clone() });
        item.resolved_at = None;
        assert_ne!(open, line_key(&TranscriptItem::Card { item }));
    }

    #[test]
    fn access_options_explain_what_this_model_cannot_do() {
        let codex_ask = level_unavailable(AccessLevel::Ask, AgentProvider::Codex);
        assert!(codex_ask.is_some());
        assert_eq!(
            access_options("gpt-5.1-codex"),
            vec![
                (AccessLevel::ReadOnly, None),
                (AccessLevel::Ask, codex_ask),
                (AccessLevel::Full, None),
            ]
        );
        assert_eq!(
            access_options("claude-sonnet-4-6"),
            vec![
                (AccessLevel::ReadOnly, None),
                (AccessLevel::Ask, None),
                (AccessLevel::Full, None),
            ]
        );
    }

    #[test]
    fn a_model_that_cannot_ask_drops_ask_to_read_only() {
        assert_eq!(
            clamp_level(AccessLevel::Ask, "gpt-5.1-codex"),
            AccessLevel::ReadOnly
        );
        assert_eq!(
            clamp_level(AccessLevel::Full, "gpt-5.1-codex"),
            AccessLevel::Full
        );
        assert_eq!(
            clamp_level(AccessLevel::Ask, "claude-sonnet-4-6"),
            AccessLevel::Ask
        );
    }

    #[test]
    fn current_settings_join_the_agent_and_its_bruh() {
        let agent = ReactiveAgent {
            id: "a".into(),
            name: "Ada".into(),
            instructions: "Watch bills.".into(),
            memory: String::new(),
            model: "claude-sonnet-4-6".into(),
            project_path: "/tmp".into(),
            enabled: true,
        };
        let state = BruhState {
            agent_id: "a".into(),
            name: "Ada".into(),
            look: BruhLook::default(),
            access: BruhAccess {
                level: AccessLevel::Full,
                send_as_me: true,
                can_deliver: false,
            },
            effort: Some("high".into()),
            status: BruhStatus::Asleep,
            next_wake_ms: None,
            unread: 0,
        };
        let mut snapshot = ReactiveSnapshot {
            agents: vec![agent],
            bruhs: vec![state],
            ..ReactiveSnapshot::default()
        };
        let settings = current_settings(&snapshot, "a").unwrap();
        assert_eq!(
            (
                settings.name.as_str(),
                settings.model.as_str(),
                settings.effort.as_deref(),
                settings.access.level,
            ),
            ("Ada", "claude-sonnet-4-6", Some("high"), AccessLevel::Full)
        );
        assert_eq!(current_settings(&snapshot, "b"), None);
        snapshot.bruhs.clear();
        assert_eq!(
            current_settings(&snapshot, "a"),
            None,
            "an agent with no bruh"
        );
    }

    #[test]
    fn a_chat_opened_under_an_open_card_follows_the_runs_that_load_above_it() {
        let mut card: OutboxItem = serde_json::from_value(serde_json::json!({
            "id": "q1", "agent_id": "a", "run_id": "r3", "kind": "ask",
            "title": "Which?", "message": "", "created_at": 9,
        }))
        .unwrap();
        card.resolved_at = None;
        let at = |id: &str, status: &str, created: i64| {
            let mut run = run(id, status);
            run.created_at = EpochMs::saturating_from_i64(created);
            run.updated_at = run.created_at;
            run.result = (status == "completed").then(|| format!("{id} done"));
            run.trigger = Some(crate::reactive::RunTrigger::Chat);
            run.owner_message = Some(format!("{id} asks"));
            run
        };
        let lines =
            |runs: &[ReactiveRun]| crate::crew::transcript("a", runs, std::slice::from_ref(&card));
        // Opened mid-run: only the open card until the runs page loads.
        let opened = chat_follow_key(&lines(&[]));
        let newest = [at("r3", "running", 8), at("r2", "completed", 5)];
        let loaded = chat_follow_key(&lines(&newest));
        assert_eq!(
            lines(&newest).last().map(line_key),
            lines(&[]).last().map(line_key),
            "the card stays last"
        );
        assert_ne!(opened, loaded);
        // Load earlier puts older runs above: nothing to follow.
        let mut shown = newest.to_vec();
        merge_runs(&mut shown, vec![at("r1", "completed", 2)]);
        assert_eq!(loaded, chat_follow_key(&lines(&shown)));
        assert_eq!(chat_follow_key(&[]), (None, None));
    }
}

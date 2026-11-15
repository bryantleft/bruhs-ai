//! The `bruhs-report` block a reactive agent appends to its answer.
//!
//! A run ends one of three ways: it tells the owner something, it stays
//! quiet, or it asks a question. The same block saves memory and schedules
//! follow-ups. The runtime applies it only when the run completes, in the
//! same transaction as the run's receipt.

use super::always_on::{valid_key, MemoryKind, MAX_MEMORY_ENTRY_CHARS};
use serde::{Deserialize, Serialize};

/// Info string of the fenced block.
pub const REPORT_FENCE: &str = "bruhs-report";
pub const MAX_TITLE_CHARS: usize = 120;
pub const MAX_MESSAGE_CHARS: usize = 4_000;
pub const MAX_REMEMBER: usize = 12;
pub const MAX_FOLLOW_UPS_PER_RUN: usize = 5;
pub const MAX_NOTE_CHARS: usize = 1_000;

/// System-prompt section that teaches the block. Stable text, so it caches.
pub const REPORT_PROTOCOL: &str = r#"End every answer with one fenced block whose info string is bruhs-report, containing one JSON object:
```bruhs-report
{"outcome":"silent","title":"","message":"","remember":[],"forget":[],"follow_ups":[],"cancel_follow_ups":[]}
```
- outcome: "notify" tells the owner something they would want to know now; "ask" asks the owner a question you need answered to continue (their answer comes back as your next message); "silent" records the result without interrupting anyone. Start quiet: notify only for things that need attention or that the owner asked to hear about, never for routine "nothing changed" results. Never repeat something listed under "Recently told the owner".
- title: at most 120 characters. message: what the owner reads, at most 4000 characters, plain and specific.
- remember: [{"key":"short-slug","kind":"note"|"progress","text":"..."}]. Save only what you cannot look up again: owner preferences and corrections (note), and where a multi-run task stands (progress: what is done, what remains). Keys are 1-64 of a-z 0-9 - _ . and reusing a key replaces that entry. Store pointers (IDs, paths, URLs), not copies. Your provider conversation can be reset at any time; anything not remembered is lost.
- forget: ["key"] removes entries that are stale or wrong.
- follow_ups: [{"key":"short-slug","in_minutes":120,"note":"what to check"}] or use "at" with an RFC 3339 time instead of in_minutes. You will be woken with the note at that time. Reusing a key reschedules it. Whenever your instructions say to check back, or someone promises something by a date or within a time, schedule a follow-up for just after that deadline; the follow-up is how you remember to check.
- cancel_follow_ups: ["key"] cancels pending follow-ups.
Leave out fields you do not need. Put the block after everything else, including any JSON workflow draft."#;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunOutcome {
    #[default]
    Silent,
    Notify,
    Ask,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MemoryWrite {
    pub key: String,
    #[serde(default = "note_kind")]
    pub kind: MemoryKind,
    pub text: String,
}

fn note_kind() -> MemoryKind {
    MemoryKind::Note
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FollowUpRequest {
    pub key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub in_minutes: Option<u64>,
    /// RFC 3339 time; parsed by the runtime.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<String>,
    pub note: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunReport {
    #[serde(default)]
    pub outcome: RunOutcome,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub title: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub message: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub remember: Vec<MemoryWrite>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub forget: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub follow_ups: Vec<FollowUpRequest>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cancel_follow_ups: Vec<String>,
}

/// An answer with its report block taken out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedAnswer {
    /// What the owner sees; a workflow draft, when present, is parsed from this.
    pub body: String,
    pub report: Option<RunReport>,
    /// Parts of the block that were rejected, in plain words.
    pub warnings: Vec<String>,
}

/// Split the final answer into its body and report. Never fails: a malformed
/// block becomes a warning and the run is treated as silent.
pub fn parse_answer(text: &str) -> ParsedAnswer {
    let opener = format!("```{REPORT_FENCE}");
    let Some(start) = text.rfind(&opener) else {
        return ParsedAnswer {
            body: text.to_owned(),
            report: None,
            warnings: Vec::new(),
        };
    };
    let body = text[..start].trim_end().to_owned();
    let rest = &text[start + opener.len()..];
    let Some(close) = rest.find("```") else {
        return ParsedAnswer {
            body,
            report: None,
            warnings: vec!["The report block was not closed; it was ignored.".into()],
        };
    };
    let mut warnings = Vec::new();
    if !rest[close + 3..].trim().is_empty() {
        warnings.push("Text after the report block was dropped.".into());
    }
    match serde_json::from_str::<RunReport>(rest[..close].trim()) {
        Ok(report) => {
            let report = sanitize(report, &mut warnings);
            ParsedAnswer {
                body,
                report: Some(report),
                warnings,
            }
        }
        Err(error) => {
            warnings.push(format!(
                "The report block was not valid ({error}); it was ignored."
            ));
            ParsedAnswer {
                body,
                report: None,
                warnings,
            }
        }
    }
}

fn bounded(text: &str, limit: usize) -> String {
    text.trim().chars().take(limit).collect()
}

fn sanitize(mut report: RunReport, warnings: &mut Vec<String>) -> RunReport {
    if report.title.chars().count() > MAX_TITLE_CHARS {
        warnings.push("The update title was shortened.".into());
    }
    report.title = bounded(&report.title, MAX_TITLE_CHARS);
    if report.message.chars().count() > MAX_MESSAGE_CHARS {
        warnings.push("The update message was shortened.".into());
    }
    report.message = bounded(&report.message, MAX_MESSAGE_CHARS);
    if report.outcome != RunOutcome::Silent && report.title.is_empty() && report.message.is_empty()
    {
        warnings.push("An update with no title or message was kept silent.".into());
        report.outcome = RunOutcome::Silent;
    }
    let before = report.remember.len();
    report.remember.retain(|write| {
        valid_key(&write.key)
            && !write.text.trim().is_empty()
            && write.text.chars().count() <= MAX_MEMORY_ENTRY_CHARS
    });
    if report.remember.len() != before {
        warnings.push(format!(
            "Memory entries need a valid key and 1 to {MAX_MEMORY_ENTRY_CHARS} characters; {} were skipped.",
            before - report.remember.len()
        ));
    }
    if report.remember.len() > MAX_REMEMBER {
        warnings.push(format!(
            "Only {MAX_REMEMBER} memory entries are saved per run."
        ));
        report.remember.truncate(MAX_REMEMBER);
    }
    report.forget.retain(|key| valid_key(key));
    report.cancel_follow_ups.retain(|key| valid_key(key));
    let before = report.follow_ups.len();
    report.follow_ups.retain(|request| {
        valid_key(&request.key)
            && !request.note.trim().is_empty()
            && request.note.chars().count() <= MAX_NOTE_CHARS
            && (request.in_minutes.is_some() != request.at.is_some())
    });
    if report.follow_ups.len() != before {
        warnings.push(format!(
            "Follow-ups need a valid key, a note, and exactly one of in_minutes or at; {} were skipped.",
            before - report.follow_ups.len()
        ));
    }
    if report.follow_ups.len() > MAX_FOLLOW_UPS_PER_RUN {
        warnings.push(format!(
            "Only {MAX_FOLLOW_UPS_PER_RUN} follow-ups are scheduled per run."
        ));
        report.follow_ups.truncate(MAX_FOLLOW_UPS_PER_RUN);
    }
    report
}

/// What a completed run tells the owner, given its body and report.
/// Without a report, a workflow draft asks for review and anything else stays quiet.
pub fn effective_outcome(body: &str, report: Option<&RunReport>) -> (RunOutcome, String, String) {
    if let Some(report) = report {
        if report.outcome != RunOutcome::Silent {
            let title = if report.title.is_empty() {
                report
                    .message
                    .lines()
                    .next()
                    .unwrap_or_default()
                    .chars()
                    .take(MAX_TITLE_CHARS)
                    .collect()
            } else {
                report.title.clone()
            };
            return (report.outcome, title, report.message.clone());
        }
        // A draft always needs a decision, even when the agent chose silence.
        if let Some(draft) = super::workflow_draft(body) {
            return (
                RunOutcome::Ask,
                "Review proposed changes".into(),
                draft.summary,
            );
        }
        return (RunOutcome::Silent, String::new(), String::new());
    }
    match super::workflow_draft(body) {
        Some(draft) => (
            RunOutcome::Ask,
            "Review proposed changes".into(),
            draft.summary,
        ),
        None => (RunOutcome::Silent, String::new(), String::new()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_is_split_from_the_body_and_bounded() {
        let text = "Found two invoices.\n\n```bruhs-report\n{\"outcome\":\"notify\",\"title\":\"Two invoices due\",\"message\":\"ACME 42 EUR due Friday.\",\"remember\":[{\"key\":\"acme-vendor\",\"kind\":\"note\",\"text\":\"ACME bills monthly\"},{\"key\":\"BAD KEY\",\"text\":\"x\"}],\"follow_ups\":[{\"key\":\"acme-due\",\"in_minutes\":60,\"note\":\"Check payment\"},{\"key\":\"both\",\"in_minutes\":1,\"at\":\"2026-01-01T00:00:00Z\",\"note\":\"x\"}]}\n```\n";
        let parsed = parse_answer(text);
        assert_eq!(parsed.body, "Found two invoices.");
        let report = parsed.report.unwrap();
        assert_eq!(report.outcome, RunOutcome::Notify);
        assert_eq!(report.remember.len(), 1);
        assert_eq!(report.follow_ups.len(), 1);
        assert_eq!(parsed.warnings.len(), 2);
        assert_eq!(
            effective_outcome(&parsed.body, Some(&report)).0,
            RunOutcome::Notify
        );
    }

    #[test]
    fn missing_or_malformed_reports_stay_quiet_but_drafts_still_ask() {
        let plain = parse_answer("Nothing new.");
        assert!(plain.report.is_none() && plain.warnings.is_empty());
        assert_eq!(effective_outcome(&plain.body, None).0, RunOutcome::Silent);

        let broken = parse_answer("Body\n```bruhs-report\n{not json}\n```");
        assert_eq!(broken.body, "Body");
        assert!(broken.report.is_none());
        assert_eq!(broken.warnings.len(), 1);

        let unclosed = parse_answer("Body\n```bruhs-report\n{\"outcome\":\"ask\"");
        assert_eq!(unclosed.body, "Body");
        assert!(unclosed.report.is_none());

        let draft =
            r#"{"summary":"Follow up","operations":[{"type":"create_task","title":"Reply"}]}"#;
        let parsed = parse_answer(&format!(
            "{draft}\n```bruhs-report\n{{\"outcome\":\"silent\"}}\n```"
        ));
        assert_eq!(parsed.body, draft);
        let (outcome, title, message) = effective_outcome(&parsed.body, parsed.report.as_ref());
        assert_eq!(outcome, RunOutcome::Ask);
        assert_eq!(title, "Review proposed changes");
        assert_eq!(message, "Follow up");
    }

    #[test]
    fn empty_updates_are_silenced_and_unknown_fields_rejected() {
        let parsed = parse_answer("x\n```bruhs-report\n{\"outcome\":\"notify\"}\n```");
        assert_eq!(parsed.report.unwrap().outcome, RunOutcome::Silent);
        let parsed =
            parse_answer("x\n```bruhs-report\n{\"outcome\":\"notify\",\"approve\":true}\n```");
        assert!(parsed.report.is_none());
    }
}

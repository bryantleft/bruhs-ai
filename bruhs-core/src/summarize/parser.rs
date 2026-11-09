//! Parse Claude's response into MeetingSummary. Tolerates markdown code
//! fences and stray prose around the JSON.

use std::collections::HashMap;

use bruhs_models::{MeetingSummary, ObservationKind, PersonObservation};

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ParseError {
    #[error("no JSON object found in response")]
    NoJson,
    #[error("JSON parse: {0}")]
    Serde(#[from] serde_json::Error),
}

/// The model's per-person observations break the contract, so none are
/// saved (rather than replacing existing ones with an empty set).
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("Invalid model observations; existing summary and observations were retained.")]
pub struct InvalidObservations;

/// Raw per-person observation as returned by Claude.
/// The `kind` field is a plain string so we can skip unknown values gracefully.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct PersonObservationRaw {
    pub speaker_label: String,
    pub kind: String,
    pub text: String,
}

/// Full response shape from Claude. Mirrors MeetingSummary but includes the
/// optional per_person_observations array. Using `serde(default)` on the
/// new field keeps older Claude responses (without the field) parseable.
#[derive(Debug, serde::Deserialize)]
struct SummaryResponse {
    pub tldr: String,
    pub action_items: Vec<bruhs_models::ActionItem>,
    pub decisions: Vec<String>,
    pub topics: Vec<bruhs_models::Topic>,
    #[serde(default)]
    pub per_person_observations: Vec<PersonObservationRaw>,
}

/// Convert raw observations from Claude into a map of `speaker_label →
/// Vec<PersonObservation>`. Unknown `kind` values and blank labels / text
/// are silently skipped (non-fatal — summary was already saved).
pub fn group_observations(
    raws: &[PersonObservationRaw],
    meeting_id: &str,
    now_ms: i64,
) -> HashMap<String, Vec<PersonObservation>> {
    let mut out: HashMap<String, Vec<PersonObservation>> = HashMap::new();
    for r in raws {
        if r.speaker_label.trim().is_empty() || r.text.trim().is_empty() {
            continue;
        }
        let kind = match r.kind.as_str() {
            "tone" => ObservationKind::Tone,
            "topic" => ObservationKind::Topic,
            "decision" => ObservationKind::Decision,
            "action_item_owner" => ObservationKind::ActionItemOwner,
            "quote" => ObservationKind::Quote,
            other => {
                tracing::debug!(
                    "group_observations: unknown kind {:?} for speaker {:?} — skipping",
                    other,
                    r.speaker_label,
                );
                continue;
            }
        };
        out.entry(r.speaker_label.trim().to_string())
            .or_default()
            .push(PersonObservation {
                at_meeting_id: meeting_id.to_string(),
                kind,
                text: r.text.trim().to_string(),
                created_at: now_ms,
            });
    }
    out
}

/// Replacement cannot silently turn invalid model observations into an empty deletion.
pub fn group_observations_strict(
    raws: &[PersonObservationRaw],
    meeting_id: &str,
    now_ms: i64,
) -> Result<HashMap<String, Vec<PersonObservation>>, InvalidObservations> {
    if raws.len() > 1024
        || raws.iter().any(|r| {
            r.speaker_label.trim().is_empty()
                || r.text.trim().is_empty()
                || r.text.len() > 8192
                || !matches!(
                    r.kind.as_str(),
                    "tone" | "topic" | "decision" | "action_item_owner" | "quote"
                )
        })
    {
        return Err(InvalidObservations);
    }
    Ok(group_observations(raws, meeting_id, now_ms))
}

/// Extract the first balanced `{...}` block from `text`. Returns None
/// if no `{` is present, or if the braces don't balance.
pub fn extract_json_block(text: &str) -> Option<&str> {
    let bytes = text.as_bytes();
    let start = bytes.iter().position(|&b| b == b'{')?;
    let mut depth: i32 = 0;
    let mut in_string = false;
    let mut escape = false;
    for (idx, &b) in bytes.iter().enumerate().skip(start) {
        if escape {
            escape = false;
            continue;
        }
        match b {
            b'\\' if in_string => escape = true,
            b'"' => in_string = !in_string,
            b'{' if !in_string => depth += 1,
            b'}' if !in_string => {
                depth -= 1;
                if depth == 0 {
                    return Some(&text[start..=idx]);
                }
            }
            _ => {}
        }
    }
    None
}

/// Build a MeetingSummary from the LLM response text. `generated_by` is
/// the model id used for the call (we fill it on the summary record);
/// `generated_at` is set by the caller from the wall clock.
///
/// Returns the summary alongside any raw per-person observations extracted
/// from the response. The observations field defaults to empty when absent
/// (back-compat with older prompt outputs that predate Phase 6).
pub fn parse_summary(
    response: &str,
    generated_by: String,
    generated_at: i64,
) -> Result<(MeetingSummary, Vec<PersonObservationRaw>), ParseError> {
    let json = extract_json_block(response).ok_or(ParseError::NoJson)?;
    let raw: SummaryResponse = serde_json::from_str(json).map_err(ParseError::Serde)?;
    let per_person_observations = raw.per_person_observations;
    let summary = MeetingSummary {
        tldr: raw.tldr,
        action_items: raw.action_items,
        decisions: raw.decisions,
        topics: raw.topics,
        generated_by,
        generated_at,
    };
    Ok((summary, per_person_observations))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_strips_markdown_fence() {
        let s = "```json\n{\"a\": 1}\n```";
        assert_eq!(extract_json_block(s), Some("{\"a\": 1}"));
    }

    #[test]
    fn extract_strips_leading_prose() {
        let s = "Sure, here's the JSON:\n{\"a\": 1}";
        assert_eq!(extract_json_block(s), Some("{\"a\": 1}"));
    }

    #[test]
    fn extract_handles_nested_braces() {
        let s = r#"{"outer": {"inner": 1}}"#;
        assert_eq!(extract_json_block(s), Some(s));
    }

    #[test]
    fn extract_ignores_braces_inside_strings() {
        let s = r#"{"text": "with } brace"}"#;
        assert_eq!(extract_json_block(s), Some(s));
    }

    #[test]
    fn extract_returns_none_when_unbalanced() {
        assert!(extract_json_block("{ no close").is_none());
        assert!(extract_json_block("no open }").is_none());
    }

    #[test]
    fn parse_summary_happy_path() {
        let resp = r#"```json
{
  "tldr": "Discussed roadmap",
  "action_items": [
    {"text": "File ticket", "assignee_person_id": null, "due_hint": "by Friday"}
  ],
  "decisions": ["Ship behind a flag"],
  "topics": [{"title": "Roadmap", "bullets": ["Q3 focus"]}]
}
```"#;
        let (s, obs) = parse_summary(resp, "claude-test".into(), 42).unwrap();
        assert_eq!(s.tldr, "Discussed roadmap");
        assert_eq!(s.action_items.len(), 1);
        assert_eq!(s.action_items[0].due_hint.as_deref(), Some("by Friday"));
        assert_eq!(s.decisions, vec!["Ship behind a flag"]);
        assert_eq!(s.topics[0].title, "Roadmap");
        assert_eq!(s.generated_by, "claude-test");
        assert_eq!(s.generated_at, 42);
        assert!(
            obs.is_empty(),
            "no per_person_observations in this response"
        );
    }

    #[test]
    fn parse_summary_with_observations() {
        let resp = r#"{
  "tldr": "Planning",
  "action_items": [],
  "decisions": [],
  "topics": [],
  "per_person_observations": [
    {"speaker_label": "Alice", "kind": "topic", "text": "Focused on pricing."}
  ]
}"#;
        let (s, obs) = parse_summary(resp, "m".into(), 0).unwrap();
        assert_eq!(s.tldr, "Planning");
        assert_eq!(obs.len(), 1);
        assert_eq!(obs[0].speaker_label, "Alice");
        assert_eq!(obs[0].kind, "topic");
    }

    #[test]
    fn parse_summary_rejects_no_json() {
        let err = parse_summary("no json here", "m".into(), 0).unwrap_err();
        assert!(matches!(err, ParseError::NoJson));
    }

    #[test]
    fn group_observations_buckets_by_speaker() {
        let raws = vec![
            PersonObservationRaw {
                speaker_label: "Alice".into(),
                kind: "topic".into(),
                text: "Pricing".into(),
            },
            PersonObservationRaw {
                speaker_label: "Alice".into(),
                kind: "decision".into(),
                text: "Ship Friday".into(),
            },
            PersonObservationRaw {
                speaker_label: "Bob".into(),
                kind: "tone".into(),
                text: "Skeptical".into(),
            },
        ];
        let grouped = group_observations(&raws, "m-1", 100);
        assert_eq!(grouped.len(), 2);
        assert_eq!(grouped.get("Alice").unwrap().len(), 2);
        assert_eq!(grouped.get("Bob").unwrap().len(), 1);
    }

    #[test]
    fn group_observations_skips_invalid() {
        let raws = vec![
            PersonObservationRaw {
                speaker_label: "".into(),
                kind: "topic".into(),
                text: "x".into(),
            },
            PersonObservationRaw {
                speaker_label: "Alice".into(),
                kind: "garbage".into(),
                text: "x".into(),
            },
            PersonObservationRaw {
                speaker_label: "Alice".into(),
                kind: "topic".into(),
                text: "  ".into(),
            },
        ];
        let grouped = group_observations(&raws, "m-1", 100);
        assert!(grouped.is_empty());
    }
}

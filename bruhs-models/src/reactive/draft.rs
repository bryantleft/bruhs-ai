use super::{MAX_PAYLOAD_BYTES, MAX_TEXT_CHARS};
use crate::WorkflowOperation;
use serde::{Deserialize, Serialize};

/// An agent answer may propose records. Parsing never approves or executes them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReactiveWorkflowDraft {
    pub summary: String,
    pub operations: Vec<WorkflowOperation>,
}

/// Only a complete JSON envelope becomes a proposal; prose and malformed
/// operations stay ordinary answer text. The existing workflow gates run next.
pub fn workflow_draft(text: &str) -> Option<ReactiveWorkflowDraft> {
    if text.len() > MAX_PAYLOAD_BYTES {
        return None;
    }
    let text = text.trim();
    let text = text
        .strip_prefix("```json")
        .or_else(|| text.strip_prefix("```"))
        .and_then(|text| text.trim().strip_suffix("```"))
        .unwrap_or(text)
        .trim();
    let draft: ReactiveWorkflowDraft = serde_json::from_str(text).ok()?;
    // Match the existing workflow preview's maximum operation count.
    if draft.operations.is_empty()
        || draft.operations.len() > 128
        || draft.summary.trim().is_empty()
        || draft.summary.chars().count() > MAX_TEXT_CHARS
        || !draft.operations.iter().all(WorkflowOperation::is_valid)
    {
        return None;
    }
    Some(draft)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn structured_drafts_preserve_exact_operations_for_existing_review() {
        let text = r#"{"summary":"Follow up","operations":[{"type":"create_task","title":"Reply to client"},{"type":"create_knowledge_note","title":"Meeting notes","body":"Decision: review the draft"}]}"#;
        let draft = workflow_draft(&format!("```json\n{text}\n```")).unwrap();
        assert_eq!(draft.operations.len(), 2);
        assert_eq!(
            draft.operations[0],
            WorkflowOperation::CreateTask {
                title: "Reply to client".into()
            }
        );
        assert_eq!(draft.summary, "Follow up");
    }

    #[test]
    fn invalid_extra_or_empty_operations_never_become_actionable() {
        for text in [
            r#"{"summary":"Bad","operations":[{"type":"create_task","title":""}]}"#,
            r#"{"summary":"Bad","operations":[{"type":"shell","command":"anything"}]}"#,
            r#"{"summary":"Bad","operations":[],"approved":true}"#,
            r#"{"summary":"Bad","operations":[]}"#,
            "Here is some prose and then {\"summary\":\"Bad\",\"operations\":[]}",
        ] {
            assert!(workflow_draft(text).is_none());
        }
    }
}

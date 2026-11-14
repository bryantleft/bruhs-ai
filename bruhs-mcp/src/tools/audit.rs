//! Preserve the audit JSON layout while keeping only one row's Value tree live.
use bruhs_models::AutomationState;
use serde::ser::{Error as _, SerializeMap};
use serde::{Serialize, Serializer};
use serde_json::json;

struct Canonical<'a, T>(&'a T);

impl<T: Serialize> Serialize for Canonical<'_, T> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serde_json::to_value(self.0)
            .map_err(S::Error::custom)?
            .serialize(serializer)
    }
}

struct Rows<'a, T>(&'a [T]);

impl<T: Serialize> Serialize for Rows<'_, T> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_seq(self.0.iter().map(Canonical))
    }
}

struct Audit<'a>(&'a AutomationState);

impl Serialize for Audit<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        // Use the same key order as the previous json! object, including when
        // serde_json's preserve_order feature is selected by a caller.
        let layout = json!({"permissions":null,"workflows":null,"jobs":null,"audit":null});
        let mut map = serializer.serialize_map(Some(4))?;
        for key in layout.as_object().expect("fixed audit object").keys() {
            match key.as_str() {
                "permissions" => map.serialize_entry(key, &Canonical(&self.0.permissions))?,
                "workflows" => map.serialize_entry(key, &Rows(&self.0.workflows))?,
                "jobs" => map.serialize_entry(key, &Rows(&self.0.jobs))?,
                "audit" => map.serialize_entry(key, &Rows(&self.0.audit))?,
                _ => unreachable!("fixed audit key"),
            }
        }
        map.end()
    }
}

pub(super) fn encode(
    state: &AutomationState,
    mut buffer: Vec<u8>,
) -> Result<String, serde_json::Error> {
    buffer.clear();
    let mut serializer = serde_json::Serializer::pretty(&mut buffer);
    Audit(state).serialize(&mut serializer)?;
    // SAFETY: serde_json's serializer emits only valid UTF-8, including escaped
    // string data. The cleared buffer contains only this completed JSON output.
    Ok(unsafe { String::from_utf8_unchecked(buffer) })
}

#[cfg(test)]
mod tests {
    use super::*;
    use bruhs_models::{AgentAccess, AgentDomain, WorkflowOperation};

    #[test]
    fn streamed_rows_preserve_the_complete_pretty_json_bytes() {
        let mut state = AutomationState::default();
        for allowed in [false, true] {
            state
                .permissions
                .set(AgentDomain::Knowledge, AgentAccess::Write, allowed);
            let id = state.create_workflow(
                "Audit ΟΣ Café 👩🏽‍💻",
                vec![WorkflowOperation::CreateKnowledgeNote {
                    title: "A \"quoted\" title".into(),
                    body: "line one\n\\ literal \"created_at_ms\": 7\tkept".repeat(64),
                }],
                123,
            );
            state.preview(&id, 456).expect("valid preview");
            let expected = serde_json::to_string_pretty(&json!({
                "permissions":state.permissions,"workflows":state.workflows,
                "jobs":state.jobs,"audit":state.audit,
            }))
            .expect("reference audit");
            assert_eq!(
                encode(&state, Vec::new()).expect("streamed audit"),
                expected
            );
        }
        assert_eq!(
            encode(&AutomationState::default(), Vec::new()).expect("empty audit"),
            serde_json::to_string_pretty(&json!({
                "permissions":AutomationState::default().permissions,
                "workflows":[],"jobs":[],"audit":[],
            }))
            .expect("empty reference"),
        );
    }

    #[test]
    fn reused_storage_encodes_current_state_without_old_response_bytes() {
        let mut state = AutomationState::default();
        let id = state.create_workflow(
            "Long prior response",
            vec![
                WorkflowOperation::CreateKnowledgeNote {
                    title: "Old title".into(),
                    body: "old private text ΟΣ Café 👩🏽‍💻".repeat(1024),
                };
                32
            ],
            1,
        );
        state.preview(&id, 2).unwrap();
        let prior = encode(&state, Vec::new()).unwrap();
        let allocation = prior.as_ptr();
        let capacity = prior.capacity();
        let state = AutomationState::default();
        let text = encode(&state, prior.into_bytes()).unwrap();
        assert_eq!(text.as_ptr(), allocation);
        assert_eq!(text.capacity(), capacity);
        assert_eq!(text, encode(&state, Vec::new()).unwrap());
        assert!(!text.contains("old private text"));
    }
}

//! Durable identity for one explicitly requested summary generation.
use super::MeetingSummary;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SummaryGenerationRequest {
    pub request_id: String,
    pub meeting_id: String,
    pub extract_observations: bool,
    pub replace_observations: bool,
}

impl SummaryGenerationRequest {
    pub fn new(meeting_id: String, extract_observations: bool, replace_observations: bool) -> Self {
        Self {
            request_id: uuid::Uuid::new_v4().to_string(),
            meeting_id,
            extract_observations,
            replace_observations,
        }
    }
    pub fn valid(&self) -> bool {
        [&self.request_id, &self.meeting_id]
            .into_iter()
            .all(|id| !id.trim().is_empty() && id.len() <= 128)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum SummaryGenerationState {
    Pending,
    Succeeded { summary: MeetingSummary },
    Failed,
    Cancelled,
}

impl<'de> Deserialize<'de> for SummaryGenerationState {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        // Empty struct variants reject extra fields; tagged unit variants silently
        // accept them even with deny_unknown_fields on the enclosing enum.
        #[derive(Deserialize)]
        #[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
        enum Wire {
            Pending {},
            Succeeded { summary: MeetingSummary },
            Failed {},
            Cancelled {},
        }
        Ok(match Wire::deserialize(deserializer)? {
            Wire::Pending {} => Self::Pending,
            Wire::Succeeded { summary } => Self::Succeeded { summary },
            Wire::Failed {} => Self::Failed,
            Wire::Cancelled {} => Self::Cancelled,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SummaryGenerationReceipt {
    pub request: SummaryGenerationRequest,
    pub state: SummaryGenerationState,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_and_pending_states_roundtrip_without_accepting_extra_fields() {
        for state in [
            SummaryGenerationState::Pending,
            SummaryGenerationState::Failed,
            SummaryGenerationState::Cancelled,
        ] {
            let mut encoded = serde_json::to_value(&state).unwrap();
            assert_eq!(
                serde_json::from_value::<SummaryGenerationState>(encoded.clone()).unwrap(),
                state
            );
            encoded["unexpected"] = true.into();
            assert!(serde_json::from_value::<SummaryGenerationState>(encoded).is_err());
        }
    }

    #[test]
    fn successful_receipt_requires_its_summary_and_known_fields() {
        for encoded in [
            serde_json::json!({"status":"succeeded"}),
            serde_json::json!({"status":"unknown"}),
            serde_json::json!({"status":"pending", "summary":null}),
        ] {
            assert!(serde_json::from_value::<SummaryGenerationState>(encoded).is_err());
        }
    }
}

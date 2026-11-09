use super::*;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Expected {
    pub summary: Option<bruhs_models::MeetingSummary>,
    pub skip: Option<bool>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Checkpoint {
    pub token: String,
    pub minimum_version: i64,
    pub expected: Option<Box<Expected>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generation: Option<bruhs_models::SummaryGenerationRequest>,
}

/// Saved uncertainty, never an instruction to replay a provider mutation.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "BTreeMap<String, Checkpoint>")]
pub struct ActionCheckpoints(BTreeMap<String, Checkpoint>);

impl TryFrom<BTreeMap<String, Checkpoint>> for ActionCheckpoints {
    type Error = &'static str;
    fn try_from(records: BTreeMap<String, Checkpoint>) -> Result<Self, Self::Error> {
        if records.iter().any(|(id, record)| {
            id.trim().is_empty()
                || id.len() > 128
                || uuid::Uuid::parse_str(&record.token).is_err()
                || record.generation.as_ref().is_some_and(|request| {
                    !request.valid()
                        || request.meeting_id != *id
                        || request.request_id != record.token
                        || (request.replace_observations && !request.extract_observations)
                })
        }) {
            return Err("Saved meeting request contains an invalid identity.");
        }
        Ok(Self(records))
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Action {
    Generate { extract: bool, replace: bool },
    SetSkip(bool),
    LoadLatest,
    CancelGeneration,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Phase {
    Queued(Action),
    Running,
    Recovery,
    AwaitingSave,
    Complete,
    /// Read-only server check; never persisted as saved uncertainty.
    Reading,
}

pub(in crate::views::meetings_hub) struct Entry {
    pub(super) checkpoint: Checkpoint,
    pub(super) phase: Phase,
    pub(super) message: String,
    /// Discovered server-side request that this device never saved.
    pub(super) foreign: Option<bruhs_models::SummaryGenerationRequest>,
}

pub struct MeetingActionStatus {
    pub busy: bool,
    pub recovery: bool,
    /// A saved generation request exists and can be fenced explicitly.
    pub cancellable: bool,
    /// An unresolved request from another client was discovered on the server.
    pub foreign_pending: bool,
    pub message: String,
}

#[derive(Clone)]
pub struct ActionSaveReceipt {
    pub(super) id: String,
    pub(super) token: String,
    pub(super) phase: Phase,
}

impl MeetingsHub {
    pub fn action_checkpoints(&self) -> ActionCheckpoints {
        ActionCheckpoints(
            self.meeting_actions
                .iter()
                .filter(|(_, entry)| {
                    !matches!(
                        entry.phase,
                        Phase::Complete | Phase::AwaitingSave | Phase::Reading
                    )
                })
                .map(|(id, entry)| (id.clone(), entry.checkpoint.clone()))
                .collect(),
        )
    }

    pub fn restore_action_checkpoints(&mut self, saved: ActionCheckpoints) {
        self.meeting_actions = saved.0.into_iter().map(|(id, checkpoint)| (id, Entry {
            checkpoint,
            phase: Phase::Recovery,
            message: "A previous meeting request may have finished. Load the latest saved meeting; generation will not restart automatically.".into(),
            foreign: None,
        })).collect();
    }

    pub fn action_save_receipts(&self) -> Vec<ActionSaveReceipt> {
        self.meeting_actions
            .iter()
            .filter(|(_, entry)| matches!(entry.phase, Phase::Queued(_) | Phase::AwaitingSave))
            .map(|(id, entry)| ActionSaveReceipt {
                id: id.clone(),
                token: entry.checkpoint.token.clone(),
                phase: entry.phase,
            })
            .collect()
    }

    pub fn meeting_action_status(&self, id: &str) -> Option<MeetingActionStatus> {
        self.meeting_actions
            .get(id)
            .map(|entry| MeetingActionStatus {
                busy: matches!(
                    entry.phase,
                    Phase::Queued(_) | Phase::Running | Phase::AwaitingSave | Phase::Reading
                ),
                recovery: entry.phase == Phase::Recovery,
                cancellable: entry.phase == Phase::Recovery
                    && entry.checkpoint.generation.is_some(),
                foreign_pending: entry.phase != Phase::Reading && entry.foreign.is_some(),
                message: entry.message.clone(),
            })
    }

    pub fn meeting_action_blocked(&self, id: &str) -> bool {
        self.meeting_actions
            .get(id)
            .is_some_and(|entry| entry.phase != Phase::Complete)
    }

    pub fn meeting_actions_saved(
        &mut self,
        receipts: Vec<ActionSaveReceipt>,
        cx: &mut Context<'_, Self>,
    ) {
        for receipt in receipts {
            let Some(entry) = self.meeting_actions.get_mut(&receipt.id) else {
                continue;
            };
            if entry.checkpoint.token != receipt.token || entry.phase != receipt.phase {
                continue;
            }
            match entry.phase {
                Phase::Queued(action) => self.launch_meeting_action(receipt.id, action, cx),
                Phase::AwaitingSave => {
                    entry.phase = Phase::Complete;
                    entry.message =
                        "Latest meeting saved on this device. Contact updates sync separately."
                            .into();
                }
                _ => {}
            }
        }
        cx.notify();
    }

    pub fn meeting_actions_save_failed(
        &mut self,
        receipts: Vec<ActionSaveReceipt>,
        cx: &mut Context<'_, Self>,
    ) {
        for receipt in receipts {
            let Some(entry) = self.meeting_actions.get_mut(&receipt.id) else {
                continue;
            };
            if entry.checkpoint.token != receipt.token || entry.phase != receipt.phase {
                continue;
            }
            entry.message = match entry.phase {
                Phase::Queued(_) => "Request not sent: its local intent could not be saved. Use Retry save before continuing.",
                _ => "The server result is not saved on this device. Use Retry save; do not regenerate it.",
            }.into();
        }
        cx.notify();
    }
}

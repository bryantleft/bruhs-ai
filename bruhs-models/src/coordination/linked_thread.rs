//! Wire types for a conversation owned by a linked checkout's host.
use super::{PendingApproval, TaskMessage, TaskState};
use crate::{
    delivery::generation::{GenerationOptions, OptionKind},
    ids::ThreadId,
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckoutContext {
    pub path: String,
    pub branch: Option<String>,
    pub model: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinkedAttachment {
    pub id: String,
    pub name: String,
    pub bytes: u64,
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StartThread {
    pub checkout: CheckoutContext,
    pub message: String,
    #[serde(default, skip_serializing_if = "GenerationOptions::is_empty")]
    pub options: GenerationOptions,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<LinkedAttachment>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LinkedProjectReply {
    pub request_id: uuid::Uuid,
    pub message: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<LinkedAttachment>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct LinkedThread {
    pub id: ThreadId,
    pub title: String,
    pub checkout: CheckoutContext,
    pub state: TaskState,
    pub messages: Vec<TaskMessage>,
    pub live_output: String,
    pub error: Option<String>,
    #[serde(default)]
    pub approvals: Vec<PendingApproval>,
    #[serde(default)]
    pub options: GenerationOptions,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinkedThreadSummary {
    pub id: ThreadId,
    pub title: String,
    pub checkout: CheckoutContext,
    pub updated_at: crate::EpochMs,
}

#[derive(Serialize, Deserialize)]
pub struct LinkedGuestAccess {
    pub token: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum LinkedThreadAction {
    Stop,
    SetModel {
        model: String,
    },
    SetModelOption {
        model: String,
        kind: OptionKind,
        value: String,
    },
    Approval {
        request_id: String,
        allow: bool,
    },
}

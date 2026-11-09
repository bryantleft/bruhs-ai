//! Conversation originals stay separate from normalized display messages.
use std::collections::{HashMap, HashSet};

use leptos::prelude::*;

use super::{Kind, Record, TaskSaves};
use bruhs_models::agent::Role;

use crate::state::{AgentMessage, MessageStatus, Thread};
use crate::storage::MessagesRecord;

#[derive(Default)]
pub(super) struct Conversations {
    latest: HashMap<String, Option<MessagesRecord>>,
    deleted: HashSet<String>,
}

const UNFINISHED_RESPONSE: &str = "This saved response is unfinished, and this tab isn’t connected to it. Check other Bruhs tabs and review any actions before retrying. Retry sends the request again.";

/// Older records without ordered parts are rebuilt when they are read
/// (`state::MessageRecord`), so loaded copies display as they are.
fn displayed(record: Option<MessagesRecord>) -> Vec<AgentMessage> {
    record.map(|record| record.messages).unwrap_or_default()
}

impl TaskSaves {
    pub fn edit_thread(self, expected: Option<Thread>, draft: Thread) {
        let id = draft.id.clone();
        if self
            .conversations
            .with_value(|state| state.deleted.contains(&id))
        {
            return;
        }
        if expected.is_none() {
            self.conversations.update_value(|state| {
                state.latest.entry(id.clone()).or_insert(None);
            });
        }
        self.queue.update(|queue| {
            queue.edit_record(
                Kind::Thread,
                id.clone(),
                expected.map(Record::Thread),
                Some(Record::Thread(draft)),
            );
            queue.group_records(&[(Kind::Thread, id.clone()), (Kind::Conversation, id)]);
        });
        self.refresh_status();
        self.save();
    }

    pub fn delete_thread(self, original: Thread) {
        let id = original.id.clone();
        let mut conversation = None;
        self.conversations.update_value(|state| {
            state.deleted.insert(id.clone());
            conversation = state.latest.remove(&id);
        });
        self.queue.update(|queue| {
            queue.edit_record(
                Kind::Thread,
                id.clone(),
                Some(Record::Thread(original)),
                None,
            );
            if let Some(original) = conversation {
                queue.edit_record(
                    Kind::Conversation,
                    id.clone(),
                    original.map(Record::Conversation),
                    None,
                );
            }
            queue.group_records(&[(Kind::Thread, id.clone()), (Kind::Conversation, id)]);
        });
        self.refresh_status();
        self.save();
    }

    /// Pending drafts win over storage when navigating away and back, even
    /// after another stream has replaced the single live stream buffer.
    pub fn pending_conversation(self, id: &str) -> Option<Vec<AgentMessage>> {
        self.queue
            .with_untracked(|queue| queue.conversation(id))
            .map(displayed)
    }

    pub(crate) fn loaded_conversation(
        self,
        id: &str,
        loaded: Option<MessagesRecord>,
    ) -> Vec<AgentMessage> {
        if let Some(pending) = self.pending_conversation(id) {
            return pending;
        }
        self.conversations.update_value(|state| {
            if !state.deleted.contains(id) {
                state.latest.insert(id.to_owned(), loaded.clone());
            }
        });
        recovered(loaded)
    }

    pub fn edit_conversation(self, id: String, messages: Vec<AgentMessage>) {
        let mut original = None;
        let draft = MessagesRecord {
            thread_id: id.clone(),
            messages,
        };
        if self
            .conversations
            .with_value(|state| state.deleted.contains(&id))
        {
            return;
        }
        self.conversations.update_value(|state| {
            original = state.latest.insert(id.clone(), Some(draft.clone()));
        });
        let known = original.is_some();
        self.queue.update(|queue| {
            queue.edit_record(Kind::Conversation, id.clone(), original.flatten().map(Record::Conversation), Some(Record::Conversation(draft)));
            queue.group_records(&[(Kind::Thread, id.clone()), (Kind::Conversation, id.clone())]);
            if !known {
                queue.failed_record(Kind::Conversation, &id,
                    "The saved conversation has not loaded. Your unsaved messages are kept here; copy the recovery data before reloading.".into(), true);
            }
        });
        self.refresh_status();
        self.save();
    }
}

/// A persisted streaming flag is not proof of a live connection in this tab.
/// Only loaded display copies change; raw originals and live buffers do not.
fn recovered(record: Option<MessagesRecord>) -> Vec<AgentMessage> {
    let mut messages = displayed(record);
    for message in &mut messages {
        if message.role == Role::Assistant && message.is_streaming() {
            message.status = MessageStatus::Finished { at_ms: None };
            if message.error.is_none() {
                message.error = Some(UNFINISHED_RESPONSE.into());
            }
        }
    }
    messages
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::MessagePart;

    /// A record from before ordered parts reads with them, and reading the
    /// same stored row twice gives equal originals (the save conflict check
    /// compares a fresh read against the one the edit started from).
    #[test]
    fn legacy_records_display_with_parts_and_read_the_same_every_time() {
        let raw = serde_json::json!({
            "thread_id": "a",
            "messages": [{
                "id": "m", "role": "user", "content": "Legacy text",
                "tool_calls": [], "is_streaming": false
            }]
        });
        let first: MessagesRecord = serde_json::from_value(raw.clone()).unwrap();
        let second: MessagesRecord = serde_json::from_value(raw).unwrap();
        assert_eq!(first, second);
        let shown = displayed(Some(first));
        assert_eq!(shown[0].text(), "Legacy text");
        assert!(!shown[0].parts.is_empty());
    }

    fn unfinished() -> MessagesRecord {
        let mut message = AgentMessage::assistant_placeholder("synthetic".into(), 1.0);
        message
            .parts
            .push(MessagePart::Text("Partial response 日本語 🦀".into()));
        MessagesRecord {
            thread_id: "a".into(),
            messages: vec![AgentMessage::user("Original prompt".into(), 1.0), message],
        }
    }

    #[test]
    fn loaded_unfinished_response_becomes_explicitly_disconnected_without_rewriting_original() {
        let raw = unfinished();
        let shown = recovered(Some(raw.clone()));
        assert!(!shown[1].is_streaming());
        assert_eq!(shown[1].error.as_deref(), Some(UNFINISHED_RESPONSE));
        assert_eq!(shown[1].parts, raw.messages[1].parts);
        assert_eq!(shown[1].status, MessageStatus::Finished { at_ms: None });
        assert!(raw.messages[1].is_streaming());
        assert!(raw.messages[1].error.is_none());
        assert_eq!(shown[0], raw.messages[0]);
    }

    #[test]
    fn live_pending_display_is_not_marked_disconnected() {
        let shown = displayed(Some(unfinished()));
        assert!(shown[1].is_streaming());
        assert!(shown[1].error.is_none());
    }

    #[test]
    fn recovery_preserves_existing_errors_and_completed_responses() {
        let mut raw = unfinished();
        raw.messages[1].error = Some("Existing transport error".into());
        let shown = recovered(Some(raw.clone()));
        assert_eq!(shown[1].error, raw.messages[1].error);
        assert!(!shown[1].is_streaming());
        raw.messages[1].status = MessageStatus::Finished { at_ms: None };
        raw.messages[1].error = None;
        assert_eq!(recovered(Some(raw.clone())), displayed(Some(raw)));
    }
}

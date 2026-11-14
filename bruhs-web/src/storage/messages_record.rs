//! Cache and save-queue originals share an immutable conversation snapshot.
//! Display recovery takes an independent message vector before changing it.

use std::sync::Arc;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::state::AgentMessage;

#[derive(Clone, Debug)]
pub(crate) struct MessagesRecord(Arc<Data>);

#[derive(Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename = "MessagesRecord")]
pub(super) struct Data {
    pub(super) thread_id: String,
    pub(super) messages: Vec<AgentMessage>,
}

impl MessagesRecord {
    pub(crate) fn new(thread_id: String, messages: Vec<AgentMessage>) -> Self {
        Data {
            thread_id,
            messages,
        }
        .into()
    }

    pub(crate) fn thread_id(&self) -> &str {
        &self.0.thread_id
    }

    pub(crate) fn messages(&self) -> &[AgentMessage] {
        &self.0.messages
    }

    pub(crate) fn into_messages(self) -> Vec<AgentMessage> {
        match Arc::try_unwrap(self.0) {
            Ok(data) => data.messages,
            Err(data) => data.messages.clone(),
        }
    }
}

impl From<Data> for MessagesRecord {
    fn from(data: Data) -> Self {
        Self(Arc::new(data))
    }
}

impl PartialEq for MessagesRecord {
    fn eq(&self, other: &Self) -> bool {
        // Preserve the existing comparison for non-reflexive float values.
        self.0.as_ref() == other.0.as_ref()
    }
}

impl Serialize for MessagesRecord {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.0.as_ref().serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for MessagesRecord {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Data::deserialize(deserializer).map(Self::from)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot() -> MessagesRecord {
        MessagesRecord::new(
            "thread".into(),
            vec![AgentMessage::user("Original 雪 😀".into(), 1.0)],
        )
    }

    #[test]
    fn cloned_save_snapshots_share_the_message_allocation() {
        let original = snapshot();
        let queued = original.clone();
        assert_eq!(original.messages().as_ptr(), queued.messages().as_ptr());
    }

    #[test]
    fn mutable_display_copy_does_not_change_a_shared_original() {
        let original = snapshot();
        let mut displayed = original.clone().into_messages();
        displayed[0].parts.clear();
        assert_eq!(original.messages()[0].text(), "Original 雪 😀");
    }

    #[test]
    fn consuming_an_unshared_snapshot_moves_its_messages() {
        let original = snapshot();
        let allocation = original.messages().as_ptr();
        let messages = original.into_messages();
        assert_eq!(messages.as_ptr(), allocation);
    }

    #[test]
    fn sharing_does_not_make_nan_timestamps_compare_equal() {
        let mut message = AgentMessage::user("non-finite".into(), 1.0);
        message.started_at_ms = Some(f64::NAN);
        let original = MessagesRecord::new("thread".into(), vec![message]);
        assert_ne!(original, original.clone());
    }
}

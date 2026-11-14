use bruhs_models::{KnowledgeAttachment, KnowledgeDocument, PersonProfile, SourceClip};
use serde::Serialize;
use std::sync::Arc;

use crate::state::{CalendarEvent, CaptureRecord, InboxConversation, Project, WorkItem};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Task,
    Capture,
    Inbox,
    Event,
    Document,
    Attachment,
    SourceClip,
    Person,
    Project,
    Thread,
    Conversation,
}

/// Keep the existing store wire shapes while distinguishing identities across
/// stores. A task and capture may legitimately have the same string ID.
#[derive(Clone, PartialEq, Serialize, Debug)]
#[serde(untagged)]
pub enum Record {
    Task(WorkItem),
    Capture(CaptureRecord),
    Inbox(InboxConversation),
    Event(CalendarEvent),
    Document(KnowledgeDocument),
    Attachment(#[serde(serialize_with = "serialize_attachment")] Arc<KnowledgeAttachment>),
    SourceClip(SourceClip),
    Person(PersonProfile),
    Project(Project),
    Thread(crate::state::Thread),
    Conversation(crate::storage::MessagesRecord),
}

fn serialize_attachment<S: serde::Serializer>(
    attachment: &Arc<KnowledgeAttachment>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    attachment.as_ref().serialize(serializer)
}

impl Record {
    pub fn id(&self) -> &str {
        match self {
            Self::Task(record) => &record.id,
            Self::Capture(record) => &record.id,
            Self::Inbox(record) => &record.id,
            Self::Event(record) => &record.id,
            Self::Document(record) => &record.id,
            Self::Attachment(record) => &record.id,
            Self::SourceClip(record) => &record.id,
            Self::Person(record) => &record.id,
            Self::Project(record) => &record.id,
            Self::Thread(record) => &record.id,
            Self::Conversation(record) => record.thread_id(),
        }
    }

    pub fn kind(&self) -> Kind {
        match self {
            Self::Task(_) => Kind::Task,
            Self::Capture(_) => Kind::Capture,
            Self::Inbox(_) => Kind::Inbox,
            Self::Event(_) => Kind::Event,
            Self::Document(_) => Kind::Document,
            Self::Attachment(_) => Kind::Attachment,
            Self::SourceClip(_) => Kind::SourceClip,
            Self::Person(_) => Kind::Person,
            Self::Project(_) => Kind::Project,
            Self::Thread(_) => Kind::Thread,
            Self::Conversation(_) => Kind::Conversation,
        }
    }
}

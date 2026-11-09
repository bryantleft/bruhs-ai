//! Private relationship context layered over meeting-derived people profiles.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::{InboxConversation, MessageDirection};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RelationshipItemKind {
    Interaction,
    Commitment,
    Reminder,
    Note,
}

impl RelationshipItemKind {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Interaction => "Interaction",
            Self::Commitment => "Commitment",
            Self::Reminder => "Reminder",
            Self::Note => "Note",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RelationshipItem {
    pub id: String,
    pub kind: RelationshipItemKind,
    pub text: String,
    #[serde(default)]
    pub due_at_ms: Option<i64>,
    #[serde(default)]
    pub completed_at_ms: Option<i64>,
    #[serde(default)]
    pub source_record_id: Option<String>,
    pub created_at_ms: i64,
}

impl RelationshipItem {
    pub fn new(kind: RelationshipItemKind, text: impl Into<String>, now_ms: i64) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            kind,
            text: text.into().trim().to_string(),
            due_at_ms: None,
            completed_at_ms: None,
            source_record_id: None,
            created_at_ms: now_ms,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RelationshipProfile {
    pub person_id: String,
    #[serde(default)]
    pub company: String,
    #[serde(default)]
    pub role: String,
    #[serde(default)]
    pub email: String,
    #[serde(default)]
    pub phone: String,
    #[serde(default)]
    pub location: String,
    #[serde(default)]
    pub notes: String,
    #[serde(default)]
    pub items: Vec<RelationshipItem>,
    #[serde(default)]
    pub last_contacted_at_ms: Option<i64>,
    pub updated_at_ms: i64,
}

impl RelationshipProfile {
    pub fn new(person_id: impl Into<String>, now_ms: i64) -> Self {
        Self {
            person_id: person_id.into(),
            company: String::new(),
            role: String::new(),
            email: String::new(),
            phone: String::new(),
            location: String::new(),
            notes: String::new(),
            items: Vec::new(),
            last_contacted_at_ms: None,
            updated_at_ms: now_ms,
        }
    }

    pub fn add_item(&mut self, item: RelationshipItem, now_ms: i64) -> bool {
        if item.text.is_empty() {
            return false;
        }
        self.items.insert(0, item);
        self.updated_at_ms = now_ms;
        true
    }

    pub fn toggle_item(&mut self, item_id: &str, now_ms: i64) -> bool {
        let Some(item) = self.items.iter_mut().find(|item| item.id == item_id) else {
            return false;
        };
        item.completed_at_ms = item.completed_at_ms.map_or(Some(now_ms), |_| None);
        self.updated_at_ms = now_ms;
        true
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SuggestedFollowUp {
    pub person_id: String,
    pub reason: String,
    pub source_record_id: Option<String>,
    pub due_at_ms: Option<i64>,
}

pub fn suggested_follow_ups(
    relationships: &[RelationshipProfile],
    conversations: &[InboxConversation],
    now_ms: i64,
) -> Vec<SuggestedFollowUp> {
    let mut suggestions = Vec::new();
    for relationship in relationships {
        for item in relationship.items.iter().filter(|item| {
            item.completed_at_ms.is_none()
                && matches!(
                    item.kind,
                    RelationshipItemKind::Commitment | RelationshipItemKind::Reminder
                )
                && item.due_at_ms.is_some_and(|due| due <= now_ms)
        }) {
            suggestions.push(SuggestedFollowUp {
                person_id: relationship.person_id.clone(),
                reason: item.text.clone(),
                source_record_id: item.source_record_id.clone(),
                due_at_ms: item.due_at_ms,
            });
        }
    }
    for conversation in conversations
        .iter()
        .filter(|conversation| conversation.needs_reply())
    {
        for participant in &conversation.participants {
            if let Some(person_id) = &participant.person_id {
                suggestions.push(SuggestedFollowUp {
                    person_id: person_id.clone(),
                    reason: format!("Reply to {}", conversation.subject),
                    source_record_id: Some(conversation.id.clone()),
                    due_at_ms: conversation
                        .last_message()
                        .filter(|message| message.direction == MessageDirection::Incoming)
                        .map(|message| message.sent_at_ms),
                });
            }
        }
    }
    suggestions.sort_unstable_by_key(|suggestion| suggestion.due_at_ms.unwrap_or(i64::MAX));
    suggestions
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{InboxMessage, InboxParticipant, InboxSource};

    #[test]
    fn suggestions_join_due_commitments_and_unanswered_conversations() {
        let mut relationship = RelationshipProfile::new("person-1", 1);
        let mut commitment =
            RelationshipItem::new(RelationshipItemKind::Commitment, "Send the brief", 1);
        commitment.due_at_ms = Some(10);
        relationship.add_item(commitment, 1);

        let mut conversation = InboxConversation::new(
            InboxSource::Slack,
            "Launch",
            vec![InboxParticipant {
                id: "participant-1".into(),
                display_name: "Ada".into(),
                address: String::new(),
                person_id: Some("person-1".into()),
            }],
            1,
        );
        conversation.receive(InboxMessage::incoming("Ada", "Any update?", 20));

        let suggestions = suggested_follow_ups(&[relationship], &[conversation], 30);
        assert_eq!(suggestions.len(), 2);
        assert_eq!(suggestions[0].reason, "Send the brief");
        assert_eq!(suggestions[1].reason, "Reply to Launch");
    }
}

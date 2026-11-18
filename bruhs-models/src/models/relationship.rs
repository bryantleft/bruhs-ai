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

    /// Replace an item's kind, text and due time. A blank text is refused
    /// and changes nothing. Completion, source and identity are kept.
    pub fn edit_item(&mut self, item_id: &str, edit: RelationshipItemEdit, now_ms: i64) -> bool {
        let text = edit.text.trim();
        if text.is_empty() {
            return false;
        }
        let Some(item) = self.items.iter_mut().find(|item| item.id == item_id) else {
            return false;
        };
        let due_at_ms = edit.due.resolve(edit.kind, item.due_at_ms, now_ms);
        if item.kind == edit.kind && item.text == text && item.due_at_ms == due_at_ms {
            return false;
        }
        item.kind = edit.kind;
        item.text = text.to_string();
        item.due_at_ms = due_at_ms;
        self.updated_at_ms = now_ms;
        true
    }

    /// Remove one item. Other items keep their order.
    pub fn remove_item(&mut self, item_id: &str, now_ms: i64) -> bool {
        let before = self.items.len();
        self.items.retain(|item| item.id != item_id);
        if self.items.len() == before {
            return false;
        }
        self.updated_at_ms = now_ms;
        true
    }
}

/// The due options both surfaces offer for a relationship item.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelationshipDue {
    /// No due time.
    None,
    /// Keep the current due time, or set tomorrow if there is none.
    Tomorrow,
}

impl RelationshipDue {
    /// Only commitments and reminders can be due; other kinds never are.
    pub const fn applies_to(kind: RelationshipItemKind) -> bool {
        matches!(
            kind,
            RelationshipItemKind::Commitment | RelationshipItemKind::Reminder
        )
    }

    /// The due time to save for `kind`, given the item's current one.
    pub fn resolve(
        self,
        kind: RelationshipItemKind,
        current: Option<i64>,
        now_ms: i64,
    ) -> Option<i64> {
        match self {
            Self::Tomorrow if Self::applies_to(kind) => Some(
                current
                    .unwrap_or_else(|| now_ms.saturating_add(super::RELATIONSHIP_DUE_TOMORROW_MS)),
            ),
            _ => None,
        }
    }
}

/// A reviewed edit of one relationship item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelationshipItemEdit {
    pub kind: RelationshipItemKind,
    pub text: String,
    pub due: RelationshipDue,
}

/// Every kind, in the order both surfaces show them.
pub const RELATIONSHIP_ITEM_KINDS: [RelationshipItemKind; 4] = [
    RelationshipItemKind::Interaction,
    RelationshipItemKind::Commitment,
    RelationshipItemKind::Reminder,
    RelationshipItemKind::Note,
];

impl RelationshipItem {
    /// A new item with the chosen due option applied.
    pub fn with_due(
        kind: RelationshipItemKind,
        text: impl Into<String>,
        due: RelationshipDue,
        now_ms: i64,
    ) -> Self {
        let mut item = Self::new(kind, text, now_ms);
        item.due_at_ms = due.resolve(kind, None, now_ms);
        item
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

/// The follow ups one profile suggests: that person's due commitments and
/// reminders, and replies owed in conversations they take part in. Another
/// person's conversation never shows on this profile, and a person with no
/// saved relationship details still gets their replies.
pub fn person_follow_ups(
    person_id: &str,
    relationships: &[RelationshipProfile],
    conversations: &[InboxConversation],
    now_ms: i64,
) -> Vec<SuggestedFollowUp> {
    let mut suggestions = suggested_follow_ups(relationships, conversations, now_ms);
    suggestions.retain(|suggestion| suggestion.person_id == person_id);
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

    fn unanswered(subject: &str, person_id: &str, at: i64) -> InboxConversation {
        let mut conversation = InboxConversation::new(
            InboxSource::Email,
            subject,
            vec![InboxParticipant {
                id: format!("participant-{person_id}"),
                display_name: person_id.into(),
                address: String::new(),
                person_id: Some(person_id.into()),
            }],
            at,
        );
        conversation.receive(InboxMessage::incoming(person_id, "Any update?", at));
        conversation
    }

    /// A profile lists only its own person's follow ups, with or without
    /// saved relationship details.
    #[test]
    fn a_profile_suggests_only_its_own_follow_ups() {
        let mut ada = RelationshipProfile::new("ada", 1);
        let mut reminder = RelationshipItem::new(RelationshipItemKind::Reminder, "Call Ada", 1);
        reminder.due_at_ms = Some(5);
        ada.add_item(reminder, 1);
        let conversations = [
            unanswered("Ada's question", "ada", 10),
            unanswered("Grace's question", "grace", 20),
        ];
        let relationships = [ada];

        let reasons = |person: &str| {
            person_follow_ups(person, &relationships, &conversations, 30)
                .into_iter()
                .map(|suggestion| suggestion.reason)
                .collect::<Vec<_>>()
        };
        assert_eq!(reasons("ada"), ["Call Ada", "Reply to Ada's question"]);
        assert_eq!(
            reasons("grace"),
            ["Reply to Grace's question"],
            "no saved details is no reason to hide a reply"
        );
        assert!(reasons("nobody").is_empty());
    }

    #[test]
    fn items_can_be_edited_and_removed_with_the_same_due_options() {
        let mut profile = RelationshipProfile::new("person-1", 1);
        let item = RelationshipItem::with_due(
            RelationshipItemKind::Reminder,
            "  Call back ",
            RelationshipDue::Tomorrow,
            100,
        );
        assert_eq!(item.text, "Call back");
        assert_eq!(
            item.due_at_ms,
            Some(100 + crate::RELATIONSHIP_DUE_TOMORROW_MS)
        );
        let note = RelationshipItem::with_due(
            RelationshipItemKind::Note,
            "Likes tea",
            RelationshipDue::Tomorrow,
            100,
        );
        assert_eq!(note.due_at_ms, None, "notes are never due");
        let id = item.id.clone();
        profile.add_item(item, 100);
        profile.add_item(note.clone(), 101);
        profile.toggle_item(&id, 102);

        assert!(!profile.edit_item(
            &id,
            RelationshipItemEdit {
                kind: RelationshipItemKind::Commitment,
                text: "   ".into(),
                due: RelationshipDue::None,
            },
            103,
        ));
        assert_eq!(profile.updated_at_ms, 102, "a blank edit changes nothing");

        assert!(profile.edit_item(
            &id,
            RelationshipItemEdit {
                kind: RelationshipItemKind::Commitment,
                text: "Send the deck".into(),
                due: RelationshipDue::Tomorrow,
            },
            104,
        ));
        let edited = profile.items.iter().find(|item| item.id == id).unwrap();
        assert_eq!(edited.kind, RelationshipItemKind::Commitment);
        assert_eq!(edited.text, "Send the deck");
        assert_eq!(
            edited.due_at_ms,
            Some(100 + crate::RELATIONSHIP_DUE_TOMORROW_MS),
            "an existing due time is kept"
        );
        assert_eq!(edited.completed_at_ms, Some(102), "completion is kept");

        assert!(profile.edit_item(
            &id,
            RelationshipItemEdit {
                kind: RelationshipItemKind::Interaction,
                text: "Send the deck".into(),
                due: RelationshipDue::Tomorrow,
            },
            105,
        ));
        assert_eq!(
            profile
                .items
                .iter()
                .find(|item| item.id == id)
                .unwrap()
                .due_at_ms,
            None,
            "interactions are never due"
        );

        assert!(profile.remove_item(&id, 106));
        assert!(!profile.remove_item(&id, 107));
        assert_eq!(profile.items, vec![note]);
        assert_eq!(profile.updated_at_ms, 106);
    }
}

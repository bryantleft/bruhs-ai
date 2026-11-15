//! Read projections for browsing/searching without copying conversation history.
use super::InboxConversation;
use super::InboxSource;
use serde::{Deserialize, Serialize};

// Uses the same mounted-row budget as the web task list. The diagnostic native
// inbox fixture exercises 100/1,000/10,000 records; see the performance goal report.
pub const INBOX_PAGE_SIZE: usize = 100;
// Both inbox surfaces display one truncated mono line, at most 1,024px wide.
// 160 scalars cover that line at their 11–14px font sizes; search keeps full text.
const PREVIEW_CHARS: usize = 160;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InboxFilter {
    #[default]
    Inbox,
    Unread,
    Snoozed,
    Archived,
    All,
}

impl InboxFilter {
    pub const ALL: [Self; 5] = [
        Self::Inbox,
        Self::Unread,
        Self::Snoozed,
        Self::Archived,
        Self::All,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Inbox => "Inbox",
            Self::Unread => "Unread",
            Self::Snoozed => "Snoozed",
            Self::Archived => "Archived",
            Self::All => "All",
        }
    }

    pub const fn empty_message(self) -> &'static str {
        match self {
            Self::Inbox => "Your inbox is clear.",
            Self::Unread => "No unread conversations.",
            Self::Snoozed => "No snoozed conversations.",
            Self::Archived => "No archived conversations.",
            Self::All => "No conversations yet.",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct InboxCard {
    pub id: String,
    pub source: InboxSource,
    pub subject: String,
    pub sender: String,
    pub preview: String,
    pub unread: bool,
    pub important: bool,
    pub message_at_ms: i64,
}

impl From<&InboxConversation> for InboxCard {
    fn from(conversation: &InboxConversation) -> Self {
        let last_message = conversation.last_message();
        Self {
            id: conversation.id.clone(),
            source: conversation.source,
            subject: conversation.subject.clone(),
            sender: conversation
                .participants
                .first()
                .map_or("Unknown", |person| person.display_name.as_str())
                .to_owned(),
            preview: last_message
                .map_or("Draft only", |message| message.body.as_str())
                .chars()
                .take(PREVIEW_CHARS)
                .map(|c| if c == '\n' { ' ' } else { c })
                .collect(),
            unread: conversation.unread,
            important: conversation.labels.iter().any(|label| label == "important"),
            message_at_ms: last_message
                .map_or(conversation.created_at_ms, |message| message.sent_at_ms),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct InboxCounts {
    pub open: usize,
    pub unread: usize,
    pub needs_reply: usize,
    pub snoozed: usize,
}

#[derive(PartialEq, Eq)]
struct Entry {
    card: InboxCard,
    fields: Vec<String>,
    archived: bool,
    snoozed_until: Option<i64>,
    needs_reply: bool,
    updated_at: i64,
}

#[derive(PartialEq, Eq)]
pub struct InboxDirectory(Vec<Entry>);

impl InboxDirectory {
    pub fn new(conversations: &[InboxConversation]) -> Self {
        let mut entries =
            conversations
                .iter()
                .map(|conversation| Entry {
                    card: InboxCard::from(conversation),
                    fields: std::iter::once(conversation.subject.as_str())
                        .chain(conversation.participants.iter().flat_map(|person| {
                            [person.display_name.as_str(), person.address.as_str()]
                        }))
                        .chain(
                            conversation
                                .messages
                                .iter()
                                .map(|message| message.body.as_str()),
                        )
                        .map(str::to_lowercase)
                        .collect(),
                    archived: conversation.archived,
                    snoozed_until: conversation.snoozed_until_ms,
                    needs_reply: conversation.needs_reply(),
                    updated_at: conversation.updated_at_ms,
                })
                .collect::<Vec<_>>();
        entries.sort_unstable_by(|a, b| {
            b.updated_at
                .cmp(&a.updated_at)
                .then_with(|| a.card.id.cmp(&b.card.id))
        });
        Self(entries)
    }

    pub fn matching(&self, query: &str, filter: InboxFilter, now: i64) -> Vec<usize> {
        let query = query.trim().to_lowercase();
        self.0
            .iter()
            .enumerate()
            .filter(|(_, entry)| {
                let in_filter = match filter {
                    InboxFilter::Inbox => {
                        !entry.archived && entry.snoozed_until.is_none_or(|until| until <= now)
                    }
                    InboxFilter::Unread => entry.card.unread && !entry.archived,
                    InboxFilter::Snoozed => entry.snoozed_until.is_some_and(|until| until > now),
                    InboxFilter::Archived => entry.archived,
                    InboxFilter::All => true,
                };
                in_filter
                    && (query.is_empty() || entry.fields.iter().any(|field| field.contains(&query)))
            })
            .map(|(index, _)| index)
            .collect()
    }

    pub fn page(&self, matches: &[usize], page: usize) -> Vec<InboxCard> {
        let start = Self::page_start(matches.len(), page);
        matches
            .iter()
            .skip(start)
            .take(INBOX_PAGE_SIZE)
            .filter_map(|index| self.0.get(*index))
            .map(|entry| entry.card.clone())
            .collect()
    }

    pub fn page_start(total: usize, page: usize) -> usize {
        page.min(total.saturating_sub(1) / INBOX_PAGE_SIZE) * INBOX_PAGE_SIZE
    }

    pub fn counts(&self, now: i64) -> InboxCounts {
        let mut counts = InboxCounts::default();
        for entry in &self.0 {
            let snoozed = entry.snoozed_until.is_some_and(|until| until > now);
            counts.open += usize::from(!entry.archived && !snoozed);
            counts.unread += usize::from(entry.card.unread && !entry.archived);
            counts.needs_reply += usize::from(entry.needs_reply && !entry.archived);
            counts.snoozed += usize::from(snoozed);
        }
        counts
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::InboxMessage;

    #[test]
    fn displayed_message_date_does_not_change_when_a_draft_is_saved() {
        let mut conversation = InboxConversation::new(InboxSource::Email, "Launch", vec![], 10);
        assert_eq!(InboxCard::from(&conversation).message_at_ms, 10);
        conversation.receive(InboxMessage::incoming("Ada", "Original message", 20));
        conversation.upsert_draft("Unsent reply", 40);
        let card = InboxCard::from(&conversation);
        assert_eq!(card.message_at_ms, 20);
        assert_eq!(card.preview, "Original message");
        conversation.send_draft(50);
        assert_eq!(InboxCard::from(&conversation).message_at_ms, 50);
    }

    fn records(count: usize) -> Vec<InboxConversation> {
        (0..count)
            .map(|i| {
                let at = i64::try_from(i).unwrap();
                let mut item =
                    InboxConversation::new(InboxSource::Email, format!("Subject {i}"), vec![], at);
                item.id = format!("conversation-{i:04}");
                item.receive(InboxMessage::incoming(
                    "Person",
                    format!("{}\nDeep CAFÉ 世界 {i}", "🦀".repeat(200)),
                    at,
                ));
                item
            })
            .collect()
    }

    #[test]
    fn pages_reach_every_record_and_clamp_after_filters_or_deletion() {
        let directory = InboxDirectory::new(&records(205));
        let matches = directory.matching("", InboxFilter::All, 500);
        let all = (0..3)
            .flat_map(|page| directory.page(&matches, page))
            .map(|card| card.id)
            .collect::<Vec<_>>();
        assert_eq!(all.len(), 205);
        assert_eq!(
            all.iter().collect::<std::collections::HashSet<_>>().len(),
            205
        );
        assert_eq!(all[0], "conversation-0204");
        assert_eq!(all[204], "conversation-0000");
        assert_eq!(directory.page(&matches, usize::MAX).len(), 5);
        let narrowed = directory.matching("DEEP café 世界 204", InboxFilter::All, 500);
        assert_eq!(
            directory.page(&narrowed, usize::MAX)[0].id,
            "conversation-0204"
        );
        assert!(directory.page(&[], usize::MAX).is_empty());
        assert_eq!(InboxDirectory::page_start(0, usize::MAX), 0);
    }

    #[test]
    fn previews_are_bounded_but_search_keeps_full_message_and_participant_fields() {
        let mut items = records(1);
        items[0].participants.push(super::super::InboxParticipant {
            id: "person".into(),
            display_name: "Renée".into(),
            address: "ADDRESS@EXAMPLE.COM".into(),
            person_id: None,
        });
        let directory = InboxDirectory::new(&items);
        for query in ["renée", "address@example.com", "DEEP café 世界 0"] {
            assert_eq!(directory.matching(query, InboxFilter::All, 1), [0]);
        }
        let card = &directory.page(&[0], 0)[0];
        assert_eq!(card.preview.chars().count(), PREVIEW_CHARS);
        assert_eq!(card.sender, "Renée");
        assert!(!card.preview.contains("Deep"));
        assert!(directory
            .matching("Renée Subject", InboxFilter::All, 1)
            .is_empty());
    }

    #[test]
    fn counts_and_filter_membership_follow_snooze_deadlines_and_triage() {
        let mut items = records(3);
        items[0].set_archived(true);
        items[1].snooze_until(Some(10));
        items[2].mark_read(true);
        let directory = InboxDirectory::new(&items);
        assert_eq!(
            directory.counts(5),
            InboxCounts {
                open: 1,
                unread: 1,
                needs_reply: 2,
                snoozed: 1
            }
        );
        assert_eq!(directory.counts(10).open, 2);
        for (filter, expected) in [
            (InboxFilter::Inbox, 1),
            (InboxFilter::Unread, 1),
            (InboxFilter::Snoozed, 1),
            (InboxFilter::Archived, 1),
            (InboxFilter::All, 3),
        ] {
            assert_eq!(directory.matching("", filter, 5).len(), expected);
        }
    }
}

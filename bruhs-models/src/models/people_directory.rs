//! People page rules every surface shares: search, paging, alias review, name
//! matching for auto links, a person's linked records, and the cascade that
//! deleting, merging or undoing a contact applies to private links.
//!
//! Names always compare through [`person_name_key`], so search, alias review
//! and auto links agree with the opt-out matcher.

use serde::{Deserialize, Serialize};

use super::{
    person_name_key, InboxConversation, KnowledgeDocument, KnowledgeLink, KnowledgeLinkKind,
    Meeting, PersonProfile,
};

/// People shown on one directory page, on every surface.
pub const PEOPLE_PAGE_SIZE: usize = 25;

/// How far "Due tomorrow" sets a reminder's due time.
pub const RELATIONSHIP_DUE_TOMORROW_MS: i64 = 86_400_000;

/// App-owned People copy both surfaces render, so web and desktop match word
/// for word. Follows `docs/ui-standards.md`: short sentences, no dashes.
pub mod people_copy {
    pub const PAGE_SUBTITLE: &str = "Profiles from your meetings, with notes and reminders.";
    pub const CREATE_LABEL: &str = "New person";
    pub const CREATE_PLACEHOLDER: &str = "Name";
    pub const CREATE_BUTTON: &str = "Add person";
    pub const CREATE_BLANK: &str = "A name is required. No profile was created.";
    pub const SEARCH_LABEL: &str = "Search people";
    pub const SEARCH_PLACEHOLDER: &str = "Search by name or alias";
    pub const SEARCH_EMPTY: &str = "No people match this search.";
    pub const PREVIOUS_PAGE: &str = "Previous people";
    pub const NEXT_PAGE: &str = "Next people";
    pub const RENAME_BLANK: &str = "A name is required. The profile kept its previous name.";
    pub const ALIASES_HEADING: &str = "Aliases";
    pub const ALIASES_EMPTY: &str = "No aliases yet.";
    pub const ALIAS_PLACEHOLDER: &str = "Add an alias";
    pub const ALIAS_BUTTON: &str = "Add alias";
    pub const MEETINGS_HEADING: &str = "Meetings";
    pub const MEETINGS_EMPTY: &str = "No meetings with this person yet.";
    pub const CONVERSATIONS_HEADING: &str = "Conversations";
    pub const CONVERSATIONS_EMPTY: &str = "No Inbox conversations with this person yet.";
    /// A linked meeting saved without a title.
    pub const UNTITLED_MEETING: &str = "Untitled meeting";
    /// The heading over a profile's follow up suggestions.
    pub const FOLLOW_UPS_HEADING: &str = "Suggested follow ups";
    pub const BACK_TO_PEOPLE: &str = "Back to People";
    pub const MERGE_CONFIRM_BUTTON: &str = "Confirm merge";
    pub const DELETE_BUTTON: &str = "Delete profile";
    pub const CANCEL: &str = "Cancel";
    pub const DUE_TOMORROW: &str = "Due tomorrow";
    pub const ITEM_SAVE: &str = "Save";
    pub const ITEM_BLANK: &str = "Text is required. The item was not changed.";
    pub const ALIAS_INPUT_LABEL: &str = "New alias";
    pub const UNDO_DISMISS: &str = "Dismiss";
    pub const LOADING: &str = "Loading contacts… Profiles will appear as they are saved locally.";
    pub const EMPTY: &str =
        "No people yet. Add one above, or profiles populate after your first meeting summary completes.";

    // Unsubmitted relationship drafts (web `RelationshipDraftStatus`).
    pub const DRAFTS_LABEL: &str = "Unsubmitted relationship drafts";
    /// Web keeps drafts per tab.
    pub const DRAFTS_KEPT_TAB: &str = "Kept only in this tab. Reopen a contact to Save details or Add its item. Nothing here is saved automatically.";
    /// Desktop keeps drafts until the app quits.
    pub const DRAFTS_KEPT_APP: &str = "Kept only until you quit Bruhs. Reopen a contact to Save details or Add its item. Nothing here is saved automatically.";
    pub const DRAFT_OPEN: &str = "Open draft";
    pub const DRAFT_DISCARD: &str = "Discard draft";
    pub const DRAFTS_TRUNCATED: &str = "Showing the first 24 drafts. Save or discard these to reach the rest; private recovery includes all drafts.";
    pub const DRAFTS_VIEW: &str = "View unsubmitted drafts (private)";
    pub const DRAFTS_HIDE: &str = "Hide unsubmitted drafts";
    pub const DRAFTS_COPY_NOTE: &str = "Copy somewhere private before discarding or leaving. This includes unsaved notes and original comparison fields.";
    pub const DRAFTS_RECOVERY_LABEL: &str = "Unsubmitted relationship recovery data";
    /// Save details refused: a field it changes was also saved elsewhere.
    pub const DETAILS_CHANGED: &str = "Saved details changed while you were editing. Your draft is kept. Copy it before discarding to review the latest details.";
    /// A merge refused while either contact has unsaved relationship edits.
    pub const MERGE_DRAFTS_UNSAVED: &str =
        "Save the unsaved relationship details before merging these contacts.";

    /// "1 unsubmitted relationship draft", "3 unsubmitted relationship drafts".
    pub fn drafts_status(count: usize) -> String {
        match count {
            1 => "1 unsubmitted relationship draft".to_string(),
            count => format!("{count} unsubmitted relationship drafts"),
        }
    }

    /// A draft whose contact no longer exists.
    pub fn draft_unavailable_contact(id: &str) -> String {
        format!("Unavailable contact ({id})")
    }

    pub fn draft_open_label(name: &str) -> String {
        format!("Open draft for {name}")
    }

    pub fn draft_discard_label(name: &str) -> String {
        format!("Discard draft for {name}")
    }

    /// The merge confirmation sentence.
    pub fn merge_confirm(source: &str, target: &str) -> String {
        format!(
            "Merge {source} into {target}? Aliases, observations, reminders, voice and links move to {target}. You can undo right after."
        )
    }

    /// The delete confirmation sentence.
    pub fn delete_confirm(name: &str) -> String {
        format!(
            "Delete {name}? Their observations, aliases and reminders are deleted. Meetings and Inbox conversations stay, unlinked. You can undo right after."
        )
    }

    /// "Page 2 of 5".
    pub fn page_status(page: usize, count: usize) -> String {
        format!("Page {} of {}", page.saturating_add(1), count)
    }

    /// A linked meeting's label: its title, or Untitled meeting.
    pub fn meeting_link_title(title: &str) -> &str {
        if title.trim().is_empty() {
            UNTITLED_MEETING
        } else {
            title
        }
    }
}

/// Every name a profile answers to: the display name, then its aliases.
pub fn person_names(person: &PersonProfile) -> impl Iterator<Item = &str> {
    std::iter::once(person.display_name.as_str()).chain(person.aliases.iter().map(String::as_str))
}

/// Whether the profile's display name or any alias equals `name` after case
/// folding.
pub fn person_answers_to(person: &PersonProfile, name: &str) -> bool {
    let key = person_name_key(name.trim());
    !key.is_empty() && person_names(person).any(|candidate| person_name_key(candidate) == key)
}

/// People page search: the query is a case-folded substring of the display
/// name or of any alias. A blank query matches everyone.
pub fn person_matches_query(person: &PersonProfile, query: &str) -> bool {
    names_match_query(person_names(person), query)
}

/// [`person_matches_query`] over a list of names, for directories that keep
/// only a profile's names.
pub fn names_match_query<'a>(names: impl IntoIterator<Item = &'a str>, query: &str) -> bool {
    let query = person_name_key(query.trim());
    query.is_empty()
        || names
            .into_iter()
            .any(|name| person_name_key(name).contains(&query))
}

/// The number of directory pages for `len` people. Never zero, so an empty
/// result still has a page to show its empty state on.
pub fn people_page_count(len: usize) -> usize {
    len.div_ceil(PEOPLE_PAGE_SIZE).max(1)
}

/// The page that holds position `index`.
pub fn people_page_of(index: usize) -> usize {
    index / PEOPLE_PAGE_SIZE
}

/// One directory page of `items`. A page past the end is empty.
pub fn people_page<T>(items: &[T], page: usize) -> &[T] {
    let start = page.saturating_mul(PEOPLE_PAGE_SIZE).min(items.len());
    let end = start.saturating_add(PEOPLE_PAGE_SIZE).min(items.len());
    &items[start..end]
}

/// The one profile whose display name or alias matches `name`. `None` when no
/// profile matches, or when two or more do: an ambiguous name never links.
pub fn unique_person_named<'a>(
    people: impl IntoIterator<Item = &'a PersonProfile>,
    name: &str,
) -> Option<&'a PersonProfile> {
    let mut matches = people
        .into_iter()
        .filter(|person| person_answers_to(person, name));
    let first = matches.next()?;
    matches.next().is_none().then_some(first)
}

/// Why a new alias was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AliasProblem {
    /// The alias is blank after trimming.
    Blank,
    /// The profile already answers to this name.
    AlreadyOnProfile,
    /// Another profile answers to this name. Merging is the way to join them.
    UsedBy {
        person_id: String,
        display_name: String,
    },
}

impl AliasProblem {
    /// The user-facing sentence.
    pub fn message(&self) -> String {
        match self {
            Self::Blank => "An alias is required. Nothing was added.".into(),
            Self::AlreadyOnProfile => {
                "This profile already uses that name. Nothing was added.".into()
            }
            Self::UsedBy { display_name, .. } => format!(
                "{display_name} already uses that name. Merge the two profiles or choose another alias. Nothing was added."
            ),
        }
    }
}

impl std::fmt::Display for AliasProblem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message())
    }
}

/// Review a new alias for `person_id` against every profile. Returns the
/// trimmed alias to add.
pub fn review_alias<'a>(
    people: impl IntoIterator<Item = &'a PersonProfile>,
    person_id: &str,
    alias: &str,
) -> Result<String, AliasProblem> {
    let alias = alias.trim();
    if alias.is_empty() {
        return Err(AliasProblem::Blank);
    }
    for person in people {
        if !person_answers_to(person, alias) {
            continue;
        }
        if person.id == person_id {
            return Err(AliasProblem::AlreadyOnProfile);
        }
        return Err(AliasProblem::UsedBy {
            person_id: person.id.clone(),
            display_name: person.display_name.clone(),
        });
    }
    Ok(alias.to_string())
}

/// The alias list to save for `person`: trimmed, without blanks, without the
/// display name, and without case-folded duplicates. Order is kept.
pub fn normalize_aliases(display_name: &str, aliases: &[String]) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    seen.insert(person_name_key(display_name.trim()));
    aliases
        .iter()
        .map(|alias| alias.trim())
        .filter(|alias| !alias.is_empty() && seen.insert(person_name_key(alias)))
        .map(str::to_string)
        .collect()
}

/// Meetings that involve the person, newest first: a linked speaker, an
/// assigned action item, or an observation sourced from the meeting.
pub fn person_meetings<'a>(person: &PersonProfile, meetings: &'a [Meeting]) -> Vec<&'a Meeting> {
    let observed = person
        .observations
        .iter()
        .map(|observation| observation.at_meeting_id.as_str())
        .collect::<std::collections::HashSet<_>>();
    let mut found = meetings
        .iter()
        .filter(|meeting| {
            observed.contains(meeting.id.as_str())
                || meeting
                    .speakers
                    .iter()
                    .any(|speaker| speaker.person_id.as_deref() == Some(person.id.as_str()))
                || meeting.summary.as_ref().is_some_and(|summary| {
                    summary
                        .action_items
                        .iter()
                        .any(|item| item.assignee_person_id.as_deref() == Some(person.id.as_str()))
                })
        })
        .collect::<Vec<_>>();
    found.sort_by(|a, b| {
        b.started_at_ms
            .total_cmp(&a.started_at_ms)
            .then_with(|| a.id.cmp(&b.id))
    });
    found
}

/// Inbox conversations with a participant linked to the person, newest first.
pub fn person_conversations<'a>(
    person_id: &str,
    conversations: &'a [InboxConversation],
) -> Vec<&'a InboxConversation> {
    let mut found = conversations
        .iter()
        .filter(|conversation| {
            conversation
                .participants
                .iter()
                .any(|participant| participant.person_id.as_deref() == Some(person_id))
        })
        .collect::<Vec<_>>();
    found.sort_by(|a, b| {
        b.updated_at_ms
            .cmp(&a.updated_at_ms)
            .then_with(|| a.id.cmp(&b.id))
    });
    found
}

/// One Inbox participant that pointed at a person.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ParticipantRef {
    pub conversation_id: String,
    pub participant_id: String,
}

/// One knowledge link that pointed at a person, kept whole so it can be put
/// back exactly.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DocumentLinkRef {
    pub document_id: String,
    pub link: KnowledgeLink,
}

/// Private links a People change touched outside the contact record, so undo
/// can put back exactly those links and no others.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PersonLinkSet {
    #[serde(default)]
    pub participants: Vec<ParticipantRef>,
    #[serde(default)]
    pub document_links: Vec<DocumentLinkRef>,
}

impl PersonLinkSet {
    pub fn is_empty(&self) -> bool {
        self.participants.is_empty() && self.document_links.is_empty()
    }

    /// Conversations this set names, without duplicates.
    pub fn conversation_ids(&self) -> std::collections::BTreeSet<&str> {
        self.participants
            .iter()
            .map(|participant| participant.conversation_id.as_str())
            .collect()
    }

    /// Documents this set names, without duplicates.
    pub fn document_ids(&self) -> std::collections::BTreeSet<&str> {
        self.document_links
            .iter()
            .map(|link| link.document_id.as_str())
            .collect()
    }
}

fn is_person_link(link: &KnowledgeLink, person_id: &str) -> bool {
    link.kind == KnowledgeLinkKind::Person && link.target_id == person_id
}

/// Delete cascade for private links: Inbox participants keep their name and
/// address but lose the person link, and documents lose only their link to
/// the person. Nothing else is removed. Returns what changed.
pub fn unlink_person(
    person_id: &str,
    conversations: &mut [InboxConversation],
    documents: &mut [KnowledgeDocument],
) -> PersonLinkSet {
    let mut changed = PersonLinkSet::default();
    for conversation in conversations.iter_mut() {
        for participant in &mut conversation.participants {
            if participant.person_id.as_deref() == Some(person_id) {
                participant.person_id = None;
                changed.participants.push(ParticipantRef {
                    conversation_id: conversation.id.clone(),
                    participant_id: participant.id.clone(),
                });
            }
        }
    }
    for document in documents.iter_mut() {
        let mut kept = Vec::with_capacity(document.links.len());
        for link in std::mem::take(&mut document.links) {
            if is_person_link(&link, person_id) {
                changed.document_links.push(DocumentLinkRef {
                    document_id: document.id.clone(),
                    link,
                });
            } else {
                kept.push(link);
            }
        }
        document.links = kept;
    }
    changed
}

/// Undo of [`unlink_person`]: relink only the recorded participants that are
/// still unlinked, and re-add only recorded links the document lacks.
/// Returns the links that were put back.
pub fn relink_person(
    person_id: &str,
    links: &PersonLinkSet,
    conversations: &mut [InboxConversation],
    documents: &mut [KnowledgeDocument],
) -> PersonLinkSet {
    let mut restored = PersonLinkSet::default();
    for reference in &links.participants {
        let participant = conversations
            .iter_mut()
            .filter(|conversation| conversation.id == reference.conversation_id)
            .flat_map(|conversation| conversation.participants.iter_mut())
            .find(|participant| participant.id == reference.participant_id);
        if let Some(participant) = participant.filter(|p| p.person_id.is_none()) {
            participant.person_id = Some(person_id.to_string());
            restored.participants.push(reference.clone());
        }
    }
    for reference in &links.document_links {
        if !is_person_link(&reference.link, person_id) {
            continue;
        }
        let Some(document) = documents
            .iter_mut()
            .find(|document| document.id == reference.document_id)
        else {
            continue;
        };
        if !document.links.contains(&reference.link) {
            document.links.push(reference.link.clone());
            restored.document_links.push(reference.clone());
        }
    }
    restored
}

/// Merge cascade for private links: every participant and knowledge link that
/// pointed at `from` now points at `to`. Returns the links as they were.
pub fn retarget_person_links(
    from: &str,
    to: &str,
    conversations: &mut [InboxConversation],
    documents: &mut [KnowledgeDocument],
) -> PersonLinkSet {
    let mut changed = PersonLinkSet::default();
    for conversation in conversations.iter_mut() {
        for participant in &mut conversation.participants {
            if participant.person_id.as_deref() == Some(from) {
                participant.person_id = Some(to.to_string());
                changed.participants.push(ParticipantRef {
                    conversation_id: conversation.id.clone(),
                    participant_id: participant.id.clone(),
                });
            }
        }
    }
    for document in documents.iter_mut() {
        for link in &mut document.links {
            if is_person_link(link, from) {
                changed.document_links.push(DocumentLinkRef {
                    document_id: document.id.clone(),
                    link: link.clone(),
                });
                link.target_id = to.to_string();
            }
        }
    }
    changed
}

/// Undo of [`retarget_person_links`]: recorded links that still point at `to`
/// point back at `from`. Links added or edited since are left alone. Returns
/// the links that were restored.
pub fn restore_person_links(
    from: &str,
    to: &str,
    links: &PersonLinkSet,
    conversations: &mut [InboxConversation],
    documents: &mut [KnowledgeDocument],
) -> PersonLinkSet {
    let mut restored = PersonLinkSet::default();
    for reference in &links.participants {
        let participant = conversations
            .iter_mut()
            .filter(|conversation| conversation.id == reference.conversation_id)
            .flat_map(|conversation| conversation.participants.iter_mut())
            .find(|participant| participant.id == reference.participant_id);
        if let Some(participant) = participant.filter(|p| p.person_id.as_deref() == Some(to)) {
            participant.person_id = Some(from.to_string());
            restored.participants.push(reference.clone());
        }
    }
    for reference in &links.document_links {
        if !is_person_link(&reference.link, from) {
            continue;
        }
        let Some(document) = documents
            .iter_mut()
            .find(|document| document.id == reference.document_id)
        else {
            continue;
        };
        let retargeted = KnowledgeLink {
            target_id: to.to_string(),
            ..reference.link.clone()
        };
        if let Some(link) = document.links.iter_mut().find(|link| **link == retargeted) {
            link.target_id = from.to_string();
            restored.document_links.push(reference.clone());
        }
    }
    restored
}

/// Drop every observation sourced from `meeting_id`. Returns how many went.
/// This is the PEOPLE-14 rule: deleting a meeting deletes the observations
/// derived from it.
pub fn drop_meeting_observations(person: &mut PersonProfile, meeting_id: &str) -> usize {
    let before = person.observations.len();
    person
        .observations
        .retain(|observation| observation.at_meeting_id != meeting_id);
    before - person.observations.len()
}

#[cfg(test)]
mod tests;

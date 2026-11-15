use super::{MeetingStoreError, MeetingStoreResult};
use bruhs_models::{
    normalize_aliases, person_name_key, person_names, review_alias, ActionItemRef, Meeting,
    MeetingSpeakerRef, PersonDeletion, PersonProfile,
};
use serde::{Deserialize, Serialize};

pub(super) fn next_version(current: i64) -> MeetingStoreResult<i64> {
    current
        .checked_add(1)
        .map(|next| next.max(chrono::Utc::now().timestamp_millis()))
        .ok_or_else(|| {
            MeetingStoreError::Conflict("Record version is exhausted; no change was saved.".into())
        })
}

pub(super) fn merged_profile(
    source: &PersonProfile,
    target: &PersonProfile,
) -> MeetingStoreResult<PersonProfile> {
    let version = next_version(target.updated_at.max(source.updated_at))?;
    let mut merged = target.clone();
    merged
        .observations
        .extend(source.observations.iter().cloned());
    if merged.voice_embedding.is_none() {
        merged.voice_embedding.clone_from(&source.voice_embedding);
    }
    let mut names = merged
        .aliases
        .iter()
        .map(|name| name.to_lowercase())
        .collect::<std::collections::HashSet<_>>();
    names.insert(merged.display_name.to_lowercase());
    for alias in std::iter::once(&source.display_name).chain(&source.aliases) {
        if names.insert(alias.to_lowercase()) {
            merged.aliases.push(alias.clone());
        }
    }
    merged.tracked &= source.tracked;
    merged.updated_at = version;
    Ok(merged)
}

pub(super) fn validate_person(person: &PersonProfile) -> MeetingStoreResult<()> {
    if person.id.trim().is_empty()
        || person.display_name.trim().is_empty()
        || person
            .voice_embedding
            .as_ref()
            .is_some_and(|values| values.iter().any(|v| !v.is_finite()))
    {
        return Err(MeetingStoreError::InvalidInput(
            "Contact identity, name and voice values must be valid.".into(),
        ));
    }
    Ok(())
}
pub(super) fn validate_resolution_ids(ids: &[String]) -> MeetingStoreResult<()> {
    let mut unique = std::collections::HashSet::new();
    if ids.len() > 64
        || ids
            .iter()
            .any(|id| id.trim().is_empty() || id.len() > 1024 || !unique.insert(id))
    {
        return Err(MeetingStoreError::InvalidInput(
            "Contact lookup requires at most 64 unique, nonempty contact IDs.".into(),
        ));
    }
    Ok(())
}

// ── Delete, restore and unmerge (PEOPLE-08, PEOPLE-09, PEOPLE-10) ───────────

/// A speaker a merge moved from the source to the target. `label` is the
/// speaker's label before the merge when the merge rewrote it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct MovedSpeaker {
    pub meeting_id: String,
    pub speaker_id: String,
    #[serde(default)]
    pub label: Option<String>,
}

/// Meeting links a merge moved, kept so the merge can be undone exactly.
/// Merges saved by older builds have no record and restore profiles only.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct MergeLinks {
    #[serde(default)]
    pub speakers: Vec<MovedSpeaker>,
    #[serde(default)]
    pub action_items: Vec<ActionItemRef>,
}

impl MergeLinks {
    pub(super) fn extend(&mut self, other: Self) {
        self.speakers.extend(other.speakers);
        self.action_items.extend(other.action_items);
    }

    pub(super) fn meeting_ids(&self) -> std::collections::BTreeSet<&str> {
        self.speakers
            .iter()
            .map(|speaker| speaker.meeting_id.as_str())
            .chain(self.action_items.iter().map(|item| item.meeting_id.as_str()))
            .collect()
    }
}

/// One saved merge redirect with the originals it replaced.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ArchivedMerge {
    pub source_id: String,
    pub target_id: String,
    pub source_original: PersonProfile,
    pub target_original: PersonProfile,
    #[serde(default)]
    pub links: Option<MergeLinks>,
}

/// The server-side record of a deleted contact. Restore needs only the id.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct DeletionArchive {
    pub deletion: PersonDeletion,
    /// Merge redirects that ended at the deleted contact. They were removed
    /// so retired ids resolve as unknown instead of failing.
    #[serde(default)]
    pub merges: Vec<ArchivedMerge>,
}

/// Unlink `person_id` from one meeting: speakers keep their label, action
/// items keep their text. Returns the refs that changed.
pub(super) fn unlink_meeting(
    meeting: &mut Meeting,
    person_id: &str,
) -> (Vec<MeetingSpeakerRef>, Vec<ActionItemRef>) {
    let mut speakers = Vec::new();
    for speaker in &mut meeting.speakers {
        if speaker.person_id.as_deref() == Some(person_id) {
            speaker.person_id = None;
            speakers.push(MeetingSpeakerRef {
                meeting_id: meeting.id.clone(),
                speaker_id: speaker.id.clone(),
            });
        }
    }
    let mut items = Vec::new();
    if let Some(summary) = &mut meeting.summary {
        for (index, item) in summary.action_items.iter_mut().enumerate() {
            if item.assignee_person_id.as_deref() == Some(person_id) {
                item.assignee_person_id = None;
                items.push(ActionItemRef {
                    meeting_id: meeting.id.clone(),
                    index,
                    text: item.text.clone(),
                });
            }
        }
    }
    (speakers, items)
}

/// Undo of [`unlink_meeting`] for one meeting: relink recorded speakers that
/// are still unlinked, and reassign recorded items whose text is unchanged
/// and which nobody else took. Returns whether anything changed.
pub(super) fn relink_meeting(
    meeting: &mut Meeting,
    person_id: &str,
    speakers: &[MeetingSpeakerRef],
    items: &[ActionItemRef],
) -> bool {
    let mut changed = false;
    for reference in speakers.iter().filter(|r| r.meeting_id == meeting.id) {
        if let Some(speaker) = meeting
            .speakers
            .iter_mut()
            .find(|speaker| speaker.id == reference.speaker_id && speaker.person_id.is_none())
        {
            speaker.person_id = Some(person_id.to_string());
            changed = true;
        }
    }
    if let Some(summary) = &mut meeting.summary {
        for reference in items.iter().filter(|r| r.meeting_id == meeting.id) {
            if let Some(item) = summary.action_items.get_mut(reference.index).filter(|item| {
                item.text == reference.text && item.assignee_person_id.is_none()
            }) {
                item.assignee_person_id = Some(person_id.to_string());
                changed = true;
            }
        }
    }
    changed
}

/// Merge cascade for one meeting: speakers and action items of `source` move
/// to `target`. A speaker labeled with the source's name takes the target's.
pub(super) fn retarget_meeting(
    meeting: &mut Meeting,
    source: &PersonProfile,
    target: &PersonProfile,
) -> MergeLinks {
    let mut links = MergeLinks::default();
    for speaker in &mut meeting.speakers {
        if speaker.person_id.as_deref() == Some(source.id.as_str()) {
            speaker.person_id = Some(target.id.clone());
            let label = (speaker.label == source.display_name).then(|| {
                std::mem::replace(&mut speaker.label, target.display_name.clone())
            });
            links.speakers.push(MovedSpeaker {
                meeting_id: meeting.id.clone(),
                speaker_id: speaker.id.clone(),
                label,
            });
        }
    }
    if let Some(summary) = &mut meeting.summary {
        for (index, item) in summary.action_items.iter_mut().enumerate() {
            if item.assignee_person_id.as_deref() == Some(source.id.as_str()) {
                item.assignee_person_id = Some(target.id.clone());
                links.action_items.push(ActionItemRef {
                    meeting_id: meeting.id.clone(),
                    index,
                    text: item.text.clone(),
                });
            }
        }
    }
    links
}

/// Undo of [`retarget_meeting`] for one meeting: recorded links that still
/// point at `target_id` point back at `source_id`. A label the merge rewrote
/// returns only while it still shows `merged_label`, the name the merge wrote.
/// Returns whether anything changed.
pub(super) fn restore_meeting_links(
    meeting: &mut Meeting,
    source_id: &str,
    target_id: &str,
    merged_label: &str,
    links: &MergeLinks,
) -> bool {
    let mut changed = false;
    for moved in links.speakers.iter().filter(|m| m.meeting_id == meeting.id) {
        if let Some(speaker) = meeting.speakers.iter_mut().find(|speaker| {
            speaker.id == moved.speaker_id && speaker.person_id.as_deref() == Some(target_id)
        }) {
            speaker.person_id = Some(source_id.to_string());
            if let Some(label) = &moved.label {
                if speaker.label == merged_label {
                    speaker.label.clone_from(label);
                }
            }
            changed = true;
        }
    }
    if let Some(summary) = &mut meeting.summary {
        for reference in links
            .action_items
            .iter()
            .filter(|r| r.meeting_id == meeting.id)
        {
            if let Some(item) = summary.action_items.get_mut(reference.index).filter(|item| {
                item.text == reference.text && item.assignee_person_id.as_deref() == Some(target_id)
            }) {
                item.assignee_person_id = Some(source_id.to_string());
                changed = true;
            }
        }
    }
    changed
}

/// The source of an undone merge, with a fresh version so every cache takes it.
pub(super) fn restored_source(original: &PersonProfile) -> MeetingStoreResult<PersonProfile> {
    let mut source = original.clone();
    source.updated_at = next_version(original.updated_at)?;
    Ok(source)
}

/// The target of an undone merge, taken from its current record so edits
/// made since the merge survive. Only what the merge brought is removed: one
/// copy of each source observation, aliases that came from the source, and
/// a voice copied from the source. The tracked flag returns to the target's
/// own value unless it was changed after the merge, so an opt-out on either
/// side stays sticky until the user changes it.
pub(super) fn unmerged_target(
    current: &PersonProfile,
    source_original: &PersonProfile,
    target_original: &PersonProfile,
) -> MeetingStoreResult<PersonProfile> {
    let mut target = current.clone();
    for observation in &source_original.observations {
        if let Some(index) = target
            .observations
            .iter()
            .position(|candidate| candidate == observation)
        {
            target.observations.remove(index);
        }
    }
    let own = person_names(target_original)
        .map(person_name_key)
        .collect::<std::collections::HashSet<_>>();
    let brought = person_names(source_original)
        .map(person_name_key)
        .filter(|key| !own.contains(key))
        .collect::<std::collections::HashSet<_>>();
    target
        .aliases
        .retain(|alias| !brought.contains(&person_name_key(alias)));
    if target_original.voice_embedding.is_none()
        && target.voice_embedding.is_some()
        && target.voice_embedding == source_original.voice_embedding
    {
        target.voice_embedding = None;
    }
    if current.tracked == (target_original.tracked && source_original.tracked) {
        target.tracked = target_original.tracked;
    }
    target.updated_at = next_version(current.updated_at)?;
    Ok(target)
}

/// The alias list to save for `current`. Aliases the profile did not already
/// answer to must not be a name of any other profile.
pub(super) fn aliases_to_save<'a>(
    current: &PersonProfile,
    requested: &[String],
    people: impl Iterator<Item = &'a PersonProfile> + Clone,
) -> MeetingStoreResult<Vec<String>> {
    let next = normalize_aliases(&current.display_name, requested);
    let existing = person_names(current)
        .map(person_name_key)
        .collect::<std::collections::HashSet<_>>();
    for alias in next
        .iter()
        .filter(|alias| !existing.contains(&person_name_key(alias)))
    {
        review_alias(
            people.clone().filter(|person| person.id != current.id),
            &current.id,
            alias,
        )
        .map_err(|problem| MeetingStoreError::Conflict(problem.message()))?;
    }
    Ok(next)
}

/// Sources of saved merges whose redirect chain ends at `person_id`.
pub(super) fn merges_ending_at<'a>(
    merges: impl Iterator<Item = (&'a str, &'a str)> + Clone,
    person_id: &str,
) -> std::collections::BTreeSet<String> {
    let mut ending = std::collections::BTreeSet::new();
    let mut frontier = vec![person_id.to_string()];
    while let Some(target) = frontier.pop() {
        for (source, _) in merges.clone().filter(|(_, to)| *to == target) {
            if source != person_id && ending.insert(source.to_string()) {
                frontier.push(source.to_string());
            }
        }
    }
    ending
}

pub(super) fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

pub(super) const UNMERGE_TARGET_GONE: &str =
    "The surviving contact was merged or deleted after this merge, so it cannot be undone. No records were changed.";
pub(super) const UNMERGE_SOURCE_LIVE: &str =
    "The merged contact already exists again. No records were changed.";
pub(super) const UNMERGE_SOURCE_TARGETED: &str =
    "Another contact was merged into this one, so this merge cannot be undone. No records were changed.";
pub(super) const RESTORE_LIVE: &str =
    "A contact with this ID exists again, so the deleted one cannot be restored. No records were changed.";

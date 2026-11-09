//! One generation result, committed only against the input it actually summarized.
use super::{person_changes::next_version, MeetingStoreError, MeetingStoreResult};
use bruhs_models::{Meeting, MeetingSummary, PersonObservation, PersonProfile, TranscriptSegment};
use std::collections::{BTreeMap, HashMap};

pub struct SummaryUpdate {
    pub expected: Meeting,
    pub people_versions: BTreeMap<String, i64>,
    pub transcript: Vec<TranscriptSegment>,
    pub summary: MeetingSummary,
    pub observations: HashMap<String, Vec<PersonObservation>>,
    pub replace_observations: bool,
}

pub(super) fn is_summary_contact(person: &PersonProfile, meeting: &Meeting) -> bool {
    person
        .observations
        .iter()
        .any(|o| o.at_meeting_id == meeting.id)
        || meeting.speakers.iter().any(|s| {
            s.person_id.as_ref() == Some(&person.id)
                || person.display_name.eq_ignore_ascii_case(&s.label)
                || person
                    .aliases
                    .iter()
                    .any(|alias| alias.eq_ignore_ascii_case(&s.label))
        })
}

pub(super) fn versions<'a>(
    people: impl Iterator<Item = &'a PersonProfile>,
    meeting: &Meeting,
) -> BTreeMap<String, i64> {
    people
        .filter(|p| is_summary_contact(p, meeting))
        .map(|p| (p.id.clone(), p.updated_at))
        .collect()
}

impl SummaryUpdate {
    pub(super) fn validate(
        &self,
        current: &Meeting,
        segments: &[TranscriptSegment],
    ) -> MeetingStoreResult<()> {
        if current != &self.expected || segments != self.transcript {
            return Err(MeetingStoreError::Conflict("Meeting or transcript changed during generation; retry. Existing summary and observations were retained.".into()));
        }
        let invalid = self.transcript.is_empty()
            || self.transcript.iter().any(|s| s.partial)
            || (current.skip_observations
                && (self.replace_observations || !self.observations.is_empty()))
            || self.summary.tldr.trim().is_empty()
            || self.summary.tldr.len() > 64 * 1024
            || self.summary.action_items.len() > 128
            || self.summary.decisions.len() > 128
            || self.summary.topics.len() > 128
            || self.summary.action_items.iter().any(|item| {
                item.text.len() > 8192
                    || item.due_hint.as_ref().is_some_and(|hint| hint.len() > 8192)
            })
            || self.summary.decisions.iter().any(|text| text.len() > 8192)
            || self.summary.topics.iter().any(|topic| {
                topic.title.len() > 8192
                    || topic.bullets.len() > 128
                    || topic.bullets.iter().any(|text| text.len() > 8192)
            })
            || self.observations.len() > 128
            || self.observations.iter().any(|(label, observations)| {
                label.trim().is_empty()
                    || observations.len() > 8
                    || observations.iter().any(|o| {
                        o.at_meeting_id != current.id
                            || o.text.trim().is_empty()
                            || o.text.len() > 8192
                    })
            });
        if invalid {
            return Err(MeetingStoreError::InvalidInput(
                "Invalid summary result; no summary or observations were replaced.".into(),
            ));
        }
        for item in &self.summary.action_items {
            if let Some(id) = &item.assignee_person_id {
                if !current
                    .speakers
                    .iter()
                    .any(|speaker| speaker.person_id.as_ref() == Some(id))
                {
                    return Err(MeetingStoreError::InvalidInput(
                        "Summary assignee is not a linked meeting participant.".into(),
                    ));
                }
            }
        }
        Ok(())
    }

    /// Stage affected contacts without changing the source collection. Untracked
    /// contacts retain their old observations, including during replacement.
    pub(super) fn stage_people<'a>(
        &self,
        people: impl Iterator<Item = &'a PersonProfile> + Clone,
    ) -> MeetingStoreResult<BTreeMap<String, PersonProfile>> {
        if versions(people.clone(), &self.expected) != self.people_versions {
            return Err(MeetingStoreError::Conflict("Participants or their observations changed during generation; retry without replacing existing data.".into()));
        }
        let mut changed = BTreeMap::new();
        if self.replace_observations {
            for person in people.clone().filter(|p| {
                p.tracked
                    && p.observations
                        .iter()
                        .any(|o| o.at_meeting_id == self.expected.id)
            }) {
                let mut next = person.clone();
                next.observations
                    .retain(|o| o.at_meeting_id != self.expected.id);
                changed.insert(next.id.clone(), next);
            }
        }
        for (label, observations) in &self.observations {
            let mut speakers = self.expected.speakers.iter().filter(|s| s.label == *label);
            let speaker = speakers.next().ok_or_else(|| {
                MeetingStoreError::InvalidInput(
                    "Observation references an unknown speaker label.".into(),
                )
            })?;
            if speakers.next().is_some() {
                return Err(MeetingStoreError::Conflict(
                    "Speaker label is ambiguous; rename or link the speaker before retrying."
                        .into(),
                ));
            }
            let mut matches = people.clone().filter(|p| match &speaker.person_id {
                Some(id) => &p.id == id,
                None => {
                    p.display_name.eq_ignore_ascii_case(label)
                        || p.aliases.iter().any(|a| a.eq_ignore_ascii_case(label))
                }
            });
            let matched = matches.next();
            if matches.next().is_some() {
                return Err(MeetingStoreError::Conflict(
                    "Contact label is ambiguous; link the speaker before retrying.".into(),
                ));
            }
            let mut next = match matched {
                Some(person) if !person.tracked => continue,
                Some(person) => changed.remove(&person.id).unwrap_or_else(|| person.clone()),
                None if speaker.person_id.is_some() => {
                    return Err(MeetingStoreError::Conflict(
                        "Linked participant is no longer available; reload before retrying.".into(),
                    ))
                }
                None => {
                    // Two case variants of a new speaker name must not mint
                    // separate contacts in the same generation transaction.
                    let staged = changed
                        .values()
                        .find(|p| p.display_name.eq_ignore_ascii_case(label))
                        .map(|p| p.id.clone());
                    match staged.and_then(|id| changed.remove(&id)) {
                        Some(person) => person,
                        None => PersonProfile::new(label),
                    }
                }
            };
            next.observations.extend(observations.iter().cloned());
            changed.insert(next.id.clone(), next);
        }
        for person in changed.values_mut() {
            person.updated_at = next_version(person.updated_at)?;
            super::person_changes::validate_person(person)?;
        }
        Ok(changed)
    }
}

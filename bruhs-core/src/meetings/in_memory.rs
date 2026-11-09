//! In-memory implementation of `MeetingStore`. Suitable for the dev server
//! and tests; swap a real backend in when persistence across restarts is needed.

use std::collections::{BTreeMap, HashMap, HashSet};

use tokio::sync::RwLock;

use bruhs_models::{Meeting, PersonProfile, TranscriptSegment};

use super::error::{MeetingStoreError, MeetingStoreResult};
use super::person_changes::{merged_profile, next_version, validate_person};
use super::{MeetingRecords, PeopleStore, SpeakerLinkJobs, SummaryGenerationStore};

mod generation;

pub struct InMemoryMeetingStore {
    pub(super) meetings: RwLock<HashMap<String, Meeting>>,
    pub(super) segments: RwLock<HashMap<String, Vec<TranscriptSegment>>>,
    removals: RwLock<HashMap<String, HashSet<String>>>,
    pub(super) people: RwLock<BTreeMap<String, PersonProfile>>,
    merged_people: RwLock<HashMap<String, String>>,
    generations: RwLock<HashMap<String, bruhs_models::SummaryGenerationReceipt>>,
    link_jobs: RwLock<HashMap<String, super::SpeakerLinkJob>>,
    local_focus: RwLock<HashMap<String, Vec<bruhs_models::LocalFocusObservation>>>,
}

impl InMemoryMeetingStore {
    pub fn new() -> Self {
        Self {
            meetings: RwLock::new(HashMap::new()),
            segments: RwLock::new(HashMap::new()),
            removals: RwLock::new(HashMap::new()),
            people: RwLock::new(BTreeMap::new()),
            merged_people: RwLock::new(HashMap::new()),
            generations: RwLock::new(HashMap::new()),
            link_jobs: RwLock::new(HashMap::new()),
            local_focus: RwLock::new(HashMap::new()),
        }
    }
}

impl Default for InMemoryMeetingStore {
    fn default() -> Self {
        Self::new()
    }
}

/// Dot-product similarity for **already L2-normalized** vectors.
/// Equivalent to cosine similarity but skips the normalization step —
/// only valid when both `a` and `b` are unit vectors.
/// Debug-asserts that each input has unit length (within ±1 %).
pub(super) fn cosine_similarity_unit(a: &[f32], b: &[f32]) -> f32 {
    debug_assert!(
        {
            let norm_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
            (norm_a - 1.0).abs() < 0.01
        },
        "cosine_similarity_unit: `a` is not unit-length (norm ≠ 1)"
    );
    debug_assert!(
        {
            let norm_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
            (norm_b - 1.0).abs() < 0.01
        },
        "cosine_similarity_unit: `b` is not unit-length (norm ≠ 1)"
    );
    a.iter().zip(b.iter()).map(|(x, y)| x * y).sum()
}

#[async_trait::async_trait]
impl MeetingRecords for InMemoryMeetingStore {
    async fn apply_capture_failure(
        &self,
        receipt: &bruhs_models::CaptureFailureReceipt,
    ) -> MeetingStoreResult<bool> {
        let mut meetings = self.meetings.write().await;
        let meeting = meetings.get_mut(&receipt.meeting_id).ok_or_else(|| {
            MeetingStoreError::MeetingNotFound {
                id: receipt.meeting_id.clone(),
            }
        })?;
        let applied = super::capture_failures::apply(meeting, receipt);
        drop(meetings);
        applied
    }

    async fn append_local_focus(
        &self,
        observation: bruhs_models::LocalFocusObservation,
    ) -> MeetingStoreResult<()> {
        if !observation.is_valid() {
            return Err(MeetingStoreError::InvalidInput(
                "Invalid local focus metadata.".into(),
            ));
        }
        let meetings = self.meetings.read().await;
        if !meetings.contains_key(&observation.meeting_id) {
            return Err(MeetingStoreError::MeetingNotFound {
                id: observation.meeting_id,
            });
        }
        let mut all = self.local_focus.write().await;
        if let Some(existing) = all.values().flatten().find(|row| row.id == observation.id) {
            return if existing == &observation {
                Ok(())
            } else {
                Err(MeetingStoreError::Conflict(
                    "Local focus identity already used.".into(),
                ))
            };
        }
        let rows = all.entry(observation.meeting_id.clone()).or_default();
        if rows.len() >= bruhs_models::MAX_LOCAL_FOCUS_OBSERVATIONS {
            return Err(MeetingStoreError::Conflict(
                "Local focus timeline is full; tracking must stop.".into(),
            ));
        }
        rows.push(observation);
        // The meeting read lock is held until the row is in, so a concurrent
        // delete cannot leave an orphaned focus timeline.
        drop(all);
        drop(meetings);
        Ok(())
    }

    async fn list_local_focus(
        &self,
        meeting_id: &str,
    ) -> MeetingStoreResult<Vec<bruhs_models::LocalFocusObservation>> {
        let mut rows = self
            .local_focus
            .read()
            .await
            .get(meeting_id)
            .cloned()
            .unwrap_or_default();
        rows.sort_by(|a, b| (a.observed_at_ms, &a.id).cmp(&(b.observed_at_ms, &b.id)));
        Ok(rows)
    }

    async fn create_meeting(&self, meeting: Meeting) -> MeetingStoreResult<Meeting> {
        self.meetings
            .write()
            .await
            .insert(meeting.id.clone(), meeting.clone());
        Ok(meeting)
    }

    async fn get_meeting(&self, id: &str) -> MeetingStoreResult<Option<Meeting>> {
        Ok(self.meetings.read().await.get(id).cloned())
    }

    async fn list_meetings(&self) -> MeetingStoreResult<Vec<Meeting>> {
        let mut out: Vec<Meeting> = self.meetings.read().await.values().cloned().collect();
        out.sort_by(|a, b| {
            b.started_at_ms
                .partial_cmp(&a.started_at_ms)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        Ok(out)
    }

    async fn upsert_meeting(&self, meeting: Meeting) -> MeetingStoreResult<Meeting> {
        let mut meetings = self.meetings.write().await;
        if meetings
            .get(&meeting.id)
            .is_some_and(|current| current != &meeting)
        {
            return Err(MeetingStoreError::Conflict(
                "Meeting already exists; use a field-specific update.".into(),
            ));
        }
        meetings.insert(meeting.id.clone(), meeting.clone());
        drop(meetings);
        Ok(meeting)
    }

    async fn patch_meeting(
        &self,
        id: &bruhs_models::ids::MeetingId,
        patch: super::MeetingPatch,
    ) -> MeetingStoreResult<Option<Meeting>> {
        let id = id.as_str();
        let mut map = self.meetings.write().await;
        let Some(m) = map.get_mut(id) else {
            return Ok(None);
        };
        patch.apply(m);
        m.updated_at += 1;
        let updated = m.clone();
        drop(map);
        Ok(Some(updated))
    }

    async fn patch_meeting_summary(
        &self,
        id: &bruhs_models::ids::MeetingId,
        summary: bruhs_models::MeetingSummary,
    ) -> MeetingStoreResult<Option<bruhs_models::Meeting>> {
        let id = id.as_str();
        let mut map = self.meetings.write().await;
        let Some(m) = map.get_mut(id) else {
            return Ok(None);
        };
        m.summary = Some(summary);
        m.updated_at += 1;
        let updated = m.clone();
        drop(map);
        Ok(Some(updated))
    }

    async fn patch_speaker_label(
        &self,
        meeting_id: &bruhs_models::ids::MeetingId,
        speaker_id: &bruhs_models::ids::SpeakerId,
        label: String,
    ) -> MeetingStoreResult<Meeting> {
        let meeting_id = meeting_id.as_str();
        let speaker_id = speaker_id.as_str();
        let mut map = self.meetings.write().await;
        let Some(m) = map.get_mut(meeting_id) else {
            return Err(MeetingStoreError::MeetingNotFound {
                id: meeting_id.to_string(),
            });
        };
        let Some(speaker) = m.speakers.iter_mut().find(|s| s.id == speaker_id) else {
            return Err(MeetingStoreError::SpeakerNotFound {
                meeting_id: meeting_id.to_string(),
                speaker_id: speaker_id.to_string(),
            });
        };
        speaker.label = label;
        m.updated_at += 1;
        let updated = m.clone();
        drop(map);
        Ok(updated)
    }

    async fn patch_meeting_thread_id(
        &self,
        meeting_id: &bruhs_models::ids::MeetingId,
        thread_id: &bruhs_models::ids::ThreadId,
    ) -> MeetingStoreResult<Meeting> {
        let meeting_id = meeting_id.as_str();
        let thread_id = thread_id.as_str();
        let mut map = self.meetings.write().await;
        let Some(m) = map.get_mut(meeting_id) else {
            return Err(MeetingStoreError::MeetingNotFound {
                id: meeting_id.to_string(),
            });
        };
        m.thread_id = Some(thread_id.into());
        m.updated_at += 1;
        let updated = m.clone();
        drop(map);
        Ok(updated)
    }

    async fn ensure_meeting_speakers(
        &self,
        meeting_id: &bruhs_models::ids::MeetingId,
        speakers: Vec<bruhs_models::Speaker>,
    ) -> MeetingStoreResult<Meeting> {
        let meeting_id = meeting_id.as_str();
        let mut map = self.meetings.write().await;
        let Some(m) = map.get_mut(meeting_id) else {
            return Err(MeetingStoreError::MeetingNotFound {
                id: meeting_id.to_string(),
            });
        };
        for speaker in speakers {
            if !m.speakers.iter().any(|s| s.id == speaker.id) {
                m.speakers.push(speaker);
            }
        }
        m.updated_at += 1;
        let updated = m.clone();
        drop(map);
        Ok(updated)
    }

    async fn patch_meeting_skip_observations(
        &self,
        meeting_id: &bruhs_models::ids::MeetingId,
        skip: bool,
    ) -> MeetingStoreResult<Meeting> {
        let meeting_id = meeting_id.as_str();
        let mut map = self.meetings.write().await;
        let Some(m) = map.get_mut(meeting_id) else {
            return Err(MeetingStoreError::MeetingNotFound {
                id: meeting_id.to_string(),
            });
        };
        m.skip_observations = skip;
        m.updated_at = chrono::Utc::now().timestamp_millis();
        let updated = m.clone();
        drop(map);
        Ok(updated)
    }

    async fn delete_meeting(&self, id: &bruhs_models::ids::MeetingId) -> MeetingStoreResult<bool> {
        let id = id.as_str();
        let mut generations = self.generations.write().await;
        let mut meetings = self.meetings.write().await;
        let removed = meetings.remove(id).is_some();
        self.link_jobs.write().await.remove(id);
        self.segments.write().await.remove(id);
        self.removals.write().await.remove(id);
        self.local_focus.write().await.remove(id);
        generations.retain(|_, receipt| receipt.request.meeting_id != id);
        // Both locks span every removal, so no reader sees a half-deleted meeting.
        drop(meetings);
        drop(generations);
        Ok(removed)
    }

    async fn append_segment(&self, segment: TranscriptSegment) -> MeetingStoreResult<()> {
        if self
            .removals
            .read()
            .await
            .get(&segment.meeting_id)
            .is_some_and(|ids| ids.contains(&segment.id))
        {
            return Ok(());
        }
        self.segments
            .write()
            .await
            .entry(segment.meeting_id.clone())
            .or_default()
            .push(segment);
        Ok(())
    }

    async fn remove_segment(&self, meeting_id: &str, segment_id: &str) -> MeetingStoreResult<()> {
        crate::meetings::valid_chunk_id(segment_id)?;
        if !self.meetings.read().await.contains_key(meeting_id) {
            return Err(MeetingStoreError::MeetingNotFound {
                id: meeting_id.into(),
            });
        }
        self.removals
            .write()
            .await
            .entry(meeting_id.to_owned())
            .or_default()
            .insert(segment_id.to_owned());
        if let Some(segments) = self.segments.write().await.get_mut(meeting_id) {
            segments.retain(|segment| segment.id != segment_id);
        }
        Ok(())
    }

    async fn segment_removed(
        &self,
        meeting_id: &str,
        segment_id: &str,
    ) -> MeetingStoreResult<bool> {
        Ok(self
            .removals
            .read()
            .await
            .get(meeting_id)
            .is_some_and(|ids| ids.contains(segment_id)))
    }

    async fn removed_segment_ids(&self, meeting_id: &str) -> MeetingStoreResult<Vec<String>> {
        if !self.meetings.read().await.contains_key(meeting_id) {
            return Err(MeetingStoreError::MeetingNotFound {
                id: meeting_id.into(),
            });
        }
        let mut ids = self
            .removals
            .read()
            .await
            .get(meeting_id)
            .map(|ids| ids.iter().cloned().collect::<Vec<_>>())
            .unwrap_or_default();
        ids.sort();
        Ok(ids)
    }

    async fn list_segments(&self, meeting_id: &str) -> MeetingStoreResult<Vec<TranscriptSegment>> {
        let removed = self
            .removals
            .read()
            .await
            .get(meeting_id)
            .cloned()
            .unwrap_or_default();
        let mut segments = self
            .segments
            .read()
            .await
            .get(meeting_id)
            .cloned()
            .unwrap_or_default();
        segments.retain(|segment| !removed.contains(&segment.id));
        segments.sort_unstable_by(|a, b| (a.start_ms, &a.id).cmp(&(b.start_ms, &b.id)));
        Ok(segments)
    }

    async fn cache_transcript(
        &self,
        meeting_id: &str,
        mut expected: Vec<TranscriptSegment>,
        mut incoming: Vec<TranscriptSegment>,
    ) -> MeetingStoreResult<Vec<TranscriptSegment>> {
        let removed = self
            .removals
            .read()
            .await
            .get(meeting_id)
            .cloned()
            .unwrap_or_default();
        expected.retain(|segment| !removed.contains(&segment.id));
        incoming.retain(|segment| !removed.contains(&segment.id));
        bruhs_models::validate_transcript_snapshot(meeting_id, &expected, &mut incoming)
            .map_err(|error| MeetingStoreError::Conflict(error.to_string()))?;
        let meetings = self.meetings.read().await;
        if !meetings.contains_key(meeting_id) {
            return Err(MeetingStoreError::MeetingNotFound {
                id: meeting_id.into(),
            });
        }
        let mut all = self.segments.write().await;
        let incoming_ids = incoming
            .iter()
            .map(|segment| segment.id.as_str())
            .collect::<std::collections::HashSet<_>>();
        if all.iter().any(|(id, segments)| {
            id != meeting_id
                && segments
                    .iter()
                    .any(|segment| incoming_ids.contains(segment.id.as_str()))
        }) {
            return Err(MeetingStoreError::Conflict(
                "Transcript identity belongs to another meeting.".into(),
            ));
        }
        let current = all.entry(meeting_id.into()).or_default();
        current.retain(|segment| !removed.contains(&segment.id));
        current.sort_unstable_by(|a, b| (a.start_ms, &a.id).cmp(&(b.start_ms, &b.id)));
        if *current != expected {
            return Err(MeetingStoreError::Conflict("Transcript changed while capture was being fetched; retry with the current saved copy.".into()));
        }
        current.clone_from(&incoming);
        // The meeting read lock spans the write so a concurrent delete cannot
        // leave orphaned segments behind.
        drop(all);
        drop(meetings);
        Ok(incoming)
    }
}

#[async_trait::async_trait]
impl PeopleStore for InMemoryMeetingStore {
    async fn upsert_person(&self, person: PersonProfile) -> MeetingStoreResult<PersonProfile> {
        validate_person(&person)?;
        // Same lock order as merge: a delayed create cannot resurrect its source.
        let merged = self.merged_people.read().await;
        if merged.contains_key(&person.id) {
            return Err(MeetingStoreError::InvalidInput(
                "This contact was already merged.".into(),
            ));
        }
        let mut people = self.people.write().await;
        if people
            .get(&person.id)
            .is_some_and(|current| current != &person)
        {
            return Err(MeetingStoreError::Conflict(
                "Contact already exists; use a field-specific update.".into(),
            ));
        }
        people.insert(person.id.clone(), person.clone());
        drop(people);
        drop(merged);
        Ok(person)
    }

    async fn insert_person_if_absent(
        &self,
        person: PersonProfile,
    ) -> MeetingStoreResult<super::PersonInsert> {
        validate_person(&person)?;
        // Same lock order as merge and upsert: a delayed create cannot
        // resurrect a merged source.
        let merged = self.merged_people.read().await;
        if merged.contains_key(&person.id) {
            return Err(MeetingStoreError::Conflict(
                "This contact was already merged.".into(),
            ));
        }
        let mut people = self.people.write().await;
        if let Some(current) = people.get(&person.id) {
            return Ok(super::PersonInsert::Existing(current.clone()));
        }
        people.insert(person.id.clone(), person.clone());
        drop(people);
        drop(merged);
        Ok(super::PersonInsert::Created(person))
    }

    async fn list_people(&self) -> MeetingStoreResult<Vec<PersonProfile>> {
        Ok(self.people.read().await.values().cloned().collect())
    }

    async fn get_person(&self, id: &str) -> MeetingStoreResult<Option<PersonProfile>> {
        Ok(self.people.read().await.get(id).cloned())
    }

    async fn person_versions(
        &self,
        after: Option<&str>,
    ) -> MeetingStoreResult<bruhs_models::PersonVersionPage> {
        use std::ops::Bound::{Excluded, Unbounded};
        let people = self.people.read().await;
        let entries = people
            .range((Excluded(after.unwrap_or_default().to_owned()), Unbounded))
            .take(bruhs_models::PERSON_VERSION_PAGE_SIZE + 1)
            .map(|(id, person)| bruhs_models::PersonVersion {
                id: id.clone(),
                updated_at: person.updated_at,
            })
            .collect();
        drop(people);
        Ok(bruhs_models::PersonVersionPage::from_sorted(entries))
    }

    async fn get_people(&self, ids: &[String]) -> MeetingStoreResult<Vec<PersonProfile>> {
        super::person_changes::validate_resolution_ids(ids)?;
        let people = self.people.read().await;
        let found = ids
            .iter()
            .filter_map(|id| people.get(id).cloned())
            .collect();
        drop(people);
        Ok(found)
    }

    async fn resolve_person_merges(
        &self,
        ids: &[String],
    ) -> MeetingStoreResult<Vec<bruhs_models::PersonMergeResolution>> {
        super::person_changes::validate_resolution_ids(ids)?;
        let merged = self.merged_people.read().await;
        let people = self.people.read().await;
        let mut result = Vec::new();
        for id in ids {
            let Some(mut target_id) = merged.get(id) else {
                continue;
            };
            let mut seen = std::collections::HashSet::from([id]);
            while let Some(next) = merged.get(target_id) {
                if !seen.insert(target_id) {
                    return Err(MeetingStoreError::PersistenceFailed(
                        "Saved contact merge contains a cycle.".into(),
                    ));
                }
                target_id = next;
            }
            let target =
                people
                    .get(target_id)
                    .ok_or_else(|| MeetingStoreError::PersonNotFound {
                        id: target_id.clone(),
                    })?;
            result.push(bruhs_models::PersonMergeResolution {
                source_id: id.clone(),
                target: target.clone(),
            });
        }
        drop(people);
        drop(merged);
        Ok(result)
    }

    async fn append_person_observations(
        &self,
        by_display_name: HashMap<String, Vec<bruhs_models::PersonObservation>>,
    ) -> MeetingStoreResult<Vec<PersonProfile>> {
        let mut people = self.people.write().await;
        let mut updated: HashMap<String, PersonProfile> = HashMap::new();
        for (raw_label, observations) in by_display_name {
            let label = raw_label.trim().to_string();
            if label.is_empty() || observations.is_empty() {
                continue;
            }
            let matches: Vec<_> = people
                .values()
                .filter(|p| !updated.contains_key(&p.id))
                .chain(updated.values())
                .filter(|p| {
                    p.display_name.eq_ignore_ascii_case(&label)
                        || p.aliases.iter().any(|a| a.eq_ignore_ascii_case(&label))
                })
                .take(2)
                .cloned()
                .collect();
            if matches.len() > 1 {
                return Err(MeetingStoreError::Conflict(
                    "Contact name is ambiguous; no observations were saved.".into(),
                ));
            }
            let profile = if let Some(mut p) = matches.into_iter().next() {
                // If the matching profile is untracked, silently drop — don't
                // extend it and don't create a new profile either.
                if !p.tracked {
                    continue;
                }
                p.updated_at = next_version(p.updated_at)?;
                p.observations.extend(observations);
                p
            } else {
                let mut p = bruhs_models::PersonProfile::new(label);
                p.observations.extend(observations);
                p.updated_at = next_version(p.updated_at)?;
                p
            };
            updated.insert(profile.id.clone(), profile);
        }
        let result = updated.values().cloned().collect();
        people.extend(updated);
        drop(people);
        Ok(result)
    }

    async fn match_centroid_to_person(
        &self,
        embedding: &[f32],
        threshold: f32,
    ) -> MeetingStoreResult<Option<PersonProfile>> {
        let people = self.people.read().await;
        let mut best: Option<(f32, PersonProfile)> = None;
        for p in people.values() {
            let Some(profile_emb) = p.voice_embedding.as_ref() else {
                continue;
            };
            if profile_emb.len() != embedding.len() {
                continue;
            }
            let sim = cosine_similarity_unit(embedding, profile_emb);
            if sim >= threshold && best.as_ref().is_none_or(|(s, _)| sim > *s) {
                best = Some((sim, p.clone()));
            }
        }
        drop(people);
        Ok(best.map(|(_, p)| p))
    }

    async fn upsert_voice_embedding(
        &self,
        person_id: &bruhs_models::ids::PersonId,
        embedding: Vec<f32>,
    ) -> MeetingStoreResult<PersonProfile> {
        let person_id = person_id.as_str();
        let mut people = self.people.write().await;
        let Some(p) = people.get_mut(person_id) else {
            return Err(MeetingStoreError::PersonNotFound {
                id: person_id.to_string(),
            });
        };
        let version = next_version(p.updated_at)?;
        if embedding.iter().any(|value| !value.is_finite()) {
            return Err(MeetingStoreError::InvalidInput(
                "Voice values must be finite.".into(),
            ));
        }
        p.voice_embedding = Some(embedding);
        p.updated_at = version;
        let updated = p.clone();
        drop(people);
        Ok(updated)
    }

    async fn fill_voice_embedding(
        &self,
        person_id: &bruhs_models::ids::PersonId,
        embedding: Vec<f32>,
    ) -> MeetingStoreResult<PersonProfile> {
        let person_id = person_id.as_str();
        let mut people = self.people.write().await;
        let p = people
            .get_mut(person_id)
            .ok_or_else(|| MeetingStoreError::PersonNotFound {
                id: person_id.into(),
            })?;
        if p.voice_embedding.is_none() {
            let version = next_version(p.updated_at)?;
            if embedding.iter().any(|value| !value.is_finite()) {
                return Err(MeetingStoreError::InvalidInput(
                    "Voice values must be finite.".into(),
                ));
            }
            p.voice_embedding = Some(embedding);
            p.updated_at = version;
        }
        let updated = p.clone();
        drop(people);
        Ok(updated)
    }

    async fn patch_person_tracked(
        &self,
        person_id: &bruhs_models::ids::PersonId,
        tracked: bool,
    ) -> MeetingStoreResult<PersonProfile> {
        let person_id = person_id.as_str();
        let mut people = self.people.write().await;
        let Some(p) = people.get_mut(person_id) else {
            return Err(MeetingStoreError::PersonNotFound {
                id: person_id.to_string(),
            });
        };
        let version = next_version(p.updated_at)?;
        p.tracked = tracked;
        p.updated_at = version;
        let updated = p.clone();
        drop(people);
        Ok(updated)
    }

    async fn delete_observations_for_meeting(
        &self,
        meeting_id: &bruhs_models::ids::MeetingId,
    ) -> MeetingStoreResult<usize> {
        let meeting_id = meeting_id.as_str();
        let mut people = self.people.write().await;
        // Validate every affected version before mutating any profile.
        let versions = people
            .values()
            .filter(|profile| {
                profile
                    .observations
                    .iter()
                    .any(|o| o.at_meeting_id == meeting_id)
            })
            .map(|profile| Ok((profile.id.clone(), next_version(profile.updated_at)?)))
            .collect::<MeetingStoreResult<Vec<_>>>()?;
        let mut total_deleted = 0usize;
        for (id, version) in versions {
            let profile = people
                .get_mut(&id)
                .expect("write lock keeps the validated profile present");
            let before = profile.observations.len();
            profile
                .observations
                .retain(|o| o.at_meeting_id != meeting_id);
            let deleted = before - profile.observations.len();
            if deleted > 0 {
                total_deleted += deleted;
                profile.updated_at = version;
            }
        }
        drop(people);
        Ok(total_deleted)
    }

    async fn rename_person(
        &self,
        person_id: &bruhs_models::ids::PersonId,
        new_name: String,
    ) -> MeetingStoreResult<PersonProfile> {
        let person_id = person_id.as_str();
        let trimmed = new_name.trim().to_string();
        if trimmed.is_empty() {
            return Err(MeetingStoreError::InvalidInput(
                "display_name must not be empty".into(),
            ));
        }
        let mut people = self.people.write().await;
        let Some(p) = people.get_mut(person_id) else {
            return Err(MeetingStoreError::PersonNotFound {
                id: person_id.to_string(),
            });
        };
        let version = next_version(p.updated_at)?;
        p.display_name = trimmed;
        p.updated_at = version;
        let updated = p.clone();
        drop(people);
        Ok(updated)
    }

    async fn merge_person_into(
        &self,
        source_id: &bruhs_models::ids::PersonId,
        target_id: &bruhs_models::ids::PersonId,
    ) -> MeetingStoreResult<PersonProfile> {
        let source_id = source_id.as_str();
        let target_id = target_id.as_str();
        if source_id == target_id {
            return Err(MeetingStoreError::InvalidInput(
                "source and target must be different profiles".into(),
            ));
        }
        let mut merged = self.merged_people.write().await;
        let mut people = self.people.write().await;
        if let Some(committed_target) = merged.get(source_id) {
            if committed_target != target_id {
                return Err(MeetingStoreError::InvalidInput(
                    "This source was already merged into another contact.".into(),
                ));
            }
            return people.get(target_id).cloned().ok_or_else(|| {
                MeetingStoreError::PersonNotFound {
                    id: target_id.into(),
                }
            });
        }
        let source = people
            .get(source_id)
            .ok_or_else(|| MeetingStoreError::PersonNotFound {
                id: source_id.to_string(),
            })?
            .clone();
        if !people.contains_key(target_id) {
            return Err(MeetingStoreError::PersonNotFound {
                id: target_id.to_string(),
            });
        }
        let Some(target) = people.get(target_id) else {
            unreachable!(
                "target_id existence was verified with contains_key while holding the write lock"
            );
        };
        let updated = merged_profile(&source, target)?;
        people.insert(target_id.into(), updated.clone());
        merged.insert(source_id.into(), target_id.into());
        people.remove(source_id);
        drop(people);
        drop(merged);
        Ok(updated)
    }

    async fn link_speaker_to_person(
        &self,
        meeting_id: &bruhs_models::ids::MeetingId,
        speaker_id: &bruhs_models::ids::SpeakerId,
        person_id: &bruhs_models::ids::PersonId,
    ) -> MeetingStoreResult<Meeting> {
        let meeting_id = meeting_id.as_str();
        let speaker_id = speaker_id.as_str();
        let person_id = person_id.as_str();
        // Look up person first (cheap read) before acquiring the meeting write lock.
        let person = self
            .people
            .read()
            .await
            .get(person_id)
            .cloned()
            .ok_or_else(|| MeetingStoreError::PersonNotFound {
                id: person_id.to_string(),
            })?;

        let mut meetings = self.meetings.write().await;
        let Some(m) = meetings.get_mut(meeting_id) else {
            return Err(MeetingStoreError::MeetingNotFound {
                id: meeting_id.to_string(),
            });
        };

        if let Some(speaker) = m.speakers.iter_mut().find(|s| s.id == speaker_id) {
            // Update existing entry.
            speaker.label = person.display_name.clone();
            speaker.person_id = Some(person.id.clone());
        } else {
            // Create entry — speaker_id may be a raw segment label that hasn't
            // been through ensure_meeting_speakers yet.
            m.speakers.push(bruhs_models::Speaker {
                id: speaker_id.into(),
                label: person.display_name.clone(),
                person_id: Some(person.id.clone()),
                color: None,
            });
        }
        m.updated_at += 1;
        let updated = m.clone();
        drop(meetings);
        Ok(updated)
    }
}

#[async_trait::async_trait]
impl SummaryGenerationStore for InMemoryMeetingStore {
    async fn pending_summary_generation(
        &self,
        meeting_id: &str,
    ) -> MeetingStoreResult<Option<bruhs_models::SummaryGenerationReceipt>> {
        Ok(self
            .generations
            .read()
            .await
            .values()
            .find(|receipt| {
                receipt.request.meeting_id == meeting_id
                    && matches!(receipt.state, bruhs_models::SummaryGenerationState::Pending)
            })
            .cloned())
    }

    async fn begin_summary_generation(
        &self,
        request: bruhs_models::SummaryGenerationRequest,
    ) -> MeetingStoreResult<super::GenerationReservation> {
        self.begin_generation(request).await
    }

    async fn summary_generation(
        &self,
        meeting_id: &str,
        request_id: &str,
    ) -> MeetingStoreResult<Option<bruhs_models::SummaryGenerationReceipt>> {
        Ok(self
            .generations
            .read()
            .await
            .get(request_id)
            .filter(|receipt| receipt.request.meeting_id == meeting_id)
            .cloned())
    }

    async fn finish_summary_generation(
        &self,
        meeting_id: &str,
        request_id: &str,
        outcome: super::GenerationOutcome,
    ) -> MeetingStoreResult<bruhs_models::SummaryGenerationReceipt> {
        self.finish_generation(meeting_id, request_id, outcome)
            .await
    }

    async fn fence_summary_generation(
        &self,
        request: bruhs_models::SummaryGenerationRequest,
    ) -> MeetingStoreResult<bruhs_models::SummaryGenerationReceipt> {
        self.fence_generation(request).await
    }

    async fn commit_summary_generation(
        &self,
        request_id: &str,
        update: super::SummaryUpdate,
    ) -> MeetingStoreResult<Meeting> {
        self.commit_generation(request_id, update).await
    }

    async fn summary_people_versions(
        &self,
        expected: &Meeting,
    ) -> MeetingStoreResult<BTreeMap<String, i64>> {
        let meetings = self.meetings.read().await;
        if meetings.get(&expected.id) != Some(expected) {
            return Err(MeetingStoreError::Conflict(
                "Meeting changed before generation; reload and retry.".into(),
            ));
        }
        let people = self.people.read().await;
        let versions = super::summary_update::versions(people.values(), expected);
        // Read under the meeting lock: the versions belong to `expected`.
        drop(people);
        drop(meetings);
        Ok(versions)
    }

    async fn commit_summary(&self, update: super::SummaryUpdate) -> MeetingStoreResult<Meeting> {
        let mut meetings = self.meetings.write().await;
        let segments = self.segments.read().await;
        let mut people = self.people.write().await;
        let current = meetings.get(&update.expected.id).ok_or_else(|| {
            MeetingStoreError::MeetingNotFound {
                id: update.expected.id.clone(),
            }
        })?;
        let mut current_segments = segments.get(&current.id).cloned().unwrap_or_default();
        current_segments.sort_unstable_by(|a, b| (a.start_ms, &a.id).cmp(&(b.start_ms, &b.id)));
        update.validate(current, &current_segments)?;
        let changed = update.stage_people(people.values())?;
        let mut meeting = current.clone();
        meeting.updated_at = next_version(meeting.updated_at)?;
        meeting.summary = Some(update.summary);
        people.extend(changed);
        meetings.insert(meeting.id.clone(), meeting.clone());
        // All three locks span validate-then-publish, so the summary commits
        // against exactly the meeting, segments and people it was checked on.
        drop(people);
        drop(segments);
        drop(meetings);
        Ok(meeting)
    }
}

#[async_trait::async_trait]
impl SpeakerLinkJobs for InMemoryMeetingStore {
    async fn enqueue_speaker_link(&self, meeting_id: &str) -> MeetingStoreResult<()> {
        let meetings = self.meetings.read().await;
        if !meetings.contains_key(meeting_id) {
            return Err(MeetingStoreError::MeetingNotFound {
                id: meeting_id.into(),
            });
        }
        self.link_jobs
            .write()
            .await
            .insert(meeting_id.into(), super::SpeakerLinkJob::new(meeting_id));
        // Held across the insert so a concurrent delete cannot strand the job.
        drop(meetings);
        Ok(())
    }

    async fn enqueue_capture_receipt(
        &self,
        receipt: &bruhs_models::CaptureRecoveryReceipt,
    ) -> MeetingStoreResult<()> {
        let job = super::SpeakerLinkJob::from_capture(receipt)?;
        let meetings = self.meetings.read().await;
        if !meetings.contains_key(&job.meeting_id) {
            return Err(MeetingStoreError::MeetingNotFound { id: job.meeting_id });
        }
        let mut jobs = self.link_jobs.write().await;
        if jobs
            .get(&job.meeting_id)
            .is_none_or(|pending| pending.token != job.token)
        {
            jobs.insert(job.meeting_id.clone(), job);
        }
        drop(jobs);
        drop(meetings);
        Ok(())
    }

    async fn next_speaker_link(
        &self,
        now_ms: i64,
    ) -> MeetingStoreResult<Option<super::SpeakerLinkJob>> {
        Ok(self
            .link_jobs
            .read()
            .await
            .values()
            .filter(|job| job.due_at_ms <= now_ms)
            .min_by(|a, b| (a.due_at_ms, &a.meeting_id).cmp(&(b.due_at_ms, &b.meeting_id)))
            .cloned())
    }

    async fn finish_speaker_link(&self, job: &super::SpeakerLinkJob) -> MeetingStoreResult<()> {
        let mut jobs = self.link_jobs.write().await;
        if jobs
            .get(&job.meeting_id)
            .is_some_and(|saved| saved.token == job.token)
        {
            jobs.remove(&job.meeting_id);
        }
        drop(jobs);
        Ok(())
    }

    async fn retry_speaker_link(
        &self,
        job: &super::SpeakerLinkJob,
        now_ms: i64,
    ) -> MeetingStoreResult<()> {
        let mut jobs = self.link_jobs.write().await;
        if jobs.get(&job.meeting_id).is_some_and(|saved| saved == job) {
            jobs.insert(job.meeting_id.clone(), job.retry(now_ms));
        }
        drop(jobs);
        Ok(())
    }

    async fn link_speaker_centroids(
        &self,
        meeting_id: &str,
        centroids: &bruhs_models::CentroidsFile,
    ) -> MeetingStoreResult<bool> {
        // Match the existing summary lock order. Nothing is published until
        // every identity and the final meeting version have been validated.
        let mut meetings = self.meetings.write().await;
        let mut people = self.people.write().await;
        let current =
            meetings
                .get(meeting_id)
                .ok_or_else(|| MeetingStoreError::MeetingNotFound {
                    id: meeting_id.into(),
                })?;
        let Some(update) = super::speaker_linking::plan(current.clone(), &people, centroids)?
        else {
            return Ok(false);
        };
        people.extend(update.people);
        meetings.insert(meeting_id.into(), update.meeting);
        drop(people);
        drop(meetings);
        Ok(true)
    }
}

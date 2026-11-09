//! Server-side meeting persistence: [`SqliteMeetingStore`] on disk and
//! [`InMemoryMeetingStore`] for tests. The store API is split by area
//! ([`MeetingRecords`], [`PeopleStore`], [`SummaryGenerationStore`],
//! [`SpeakerLinkJobs`]); [`MeetingStore`] is all four together.

#[cfg(test)]
mod contact_sync_tests;
pub mod error;
mod generation;
pub use generation::{GenerationOutcome, GenerationReservation};
mod capture_failures;
mod link_jobs;
#[cfg(test)]
mod local_focus_tests;
pub use link_jobs::SpeakerLinkJob;
pub mod in_memory;
#[cfg(test)]
mod merge_resolution_tests;
#[cfg(test)]
mod merge_retry_tests;
mod person_changes;
mod speaker_linking;
#[cfg(test)]
mod summary_tests;
mod summary_update;
pub use summary_update::SummaryUpdate;
pub mod sqlite;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod transcript_tests;

pub use error::{MeetingStoreError, MeetingStoreResult};

pub(crate) fn valid_chunk_id(segment_id: &str) -> MeetingStoreResult<()> {
    if segment_id.trim().is_empty()
        || segment_id.len() > 1024
        || segment_id.chars().any(char::is_control)
    {
        Err(MeetingStoreError::InvalidInput(
            "Transcript chunk identity must be valid.".into(),
        ))
    } else {
        Ok(())
    }
}
pub use in_memory::InMemoryMeetingStore;
pub use sqlite::SqliteMeetingStore;

use async_trait::async_trait;
use bruhs_models::{Meeting, MeetingStatus, PersonProfile, TranscriptSegment};
use std::collections::HashMap;
use std::sync::Arc;

/// Result of [`PeopleStore::insert_person_if_absent`].
#[derive(Debug, Clone, PartialEq)]
pub enum PersonInsert {
    /// The contact was new and is now saved.
    Created(PersonProfile),
    /// A contact with that id already existed; this is it, unchanged.
    Existing(PersonProfile),
}

impl PersonInsert {
    /// The saved contact, either way.
    pub fn into_person(self) -> PersonProfile {
        match self {
            Self::Created(person) | Self::Existing(person) => person,
        }
    }
}

/// Fields of a meeting to change; `None` leaves a field as it is.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MeetingPatch {
    pub title: Option<String>,
    pub status: Option<MeetingStatus>,
    pub ended_at_ms: Option<f64>,
}

impl MeetingPatch {
    /// Apply to `meeting`. A failed meeting stays failed (a late "ended"
    /// does not revive it) and keeps the end time it failed with.
    pub(crate) fn apply(self, meeting: &mut Meeting) {
        if let Some(title) = self.title {
            meeting.title = title;
        }
        if let Some(status) = self.status {
            if !(meeting.status == MeetingStatus::Failed && status == MeetingStatus::Ended) {
                meeting.status = status;
            }
        }
        if let Some(ended) = self.ended_at_ms {
            if meeting.status != MeetingStatus::Failed || meeting.ended_at_ms.is_none() {
                meeting.ended_at_ms = Some(ended);
            }
        }
    }
}

// ── Traits ───────────────────────────────────────────────────────────────────

/// Meetings, their transcripts and per-meeting metadata.
#[async_trait]
pub trait MeetingRecords: Send + Sync {
    /// Atomically fail only the matching recording attempt, preserving all
    /// transcript/summary data. Replays and stale attempts do not change versions.
    async fn apply_capture_failure(
        &self,
        receipt: &bruhs_models::CaptureFailureReceipt,
    ) -> MeetingStoreResult<bool>;

    /// Local metadata only. Kept separate from transcripts, summaries and exports.
    async fn append_local_focus(
        &self,
        observation: bruhs_models::LocalFocusObservation,
    ) -> MeetingStoreResult<()>;

    async fn list_local_focus(
        &self,
        meeting_id: &str,
    ) -> MeetingStoreResult<Vec<bruhs_models::LocalFocusObservation>>;

    async fn create_meeting(&self, meeting: Meeting) -> MeetingStoreResult<Meeting>;

    async fn get_meeting(&self, id: &str) -> MeetingStoreResult<Option<Meeting>>;

    async fn list_meetings(&self) -> MeetingStoreResult<Vec<Meeting>>;

    /// Insert a meeting or replay an identical insert. Existing meetings require
    /// field-specific mutations, so delayed snapshots cannot erase newer fields.
    async fn upsert_meeting(&self, meeting: Meeting) -> MeetingStoreResult<Meeting>;

    /// Apply `patch`; `None` when there is no such meeting.
    async fn patch_meeting(
        &self,
        id: &bruhs_models::ids::MeetingId,
        patch: MeetingPatch,
    ) -> MeetingStoreResult<Option<Meeting>>;

    async fn patch_meeting_summary(
        &self,
        id: &bruhs_models::ids::MeetingId,
        summary: bruhs_models::MeetingSummary,
    ) -> MeetingStoreResult<Option<bruhs_models::Meeting>>;

    async fn patch_speaker_label(
        &self,
        meeting_id: &bruhs_models::ids::MeetingId,
        speaker_id: &bruhs_models::ids::SpeakerId,
        label: String,
    ) -> MeetingStoreResult<Meeting>;

    async fn patch_meeting_thread_id(
        &self,
        meeting_id: &bruhs_models::ids::MeetingId,
        thread_id: &bruhs_models::ids::ThreadId,
    ) -> MeetingStoreResult<Meeting>;

    /// For each Speaker in `speakers`, insert into Meeting.speakers if
    /// no entry with the same id exists. Existing entries are preserved
    /// verbatim (user renames are not clobbered).
    async fn ensure_meeting_speakers(
        &self,
        meeting_id: &bruhs_models::ids::MeetingId,
        speakers: Vec<bruhs_models::Speaker>,
    ) -> MeetingStoreResult<Meeting>;

    /// Toggle `Meeting.skip_observations`. Returns the updated meeting or an
    /// error if no meeting with `meeting_id` exists.
    async fn patch_meeting_skip_observations(
        &self,
        meeting_id: &bruhs_models::ids::MeetingId,
        skip: bool,
    ) -> MeetingStoreResult<Meeting>;

    async fn delete_meeting(&self, id: &bruhs_models::ids::MeetingId) -> MeetingStoreResult<bool>;

    async fn append_segment(&self, segment: TranscriptSegment) -> MeetingStoreResult<()>;

    /// Drop one transcript chunk and ignore later capture replays of its id.
    /// The meeting must exist. An unknown chunk is still remembered so an
    /// in-flight partial cannot reappear. Repeating the call changes nothing.
    async fn remove_segment(&self, meeting_id: &str, segment_id: &str) -> MeetingStoreResult<()>;

    /// Whether capture replays of this chunk must be ignored.
    async fn segment_removed(&self, meeting_id: &str, segment_id: &str)
        -> MeetingStoreResult<bool>;

    /// Removed chunk ids for this meeting, in sorted order.
    async fn removed_segment_ids(&self, meeting_id: &str) -> MeetingStoreResult<Vec<String>>;

    async fn list_segments(&self, meeting_id: &str) -> MeetingStoreResult<Vec<TranscriptSegment>>;

    /// Cache a validated capture snapshot against the exact pre-fetch baseline.
    /// A stale reply, missing segment or rewritten final speech cannot replace saved data.
    async fn cache_transcript(
        &self,
        meeting_id: &str,
        expected: Vec<TranscriptSegment>,
        incoming: Vec<TranscriptSegment>,
    ) -> MeetingStoreResult<Vec<TranscriptSegment>>;
}

/// People (contacts), their observations, voices and merges, and linking meeting speakers to them.
#[async_trait]
pub trait PeopleStore: Send + Sync {
    /// Insert a new identity or replay an identical insert. Existing profiles
    /// require field-specific mutations; a client timestamp cannot authorize replacement.
    async fn upsert_person(&self, person: PersonProfile) -> MeetingStoreResult<PersonProfile>;

    /// Insert `person` unless a contact with its id exists, in one atomic
    /// step: [`PersonInsert::Existing`] returns the saved contact unchanged
    /// (whatever `person` says), so concurrent creates of one id yield
    /// exactly one [`PersonInsert::Created`]. An id that was merged away is
    /// a conflict.
    async fn insert_person_if_absent(
        &self,
        person: PersonProfile,
    ) -> MeetingStoreResult<PersonInsert>;

    async fn list_people(&self) -> MeetingStoreResult<Vec<PersonProfile>>;

    async fn get_person(&self, id: &str) -> MeetingStoreResult<Option<PersonProfile>>;

    /// Identity/version-only keyset page. Pages never imply deletion.
    async fn person_versions(
        &self,
        after: Option<&str>,
    ) -> MeetingStoreResult<bruhs_models::PersonVersionPage>;

    /// At most 64 unique identities in one read snapshot; missing identities are omitted.
    async fn get_people(&self, ids: &[String]) -> MeetingStoreResult<Vec<PersonProfile>>;

    /// Read-only resolution for at most 64 unique cached identities missing from
    /// the server directory. Unknown identities are not evidence of deletion.
    async fn resolve_person_merges(
        &self,
        ids: &[String],
    ) -> MeetingStoreResult<Vec<bruhs_models::PersonMergeResolution>>;

    async fn append_person_observations(
        &self,
        by_display_name: HashMap<String, Vec<bruhs_models::PersonObservation>>,
    ) -> MeetingStoreResult<Vec<PersonProfile>>;

    /// Find the best-matching PersonProfile for an L2-normalized embedding.
    /// Returns the highest-similarity profile if it exceeds `threshold`
    /// (cosine similarity in 0..1). Profiles without voice_embedding are
    /// ignored.
    async fn match_centroid_to_person(
        &self,
        embedding: &[f32],
        threshold: f32,
    ) -> MeetingStoreResult<Option<PersonProfile>>;

    /// Set or overwrite PersonProfile.voice_embedding. Errors if no
    /// profile exists with this id.
    async fn upsert_voice_embedding(
        &self,
        person_id: &bruhs_models::ids::PersonId,
        embedding: Vec<f32>,
    ) -> MeetingStoreResult<PersonProfile>;

    /// Background identification must not overwrite an explicitly enrolled voice.
    async fn fill_voice_embedding(
        &self,
        person_id: &bruhs_models::ids::PersonId,
        embedding: Vec<f32>,
    ) -> MeetingStoreResult<PersonProfile>;

    /// Toggle `PersonProfile.tracked`. Returns the updated profile or an error
    /// if no profile with `person_id` exists.
    async fn patch_person_tracked(
        &self,
        person_id: &bruhs_models::ids::PersonId,
        tracked: bool,
    ) -> MeetingStoreResult<PersonProfile>;

    /// Delete all `PersonObservation` entries across all `PersonProfile`s where
    /// `at_meeting_id == meeting_id`. Updates `updated_at` for each modified
    /// profile. Returns the total count of deleted observations.
    async fn delete_observations_for_meeting(
        &self,
        meeting_id: &bruhs_models::ids::MeetingId,
    ) -> MeetingStoreResult<usize>;

    /// Rename a PersonProfile's `display_name`. Trims whitespace and rejects
    /// empty names. Returns the updated profile or an error if not found.
    async fn rename_person(
        &self,
        person_id: &bruhs_models::ids::PersonId,
        new_name: String,
    ) -> MeetingStoreResult<PersonProfile>;

    /// Absorb `source_id` into `target_id`:
    /// - Moves all source observations to target.
    /// - Copies source voice_embedding to target only if target has none.
    /// - Appends source display_name to target aliases (case-insensitive dedupe).
    /// - Updates target updated_at.
    /// - Deletes source from the people map.
    /// Returns the updated target or an error if either id is missing or
    /// source_id == target_id.
    async fn merge_person_into(
        &self,
        source_id: &bruhs_models::ids::PersonId,
        target_id: &bruhs_models::ids::PersonId,
    ) -> MeetingStoreResult<PersonProfile>;

    /// Phase 3d: manually link a speaker entry in a meeting to an existing
    /// PersonProfile. Updates the Speaker entry's `label` to the person's
    /// `display_name` and sets `person_id`. If no Speaker entry with
    /// `speaker_id` exists in the meeting, one is created.
    ///
    /// v1 does NOT transfer voice_embedding (deferred to Phase 3e).
    ///
    /// Returns the updated Meeting. Errors if meeting or person not found.
    async fn link_speaker_to_person(
        &self,
        meeting_id: &bruhs_models::ids::MeetingId,
        speaker_id: &bruhs_models::ids::SpeakerId,
        person_id: &bruhs_models::ids::PersonId,
    ) -> MeetingStoreResult<Meeting>;
}

/// Summary generation: reservations, receipts and the atomic commit of a generated summary.
#[async_trait]
pub trait SummaryGenerationStore: Send + Sync {
    /// Discover unresolved legacy requests whose client did not retain an identity.
    async fn pending_summary_generation(
        &self,
        meeting_id: &str,
    ) -> MeetingStoreResult<Option<bruhs_models::SummaryGenerationReceipt>>;

    /// Reserve once, serializing generation per meeting across clients.
    async fn begin_summary_generation(
        &self,
        request: bruhs_models::SummaryGenerationRequest,
    ) -> MeetingStoreResult<GenerationReservation>;

    async fn summary_generation(
        &self,
        meeting_id: &str,
        request_id: &str,
    ) -> MeetingStoreResult<Option<bruhs_models::SummaryGenerationReceipt>>;

    /// Stop accepting this result: a still-pending receipt settles as
    /// `outcome`; a terminal one is unchanged. Cancellation never replays or
    /// starts a provider.
    async fn finish_summary_generation(
        &self,
        meeting_id: &str,
        request_id: &str,
        outcome: GenerationOutcome,
    ) -> MeetingStoreResult<bruhs_models::SummaryGenerationReceipt>;

    /// Refuse a request's result before it reaches the server. An unknown identity
    /// becomes a cancelled tombstone; a pending one is cancelled; terminal ones are unchanged.
    async fn fence_summary_generation(
        &self,
        request: bruhs_models::SummaryGenerationRequest,
    ) -> MeetingStoreResult<bruhs_models::SummaryGenerationReceipt>;

    /// The receipt and all generated records share the same atomic commit.
    async fn commit_summary_generation(
        &self,
        request_id: &str,
        update: SummaryUpdate,
    ) -> MeetingStoreResult<Meeting>;

    /// Relevant participant/observation versions before generation, without voice data.
    async fn summary_people_versions(
        &self,
        expected: &Meeting,
    ) -> MeetingStoreResult<std::collections::BTreeMap<String, i64>>;

    /// Atomically save a generated summary and its observation changes. A stale
    /// meeting/transcript, invalid result or contact failure saves neither.
    async fn commit_summary(&self, update: SummaryUpdate) -> MeetingStoreResult<Meeting>;
}

/// The durable queue of automatic speaker-linking work.
#[async_trait]
pub trait SpeakerLinkJobs: Send + Sync {
    /// Persist/coalesce work for an existing meeting before acknowledging its event.
    /// A fresh token fences acknowledgments from an older in-flight attempt.
    async fn enqueue_speaker_link(&self, meeting_id: &str) -> MeetingStoreResult<()>;

    /// Import an at-least-once capture receipt without resetting the same pending
    /// job's retry delay. Completed linking remains idempotent on later replay.
    async fn enqueue_capture_receipt(
        &self,
        receipt: &bruhs_models::CaptureRecoveryReceipt,
    ) -> MeetingStoreResult<()>;

    /// Read at most one due job. Taking a job does not remove it or hide it on restart.
    async fn next_speaker_link(&self, now_ms: i64) -> MeetingStoreResult<Option<SpeakerLinkJob>>;

    /// Only acknowledge the exact receipt that was processed, after the linking commit.
    async fn finish_speaker_link(&self, job: &SpeakerLinkJob) -> MeetingStoreResult<()>;

    /// Persist bounded exponential backoff, without discarding the job.
    async fn retry_speaker_link(&self, job: &SpeakerLinkJob, now_ms: i64)
        -> MeetingStoreResult<()>;

    /// Atomically select/create speaker identities and attach every missing
    /// cluster. Completed replays preserve existing choices and record versions.
    async fn link_speaker_centroids(
        &self,
        meeting_id: &str,
        centroids: &bruhs_models::CentroidsFile,
    ) -> MeetingStoreResult<bool>;
}

/// Everything a meeting backend stores. Implemented automatically for any
/// type that implements the four area traits, so `dyn MeetingStore` keeps
/// working for callers that need all of it.
pub trait MeetingStore:
    MeetingRecords + PeopleStore + SummaryGenerationStore + SpeakerLinkJobs
{
}

impl<T> MeetingStore for T where
    T: MeetingRecords + PeopleStore + SummaryGenerationStore + SpeakerLinkJobs + ?Sized
{
}

// ── Utilities ────────────────────────────────────────────────────────────────

/// Walk all meetings. For each meeting whose `status == Live` and whose `id`
/// is not `active_meeting_id`, set the status to `Failed`.
///
/// Called on server boot (`AppState::reconcile_orphaned_meetings`) so stale Live meetings
/// (left over from a daemon crash or server restart) don't remain stuck in the
/// Live state. If the daemon reports a currently-active meeting via `active_meeting_id`,
/// that meeting is left untouched.
pub async fn mark_orphaned_live_as_failed(
    store: &dyn MeetingStore,
    active_meeting_id: Option<&str>,
) -> MeetingStoreResult<usize> {
    let meetings = store.list_meetings().await?;
    let mut count = 0usize;
    for m in meetings {
        if m.status != MeetingStatus::Live {
            continue;
        }
        if active_meeting_id.is_some_and(|a| a == m.id) {
            continue;
        }
        let id = match bruhs_models::ids::MeetingId::try_from(m.id.as_str()) {
            Ok(id) => id,
            Err(e) => {
                tracing::warn!("mark_orphaned_live_as_failed: skipping malformed id: {e}");
                continue;
            }
        };
        store
            .patch_meeting(
                &id,
                MeetingPatch {
                    status: Some(MeetingStatus::Failed),
                    ..MeetingPatch::default()
                },
            )
            .await?;
        count += 1;
        tracing::info!(
            "mark_orphaned_live_as_failed: meeting {} marked Failed (daemon crash recovery)",
            m.id
        );
    }
    Ok(count)
}

/// Convenience helper for `AppState::new`.
pub fn default_meeting_store() -> Arc<dyn MeetingStore> {
    Arc::new(InMemoryMeetingStore::new())
}

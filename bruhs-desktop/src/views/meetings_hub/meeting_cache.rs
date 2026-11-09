use super::*;
mod stream;
use stream::StreamFragments;
pub(super) use stream::TranscriptIndex;

/// Validated server context; stream fragments are distinct from full snapshots.
/// Optimistic speaker edits are not receipts.
/// Arc-backed records avoid copying text when cloning workspace snapshots.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "StoredMeetingCache")]
pub struct MeetingCache {
    records: BTreeMap<String, Arc<Meeting>>,
    transcripts: BTreeMap<String, Arc<Vec<TranscriptSegment>>>,
    fragments: BTreeMap<String, StreamFragments>,
}

pub(super) struct TranscriptReceipt {
    fragments: Option<StreamFragments>,
    snapshot: Option<Arc<Vec<TranscriptSegment>>>,
}

#[derive(Deserialize)]
struct StoredMeetingCache {
    records: BTreeMap<String, Arc<Meeting>>,
    transcripts: BTreeMap<String, Arc<Vec<TranscriptSegment>>>,
    #[serde(default)]
    fragments: BTreeMap<String, StreamFragments>,
}

impl TryFrom<StoredMeetingCache> for MeetingCache {
    type Error = &'static str;

    fn try_from(value: StoredMeetingCache) -> Result<Self, Self::Error> {
        for (id, meeting) in &value.records {
            validate_meeting(meeting)?;
            if id != &meeting.id {
                return Err("Saved meeting identity does not match its cache key.");
            }
        }
        for (id, segments) in &value.transcripts {
            if !value.records.contains_key(id) {
                return Err("Saved transcript has no matching meeting.");
            }
            bruhs_models::validate_transcript_records(id, segments)
                .map_err(bruhs_models::TranscriptValidationError::message)?;
        }
        for (id, fragments) in &value.fragments {
            if !value.records.contains_key(id) {
                return Err("Saved stream fragments have no matching meeting.");
            }
            fragments.validate(
                id,
                value
                    .transcripts
                    .get(id)
                    .map_or(&[], |records| records.as_slice()),
            )?;
        }
        Ok(Self {
            records: value.records,
            transcripts: value.transcripts,
            fragments: value.fragments,
        })
    }
}

pub(super) fn validate_meeting(meeting: &Meeting) -> Result<(), &'static str> {
    if meeting.id.trim().is_empty()
        || meeting.id.len() > 128
        || !meeting.started_at_ms.is_finite()
        || meeting
            .ended_at_ms
            .is_some_and(|end| !end.is_finite() || end < meeting.started_at_ms)
    {
        return Err("Meeting cache contains an invalid identity or timestamp.");
    }
    Ok(())
}

impl MeetingsHub {
    pub(super) fn transcript_read_receipt(&self, id: &str) -> TranscriptReceipt {
        TranscriptReceipt {
            fragments: self.meeting_cache.fragments.get(id).cloned(),
            snapshot: self.meeting_cache.transcripts.get(id).cloned(),
        }
    }

    pub(super) fn transcript_receipt_current(&self, id: &str, receipt: &TranscriptReceipt) -> bool {
        same_arc_snapshot(
            self.meeting_cache.transcripts.get(id),
            receipt.snapshot.as_ref(),
        ) && match (
            self.meeting_cache.fragments.get(id),
            receipt.fragments.as_ref(),
        ) {
            (Some(current), Some(before)) => current.same_snapshot(before),
            (None, None) => true,
            _ => false,
        }
    }

    pub(super) fn receive_transcript_since(
        &mut self,
        id: String,
        receipt: TranscriptReceipt,
        result: Result<Vec<TranscriptSegment>, ServerError>,
        cx: &mut Context<'_, Self>,
    ) {
        self.receive_authorized_transcript(id, receipt, result, Ok(Vec::new()), cx);
    }

    /// Apply removals only after the read receipt still matches. Forgetting a
    /// chunk changes that receipt, so doing it earlier would reject the snapshot.
    pub(super) fn receive_authorized_transcript(
        &mut self,
        id: String,
        receipt: TranscriptReceipt,
        result: Result<Vec<TranscriptSegment>, ServerError>,
        removed: Result<Vec<String>, ServerError>,
        cx: &mut Context<'_, Self>,
    ) {
        if !self.transcript_receipt_current(&id, &receipt) {
            self.notify_notice("New speech arrived or a newer transcript was loaded during the request. Current words are retained; reopen the meeting to refresh after streaming settles.", true, cx);
            return;
        }
        let removed = match removed {
            Ok(ids) => ids,
            Err(error) => {
                self.notify_notice(
                    format!("Could not load deleted transcript parts. {error} Current words are retained."),
                    true,
                    cx,
                );
                return;
            }
        };
        for segment_id in removed {
            self.forget_transcript_chunk(&id, &segment_id, cx);
        }
        self.receive_transcript(id, result, cx);
    }

    pub fn meeting_cache(&self) -> MeetingCache {
        self.meeting_cache.clone()
    }

    /// Restore before networking; live capture and in-flight actions never resume here.
    pub fn restore_meetings(&mut self, cache: MeetingCache) {
        self.meetings = cache
            .records
            .values()
            .map(|record| record.as_ref().clone())
            .collect();
        self.meetings
            .sort_by(|a, b| b.started_at_ms.total_cmp(&a.started_at_ms));
        self.segments = cache
            .transcripts
            .iter()
            .map(|(id, records)| (id.clone(), records.as_ref().clone()))
            .collect();
        self.search_cached = cache.transcripts.keys().cloned().collect();
        for (id, fragments) in &cache.fragments {
            let base = self.segments.remove(id).unwrap_or_default();
            self.segments.insert(id.clone(), fragments.merge(base));
            self.search_cached.remove(id);
        }
        self.transcript_indexes = self
            .segments
            .iter()
            .map(|(id, records)| {
                (
                    id.clone(),
                    TranscriptIndex::new(records, cache.fragments.get(id)),
                )
            })
            .collect();
        self.meeting_cache = cache;
        self.bump_search_revision();
    }

    pub(super) fn cache_meeting(&mut self, updated: &Meeting) -> Result<bool, &'static str> {
        validate_meeting(updated)?;
        if self.deleted_meeting_ids.contains(&updated.id) {
            return Err("A late meeting reply cannot restore deleted context. Refresh Meetings to confirm it exists.");
        }
        if let Some(saved) = self.meeting_cache.records.get(&updated.id) {
            if updated.updated_at < saved.updated_at
                || (updated.updated_at == saved.updated_at && updated != saved.as_ref())
            {
                return Err("An older or conflicting meeting reply was ignored. Cached context is unchanged.");
            }
            if updated == saved.as_ref() {
                return Ok(false);
            }
        }
        self.meeting_cache
            .records
            .insert(updated.id.clone(), Arc::new(updated.clone()));
        Ok(true)
    }

    pub(super) fn merge_meetings(
        &mut self,
        meetings: Vec<Meeting>,
        expected: &MeetingCache,
        cx: &mut Context<'_, Self>,
    ) {
        let mut ids = HashSet::new();
        if meetings
            .iter()
            .any(|m| validate_meeting(m).is_err() || !ids.insert(&m.id))
        {
            self.notify_notice("Meeting refresh contains invalid or duplicate identities; cached meetings are unchanged.", true, cx);
            return;
        }
        let mut changed = false;
        // Only a successful authoritative list can remove records, and never
        // a meeting that changed after this particular request began.
        let removed: HashSet<_> = expected
            .records
            .iter()
            .filter(|(id, baseline)| {
                !ids.contains(id)
                    && self.meeting_cache.records.get(*id) == Some(*baseline)
                    && same_arc_snapshot(
                        self.meeting_cache.transcripts.get(*id),
                        expected.transcripts.get(*id),
                    )
                    && match (
                        self.meeting_cache.fragments.get(*id),
                        expected.fragments.get(*id),
                    ) {
                        (Some(current), Some(before)) => current.same_snapshot(before),
                        (None, None) => true,
                        _ => false,
                    }
            })
            .map(|(id, _)| id.clone())
            .collect();
        if !removed.is_empty() {
            if self
                .live_meeting_id
                .as_ref()
                .is_some_and(|id| removed.contains(id))
            {
                self.stop_local_focus("Local app context is off: meeting removed.", cx);
            }
            self.local_focus.rows.retain(|id, _| !removed.contains(id));
            self.deleted_meeting_ids.extend(removed.iter().cloned());
            self.meeting_cache
                .records
                .retain(|id, _| !removed.contains(id));
            self.meeting_cache
                .transcripts
                .retain(|id, _| !removed.contains(id));
            self.segments.retain(|id, _| !removed.contains(id));
            self.removed_chunks.retain(|id, _| !removed.contains(id));
            self.meeting_cache
                .fragments
                .retain(|id, _| !removed.contains(id));
            self.transcript_indexes
                .retain(|id, _| !removed.contains(id));
            self.search_cached.retain(|id| !removed.contains(id));
            self.meetings
                .retain(|meeting| !removed.contains(&meeting.id));
            changed = true;
        }
        let mut positions: std::collections::HashMap<_, _> = self
            .meetings
            .iter()
            .enumerate()
            .map(|(index, meeting)| (meeting.id.clone(), index))
            .collect();
        for meeting in meetings {
            // A fresh authoritative list, unlike a late mutation response,
            // can confirm a restored server record with this identity.
            self.deleted_meeting_ids.remove(&meeting.id);
            match self.cache_meeting(&meeting) {
                Ok(updated) => {
                    changed |= updated;
                    if let Some(index) = positions.get(&meeting.id) {
                        self.meetings[*index] = meeting;
                    } else {
                        positions.insert(meeting.id.clone(), self.meetings.len());
                        self.meetings.push(meeting);
                    }
                }
                Err(error) => self.notify_notice(error, true, cx),
            }
        }
        self.meetings
            .sort_by(|a, b| b.started_at_ms.total_cmp(&a.started_at_ms));
        self.bump_search_revision();
        if changed {
            cx.emit(HubEvent::MeetingsChanged);
        }
    }

    pub(super) fn apply_transcript_snapshot(
        &mut self,
        id: String,
        mut incoming: Vec<TranscriptSegment>,
        cx: &mut Context<'_, Self>,
    ) -> Result<(), &'static str> {
        if !self.meeting_cache.records.contains_key(&id) {
            return Err("Load the meeting before caching its transcript.");
        }
        if let Some(removed) = self.removed_chunks.get(&id) {
            if !removed.is_empty() {
                incoming.retain(|segment| !removed.contains(&segment.id));
            }
        }
        let expected = self
            .segments
            .get(&id)
            .map(Vec::as_slice)
            .unwrap_or_default();
        bruhs_models::validate_transcript_snapshot(&id, expected, &mut incoming)
            .map_err(bruhs_models::TranscriptValidationError::message)?;
        let changed = self
            .meeting_cache
            .transcripts
            .get(&id)
            .is_none_or(|saved| saved.as_ref() != &incoming);
        let had_fragments = self.meeting_cache.fragments.remove(&id).is_some();
        if changed {
            self.meeting_cache
                .transcripts
                .insert(id.clone(), Arc::new(incoming.clone()));
        }
        if changed || had_fragments {
            cx.emit(HubEvent::MeetingsChanged);
        }
        self.transcript_indexes
            .insert(id.clone(), TranscriptIndex::new(&incoming, None));
        self.search_cached.insert(id.clone());
        self.segments.insert(id.clone(), incoming);
        cx.emit(HubEvent::TranscriptChanged {
            meeting_id: id,
            range: None,
        });
        self.bump_search_revision();
        cx.notify();
        Ok(())
    }

    pub(super) fn receive_transcript(
        &mut self,
        id: String,
        result: Result<Vec<TranscriptSegment>, ServerError>,
        cx: &mut Context<'_, Self>,
    ) {
        let result = result
            .map_err(|error| error.to_string())
            .and_then(|segments| {
                self.apply_transcript_snapshot(id, segments, cx)
                    .map_err(str::to_owned)
            });
        if let Err(error) = result {
            self.notify_notice(
                format!("Transcript refresh failed; cached speech is unchanged. {error}"),
                true,
                cx,
            );
        }
    }

    fn transcript_chunk_removed(&self, meeting_id: &str, segment_id: &str) -> bool {
        self.removed_chunks
            .get(meeting_id)
            .is_some_and(|ids| ids.contains(segment_id))
    }

    /// Drop one chunk from the open transcript, its saved snapshot, and stream
    /// fragments. Later replays of `segment_id` are ignored. Returns whether a
    /// visible row changed.
    pub(super) fn forget_transcript_chunk(
        &mut self,
        meeting_id: &str,
        segment_id: &str,
        cx: &mut Context<'_, Self>,
    ) -> bool {
        self.removed_chunks
            .entry(meeting_id.to_owned())
            .or_default()
            .insert(segment_id.to_owned());
        let mut changed = false;
        if let Some(records) = self.segments.get_mut(meeting_id) {
            let before = records.len();
            records.retain(|row| row.id != segment_id);
            changed |= records.len() != before;
        }
        if let Some(snapshot) = self.meeting_cache.transcripts.get(meeting_id).cloned() {
            if snapshot.iter().any(|row| row.id == segment_id) {
                let mut next = snapshot.as_ref().clone();
                next.retain(|row| row.id != segment_id);
                self.meeting_cache
                    .transcripts
                    .insert(meeting_id.to_owned(), Arc::new(next));
                changed = true;
            }
        }
        if self
            .meeting_cache
            .fragments
            .get(meeting_id)
            .is_some_and(|fragments| fragments.iter().any(|row| row.id == segment_id))
        {
            let next = self.meeting_cache.fragments[meeting_id].without(segment_id);
            if next.iter().next().is_none() {
                self.meeting_cache.fragments.remove(meeting_id);
            } else {
                self.meeting_cache
                    .fragments
                    .insert(meeting_id.to_owned(), next);
            }
            changed = true;
        }
        if !changed {
            return false;
        }
        if let Some(records) = self.segments.get(meeting_id) {
            let fragments = self.meeting_cache.fragments.get(meeting_id);
            self.transcript_indexes.insert(
                meeting_id.to_owned(),
                TranscriptIndex::new(records, fragments),
            );
        } else {
            self.transcript_indexes.remove(meeting_id);
        }
        self.search_cached.remove(meeting_id);
        self.bump_search_revision();
        cx.emit(HubEvent::TranscriptChanged {
            meeting_id: meeting_id.to_owned(),
            range: None,
        });
        cx.emit(HubEvent::MeetingsChanged);
        cx.notify();
        true
    }

    pub(super) fn restore_transcript_chunk(
        &mut self,
        segment: TranscriptSegment,
        cx: &mut Context<'_, Self>,
    ) {
        let meeting_id = segment.meeting_id.clone();
        if let Some(ids) = self.removed_chunks.get_mut(&meeting_id) {
            ids.remove(&segment.id);
        }
        if self
            .removed_chunks
            .get(&meeting_id)
            .is_some_and(|ids| ids.is_empty())
        {
            self.removed_chunks.remove(&meeting_id);
        }
        let records = self.segments.entry(meeting_id.clone()).or_default();
        if !records.iter().any(|row| row.id == segment.id) {
            records.push(segment.clone());
            records.sort_unstable_by(|left, right| {
                (left.start_ms, &left.id).cmp(&(right.start_ms, &right.id))
            });
        }
        if let Some(snapshot) = self.meeting_cache.transcripts.get(&meeting_id).cloned() {
            if !snapshot.iter().any(|row| row.id == segment.id) {
                let mut next = snapshot.as_ref().clone();
                next.push(segment.clone());
                next.sort_unstable_by(|left, right| {
                    (left.start_ms, &left.id).cmp(&(right.start_ms, &right.id))
                });
                self.meeting_cache
                    .transcripts
                    .insert(meeting_id.clone(), Arc::new(next));
            }
        }
        let has_snapshot = self.meeting_cache.transcripts.contains_key(&meeting_id);
        if !has_snapshot || self.meeting_cache.fragments.contains_key(&meeting_id) {
            let fragments = self
                .meeting_cache
                .fragments
                .entry(meeting_id.clone())
                .or_default();
            if !fragments.iter().any(|row| row.id == segment.id) {
                let _ = fragments.update(None, segment);
            }
        }
        let records = self
            .segments
            .get(&meeting_id)
            .map(Vec::as_slice)
            .unwrap_or_default();
        let fragments = self.meeting_cache.fragments.get(&meeting_id);
        self.transcript_indexes
            .insert(meeting_id.clone(), TranscriptIndex::new(records, fragments));
        self.search_cached.remove(&meeting_id);
        self.bump_search_revision();
        cx.emit(HubEvent::TranscriptChanged {
            meeting_id,
            range: None,
        });
        cx.emit(HubEvent::MeetingsChanged);
        cx.notify();
    }

    #[cfg(test)]
    pub(crate) fn install_meeting_test_snapshot(
        &mut self,
        meeting: Meeting,
        segments: Vec<TranscriptSegment>,
        cx: &mut Context<'_, Self>,
    ) {
        let id = meeting.id.clone();
        self.replace_meeting(meeting, cx);
        self.apply_transcript_snapshot(id, segments, cx)
            .expect("valid test transcript");
    }
}

fn same_arc_snapshot<T>(current: Option<&Arc<T>>, expected: Option<&Arc<T>>) -> bool {
    match (current, expected) {
        (Some(current), Some(expected)) => Arc::ptr_eq(current, expected),
        (None, None) => true,
        _ => false,
    }
}

#[cfg(test)]
mod tests;

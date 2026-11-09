use super::*;
use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;

#[derive(Default)]
pub(super) struct StreamBuffer {
    pending: BTreeMap<(String, String), Arc<TranscriptSegment>>,
    pending_bytes: usize,
    receipts: BTreeMap<String, Arc<()>>,
    indexes: BTreeMap<String, Index>,
    removed: BTreeMap<String, BTreeSet<String>>,
    scheduled: bool,
    pub gate: Arc<futures_util::lock::Mutex<()>>,
}

struct Index {
    positions: HashMap<String, usize>,
    text_bytes: usize,
}

impl StreamBuffer {
    pub fn receipt(&mut self, id: &str) -> Arc<()> {
        self.receipts.entry(id.into()).or_default().clone()
    }

    pub fn current(&self, id: &str, receipt: &Arc<()>) -> bool {
        self.receipts
            .get(id)
            .is_some_and(|current| Arc::ptr_eq(current, receipt))
    }

    pub fn replaced(&mut self, id: &str) {
        self.receipts.insert(id.into(), Arc::new(()));
        self.indexes.remove(id);
    }

    pub fn receive(
        &mut self,
        id: &str,
        segment: TranscriptSegment,
        records: &mut Vec<TranscriptSegment>,
    ) -> Result<bool, &'static str> {
        bruhs_models::validate_transcript_records(id, std::slice::from_ref(&segment))
            .map_err(bruhs_models::TranscriptValidationError::message)?;
        if self
            .removed
            .get(id)
            .is_some_and(|ids| ids.contains(&segment.id))
        {
            return Ok(false);
        }
        let index = self.indexes.entry(id.into()).or_insert_with(|| Index {
            positions: records
                .iter()
                .enumerate()
                .map(|(position, row)| (row.id.clone(), position))
                .collect(),
            text_bytes: records.iter().map(|row| row.text.len()).sum(),
        });
        let position = index.positions.get(&segment.id).copied();
        let previous = position.and_then(|position| records.get(position));
        if !transcripts::stream::should_write(previous, &segment)? {
            return Ok(false);
        }
        let text_bytes = index
            .text_bytes
            .saturating_sub(previous.map_or(0, |row| row.text.len()))
            .saturating_add(segment.text.len());
        let key = (id.to_owned(), segment.id.clone());
        let pending_bytes = self
            .pending_bytes
            .saturating_sub(self.pending.get(&key).map_or(0, |row| row.text.len()))
            .saturating_add(segment.text.len());
        if (position.is_none() && records.len() >= bruhs_models::MAX_TRANSCRIPT_SEGMENTS)
            || (!self.pending.contains_key(&key)
                && self.pending.len() >= bruhs_models::MAX_TRANSCRIPT_SEGMENTS)
            || text_bytes > 64 * 1024 * 1024
            || pending_bytes > 64 * 1024 * 1024
        {
            return Err("Live transcript limit reached. Existing speech is retained; load the saved transcript to recover remaining words.");
        }
        self.pending.insert(key, Arc::new(segment.clone()));
        self.pending_bytes = pending_bytes;
        self.receipts.insert(id.into(), Arc::new(()));
        index.text_bytes = text_bytes;
        if let Some(position) = position {
            records[position] = segment;
        } else {
            index.positions.insert(segment.id.clone(), records.len());
            records.push(segment);
        }
        Ok(true)
    }

    fn batch(&self, id: &str) -> Vec<Arc<TranscriptSegment>> {
        self.pending
            .range((id.to_owned(), String::new())..)
            .take_while(|((meeting, _), _)| meeting == id)
            .map(|(_, row)| row.clone())
            .collect()
    }

    /// Remember `segment_id` and drop it from `records`.
    ///
    /// The receipt changes only when this call removes something. A repeated
    /// forget must not invalidate an in-flight transcript refresh.
    pub fn forget(
        &mut self,
        meeting_id: &str,
        segment_id: &str,
        records: &mut Vec<TranscriptSegment>,
    ) -> (bool, Option<TranscriptSegment>) {
        let newly_removed = self
            .removed
            .entry(meeting_id.to_owned())
            .or_default()
            .insert(segment_id.to_owned());
        let key = (meeting_id.to_owned(), segment_id.to_owned());
        let dropped_pending = if let Some(pending) = self.pending.remove(&key) {
            self.pending_bytes = self.pending_bytes.saturating_sub(pending.text.len());
            true
        } else {
            false
        };
        let position = records.iter().position(|row| row.id == segment_id);
        if !newly_removed && !dropped_pending && position.is_none() {
            return (false, None);
        }
        self.receipts.insert(meeting_id.to_owned(), Arc::new(()));
        let Some(position) = position else {
            return (true, None);
        };
        let segment = records.remove(position);
        if let Some(index) = self.indexes.get_mut(meeting_id) {
            index.text_bytes = index.text_bytes.saturating_sub(segment.text.len());
            index.positions.clear();
            for (position, row) in records.iter().enumerate() {
                index.positions.insert(row.id.clone(), position);
            }
        }
        (true, Some(segment))
    }

    pub fn restore(&mut self, segment: &TranscriptSegment, records: &mut Vec<TranscriptSegment>) {
        if let Some(ids) = self.removed.get_mut(&segment.meeting_id) {
            ids.remove(&segment.id);
        }
        self.receipts
            .insert(segment.meeting_id.clone(), Arc::new(()));
        if records.iter().any(|row| row.id == segment.id) {
            return;
        }
        records.push(segment.clone());
        records.sort_unstable_by(|left, right| {
            (left.start_ms, &left.id).cmp(&(right.start_ms, &right.id))
        });
        if let Some(index) = self.indexes.get_mut(&segment.meeting_id) {
            index.text_bytes = index.text_bytes.saturating_add(segment.text.len());
            index.positions.clear();
            for (position, row) in records.iter().enumerate() {
                index.positions.insert(row.id.clone(), position);
            }
        }
    }

    fn acknowledge(&mut self, batch: &[Arc<TranscriptSegment>]) {
        for row in batch {
            let key = (row.meeting_id.clone(), row.id.clone());
            if self
                .pending
                .get(&key)
                .is_some_and(|current| Arc::ptr_eq(current, row))
            {
                self.pending.remove(&key);
                self.pending_bytes -= row.text.len();
            }
        }
    }
}

impl TranscriptSync {
    /// Drop one chunk from memory and ignore later replays of its id.
    ///
    /// Returns whether the buffer changed, and the row when it was visible.
    /// A meeting that is not open yet still records the tombstone without
    /// inserting an empty transcript key.
    pub fn forget_chunk(
        self,
        meeting_id: &str,
        segment_id: &str,
    ) -> (bool, Option<TranscriptSegment>) {
        let mut outcome = (false, None);
        self.set_segments.update_untracked(|all| {
            let mut empty = Vec::new();
            let records = all.get_mut(meeting_id).unwrap_or(&mut empty);
            self.stream
                .update_value(|buffer| outcome = buffer.forget(meeting_id, segment_id, records));
        });
        if outcome.1.is_some() {
            self.set_segments.notify();
        }
        outcome
    }

    /// Put a chunk back after the server refused the delete.
    pub fn restore_chunk(self, segment: TranscriptSegment) {
        self.set_segments.update(|all| {
            let records = all.entry(segment.meeting_id.clone()).or_default();
            self.stream
                .update_value(|buffer| buffer.restore(&segment, records));
        });
    }

    pub fn receive_segment(self, id: String, segment: TranscriptSegment) {
        let mut result = Ok(false);
        self.set_segments.update_untracked(|all| {
            self.stream.update_value(|buffer| {
                result = buffer.receive(&id, segment, all.entry(id.clone()).or_default());
            })
        });
        match result {
            Ok(true) => {
                self.set_segments.notify();
                self.schedule_stream_save();
            }
            Ok(false) => {}
            Err(error) => self.report(&id, error.into()),
        }
    }

    fn schedule_stream_save(self) {
        if self
            .stream
            .with_value(|buffer| buffer.scheduled || buffer.pending.is_empty())
        {
            return;
        }
        self.stream.update_value(|buffer| buffer.scheduled = true);
        wasm_bindgen_futures::spawn_local(async move {
            gloo_timers::future::TimeoutFuture::new(500).await;
            let ids = self.stream.with_value(|buffer| {
                buffer
                    .pending
                    .keys()
                    .map(|(id, _)| id.as_str())
                    .collect::<std::collections::BTreeSet<_>>()
                    .into_iter()
                    .map(str::to_owned)
                    .collect::<Vec<_>>()
            });
            let mut failed = false;
            for id in ids {
                if let Err(error) = self.flush_stream(&id).await {
                    failed = true;
                    let warning = format!("Live speech is not saved on this device. {error} Keep this tab open and retry loading the transcript.");
                    self.report(&id, warning.clone());
                    self.banners.push(
                        crate::components::banner::BannerKind::Warning,
                        warning,
                        None,
                    );
                }
            }
            self.stream.update_value(|buffer| buffer.scheduled = false);
            // Retain failed batches, but do not spin on unavailable storage.
            if !failed {
                self.schedule_stream_save();
            }
        });
    }

    pub async fn flush_stream(self, id: &str) -> Result<(), String> {
        let gate = self.stream.with_value(|buffer| buffer.gate.clone());
        let _guard = gate.lock().await;
        let batch = self.stream.with_value(|buffer| buffer.batch(id));
        // Freeze this pass: continued speech cannot indefinitely postpone a refresh.
        for chunk in batch.chunks(256) {
            transcripts::stream::cache_stream(chunk)
                .await
                .map_err(|error| error.to_string())?;
            self.stream.update_value(|buffer| buffer.acknowledge(chunk));
            gloo_timers::future::TimeoutFuture::new(0).await;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;

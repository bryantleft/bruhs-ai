use super::*;
use std::collections::HashMap;

const PAGE_SIZE: usize = 128;

/// Received fragments are durable evidence, not a complete server snapshot.
/// Copy-on-write pages keep snapshots from cloning all earlier speech per event.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub(super) struct StreamFragments(Vec<Arc<Vec<Arc<TranscriptSegment>>>>);

impl StreamFragments {
    fn len(&self) -> usize {
        self.0.len().saturating_sub(1) * PAGE_SIZE + self.0.last().map_or(0, |page| page.len())
    }

    pub(super) fn iter(&self) -> impl Iterator<Item = &TranscriptSegment> {
        self.0.iter().flat_map(|page| page.iter().map(Arc::as_ref))
    }

    pub(super) fn same_snapshot(&self, other: &Self) -> bool {
        self.0.len() == other.0.len() && self.0.iter().zip(&other.0).all(|(a, b)| Arc::ptr_eq(a, b))
    }

    pub(super) fn update(
        &mut self,
        position: Option<usize>,
        segment: TranscriptSegment,
    ) -> Result<usize, &'static str> {
        let index = position.unwrap_or_else(|| self.len());
        if index > self.len() || index >= bruhs_models::MAX_TRANSCRIPT_SEGMENTS {
            return Err("Transcript fragment position exceeds its record limit.");
        }
        if index == self.len() && index.is_multiple_of(PAGE_SIZE) {
            self.0.push(Arc::new(Vec::new()));
        }
        let page = Arc::make_mut(&mut self.0[index / PAGE_SIZE]);
        let offset = index % PAGE_SIZE;
        if offset == page.len() {
            page.push(Arc::new(segment));
        } else {
            page[offset] = Arc::new(segment);
        }
        Ok(index)
    }

    pub(super) fn validate(
        &self,
        id: &str,
        base: &[TranscriptSegment],
    ) -> Result<(), &'static str> {
        if self.0.len() > bruhs_models::MAX_TRANSCRIPT_SEGMENTS.div_ceil(PAGE_SIZE)
            || self.0.iter().enumerate().any(|(index, page)| {
                page.is_empty()
                    || page.len() > PAGE_SIZE
                    || (index + 1 < self.0.len() && page.len() != PAGE_SIZE)
            })
        {
            return Err("Saved transcript fragments have invalid pages.");
        }
        let fragments: Vec<_> = self.iter().cloned().collect();
        bruhs_models::validate_transcript_records(id, &fragments)
            .map_err(bruhs_models::TranscriptValidationError::message)?;
        let existing: HashMap<_, _> = base
            .iter()
            .map(|record| (record.id.as_str(), record))
            .collect();
        if fragments.iter().any(|record| {
            existing
                .get(record.id.as_str())
                .is_some_and(|saved| !saved.partial && *saved != record)
        }) {
            return Err("Saved stream fragments cannot rewrite finalized speech.");
        }
        bruhs_models::validate_transcript_records(id, &self.merge(base.to_vec()))
            .map_err(bruhs_models::TranscriptValidationError::message)
    }

    pub(super) fn merge(&self, mut records: Vec<TranscriptSegment>) -> Vec<TranscriptSegment> {
        let mut positions: HashMap<_, _> = records
            .iter()
            .enumerate()
            .map(|(index, record)| (record.id.clone(), index))
            .collect();
        for record in self.iter() {
            if let Some(index) = positions.get(&record.id) {
                records[*index] = record.clone();
            } else {
                positions.insert(record.id.clone(), records.len());
                records.push(record.clone());
            }
        }
        records.sort_unstable_by(|a, b| (a.start_ms, &a.id).cmp(&(b.start_ms, &b.id)));
        records
    }

    /// Rebuild pages without `segment_id`. Full pages stay full except the last.
    pub(super) fn without(&self, segment_id: &str) -> Self {
        let mut next = Self::default();
        for record in self.iter().filter(|row| row.id != segment_id) {
            if next.update(None, record.clone()).is_err() {
                break;
            }
        }
        next
    }
}

#[derive(Default)]
pub(in crate::views::meetings_hub) struct TranscriptIndex {
    positions: HashMap<String, usize>,
    fragment_positions: HashMap<String, usize>,
    text_bytes: usize,
}

impl TranscriptIndex {
    pub(super) fn new(records: &[TranscriptSegment], fragments: Option<&StreamFragments>) -> Self {
        Self {
            positions: records
                .iter()
                .enumerate()
                .map(|(index, record)| (record.id.clone(), index))
                .collect(),
            fragment_positions: fragments
                .into_iter()
                .flat_map(StreamFragments::iter)
                .enumerate()
                .map(|(index, record)| (record.id.clone(), index))
                .collect(),
            text_bytes: records.iter().map(|record| record.text.len()).sum(),
        }
    }
}

impl MeetingsHub {
    pub(in crate::views::meetings_hub) fn receive_segment(
        &mut self,
        id: String,
        segment: TranscriptSegment,
        cx: &mut Context<'_, Self>,
    ) {
        let segment_id = segment.id.clone();
        match self.apply_stream_segment(&id, segment) {
            Ok(true) => {
                self.bump_search_revision();
                if let Some(position) = self
                    .transcript_indexes
                    .get(&id)
                    .and_then(|index| index.positions.get(&segment_id))
                {
                    cx.emit(HubEvent::TranscriptChanged {
                        meeting_id: id,
                        range: Some(*position..*position + 1),
                    });
                }
                cx.emit(HubEvent::MeetingsChanged);
            }
            Ok(false) => {}
            Err(error) => self.notify_notice(error, true, cx),
        }
    }

    fn apply_stream_segment(
        &mut self,
        id: &str,
        segment: TranscriptSegment,
    ) -> Result<bool, &'static str> {
        if !self.meeting_cache.records.contains_key(id) || self.deleted_meeting_ids.contains(id) {
            return Err("Load the meeting before accepting its streamed transcript.");
        }
        bruhs_models::validate_transcript_records(id, std::slice::from_ref(&segment))
            .map_err(bruhs_models::TranscriptValidationError::message)?;
        if self.transcript_chunk_removed(id, &segment.id) {
            return Ok(false);
        }
        let records = self.segments.entry(id.into()).or_default();
        let index = self
            .transcript_indexes
            .entry(id.into())
            .or_insert_with(|| TranscriptIndex::new(records, self.meeting_cache.fragments.get(id)));
        let position = index.positions.get(&segment.id).copied();
        let previous = position.and_then(|position| records.get(position));
        if previous == Some(&segment) {
            return Ok(false);
        }
        if previous.is_some_and(|record| !record.partial) {
            return Err("A stream replay cannot rewrite finalized speech. Refresh the transcript to load verified annotations.");
        }
        if position.is_none() && records.len() >= bruhs_models::MAX_TRANSCRIPT_SEGMENTS {
            return Err("Transcript record limit reached; cached speech is unchanged.");
        }
        let text_bytes = index
            .text_bytes
            .saturating_sub(previous.map_or(0, |record| record.text.len()))
            .saturating_add(segment.text.len());
        if text_bytes > 64 * 1024 * 1024 {
            return Err("Transcript text limit reached; cached speech is unchanged.");
        }
        let fragment_position = self
            .meeting_cache
            .fragments
            .entry(id.into())
            .or_default()
            .update(
                index.fragment_positions.get(&segment.id).copied(),
                segment.clone(),
            )?;
        index
            .fragment_positions
            .insert(segment.id.clone(), fragment_position);
        index.text_bytes = text_bytes;
        if let Some(position) = position {
            records[position] = segment;
        } else {
            index.positions.insert(segment.id.clone(), records.len());
            records.push(segment);
        }
        self.search_cached.remove(id);
        Ok(true)
    }
}

#[cfg(test)]
mod tests;

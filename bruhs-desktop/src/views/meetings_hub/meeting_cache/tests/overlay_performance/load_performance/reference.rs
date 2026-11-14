//! Frozen pre-change restoration, used only as a controlled test reference.
use super::*;

impl MeetingsHub {
    pub(super) fn restore_original(&mut self, cache: MeetingCache) {
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
}

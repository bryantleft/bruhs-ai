//! Stable transcript rows. A partial/final segment update changes that row's
//! signal without replacing the other text or speaker editors.

use crate::state::TranscriptSegment;
use leptos::prelude::*;
use std::collections::HashMap;
use std::sync::Arc;

pub(super) type TranscriptRows = Vec<ArcRwSignal<TranscriptSegment>>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct TranscriptGroup {
    pub speaker_id: Option<Arc<str>>,
    pub rows: ArcRwSignal<TranscriptRows>,
}

struct CachedSegment {
    value: ArcRwSignal<TranscriptSegment>,
    seen: bool,
}

struct CachedGroup {
    value: TranscriptGroup,
    seen: bool,
}

#[derive(Default)]
pub(super) struct TranscriptProjection {
    segments: HashMap<String, CachedSegment>,
    groups: HashMap<String, CachedGroup>,
    ordered: Vec<TranscriptGroup>,
    seen: bool,
}

impl TranscriptProjection {
    /// Keeps all source records and their order. Only new or changed records
    /// copy text; unchanged rows and speaker runs retain their signal identity.
    /// Returns whether the ordered group list changed; row/member signals are
    /// published independently.
    pub(super) fn sync(&mut self, segments: &[TranscriptSegment]) -> bool {
        struct Run<'a> {
            first_id: &'a str,
            speaker_id: Option<&'a str>,
            rows: TranscriptRows,
        }

        if self.update_existing_rows(segments) {
            return false;
        }

        self.seen = !self.seen;
        self.segments
            .reserve(segments.len().saturating_sub(self.segments.len()));
        let mut runs: Vec<Run<'_>> = Vec::with_capacity(self.ordered.len());
        for segment in segments {
            let row = if let Some(cached) = self.segments.get_mut(&segment.id) {
                cached.seen = self.seen;
                if cached.value.with_untracked(|saved| saved != segment) {
                    cached.value.set(segment.clone());
                }
                cached.value.clone()
            } else {
                let value = ArcRwSignal::new(segment.clone());
                self.segments.insert(
                    segment.id.clone(),
                    CachedSegment {
                        value: value.clone(),
                        seen: self.seen,
                    },
                );
                value
            };
            if let Some(run) = runs.last_mut() {
                if run.speaker_id == segment.speaker_id.as_deref() {
                    run.rows.push(row);
                    continue;
                }
            }
            runs.push(Run {
                first_id: &segment.id,
                speaker_id: segment.speaker_id.as_deref(),
                rows: vec![row],
            });
        }

        self.groups
            .reserve(runs.len().saturating_sub(self.groups.len()));
        let mut ordered = Vec::with_capacity(runs.len());
        for run in runs {
            let group = match self.groups.get_mut(run.first_id) {
                Some(cached) if cached.value.speaker_id.as_deref() == run.speaker_id => {
                    cached.seen = self.seen;
                    if cached.value.rows.with_untracked(|rows| *rows != run.rows) {
                        cached.value.rows.set(run.rows);
                    }
                    cached.value.clone()
                }
                _ => {
                    let value = TranscriptGroup {
                        speaker_id: run.speaker_id.map(Arc::from),
                        rows: ArcRwSignal::new(run.rows),
                    };
                    self.groups.insert(
                        run.first_id.to_string(),
                        CachedGroup {
                            value: value.clone(),
                            seen: self.seen,
                        },
                    );
                    value
                }
            };
            ordered.push(group);
        }
        self.segments.retain(|_, row| row.seen == self.seen);
        self.groups.retain(|_, group| group.seen == self.seen);
        let changed = self.ordered != ordered;
        self.ordered = ordered;
        changed
    }

    /// Text, confidence and partial/final updates do not need to rebuild run
    /// membership or look up every source ID in the maps. Check structure and
    /// collect changed rows in one borrowed pass before publishing anything.
    fn update_existing_rows(&self, segments: &[TranscriptSegment]) -> bool {
        if segments.len() != self.segments.len() {
            return false;
        }
        let mut incoming = segments.iter();
        let mut updates = Vec::new();
        let same_structure = self.ordered.iter().all(|group| {
            group.rows.with_untracked(|rows| {
                rows.iter().all(|value| {
                    let Some(segment) = incoming.next() else {
                        return false;
                    };
                    value.with_untracked(|saved| {
                        if saved.id != segment.id || saved.speaker_id != segment.speaker_id {
                            return false;
                        }
                        if saved != segment {
                            updates.push((value.clone(), segment));
                        }
                        true
                    })
                })
            })
        });
        if !same_structure || incoming.next().is_some() {
            return false;
        }
        for (value, segment) in updates {
            value.set(segment.clone());
        }
        true
    }

    pub(super) fn groups(&self) -> &[TranscriptGroup] {
        &self.ordered
    }
}

#[cfg(test)]
mod tests;

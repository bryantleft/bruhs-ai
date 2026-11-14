//! Validate durable fragments against their base without materializing speech text.
use super::{transcript_identities, TranscriptSegment, TranscriptValidationError};
use std::collections::HashMap;

/// Validate fragment identities, finalized speech and the complete merged record set.
/// Neither input is cloned or reordered. Publishing the overlay remains the caller's job.
pub fn validate_transcript_overlay(
    meeting_id: &str,
    base: &[TranscriptSegment],
    fragments: &[&TranscriptSegment],
) -> Result<(), &'static str> {
    transcript_identities(meeting_id, fragments.iter().copied())
        .map_err(TranscriptValidationError::message)?;
    let existing: HashMap<_, _> = base
        .iter()
        .map(|record| (record.id.as_str(), record))
        .collect();
    if fragments.iter().any(|record| {
        existing
            .get(record.id.as_str())
            .is_some_and(|saved| !saved.partial && *saved != *record)
    }) {
        return Err("Saved stream fragments cannot rewrite finalized speech.");
    }
    let mut merged: Vec<_> = base.iter().collect();
    let mut positions: HashMap<_, _> = base
        .iter()
        .enumerate()
        .map(|(index, record)| (record.id.as_str(), index))
        .collect();
    for &record in fragments {
        if let Some(index) = positions.get(record.id.as_str()) {
            merged[*index] = record;
        } else {
            positions.insert(record.id.as_str(), merged.len());
            merged.push(record);
        }
    }
    // Sorting affects publication order, not validation: all malformed rows have
    // the same refusal, and the record-limit check still runs before row checks.
    transcript_identities(meeting_id, merged.into_iter())
        .map(|_| ())
        .map_err(TranscriptValidationError::message)
}

#[cfg(test)]
mod tests;

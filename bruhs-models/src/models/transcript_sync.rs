//! Shared capture-snapshot validation for server and browser caches.
use super::TranscriptSegment;
use std::collections::HashMap;

pub const MAX_TRANSCRIPT_SEGMENTS: usize = 100_000;

/// Why a transcript snapshot or record set was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("{}", self.message())]
pub enum TranscriptValidationError {
    /// An incomplete snapshot drops a finalized segment.
    RemovesSavedSegment,
    RewritesFinalizedSpeech,
    /// Blank meeting id or more than `MAX_TRANSCRIPT_SEGMENTS` records.
    TooLarge,
    /// A malformed, duplicate or foreign segment.
    InvalidSegments,
}

impl TranscriptValidationError {
    /// The user-facing sentence.
    pub const fn message(self) -> &'static str {
        match self {
            Self::RemovesSavedSegment => {
                "An incomplete capture snapshot cannot remove saved transcript segments."
            }
            Self::RewritesFinalizedSpeech => "A capture replay cannot rewrite finalized speech.",
            Self::TooLarge => "Transcript snapshot exceeds its identity or record limits.",
            Self::InvalidSegments => {
                "Transcript has invalid, duplicate or foreign segments; saved data was retained."
            }
        }
    }
}

pub fn validate_transcript_snapshot(
    meeting_id: &str,
    expected: &[TranscriptSegment],
    incoming: &mut [TranscriptSegment],
) -> Result<(), TranscriptValidationError> {
    let identities = transcript_identities(meeting_id, incoming)?;
    for saved in expected {
        let Some(next) = identities.get(saved.id.as_str()) else {
            if saved.partial {
                continue;
            }
            return Err(TranscriptValidationError::RemovesSavedSegment);
        };
        // Diarization changes annotations, not already finalized words.
        if !saved.partial
            && (next.partial
                || next.text != saved.text
                || next.start_ms != saved.start_ms
                || next.end_ms != saved.end_ms
                || next.confidence != saved.confidence)
        {
            return Err(TranscriptValidationError::RewritesFinalizedSpeech);
        }
    }
    incoming.sort_unstable_by(|a, b| (a.start_ms, &a.id).cmp(&(b.start_ms, &b.id)));
    Ok(())
}

/// Validate a borrowed transcript without cloning its text or reordering it.
pub fn validate_transcript_records(
    meeting_id: &str,
    records: &[TranscriptSegment],
) -> Result<(), TranscriptValidationError> {
    transcript_identities(meeting_id, records).map(|_| ())
}

fn transcript_identities<'a>(
    meeting_id: &str,
    incoming: &'a [TranscriptSegment],
) -> Result<HashMap<&'a str, &'a TranscriptSegment>, TranscriptValidationError> {
    if meeting_id.trim().is_empty() || incoming.len() > MAX_TRANSCRIPT_SEGMENTS {
        return Err(TranscriptValidationError::TooLarge);
    }
    let mut identities = HashMap::with_capacity(incoming.len());
    let mut text_bytes = 0usize;
    for segment in incoming.iter() {
        text_bytes = text_bytes.saturating_add(segment.text.len());
        if segment.meeting_id != meeting_id
            || segment.id.trim().is_empty()
            || segment.id.len() > 1024
            || segment.meeting_id.len() > 1024
            || segment
                .speaker_id
                .as_ref()
                .is_some_and(|id| id.trim().is_empty() || id.len() > 1024)
            || segment.end_ms < segment.start_ms
            || !segment.confidence.is_finite()
            || segment
                .speaker_confidence
                .is_some_and(|value| !value.is_finite())
            || segment.text.len() > 64 * 1024
            || text_bytes > 64 * 1024 * 1024
            || identities.insert(segment.id.as_str(), segment).is_some()
        {
            return Err(TranscriptValidationError::InvalidSegments);
        }
    }
    Ok(identities)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn segment(id: &str, text: &str, partial: bool) -> TranscriptSegment {
        TranscriptSegment {
            id: id.into(),
            meeting_id: "m".into(),
            speaker_id: Some("me".into()),
            start_ms: 0,
            end_ms: 10,
            text: text.into(),
            confidence: 0.9,
            partial,
            speaker_confidence: None,
        }
    }

    #[test]
    fn each_refusal_has_its_own_variant_and_unchanged_text() {
        assert_eq!(
            validate_transcript_records("", &[]),
            Err(TranscriptValidationError::TooLarge)
        );
        let mut foreign = segment("s", "hi", false);
        foreign.meeting_id = "other".into();
        assert_eq!(
            validate_transcript_records("m", &[foreign]),
            Err(TranscriptValidationError::InvalidSegments)
        );
        let saved = [segment("s", "hello", false)];
        assert_eq!(
            validate_transcript_snapshot("m", &saved, &mut [segment("s", "edited", false)]),
            Err(TranscriptValidationError::RewritesFinalizedSpeech)
        );
        assert_eq!(
            validate_transcript_snapshot("m", &saved, &mut []),
            Err(TranscriptValidationError::RemovesSavedSegment)
        );
        assert_eq!(
            validate_transcript_snapshot("m", &[segment("p", "draft", true)], &mut []),
            Ok(())
        );
        assert_eq!(
            TranscriptValidationError::RewritesFinalizedSpeech.to_string(),
            "A capture replay cannot rewrite finalized speech."
        );
    }
}

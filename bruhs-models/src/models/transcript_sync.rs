//! Shared capture-snapshot validation for server and browser caches.
use super::TranscriptSegment;
use std::collections::HashMap;
mod overlay;
pub use overlay::validate_transcript_overlay;

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
    let identities = transcript_identities(meeting_id, incoming.iter())?;
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
    transcript_identities(meeting_id, records.iter()).map(|_| ())
}

fn transcript_identities<'a>(
    meeting_id: &str,
    incoming: impl ExactSizeIterator<Item = &'a TranscriptSegment>,
) -> Result<HashMap<&'a str, &'a TranscriptSegment>, TranscriptValidationError> {
    if meeting_id.trim().is_empty() || incoming.len() > MAX_TRANSCRIPT_SEGMENTS {
        return Err(TranscriptValidationError::TooLarge);
    }
    let mut identities = HashMap::with_capacity(incoming.len());
    let mut text_bytes = 0usize;
    for segment in incoming {
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

    #[test]
    fn record_and_snapshot_validation_check_corruption_beyond_the_first_page() {
        let records = (0..123)
            .map(|index| segment(&format!("s{index}"), "Saved words ΟΣ", index == 122))
            .collect::<Vec<_>>();
        let before = records.clone();
        assert_eq!(validate_transcript_records("m", &records), Ok(()));
        assert_eq!(records, before);
        let corruptions: [fn(&mut TranscriptSegment); 11] = [
            |record| record.id.clear(),
            |record| record.id = "x".repeat(1025),
            |record| record.id = "s0".into(),
            |record| record.meeting_id = "foreign".into(),
            |record| record.speaker_id = Some(" ".into()),
            |record| record.speaker_id = Some("x".repeat(1025)),
            |record| {
                record.start_ms = 11;
                record.end_ms = 10;
            },
            |record| record.confidence = f32::NAN,
            |record| record.speaker_confidence = Some(f32::INFINITY),
            |record| record.text = "x".repeat(64 * 1024 + 1),
            |record| record.id = "\n\t".into(),
        ];
        for corrupt in corruptions {
            let mut records = records.clone();
            corrupt(&mut records[122]);
            // Debug preserves NaN for the immutability check; float equality does not.
            let before = format!("{records:?}");
            assert_eq!(
                validate_transcript_records("m", &records),
                Err(TranscriptValidationError::InvalidSegments)
            );
            assert_eq!(
                validate_transcript_snapshot("m", &[], &mut records),
                Err(TranscriptValidationError::InvalidSegments)
            );
            assert_eq!(format!("{records:?}"), before);
        }
        let mut too_many = vec![segment("s", "saved", true); MAX_TRANSCRIPT_SEGMENTS + 1];
        assert_eq!(
            validate_transcript_records("m", &too_many),
            Err(TranscriptValidationError::TooLarge)
        );
        assert_eq!(
            validate_transcript_snapshot("m", &[], &mut too_many),
            Err(TranscriptValidationError::TooLarge)
        );
    }

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

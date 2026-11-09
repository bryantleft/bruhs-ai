//! Generating a meeting summary under a durable request identity: reserve
//! the identity, generate, then commit the summary and its observations
//! atomically against the exact input snapshot. A request identity runs the
//! model at most once; a replay only reads its receipt.

use std::future::Future;

use bruhs_models::{
    Meeting, MeetingSummary, Speaker, SummaryGenerationReceipt, SummaryGenerationRequest,
    SummaryGenerationState, TranscriptSegment, TranscriptValidationError,
};

use crate::meetings::{
    GenerationOutcome, GenerationReservation, MeetingStore, MeetingStoreError, SummaryUpdate,
};
use crate::summarize::parser::{self, InvalidObservations, PersonObservationRaw};
use crate::summarize::SummarizeError;

/// Why no summary was returned. Except where noted, the meeting's existing
/// summary and observations are unchanged.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum SummaryGenerationError {
    /// The request identity was already used with different options.
    #[error("Generation request identity was already used with different options.")]
    IdentityReused,
    /// The identity's generation has not settled.
    #[error("Generation remains unresolved. Check its receipt; cancel it explicitly before requesting a new generation.")]
    Unresolved,
    /// The identity's generation failed or was cancelled; it never runs
    /// again.
    #[error("This generation ended without a committed result. Start a new explicit request; this request will not run again.")]
    Ended,
    /// The transcript is not a consistent snapshot of the meeting.
    #[error("{0}")]
    InvalidTranscript(#[source] TranscriptValidationError),
    /// The transcript is empty or still has partial segments.
    #[error(
        "A complete saved transcript is required; existing summary and observations were retained."
    )]
    IncompleteTranscript,
    /// The model did not produce a summary.
    #[error("Summary generation failed. Existing summary and observations were retained; retry when the model is available.")]
    Generation(#[source] SummarizeError),
    /// The model's observations did not parse.
    #[error("{0}")]
    InvalidObservations(#[source] InvalidObservations),
    /// The commit succeeded but returned no summary (a store bug).
    #[error("Summary transaction returned no saved summary.")]
    NoSavedSummary,
    #[error(transparent)]
    Store(#[from] MeetingStoreError),
}

/// The result a settled receipt stands for, for a request with the same
/// identity and options.
///
/// # Errors
///
/// [`SummaryGenerationError::IdentityReused`] when the options differ,
/// [`SummaryGenerationError::Unresolved`] while pending and
/// [`SummaryGenerationError::Ended`] for a failed or cancelled generation.
pub fn receipt_result(
    receipt: SummaryGenerationReceipt,
    request: &SummaryGenerationRequest,
) -> Result<MeetingSummary, SummaryGenerationError> {
    if &receipt.request != request {
        return Err(SummaryGenerationError::IdentityReused);
    }
    match receipt.state {
        SummaryGenerationState::Succeeded { summary } => Ok(summary),
        SummaryGenerationState::Pending => Err(SummaryGenerationError::Unresolved),
        SummaryGenerationState::Failed | SummaryGenerationState::Cancelled => {
            Err(SummaryGenerationError::Ended)
        }
    }
}

/// Reserve `request`, run `generate` on the validated `segments`, and
/// commit the summary (and, when requested, the observations) atomically
/// against `meeting` as read before generation. An identity seen before
/// returns its receipt's result without calling `generate`.
///
/// `generate` gets the segments, the meeting's speakers and whether to
/// extract observations (requested and not opted out by the meeting).
///
/// A failure after the reservation settles the receipt as failed; if even
/// that fails, the receipt stays pending, so a possibly committed result is
/// never replayed automatically.
pub async fn generate_and_commit<F, Fut>(
    store: &dyn MeetingStore,
    meeting: Meeting,
    mut segments: Vec<TranscriptSegment>,
    request: SummaryGenerationRequest,
    generate: F,
) -> Result<MeetingSummary, SummaryGenerationError>
where
    F: FnOnce(Vec<TranscriptSegment>, Vec<Speaker>, bool) -> Fut,
    Fut: Future<Output = Result<(MeetingSummary, Vec<PersonObservationRaw>), SummarizeError>>,
{
    match store.begin_summary_generation(request.clone()).await? {
        GenerationReservation::Existing(receipt) => return receipt_result(receipt, &request),
        GenerationReservation::Started => {}
    }
    let result = async {
        bruhs_models::validate_transcript_snapshot(&meeting.id, &[], &mut segments)
            .map_err(SummaryGenerationError::InvalidTranscript)?;
        if segments.is_empty() || segments.iter().any(|segment| segment.partial) {
            return Err(SummaryGenerationError::IncompleteTranscript);
        }
        let extract = request.extract_observations && !meeting.skip_observations;
        let people_versions = store.summary_people_versions(&meeting).await?;
        let (summary, raw) = generate(segments.clone(), meeting.speakers.clone(), extract)
            .await
            .map_err(SummaryGenerationError::Generation)?;
        let observations = if extract {
            parser::group_observations_strict(
                &raw,
                &meeting.id,
                chrono::Utc::now().timestamp_millis(),
            )
            .map_err(SummaryGenerationError::InvalidObservations)?
        } else {
            Default::default()
        };
        let updated = store
            .commit_summary_generation(
                &request.request_id,
                SummaryUpdate {
                    expected: meeting,
                    people_versions,
                    transcript: segments,
                    summary,
                    observations,
                    replace_observations: extract && request.replace_observations,
                },
            )
            .await?;
        updated
            .summary
            .ok_or(SummaryGenerationError::NoSavedSummary)
    }
    .await;
    if result.is_err() {
        if let Err(error) = store
            .finish_summary_generation(
                &request.meeting_id,
                &request.request_id,
                GenerationOutcome::Failed,
            )
            .await
        {
            tracing::warn!("Could not settle summary generation receipt: {error}");
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn summary() -> MeetingSummary {
        MeetingSummary {
            tldr: "Done".into(),
            action_items: vec![],
            decisions: vec![],
            topics: vec![],
            generated_by: "fixture".into(),
            generated_at: 1,
        }
    }

    #[test]
    fn receipts_replay_only_for_the_same_options() {
        let request = SummaryGenerationRequest::new("m".into(), true, false);
        let receipt = |state| SummaryGenerationReceipt {
            request: request.clone(),
            state,
        };
        assert_eq!(
            receipt_result(
                receipt(SummaryGenerationState::Succeeded { summary: summary() }),
                &request
            )
            .unwrap(),
            summary()
        );
        let mut other = request.clone();
        other.replace_observations = true;
        assert!(matches!(
            receipt_result(receipt(SummaryGenerationState::Pending), &other),
            Err(SummaryGenerationError::IdentityReused)
        ));
        assert!(matches!(
            receipt_result(receipt(SummaryGenerationState::Pending), &request),
            Err(SummaryGenerationError::Unresolved)
        ));
        assert!(matches!(
            receipt_result(receipt(SummaryGenerationState::Cancelled), &request),
            Err(SummaryGenerationError::Ended)
        ));
    }

    /// The provider's error stays attached as the source instead of being
    /// reduced to a generic message.
    #[tokio::test]
    async fn a_provider_failure_keeps_its_cause_and_settles_the_receipt() {
        use crate::meetings::{InMemoryMeetingStore, MeetingRecords, SummaryGenerationStore};
        let store = InMemoryMeetingStore::new();
        let meeting = store
            .create_meeting(Meeting::new("Fixture", 0.0))
            .await
            .unwrap();
        let segment = TranscriptSegment {
            id: "s".into(),
            meeting_id: meeting.id.clone(),
            speaker_id: None,
            start_ms: 0,
            end_ms: 1,
            text: "Final".into(),
            confidence: 1.0,
            partial: false,
            speaker_confidence: None,
        };
        let request = SummaryGenerationRequest::new(meeting.id.clone(), false, false);
        let error = generate_and_commit(
            &store,
            meeting,
            vec![segment],
            request.clone(),
            |_, _, _| async { Err(SummarizeError::Provider("rate limited".into())) },
        )
        .await
        .unwrap_err();
        assert!(matches!(error, SummaryGenerationError::Generation(_)));
        assert_eq!(
            std::error::Error::source(&error).map(ToString::to_string),
            Some(SummarizeError::Provider("rate limited".into()).to_string())
        );
        let receipt = store
            .summary_generation(&request.meeting_id, &request.request_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(receipt.state, SummaryGenerationState::Failed);
    }
}

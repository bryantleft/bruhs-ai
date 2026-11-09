use super::{MeetingStoreError, MeetingStoreResult};
use bruhs_models::{SummaryGenerationReceipt, SummaryGenerationRequest, SummaryGenerationState};

/// Only a newly reserved request may invoke the model. Replays are read-only.
pub enum GenerationReservation {
    Started,
    Existing(SummaryGenerationReceipt),
}

/// How a generation that will not be committed ended; see
/// [`super::SummaryGenerationStore::finish_summary_generation`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GenerationOutcome {
    /// The provider or the commit failed.
    Failed,
    /// The user cancelled the request.
    Cancelled,
}

impl GenerationOutcome {
    /// The receipt state a still-pending generation settles into.
    pub(super) fn state(self) -> SummaryGenerationState {
        match self {
            Self::Failed => SummaryGenerationState::Failed,
            Self::Cancelled => SummaryGenerationState::Cancelled,
        }
    }
}

pub(super) fn validate(request: &SummaryGenerationRequest) -> MeetingStoreResult<()> {
    if !request.valid() {
        return Err(MeetingStoreError::InvalidInput(
            "Generation request and meeting identities must be nonempty and at most 128 bytes."
                .into(),
        ));
    }
    Ok(())
}

pub(super) fn replay(
    receipt: SummaryGenerationReceipt,
    request: &SummaryGenerationRequest,
) -> MeetingStoreResult<GenerationReservation> {
    if &receipt.request != request {
        return Err(MeetingStoreError::Conflict(
            "Generation request identity was already used with different options.".into(),
        ));
    }
    Ok(GenerationReservation::Existing(receipt))
}

pub(super) fn pending(
    receipt: &SummaryGenerationReceipt,
    update: &super::SummaryUpdate,
) -> MeetingStoreResult<()> {
    if !matches!(receipt.state, SummaryGenerationState::Pending) {
        return Err(MeetingStoreError::Conflict(
            "Generation is no longer pending; its result cannot replace current data.".into(),
        ));
    }
    if receipt.request.meeting_id != update.expected.id
        || (!receipt.request.extract_observations
            && (!update.observations.is_empty() || update.replace_observations))
        || (!receipt.request.replace_observations && update.replace_observations)
    {
        return Err(MeetingStoreError::Conflict(
            "Generation result exceeds its reserved observation options.".into(),
        ));
    }
    Ok(())
}

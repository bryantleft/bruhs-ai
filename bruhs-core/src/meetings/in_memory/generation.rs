use super::*;
use crate::meetings::{generation as rules, GenerationReservation, SummaryUpdate};
use bruhs_models::{SummaryGenerationReceipt, SummaryGenerationRequest, SummaryGenerationState};

impl InMemoryMeetingStore {
    pub(super) async fn begin_generation(
        &self,
        request: SummaryGenerationRequest,
    ) -> MeetingStoreResult<GenerationReservation> {
        rules::validate(&request)?;
        // All generation operations acquire this gate before meeting/people locks.
        let mut receipts = self.generations.write().await;
        if self.get_meeting(&request.meeting_id).await?.is_none() {
            return Err(MeetingStoreError::MeetingNotFound {
                id: request.meeting_id,
            });
        }
        if let Some(receipt) = receipts.get(&request.request_id) {
            return rules::replay(receipt.clone(), &request);
        }
        if receipts.values().any(|receipt| {
            receipt.request.meeting_id == request.meeting_id
                && matches!(receipt.state, SummaryGenerationState::Pending)
        }) {
            return Err(MeetingStoreError::Conflict("This meeting has an unresolved generation. Review or cancel that request before starting another.".into()));
        }
        receipts.insert(
            request.request_id.clone(),
            SummaryGenerationReceipt {
                request,
                state: SummaryGenerationState::Pending,
            },
        );
        drop(receipts);
        Ok(GenerationReservation::Started)
    }

    pub(super) async fn finish_generation(
        &self,
        meeting: &str,
        id: &str,
        outcome: super::super::GenerationOutcome,
    ) -> MeetingStoreResult<SummaryGenerationReceipt> {
        let mut receipts = self.generations.write().await;
        let receipt = receipts
            .get_mut(id)
            .filter(|receipt| receipt.request.meeting_id == meeting)
            .ok_or_else(|| {
                MeetingStoreError::InvalidInput("Generation request was not found.".into())
            })?;
        if matches!(receipt.state, SummaryGenerationState::Pending) {
            receipt.state = outcome.state();
        }
        let receipt = receipt.clone();
        drop(receipts);
        Ok(receipt)
    }

    pub(super) async fn fence_generation(
        &self,
        request: SummaryGenerationRequest,
    ) -> MeetingStoreResult<SummaryGenerationReceipt> {
        rules::validate(&request)?;
        let mut receipts = self.generations.write().await;
        if self.get_meeting(&request.meeting_id).await?.is_none() {
            return Err(MeetingStoreError::MeetingNotFound {
                id: request.meeting_id,
            });
        }
        if let Some(receipt) = receipts.get_mut(&request.request_id) {
            rules::replay(receipt.clone(), &request)?;
            if matches!(receipt.state, SummaryGenerationState::Pending) {
                receipt.state = SummaryGenerationState::Cancelled;
            }
            return Ok(receipt.clone());
        }
        let receipt = SummaryGenerationReceipt {
            request,
            state: SummaryGenerationState::Cancelled,
        };
        receipts.insert(receipt.request.request_id.clone(), receipt.clone());
        drop(receipts);
        Ok(receipt)
    }

    pub(super) async fn commit_generation(
        &self,
        id: &str,
        update: SummaryUpdate,
    ) -> MeetingStoreResult<Meeting> {
        let mut receipts = self.generations.write().await;
        let receipt = receipts
            .get_mut(id)
            .filter(|receipt| receipt.request.meeting_id == update.expected.id)
            .ok_or_else(|| {
                MeetingStoreError::Conflict(
                    "Generation reservation is missing; nothing was replaced.".into(),
                )
            })?;
        rules::pending(receipt, &update)?;
        let summary = update.summary.clone();
        let meeting = self.commit_summary(update).await?;
        receipt.state = SummaryGenerationState::Succeeded { summary };
        // The gate spans the commit so the receipt and the summary move together.
        drop(receipts);
        Ok(meeting)
    }
}

use super::*;
use bruhs_models::{MeetingSummary, SummaryGenerationReceipt};

pub(super) struct WebTransport {
    pub actions: MeetingActions,
    pub baseline: Option<Meeting>,
}

impl flow::Transport for WebTransport {
    async fn reserve(&self, checkpoint: &Checkpoint) -> Result<generations::Reservation, String> {
        generations::begin(checkpoint)
            .await
            .map_err(|error| error.to_string())
    }

    fn current_matches(&self, checkpoint: &Checkpoint) -> bool {
        let unchanged = self.actions.meetings.with_untracked(|all| {
            let mut matching = all.iter().filter(|current| current.id == checkpoint.id);
            matching.next() == self.baseline.as_ref() && matching.next().is_none()
        });
        unchanged
            && (!checkpoint.request.extract_observations
                || self
                    .actions
                    .personality
                    .get_untracked()
                    .extract_observations)
    }

    async fn send(&self, checkpoint: &Checkpoint) -> Result<MeetingSummary, String> {
        let request = &checkpoint.request;
        deadline(
            async {
                api::generate_meeting_summary(
                    &checkpoint.id,
                    &request.request_id,
                    request.extract_observations,
                    request.replace_observations,
                )
                .await
                .map_err(|error| api::summary_error_message(&error))
            },
            135_000,
        )
        .await
    }

    async fn receipt(
        &self,
        checkpoint: &Checkpoint,
    ) -> Result<Option<SummaryGenerationReceipt>, String> {
        deadline(
            api::summary_generation(&checkpoint.id, &checkpoint.request.request_id),
            15_000,
        )
        .await
    }

    async fn cache(
        &self,
        checkpoint: &Checkpoint,
        expected: Option<MeetingSummary>,
    ) -> Result<(), String> {
        self.actions
            .readback(
                &checkpoint.id,
                expected.map(|summary| Expected {
                    summary: Some(summary),
                    skip: None,
                }),
                checkpoint.minimum_version,
            )
            .await
            .map_err(|(error, _)| error)
    }

    async fn clear(&self, checkpoint: &Checkpoint) -> Result<(), String> {
        generations::clear(checkpoint)
            .await
            .map_err(|error| error.to_string())
    }

    async fn cancel(
        &self,
        checkpoint: &Checkpoint,
    ) -> Result<Option<SummaryGenerationReceipt>, String> {
        deadline(
            async {
                api::cancel_summary_generation(&checkpoint.request)
                    .await
                    .map_err(|error| api::summary_error_message(&error))
            },
            15_000,
        )
        .await
    }
}

use super::*;

pub(super) struct Failure {
    pub message: &'static str,
    pub expected: Option<Box<Expected>>,
}
pub(super) type Outcome = Result<Meeting, Failure>;

pub(super) async fn perform(
    client: &ServerClient,
    id: &str,
    action: Action,
    mut checkpoint: Checkpoint,
) -> Outcome {
    let meeting = match action {
        Action::Generate { extract, replace } => {
            let summary = client.summarize_meeting(id, &checkpoint.token, extract, replace).await.map_err(|_| Failure {
                message: "Summary not confirmed; the server outcome may be uncertain. Load the latest saved meeting before trying again.", expected: None,
            })?;
            checkpoint.expected = Some(Box::new(Expected {
                summary: Some(summary),
                skip: None,
            }));
            load(client, id, &checkpoint).await?
        }
        Action::SetSkip(skip) => {
            checkpoint.expected = Some(Box::new(Expected {
                summary: None,
                skip: Some(skip),
            }));
            client.set_meeting_skip_observations(id, skip).await.map_err(|_| Failure {
                message: "Observation setting not confirmed; the server outcome may be uncertain. Load the latest saved meeting before trying again.", expected: None,
            })?
        }
        Action::LoadLatest => {
            settle_generation(client, id, &mut checkpoint).await?;
            load(client, id, &checkpoint).await?
        }
        Action::CancelGeneration => {
            cancel_generation(client, id, &mut checkpoint).await?;
            load(client, id, &checkpoint).await?
        }
    };
    validate(id, &checkpoint, &meeting)?;
    Ok(meeting)
}

async fn settle_generation(
    client: &ServerClient,
    id: &str,
    checkpoint: &mut Checkpoint,
) -> Result<(), Failure> {
    let Some(request) = &checkpoint.generation else {
        return Ok(());
    };
    let receipt = client.summary_generation(id, &checkpoint.token).await
        .ok().flatten().ok_or_else(|| Failure {
            message: "Generation status could not be confirmed. Existing content is retained; retry loading will not generate again.",
            expected: checkpoint.expected.clone(),
        })?;
    if receipt.request != *request
        || request.meeting_id != id
        || request.request_id != checkpoint.token
    {
        return Err(Failure {
            message: "The server returned a mismatched generation receipt. Existing content and request are retained.",
            expected: checkpoint.expected.clone(),
        });
    }
    if matches!(receipt.state, bruhs_models::SummaryGenerationState::Pending) {
        return Err(Failure {
            message: "This generation is still unresolved on the server. Retry loading later; another generation will not be sent.",
            expected: checkpoint.expected.clone(),
        });
    }
    // A terminal receipt fences this request. Explicit recovery adopts the latest
    // validated meeting, which may legitimately contain a newer summary.
    checkpoint.expected = None;
    Ok(())
}

async fn cancel_generation(
    client: &ServerClient,
    id: &str,
    checkpoint: &mut Checkpoint,
) -> Result<(), Failure> {
    let Some(request) = checkpoint.generation.clone() else {
        return Ok(());
    };
    let receipt = client.cancel_summary_generation(&request).await
        .ok().flatten().ok_or_else(|| Failure {
            message: "Cancellation could not be confirmed. Existing content and the saved request are retained; no generation will run.",
            expected: checkpoint.expected.clone(),
        })?;
    if receipt.request != request
        || request.meeting_id != id
        || request.request_id != checkpoint.token
    {
        return Err(Failure {
            message: "The server returned a mismatched generation receipt. Existing content and request are retained.",
            expected: checkpoint.expected.clone(),
        });
    }
    if matches!(receipt.state, bruhs_models::SummaryGenerationState::Pending) {
        return Err(Failure {
            message: "The server did not cancel this generation; it remains unresolved. Retry later; another generation will not be sent.",
            expected: checkpoint.expected.clone(),
        });
    }
    // Cancelled or already terminal: the request is fenced, so adopt the latest meeting.
    checkpoint.expected = None;
    Ok(())
}

async fn load(client: &ServerClient, id: &str, checkpoint: &Checkpoint) -> Outcome {
    client.get_meeting(id).await.ok().flatten().ok_or_else(|| Failure {
        message: "Could not load the saved meeting. Existing content is retained; retry loading will not generate again.",
        expected: checkpoint.expected.clone(),
    })
}

fn validate(id: &str, checkpoint: &Checkpoint, meeting: &Meeting) -> Result<(), Failure> {
    if meeting.id != id
        || meeting.updated_at < checkpoint.minimum_version
        || meeting_cache::validate_meeting(meeting).is_err()
    {
        return Err(Failure { message: "The server returned a foreign, invalid or older meeting. Existing content is retained; load the latest saved meeting.", expected: None });
    }
    if checkpoint.expected.as_ref().is_some_and(|expected| {
        expected
            .summary
            .as_ref()
            .is_some_and(|summary| meeting.summary.as_ref() != Some(summary))
            || expected
                .skip
                .is_some_and(|skip| meeting.skip_observations != skip)
    }) {
        return Err(Failure { message: "The latest meeting differs from this request. Existing content is retained; load the latest meeting to review it.", expected: None });
    }
    Ok(())
}

//! Generate first; atomically commit summary and observations against the input snapshot.
use crate::error::AppError;
use axum::{
    extract::{Path, State},
    Json,
};
use bruhs_core::{
    meetings::GenerationOutcome,
    summarize::{self, DEFAULT_SUMMARIZE_MODEL},
    summary_generation::{self, SummaryGenerationError},
    AppState,
};
use bruhs_models::ids::MeetingId;
use bruhs_models::{MeetingEvent, MeetingSummary};
use std::sync::Arc;

/// The shared request body (unknown fields rejected).
pub use bruhs_models::api::SummarizeRequest as SummarizeBody;

pub async fn post_summarize(
    State(state): State<Arc<AppState>>,
    Path(meeting_id): Path<MeetingId>,
    body: Option<Json<SummarizeBody>>,
) -> Result<Json<MeetingSummary>, AppError> {
    let id = meeting_id.as_str();
    let body = body.map(|Json(body)| body).unwrap_or_default();
    // The user's model for summaries, or the built-in one. Summaries run on
    // the Claude CLI, so another provider's model is refused, not swapped.
    let model = match &body.model {
        Some(model) if model.provider() != bruhs_models::agent::AgentProvider::Claude => {
            return Err(AppError::BadRequest(
                "Meeting summaries need a Claude model.".into(),
            ));
        }
        Some(model) => model.as_str().to_string(),
        None => DEFAULT_SUMMARIZE_MODEL.to_string(),
    };
    // Read a known receipt before fetching transcript data or invoking capture/model services.
    if let Some(request_id) = &body.request_id {
        let request = generation_request(id, &body);
        if !request.valid() {
            return Err(AppError::BadRequest(
                "Invalid generation request identity.".into(),
            ));
        }
        if let Some(receipt) = state.meetings().summary_generation(id, request_id).await? {
            return Ok(Json(summary_generation::receipt_result(receipt, &request)?));
        }
    }
    let meeting = state
        .meetings()
        .get_meeting(id)
        .await?
        .ok_or_else(|| AppError::NotFound("Meeting was not found.".into()))?;
    // The strict, durable capture snapshot the transcript route serves. No
    // synchronous JSONL read or silent malformed-line skipping.
    let segments = state.refreshed_transcript(&meeting_id).await?.segments;
    let summary = generate_and_commit(
        state.meetings().as_ref(),
        meeting,
        segments,
        body,
        |segments, speakers, extract| async move {
            summarize::summarize_segments(&segments, &speakers, &model, extract).await
        },
    )
    .await?;
    state.emit_meeting_event(MeetingEvent::PersonObservationsAppended {
        meeting_id: id.to_string(),
    });
    // This optional convenience is independent of the already committed summary.
    if let Err(error) = state.create_thread_for_meeting(&meeting_id).await {
        tracing::warn!("Meeting chat creation failed after summary commit: {error}");
    }
    Ok(Json(summary))
}

/// [`summary_generation::generate_and_commit`] for `body`'s request identity.
async fn generate_and_commit<F, Fut>(
    store: &dyn bruhs_core::MeetingStore,
    meeting: bruhs_models::Meeting,
    segments: Vec<bruhs_models::TranscriptSegment>,
    body: SummarizeBody,
    generate: F,
) -> Result<MeetingSummary, AppError>
where
    F: FnOnce(Vec<bruhs_models::TranscriptSegment>, Vec<bruhs_models::Speaker>, bool) -> Fut,
    Fut: std::future::Future<
        Output = Result<
            (MeetingSummary, Vec<summarize::parser::PersonObservationRaw>),
            summarize::SummarizeError,
        >,
    >,
{
    let request = generation_request(&meeting.id, &body);
    Ok(
        summary_generation::generate_and_commit(store, meeting, segments, request, generate)
            .await?,
    )
}

fn generation_request(
    meeting: &str,
    body: &SummarizeBody,
) -> bruhs_models::SummaryGenerationRequest {
    let mut request = bruhs_models::SummaryGenerationRequest::new(
        meeting.into(),
        body.extract_observations,
        body.replace_observations,
    );
    if let Some(id) = &body.request_id {
        request.request_id = id.clone();
    }
    request
}

impl From<SummaryGenerationError> for AppError {
    fn from(error: SummaryGenerationError) -> Self {
        use SummaryGenerationError as E;
        match error {
            E::IdentityReused | E::Unresolved | E::Ended => AppError::Conflict(error.to_string()),
            E::InvalidTranscript(_) | E::IncompleteTranscript | E::InvalidObservations(_) => {
                AppError::BadRequest(error.to_string())
            }
            E::Generation(ref cause) => {
                // 503 carries no source; keep the provider's reason in the log.
                tracing::warn!("summary generation failed: {cause}");
                AppError::Unavailable(error.to_string())
            }
            E::Store(store) => store.into(),
            _ => AppError::internal(error),
        }
    }
}

pub async fn get_generation(
    State(state): State<Arc<AppState>>,
    Path((id, request_id)): Path<(String, String)>,
) -> Result<Json<bruhs_models::SummaryGenerationReceipt>, AppError> {
    state
        .meetings()
        .summary_generation(&id, &request_id)
        .await?
        .map(Json)
        .ok_or_else(|| AppError::NotFound("Generation request was not found.".into()))
}

/// Exact options of the request being fenced. Supplying them lets a client cancel
/// an identity the server has not seen yet, so a delayed arrival cannot run.
pub async fn cancel_generation(
    State(state): State<Arc<AppState>>,
    Path((id, request_id)): Path<(String, String)>,
    body: Option<Json<bruhs_models::api::CancelSummaryGenerationBody>>,
) -> Result<Json<bruhs_models::SummaryGenerationReceipt>, AppError> {
    // Cancellation fences the result commit; it does not promise to stop an already running model process.
    if let Some(Json(body)) = body {
        let request = bruhs_models::SummaryGenerationRequest {
            request_id,
            meeting_id: id,
            extract_observations: body.extract_observations,
            replace_observations: body.replace_observations,
        };
        if !request.valid() || (request.replace_observations && !request.extract_observations) {
            return Err(AppError::BadRequest(
                "Invalid generation request identity or options.".into(),
            ));
        }
        return Ok(Json(
            state.meetings().fence_summary_generation(request).await?,
        ));
    }
    if state
        .meetings()
        .summary_generation(&id, &request_id)
        .await?
        .is_none()
    {
        return Err(AppError::NotFound(
            "Generation request was not found.".into(),
        ));
    }
    Ok(Json(
        state
            .meetings()
            .finish_summary_generation(&id, &request_id, GenerationOutcome::Cancelled)
            .await?,
    ))
}

#[cfg(test)]
mod tests;

pub async fn get_pending_generation(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<Option<bruhs_models::SummaryGenerationReceipt>>, AppError> {
    if state.meetings().get_meeting(&id).await?.is_none() {
        return Err(AppError::NotFound("Meeting was not found.".into()));
    }
    Ok(Json(
        state.meetings().pending_summary_generation(&id).await?,
    ))
}

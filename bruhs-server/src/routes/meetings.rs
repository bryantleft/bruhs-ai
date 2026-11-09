use axum::{
    extract::{Path, State},
    http::{HeaderMap, HeaderValue, StatusCode},
    Json,
};
use bruhs_core::meetings::MeetingPatch;
use bruhs_core::{AppState, CreateThreadError, RefreshedTranscript, TranscriptError};
use bruhs_models::api::Thread;
use bruhs_models::ids::{MeetingId, PersonId, SpeakerId, ThreadId};
use bruhs_models::{Meeting, MeetingStatus, TranscriptSegment};
use serde::Deserialize;
use std::sync::Arc;

use crate::error::AppError;

#[derive(Deserialize)]
pub struct CreateMeetingRequest {
    pub title: String,
    pub started_at_ms: f64,
    #[serde(default)]
    pub calendar_event_id: Option<String>,
}

#[derive(Deserialize)]
pub struct PatchMeetingRequest {
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub status: Option<MeetingStatus>,
    #[serde(default)]
    pub ended_at_ms: Option<f64>,
}

#[derive(Deserialize)]
pub struct StopMeetingRequest {
    pub ended_at_ms: f64,
}

/// GET /api/meetings
pub async fn list_meetings(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Vec<Meeting>>, AppError> {
    Ok(Json(state.meetings().list_meetings().await?))
}

/// POST /api/meetings
pub async fn create_meeting(
    State(state): State<Arc<AppState>>,
    Json(body): Json<CreateMeetingRequest>,
) -> Result<Json<Meeting>, AppError> {
    let title = body.title.trim();
    if title.is_empty() {
        return Err(AppError::BadRequest("title must not be empty".into()));
    }
    let mut m = Meeting::new(title, body.started_at_ms);
    m.calendar_event_id = body.calendar_event_id;
    Ok(Json(state.meetings().create_meeting(m).await?))
}

/// GET /api/meetings/:id
pub async fn get_meeting(
    State(state): State<Arc<AppState>>,
    Path(id): Path<MeetingId>,
) -> Result<Json<Meeting>, AppError> {
    state
        .meetings()
        .get_meeting(id.as_str())
        .await?
        .map(Json)
        .ok_or_else(|| meeting_not_found(&id))
}

fn meeting_not_found(id: &MeetingId) -> AppError {
    AppError::NotFound(format!("meeting not found: {id}"))
}

/// PATCH /api/meetings/:id
pub async fn patch_meeting(
    State(state): State<Arc<AppState>>,
    Path(id): Path<MeetingId>,
    Json(body): Json<PatchMeetingRequest>,
) -> Result<Json<Meeting>, AppError> {
    state
        .meetings()
        .patch_meeting(
            &id,
            MeetingPatch {
                title: body.title,
                status: body.status,
                ended_at_ms: body.ended_at_ms,
            },
        )
        .await?
        .map(Json)
        .ok_or_else(|| meeting_not_found(&id))
}

/// POST /api/meetings/:id/stop
pub async fn stop_meeting(
    State(state): State<Arc<AppState>>,
    Path(id): Path<MeetingId>,
    Json(body): Json<StopMeetingRequest>,
) -> Result<Json<Meeting>, AppError> {
    state
        .meetings()
        .patch_meeting(
            &id,
            MeetingPatch {
                title: None,
                status: Some(MeetingStatus::Ended),
                ended_at_ms: Some(body.ended_at_ms),
            },
        )
        .await?
        .map(Json)
        .ok_or_else(|| meeting_not_found(&id))
}

#[derive(Debug, Deserialize)]
pub struct PatchSpeakerBody {
    pub label: String,
}

/// PATCH /api/meetings/:id/speakers/:speaker_id — 404 for an unknown
/// meeting or speaker.
pub async fn patch_speaker(
    State(state): State<Arc<AppState>>,
    Path((meeting_id, speaker_id)): Path<(MeetingId, SpeakerId)>,
    Json(body): Json<PatchSpeakerBody>,
) -> Result<Json<Meeting>, AppError> {
    Ok(Json(
        state
            .meetings()
            .patch_speaker_label(&meeting_id, &speaker_id, body.label)
            .await?,
    ))
}

/// GET /api/meetings/:id/segments
///
/// Cache validated capture snapshots before acknowledging them. Unavailable
/// capture is not evidence of an empty meeting; a retained copy remains usable.
pub async fn list_segments(
    State(state): State<Arc<AppState>>,
    Path(meeting_id): Path<MeetingId>,
) -> Result<(HeaderMap, Json<Vec<TranscriptSegment>>), AppError> {
    let RefreshedTranscript {
        segments,
        source,
        refresh,
    } = state.refreshed_transcript(&meeting_id).await?;
    let mut headers = HeaderMap::new();
    headers.insert(
        "x-bruhs-transcript-source",
        HeaderValue::from_static(source.as_str()),
    );
    headers.insert(
        "x-bruhs-transcript-refresh",
        HeaderValue::from_static(refresh.as_str()),
    );
    Ok((headers, Json(segments)))
}

/// DELETE /api/meetings/:id/segments/:segment_id
///
/// Removes one transcript chunk. Later capture replays of that id are ignored.
pub async fn remove_segment(
    State(state): State<Arc<AppState>>,
    Path((meeting_id, segment_id)): Path<(MeetingId, String)>,
) -> Result<StatusCode, AppError> {
    state
        .meetings()
        .remove_segment(meeting_id.as_str(), &segment_id)
        .await?;
    state.emit_meeting_event(bruhs_models::MeetingEvent::SegmentRemoved {
        meeting_id: meeting_id.as_str().to_owned(),
        segment_id,
    });
    Ok(StatusCode::NO_CONTENT)
}

/// GET /api/meetings/:id/segment-removals
pub async fn list_segment_removals(
    State(state): State<Arc<AppState>>,
    Path(meeting_id): Path<MeetingId>,
) -> Result<Json<Vec<String>>, AppError> {
    Ok(Json(
        state
            .meetings()
            .removed_segment_ids(meeting_id.as_str())
            .await?,
    ))
}

impl From<TranscriptError> for AppError {
    fn from(error: TranscriptError) -> Self {
        match error {
            TranscriptError::MeetingNotFound => AppError::NotFound(error.to_string()),
            TranscriptError::Unavailable(_) => AppError::Unavailable(error.to_string()),
            TranscriptError::Store(store) => store.into(),
            _ => AppError::internal(error),
        }
    }
}

/// POST /api/meetings/:id/chat
///
/// Idempotent: if the meeting already has a `thread_id` that resolves to a
/// live thread, returns that thread. Otherwise creates a new thread via
/// `create_thread_for_meeting` and links it to the meeting.
pub async fn create_or_get_chat(
    State(state): State<Arc<AppState>>,
    Path(meeting_id): Path<MeetingId>,
) -> Result<Json<Thread>, AppError> {
    let meeting = state
        .meetings()
        .get_meeting(meeting_id.as_str())
        .await?
        .ok_or_else(|| meeting_not_found(&meeting_id))?;

    let linked = meeting
        .thread_id
        .as_deref()
        .and_then(|id| ThreadId::try_from(id).ok());
    if let Some(thread_id) = linked {
        if let Some(thread) = state.thread(&thread_id).await {
            return Ok(Json(thread));
        }
        // thread_id points at a stale/deleted thread — fall through and create a fresh one.
    }

    Ok(Json(state.create_thread_for_meeting(&meeting_id).await?))
}

impl From<CreateThreadError> for AppError {
    fn from(error: CreateThreadError) -> Self {
        match error {
            CreateThreadError::MeetingStore(store) => store.into(),
            CreateThreadError::MeetingNotFound(_) => AppError::NotFound(error.to_string()),
            _ => AppError::internal(error),
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct PatchMeetingSkipObservationsBody {
    pub skip: bool,
}

/// PATCH /api/meetings/:id/skip_observations
pub async fn patch_meeting_skip_observations(
    State(state): State<Arc<AppState>>,
    Path(id): Path<MeetingId>,
    Json(body): Json<PatchMeetingSkipObservationsBody>,
) -> Result<Json<Meeting>, AppError> {
    Ok(Json(
        state
            .meetings()
            .patch_meeting_skip_observations(&id, body.skip)
            .await?,
    ))
}

/// DELETE /api/meetings/:id
pub async fn delete_meeting(
    State(state): State<Arc<AppState>>,
    Path(id): Path<MeetingId>,
) -> Result<StatusCode, AppError> {
    if state.meetings().delete_meeting(&id).await? {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(meeting_not_found(&id))
    }
}

/// POST /api/meetings/:meeting_id/speakers/:speaker_id/link/:person_id
///
/// Phase 3d: manually link a speaker entry to an existing PersonProfile.
/// Updates the Speaker entry's label to the person's display_name and sets
/// person_id. Emits `PeopleUpdated` so the web refetches people + meeting.
pub async fn link_speaker(
    State(state): State<Arc<AppState>>,
    Path((meeting_id, speaker_id, person_id)): Path<(MeetingId, SpeakerId, PersonId)>,
) -> Result<Json<Meeting>, AppError> {
    let updated = state
        .meetings()
        .link_speaker_to_person(&meeting_id, &speaker_id, &person_id)
        .await?;

    // Emit PeopleUpdated so the web refetches both the meeting and the
    // people list — the linked speaker now appears under a named profile.
    state.emit_meeting_event(bruhs_models::MeetingEvent::PeopleUpdated);

    Ok(Json(updated))
}

#[cfg(test)]
mod tests {
    use super::*;
    use bruhs_core::AppState;
    use std::sync::Arc;

    #[tokio::test]
    async fn create_then_list_returns_one() {
        let state = Arc::new(AppState::new());
        let _: Json<Meeting> = create_meeting(
            State(state.clone()),
            Json(CreateMeetingRequest {
                title: "Sync".into(),
                started_at_ms: 1.0,
                calendar_event_id: None,
            }),
        )
        .await
        .unwrap();
        let listed = list_meetings(State(state)).await.unwrap();
        assert_eq!(listed.0.len(), 1);
        assert_eq!(listed.0[0].title, "Sync");
    }

    #[tokio::test]
    async fn create_rejects_empty_title() {
        let state = Arc::new(AppState::new());
        let res: Result<Json<Meeting>, _> = create_meeting(
            State(state),
            Json(CreateMeetingRequest {
                title: "   ".into(),
                started_at_ms: 1.0,
                calendar_event_id: None,
            }),
        )
        .await;
        assert!(res.is_err());
    }

    #[tokio::test]
    async fn stop_unknown_meeting_404s() {
        let state = Arc::new(AppState::new());
        let res = stop_meeting(
            State(state),
            Path("missing".parse().unwrap()),
            Json(StopMeetingRequest { ended_at_ms: 5.0 }),
        )
        .await;
        assert!(matches!(res, Err(AppError::NotFound(_))));
    }

    // ── Phase 3d: link_speaker ────────────────────────────────────────────────

    #[tokio::test]
    async fn link_speaker_happy_path() {
        use bruhs_models::PersonProfile;

        let state = Arc::new(AppState::new());

        // Create meeting.
        let m = create_meeting(
            State(state.clone()),
            Json(CreateMeetingRequest {
                title: "Sync".into(),
                started_at_ms: 1.0,
                calendar_event_id: None,
            }),
        )
        .await
        .unwrap();
        let mid = m.0.id.clone();

        // Create person.
        let alice = PersonProfile::new("Alice");
        let pid = alice.id.clone();
        state.meetings().upsert_person(alice).await.unwrap();

        // Link "them_1" → Alice.
        let result = link_speaker(
            State(state.clone()),
            Path((
                mid.parse().unwrap(),
                "them_1".parse().unwrap(),
                pid.parse().unwrap(),
            )),
        )
        .await
        .unwrap();

        let sp = result.0.speakers.iter().find(|s| s.id == "them_1").unwrap();
        assert_eq!(sp.label, "Alice");
        assert_eq!(sp.person_id.as_deref(), Some(pid.as_str()));
    }

    #[tokio::test]
    async fn link_speaker_missing_meeting_is_not_found() {
        use bruhs_models::PersonProfile;

        let state = Arc::new(AppState::new());
        let alice = PersonProfile::new("Alice");
        let pid = alice.id.clone();
        state.meetings().upsert_person(alice).await.unwrap();

        let result = link_speaker(
            State(state),
            Path((
                "ghost-meeting".parse().unwrap(),
                "them_1".parse().unwrap(),
                pid.parse().unwrap(),
            )),
        )
        .await;
        assert!(matches!(result, Err(AppError::NotFound(_))));
    }

    #[tokio::test]
    async fn link_speaker_missing_person_is_not_found() {
        let state = Arc::new(AppState::new());

        let m = create_meeting(
            State(state.clone()),
            Json(CreateMeetingRequest {
                title: "Sync".into(),
                started_at_ms: 1.0,
                calendar_event_id: None,
            }),
        )
        .await
        .unwrap();
        let mid = m.0.id.clone();

        let result = link_speaker(
            State(state),
            Path((
                mid.parse().unwrap(),
                "them_1".parse().unwrap(),
                "ghost-person".parse().unwrap(),
            )),
        )
        .await;
        assert!(matches!(result, Err(AppError::NotFound(_))));
    }

    /// An unknown meeting or speaker is a 404 (it used to be a 400).
    #[tokio::test]
    async fn patch_speaker_unknown_meeting_or_speaker_is_not_found() {
        let state = Arc::new(AppState::new());
        let Json(meeting) = create_meeting(
            State(state.clone()),
            Json(CreateMeetingRequest {
                title: "Sync".into(),
                started_at_ms: 1.0,
                calendar_event_id: None,
            }),
        )
        .await
        .unwrap();
        for (meeting_id, speaker_id) in [("ghost", "them_1"), (meeting.id.as_str(), "ghost")] {
            let result = patch_speaker(
                State(state.clone()),
                Path((meeting_id.parse().unwrap(), speaker_id.parse().unwrap())),
                Json(PatchSpeakerBody {
                    label: "Alice".into(),
                }),
            )
            .await;
            assert!(
                matches!(result, Err(AppError::NotFound(_))),
                "{meeting_id}/{speaker_id}"
            );
        }
    }

    /// A store refusing the chat link as invalid input is the client's
    /// error (400), not a 500.
    #[test]
    fn chat_creation_errors_keep_their_status() {
        use bruhs_core::MeetingStoreError;
        let invalid = AppError::from(CreateThreadError::MeetingStore(
            MeetingStoreError::InvalidInput("failed meeting".into()),
        ));
        assert_eq!(invalid.status(), StatusCode::BAD_REQUEST);
        let missing = AppError::from(CreateThreadError::MeetingNotFound("m-1".parse().unwrap()));
        assert_eq!(missing.status(), StatusCode::NOT_FOUND);
    }

    /// A path id that is not a valid id is a 400 before the store is
    /// touched (`%2F` decodes to `/`, `%3F` to `?`, `%25` to `%`).
    #[tokio::test]
    async fn malformed_path_ids_are_bad_requests() {
        let state = Arc::new(AppState::new());
        let app = axum::Router::new()
            .route("/api/meetings/{id}", axum::routing::delete(delete_meeting))
            .with_state(state);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}/api/meetings", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let client = reqwest::Client::new();
        for id in ["a%2Fb", "a%3Fb", "a%25b"] {
            let status = client
                .delete(format!("{base}/{id}"))
                .send()
                .await
                .unwrap()
                .status();
            assert_eq!(status, reqwest::StatusCode::BAD_REQUEST, "{id}");
        }
        let status = client
            .delete(format!("{base}/missing"))
            .send()
            .await
            .unwrap()
            .status();
        assert_eq!(status, reqwest::StatusCode::NOT_FOUND);
        server.abort();
    }
}

use axum::{
    body::Bytes,
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
};
use bruhs_core::meetings::PersonInsert;
use bruhs_core::AppState;
use bruhs_models::ids::PersonId;
use bruhs_models::{MeetingEvent, PersonProfile};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::error::AppError;

#[derive(Deserialize)]
pub struct PatchPersonRequest {
    #[serde(default)]
    pub display_name: Option<String>,
}

#[derive(Deserialize)]
pub struct PatchPersonTrackedBody {
    pub tracked: bool,
}

#[derive(Deserialize)]
pub struct RenamePersonBody {
    pub display_name: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResolveMergesRequest {
    pub ids: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VersionPageQuery {
    pub after: Option<String>,
}

/// GET /api/people/versions — bounded identity/version metadata, never voice or notes.
pub async fn person_versions(
    State(state): State<Arc<AppState>>,
    Query(query): Query<VersionPageQuery>,
) -> Result<Json<bruhs_models::PersonVersionPage>, AppError> {
    Ok(Json(
        state
            .meetings()
            .person_versions(query.after.as_deref())
            .await?,
    ))
}

/// POST /api/people/profiles — read-only batch lookup, not a contact mutation.
pub async fn get_profiles(
    State(state): State<Arc<AppState>>,
    Json(body): Json<ResolveMergesRequest>,
) -> Result<Json<Vec<PersonProfile>>, AppError> {
    Ok(Json(state.meetings().get_people(&body.ids).await?))
}

/// POST /api/people/resolve-merges — read-only, bounded reconciliation lookup.
pub async fn resolve_merges(
    State(state): State<Arc<AppState>>,
    Json(body): Json<ResolveMergesRequest>,
) -> Result<Json<Vec<bruhs_models::PersonMergeResolution>>, AppError> {
    Ok(Json(
        state.meetings().resolve_person_merges(&body.ids).await?,
    ))
}

#[derive(Deserialize, Serialize)]
pub struct CreatePersonRequest {
    pub id: String,
    pub display_name: String,
    pub created_at: i64,
    pub updated_at: i64,
}

/// GET /api/people
pub async fn list_people(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Vec<PersonProfile>>, AppError> {
    Ok(Json(state.meetings().list_people().await?))
}

/// POST /api/people — create a manual contact with a client-generated id.
/// 201 with the new contact, or 200 with the one already saved under that
/// id (a retried create). The check and the insert are one atomic store
/// operation, so two concurrent creates cannot both write.
pub async fn create_person(
    State(state): State<Arc<AppState>>,
    Json(body): Json<CreatePersonRequest>,
) -> Result<(StatusCode, Json<PersonProfile>), AppError> {
    let person = person_from_request(body)?;
    match state.meetings().insert_person_if_absent(person).await? {
        PersonInsert::Existing(existing) => Ok((StatusCode::OK, Json(existing))),
        PersonInsert::Created(saved) => {
            emit_person_changed(&state, &saved);
            Ok((StatusCode::CREATED, Json(saved)))
        }
    }
}

fn person_from_request(body: CreatePersonRequest) -> Result<PersonProfile, AppError> {
    let name = body.display_name.trim();
    if body.id.trim().is_empty() || name.is_empty() {
        return Err(AppError::BadRequest(
            "id and display_name must not be empty".into(),
        ));
    }
    let id = PersonId::try_from(body.id).map_err(|e| AppError::BadRequest(format!("id: {e}")))?;
    let mut person = PersonProfile::new(name);
    person.id = id.into_inner();
    person.created_at = body.created_at;
    person.updated_at = body.updated_at;
    Ok(person)
}

fn emit_person_changed(state: &AppState, person: &PersonProfile) {
    state.emit_meeting_event(MeetingEvent::PersonChanged {
        person_id: person.id.clone(),
        updated_at: person.updated_at,
    });
}

/// GET /api/people/:id — indexed lookup for narrow live-update reconciliation.
pub async fn get_person(
    State(state): State<Arc<AppState>>,
    Path(id): Path<PersonId>,
) -> Result<Json<PersonProfile>, AppError> {
    state
        .meetings()
        .get_person(id.as_str())
        .await?
        .map(Json)
        .ok_or_else(|| person_not_found(&id))
}

fn person_not_found(id: &PersonId) -> AppError {
    AppError::NotFound(format!("person not found: {id}"))
}

/// PATCH /api/people/:id/tracked
pub async fn patch_person_tracked(
    State(state): State<Arc<AppState>>,
    Path(id): Path<PersonId>,
    Json(body): Json<PatchPersonTrackedBody>,
) -> Result<Json<PersonProfile>, AppError> {
    let person = state
        .meetings()
        .patch_person_tracked(&id, body.tracked)
        .await?;
    emit_person_changed(&state, &person);
    Ok(Json(person))
}

/// PATCH /api/people/:id
pub async fn patch_person(
    State(state): State<Arc<AppState>>,
    Path(id): Path<PersonId>,
    Json(body): Json<PatchPersonRequest>,
) -> Result<Json<PersonProfile>, AppError> {
    if let Some(name) = body.display_name {
        return rename_person(
            State(state),
            Path(id),
            Json(RenamePersonBody { display_name: name }),
        )
        .await;
    }
    get_person(State(state), Path(id)).await
}

/// PATCH /api/people/:id/rename — rename a profile's display_name.
/// Body: `{ "display_name": "New Name" }`.
/// Emits a versioned single-contact invalidation on success.
pub async fn rename_person(
    State(state): State<Arc<AppState>>,
    Path(id): Path<PersonId>,
    Json(body): Json<RenamePersonBody>,
) -> Result<Json<PersonProfile>, AppError> {
    let person = state
        .meetings()
        .rename_person(&id, body.display_name)
        .await?;
    emit_person_changed(&state, &person);
    Ok(Json(person))
}

/// POST /api/people/:id/enroll — accept a WAV body, forward to the capture
/// daemon to extract a voice embedding, upsert it on the PersonProfile, and
/// emit a versioned single-contact invalidation. The id is a validated
/// [`PersonId`] (pushed to the daemon as one path segment), and an unknown
/// person is a 404 before the daemon is called.
pub async fn enroll_voice(
    State(state): State<Arc<AppState>>,
    Path(id): Path<PersonId>,
    body: Bytes,
) -> Result<Json<PersonProfile>, AppError> {
    if state.meetings().get_person(id.as_str()).await?.is_none() {
        return Err(person_not_found(&id));
    }
    let embedding = bruhs_core::capture_client::enroll_voice(id.as_str(), body.to_vec())
        .await
        .map_err(|e| AppError::bad_gateway(format!("capture daemon enroll failed: {e}"), e))?;
    let updated = state
        .meetings()
        .upsert_voice_embedding(&id, embedding)
        .await?;
    emit_person_changed(&state, &updated);
    Ok(Json(updated))
}

/// POST /api/people/:source_id/merge_into/:target_id — absorb source into target.
/// Emits a merge-specific invalidation on success.
pub async fn merge_person_into(
    State(state): State<Arc<AppState>>,
    Path((source_id, target_id)): Path<(PersonId, PersonId)>,
) -> Result<Json<PersonProfile>, AppError> {
    let merged = state
        .meetings()
        .merge_person_into(&source_id, &target_id)
        .await?;
    state.emit_meeting_event(MeetingEvent::PersonMerged {
        source_id: source_id.into_inner(),
    });
    Ok(Json(merged))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn contact_changes_emit_only_the_committed_identity_and_version() {
        let state = Arc::new(AppState::new());
        let mut events = state.subscribe_meeting_events();
        let (_, Json(created)) = create_person(
            State(state.clone()),
            Json(CreatePersonRequest {
                id: "narrow-contact".into(),
                display_name: "Original".into(),
                created_at: 1,
                updated_at: 1,
            }),
        )
        .await
        .unwrap();
        assert_changed(events.try_recv().unwrap(), &created);
        let Json(renamed) = rename_person(
            State(state.clone()),
            Path(created.id.parse().unwrap()),
            Json(RenamePersonBody {
                display_name: "Renamed".into(),
            }),
        )
        .await
        .unwrap();
        assert_changed(events.try_recv().unwrap(), &renamed);
        let Json(tracked) = patch_person_tracked(
            State(state.clone()),
            Path(created.id.parse().unwrap()),
            Json(PatchPersonTrackedBody { tracked: false }),
        )
        .await
        .unwrap();
        assert_changed(events.try_recv().unwrap(), &tracked);
        let Json(fetched) = get_person(State(state.clone()), Path(created.id.parse().unwrap()))
            .await
            .unwrap();
        assert_eq!(fetched, tracked);
        assert!(matches!(
            get_person(State(state.clone()), Path("missing".parse().unwrap()))
                .await
                .unwrap_err(),
            AppError::NotFound(_)
        ));
        assert!(matches!(
            rename_person(
                State(state.clone()),
                Path(created.id.parse().unwrap()),
                Json(RenamePersonBody {
                    display_name: " ".into()
                })
            )
            .await
            .unwrap_err(),
            AppError::BadRequest(_)
        ));
        assert!(matches!(
            events.try_recv(),
            Err(tokio::sync::broadcast::error::TryRecvError::Empty)
        ));
    }

    fn assert_changed(event: MeetingEvent, person: &PersonProfile) {
        assert_eq!(
            serde_json::to_value(event).unwrap(),
            serde_json::json!({
                "kind": "person_changed", "person_id": person.id, "updated_at": person.updated_at,
            })
        );
    }

    #[tokio::test]
    async fn contact_merge_emits_source_identity_without_invalidating_the_directory() {
        let state = Arc::new(AppState::new());
        let source = state
            .meetings()
            .upsert_person(PersonProfile::new("Source"))
            .await
            .unwrap();
        let target = state
            .meetings()
            .upsert_person(PersonProfile::new("Target"))
            .await
            .unwrap();
        let mut events = state.subscribe_meeting_events();
        let Json(merged) = merge_person_into(
            State(state.clone()),
            Path((source.id.parse().unwrap(), target.id.parse().unwrap())),
        )
        .await
        .unwrap();
        assert_eq!(
            serde_json::to_value(events.try_recv().unwrap()).unwrap(),
            serde_json::json!({"kind":"person_merged", "source_id":source.id})
        );
        assert!(matches!(
            get_person(State(state.clone()), Path(source.id.parse().unwrap()))
                .await
                .unwrap_err(),
            AppError::NotFound(_)
        ));
        assert_eq!(
            get_person(State(state.clone()), Path(target.id.parse().unwrap()))
                .await
                .unwrap()
                .0,
            merged
        );
        assert!(matches!(
            events.try_recv(),
            Err(tokio::sync::broadcast::error::TryRecvError::Empty)
        ));
    }

    async fn serve(state: Arc<AppState>) -> (String, tokio::task::JoinHandle<()>) {
        let app = axum::Router::new()
            .route("/api/people", axum::routing::post(create_person))
            .route("/api/people/{id}/enroll", axum::routing::post(enroll_voice))
            .with_state(state);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        (url, server)
    }

    /// A person id cannot carry URL structure to the capture daemon: an
    /// encoded `../capture/stop` used to reach the daemon's stop route.
    /// Unknown people are refused before the daemon is called at all.
    #[tokio::test]
    async fn enroll_rejects_path_injection_and_unknown_people_before_the_daemon() {
        let (url, server) = serve(Arc::new(AppState::new())).await;
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        for (id, status) in [
            ("..%2Fcapture%2Fstop", reqwest::StatusCode::BAD_REQUEST),
            ("a%3Fb", reqwest::StatusCode::BAD_REQUEST),
            ("nobody", reqwest::StatusCode::NOT_FOUND),
        ] {
            let response = client
                .post(format!("{url}/api/people/{id}/enroll"))
                .body(vec![0u8; 44])
                .send()
                .await
                .unwrap();
            assert_eq!(response.status(), status, "{id}");
        }
        server.abort();
    }

    /// Concurrent creates of one id write once: one 201, the rest 200 with
    /// the saved contact (check-then-insert used to race).
    #[tokio::test]
    async fn concurrent_creates_of_one_id_write_once() {
        let state = Arc::new(AppState::new());
        let mut events = state.subscribe_meeting_events();
        let (url, server) = serve(state.clone()).await;
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let creates = (0..8).map(|i| {
            let client = client.clone();
            let url = url.clone();
            async move {
                let response = client
                    .post(format!("{url}/api/people"))
                    .json(&serde_json::json!({
                        "id": "same-contact",
                        "display_name": format!("Name {i}"),
                        "created_at": 1,
                        "updated_at": 1,
                    }))
                    .send()
                    .await
                    .unwrap();
                let status = response.status();
                (status, response.json::<PersonProfile>().await.unwrap())
            }
        });
        let results = futures_util::future::join_all(creates).await;
        let created: Vec<_> = results
            .iter()
            .filter(|(status, _)| *status == reqwest::StatusCode::CREATED)
            .collect();
        assert_eq!(created.len(), 1, "{results:?}");
        assert!(results.iter().all(|(_, person)| person == &created[0].1));
        assert!(events.try_recv().is_ok());
        assert!(events.try_recv().is_err(), "one change event");

        let invalid = client
            .post(format!("{url}/api/people"))
            .json(&serde_json::json!({
                "id": "a/b", "display_name": "X", "created_at": 1, "updated_at": 1,
            }))
            .send()
            .await
            .unwrap();
        assert_eq!(invalid.status(), reqwest::StatusCode::BAD_REQUEST);
        server.abort();
    }

    #[test]
    fn person_from_request_should_trim_and_preserve_client_identity() {
        let person = person_from_request(CreatePersonRequest {
            id: "person-1".to_string(),
            display_name: "  Ada Lovelace  ".to_string(),
            created_at: 10,
            updated_at: 20,
        })
        .expect("valid contact");

        assert_eq!(person.id, "person-1");
        assert_eq!(person.display_name, "Ada Lovelace");
    }

    #[test]
    fn person_from_request_should_reject_blank_names() {
        let result = person_from_request(CreatePersonRequest {
            id: "person-1".to_string(),
            display_name: "  ".to_string(),
            created_at: 10,
            updated_at: 10,
        });

        assert!(matches!(result.unwrap_err(), AppError::BadRequest(_)));
    }
}

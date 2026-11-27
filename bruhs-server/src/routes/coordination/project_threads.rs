use super::*;
use axum::{
    extract::{DefaultBodyLimit, Query},
    routing::put,
};
use bruhs_models::{
    coordination::linked_thread::{
        CheckoutContext, LinkedGuestAccess, LinkedProjectReply, LinkedThreadAction, StartThread,
    },
    ids::ThreadId,
    logic::attachments::MAX_FILE_BYTES,
};

pub(super) fn routes() -> Router<Arc<RemoteState>> {
    Router::new()
        .route("/api/projects/thread-checkout", get(checkout))
        .route("/api/projects/thread-models", get(models))
        .route("/api/projects/threads", get(list))
        .route("/api/projects/threads/{id}/actions", post(act))
        .route(
            "/api/projects/threads/{id}/attachments",
            post(upload_attachment).layer(DefaultBodyLimit::max(
                usize::try_from(MAX_FILE_BYTES).unwrap_or(usize::MAX),
            )),
        )
        .route("/api/projects/threads/{id}/guest-link", post(guest_link))
        .route(
            "/api/projects/threads/{id}",
            put(start).get(read).post(reply),
        )
}

#[derive(Deserialize)]
struct UploadQuery {
    name: String,
}

async fn upload_attachment(
    State(state): State<Arc<RemoteState>>,
    headers: HeaderMap,
    Path(id): Path<ThreadId>,
    Query(query): Query<UploadQuery>,
    body: axum::body::Bytes,
) -> Result<impl IntoResponse, AppError> {
    let bearer = permit(&state, &headers).await?;
    let attachment = state
        .runtime
        .upload_project_thread_attachment(&bearer, &id, query.name, body.to_vec())
        .await?;
    Ok(Json(attachment))
}

async fn guest_link(
    State(state): State<Arc<RemoteState>>,
    headers: HeaderMap,
    Path(id): Path<ThreadId>,
) -> Result<impl IntoResponse, AppError> {
    let bearer = permit(&state, &headers).await?;
    let runtime = state.runtime.clone();
    let access = tokio::spawn(async move { runtime.share_project_thread(&bearer, &id).await })
        .await
        .map_err(AppError::internal)??;
    Ok(Json(LinkedGuestAccess {
        token: access.token,
    }))
}

async fn permit(state: &RemoteState, headers: &HeaderMap) -> Result<String, AppError> {
    let bearer = token(headers)?;
    if !state
        .runtime
        .authorize_grant(&bearer)
        .await?
        .start_project_threads
    {
        return Err(CoordinationError::Unauthorized.into());
    }
    Ok(bearer)
}

#[derive(Deserialize)]
struct CheckoutQuery {
    path: String,
}

async fn resolve_checkout(
    state: &RemoteState,
    path: String,
) -> Result<(CheckoutContext, String), AppError> {
    let file = state
        .desktop_state
        .clone()
        .ok_or_else(|| AppError::Unavailable("The host has no desktop workspace.".into()))?;
    let saved = bruhs_core::linked_projects::task_projects(file)
        .await
        .map_err(|_| AppError::Unavailable("Could not read the host’s projects.".into()))?;
    let preferred = saved
        .projects
        .get(&path)
        .ok_or_else(|| AppError::NotFound("The host no longer shares this checkout.".into()))?;
    let model = preferred.clone().unwrap_or(saved.default_model);
    let folder = HomePath::resolve(&path).await?;
    if !tokio::fs::metadata(folder.as_path())
        .await
        .is_ok_and(|meta| meta.is_dir())
    {
        return Err(AppError::BadRequest("Choose a checkout directory.".into()));
    }
    let branch = bruhs_core::coordination::checkout_branch(folder.as_path()).await;
    let canonical = folder
        .as_path()
        .to_str()
        .ok_or_else(|| AppError::BadRequest("The checkout path is not valid text.".into()))?
        .to_owned();
    Ok((
        CheckoutContext {
            path,
            branch,
            model,
        },
        canonical,
    ))
}

async fn checkout(
    State(state): State<Arc<RemoteState>>,
    headers: HeaderMap,
    Query(query): Query<CheckoutQuery>,
) -> Result<impl IntoResponse, AppError> {
    permit(&state, &headers).await?;
    let (checkout, _) = resolve_checkout(&state, query.path).await?;
    permit(&state, &headers).await?;
    Ok(Json(checkout))
}

async fn models(
    State(state): State<Arc<RemoteState>>,
    headers: HeaderMap,
    Query(query): Query<CheckoutQuery>,
) -> Result<impl IntoResponse, AppError> {
    permit(&state, &headers).await?;
    let (_, canonical) = resolve_checkout(&state, query.path).await?;
    let models =
        bruhs_core::agent::writer_catalog_for_project(std::path::Path::new(&canonical)).await;
    permit(&state, &headers).await?;
    Ok(Json(models))
}

async fn start(
    State(state): State<Arc<RemoteState>>,
    headers: HeaderMap,
    Path(id): Path<ThreadId>,
    Json(start): Json<StartThread>,
) -> Result<impl IntoResponse, AppError> {
    let bearer = permit(&state, &headers).await?;
    if state
        .runtime
        .linked_start_replayed(&bearer, &id, &start)
        .await?
    {
        return Ok(Json(id));
    }
    let (current, canonical) = resolve_checkout(&state, start.checkout.path.clone()).await?;
    if start.checkout.branch != current.branch {
        return Err(AppError::Conflict(
            "The host’s branch changed. Refresh the checkout before sending.".into(),
        ));
    }
    require_model(&current, &canonical, &start.checkout.model).await?;
    // Let the durable write and launch finish if the client loses its connection.
    let runtime = state.runtime.clone();
    let receipt = tokio::spawn(async move {
        runtime
            .start_project_thread(bearer, id, canonical, start)
            .await
    })
    .await
    .map_err(AppError::internal)??;
    Ok(Json(receipt))
}

async fn read(
    State(state): State<Arc<RemoteState>>,
    headers: HeaderMap,
    Path(id): Path<ThreadId>,
) -> Result<impl IntoResponse, AppError> {
    Ok(Json(
        state.runtime.project_thread(&token(&headers)?, &id).await?,
    ))
}

async fn reply(
    State(state): State<Arc<RemoteState>>,
    headers: HeaderMap,
    Path(id): Path<ThreadId>,
    Json(reply): Json<LinkedProjectReply>,
) -> Result<impl IntoResponse, AppError> {
    let bearer = permit(&state, &headers).await?;
    let receipt = reply.request_id;
    if state
        .runtime
        .linked_reply_replayed(&bearer, &id, &reply)
        .await?
    {
        return Ok(Json(receipt));
    }
    let stored = state.runtime.project_thread(&bearer, &id).await?.checkout;
    let (current, _) = resolve_checkout(&state, stored.path).await?;
    if current.branch != stored.branch {
        return Err(AppError::Conflict(
            "The host’s branch changed. Return to the original branch to continue.".into(),
        ));
    }
    let runtime = state.runtime.clone();
    tokio::spawn(async move { runtime.reply_to_project_thread(&bearer, &id, reply).await })
        .await
        .map_err(AppError::internal)??;
    Ok(Json(receipt))
}

async fn list(
    State(state): State<Arc<RemoteState>>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, AppError> {
    Ok(Json(
        state.runtime.project_threads(&token(&headers)?).await?,
    ))
}

async fn act(
    State(state): State<Arc<RemoteState>>,
    headers: HeaderMap,
    Path(id): Path<ThreadId>,
    Json(action): Json<LinkedThreadAction>,
) -> Result<impl IntoResponse, AppError> {
    let bearer = permit(&state, &headers).await?;
    if let LinkedThreadAction::SetModel { model } = &action {
        let thread = state.runtime.project_thread(&bearer, &id).await?;
        let (checkout, canonical) = resolve_checkout(&state, thread.checkout.path).await?;
        if checkout.branch != thread.checkout.branch {
            return Err(AppError::Conflict(
                "The host’s branch changed. Return to the original branch to continue.".into(),
            ));
        }
        require_model(&checkout, &canonical, model).await?;
    }
    let runtime = state.runtime.clone();
    let receipt = id.clone();
    tokio::spawn(async move { runtime.act_on_project_thread(&bearer, &id, action).await })
        .await
        .map_err(AppError::internal)??;
    Ok(Json(receipt))
}

async fn require_model(
    checkout: &CheckoutContext,
    canonical: &str,
    model: &str,
) -> Result<(), AppError> {
    if model != checkout.model
        && !bruhs_core::agent::writer_catalog_for_project(std::path::Path::new(canonical))
            .await
            .models
            .iter()
            .any(|candidate| candidate.id == model)
    {
        return Err(AppError::Conflict(
            "That model is no longer available on the host.".into(),
        ));
    }
    Ok(())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use bruhs_models::coordination::{
        linked_thread::{LinkedThread, LinkedThreadSummary},
        TaskState,
    };
    use std::{os::unix::fs::PermissionsExt, time::Duration};

    #[tokio::test]
    async fn linked_thread_runs_on_checkout_once_and_requires_its_own_grant() {
        const FIXTURE: &str = "BRUHS_LINKED_THREAD_TEST_PROVIDER";
        if std::env::var_os(FIXTURE).is_none() {
            // Run a fixture CLI in a child process to avoid changing provider
            // settings for parallel tests or launching a real provider.
            let root = tempfile::tempdir().unwrap();
            let cli = root.path().join("opencode");
            std::fs::write(&cli, r#"#!/bin/sh
if [ "$1" = debug ]; then
  printf '{}\n'
else
  printf 'run\n' >> .fixture-runs
  if [ -e .hold-fixture ]; then sleep 20; fi
  printf '%s\n' '{"type":"text","sessionID":"fixture","part":{"text":"Ran inside the selected checkout"}}' '{"type":"step_finish","part":{"reason":"stop"}}'
fi
"#).unwrap();
            std::fs::set_permissions(&cli, std::fs::Permissions::from_mode(0o700)).unwrap();
            let test = format!(
                "{}::linked_thread_runs_on_checkout_once_and_requires_its_own_grant",
                module_path!().split_once("::").unwrap().1
            );
            let child = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", &test, "--nocapture"])
                .env(FIXTURE, "1")
                .env("BRUHS_OPENCODE_BIN", cli)
                .output()
                .unwrap();
            let output = String::from_utf8_lossy(&child.stdout);
            assert!(
                child.status.success() && output.contains("1 passed"),
                "{output}\n{}",
                String::from_utf8_lossy(&child.stderr)
            );
            return;
        }

        let root = tempfile::tempdir_in(std::env::home_dir().unwrap()).unwrap();
        let checkout = std::fs::canonicalize(root.path()).unwrap().join("checkout");
        std::fs::create_dir(&checkout).unwrap();
        assert!(std::process::Command::new("git")
            .args(["init", "-q", "-b", "studio"])
            .current_dir(&checkout)
            .status()
            .unwrap()
            .success());
        let path = checkout.to_str().unwrap();
        let saved = root.path().join("desktop.json");
        std::fs::write(
            &saved,
            serde_json::to_vec(&serde_json::json!({
                "current_model":"opencode/default",
                "project_list":{"entries":[{"path":path,"name":"Studio checkout"}]}
            }))
            .unwrap(),
        )
        .unwrap();
        let store = root.path().join("tasks.json");
        let app_state = || Arc::new(bruhs_core::AppState::new());
        let owner = CoordinationRuntime::open(store.clone(), app_state())
            .await
            .unwrap();
        let first = owner
            .issue_access_with_capabilities(
                AccessScope::Projects,
                60,
                ProjectCapabilities {
                    start_project_threads: true,
                    share_project_threads: true,
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        let sibling = owner
            .issue_access_with_capabilities(
                AccessScope::Projects,
                60,
                ProjectCapabilities {
                    start_project_threads: true,
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        let reader = owner.issue_access(AccessScope::Projects, 60).await.unwrap();
        let delivery = bruhs_core::delivery::DeliveryService::open(root.path().join("delivery"))
            .await
            .unwrap();
        let router = remote_router_with_state(
            owner.clone(),
            BoardLinks::new(delivery),
            Some(saved.clone()),
            None,
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let projects = format!("http://{}/api/projects", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        let client = reqwest::Client::new();
        let context = |token: &str, path: &str| {
            client
                .get(format!("{projects}/thread-checkout"))
                .bearer_auth(token)
                .query(&[("path", path)])
                .send()
        };
        let id = ThreadId::new_v4();
        let thread = format!("{projects}/threads/{id}");
        let send = |token: &str, body: &StartThread| {
            client.put(&thread).bearer_auth(token).json(body).send()
        };
        let read = |token: &str| client.get(&thread).bearer_auth(token).send();
        assert_eq!(
            context(&reader.token, path).await.unwrap().status(),
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            context(&first.token, root.path().to_str().unwrap())
                .await
                .unwrap()
                .status(),
            StatusCode::NOT_FOUND
        );
        let context = context(&first.token, path)
            .await
            .unwrap()
            .error_for_status()
            .unwrap()
            .json::<CheckoutContext>()
            .await
            .unwrap();
        assert_eq!(context.branch.as_deref(), Some("studio"));
        let mut request = StartThread {
            checkout: context,
            message: "Run once".into(),
            options: Default::default(),
            attachments: Vec::new(),
        };
        request.checkout.branch = Some("stale-branch".into());
        assert_eq!(
            send(&first.token, &request).await.unwrap().status(),
            StatusCode::CONFLICT
        );
        assert!(owner.snapshot().await.tasks.is_empty());
        request.checkout.branch = Some("studio".into());
        assert_eq!(
            send(&reader.token, &request).await.unwrap().status(),
            StatusCode::FORBIDDEN
        );
        let (a, b) = tokio::join!(send(&first.token, &request), send(&first.token, &request));
        for response in [a, b] {
            assert_eq!(
                response
                    .unwrap()
                    .error_for_status()
                    .unwrap()
                    .json::<ThreadId>()
                    .await
                    .unwrap(),
                id
            );
        }
        assert_eq!(
            read(&sibling.token).await.unwrap().status(),
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            send(&sibling.token, &request).await.unwrap().status(),
            StatusCode::NOT_FOUND
        );
        let list = |token: &str| {
            client
                .get(format!("{projects}/threads"))
                .bearer_auth(token)
                .send()
        };
        let own_threads = list(&first.token)
            .await
            .unwrap()
            .json::<Vec<LinkedThreadSummary>>()
            .await
            .unwrap();
        assert_eq!(own_threads.len(), 1);
        assert_eq!(own_threads[0].id, id);
        assert!(list(&sibling.token)
            .await
            .unwrap()
            .json::<Vec<LinkedThreadSummary>>()
            .await
            .unwrap()
            .is_empty());
        assert_eq!(
            list(&reader.token).await.unwrap().status(),
            StatusCode::FORBIDDEN
        );
        let action = |token: &str, action: LinkedThreadAction| {
            client
                .post(format!("{thread}/actions"))
                .bearer_auth(token)
                .json(&action)
                .send()
        };
        assert_eq!(
            action(&sibling.token, LinkedThreadAction::Stop)
                .await
                .unwrap()
                .status(),
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            action(
                &first.token,
                LinkedThreadAction::Approval {
                    request_id: "unknown".into(),
                    allow: true
                }
            )
            .await
            .unwrap()
            .status(),
            StatusCode::FORBIDDEN
        );
        request.message = "Different text".into();
        assert_eq!(
            send(&first.token, &request).await.unwrap().status(),
            StatusCode::CONFLICT
        );
        request.message = "Run once".into();
        let result = tokio::time::timeout(Duration::from_secs(20), async {
            loop {
                let current = read(&first.token)
                    .await
                    .unwrap()
                    .json::<LinkedThread>()
                    .await
                    .unwrap();
                if !current.state.active() && current.state != TaskState::Ready {
                    break current;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap();
        assert_eq!(result.state, TaskState::Completed, "{result:?}");
        assert_eq!(result.messages.len(), 2);
        assert_eq!(result.messages[1].text, "Ran inside the selected checkout");
        assert_eq!(
            std::fs::read_to_string(checkout.join(".fixture-runs")).unwrap(),
            "run\n"
        );
        let tasks = owner.snapshot().await.tasks;
        assert_eq!(tasks.len(), 1);
        assert_ne!(tasks[0].id, id);

        let share = |token: &str| {
            client
                .post(format!("{thread}/guest-link"))
                .bearer_auth(token)
                .send()
        };
        assert_eq!(
            share(&sibling.token).await.unwrap().status(),
            StatusCode::NOT_FOUND
        );
        let guest = share(&first.token)
            .await
            .unwrap()
            .error_for_status()
            .unwrap()
            .json::<LinkedGuestAccess>()
            .await
            .unwrap();
        let access = owner.authorize_grant(&guest.token).await.unwrap();
        assert_eq!(
            access.scope,
            AccessScope::Guest {
                task_id: tasks[0].id.clone()
            }
        );
        assert!(access.expires_at <= first.grant.expires_at);
        assert_eq!(
            list(&guest.token).await.unwrap().status(),
            StatusCode::FORBIDDEN
        );

        let original_settings = std::fs::read(&saved).unwrap();
        let mut settings: serde_json::Value = serde_json::from_slice(&original_settings).unwrap();
        let next_model = "opencode/fixture-next";
        settings["current_model"] = serde_json::json!(next_model);
        std::fs::write(&saved, serde_json::to_vec(&settings).unwrap()).unwrap();
        let set_model = || LinkedThreadAction::SetModel {
            model: next_model.into(),
        };
        assert_eq!(
            action(&sibling.token, set_model()).await.unwrap().status(),
            StatusCode::NOT_FOUND
        );
        action(&first.token, set_model())
            .await
            .unwrap()
            .error_for_status()
            .unwrap();
        let switched = owner.snapshot().await.tasks.pop().unwrap();
        assert_eq!(switched.model, next_model);
        assert!(switched.session_id.is_none());
        assert_eq!(
            action(
                &first.token,
                LinkedThreadAction::SetModelOption {
                    model: next_model.into(),
                    kind: bruhs_models::delivery::generation::OptionKind::FastMode,
                    value: "on".into(),
                }
            )
            .await
            .unwrap()
            .status(),
            StatusCode::BAD_REQUEST,
        );

        let png_bytes = {
            let mut bytes = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0xFF];
            // Exceed the outer router's JSON body limit (MESSAGE_CHAR_LIMIT * 4 = 480_000)
            // to prove the attachment route's 50 MiB limit applies.
            bytes.resize(512 * 1024, 0xAB);
            bytes
        };
        let upload = |token: &str, url: &str, name: &str, body: Vec<u8>| {
            client
                .post(format!("{url}/attachments"))
                .bearer_auth(token)
                .query(&[("name", name)])
                .body(body)
                .send()
        };
        assert_eq!(
            upload(&sibling.token, &thread, "shot.png", png_bytes.clone())
                .await
                .unwrap()
                .status(),
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            upload(&reader.token, &thread, "shot.png", png_bytes.clone())
                .await
                .unwrap()
                .status(),
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            upload(&first.token, &thread, "photo.heic", b"heic".to_vec())
                .await
                .unwrap()
                .status(),
            StatusCode::BAD_REQUEST
        );
        let uploaded_png = upload(&first.token, &thread, "shot.png", png_bytes.clone())
            .await
            .unwrap()
            .error_for_status()
            .unwrap()
            .json::<bruhs_models::coordination::linked_thread::LinkedAttachment>()
            .await
            .unwrap();
        assert_eq!(uploaded_png.name, "shot.png");
        assert_eq!(uploaded_png.bytes, png_bytes.len() as u64);
        assert!(!uploaded_png.id.contains('/') && !uploaded_png.id.contains('\\'));

        let reply = LinkedProjectReply {
            request_id: uuid::Uuid::new_v4(),
            message: "A follow-up".into(),
            attachments: vec![uploaded_png.clone()],
        };
        let send_reply = |token: &str| client.post(&thread).bearer_auth(token).json(&reply).send();
        assert_eq!(
            send_reply(&sibling.token).await.unwrap().status(),
            StatusCode::NOT_FOUND
        );
        let first_reply = tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let response = send_reply(&first.token).await.unwrap();
                if response.status() != StatusCode::CONFLICT {
                    break response;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap();
        for response in [first_reply, send_reply(&first.token).await.unwrap()] {
            assert_eq!(
                response
                    .error_for_status()
                    .unwrap()
                    .json::<uuid::Uuid>()
                    .await
                    .unwrap(),
                reply.request_id
            );
        }
        let result = tokio::time::timeout(Duration::from_secs(20), async {
            loop {
                let current = read(&first.token)
                    .await
                    .unwrap()
                    .json::<LinkedThread>()
                    .await
                    .unwrap();
                if !current.state.active() && current.state != TaskState::Ready {
                    break current;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap();
        assert_eq!(result.state, TaskState::Completed, "{result:?}");
        assert_eq!(result.checkout.model, next_model);
        assert_eq!(result.messages.len(), 4);
        assert_eq!(result.messages[2].text, "A follow-up\n\nAttached: shot.png");
        assert!(!result.messages[2].text.contains(&uploaded_png.id));
        if let Ok(root) = bruhs_core::uploads::media_output_directory() {
            let _ = std::fs::remove_file(root.join(&uploaded_png.id));
        }
        assert_eq!(
            std::fs::read_to_string(checkout.join(".fixture-runs")).unwrap(),
            "run\nrun\n"
        );
        std::fs::write(&saved, original_settings).unwrap();
        std::fs::write(checkout.join(".hold-fixture"), b"").unwrap();
        let stop_id = ThreadId::new_v4();
        let stop_url = format!("{projects}/threads/{stop_id}");
        let receipt = client
            .put(&stop_url)
            .bearer_auth(&first.token)
            .json(&request)
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap()
            .json::<ThreadId>()
            .await
            .unwrap();
        assert_eq!(receipt, stop_id);
        client
            .post(format!("{stop_url}/actions"))
            .bearer_auth(&first.token)
            .json(&LinkedThreadAction::Stop)
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap();
        assert_eq!(
            client
                .get(&stop_url)
                .bearer_auth(&first.token)
                .send()
                .await
                .unwrap()
                .json::<LinkedThread>()
                .await
                .unwrap()
                .state,
            TaskState::Cancelled
        );
        // Reopen the host store and verify that the receipt survives a restart.
        server.abort();
        let _ = server.await;
        drop(owner);
        let reopened = tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if let Ok(runtime) = CoordinationRuntime::open(store.clone(), app_state()).await {
                    break runtime;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap();
        assert!(reopened
            .linked_start_replayed(&first.token, &id, &request)
            .await
            .unwrap());
        assert_eq!(
            reopened
                .project_thread(&first.token, &id)
                .await
                .unwrap()
                .checkout
                .model,
            next_model
        );
        reopened.revoke_access(&first.grant.id).await.unwrap();
        assert!(matches!(
            reopened.authorize_grant(&guest.token).await,
            Err(CoordinationError::Unauthorized)
        ));
        assert!(matches!(
            reopened.project_thread(&first.token, &id).await,
            Err(CoordinationError::Unauthorized)
        ));
        drop(reopened);
    }
}

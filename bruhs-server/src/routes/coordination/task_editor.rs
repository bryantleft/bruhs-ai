//! Task editing on the environment that owns the scheduler.
use super::*;
use bruhs_core::reactive::{ReactiveError, ReactiveRuntime};
use bruhs_models::reactive::LinkedTaskSave;

pub(super) fn routes(runtime: CoordinationRuntime) -> Router<Arc<RemoteState>> {
    Router::new()
        .route("/api/projects/task-editor/{id}", get(read).put(save))
        .route("/api/projects/task-models", get(models))
        .route("/api/projects/task-projects", get(projects))
        .route("/api/projects/task-conversations", get(conversations))
        .route("/api/projects/task-branches", get(branches))
        .route_layer(axum::middleware::from_fn_with_state(runtime, authorize))
}

async fn require(runtime: &CoordinationRuntime, bearer: &str) -> Result<(), AppError> {
    match runtime.authorize(bearer).await? {
        AccessScope::ProjectSchedules { edit: true, .. } => Ok(()),
        _ => Err(CoordinationError::Unauthorized.into()),
    }
}

async fn authorize(
    State(runtime): State<CoordinationRuntime>,
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> Result<axum::response::Response, AppError> {
    let bearer = token(request.headers())?;
    require(&runtime, &bearer).await?;
    let response = next.run(request).await;
    require(&runtime, &bearer).await?;
    Ok(response)
}

async fn task_projects(
    state: &RemoteState,
) -> Result<bruhs_core::linked_projects::TaskProjects, AppError> {
    let file = state
        .desktop_state
        .clone()
        .ok_or_else(|| AppError::Unavailable("The host has no desktop workspace.".into()))?;
    bruhs_core::linked_projects::task_projects(file)
        .await
        .map_err(|_| AppError::Unavailable("Could not read the host’s projects.".into()))
}

async fn projects(State(state): State<Arc<RemoteState>>) -> Result<impl IntoResponse, AppError> {
    Ok((
        [(header::CACHE_CONTROL, "no-store")],
        Json(task_projects(&state).await?),
    ))
}

pub(super) fn task_error(e: ReactiveError) -> AppError {
    match e {
        ReactiveError::Storage(_) => {
            AppError::Unavailable("Could not access this host’s scheduled task.".into())
        }
        other => AppError::from(other),
    }
}

async fn can_use_conversations(state: &RemoteState, headers: &HeaderMap) -> Result<bool, AppError> {
    Ok(matches!(
        state.runtime.authorize(&token(headers)?).await?,
        AccessScope::ProjectSchedules {
            edit: true,
            conversations: true,
            ..
        }
    ))
}

#[derive(Deserialize)]
struct ConversationSearch {
    path: String,
    #[serde(default)]
    query: String,
}

async fn conversations(
    State(state): State<Arc<RemoteState>>,
    Extension(runtime): Extension<ReactiveRuntime>,
    headers: HeaderMap,
    axum::extract::Query(search): axum::extract::Query<ConversationSearch>,
) -> Result<impl IntoResponse, AppError> {
    if !can_use_conversations(&state, &headers).await? {
        return Err(CoordinationError::Unauthorized.into());
    }
    shared_project(&state, &runtime, &search.path).await?;
    let rows = runtime
        .project_conversations(search.path, search.query)
        .await
        .map_err(task_error)?;
    Ok(([(header::CACHE_CONTROL, "no-store")], Json(rows)))
}

async fn read(
    State(state): State<Arc<RemoteState>>,
    headers: HeaderMap,
    Extension(runtime): Extension<ReactiveRuntime>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    let mut task = runtime
        .linked_task_definition(id)
        .await
        .map_err(task_error)?;
    if !can_use_conversations(&state, &headers).await? {
        task.conversation = None;
        task.conversation_chosen = false;
    }
    Ok(([(header::CACHE_CONTROL, "no-store")], Json(task)))
}

async fn save(
    State(state): State<Arc<RemoteState>>,
    Extension(runtime): Extension<ReactiveRuntime>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(edit): Json<LinkedTaskSave>,
) -> Result<impl IntoResponse, AppError> {
    if edit.destination.is_some() && !can_use_conversations(&state, &headers).await? {
        return Err(CoordinationError::Unauthorized.into());
    }
    let mut allowed: Vec<String> = task_projects(&state).await?.projects.into_keys().collect();
    // The picker also lists checkouts referenced by the host's schedules.
    if !allowed.contains(&edit.setup.agent.project_path)
        && runtime
            .scheduled_task_inventory()
            .await
            .map_err(task_error)?
            .iter()
            .any(|t| t.project_path == edit.setup.agent.project_path)
    {
        allowed.push(edit.setup.agent.project_path.clone());
    }
    require(&state.runtime, &token(&headers)?).await?;
    let task = runtime
        .save_linked_task(id, edit, allowed)
        .await
        .map_err(task_error)?;
    Ok(([(header::CACHE_CONTROL, "no-store")], Json(task)))
}

#[derive(Deserialize)]
struct ModelScope {
    path: String,
}
async fn models(
    State(state): State<Arc<RemoteState>>,
    Extension(runtime): Extension<ReactiveRuntime>,
    axum::extract::Query(scope): axum::extract::Query<ModelScope>,
) -> Result<impl IntoResponse, AppError> {
    let path = &scope.path;
    shared_project(&state, &runtime, path).await?;
    let catalog = bruhs_core::agent::writer_catalog_for_project(std::path::Path::new(path)).await;
    Ok(([(header::CACHE_CONTROL, "no-store")], Json(catalog)))
}

async fn shared_project(
    state: &RemoteState,
    runtime: &ReactiveRuntime,
    path: &str,
) -> Result<(), AppError> {
    if !task_projects(state).await?.projects.contains_key(path)
        && !runtime
            .scheduled_task_inventory()
            .await
            .map_err(task_error)?
            .iter()
            .any(|t| t.project_path == path)
    {
        return Err(AppError::BadRequest(
            "Choose a project shared by this host.".into(),
        ));
    }
    Ok(())
}
async fn branches(
    State(state): State<Arc<RemoteState>>,
    Extension(runtime): Extension<ReactiveRuntime>,
    axum::extract::Query(scope): axum::extract::Query<ModelScope>,
) -> Result<impl IntoResponse, AppError> {
    shared_project(&state, &runtime, &scope.path).await?;
    let list = bruhs_core::git::branches(std::path::Path::new(&scope.path))
        .await
        .map_err(|_| AppError::BadRequest("Could not read branches in this checkout.".into()))?;
    Ok(([(header::CACHE_CONTROL, "no-store")], Json(list)))
}

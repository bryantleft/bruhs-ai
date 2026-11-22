use super::*;
use bruhs_core::reactive::ReactiveRuntime;
use bruhs_models::{ids::ThreadId, reactive::LinkedThreadReply};

pub(super) fn routes(runtime: CoordinationRuntime) -> Router<Arc<RemoteState>> {
    Router::new()
        .route(
            "/api/projects/scheduled-tasks/{task}/thread/{thread}",
            get(read).post(send),
        )
        .route_layer(axum::middleware::from_fn_with_state(runtime, authorize))
}

async fn permit(runtime: &CoordinationRuntime, bearer: &str, write: bool) -> Result<(), AppError> {
    match runtime.authorize(bearer).await? {
        AccessScope::ProjectSchedules {
            read_threads: true,
            reply_threads,
            ..
        } if !write || reply_threads => Ok(()),
        _ => Err(CoordinationError::Unauthorized.into()),
    }
}
async fn authorize(
    State(runtime): State<CoordinationRuntime>,
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> Result<axum::response::Response, AppError> {
    let write = request.method() == axum::http::Method::POST;
    let bearer = token(request.headers())?;
    permit(&runtime, &bearer, write).await?;
    let response = next.run(request).await;
    permit(&runtime, &bearer, write).await?;
    Ok(response)
}
#[derive(Deserialize)]
struct History {
    before: Option<String>,
}
async fn read(
    Extension(runtime): Extension<ReactiveRuntime>,
    Path((task, thread)): Path<(String, ThreadId)>,
    axum::extract::Query(history): axum::extract::Query<History>,
) -> Result<impl IntoResponse, AppError> {
    let page = runtime
        .linked_thread(task, thread, history.before)
        .await
        .map_err(task_editor::task_error)?;
    Ok(([(header::CACHE_CONTROL, "no-store")], Json(page)))
}
async fn send(
    State(state): State<Arc<RemoteState>>,
    headers: HeaderMap,
    Extension(runtime): Extension<ReactiveRuntime>,
    Path((task, thread)): Path<(String, ThreadId)>,
    Json(reply): Json<LinkedThreadReply>,
) -> Result<impl IntoResponse, AppError> {
    permit(&state.runtime, &token(&headers)?, true).await?;
    let receipt = reply.request_id;
    runtime
        .reply_to_linked_thread(task, thread, reply)
        .await
        .map_err(task_editor::task_error)?;
    Ok(([(header::CACHE_CONTROL, "no-store")], Json(receipt)))
}

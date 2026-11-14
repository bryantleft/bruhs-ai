use crate::error::AppError;
use axum::{
    extract::{Path, Query},
    http::{header::CACHE_CONTROL, StatusCode},
    response::IntoResponse,
    Extension, Json,
};
use bruhs_core::api_client::{ApiClientError, ApiClientHost};
use bruhs_models::api_client::*;
use uuid::Uuid;
fn host(value: Option<Extension<ApiClientHost>>) -> Result<ApiClientHost, AppError> {
    value.map(|Extension(host)| host).ok_or_else(|| {
        AppError::Unavailable("The API client is unavailable. Restart the companion server.".into())
    })
}
impl From<ApiClientError> for AppError {
    fn from(error: ApiClientError) -> Self {
        match error {
            ApiClientError::Conflict | ApiClientError::Busy | ApiClientError::NotPending => {
                Self::Conflict(error.to_string())
            }
            ApiClientError::Storage(_) => Self::Unavailable(error.to_string()),
            _ => Self::BadRequest(error.to_string()),
        }
    }
}
fn private<T: serde::Serialize>(value: T) -> impl IntoResponse {
    ([(CACHE_CONTROL, "no-store")], Json(value))
}
#[derive(serde::Deserialize)]
pub struct Project {
    project_path: String,
}
#[derive(serde::Deserialize)]
pub struct ProposalQuery {
    project_path: String,
    thread_id: String,
    scope: Uuid,
}
pub async fn collection(
    value: Option<Extension<ApiClientHost>>,
    Query(project): Query<Project>,
) -> Result<impl IntoResponse, AppError> {
    Ok(private(
        host(value)?.collection(project.project_path).await?,
    ))
}
pub async fn save(
    value: Option<Extension<ApiClientHost>>,
    body: Result<Json<SaveApiCollection>, axum::extract::rejection::JsonRejection>,
) -> Result<impl IntoResponse, AppError> {
    let Json(request) =
        body.map_err(|_| AppError::BadRequest("Invalid API collection input.".into()))?;
    Ok(private(host(value)?.save(request).await?))
}
pub async fn send(
    value: Option<Extension<ApiClientHost>>,
    body: Result<Json<ApiExecution>, axum::extract::rejection::JsonRejection>,
) -> Result<impl IntoResponse, AppError> {
    let Json(request) =
        body.map_err(|_| AppError::BadRequest("Invalid API request input.".into()))?;
    Ok(private(host(value)?.send(request).await?))
}
pub async fn cancel(
    value: Option<Extension<ApiClientHost>>,
    Path(id): Path<Uuid>,
) -> Result<impl IntoResponse, AppError> {
    host(value)?.cancel(id)?;
    Ok(([(CACHE_CONTROL, "no-store")], StatusCode::NO_CONTENT))
}
pub async fn proposals(
    value: Option<Extension<ApiClientHost>>,
    Query(project): Query<Project>,
) -> Result<impl IntoResponse, AppError> {
    Ok(private(host(value)?.proposals(project.project_path).await?))
}
pub async fn propose(
    value: Option<Extension<ApiClientHost>>,
    body: Result<Json<ProposeApiRequest>, axum::extract::rejection::JsonRejection>,
) -> Result<impl IntoResponse, AppError> {
    let Json(request) =
        body.map_err(|_| AppError::BadRequest("Invalid API proposal input.".into()))?;
    Ok(private(host(value)?.propose(request).await?))
}
pub async fn result(
    value: Option<Extension<ApiClientHost>>,
    Path(id): Path<Uuid>,
    Query(query): Query<ProposalQuery>,
) -> Result<impl IntoResponse, AppError> {
    Ok(private(
        host(value)?
            .proposal_result(query.project_path, query.scope, &query.thread_id, id)
            .await?,
    ))
}
pub async fn decide(
    value: Option<Extension<ApiClientHost>>,
    Json(decision): Json<ApiProposalDecision>,
) -> Result<impl IntoResponse, AppError> {
    Ok(private(host(value)?.decide(decision).await?))
}
pub async fn credential(
    value: Option<Extension<ApiClientHost>>,
    body: Result<Json<ApiCredentialInput>, axum::extract::rejection::JsonRejection>,
) -> Result<impl IntoResponse, AppError> {
    let Json(input) =
        body.map_err(|_| AppError::BadRequest("Invalid private API credential input.".into()))?;
    host(value)?.save_credential(input).await?;
    Ok(private(serde_json::json!({"saved":true})))
}
#[cfg(test)]
mod tests;

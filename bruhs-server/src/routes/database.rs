//! Native database access behind the same local Host/Origin guards as other private controls.
use crate::error::AppError;
use axum::{
    extract::{Path, Query},
    http::{header::CACHE_CONTROL, StatusCode},
    response::IntoResponse,
    Extension, Json,
};
use bruhs_core::database::{DatabaseError, DatabaseHost, DatabaseReader};
use bruhs_models::database::*;
use uuid::Uuid;

fn host(value: Option<Extension<DatabaseHost>>) -> Result<DatabaseHost, AppError> {
    value.map(|Extension(host)| host).ok_or_else(|| {
        AppError::Unavailable(
            "Database browsing is not running. Restart the companion server.".into(),
        )
    })
}
impl From<DatabaseError> for AppError {
    fn from(error: DatabaseError) -> Self {
        match error {
            DatabaseError::Denied => Self::Forbidden(error.to_string()),
            DatabaseError::Changed | DatabaseError::Retargeted => Self::Conflict(error.to_string()),
            DatabaseError::Storage(_) => Self::Unavailable(error.to_string()),
            _ => Self::BadRequest(error.to_string()),
        }
    }
}
#[derive(serde::Deserialize)]
pub struct Project {
    pub project_path: String,
}
#[derive(serde::Deserialize)]
pub struct Schema {
    pub project_path: String,
    pub revision: u64,
}
fn private<T: serde::Serialize>(value: T) -> impl IntoResponse {
    ([(CACHE_CONTROL, "no-store")], Json(value))
}
pub async fn list(
    value: Option<Extension<DatabaseHost>>,
    Query(project): Query<Project>,
) -> Result<impl IntoResponse, AppError> {
    Ok(private(
        host(value)?
            .list(project.project_path, DatabaseReader::User)
            .await?,
    ))
}
pub async fn agent_list(
    value: Option<Extension<DatabaseHost>>,
    Query(project): Query<Project>,
) -> Result<impl IntoResponse, AppError> {
    Ok(private(
        host(value)?
            .list(project.project_path, DatabaseReader::Agent)
            .await?,
    ))
}
pub async fn save(
    value: Option<Extension<DatabaseHost>>,
    body: Result<Json<SaveDatabase>, axum::extract::rejection::JsonRejection>,
) -> Result<impl IntoResponse, AppError> {
    // A JSON rejection can quote password input. Never expose the raw rejection text.
    let Json(request) =
        body.map_err(|_| AppError::BadRequest("Invalid database connection input.".into()))?;
    Ok(private(host(value)?.save(request).await?))
}
pub async fn grant(
    value: Option<Extension<DatabaseHost>>,
    Json(request): Json<DatabaseGrant>,
) -> Result<impl IntoResponse, AppError> {
    Ok(private(host(value)?.grant(request).await?))
}
pub async fn remove(
    value: Option<Extension<DatabaseHost>>,
    Path(id): Path<Uuid>,
    Query(schema): Query<Schema>,
) -> Result<StatusCode, AppError> {
    host(value)?.remove(id, schema.revision).await?;
    Ok(StatusCode::NO_CONTENT)
}
pub async fn tables(
    value: Option<Extension<DatabaseHost>>,
    Path(id): Path<Uuid>,
    Query(schema): Query<Schema>,
) -> Result<impl IntoResponse, AppError> {
    Ok(private(
        host(value)?
            .tables(
                schema.project_path,
                id,
                schema.revision,
                DatabaseReader::User,
            )
            .await?,
    ))
}
pub async fn agent_tables(
    value: Option<Extension<DatabaseHost>>,
    Path(id): Path<Uuid>,
    Query(schema): Query<Schema>,
) -> Result<impl IntoResponse, AppError> {
    Ok(private(
        host(value)?
            .tables(
                schema.project_path,
                id,
                schema.revision,
                DatabaseReader::Agent,
            )
            .await?,
    ))
}
pub async fn query(
    value: Option<Extension<DatabaseHost>>,
    Json(request): Json<ProjectDatabaseQuery>,
) -> Result<impl IntoResponse, AppError> {
    Ok(private(
        host(value)?
            .query(request.project_path, request.query, DatabaseReader::User)
            .await?,
    ))
}
pub async fn agent_query(
    value: Option<Extension<DatabaseHost>>,
    Json(request): Json<ProjectDatabaseQuery>,
) -> Result<impl IntoResponse, AppError> {
    Ok(private(
        host(value)?
            .query(request.project_path, request.query, DatabaseReader::Agent)
            .await?,
    ))
}
pub async fn browse(
    value: Option<Extension<DatabaseHost>>,
    Json(request): Json<ProjectDatabaseBrowse>,
) -> Result<impl IntoResponse, AppError> {
    Ok(private(
        host(value)?
            .browse(request.project_path, request.browse, DatabaseReader::User)
            .await?,
    ))
}
pub async fn describe(
    value: Option<Extension<DatabaseHost>>,
    Json(request): Json<ProjectDatabaseBrowse>,
) -> Result<impl IntoResponse, AppError> {
    Ok(private(
        host(value)?
            .describe(request.project_path, request.browse, DatabaseReader::User)
            .await?,
    ))
}
pub async fn agent_describe(
    value: Option<Extension<DatabaseHost>>,
    Json(request): Json<ProjectDatabaseBrowse>,
) -> Result<impl IntoResponse, AppError> {
    Ok(private(
        host(value)?
            .describe(request.project_path, request.browse, DatabaseReader::Agent)
            .await?,
    ))
}
pub async fn cancel(
    value: Option<Extension<DatabaseHost>>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, AppError> {
    host(value)?.cancel(id)?;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests;

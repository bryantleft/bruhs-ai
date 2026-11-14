use super::{get_json, segment, send_json, ApiError};
use bruhs_models::database::*;
use gloo_net::http::Method;
use uuid::Uuid;

pub async fn databases(project: &str) -> Result<Vec<DatabaseConnection>, ApiError> {
    get_json(&format!("/api/databases?project_path={}", segment(project))).await
}
pub async fn save_database(request: &SaveDatabase) -> Result<DatabaseConnection, ApiError> {
    send_json(Method::POST, "/api/databases", Some(request)).await
}
pub async fn database_grant(request: &DatabaseGrant) -> Result<DatabaseConnection, ApiError> {
    send_json(Method::POST, "/api/databases/grant", Some(request)).await
}
pub async fn database_tables(
    project: &str,
    connection: &DatabaseConnection,
) -> Result<Vec<DatabaseTable>, ApiError> {
    get_json(&format!(
        "/api/databases/{}/tables?project_path={}&revision={}",
        connection.id,
        segment(project),
        connection.revision
    ))
    .await
}
pub async fn database_query(request: &ProjectDatabaseQuery) -> Result<DatabaseResult, ApiError> {
    send_json(Method::POST, "/api/databases/query", Some(request)).await
}
pub async fn database_browse(request: &ProjectDatabaseBrowse) -> Result<DatabaseResult, ApiError> {
    send_json(Method::POST, "/api/databases/browse", Some(request)).await
}
pub async fn database_describe(
    request: &ProjectDatabaseBrowse,
) -> Result<DatabaseResult, ApiError> {
    send_json(Method::POST, "/api/databases/describe", Some(request)).await
}
pub async fn cancel_database_query(id: Uuid) -> Result<(), ApiError> {
    super::http::send_request(
        gloo_net::http::Request::post(&super::http::url(&format!("/api/databases/cancel/{id}")))
            .build()?,
    )
    .await
    .map(|_| ())
}

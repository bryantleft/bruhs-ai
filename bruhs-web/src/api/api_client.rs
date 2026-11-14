use super::{get_json, segment, send_json, ApiError};
use bruhs_models::api_client::*;
use gloo_net::http::Method;
use uuid::Uuid;
pub async fn save_api_credential(input: &ApiCredentialInput) -> Result<(), ApiError> {
    let _: serde_json::Value =
        send_json(Method::POST, "/api/api-client/credential", Some(input)).await?;
    Ok(())
}
pub async fn api_collection(project: &str) -> Result<ApiCollectionSnapshot, ApiError> {
    get_json(&format!(
        "/api/api-client/collection?project_path={}",
        segment(project)
    ))
    .await
}
pub async fn save_api_collection(
    request: &SaveApiCollection,
) -> Result<ApiCollectionSnapshot, ApiError> {
    send_json(Method::POST, "/api/api-client/collection", Some(request)).await
}
pub async fn send_api_request(request: &ApiExecution) -> Result<ApiResponse, ApiError> {
    send_json(Method::POST, "/api/api-client/send", Some(request)).await
}
pub async fn api_proposals(project: &str) -> Result<Vec<ApiProposal>, ApiError> {
    get_json(&format!(
        "/api/api-client/proposals?project_path={}",
        segment(project)
    ))
    .await
}
pub async fn decide_api_proposal(request: &ApiProposalDecision) -> Result<ApiProposal, ApiError> {
    send_json(Method::POST, "/api/api-client/decision", Some(request)).await
}
pub async fn cancel_api_request(id: Uuid) -> Result<(), ApiError> {
    super::http::send_request(
        gloo_net::http::Request::post(&super::http::url(&format!("/api/api-client/cancel/{id}")))
            .build()?,
    )
    .await
    .map(|_| ())
}

//! Server-owned event agents; these records never enter browser metadata storage.
use super::{get_json, segment, send_json, ApiError};
use bruhs_models::reactive::{ReactiveCommand, ReactiveSnapshot};
use gloo_net::http::Method;
use serde::Deserialize;

#[derive(Clone, Debug, Deserialize)]
pub struct ReactiveHookToken {
    pub token: String,
    pub path: String,
}

pub async fn reactive_snapshot() -> Result<ReactiveSnapshot, ApiError> {
    get_json("/api/reactive").await
}

pub async fn reactive_command(command: &ReactiveCommand) -> Result<ReactiveSnapshot, ApiError> {
    send_json(Method::POST, "/api/reactive/commands", Some(command)).await
}

pub async fn reactive_hook_token(source_id: &str) -> Result<ReactiveHookToken, ApiError> {
    get_json(&format!(
        "/api/reactive/sources/{}/token",
        segment(source_id)
    ))
    .await
}

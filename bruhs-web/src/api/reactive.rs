//! Server-owned event agents; these records never enter browser metadata storage.
use super::{get_json, segment, send_json, ApiError};
use bruhs_models::api::{BackgroundService, BackgroundServiceAction};
use bruhs_models::reactive::{ReactiveCommand, ReactiveSnapshot};
use bruhs_models::weekly_reports::{EngineeringReport, WeeklyReportCommand, WeeklyReportsSnapshot};
use gloo_net::http::Method;
use serde::Deserialize;

pub async fn weekly_reports(before: Option<&str>) -> Result<WeeklyReportsSnapshot, ApiError> {
    get_json(&before.map_or_else(
        || "/api/weekly-reports".into(),
        |before| format!("/api/weekly-reports?before={}", segment(before)),
    ))
    .await
}

pub async fn weekly_report(id: &str) -> Result<EngineeringReport, ApiError> {
    get_json(&format!("/api/weekly-reports/{}", segment(id))).await
}

pub async fn weekly_report_command(
    command: &WeeklyReportCommand,
) -> Result<WeeklyReportsSnapshot, ApiError> {
    send_json(Method::POST, "/api/weekly-reports/commands", Some(command)).await
}

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

/// Whether bruhs-server runs as a login service, so agents keep running after quit.
pub async fn background_service() -> Result<BackgroundService, ApiError> {
    get_json("/api/service").await
}

pub async fn set_background_service(
    action: BackgroundServiceAction,
) -> Result<BackgroundService, ApiError> {
    send_json(Method::POST, "/api/service", Some(&action)).await
}

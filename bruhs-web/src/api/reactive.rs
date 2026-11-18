//! Server-owned event agents; these records never enter browser metadata storage.
use super::{get_json, segment, send_json, ApiError, BASE_URL};
use bruhs_models::api::{BackgroundService, BackgroundServiceAction};
use bruhs_models::local_tools::{ComputerRequest, ComputerState};
use bruhs_models::reactive::{ReactiveCommand, ReactiveRun, ReactiveSnapshot};
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

/// One agent's runs, newest first; `before` is the id of the last run already shown.
pub async fn agent_runs(
    agent_id: &str,
    before: Option<&str>,
) -> Result<Vec<ReactiveRun>, ApiError> {
    let base = format!("/api/reactive/agents/{}/runs", segment(agent_id));
    get_json(&before.map_or(base.clone(), |before| {
        format!("{base}?before={}", segment(before))
    }))
    .await
}

/// One room's messages, newest first; `before` is the id of the last message already shown.
pub async fn room_messages(
    room_id: &str,
    before: Option<&str>,
) -> Result<Vec<bruhs_models::crew::room::RoomMessage>, ApiError> {
    let base = format!("/api/reactive/rooms/{}/messages", segment(room_id));
    get_json(&before.map_or(base.clone(), |before| {
        format!("{base}?before={}", segment(before))
    }))
    .await
}

#[expect(
    dead_code,
    reason = "the bruh page's Computer tab is the first caller, in the next change"
)]
/// A bruh's Computer as the service has it now.
pub async fn computer_state(agent_id: &str) -> Result<ComputerState, ApiError> {
    get_json(&format!(
        "/api/reactive/agents/{}/computer",
        segment(agent_id)
    ))
    .await
}

#[expect(
    dead_code,
    reason = "the bruh page's Computer tab is the first caller, in the next change"
)]
/// Opens, closes, takes control of, hands back or opens a window for a
/// bruh's Computer.
pub async fn computer_request(
    agent_id: &str,
    request: &ComputerRequest,
) -> Result<ComputerState, ApiError> {
    send_json(
        Method::POST,
        &format!("/api/reactive/agents/{}/computer", segment(agent_id)),
        Some(request),
    )
    .await
}

#[expect(
    dead_code,
    reason = "the bruh page's Computer tab is the first caller, in the next change"
)]
/// The live view's WebSocket address for a browser session.
pub fn screencast_url(session_id: &str) -> String {
    bruhs_models::local_tools::screencast_url(BASE_URL, session_id)
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

use axum::{
    extract::{Query, State},
    response::{Html, IntoResponse},
    Json,
};
use bruhs_core::{AppState, GoogleCalendarHandleError, StartOAuthResponse};
use bruhs_models::GoogleCalendarSyncState;
use serde::Deserialize;
use std::sync::Arc;

use crate::error::AppError;

/// POST /api/calendar/google/oauth/start
pub async fn oauth_start(
    State(state): State<Arc<AppState>>,
) -> Result<Json<StartOAuthResponse>, AppError> {
    state
        .google_calendar()
        .start_oauth()
        .map(Json)
        .map_err(handle_err_to_app)
}

#[derive(Deserialize)]
pub struct OAuthCallbackQuery {
    pub code: Option<String>,
    pub state: Option<String>,
    pub error: Option<String>,
}

/// GET /api/calendar/google/oauth/callback
pub async fn oauth_callback(
    State(state): State<Arc<AppState>>,
    Query(q): Query<OAuthCallbackQuery>,
) -> impl IntoResponse {
    if let Some(err) = q.error.as_deref() {
        return Html(error_page(&format!("Google returned: {err}"))).into_response();
    }
    let (code, state_token) = match (q.code, q.state) {
        (Some(c), Some(s)) => (c, s),
        _ => return Html(error_page("Missing code or state parameter")).into_response(),
    };
    match state
        .google_calendar()
        .complete_oauth(&code, &state_token)
        .await
    {
        Ok(()) => Html(success_page()).into_response(),
        Err(GoogleCalendarHandleError::InvalidState) => Html(error_page(
            "OAuth state expired or unrecognized — try Connect again.",
        ))
        .into_response(),
        Err(e) => Html(error_page(&format!("OAuth failed: {e}"))).into_response(),
    }
}

/// POST /api/calendar/google/disconnect
pub async fn disconnect(State(state): State<Arc<AppState>>) -> Result<(), AppError> {
    state
        .google_calendar()
        .disconnect()
        .await
        .map_err(handle_err_to_app)
}

/// GET /api/calendar/google/state
pub async fn get_state(State(state): State<Arc<AppState>>) -> Json<GoogleCalendarSyncState> {
    Json(state.google_calendar().state().await)
}

/// POST /api/calendar/google/sync — trigger an immediate sync tick.
pub async fn force_sync(State(state): State<Arc<AppState>>) -> Result<(), AppError> {
    state.google_calendar().force_sync();
    Ok(())
}

/// GET /api/calendar/google/calendars
pub async fn list_calendars(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Vec<bruhs_models::Calendar>>, AppError> {
    state
        .google_calendar()
        .list_calendars()
        .await
        .map(Json)
        .map_err(handle_err_to_app)
}

#[derive(serde::Deserialize)]
pub struct PatchCalendarRequest {
    pub sync_enabled: bool,
}

/// PATCH /api/calendar/google/calendars/{id}
pub async fn patch_calendar(
    State(state): State<Arc<AppState>>,
    axum::extract::Path(id): axum::extract::Path<String>,
    Json(body): Json<PatchCalendarRequest>,
) -> Result<Json<bruhs_models::Calendar>, AppError> {
    match state
        .google_calendar()
        .patch_calendar(&id, body.sync_enabled)
        .await
    {
        Ok(Some(c)) => Ok(Json(c)),
        Ok(None) => Err(AppError::NotFound(format!("unknown calendar id: {id}"))),
        Err(e) => Err(handle_err_to_app(e)),
    }
}

fn handle_err_to_app(e: GoogleCalendarHandleError) -> AppError {
    match e {
        GoogleCalendarHandleError::NotConfigured(inner) => AppError::BadRequest(format!("{inner}")),
        GoogleCalendarHandleError::InvalidState => {
            AppError::BadRequest("invalid or expired OAuth state".into())
        }
        other => AppError::internal(other),
    }
}

fn success_page() -> String {
    r#"<!doctype html>
<html><head><meta charset="utf-8"><title>Connected — bruhs</title>
<style>
  body { font-family: -apple-system, BlinkMacSystemFont, system-ui, sans-serif;
         background: #1a1614; color: #e6cfb8; padding: 4rem 2rem; text-align: center; }
  h1 { font-size: 1.4rem; font-weight: 600; margin-bottom: .5rem; }
  p { color: #a08d76; font-size: 0.95rem; }
</style></head>
<body>
  <h1>Google Calendar connected</h1>
  <p>You can close this tab and return to bruhs.</p>
</body></html>"#
        .to_string()
}

fn error_page(message: &str) -> String {
    format!(
        r#"<!doctype html>
<html><head><meta charset="utf-8"><title>OAuth failed — bruhs</title>
<style>
  body {{ font-family: -apple-system, BlinkMacSystemFont, system-ui, sans-serif;
         background: #1a1614; color: #e6cfb8; padding: 4rem 2rem; text-align: center; }}
  h1 {{ font-size: 1.4rem; font-weight: 600; margin-bottom: .5rem; color: #e89191; }}
  p {{ color: #a08d76; font-size: 0.95rem; }}
  code {{ font-family: ui-monospace, monospace; background: #2a2421; padding: .15rem .35rem; border-radius: 3px; }}
</style></head>
<body>
  <h1>Could not connect Google Calendar</h1>
  <p><code>{}</code></p>
  <p>You can close this tab and try again from bruhs.</p>
</body></html>"#,
        html_escape(message)
    )
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use bruhs_core::google_calendar::{
        cursor::CalendarFiles, tokens::TokenStore, GoogleCalendarHandle,
    };

    /// App state whose Google handle uses an in-memory keychain and a temp
    /// directory, so tests never touch the real keychain or `~/Library`.
    fn hermetic_state(dir: &std::path::Path) -> Arc<AppState> {
        let app = AppState::builder()
            .google_calendar(Arc::new(GoogleCalendarHandle::new(
                None,
                TokenStore::in_memory(),
                CalendarFiles::at(dir),
            )))
            .build();
        Arc::new(app)
    }

    #[tokio::test]
    async fn get_state_reports_disconnected_without_tokens() {
        let tmp = tempfile::tempdir().unwrap();
        let json = get_state(State(hermetic_state(tmp.path()))).await;
        assert!(!json.0.is_connected());
    }

    #[tokio::test]
    async fn patching_an_unknown_calendar_is_not_found() {
        let tmp = tempfile::tempdir().unwrap();
        let result = patch_calendar(
            State(hermetic_state(tmp.path())),
            axum::extract::Path("missing".to_string()),
            Json(PatchCalendarRequest { sync_enabled: true }),
        )
        .await;
        assert!(matches!(result, Err(AppError::NotFound(_))), "{result:?}");
    }

    #[tokio::test]
    async fn corrupt_calendar_list_is_a_server_error_not_an_empty_list() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("calendars.json"), b"oops").unwrap();
        let state = hermetic_state(tmp.path());
        assert!(matches!(
            list_calendars(State(state.clone())).await,
            Err(AppError::Internal { .. })
        ));
        assert!(matches!(
            patch_calendar(
                State(state),
                axum::extract::Path("one".to_string()),
                Json(PatchCalendarRequest { sync_enabled: true }),
            )
            .await,
            Err(AppError::Internal { .. })
        ));
        assert_eq!(
            std::fs::read(tmp.path().join("calendars.json")).unwrap(),
            b"oops"
        );
    }
}

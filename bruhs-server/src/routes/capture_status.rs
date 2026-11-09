//! GET /api/capture/status — surfaces whether bruhs-capture is running
//! and (if so) what it's capturing. The web UI polls this on mount.

use axum::{extract::Extension, Json};
use bruhs_core::capture_client::{self, CaptureClientError};
use bruhs_core::DaemonStatus;
use bruhs_models::api::{ActiveRecording, CaptureStatus, RunningCapture, SystemAudioPermission};
use bruhs_models::ids::MeetingId;
use bruhs_models::EpochMs;

use crate::error::AppError;
use crate::routes::capture_proxy::{token_refused, Daemon};

/// GET /api/capture/status — the shared [`CaptureStatus`] (its wire shape is
/// the flat object this route has always sent). A daemon that is not running
/// or not reachable is `running: false`; one that answers but refuses the
/// server's token, or answers with anything else than its status, is 502
/// with an explanatory error, like the capture proxy, so clients never
/// report a running daemon as off.
pub async fn get_capture_status(
    Extension(daemon): Daemon,
) -> Result<Json<CaptureStatus>, AppError> {
    match capture_client::fetch_status_from(&daemon).await {
        Ok(status) => Ok(Json(running(status))),
        Err(CaptureClientError::NotRunning) => Ok(Json(CaptureStatus::NotRunning)),
        Err(CaptureClientError::Status { status: 401, .. }) => Err(token_refused()),
        Err(error) => Err(AppError::bad_gateway(
            "capture daemon status request failed",
            error,
        )),
    }
}

fn running(status: DaemonStatus) -> CaptureStatus {
    let recording = if status.recording {
        let meeting_id = status.meeting_id.map(MeetingId::try_from);
        let started_at_ms = status.started_at_ms.map(EpochMs::from_f64_floor);
        match (meeting_id, started_at_ms) {
            (Some(Ok(meeting_id)), Some(Ok(started_at_ms))) => Some(ActiveRecording {
                meeting_id,
                started_at_ms,
            }),
            (meeting_id, started_at_ms) => {
                tracing::warn!(
                    "capture daemon reports a recording without a usable meeting id \
                     ({meeting_id:?}) or start time ({started_at_ms:?})"
                );
                None
            }
        }
    } else {
        None
    };
    // An unknown permission string reads as Unknown rather than failing.
    let system_audio_permission =
        serde_json::from_value(serde_json::Value::String(status.system_audio_permission))
            .unwrap_or(SystemAudioPermission::Unknown);
    CaptureStatus::Running(RunningCapture {
        recording,
        system_audio_permission,
        system_audio_capturing: status.system_audio_capturing,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::StatusCode;
    use bruhs_core::capture_client::DaemonEndpoint;
    use serde_json::json;
    use std::sync::Arc;

    const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    /// A daemon that answers `/status` only with the bearer token.
    async fn fake_daemon() -> String {
        let app = axum::Router::new().route(
            "/status",
            axum::routing::get(|headers: axum::http::HeaderMap| async move {
                let authorized = headers
                    .get(axum::http::header::AUTHORIZATION)
                    .and_then(|value| value.to_str().ok())
                    == Some(&format!("Bearer {TOKEN}"));
                if !authorized {
                    return Err(StatusCode::UNAUTHORIZED);
                }
                Ok(Json(json!({
                    "recording": false,
                    "meeting_id": null,
                    "started_at_ms": null,
                    "system_audio_permission": "granted",
                    "system_audio_capturing": false,
                })))
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        url
    }

    async fn status(daemon: DaemonEndpoint) -> axum::response::Response {
        use axum::response::IntoResponse;
        get_capture_status(Extension(Arc::new(daemon)))
            .await
            .into_response()
    }

    async fn json_body(response: axum::response::Response) -> serde_json::Value {
        let bytes = axum::body::to_bytes(response.into_body(), 64 * 1024)
            .await
            .unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    /// A daemon that refuses the server's token is running: the status says
    /// so with a 502 instead of reporting `running: false`.
    #[tokio::test]
    async fn a_refused_token_is_a_bad_gateway_not_a_stopped_daemon() {
        let daemon = fake_daemon().await;
        let refused = status(DaemonEndpoint::new(daemon.clone(), None)).await;
        assert_eq!(refused.status(), StatusCode::BAD_GATEWAY);
        let body = json_body(refused).await;
        assert!(body["error"].as_str().unwrap().contains("token"), "{body}");

        let dir = tempfile::tempdir().unwrap();
        let token_file = dir.path().join("daemon-token");
        std::fs::write(&token_file, TOKEN).unwrap();
        let ok = status(DaemonEndpoint::new(daemon, Some(token_file))).await;
        assert_eq!(ok.status(), StatusCode::OK);
        let body = json_body(ok).await;
        assert_eq!(body["running"], true, "{body}");
        assert_eq!(body["system_audio_permission"], "granted", "{body}");
    }

    #[tokio::test]
    async fn an_unreachable_daemon_is_not_running() {
        // Nothing listens on port 1.
        let down = status(DaemonEndpoint::new("http://127.0.0.1:1", None)).await;
        assert_eq!(down.status(), StatusCode::OK);
        assert_eq!(json_body(down).await["running"], false);
    }

    #[test]
    fn daemon_status_keeps_the_flat_wire_shape() {
        let status = running(DaemonStatus {
            recording: true,
            meeting_id: Some("m-1".into()),
            started_at_ms: Some(1500.0),
            system_audio_permission: "not_determined".into(),
            system_audio_capturing: true,
        });
        assert_eq!(
            serde_json::to_value(&status).unwrap(),
            json!({
                "running": true, "recording": true, "meeting_id": "m-1",
                "started_at_ms": 1500.0, "system_audio_permission": "not_determined",
                "system_audio_capturing": true,
            })
        );
        assert_eq!(
            serde_json::to_value(CaptureStatus::NotRunning).unwrap(),
            json!({
                "running": false, "recording": false, "meeting_id": null,
                "started_at_ms": null, "system_audio_permission": "",
                "system_audio_capturing": false,
            })
        );
    }

    /// A contradictory daemon reply degrades to "not recording" instead of
    /// failing the whole status.
    #[test]
    fn a_recording_without_a_meeting_id_is_not_a_recording() {
        let status = running(DaemonStatus {
            recording: true,
            meeting_id: None,
            started_at_ms: Some(1.0),
            system_audio_permission: "granted".into(),
            system_audio_capturing: false,
        });
        assert!(status.is_running() && status.recording().is_none());
    }
}

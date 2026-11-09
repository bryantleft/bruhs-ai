//! Liveness + status endpoints. `/health` is cheap; `/status` returns
//! whether a meeting is currently being captured.

use axum::{extract::State, Json};
use serde::Serialize;
use std::sync::Arc;

use crate::state::CaptureState;

#[derive(Debug, Serialize)]
pub struct HealthResponse {
    pub ok: bool,
    /// Always `bruhs-capture`: Bruhs.app checks it before adopting a daemon
    /// already on the port.
    pub app: &'static str,
    pub version: &'static str,
    /// The commit this daemon was built from (empty outside a checkout).
    pub build: &'static str,
}

#[derive(Debug, Serialize)]
pub struct StatusResponse {
    pub recording: bool,
    pub meeting_id: Option<String>,
    pub started_at_ms: Option<f64>,
    pub system_audio_permission: String, // "granted" | "denied" | "not_determined" | "unsupported"
    pub system_audio_capturing: bool,
}

/// GET /health
pub async fn health() -> Json<HealthResponse> {
    Json(HealthResponse {
        ok: true,
        app: "bruhs-capture",
        version: env!("CARGO_PKG_VERSION"),
        build: env!("BRUHS_BUILD_COMMIT"),
    })
}

/// GET /status
pub async fn status(State(state): State<Arc<CaptureState>>) -> Json<StatusResponse> {
    let perm = match state.permission.status().await {
        crate::capture::PermissionStatus::Granted => "granted",
        crate::capture::PermissionStatus::Denied => "denied",
        crate::capture::PermissionStatus::NotDetermined => "not_determined",
        crate::capture::PermissionStatus::Unsupported => "unsupported",
    };
    let capturing = state.has_sys_recorder();
    Json(state.with_current(|cur| match cur {
        Some(s) => StatusResponse {
            recording: true,
            meeting_id: Some(s.meeting_id.clone()),
            started_at_ms: Some(s.started_at_ms),
            system_audio_permission: perm.into(),
            system_audio_capturing: capturing,
        },
        None => StatusResponse {
            recording: false,
            meeting_id: None,
            started_at_ms: None,
            system_audio_permission: perm.into(),
            system_audio_capturing: capturing,
        },
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn health_names_the_daemon_and_its_build() {
        let Json(reply) = health().await;
        assert!(reply.ok);
        assert_eq!(reply.app, "bruhs-capture");
        assert_eq!(reply.build, env!("BRUHS_BUILD_COMMIT"));
    }

    #[tokio::test]
    async fn status_reports_the_cached_permission() {
        let (_root, mut state) = crate::state::isolated_state();
        state.permission =
            crate::capture::PermissionCache::new(|| crate::capture::PermissionStatus::Denied);
        let Json(reply) = status(State(Arc::new(state))).await;
        assert!(!reply.recording);
        assert_eq!(reply.system_audio_permission, "denied");
    }
}

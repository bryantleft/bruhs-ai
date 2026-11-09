//! Provider CLI and app update checks for the web (desktop calls
//! `bruhs_core::updates` directly).
//!
//! - `GET /api/updates` — the cached report with this server's update runs,
//!   checked first when it is stale. `?refresh=1` checks now. `?online=0`
//!   (update checks switched off) reads installed versions only and asks no
//!   registry or remote.
//! - `POST /api/updates/providers/{provider}` — start that CLI's installer
//!   (the command is recomputed here, never sent by the client) and return
//!   the run at once. The run is detached: a closed tab or a reload never
//!   stops an installer halfway. 409 while one is already running.

use axum::extract::{rejection::PathRejection, Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use bruhs_core::updates::UpdateChecker;
use bruhs_models::agent::AgentProvider;
use bruhs_models::updates::{ProviderRun, UpdatesReport};
use serde::Deserialize;

use crate::error::AppError;
use crate::routes::cli;

#[derive(Debug, Deserialize, Default)]
pub struct ReportQuery {
    #[serde(default)]
    refresh: Option<String>,
    #[serde(default)]
    online: Option<String>,
}

fn flag(value: Option<&str>, default: bool) -> bool {
    match value {
        Some("1" | "true") => true,
        Some("0" | "false") => false,
        _ => default,
    }
}

pub async fn get_updates(
    State(checker): State<UpdateChecker>,
    Query(query): Query<ReportQuery>,
) -> Json<UpdatesReport> {
    let force = flag(query.refresh.as_deref(), false);
    let online = flag(query.online.as_deref(), true);
    Json(checker.report(force, online).await)
}

pub async fn post_provider_update(
    State(checker): State<UpdateChecker>,
    provider: Result<Path<AgentProvider>, PathRejection>,
) -> Result<(StatusCode, Json<ProviderRun>), AppError> {
    let provider = cli::provider(provider)?;
    let run = checker
        .start_update(provider)
        .map_err(|error| AppError::Conflict(error.to_string()))?;
    tracing::info!("started updating {} (run {})", provider.as_str(), run.id);
    Ok((StatusCode::ACCEPTED, Json(run)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use bruhs_core::updates::BuildInfo;

    #[test]
    fn query_flags_default_sensibly() {
        assert!(flag(None, true));
        assert!(!flag(None, false));
        assert!(flag(Some("1"), false));
        assert!(!flag(Some("0"), true));
        assert!(flag(Some("junk"), true));
    }

    #[tokio::test]
    async fn unknown_providers_are_rejected_before_anything_runs() {
        let app = axum::Router::new()
            .route(
                "/api/updates/providers/{provider}",
                axum::routing::post(post_provider_update),
            )
            .with_state(UpdateChecker::new(BuildInfo::default()));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!(
            "http://{}/api/updates/providers/rm",
            listener.local_addr().unwrap()
        );
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let response = reqwest::Client::builder()
            .no_proxy()
            .build()
            .unwrap()
            .post(url)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::BAD_REQUEST);
        server.abort();
    }
}

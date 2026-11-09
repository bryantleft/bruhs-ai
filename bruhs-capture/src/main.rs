//! bruhs-capture — local audio capture daemon. Runs on 127.0.0.1:3002.
//!
//! Records the microphone and, on macOS, system audio (ScreenCaptureKit)
//! to per-meeting WAV files, and emits lifecycle events over SSE.

use axum::{
    routing::{get, post},
    Router,
};
use std::sync::Arc;

mod activity_monitor;
mod audio;
mod auth;
mod capture;
mod capture_lifecycle;
mod diarize;
mod live_transcribe;
mod local_access;
mod mix;
mod models_dir;
mod parent_watch;
mod paths;
mod postprocess;
mod recorder_start;
mod recovery;
mod routes;
mod state;
mod subdiarize;
mod transcribe;
mod video_ocr;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt::init();
    parent_watch::exit_with_parent();

    let data = paths::DataRoot::from_project_dirs()?;

    // Env-configurable bind port so the daemon coexists with other dev
    // projects without port collisions. Defaults to 3002.
    let port = std::env::var("BRUHS_CAPTURE_PORT").unwrap_or_else(|_| "3002".into());
    let addr = format!("127.0.0.1:{port}");
    // Bind before writing the token: a second instance that cannot bind must
    // not replace the token of the daemon already serving this port.
    let listener = tokio::net::TcpListener::bind(&addr).await?;

    // A fresh token per launch; clients re-read the file on every request.
    let token = auth::DaemonToken::generate();
    let token_path = auth::token_path(&data);
    tokio::task::spawn_blocking({
        let (token, token_path, data) = (token.clone(), token_path.clone(), data.clone());
        move || token.write_to(&token_path, &data)
    })
    .await??;
    tracing::info!("capture auth token written to {}", token_path.display());

    let state = Arc::new(state::CaptureState::new(data));

    // The worker starts idle; only an explicit, live consent lease permits
    // microphone probing. Startup itself never opens an input device.
    activity_monitor::spawn(Arc::clone(&state));

    let app = router(state, token);
    tracing::info!("bruhs-capture listening on http://{addr}");
    axum::serve(listener, app).await?;
    Ok(())
}

/// Build handlers without starting hardware workers (also used by HTTP tests).
/// Every route but `/health` requires `token`.
fn router(state: Arc<state::CaptureState>, token: auth::DaemonToken) -> Router {
    Router::new()
        .route("/health", get(routes::health::health))
        .route("/status", get(routes::health::status))
        .route(
            "/activity-monitor",
            get(routes::activity_monitor::status).post(routes::activity_monitor::command),
        )
        .route("/capture/start", post(routes::capture::start_capture))
        .route("/capture/stop", post(routes::capture::stop_capture))
        .route("/events", get(routes::events::get_events))
        .route("/recovery", get(routes::recovery::pending))
        .route("/recovery/failures", get(routes::recovery::failures))
        .route(
            "/recovery/failures/ack",
            post(routes::recovery::acknowledge_failures)
                .layer(axum::extract::DefaultBodyLimit::max(32 * 1024)),
        )
        .route(
            "/recovery/ack",
            post(routes::recovery::acknowledge)
                .layer(axum::extract::DefaultBodyLimit::max(32 * 1024)),
        )
        .route(
            "/centroids/{meeting_id}",
            get(routes::centroids::list_centroids),
        )
        .route("/ocr/{meeting_id}", get(routes::ocr::list_ocr))
        .route(
            "/segments/{meeting_id}",
            get(routes::segments::list_segments),
        )
        .route("/enroll/{person_id}", post(routes::enroll::enroll_voice))
        .layer(axum::middleware::from_fn_with_state(
            token,
            auth::require_token,
        ))
        .layer(axum::middleware::from_fn(local_access::guard))
        .with_state(state)
}

/// The production router with a fresh token, plus a client that sends it.
#[cfg(test)]
fn test_router(state: Arc<state::CaptureState>) -> (Router, reqwest::Client) {
    let token = auth::DaemonToken::generate();
    let mut headers = reqwest::header::HeaderMap::new();
    let mut bearer = reqwest::header::HeaderValue::from_str(&format!("Bearer {}", token.as_str()))
        .expect("hex token is a valid header value");
    bearer.set_sensitive(true);
    headers.insert(reqwest::header::AUTHORIZATION, bearer);
    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(std::time::Duration::from_secs(5))
        .default_headers(headers)
        .build()
        .expect("test client");
    (router(state, token), client)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every route but `/health` answers 401 without the launch token.
    #[tokio::test]
    async fn routes_require_the_launch_token() {
        let (_root, mut state) = state::isolated_state();
        state.permission =
            capture::PermissionCache::new(|| capture::PermissionStatus::NotDetermined);
        let (app, authorized) = test_router(Arc::new(state));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let anonymous = reqwest::Client::builder().no_proxy().build().unwrap();
        let wrong = format!("Bearer {}", auth::DaemonToken::generate().as_str());
        for (method, path) in [
            ("GET", "/status"),
            ("GET", "/activity-monitor"),
            ("POST", "/capture/start"),
            ("POST", "/capture/stop"),
            ("GET", "/events"),
            ("GET", "/recovery"),
            ("GET", "/recovery/failures"),
            ("GET", "/segments/m"),
            ("GET", "/centroids/m"),
            ("GET", "/ocr/m"),
            ("POST", "/enroll/p"),
        ] {
            let method = reqwest::Method::from_bytes(method.as_bytes()).unwrap();
            let url = format!("{base}{path}");
            for request in [
                anonymous.request(method.clone(), &url),
                anonymous
                    .request(method.clone(), &url)
                    .header("authorization", &wrong),
                anonymous
                    .request(method.clone(), &url)
                    .header("authorization", "Basic Zm9vOmJhcg=="),
            ] {
                let response = request.send().await.unwrap();
                assert_eq!(response.status(), 401, "{method} {path}");
                assert_eq!(response.headers()["www-authenticate"], "Bearer");
            }
        }
        assert_eq!(
            anonymous
                .get(format!("{base}/health"))
                .send()
                .await
                .unwrap()
                .status(),
            200
        );
        assert_eq!(
            authorized
                .get(format!("{base}/status"))
                .send()
                .await
                .unwrap()
                .status(),
            200
        );
        server.abort();
    }
}

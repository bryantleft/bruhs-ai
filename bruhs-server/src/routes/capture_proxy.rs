//! `/api/capture/{start,stop,activity-monitor}` — the browser's way to drive
//! the capture daemon. The daemon requires a per-launch bearer token that only
//! native processes can read, so the server forwards these requests with the
//! token attached and relays the daemon's status and body unchanged. The
//! server's local-access guard still applies to the browser request.

use axum::{
    body::Bytes,
    extract::Extension,
    http::{header, HeaderMap, HeaderValue, Method, StatusCode},
    response::{IntoResponse, Response},
};
use bruhs_core::capture_client::{self, CaptureClientError, DaemonEndpoint, ProxiedRoute};
use std::sync::Arc;

use crate::error::AppError;

/// The daemon the proxy forwards to (an `Extension` so tests can point it at a
/// fake daemon).
pub type Daemon = Extension<Arc<DaemonEndpoint>>;

/// POST /api/capture/start
pub async fn start(
    Extension(daemon): Daemon,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, AppError> {
    forward(&daemon, Method::POST, ProxiedRoute::Start, &headers, body).await
}

/// POST /api/capture/stop
pub async fn stop(
    Extension(daemon): Daemon,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, AppError> {
    forward(&daemon, Method::POST, ProxiedRoute::Stop, &headers, body).await
}

/// GET /api/capture/activity-monitor
pub async fn activity_monitor_status(Extension(daemon): Daemon) -> Result<Response, AppError> {
    forward(
        &daemon,
        Method::GET,
        ProxiedRoute::ActivityMonitor,
        &HeaderMap::new(),
        Bytes::new(),
    )
    .await
}

/// POST /api/capture/activity-monitor
pub async fn activity_monitor_command(
    Extension(daemon): Daemon,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, AppError> {
    forward(
        &daemon,
        Method::POST,
        ProxiedRoute::ActivityMonitor,
        &headers,
        body,
    )
    .await
}

/// The daemon refused the server's token (missing, stale or from another
/// daemon): this server's misconfiguration, reported as 502 so the browser
/// never sees a 401 for the server's own credential.
pub(crate) fn token_refused() -> AppError {
    AppError::BadGateway {
        message: "capture daemon rejected the server's token; restart the capture daemon or \
                  check BRUHS_CAPTURE_TOKEN_FILE"
            .into(),
        source: None,
    }
}

async fn forward(
    daemon: &DaemonEndpoint,
    method: Method,
    route: ProxiedRoute,
    headers: &HeaderMap,
    body: Bytes,
) -> Result<Response, AppError> {
    let body = (!body.is_empty()).then(|| {
        let content_type = headers
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("application/json")
            .to_owned();
        (content_type, reqwest::Body::from(body))
    });
    let reply = match capture_client::proxy(daemon, method, route, body).await {
        Ok(reply) => reply,
        Err(CaptureClientError::NotRunning) => {
            return Err(AppError::Unavailable("capture daemon not running".into()))
        }
        Err(error) => {
            return Err(AppError::bad_gateway(
                "capture daemon request failed",
                error,
            ))
        }
    };
    let status = StatusCode::from_u16(reply.status).map_err(|error| {
        AppError::bad_gateway("capture daemon replied with an invalid status", error)
    })?;
    if status == StatusCode::UNAUTHORIZED {
        // The daemon refused the server's token (missing, stale or from
        // another daemon). That is this server's misconfiguration, not the
        // browser's credentials, so it must not reach the browser as 401.
        return Err(token_refused());
    }
    let mut response = (status, reply.body).into_response();
    if let Some(content_type) = reply
        .content_type
        .and_then(|value| HeaderValue::from_str(&value).ok())
    {
        response
            .headers_mut()
            .insert(header::CONTENT_TYPE, content_type);
    }
    Ok(response)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::routing::{get, post};
    use std::sync::Mutex;

    /// One request as the fake daemon received it.
    #[derive(Debug)]
    struct Received {
        method: String,
        path: String,
        authorization: Option<String>,
        content_type: Option<String>,
        body: Vec<u8>,
    }

    #[derive(Default, Debug)]
    struct Seen {
        requests: Vec<Received>,
    }

    /// A fake daemon that records what reached it and answers like the real
    /// one: 401 without the token.
    async fn fake_daemon(token: &'static str) -> (String, Arc<Mutex<Seen>>) {
        let seen = Arc::new(Mutex::new(Seen::default()));
        let record = seen.clone();
        let app = axum::Router::new().fallback(
            move |method: Method, uri: axum::http::Uri, headers: HeaderMap, body: Bytes| {
                let record = record.clone();
                async move {
                    let header_text = |name| {
                        headers
                            .get(name)
                            .and_then(|v: &HeaderValue| v.to_str().ok())
                            .map(str::to_owned)
                    };
                    let authorization = header_text(header::AUTHORIZATION);
                    record.lock().unwrap().requests.push(Received {
                        method: method.to_string(),
                        path: uri.path().to_owned(),
                        authorization: authorization.clone(),
                        content_type: header_text(header::CONTENT_TYPE),
                        body: body.to_vec(),
                    });
                    if authorization.as_deref() != Some(&format!("Bearer {token}")) {
                        return StatusCode::UNAUTHORIZED.into_response();
                    }
                    match uri.path() {
                        "/capture/start" => (
                            StatusCode::CONFLICT,
                            [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
                            "already recording",
                        )
                            .into_response(),
                        _ => (
                            [(header::CONTENT_TYPE, "application/json")],
                            r#"{"ok":true}"#,
                        )
                            .into_response(),
                    }
                }
            },
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (url, seen)
    }

    fn app(daemon: DaemonEndpoint) -> axum::Router {
        axum::Router::new()
            .route("/api/capture/start", post(start))
            .route("/api/capture/stop", post(stop))
            .route(
                "/api/capture/activity-monitor",
                get(activity_monitor_status).post(activity_monitor_command),
            )
            .layer(Extension(Arc::new(daemon)))
    }

    async fn serve(app: axum::Router) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        url
    }

    /// The browser never holds the daemon token: the server reads it from the
    /// daemon's file and attaches it, and relays the daemon's reply as-is.
    #[tokio::test]
    async fn proxy_attaches_the_daemon_token_and_relays_replies() {
        const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        let (daemon_url, seen) = fake_daemon(TOKEN).await;
        let dir = tempfile::tempdir().unwrap();
        let token_file = dir.path().join("daemon-token");
        std::fs::write(&token_file, TOKEN).unwrap();
        let server = serve(app(DaemonEndpoint::new(daemon_url, Some(token_file)))).await;
        let browser = reqwest::Client::builder().no_proxy().build().unwrap();

        let start_body = r#"{"meeting_id":"m-1","started_at_ms":1.0}"#;
        let started = browser
            .post(format!("{server}/api/capture/start"))
            .header("content-type", "application/json")
            .body(start_body)
            .send()
            .await
            .unwrap();
        assert_eq!(started.status(), StatusCode::CONFLICT);
        assert_eq!(
            started.headers()["content-type"],
            "text/plain; charset=utf-8"
        );
        assert_eq!(started.text().await.unwrap(), "already recording");

        let stopped = browser
            .post(format!("{server}/api/capture/stop"))
            .send()
            .await
            .unwrap();
        assert_eq!(stopped.status(), StatusCode::OK);
        let monitor = browser
            .get(format!("{server}/api/capture/activity-monitor"))
            .send()
            .await
            .unwrap();
        assert_eq!(monitor.status(), StatusCode::OK);
        assert_eq!(monitor.text().await.unwrap(), r#"{"ok":true}"#);
        let command = br#"{"operation":"disable","epoch":"e"}"#;
        assert_eq!(
            browser
                .post(format!("{server}/api/capture/activity-monitor"))
                .header("content-type", "application/json")
                .body(command.to_vec())
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::OK
        );

        let bearer = format!("Bearer {TOKEN}");
        let seen = &seen.lock().unwrap().requests;
        let summary: Vec<_> = seen
            .iter()
            .map(|r| {
                (
                    r.method.as_str(),
                    r.path.as_str(),
                    r.authorization.as_deref(),
                )
            })
            .collect();
        assert_eq!(
            summary,
            [
                ("POST", "/capture/start", Some(bearer.as_str())),
                ("POST", "/capture/stop", Some(bearer.as_str())),
                ("GET", "/activity-monitor", Some(bearer.as_str())),
                ("POST", "/activity-monitor", Some(bearer.as_str())),
            ]
        );
        assert_eq!(seen[0].content_type.as_deref(), Some("application/json"));
        assert_eq!(seen[0].body, start_body.as_bytes());
        assert_eq!(seen[3].body, command);
    }

    #[tokio::test]
    async fn missing_daemon_or_token_is_reported_not_hidden() {
        let browser = reqwest::Client::builder().no_proxy().build().unwrap();
        // Nothing listens on port 1.
        let server = serve(app(DaemonEndpoint::new("http://127.0.0.1:1", None))).await;
        let down = browser
            .post(format!("{server}/api/capture/stop"))
            .send()
            .await
            .unwrap();
        assert_eq!(down.status(), StatusCode::SERVICE_UNAVAILABLE);

        // Without a token file the daemon refuses the server: a server-side
        // misconfiguration (502), never a 401 the browser could act on.
        let (daemon_url, _) = fake_daemon("x").await;
        let server = serve(app(DaemonEndpoint::new(daemon_url, None))).await;
        let refused = browser
            .post(format!("{server}/api/capture/stop"))
            .send()
            .await
            .unwrap();
        assert_eq!(refused.status(), StatusCode::BAD_GATEWAY);
        let body: serde_json::Value = refused.json().await.unwrap();
        assert!(body["error"].as_str().unwrap().contains("token"), "{body}");
    }
}

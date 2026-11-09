//! Terminal sessions over HTTP + WebSocket.
//!
//! - `GET    /api/terminal/sessions`        — live sessions
//! - `POST   /api/terminal/sessions`        — spawn a login shell (cwd confined to `$HOME`)
//! - `DELETE /api/terminal/sessions/{id}`   — kill it
//! - `GET    /api/terminal/ws?session=<id>` — attach: replay, then live bytes
//!
//! Frames on the socket: binary = raw bytes both ways; text = JSON control
//! messages from the client (`{"type":"resize","cols":n,"rows":n}`). Closing
//! the socket only detaches; the shell keeps running in
//! [`bruhs_core::terminal::TerminalManager`] until it exits or is deleted.
//!
//! WebSockets are not covered by CORS, so the upgrade checks `Origin`
//! against loopback origins (and `BRUHS_API_URL`) before attaching.

use std::sync::Arc;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, Query, State};
use axum::http::{header::ORIGIN, HeaderMap, StatusCode};
use axum::response::Response;
use axum::Json;
use bruhs_core::terminal::{TerminalError, TerminalInfo, TerminalManager, TerminalSpec};
use bruhs_core::AppState;
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast::{self, error::RecvError};

use crate::config::Config;
use crate::error::AppError;
use crate::routes::paths::HomePath;

/// True for syntactically valid loopback browser origins, plus the exact
/// configured origins (`BRUHS_API_URL`, `BRUHS_WEB_ORIGIN`).
pub fn origin_allowed(origin: Option<&str>, policy: &crate::local_access::Policy) -> bool {
    origin
        .and_then(|value| value.parse::<axum::http::HeaderValue>().ok())
        .is_some_and(|origin| policy.allows_origin(&origin))
}

#[derive(Debug, PartialEq, Eq)]
pub enum Control {
    Resize { cols: u16, rows: u16 },
}

#[derive(Deserialize)]
struct ControlWire {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    cols: Option<u16>,
    #[serde(default)]
    rows: Option<u16>,
}

/// Parse a client control frame; anything unknown is ignored (`None`).
pub fn parse_control(text: &str) -> Option<Control> {
    let wire: ControlWire = serde_json::from_str(text).ok()?;
    match wire.kind.as_str() {
        "resize" => {
            let cols = wire.cols.filter(|c| *c > 0)?;
            let rows = wire.rows.filter(|r| *r > 0)?;
            Some(Control::Resize { cols, rows })
        }
        _ => None,
    }
}

fn map_terminal_error(err: TerminalError) -> AppError {
    match err {
        TerminalError::NotFound(id) => AppError::NotFound(format!("no terminal session {id}")),
        TerminalError::MissingCwd(p) => AppError::BadRequest(format!("directory not found: {p}")),
        other => AppError::internal(other),
    }
}

#[derive(Serialize)]
pub struct SessionsResponse {
    pub sessions: Vec<TerminalInfo>,
}

pub async fn list_sessions(State(state): State<Arc<AppState>>) -> Json<SessionsResponse> {
    Json(SessionsResponse {
        sessions: state.terminals().list(),
    })
}

#[derive(Deserialize, Default)]
pub struct CreateSessionBody {
    /// Empty or missing = home directory.
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub project_name: Option<String>,
    #[serde(default)]
    pub cols: Option<u16>,
    #[serde(default)]
    pub rows: Option<u16>,
}

pub async fn create_session(
    State(state): State<Arc<AppState>>,
    Json(body): Json<CreateSessionBody>,
) -> Result<(StatusCode, Json<TerminalInfo>), AppError> {
    let cwd = match body.cwd.filter(|c| !c.trim().is_empty()) {
        Some(c) => Some(HomePath::resolve(&c).await?.into_path_buf()),
        None => None,
    };
    let info = state
        .terminals()
        .create(TerminalSpec {
            cwd,
            title: body.title,
            project_name: body.project_name,
            cols: body.cols,
            rows: body.rows,
        })
        .map_err(map_terminal_error)?;
    Ok((StatusCode::CREATED, Json(info)))
}

pub async fn delete_session(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<StatusCode, AppError> {
    if state.terminals().close(&id) {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(AppError::NotFound(format!("no terminal session {id}")))
    }
}

#[derive(Deserialize)]
pub struct AttachQuery {
    pub session: Option<String>,
}

pub async fn ws_terminal(
    ws: WebSocketUpgrade,
    headers: HeaderMap,
    Query(q): Query<AttachQuery>,
    State(state): State<Arc<AppState>>,
    State(config): State<Arc<Config>>,
) -> Result<Response, AppError> {
    let origin = headers.get(ORIGIN).and_then(|v| v.to_str().ok());
    if !origin_allowed(origin, &config.access) {
        return Err(AppError::Forbidden("origin not allowed".to_string()));
    }
    let id = q
        .session
        .filter(|s| !s.is_empty())
        .ok_or_else(|| AppError::BadRequest("session query parameter required".to_string()))?;
    if state.terminals().get(&id).is_none() {
        return Err(AppError::NotFound(format!("no terminal session {id}")));
    }
    let manager = state.terminals().clone();
    Ok(ws.on_upgrade(move |socket| run_attached(socket, manager, id)))
}

async fn run_attached(socket: WebSocket, manager: TerminalManager, id: String) {
    let (mut sink, mut stream) = socket.split();

    let attachment = match manager.attach(&id) {
        Ok(a) => a,
        Err(e) => {
            // Best effort: the session vanished between upgrade and attach;
            // if the client is gone too there is no one to tell.
            if let Err(error) = sink
                .send(Message::Text(format!("\r\n[{e}]\r\n").into()))
                .await
            {
                tracing::debug!("terminal socket closed before the error was sent: {error}");
            }
            if let Err(error) = sink.close().await {
                tracing::debug!("terminal socket close failed: {error}");
            }
            return;
        }
    };
    let output = attachment.output;
    if !attachment.replay.is_empty()
        && sink
            .send(Message::Binary(attachment.replay.into()))
            .await
            .is_err()
    {
        return;
    }

    // Session → socket. Ends when the shell exits (channel closed) or the
    // socket goes away.
    let resync_manager = manager.clone();
    let resync_id = id.clone();
    let send_task = tokio::spawn(async move {
        // The only attach error is "session gone": then the shell exited
        // and the stream ends, as it would on the closed channel.
        let resync = || {
            resync_manager
                .attach(&resync_id)
                .ok()
                .map(|attachment| (attachment.replay, attachment.output))
        };
        let frames = output_frames(output, resync);
        tokio::pin!(frames);
        while let Some(bytes) = frames.next().await {
            if sink.send(Message::Binary(bytes.into())).await.is_err() {
                break;
            }
        }
        if let Err(error) = sink.close().await {
            tracing::debug!("terminal socket close failed: {error}");
        }
    });

    // Socket → session.
    while let Some(Ok(msg)) = stream.next().await {
        match msg {
            Message::Binary(bytes) => {
                // Waits for room in the session's input queue (back-pressure
                // on this socket) without blocking a tokio worker.
                if manager.write_async(&id, &bytes).await.is_err() {
                    break;
                }
            }
            Message::Text(text) => {
                if let Some(Control::Resize { cols, rows }) = parse_control(&text) {
                    match manager.resize(&id, cols, rows) {
                        Ok(()) => {}
                        // The shell exited; the output side ends the socket.
                        Err(TerminalError::NotFound(_)) => break,
                        Err(error) => tracing::warn!("terminal {id}: resize failed: {error}"),
                    }
                }
            }
            Message::Close(_) => break,
            Message::Ping(_) | Message::Pong(_) => {}
        }
    }

    // Detach only: the shell keeps running for the next client.
    send_task.abort();
}

/// Clears the screen and scrollback before a replay, so a client that
/// missed output redraws from the snapshot instead of mixing the two.
const RESET_TERMINAL: &[u8] = b"\x1bc";

/// The frames to send for `output`, until the shell exits. A client too
/// slow to keep up (the channel lagged and dropped bytes) gets the screen
/// reset and the session's scrollback snapshot from `resync` (a fresh
/// attach), then live output again, instead of silently missing bytes
/// mid-escape-sequence.
fn output_frames<R>(
    mut output: broadcast::Receiver<Arc<[u8]>>,
    resync: R,
) -> impl futures_core::Stream<Item = Vec<u8>>
where
    R: Fn() -> Option<(Vec<u8>, broadcast::Receiver<Arc<[u8]>>)>,
{
    async_stream::stream! {
        loop {
            match output.recv().await {
                Ok(chunk) => yield chunk.to_vec(),
                Err(RecvError::Lagged(skipped)) => {
                    tracing::debug!("terminal client lagged by {skipped} chunks; resending scrollback");
                    let Some((replay, fresh)) = resync() else {
                        break;
                    };
                    output = fresh;
                    yield [RESET_TERMINAL, &replay].concat();
                }
                Err(RecvError::Closed) => break,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A lagged client used to skip the lost bytes and carry on with a
    /// corrupted screen; it now gets a reset plus the scrollback snapshot,
    /// then live output from the fresh subscription.
    #[tokio::test]
    async fn a_lagged_client_gets_the_scrollback_again() {
        let (tx, lagging) = broadcast::channel::<Arc<[u8]>>(2);
        for chunk in [&b"a"[..], b"b", b"c", b"d"] {
            tx.send(Arc::from(chunk)).unwrap();
        }
        let fresh = tx.subscribe();
        let resynced = std::sync::Mutex::new(Some(fresh));
        let resync = || {
            resynced
                .lock()
                .unwrap()
                .take()
                .map(|output| (b"SCREEN".to_vec(), output))
        };
        let frames = output_frames(lagging, resync);
        tokio::pin!(frames);
        assert_eq!(frames.next().await.unwrap(), b"\x1bcSCREEN");
        // Live output continues on the fresh subscription.
        tx.send(Arc::from(&b"e"[..])).unwrap();
        assert_eq!(frames.next().await.unwrap(), b"e");
        drop(tx);
        assert!(frames.next().await.is_none(), "ends with the shell");
    }

    #[test]
    fn origin_allowlist() {
        use crate::local_access::Policy;
        let local = || Policy::from_config(None, None);
        // A policy that failed to load allows nothing.
        let allowed = |origin: Option<&str>, policy: Result<Policy, &'static str>| {
            policy.is_ok_and(|policy| origin_allowed(origin, &policy))
        };
        assert!(allowed(Some("http://127.0.0.1:8081"), local()));
        assert!(allowed(Some("http://localhost:3001"), local()));
        assert!(allowed(Some("http://[::1]:8080"), local()));
        // Dev `*.localhost` names need explicit configuration.
        assert!(!allowed(Some("http://bruhs.localhost"), local()));
        assert!(allowed(
            Some("https://bruhs.localhost"),
            Policy::from_config(None, Some("https://bruhs.localhost"))
        ));
        assert!(!allowed(Some("https://evil.example"), local()));
        assert!(!allowed(Some("http://localhost.evil.example"), local()));
        assert!(!allowed(None, local()));
        assert!(!allowed(Some(""), local()));
        assert!(allowed(
            Some("https://api.bruhs.dev"),
            Policy::from_config(Some("https://api.bruhs.dev/"), None)
        ));
        assert!(!allowed(Some("https://api.bruhs.dev"), local()));
        assert!(!allowed(
            Some("http://localhost:3001"),
            Policy::from_config(Some("not an origin"), None)
        ));
        for origin in [
            "http://localhost/path",
            "http://localhost#fragment",
            "http://localhost:99999",
            "http://foreign@localhost",
        ] {
            assert!(!allowed(Some(origin), local()), "{origin}");
        }
    }

    #[test]
    fn parse_resize_control() {
        assert_eq!(
            parse_control(r#"{"type":"resize","cols":120,"rows":40}"#),
            Some(Control::Resize {
                cols: 120,
                rows: 40
            })
        );
        assert_eq!(
            parse_control(r#"{"type":"resize","cols":0,"rows":40}"#),
            None
        );
        assert_eq!(parse_control(r#"{"type":"other"}"#), None);
        assert_eq!(parse_control("garbage"), None);
    }

    #[test]
    fn terminal_errors_map_to_http_status() {
        assert!(matches!(
            map_terminal_error(TerminalError::NotFound("x".into())),
            AppError::NotFound(_)
        ));
        assert!(matches!(
            map_terminal_error(TerminalError::MissingCwd("/nope".into())),
            AppError::BadRequest(_)
        ));
        assert!(matches!(
            map_terminal_error(TerminalError::Pty(std::io::Error::other("boom").into())),
            AppError::Internal { .. }
        ));
    }

    #[test]
    fn create_body_defaults_are_all_optional() {
        let body: CreateSessionBody = serde_json::from_str("{}").unwrap();
        assert!(body.cwd.is_none() && body.cols.is_none() && body.project_name.is_none());
        let body: CreateSessionBody =
            serde_json::from_str(r#"{"cwd":"/tmp","cols":80,"rows":24,"project_name":"p"}"#)
                .unwrap();
        assert_eq!(body.cwd.as_deref(), Some("/tmp"));
        assert_eq!((body.cols, body.rows), (Some(80), Some(24)));
    }
}

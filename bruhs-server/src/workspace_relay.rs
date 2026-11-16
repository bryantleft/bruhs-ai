//! The web side of the MCP workspace connection
//! (`bruhs_models::workspace_bridge::relay`, `docs/local-mcp-workspace.md`).
//!
//! The web keeps its records in the tab's IndexedDB, so this server cannot
//! answer a tool call itself. It relays: `bruhs-mcp` calls [`CALL_PATH`] with
//! the session token, the request travels to the tab that enabled the session
//! over that tab's live events stream, and the tab answers at [`REPLY_PATH`]
//! after checking its own grants. The relay never reads, grants, approves or
//! executes anything.
//!
//! One session exists at a time, bound to one stream. It ends when the tab
//! disables it, when another tab enables one, when the bound stream closes
//! (the tab closed, reloaded or was hidden), and when the server stops. Its
//! private connection file goes with it. Nothing here logs a token, a stream
//! ID or a request body. Like the native connection, it is Unix-only: the
//! connection file's privacy relies on Unix permissions.

use crate::error::AppError;
use axum::{
    body::Bytes,
    extract::rejection::JsonRejection,
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Extension, Json,
};
use bruhs_models::workspace_bridge::{
    relay::{
        DisableSession, EnableSession, RelayFrame, RelayReply, RevokeReason, SessionEnabled,
        MAX_PENDING_REQUESTS, REPLY_TIMEOUT_MS,
    },
    BridgeRequest, BridgeResponse, MAX_RESPONSE_BYTES,
};
use futures_core::Stream;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex, MutexGuard, PoisonError},
    time::Duration,
};
use tokio::sync::{mpsc, oneshot};

pub use bruhs_models::workspace_bridge::relay::{CALL_PATH, REPLY_PATH, SESSION_PATH};

/// Frames a tab's stream may have queued; requests are capped well below it.
const STREAM_BUFFER: usize = MAX_PENDING_REQUESTS * 2;
/// The reply route's body limit: the largest response plus its envelope.
pub const MAX_REPLY_BODY_BYTES: usize = MAX_RESPONSE_BYTES + 64 * 1024;

const NOT_AUTHORIZED: &str = "Workspace connection is not authorized.";
const BUSY: &str = "Workspace is busy or disconnected.";

#[derive(Clone)]
pub struct WorkspaceRelay {
    inner: Arc<Mutex<State>>,
    /// This server's loopback origin, written into connection files.
    origin: Arc<str>,
}

#[derive(Default)]
struct State {
    /// Open live streams by their secret ID.
    streams: HashMap<String, mpsc::Sender<RelayFrame>>,
    session: Option<Session>,
}

struct Session {
    id: String,
    stream_id: String,
    token: String,
    /// Removed from disk when the session ends.
    _file: bruhs_core::workspace_bridge::RelayConnectionFile,
    /// Callers waiting for the tab, by request ID. Dropping a sender fails
    /// that call as disconnected; nothing is ever reported done.
    pending: HashMap<String, oneshot::Sender<BridgeResponse>>,
}

/// Why a session could not start.
#[derive(Debug, thiserror::Error)]
pub enum EnableError {
    #[error("This tab's live connection to bruhs-server is not open. Wait for it to reconnect, then try again.")]
    StreamNotOpen,
    #[error("The local connection is unavailable. Access is disabled; you can retry.")]
    Unavailable(#[source] std::io::Error),
}

/// What became of one relayed call.
#[derive(Debug, PartialEq)]
pub enum CallOutcome {
    /// The tab answered (with records or its own refusal).
    Answered(BridgeResponse),
    /// No session, or a token that is not this session's.
    Unauthorized,
    /// The session's queue is full or its tab cannot take the request.
    Busy,
    /// The session ended or the tab went away after taking the request.
    Disconnected,
    /// The tab did not answer in time. What it did is unknown.
    TimedOut,
}

/// Holds a stream's place in the relay; dropping it (the stream closed)
/// detaches the stream and ends a session bound to it.
struct Attachment {
    relay: WorkspaceRelay,
    stream_id: String,
}

impl Drop for Attachment {
    fn drop(&mut self) {
        self.relay.detach(&self.stream_id);
    }
}

impl WorkspaceRelay {
    /// `origin` is this server's loopback origin, such as `http://127.0.0.1:3001`.
    pub fn new(origin: impl Into<Arc<str>>) -> Self {
        Self {
            inner: Arc::default(),
            origin: origin.into(),
        }
    }

    fn state(&self) -> MutexGuard<'_, State> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// A new live stream's relay frames: [`RelayFrame::Hello`] first, then
    /// whatever a session bound to it receives. `None` if no secret could be
    /// drawn; the stream then simply carries no relay frames.
    pub fn attach(&self) -> Option<impl Stream<Item = RelayFrame> + Send + 'static> {
        let stream_id = bruhs_core::workspace_bridge::new_secret().ok()?;
        let (sender, mut receiver) = mpsc::channel(STREAM_BUFFER);
        sender
            .try_send(RelayFrame::Hello {
                stream_id: stream_id.clone(),
            })
            .ok()?;
        self.state().streams.insert(stream_id.clone(), sender);
        let attachment = Attachment {
            relay: self.clone(),
            stream_id,
        };
        Some(async_stream::stream! {
            let _attachment = attachment;
            while let Some(frame) = receiver.recv().await {
                yield frame;
            }
        })
    }

    fn detach(&self, stream_id: &str) {
        let mut state = self.state();
        state.streams.remove(stream_id);
        let bound = state
            .session
            .as_ref()
            .is_some_and(|session| session.stream_id == stream_id);
        let ended = bound.then(|| state.session.take());
        drop(state);
        if ended.is_some() {
            tracing::info!("MCP workspace session ended: its tab's live stream closed");
        }
    }

    /// Bind a new session to the stream `stream_id`, ending any other one.
    pub fn enable(&self, stream_id: &str) -> Result<SessionEnabled, EnableError> {
        let mut state = self.state();
        let Some(bound) = state
            .streams
            .keys()
            .find(|id| bruhs_core::workspace_bridge::same_secret(stream_id, id))
            .cloned()
        else {
            return Err(EnableError::StreamNotOpen);
        };
        let token = bruhs_core::workspace_bridge::new_secret().map_err(EnableError::Unavailable)?;
        let file = bruhs_core::workspace_bridge::RelayConnectionFile::create(&self.origin, &token)
            .map_err(EnableError::Unavailable)?;
        let connection_path = file.path().to_string_lossy().into_owned();
        let previous = state.session.take();
        let replaced = previous.is_some();
        if let Some(previous) = previous {
            if let Some(stream) = state.streams.get(&previous.stream_id) {
                // A full or closed stream ends that tab's session all the same.
                let _ = stream.try_send(RelayFrame::Revoked {
                    session_id: previous.id.clone(),
                    reason: RevokeReason::Replaced,
                });
            }
        }
        let id = uuid::Uuid::new_v4().to_string();
        state.session = Some(Session {
            id: id.clone(),
            stream_id: bound,
            token,
            _file: file,
            pending: HashMap::new(),
        });
        drop(state);
        tracing::info!(replaced, "MCP workspace session enabled for a web tab");
        Ok(SessionEnabled {
            session_id: id,
            connection_path,
            replaced,
        })
    }

    /// End the session if it is `session_id` and bound to `stream_id`.
    pub fn disable(&self, stream_id: &str, session_id: &str) -> bool {
        let mut state = self.state();
        let owned = state.session.as_ref().is_some_and(|session| {
            session.id == session_id
                && bruhs_core::workspace_bridge::same_secret(stream_id, &session.stream_id)
        });
        let ended = owned.then(|| state.session.take()).flatten();
        drop(state);
        if ended.is_some() {
            tracing::info!("MCP workspace session disabled by its tab");
        }
        ended.is_some()
    }

    /// Relay one request to the session's tab and wait for its answer.
    pub async fn call(&self, token: &str, request: BridgeRequest) -> CallOutcome {
        let (answer, session_id, request_id) = {
            let mut state = self.state();
            let State { streams, session } = &mut *state;
            let Some(session) = session
                .as_mut()
                .filter(|session| bruhs_core::workspace_bridge::same_secret(token, &session.token))
            else {
                return CallOutcome::Unauthorized;
            };
            if session.pending.len() >= MAX_PENDING_REQUESTS {
                return CallOutcome::Busy;
            }
            let Some(stream) = streams.get(&session.stream_id) else {
                return CallOutcome::Busy;
            };
            let request_id = uuid::Uuid::new_v4().to_string();
            let frame = RelayFrame::Request {
                session_id: session.id.clone(),
                request_id: request_id.clone(),
                request,
            };
            if stream.try_send(frame).is_err() {
                return CallOutcome::Busy;
            }
            let (sender, answer) = oneshot::channel();
            session.pending.insert(request_id.clone(), sender);
            let session_id = session.id.clone();
            drop(state);
            (answer, session_id, request_id)
        };
        match tokio::time::timeout(Duration::from_millis(REPLY_TIMEOUT_MS), answer).await {
            Ok(Ok(response)) => CallOutcome::Answered(response),
            Ok(Err(_)) => CallOutcome::Disconnected,
            Err(_) => {
                if let Some(session) = self
                    .state()
                    .session
                    .as_mut()
                    .filter(|session| session.id == session_id)
                {
                    session.pending.remove(&request_id);
                }
                CallOutcome::TimedOut
            }
        }
    }

    /// Deliver the tab's answer. `false` when the session, stream or request
    /// does not match (a late answer to a request that already timed out).
    pub fn reply(&self, reply: RelayReply) -> bool {
        let mut state = self.state();
        let Some(session) = state.session.as_mut().filter(|session| {
            session.id == reply.session_id
                && bruhs_core::workspace_bridge::same_secret(&reply.stream_id, &session.stream_id)
        }) else {
            return false;
        };
        let Some(sender) = session.pending.remove(&reply.request_id) else {
            return false;
        };
        drop(state);
        sender.send(reply.response).is_ok()
    }

    #[cfg(test)]
    pub fn session_active(&self) -> bool {
        self.state().session.is_some()
    }
}

fn relay(value: Option<Extension<WorkspaceRelay>>) -> Result<WorkspaceRelay, AppError> {
    value.map(|Extension(relay)| relay).ok_or_else(|| {
        AppError::Unavailable("The MCP workspace relay is not running in this server.".into())
    })
}

/// `POST /api/workspace-bridge/session`: enable a session for the caller's
/// stream. The reply names the connection file, never the token.
pub async fn enable_session(
    value: Option<Extension<WorkspaceRelay>>,
    body: Result<Json<EnableSession>, JsonRejection>,
) -> Result<Json<SessionEnabled>, AppError> {
    let Json(body) = body.map_err(|rejection| AppError::BadRequest(rejection.body_text()))?;
    relay(value)?
        .enable(&body.stream_id)
        .map(Json)
        .map_err(|error| match error {
            EnableError::StreamNotOpen => AppError::Conflict(error.to_string()),
            EnableError::Unavailable(_) => AppError::internal(error),
        })
}

/// `DELETE /api/workspace-bridge/session`. Succeeds when nothing was left to end.
pub async fn disable_session(
    value: Option<Extension<WorkspaceRelay>>,
    body: Result<Json<DisableSession>, JsonRejection>,
) -> Result<Json<serde_json::Value>, AppError> {
    let Json(body) = body.map_err(|rejection| AppError::BadRequest(rejection.body_text()))?;
    let revoked = relay(value)?.disable(&body.stream_id, &body.session_id);
    Ok(Json(serde_json::json!({ "revoked": revoked })))
}

/// `POST /api/workspace-bridge/reply`: the tab's answer to one request.
pub async fn post_reply(
    value: Option<Extension<WorkspaceRelay>>,
    body: Result<Json<RelayReply>, JsonRejection>,
) -> Result<Json<serde_json::Value>, AppError> {
    let Json(body) = body.map_err(|rejection| AppError::BadRequest(rejection.body_text()))?;
    if relay(value)?.reply(body) {
        Ok(Json(serde_json::json!({ "delivered": true })))
    } else {
        Err(AppError::NotFound(
            "That request is no longer waiting for an answer.".into(),
        ))
    }
}

/// `POST /api/workspace-bridge/call`: `bruhs-mcp`'s request, authorized by
/// the session token. Browsers cannot use it: any request carrying `Origin`
/// or a `Sec-Fetch-*` header is refused before the token is checked.
pub async fn post_call(
    value: Option<Extension<WorkspaceRelay>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let browser = headers.contains_key(header::ORIGIN)
        || headers
            .keys()
            .any(|name| name.as_str().starts_with("sec-fetch-"));
    if browser {
        return (
            StatusCode::FORBIDDEN,
            "The workspace relay accepts only a local MCP client.",
        )
            .into_response();
    }
    let Ok(relay) = relay(value) else {
        return refusal(StatusCode::SERVICE_UNAVAILABLE, BUSY);
    };
    let token = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .unwrap_or_default();
    if token.is_empty() {
        return refusal(StatusCode::UNAUTHORIZED, NOT_AUTHORIZED);
    }
    // The token is checked before the body is decoded, so an unauthorized
    // caller learns nothing about what the relay would accept.
    let outcome = match serde_json::from_slice::<BridgeRequest>(&body) {
        Ok(request) => relay.call(token, request).await,
        Err(_) if !relay.authorized(token) => CallOutcome::Unauthorized,
        Err(_) => return (StatusCode::BAD_REQUEST, "Invalid workspace request.").into_response(),
    };
    match outcome {
        CallOutcome::Answered(response) => match serde_json::to_vec(&response) {
            Ok(bytes) if bytes.len() <= MAX_RESPONSE_BYTES => (
                StatusCode::OK,
                [(header::CONTENT_TYPE, "application/json")],
                bytes,
            )
                .into_response(),
            _ => (
                StatusCode::BAD_GATEWAY,
                "The workspace response exceeds its limit.",
            )
                .into_response(),
        },
        CallOutcome::Unauthorized => refusal(StatusCode::UNAUTHORIZED, NOT_AUTHORIZED),
        CallOutcome::Busy => refusal(StatusCode::SERVICE_UNAVAILABLE, BUSY),
        // Not a workspace response: the client reports that nothing is known.
        CallOutcome::Disconnected => {
            (StatusCode::BAD_GATEWAY, "The workspace tab disconnected.").into_response()
        }
        CallOutcome::TimedOut => (
            StatusCode::GATEWAY_TIMEOUT,
            "The workspace tab did not answer in time.",
        )
            .into_response(),
    }
}

/// A relay refusal in the workspace response shape, so the client shows the
/// same message the native socket gives.
fn refusal(status: StatusCode, message: &str) -> Response {
    (status, Json(BridgeResponse::Error(message.into()))).into_response()
}

impl WorkspaceRelay {
    fn authorized(&self, token: &str) -> bool {
        self.state()
            .session
            .as_ref()
            .is_some_and(|session| bruhs_core::workspace_bridge::same_secret(token, &session.token))
    }
}

#[cfg(test)]
mod tests;

//! A bruh's Computer over HTTP and WebSocket.
//!
//! - `GET|POST /api/reactive/agents/{id}/computer`: the owner's Computer
//!   tab reads the state and sends [`ComputerRequest`]s.
//! - `POST /api/reactive/agents/{id}/computer/command`: one browser step
//!   from the bruh's own helper, run only if the gate let it through.
//! - `GET /api/local-tools/browser/{session}/screencast`: the live view.
//!
//! The owner routes and the live view sit behind the shared loopback and
//! origin guard. A Full bruh's sandboxed shell is denied loopback, so its
//! run can't take its own computer, read its session id or open the live
//! view. The command route answers only the helper: a request with an
//! `Origin` comes from a web page and is refused, and each step must use up
//! the one-shot allowance the gate left for it.

use std::{sync::Arc, time::Duration};

use axum::{
    body::Bytes,
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Path, State,
    },
    http::{header::ORIGIN, HeaderMap},
    response::Response,
    Extension, Json,
};
use bruhs_core::local_tools::{ComputerWatch, LocalToolsHost};
use bruhs_core::reactive::ReactiveRuntime;
use bruhs_models::crew::{
    LIVE_VIEW_ENDED, LIVE_VIEW_FOREIGN_PAGE, LIVE_VIEW_STALLED, LIVE_VIEW_UNREADABLE,
    STEP_FROM_A_PAGE, STEP_NOT_APPROVED, UNKNOWN_BROWSER_ACTION,
};
use bruhs_models::local_tools::{
    BruhBrowserCommand, ComputerInput, ComputerRequest, ComputerState, ScreencastClientMessage,
    ScreencastMessage, ToolReply, FRAME_ACK,
};
use futures_util::SinkExt;
use serde_json::Value;
use tokio::{sync::mpsc, time::Instant};

use super::local_tools::error;
use crate::config::Config;
use crate::error::AppError;

/// The largest message a live view may send: input and acknowledgements
/// are small.
const MAX_MESSAGE_BYTES: usize = 64 << 10;
/// Owner inputs waiting for the page, and refusals waiting for the socket.
const INPUT_QUEUE: usize = 64;

#[cfg(test)]
mod tests;

/// `GET /api/reactive/agents/{id}/computer`.
pub async fn state(
    Extension(host): Extension<Arc<LocalToolsHost>>,
    Path(id): Path<String>,
) -> Result<Json<ComputerState>, AppError> {
    host.computer_state(&id).await.map(Json).map_err(error)
}

/// `POST /api/reactive/agents/{id}/computer`: open, close, take control,
/// hand back or open in a window.
pub async fn request(
    Extension(host): Extension<Arc<LocalToolsHost>>,
    Path(id): Path<String>,
    Json(request): Json<ComputerRequest>,
) -> Result<Json<ComputerState>, AppError> {
    host.computer_request(&id, request)
        .await
        .map(Json)
        .map_err(error)
}

/// `POST /api/reactive/agents/{id}/computer/command`: one step from the
/// bruh's helper, with a [`BruhBrowserCommand`] body.
pub async fn command(
    runtime: Option<Extension<ReactiveRuntime>>,
    Extension(host): Extension<Arc<LocalToolsHost>>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Result<Json<ToolReply>, AppError> {
    if headers.contains_key(ORIGIN) {
        return Err(AppError::Forbidden(STEP_FROM_A_PAGE.into()));
    }
    let runtime = super::reactive::runtime(runtime)?;
    // Used up first, matched against the step exactly as sent (parsing
    // would turn 100 into 100.0), so a step refused below can't be sent
    // again without the gate.
    let sent = body.get("command").cloned().unwrap_or(Value::Null);
    let allowance = runtime.consume_browser_allowance(&id, &sent).await?;
    let action = serde_json::from_value::<BruhBrowserCommand>(body)
        .map_err(|_| AppError::BadRequest(UNKNOWN_BROWSER_ACTION.into()))?
        .command;
    // The host's own refusals say why a step can't run, approved or not.
    host.check_bruh_command(&id, &action).map_err(error)?;
    let allowance = allowance.ok_or_else(|| AppError::Forbidden(STEP_NOT_APPROVED.into()))?;
    host.bruh_command_on(&id, action, allowance.page_url.as_deref())
        .await
        .map(Json)
        .map_err(error)
}

/// `GET /api/local-tools/browser/{session}/screencast`: the live view of
/// the bruh browser `session` names.
pub async fn screencast(
    upgrade: WebSocketUpgrade,
    headers: HeaderMap,
    Path(session): Path<String>,
    State(config): State<Arc<Config>>,
    Extension(host): Extension<Arc<LocalToolsHost>>,
) -> Result<Response, AppError> {
    // WebSockets skip CORS, so a page's Origin is checked here as well as
    // in the guard. The desktop's native client sends none.
    if let Some(origin) = headers.get(ORIGIN) {
        let origin = origin.to_str().ok();
        if !super::terminal::origin_allowed(origin, &config.access) {
            return Err(AppError::Forbidden(LIVE_VIEW_FOREIGN_PAGE.into()));
        }
    }
    // A refusal (an unknown or closed browser, four viewers) still opens
    // the socket, to say why: a browser can't read a refused upgrade.
    let watch = host.watch_computer(&session);
    let upgrade = upgrade.max_message_size(MAX_MESSAGE_BYTES);
    Ok(match watch {
        Ok(watch) => upgrade.on_upgrade(move |socket| live_view(host, watch, socket)),
        Err(refusal) => {
            let deadline = host.command_timeout();
            let error = refusal.to_string();
            upgrade.on_upgrade(move |socket| refuse(socket, error, deadline))
        }
    })
}

/// Say why the live view can't open, then close.
async fn refuse(mut socket: WebSocket, error: String, deadline: Duration) {
    let _ = send(&mut socket, &ScreencastMessage::Error { error }, deadline).await;
    let _ = tokio::time::timeout(deadline, socket.close()).await;
}

/// The state on connect and on every change, frames one at a time (the
/// next only after [`FRAME_ACK`]), and owner input passed to the page.
async fn live_view(host: Arc<LocalToolsHost>, watch: ComputerWatch, mut socket: WebSocket) {
    let deadline = host.command_timeout();
    let ComputerWatch {
        agent_id,
        mut viewer,
        mut state,
    } = watch;
    let (inputs, mut refusals) = owner_inputs(Arc::clone(&host), agent_id);
    let ended = async {
        let mut shown = state.borrow_and_update().clone();
        send(
            &mut socket,
            &ScreencastMessage::State(shown.clone()),
            deadline,
        )
        .await?;
        // When the frame in flight must be acknowledged by.
        let mut unacked: Option<Instant> = None;
        loop {
            let ack_by = unacked.unwrap_or_else(Instant::now);
            tokio::select! {
                changed = state.changed() => {
                    changed.map_err(|_| LIVE_VIEW_ENDED.to_owned())?;
                    let now = state.borrow_and_update().clone();
                    if now != shown {
                        shown = now;
                        send(&mut socket, &ScreencastMessage::State(shown.clone()), deadline).await?;
                    }
                }
                frame = viewer.next_frame(), if unacked.is_none() => {
                    let frame = frame.ok_or_else(|| LIVE_VIEW_ENDED.to_owned())?;
                    let frame = Message::Binary(Bytes::from_owner(frame));
                    timed(deadline, socket.send(frame)).await?;
                    unacked = Some(Instant::now() + deadline);
                }
                () = tokio::time::sleep_until(ack_by), if unacked.is_some() => {
                    return Err(LIVE_VIEW_STALLED.to_owned());
                }
                Some(refusal) = refusals.recv() => {
                    send(&mut socket, &ScreencastMessage::Error { error: refusal }, deadline).await?;
                }
                message = socket.recv() => match message {
                    None | Some(Err(_) | Ok(Message::Close(_))) => return Ok(()),
                    Some(Ok(Message::Ping(_) | Message::Pong(_))) => {}
                    Some(Ok(Message::Text(text))) if text.as_str() == FRAME_ACK => {
                        if unacked.take().is_some() {
                            viewer.ack();
                        }
                    }
                    Some(Ok(Message::Text(text))) => {
                        match serde_json::from_str::<ScreencastClientMessage>(&text) {
                            Ok(ScreencastClientMessage::Input { input }) => {
                                if inputs.send(input).await.is_err() {
                                    return Ok(());
                                }
                            }
                            Err(_) => unreadable(&mut socket, deadline).await?,
                        }
                    }
                    Some(Ok(Message::Binary(_))) => unreadable(&mut socket, deadline).await?,
                },
            }
        }
    };
    let ended: Result<(), String> = ended.await;
    if let Err(error) = ended {
        let _ = send(&mut socket, &ScreencastMessage::Error { error }, deadline).await;
    }
    let _ = tokio::time::timeout(deadline, socket.close()).await;
}

/// Owner input goes to the page in order, off the socket's loop, so a slow
/// page doesn't hold back frames. Refusals come back to be answered.
fn owner_inputs(
    host: Arc<LocalToolsHost>,
    agent_id: String,
) -> (mpsc::Sender<ComputerInput>, mpsc::Receiver<String>) {
    let (inputs, mut queue) = mpsc::channel::<ComputerInput>(INPUT_QUEUE);
    let (refusals, refused) = mpsc::channel(INPUT_QUEUE);
    tokio::spawn(async move {
        while let Some(input) = queue.recv().await {
            if let Err(refusal) = host.owner_input(&agent_id, input).await {
                if refusals.try_send(refusal.to_string()).is_err() && refusals.is_closed() {
                    return;
                }
            }
        }
    });
    (inputs, refused)
}

async fn unreadable(socket: &mut WebSocket, deadline: Duration) -> Result<(), String> {
    let error = LIVE_VIEW_UNREADABLE.to_owned();
    send(socket, &ScreencastMessage::Error { error }, deadline).await
}

async fn send(
    socket: &mut WebSocket,
    message: &ScreencastMessage,
    deadline: Duration,
) -> Result<(), String> {
    let text = serde_json::to_string(message).map_err(|error| error.to_string())?;
    timed(deadline, socket.send(Message::Text(text.into()))).await
}

/// A send that gives up on a viewer that stopped reading.
async fn timed(
    deadline: Duration,
    sent: impl std::future::Future<Output = Result<(), axum::Error>>,
) -> Result<(), String> {
    tokio::time::timeout(deadline, sent)
        .await
        .map_err(|_| LIVE_VIEW_STALLED.to_owned())?
        .map_err(|error| error.to_string())
}

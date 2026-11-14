use crate::error::AppError;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::response::IntoResponse;
use axum::{
    body::Bytes,
    extract::{Extension, Path, Query},
    http::HeaderMap,
    Json,
};
use bruhs_core::local_tools::{LocalToolsHost, ToolError};
use bruhs_models::local_tools::*;
use futures_util::SinkExt;
use serde::Deserialize;
use std::sync::Arc;

pub async fn ui() -> axum::response::Html<&'static str> {
    axum::response::Html(include_str!("local_tools_ui.html"))
}

pub async fn snapshot(
    Extension(host): Extension<Arc<LocalToolsHost>>,
) -> Result<Json<LocalToolsSnapshot>, AppError> {
    host.snapshot().await.map(Json).map_err(error)
}

/// This permission endpoint is intentionally absent from the agent tool catalogue.
pub async fn grant(
    Extension(host): Extension<Arc<LocalToolsHost>>,
    Json(request): Json<GrantRequest>,
) -> Result<Json<ToolGrant>, AppError> {
    host.grant(request).map(Json).map_err(error)
}

pub async fn revoke(
    Extension(host): Extension<Arc<LocalToolsHost>>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    host.revoke(&id).await.map_err(error)?;
    Ok(Json(serde_json::json!({"revoked":true})))
}

pub async fn browser_start(
    Extension(host): Extension<Arc<LocalToolsHost>>,
    Json(request): Json<BrowserStart>,
) -> Result<Json<BrowserSession>, AppError> {
    host.start_browser(request).await.map(Json).map_err(error)
}

pub async fn browser_command(
    Extension(host): Extension<Arc<LocalToolsHost>>,
    Json(request): Json<BrowserCommand>,
) -> Result<Json<ToolReply>, AppError> {
    host.browser_command(request).await.map(Json).map_err(error)
}

#[derive(Deserialize)]
pub struct DeviceQuery {
    pub grant_id: String,
}

pub async fn devices(
    Extension(host): Extension<Arc<LocalToolsHost>>,
    Query(query): Query<DeviceQuery>,
) -> Result<Json<DeviceInventory>, AppError> {
    host.devices(&query.grant_id).await.map(Json).map_err(error)
}

pub async fn device_command(
    Extension(host): Extension<Arc<LocalToolsHost>>,
    Json(request): Json<DeviceCommand>,
) -> Result<Json<ToolReply>, AppError> {
    host.device_command(request).await.map(Json).map_err(error)
}

pub async fn device_video(
    Extension(host): Extension<Arc<LocalToolsHost>>,
    upgrade: WebSocketUpgrade,
) -> impl IntoResponse {
    // The capability is sent inside the socket, never in a URL/access log.
    upgrade
        .max_message_size(2 << 20)
        .on_upgrade(move |socket| video_socket(host, socket))
}

async fn video_socket(host: Arc<LocalToolsHost>, mut socket: WebSocket) {
    let result = async {
        let first = tokio::time::timeout(host.command_timeout(), video_message(&mut socket)).await
            .map_err(|_| "Send a device video request before the local tool deadline.".to_owned())?
            .ok_or_else(|| "Device video socket closed before its request.".to_owned())?
            .map_err(|error| error.to_string())?;
        let Message::Text(text) = first else { return Err("Send the device video request as JSON text.".to_owned()); };
        let request: DeviceVideoStart = serde_json::from_str(&text).map_err(|error| format!("Invalid device video request: {error}"))?;
        let mut revoked = host.require(&request.grant_id, ToolScope::Devices).map_err(|error| error.to_string())?;
        let mut video = tokio::select! {
            biased;
            _ = revoked.changed() => return Err("Device access was revoked.".to_owned()),
            message = video_message(&mut socket) => { let _ = message; return Ok(()); },
            video = host.start_device_video(request) => video.map_err(|error| error.to_string())?,
        };
        loop {
            let frame = tokio::select! {
                biased;
                _ = revoked.changed() => return Err("Device access was revoked.".to_owned()),
                message = video_message(&mut socket) => { let _ = message; return Ok(()); },
                frame = video.next_frame() => frame.map_err(|error| error.to_string())?,
            };
            let Some(frame) = frame else { return Err("Device recorder ended. Start video again to resume.".to_owned()); };
            tokio::select! {
                biased;
                _ = revoked.changed() => return Err("Device access was revoked.".to_owned()),
                sent = tokio::time::timeout(host.command_timeout(), socket.send(Message::Binary(frame.into()))) => {
                    sent.map_err(|_| "Device video client stopped receiving frames; capture was stopped.".to_owned())?.map_err(|error| error.to_string())?;
                }
            }
            // One in-flight frame bounds the browser's receive queue as well as
            // the server. The UI acknowledges only after the image decodes.
            let acknowledged = tokio::select! {
                biased;
                _ = revoked.changed() => return Err("Device access was revoked.".to_owned()),
                received = tokio::time::timeout(host.command_timeout(), async {
                    loop {
                        match video_message(&mut socket).await {
                            Some(Ok(Message::Text(text))) if text == "frame_received" => return Ok(true),
                            Some(Ok(Message::Ping(_) | Message::Pong(_))) => {},
                            None | Some(Ok(Message::Close(_))) => return Ok(false),
                            _ => return Err("Device video expected a frame acknowledgement.".to_owned()),
                        }
                    }
                }) => received.map_err(|_|"Device video client did not display its frame before the tool deadline; capture was stopped.".to_owned())??,
            };
            if !acknowledged { return Ok(()); }
        }
    }.await;
    if let Err(error) = result {
        let _ = tokio::time::timeout(
            host.command_timeout(),
            socket.send(Message::Text(
                serde_json::json!({"error":error}).to_string().into(),
            )),
        )
        .await;
    }
    let _ = socket.close().await;
}

async fn video_message(socket: &mut WebSocket) -> Option<Result<Message, axum::Error>> {
    loop {
        let message = socket.recv().await;
        if matches!(message, Some(Ok(Message::Ping(_) | Message::Pong(_)))) {
            continue;
        }
        return message;
    }
}

pub async fn dictation(
    Extension(host): Extension<Arc<LocalToolsHost>>,
    Extension(daemon): Extension<Arc<bruhs_core::capture_client::DaemonEndpoint>>,
    Query(options): Query<DictationOptions>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<DictationTranscript>, AppError> {
    let id = headers
        .get("x-bruhs-tool-grant")
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| {
            AppError::Forbidden("Allow dictation before sending microphone audio.".into())
        })?;
    let mut revoked = host.require(id, ToolScope::Dictation).map_err(error)?;
    tokio::select! {
        biased;
        _ = revoked.changed() => Err(AppError::Forbidden("Dictation access was revoked.".into())),
        result = bruhs_core::capture_client::transcribe_dictation(&daemon, &options, body.to_vec()) => result.map(Json).map_err(|error| AppError::bad_gateway("Local dictation failed", error)),
    }
}

fn error(error: ToolError) -> AppError {
    match error {
        ToolError::Revoked => AppError::Forbidden(error.to_string()),
        ToolError::Invalid(message) => AppError::BadRequest(message),
        ToolError::Unavailable(message) => AppError::Unavailable(message),
        error => AppError::internal_in("Local tool failed", error),
    }
}

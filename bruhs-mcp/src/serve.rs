//! The stdio request loop. A read or write failure on the transport ends the
//! loop with an error (and a non-zero exit), never a silent drop.
use crate::protocol::{ErrorCode, JsonRpcRequest, JsonRpcResponse, ToolInfo};
use crate::stdio_input::{Frame, StdioInput};
use crate::tools::{self, Mode};
use bruhs_core::AppState;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::io;
use std::sync::Arc;
use tokio::io::{AsyncBufRead, AsyncWrite, AsyncWriteExt};

/// Where `tools/call` is answered.
pub enum Backend {
    Standalone(Arc<AppState>),
    #[cfg(unix)]
    Bridge(bruhs_core::workspace_bridge::BridgeClient),
}

impl Backend {
    fn mode(&self) -> Mode {
        match self {
            Self::Standalone(_) => Mode::Standalone,
            #[cfg(unix)]
            Self::Bridge(_) => Mode::Bridge,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ServeError {
    #[error("cannot read stdin: {0}")]
    Read(#[source] io::Error),
    #[error("cannot write to stdout: {0}")]
    Write(#[source] io::Error),
}

/// Serves requests until the input ends (`Ok`) or the transport fails.
pub async fn serve<R, W>(
    mut input: StdioInput<R>,
    output: &mut W,
    backend: &Backend,
) -> Result<(), ServeError>
where
    R: AsyncBufRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let mut buffer = Vec::new();
    let mut text_buffer = Vec::new();
    while let Some(frame) = input.next().await.map_err(ServeError::Read)? {
        let Some(response) = respond(frame, backend, &mut text_buffer).await else {
            continue;
        };
        write_frame(output, &response, &mut buffer)
            .await
            .map_err(ServeError::Write)?;
        recycle_text(response, &mut text_buffer);
    }
    Ok(())
}

/// The response to one frame, or `None` when none is owed (blank lines and
/// notifications).
async fn respond(
    frame: Frame<'_>,
    backend: &Backend,
    text_buffer: &mut Vec<u8>,
) -> Option<JsonRpcResponse> {
    let line = match frame {
        Frame::Message(line) => line,
        Frame::Oversized => {
            return Some(JsonRpcResponse::error(
                None,
                ErrorCode::InvalidRequest,
                "Request exceeds the 64 KiB stdio frame limit. Nothing was executed.".into(),
            ))
        }
    };
    if line.iter().all(u8::is_ascii_whitespace) {
        return None;
    }
    let request = match JsonRpcRequest::parse(line) {
        Ok(request) => request,
        Err((id, error)) => return Some(error.into_response(id)),
    };
    // JSON-RPC notifications (no id) don't get responses.
    if request.id.is_none() && request.method.starts_with("notifications/") {
        return None;
    }
    Some(handle_request(request, backend, text_buffer).await)
}

/// Retain one consumed tool text allocation after its frame has been flushed.
/// The next audit serializes current state into it; no response data is cached.
fn recycle_text(response: JsonRpcResponse, buffer: &mut Vec<u8>) {
    let Some(Value::Object(mut result)) = response.result else {
        return;
    };
    let Some(Value::Array(content)) = result.remove("content") else {
        return;
    };
    for block in content {
        let Value::Object(mut block) = block else {
            continue;
        };
        if let Some(Value::String(text)) = block.remove("text") {
            if text.capacity() > buffer.capacity() {
                *buffer = text.into_bytes();
                buffer.clear();
            }
        }
    }
}

/// Writes one response as a single newline-terminated frame and flushes it.
pub async fn write_frame<W: AsyncWrite + Unpin>(
    output: &mut W,
    response: &JsonRpcResponse,
    buffer: &mut Vec<u8>,
) -> io::Result<()> {
    encode_frame(response, &response.id, buffer);
    output.write_all(buffer).await?;
    output.flush().await
}

/// A response that cannot be serialized becomes a JSON-RPC internal error for
/// the same request, never an empty frame.
fn encode_frame<T: Serialize>(response: &T, id: &Option<Value>, buffer: &mut Vec<u8>) {
    /// Used only if even the error response cannot be encoded.
    const LAST_RESORT: &[u8] =
        br#"{"jsonrpc":"2.0","id":null,"error":{"code":-32603,"message":"Internal error"}}"#;
    buffer.clear();
    if let Err(error) = serde_json::to_writer(&mut *buffer, response) {
        buffer.clear();
        let fallback = JsonRpcResponse::error(
            id.clone(),
            ErrorCode::InternalError,
            format!("Internal error: cannot encode the response: {error}"),
        );
        if serde_json::to_writer(&mut *buffer, &fallback).is_err() {
            buffer.clear();
            buffer.extend_from_slice(LAST_RESORT);
        }
    }
    buffer.push(b'\n');
}

#[derive(Serialize)]
struct ToolsList {
    tools: Vec<&'static ToolInfo>,
}

async fn handle_request(
    request: JsonRpcRequest,
    backend: &Backend,
    text_buffer: &mut Vec<u8>,
) -> JsonRpcResponse {
    let id = request.id;
    match request.method.as_str() {
        "initialize" => JsonRpcResponse::success(
            id,
            json!({
                "protocolVersion": "2024-11-05",
                "capabilities": { "tools": {} },
                "serverInfo": {
                    "name": "bruhs-ai",
                    "version": env!("CARGO_PKG_VERSION")
                }
            }),
        ),
        "tools/list" => encoded(
            id,
            &ToolsList {
                tools: tools::list_tools(backend.mode()),
            },
        ),
        "tools/call" => call_tool(id, request.params, backend, text_buffer).await,
        "ping" => JsonRpcResponse::success(id, json!({})),
        method => JsonRpcResponse::error(
            id,
            ErrorCode::MethodNotFound,
            format!("Method not found: {method}"),
        ),
    }
}

fn encoded(id: Option<Value>, result: &impl Serialize) -> JsonRpcResponse {
    match serde_json::to_value(result) {
        Ok(value) => JsonRpcResponse::success(id, value),
        Err(error) => JsonRpcResponse::error(
            id,
            ErrorCode::InternalError,
            format!("Internal error: {error}"),
        ),
    }
}

#[derive(Deserialize)]
struct CallParams {
    name: String,
    #[serde(default)]
    arguments: Value,
}

async fn call_tool(
    id: Option<Value>,
    params: Value,
    backend: &Backend,
    text_buffer: &mut Vec<u8>,
) -> JsonRpcResponse {
    let params = if params.is_null() { json!({}) } else { params };
    let params: CallParams = match serde_json::from_value(params) {
        Ok(params) => params,
        Err(error) => {
            return JsonRpcResponse::error(
                id,
                ErrorCode::InvalidParams,
                format!("Invalid params: {error}"),
            )
        }
    };
    let Some(tool) = tools::lookup(backend.mode(), &params.name) else {
        return JsonRpcResponse::error(
            id,
            ErrorCode::InvalidParams,
            format!("Unknown tool: {}", params.name),
        );
    };
    let result = match backend {
        Backend::Standalone(state) => {
            tools::call_standalone(tool, &params.arguments, state, text_buffer).await
        }
        #[cfg(unix)]
        Backend::Bridge(client) => {
            crate::workspace_bridge::call(tool, &params.arguments, client).await
        }
    };
    JsonRpcResponse::success(id, result.into_value())
}

#[cfg(test)]
mod tests;

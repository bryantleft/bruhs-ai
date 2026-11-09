//! `POST /api/agent`: run one agent turn and read its SSE reply.
//!
//! The body is read as bytes and split into events on the blank line that
//! ends each one *before* UTF-8 decoding, so a multi-byte character split
//! across two network chunks is decoded whole. A failed read ends the turn
//! as failed ([`StreamError`]), never as a success.

use bruhs_models::api::{AgentRequest, AgentStreamMessage};
use gloo_net::http::{Method, RequestBuilder};
use wasm_bindgen::JsValue;

use super::http::{send_request, url};
use super::ApiError;

/// Why an agent turn's stream ended without finishing normally.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum StreamError {
    /// The user pressed Stop (the request was aborted).
    #[error("Stopped")]
    Stopped,
    /// The connection failed before or while the reply streamed.
    #[error("Could not reach the server: {0}")]
    Transport(String),
    /// The server refused the turn. `detail` is its message, if it sent a
    /// readable one (see [`ApiError::http`]).
    #[error("{}", http_message(*status, detail.as_deref()))]
    Http { status: u16, detail: Option<String> },
    /// The reply was not a well-formed event stream.
    #[error("The agent stream could not be read: {0}")]
    Decode(String),
}

fn http_message(status: u16, detail: Option<&str>) -> String {
    match detail {
        Some(detail) => format!("Server returned {status}: {detail}"),
        None => format!("Server returned status {status}"),
    }
}

impl StreamError {
    /// The server answered 409: the thread is still running another turn.
    pub fn is_busy(&self) -> bool {
        matches!(self, Self::Http { status: 409, .. })
    }
}

impl From<ApiError> for StreamError {
    fn from(error: ApiError) -> Self {
        match error {
            ApiError::Aborted => Self::Stopped,
            ApiError::Network(message) => Self::Transport(message),
            ApiError::Http { status, detail } => Self::Http { status, detail },
            ApiError::Decode(message) => Self::Decode(message),
            ApiError::Timeout { after_ms } => {
                Self::Transport(format!("no answer within {after_ms} ms"))
            }
        }
    }
}

/// A rejected read: an abort is the user's Stop, anything else a transport
/// failure. Decided by the exception's `name` (`DOMException.name`), not by
/// what its message happens to say.
fn read_error(error: &JsValue) -> StreamError {
    let field = |key: &str| {
        js_sys::Reflect::get(error, &JsValue::from_str(key))
            .ok()
            .and_then(|value| value.as_string())
            .unwrap_or_default()
    };
    match ApiError::from_transport(&field("name"), &field("message")) {
        ApiError::Aborted => StreamError::Stopped,
        other => StreamError::from(other),
    }
}

/// Run one turn. Each SSE `data:` payload is handed to `on_event` as it
/// arrives; the call returns when the server closes the stream.
pub async fn send_agent_message_streaming(
    request: &AgentRequest,
    abort_signal: Option<&web_sys::AbortSignal>,
    mut on_event: impl FnMut(AgentStreamMessage),
) -> Result<(), StreamError> {
    use wasm_bindgen::JsCast;
    use wasm_bindgen_futures::JsFuture;

    let request = RequestBuilder::new(&url("/api/agent"))
        .method(Method::POST)
        .abort_signal(abort_signal)
        .json(request)
        .map_err(ApiError::from)?;
    let response = send_request(request).await?;
    let body = response
        .body()
        .ok_or_else(|| StreamError::Decode("the reply has no body".into()))?;
    let reader: web_sys::ReadableStreamDefaultReader = body
        .get_reader()
        .dyn_into()
        .map_err(|_| StreamError::Decode("the reply body cannot be read as a stream".into()))?;

    let mut decoder = SseDecoder::default();
    loop {
        let chunk = JsFuture::from(reader.read())
            .await
            .map_err(|error| read_error(&error))?;
        let done = js_sys::Reflect::get(&chunk, &JsValue::from_str("done"))
            .ok()
            .and_then(|done| done.as_bool())
            .unwrap_or(true);
        if done {
            break;
        }
        let value = js_sys::Reflect::get(&chunk, &JsValue::from_str("value"))
            .map_err(|_| StreamError::Decode("a read returned no bytes".into()))?;
        let bytes = js_sys::Uint8Array::new(&value).to_vec();
        let events = match decoder.push(&bytes) {
            Ok(events) => events,
            Err(error) => {
                // Stop the server streaming into a reader nobody drains; the
                // turn has already failed, so the cancel's outcome is moot.
                let _ = reader.cancel();
                return Err(error);
            }
        };
        for data in events {
            dispatch(&data, &mut on_event);
        }
    }
    if decoder.has_partial_event() {
        log::warn!("agent stream: the server closed the stream inside an event; dropped it");
    }
    Ok(())
}

fn dispatch(data: &str, on_event: &mut impl FnMut(AgentStreamMessage)) {
    if data == "[DONE]" {
        return;
    }
    match serde_json::from_str::<serde_json::Value>(data) {
        Ok(value) => on_event(AgentStreamMessage::from_value(value)),
        // One unreadable frame should not end a turn that is still running.
        Err(error) => log::warn!("agent stream: skipped a frame that is not JSON: {error}"),
    }
}

/// Splits an SSE byte stream into the `data` of each complete event.
///
/// Bytes are buffered until the blank line that ends an event (`\n\n`, or
/// `\r\n\r\n`); only then is the event decoded as UTF-8, so chunk boundaries
/// inside a character are harmless. Comment lines (`:keep-alive`) and events
/// without data are skipped; several `data:` lines join with `\n`.
#[derive(Debug, Default)]
pub(crate) struct SseDecoder {
    buffer: Vec<u8>,
    /// How far `buffer` has been searched for a boundary.
    scanned: usize,
}

impl SseDecoder {
    pub(crate) fn push(&mut self, bytes: &[u8]) -> Result<Vec<String>, StreamError> {
        self.buffer.extend_from_slice(bytes);
        let mut events = Vec::new();
        while let Some((end, next)) = self.boundary() {
            let raw: Vec<u8> = self.buffer.drain(..next).take(end).collect();
            self.scanned = 0;
            let text = std::str::from_utf8(&raw)
                .map_err(|error| StreamError::Decode(format!("an event is not UTF-8: {error}")))?;
            if let Some(data) = event_data(text) {
                events.push(data);
            }
        }
        Ok(events)
    }

    /// Bytes of an event the stream has not finished.
    pub(crate) fn has_partial_event(&self) -> bool {
        self.buffer.iter().any(|byte| !byte.is_ascii_whitespace())
    }

    /// `(end of the event, start of the next one)` for the first blank line.
    fn boundary(&mut self) -> Option<(usize, usize)> {
        // A boundary can straddle the previous search end by up to 3 bytes.
        let from = self.scanned.saturating_sub(3);
        let found = (from..self.buffer.len()).find_map(|i| {
            let rest = &self.buffer[i..];
            if rest.starts_with(b"\n\n") {
                Some((i, i + 2))
            } else if rest.starts_with(b"\r\n\r\n") {
                Some((i, i + 4))
            } else {
                None
            }
        });
        if found.is_none() {
            self.scanned = self.buffer.len();
        }
        found
    }
}

/// The `data` of one event, or `None` when it has none.
fn event_data(event: &str) -> Option<String> {
    let mut data: Option<String> = None;
    for line in event.lines() {
        let Some(value) = line.strip_prefix("data:") else {
            continue;
        };
        let value = value.strip_prefix(' ').unwrap_or(value);
        match data.as_mut() {
            Some(data) => {
                data.push('\n');
                data.push_str(value);
            }
            None => data = Some(value.to_owned()),
        }
    }
    data
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decode_all(chunks: &[&[u8]]) -> Vec<String> {
        let mut decoder = SseDecoder::default();
        let mut out = Vec::new();
        for chunk in chunks {
            out.extend(decoder.push(chunk).unwrap());
        }
        assert!(!decoder.has_partial_event());
        out
    }

    /// A 4-byte emoji split at every byte offset decodes whole (the old
    /// reader decoded each chunk lossily and turned it into U+FFFD).
    #[test]
    fn a_character_split_across_chunks_decodes_whole() {
        let wire = "data: {\"type\":\"text\",\"text\":\"hi 🦀 there\"}\n\n".as_bytes();
        let emoji_at = wire.windows(4).position(|w| w == "🦀".as_bytes()).unwrap();
        for split in 0..=wire.len() {
            let events = decode_all(&[&wire[..split], &wire[split..]]);
            assert_eq!(
                events,
                vec!["{\"type\":\"text\",\"text\":\"hi 🦀 there\"}".to_string()],
                "split at {split}"
            );
        }
        // Every offset inside the emoji, one byte per chunk.
        for split in emoji_at + 1..emoji_at + 4 {
            let bytes: Vec<&[u8]> = wire[..split]
                .chunks(1)
                .chain(std::iter::once(&wire[split..]))
                .collect();
            assert_eq!(decode_all(&bytes).len(), 1);
        }
    }

    #[test]
    fn the_event_boundary_can_be_split_too() {
        let wire = b"data: {\"a\":1}\n\ndata: {\"b\":2}\r\n\r\n";
        for split in 0..=wire.len() {
            assert_eq!(
                decode_all(&[&wire[..split], &wire[split..]]),
                vec!["{\"a\":1}".to_string(), "{\"b\":2}".to_string()],
                "split at {split}"
            );
        }
    }

    #[test]
    fn comments_empty_events_and_multi_line_data() {
        let events = decode_all(&[b":\n\nevent: x\n\ndata: one\ndata: two\n\ndata:three\n\n"]);
        assert_eq!(events, vec!["one\ntwo".to_string(), "three".to_string()]);
    }

    #[test]
    fn invalid_utf8_in_a_complete_event_is_a_decode_error() {
        let mut decoder = SseDecoder::default();
        assert!(matches!(
            decoder.push(b"data: \xff\xfe\n\n"),
            Err(StreamError::Decode(_))
        ));
    }

    #[test]
    fn an_unterminated_event_is_reported_as_partial() {
        let mut decoder = SseDecoder::default();
        assert!(decoder.push(b"data: {\"a\":").unwrap().is_empty());
        assert!(decoder.has_partial_event());
    }

    #[test]
    fn errors_map_to_stream_errors() {
        assert_eq!(StreamError::from(ApiError::Aborted), StreamError::Stopped);
        let busy = StreamError::from(ApiError::http(409, r#"{"error":"thread busy"}"#));
        assert!(busy.is_busy());
        assert_eq!(busy.to_string(), "Server returned 409: thread busy");
        assert!(!StreamError::from(ApiError::http(500, "")).is_busy());
        assert_eq!(
            StreamError::from(ApiError::http(500, "")).to_string(),
            "Server returned status 500"
        );
    }
}

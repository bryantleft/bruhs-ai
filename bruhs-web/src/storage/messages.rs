//! Batch small conversation headers across the WASM/JavaScript boundary.
//!
//! Content strings and arbitrary tool inputs stay in structured-clone arrays:
//! large strings need no JSON escaping/parsing, and tool inputs keep the
//! established codec's depth and number behavior. Reads reconstruct the full
//! typed original; only a successful checked save replaces a legacy record.

use serde::{ser::SerializeSeq, Deserialize, Serialize, Serializer};
use wasm_bindgen::JsValue;

use super::{from_js, metadata, to_js, MessagesRecord};
use crate::state::{AgentMessage, MessagePart, ToolCallInfo};
use metadata::SaveError;

const MARKER: &str = "bruhs_messages_encoding";

#[derive(Clone, Copy, Serialize, Deserialize)]
enum Encoding {
    #[serde(rename = "parts-v1")]
    PartsV1,
}

#[derive(Serialize)]
struct Encoded<'a> {
    thread_id: &'a str,
    bruhs_messages_encoding: Encoding,
    payload: String,
    strings: Vec<&'a str>,
    inputs: Vec<&'a serde_json::Value>,
}

#[derive(Deserialize)]
#[cfg_attr(test, derive(Clone))]
#[serde(deny_unknown_fields)]
struct Envelope {
    thread_id: String,
    bruhs_messages_encoding: Encoding,
    payload: String,
    strings: Vec<String>,
    inputs: Vec<serde_json::Value>,
}

/// Ordered parts need no duplicate flat compatibility fields in this format.
/// Content placeholders mark the order of the two structured-clone arrays.
#[derive(Serialize)]
struct StoredRecord<'a> {
    thread_id: &'a str,
    messages: StoredMessages<'a>,
}

struct StoredMessages<'a>(&'a [AgentMessage]);
struct StoredParts<'a>(&'a [MessagePart]);

#[derive(Serialize)]
struct StoredMessage<'a> {
    id: &'a str,
    role: &'static str,
    is_streaming: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    usage: Option<&'a crate::state::UsageInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    started_at_ms: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    completed_at_ms: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    model: Option<&'a str>,
    parts: StoredParts<'a>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<&'a str>,
    #[serde(skip_serializing_if = "<[bruhs_models::logic::attachments::UploadedFile]>::is_empty")]
    files: &'a [bruhs_models::logic::attachments::UploadedFile],
    #[serde(skip_serializing_if = "Option::is_none")]
    checkpoint: Option<&'a str>,
}

#[derive(Serialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
enum StoredPart<'a> {
    Text(&'static str),
    Thinking(&'static str),
    ToolCall(StoredTool<'a>),
}

#[derive(Serialize)]
struct StoredTool<'a> {
    id: &'a str,
    tool: &'a str,
    input: (),
    output: Option<&'static str>,
    is_error: bool,
    started_at_ms: Option<f64>,
    ended_at_ms: Option<f64>,
}

impl Serialize for StoredMessages<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut messages = serializer.serialize_seq(Some(self.0.len()))?;
        for message in self.0 {
            // Exhaustive projection: a new message field must update storage.
            let AgentMessage {
                id,
                role,
                status,
                usage,
                started_at_ms,
                model,
                parts,
                error,
                files,
                checkpoint,
            } = message;
            messages.serialize_element(&StoredMessage {
                id,
                role: role.as_str(),
                is_streaming: status.is_streaming(),
                usage: usage.as_ref(),
                started_at_ms: *started_at_ms,
                completed_at_ms: status.finished_at_ms(),
                model: model.as_deref(),
                parts: StoredParts(parts),
                error: error.as_deref(),
                files,
                checkpoint: checkpoint.as_deref(),
            })?;
        }
        messages.end()
    }
}

impl Serialize for StoredParts<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut parts = serializer.serialize_seq(Some(self.0.len()))?;
        for part in self.0 {
            let stored = match part {
                MessagePart::Text(_) => StoredPart::Text(""),
                MessagePart::Thinking(_) => StoredPart::Thinking(""),
                MessagePart::ToolCall(tool) => {
                    let ToolCallInfo {
                        id,
                        tool,
                        input: _,
                        output,
                        is_error,
                        started_at_ms,
                        ended_at_ms,
                    } = tool;
                    StoredPart::ToolCall(StoredTool {
                        id,
                        tool,
                        input: (),
                        output: output.as_ref().map(|_| ""),
                        is_error: *is_error,
                        started_at_ms: *started_at_ms,
                        ended_at_ms: *ended_at_ms,
                    })
                }
            };
            parts.serialize_element(&stored)?;
        }
        parts.end()
    }
}

fn prepared(record: &MessagesRecord) -> Option<Encoded<'_>> {
    let payload = metadata::json_string(&StoredRecord {
        thread_id: &record.thread_id,
        messages: StoredMessages(&record.messages),
    })?;
    let mut strings = Vec::new();
    let mut inputs = Vec::new();
    for part in record.messages.iter().flat_map(|message| &message.parts) {
        match part {
            MessagePart::Text(text) | MessagePart::Thinking(text) => strings.push(text.as_str()),
            MessagePart::ToolCall(tool) => {
                inputs.push(&tool.input);
                if let Some(output) = &tool.output {
                    strings.push(output.as_str());
                }
            }
        }
    }
    Some(Encoded {
        thread_id: &record.thread_id,
        bruhs_messages_encoding: Encoding::PartsV1,
        payload,
        strings,
        inputs,
    })
}

impl Envelope {
    fn record(self) -> Result<MessagesRecord, SaveError> {
        let Self {
            thread_id,
            bruhs_messages_encoding: Encoding::PartsV1,
            payload,
            strings,
            inputs,
        } = self;
        let mut record: MessagesRecord =
            serde_json::from_str(&payload).map_err(|_| SaveError::Corrupt)?;
        if record.thread_id != thread_id {
            return Err(SaveError::Corrupt);
        }
        let mut strings = strings.into_iter();
        let mut inputs = inputs.into_iter();
        for part in record
            .messages
            .iter_mut()
            .flat_map(|message| &mut message.parts)
        {
            match part {
                MessagePart::Text(text) | MessagePart::Thinking(text) => {
                    if !text.is_empty() {
                        return Err(SaveError::Corrupt);
                    }
                    *text = strings.next().ok_or(SaveError::Corrupt)?;
                }
                MessagePart::ToolCall(tool) => {
                    if !tool.input.is_null()
                        || tool
                            .output
                            .as_ref()
                            .is_some_and(|output| !output.is_empty())
                    {
                        return Err(SaveError::Corrupt);
                    }
                    tool.input = inputs.next().ok_or(SaveError::Corrupt)?;
                    if let Some(output) = &mut tool.output {
                        *output = strings.next().ok_or(SaveError::Corrupt)?;
                    }
                }
            }
        }
        if strings.next().is_some() || inputs.next().is_some() {
            return Err(SaveError::Corrupt);
        }
        Ok(record)
    }
}

pub(super) fn encode(record: &MessagesRecord) -> Result<JsValue, idb::Error> {
    if let Some(encoded) = prepared(record) {
        return to_js(&encoded);
    }
    // Preserve special header numbers and the existing codec's error behavior.
    to_js(record)
}

pub(super) fn decode(raw: JsValue) -> Result<MessagesRecord, SaveError> {
    let marked =
        js_sys::Reflect::has(&raw, &JsValue::from_str(MARKER)).map_err(|_| SaveError::Corrupt)?;
    if marked {
        // serde-wasm-bindgen visits only declared struct fields. Check this
        // five-field schema explicitly before it can discard unknown fields.
        let keys = js_sys::Object::keys(&js_sys::Object::from(raw.clone()));
        if keys.length() != 5
            || keys.iter().any(|key| {
                !matches!(
                    key.as_string().as_deref(),
                    Some("thread_id" | MARKER | "payload" | "strings" | "inputs")
                )
            })
        {
            return Err(SaveError::Corrupt);
        }
        let envelope: Envelope = from_js(raw).map_err(|_| SaveError::Corrupt)?;
        envelope.record()
    } else {
        from_js(raw).map_err(|_| SaveError::Corrupt)
    }
}

#[cfg(test)]
mod tests;

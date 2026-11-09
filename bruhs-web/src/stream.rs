//! Reduces one `/api/agent` SSE message into an `AgentMessage`. Pure so the
//! wire format is testable on the host target.
//!
//! Messages arrive as the shared [`AgentStreamMessage`]. Ids the wire leaves
//! empty are treated as missing: a `tool_result` without an id patches no
//! call (it used to patch every call whose id was also empty), and a
//! permission request without an id is not surfaced (it could not be
//! answered).

use std::collections::HashSet;

use bruhs_models::api::{AgentStreamEvent, AgentStreamMessage};
use serde_json::Value;

use crate::state::{AgentMessage, MessagePart, PermissionRequest, ToolCallInfo, UsageInfo};

/// Side effects the caller must apply after mutating the message.
#[derive(Debug, Clone, PartialEq)]
pub enum StreamEffect {
    SecretRequest(bruhs_models::secrets::SecretRequest),
    SecretResolved(String),
    None,
    /// The provider announced (or confirmed) the session id for this thread.
    SessionId(String),
    /// The agent is waiting on a permission decision.
    PermissionRequest(PermissionRequest),
    /// The provider withdrew the permission request with this id.
    PermissionCancelled(String),
    /// The turn is over; `is_error` mirrors the provider's result flag.
    Finished {
        is_error: bool,
    },
}

/// An id the wire may leave empty; empty means there is none.
fn present(id: &str) -> Option<&str> {
    (!id.is_empty()).then_some(id)
}

/// `usage` as the message's usage. A turn sum carries no window size, so
/// the window last reported (`prior`) is kept.
fn usage_from(usage: &bruhs_models::api::UsageInfo, prior: Option<&UsageInfo>) -> UsageInfo {
    UsageInfo {
        input_tokens: usage.input_tokens,
        output_tokens: usage.output_tokens,
        cache_read_tokens: usage.cache_read_input_tokens,
        cache_creation_tokens: usage.cache_creation_input_tokens,
        context_tokens: usage
            .context_tokens
            .or_else(|| prior.and_then(|prior| prior.context_tokens)),
    }
}

/// Apply one stream message. Messages this reducer has no use for
/// (background-task announcements, which arrive on `/api/tasks/stream`, and
/// unknown payloads, which `unknown` logs once per type) change nothing.
pub fn apply_message(
    msg: &mut AgentMessage,
    message: &AgentStreamMessage,
    now_ms: f64,
    unknown: &mut UnknownEvents,
) -> StreamEffect {
    match message {
        AgentStreamMessage::Event(event) => apply_event(msg, event, now_ms),
        AgentStreamMessage::BackgroundTaskRegistered(_) => StreamEffect::None,
        AgentStreamMessage::Unknown(value) => {
            if let Some(kind) = unknown.first_sighting(value) {
                log::warn!("agent stream: ignoring an event this build cannot read ({kind})");
            }
            StreamEffect::None
        }
    }
}

pub fn apply_event(msg: &mut AgentMessage, event: &AgentStreamEvent, now_ms: f64) -> StreamEffect {
    match event {
        AgentStreamEvent::SecretRequest(request) => StreamEffect::SecretRequest(request.clone()),
        AgentStreamEvent::SecretResolved { id } => StreamEffect::SecretResolved(id.clone()),
        AgentStreamEvent::Text { text } => {
            if text.is_empty() {
                return StreamEffect::None;
            }
            match msg.parts.last_mut() {
                Some(MessagePart::Text(t)) => t.push_str(text),
                _ => msg.parts.push(MessagePart::Text(text.clone())),
            }
            StreamEffect::None
        }
        AgentStreamEvent::Thinking { thinking } => {
            if thinking.is_empty() {
                return StreamEffect::None;
            }
            match msg.parts.last_mut() {
                Some(MessagePart::Thinking(t)) => t.push_str(thinking),
                _ => msg.parts.push(MessagePart::Thinking(thinking.clone())),
            }
            StreamEffect::None
        }
        AgentStreamEvent::ToolCall { id, tool, input } => {
            let call = ToolCallInfo {
                id: id.clone(),
                tool: tool.clone(),
                input: input.clone(),
                output: None,
                is_error: false,
                started_at_ms: Some(now_ms),
                ended_at_ms: None,
            };
            msg.parts.push(MessagePart::ToolCall(call));
            StreamEffect::None
        }
        AgentStreamEvent::ToolResult {
            tool_use_id,
            content,
            is_error,
        } => {
            let Some(id) = present(tool_use_id) else {
                log::warn!("agent stream: a tool result has no tool id; not attached to any call");
                return StreamEffect::None;
            };
            for part in msg.parts.iter_mut() {
                if let MessagePart::ToolCall(c) = part {
                    if c.id == id {
                        c.output = Some(content.clone());
                        c.is_error = *is_error;
                        c.ended_at_ms = Some(now_ms);
                    }
                }
            }
            StreamEffect::None
        }
        AgentStreamEvent::Usage(usage) => {
            msg.usage = Some(usage_from(usage, msg.usage.as_ref()));
            StreamEffect::None
        }
        AgentStreamEvent::Init { session_id, .. } => match present(session_id) {
            Some(session_id) => StreamEffect::SessionId(session_id.to_owned()),
            None => StreamEffect::None,
        },
        AgentStreamEvent::Result {
            result,
            usage,
            is_error,
            ..
        } => {
            // A result without usage keeps the running totals.
            if *usage != bruhs_models::api::UsageInfo::default() {
                msg.usage = Some(usage_from(usage, msg.usage.as_ref()));
            }
            if *is_error {
                msg.error = Some(if result.is_empty() {
                    "The agent returned an error.".to_string()
                } else {
                    result.clone()
                });
            }
            StreamEffect::Finished {
                is_error: *is_error,
            }
        }
        AgentStreamEvent::Error { message } => {
            msg.error = Some(if message.is_empty() {
                "The agent stream failed.".to_string()
            } else {
                message.clone()
            });
            StreamEffect::Finished { is_error: true }
        }
        AgentStreamEvent::PermissionRequest {
            id, tool, input, ..
        } => match present(id) {
            Some(id) => StreamEffect::PermissionRequest(PermissionRequest {
                id: id.to_owned(),
                tool: tool.clone(),
                input: input.clone(),
            }),
            None => {
                log::warn!("agent stream: a permission request has no id; it cannot be answered");
                StreamEffect::None
            }
        },
        AgentStreamEvent::PermissionCancelled { id } => match present(id) {
            Some(id) => StreamEffect::PermissionCancelled(id.to_owned()),
            None => StreamEffect::None,
        },
        // Provider notices bruhs passes through uninterpreted.
        AgentStreamEvent::Raw { .. } => StreamEffect::None,
        // `AgentStreamMessage` keeps unknown types as `Unknown`; a bare
        // event can still carry one.
        AgentStreamEvent::Unknown => StreamEffect::None,
    }
}

/// The unreadable event kinds a turn has already logged, so a provider that
/// streams a new event type does not flood the console.
#[derive(Debug, Default)]
pub struct UnknownEvents(HashSet<String>);

impl UnknownEvents {
    /// The payload's kind (`envelope`/`type`) the first time it is seen;
    /// `None` after that.
    pub fn first_sighting(&mut self, value: &Value) -> Option<String> {
        let field = |key: &str| value.get(key).and_then(Value::as_str).unwrap_or("-");
        let kind = format!("envelope {}, type {}", field("envelope"), field("type"));
        self.0.insert(kind.clone()).then_some(kind)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{AgentMessage, MessagePart};
    use serde_json::json;

    #[test]
    fn private_request_events_never_become_saved_message_parts() {
        let mut message = msg();
        let request = bruhs_models::secrets::SecretRequest {
            id: "private".into(),
            thread_id: bruhs_models::ids::ThreadId::new_v4(),
            operation: bruhs_models::secrets::SecretOperation {
                name: "Key".into(),
                reason: "Deploy".into(),
                env_var: "API_KEY".into(),
                command: vec!["deploy".into()],
            },
            cwd: "/tmp".into(),
            saved_available: false,
            storage_unavailable: false,
        };
        assert!(matches!(
            apply_event(&mut message, &AgentStreamEvent::SecretRequest(request), 0.0),
            StreamEffect::SecretRequest(_)
        ));
        assert!(matches!(
            apply_event(
                &mut message,
                &AgentStreamEvent::SecretResolved {
                    id: "private".into()
                },
                0.0
            ),
            StreamEffect::SecretResolved(_)
        ));
        assert!(message.parts.is_empty());
    }

    fn msg() -> AgentMessage {
        AgentMessage::assistant_placeholder("m".into(), 0.0)
    }

    /// Apply a wire payload the way the chat page does.
    fn apply(m: &mut AgentMessage, wire: Value, now: f64) -> StreamEffect {
        let message = AgentStreamMessage::from_value(wire);
        apply_message(m, &message, now, &mut UnknownEvents::default())
    }

    #[test]
    fn text_deltas_coalesce_and_order_is_kept() {
        let mut m = msg();
        apply(&mut m, json!({"type":"text","text":"Hel"}), 1.0);
        apply(&mut m, json!({"type":"text","text":"lo"}), 2.0);
        apply(
            &mut m,
            json!({"type":"tool_call","id":"t1","tool":"Read","input":{"file_path":"a.rs"}}),
            3.0,
        );
        apply(&mut m, json!({"type":"text","text":" done"}), 4.0);
        assert_eq!(m.parts.len(), 3);
        assert!(matches!(&m.parts[0], MessagePart::Text(t) if t == "Hello"));
        assert!(matches!(&m.parts[2], MessagePart::Text(t) if t == " done"));
        assert_eq!(m.text(), "Hello done");
        assert_eq!(m.tool_calls_iter().count(), 1);
    }

    #[test]
    fn tool_result_patches_call_and_stamps_end() {
        let mut m = msg();
        apply(
            &mut m,
            json!({"envelope":"query","type":"tool_call","id":"t1","tool":"Bash","input":{"command":"ls"}}),
            5.0,
        );
        apply(
            &mut m,
            json!({"envelope":"query","type":"tool_result","tool_use_id":"t1","content":"a\nb","is_error":false}),
            9.0,
        );
        let MessagePart::ToolCall(c) = &m.parts[0] else {
            panic!("expected tool call part")
        };
        assert_eq!(c.output.as_deref(), Some("a\nb"));
        assert_eq!(c.started_at_ms, Some(5.0));
        assert_eq!(c.ended_at_ms, Some(9.0));
    }

    /// A result with no id used to read as id `""` and finish every call
    /// that also had no id.
    #[test]
    fn a_tool_result_without_an_id_patches_nothing() {
        let mut m = msg();
        apply(
            &mut m,
            json!({"type":"tool_call","tool":"Bash","input":{}}),
            1.0,
        );
        apply(
            &mut m,
            json!({"type":"tool_call","id":"","tool":"Read","input":{}}),
            1.0,
        );
        apply(&mut m, json!({"type":"tool_result","content":"x"}), 2.0);
        apply(
            &mut m,
            json!({"type":"tool_result","tool_use_id":"","content":"y"}),
            2.0,
        );
        assert_eq!(m.tool_calls_iter().count(), 2);
        assert!(m
            .tool_calls_iter()
            .all(|c| c.output.is_none() && c.ended_at_ms.is_none()));
    }

    #[test]
    fn thinking_coalesces_separately() {
        let mut m = msg();
        apply(&mut m, json!({"type":"thinking","thinking":"a"}), 0.0);
        apply(&mut m, json!({"type":"thinking","thinking":"b"}), 0.0);
        apply(&mut m, json!({"type":"text","text":"x"}), 0.0);
        apply(&mut m, json!({"type":"thinking","thinking":"c"}), 0.0);
        assert_eq!(m.parts.len(), 3);
        assert_eq!(m.thinking(), "abc");
    }

    #[test]
    fn init_and_result_yield_session_and_finish() {
        let mut m = msg();
        assert!(matches!(
            apply(&mut m, json!({"type":"init","session_id":"s1"}), 0.0),
            StreamEffect::SessionId(s) if s == "s1"
        ));
        assert_eq!(
            apply(&mut m, json!({"type":"init","session_id":""}), 0.0),
            StreamEffect::None
        );
        let eff = apply(
            &mut m,
            json!({"type":"result","result":"ok","session_id":"s1","duration_ms":10,"is_error":false,"usage":{"input_tokens":5,"output_tokens":7}}),
            0.0,
        );
        assert!(matches!(eff, StreamEffect::Finished { is_error: false }));
        assert_eq!(m.usage.as_ref().map(|u| u.output_tokens), Some(7));
        assert!(m.error.is_none());
    }

    #[test]
    fn a_result_without_usage_keeps_the_running_totals() {
        let mut m = msg();
        apply(
            &mut m,
            json!({"type":"usage","input_tokens":3,"output_tokens":4}),
            0.0,
        );
        apply(
            &mut m,
            json!({"type":"result","result":"ok","usage":{}}),
            0.0,
        );
        assert_eq!(m.usage.as_ref().map(|u| u.output_tokens), Some(4));
    }

    #[test]
    fn error_event_sets_error_and_finishes() {
        let mut m = msg();
        let eff = apply(&mut m, json!({"type":"error","message":"boom"}), 0.0);
        assert!(matches!(eff, StreamEffect::Finished { is_error: true }));
        assert_eq!(m.error.as_deref(), Some("boom"));
    }

    #[test]
    fn result_error_flag_populates_error_from_text() {
        let mut m = msg();
        apply(
            &mut m,
            json!({"type":"result","result":"Rate limited","session_id":"s","duration_ms":1,"is_error":true,"usage":{}}),
            0.0,
        );
        assert_eq!(m.error.as_deref(), Some("Rate limited"));
    }

    #[test]
    fn usage_event_maps_anthropic_names() {
        let mut m = msg();
        apply(
            &mut m,
            json!({"type":"usage","input_tokens":1,"output_tokens":2,"cache_read_input_tokens":3,"cache_creation_input_tokens":4}),
            0.0,
        );
        let u = m.usage.unwrap();
        assert_eq!(
            (
                u.input_tokens,
                u.output_tokens,
                u.cache_read_tokens,
                u.cache_creation_tokens
            ),
            (1, 2, 3, 4)
        );
    }

    /// Claude's `result.usage` sums every request of the turn; the window
    /// reported by the last request must survive it.
    #[test]
    fn a_turn_sum_keeps_the_last_reported_window() {
        let mut m = msg();
        apply(
            &mut m,
            json!({"type":"usage","input_tokens":2,"output_tokens":3,"cache_read_input_tokens":47261,"cache_creation_input_tokens":1928,"context_tokens":49191}),
            0.0,
        );
        apply(
            &mut m,
            json!({"type":"result","result":"done","session_id":"s","duration_ms":1,"is_error":false,
                   "usage":{"input_tokens":4,"output_tokens":350,"cache_read_input_tokens":74003,"cache_creation_input_tokens":22447}}),
            0.0,
        );
        let u = m.usage.unwrap();
        assert_eq!(
            u.cache_read_tokens, 74_003,
            "billed counts are the turn sum"
        );
        assert_eq!(u.context_tokens, Some(49_191));
        assert_eq!(u.window(), 49_191);
    }

    #[test]
    fn permission_request_is_surfaced() {
        let mut m = msg();
        let eff = apply(
            &mut m,
            json!({"type":"permission_request","id":"p","tool":"Bash","input":{"command":"rm"}}),
            0.0,
        );
        assert!(
            matches!(eff, StreamEffect::PermissionRequest(r) if r.tool == "Bash" && r.id == "p")
        );
        let cancel = apply(
            &mut m,
            json!({"type":"permission_cancelled","id":"p1"}),
            2.0,
        );
        assert!(matches!(cancel, StreamEffect::PermissionCancelled(id) if id == "p1"));
        // Without an id there is nothing to answer or withdraw.
        assert_eq!(
            apply(
                &mut m,
                json!({"type":"permission_request","tool":"Bash"}),
                0.0
            ),
            StreamEffect::None
        );
        assert_eq!(
            apply(&mut m, json!({"type":"permission_cancelled"}), 0.0),
            StreamEffect::None
        );
    }

    #[test]
    fn unknown_and_task_messages_change_nothing_and_log_once_per_kind() {
        let mut m = msg();
        let before = m.clone();
        let mut unknown = UnknownEvents::default();
        for wire in [
            json!({"envelope":"query","type":"brand_new","x":1}),
            json!({"envelope":"other"}),
            json!({"envelope":"background_task_registered","task":{"id":"bg","thread_id":"t","label":"x","status":"running","started_at":1}}),
            json!({"type":"raw","data":{"a":1}}),
        ] {
            let message = AgentStreamMessage::from_value(wire);
            assert_eq!(
                apply_message(&mut m, &message, 0.0, &mut unknown),
                StreamEffect::None
            );
        }
        assert_eq!(m, before);
        assert_eq!(
            unknown.first_sighting(&json!({"envelope":"query","type":"brand_new"})),
            None
        );
        assert_eq!(
            unknown.first_sighting(&json!({"type":"another"})),
            Some("envelope -, type another".to_string())
        );
    }
}

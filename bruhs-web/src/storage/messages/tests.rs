use super::*;
use crate::state::AgentMessage;

fn envelope(record: &MessagesRecord) -> Envelope {
    serde_json::from_value(
        serde_json::to_value(prepared(record).expect("ordinary headers are JSON safe")).unwrap(),
    )
    .unwrap()
}

#[test]
fn payload_preserves_ordered_and_legacy_message_behavior() {
    let record: MessagesRecord = serde_json::from_value(serde_json::json!({
        "thread_id": "thread",
        "messages": [
            {"id":"legacy", "role":"user", "content":"雪 😀", "thinking":"thought",
             "tool_calls":[], "is_streaming":false},
            {"id":"ordered", "role":"assistant", "content":"ignored flat text",
             "is_streaming":true, "completed_at":12.5,
             "parts":[{"kind":"thinking","value":"first"},
                      {"kind":"text","value":"second"}],
             "error":"saved error", "files":[{"name":"file","path":"/file"}],
             "checkpoint":"tree"}
        ]
    }))
    .unwrap();
    let decoded = envelope(&record).record().unwrap();
    assert_eq!(decoded, record);
    assert_eq!(decoded.messages[0].text(), "雪 😀");
    assert_eq!(decoded.messages[1].text(), "second");
    assert!(decoded.messages[1].is_streaming());
}

#[test]
fn compact_payload_keeps_tools_and_metadata_without_flat_duplicates() {
    let record: MessagesRecord = serde_json::from_value(serde_json::json!({
        "thread_id":"thread", "messages":[{
            "id":"tool-reply", "role":"assistant", "is_streaming":false,
            "started_at_ms":1.25, "completed_at_ms":2.5, "model":"model",
            "usage":{"input_tokens":11,"output_tokens":12,"cache_read_tokens":13,
                     "cache_creation_tokens":14,"context_tokens":50},
            "parts":[{"kind":"tool_call","value":{
                "id":"tool", "tool":"Read", "input":{"path":"/雪","ratio":1.25},
                "output":"large output 😀", "is_error":true,
                "started_at_ms":1.5,"ended_at_ms":2.25
            }}],
            "error":"failure", "files":[{"name":"file","path":"/file"}],
            "checkpoint":"tree"
        }]
    }))
    .unwrap();
    let encoded = envelope(&record);
    let stored: serde_json::Value = serde_json::from_str(&encoded.payload).unwrap();
    let message = &stored["messages"][0];
    for key in ["content", "thinking", "tool_calls"] {
        assert!(
            message.get(key).is_none(),
            "flat field {key} duplicates parts"
        );
    }
    assert_eq!(encoded.payload.matches("large output 😀").count(), 0);
    assert_eq!(
        encoded
            .strings
            .iter()
            .filter(|s| s.as_str() == "large output 😀")
            .count(),
        1
    );
    assert_eq!(encoded.record().unwrap(), record);
}

#[test]
fn finite_float_bits_survive_json_storage() {
    for value in [
        0.0,
        -0.0,
        f64::MIN_POSITIVE,
        f64::from_bits(1),
        1.000_000_000_000_000_2,
        f64::MAX,
        -f64::MAX,
    ] {
        let record = MessagesRecord {
            thread_id: "thread".into(),
            messages: vec![AgentMessage::user("prompt".into(), value)],
        };
        let decoded = envelope(&record).record().unwrap();
        assert_eq!(
            decoded.messages[0].started_at_ms.unwrap().to_bits(),
            value.to_bits()
        );
    }
}

#[test]
fn special_numbers_require_the_structured_codec() {
    for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        let record = MessagesRecord {
            thread_id: "thread".into(),
            messages: vec![AgentMessage::user("prompt".into(), value)],
        };
        assert!(prepared(&record).is_none());
    }
    assert!(metadata::json_string(&serde_json::json!({"integer": u64::MAX})).is_none());
}

#[test]
fn malformed_or_mismatched_envelopes_cannot_be_read_as_empty_history() {
    let mut encoded = envelope(&MessagesRecord {
        thread_id: "thread".into(),
        messages: Vec::new(),
    });
    encoded.thread_id = "other".into();
    assert!(matches!(encoded.clone().record(), Err(SaveError::Corrupt)));
    encoded.payload = "{\"thread_id\":\"other\",\"messages\":[{\"id\":\"broken\"}]}".into();
    assert!(matches!(encoded.clone().record(), Err(SaveError::Corrupt)));
    encoded.payload = "{".into();
    assert!(matches!(encoded.record(), Err(SaveError::Corrupt)));
    for raw in [
        serde_json::json!({"thread_id":"thread", "bruhs_messages_encoding":"parts-v2", "payload":"{}"}),
        serde_json::json!({"thread_id":"thread", "bruhs_messages_encoding":"parts-v1", "payload":"{}", "messages":[]}),
        serde_json::json!({"thread_id":"thread", "bruhs_messages_encoding":"parts-v1"}),
    ] {
        assert!(serde_json::from_value::<Envelope>(raw).is_err());
    }
}

#[test]
fn deep_tool_inputs_stay_out_of_the_json_parser() {
    let mut input = serde_json::json!({"leaf":"saved"});
    for _ in 0..140 {
        input = serde_json::json!({"nested":input});
    }
    let mut message = AgentMessage::user("prompt".into(), 1.0);
    message.parts = vec![crate::state::MessagePart::ToolCall(
        crate::state::ToolCallInfo {
            id: "tool".into(),
            tool: "Synthetic".into(),
            input,
            output: None,
            is_error: false,
            started_at_ms: None,
            ended_at_ms: None,
        },
    )];
    let record = MessagesRecord {
        thread_id: "thread".into(),
        messages: vec![message],
    };
    let encoded = envelope(&record);
    assert!(
        !encoded.payload.contains("nested"),
        "tool input must be structured, not JSON headers"
    );
    assert_eq!(encoded.record().unwrap(), record);
}

#[test]
fn content_arrays_require_exact_members_and_empty_placeholders() {
    let record = MessagesRecord {
        thread_id: "thread".into(),
        messages: vec![AgentMessage::user("saved".into(), 1.0)],
    };
    let encoded = envelope(&record);
    let mut missing = encoded.clone();
    missing.strings.clear();
    assert!(matches!(missing.record(), Err(SaveError::Corrupt)));
    let mut extra = encoded.clone();
    extra.strings.push("orphan".into());
    assert!(matches!(extra.record(), Err(SaveError::Corrupt)));
    let mut extra = encoded.clone();
    extra.inputs.push(serde_json::Value::Null);
    assert!(matches!(extra.record(), Err(SaveError::Corrupt)));
    let mut bad = encoded;
    bad.payload = bad
        .payload
        .replace("\"value\":\"\"", "\"value\":\"hidden\"");
    assert!(matches!(bad.record(), Err(SaveError::Corrupt)));
}

#[test]
fn missing_and_empty_tool_outputs_remain_distinct() {
    let messages = [None, Some(String::new()), Some("output".into())]
        .into_iter()
        .enumerate()
        .map(|(index, output)| {
            let mut message = AgentMessage::user(index.to_string(), 1.0);
            message.parts.push(crate::state::MessagePart::ToolCall(
                crate::state::ToolCallInfo {
                    id: index.to_string(),
                    tool: "Synthetic".into(),
                    input: serde_json::Value::Null,
                    output,
                    is_error: false,
                    started_at_ms: None,
                    ended_at_ms: None,
                },
            ));
            message
        })
        .collect();
    let record = MessagesRecord {
        thread_id: "thread".into(),
        messages,
    };
    let encoded = envelope(&record);
    assert_eq!(encoded.inputs.len(), 3);
    assert_eq!(encoded.strings.len(), 5);
    assert_eq!(encoded.record().unwrap(), record);
}

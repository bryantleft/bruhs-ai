//! Round-trip hand-written seed JSON through the real bruhs-models types.
//! Fails on any shape error; warns about keys the type silently dropped.
use bruhs_models::*;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::Value;

#[derive(Serialize, Deserialize)]
struct Thread {
    id: String,
    title: String,
    project_name: String,
    #[serde(deserialize_with = "bruhs_models::time::deserialize_legacy")]
    updated_at: EpochMs,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    project_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    branch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    model: Option<String>,
    #[serde(default)]
    pinned: bool,
    #[serde(default)]
    settled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    worktree_path: Option<String>,
}
#[derive(Serialize, Deserialize)]
struct ToolCallInfo {
    id: String,
    tool: String,
    input: Value,
    output: Option<String>,
    is_error: bool,
    #[serde(default)]
    started_at_ms: Option<f64>,
    #[serde(default)]
    ended_at_ms: Option<f64>,
}
#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
enum MessagePart {
    Text(String),
    Thinking(String),
    ToolCall(ToolCallInfo),
}
#[derive(Serialize, Deserialize)]
struct UsageInfo { input_tokens: u64, output_tokens: u64, cache_read_tokens: u64, cache_creation_tokens: u64 }
#[derive(Serialize, Deserialize)]
struct MessageRecord {
    id: String,
    role: String,
    #[serde(default)] content: String,
    #[serde(default)] tool_calls: Vec<ToolCallInfo>,
    #[serde(default)] is_streaming: bool,
    #[serde(default)] thinking: String,
    #[serde(default)] usage: Option<UsageInfo>,
    #[serde(default)] started_at_ms: Option<f64>,
    #[serde(default)] completed_at_ms: Option<f64>,
    #[serde(default)] model: Option<String>,
    #[serde(default)] parts: Vec<MessagePart>,
    #[serde(default)] error: Option<String>,
}
#[derive(Serialize, Deserialize)]
struct MessagesRecord { thread_id: String, messages: Vec<MessageRecord> }

fn missing_keys(input: &Value, output: &Value, path: &str, out: &mut Vec<String>) {
    match (input, output) {
        (Value::Object(a), Value::Object(b)) => {
            for (k, v) in a {
                match b.get(k) {
                    Some(w) => missing_keys(v, w, &format!("{path}.{k}"), out),
                    None if v.is_null() => {}
                    None => out.push(format!("{path}.{k}")),
                }
            }
        }
        (Value::Array(a), Value::Array(b)) => {
            for (i, (v, w)) in a.iter().zip(b).enumerate() {
                missing_keys(v, w, &format!("{path}[{i}]"), out);
            }
        }
        _ => {}
    }
}

fn check<T: Serialize + DeserializeOwned>(name: &str, v: &Value) -> Value {
    let typed: T = serde_json::from_value(v.clone())
        .unwrap_or_else(|e| panic!("section {name}: {e}"));
    let back = serde_json::to_value(&typed).unwrap();
    let mut dropped = Vec::new();
    missing_keys(v, &back, name, &mut dropped);
    for d in dropped {
        eprintln!("WARN dropped key {d}");
    }
    back
}

fn main() {
    let path = std::env::args().nth(1).expect("seed json path");
    let raw: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    let mut out = serde_json::Map::new();
    for (k, v) in raw.as_object().unwrap() {
        let canon = match k.as_str() {
            "projects" => check::<Vec<Project>>(k, v),
            "work_items" => check::<Vec<WorkItem>>(k, v),
            "events" => check::<Vec<CalendarEvent>>(k, v),
            "calendars" => check::<Vec<Calendar>>(k, v),
            "meetings" => check::<Vec<Meeting>>(k, v),
            "segments" => check::<Vec<TranscriptSegment>>(k, v),
            "people" => check::<Vec<PersonProfile>>(k, v),
            "relationships" => check::<Vec<RelationshipProfile>>(k, v),
            "knowledge_documents" => check::<Vec<KnowledgeDocument>>(k, v),
            "source_clips" => check::<Vec<SourceClip>>(k, v),
            "meeting_notes" => check::<Vec<MeetingNote>>(k, v),
            "life" => check::<LifeContext>(k, v),
            "threads" => check::<Vec<Thread>>(k, v),
            "messages" => check::<Vec<MessagesRecord>>(k, v),
            other => { eprintln!("WARN passthrough section {other}"); v.clone() }
        };
        // Keep the hand-written form for web-only types (their real serializer
        // writes extra derived fields we don't need), canonical form otherwise.
        let keep = if matches!(k.as_str(), "threads" | "messages") { v.clone() } else { canon };
        out.insert(k.clone(), keep);
    }
    println!("{}", serde_json::to_string(&Value::Object(out)).unwrap());
}

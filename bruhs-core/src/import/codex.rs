//! Codex sessions: `<codex home>/sessions/YYYY/MM/DD/rollout-*-<id>.jsonl`.
//! bruhs resumes the thread itself (`codex exec resume <id>`), so the
//! rollout is only read to show the thread.
//!
//! The user's prompts are `event_msg` `user_message` records in older
//! rollouts and `response_item` user messages in newer ones (which also
//! carry injected context: AGENTS.md, the environment, T3's
//! instructions, all filtered out).

use super::claude::last_model;
use super::{
    files_in, lossy_lines, newest_first, output_text, parse_time, user_text, ImportError,
    Transcript,
};
use bruhs_models::agent::AgentProvider;
use bruhs_models::logic::portable::{
    title_from_prompt, ImportCandidate, ImportSource, PortableMessage, PortablePart,
    PortableToolCall,
};
use serde_json::Value;
use std::io::BufReader;
use std::path::{Path, PathBuf};

/// T3 Code's `originator`: its threads are listed under T3.
const T3_ORIGINATOR: &str = "T3 Code";

const HEAD_LINES: usize = 400;

fn rollouts(root: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    for year in files_in(&root.join("sessions")) {
        for month in files_in(&year) {
            for day in files_in(&month) {
                found.extend(files_in(&day).into_iter().filter(|file| {
                    file.file_name()
                        .and_then(|name| name.to_str())
                        .is_some_and(|name| {
                            name.starts_with("rollout-") && name.ends_with(".jsonl")
                        })
                }));
            }
        }
    }
    found
}

/// What `session_meta` says.
#[derive(Default)]
struct Meta {
    id: Option<String>,
    cwd: Option<String>,
    originator: Option<String>,
    /// A subagent's thread (it has a parent), not one the user started.
    child: bool,
}

fn meta_of(row: &Value) -> Option<Meta> {
    if row.get("type").and_then(Value::as_str) != Some("session_meta") {
        return None;
    }
    let payload = row.get("payload")?;
    let text = |key: &str| payload.get(key).and_then(Value::as_str).map(str::to_string);
    Some(Meta {
        id: text("id"),
        cwd: text("cwd"),
        originator: text("originator"),
        child: payload
            .get("parent_thread_id")
            .is_some_and(|parent| !parent.is_null()),
    })
}

/// A user prompt in `row`: `event_msg` `user_message` when `events` (the
/// rollout has those), else a `response_item` user message.
fn prompt_of(row: &Value, events: bool) -> Option<String> {
    let payload = row.get("payload")?;
    let kind = payload.get("type").and_then(Value::as_str);
    let text = match (row.get("type").and_then(Value::as_str), kind) {
        (Some("event_msg"), Some("user_message")) if events => {
            payload.get("message").and_then(Value::as_str)?.to_string()
        }
        (Some("response_item"), Some("message"))
            if !events && payload.get("role").and_then(Value::as_str) == Some("user") =>
        {
            payload
                .get("content")
                .and_then(Value::as_array)?
                .iter()
                .filter_map(|item| item.get("text").and_then(Value::as_str))
                .map(user_text)
                .filter(|text| !text.is_empty())
                .collect::<Vec<_>>()
                .join("\n\n")
        }
        _ => return None,
    };
    let text = user_text(&text);
    (!text.is_empty()).then_some(text)
}

fn is_user_event(row: &Value) -> bool {
    row.get("type").and_then(Value::as_str) == Some("event_msg")
        && row
            .get("payload")
            .and_then(|payload| payload.get("type"))
            .and_then(Value::as_str)
            == Some("user_message")
}

pub(super) fn list(
    root: &Path,
    limit: usize,
    keep: &dyn Fn(&ImportCandidate) -> bool,
) -> Result<Vec<ImportCandidate>, ImportError> {
    let mut found = Vec::new();
    for (path, updated_at) in newest_first(rollouts(root)) {
        if found.len() >= limit {
            break;
        }
        let Ok(file) = std::fs::File::open(&path) else {
            continue;
        };
        let rows: Vec<Value> = lossy_lines(BufReader::new(file))
            .take(HEAD_LINES)
            .map_while(Result::ok)
            .filter_map(|line| serde_json::from_str(&line).ok())
            .collect();
        let Some(meta) = rows.iter().find_map(meta_of) else {
            continue;
        };
        if meta.child || meta.originator.as_deref() == Some(T3_ORIGINATOR) {
            continue;
        }
        let events = rows.iter().any(is_user_event);
        let (Some(id), Some(prompt)) =
            (meta.id, rows.iter().find_map(|row| prompt_of(row, events)))
        else {
            continue;
        };
        let candidate = ImportCandidate {
            source: ImportSource::Codex,
            title: title_from_prompt(&prompt),
            cwd: meta.cwd,
            updated_at,
            provider: Some(AgentProvider::Codex),
            session_id: Some(id.clone()),
            model: None,
            id,
        };
        if keep(&candidate) {
            found.push(candidate);
        }
    }
    Ok(found)
}

/// The rollout of thread `id`.
pub(super) fn find(root: &Path, id: &str) -> Option<PathBuf> {
    if id.is_empty() || id.contains(['/', '\\']) {
        return None;
    }
    let suffix = format!("-{id}.jsonl");
    rollouts(root).into_iter().find(|path| {
        path.file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.ends_with(&suffix))
    })
}

pub(super) fn load(
    root: &Path,
    id: &str,
) -> Result<(ImportCandidate, Vec<PortableMessage>), ImportError> {
    let path = find(root, id).ok_or_else(|| ImportError::not_found(ImportSource::Codex, id))?;
    let bytes = std::fs::read(&path).map_err(|error| ImportError::io(&path, error))?;
    let rows: Vec<Value> = String::from_utf8_lossy(&bytes)
        .lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect();
    let meta = rows.iter().find_map(meta_of).unwrap_or_default();
    let events = rows.iter().any(is_user_event);
    let mut transcript = Transcript::default();
    let mut model: Option<String> = None;
    let mut first_prompt = None;
    for row in &rows {
        let at_ms = row
            .get("timestamp")
            .and_then(Value::as_str)
            .and_then(parse_time);
        if let Some(prompt) = prompt_of(row, events) {
            first_prompt.get_or_insert_with(|| prompt.clone());
            transcript.user(prompt, at_ms);
            continue;
        }
        let Some(payload) = row.get("payload") else {
            continue;
        };
        let kind = payload.get("type").and_then(Value::as_str);
        match row.get("type").and_then(Value::as_str) {
            Some("turn_context") => {
                model = payload
                    .get("model")
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .or(model);
            }
            Some("response_item") => {
                apply_item(&mut transcript, payload, kind, at_ms, model.as_deref())
            }
            _ => {}
        }
    }
    let messages = transcript.finish();
    let candidate = ImportCandidate {
        source: ImportSource::Codex,
        id: id.to_string(),
        title: title_from_prompt(first_prompt.as_deref().unwrap_or("")),
        cwd: meta.cwd,
        updated_at: super::modified(&path),
        provider: Some(AgentProvider::Codex),
        session_id: Some(meta.id.unwrap_or_else(|| id.to_string())),
        model: model.or_else(|| last_model(&messages)),
    };
    Ok((candidate, messages))
}

fn apply_item(
    transcript: &mut Transcript,
    payload: &Value,
    kind: Option<&str>,
    at_ms: Option<i64>,
    model: Option<&str>,
) {
    let text = |key: &str| {
        payload
            .get(key)
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string()
    };
    match kind {
        Some("message") if payload.get("role").and_then(Value::as_str) == Some("assistant") => {
            let body = payload
                .get("content")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|item| item.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("\n\n");
            if !body.trim().is_empty() {
                transcript.part(PortablePart::Text(body), at_ms, model);
            }
        }
        Some("reasoning") => {
            let summary = payload
                .get("summary")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|item| item.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("\n\n");
            if !summary.trim().is_empty() {
                transcript.part(PortablePart::Thinking(summary), at_ms, model);
            }
        }
        Some("function_call" | "custom_tool_call" | "local_shell_call") => {
            let input = match kind {
                Some("function_call") => {
                    let raw = text("arguments");
                    serde_json::from_str(&raw).unwrap_or(Value::String(raw))
                }
                Some("custom_tool_call") => Value::String(text("input")),
                _ => payload.get("action").cloned().unwrap_or(Value::Null),
            };
            let tool = Some(text("name"))
                .filter(|name| !name.is_empty())
                .unwrap_or_else(|| "shell".to_string());
            transcript.part(
                PortablePart::ToolCall(PortableToolCall {
                    id: text("call_id"),
                    tool,
                    input,
                    output: None,
                    is_error: false,
                }),
                at_ms,
                model,
            );
        }
        Some("function_call_output" | "custom_tool_call_output" | "local_shell_call_output") => {
            let output = payload.get("output").and_then(|output| {
                // function_call_output wraps its text as {"content": …}.
                output_text(output.get("content").unwrap_or(output))
            });
            transcript.tool_output(&text("call_id"), output, false);
        }
        _ => {}
    }
}

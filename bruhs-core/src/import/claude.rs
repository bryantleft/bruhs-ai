//! Claude Code sessions: `<config>/projects/<cwd as dashes>/<session>.jsonl`,
//! one JSON record per line. bruhs resumes the session itself (`claude
//! --resume <id>` from the same directory), so the transcript is only read
//! to show the thread.

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

/// T3 Code runs Claude with this entrypoint; its threads are listed under
/// T3, with T3's titles.
const T3_ENTRYPOINT: &str = "t3-code";

/// Lines read from the top of a transcript to list it: enough for the
/// directory, the title and the first prompt.
const HEAD_LINES: usize = 400;

fn sessions(root: &Path) -> Vec<PathBuf> {
    files_in(&root.join("projects"))
        .into_iter()
        .filter(|dir| dir.is_dir())
        .flat_map(|dir| files_in(&dir))
        .filter(|file| file.extension().is_some_and(|ext| ext == "jsonl"))
        .collect()
}

/// What the top of a transcript says about it.
#[derive(Default)]
struct Head {
    cwd: Option<String>,
    entrypoint: Option<String>,
    title: Option<String>,
    prompt: Option<String>,
}

fn read_head(path: &Path) -> Option<Head> {
    let file = std::fs::File::open(path).ok()?;
    let mut head = Head::default();
    for line in lossy_lines(BufReader::new(file)).take(HEAD_LINES) {
        let Ok(line) = line else { break };
        let Ok(row) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        if head.cwd.is_none() {
            head.cwd = str_field(&row, "cwd");
        }
        if head.entrypoint.is_none() {
            head.entrypoint = str_field(&row, "entrypoint");
        }
        if let Some(title) = title_of(&row) {
            head.title = Some(title);
        }
        if head.prompt.is_none() {
            head.prompt = prompt_of(&row);
        }
        if head.prompt.is_some() && head.cwd.is_some() {
            break;
        }
    }
    Some(head)
}

/// Bytes read from the end of a transcript to find its newest title.
/// Claude Code repeats the title record as the session goes on, so the
/// newest one is near the end; reading only the end keeps a listing of
/// long sessions fast.
const TAIL_BYTES: u64 = 256 * 1024;

fn title_of(row: &Value) -> Option<String> {
    (row.get("type").and_then(Value::as_str) == Some("custom-title"))
        .then(|| str_field(row, "customTitle"))
        .flatten()
        .filter(|title| !title.is_empty())
}

/// The newest `custom-title` in the last [`TAIL_BYTES`] of `path`.
fn last_title(path: &Path) -> Option<String> {
    use std::io::{Read as _, Seek as _, SeekFrom};
    let mut file = std::fs::File::open(path).ok()?;
    let length = file.metadata().ok()?.len();
    file.seek(SeekFrom::Start(length.saturating_sub(TAIL_BYTES)))
        .ok()?;
    let mut tail = Vec::new();
    file.read_to_end(&mut tail).ok()?;
    String::from_utf8_lossy(&tail)
        .lines()
        .rev()
        .filter(|line| line.contains("\"custom-title\""))
        .find_map(|line| title_of(&serde_json::from_str(line).ok()?))
}

pub(super) fn list(
    root: &Path,
    limit: usize,
    keep: &dyn Fn(&ImportCandidate) -> bool,
) -> Result<Vec<ImportCandidate>, ImportError> {
    let mut found = Vec::new();
    for (path, updated_at) in newest_first(sessions(root)) {
        if found.len() >= limit {
            break;
        }
        let Some(head) = read_head(&path) else {
            continue;
        };
        if head.entrypoint.as_deref() == Some(T3_ENTRYPOINT) {
            continue;
        }
        let Some(prompt) = head.prompt else {
            continue;
        };
        let Some(id) = session_id_of(&path) else {
            continue;
        };
        let candidate = ImportCandidate {
            source: ImportSource::ClaudeCode,
            title: last_title(&path)
                .or(head.title)
                .unwrap_or_else(|| title_from_prompt(&prompt)),
            cwd: head.cwd,
            updated_at,
            provider: Some(AgentProvider::Claude),
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

fn session_id_of(path: &Path) -> Option<String> {
    path.file_stem()
        .and_then(|stem| stem.to_str())
        .map(str::to_string)
}

/// The transcript of session `id`.
pub(super) fn find(root: &Path, id: &str) -> Option<PathBuf> {
    if id.is_empty() || id.contains(['/', '\\']) || id.starts_with('.') {
        return None;
    }
    files_in(&root.join("projects"))
        .into_iter()
        .map(|dir| dir.join(format!("{id}.jsonl")))
        .find(|path| path.is_file())
}

pub(super) fn load(
    root: &Path,
    id: &str,
) -> Result<(ImportCandidate, Vec<PortableMessage>), ImportError> {
    let path =
        find(root, id).ok_or_else(|| ImportError::not_found(ImportSource::ClaudeCode, id))?;
    let file = std::fs::File::open(&path).map_err(|error| ImportError::io(&path, error))?;
    let mut transcript = Transcript::default();
    let (mut cwd, mut title, mut first_prompt) = (None, None, None);
    for line in lossy_lines(BufReader::new(file)) {
        let line = line.map_err(|error| ImportError::io(&path, error))?;
        let Ok(row) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        if cwd.is_none() {
            cwd = str_field(&row, "cwd");
        }
        if let Some(found) = title_of(&row) {
            title = Some(found);
        }
        apply(&mut transcript, &row, &mut first_prompt);
    }
    let messages = transcript.finish();
    let candidate = ImportCandidate {
        source: ImportSource::ClaudeCode,
        id: id.to_string(),
        title: title
            .or_else(|| first_prompt.as_deref().map(title_from_prompt))
            .unwrap_or_else(|| title_from_prompt("")),
        cwd,
        updated_at: super::modified(&path),
        provider: Some(AgentProvider::Claude),
        session_id: Some(id.to_string()),
        model: last_model(&messages),
    };
    Ok((candidate, messages))
}

pub(super) fn last_model(messages: &[PortableMessage]) -> Option<String> {
    messages
        .iter()
        .rev()
        .find_map(|message| message.model.clone())
}

fn str_field(row: &Value, key: &str) -> Option<String> {
    row.get(key).and_then(Value::as_str).map(str::to_string)
}

/// A record that is not part of the visible conversation: a subagent's,
/// a meta note, or a compaction summary.
fn hidden(row: &Value) -> bool {
    let flag = |key: &str| row.get(key).and_then(Value::as_bool).unwrap_or(false);
    flag("isSidechain") || flag("isMeta") || flag("isCompactSummary")
}

/// The typed text of a user record, if it is a prompt (not tool results).
fn prompt_of(row: &Value) -> Option<String> {
    if row.get("type").and_then(Value::as_str) != Some("user") || hidden(row) {
        return None;
    }
    let content = row.get("message")?.get("content")?;
    let text = match content {
        Value::String(text) => text.clone(),
        Value::Array(blocks) => blocks
            .iter()
            .filter(|block| block.get("type").and_then(Value::as_str) == Some("text"))
            .filter_map(|block| block.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => return None,
    };
    let text = user_text(&text);
    (!text.is_empty()).then_some(text)
}

fn apply(transcript: &mut Transcript, row: &Value, first_prompt: &mut Option<String>) {
    if hidden(row) {
        return;
    }
    let at_ms = row
        .get("timestamp")
        .and_then(Value::as_str)
        .and_then(parse_time);
    let Some(message) = row.get("message") else {
        return;
    };
    match row.get("type").and_then(Value::as_str) {
        Some("user") => {
            if let Some(blocks) = message.get("content").and_then(Value::as_array) {
                for block in blocks {
                    if block.get("type").and_then(Value::as_str) == Some("tool_result") {
                        let id = block
                            .get("tool_use_id")
                            .and_then(Value::as_str)
                            .unwrap_or("");
                        let output = block.get("content").and_then(output_text);
                        let is_error = block
                            .get("is_error")
                            .and_then(Value::as_bool)
                            .unwrap_or(false);
                        transcript.tool_output(id, output, is_error);
                    }
                }
            }
            if let Some(prompt) = prompt_of(row) {
                first_prompt.get_or_insert_with(|| prompt.clone());
                transcript.user(prompt, at_ms);
            }
        }
        Some("assistant") => {
            let model = message.get("model").and_then(Value::as_str);
            for block in message
                .get("content")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                let part = match block.get("type").and_then(Value::as_str) {
                    Some("text") => block
                        .get("text")
                        .and_then(Value::as_str)
                        .filter(|text| !text.trim().is_empty())
                        .map(|text| PortablePart::Text(text.to_string())),
                    Some("thinking") => block
                        .get("thinking")
                        .and_then(Value::as_str)
                        .filter(|text| !text.trim().is_empty())
                        .map(|text| PortablePart::Thinking(text.to_string())),
                    Some("tool_use") => Some(PortablePart::ToolCall(PortableToolCall {
                        id: block
                            .get("id")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string(),
                        tool: block
                            .get("name")
                            .and_then(Value::as_str)
                            .unwrap_or("tool")
                            .to_string(),
                        input: block.get("input").cloned().unwrap_or(Value::Null),
                        output: None,
                        is_error: false,
                    })),
                    _ => None,
                };
                if let Some(part) = part {
                    transcript.part(part, at_ms, model);
                }
            }
        }
        _ => {}
    }
}

//! Cursor agent transcripts: `<~/.cursor>/projects/<folder>/agent-transcripts/`
//! holds one JSONL file per agent chat, flat (`<id>.jsonl`) or nested
//! (`<id>/<id>.jsonl`). The Cursor Agent CLI (`cursor-agent`) writes its
//! chats only here (its own `chats/*/store.db` keeps them as protobuf);
//! the editor writes here too, for chats its `state.vscdb` already has, so
//! those are listed from the database instead.
//!
//! Each line is `{"role": "user"|"assistant", "message": {"content": [...]}}`
//! with `text` and `tool_use` blocks. `<folder>` is the chat's directory
//! with every character that is not a letter or digit turned into `-`, so
//! it is found again by walking the file system.

use super::{lossy_lines, user_text, without_blocks, ImportError, Transcript};
use bruhs_models::agent::Role;
use bruhs_models::logic::portable::{
    title_from_prompt, ImportCandidate, ImportSource, PortableMessage, PortablePart,
    PortableToolCall,
};
use bruhs_models::EpochMs;
use serde_json::Value;
use std::io::BufReader;
use std::path::{Path, PathBuf};

/// Lines read from the top of a transcript to find its first prompt.
const HEAD_LINES: usize = 50;

/// Most directories a folder name may match before it counts as unknown.
const MATCH_LIMIT: usize = 2;

/// One agent transcript on disk.
pub(super) struct AgentTranscript {
    pub(super) id: String,
    pub(super) path: PathBuf,
    /// The encoded name of the chat's directory.
    pub(super) folder: String,
    pub(super) updated_at: EpochMs,
}

/// Every top-level agent transcript under `home` (`~/.cursor`). A
/// subagent's transcript (`<id>/subagents/`) belongs to its parent's chat
/// and is not listed.
pub(super) fn transcripts(home: &Path) -> Vec<AgentTranscript> {
    let mut found = Vec::new();
    for project in super::files_in(&home.join("projects")) {
        let Some(folder) = project.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        for entry in super::files_in(&project.join("agent-transcripts")) {
            let path = if entry.is_dir() {
                let Some(id) = entry.file_name().and_then(|name| name.to_str()) else {
                    continue;
                };
                entry.join(format!("{id}.jsonl"))
            } else {
                entry
            };
            if path.extension().is_none_or(|ext| ext != "jsonl") || !path.is_file() {
                continue;
            }
            let Some(id) = path.file_stem().and_then(|stem| stem.to_str()) else {
                continue;
            };
            found.push(AgentTranscript {
                id: id.to_string(),
                updated_at: super::modified(&path),
                folder: folder.to_string(),
                path,
            });
        }
    }
    found
}

/// The transcript of chat `id`.
pub(super) fn find(home: &Path, id: &str) -> Option<AgentTranscript> {
    if id.is_empty() || id.contains(['/', '\\']) || id.starts_with('.') {
        return None;
    }
    transcripts(home)
        .into_iter()
        .find(|transcript| transcript.id == id)
}

/// `transcript` as a row of the import picker, in directory `cwd`; `None`
/// when it has no prompt yet.
pub(super) fn candidate(
    transcript: &AgentTranscript,
    cwd: Option<String>,
) -> Option<ImportCandidate> {
    let file = std::fs::File::open(&transcript.path).ok()?;
    let prompt = lossy_lines(BufReader::new(file))
        .take(HEAD_LINES)
        .map_while(Result::ok)
        .filter_map(|line| serde_json::from_str::<Value>(&line).ok())
        .find_map(|row| prompt_of(&row))?;
    Some(ImportCandidate {
        source: ImportSource::Cursor,
        id: transcript.id.clone(),
        title: title_from_prompt(&prompt),
        cwd,
        updated_at: transcript.updated_at,
        provider: None,
        session_id: None,
        model: None,
    })
}

pub(super) fn load(
    transcript: &AgentTranscript,
) -> Result<(ImportCandidate, Vec<PortableMessage>), ImportError> {
    let path = &transcript.path;
    let file = std::fs::File::open(path).map_err(|error| ImportError::io(path, error))?;
    let mut messages = Transcript::default();
    for line in lossy_lines(BufReader::new(file)) {
        let line = line.map_err(|error| ImportError::io(path, error))?;
        let Ok(row) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        if let Some(prompt) = prompt_of(&row) {
            messages.user(prompt, None);
        } else if row.get("role").and_then(Value::as_str) == Some("assistant") {
            for part in content(&row).iter().filter_map(assistant_part) {
                messages.part(part, None, None);
            }
        }
    }
    let messages = messages.finish();
    let first_prompt = messages
        .iter()
        .find(|message| message.role == Role::User)
        .map(PortableMessage::text)
        .unwrap_or_default();
    let candidate = ImportCandidate {
        source: ImportSource::Cursor,
        id: transcript.id.clone(),
        title: title_from_prompt(&first_prompt),
        cwd: folder_path(&transcript.folder),
        updated_at: transcript.updated_at,
        provider: None,
        session_id: None,
        model: None,
    };
    Ok((candidate, messages))
}

fn content(row: &Value) -> Vec<Value> {
    match row
        .get("message")
        .and_then(|message| message.get("content"))
    {
        Some(Value::Array(blocks)) => blocks.clone(),
        Some(Value::String(text)) => vec![serde_json::json!({"type": "text", "text": text})],
        _ => Vec::new(),
    }
}

/// What the user typed in a `user` row: the `<user_query>` Cursor wraps
/// it in, without the timestamp and attachments it adds around it.
fn prompt_of(row: &Value) -> Option<String> {
    if row.get("role").and_then(Value::as_str) != Some("user") {
        return None;
    }
    let text = content(row)
        .iter()
        .filter(|block| block.get("type").and_then(Value::as_str) == Some("text"))
        .filter_map(|block| block.get("text").and_then(Value::as_str))
        .map(query_of)
        .filter(|text| !text.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n");
    (!text.is_empty()).then_some(text)
}

fn query_of(text: &str) -> String {
    let inner = text
        .split_once("<user_query>")
        .and_then(|(_, rest)| rest.split_once("</user_query>"))
        .map_or(text, |(query, _)| query);
    user_text(&without_blocks(inner, "timestamp"))
}

fn assistant_part(block: &Value) -> Option<PortablePart> {
    let text = |key: &str| block.get(key).and_then(Value::as_str);
    match text("type")? {
        "text" => text("text")
            .filter(|body| !body.trim().is_empty())
            .map(|body| PortablePart::Text(body.to_string())),
        "thinking" => text("thinking")
            .filter(|body| !body.trim().is_empty())
            .map(|body| PortablePart::Thinking(body.to_string())),
        "tool_use" => Some(PortablePart::ToolCall(PortableToolCall {
            id: text("id").unwrap_or("").to_string(),
            tool: text("name").unwrap_or("tool").to_string(),
            input: block.get("input").cloned().unwrap_or(Value::Null),
            output: None,
            is_error: false,
        })),
        _ => None,
    }
}

/// The words of a path as Cursor encodes it: split on anything that is
/// not a letter or digit.
fn words(text: &str) -> Vec<&str> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .collect()
}

/// The directory `folder` names, when exactly one exists.
pub(super) fn folder_path(folder: &str) -> Option<String> {
    folder_path_in(Path::new("/"), folder)
}

fn folder_path_in(root: &Path, folder: &str) -> Option<String> {
    let wanted = words(folder);
    if wanted.is_empty() {
        return None;
    }
    let mut matches = Vec::new();
    walk(root, &wanted, &mut matches);
    match matches.as_slice() {
        [only] => Some(only.display().to_string()),
        _ => None,
    }
}

/// Directories under `dir` whose names spell out `rest`, word by word.
fn walk(dir: &Path, rest: &[&str], matches: &mut Vec<PathBuf>) {
    if rest.is_empty() {
        matches.push(dir.to_path_buf());
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        if matches.len() >= MATCH_LIMIT {
            return;
        }
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        let name_words = words(name);
        if name_words.is_empty() || !rest.starts_with(&name_words) {
            continue;
        }
        let path = entry.path();
        if path.is_dir() {
            walk(&path, &rest[name_words.len()..], matches);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_folder_name_finds_its_directory_when_only_one_matches() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join("Users/me/my-app")).unwrap();
        std::fs::create_dir_all(root.path().join("Users/me/my_app.v2")).unwrap();
        let found = folder_path_in(root.path(), "Users-me-my-app").unwrap();
        assert!(found.ends_with("Users/me/my-app"), "{found}");
        assert!(folder_path_in(root.path(), "Users-me-my-app-v2")
            .unwrap()
            .ends_with("my_app.v2"));
        assert_eq!(folder_path_in(root.path(), "Users-me-gone"), None);

        // Two directories that encode the same: unknown.
        std::fs::create_dir_all(root.path().join("Users/me/my/app")).unwrap();
        assert_eq!(folder_path_in(root.path(), "Users-me-my-app"), None);
    }

    #[test]
    fn a_prompt_is_the_users_query_without_cursors_wrapping() {
        assert_eq!(
            query_of("<timestamp>Friday</timestamp>\n<user_query>\nfix it @a.json\n</user_query>"),
            "fix it @a.json"
        );
        assert_eq!(query_of("<timestamp>Friday</timestamp>\nplain"), "plain");
    }
}

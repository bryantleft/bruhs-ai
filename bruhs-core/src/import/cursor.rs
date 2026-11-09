//! Cursor chats ("composers"), from `<Cursor User>/globalStorage/
//! state.vscdb`, opened read-only: `composerData:<id>` holds a chat's
//! name and the order of its messages, `bubbleId:<id>:<bubble>` each
//! message. Which folder a chat belongs to is in the workspace's own
//! `workspaceStorage/<hash>/state.vscdb` (`composer.composerData`) next to
//! its `workspace.json`.
//!
//! Cursor has no session another tool can resume, so a Cursor chat comes
//! in as a transcript (see `ImportedThread::context`).

use super::{files_in, ImportError};
use bruhs_models::agent::Role;
use bruhs_models::logic::portable::{
    title_from_prompt, ImportCandidate, ImportSource, PortableMessage, PortablePart,
    PortableToolCall,
};
use bruhs_models::EpochMs;
use rusqlite::{Connection, OpenFlags, OptionalExtension};
use serde_json::Value;
use std::collections::HashMap;
use std::path::Path;

fn open(path: &Path) -> Result<Option<Connection>, ImportError> {
    if !path.is_file() {
        return Ok(None);
    }
    let connection = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|error| ImportError::Database("Cursor", error.to_string()))?;
    let _ = connection.busy_timeout(std::time::Duration::from_secs(2));
    Ok(Some(connection))
}

fn global(root: &Path) -> Result<Option<Connection>, ImportError> {
    open(&root.join("globalStorage").join("state.vscdb"))
}

fn unreadable(error: rusqlite::Error) -> ImportError {
    ImportError::Database("Cursor", error.to_string())
}

/// The value under `key` in Cursor's key-value table.
fn get(connection: &Connection, key: &str) -> Option<Value> {
    let text: Option<String> = connection
        .query_row(
            "SELECT value FROM cursorDiskKV WHERE key = ?1",
            [key],
            |row| row.get(0),
        )
        .optional()
        .ok()
        .flatten();
    serde_json::from_str(&text?).ok()
}

/// Folder of each chat, from every workspace that lists it.
fn folders(root: &Path) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for dir in files_in(&root.join("workspaceStorage")) {
        let Some(folder) = std::fs::read_to_string(dir.join("workspace.json"))
            .ok()
            .and_then(|text| serde_json::from_str::<Value>(&text).ok())
            .and_then(|json| json.get("folder")?.as_str().map(str::to_string))
            .and_then(|uri| url::Url::parse(&uri).ok()?.to_file_path().ok())
            .map(|path| path.display().to_string())
        else {
            continue;
        };
        let Ok(Some(connection)) = open(&dir.join("state.vscdb")) else {
            continue;
        };
        let listed: Option<String> = connection
            .query_row(
                "SELECT value FROM ItemTable WHERE key = 'composer.composerData'",
                [],
                |row| row.get(0),
            )
            .optional()
            .ok()
            .flatten();
        let Some(listed) = listed.and_then(|text| serde_json::from_str::<Value>(&text).ok()) else {
            continue;
        };
        for composer in listed
            .get("allComposers")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            if let Some(id) = composer.get("composerId").and_then(Value::as_str) {
                map.insert(id.to_string(), folder.clone());
            }
        }
    }
    map
}

/// A chat's bubbles in order, as (id, type) where type 1 is the user's.
fn headers(composer: &Value) -> Vec<(String, i64)> {
    composer
        .get("fullConversationHeadersOnly")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|header| {
            Some((
                header.get("bubbleId")?.as_str()?.to_string(),
                header.get("type")?.as_i64()?,
            ))
        })
        .collect()
}

fn millis(value: Option<&Value>) -> Option<i64> {
    match value? {
        Value::Number(number) => number.as_i64(),
        Value::String(text) => super::parse_time(text),
        _ => None,
    }
}

fn is_listable(composer: &Value) -> bool {
    let flag = |key: &str| composer.get(key).and_then(Value::as_bool).unwrap_or(false);
    !flag("isBestOfNSubcomposer") && !headers(composer).is_empty()
}

/// The newest `limit` chats `keep` accepts. Every chat's header is read
/// (it holds the time), but a title that needs its first message is looked
/// up only on the way to `limit`.
pub(super) fn list(
    root: &Path,
    limit: usize,
    keep: &dyn Fn(&ImportCandidate) -> bool,
) -> Result<Vec<ImportCandidate>, ImportError> {
    let Some(connection) = global(root)? else {
        return Ok(Vec::new());
    };
    let folders = folders(root);
    let mut statement = connection
        .prepare("SELECT key, value FROM cursorDiskKV WHERE key LIKE 'composerData:%'")
        .map_err(unreadable)?;
    let composers: Vec<(String, String)> = statement
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .map_err(unreadable)?
        .filter_map(Result::ok)
        .collect();
    let mut chats: Vec<(String, Value, EpochMs)> = composers
        .into_iter()
        .filter_map(|(key, value)| {
            let composer = serde_json::from_str::<Value>(&value).ok()?;
            is_listable(&composer).then(|| {
                let updated_at = millis(composer.get("lastUpdatedAt"))
                    .or_else(|| millis(composer.get("createdAt")))
                    .map_or(EpochMs::UNIX_EPOCH, EpochMs::saturating_from_i64);
                let id = key.trim_start_matches("composerData:").to_string();
                (id, composer, updated_at)
            })
        })
        .collect();
    chats.sort_by_key(|(_, _, updated_at)| std::cmp::Reverse(*updated_at));
    let mut found = Vec::new();
    for (id, composer, updated_at) in chats {
        if found.len() >= limit {
            break;
        }
        let candidate = ImportCandidate {
            source: ImportSource::Cursor,
            title: title_of(&connection, &id, &composer),
            cwd: folders.get(&id).cloned(),
            updated_at,
            provider: None,
            session_id: None,
            model: None,
            id,
        };
        if keep(&candidate) {
            found.push(candidate);
        }
    }
    Ok(found)
}

/// The chat's name, else its first prompt.
fn title_of(connection: &Connection, id: &str, composer: &Value) -> String {
    if let Some(name) = composer
        .get("name")
        .and_then(Value::as_str)
        .filter(|name| !name.trim().is_empty())
    {
        return name.trim().to_string();
    }
    let first = headers(composer)
        .into_iter()
        .find(|(_, kind)| *kind == 1)
        .and_then(|(bubble, _)| get(connection, &format!("bubbleId:{id}:{bubble}")))
        .and_then(|bubble| bubble.get("text")?.as_str().map(str::to_string))
        .unwrap_or_default();
    title_from_prompt(&first)
}

pub(super) fn load(
    root: &Path,
    id: &str,
) -> Result<(ImportCandidate, Vec<PortableMessage>), ImportError> {
    let not_found = || ImportError::not_found(ImportSource::Cursor, id);
    let connection = global(root)?.ok_or_else(not_found)?;
    let composer = get(&connection, &format!("composerData:{id}")).ok_or_else(not_found)?;
    let mut transcript = super::Transcript::default();
    for (bubble_id, kind) in headers(&composer) {
        let Some(bubble) = get(&connection, &format!("bubbleId:{id}:{bubble_id}")) else {
            continue;
        };
        let at_ms = millis(bubble.get("createdAt"));
        let text = bubble
            .get("text")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_string();
        if kind == 1 {
            if !text.is_empty() {
                transcript.user(text, at_ms);
            }
            continue;
        }
        if let Some(thinking) = bubble
            .get("thinking")
            .and_then(|thinking| thinking.get("text"))
            .and_then(Value::as_str)
            .filter(|text| !text.trim().is_empty())
        {
            transcript.part(PortablePart::Thinking(thinking.to_string()), at_ms, None);
        }
        if let Some(tool) = bubble.get("toolFormerData").filter(|tool| tool.is_object()) {
            let field = |key: &str| tool.get(key).and_then(Value::as_str).unwrap_or("");
            let raw = field("rawArgs");
            transcript.part(
                PortablePart::ToolCall(PortableToolCall {
                    id: field("toolCallId").to_string(),
                    tool: Some(field("name"))
                        .filter(|name| !name.is_empty())
                        .unwrap_or("tool")
                        .to_string(),
                    input: serde_json::from_str(raw).unwrap_or(Value::String(raw.to_string())),
                    output: Some(field("result").to_string()).filter(|out| !out.is_empty()),
                    is_error: field("status") == "error",
                }),
                at_ms,
                None,
            );
        }
        if !text.is_empty() {
            transcript.part(PortablePart::Text(text), at_ms, None);
        }
    }
    let messages = transcript.finish();
    let first_prompt = messages
        .iter()
        .find(|message| message.role == Role::User)
        .map(PortableMessage::text)
        .unwrap_or_default();
    let candidate = ImportCandidate {
        source: ImportSource::Cursor,
        id: id.to_string(),
        title: composer
            .get("name")
            .and_then(Value::as_str)
            .filter(|name| !name.trim().is_empty())
            .map_or_else(
                || title_from_prompt(&first_prompt),
                |name| name.trim().to_string(),
            ),
        cwd: folders(root).get(id).cloned(),
        updated_at: millis(composer.get("lastUpdatedAt"))
            .map_or(EpochMs::UNIX_EPOCH, EpochMs::saturating_from_i64),
        provider: None,
        session_id: None,
        model: None,
    };
    Ok((candidate, messages))
}

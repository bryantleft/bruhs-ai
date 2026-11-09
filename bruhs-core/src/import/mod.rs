//! Threads from the other agent tools on this machine: listing them and
//! reading one as a bruhs thread. See [`bruhs_models::logic::portable`] for
//! what moves and how.
//!
//! Every reader opens the other tool's files read-only and never writes to
//! them. The formats are the tools' private ones (Claude Code transcripts,
//! Codex rollouts, T3's state database, Cursor's `state.vscdb`), so a reader
//! skips records it does not understand instead of failing the import.
//! Everything here is blocking file I/O: async callers use
//! `spawn_blocking`.

mod claude;
mod codex;
mod cursor;
mod export;
mod t3;

pub use export::export_to_t3;

use bruhs_models::logic::portable::{ImportCandidate, ImportSource, ImportedThread};
use bruhs_models::EpochMs;
use std::path::{Path, PathBuf};

/// Where each tool keeps its threads.
#[derive(Debug, Clone)]
pub struct ImportRoots {
    /// Claude Code's config dir (`~/.claude`, or `CLAUDE_CONFIG_DIR`).
    pub claude: PathBuf,
    /// Codex's home (`~/.codex`, or `CODEX_HOME`).
    pub codex: PathBuf,
    /// T3 Code's data dir (`~/.t3`, or `T3CODE_HOME`).
    pub t3: PathBuf,
    /// Cursor's `User` dir.
    pub cursor: PathBuf,
}

impl ImportRoots {
    /// The default locations, with each tool's own override variable.
    pub fn from_env() -> Self {
        let home = directories::BaseDirs::new()
            .map(|dirs| dirs.home_dir().to_path_buf())
            .unwrap_or_default();
        let var = |name: &str| std::env::var_os(name).filter(|value| !value.is_empty());
        let cursor = directories::BaseDirs::new().map_or_else(
            || home.join(".config/Cursor/User"),
            |dirs| dirs.config_dir().join("Cursor").join("User"),
        );
        Self {
            claude: var("CLAUDE_CONFIG_DIR").map_or_else(|| home.join(".claude"), PathBuf::from),
            codex: var("CODEX_HOME").map_or_else(|| home.join(".codex"), PathBuf::from),
            t3: var("T3CODE_HOME").map_or_else(|| home.join(".t3"), PathBuf::from),
            cursor,
        }
    }

    /// Every root under one directory, laid out like a home directory
    /// (tests).
    pub fn under(home: &Path) -> Self {
        Self {
            claude: home.join(".claude"),
            codex: home.join(".codex"),
            t3: home.join(".t3"),
            cursor: home.join("Cursor/User"),
        }
    }
}

/// Why a listing or an import failed.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ImportError {
    #[error("no {tool} thread with id {id}")]
    NotFound { tool: &'static str, id: String },
    #[error("cannot read {path}: {error}")]
    Io { path: String, error: std::io::Error },
    #[error("{0}'s database could not be read: {1}")]
    Database(&'static str, String),
}

impl ImportError {
    fn not_found(source: ImportSource, id: &str) -> Self {
        Self::NotFound {
            tool: source.label(),
            id: id.to_string(),
        }
    }

    fn io(path: &Path, error: std::io::Error) -> Self {
        Self::Io {
            path: path.display().to_string(),
            error,
        }
    }
}

/// Default and largest number of candidates a listing returns.
pub const DEFAULT_LIMIT: usize = 100;
pub const MAX_LIMIT: usize = 500;

/// The threads `source` has, newest first, at most `limit`, keeping those
/// whose title or directory contains `query` (any case). A tool that is
/// not installed has none.
///
/// # Errors
///
/// When the tool's store exists but cannot be read.
pub fn list(
    roots: &ImportRoots,
    source: ImportSource,
    query: Option<&str>,
    limit: Option<usize>,
) -> Result<Vec<ImportCandidate>, ImportError> {
    let limit = limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT);
    let query = query
        .map(str::trim)
        .filter(|q| !q.is_empty())
        .map(str::to_lowercase);
    let keep = |candidate: &ImportCandidate| {
        query.as_deref().is_none_or(|q| {
            candidate.title.to_lowercase().contains(q)
                || candidate
                    .cwd
                    .as_deref()
                    .is_some_and(|cwd| cwd.to_lowercase().contains(q))
        })
    };
    // A session T3 runs is listed under T3, with T3's title. Older T3
    // builds left no mark on the session itself, so ask T3's database.
    let t3_sessions: std::collections::HashSet<String> = match source {
        ImportSource::ClaudeCode | ImportSource::Codex => t3::list(&roots.t3, &|_| true)
            .unwrap_or_default()
            .into_iter()
            .filter_map(|candidate| candidate.session_id)
            .collect(),
        ImportSource::T3 | ImportSource::Cursor => std::collections::HashSet::new(),
    };
    let keep = |candidate: &ImportCandidate| {
        keep(candidate)
            && candidate
                .session_id
                .as_ref()
                .is_none_or(|session| !t3_sessions.contains(session))
    };
    let mut found = match source {
        ImportSource::ClaudeCode => claude::list(&roots.claude, limit, &keep)?,
        ImportSource::Codex => codex::list(&roots.codex, limit, &keep)?,
        ImportSource::T3 => t3::list(&roots.t3, &keep)?,
        ImportSource::Cursor => cursor::list(&roots.cursor, limit, &keep)?,
    };
    found.sort_by_key(|candidate| std::cmp::Reverse(candidate.updated_at));
    found.truncate(limit);
    Ok(found)
}

/// The thread `id` from `source`, with its messages.
///
/// # Errors
///
/// [`ImportError::NotFound`] when the source has no such thread, else when
/// its store cannot be read.
pub fn load(
    roots: &ImportRoots,
    source: ImportSource,
    id: &str,
) -> Result<ImportedThread, ImportError> {
    let (candidate, messages) = match source {
        ImportSource::ClaudeCode => claude::load(&roots.claude, id)?,
        ImportSource::Codex => codex::load(&roots.codex, id)?,
        ImportSource::T3 => t3::load(roots, id)?,
        ImportSource::Cursor => cursor::load(&roots.cursor, id)?,
    };
    Ok(ImportedThread::new(candidate, messages))
}

/// An RFC 3339 timestamp (`2026-11-08T01:40:28.550Z`) as epoch ms.
fn parse_time(text: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(text)
        .ok()
        .map(|time| time.timestamp_millis())
}

/// When `path` was last written.
fn modified(path: &Path) -> EpochMs {
    std::fs::metadata(path)
        .and_then(|meta| meta.modified())
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .and_then(|elapsed| i64::try_from(elapsed.as_millis()).ok())
        .map_or(EpochMs::UNIX_EPOCH, EpochMs::saturating_from_i64)
}

/// Files directly in `dir` (none when it does not exist).
/// The lines of a JSONL transcript. A line that is not UTF-8 comes through
/// with replacement characters (its JSON then fails to parse and is
/// skipped) instead of ending the read.
fn lossy_lines(reader: impl std::io::BufRead) -> impl Iterator<Item = std::io::Result<String>> {
    reader
        .split(b'\n')
        .map(|line| line.map(|bytes| String::from_utf8_lossy(&bytes).into_owned()))
}

fn files_in(dir: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .map(|entry| entry.path())
                .collect()
        })
        .unwrap_or_default()
}

/// `paths`, newest first by modification time.
fn newest_first(paths: Vec<PathBuf>) -> Vec<(PathBuf, EpochMs)> {
    let mut dated: Vec<_> = paths
        .into_iter()
        .map(|path| {
            let time = modified(&path);
            (path, time)
        })
        .collect();
    dated.sort_by_key(|(_, time)| std::cmp::Reverse(*time));
    dated
}

/// The text of a prompt with the tools' injected blocks removed:
/// `<system-reminder>`s, command caveats and the like. Empty when nothing
/// the user typed is left.
fn user_text(text: &str) -> String {
    let mut out = text.to_string();
    for tag in ["system-reminder", "bruhs_harness"] {
        out = without_blocks(&out, tag);
    }
    let out = out.trim();
    let injected = [
        "<command-",
        "<local-command",
        "Caveat: The messages below",
        "# AGENTS.md instructions",
        "<environment_context>",
        "<user_instructions>",
        "<INSTRUCTIONS>",
        "<turn_aborted>",
    ];
    if injected.iter().any(|prefix| out.starts_with(prefix)) {
        String::new()
    } else {
        out.to_string()
    }
}

/// `text` without its `<tag>…</tag>` blocks (an unclosed one runs to the
/// end).
fn without_blocks(text: &str, tag: &str) -> String {
    let (open, close) = (format!("<{tag}>"), format!("</{tag}>"));
    let mut out = String::new();
    let mut rest = text;
    while let Some(start) = rest.find(&open) {
        out.push_str(&rest[..start]);
        rest = rest[start..]
            .find(&close)
            .map_or("", |end| &rest[start + end + close.len()..]);
    }
    out.push_str(rest);
    out
}

/// A tool output as text: a string, or the text items of a content array.
fn output_text(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::String(text) => Some(text.clone()),
        serde_json::Value::Array(items) => {
            let texts: Vec<&str> = items
                .iter()
                .filter_map(|item| item.get("text").and_then(serde_json::Value::as_str))
                .collect();
            (!texts.is_empty()).then(|| texts.join("\n"))
        }
        serde_json::Value::Null => None,
        other => Some(other.to_string()),
    }
}

/// Builds messages in order: assistant parts collect into one message until
/// the next user prompt.
#[derive(Default)]
struct Transcript {
    messages: Vec<bruhs_models::logic::portable::PortableMessage>,
    assistant: Option<bruhs_models::logic::portable::PortableMessage>,
}

impl Transcript {
    fn user(&mut self, text: String, at_ms: Option<i64>) {
        self.flush();
        self.messages
            .push(bruhs_models::logic::portable::PortableMessage {
                role: bruhs_models::agent::Role::User,
                parts: vec![bruhs_models::logic::portable::PortablePart::Text(text)],
                started_at_ms: at_ms,
                model: None,
            });
    }

    fn part(
        &mut self,
        part: bruhs_models::logic::portable::PortablePart,
        at_ms: Option<i64>,
        model: Option<&str>,
    ) {
        let message =
            self.assistant
                .get_or_insert_with(|| bruhs_models::logic::portable::PortableMessage {
                    role: bruhs_models::agent::Role::Assistant,
                    parts: Vec::new(),
                    started_at_ms: at_ms,
                    model: None,
                });
        if let Some(model) = model.filter(|m| !m.is_empty() && *m != "<synthetic>") {
            message.model = Some(model.to_string());
        }
        message.parts.push(part);
    }

    /// Put `output` on the tool call `id` of the open assistant message.
    fn tool_output(&mut self, id: &str, output: Option<String>, is_error: bool) {
        let Some(message) = self.assistant.as_mut() else {
            return;
        };
        for part in message.parts.iter_mut().rev() {
            if let bruhs_models::logic::portable::PortablePart::ToolCall(call) = part {
                if call.id == id {
                    call.output = output;
                    call.is_error = is_error;
                    return;
                }
            }
        }
    }

    fn flush(&mut self) {
        if let Some(message) = self.assistant.take() {
            if !message.parts.is_empty() {
                self.messages.push(message);
            }
        }
    }

    fn finish(mut self) -> Vec<bruhs_models::logic::portable::PortableMessage> {
        self.flush();
        self.messages
    }
}

#[cfg(test)]
mod tests;

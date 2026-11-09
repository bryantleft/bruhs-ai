//! Threads that move between bruhs and the other agent tools (Claude Code,
//! Codex, T3 Code, Cursor), in either direction.
//!
//! bruhs drives the same CLIs as most of them, so a thread usually moves
//! without losing anything: a Claude Code session or a Codex rollout (T3
//! threads are one of the two underneath) is resumed by id, and the thread
//! continues in whichever tool opens it next. Cursor keeps its chats in its
//! own database with no session another tool can resume, so a Cursor chat
//! comes in as its transcript, sent along with the first bruhs turn. Going
//! out, Claude Code and Codex get a resume command; T3 and Cursor get the
//! transcript as Markdown.
//!
//! [`PortableMessage`] is the shape every importer produces and every
//! exporter reads; each surface converts its own message type to and from it.

use crate::agent::{AgentProvider, Role};
use crate::time::EpochMs;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fmt::Write as _;

/// A tool whose threads bruhs can import.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImportSource {
    ClaudeCode,
    Codex,
    T3,
    Cursor,
}

impl ImportSource {
    /// Every source, in picker order.
    pub const ALL: [Self; 4] = [Self::ClaudeCode, Self::Codex, Self::T3, Self::Cursor];

    /// The wire string (`?source=`).
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ClaudeCode => "claude_code",
            Self::Codex => "codex",
            Self::T3 => "t3",
            Self::Cursor => "cursor",
        }
    }

    /// `s` as a source.
    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|source| source.as_str() == s)
    }

    /// Name shown in pickers.
    pub fn label(self) -> &'static str {
        match self {
            Self::ClaudeCode => "Claude Code",
            Self::Codex => "Codex",
            Self::T3 => "T3 Code",
            Self::Cursor => "Cursor",
        }
    }
}

/// A tool call inside an assistant message.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PortableToolCall {
    pub id: String,
    pub tool: String,
    #[serde(default)]
    pub input: Value,
    #[serde(default)]
    pub output: Option<String>,
    #[serde(default)]
    pub is_error: bool,
}

/// One ordered piece of a message.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum PortablePart {
    Text(String),
    Thinking(String),
    ToolCall(PortableToolCall),
}

/// One message of an imported or exported thread. A user message holds
/// text; an assistant message holds everything the agent produced between
/// two user messages, in order.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PortableMessage {
    pub role: Role,
    pub parts: Vec<PortablePart>,
    #[serde(default)]
    pub started_at_ms: Option<i64>,
    #[serde(default)]
    pub model: Option<String>,
}

impl PortableMessage {
    /// The message's text parts, joined by blank lines.
    pub fn text(&self) -> String {
        let texts: Vec<&str> = self
            .parts
            .iter()
            .filter_map(|part| match part {
                PortablePart::Text(text) => Some(text.trim()),
                _ => None,
            })
            .filter(|text| !text.is_empty())
            .collect();
        texts.join("\n\n")
    }
}

/// A thread another tool has, as the import picker lists it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImportCandidate {
    pub source: ImportSource,
    /// The source's own id: a Claude session id, a Codex thread id, a T3
    /// thread id or a Cursor composer id.
    pub id: String,
    pub title: String,
    /// Working directory the thread ran in, when known.
    #[serde(default)]
    pub cwd: Option<String>,
    pub updated_at: EpochMs,
    /// The CLI session bruhs resumes, and its provider. `None` for a thread
    /// that comes in as a transcript (Cursor).
    #[serde(default)]
    pub provider: Option<AgentProvider>,
    #[serde(default)]
    pub session_id: Option<String>,
    /// Model of the thread's last turn, when recorded.
    #[serde(default)]
    pub model: Option<String>,
}

/// `GET /api/import/sessions`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ImportListQuery {
    pub source: Option<String>,
    /// Case-insensitive filter on the title and directory.
    #[serde(default)]
    pub q: Option<String>,
    #[serde(default)]
    pub limit: Option<usize>,
}

/// `GET /api/import/thread`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImportThreadQuery {
    pub source: String,
    pub id: String,
}

/// A thread read from another tool, ready to become a bruhs thread.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImportedThread {
    pub candidate: ImportCandidate,
    pub messages: Vec<PortableMessage>,
    /// The transcript the first bruhs turn sends along (see
    /// [`with_import_context`]), for a thread bruhs cannot resume.
    #[serde(default)]
    pub context: Option<String>,
}

/// Longest transcript, in characters, sent with the first turn of a thread
/// imported without a session: the most recent messages are kept.
pub const IMPORT_CONTEXT_MAX_CHARS: usize = 48_000;

/// Longest tool output kept per call in a transcript, in characters.
const TOOL_OUTPUT_MAX_CHARS: usize = 1_200;

impl ImportedThread {
    /// A thread with its context filled in when it has no session to resume.
    pub fn new(candidate: ImportCandidate, messages: Vec<PortableMessage>) -> Self {
        let context = candidate.session_id.is_none().then(|| {
            transcript_markdown(
                &candidate.title,
                candidate.source.label(),
                &messages,
                IMPORT_CONTEXT_MAX_CHARS,
            )
        });
        Self {
            candidate,
            messages,
            context,
        }
    }
}

/// The model an imported thread continues on: its own last model when that
/// belongs to its session's provider, else the first catalog model of that
/// provider (a resumed session must stay on its provider), else the
/// default model (a transcript import can go anywhere).
pub fn import_model(candidate: &ImportCandidate) -> String {
    let Some(provider) = candidate.provider else {
        return crate::agent::DEFAULT_AGENT_MODEL.to_string();
    };
    candidate
        .model
        .as_deref()
        .filter(|model| AgentProvider::for_model(model) == provider)
        .map(str::to_string)
        .or_else(|| {
            crate::agent::MODEL_CATALOG
                .iter()
                .find(|model| model.provider == provider)
                .map(|model| model.id.to_string())
        })
        .unwrap_or_else(|| crate::agent::DEFAULT_AGENT_MODEL.to_string())
}

/// A prompt sent without a session that holds the earlier conversation (an
/// import with no session, or a thread whose session could not be resumed):
/// that conversation, then what the user asks now. A slash command goes out
/// as typed, since the CLI reads it only at the start of the prompt.
pub fn with_import_context(context: &str, prompt: &str) -> String {
    if prompt.trim_start().starts_with('/') {
        return prompt.to_string();
    }
    format!(
        "This thread continues an earlier conversation, which follows; use it \
         as context.\n\n{context}\n\n---\n\n{prompt}"
    )
}

/// The earlier conversation of a thread whose session is gone, as context
/// for its next prompt; `None` when there is nothing before it.
pub fn thread_context(title: &str, messages: &[PortableMessage]) -> Option<String> {
    (!messages.is_empty())
        .then(|| transcript_markdown(title, "bruhs", messages, IMPORT_CONTEXT_MAX_CHARS))
}

/// `messages` as Markdown, for another tool to read: a heading, then each
/// message under its role, tool calls as one line each with their output
/// cut short. When it is longer than `max_chars`, the oldest messages are
/// dropped (and a note says so).
pub fn transcript_markdown(
    title: &str,
    source: &str,
    messages: &[PortableMessage],
    max_chars: usize,
) -> String {
    let header = format!("# {}\n\n_Conversation from {source}._\n\n", one_line(title));
    let blocks: Vec<String> = messages.iter().filter_map(message_markdown).collect();
    let mut kept = Vec::new();
    let mut used = header.chars().count();
    for block in blocks.iter().rev() {
        let size = block.chars().count() + 2;
        if used + size > max_chars && !kept.is_empty() {
            break;
        }
        used += size;
        kept.push(block.as_str());
    }
    let dropped = blocks.len() - kept.len();
    let mut out = header;
    if dropped > 0 {
        let _ = writeln!(out, "_{dropped} earlier messages omitted._\n");
    }
    for block in kept.into_iter().rev() {
        out.push_str(block);
        out.push_str("\n\n");
    }
    out.truncate(out.trim_end().len());
    out.push('\n');
    out
}

fn message_markdown(message: &PortableMessage) -> Option<String> {
    let mut body = String::new();
    for part in &message.parts {
        match part {
            PortablePart::Text(text) if !text.trim().is_empty() => {
                let _ = write!(body, "{}\n\n", text.trim());
            }
            PortablePart::ToolCall(call) => {
                let _ = write!(body, "- tool `{}`", call.tool);
                if let Some(summary) = input_summary(&call.input) {
                    let _ = write!(body, ": {summary}");
                }
                body.push('\n');
                if let Some(output) = call.output.as_deref().map(str::trim) {
                    if !output.is_empty() {
                        let mark = if call.is_error { "error" } else { "output" };
                        let _ = writeln!(
                            body,
                            "  {mark}: {}",
                            one_line(&clip(output, TOOL_OUTPUT_MAX_CHARS))
                        );
                    }
                }
            }
            // Thinking stays out: it is long and the next agent re-derives it.
            _ => {}
        }
    }
    let body = body.trim_end();
    if body.is_empty() {
        return None;
    }
    let who = match message.role {
        Role::User => "User",
        Role::Assistant | Role::Tool => "Assistant",
    };
    Some(format!("## {who}\n\n{body}"))
}

/// A tool input in one short line: its command, path or query when it has
/// one, else the JSON.
fn input_summary(input: &Value) -> Option<String> {
    let text = match input {
        Value::Null => return None,
        Value::String(text) => text.clone(),
        Value::Object(map) => [
            "command",
            "cmd",
            "file_path",
            "path",
            "pattern",
            "query",
            "url",
        ]
        .iter()
        .find_map(|key| map.get(*key))
        .map_or_else(
            || input.to_string(),
            |value| {
                value
                    .as_str()
                    .map_or_else(|| value.to_string(), str::to_string)
            },
        ),
        other => other.to_string(),
    };
    let text = one_line(&clip(&text, 200));
    (!text.is_empty()).then_some(text)
}

fn clip(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    let mut out: String = text.chars().take(max_chars).collect();
    out.push('…');
    out
}

fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// A title for a thread that has none: its first prompt, on one line and
/// at most 60 characters.
pub fn title_from_prompt(prompt: &str) -> String {
    let line = one_line(prompt);
    if line.is_empty() {
        "Imported thread".to_string()
    } else {
        clip(&line, 60)
    }
}

/// The shell command that continues `session_id` in its own CLI, from
/// `cwd` (Claude Code finds a session by the directory it ran in; with no
/// `cwd`, a chat that ran in the home directory). Codex finds its sessions
/// from anywhere.
pub fn resume_command(provider: AgentProvider, session_id: &str, cwd: Option<&str>) -> String {
    let resume = match provider {
        AgentProvider::Claude => format!("claude --resume {}", shell_quote(session_id)),
        AgentProvider::Codex => format!("codex resume {}", shell_quote(session_id)),
    };
    match (cwd.filter(|dir| !dir.is_empty()), provider) {
        (Some(dir), _) => format!("cd {} && {resume}", shell_quote(dir)),
        (None, AgentProvider::Claude) => format!("cd ~ && {resume}"),
        (None, AgentProvider::Codex) => resume,
    }
}

/// `text` as one POSIX shell word: bare when it is only safe characters,
/// else single-quoted.
fn shell_quote(text: &str) -> String {
    let safe = !text.is_empty()
        && text
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_./:@%+=,".contains(c));
    if safe {
        text.to_string()
    } else {
        format!("'{}'", text.replace('\'', r"'\''"))
    }
}

/// `POST /api/export/t3`: start a T3 Code thread in `project_path` that
/// carries this transcript.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExportT3Request {
    pub project_path: String,
    pub title: String,
    /// Model the T3 thread runs on (its provider follows from it).
    pub model: String,
    pub markdown: String,
}

/// The T3 thread [`ExportT3Request`] started.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExportT3Response {
    pub thread_id: String,
}

/// The first message of a T3 thread that continues a bruhs thread. T3
/// runs it as a turn, so it asks for a one-word reply.
pub fn t3_export_prompt(markdown: &str) -> String {
    format!(
        "This thread continues a conversation from bruhs. Read it as context, \
         then reply only \"Ready.\" and wait for my next message.\n\n{markdown}"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn user(text: &str) -> PortableMessage {
        PortableMessage {
            role: Role::User,
            parts: vec![PortablePart::Text(text.into())],
            started_at_ms: None,
            model: None,
        }
    }

    fn assistant(parts: Vec<PortablePart>) -> PortableMessage {
        PortableMessage {
            role: Role::Assistant,
            parts,
            started_at_ms: None,
            model: None,
        }
    }

    #[test]
    fn sources_round_trip_their_wire_names() {
        for source in ImportSource::ALL {
            assert_eq!(ImportSource::parse(source.as_str()), Some(source));
            let json = serde_json::to_string(&source).unwrap();
            assert_eq!(json, format!("\"{}\"", source.as_str()));
        }
        assert_eq!(ImportSource::parse("vim"), None);
    }

    #[test]
    fn markdown_keeps_text_and_tools_and_drops_thinking() {
        let messages = vec![
            user("Fix the paging bug"),
            assistant(vec![
                PortablePart::Thinking("secret reasoning".into()),
                PortablePart::ToolCall(PortableToolCall {
                    id: "t1".into(),
                    tool: "Bash".into(),
                    input: serde_json::json!({"command": "pytest -q"}),
                    output: Some("1 failed\n2 passed".into()),
                    is_error: false,
                }),
                PortablePart::Text("Fixed: the last page is kept.".into()),
            ]),
        ];
        let md = transcript_markdown("Paging", "Claude Code", &messages, 10_000);
        assert!(md.starts_with("# Paging\n\n_Conversation from Claude Code._"));
        assert!(md.contains("## User\n\nFix the paging bug"));
        assert!(md.contains("- tool `Bash`: pytest -q\n  output: 1 failed 2 passed"));
        assert!(md.contains("Fixed: the last page is kept."));
        assert!(!md.contains("secret reasoning"));
    }

    #[test]
    fn a_long_transcript_keeps_the_newest_messages() {
        let messages: Vec<_> = (0..50)
            .map(|i| user(&format!("message {i} {}", "x".repeat(80))))
            .collect();
        let md = transcript_markdown("Long", "Cursor", &messages, 1_000);
        assert!(md.chars().count() <= 1_000);
        assert!(md.contains("message 49"));
        assert!(!md.contains("message 0 "));
        assert!(md.contains("earlier messages omitted"));
    }

    #[test]
    fn only_a_thread_without_a_session_carries_context() {
        let candidate = ImportCandidate {
            source: ImportSource::Cursor,
            id: "c1".into(),
            title: "Chat".into(),
            cwd: None,
            updated_at: EpochMs::UNIX_EPOCH,
            provider: None,
            session_id: None,
            model: None,
        };
        let imported = ImportedThread::new(candidate.clone(), vec![user("hi")]);
        assert!(imported
            .context
            .as_deref()
            .is_some_and(|c| c.contains("hi")));
        let resumable = ImportedThread::new(
            ImportCandidate {
                session_id: Some("s".into()),
                provider: Some(AgentProvider::Claude),
                ..candidate
            },
            vec![user("hi")],
        );
        assert_eq!(resumable.context, None);
    }

    #[test]
    fn resume_commands_quote_what_the_shell_would_split() {
        assert_eq!(
            resume_command(AgentProvider::Claude, "abc-1", Some("/Users/me/my repo")),
            "cd '/Users/me/my repo' && claude --resume abc-1"
        );
        assert_eq!(
            resume_command(AgentProvider::Codex, "019a", None),
            "codex resume 019a"
        );
        assert_eq!(
            resume_command(AgentProvider::Claude, "abc-1", None),
            "cd ~ && claude --resume abc-1"
        );
        assert_eq!(shell_quote("it's"), r"'it'\''s'");
    }

    #[test]
    fn an_import_continues_on_its_sessions_provider() {
        let base = ImportCandidate {
            source: ImportSource::Codex,
            id: "x".into(),
            title: "t".into(),
            cwd: None,
            updated_at: EpochMs::UNIX_EPOCH,
            provider: Some(AgentProvider::Codex),
            session_id: Some("s".into()),
            model: Some("gpt-5.5".into()),
        };
        assert_eq!(import_model(&base), "gpt-5.5");
        let unknown = ImportCandidate {
            model: None,
            ..base.clone()
        };
        assert_eq!(
            AgentProvider::for_model(&import_model(&unknown)),
            AgentProvider::Codex
        );
        let claude_model_on_codex = ImportCandidate {
            model: Some("claude-opus-5-5".into()),
            ..base.clone()
        };
        assert_eq!(
            AgentProvider::for_model(&import_model(&claude_model_on_codex)),
            AgentProvider::Codex
        );
        let cursor = ImportCandidate {
            provider: None,
            session_id: None,
            model: None,
            ..base
        };
        assert_eq!(import_model(&cursor), crate::agent::DEFAULT_AGENT_MODEL);
    }

    #[test]
    fn context_rides_ahead_of_prose_but_not_slash_commands() {
        let carried = with_import_context("# Earlier", "next");
        assert!(carried.starts_with("This thread continues"));
        assert!(carried.ends_with("# Earlier\n\n---\n\nnext"));
        assert_eq!(with_import_context("# Earlier", " /compact"), " /compact");
        assert_eq!(thread_context("t", &[]), None);
        assert!(thread_context("t", &[user("hi")]).is_some_and(|text| text.contains("hi")));
    }

    #[test]
    fn titles_come_from_the_first_prompt() {
        assert_eq!(title_from_prompt("  fix\n the bug  "), "fix the bug");
        assert_eq!(title_from_prompt(""), "Imported thread");
        assert_eq!(title_from_prompt(&"a".repeat(80)).chars().count(), 61);
    }
}

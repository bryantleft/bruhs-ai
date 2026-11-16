//! `bruhs_read_chat` and `bruhs_read_chat_messages`: thread summaries and
//! completed message history. Each app keeps its own message type, so both
//! project into the records here and a client sees one shape. Streaming
//! messages, unsent input and stashed drafts are never projected.
use super::page::read_matching_page;
use super::{require_read, LowercaseQuery, MAX_ID_BYTES, MAX_OFFSET, MAX_QUERY_BYTES};
use crate::{AgentDomain, AgentPermissionPolicy};
use serde::Serialize;
use serde_json::Value;
use std::borrow::Cow;

/// The error for a thread ID that resolves to nothing.
pub const CHAT_THREAD_NOT_FOUND: &str = "Chat thread was not found in the current workspace.";

/// One thread in a `bruhs_read_chat` page; no message bodies.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ChatThreadSummary<'a> {
    pub id: &'a str,
    pub title: &'a str,
    pub project_name: &'a str,
    pub updated_at_ms: i64,
    pub pinned: bool,
    pub settled: bool,
    pub streaming: bool,
}

/// Billed tokens of a turn, named as the agent stream reports them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ChatUsageRecord {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_input_tokens: u64,
    pub cache_creation_input_tokens: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_tokens: Option<u64>,
}

/// One tool call of a message.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ChatToolCallRecord<'a> {
    pub id: &'a str,
    pub tool: &'a str,
    pub input: &'a Value,
    pub output: Option<&'a str>,
    pub is_error: bool,
}

/// One completed message in a `bruhs_read_chat_messages` page.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ChatMessageRecord<'a> {
    pub role: &'a str,
    pub content: Cow<'a, str>,
    pub model: Option<&'a str>,
    pub thinking: Cow<'a, str>,
    pub usage: Option<ChatUsageRecord>,
    pub duration_ms: Option<u64>,
    pub started_at_ms: Option<i64>,
    pub completed_at_ms: Option<i64>,
    pub tool_calls: Vec<ChatToolCallRecord<'a>>,
    pub stopped: bool,
    pub error: Option<&'a str>,
}

/// Thread summaries. `summary` projects an app thread; `query` matches its
/// ID, title or project name case-insensitively, and `offset` counts matching
/// threads. The caller has checked Chat Read.
pub fn read_chat_threads<'a, R>(
    threads: impl Iterator<Item = R>,
    summary: impl Fn(&R) -> ChatThreadSummary<'a>,
    query: &str,
    offset: usize,
) -> Result<Value, String> {
    if query.len() > MAX_QUERY_BYTES {
        return Err("Query is too large.".into());
    }
    let query = LowercaseQuery::new(query);
    let mut matcher = query.matcher();
    read_matching_page(
        threads,
        query.is_empty(),
        |thread| {
            let thread = summary(thread);
            [thread.id, thread.title, thread.project_name]
                .iter()
                .any(|field| matcher.matches(field))
        },
        |thread| summary(&thread),
        offset,
    )
}

/// Chat Read and the bounds, checked before the thread ID is resolved, so
/// revocation cannot reveal whether a thread exists.
pub fn authorize_chat_messages(
    policy: &AgentPermissionPolicy,
    thread_id: &str,
    offset: usize,
) -> Result<(), String> {
    require_read(policy, AgentDomain::Chat)?;
    if thread_id.trim().is_empty() || thread_id.len() > MAX_ID_BYTES || offset > MAX_OFFSET {
        return Err("Invalid thread ID or offset.".into());
    }
    Ok(())
}

/// A page of one thread's completed messages, in conversation order.
/// `messages` must already exclude streaming ones; `offset` counts only the
/// messages it yields.
pub fn read_chat_messages<'a, M>(
    messages: impl Iterator<Item = M>,
    project: impl FnMut(M) -> ChatMessageRecord<'a>,
    thread_id: &str,
    offset: usize,
) -> Result<Value, String> {
    let mut result = read_matching_page(messages, true, |_| true, project, offset)?;
    result["thread_id"] = thread_id.into();
    Ok(result)
}

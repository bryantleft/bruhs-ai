//! Claude.ai conversations and memory from a data export (R§2): one
//! record of `conversations.json` becomes its newest branch, and a
//! `memories.json` record or `memories/<account>.json` file becomes one
//! memory suggestion. `users.json` is personal data and matches none of
//! these shapes, so it never comes in.
//!
//! A conversation's `chat_messages` (or, from browser tools, `messages`)
//! aren't in order. Since about March 2026 each names its
//! `parent_message_uuid`, the first ones naming the root sentinel. With
//! no export-side pointer to the branch the owner last saw, the newest
//! leaf wins. A parent that is missing attaches to the message before it
//! in time, and an export with no parents at all is read in time order.
//! Every walk keeps a visited set and stops at `MAX_NODES`, so a parent
//! loop ends it instead of looping.

use super::text::{
    attached_line, attachment_line, clip_part, clip_strings, export_time, shown_name,
    shows_as_title, NAME_MAX_CHARS,
};
use super::{output_text, ParsedConversation, Transcript};
use bruhs_models::imports::{
    conversation_key, MemorySuggestion, AUDIO_NOT_IMPORTED, DOCUMENT_NOT_IMPORTED,
    IMAGE_NOT_IMPORTED, MAX_ATTACHMENT_TEXT_CHARS, MAX_MESSAGES, MAX_NODES, MAX_PART_CHARS,
    PROJECT_MEMORY,
};
use bruhs_models::logic::portable::{
    title_from_prompt, ImportCandidate, ImportSource, PortablePart, PortableToolCall,
};
use bruhs_models::num::{ToFloatLossy as _, ToIntSaturating as _};
use bruhs_models::EpochMs;
use serde_json::Value;
use std::collections::{HashMap, HashSet};

/// The parent a conversation's first messages name.
const ROOT: &str = "00000000-0000-4000-8000-000000000000";

/// The tools whose calls are the conversation's work (artifacts, in each
/// of their names over time). Every other tool call, web search
/// included, is plumbing.
const KEPT_TOOLS: [&str; 5] = ["artifacts", "create_file", "str_replace", "Write", "Edit"];

/// The keys only Claude's memory has.
const MEMORY_KEYS: [&str; 3] = ["conversations_memory", "memory_files", "project_memories"];

/// Whether `v` is a Claude.ai conversation: `chat_messages`, or a `uuid`
/// with `messages[].sender`, and no ChatGPT `mapping`.
pub(crate) fn is_conversation(v: &Value) -> bool {
    if v.get("mapping").is_some() {
        return false;
    }
    v.get("chat_messages").is_some_and(Value::is_array)
        || (v.get("uuid").is_some_and(Value::is_string)
            && v.get("messages")
                .and_then(Value::as_array)
                .is_some_and(|messages| {
                    messages
                        .iter()
                        .any(|message| message.get("sender").is_some_and(Value::is_string))
                }))
}

/// Whether `v` is Claude's export manifest: `data_files[]` with
/// `export_url`, `category` and `filename`. Its links are single-use and
/// remote, so they are never followed.
pub(crate) fn is_manifest(v: &Value) -> bool {
    v.get("data_files")
        .and_then(Value::as_array)
        .is_some_and(|files| {
            files.iter().any(|file| {
                ["export_url", "category", "filename"]
                    .iter()
                    .all(|key| file.get(*key).is_some())
            })
        })
}

/// Whether `v` is Claude's memory: an object with `conversations_memory`,
/// `memory_files` or `project_memories`, or an array holding one (the
/// older `memories.json` read whole).
pub(crate) fn is_memory(v: &Value) -> bool {
    match v {
        Value::Array(items) => items.iter().any(is_memory_object),
        _ => is_memory_object(v),
    }
}

fn is_memory_object(v: &Value) -> bool {
    MEMORY_KEYS.iter().any(|key| v.get(*key).is_some())
}

/// Claude's memory in `v` as one suggestion: `conversations_memory`, then
/// each project memory under `PROJECT_MEMORY`, then each memory file
/// under its path. `None` when `v` isn't memory or holds no text.
pub(crate) fn memory(v: &Value) -> Option<MemorySuggestion> {
    let records: Vec<&Value> = match v {
        Value::Array(items) => items.iter().filter(|item| is_memory_object(item)).collect(),
        _ if is_memory_object(v) => vec![v],
        _ => return None,
    };
    let mut sections: Vec<String> = Vec::new();
    for record in records {
        if let Some(text) = non_empty(record.get("conversations_memory")) {
            sections.push(text.to_string());
        }
        for text in record
            .get("project_memories")
            .and_then(Value::as_object)
            .into_iter()
            .flat_map(|projects| projects.values())
            .filter_map(|text| non_empty(Some(text)))
        {
            sections.push(format!("{PROJECT_MEMORY}\n{text}"));
        }
        for file in record
            .get("memory_files")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let Some(content) = non_empty(file.get("content")) else {
                continue;
            };
            sections.push(match non_empty(file.get("path")) {
                Some(path) => format!("## {path}\n\n{content}"),
                None => content.to_string(),
            });
        }
    }
    if sections.is_empty() {
        return None;
    }
    Some(MemorySuggestion {
        source: ImportSource::ClaudeAi,
        text: clip_part(sections.join("\n\n"), MAX_PART_CHARS),
    })
}

/// The conversation `v` along its newest branch. `None` when `v` is not a
/// Claude.ai conversation, has no id, or has no visible message.
pub(crate) fn conversation(v: &Value) -> Option<ParsedConversation> {
    if !is_conversation(v) {
        return None;
    }
    let raw_id = ["uuid", "id"].iter().find_map(|key| {
        v.get(*key)
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty())
    })?;
    let nodes: Vec<&Value> = ["chat_messages", "messages"]
        .iter()
        .find_map(|key| v.get(*key).and_then(Value::as_array))?
        .iter()
        .filter(|message| message.is_object())
        .take(MAX_NODES)
        .collect();
    let (branch, dropped) = newest_branch(&nodes);
    let model = v.get("model").and_then(Value::as_str);

    let mut transcript = Transcript::default();
    let mut newest = None;
    for &at in &branch {
        if let Some(time) = add(nodes[at], &mut transcript, model) {
            newest = newest.max(Some(time));
        }
    }
    let mut messages = transcript.finish();
    if messages.is_empty() {
        return None;
    }
    if messages.len() > MAX_MESSAGES {
        messages.drain(..messages.len() - MAX_MESSAGES);
    }

    let title = non_empty(v.get("name"))
        .filter(|title| shows_as_title(title))
        .map_or_else(
            || {
                let first = branch.iter().find_map(|&at| {
                    let node = nodes[at];
                    (sender(node) == Some(Sender::Human))
                        .then(|| typed_text(node))
                        .map(|text| text.trim().to_string())
                        .filter(|text| !text.is_empty())
                });
                title_from_prompt(&first.unwrap_or_default())
            },
            str::to_string,
        );
    let updated = v
        .get("updated_at")
        .and_then(export_time)
        .or_else(|| v.get("created_at").and_then(export_time))
        .max(newest)
        .unwrap_or(0);
    Some(ParsedConversation {
        candidate: ImportCandidate {
            source: ImportSource::ClaudeAi,
            id: conversation_key(raw_id),
            title,
            cwd: None,
            updated_at: EpochMs::saturating_from_i64(updated),
            provider: None,
            session_id: None,
            model: None,
        },
        messages,
        dropped_branches: dropped.to_f64_lossy().to_u32_saturating(),
    })
}

/// The nodes from the root down to the newest leaf, and how many other
/// leaves stayed behind. With no parents anywhere, every node in time
/// order.
fn newest_branch(nodes: &[&Value]) -> (Vec<usize>, usize) {
    let count = nodes.len();
    let time: Vec<Option<i64>> = nodes
        .iter()
        .map(|node| node.get("created_at").and_then(export_time))
        .collect();
    let mut by_time: Vec<usize> = (0..count).collect();
    by_time.sort_by_key(|&at| (time[at], at));
    let threaded = nodes.iter().any(|node| {
        node.get("parent_message_uuid")
            .is_some_and(Value::is_string)
    });
    if !threaded {
        return (by_time, 0);
    }

    // Each node's id, sorted, so a parent is found by binary search.
    let mut ids: Vec<(&str, usize)> = nodes
        .iter()
        .enumerate()
        .filter_map(|(at, node)| {
            node.get("uuid")
                .and_then(Value::as_str)
                .filter(|id| !id.is_empty())
                .map(|id| (id, at))
        })
        .collect();
    ids.sort_unstable();
    let find = |id: &str| {
        ids.binary_search_by_key(&id, |(node, _)| *node)
            .ok()
            .map(|found| ids[found].1)
    };
    let mut previous = vec![None; count];
    for pair in by_time.windows(2) {
        previous[pair[1]] = Some(pair[0]);
    }
    let parent: Vec<Option<usize>> = (0..count)
        .map(
            |at| match nodes[at].get("parent_message_uuid").and_then(Value::as_str) {
                Some(ROOT) => None,
                Some(id) => find(id).or(previous[at]),
                None => previous[at],
            },
        )
        .collect();
    let mut has_child = vec![false; count];
    for &up in parent.iter().flatten() {
        has_child[up] = true;
    }

    // The newest leaf (the later in the array when times tie); with no
    // leaf at all (every node in a loop), the newest node.
    let newest = |ids: &mut dyn Iterator<Item = usize>| ids.max_by_key(|&at| (time[at], at));
    let leaves = || (0..count).filter(|&at| !has_child[at]);
    let Some(leaf) = newest(&mut leaves()).or_else(|| newest(&mut (0..count))) else {
        return (Vec::new(), 0);
    };
    let dropped = leaves().count() - usize::from(!has_child[leaf]);

    let mut seen = vec![false; count];
    let mut path = Vec::new();
    let mut at = Some(leaf);
    while let Some(node) = at {
        if path.len() >= MAX_NODES || seen[node] {
            break;
        }
        seen[node] = true;
        path.push(node);
        at = parent[node];
    }
    path.reverse();
    (path, dropped)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Sender {
    Human,
    Assistant,
}

fn sender(message: &Value) -> Option<Sender> {
    match message.get("sender").and_then(Value::as_str)? {
        "human" => Some(Sender::Human),
        "assistant" => Some(Sender::Assistant),
        _ => None,
    }
}

/// Adds `message` to `transcript` when it shows anything, and answers its
/// time.
fn add(message: &Value, transcript: &mut Transcript, model: Option<&str>) -> Option<i64> {
    let at = message.get("created_at").and_then(export_time);
    match sender(message)? {
        Sender::Human => {
            let mut sections: Vec<String> = Vec::new();
            let typed = typed_text(message);
            let typed = typed.trim();
            if !typed.is_empty() {
                sections.push(typed.to_string());
            }
            sections.extend(left_behind(message));
            if sections.is_empty() {
                return None;
            }
            transcript.user(clip_part(sections.join("\n\n"), MAX_PART_CHARS), at);
        }
        Sender::Assistant => {
            let model = message.get("model").and_then(Value::as_str).or(model);
            let parts = assistant_parts(message);
            if parts.is_empty() {
                return None;
            }
            for part in parts {
                transcript.part(part, at, model);
            }
        }
    }
    at
}

fn blocks(message: &Value) -> impl Iterator<Item = &Value> {
    message
        .get("content")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
}

/// Whether `message` has any `content[]` block. Without one, its flat
/// `text` is all there is (old exports); with one, `text` only repeats
/// the text blocks, or holds filler the blocks hide.
fn has_blocks(message: &Value) -> bool {
    message
        .get("content")
        .and_then(Value::as_array)
        .is_some_and(|blocks| !blocks.is_empty())
}

fn block_type(block: &Value) -> Option<&str> {
    block.get("type").and_then(Value::as_str)
}

/// What the sender typed: the `text` blocks, or the flat `text` when
/// there are no blocks.
fn typed_text(message: &Value) -> String {
    if has_blocks(message) {
        blocks(message)
            .filter(|block| block_type(block) == Some("text"))
            .filter_map(|block| non_empty(block.get("text")))
            .collect::<Vec<_>>()
            .join("\n\n")
    } else {
        non_empty(message.get("text"))
            .unwrap_or_default()
            .to_string()
    }
}

/// A reply's parts in order: text, thinking and the kept tool calls with
/// their results; the flat `text` when there are no blocks; then one text
/// part for what stays behind.
fn assistant_parts(message: &Value) -> Vec<PortablePart> {
    let mut parts = Vec::new();
    // Kept calls by id, and the last call when it was kept, for results
    // naming no id.
    let mut calls: HashMap<&str, usize> = HashMap::new();
    let mut last_call: Option<usize> = None;
    for (position, block) in blocks(message).enumerate() {
        match block_type(block) {
            Some("text") => {
                if let Some(text) = non_empty(block.get("text")) {
                    parts.push(PortablePart::Text(clip_part(
                        text.to_string(),
                        MAX_PART_CHARS,
                    )));
                }
            }
            Some("thinking") => {
                if let Some(text) = non_empty(block.get("thinking")) {
                    parts.push(PortablePart::Thinking(clip_part(
                        text.to_string(),
                        MAX_PART_CHARS,
                    )));
                }
            }
            Some("tool_use") => {
                let Some(tool) = block
                    .get("name")
                    .and_then(Value::as_str)
                    .filter(|name| KEPT_TOOLS.contains(name))
                else {
                    // A result after a skipped call is that call's.
                    last_call = None;
                    continue;
                };
                // Results find their call by the id as written; the part
                // keeps it as one clean, short line.
                let raw = non_empty(block.get("id"));
                if let Some(raw) = raw {
                    calls.insert(raw, parts.len());
                }
                let id = raw
                    .map(|raw| shown_name(raw, NAME_MAX_CHARS))
                    .filter(|id| !id.is_empty())
                    .unwrap_or_else(|| format!("call-{position}"));
                last_call = Some(parts.len());
                parts.push(PortablePart::ToolCall(PortableToolCall {
                    id,
                    tool: tool.to_string(),
                    input: block
                        .get("input")
                        .map_or(Value::Null, |input| clip_strings(input, MAX_PART_CHARS)),
                    output: None,
                    is_error: false,
                }));
            }
            Some("tool_result") => {
                let target = match non_empty(block.get("tool_use_id")) {
                    Some(id) => calls.get(id).copied(),
                    None => last_call,
                };
                let Some(PortablePart::ToolCall(call)) = target.and_then(|at| parts.get_mut(at))
                else {
                    continue;
                };
                let named = block.get("name").and_then(Value::as_str);
                if named.is_some_and(|name| name != call.tool) {
                    continue;
                }
                call.output = block
                    .get("content")
                    .and_then(output_text)
                    .map(|output| clip_part(output, MAX_PART_CHARS));
                call.is_error = block
                    .get("is_error")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
            }
            // Injected prompts and budgets are plumbing; redacted thinking
            // is hidden reasoning. Media and documents are `left_behind`.
            _ => {}
        }
    }
    if !has_blocks(message) {
        if let Some(text) = non_empty(message.get("text")) {
            parts.push(PortablePart::Text(clip_part(
                text.to_string(),
                MAX_PART_CHARS,
            )));
        }
    }
    let behind = left_behind(message);
    if !behind.is_empty() {
        parts.push(PortablePart::Text(clip_part(
            behind.join("\n\n"),
            MAX_PART_CHARS,
        )));
    }
    parts
}

/// A placeholder line for each thing in `message` whose bytes stay
/// behind, the same for either sender (D17):
/// - `IMAGE_NOT_IMPORTED` once, for image blocks no attachment or file
///   already names (by `file_name` or `file_uuid`);
/// - `AUDIO_NOT_IMPORTED` once, for voice notes;
/// - each attachment's extracted text under `attached_line` (cut to
///   `MAX_ATTACHMENT_TEXT_CHARS`), else `attachment_line`;
/// - `attachment_line` for each document block and file not already
///   named;
/// - `DOCUMENT_NOT_IMPORTED` once, for documents with no name.
fn left_behind(message: &Value) -> Vec<String> {
    let listed = |key: &'static str| {
        message
            .get(key)
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
    };
    let known: HashSet<&str> = listed("attachments")
        .chain(listed("files"))
        .flat_map(|item| ["file_name", "file_uuid"].map(|key| non_empty(item.get(key))))
        .flatten()
        .collect();
    let (mut image, mut audio, mut nameless) = (false, false, false);
    let mut documents: Vec<&str> = Vec::new();
    for block in blocks(message) {
        match block_type(block) {
            Some("image") => {
                image |= !["file_name", "file_uuid"]
                    .iter()
                    .filter_map(|key| non_empty(block.get(*key)))
                    .any(|id| known.contains(id));
            }
            Some("voice_note") => audio = true,
            Some("document") => match ["file_name", "title"]
                .iter()
                .find_map(|key| non_empty(block.get(*key)))
            {
                Some(name) => documents.push(name),
                None => nameless = true,
            },
            _ => {}
        }
    }

    let mut lines = Vec::new();
    if image {
        lines.push(IMAGE_NOT_IMPORTED.to_string());
    }
    if audio {
        lines.push(AUDIO_NOT_IMPORTED.to_string());
    }
    let named_in = |key: &'static str| {
        listed(key).filter_map(|item| non_empty(item.get("file_name")).map(|name| (name, item)))
    };
    let mut named: HashSet<&str> = HashSet::new();
    for (name, attachment) in named_in("attachments") {
        named.insert(name);
        lines.push(match non_empty(attachment.get("extracted_content")) {
            Some(text) => format!(
                "{}\n{}",
                attached_line(name),
                clip_part(text.to_string(), MAX_ATTACHMENT_TEXT_CHARS)
            ),
            None => attachment_line(name),
        });
    }
    for name in documents
        .into_iter()
        .chain(named_in("files").map(|(name, _)| name))
    {
        if named.insert(name) {
            lines.push(attachment_line(name));
        }
    }
    if nameless {
        lines.push(DOCUMENT_NOT_IMPORTED.to_string());
    }
    lines
}

/// `value` as trimmed text, when it is a string with any.
fn non_empty(value: Option<&Value>) -> Option<&str> {
    value
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
}

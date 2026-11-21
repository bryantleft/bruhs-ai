//! ChatGPT conversations from a data export (R§1): one record of
//! `conversations.json` or a `conversations-NNN.json` shard becomes the
//! branch the owner last saw.
//!
//! A record's `mapping` is a tree of `{node_id: {message?, parent,
//! children?}}`. 2026 exports drop `children`, so the tree is always
//! rebuilt from `parent`. The visible branch runs from `current_node` (or,
//! when that is missing or dangling, the newest leaf) up to the root. Every
//! walk keeps a visited set and stops at `MAX_NODES`, so a parent cycle or
//! a self-parent ends it instead of looping.

use super::text::{clip_part, export_time, strip_citations, strip_writing_directives};
use super::{ParsedConversation, Transcript};
use bruhs_models::agent::Role;
use bruhs_models::imports::{
    attachment_line, conversation_key, IMAGE_NOT_IMPORTED, MAX_MESSAGES, MAX_NODES, MAX_PART_CHARS,
};
use bruhs_models::logic::portable::{
    title_from_prompt, ImportCandidate, ImportSource, PortableMessage, PortablePart,
};
use bruhs_models::EpochMs;
use serde_json::{Map, Value};

/// Content types that are plumbing, not conversation: custom
/// instructions, memory snapshots, browsing, errors, the python tool and
/// "Thought for Ns" recaps.
const HIDDEN_CONTENT: [&str; 9] = [
    "user_editable_context",
    "model_editable_context",
    "tether_browsing_display",
    "tether_browsing_code",
    "sonic_webpage",
    "system_error",
    "code",
    "execution_output",
    "reasoning_recap",
];

/// Whether `v` is a ChatGPT conversation: an object `mapping`, and no
/// `inserted_at` (DeepSeek's export has a `mapping` too).
pub(crate) fn is_conversation(v: &Value) -> bool {
    v.get("mapping").is_some_and(Value::is_object) && v.get("inserted_at").is_none()
}

/// Whether `v` is a ChatGPT `export_manifest.json`: `logical_files` or
/// `export_files`.
pub(crate) fn is_manifest(v: &Value) -> bool {
    v.get("logical_files").is_some_and(Value::is_object)
        || v.get("export_files").is_some_and(Value::is_array)
}

/// The conversation `v` along its visible branch. `None` when `v` is not a
/// ChatGPT conversation, has no id, or has no visible message.
pub(crate) fn conversation(v: &Value) -> Option<ParsedConversation> {
    if !is_conversation(v) {
        return None;
    }
    let raw_id = ["conversation_id", "id"]
        .iter()
        .find_map(|key| v.get(*key).and_then(Value::as_str))
        .filter(|id| !id.is_empty())?;
    let mapping = v.get("mapping").and_then(Value::as_object)?;
    let tree = Tree::new(mapping);
    let leaf = v
        .get("current_node")
        .and_then(Value::as_str)
        .and_then(|id| tree.index(id))
        .or_else(|| tree.newest_leaf())?;
    let dropped = tree.leaves().filter(|at| *at != leaf).count();

    // Only the newest MAX_MESSAGES messages are kept, so read from the
    // MAX_MESSAGES-th visible prompt from the end, which starts that many
    // messages at least. Prompts that turn out empty make fewer: then read
    // the whole branch.
    let branch = tree.branch(leaf);
    let start = newest_start(&tree, &branch);
    let (mut messages, mut newest) = read(&tree, &branch[start..]);
    if start > 0 && messages.len() < MAX_MESSAGES {
        (messages, newest) = read(&tree, &branch);
    }
    if messages.is_empty() {
        return None;
    }
    if messages.len() > MAX_MESSAGES {
        messages.drain(..messages.len() - MAX_MESSAGES);
    }

    let title = v
        .get("title")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|title| !title.is_empty())
        .map_or_else(
            || {
                let prompt = messages
                    .iter()
                    .find(|message| message.role == Role::User)
                    .map(PortableMessage::text)
                    .unwrap_or_default();
                title_from_prompt(&prompt)
            },
            str::to_string,
        );
    let updated = v
        .get("update_time")
        .and_then(export_time)
        .or_else(|| v.get("create_time").and_then(export_time))
        .max(newest)
        .unwrap_or(0);
    Some(ParsedConversation {
        candidate: ImportCandidate {
            source: ImportSource::ChatGpt,
            id: conversation_key(raw_id),
            title,
            cwd: None,
            updated_at: EpochMs::saturating_from_i64(updated),
            provider: None,
            session_id: None,
            model: None,
        },
        messages,
        dropped_branches: u32::try_from(dropped).unwrap_or(u32::MAX),
    })
}

/// The messages the owner saw along `branch`, and the newest one's time.
fn read(tree: &Tree<'_>, branch: &[usize]) -> (Vec<PortableMessage>, Option<i64>) {
    let mut transcript = Transcript::default();
    let mut newest = None;
    for &at in branch {
        if let Some(time) = tree
            .message(at)
            .and_then(|message| visible(message, &mut transcript))
        {
            newest = newest.max(Some(time));
        }
    }
    (transcript.finish(), newest)
}

/// Where along `branch` the `MAX_MESSAGES`-th visible prompt from the end
/// sits (0 when there are fewer).
fn newest_start(tree: &Tree<'_>, branch: &[usize]) -> usize {
    let mut prompts = 0;
    for (step, &at) in branch.iter().enumerate().rev() {
        if tree
            .message(at)
            .and_then(shown)
            .is_some_and(|(role, _)| role == "user")
        {
            prompts += 1;
            if prompts == MAX_MESSAGES {
                return step;
            }
        }
    }
    0
}

/// A conversation's nodes (at most `MAX_NODES`), sorted by id, and which
/// of them are a parent, rebuilt from each node's `parent`. Nodes are
/// found by binary search and marked by index, so a huge mapping costs no
/// hashing.
struct Tree<'a> {
    nodes: Vec<(&'a str, &'a Value)>,
    /// For each node, its parent's index.
    parent: Vec<Option<usize>>,
    /// For each node, whether another node names it as its parent.
    has_child: Vec<bool>,
}

impl<'a> Tree<'a> {
    fn new(mapping: &'a Map<String, Value>) -> Self {
        let mut nodes: Vec<(&str, &Value)> = mapping
            .iter()
            .take(MAX_NODES)
            .map(|(id, node)| (id.as_str(), node))
            .collect();
        nodes.sort_unstable_by_key(|(id, _)| *id);
        let mut tree = Self {
            parent: Vec::with_capacity(nodes.len()),
            has_child: vec![false; nodes.len()],
            nodes,
        };
        for at in 0..tree.nodes.len() {
            let parent = tree.nodes[at]
                .1
                .get("parent")
                .and_then(Value::as_str)
                .and_then(|id| tree.index(id));
            if let Some(parent) = parent {
                tree.has_child[parent] = true;
            }
            tree.parent.push(parent);
        }
        tree
    }

    fn index(&self, id: &str) -> Option<usize> {
        self.nodes.binary_search_by_key(&id, |(node, _)| *node).ok()
    }

    fn message(&self, at: usize) -> Option<&'a Value> {
        self.nodes.get(at).and_then(|(_, node)| node.get("message"))
    }

    /// Nodes no other node names as its parent.
    fn leaves(&self) -> impl Iterator<Item = usize> + '_ {
        (0..self.nodes.len()).filter(|&at| !self.has_child[at])
    }

    /// The leaf whose message is newest (the greatest id when times tie).
    /// With no leaf at all (every node in a cycle), the newest node.
    fn newest_leaf(&self) -> Option<usize> {
        let time = |at: &usize| {
            self.message(*at)
                .and_then(|message| message.get("create_time"))
                .and_then(export_time)
        };
        let newest = |ids: &mut dyn Iterator<Item = usize>| ids.max_by_key(|at| (time(at), *at));
        newest(&mut self.leaves()).or_else(|| newest(&mut (0..self.nodes.len())))
    }

    /// The nodes from the root down to `leaf`, walked up through `parent`
    /// until the root, a node already seen or `MAX_NODES` steps.
    fn branch(&self, leaf: usize) -> Vec<usize> {
        let mut seen = vec![false; self.nodes.len()];
        let mut path = Vec::new();
        let mut at = Some(leaf);
        while let Some(node) = at {
            if path.len() >= MAX_NODES || seen[node] {
                break;
            }
            seen[node] = true;
            path.push(node);
            at = self.parent[node];
        }
        path.reverse();
        path
    }
}

/// A message's role and content type, when it is one the owner saw: from
/// the user or the assistant, not hidden, not addressed to a tool, and not
/// plumbing.
fn shown(message: &Value) -> Option<(&str, &str)> {
    let role = message.get("author")?.get("role")?.as_str()?;
    if !matches!(role, "user" | "assistant") {
        return None;
    }
    let metadata = message.get("metadata");
    let hidden = metadata
        .and_then(|metadata| metadata.get("is_visually_hidden_from_conversation"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let to_tool = message
        .get("recipient")
        .and_then(Value::as_str)
        .is_some_and(|recipient| recipient != "all");
    let kind = message
        .get("content")?
        .get("content_type")
        .and_then(Value::as_str)
        .unwrap_or("text");
    (!hidden && !to_tool && !HIDDEN_CONTENT.contains(&kind)).then_some((role, kind))
}

/// Adds `message` to `transcript` when the owner saw it, and answers its
/// time.
fn visible(message: &Value, transcript: &mut Transcript) -> Option<i64> {
    let (role, kind) = shown(message)?;
    let content = message.get("content")?;
    let at = message.get("create_time").and_then(export_time);
    let model = message
        .get("metadata")
        .and_then(|metadata| metadata.get("model_slug"))
        .and_then(Value::as_str);
    match role {
        "user" => {
            let text = message_text(content, message);
            let text = text.trim();
            if text.is_empty() {
                return None;
            }
            transcript.user(clip_part(text.to_string(), MAX_PART_CHARS), at);
        }
        "assistant" if kind == "thoughts" => {
            let thinking = thoughts(content);
            if thinking.is_empty() {
                return None;
            }
            transcript.part(
                PortablePart::Thinking(clip_part(thinking, MAX_PART_CHARS)),
                at,
                model,
            );
        }
        "assistant" => {
            let text = strip_writing_directives(&strip_citations(&message_text(content, message)));
            let text = text.trim();
            if text.is_empty() {
                return None;
            }
            transcript.part(
                PortablePart::Text(clip_part(text.to_string(), MAX_PART_CHARS)),
                at,
                model,
            );
        }
        _ => return None,
    }
    at
}

/// A message's text: its parts in order (an image or audio clip as a
/// placeholder line), then a placeholder line per attachment.
fn message_text(content: &Value, message: &Value) -> String {
    let mut lines: Vec<String> = match content.get("parts").and_then(Value::as_array) {
        Some(parts) => parts.iter().filter_map(part_text).collect(),
        None => content
            .get("text")
            .and_then(Value::as_str)
            .map(str::to_string)
            .into_iter()
            .collect(),
    };
    lines.retain(|line| !line.trim().is_empty());
    let mut text = lines.join("\n");
    let attachments: Vec<String> = message
        .get("metadata")
        .and_then(|metadata| metadata.get("attachments"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|attachment| {
            ["name", "id"]
                .iter()
                .find_map(|key| attachment.get(*key).and_then(Value::as_str))
                .filter(|name| !name.trim().is_empty())
        })
        .map(attachment_line)
        .collect();
    if !attachments.is_empty() {
        if !text.is_empty() {
            text.push_str("\n\n");
        }
        text.push_str(&attachments.join("\n"));
    }
    text
}

/// One part of `content.parts`: a string, a `{text}` object, a spoken
/// transcription, or a placeholder for media whose bytes stay behind.
fn part_text(part: &Value) -> Option<String> {
    if let Some(text) = part.as_str() {
        return Some(text.to_string());
    }
    let kind = part.get("content_type").and_then(Value::as_str);
    match kind {
        Some("image_asset_pointer") => Some(IMAGE_NOT_IMPORTED.to_string()),
        Some("audio_asset_pointer" | "real_time_user_audio_video_asset_pointer") => {
            let pointer = part
                .get("asset_pointer")
                .or_else(|| part.pointer("/audio_asset_pointer/asset_pointer"))
                .and_then(Value::as_str)
                .or(kind)?;
            Some(attachment_line(pointer))
        }
        _ => part.get("text").and_then(Value::as_str).map(str::to_string),
    }
}

/// A `thoughts` message as one block: each thought's summary, else its
/// content, joined by blank lines.
fn thoughts(content: &Value) -> String {
    content
        .get("thoughts")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|thought| {
            ["summary", "content"].iter().find_map(|key| {
                thought
                    .get(*key)
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|text| !text.is_empty())
            })
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

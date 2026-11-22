//! Grok conversations from an xAI account export (R§3), best effort (D9):
//! xAI documents no export format, so this follows the shape community
//! tools observed in `prod-grok-backend.json`. Each record of its
//! `conversations` list is `{conversation: {…}, responses: [{response:
//! {…}}]}`; anything else isn't a Grok conversation, and the batch counts
//! it as a record Bruhs didn't recognize. The file's `projects`, `tasks`
//! and `media_posts` never come in.
//!
//! Responses are read in time order (ms, then the owner's before Grok's,
//! then `_id`). When the conversation names its `leaf_response_id` and
//! the responses name their `parent_response_id`, the chain up from that
//! leaf wins instead, and the other leaves are counted as dropped
//! branches. A parent that names nothing joins the response before it in
//! time; a chain that loops falls back to time order. The sender decides
//! the role, matched without regard to case, and never `model`, which
//! human responses carry too.

use super::text::{attachment_line, clip_part, export_time, shows_as_title, strip_grok_markup};
use super::{ParsedConversation, Transcript};
use bruhs_models::imports::{
    conversation_key, IMAGE_NOT_IMPORTED, MAX_MESSAGES, MAX_NODES, MAX_PART_CHARS,
};
use bruhs_models::logic::portable::{
    title_from_prompt, ImportCandidate, ImportSource, PortablePart,
};
use bruhs_models::num::{ToFloatLossy as _, ToIntSaturating as _};
use bruhs_models::EpochMs;
use serde_json::Value;
use std::collections::HashSet;

/// Whether `v` is a Grok conversation record: a `conversation` object and
/// a `responses` list.
pub(crate) fn is_conversation(v: &Value) -> bool {
    v.get("conversation").is_some_and(Value::is_object)
        && v.get("responses").is_some_and(Value::is_array)
}

/// The conversation `v` in the order the owner saw it. `None` when `v` is
/// not a Grok conversation, has no id, or has no visible response.
pub(crate) fn conversation(v: &Value) -> Option<ParsedConversation> {
    if !is_conversation(v) {
        return None;
    }
    let meta = v.get("conversation")?;
    let raw_id = ["id", "conversation_id"].iter().find_map(|key| {
        meta.get(*key)
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty())
    })?;
    let responses: Vec<Response<'_>> = v
        .get("responses")?
        .as_array()?
        .iter()
        .filter_map(Response::new)
        .take(MAX_NODES)
        .collect();
    let by_time = time_order(&responses);
    let (order, dropped) = leaf_chain(meta, &responses, &by_time).unwrap_or((by_time, 0));

    let mut transcript = Transcript::default();
    let mut newest = None;
    let mut first_prompt = None;
    for &at in &order {
        let response = &responses[at];
        if first_prompt.is_none() && response.sender == Some(Sender::Human) {
            first_prompt = typed(response.value);
        }
        if add(response, &mut transcript) {
            newest = newest.max(response.time);
        }
    }
    let mut messages = transcript.finish();
    if messages.is_empty() {
        return None;
    }
    if messages.len() > MAX_MESSAGES {
        messages.drain(..messages.len() - MAX_MESSAGES);
    }

    let title = non_empty(meta.get("title"))
        .filter(|title| shows_as_title(title))
        .map_or_else(
            || title_from_prompt(&first_prompt.unwrap_or_default()),
            str::to_string,
        );
    // Grok's modify_time lags the newest response by up to days.
    let updated = meta
        .get("modify_time")
        .and_then(export_time)
        .or_else(|| meta.get("create_time").and_then(export_time))
        .max(newest)
        .unwrap_or(0);
    Some(ParsedConversation {
        candidate: ImportCandidate {
            source: ImportSource::Grok,
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

#[derive(Clone, Copy, PartialEq, Eq)]
enum Sender {
    Human,
    Assistant,
}

/// One response, read once.
struct Response<'a> {
    value: &'a Value,
    /// `_id`, or empty.
    id: &'a str,
    parent: Option<&'a str>,
    time: Option<i64>,
    sender: Option<Sender>,
}

impl<'a> Response<'a> {
    /// The response in `entry`: its `response` object, or the entry itself
    /// when it isn't wrapped. `None` for anything but an object.
    fn new(entry: &'a Value) -> Option<Self> {
        let value = match entry.get("response") {
            Some(inner) if inner.is_object() => inner,
            _ => entry,
        };
        if !value.is_object() {
            return None;
        }
        let sender = value.get("sender").and_then(Value::as_str);
        Some(Self {
            value,
            id: value.get("_id").and_then(Value::as_str).unwrap_or_default(),
            parent: value
                .get("parent_response_id")
                .and_then(Value::as_str)
                .filter(|id| !id.is_empty()),
            time: value.get("create_time").and_then(export_time),
            sender: if sender.is_some_and(|s| s.eq_ignore_ascii_case("human")) {
                Some(Sender::Human)
            } else if sender.is_some_and(|s| s.eq_ignore_ascii_case("assistant")) {
                Some(Sender::Assistant)
            } else {
                None
            },
        })
    }

    /// The owner's responses sort before Grok's at the same time.
    fn rank(&self) -> u8 {
        match self.sender {
            Some(Sender::Human) => 0,
            Some(Sender::Assistant) => 1,
            None => 2,
        }
    }
}

/// Every response by time, then the owner's first, then `_id`, then the
/// list's order. A response with no time sorts at the time of the one
/// before it in the list, so it stays beside it.
fn time_order(responses: &[Response<'_>]) -> Vec<usize> {
    let mut last = None;
    let times: Vec<Option<i64>> = responses
        .iter()
        .map(|response| {
            last = response.time.or(last);
            last
        })
        .collect();
    let mut order: Vec<usize> = (0..responses.len()).collect();
    order.sort_by_key(|&at| (times[at], responses[at].rank(), responses[at].id));
    order
}

/// The responses from the first down to `leaf_response_id`, and how many
/// other leaves stayed behind. `None` when the conversation names no leaf
/// that is there, no response names a parent, or the chain loops.
fn leaf_chain(
    meta: &Value,
    responses: &[Response<'_>],
    by_time: &[usize],
) -> Option<(Vec<usize>, usize)> {
    let leaf_id = meta
        .get("leaf_response_id")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())?;
    if responses.iter().all(|response| response.parent.is_none()) {
        return None;
    }
    // Each response's id, sorted, so a parent is found by binary search.
    let mut ids: Vec<(&str, usize)> = responses
        .iter()
        .enumerate()
        .filter(|(_, response)| !response.id.is_empty())
        .map(|(at, response)| (response.id, at))
        .collect();
    ids.sort_unstable();
    let find = |id: &str| {
        ids.binary_search_by_key(&id, |(response, _)| *response)
            .ok()
            .map(|found| ids[found].1)
    };
    let leaf = find(leaf_id)?;

    let count = responses.len();
    let mut previous = vec![None; count];
    for pair in by_time.windows(2) {
        previous[pair[1]] = Some(pair[0]);
    }
    let parent: Vec<Option<usize>> = (0..count)
        .map(|at| responses[at].parent.and_then(find).or(previous[at]))
        .collect();

    let mut seen = vec![false; count];
    let mut path = Vec::new();
    let mut at = Some(leaf);
    while let Some(node) = at {
        if seen[node] {
            return None;
        }
        seen[node] = true;
        path.push(node);
        at = parent[node];
    }
    path.reverse();

    let mut has_child = vec![false; count];
    for &up in parent.iter().flatten() {
        has_child[up] = true;
    }
    let leaves = has_child.iter().filter(|&&child| !child).count();
    Some((path, leaves - usize::from(!has_child[leaf])))
}

/// Adds `response` to `transcript` when it shows anything: text, or a
/// placeholder for what stays behind. Reasoning alone, from an answer
/// that never came, doesn't count. Answers whether it was added.
fn add(response: &Response<'_>, transcript: &mut Transcript) -> bool {
    let value = response.value;
    let at = response.time;
    match response.sender {
        Some(Sender::Human) => {
            // What the owner typed is never rewritten.
            let text = with_lines(typed(value).unwrap_or_default(), left_behind(value));
            if text.is_empty() {
                return false;
            }
            transcript.user(clip_part(text, MAX_PART_CHARS), at);
        }
        Some(Sender::Assistant) => {
            let message = non_empty(value.get("message"))
                .map(strip_grok_markup)
                .unwrap_or_default();
            let text = with_lines(message.trim().to_string(), left_behind(value));
            if text.is_empty() {
                return false;
            }
            let model = value.get("model").and_then(Value::as_str);
            if let Some(thinking) = thinking(value) {
                transcript.part(
                    PortablePart::Thinking(clip_part(thinking, MAX_PART_CHARS)),
                    at,
                    model,
                );
            }
            transcript.part(
                PortablePart::Text(clip_part(text, MAX_PART_CHARS)),
                at,
                model,
            );
        }
        None => return false,
    }
    true
}

/// What the owner typed in a response, trimmed.
fn typed(value: &Value) -> Option<String> {
    non_empty(value.get("message")).map(str::to_string)
}

/// `text`, then a blank line and `lines` one per line.
fn with_lines(mut text: String, lines: Vec<String>) -> String {
    if !lines.is_empty() {
        if !text.is_empty() {
            text.push_str("\n\n");
        }
        text.push_str(&lines.join("\n"));
    }
    text
}

/// `thinking_trace` as one block, its markup stripped: a string, or a
/// list of strings joined by blank lines.
fn thinking(value: &Value) -> Option<String> {
    let trace = value.get("thinking_trace")?;
    let text = match trace {
        Value::Array(items) => items
            .iter()
            .filter_map(|item| non_empty(Some(item)))
            .collect::<Vec<_>>()
            .join("\n\n"),
        _ => non_empty(Some(trace))?.to_string(),
    };
    let text = strip_grok_markup(&text);
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_string())
}

/// A placeholder line for each thing in a response whose bytes stay
/// behind (D17): `IMAGE_NOT_IMPORTED` once for generated images or image
/// cards (never their links), then `attachment_line` for each attached
/// asset id, once each.
fn left_behind(value: &Value) -> Vec<String> {
    let listed = |key: &str| {
        value
            .get(key)
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
    };
    let mut lines = Vec::new();
    let generated = listed("generated_image_urls").any(|url| non_empty(Some(url)).is_some());
    if generated || listed("card_attachments_json").any(is_image_card) {
        lines.push(IMAGE_NOT_IMPORTED.to_string());
    }
    let mut seen: HashSet<&str> = HashSet::new();
    for asset in listed("file_attachments").filter_map(|file| match file {
        Value::Object(_) => ["name", "file_name", "asset_id", "id"]
            .iter()
            .find_map(|key| non_empty(file.get(*key))),
        _ => non_empty(Some(file)),
    }) {
        if seen.insert(asset) {
            lines.push(attachment_line(asset));
        }
    }
    lines
}

/// Whether a `card_attachments_json` entry is an image card: as JSON
/// text, or as an object whose card type says so.
fn is_image_card(card: &Value) -> bool {
    match card {
        Value::String(text) => text.contains("image_card"),
        Value::Object(_) => ["cardType", "card_type", "type"]
            .iter()
            .any(|key| card.get(*key).and_then(Value::as_str) == Some("image_card")),
        _ => false,
    }
}

/// `value` as trimmed text, when it is a string with any.
fn non_empty(value: Option<&Value>) -> Option<&str> {
    value
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
}

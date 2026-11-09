//! Messages between chat threads: what an agent running in one thread may
//! ask about the others, and how the app answers.
//!
//! The app that owns the threads (web or desktop) answers every request;
//! the provider, the MCP helper and the server only relay it. A message to
//! another thread is delivered through that thread's prompt queue, so it
//! runs as an ordinary turn there and the user can still edit or remove it
//! while it waits.
//!
//! A delivered message opens with a header line naming the sender and a hop
//! count ([`delivery_prompt`]). The count is read back from the prompt of
//! the turn that sends the next message ([`prompt_hops`]), so a chain of
//! agents messaging each other stops at the user's limit ([`parse_hops`])
//! without either client storing anything new. [`parse_delivery`] reads the
//! header back for display.
//!
//! A sender may wait for the answer inside its tool call where the app can
//! run two threads at once (desktop). The recipient's reply to that turn is
//! then the tool result ([`answer_reply`]).

use serde::{Deserialize, Serialize};

use crate::ids::ThreadId;

/// Longest chain of agent-to-agent messages after the user's own prompt,
/// unless the user picks another of [`HOP_CHOICES`].
pub const MAX_HOPS: u32 = 6;
/// The chain lengths Settings offers.
pub const HOP_CHOICES: [u32; 4] = [2, 4, 6, 10];
/// Threads one `list_threads` reply names.
pub const LIST_LIMIT: usize = 30;
/// Messages `read_thread` returns when the caller gives no limit.
pub const READ_DEFAULT: usize = 6;
/// Most messages one `read_thread` reply returns.
pub const READ_MAX: usize = 20;
/// Longest message one thread may send another, in characters.
pub const MAX_MESSAGE_CHARS: usize = 16_000;
/// Longest title for a thread an agent starts, in characters.
pub const MAX_TITLE_CHARS: usize = 80;
/// Longest thread reference accepted, in bytes.
const MAX_REFERENCE_BYTES: usize = crate::ids::MAX_ID_LEN;
/// Characters of one message kept in a `read_thread` reply.
const READ_MESSAGE_CHARS: usize = 4_000;
/// Characters of messages one `read_thread` reply holds in total.
const READ_TOTAL_CHARS: usize = 16_000;
/// Characters of a waited-for answer returned to the sender.
const ANSWER_CHARS: usize = 16_000;
/// Characters of a thread id shown in replies. A longer prefix, or the whole
/// id, is printed when this many are not unique.
const SHORT_ID_CHARS: usize = 8;
/// Fewest characters accepted as a thread id prefix.
const MIN_PREFIX_CHARS: usize = 4;

const HEADER_OPEN: &str = "[Message from thread \"";
const HEADER_ID: &str = "\" (id ";
const HEADER_HOP: &str = "), hop ";
/// Follows the hop count when the sender waits for the reply.
const HEADER_AWAITED: &str = ", awaiting your reply";

/// The stored hop limit: one of [`HOP_CHOICES`], else [`MAX_HOPS`].
pub fn parse_hops(stored: &str) -> u32 {
    stored
        .trim()
        .parse()
        .ok()
        .filter(|hops| HOP_CHOICES.contains(hops))
        .unwrap_or(MAX_HOPS)
}

/// What the user lets agents do with other threads.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThreadAccess {
    /// List, read and message them.
    #[default]
    Message,
    /// List and read them only.
    Read,
    /// No thread tools at all.
    Off,
}

impl ThreadAccess {
    pub fn parse(stored: &str) -> Self {
        match stored {
            "read" => Self::Read,
            "off" => Self::Off,
            _ => Self::Message,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Message => "message",
            Self::Read => "read",
            Self::Off => "off",
        }
    }
}

#[expect(
    clippy::trivially_copy_pass_by_ref,
    reason = "serde's skip_serializing_if passes a reference"
)]
const fn is_false(value: &bool) -> bool {
    !*value
}

/// What an agent asks of the app's threads.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum ThreadOperation {
    /// Name the app's threads.
    List,
    /// Read the latest messages of one thread.
    Read {
        thread_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        limit: Option<u32>,
    },
    /// Send a message to a thread. No `thread_id` starts a new thread.
    Message {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        thread_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        title: Option<String>,
        message: String,
        /// Hold the tool call until the recipient has replied.
        #[serde(default, skip_serializing_if = "is_false")]
        wait: bool,
    },
}

impl ThreadOperation {
    /// Whether the operation changes anything. Read-only turns get the rest.
    pub const fn writes(&self) -> bool {
        matches!(self, Self::Message { .. })
    }

    /// Whether the caller asked to wait for the recipient's reply.
    pub const fn waits(&self) -> bool {
        matches!(self, Self::Message { wait: true, .. })
    }

    /// Reject what no client could answer, before it reaches one.
    pub fn validate(&self) -> Result<(), &'static str> {
        let reference = |id: &str| {
            if id.trim().is_empty() {
                Err("thread_id is empty.")
            } else if id.len() > MAX_REFERENCE_BYTES {
                Err("thread_id is too long.")
            } else {
                Ok(())
            }
        };
        match self {
            Self::List => Ok(()),
            Self::Read { thread_id, .. } => reference(thread_id),
            Self::Message {
                thread_id,
                title,
                message,
                ..
            } => {
                if let Some(id) = thread_id {
                    reference(id)?;
                }
                if title
                    .as_deref()
                    .is_some_and(|title| title.chars().count() > MAX_TITLE_CHARS)
                {
                    return Err("title is too long.");
                }
                if message.trim().is_empty() {
                    return Err("message is empty.");
                }
                if message.chars().count() > MAX_MESSAGE_CHARS {
                    return Err("message is too long.");
                }
                Ok(())
            }
        }
    }
}

/// One pending request, streamed to the app that owns the calling thread.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThreadRequest {
    pub id: String,
    /// The thread whose turn asked.
    pub thread_id: ThreadId,
    pub operation: ThreadOperation,
}

/// The app's answer. `text` is what the model reads.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThreadReply {
    pub ok: bool,
    pub text: String,
}

impl ThreadReply {
    pub fn ok(text: impl Into<String>) -> Self {
        Self {
            ok: true,
            text: text.into(),
        }
    }

    pub fn refused(text: impl Into<String>) -> Self {
        Self {
            ok: false,
            text: text.into(),
        }
    }
}

/// Body of the app's answer to a [`ThreadRequest`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ThreadReplySubmission {
    pub thread_id: ThreadId,
    pub request_id: String,
    pub reply: ThreadReply,
}

/// One thread as the app knows it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ThreadCard<'a> {
    pub id: &'a str,
    pub title: &'a str,
    pub project: Option<&'a str>,
    /// A turn is streaming in it.
    pub running: bool,
    /// Prompts waiting in its queue.
    pub queued: usize,
}

/// One settled message of a thread.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ThreadLine<'a> {
    pub from_user: bool,
    pub text: &'a str,
}

/// Where a message goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Recipient {
    /// The thread with this id.
    Existing(String),
    /// A thread to create with this title, in the sender's project.
    New { title: String },
}

/// A message ready to queue: [`plan_message`]'s approval.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Delivery {
    pub recipient: Recipient,
    /// The prompt to queue in the recipient, header included.
    pub prompt: String,
}

/// One `message_thread` call, as the app sees it.
#[derive(Debug, Clone, Copy)]
pub struct Outgoing<'a> {
    pub caller_id: &'a str,
    pub caller_title: &'a str,
    /// The prompt of the turn that is sending.
    pub caller_prompt: &'a str,
    /// Every thread the app has.
    pub ids: &'a [&'a str],
    pub thread_id: Option<&'a str>,
    pub title: Option<&'a str>,
    pub message: &'a str,
    /// The sender will wait for the reply, and the app can let it.
    pub wait: bool,
    /// The user's hop limit.
    pub max_hops: u32,
}

/// What became of a message once it was handed to its recipient.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeliveryState {
    /// The recipient began its turn at once.
    Started,
    /// It waits in the recipient's queue.
    Queued,
    /// It waits in the queue, and the sender asked to wait for the reply
    /// but the app cannot hold the call (one turn at a time, or a paused
    /// queue).
    QueuedNotAwaited,
}

/// How a waited-for message ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Answer<'a> {
    /// The recipient's turn finished with this reply.
    Replied(&'a str),
    /// The user stopped the recipient's turn.
    Stopped,
    /// The recipient's turn failed with this error.
    Failed(&'a str),
    /// The message left the queue before it ran, or its thread was deleted.
    Removed,
}

/// A delivered message's header, read back from its prompt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeliveryView<'a> {
    pub from_title: &'a str,
    pub from_id: &'a str,
    pub hops: u32,
    /// The sender is waiting for the reply to this message.
    pub awaited: bool,
    /// What the sending agent wrote.
    pub body: &'a str,
}

impl DeliveryView<'_> {
    /// The line both apps show above the message.
    pub fn caption(&self) -> String {
        if self.awaited {
            format!("From thread “{}” · waiting for this reply", self.from_title)
        } else {
            format!("From thread “{}”", self.from_title)
        }
    }
}

/// A title on one line with nothing that could end the header early.
fn header_title(title: &str) -> String {
    let cleaned: String = title
        .chars()
        .map(|c| match c {
            '"' => '\'',
            c if c.is_control() => ' ',
            c => c,
        })
        .collect();
    let trimmed = cleaned.trim();
    if trimmed.is_empty() {
        "Untitled".to_string()
    } else {
        clip(trimmed, MAX_TITLE_CHARS)
    }
}

/// The first `max` characters of `text`, with an ellipsis when cut.
fn clip(text: &str, max: usize) -> String {
    match text.char_indices().nth(max) {
        Some((end, _)) => format!("{}…", text[..end].trim_end()),
        None => text.to_string(),
    }
}

/// The shortest prefix of `id` (at least [`SHORT_ID_CHARS`] characters) that
/// no other id in `ids` starts with.
pub fn short_id<'a>(id: &'a str, ids: &[&str]) -> &'a str {
    let mut end = id
        .char_indices()
        .nth(SHORT_ID_CHARS)
        .map_or(id.len(), |(index, _)| index);
    loop {
        let prefix = &id[..end];
        // `ids` may or may not hold `id` itself, even twice.
        let unique = !ids
            .iter()
            .any(|other| *other != id && other.starts_with(prefix));
        if unique || end == id.len() {
            return prefix;
        }
        end = id[end..]
            .char_indices()
            .nth(1)
            .map_or(id.len(), |(index, _)| end + index);
    }
}

/// The id in `ids` that `reference` names: the whole id, or a prefix only
/// one id starts with.
pub fn resolve_thread<'a>(reference: &str, ids: &[&'a str]) -> Result<&'a str, ThreadReply> {
    let reference = reference.trim();
    if let Some(exact) = ids.iter().find(|id| **id == reference) {
        return Ok(exact);
    }
    if reference.chars().count() < MIN_PREFIX_CHARS {
        return Err(ThreadReply::refused(
            "No thread has that id. Call list_threads for current ids.",
        ));
    }
    let mut matches = ids.iter().filter(|id| id.starts_with(reference));
    match (matches.next(), matches.next()) {
        (Some(only), None) => Ok(only),
        (Some(_), Some(_)) => Err(ThreadReply::refused(
            "More than one thread id starts with that. Use the full id from list_threads.",
        )),
        _ => Err(ThreadReply::refused(
            "No thread has that id. Call list_threads for current ids.",
        )),
    }
}

/// The `list_threads` reply. `threads` is most recent first; `caller` is the
/// id of the thread that asked.
pub fn list_reply(threads: &[ThreadCard<'_>], caller: &str) -> ThreadReply {
    let ids: Vec<&str> = threads.iter().map(|thread| thread.id).collect();
    let mut text = String::from("Threads, most recent first (id · title · project · state):\n");
    for thread in threads.iter().take(LIST_LIMIT) {
        let state = if thread.id == caller {
            "this thread".to_string()
        } else {
            match (thread.running, thread.queued) {
                (true, 0) => "running".to_string(),
                (true, queued) => format!("running, {queued} queued"),
                (false, 0) => "idle".to_string(),
                (false, queued) => format!("idle, {queued} queued"),
            }
        };
        text.push_str(&format!(
            "{} · {} · {} · {state}\n",
            short_id(thread.id, &ids),
            header_title(thread.title),
            thread.project.map_or("no project", str::trim),
        ));
    }
    if threads.len() > LIST_LIMIT {
        text.push_str(&format!(
            "{} older threads not shown.\n",
            threads.len() - LIST_LIMIT
        ));
    }
    ThreadReply::ok(text)
}

/// The `read_thread` reply: the last `limit` settled messages of a thread,
/// oldest first, each clipped, the whole reply inside a fixed budget.
pub fn read_reply(
    title: &str,
    running: bool,
    lines: &[ThreadLine<'_>],
    limit: Option<u32>,
) -> ThreadReply {
    let limit = limit
        .and_then(|limit| usize::try_from(limit).ok())
        .map_or(READ_DEFAULT, |limit| limit.clamp(1, READ_MAX));
    let spoken: Vec<&ThreadLine<'_>> = lines
        .iter()
        .filter(|line| !line.text.trim().is_empty())
        .collect();
    let mut kept: Vec<(bool, String)> = Vec::new();
    let mut budget = READ_TOTAL_CHARS;
    for line in spoken.iter().rev().take(limit) {
        let text = clip(line.text.trim(), READ_MESSAGE_CHARS);
        let cost = text.chars().count();
        if cost > budget && !kept.is_empty() {
            break;
        }
        budget = budget.saturating_sub(cost);
        kept.push((line.from_user, text));
    }
    kept.reverse();
    let state = if running {
        "a turn is running; its reply is not included"
    } else {
        "idle"
    };
    let mut text = format!(
        "Thread \"{}\" ({state}). Last {} of {} messages:\n",
        header_title(title),
        kept.len(),
        spoken.len(),
    );
    if kept.is_empty() {
        text.push_str("\n(no messages yet)\n");
    }
    for (from_user, body) in kept {
        text.push_str(if from_user {
            "\nUser:\n"
        } else {
            "\nAssistant:\n"
        });
        text.push_str(&body);
        text.push('\n');
    }
    ThreadReply::ok(text)
}

/// The prompt a recipient thread runs: a header naming the sender and the
/// hop count, then the message. `awaited` tells the recipient its reply is
/// returned to the sender, so it must not message back.
pub fn delivery_prompt(
    from_title: &str,
    from_id: &str,
    hops: u32,
    max_hops: u32,
    awaited: bool,
    message: &str,
) -> String {
    let (mark, how) = if awaited {
        (
            HEADER_AWAITED,
            "and that agent is waiting: your final reply to this message is returned to it, so do not message it back.",
        )
    } else {
        (
            "",
            "Answer it with message_thread to that id only if it asks for an answer.",
        )
    };
    format!(
        "{HEADER_OPEN}{}{HEADER_ID}{from_id}{HEADER_HOP}{hops}/{max_hops}{mark}. It was sent by that thread's agent, not typed by the user. {how}]\n\n{}",
        header_title(from_title),
        message.trim(),
    )
}

/// The header of a delivered message, or `None` for a prompt the user
/// wrote.
pub fn parse_delivery(prompt: &str) -> Option<DeliveryView<'_>> {
    let trimmed = prompt.trim_start();
    let (first, rest) = trimmed.split_once('\n').unwrap_or((trimmed, ""));
    let named = first.strip_prefix(HEADER_OPEN)?;
    // The last occurrence: a title may hold the same text.
    let hop_at = named.rfind(HEADER_HOP)?;
    let (from_title, from_id) = named[..hop_at].rsplit_once(HEADER_ID)?;
    let after = &named[hop_at + HEADER_HOP.len()..];
    let (count, after) = after.split_once('/')?;
    let hops = count.parse().ok()?;
    let limit_end = after
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(after.len());
    Some(DeliveryView {
        from_title,
        from_id,
        hops,
        awaited: after[limit_end..].starts_with(HEADER_AWAITED),
        body: rest.trim_start(),
    })
}

/// How many agent-to-agent hops led to `prompt`: the count in its delivery
/// header, or 0 for a prompt the user wrote.
pub fn prompt_hops(prompt: &str) -> u32 {
    parse_delivery(prompt).map_or(0, |delivery| delivery.hops)
}

/// `prompt` without its delivery header: what the sending agent wrote. A
/// prompt the user wrote comes back whole. For titles and previews.
pub fn delivery_body(prompt: &str) -> &str {
    parse_delivery(prompt).map_or(prompt, |delivery| delivery.body)
}

/// Decide a `message_thread` call.
///
/// # Errors
///
/// The refusal to return to the model: the hop limit, an unknown or
/// ambiguous recipient, or a thread messaging itself.
pub fn plan_message(outgoing: &Outgoing<'_>) -> Result<Delivery, ThreadReply> {
    let hops = prompt_hops(outgoing.caller_prompt).saturating_add(1);
    let max_hops = outgoing.max_hops;
    if hops > max_hops {
        return Err(ThreadReply::refused(format!(
            "Not sent: this conversation between threads reached its limit of {max_hops} messages without the user. Summarize where things stand and let the user decide what happens next."
        )));
    }
    let recipient = match outgoing.thread_id {
        Some(reference) => {
            let id = resolve_thread(reference, outgoing.ids)?;
            if id == outgoing.caller_id {
                return Err(ThreadReply::refused(
                    "Not sent: that is this thread. Pick another thread, or omit thread_id to start a new one.",
                ));
            }
            Recipient::Existing(id.to_string())
        }
        None => Recipient::New {
            title: header_title(
                outgoing
                    .title
                    .filter(|title| !title.trim().is_empty())
                    .unwrap_or_else(|| outgoing.message.trim().lines().next().unwrap_or_default()),
            ),
        },
    };
    Ok(Delivery {
        recipient,
        prompt: delivery_prompt(
            outgoing.caller_title,
            short_id(outgoing.caller_id, outgoing.ids),
            hops,
            max_hops,
            outgoing.wait,
            outgoing.message,
        ),
    })
}

/// The `message_thread` reply once the message is handed over and nobody
/// waits for its answer.
pub fn delivered_reply(
    recipient_id: &str,
    recipient_title: &str,
    ids: &[&str],
    created: bool,
    state: DeliveryState,
) -> ThreadReply {
    let what = if created {
        "Started a new thread"
    } else {
        "Sent to thread"
    };
    let when = match state {
        DeliveryState::Started => "It is running now.",
        DeliveryState::Queued => {
            "It is queued there and runs when the app is free to run that thread."
        }
        DeliveryState::QueuedNotAwaited => {
            "It is queued there and runs when the app is free to run that thread, so this call could not wait for it."
        }
    };
    ThreadReply::ok(format!(
        "{what} {} \"{}\". {when} Any answer arrives later as a new message in this thread, so do not wait or poll for it: finish this turn.",
        short_id(recipient_id, ids),
        header_title(recipient_title),
    ))
}

/// The `message_thread` reply when the sender waited: the recipient's
/// answer, or why there is none.
pub fn answer_reply(
    recipient_id: &str,
    recipient_title: &str,
    ids: &[&str],
    answer: Answer<'_>,
) -> ThreadReply {
    let thread = format!(
        "Thread {} \"{}\"",
        short_id(recipient_id, ids),
        header_title(recipient_title)
    );
    match answer {
        Answer::Replied(text) if text.trim().is_empty() => ThreadReply::ok(format!(
            "{thread} finished its turn without writing a reply. Use read_thread to see what it did."
        )),
        Answer::Replied(text) => ThreadReply::ok(format!(
            "{thread} answered:\n\n{}",
            clip(text.trim(), ANSWER_CHARS)
        )),
        Answer::Stopped => ThreadReply::refused(format!(
            "{thread} was stopped by the user before it answered."
        )),
        Answer::Failed(error) => ThreadReply::refused(format!(
            "{thread} could not answer: {}",
            clip(error.trim(), READ_MESSAGE_CHARS)
        )),
        Answer::Removed => ThreadReply::refused(format!(
            "{thread} never ran the message: it was removed from that thread before its turn."
        )),
    }
}

/// The refusal when the recipient is itself waiting on another thread, so a
/// second wait could never finish.
pub fn recipient_waiting_reply(
    recipient_id: &str,
    recipient_title: &str,
    ids: &[&str],
) -> ThreadReply {
    ThreadReply::refused(format!(
        "Not sent: thread {} \"{}\" is itself waiting for another thread's reply, so it cannot answer inside this call. Send again without wait.",
        short_id(recipient_id, ids),
        header_title(recipient_title),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: &str = "11111111-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
    const B: &str = "22222222-bbbb-4bbb-8bbb-bbbbbbbbbbbb";
    const B2: &str = "22222222-cccc-4ccc-8ccc-cccccccccccc";

    fn outgoing<'a>(ids: &'a [&'a str], to: Option<&'a str>, message: &'a str) -> Outgoing<'a> {
        Outgoing {
            caller_id: A,
            caller_title: "Sender",
            caller_prompt: "",
            ids,
            thread_id: to,
            title: None,
            message,
            wait: false,
            max_hops: MAX_HOPS,
        }
    }

    #[test]
    fn operations_round_trip_and_reject_unknown_fields() {
        let message: ThreadOperation =
            serde_json::from_str(r#"{"op":"message","message":"hi"}"#).unwrap();
        assert_eq!(
            message,
            ThreadOperation::Message {
                thread_id: None,
                title: None,
                message: "hi".into(),
                wait: false,
            }
        );
        assert!(message.writes() && !message.waits() && !ThreadOperation::List.writes());
        // An unset wait stays off the wire, so older readers still parse it.
        assert_eq!(
            serde_json::to_string(&message).unwrap(),
            r#"{"op":"message","message":"hi"}"#
        );
        let waiting: ThreadOperation =
            serde_json::from_str(r#"{"op":"message","message":"hi","wait":true}"#).unwrap();
        assert!(waiting.waits());
        assert!(
            serde_json::from_str::<ThreadOperation>(r#"{"op":"read","thread_id":"a","x":1}"#)
                .is_err()
        );
        assert!(serde_json::from_str::<ThreadOperation>(r#"{"op":"delete"}"#).is_err());
    }

    #[test]
    fn validation_bounds_every_field() {
        let message = |thread_id: Option<&str>, title: Option<String>, message: String| {
            ThreadOperation::Message {
                thread_id: thread_id.map(str::to_string),
                title,
                message,
                wait: false,
            }
            .validate()
        };
        assert!(message(None, None, "hi".into()).is_ok());
        assert!(message(None, None, "  ".into()).is_err());
        assert!(message(Some(" "), None, "hi".into()).is_err());
        assert!(message(None, Some("t".repeat(MAX_TITLE_CHARS + 1)), "hi".into()).is_err());
        assert!(message(None, None, "m".repeat(MAX_MESSAGE_CHARS + 1)).is_err());
        assert!(ThreadOperation::Read {
            thread_id: "x".repeat(MAX_REFERENCE_BYTES + 1),
            limit: None
        }
        .validate()
        .is_err());
    }

    #[test]
    fn stored_preferences_fall_back_to_the_defaults() {
        for access in [ThreadAccess::Message, ThreadAccess::Read, ThreadAccess::Off] {
            assert_eq!(ThreadAccess::parse(access.as_str()), access);
            assert_eq!(
                serde_json::to_string(&access).unwrap(),
                format!("\"{}\"", access.as_str())
            );
        }
        assert_eq!(ThreadAccess::parse(""), ThreadAccess::Message);
        assert_eq!(ThreadAccess::default(), ThreadAccess::Message);
        assert!(HOP_CHOICES.contains(&MAX_HOPS));
        for hops in HOP_CHOICES {
            assert_eq!(parse_hops(&hops.to_string()), hops);
        }
        for stored in ["", "0", "7", "999", "six"] {
            assert_eq!(parse_hops(stored), MAX_HOPS, "{stored:?}");
        }
    }

    #[test]
    fn short_ids_grow_until_unique_and_resolve_back() {
        let ids = [A, B, B2];
        assert_eq!(short_id(A, &ids), "11111111");
        let b = short_id(B, &ids);
        assert_eq!(b, "22222222-b");
        assert_eq!(resolve_thread(b, &ids), Ok(B));
        assert_eq!(resolve_thread(A, &ids), Ok(A));
        assert!(resolve_thread("22222222", &ids)
            .unwrap_err()
            .text
            .contains("More than one"));
        assert!(!resolve_thread("111", &ids).unwrap_err().ok);
        assert!(!resolve_thread("99999999", &ids).unwrap_err().ok);
        // An id shorter than the short length is its own short id.
        assert_eq!(short_id("abc", &["abc"]), "abc");
        // Listing the id itself, even twice, does not lengthen it.
        assert_eq!(short_id(A, &[A, A, B]), "11111111");
        assert_eq!(short_id(A, &[B]), "11111111");
    }

    #[test]
    fn list_names_each_thread_once_with_its_state() {
        let card = |id, title, running, queued| ThreadCard {
            id,
            title,
            project: None,
            running,
            queued,
        };
        let reply = list_reply(
            &[
                card(A, "Fix \"login\"\nbug", true, 0),
                ThreadCard {
                    project: Some("bruhs-ai"),
                    ..card(B, "Research", true, 2)
                },
            ],
            A,
        );
        assert!(reply.ok);
        assert!(reply
            .text
            .contains("11111111 · Fix 'login' bug · no project · this thread\n"));
        assert!(reply
            .text
            .contains("22222222 · Research · bruhs-ai · running, 2 queued\n"));
    }

    #[test]
    fn list_caps_what_it_names() {
        let ids: Vec<String> = (0..LIST_LIMIT + 3)
            .map(|n| format!("{n:08}-0000-4000-8000-000000000000"))
            .collect();
        let cards: Vec<ThreadCard<'_>> = ids
            .iter()
            .map(|id| ThreadCard {
                id,
                title: "t",
                project: None,
                running: false,
                queued: 0,
            })
            .collect();
        let reply = list_reply(&cards, "none");
        assert_eq!(reply.text.matches(" · idle\n").count(), LIST_LIMIT);
        assert!(reply.text.ends_with("3 older threads not shown.\n"));
    }

    #[test]
    fn read_keeps_the_latest_messages_inside_its_budget() {
        let long = "x".repeat(READ_MESSAGE_CHARS + 50);
        let lines = [
            ThreadLine {
                from_user: true,
                text: "first",
            },
            ThreadLine {
                from_user: false,
                text: "  ",
            },
            ThreadLine {
                from_user: false,
                text: &long,
            },
            ThreadLine {
                from_user: true,
                text: "last",
            },
        ];
        let reply = read_reply("T", false, &lines, Some(2));
        assert!(reply
            .text
            .starts_with("Thread \"T\" (idle). Last 2 of 3 messages:\n"));
        assert!(!reply.text.contains("first"));
        assert!(reply.text.contains("…\n\nUser:\nlast\n"));
        assert!(reply.text.chars().count() < READ_MESSAGE_CHARS + 200);

        let many: Vec<ThreadLine<'_>> = (0..READ_MAX + 5)
            .map(|_| ThreadLine {
                from_user: false,
                text: &long,
            })
            .collect();
        let reply = read_reply("T", true, &many, Some(999));
        assert!(reply.text.contains("a turn is running"));
        assert!(reply.text.chars().count() <= READ_TOTAL_CHARS + 400);
        assert!(read_reply("T", false, &[], None)
            .text
            .contains("(no messages yet)"));
    }

    #[test]
    fn a_delivery_header_reads_back_and_only_a_header_is_one() {
        let prompt = delivery_prompt(
            "Plan), hop 9/9 \"x\"",
            "11111111",
            3,
            10,
            false,
            "  do it  ",
        );
        assert!(prompt.ends_with("]\n\ndo it"));
        assert_eq!(
            parse_delivery(&prompt),
            Some(DeliveryView {
                from_title: "Plan), hop 9/9 'x'",
                from_id: "11111111",
                hops: 3,
                awaited: false,
                body: "do it",
            })
        );
        assert_eq!(prompt_hops(&prompt), 3);
        assert_eq!(delivery_body(&prompt), "do it");
        // A goal appended after the message is still its body.
        assert_eq!(prompt_hops(&format!("\n{prompt}\n\ngoal: ship")), 3);

        for typed in [
            "hello",
            "typed\nby hand",
            "please read [Message from thread \"x\" (id 1), hop 4/6.",
            "[Message from thread \"x\" (id 1), hop many/6.",
            "[Message from thread \"x\", hop 2/6.",
        ] {
            assert_eq!(parse_delivery(typed), None, "{typed:?}");
            assert_eq!(prompt_hops(typed), 0);
            assert_eq!(delivery_body(typed), typed);
        }
    }

    #[test]
    fn an_awaited_delivery_tells_the_recipient_not_to_message_back() {
        let awaited = delivery_prompt("Sender", "11111111", 1, 6, true, "sum 2 and 2");
        let view = parse_delivery(&awaited).unwrap();
        assert!(view.awaited && view.hops == 1 && view.body == "sum 2 and 2");
        assert!(awaited.contains("hop 1/6, awaiting your reply."));
        assert!(awaited.contains("do not message it back"));
        assert!(!awaited.contains("Answer it with message_thread"));
        assert_eq!(
            view.caption(),
            "From thread “Sender” · waiting for this reply"
        );

        let plain = delivery_prompt("Sender", "11111111", 1, 6, false, "fyi");
        let view = parse_delivery(&plain).unwrap();
        assert!(!view.awaited);
        assert!(plain.contains("Answer it with message_thread"));
        assert_eq!(view.caption(), "From thread “Sender”");
    }

    #[test]
    fn a_plan_names_the_recipient_and_counts_the_next_hop() {
        let ids = [A, B];
        let plan = plan_message(&Outgoing {
            caller_prompt: "user prompt",
            ..outgoing(&ids, Some("22222222"), "hi")
        })
        .unwrap();
        assert_eq!(plan.recipient, Recipient::Existing(B.into()));
        assert_eq!(prompt_hops(&plan.prompt), 1);
        assert!(plan.prompt.contains("\"Sender\" (id 11111111), hop 1/6."));

        let new = plan_message(&outgoing(&ids, None, "Check the logs\nthen report")).unwrap();
        assert_eq!(
            new.recipient,
            Recipient::New {
                title: "Check the logs".into()
            }
        );
        let titled = plan_message(&Outgoing {
            title: Some("Logs"),
            ..outgoing(&ids, None, "x")
        })
        .unwrap();
        assert_eq!(
            titled.recipient,
            Recipient::New {
                title: "Logs".into()
            }
        );
        let awaited = plan_message(&Outgoing {
            wait: true,
            max_hops: 10,
            ..outgoing(&ids, Some(B), "hi")
        })
        .unwrap();
        assert!(awaited.prompt.contains("hop 1/10, awaiting your reply."));
    }

    #[test]
    fn a_plan_refuses_self_unknown_recipients_and_the_users_hop_limit() {
        let ids = [A, B];
        let plan = |prompt: &str, to: &str, max_hops: u32| {
            plan_message(&Outgoing {
                caller_prompt: prompt,
                max_hops,
                ..outgoing(&ids, Some(to), "hi")
            })
        };
        assert!(plan("", A, MAX_HOPS)
            .unwrap_err()
            .text
            .contains("this thread"));
        assert!(plan("", "33333333", MAX_HOPS)
            .unwrap_err()
            .text
            .contains("No thread"));
        let last = delivery_prompt("B", "22222222", MAX_HOPS, MAX_HOPS, false, "ping");
        let refused = plan(&last, B, MAX_HOPS).unwrap_err();
        assert!(!refused.ok && refused.text.contains("limit of 6"));
        let before = delivery_prompt("B", "22222222", MAX_HOPS - 1, MAX_HOPS, false, "ping");
        assert_eq!(
            prompt_hops(&plan(&before, B, MAX_HOPS).unwrap().prompt),
            MAX_HOPS
        );
        // The limit is the user's setting now, not the one printed in an
        // older header.
        assert!(plan(&last, B, 10).is_ok());
        let second = delivery_prompt("B", "22222222", 2, MAX_HOPS, false, "ping");
        assert!(plan(&second, B, 2).unwrap_err().text.contains("limit of 2"));
    }

    #[test]
    fn the_delivered_reply_tells_the_sender_not_to_wait() {
        let ids = [A, B];
        let queued = delivered_reply(B, "Research", &ids, false, DeliveryState::Queued);
        assert!(
            queued.ok
                && queued
                    .text
                    .starts_with("Sent to thread 22222222 \"Research\". It is queued")
        );
        let started = delivered_reply(B, "Research", &ids, true, DeliveryState::Started);
        assert!(started.text.starts_with("Started a new thread 22222222"));
        assert!(started.text.contains("running now") && started.text.contains("do not wait"));
        let not_awaited =
            delivered_reply(B, "Research", &ids, false, DeliveryState::QueuedNotAwaited);
        assert!(not_awaited.ok && not_awaited.text.contains("could not wait for it"));
    }

    #[test]
    fn a_waited_for_answer_is_the_reply_or_the_reason_there_is_none() {
        let ids = [A, B];
        let answered = answer_reply(B, "Research", &ids, Answer::Replied("  42  "));
        assert_eq!(
            answered,
            ThreadReply::ok("Thread 22222222 \"Research\" answered:\n\n42")
        );
        let long = "x".repeat(ANSWER_CHARS + 10);
        let clipped = answer_reply(B, "Research", &ids, Answer::Replied(&long));
        assert!(clipped.ok && clipped.text.chars().count() < ANSWER_CHARS + 100);
        let silent = answer_reply(B, "Research", &ids, Answer::Replied(" "));
        assert!(silent.ok && silent.text.contains("without writing a reply"));
        for (answer, said) in [
            (Answer::Stopped, "stopped by the user"),
            (
                Answer::Failed("rate limited"),
                "could not answer: rate limited",
            ),
            (Answer::Removed, "never ran the message"),
        ] {
            let reply = answer_reply(B, "Research", &ids, answer);
            assert!(!reply.ok && reply.text.contains(said), "{}", reply.text);
        }
        let blocked = recipient_waiting_reply(B, "Research", &ids);
        assert!(!blocked.ok && blocked.text.contains("Send again without wait"));
    }
}

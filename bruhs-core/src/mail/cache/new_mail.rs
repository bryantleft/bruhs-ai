//! New mail for app polls (D20, ruling F28): unread threads that aren't
//! archived, read from the cache in the order they arrived.
//!
//! A thread counts only when its newest message came in: one whose newest
//! message is the owner's own is skipped. The sender, subject and message id
//! come from that newest message, so a reply can bind to it.

use super::{CacheError, CachedThread, MailCache};
use bruhs_models::MessageDirection;
use rusqlite::params;

/// The most of a message body a new-mail preview keeps.
const PREVIEW_CHARS: usize = 300;

/// One unread thread whose newest message came in after a poll's cursor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewMailThread {
    pub external_thread_id: String,
    /// The newest incoming message's id, as a reply names it.
    pub message_id: String,
    /// The sender's display name, or their address when it has none.
    pub sender: String,
    /// The sender's address, which a display name can't fake. Empty when
    /// the message names none.
    pub sender_address: String,
    pub subject: String,
    /// The start of that message's body, at most 300 characters.
    pub preview: String,
    /// The thread's `message_at_ms`, as the cache sorts it.
    pub message_at_ms: i64,
}

impl MailCache {
    /// Up to `limit` unread, unarchived threads of `mailbox` whose
    /// `message_at_ms` is past `after_ms`, oldest first.
    pub(in crate::mail) fn unread_since(
        &self,
        mailbox: &str,
        after_ms: i64,
        limit: usize,
    ) -> Result<Vec<NewMailThread>, CacheError> {
        let mut statement = self.connection.prepare(
            "SELECT external_id,payload,message_at_ms FROM threads WHERE mailbox=?1 AND unread=1 AND archived=0 AND message_at_ms>?2 ORDER BY message_at_ms ASC, id ASC",
        )?;
        let mut rows = statement.query(params![mailbox, after_ms])?;
        let mut threads = Vec::new();
        while threads.len() < limit {
            let Some(row) = rows.next()? else {
                break;
            };
            let (external_thread_id, payload, message_at_ms): (String, String, i64) =
                (row.get(0)?, row.get(1)?, row.get(2)?);
            let record: CachedThread =
                serde_json::from_str(&payload).map_err(|_| CacheError::CorruptRecord)?;
            if let Some(thread) = new_mail(&record, external_thread_id, message_at_ms) {
                threads.push(thread);
            }
        }
        Ok(threads)
    }

    /// The newest `message_at_ms` among `mailbox`'s unread, unarchived
    /// threads: where a new poll starts.
    pub(in crate::mail) fn newest_unread_ms(
        &self,
        mailbox: &str,
    ) -> Result<Option<i64>, CacheError> {
        Ok(self.connection.query_row(
            "SELECT MAX(message_at_ms) FROM threads WHERE mailbox=?1 AND unread=1 AND archived=0",
            [mailbox],
            |row| row.get(0),
        )?)
    }
}

/// The thread as new mail, or `None` when its newest message is the
/// owner's own (or it has none).
fn new_mail(
    record: &CachedThread,
    external_thread_id: String,
    message_at_ms: i64,
) -> Option<NewMailThread> {
    let conversation = &record.conversation;
    let message = conversation.last_message()?;
    if message.direction != MessageDirection::Incoming {
        return None;
    }
    let detail = record.connected_mail.as_ref().and_then(|metadata| {
        metadata
            .messages
            .iter()
            .find(|detail| detail.id == message.id)
    });
    let from = detail.and_then(|detail| detail.envelope.from.first());
    let sender_address = from
        .map(|address| address.address.clone())
        .unwrap_or_default();
    let sender = from.map_or_else(
        || message.sender_name.clone(),
        |address| {
            if address.name.is_empty() {
                address.address.clone()
            } else {
                address.name.clone()
            }
        },
    );
    let subject = detail
        .map(|detail| detail.envelope.subject.clone())
        .filter(|subject| !subject.is_empty())
        .unwrap_or_else(|| conversation.subject.clone());
    Some(NewMailThread {
        external_thread_id,
        message_id: message.id.clone(),
        sender,
        sender_address,
        subject,
        preview: message.body.chars().take(PREVIEW_CHARS).collect(),
        message_at_ms,
    })
}

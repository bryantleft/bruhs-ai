use super::{content, wire, GmailError};
use crate::mail::MailboxId;
use bruhs_models::{
    connections::ServiceProvider,
    mail::{ConnectedMailMetadata, ConnectedMailThread, MailMessageDetail},
    EpochMs, InboxConversation, InboxMessage, InboxParticipant, InboxSource, MessageDirection,
    MessageSendState,
};
use std::collections::{BTreeMap, BTreeSet, HashSet};

pub(super) fn thread(
    mailbox: &MailboxId,
    account: &str,
    thread: wire::Thread,
) -> Result<ConnectedMailThread, GmailError> {
    if mailbox.provider() != ServiceProvider::Gmail {
        return Err(GmailError::WrongProvider);
    }
    if thread.messages.is_empty() {
        return Err(GmailError::InvalidResponse);
    }
    let reference = mailbox
        .thread(&thread.id)
        .map_err(|_| GmailError::InvalidResponse)?;
    let mut ids = HashSet::new();
    let mut messages = Vec::new();
    let mut details = BTreeMap::new();
    let mut participants = BTreeMap::new();
    let mut labels = BTreeSet::new();
    let mut unread = false;
    let mut inbox = false;
    for message in thread.messages {
        if message.thread_id != thread.id || !ids.insert(message.id.clone()) {
            return Err(GmailError::InvalidResponse);
        }
        let id = mailbox
            .message(&message.id)
            .map_err(|_| GmailError::InvalidResponse)?
            .canonical_id;
        let envelope = content::envelope(&message.payload.headers)?;
        for address in envelope
            .from
            .iter()
            .chain(&envelope.to)
            .chain(&envelope.cc)
            .chain(&envelope.bcc)
            .chain(&envelope.reply_to)
        {
            participants
                .entry(address.address.clone())
                .or_insert_with(|| InboxParticipant {
                    id: format!("{}:{}", mailbox.as_str(), address.address),
                    display_name: if address.name.is_empty() {
                        address.address.clone()
                    } else {
                        address.name.clone()
                    },
                    address: address.address.clone(),
                    person_id: None,
                });
        }
        unread |= message.label_ids.iter().any(|label| label == "UNREAD");
        inbox |= message.label_ids.iter().any(|label| label == "INBOX");
        labels.extend(
            message
                .label_ids
                .iter()
                .map(|label| label.to_ascii_lowercase()),
        );
        let outgoing = message
            .label_ids
            .iter()
            .any(|label| label == "SENT" || label == "DRAFT");
        let sent_at_ms = message
            .internal_date
            .parse::<i64>()
            .ok()
            .filter(|ms| EpochMs::new(*ms).is_ok())
            .ok_or(GmailError::InvalidResponse)?;
        let body = content::body(&message.payload, 0)?;
        let attachments = content::attachments(mailbox, &message.id, &message.payload)?;
        let mut attachment_ids = HashSet::new();
        if attachments
            .iter()
            .any(|attachment| !attachment_ids.insert(&attachment.id))
        {
            return Err(GmailError::InvalidResponse);
        }
        let sender_name = envelope.from.first().map_or_else(
            || "Unknown sender".into(),
            |sender| {
                if sender.name.is_empty() {
                    sender.address.clone()
                } else {
                    sender.name.clone()
                }
            },
        );
        messages.push(InboxMessage {
            id: id.clone(),
            external_id: Some(message.id.clone()),
            direction: if outgoing {
                MessageDirection::Outgoing
            } else {
                MessageDirection::Incoming
            },
            sender_name,
            body: body.text,
            sent_at_ms,
            send_state: if message.label_ids.iter().any(|label| label == "DRAFT") {
                MessageSendState::Draft
            } else if outgoing {
                MessageSendState::Sent
            } else {
                MessageSendState::Received
            },
            error: None,
        });
        details.insert(
            id.clone(),
            MailMessageDetail {
                id,
                external_id: message.id,
                label_ids: message.label_ids,
                envelope,
                attachments,
                body_from_html: body.from_html,
            },
        );
    }
    messages.sort_by(|left, right| {
        left.sent_at_ms
            .cmp(&right.sent_at_ms)
            .then_with(|| left.id.cmp(&right.id))
    });
    // The existing Inbox card uses the first participant as its counterparty.
    // Keep the latest incoming sender (or the latest recipient for sent-only
    // threads) ahead of Cc/Bcc recipients when projecting provider records.
    let incoming = messages
        .iter()
        .rev()
        .find(|message| message.direction == MessageDirection::Incoming);
    let primary = incoming
        .or_else(|| messages.last())
        .and_then(|message| details.get(&message.id))
        .and_then(|detail| {
            if incoming.is_some() {
                detail.envelope.from.first()
            } else {
                detail.envelope.to.first()
            }
        });
    let mut ordered_participants = Vec::new();
    if let Some(primary) = primary {
        if let Some(participant) = participants.remove(&primary.address) {
            ordered_participants.push(participant);
        }
    } else if incoming.is_some() {
        ordered_participants.push(InboxParticipant {
            id: format!("{}:unknown-sender", mailbox.as_str()),
            display_name: "Unknown sender".into(),
            address: String::new(),
            person_id: None,
        });
    }
    ordered_participants.extend(participants.into_values());
    let subject = messages
        .first()
        .and_then(|message| details.get(&message.id))
        .map(|message| message.envelope.subject.clone())
        .unwrap_or_default();
    let created_at_ms = messages.first().map_or(0, |message| message.sent_at_ms);
    let updated_at_ms = messages.last().map_or(0, |message| message.sent_at_ms);
    let mut conversation = InboxConversation::new(
        InboxSource::Email,
        subject,
        ordered_participants,
        created_at_ms,
    );
    conversation.id = reference.canonical_id;
    conversation.external_id = Some(thread.id.clone());
    conversation.account_label = account.to_owned();
    conversation.unread = unread;
    conversation.archived = !inbox;
    conversation.labels = labels.into_iter().collect();
    conversation.updated_at_ms = updated_at_ms;
    conversation.messages = messages;
    let messages = conversation
        .messages
        .iter()
        .filter_map(|message| details.remove(&message.id))
        .collect();
    Ok(ConnectedMailThread {
        conversation,
        connected_mail: ConnectedMailMetadata {
            provider: ServiceProvider::Gmail,
            mailbox_id: mailbox.as_str().to_owned(),
            external_thread_id: thread.id,
            messages,
        },
    })
}

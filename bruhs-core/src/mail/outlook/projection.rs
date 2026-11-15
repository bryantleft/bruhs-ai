use super::{wire, MailboxContext, OutlookError, MAX_BYTES};
use crate::mail::MailboxId;
use bruhs_models::{
    connections::ServiceProvider,
    mail::{
        ConnectedMailMetadata, ConnectedMailThread, MailAddress, MailAttachment, MailEnvelope,
        MailMessageDetail,
    },
    EpochMs, InboxConversation, InboxMessage, InboxParticipant, InboxSource, MessageDirection,
    MessageSendState,
};
use mail_parser::{HeaderValue, MessageParser};
use std::collections::{BTreeMap, BTreeSet};

fn addresses(recipients: Vec<wire::Recipient>) -> Vec<MailAddress> {
    recipients
        .into_iter()
        .map(|recipient| MailAddress {
            name: recipient.email_address.name.unwrap_or_default(),
            address: recipient.email_address.address,
        })
        .collect()
}

fn ids(value: &HeaderValue<'_>) -> Vec<String> {
    match value {
        HeaderValue::Text(text) => vec![text.to_string()],
        HeaderValue::TextList(values) => values.iter().map(ToString::to_string).collect(),
        _ => Vec::new(),
    }
}

fn envelope(message: &mut wire::Message) -> Result<MailEnvelope, OutlookError> {
    let mut bytes = Vec::new();
    for header in &message.internet_message_headers {
        if header.name.is_empty()
            || !header
                .name
                .bytes()
                .all(|byte| (33..=126).contains(&byte) && byte != b':')
        {
            return Err(OutlookError::InvalidResponse);
        }
        let value = header.value.as_bytes();
        for (index, byte) in value.iter().enumerate() {
            if (*byte == b'\r' && value.get(index + 1) != Some(&b'\n'))
                || (*byte == b'\n' && !matches!(value.get(index + 1), Some(b' ' | b'\t')))
                || *byte == 0
            {
                return Err(OutlookError::InvalidResponse);
            }
        }
        // Same received-header budget as the existing MIME mail reader.
        if bytes
            .len()
            .saturating_add(header.name.len())
            .saturating_add(value.len())
            .saturating_add(4)
            > 1 << 20
        {
            return Err(OutlookError::TooLarge);
        }
        bytes.extend_from_slice(header.name.as_bytes());
        bytes.extend_from_slice(b": ");
        bytes.extend_from_slice(value);
        bytes.extend_from_slice(b"\r\n");
    }
    bytes.extend_from_slice(b"\r\n");
    let parsed = MessageParser::default().parse_headers(&bytes);
    Ok(MailEnvelope {
        from: addresses(
            message
                .from
                .take()
                .or_else(|| message.sender.take())
                .into_iter()
                .collect(),
        ),
        to: addresses(std::mem::take(&mut message.to_recipients)),
        cc: addresses(std::mem::take(&mut message.cc_recipients)),
        bcc: addresses(std::mem::take(&mut message.bcc_recipients)),
        reply_to: addresses(std::mem::take(&mut message.reply_to)),
        subject: std::mem::take(&mut message.subject),
        message_id: message.internet_message_id.take(),
        in_reply_to: parsed
            .as_ref()
            .map_or_else(Vec::new, |message| ids(message.in_reply_to())),
        references: parsed
            .as_ref()
            .map_or_else(Vec::new, |message| ids(message.references())),
        headers: std::mem::take(&mut message.internet_message_headers),
    })
}

pub(super) fn attachment(
    mailbox: &MailboxId,
    message: &str,
    item: wire::Attachment,
) -> Result<MailAttachment, OutlookError> {
    if item.id.is_empty() {
        return Err(OutlookError::InvalidResponse);
    }
    let downloadable = matches!(
        item.kind.as_str(),
        "#microsoft.graph.fileAttachment" | "#microsoft.graph.itemAttachment"
    );
    Ok(MailAttachment {
        id: mailbox
            .attachment(message, &item.id)
            .map_err(|_| OutlookError::InvalidResponse)?,
        part_id: item.id.clone(),
        external_id: downloadable.then_some(item.id),
        file_name: item.name,
        content_type: item
            .content_type
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| "application/octet-stream".into()),
        size_bytes: item.size,
        content_id: item.content_id,
        inline: item.is_inline,
    })
}

pub(super) fn thread(
    mailbox: &MailboxId,
    context: &MailboxContext,
    external: &str,
    records: Vec<wire::Message>,
) -> Result<ConnectedMailThread, OutlookError> {
    if mailbox.provider() != ServiceProvider::Outlook {
        return Err(OutlookError::WrongProvider);
    }
    if records.is_empty() {
        return Err(OutlookError::NotFound);
    }
    let reference = mailbox
        .thread(external)
        .map_err(|_| OutlookError::InvalidResponse)?;
    let mut messages = Vec::with_capacity(records.len());
    let mut details = BTreeMap::new();
    let mut participants = BTreeMap::new();
    let mut labels = BTreeSet::new();
    let mut unread = false;
    let mut in_inbox = false;
    for mut record in records {
        if record.id.is_empty() || record.conversation_id != external {
            return Err(OutlookError::InvalidResponse);
        }
        let id = mailbox
            .message(&record.id)
            .map_err(|_| OutlookError::InvalidResponse)?
            .canonical_id;
        if details.contains_key(&id) {
            return Err(OutlookError::InvalidResponse);
        }
        let envelope = envelope(&mut record)?;
        let outgoing = record.is_draft
            || record.parent_folder_id == context.sent
            || record.sender.as_ref().is_some_and(|sender| {
                sender
                    .email_address
                    .address
                    .eq_ignore_ascii_case(&context.account)
            })
            || envelope
                .from
                .iter()
                .any(|address| address.address.eq_ignore_ascii_case(&context.account));
        let timestamp = if outgoing {
            record.sent_date_time.as_ref()
        } else {
            record.received_date_time.as_ref()
        }
        .unwrap_or(&record.created_date_time);
        let sent_at_ms = chrono::DateTime::parse_from_rfc3339(timestamp)
            .map_err(|_| OutlookError::InvalidResponse)?
            .timestamp_millis();
        EpochMs::new(sent_at_ms).map_err(|_| OutlookError::InvalidResponse)?;
        let body_from_html = record.body.content_type.eq_ignore_ascii_case("html");
        let body = if body_from_html {
            mail_parser::decoders::html::html_to_text(&record.body.content)
        } else if record.body.content_type.eq_ignore_ascii_case("text") {
            record.body.content
        } else {
            return Err(OutlookError::InvalidResponse);
        };
        let attachments = record
            .attachments
            .into_iter()
            .map(|item| attachment(mailbox, &record.id, item))
            .collect::<Result<Vec<_>, _>>()?;
        let mut attachment_ids = BTreeSet::new();
        if attachments
            .iter()
            .any(|item| !attachment_ids.insert(&item.id))
        {
            return Err(OutlookError::InvalidResponse);
        }
        for address in envelope
            .from
            .iter()
            .chain(&envelope.to)
            .chain(&envelope.cc)
            .chain(&envelope.bcc)
            .chain(&envelope.reply_to)
        {
            if address.address.is_empty() {
                continue;
            }
            participants
                .entry(address.address.to_lowercase())
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
        let mut message_labels = vec![format!("folder:{}", record.parent_folder_id)];
        if record.parent_folder_id == context.inbox {
            in_inbox = true;
            message_labels.push("INBOX".into());
        }
        if !record.is_read {
            unread = true;
            message_labels.push("UNREAD".into());
        }
        if record.is_draft {
            message_labels.push("DRAFT".into());
        } else if outgoing {
            message_labels.push("SENT".into());
        }
        if record.importance == "high" {
            message_labels.push("IMPORTANT".into());
        }
        message_labels.extend(
            record
                .categories
                .into_iter()
                .map(|category| format!("category:{category}")),
        );
        labels.extend(message_labels.iter().map(|label| label.to_lowercase()));
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
            external_id: Some(record.id.clone()),
            direction: if outgoing {
                MessageDirection::Outgoing
            } else {
                MessageDirection::Incoming
            },
            sender_name,
            body,
            sent_at_ms,
            send_state: if record.is_draft {
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
                external_id: record.id,
                label_ids: message_labels,
                envelope,
                attachments,
                body_from_html,
            },
        );
    }
    messages.sort_by(|left, right| {
        left.sent_at_ms
            .cmp(&right.sent_at_ms)
            .then_with(|| left.id.cmp(&right.id))
    });
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
    let mut ordered = Vec::new();
    if let Some(primary) = primary {
        if let Some(participant) = participants.remove(&primary.address.to_lowercase()) {
            ordered.push(participant);
        }
    } else if incoming.is_some() {
        ordered.push(InboxParticipant {
            id: format!("{}:unknown-sender", mailbox.as_str()),
            display_name: "Unknown sender".into(),
            address: String::new(),
            person_id: None,
        });
    }
    ordered.extend(participants.into_values());
    let created = messages.first().map_or(0, |message| message.sent_at_ms);
    let updated = messages.last().map_or(0, |message| message.sent_at_ms);
    let subject = messages
        .first()
        .and_then(|message| details.get(&message.id))
        .map_or_else(String::new, |detail| detail.envelope.subject.clone());
    let mut conversation = InboxConversation::new(InboxSource::Email, subject, ordered, created);
    conversation.id = reference.canonical_id;
    conversation.external_id = Some(external.into());
    conversation.account_label = context.account.clone();
    conversation.unread = unread;
    conversation.archived = !in_inbox;
    conversation.labels = labels.into_iter().collect();
    conversation.updated_at_ms = updated;
    conversation.messages = messages;
    let metadata = conversation
        .messages
        .iter()
        .filter_map(|message| details.remove(&message.id))
        .collect();
    let thread = ConnectedMailThread {
        conversation,
        connected_mail: ConnectedMailMetadata {
            provider: ServiceProvider::Outlook,
            mailbox_id: mailbox.as_str().into(),
            external_thread_id: external.into(),
            messages: metadata,
        },
    };
    if serde_json::to_vec(&thread)
        .map_err(|_| OutlookError::InvalidResponse)?
        .len()
        > MAX_BYTES
    {
        return Err(OutlookError::TooLarge);
    }
    Ok(thread)
}

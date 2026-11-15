use super::{wire, GmailError};
use crate::mail::MailboxId;
use base64::{
    engine::general_purpose::{URL_SAFE, URL_SAFE_NO_PAD},
    Engine as _,
};
use bruhs_models::mail::{MailAddress, MailAttachment, MailEnvelope, MailHeader};
use mail_parser::{Address, HeaderValue, MessageParser, MimeHeaders};

const MAX_MIME_DEPTH: usize = 64;
const MAX_HEADERS_BYTES: usize = 1 << 20;

pub(super) fn header_bytes(headers: &[MailHeader]) -> Result<Vec<u8>, GmailError> {
    let mut bytes = Vec::new();
    for header in headers {
        if header.name.is_empty()
            || !header
                .name
                .bytes()
                .all(|byte| (33..=126).contains(&byte) && byte != b':')
        {
            return Err(GmailError::InvalidResponse);
        }
        // Only RFC header folding may continue a provider header value.
        let value = header.value.as_bytes();
        for (position, byte) in value.iter().enumerate() {
            if (*byte == b'\r' && value.get(position + 1) != Some(&b'\n'))
                || (*byte == b'\n' && !matches!(value.get(position + 1), Some(b' ' | b'\t')))
                || *byte == 0
            {
                return Err(GmailError::InvalidResponse);
            }
        }
        let extra = header
            .name
            .len()
            .checked_add(value.len())
            .and_then(|size| size.checked_add(4))
            .ok_or(GmailError::TooLarge)?;
        if bytes.len().saturating_add(extra) > MAX_HEADERS_BYTES {
            return Err(GmailError::TooLarge);
        }
        bytes.extend_from_slice(header.name.as_bytes());
        bytes.extend_from_slice(b": ");
        bytes.extend_from_slice(value);
        bytes.extend_from_slice(b"\r\n");
    }
    bytes.extend_from_slice(b"\r\n");
    Ok(bytes)
}

fn addresses(value: Option<&Address<'_>>) -> Vec<MailAddress> {
    value
        .into_iter()
        .flat_map(Address::iter)
        .filter_map(|address| {
            Some(MailAddress {
                name: address.name().unwrap_or_default().to_owned(),
                address: address.address()?.to_owned(),
            })
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

pub(super) fn envelope(headers: &[MailHeader]) -> Result<MailEnvelope, GmailError> {
    if headers.is_empty() {
        return Ok(MailEnvelope::default());
    }
    let bytes = header_bytes(headers)?;
    let parsed = MessageParser::default()
        .parse_headers(&bytes)
        .ok_or(GmailError::InvalidResponse)?;
    Ok(MailEnvelope {
        from: addresses(parsed.from()),
        to: addresses(parsed.to()),
        cc: addresses(parsed.cc()),
        bcc: addresses(parsed.bcc()),
        reply_to: addresses(parsed.reply_to()),
        subject: parsed.subject().unwrap_or_default().to_owned(),
        message_id: parsed.message_id().map(str::to_owned),
        in_reply_to: ids(parsed.in_reply_to()),
        references: ids(parsed.references()),
        headers: headers.to_vec(),
    })
}

pub(super) fn is_attachment(part: &wire::Part) -> bool {
    !part.filename.is_empty()
        || part.headers.iter().any(|header| {
            header.name.eq_ignore_ascii_case("content-disposition")
                && header
                    .value
                    .trim_start()
                    .to_ascii_lowercase()
                    .starts_with("attachment")
        })
        || (!part
            .mime_type
            .to_ascii_lowercase()
            .starts_with("multipart/")
            && !part.mime_type.eq_ignore_ascii_case("text/plain")
            && !part.mime_type.eq_ignore_ascii_case("text/html"))
}

pub(super) fn decode(data: &str) -> Result<Vec<u8>, GmailError> {
    URL_SAFE_NO_PAD
        .decode(data)
        .or_else(|_| URL_SAFE.decode(data))
        .map_err(|_| GmailError::InvalidResponse)
}

fn text(part: &wire::Part) -> Result<String, GmailError> {
    let bytes = match part.body.data.as_deref() {
        Some(data) => decode(data)?,
        None if part.body.size == 0 => Vec::new(),
        None => return Err(GmailError::InvalidResponse),
    };
    if u64::try_from(bytes.len()).map_err(|_| GmailError::TooLarge)? != part.body.size {
        return Err(GmailError::InvalidResponse);
    }
    let header = header_bytes(&part.headers)?;
    let parsed = MessageParser::default().parse_headers(&header);
    let charset = parsed
        .as_ref()
        .and_then(|message| message.content_type())
        .and_then(|content| content.attribute("charset"))
        .unwrap_or("utf-8");
    if charset.eq_ignore_ascii_case("utf-8") || charset.eq_ignore_ascii_case("utf8") {
        return String::from_utf8(bytes).map_err(|_| GmailError::InvalidResponse);
    }
    let decoder = mail_parser::decoders::charsets::map::charset_decoder(charset.as_bytes())
        .ok_or(GmailError::UnsupportedEncoding)?;
    Ok(decoder(&bytes))
}

#[derive(Default)]
pub(super) struct BodyText {
    pub text: String,
    pub from_html: bool,
}

pub(super) fn body(part: &wire::Part, depth: usize) -> Result<BodyText, GmailError> {
    if part.mime_type.is_empty() {
        return Err(GmailError::InvalidResponse);
    }
    if depth > MAX_MIME_DEPTH {
        return Err(GmailError::TooLarge);
    }
    if is_attachment(part) {
        return Ok(BodyText::default());
    }
    if part.mime_type.eq_ignore_ascii_case("text/plain") {
        return Ok(BodyText {
            text: text(part)?,
            from_html: false,
        });
    }
    if part.mime_type.eq_ignore_ascii_case("text/html") {
        return Ok(BodyText {
            text: mail_parser::decoders::html::html_to_text(&text(part)?),
            from_html: true,
        });
    }
    let mut bodies = part
        .parts
        .iter()
        .map(|part| body(part, depth + 1))
        .collect::<Result<Vec<_>, _>>()?;
    if part.mime_type.eq_ignore_ascii_case("multipart/alternative") {
        let plain = bodies
            .iter()
            .position(|body| !body.from_html && !body.text.is_empty());
        let fallback = bodies.iter().rposition(|body| !body.text.is_empty());
        return Ok(plain
            .or(fallback)
            .map(|position| bodies.remove(position))
            .unwrap_or_default());
    }
    let from_html = bodies.iter().any(|body| body.from_html);
    Ok(BodyText {
        text: bodies
            .into_iter()
            .map(|body| body.text)
            .filter(|body| !body.is_empty())
            .collect::<Vec<_>>()
            .join("\n\n"),
        from_html,
    })
}

pub(super) fn attachments(
    mailbox: &MailboxId,
    message_id: &str,
    root: &wire::Part,
) -> Result<Vec<MailAttachment>, GmailError> {
    let mut pending = vec![(root, 0)];
    let mut result = Vec::new();
    while let Some((part, depth)) = pending.pop() {
        if part.mime_type.is_empty() {
            return Err(GmailError::InvalidResponse);
        }
        if depth > MAX_MIME_DEPTH {
            return Err(GmailError::TooLarge);
        }
        if is_attachment(part) {
            let content_id = part
                .headers
                .iter()
                .find(|header| header.name.eq_ignore_ascii_case("content-id"))
                .map(|header| header.value.trim().trim_matches(['<', '>']).to_owned());
            let inline = content_id.is_some()
                || part.headers.iter().any(|header| {
                    header.name.eq_ignore_ascii_case("content-disposition")
                        && header
                            .value
                            .trim_start()
                            .to_ascii_lowercase()
                            .starts_with("inline")
                });
            result.push(MailAttachment {
                id: mailbox
                    .attachment(message_id, &part.part_id)
                    .map_err(|_| GmailError::InvalidResponse)?,
                part_id: part.part_id.clone(),
                external_id: part.body.attachment_id.clone(),
                file_name: part.filename.clone(),
                content_type: part.mime_type.clone(),
                size_bytes: part.body.size,
                content_id,
                inline,
            });
        } else {
            pending.extend(part.parts.iter().rev().map(|part| (part, depth + 1)));
        }
    }
    Ok(result)
}

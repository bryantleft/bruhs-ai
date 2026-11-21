//! Owner-only signing keys and verification before any task claims a delivery.
use super::*;
use base64::{engine::general_purpose::STANDARD, Engine as _};
use hmac::{Hmac, Mac};
use sha2::Sha256;
use std::collections::BTreeMap;

pub(in crate::reactive) type Denials = BTreeMap<String, WebhookOutcome>;

pub(super) fn initialize(db: &Connection) -> Result<(), ReactiveError> {
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS webhook_signing_keys(task TEXT PRIMARY KEY, secret TEXT NOT NULL);
         CREATE TABLE IF NOT EXISTS webhook_retry_tasks(original TEXT NOT NULL, task TEXT NOT NULL, accepted TEXT, PRIMARY KEY(original,task));
         CREATE TRIGGER IF NOT EXISTS webhook_security_deleted_task AFTER DELETE ON reactive_subscriptions BEGIN
            DELETE FROM webhook_signing_keys WHERE task=OLD.id;
            DELETE FROM webhook_retry_tasks WHERE task=OLD.id;
         END;
         CREATE TRIGGER IF NOT EXISTS webhook_security_deleted_event AFTER DELETE ON reactive_events BEGIN
            DELETE FROM webhook_retry_tasks WHERE original=OLD.id;
         END;"
    )?;
    Ok(())
}

pub(super) fn replace_key(
    db: &Connection,
    task: &Subscription,
    secret: WebhookSigningSecret,
) -> Result<(), ReactiveError> {
    secret.validate().map_err(ReactiveError::Invalid)?;
    if task.webhook_security.signature.is_none() {
        return Err(ReactiveError::Invalid(
            "Enable signature verification before setting a secret.".into(),
        ));
    }
    db.execute("INSERT INTO webhook_signing_keys(task,secret) VALUES(?1,?2) ON CONFLICT(task) DO UPDATE SET secret=excluded.secret", params![task.id, secret.0])?;
    Ok(())
}

pub(super) fn require_key(db: &Connection, task: &Subscription) -> Result<(), ReactiveError> {
    if task.webhook_security.signature.is_some() {
        let exists: bool = db.query_row(
            "SELECT EXISTS(SELECT 1 FROM webhook_signing_keys WHERE task=?1)",
            [&task.id],
            |r| r.get(0),
        )?;
        if !exists {
            return Err(ReactiveError::Invalid(
                "Enter a signing secret for this webhook.".into(),
            ));
        }
    }
    Ok(())
}

fn verifies(sig: &WebhookSignature, key: &str, request: &WebhookMetadata) -> bool {
    let Some(body) = &request.raw_body else {
        return false;
    };
    let header = sig.header.to_ascii_lowercase();
    let Some([received]) = request.headers.get(&header).map(Vec::as_slice) else {
        return false;
    };
    let received = received.trim();
    if !received
        .get(..sig.prefix.len())
        .is_some_and(|start| start.eq_ignore_ascii_case(&sig.prefix))
    {
        return false;
    }
    let tag = &received[sig.prefix.len()..];
    let mut decoded = [0u8; 32];
    match sig.encoding {
        WebhookSignatureEncoding::Hex => {
            if tag.len() != 64 || !tag.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                return false;
            }
            for (index, value) in decoded.iter_mut().enumerate() {
                let Ok(byte) = u8::from_str_radix(&tag[2 * index..2 * index + 2], 16) else {
                    return false;
                };
                *value = byte;
            }
        }
        WebhookSignatureEncoding::Base64 => {
            if STANDARD.decode_slice(tag, &mut decoded) != Ok(32) {
                return false;
            }
        }
    }
    let Ok(mut mac) = Hmac::<Sha256>::new_from_slice(key.as_bytes()) else {
        return false;
    };
    mac.update(body);
    mac.verify_slice(&decoded).is_ok()
}

pub(in crate::reactive) fn inspect(
    db: &Connection,
    tasks: &[Subscription],
    request: &mut WebhookMetadata,
    received: EpochMs,
    now: EpochMs,
) -> Result<Denials, ReactiveError> {
    let mut denied = Denials::new();
    for task in tasks {
        let policy = &task.webhook_security;
        if !task.enabled && !policy.is_empty() {
            denied.insert(task.id.clone(), WebhookOutcome::Paused);
            continue;
        }
        if let Some(sig) = &policy.signature {
            let secret: Option<String> = db
                .query_row(
                    "SELECT secret FROM webhook_signing_keys WHERE task=?1",
                    [&task.id],
                    |r| r.get(0),
                )
                .optional()?;
            if !secret.is_some_and(|key| verifies(sig, &key, request)) {
                denied.insert(task.id.clone(), WebhookOutcome::RejectedSignature);
                continue;
            }
        }
        if policy.max_delivery_age_minutes.is_some_and(|age| {
            now.as_millis().saturating_sub(received.as_millis()) > i64::from(age) * 60_000
        }) {
            denied.insert(task.id.clone(), WebhookOutcome::Expired);
        }
    }
    // A sibling can use the same header for verification. Remove it only after checking every task.
    for sig in tasks
        .iter()
        .filter_map(|task| task.webhook_security.signature.as_ref())
    {
        request.headers.remove(&sig.header.to_ascii_lowercase());
    }
    Ok(denied)
}

pub(super) fn reason(outcome: WebhookOutcome) -> Option<String> {
    match outcome {
        WebhookOutcome::Expired => Some("The delivery exceeded this task’s maximum age.".into()),
        WebhookOutcome::RejectedSignature => {
            Some("The request signature could not be verified.".into())
        }
        _ => None,
    }
}

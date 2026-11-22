//! Bounded, task-specific delivery history, independent of the global event snapshot.
use super::*;
use std::collections::{BTreeMap, BTreeSet};

const PREVIEW_BYTES: usize = 8192;

pub(super) fn initialize(db: &Connection) -> Result<(), ReactiveError> {
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS webhook_deliveries(
            task TEXT NOT NULL, id TEXT NOT NULL, summary TEXT NOT NULL, detail TEXT NOT NULL,
            PRIMARY KEY(task,id));
         CREATE TABLE IF NOT EXISTS webhook_body_hashes(event TEXT PRIMARY KEY, digest BLOB NOT NULL);
         CREATE TRIGGER IF NOT EXISTS webhook_forget_body_hash AFTER DELETE ON reactive_events BEGIN
            DELETE FROM webhook_body_hashes WHERE event=OLD.id;
         END;
         CREATE INDEX IF NOT EXISTS webhook_deliveries_task ON webhook_deliveries(task);
         CREATE TRIGGER IF NOT EXISTS webhook_deliveries_forget_deleted_task
            AFTER DELETE ON reactive_subscriptions BEGIN
               DELETE FROM webhook_deliveries WHERE task=OLD.id;
            END;",
    )?;
    Ok(())
}

/// JSON retries historically compare parsed values. A form or opaque body
/// can decode to the same value as different wire bytes, so retain its digest.
pub(super) fn body_hash(request: &WebhookMetadata, payload: &Value) -> Option<[u8; 32]> {
    use sha2::{Digest, Sha256};
    let raw = request.raw_body.as_deref()?;
    let opaque = request.is_form()
        || std::str::from_utf8(raw).is_err()
        || payload
            .as_str()
            .is_some_and(|text| request.body_text().as_deref() == Some(text));
    opaque.then(|| Sha256::digest(raw).into())
}

pub(super) fn same_body(
    db: &Connection,
    event: &ReactiveEvent,
    payload: &Value,
    digest: Option<&[u8; 32]>,
) -> Result<bool, ReactiveError> {
    if event.payload != *payload {
        return Ok(false);
    }
    let saved: Option<Vec<u8>> = db
        .query_row(
            "SELECT digest FROM webhook_body_hashes WHERE event=?1",
            [&event.id],
            |row| row.get(0),
        )
        .optional()?;
    Ok(saved.as_deref() == digest.map(|bytes| bytes.as_slice()))
}

fn redact_json(value: &mut Value) {
    match value {
        Value::Object(fields) => {
            for (name, field) in fields {
                if sensitive_webhook_field(name) {
                    *field = Value::String("[redacted]".into());
                } else {
                    redact_json(field);
                }
            }
        }
        Value::Array(items) => items.iter_mut().for_each(redact_json),
        _ => {}
    }
}

/// Check the original JSON tokens, including escaped or overwritten keys, before
/// showing the verbatim request. The parsed object contains only the last value.
fn can_show_json_wire(mut text: &str, token: &str) -> bool {
    while let Some(start) = text.find('"') {
        let mut strings = serde_json::Deserializer::from_str(&text[start..]).into_iter::<String>();
        let Some(Ok(value)) = strings.next() else {
            return false;
        };
        text = &text[start + strings.byte_offset()..];
        if value.contains(token)
            || (text.trim_start().starts_with(':') && sensitive_webhook_field(&value))
        {
            return false;
        }
    }
    true
}

/// Retain wire formatting unless a decoded form field needs redaction. Inspect
/// every occurrence, including keys shadowed by a later occurrence in the task.
fn form_preview(text: &str, token: &str) -> String {
    use url::form_urlencoded::{parse, Serializer};
    let pairs = parse(text.strip_prefix('?').unwrap_or(text).as_bytes());
    if !pairs.clone().any(|(key, value)| {
        sensitive_webhook_field(&key) || key.contains(token) || value.contains(token)
    }) {
        return text.to_owned();
    }
    let mut output = Serializer::new(String::new());
    for (key, value) in pairs {
        let safe = if sensitive_webhook_field(&key) {
            "[redacted]".into()
        } else {
            value.replace(token, "[redacted]")
        };
        output.append_pair(&key.replace(token, "[redacted]"), &safe);
    }
    output.finish()
}

fn preview(value: &str) -> String {
    if value.len() <= PREVIEW_BYTES {
        return value.to_owned();
    }
    let end = value.floor_char_boundary(PREVIEW_BYTES);
    format!("{}…", &value[..end])
}

fn fields(values: &BTreeMap<String, Vec<String>>, token: &str) -> String {
    let lines = values
        .iter()
        .flat_map(|(name, values)| {
            values.iter().map(move |v| {
                let value = if sensitive_webhook_field(name) {
                    "[redacted]"
                } else {
                    v
                };
                format!("{name}: {value}")
            })
        })
        .collect::<Vec<_>>();
    preview(&lines.join("\n").replace(token, "[redacted]"))
}

pub(in crate::reactive) struct Receipt {
    detail: WebhookDelivery,
    compact_body: String,
    safe_body: Value,
    safe_request: WebhookMetadata,
    token: String,
    verified: BTreeSet<String>,
    prepared: Option<RenderedWebhookPrompt>,
}

impl Receipt {
    pub(in crate::reactive) fn new(
        request: &WebhookMetadata,
        payload: &Value,
        token: &str,
        received_at: EpochMs,
        verified: BTreeSet<String>,
    ) -> Result<Self, ReactiveError> {
        let mut safe_body = payload.clone();
        redact_json(&mut safe_body);
        let body = match request.body_text() {
            Some(raw) if request.is_form() => form_preview(&raw, token),
            Some(raw) if payload.as_str() == Some(raw.as_ref()) => raw.replace(token, "[redacted]"),
            Some(raw) if can_show_json_wire(&raw, token) => raw.into_owned(),
            _ => serde_json::to_string_pretty(&safe_body)
                .map_err(|_| ReactiveError::Invalid("Invalid JSON body.".into()))?
                .replace(token, "[redacted]"),
        };
        let safe_fields = |fields: &BTreeMap<String, Vec<String>>| {
            fields
                .iter()
                .map(|(name, values)| {
                    let values = values
                        .iter()
                        .map(|v| {
                            if sensitive_webhook_field(name) {
                                "[redacted]".to_owned()
                            } else {
                                v.replace(token, "[redacted]")
                            }
                        })
                        .collect();
                    (name.clone(), values)
                })
                .collect()
        };
        Ok(Self {
            safe_request: WebhookMetadata {
                headers: safe_fields(&request.headers),
                query: safe_fields(&request.query),
                uri: request
                    .uri
                    .as_ref()
                    .map(|uri| uri.replace(token, "[redacted]")),
                raw_body: Some(body.as_bytes().to_vec()),
            },
            safe_body: safe_body.clone(),
            token: token.to_owned(),
            verified,
            prepared: None,
            compact_body: preview(&encode(&safe_body)?.replace(token, "[redacted]")),
            detail: WebhookDelivery {
                summary: WebhookDeliverySummary {
                    id: uuid::Uuid::new_v4().to_string(),
                    received_at,
                    outcome: WebhookOutcome::Queued,
                    missing_field_count: 0,
                    filter_reason: None,
                },
                headers: fields(&request.headers, token),
                query: fields(&request.query, token),
                body_truncated: body.len() > PREVIEW_BYTES,
                signature_verified: false,
                body: preview(&body),
                prompt: None,
                missing_fields: Vec::new(),
                error: None,
            },
        })
    }

    pub(super) fn id(&self) -> &str {
        &self.detail.summary.id
    }

    pub(super) fn prepare_prompt(
        &mut self,
        template: &str,
        body: &Value,
        request: &WebhookMetadata,
    ) -> Result<String, &'static str> {
        let frame = uuid::Uuid::new_v4();
        let run = render_webhook_prompt(template, body, request, frame)?;
        // Render the preview from redacted fields. Deleting secrets from the final
        // string would miss structured values selected through a wildcard.
        let safe = render_webhook_prompt(template, &self.safe_body, &self.safe_request, frame);
        self.prepared = Some(RenderedWebhookPrompt {
            text: safe.map_or_else(
                |_| "The redacted preview exceeds the display limit.".into(),
                |p| preview(&p.text.replace(&self.token, "[redacted]")),
            ),
            missing: run.missing,
        });
        Ok(run.text.replace(&self.token, "[redacted]"))
    }

    pub(super) fn replace_open_preview(
        &self,
        db: &Connection,
        task: &str,
        first: &str,
    ) -> Result<(), ReactiveError> {
        let summary: Option<String> = db
            .query_row(
                "SELECT summary FROM webhook_deliveries WHERE task=?1 AND id=?2",
                params![task, first],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(summary) = summary {
            let mut latest = self.detail.clone();
            latest.summary = decode(summary)?;
            db.execute(
                "UPDATE webhook_deliveries SET detail=?3 WHERE task=?1 AND id=?2",
                params![task, first, encode(&latest)?],
            )?;
        }
        Ok(())
    }

    pub(super) fn record(
        &mut self,
        db: &Connection,
        task: &Subscription,
        outcome: WebhookOutcome,
        reason: Option<String>,
    ) -> Result<(), ReactiveError> {
        let detail = &mut self.detail;
        detail.summary.outcome = outcome;
        detail.signature_verified = self.verified.contains(&task.id);
        detail.error = reason;
        detail.missing_fields.clear();
        let prepared = self.prepared.take();
        detail.prompt = matches!(
            outcome,
            WebhookOutcome::Queued
                | WebhookOutcome::AwaitingReview
                | WebhookOutcome::Coalesced
                | WebhookOutcome::WindowOpened
        )
        .then(|| {
            if let Some(prepared) = prepared {
                detail.missing_fields = prepared.missing;
                return prepared.text;
            }
            preview(&event_prompt(
                &task.source_id,
                &task.instructions,
                &self.compact_body,
            ))
        });
        db.execute(
            "INSERT INTO webhook_deliveries(task,id,summary,detail) VALUES(?1,?2,?3,?4)",
            params![
                task.id,
                detail.summary.id,
                encode(&detail.summary)?,
                encode(detail)?
            ],
        )?;
        db.execute(
            "DELETE FROM webhook_deliveries WHERE task=?1 AND rowid <= (
            SELECT rowid FROM webhook_deliveries WHERE task=?1 ORDER BY rowid DESC LIMIT 1 OFFSET ?2
        )",
            params![
                task.id,
                i64::try_from(HISTORY_LIMIT).expect("history limit fits SQLite integer")
            ],
        )?;
        Ok(())
    }
}

fn check_task(db: &Connection, task: &str) -> Result<(), ReactiveError> {
    let subscription: Subscription = required(db, "reactive_subscriptions", task, "task")?;
    let source: EventSource = required(db, "reactive_sources", &subscription.source_id, "source")?;
    if source.kind != SourceKind::Webhook {
        return Err(ReactiveError::Invalid(
            "This task uses a time-based or internal trigger.".into(),
        ));
    }
    Ok(())
}

pub(in crate::reactive) fn list(
    db: &Connection,
    task: &str,
) -> Result<Vec<WebhookDeliverySummary>, ReactiveError> {
    check_task(db, task)?;
    let mut stmt =
        db.prepare("SELECT summary, COALESCE(json_array_length(detail,'$.missing_fields'),0),
                    json_extract(detail,'$.error') FROM webhook_deliveries WHERE task=?1 ORDER BY rowid DESC")?;
    let rows = stmt.query_map([task], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, u32>(1)?,
            row.get::<_, Option<String>>(2)?,
        ))
    })?;
    rows.map(|row| {
        let (saved, count, reason) = row?;
        let mut summary: WebhookDeliverySummary = decode(saved)?;
        summary.missing_field_count = count;
        if summary.outcome == WebhookOutcome::Filtered {
            summary.filter_reason = reason;
        }
        Ok(summary)
    })
    .collect()
}

pub(in crate::reactive) fn detail(
    db: &Connection,
    task: &str,
    id: &str,
) -> Result<WebhookDelivery, ReactiveError> {
    check_task(db, task)?;
    let data = db
        .query_row(
            "SELECT detail FROM webhook_deliveries WHERE task=?1 AND id=?2",
            params![task, id],
            |row| row.get(0),
        )
        .optional()?
        .ok_or_else(|| {
            ReactiveError::NotFound("This webhook delivery is no longer available.".into())
        })?;
    let mut delivery: WebhookDelivery = decode(data)?;
    delivery.summary.missing_field_count = u32::try_from(delivery.missing_fields.len())
        .map_err(|_| ReactiveError::Storage("Invalid delivery placeholder count.".into()))?;
    if delivery.summary.outcome == WebhookOutcome::Filtered {
        delivery.summary.filter_reason.clone_from(&delivery.error);
    }
    Ok(delivery)
}

pub(in crate::reactive) fn record_rejected(
    db: &Connection,
    source_id: &str,
    task_id: Option<&str>,
    receipt: &mut Receipt,
    error: &ReactiveError,
    denied: &webhook_security::Denials,
) -> Result<(), ReactiveError> {
    let source: EventSource = required(db, "reactive_sources", source_id, "source")?;
    let outcome = if source.enabled {
        WebhookOutcome::Rejected
    } else {
        WebhookOutcome::Paused
    };
    for task in records::<Subscription>(db, "reactive_subscriptions")?
        .iter()
        .filter(|task| task.source_id == source_id && task_id.is_none_or(|id| task.id == id))
    {
        let outcome = denied.get(&task.id).copied().unwrap_or(outcome);
        receipt.record(
            db,
            task,
            outcome,
            webhook_security::reason(outcome).or_else(|| Some(error.to_string())),
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::can_show_json_wire;

    #[test]
    fn raw_json_previews_check_shadowed_keys_and_escaped_values() {
        let token = "owner-credential";
        assert!(can_show_json_wire(r#"{ "status": "\"done\"", "state": 3 }"#, token));
        assert!(!can_show_json_wire(r#"{"row":{"pass\u0077ord":"private"},"row":{}}"#, token));
        assert!(!can_show_json_wire(r#"{"note":"owner\u002dcredential","note":null}"#, token));
    }
}

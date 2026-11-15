//! Versioned private draft payload. Browser requests contain only editable
//! content; preparation references are written exclusively by the runtime.

use super::ComposeError;
use crate::mail::MailDraftRef;
use bruhs_models::{mail::*, EpochMs};
use serde::{Deserialize, Serialize};

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PreparedReference {
    pub id: String,
    pub source_revision: i64,
    pub connection_id: String,
    pub sha256: String,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DraftPayload {
    version: u8,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    pub content: MailComposeContent,
    pub reply: Option<MailReplyTarget>,
    pub prepared: Option<PreparedReference>,
}

impl DraftPayload {
    pub fn new(
        content: MailComposeContent,
        reply: Option<MailReplyTarget>,
        now: EpochMs,
    ) -> Result<Self, ComposeError> {
        content.validate_storage()?;
        Ok(Self {
            version: 1,
            created_at_ms: now.as_millis(),
            updated_at_ms: now.as_millis(),
            content,
            reply,
            prepared: None,
        })
    }

    pub fn decode(payload: &str, reference: &MailDraftRef) -> Result<Self, ComposeError> {
        // The editable fields have independent bounds; this ceiling also bounds
        // JSON parsing before those checks, including escaped Unicode/control text.
        if payload.len() > 8 << 20 {
            return Err(ComposeError::InvalidDraft);
        }
        let record: Self = serde_json::from_str(payload).map_err(|_| ComposeError::InvalidDraft)?;
        record.content.validate_storage()?;
        if record.version != 1
            || record.created_at_ms < 0
            || record.updated_at_ms < record.created_at_ms
            || record
                .reply
                .as_ref()
                .map(|reply| reply.external_thread_id.as_str())
                != reference
                    .reply_to
                    .as_ref()
                    .map(|thread| thread.external_id.as_str())
            || record.reply.as_ref().is_some_and(|reply| {
                reply.message_id.is_empty()
                    || reply.message_id.len() > 256
                    || !reply.message_id.is_ascii()
                    || reply
                        .message_id
                        .bytes()
                        .any(|byte| !byte.is_ascii_graphic())
            })
            || record.prepared.as_ref().is_some_and(|prepared| {
                prepared.source_revision <= 0
                    || prepared.connection_id.is_empty()
                    || prepared.connection_id.len() > 1024
                    || prepared.connection_id.chars().any(char::is_control)
                    || !digest(&prepared.id)
                    || !digest(&prepared.sha256)
            })
        {
            return Err(ComposeError::InvalidDraft);
        }
        Ok(record)
    }

    pub fn encode(&self) -> Result<String, ComposeError> {
        serde_json::to_string(self).map_err(|_| ComposeError::InvalidDraft)
    }

    pub fn edit(
        &mut self,
        content: MailComposeContent,
        now: EpochMs,
    ) -> Result<bool, ComposeError> {
        content.validate_storage()?;
        if self.content == content {
            return Ok(false);
        }
        self.content = content;
        self.prepared = None;
        self.updated_at_ms = self.updated_at_ms.max(now.as_millis());
        Ok(true)
    }
}

fn digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mail::MailboxId;
    use bruhs_models::connections::ServiceProvider;

    fn reference() -> MailDraftRef {
        MailboxId::from_broker_identity(ServiceProvider::Gmail, &"a".repeat(64), "subject")
            .unwrap()
            .draft("editable", None)
            .unwrap()
    }

    fn prepared() -> PreparedReference {
        PreparedReference {
            id: "b".repeat(64),
            source_revision: 1,
            connection_id: "connection".into(),
            sha256: "c".repeat(64),
        }
    }

    #[test]
    fn incomplete_recipients_survive_restart_and_repeated_save_preserves_review() {
        let content = MailComposeContent {
            to: "Unfinished <someone@".into(),
            subject: "Unfinished subject".into(),
            ..Default::default()
        };
        let mut record =
            DraftPayload::new(content.clone(), None, EpochMs::new(10).unwrap()).unwrap();
        record.prepared = Some(prepared());
        let mut reopened = DraftPayload::decode(&record.encode().unwrap(), &reference()).unwrap();
        assert!(!reopened
            .edit(content.clone(), EpochMs::new(30).unwrap())
            .unwrap());
        assert_eq!(reopened.updated_at_ms, 10);
        assert!(reopened.prepared.is_some());
        assert_eq!(reopened.content, content);
        assert!(reopened.content.validate_send().is_err());
    }

    #[test]
    fn any_actual_edit_invalidates_preparation_and_clock_regression_keeps_update_time() {
        let mut record =
            DraftPayload::new(Default::default(), None, EpochMs::new(20).unwrap()).unwrap();
        record.prepared = Some(prepared());
        let content = MailComposeContent {
            body: "new content".into(),
            ..Default::default()
        };
        assert!(record.edit(content, EpochMs::new(10).unwrap()).unwrap());
        assert!(record.prepared.is_none());
        assert_eq!(record.updated_at_ms, 20);
    }

    #[test]
    fn unsupported_corrupt_or_foreign_reply_payloads_are_retained_as_errors() {
        let record =
            DraftPayload::new(Default::default(), None, EpochMs::new(20).unwrap()).unwrap();
        let mut value = serde_json::to_value(record).unwrap();
        for (field, invalid) in [
            ("version", serde_json::json!(2)),
            ("updated_at_ms", serde_json::json!(1)),
            (
                "prepared",
                serde_json::json!({"id":"bad","source_revision":1,"connection_id":"connection","sha256":"bad"}),
            ),
            (
                "reply",
                serde_json::json!({"external_thread_id":"another-thread","message_id":"message"}),
            ),
        ] {
            let original = value[field].clone();
            value[field] = invalid;
            assert!(matches!(
                DraftPayload::decode(&value.to_string(), &reference()),
                Err(ComposeError::InvalidDraft)
            ));
            value[field] = original;
        }
    }
}

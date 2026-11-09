//! Retry receipts for idempotent speaker linking, never provider generation.

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpeakerLinkJob {
    pub meeting_id: String,
    pub token: String,
    pub attempts: u32,
    pub due_at_ms: i64,
}

impl SpeakerLinkJob {
    pub(super) fn from_capture(
        receipt: &bruhs_models::CaptureRecoveryReceipt,
    ) -> super::MeetingStoreResult<Self> {
        let token = uuid::Uuid::parse_str(&receipt.token).map_err(|_| {
            super::MeetingStoreError::InvalidInput("Invalid capture receipt token.".into())
        })?;
        if receipt.sequence <= 0
            || receipt.meeting_id.is_empty()
            || receipt.meeting_id.len() > 128
            || !receipt
                .meeting_id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        {
            return Err(super::MeetingStoreError::InvalidInput(
                "Invalid capture receipt identity.".into(),
            ));
        }
        Ok(Self {
            meeting_id: receipt.meeting_id.clone(),
            token: format!("capture:{token}"),
            attempts: 0,
            due_at_ms: 0,
        })
    }

    pub(super) fn new(meeting_id: &str) -> Self {
        Self {
            meeting_id: meeting_id.into(),
            token: uuid::Uuid::new_v4().to_string(),
            attempts: 0,
            due_at_ms: 0,
        }
    }

    pub(super) fn retry(&self, now_ms: i64) -> Self {
        let mut next = self.clone();
        next.attempts = next.attempts.saturating_add(1);
        let delay = (1_000_i64 << self.attempts.min(9)).min(300_000);
        next.due_at_ms = now_ms.saturating_add(delay);
        next
    }
}

#[cfg(test)]
mod tests;

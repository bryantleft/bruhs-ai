//! Local capture-to-server recovery protocol. No speech or voice data is retained here.
use serde::{Deserialize, Serialize};

pub const CAPTURE_RECOVERY_PAGE_SIZE: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CaptureRecoveryReceipt {
    pub sequence: i64,
    pub token: String,
    pub meeting_id: String,
}

impl CaptureRecoveryReceipt {
    /// A positive sequence, a hyphenated UUID token and a meeting id that is
    /// a valid [`crate::ids::MeetingId`] (nothing that changes a URL path).
    pub fn is_valid(&self) -> bool {
        self.sequence > 0
            && is_hyphenated_uuid(&self.token)
            && crate::ids::MeetingId::try_from(self.meeting_id.as_str()).is_ok()
    }
}

/// `xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx`, the only form capture writes.
fn is_hyphenated_uuid(token: &str) -> bool {
    token.len() == 36 && uuid::Uuid::try_parse(token).is_ok()
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CaptureRecoveryPage {
    pub receipts: Vec<CaptureRecoveryReceipt>,
}

/// A recording whose Stop or required processing failed. No error payload, paths,
/// speech or participant data is retained. Start time fences different attempts.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CaptureFailureReceipt {
    pub sequence: i64,
    pub token: String,
    pub meeting_id: String,
    pub started_at_ms: f64,
    pub failed_at_ms: f64,
}

impl CaptureFailureReceipt {
    pub fn is_valid(&self) -> bool {
        self.sequence > 0
            && uuid::Uuid::parse_str(&self.token).is_ok()
            && !self.meeting_id.is_empty()
            && self.meeting_id.len() <= 128
            && self
                .meeting_id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
            && self.started_at_ms.is_finite()
            && self.failed_at_ms.is_finite()
            && self.started_at_ms >= 0.0
            && self.failed_at_ms >= self.started_at_ms
            && self.failed_at_ms <= 8_640_000_000_000_000.0
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CaptureFailurePage {
    pub receipts: Vec<CaptureFailureReceipt>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn receipt(sequence: i64, token: &str, meeting_id: &str) -> CaptureRecoveryReceipt {
        CaptureRecoveryReceipt {
            sequence,
            token: token.into(),
            meeting_id: meeting_id.into(),
        }
    }

    #[test]
    fn recovery_receipts_need_a_sequence_uuid_token_and_safe_meeting_id() {
        const TOKEN: &str = "00000000-0000-4000-8000-000000000001";
        assert!(receipt(1, TOKEN, "meeting-1").is_valid());
        for invalid in [
            receipt(0, TOKEN, "meeting-1"),
            receipt(1, TOKEN, "../path"),
            receipt(1, TOKEN, ""),
            receipt(1, &"x".repeat(36), "meeting-1"),
            // Other UUID spellings are not what capture writes.
            receipt(1, "00000000000040008000000000000001", "meeting-1"),
            receipt(1, "{00000000-0000-4000-8000-000000000001}", "meeting-1"),
        ] {
            assert!(!invalid.is_valid(), "{invalid:?}");
        }
    }
}

use super::{person_changes::next_version, MeetingStoreError, MeetingStoreResult};
use bruhs_models::{CaptureFailureReceipt, Meeting, MeetingStatus};

pub(super) fn apply(
    meeting: &mut Meeting,
    receipt: &CaptureFailureReceipt,
) -> MeetingStoreResult<bool> {
    if !receipt.is_valid() || meeting.id != receipt.meeting_id {
        return Err(MeetingStoreError::InvalidInput(
            "Invalid capture failure receipt.".into(),
        ));
    }
    if meeting.started_at_ms != receipt.started_at_ms || meeting.status == MeetingStatus::Failed {
        return Ok(false);
    }
    let version = next_version(meeting.updated_at)?;
    meeting.status = MeetingStatus::Failed;
    meeting.ended_at_ms = meeting.ended_at_ms.or(Some(receipt.failed_at_ms));
    meeting.updated_at = version;
    Ok(true)
}

#[cfg(test)]
mod tests;

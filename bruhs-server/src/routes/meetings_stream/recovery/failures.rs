//! Capture failures: a recording whose stop or processing failed is marked
//! Failed on the server before the daemon forgets it.
use super::pager;
use bruhs_core::{AppState, MeetingStoreError};
use bruhs_models::{CaptureFailureReceipt, MeetingEvent};

#[cfg(test)]
use super::pager::{RecoveryError, MAX_PAGE_BYTES};
#[cfg(test)]
use bruhs_core::capture_client::DaemonEndpoint;

pub(super) struct Failures;

impl pager::Feed for Failures {
    type Receipt = CaptureFailureReceipt;
    const NAME: &'static str = "capture failure recovery";
    const PAGE: &'static [&'static str] = &["recovery", "failures"];
    const ACK: &'static [&'static str] = &["recovery", "failures", "ack"];

    fn sequence(receipt: &CaptureFailureReceipt) -> i64 {
        receipt.sequence
    }

    fn is_valid(receipt: &CaptureFailureReceipt) -> bool {
        receipt.is_valid()
    }

    async fn commit(
        state: &AppState,
        receipt: &CaptureFailureReceipt,
    ) -> Result<bool, MeetingStoreError> {
        state.meetings().apply_capture_failure(receipt).await
    }

    fn announce(state: &AppState, _committed: &[CaptureFailureReceipt], changed: bool) {
        if changed {
            state.emit_meeting_event(MeetingEvent::StreamOpened);
        }
    }
}

#[cfg(test)]
async fn transfer_page(
    state: &AppState,
    client: &reqwest::Client,
    daemon: &DaemonEndpoint,
    after: i64,
) -> Result<i64, RecoveryError> {
    pager::transfer_page::<Failures>(state, client, daemon, after).await
}

#[cfg(test)]
mod tests;

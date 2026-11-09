//! Transfer completion hints into the server's durable link queue before acking.
//! Separate from SSE so retries never pause transcript delivery or require a UI.
use bruhs_core::{capture_client::DaemonEndpoint, AppState, MeetingStoreError};
use bruhs_models::{CaptureRecoveryReceipt, MeetingEvent};
use std::time::Duration;

mod failures;
mod pager;

#[cfg(test)]
use bruhs_models::{CaptureRecoveryPage, CAPTURE_RECOVERY_PAGE_SIZE};
#[cfg(test)]
use pager::{RecoveryError, MAX_PAGE_BYTES};
#[cfg(test)]
use std::sync::Arc;

pub(super) async fn run(state: std::sync::Arc<AppState>, daemon: &DaemonEndpoint) {
    let client = match reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .build()
    {
        Ok(client) => client,
        Err(error) => {
            tracing::warn!("capture recovery client unavailable: {error}");
            // A recovery-client failure must not cancel the independent SSE pump.
            std::future::pending().await
        }
    };
    tokio::join!(
        pager::run::<Completions>(&state, &client, daemon),
        pager::run::<failures::Failures>(&state, &client, daemon),
    );
}

/// Completion hints: a recording finished and its speakers can be linked.
struct Completions;

impl pager::Feed for Completions {
    type Receipt = CaptureRecoveryReceipt;
    const NAME: &'static str = "capture recovery";
    const PAGE: &'static [&'static str] = &["recovery"];
    const ACK: &'static [&'static str] = &["recovery", "ack"];

    fn sequence(receipt: &CaptureRecoveryReceipt) -> i64 {
        receipt.sequence
    }

    fn is_valid(receipt: &CaptureRecoveryReceipt) -> bool {
        receipt.is_valid()
    }

    async fn commit(
        state: &AppState,
        receipt: &CaptureRecoveryReceipt,
    ) -> Result<bool, MeetingStoreError> {
        state.meetings().enqueue_capture_receipt(receipt).await?;
        Ok(true)
    }

    fn announce(state: &AppState, committed: &[CaptureRecoveryReceipt], _changed: bool) {
        for receipt in committed {
            state.emit_meeting_event(MeetingEvent::SpeakersAssigned {
                meeting_id: receipt.meeting_id.clone(),
            });
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
    pager::transfer_page::<Completions>(state, client, daemon, after).await
}

#[cfg(test)]
fn validate_page(page: &CaptureRecoveryPage, after: i64) -> Result<(), RecoveryError> {
    pager::validate::<Completions>(&page.receipts, after)
}

#[cfg(test)]
mod tests;

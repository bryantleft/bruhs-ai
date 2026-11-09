//! One pager for both daemon recovery feeds (completion hints and capture
//! failures): read a bounded page after a cursor, validate all of it,
//! commit each receipt, then acknowledge exactly the committed ones.

use bruhs_core::capture_client::{CaptureClientError, DaemonEndpoint};
use bruhs_core::{AppState, MeetingStoreError};
use bruhs_models::CAPTURE_RECOVERY_PAGE_SIZE;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::future::Future;
use std::time::Duration;
use tokio_stream::StreamExt;

/// Largest page body accepted from the daemon.
pub(super) const MAX_PAGE_BYTES: usize = 32 * 1024;
const RETRY_INITIAL: Duration = Duration::from_secs(5);
const RETRY_MAXIMUM: Duration = Duration::from_secs(60);

/// One daemon feed of receipts.
pub(super) trait Feed {
    type Receipt: Serialize + DeserializeOwned + Send + Sync;
    /// Name used in log lines.
    const NAME: &'static str;
    /// Daemon route of the pages (`?after=<sequence>`).
    const PAGE: &'static [&'static str];
    /// Daemon route that takes the committed receipts back.
    const ACK: &'static [&'static str];

    fn sequence(receipt: &Self::Receipt) -> i64;
    fn is_valid(receipt: &Self::Receipt) -> bool;
    /// Commit one receipt; `Ok(true)` when it changed something clients
    /// should reload.
    fn commit(
        state: &AppState,
        receipt: &Self::Receipt,
    ) -> impl Future<Output = Result<bool, MeetingStoreError>> + Send;
    /// Tell clients about the page's committed receipts.
    fn announce(state: &AppState, committed: &[Self::Receipt], changed: bool);
}

#[derive(Deserialize)]
struct Page<R> {
    receipts: Vec<R>,
}

/// Why a page was not transferred. Nothing was committed or acknowledged
/// for a page that failed before its receipts were applied.
#[derive(Debug, thiserror::Error)]
pub(super) enum RecoveryError {
    #[error("invalid daemon URL")]
    Url(#[source] CaptureClientError),
    #[error("daemon unavailable")]
    Unavailable(#[source] reqwest::Error),
    #[error("daemon refused the page request")]
    Refused(#[source] reqwest::Error),
    #[error("page response interrupted")]
    Interrupted(#[source] reqwest::Error),
    #[error("page response exceeded {MAX_PAGE_BYTES} bytes")]
    TooLarge,
    #[error("invalid page response")]
    Malformed(#[source] serde_json::Error),
    #[error("page has {0} receipts, more than {CAPTURE_RECOVERY_PAGE_SIZE}")]
    TooManyReceipts(usize),
    #[error("invalid receipt at sequence {0}")]
    InvalidReceipt(i64),
}

/// Transfer pages forever, backing off while the daemon is unavailable.
pub(super) async fn run<F: Feed>(
    state: &AppState,
    client: &reqwest::Client,
    daemon: &DaemonEndpoint,
) {
    let mut after = 0;
    let mut delay = RETRY_INITIAL;
    loop {
        match transfer_page::<F>(state, client, daemon, after).await {
            Ok(next) => {
                after = next;
                delay = RETRY_INITIAL;
            }
            Err(error) => {
                tracing::warn!("{}: {error}{}", F::NAME, source_suffix(&error));
                delay = delay.saturating_mul(2).min(RETRY_MAXIMUM);
            }
        }
        // Pages advance even past unknown/deleted meetings. Retain those
        // receipts for late synchronization; wrap periodically without busy
        // retrying.
        tokio::time::sleep(delay).await;
    }
}

/// `: <cause>` of `error`, or nothing.
fn source_suffix(error: &RecoveryError) -> String {
    std::error::Error::source(error).map_or_else(String::new, |source| format!(": {source}"))
}

/// Transfer the page after `after`; returns the cursor for the next page
/// (0 to start over after the last page).
pub(super) async fn transfer_page<F: Feed>(
    state: &AppState,
    client: &reqwest::Client,
    daemon: &DaemonEndpoint,
    after: i64,
) -> Result<i64, RecoveryError> {
    let url = daemon.url(F::PAGE).map_err(RecoveryError::Url)?;
    let response = daemon
        .authorize(client.get(url).query(&[("after", after)]))
        .await
        .send()
        .await
        .map_err(RecoveryError::Unavailable)?
        .error_for_status()
        .map_err(RecoveryError::Refused)?;
    let mut stream = response.bytes_stream();
    let mut body = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(RecoveryError::Interrupted)?;
        if body.len().saturating_add(chunk.len()) > MAX_PAGE_BYTES {
            return Err(RecoveryError::TooLarge);
        }
        body.extend_from_slice(&chunk);
    }
    let page: Page<F::Receipt> = serde_json::from_slice(&body).map_err(RecoveryError::Malformed)?;
    validate::<F>(&page.receipts, after)?;
    let next = page.receipts.last().map_or(0, F::sequence);
    let mut committed = Vec::with_capacity(page.receipts.len());
    let mut changed = false;
    for receipt in page.receipts {
        // Sequential commits deliberately bound disk work. A failure
        // (including an unknown meeting) keeps the receipt at the daemon.
        match F::commit(state, &receipt).await {
            Ok(applied) => {
                changed |= applied;
                committed.push(receipt);
            }
            Err(MeetingStoreError::MeetingNotFound { .. }) => {}
            Err(error) => tracing::warn!(
                "{}: receipt {} could not be saved and is retained: {error}",
                F::NAME,
                F::sequence(&receipt)
            ),
        }
    }
    F::announce(state, &committed, changed);
    if !committed.is_empty() {
        acknowledge::<F>(client, daemon, &committed).await;
    }
    Ok(next)
}

/// The whole page or nothing: bounded, strictly increasing after `after`,
/// every receipt valid.
pub(super) fn validate<F: Feed>(receipts: &[F::Receipt], after: i64) -> Result<(), RecoveryError> {
    if receipts.len() > CAPTURE_RECOVERY_PAGE_SIZE {
        return Err(RecoveryError::TooManyReceipts(receipts.len()));
    }
    let mut previous = after;
    for receipt in receipts {
        let sequence = F::sequence(receipt);
        if sequence <= previous || !F::is_valid(receipt) {
            return Err(RecoveryError::InvalidReceipt(sequence));
        }
        previous = sequence;
    }
    Ok(())
}

/// Hand the committed receipts back. A failure keeps them at the daemon, so
/// the next pass commits them again (an idempotent no-op) and retries.
async fn acknowledge<F: Feed>(
    client: &reqwest::Client,
    daemon: &DaemonEndpoint,
    committed: &[F::Receipt],
) {
    let url = match daemon.url(F::ACK) {
        Ok(url) => url,
        Err(error) => {
            tracing::warn!("{}: acknowledgment URL invalid: {error}", F::NAME);
            return;
        }
    };
    let result = daemon
        .authorize(client.post(url).json(committed))
        .await
        .send()
        .await
        .and_then(reqwest::Response::error_for_status);
    if let Err(error) = result {
        tracing::warn!(
            "{}: acknowledgment failed; receipts retained: {error}",
            F::NAME
        );
    }
}

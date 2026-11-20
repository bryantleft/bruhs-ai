//! App poll sources (D20) on the source loop: each poll goes through the
//! attached `AppsHost`, and its failures become the source's status with no
//! checkpoint saved.

use super::{PollBatch, ReactiveError, ReactiveRuntime, SourceEmission};
use bruhs_models::apps::{AppQuery, APPS_NOT_ON_SERVER};
use serde_json::{json, Value};

/// One app poll's batch, and the status to show once it is saved (a mail
/// sync that failed while the cache still answered).
pub(super) async fn batch(
    runtime: &ReactiveRuntime,
    app: &str,
    account: &str,
    query: &AppQuery,
    cursor: Option<Value>,
    limit: usize,
) -> Result<(PollBatch, Option<String>), ReactiveError> {
    // No room in the run queue: read nothing and keep the cursor, as a
    // folder scan does (final review A2 I2). A first poll still sets its
    // baseline, which emits nothing.
    if limit == 0 {
        if let Some(cursor) = cursor {
            let batch = PollBatch {
                events: Vec::new(),
                cursor,
                backpressured: true,
            };
            return Ok((batch, None));
        }
    }
    let apps = runtime
        .apps()
        .ok_or_else(|| ReactiveError::Invalid(APPS_NOT_ON_SERVER.into()))?;
    // What the cursor belongs to: changing any of it starts a new baseline.
    let definition = json!({ "app": app, "account": account, "query": query }).to_string();
    let outcome = apps
        .poll(app, account, query, cursor, &definition, limit)
        .await
        .map_err(|error| ReactiveError::Invalid(error.to_string()))?;
    let events = outcome
        .events
        .into_iter()
        .map(|(key, payload)| SourceEmission { key, payload })
        .collect();
    Ok((
        PollBatch {
            events,
            cursor: outcome.cursor,
            backpressured: false,
        },
        outcome.status,
    ))
}

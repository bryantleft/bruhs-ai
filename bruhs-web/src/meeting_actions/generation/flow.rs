//! Provider effects are sequenced behind acknowledged local intent and cache writes.
use super::*;
use bruhs_models::{MeetingSummary, SummaryGenerationReceipt};

pub(super) trait Transport {
    async fn reserve(&self, checkpoint: &Checkpoint) -> Result<generations::Reservation, String>;
    fn current_matches(&self, checkpoint: &Checkpoint) -> bool;
    async fn send(&self, checkpoint: &Checkpoint) -> Result<MeetingSummary, String>;
    async fn receipt(
        &self,
        checkpoint: &Checkpoint,
    ) -> Result<Option<SummaryGenerationReceipt>, String>;
    async fn cache(
        &self,
        checkpoint: &Checkpoint,
        expected: Option<MeetingSummary>,
    ) -> Result<(), String>;
    async fn clear(&self, checkpoint: &Checkpoint) -> Result<(), String>;
    /// Fence the request on the server. `None` means the meeting is unknown there.
    async fn cancel(
        &self,
        checkpoint: &Checkpoint,
    ) -> Result<Option<SummaryGenerationReceipt>, String>;
}

#[derive(Debug, PartialEq)]
pub(super) enum Cancelled {
    /// The server refused the request; nothing was generated for it.
    Fenced,
    /// The request had already ended; the latest meeting is cached instead.
    AlreadyFinished,
}

pub(super) struct Failure {
    pub message: String,
    pub checkpoint: Option<Checkpoint>,
}

#[derive(Debug, PartialEq)]
pub(super) enum Generated {
    Saved,
    NotSent,
}

pub(super) async fn generate(
    io: &impl Transport,
    checkpoint: &Checkpoint,
) -> Result<Generated, Failure> {
    checkpoint
        .validate(&checkpoint.id)
        .map_err(|error| Failure {
            message: error.to_string(),
            checkpoint: None,
        })?;
    match io.reserve(checkpoint).await {
        Ok(generations::Reservation::Started) => {},
        Ok(generations::Reservation::Existing(saved)) => return Err(Failure {
            message: "Another saved generation request must be resolved first. Load the latest; no new generation was sent.".into(), checkpoint: Some(saved),
        }),
        Err(error) => return Err(Failure { message: format!("Request not sent: its local intent could not be saved. {error}"), checkpoint: None }),
    }
    let failure = |message| Failure {
        message,
        checkpoint: Some(checkpoint.clone()),
    };
    if !io.current_matches(checkpoint) {
        io.clear(checkpoint).await.map_err(|error| {
            failure(format!(
                "Request not sent, but its saved intent could not be cleared. {error}"
            ))
        })?;
        return Ok(Generated::NotSent);
    }
    let summary = io.send(checkpoint).await.map_err(|error| failure(format!("Summary not confirmed. {error} Load the latest to check this request; it will not be sent again.")))?;
    io.cache(checkpoint, Some(summary)).await.map_err(failure)?;
    io.clear(checkpoint).await.map_err(|error| failure(format!("The meeting is cached, but its request could not be marked complete on this device. {error}")))?;
    Ok(Generated::Saved)
}

pub(super) async fn recover(io: &impl Transport, checkpoint: &Checkpoint) -> Result<(), String> {
    checkpoint
        .validate(&checkpoint.id)
        .map_err(|error| error.to_string())?;
    let receipt = io.receipt(checkpoint).await
        .map_err(|error| format!("Generation status could not be confirmed. {error} The saved request is retained."))?
        .ok_or_else(|| "Generation receipt is missing. A delayed request may still arrive; the saved request is retained.".to_string())?;
    settle_terminal(io, checkpoint, &receipt).await
}

/// Explicit cancellation. A cancelled receipt only clears the local intent;
/// a request that had already finished is recovered like `recover`.
pub(super) async fn cancel(
    io: &impl Transport,
    checkpoint: &Checkpoint,
) -> Result<Cancelled, String> {
    checkpoint
        .validate(&checkpoint.id)
        .map_err(|error| error.to_string())?;
    let receipt = io.cancel(checkpoint).await
        .map_err(|error| format!("Cancellation could not be confirmed. {error} The saved request is retained; no generation will run."))?
        .ok_or_else(|| "The meeting is unknown on the server, so its request could not be fenced. The saved request is retained.".to_string())?;
    checkpoint
        .confirm_terminal(&receipt)
        .map_err(|error| error.to_string())?;
    if receipt.state == bruhs_models::SummaryGenerationState::Cancelled {
        io.clear(checkpoint).await.map_err(|error| {
            format!("The server refused the request, but that could not be saved on this device. {error}")
        })?;
        return Ok(Cancelled::Fenced);
    }
    settle_terminal(io, checkpoint, &receipt).await?;
    Ok(Cancelled::AlreadyFinished)
}

async fn settle_terminal(
    io: &impl Transport,
    checkpoint: &Checkpoint,
    receipt: &SummaryGenerationReceipt,
) -> Result<(), String> {
    checkpoint
        .confirm_terminal(receipt)
        .map_err(|error| error.to_string())?;
    io.cache(checkpoint, None).await?;
    io.clear(checkpoint).await.map_err(|error| {
        format!("The latest meeting is cached, but request recovery could not be saved. {error}")
    })
}

#[cfg(test)]
mod tests;

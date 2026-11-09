use super::*;
mod flow;
mod transport;
use transport::WebTransport;

type Outcome = Result<String, (String, Action)>;

impl MeetingActions {
    pub(super) fn restore_generations(self) {
        self.restore.set(Restore::Loading);
        wasm_bindgen_futures::spawn_local(async move {
            match generations::load_all().await {
                Ok(checkpoints) => {
                    self.entries.update(|entries| {
                        for checkpoint in checkpoints {
                            entries.insert(checkpoint.id.clone(), Entry {
                                status: Status { busy: false, recovery: true, cancellable: true, foreign_pending: false,
                                    message: Some("A saved generation may have finished. Load the latest to check its receipt, or cancel it; generation will not restart automatically.".into()) },
                                retry: Some(Action::RecoverGeneration(Some(checkpoint))),
                                foreign: None,
                            });
                        }
                    });
                    self.restore.set(Restore::Ready);
                }
                Err(error) => self.restore.set(Restore::Failed(error.to_string())),
            }
        });
    }

    pub(super) async fn generate_saved(
        self,
        id: &str,
        baseline: Option<Meeting>,
        extract: bool,
        replace: bool,
    ) -> Outcome {
        let meeting = baseline.ok_or_else(|| {
            (
                "Load this meeting before generating a summary.".into(),
                Action::RecoverGeneration(None),
            )
        })?;
        if replace
            && (meeting.summary.is_none()
                || !extract
                || meeting.skip_observations
                || !self.personality.get_untracked().extract_observations)
        {
            return Err((
                "Observation replacement requires an existing summary and enabled extraction."
                    .into(),
                Action::RecoverGeneration(None),
            ));
        }
        let extract = extract
            && !meeting.skip_observations
            && self.personality.get_untracked().extract_observations;
        let checkpoint = Checkpoint {
            id: id.into(),
            minimum_version: meeting.updated_at,
            request: bruhs_models::SummaryGenerationRequest::new(
                id.into(),
                extract,
                replace && extract,
            ),
        };
        let io = WebTransport {
            actions: self,
            baseline: Some(meeting),
        };
        let result = flow::generate(&io, &checkpoint).await.map_err(|failure| {
            (
                failure.message,
                Action::RecoverGeneration(failure.checkpoint),
            )
        })?;
        if result == flow::Generated::NotSent {
            return Ok("Request not sent: the meeting or observation settings changed while saving. Review them before trying again.".into());
        }
        self.refresh_generation_contacts();
        Ok("Summary saved on this device. Contact updates sync separately.".into())
    }

    pub(super) async fn recover_generation(
        self,
        id: &str,
        remembered: Option<Checkpoint>,
    ) -> Outcome {
        let saved = generations::load(id).await.map_err(|error| {
            (
                error.to_string(),
                Action::RecoverGeneration(remembered.clone()),
            )
        })?;
        // Prefer the current persisted intent if another tab has advanced the request.
        let Some(checkpoint) = saved.or(remembered) else {
            return Ok("No saved generation request remains on this device. Review the meeting before starting a new request.".into());
        };
        let retry = || Action::RecoverGeneration(Some(checkpoint.clone()));
        checkpoint
            .validate(id)
            .map_err(|error| (error.to_string(), retry()))?;
        let io = WebTransport {
            actions: self,
            baseline: None,
        };
        flow::recover(&io, &checkpoint)
            .await
            .map_err(|error| (error, retry()))?;
        self.refresh_generation_contacts();
        Ok("Generation receipt checked and latest meeting saved on this device. Contact updates sync separately.".into())
    }

    pub(super) async fn cancel_saved_generation(
        self,
        id: &str,
        remembered: Option<Checkpoint>,
    ) -> Outcome {
        let saved = generations::load(id).await.map_err(|error| {
            (
                error.to_string(),
                Action::RecoverGeneration(remembered.clone()),
            )
        })?;
        let Some(checkpoint) = saved.or(remembered) else {
            return Ok("No saved generation request remains on this device. Review the meeting before starting a new request.".into());
        };
        // A failed cancellation leaves Load latest as the read-only recovery path.
        let retry = || Action::RecoverGeneration(Some(checkpoint.clone()));
        checkpoint
            .validate(id)
            .map_err(|error| (error.to_string(), retry()))?;
        let io = WebTransport {
            actions: self,
            baseline: None,
        };
        match flow::cancel(&io, &checkpoint)
            .await
            .map_err(|error| (error, retry()))?
        {
            flow::Cancelled::Fenced => Ok("The saved generation request is cancelled on the server; no result will be saved for it. You can generate again.".into()),
            flow::Cancelled::AlreadyFinished => {
                self.refresh_generation_contacts();
                Ok("That generation had already ended, so nothing was cancelled. The latest meeting is saved on this device; contact updates sync separately.".into())
            }
        }
    }

    pub(super) async fn discover_pending_generation(self, id: &str) -> Outcome {
        let saved = generations::load(id)
            .await
            .map_err(|error| (error.to_string(), Action::DiscoverPending))?;
        let receipt = deadline(api::pending_summary_generation(id), 15_000)
            .await
            .map_err(|error| {
                (
                    format!("{error} Nothing was changed."),
                    Action::DiscoverPending,
                )
            })?;
        let outcome = classify_pending(
            id,
            receipt,
            saved
                .as_ref()
                .map(|checkpoint| checkpoint.request.request_id.as_str()),
        );
        let foreign = match &outcome {
            Discovery::Foreign(request) => Some(request.clone()),
            _ => None,
        };
        self.entries
            .update(|all| all.entry(id.to_string()).or_default().foreign = foreign);
        Ok(outcome.message().into())
    }

    pub(super) async fn cancel_foreign_generation(
        self,
        id: &str,
        request: bruhs_models::SummaryGenerationRequest,
        minimum_version: i64,
    ) -> Outcome {
        let load_latest = || Action::Readback {
            expected: None,
            minimum_version,
        };
        let receipt = deadline(
            async {
                api::cancel_summary_generation(&request)
                    .await
                    .map_err(|error| api::summary_error_message(&error))
            },
            15_000,
        )
            .await
            .map_err(|error| (format!("Cancellation could not be confirmed. {error} The unresolved request may still be blocking generation."), load_latest()))?;
        let outcome = foreign_cancel_outcome(&request, receipt.as_ref())
            .map_err(|error| (error.into(), load_latest()))?;
        self.entries
            .update(|all| all.entry(id.to_string()).or_default().foreign = None);
        if outcome == ForeignCancel::AlreadyFinished {
            self.readback(id, None, minimum_version).await?;
            self.refresh_generation_contacts();
            return Ok("That request had already ended, so nothing was cancelled. The latest meeting is saved on this device.".into());
        }
        Ok("The unresolved server request is cancelled; no result will be saved for it. You can generate again.".into())
    }

    fn refresh_generation_contacts(self) {
        wasm_bindgen_futures::spawn_local(async move {
            self.contacts.refresh_people().await;
        });
    }
}

#[derive(Debug, PartialEq)]
pub(super) enum Discovery {
    None,
    Own,
    Foreign(bruhs_models::SummaryGenerationRequest),
    Invalid,
}

impl Discovery {
    pub(super) fn message(&self) -> &'static str {
        match self {
            Self::None => "No unresolved generation request exists on the server for this meeting.",
            Self::Own => "The unresolved server request is this device's saved request. Use Load latest meeting or Cancel saved generation.",
            Self::Foreign(_) => "An unresolved generation request from another client is blocking this meeting. Cancel it to allow a new generation.",
            Self::Invalid => "The server reported an unresolved request for a different meeting or with an invalid identity. Nothing was changed.",
        }
    }
}

/// A discovered receipt is only actionable when it is pending, valid and for this meeting.
pub(super) fn classify_pending(
    id: &str,
    receipt: Option<bruhs_models::SummaryGenerationReceipt>,
    own_request_id: Option<&str>,
) -> Discovery {
    let Some(receipt) = receipt else {
        return Discovery::None;
    };
    if receipt.request.meeting_id != id
        || !receipt.request.valid()
        || receipt.state != bruhs_models::SummaryGenerationState::Pending
    {
        return Discovery::Invalid;
    }
    if own_request_id == Some(receipt.request.request_id.as_str()) {
        return Discovery::Own;
    }
    Discovery::Foreign(receipt.request)
}

#[derive(Debug, PartialEq)]
pub(super) enum ForeignCancel {
    Fenced,
    AlreadyFinished,
}

pub(super) fn foreign_cancel_outcome(
    request: &bruhs_models::SummaryGenerationRequest,
    receipt: Option<&bruhs_models::SummaryGenerationReceipt>,
) -> Result<ForeignCancel, &'static str> {
    let receipt = receipt.ok_or(
        "The meeting is unknown on the server, so its unresolved request could not be fenced.",
    )?;
    if receipt.request != *request {
        return Err("The server returned a mismatched receipt; the unresolved request was not confirmed cancelled.");
    }
    match receipt.state {
        bruhs_models::SummaryGenerationState::Pending => Err("The server did not cancel the unresolved request; it remains pending. Try again later."),
        bruhs_models::SummaryGenerationState::Cancelled => Ok(ForeignCancel::Fenced),
        _ => Ok(ForeignCancel::AlreadyFinished),
    }
}

#[cfg(test)]
mod discovery_tests {
    use super::*;
    use bruhs_models::{
        SummaryGenerationReceipt, SummaryGenerationRequest, SummaryGenerationState,
    };

    fn receipt(state: SummaryGenerationState) -> SummaryGenerationReceipt {
        SummaryGenerationReceipt {
            request: SummaryGenerationRequest::new("owned-meeting".into(), true, false),
            state,
        }
    }

    #[test]
    fn discovery_only_acts_on_pending_valid_requests_for_this_meeting() {
        assert_eq!(
            classify_pending("owned-meeting", None, None),
            Discovery::None
        );
        let pending = receipt(SummaryGenerationState::Pending);
        let own = pending.request.request_id.clone();
        assert_eq!(
            classify_pending("owned-meeting", Some(pending.clone()), Some(&own)),
            Discovery::Own
        );
        assert_eq!(
            classify_pending("owned-meeting", Some(pending.clone()), Some("other")),
            Discovery::Foreign(pending.request.clone())
        );
        assert_eq!(
            classify_pending("other-meeting", Some(pending.clone()), None),
            Discovery::Invalid
        );
        assert_eq!(
            classify_pending(
                "owned-meeting",
                Some(receipt(SummaryGenerationState::Failed)),
                None
            ),
            Discovery::Invalid
        );
        let mut invalid = pending;
        invalid.request.request_id = " ".into();
        assert_eq!(
            classify_pending("owned-meeting", Some(invalid), None),
            Discovery::Invalid
        );
    }

    #[test]
    fn foreign_cancellation_requires_exact_terminal_receipt() {
        let pending = receipt(SummaryGenerationState::Pending);
        let request = pending.request.clone();
        assert!(foreign_cancel_outcome(&request, None).is_err());
        assert!(foreign_cancel_outcome(&request, Some(&pending)).is_err());
        let mut foreign = pending.clone();
        foreign.request.request_id = "foreign".into();
        foreign.state = SummaryGenerationState::Cancelled;
        assert!(foreign_cancel_outcome(&request, Some(&foreign)).is_err());
        let mut cancelled = pending.clone();
        cancelled.state = SummaryGenerationState::Cancelled;
        assert_eq!(
            foreign_cancel_outcome(&request, Some(&cancelled)),
            Ok(ForeignCancel::Fenced)
        );
        let mut failed = pending;
        failed.state = SummaryGenerationState::Failed;
        assert_eq!(
            foreign_cancel_outcome(&request, Some(&failed)),
            Ok(ForeignCancel::AlreadyFinished)
        );
    }
}

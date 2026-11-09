use super::*;
mod state;
#[cfg(test)]
pub(crate) mod test_support;
#[cfg(test)]
mod tests;
mod transport;
pub(super) use state::Entry;
use state::{Action, Checkpoint, Expected, Phase};
pub use state::{ActionCheckpoints, MeetingActionStatus};

impl MeetingsHub {
    pub fn summarize(
        &mut self,
        id: String,
        extract: bool,
        replace: bool,
        cx: &mut Context<'_, Self>,
    ) {
        if self.meeting_action_blocked(&id) {
            return;
        }
        let Some(meeting) = self.meeting(&id) else {
            self.notify_notice("Load the meeting before generating a summary.", true, cx);
            return;
        };
        if replace && (meeting.summary.is_none() || meeting.skip_observations || !extract) {
            self.notify_notice(
                "Observation extraction requires an existing summary and enabled extraction.",
                true,
                cx,
            );
            return;
        }
        let extract = extract && !meeting.skip_observations;
        self.queue_meeting_action(
            id,
            Action::Generate {
                extract,
                replace: replace && extract,
            },
            cx,
        );
    }

    pub fn set_skip_observations(&mut self, id: String, skip: bool, cx: &mut Context<'_, Self>) {
        self.queue_meeting_action(id, Action::SetSkip(skip), cx);
    }

    fn queue_meeting_action(&mut self, id: String, action: Action, cx: &mut Context<'_, Self>) {
        if self.meeting_action_blocked(&id) {
            return;
        }
        let mut matching = self.meetings.iter().filter(|meeting| meeting.id == id);
        let Some(meeting) = matching.next() else {
            self.notify_notice("Load the meeting before changing it.", true, cx);
            return;
        };
        if matching.next().is_some() || meeting_cache::validate_meeting(meeting).is_err() {
            self.notify_notice(
                "Meeting identity is invalid or ambiguous. Refresh before changing it.",
                true,
                cx,
            );
            return;
        }
        let token = uuid::Uuid::new_v4().to_string();
        let generation = match action {
            Action::Generate { extract, replace } => Some(bruhs_models::SummaryGenerationRequest {
                request_id: token.clone(),
                meeting_id: id.clone(),
                extract_observations: extract,
                replace_observations: replace,
            }),
            _ => None,
        };
        let checkpoint = Checkpoint {
            token,
            minimum_version: meeting.updated_at,
            expected: None,
            generation,
        };
        self.meeting_actions.insert(
            id,
            Entry {
                checkpoint,
                phase: Phase::Queued(action),
                message: "Saving request intent before contacting the server…".into(),
                foreign: None,
            },
        );
        cx.emit(HubEvent::MeetingsChanged);
        cx.notify();
    }

    pub fn retry_meeting_action(&mut self, id: String, cx: &mut Context<'_, Self>) {
        if self
            .meeting_actions
            .get(&id)
            .is_some_and(|entry| entry.phase == Phase::Recovery)
        {
            self.launch_meeting_action(id, Action::LoadLatest, cx);
        }
    }

    /// Read-only discovery of an unresolved server request without a saved identity.
    pub fn discover_pending_generation(&mut self, id: String, cx: &mut Context<'_, Self>) {
        if self.meeting_action_blocked(&id) {
            return;
        }
        let Some(minimum_version) = self.meeting(&id).map(|meeting| meeting.updated_at) else {
            self.notify_notice(
                "Load the meeting before checking server requests.",
                true,
                cx,
            );
            return;
        };
        let entry = self
            .meeting_actions
            .entry(id.clone())
            .or_insert_with(|| Entry {
                checkpoint: Checkpoint {
                    token: uuid::Uuid::new_v4().to_string(),
                    minimum_version,
                    expected: None,
                    generation: None,
                },
                phase: Phase::Complete,
                message: String::new(),
                foreign: None,
            });
        entry.phase = Phase::Reading;
        entry.message = "Checking the server for unresolved generation requests…".into();
        let token = entry.checkpoint.token.clone();
        let client = self.client.clone();
        let requested = id.clone();
        self.run(
            async move { client.pending_summary_generation(&requested).await },
            move |hub, result, cx| {
                let Some(entry) = hub.meeting_actions.get_mut(&id) else {
                    return;
                };
                if entry.checkpoint.token != token || entry.phase != Phase::Reading {
                    return;
                }
                entry.phase = Phase::Complete;
                let (foreign, message) = match result {
                    Err(_) => (None, "Unresolved server requests could not be checked. Nothing was changed."),
                    Ok(None) => (None, "No unresolved generation request exists on the server for this meeting."),
                    Ok(Some(receipt)) => {
                        if receipt.request.meeting_id != id
                            || !receipt.request.valid()
                            || !matches!(receipt.state, bruhs_models::SummaryGenerationState::Pending)
                        {
                            (None, "The server reported an unresolved request for a different meeting or with an invalid identity. Nothing was changed.")
                        } else {
                            (Some(receipt.request), "An unresolved generation request from another client is blocking this meeting. Cancel it to allow a new generation.")
                        }
                    }
                };
                entry.foreign = foreign;
                entry.message = message.into();
                cx.emit(HubEvent::MeetingsChanged);
                cx.notify();
            },
        );
        cx.notify();
    }

    /// Fence the discovered request from another client; never starts generation.
    pub fn cancel_foreign_generation(&mut self, id: String, cx: &mut Context<'_, Self>) {
        let Some(entry) = self.meeting_actions.get_mut(&id) else {
            return;
        };
        let Some(request) = entry.foreign.clone() else {
            return;
        };
        if entry.phase != Phase::Complete {
            return;
        }
        entry.phase = Phase::Reading;
        entry.message = "Cancelling the unresolved server request; no generation will run…".into();
        let token = entry.checkpoint.token.clone();
        let client = self.client.clone();
        let fenced = request.clone();
        self.run(
            async move { client.cancel_summary_generation(&fenced).await },
            move |hub, result, cx| {
                let Some(entry) = hub.meeting_actions.get_mut(&id) else {
                    return;
                };
                if entry.checkpoint.token != token || entry.phase != Phase::Reading {
                    return;
                }
                entry.phase = Phase::Complete;
                match result.ok().flatten() {
                    None => {
                        entry.message = "Cancellation could not be confirmed. The unresolved request may still be blocking generation.".into();
                    }
                    Some(receipt) if receipt.request != request => {
                        entry.message = "The server returned a mismatched receipt; the unresolved request was not confirmed cancelled.".into();
                    }
                    Some(receipt) => match receipt.state {
                        bruhs_models::SummaryGenerationState::Pending => {
                            entry.message = "The server did not cancel the unresolved request; it remains pending. Try again later.".into();
                        }
                        bruhs_models::SummaryGenerationState::Cancelled => {
                            entry.foreign = None;
                            entry.message = "The unresolved server request is cancelled; no result will be saved for it. You can generate again.".into();
                        }
                        _ => {
                            // Its result may already be saved on the server; offer GET-only recovery.
                            entry.foreign = None;
                            entry.checkpoint.expected = None;
                            entry.phase = Phase::Recovery;
                            entry.message = "That request had already ended, so nothing was cancelled. Load the latest saved meeting to review it.".into();
                        }
                    },
                }
                cx.emit(HubEvent::MeetingsChanged);
                cx.notify();
            },
        );
        cx.notify();
    }

    /// Fence the saved generation request on the server, then adopt the latest
    /// validated meeting. Never starts or replays generation.
    pub fn cancel_meeting_action(&mut self, id: String, cx: &mut Context<'_, Self>) {
        if self.meeting_actions.get(&id).is_some_and(|entry| {
            entry.phase == Phase::Recovery && entry.checkpoint.generation.is_some()
        }) {
            self.launch_meeting_action(id, Action::CancelGeneration, cx);
        }
    }

    fn launch_meeting_action(&mut self, id: String, action: Action, cx: &mut Context<'_, Self>) {
        if !matches!(action, Action::LoadLatest | Action::CancelGeneration) {
            let mut matching = self.meetings.iter().filter(|meeting| meeting.id == id);
            let valid = matching.next().is_some_and(|meeting| {
                meeting_cache::validate_meeting(meeting).is_ok()
                    && self
                        .meeting_actions
                        .get(&id)
                        .is_some_and(|entry| meeting.updated_at == entry.checkpoint.minimum_version)
            }) && matching.next().is_none();
            if !valid {
                if let Some(entry) = self.meeting_actions.get_mut(&id) {
                    entry.checkpoint.generation = None;
                    entry.phase = Phase::Recovery;
                    entry.message = "Request not sent: the meeting changed while saving. Load the latest meeting before trying again.".into();
                }
                cx.emit(HubEvent::MeetingsChanged);
                cx.notify();
                return;
            }
        }
        if let Action::Generate { extract: true, .. } = action {
            if !self.prefs.personality.extract_observations
                || self
                    .meeting(&id)
                    .is_none_or(|meeting| meeting.skip_observations)
            {
                if let Some(entry) = self.meeting_actions.get_mut(&id) {
                    entry.checkpoint.generation = None;
                    entry.phase = Phase::Recovery;
                    entry.message = "Request not sent: observation settings changed. Load the latest meeting before trying again.".into();
                }
                cx.emit(HubEvent::MeetingsChanged);
                cx.notify();
                return;
            }
        }
        let Some(entry) = self.meeting_actions.get_mut(&id) else {
            return;
        };
        entry.phase = Phase::Running;
        entry.message = match action {
            Action::Generate { .. } => {
                "Generating summary… You can leave this view; keep the app open."
            }
            Action::SetSkip(_) => "Saving observation setting…",
            Action::LoadLatest => "Loading the latest saved meeting; no generation will run…",
            Action::CancelGeneration => {
                "Cancelling the saved generation request; no generation will run…"
            }
        }
        .into();
        let checkpoint = entry.checkpoint.clone();
        let token = checkpoint.token.clone();
        let client = self.client.clone();
        let requested_id = id.clone();
        self.run(
            async move { transport::perform(&client, &requested_id, action, checkpoint).await },
            move |hub, result, cx| hub.finish_meeting_action(id, &token, result, cx),
        );
        cx.notify();
    }

    fn finish_meeting_action(
        &mut self,
        id: String,
        token: &str,
        result: transport::Outcome,
        cx: &mut Context<'_, Self>,
    ) {
        if !self
            .meeting_actions
            .get(&id)
            .is_some_and(|entry| entry.checkpoint.token == token && entry.phase == Phase::Running)
        {
            return;
        }
        let result = result.and_then(|meeting| {
            self.cache_meeting(&meeting).map_err(|_| transport::Failure {
                message: "A stale, conflicting or deleted meeting reply was ignored. Load the latest saved meeting.", expected: None,
            })?;
            Ok(meeting)
        });
        match result {
            Ok(meeting) => {
                if let Some(current) = self.meetings.iter_mut().find(|current| current.id == id) {
                    *current = meeting;
                } else {
                    self.meetings.insert(0, meeting);
                }
                self.bump_search_revision();
                if let Some(entry) = self.meeting_actions.get_mut(&id) {
                    entry.phase = Phase::AwaitingSave;
                    entry.message = "Saving the validated meeting on this device…".into();
                }
                self.refresh_people(cx);
            }
            Err(failure) => {
                if let Some(entry) = self.meeting_actions.get_mut(&id) {
                    entry.phase = Phase::Recovery;
                    entry.checkpoint.expected = failure.expected;
                    entry.message = failure.message.into();
                }
            }
        }
        cx.emit(HubEvent::MeetingsChanged);
        cx.notify();
    }
}

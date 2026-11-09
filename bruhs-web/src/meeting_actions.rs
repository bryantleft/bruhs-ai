//! App-owned meeting mutations: acknowledge durable metadata before publishing success.
use crate::storage::meeting_generations::{self as generations, Checkpoint};
use crate::{
    api,
    state::{Meeting, MeetingSummary, MeetingsSignal},
    transcript_sync::TranscriptSync,
    workflow_executor::WorkflowExecutor,
};
use leptos::prelude::*;
use std::collections::BTreeMap;
mod generation;

#[derive(Clone, Default, PartialEq, Eq)]
pub struct Status {
    pub busy: bool,
    pub message: Option<String>,
    pub recovery: bool,
    /// A saved generation request exists and can be fenced explicitly.
    pub cancellable: bool,
    /// An unresolved request from another client was discovered on the server.
    pub foreign_pending: bool,
}

#[derive(Clone)]
enum Action {
    Generate {
        extract: bool,
        replace: bool,
    },
    SetSkip(bool),
    RecoverGeneration(Option<Checkpoint>),
    CancelGeneration(Option<Checkpoint>),
    DiscoverPending,
    CancelForeign(bruhs_models::SummaryGenerationRequest),
    Readback {
        expected: Option<Box<Expected>>,
        minimum_version: i64,
    },
}
#[derive(Clone, Default)]
struct Expected {
    summary: Option<MeetingSummary>,
    skip: Option<bool>,
}
#[derive(Default)]
struct Entry {
    status: Status,
    retry: Option<Action>,
    /// Discovered server-side request that this device never saved.
    foreign: Option<bruhs_models::SummaryGenerationRequest>,
}

#[derive(Clone, Copy)]
pub struct MeetingActions {
    entries: RwSignal<BTreeMap<String, Entry>>,
    meetings: ReadSignal<Vec<Meeting>>,
    sync: TranscriptSync,
    contacts: WorkflowExecutor,
    restore: RwSignal<Restore>,
    personality: ReadSignal<bruhs_models::PersonalitySettings>,
}

#[derive(Clone)]
enum Restore {
    Loading,
    Ready,
    Failed(String),
}

impl MeetingActions {
    pub fn install() {
        let actions = Self {
            entries: RwSignal::new(BTreeMap::new()),
            meetings: expect_context::<MeetingsSignal>().0,
            sync: expect_context(),
            contacts: expect_context(),
            restore: RwSignal::new(Restore::Loading),
            personality: expect_context::<crate::state::PersonalitySettingsSignal>().0,
        };
        provide_context(actions);
        actions.restore_generations();
    }
    pub fn status(self, id: &str) -> Status {
        match self.restore.get() {
            Restore::Loading => return Status { busy: true, message: Some("Loading saved meeting requests…".into()), recovery: false, cancellable: false, foreign_pending: false },
            Restore::Failed(error) => return Status { busy: false, message: Some(format!("Saved meeting requests could not be loaded. {error} Retry loading before making changes.")), recovery: true, cancellable: false, foreign_pending: false },
            Restore::Ready => {},
        }
        self.entries.with(|all| {
            all.get(id)
                .map(|entry| entry.status.clone())
                .unwrap_or_default()
        })
    }
    pub fn generate(self, id: String, extract: bool, replace: bool) {
        self.start(id, Action::Generate { extract, replace }, false);
    }
    pub fn set_skip(self, id: String, skip: bool) {
        self.start(id, Action::SetSkip(skip), false);
    }
    /// Fence the saved generation request; never starts or replays generation.
    pub fn cancel_generation(self, id: String) {
        let remembered = self.entries.with_untracked(|all| {
            all.get(&id).and_then(|entry| match &entry.retry {
                Some(Action::RecoverGeneration(checkpoint))
                | Some(Action::CancelGeneration(checkpoint)) => checkpoint.clone(),
                _ => None,
            })
        });
        self.start(id, Action::CancelGeneration(remembered), true);
    }
    /// Read-only discovery of an unresolved server request without a saved identity.
    pub fn discover_pending(self, id: String) {
        self.start(id, Action::DiscoverPending, true);
    }
    /// Fence the discovered request from another client; never starts generation.
    pub fn cancel_foreign(self, id: String) {
        let foreign = self
            .entries
            .with_untracked(|all| all.get(&id).and_then(|entry| entry.foreign.clone()));
        if let Some(request) = foreign {
            self.start(id, Action::CancelForeign(request), true);
        }
    }
    pub fn retry(self, id: String) {
        if matches!(self.restore.get_untracked(), Restore::Failed(_)) {
            self.restore_generations();
            return;
        }
        let retry = self
            .entries
            .with_untracked(|all| all.get(&id).and_then(|entry| entry.retry.clone()));
        if let Some(action) = retry {
            self.start(id, action, true);
        }
    }
    fn start(self, id: String, action: Action, recovery: bool) {
        if !matches!(self.restore.get_untracked(), Restore::Ready) {
            return;
        }
        let blocked = self.entries.with_untracked(|all| {
            all.get(&id)
                .is_some_and(|entry| entry.status.busy || (!recovery && entry.status.recovery))
        });
        if blocked {
            return;
        }
        let message = match action {
            Action::Generate { .. } => {
                "Generating summary… You can leave this view; keep the app open."
            }
            Action::SetSkip(_) => "Saving observation setting…",
            Action::Readback { .. } => "Loading the latest saved meeting…",
            Action::RecoverGeneration(_) => {
                "Checking the saved generation request; no generation will run…"
            }
            Action::CancelGeneration(_) => {
                "Cancelling the saved generation request; no generation will run…"
            }
            Action::DiscoverPending => "Checking the server for unresolved generation requests…",
            Action::CancelForeign(_) => {
                "Cancelling the unresolved server request; no generation will run…"
            }
        };
        self.entries.update(|all| {
            all.entry(id.clone()).or_default().status = Status {
                busy: true,
                message: Some(message.into()),
                recovery: false,
                cancellable: false,
                foreign_pending: false,
            };
        });
        wasm_bindgen_futures::spawn_local(async move {
            // Let the immediate busy state paint before network/cache work.
            gloo_timers::future::TimeoutFuture::new(0).await;
            let result = self.perform(&id, action).await;
            self.entries.update(|all| {
                let entry = all.entry(id).or_default();
                match result {
                    Ok(message) => {
                        entry.status = Status {
                            busy: false,
                            message: Some(message),
                            recovery: false,
                            cancellable: false,
                            foreign_pending: entry.foreign.is_some(),
                        };
                        entry.retry = None;
                    }
                    Err((message, retry)) => {
                        entry.status = Status {
                            busy: false,
                            message: Some(message),
                            recovery: true,
                            cancellable: matches!(
                                retry,
                                Action::RecoverGeneration(Some(_))
                                    | Action::CancelGeneration(Some(_))
                            ),
                            foreign_pending: entry.foreign.is_some(),
                        };
                        entry.retry = Some(retry);
                    }
                }
            });
        });
    }

    async fn perform(self, id: &str, action: Action) -> Result<String, (String, Action)> {
        let baseline = self
            .meetings
            .with_untracked(|all| all.iter().find(|meeting| meeting.id == id).cloned());
        let minimum_version = baseline.as_ref().map_or(0, |meeting| meeting.updated_at);
        let load_latest = || Action::Readback {
            expected: None,
            minimum_version,
        };
        match action {
            Action::Generate { extract, replace } => {
                self.generate_saved(id, baseline, extract, replace).await
            }
            Action::SetSkip(skip) => {
                let pending = generations::load(id)
                    .await
                    .map_err(|error| (error.to_string(), Action::RecoverGeneration(None)))?;
                if let Some(checkpoint) = pending {
                    return Err((
                        "A saved generation request must be resolved before changing this meeting."
                            .into(),
                        Action::RecoverGeneration(Some(checkpoint)),
                    ));
                }
                let updated = deadline(api::set_meeting_skip_observations(id, skip), 15_000).await
                    .map_err(|error| (format!("Observation setting was not confirmed. {error} Load the latest saved meeting before trying again."), load_latest()))?;
                validate_readback(
                    id,
                    minimum_version,
                    &updated,
                    Some(&Expected {
                        summary: None,
                        skip: Some(skip),
                    }),
                )
                .map_err(|error| (error, load_latest()))?;
                self.sync.acknowledge_meeting(updated).await
                    .map_err(|error| (format!("The server saved the setting, but this device could not cache it. {error}"), Action::Readback { expected: Some(Box::new(Expected { summary: None, skip: Some(skip) })), minimum_version }))?;
                Ok("Observation setting saved on this device.".into())
            }
            Action::RecoverGeneration(checkpoint) => self.recover_generation(id, checkpoint).await,
            Action::CancelGeneration(checkpoint) => {
                self.cancel_saved_generation(id, checkpoint).await
            }
            Action::DiscoverPending => self.discover_pending_generation(id).await,
            Action::CancelForeign(request) => {
                self.cancel_foreign_generation(id, request, minimum_version)
                    .await
            }
            Action::Readback {
                expected,
                minimum_version,
            } => {
                self.readback(id, expected.map(|value| *value), minimum_version)
                    .await?;
                Ok(
                    "Latest saved meeting loaded on this device. You can generate again if needed."
                        .into(),
                )
            }
        }
    }

    async fn readback(
        self,
        id: &str,
        expected: Option<Expected>,
        minimum_version: i64,
    ) -> Result<(), (String, Action)> {
        let retry = || Action::Readback {
            expected: expected.clone().map(Box::new),
            minimum_version,
        };
        let updated = deadline(async { api::get_meeting(id).await.map_err(|error| error.to_string())?.ok_or_else(|| "Meeting is unavailable on the server.".to_string()) }, 15_000).await
            .map_err(|error| (format!("Could not load the saved meeting. {error} No new generation will run when you retry loading."), retry()))?;
        validate_readback(id, minimum_version, &updated, expected.as_ref()).map_err(|error| {
            (
                error,
                Action::Readback {
                    expected: None,
                    minimum_version,
                },
            )
        })?;
        self.sync.acknowledge_meeting(updated).await
            .map_err(|error| (format!("The saved meeting could not be cached on this device. {error} Retry loading; this will not regenerate the summary."), retry()))?;
        Ok(())
    }
}

fn validate_readback(
    id: &str,
    minimum_version: i64,
    meeting: &Meeting,
    expected: Option<&Expected>,
) -> Result<(), String> {
    if meeting.id != id || meeting.updated_at < minimum_version {
        return Err("The server returned a foreign or older meeting. Existing content was retained; retry loading the latest meeting.".into());
    }
    if expected.is_some_and(|expected| {
        expected
            .summary
            .as_ref()
            .is_some_and(|summary| meeting.summary.as_ref() != Some(summary))
            || expected
                .skip
                .is_some_and(|skip| meeting.skip_observations != skip)
    }) {
        return Err("The latest saved meeting differs from this request. Existing content was retained; load the latest meeting to review it.".into());
    }
    Ok(())
}

async fn deadline<T, E: std::fmt::Display>(
    future: impl std::future::Future<Output = Result<T, E>>,
    ms: u32,
) -> Result<T, String> {
    match futures_util::future::select(
        Box::pin(future),
        Box::pin(gloo_timers::future::TimeoutFuture::new(ms)),
    )
    .await
    {
        futures_util::future::Either::Left((result, _)) => {
            result.map_err(|error| error.to_string())
        }
        futures_util::future::Either::Right(_) => {
            Err("The request timed out; its server outcome may be uncertain.".into())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meeting() -> Meeting {
        let mut meeting = Meeting::new("Summary receipt", 0.0);
        meeting.id = "owned-meeting".into();
        meeting.updated_at = 12;
        meeting.summary = Some(MeetingSummary {
            tldr: "Saved result".into(),
            action_items: vec![],
            decisions: vec![],
            topics: vec![],
            generated_by: "fixture".into(),
            generated_at: 12,
        });
        meeting
    }

    #[test]
    fn readback_requires_matching_identity_and_nondecreasing_version() {
        let meeting = meeting();
        assert!(validate_readback(&meeting.id, 12, &meeting, None).is_ok());
        assert!(validate_readback("another-meeting", 12, &meeting, None).is_err());
        assert!(validate_readback(&meeting.id, 13, &meeting, None).is_err());
    }

    #[test]
    fn generation_receipt_requires_the_complete_saved_summary() {
        let mut meeting = meeting();
        let expected = Expected {
            summary: meeting.summary.clone(),
            skip: None,
        };
        assert!(validate_readback(&meeting.id, 12, &meeting, Some(&expected)).is_ok());
        meeting.summary.as_mut().unwrap().generated_at += 1;
        assert!(validate_readback(&meeting.id, 12, &meeting, Some(&expected)).is_err());
        meeting.summary = None;
        assert!(validate_readback(&meeting.id, 12, &meeting, Some(&expected)).is_err());
        // An explicit read-only recovery can load a concurrent server result,
        // without claiming it is the result of this generation request.
        assert!(validate_readback(&meeting.id, 12, &meeting, None).is_ok());
    }

    #[test]
    fn setting_receipt_requires_requested_opt_out() {
        let mut meeting = meeting();
        let expected = Expected {
            summary: None,
            skip: Some(true),
        };
        assert!(validate_readback(&meeting.id, 12, &meeting, Some(&expected)).is_err());
        meeting.skip_observations = true;
        assert!(validate_readback(&meeting.id, 12, &meeting, Some(&expected)).is_ok());
    }
}

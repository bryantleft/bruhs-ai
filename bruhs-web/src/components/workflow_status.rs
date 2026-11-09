use bruhs_models::{AutomationState, BackgroundJobStatus};
use leptos::prelude::*;

use crate::durable_metadata::SaveState;
use crate::state::{ActiveView, AppView, AutomationStateSignal, SetActiveView};
use crate::workflow_executor::WorkflowExecutor;

#[derive(Clone, Debug, PartialEq, Eq)]
struct Notice {
    message: String,
    error: bool,
    dismiss_key: Option<String>,
}

// No workflow titles, operation contents, or private recovery data in chrome.
fn notice(state: &AutomationState, save: &SaveState, load_failed: bool) -> Option<Notice> {
    let (message, error, dismiss_key) = if load_failed || matches!(save, SaveState::Error { .. }) {
        (
            "Workflow needs attention. Open workflows to review the error and recovery options."
                .into(),
            true,
            None,
        )
    } else if let Some(job) = state
        .jobs
        .iter()
        .find(|job| job.status == BackgroundJobStatus::Running)
    {
        let mode = if job.auto_run {
            "Background workflow running"
        } else {
            "Workflow ready for next step"
        };
        (
            format!("{mode} · {} steps saved", job.next_step),
            false,
            None,
        )
    } else if state
        .jobs
        .iter()
        .any(|job| job.status == BackgroundJobStatus::Paused)
    {
        (
            "Workflow paused · saved steps are retained".into(),
            false,
            None,
        )
    } else if state
        .jobs
        .iter()
        .any(|job| job.status == BackgroundJobStatus::Failed)
    {
        (
            "Workflow needs attention. Open workflows to review the error and recovery options."
                .into(),
            true,
            None,
        )
    } else {
        let job = state.jobs.iter().max_by_key(|job| job.updated_at_ms)?;
        if *save != SaveState::Saved {
            return None;
        }
        let label = match job.status {
            BackgroundJobStatus::Completed => "Workflow completed · saved locally",
            BackgroundJobStatus::RolledBack => "Workflow undone · saved locally",
            _ => return None,
        };
        (
            label.into(),
            false,
            Some(format!("{}:{}:{:?}", job.id, job.updated_at_ms, job.status)),
        )
    };
    Some(Notice {
        message,
        error,
        dismiss_key,
    })
}

#[component]
pub fn WorkflowStatus() -> impl IntoView {
    let AutomationStateSignal(automation) = expect_context();
    let ActiveView(active) = expect_context();
    let SetActiveView(set_active) = expect_context();
    let executor = expect_context::<WorkflowExecutor>();
    let dismissed = RwSignal::new(None::<String>);
    let current = Memo::new(move |_| {
        automation.with(|state| {
            notice(
                state,
                &automation.status.get(),
                executor.load_error.get().is_some(),
            )
        })
    });
    view! {
        <Show when=move || active.get() != AppView::Automations && current.get().is_some_and(|item| item.dismiss_key.is_none() || item.dismiss_key != dismissed.get())>
            <section aria-label="Workflow status" class="shrink-0 border-b border-longan-800/50 bg-longan-900 px-4 py-2">
                {move || current.get().map(|item| {
                    let role = if item.error { "alert" } else { "status" };
                    let color = if item.error { "text-body-sm text-lychee-300" } else { "text-body-sm text-rambutan-300" };
                    view! {
                        <div class="flex flex-wrap items-center justify-between gap-x-3 gap-y-1">
                            <p role=role aria-live="polite" class=color>{item.message}</p>
                            <div class="flex flex-wrap items-center gap-2">
                                <button type="button" class="min-h-11 rounded-grape px-2 text-label-sm text-persimmon-300 cursor-pointer" on:click=move |_| set_active.set(AppView::Automations)>"Open workflows"</button>
                                {item.dismiss_key.map(|key| view! {
                                    <button type="button" aria-label="Dismiss workflow status" class="min-h-11 rounded-grape px-2 text-label-sm text-rambutan-400 cursor-pointer" on:click=move |_| dismissed.set(Some(key.clone()))>"Dismiss"</button>
                                })}
                            </div>
                        </div>
                    }
                })}
            </section>
        </Show>
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn errors_never_copy_private_recovery_messages_into_global_chrome() {
        let result = notice(
            &AutomationState::default(),
            &SaveState::Error {
                message: "private fixture".into(),
                conflict: true,
            },
            false,
        )
        .unwrap();
        assert!(!result.message.contains("private fixture"));
        assert!(result.error && result.dismiss_key.is_none());
    }
    #[test]
    fn empty_workspace_has_no_permanent_status_strip() {
        assert!(notice(&AutomationState::default(), &SaveState::Saved, false).is_none());
    }
}

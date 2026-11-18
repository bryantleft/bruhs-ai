use bruhs_models::{AutomationState, BackgroundJobStatus};
use leptos::prelude::*;

use crate::durable_metadata::SaveState;
use crate::state::{ActiveView, AppView, AutomationStateSignal, SetActiveView};
use crate::workflow_executor::WorkflowExecutor;

use bruhs_models::logic::workflow_review::{
    background_running, workflow_notice, WorkflowNotice, DISMISS_WORKFLOW_STATUS, OPEN_WORKFLOWS,
    PAUSE_BACKGROUND, PAUSE_BACKGROUND_LABEL, WORKFLOW_STATUS_LABEL,
};

// No workflow titles, operation contents, or private recovery data in chrome;
// the rule is shared with desktop (`workflow_notice`).
fn notice(
    state: &AutomationState,
    save: &SaveState,
    load_failed: bool,
    pending_review: bool,
) -> Option<WorkflowNotice> {
    workflow_notice(
        state,
        load_failed || matches!(save, SaveState::Error { .. }),
        *save == SaveState::Saved,
        pending_review,
    )
}

#[component]
pub fn WorkflowStatus() -> impl IntoView {
    let AutomationStateSignal(automation) = expect_context();
    let ActiveView(active) = expect_context();
    let SetActiveView(set_active) = expect_context();
    let executor = expect_context::<WorkflowExecutor>();
    let bridge = expect_context::<crate::workspace_bridge::WorkspaceBridge>();
    let dismissed = RwSignal::new(None::<String>);
    let current = Memo::new(move |_| {
        let pending_review = bridge.pending_review_id().is_some();
        automation.with(|state| {
            notice(
                state,
                &automation.status.get(),
                executor.load_error.get().is_some(),
                pending_review,
            )
        })
    });
    view! {
        <Show when=move || active.get() != AppView::Automations && current.get().is_some_and(|item| item.dismiss_key.is_none() || item.dismiss_key != dismissed.get())>
            <section aria-label=WORKFLOW_STATUS_LABEL class="shrink-0 border-b border-longan-800/50 bg-longan-900 px-4 py-2">
                {move || current.get().map(|item| {
                    let role = if item.error { "alert" } else { "status" };
                    let color = if item.error { "text-body-sm text-lychee-300" } else { "text-body-sm text-rambutan-300" };
                    view! {
                        <div class="flex flex-wrap items-center justify-between gap-x-3 gap-y-1">
                            <p role=role aria-live="polite" class=color>{item.message}</p>
                            <div class="flex flex-wrap items-center gap-2">
                                // As on desktop: stop every background run from any page.
                                <Show when=move || automation.with(background_running)>
                                    <button type="button" aria-label=PAUSE_BACKGROUND_LABEL class="bruhs-control min-h-control rounded-grape px-2 text-label-sm text-rambutan-300 cursor-pointer" on:click=move |_| automation.update(|state| {
                                        let running: Vec<String> = state.jobs.iter().filter(|job| job.auto_run && job.status == BackgroundJobStatus::Running).map(|job| job.id.clone()).collect();
                                        for id in running {
                                            // Refused only when it finished meanwhile.
                                            let _ = state.pause_job(&id, crate::clock::now_ms());
                                        }
                                    })>{PAUSE_BACKGROUND}</button>
                                </Show>
                                <button type="button" class="bruhs-control min-h-control rounded-grape px-2 text-label-sm text-persimmon-300 cursor-pointer" on:click=move |_| set_active.set(AppView::Automations)>{OPEN_WORKFLOWS}</button>
                                {item.dismiss_key.map(|key| view! {
                                    <button type="button" aria-label=DISMISS_WORKFLOW_STATUS class="bruhs-control min-h-control rounded-grape px-2 text-label-sm text-rambutan-400 cursor-pointer" on:click=move |_| dismissed.set(Some(key.clone()))>"Dismiss"</button>
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
            true,
        )
        .unwrap();
        assert!(!result.message.contains("private fixture"));
        assert!(result.error && result.dismiss_key.is_none());
    }
    #[test]
    fn empty_workspace_has_no_permanent_status_strip() {
        assert!(notice(&AutomationState::default(), &SaveState::Saved, false, false).is_none());
    }

    #[test]
    fn a_pending_mcp_review_is_announced_without_its_payload() {
        let result = notice(&AutomationState::default(), &SaveState::Saved, false, true).unwrap();
        assert_eq!(
            result.message,
            bruhs_models::workspace_bridge::PENDING_REVIEW_NOTICE
        );
        assert!(!result.error && result.dismiss_key.is_none());
    }
}

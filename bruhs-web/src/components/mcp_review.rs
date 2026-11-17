//! Workflow review on Agent workflows: an MCP client's undo request, and
//! every workflow still waiting in preview (an MCP proposal or one left from
//! an earlier session), each with its full payload. Start, Decline, Keep
//! changes and Undo are the user's; the client can only ask. A started
//! workflow goes on show in the workflow history, which carries its controls.

use bruhs_models::logic::workflow_history::UNDO;
use bruhs_models::workspace_bridge::ROLLBACK_REVIEW_NOTE;
use bruhs_models::{AgentAccess, AgentDomain, AutomationState, WorkflowStatus};
use leptos::prelude::*;

use crate::state::{AutomationStateSignal, ReviewedWorkflowSignal};
use crate::ui::{Button, ButtonSize, ButtonVariant};
use crate::workflow_executor::WorkflowExecutor;
use crate::workspace_bridge::WorkspaceBridge;

/// Previewed workflows newest first, apart from `shown` (the page's own
/// dry run, reviewed in its own section).
pub(crate) fn awaiting_review(state: &AutomationState, shown: Option<&str>) -> Vec<String> {
    state
        .workflows
        .iter()
        .rev()
        .filter(|workflow| {
            workflow.status == WorkflowStatus::Previewed && Some(workflow.id.as_str()) != shown
        })
        .map(|workflow| workflow.id.clone())
        .collect()
}

/// Domains of `workflow_id` whose Write grant is missing, in step order.
pub(crate) fn blocked_domains(state: &AutomationState, workflow_id: &str) -> Vec<AgentDomain> {
    let mut blocked = Vec::new();
    for step in state
        .workflows
        .iter()
        .filter(|workflow| workflow.id == workflow_id)
        .flat_map(|workflow| &workflow.steps)
    {
        let domain = step.operation.domain();
        if !state.permissions.allows(domain, AgentAccess::Write) && !blocked.contains(&domain) {
            blocked.push(domain);
        }
    }
    blocked
}

#[component]
pub fn WorkflowReviews(
    /// The page's own dry run, shown in its own section.
    #[prop(into)]
    shown: Signal<Option<String>>,
    /// Where an executor refusal is reported.
    error: RwSignal<Option<String>>,
) -> impl IntoView {
    let bridge = expect_context::<WorkspaceBridge>();
    let AutomationStateSignal(automation) = expect_context();
    let undo_requests = Memo::new(move |_| {
        bridge.reviews.with(|reviews| {
            let mut ids: Vec<String> = reviews.rollback_reviews.keys().cloned().collect();
            ids.sort();
            ids
        })
    });
    let awaiting =
        Memo::new(move |_| automation.with(|state| awaiting_review(state, shown.get().as_deref())));
    view! {
        <For each=move || undo_requests.get() key=|id| id.clone() let:id>
            <UndoRequest id=id error=error />
        </For>
        <For each=move || awaiting.get() key=|id| id.clone() let:id>
            <AwaitingReview id=id />
        </For>
    }
}

#[component]
fn UndoRequest(id: String, error: RwSignal<Option<String>>) -> impl IntoView {
    let bridge = expect_context::<WorkspaceBridge>();
    let executor = expect_context::<WorkflowExecutor>();
    let id = StoredValue::new(id);
    let pending = move || id.with_value(|id| bridge.rollback_pending(id));
    view! {
        <Show when=pending>
            <section role="group" aria-label=ROLLBACK_REVIEW_NOTE data-testid="mcp-rollback-review" class="rounded-grape bg-longan-900/70 p-4 inset-ring inset-ring-lychee-400/30">
                <h2 class="text-h4 text-rambutan-100">"Undo requested"</h2>
                <p class="mt-1 text-body-sm text-lychee-300">{ROLLBACK_REVIEW_NOTE}</p>
                <super::workflow_payload::WorkflowReview workflow_id=id.get_value() />
                <div class="mt-3 flex flex-wrap gap-2">
                    <Button variant=ButtonVariant::Outline size=ButtonSize::Small on_click=Callback::new(move |_| id.with_value(|id| bridge.keep_changes(id)))>
                        "Keep changes"
                    </Button>
                    <Button variant=ButtonVariant::Primary size=ButtonSize::Small on_click=Callback::new(move |_| {
                        error.set(None);
                        if let Err(refusal) = executor.rollback(id.get_value()) {
                            error.set(Some(refusal.to_string()));
                        }
                    })>
                        {UNDO}
                    </Button>
                </div>
            </section>
        </Show>
    }
}

#[component]
fn AwaitingReview(id: String) -> impl IntoView {
    let bridge = expect_context::<WorkspaceBridge>();
    let AutomationStateSignal(automation) = expect_context();
    let ReviewedWorkflowSignal(reviewed) = expect_context();
    let id = StoredValue::new(id);
    let title = move || {
        automation.with(|state| {
            id.with_value(|id| {
                state
                    .workflows
                    .iter()
                    .find(|workflow| &workflow.id == id)
                    .map(|workflow| workflow.title.clone())
                    .unwrap_or_default()
            })
        })
    };
    let blocked = move || automation.with(|state| id.with_value(|id| blocked_domains(state, id)));
    let operations = move || {
        automation.with(|state| {
            id.with_value(|id| {
                state
                    .workflows
                    .iter()
                    .find(|workflow| &workflow.id == id)
                    .map(|workflow| {
                        workflow
                            .steps
                            .iter()
                            .map(|step| step.operation.clone())
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default()
            })
        })
    };
    let origin = move || {
        if id.with_value(|id| bridge.owns(id)) {
            "Proposed by the connected MCP client. Nothing has run yet."
        } else {
            "Waiting in preview. Nothing has run yet."
        }
    };
    let now = crate::clock::now_ms;
    view! {
        <section class="rounded-grape bg-longan-900/70 p-4 inset-ring inset-ring-blueberry-400/30" aria-label="Workflow awaiting review" data-testid="workflow-awaiting-review">
            <div class="flex flex-wrap items-start justify-between gap-3">
                <div class="min-w-0">
                    <h2 class="text-h4 text-rambutan-100 break-words">{title}</h2>
                    <p class="text-body-sm text-rambutan-500">{origin}</p>
                </div>
                <Button variant=ButtonVariant::Ghost size=ButtonSize::Small on_click=Callback::new(move |_| automation.update(|state| {
                    let _ = id.with_value(|id| state.preview(id, now()));
                }))>"Recheck grants"</Button>
            </div>
            {move || view! { <super::workflow_payload::WorkflowPayload operations=operations() /> }}
            {move || {
                let blocked = blocked();
                (!blocked.is_empty()).then(|| view! {
                    <p role="alert" class="mt-3 text-body-sm text-durian-300">{format!(
                        "Grant Write for: {}.",
                        blocked.iter().map(|domain| domain.label()).collect::<Vec<_>>().join(", ")
                    )}</p>
                })
            }}
            <div class="mt-3 flex flex-wrap gap-2">
                // Once started, it goes on show in the history, where its Run,
                // Pause and Undo controls are (desktop starts the workflow on show).
                <Button variant=ButtonVariant::Primary size=ButtonSize::Small disabled=Signal::derive(move || !blocked().is_empty()) on_click=Callback::new(move |_| {
                    let started = automation
                        .try_update(|state| id.with_value(|id| state.start_job(id, now())))
                        .flatten();
                    if started.is_some() {
                        reviewed.set(Some(id.get_value()));
                    }
                })>"Start transaction"</Button>
                <Button variant=ButtonVariant::Ghost size=ButtonSize::Small on_click=Callback::new(move |_| automation.update(|state| {
                    let _ = id.with_value(|id| state.decline_workflow(id, now()));
                }))>"Decline proposal"</Button>
            </div>
        </section>
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bruhs_models::WorkflowOperation;

    fn task(title: &str) -> WorkflowOperation {
        WorkflowOperation::CreateTask {
            title: title.into(),
        }
    }

    #[test]
    fn previews_wait_newest_first_and_the_pages_own_dry_run_is_left_out() {
        let mut state = AutomationState::default();
        state
            .permissions
            .set(AgentDomain::Tasks, AgentAccess::Write, true);
        let older = state.create_workflow("Older", vec![task("a")], 1);
        let shown = state.create_workflow("Shown", vec![task("b")], 2);
        let newer = state.create_workflow("Newer", vec![task("c")], 3);
        let declined = state.create_workflow("Declined", vec![task("d")], 4);
        for id in [&older, &shown, &newer, &declined] {
            state.preview(id, 5).unwrap();
        }
        assert!(state.decline_workflow(&declined, 6));
        assert_eq!(awaiting_review(&state, Some(&shown)), vec![newer, older]);
    }

    #[test]
    fn missing_write_grants_are_named_once_in_step_order() {
        let mut state = AutomationState::default();
        let id = state.create_workflow(
            "Mixed",
            vec![
                WorkflowOperation::AddCapture {
                    content: "a".into(),
                },
                task("b"),
                WorkflowOperation::AddCapture {
                    content: "c".into(),
                },
            ],
            1,
        );
        assert_eq!(
            blocked_domains(&state, &id),
            vec![AgentDomain::Capture, AgentDomain::Tasks]
        );
        state
            .permissions
            .set(AgentDomain::Capture, AgentAccess::Write, true);
        assert_eq!(blocked_domains(&state, &id), vec![AgentDomain::Tasks]);
    }
}

//! The workflow history on Agent workflows: every workflow, one at a time,
//! with Previous (older) and Next (newer), as on desktop. Which workflows
//! count, their order, the wording and the paging come from
//! `bruhs_models::logic::workflow_history`. Browsing changes no record.

use bruhs_models::logic::workflow_history::{
    self, NEXT_WORKFLOW, NEXT_WORKFLOW_LABEL, NO_WORKFLOW_YET, PREVIOUS_WORKFLOW,
    PREVIOUS_WORKFLOW_LABEL,
};
use leptos::html;
use leptos::prelude::*;

use crate::state::{AutomationStateSignal, ReviewedWorkflowSignal};
use crate::ui::{button_class, ButtonSize, ButtonVariant};

/// What the panel shows of one stop, owned so the view never holds a
/// borrow of the automation state.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Stop {
    id: String,
    title: String,
    status: String,
    label: String,
    previous: Option<String>,
    next: Option<String>,
}

fn stop(page: workflow_history::Page<'_>) -> Stop {
    Stop {
        id: page.workflow.id.clone(),
        title: page.workflow.title.clone(),
        status: page.status(),
        label: page.label(),
        previous: page.previous.map(str::to_owned),
        next: page.next.map(str::to_owned),
    }
}

#[component]
pub fn WorkflowHistory() -> impl IntoView {
    let AutomationStateSignal(automation) = expect_context();
    let ReviewedWorkflowSignal(selected) = expect_context();
    let current = Memo::new(move |_| {
        automation.with(|state| {
            selected.with(|selected| workflow_history::page(state, selected.as_deref()).map(stop))
        })
    });
    let text = move |read: fn(&Stop) -> &str| {
        move || current.with(|stop| stop.as_ref().map(read).unwrap_or_default().to_owned())
    };
    // `side` picks Previous's or Next's neighbour.
    let at_end = move |side: fn(&Stop) -> &Option<String>| {
        move || current.with(|stop| stop.as_ref().is_none_or(|stop| side(stop).is_none()))
    };
    let neighbour = move |side: fn(&Stop) -> &Option<String>| {
        current.with_untracked(|stop| stop.as_ref().and_then(|stop| side(stop).clone()))
    };
    let previous_button = NodeRef::<html::Button>::new();
    let next_button = NodeRef::<html::Button>::new();
    // A button disables itself on reaching either end, which drops focus;
    // hand focus to the other one so the keyboard stays in the history.
    let show =
        move |id: Option<String>, pressed: NodeRef<html::Button>, other: NodeRef<html::Button>| {
            let Some(id) = id else {
                return;
            };
            selected.set(Some(id));
            request_animation_frame(move || {
                if pressed
                    .get_untracked()
                    .is_some_and(|button| button.disabled())
                {
                    if let Some(other) = other.get_untracked() {
                        let _ = other.focus();
                    }
                }
            });
        };
    view! {
        <section class="rounded-grape bg-longan-900/70 p-4 inset-ring inset-ring-longan-700/50" aria-labelledby="workflow-history-title">
            <h2 id="workflow-history-title" class="text-label-sm uppercase tracking-wide text-rambutan-400">"Workflow history"</h2>
            // Nothing until the saved workflows load, so the empty line never
            // flashes over a history that exists.
            <Show when=move || automation.ready()>
                <Show
                    when=move || current.with(Option::is_some)
                    fallback=|| view! { <p class="mt-1 text-body-sm text-rambutan-500">{NO_WORKFLOW_YET}</p> }
                >
                    <div role="group" aria-label=text(|stop| &stop.label) data-testid="workflow-history" class="min-w-0">
                        // Read out after each step, as the buttons keep focus.
                        <div aria-live="polite" aria-atomic="true">
                            <p class="mt-1 text-label text-rambutan-200 break-words">{text(|stop| &stop.title)}</p>
                            <p class="text-body-sm text-rambutan-500">{text(|stop| &stop.status)}</p>
                        </div>
                        <div class="mt-3 flex flex-wrap gap-2">
                            <button
                                node_ref=previous_button
                                type="button"
                                class=button_class(ButtonVariant::Outline, ButtonSize::Small, "")
                                aria-label=PREVIOUS_WORKFLOW_LABEL
                                data-testid="previous-workflow"
                                disabled=at_end(|stop| &stop.previous)
                                on:click=move |_| show(neighbour(|stop| &stop.previous), previous_button, next_button)
                            >
                                {PREVIOUS_WORKFLOW}
                            </button>
                            <button
                                node_ref=next_button
                                type="button"
                                class=button_class(ButtonVariant::Outline, ButtonSize::Small, "")
                                aria-label=NEXT_WORKFLOW_LABEL
                                data-testid="next-workflow"
                                disabled=at_end(|stop| &stop.next)
                                on:click=move |_| show(neighbour(|stop| &stop.next), next_button, previous_button)
                            >
                                {NEXT_WORKFLOW}
                            </button>
                        </div>
                        <super::workflow_payload::WorkflowReview workflow_id=Signal::derive(text(|stop| &stop.id)) />
                    </div>
                </Show>
            </Show>
        </section>
    }
}

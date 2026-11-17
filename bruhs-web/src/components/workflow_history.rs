//! The workflow history on Agent workflows: every workflow, one at a time,
//! with Previous (older) and Next (newer), as on desktop. Which workflows
//! count, their order, the wording and the paging come from
//! `bruhs_models::logic::workflow_history`. Browsing changes no record.
//!
//! The workflow on show carries its controls, as on desktop, whichever
//! workflow that is: Run next step, Run in background and Pause while its
//! job runs, Resume while it is paused, and Undo transaction while a
//! completed step can still be undone (`workflow_history::controls`). They
//! change records, so they wait for the workflow store like the page's other
//! controls; Previous and Next never do.

use bruhs_models::logic::workflow_history::{
    self, BACKGROUND_REFUSED, NEXT_WORKFLOW, NEXT_WORKFLOW_LABEL, NO_WORKFLOW_YET, PAUSE,
    PAUSE_LABEL, PREVIOUS_WORKFLOW, PREVIOUS_WORKFLOW_LABEL, RESUME, RESUME_LABEL, RESUME_REFUSED,
    RUN_IN_BACKGROUND, RUN_NEXT_STEP, UNDO,
};
use bruhs_models::AutomationState;
use leptos::html;
use leptos::prelude::*;
use wasm_bindgen::JsCast;

use crate::state::{AutomationStateSignal, ReviewedWorkflowSignal};
use crate::ui::{button_class, ButtonSize, ButtonVariant};
use crate::workflow_executor::WorkflowExecutor;

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
    /// Its job's progress, such as "Running, next checkpoint 2".
    job: Option<String>,
    /// The job running it: Run next step, Run in background and Pause.
    running: Option<String>,
    /// Whether Run in background is on already.
    in_background: bool,
    /// The job paused on it: Resume.
    paused: Option<String>,
    /// Undo transaction.
    undo: bool,
}

impl Stop {
    fn has_controls(&self) -> bool {
        self.running.is_some() || self.paused.is_some() || self.undo
    }
}

fn stop(state: &AutomationState, page: workflow_history::Page<'_>) -> Stop {
    let controls = workflow_history::controls(state, page.workflow);
    Stop {
        id: page.workflow.id.clone(),
        title: page.workflow.title.clone(),
        status: page.status(),
        label: page.label(),
        previous: page.previous.map(str::to_owned),
        next: page.next.map(str::to_owned),
        job: controls.job_status(),
        running: controls.running.map(|job| job.id.clone()),
        in_background: controls.in_background(),
        paused: controls.paused.map(|job| job.id.clone()),
        undo: controls.undo,
    }
}

/// One of the workflow's controls.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Control {
    RunNextStep,
    RunInBackground,
    Pause,
    Resume,
    Undo,
}

impl Control {
    /// Its `data-testid`, desktop's debug selector without "desktop-".
    const fn test_id(self) -> &'static str {
        match self {
            Self::RunNextStep => "run-workflow-step",
            Self::RunInBackground => "enable-workflow-background",
            Self::Pause => "pause-workflow",
            Self::Resume => "resume-workflow",
            Self::Undo => "undo-workflow",
        }
    }

    /// Where focus goes once a press has saved and its button has gone or
    /// been disabled: what the press led to, then Previous or Next, so the
    /// keyboard stays in the history.
    fn focus_after(self) -> Vec<&'static str> {
        let led_to: &[Self] = match self {
            Self::RunNextStep => &[Self::RunNextStep, Self::Undo],
            Self::RunInBackground => &[Self::Pause, Self::RunNextStep, Self::Undo],
            Self::Pause => &[Self::Resume, Self::Undo],
            Self::Resume => &[Self::Pause, Self::RunNextStep, Self::Undo],
            Self::Undo => &[],
        };
        led_to
            .iter()
            .map(|control| control.test_id())
            .chain(["previous-workflow", "next-workflow"])
            .collect()
    }
}

#[component]
pub fn WorkflowHistory(
    /// True while the controls must wait: the workflow store or the
    /// executor is busy or not ready (the page's lock).
    #[prop(into)]
    locked: Signal<bool>,
    /// Where a refusal is reported (the page's alert).
    error: RwSignal<Option<String>>,
) -> impl IntoView {
    let AutomationStateSignal(automation) = expect_context();
    let ReviewedWorkflowSignal(selected) = expect_context();
    let executor = expect_context::<WorkflowExecutor>();
    let current = Memo::new(move |_| {
        automation.with(|state| {
            selected.with(|selected| {
                workflow_history::page(state, selected.as_deref()).map(|page| stop(state, page))
            })
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
    let shows =
        move |has: fn(&Stop) -> bool| move || current.with(|stop| stop.as_ref().is_some_and(has));
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

    // A pressed control is disabled while its change saves, and may be gone
    // after (Pause becomes Resume), which drops focus to the page. Once the
    // store settles, put focus on what the press led to.
    let panel = NodeRef::<html::Div>::new();
    let refocus = RwSignal::new(None::<Control>);
    Effect::new(move |_| {
        if refocus.get().is_none() || locked.get() {
            return;
        }
        current.track();
        request_animation_frame(move || {
            let Some(pressed) = refocus.get_untracked() else {
                return;
            };
            // A background run saved one step and started the next; wait on.
            if locked.get_untracked() {
                return;
            }
            refocus.set(None);
            let document = document();
            let lost = document.active_element().is_none_or(|active| {
                document
                    .body()
                    .is_some_and(|body| body.is_same_node(Some(active.unchecked_ref())))
                    || active.matches(":disabled").unwrap_or(false)
            });
            let Some(panel) = panel.get_untracked() else {
                return;
            };
            if !lost {
                return;
            }
            for id in pressed.focus_after() {
                let found = panel
                    .query_selector(&format!("[data-testid=\"{id}\"]:not(:disabled)"))
                    .ok()
                    .flatten()
                    .and_then(|element| element.dyn_into::<web_sys::HtmlElement>().ok());
                if let Some(element) = found {
                    let _ = element.focus();
                    return;
                }
            }
        });
    });

    // The paths the page's controls always used: the executor runs a step or
    // an undo in one transaction with its records; Run in background, Pause
    // and Resume change the workflow store. Ids are read at the press, so a
    // control never acts on a workflow other than the one on show.
    let press = move |control: Control| {
        let Some(shown) = current.get_untracked() else {
            return;
        };
        error.set(None);
        refocus.set(Some(control));
        let now = crate::clock::now_ms;
        let refusal = match control {
            Control::RunNextStep => shown
                .running
                .and_then(|job| executor.run(job).err())
                .map(|refused| refused.to_string()),
            Control::RunInBackground => shown
                .running
                .and_then(|job| automation.try_update(|state| state.enable_background(&job)))
                .is_some_and(|enabled| !enabled)
                .then(|| BACKGROUND_REFUSED.to_owned()),
            Control::Pause => {
                if let Some(job) = shown.running {
                    // Refused only when the job stopped running meanwhile,
                    // which the panel already shows; desktop says nothing either.
                    let _ = automation.try_update(|state| state.pause_job(&job, now()));
                }
                None
            }
            Control::Resume => shown
                .paused
                .and_then(|job| automation.try_update(|state| state.resume_job(&job, now())))
                .is_some_and(|resumed| !resumed)
                .then(|| RESUME_REFUSED.to_owned()),
            Control::Undo => executor
                .rollback(shown.id)
                .err()
                .map(|refused| refused.to_string()),
        };
        if refusal.is_some() {
            error.set(refusal);
        }
    };
    let control_button = move |control: Control,
                               variant: ButtonVariant,
                               name: &'static str,
                               text: &'static str,
                               disabled: Signal<bool>| {
        view! {
            <button
                type="button"
                class=button_class(variant, ButtonSize::Small, "")
                aria-label=name
                data-testid=control.test_id()
                disabled=move || disabled.get()
                on:click=move |_| press(control)
            >
                {text}
            </button>
        }
    };
    let in_background = Signal::derive(move || {
        current.with(|stop| stop.as_ref().is_some_and(|stop| stop.in_background))
    });
    let never = Signal::stored(false);

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
                    <div node_ref=panel role="group" aria-label=text(|stop| &stop.label) data-testid="workflow-history" class="min-w-0">
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
                        // The job's progress, read out as a step runs. Always in
                        // the page, so a screen reader hears its first line too.
                        <p aria-live="polite" class="mt-3 text-body-sm text-rambutan-400 empty:mt-0" data-testid="workflow-job-status">
                            {move || current.with(|stop| stop.as_ref().and_then(|stop| stop.job.clone()).unwrap_or_default())}
                        </p>
                        <Show when=shows(Stop::has_controls)>
                            // Changes records: waits for the store like the
                            // page's other controls (desktop's order and kinds).
                            <fieldset disabled=move || locked.get() class="m-0 mt-3 min-w-0 border-0 p-0" data-testid="workflow-controls">
                                <div class="flex flex-wrap gap-2">
                                    <Show when=shows(|stop| stop.running.is_some())>
                                        {control_button(Control::RunNextStep, ButtonVariant::Outline, RUN_NEXT_STEP, RUN_NEXT_STEP, never)}
                                        {control_button(Control::RunInBackground, ButtonVariant::Outline, RUN_IN_BACKGROUND, RUN_IN_BACKGROUND, in_background)}
                                        {control_button(Control::Pause, ButtonVariant::Ghost, PAUSE_LABEL, PAUSE, never)}
                                    </Show>
                                    <Show when=shows(|stop| stop.paused.is_some())>
                                        {control_button(Control::Resume, ButtonVariant::Outline, RESUME_LABEL, RESUME, never)}
                                    </Show>
                                    <Show when=shows(|stop| stop.undo)>
                                        {control_button(Control::Undo, ButtonVariant::Destructive, UNDO, UNDO, never)}
                                    </Show>
                                </div>
                            </fieldset>
                        </Show>
                        <super::workflow_payload::WorkflowReview workflow_id=Signal::derive(text(|stop| &stop.id)) />
                    </div>
                </Show>
            </Show>
        </section>
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bruhs_models::{AgentAccess, AgentDomain, WorkflowOperation};

    fn task(title: &str) -> WorkflowOperation {
        WorkflowOperation::CreateTask {
            title: title.into(),
        }
    }

    /// An older two-step workflow with one step done and a newer one still
    /// running: the history shows the newest by default.
    fn two_running() -> (AutomationState, String, String, String) {
        let mut state = AutomationState::default();
        state
            .permissions
            .set(AgentDomain::Tasks, AgentAccess::Write, true);
        let older = state.create_workflow("Older plan", vec![task("a"), task("b")], 1);
        state.preview(&older, 1).unwrap();
        let older_job = state.start_job(&older, 1).unwrap();
        assert!(state.complete_step(&older_job, "task-1", 2));
        let newer = state.create_workflow("Newer plan", vec![task("c")], 3);
        state.preview(&newer, 3).unwrap();
        state.start_job(&newer, 3).unwrap();
        (state, older, older_job, newer)
    }

    fn shown(state: &AutomationState, selected: Option<&str>) -> Stop {
        stop(state, workflow_history::page(state, selected).unwrap())
    }

    #[test]
    fn an_older_workflow_on_show_carries_its_own_job_and_undo() {
        let (state, older, older_job, newer) = two_running();
        let newest = shown(&state, None);
        assert_eq!(newest.id, newer);
        assert_ne!(newest.running.as_deref(), Some(older_job.as_str()));
        assert!(!newest.undo);

        let older_stop = shown(&state, Some(&older));
        assert_eq!(older_stop.running.as_deref(), Some(older_job.as_str()));
        assert_eq!(older_stop.paused, None);
        assert!(older_stop.undo && !older_stop.in_background && older_stop.has_controls());
        assert_eq!(
            older_stop.job.as_deref(),
            Some("Running, next checkpoint 2")
        );
        assert_eq!(older_stop.status, "Running, workflow 1 of 2");
    }

    #[test]
    fn a_paused_older_workflow_offers_resume_and_a_finished_one_only_undo() {
        let (mut state, older, older_job, _) = two_running();
        assert!(state.pause_job(&older_job, 4));
        let paused = shown(&state, Some(&older));
        assert_eq!(
            (paused.running.as_deref(), paused.paused.as_deref()),
            (None, Some(older_job.as_str()))
        );
        assert_eq!(
            paused.job.as_deref(),
            Some("Paused, resumes at checkpoint 2")
        );

        assert!(state.resume_job(&older_job, 5));
        assert!(state.complete_step(&older_job, "task-2", 6));
        let finished = shown(&state, Some(&older));
        assert_eq!(
            (finished.running, finished.paused, finished.job),
            (None, None, None)
        );
        assert!(finished.undo);

        assert!(state.mark_rolled_back(&older, 7));
        assert!(!shown(&state, Some(&older)).has_controls());
    }

    #[test]
    fn focus_follows_what_a_press_led_to_then_stays_in_the_history() {
        assert_eq!(
            Control::Pause.focus_after(),
            [
                "resume-workflow",
                "undo-workflow",
                "previous-workflow",
                "next-workflow"
            ]
        );
        assert_eq!(Control::Resume.focus_after()[0], Control::Pause.test_id());
        assert_eq!(
            Control::RunNextStep.focus_after()[..2],
            ["run-workflow-step", "undo-workflow"]
        );
        assert_eq!(
            Control::Undo.focus_after(),
            ["previous-workflow", "next-workflow"]
        );
    }
}

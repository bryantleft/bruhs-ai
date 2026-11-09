use bruhs_models::{
    focus_workflow_operations as focus_operations, AgentAccess, AgentDomain, BackgroundJobStatus,
    ProactiveSuggestion, WorkflowOperation, WorkflowPreview, WorkflowStatus,
};
use leptos::prelude::*;

use crate::state::{
    AutomationStateSignal, CalendarEventsSignal, CapturesSignal, InboxConversationsSignal,
    KnowledgeDocumentsSignal, LifeContextSignal, WorkItemsSignal,
};
use crate::ui::{Button, ButtonSize, ButtonVariant};

fn status_label(status: WorkflowStatus) -> &'static str {
    match status {
        WorkflowStatus::Draft => "Draft",
        WorkflowStatus::Previewed => "Previewed",
        WorkflowStatus::Declined => "Declined",
        WorkflowStatus::Running => "Running",
        WorkflowStatus::Paused => "Paused",
        WorkflowStatus::Completed => "Completed",
        WorkflowStatus::Failed => "Failed",
        WorkflowStatus::RolledBack => "Rolled back",
    }
}

#[component]
pub fn AutomationsPage() -> impl IntoView {
    let AutomationStateSignal(automation) = expect_context::<AutomationStateSignal>();
    let WorkItemsSignal(work_items) = expect_context::<WorkItemsSignal>();
    let CalendarEventsSignal(events) = expect_context::<CalendarEventsSignal>();
    let CapturesSignal(captures) = expect_context::<CapturesSignal>();
    let KnowledgeDocumentsSignal(documents) = expect_context::<KnowledgeDocumentsSignal>();
    let InboxConversationsSignal(conversations) = expect_context::<InboxConversationsSignal>();
    let LifeContextSignal(life) = expect_context::<LifeContextSignal>();
    let threads = expect_context::<crate::state::ThreadsSignal>().0;
    let meeting_notes =
        expect_context::<crate::workflow_executor::WorkflowExecutor>().meeting_notes;

    let now_ms = crate::clock::now_ms();
    Effect::new(move |_| {
        if !automation.ready() {
            return;
        }
        automation.update(|state| {
        if state
            .suggestions
            .iter()
            .all(|suggestion| suggestion.id != "protect-focus")
        {
            let unfinished = work_items
                .with_untracked(|items| items.iter().filter(|item| !item.is_completed()).count());
            let conflicts = events.with_untracked(|evs| {
                evs.iter()
                    .filter(|event| event.end_at_ms >= bruhs_models::EpochMs::saturating_from_i64(now_ms).as_f64())
                    .count()
            });
            state.suggestions.push(ProactiveSuggestion {
                id: "protect-focus".into(),
                title: "Protect a focus block".into(),
                explanation:
                    "A task, calendar block, and local note can be created as one reversible transaction."
                        .into(),
                evidence: format!(
                    "{unfinished} unfinished tasks and {conflicts} upcoming calendar entries are currently stored locally."
                ),
                domain: AgentDomain::Tasks,
                operations: focus_operations(now_ms),
                dismissed: false,
                shown_count: 0,
                last_shown_at_ms: None,
            });
        }
        });
    });

    let active_workflow = RwSignal::new(None::<String>);
    let preview = RwSignal::new(None::<WorkflowPreview>);
    let execution_error = RwSignal::new(None::<String>);

    let make_preview = move |operations: Vec<WorkflowOperation>, title: &str| {
        let now = crate::clock::now_ms();
        let mut workflow_id = String::new();
        let mut next_preview = None;
        automation.update(|state| {
            workflow_id = state.create_workflow(title, operations, now);
            next_preview = state.preview(&workflow_id, now);
        });
        active_workflow.set(Some(workflow_id));
        preview.set(next_preview);
    };

    let recheck_preview = move |_| {
        let Some(id) = active_workflow.get_untracked() else {
            return;
        };
        let next = automation.try_update(|state| state.preview(&id, crate::clock::now_ms()));
        preview.set(next.flatten());
    };

    let start_job = move |_| {
        let Some(id) = active_workflow.get_untracked() else {
            return;
        };
        automation.update(|state| {
            let _ = state.start_job(&id, crate::clock::now_ms());
        });
    };

    let executor = expect_context::<crate::workflow_executor::WorkflowExecutor>();
    let run_next_step = move |job_id: String| {
        execution_error.set(None);
        if let Err(error) = executor.run(job_id) {
            execution_error.set(Some(error.to_string()));
        }
    };
    let rollback = move |workflow_id: String| {
        execution_error.set(None);
        if let Err(error) = executor.rollback(workflow_id) {
            execution_error.set(Some(error.to_string()));
        }
    };

    view! {
        <section class="flex min-h-0 flex-1 flex-col bg-longan-950" aria-labelledby="automations-title">
            <header class="shrink-0 border-b border-longan-800/50 px-4 py-3 sm:px-6">
                <h1 id="automations-title" class="text-h3 text-rambutan-100">"Agent workflows"</h1>
                <p class="mt-0.5 text-body-sm text-rambutan-500">"Preview changes, grant only what is needed, and keep every action reversible."</p>
            </header>
            <super::metadata_status::MetadataStatus status=automation.status retry=Callback::new(move |_| automation.retry()) recovery=Signal::derive(move || automation.recovery_json()) />
            <Show when=move || !executor.ready.get()>
                <p role="status" class="px-4 py-2 text-body-sm text-rambutan-400">"Loading local workflow records…"</p>
                {move || executor.load_error.get().map(|message| view! {
                    <div class="px-4 py-2"><p role="alert">{message}</p><button type="button" class="py-2 text-label-sm text-persimmon-300" on:click=move |_| executor.load()>"Retry loading records"</button></div>
                })}
            </Show>
            <div class="flex-1 overflow-y-auto px-4 py-4 sm:px-6 sm:py-5">
                <fieldset disabled=move || !automation.editable() || matches!(automation.status.get(), crate::durable_metadata::SaveState::Saving) || !executor.ready.get() class="m-0 min-w-0 border-0 p-0">
                {move || execution_error.get().map(|message| view! {
                    <p role="alert" class="mx-auto mb-3 max-w-6xl rounded-grape bg-longan-900 px-3 py-2 text-body-sm text-lychee-300">{message}</p>
                })}
                <div class="mx-auto grid max-w-6xl gap-4 lg:grid-cols-[minmax(0,1fr)_22rem]">
                    <div class="space-y-4">
                        <section class="rounded-grape bg-longan-900/70 p-4 inset-ring inset-ring-longan-700/50" aria-labelledby="suggestions-title">
                            <div class="flex items-center justify-between gap-3">
                                <div>
                                    <h2 id="suggestions-title" class="text-h4 text-rambutan-100">"Local suggestions"</h2>
                                    <p class="text-body-sm text-rambutan-500">"Evidence is shown before any proposal. Suggestions appear at most three times, no more than daily."</p>
                                </div>
                                <Button class="whitespace-nowrap" variant=ButtonVariant::Ghost size=ButtonSize::Small on_click=Callback::new(move |_| make_preview(focus_operations(crate::clock::now_ms()), "Protect focus"))>
                                    "New dry run"
                                </Button>
                            </div>
                            <div class="mt-3 space-y-2">
                                {move || automation.with(|state| state.surfaceable_suggestions(crate::clock::now_ms()).into_iter().map(|suggestion| {
                                    let id = suggestion.id.clone();
                                    let id_for_preview = id.clone();
                                    let title = suggestion.title.clone();
                                    let workflow_title = title.clone();
                                    let operations = suggestion.operations.clone();
                                    let preview_label = format!("Preview {} changes", operations.len());
                                    view! {
                                        <article class="rounded-grape bg-longan-950/70 p-3 inset-ring inset-ring-longan-700/40">
                                            <div class="flex items-start justify-between gap-3">
                                                <div class="min-w-0">
                                                    <h3 class="text-label text-rambutan-200">{title}</h3>
                                                    <p class="mt-1 text-body-sm text-rambutan-400">{suggestion.explanation.clone()}</p>
                                                    <p class="mt-2 text-label-sm text-blueberry-300">{format!("Evidence · {}", suggestion.evidence)}</p>
                                                </div>
                                                <button type="button" class="shrink-0 text-label-sm text-rambutan-500 hover:text-rambutan-200 cursor-pointer" aria-label="Dismiss suggestion" on:click=move |_| automation.update(|state| { let _ = state.dismiss_suggestion(&id, crate::clock::now_ms()); })>"Dismiss"</button>
                                            </div>
                                            <button type="button" class="mt-3 rounded-grape bg-persimmon-500 px-3 py-1.5 text-label-sm text-longan-950 hover:bg-persimmon-400 cursor-pointer" on:click=move |_| {
                                                automation.update(|state| { let _ = state.mark_suggestion_surfaced(&id_for_preview, crate::clock::now_ms()); });
                                                make_preview(operations.clone(), &workflow_title);
                                            }>{preview_label}</button>
                                        </article>
                                    }
                                }).collect_view())}
                            </div>
                        </section>

                        <Show when=move || preview.get().is_some() && automation.with(|state| state.workflows.iter().any(|workflow| Some(&workflow.id) == active_workflow.get().as_ref() && workflow.status == WorkflowStatus::Previewed)) fallback=|| ()>
                            <section class="rounded-grape bg-longan-900/70 p-4 inset-ring inset-ring-longan-700/50" aria-labelledby="dry-run-title">
                                <div class="flex items-center justify-between gap-3">
                                    <div>
                                        <h2 id="dry-run-title" class="text-h4 text-rambutan-100">"Dry-run preview"</h2>
                                        <p class="text-body-sm text-rambutan-500">"Nothing below has run yet."</p>
                                    </div>
                                    <Button variant=ButtonVariant::Ghost size=ButtonSize::Small on_click=Callback::new(recheck_preview)>"Recheck grants"</Button>
                                </div>
                                {move || automation.with(|state| state.workflows.iter().find(|workflow| Some(&workflow.id) == active_workflow.get().as_ref()).map(|workflow| {
                                    let operations = workflow.steps.iter().map(|step| step.operation.clone()).collect();
                                    view! { <super::workflow_payload::WorkflowPayload operations=operations /> }
                                }))}
                                <Show when=move || preview.get().is_some_and(|preview| !preview.allowed) fallback=|| ()>
                                    <p role="alert" class="mt-3 text-body-sm text-durian-300">{move || preview.get().map(|preview| format!("Grant Write for: {}.", preview.blocked_domains.iter().map(|domain| domain.label()).collect::<Vec<_>>().join(", "))).unwrap_or_default()}</p>
                                </Show>
                                <div class="mt-3">
                                    <Button variant=ButtonVariant::Primary size=ButtonSize::Small disabled=Signal::derive(move || !preview.get().is_some_and(|preview| preview.allowed)) on_click=Callback::new(start_job)>
                                        "Start transaction"
                                    </Button>
                                </div>
                            </section>
                        </Show>

                        <section class="rounded-grape bg-longan-900/70 p-4 inset-ring inset-ring-longan-700/50" aria-labelledby="jobs-title">
                            <h2 id="jobs-title" class="text-h4 text-rambutan-100">"Background jobs"</h2>
                            <p class="text-body-sm text-rambutan-500">"Each record and checkpoint commit together. Background runs continue across pages while the app is open and resume after reload."</p>
                            <div class="mt-3 space-y-2">
                                {move || automation.with(|state| state.jobs.iter().rev().take(8).map(|job| {
                                    let run_id = job.id.clone();
                                    let background_id = job.id.clone();
                                    let auto_run = job.auto_run;
                                    let pause_id = job.id.clone();
                                    let resume_id = job.id.clone();
                                    let status = job.status;
                                    let next_step = job.next_step;
                                    let workflow = state.workflows.iter().find(|workflow| workflow.id == job.workflow_id);
                                    let total = workflow.map_or(0, |workflow| workflow.steps.len());
                                    let review_id = job.workflow_id.clone();
                                    let actions = if status == BackgroundJobStatus::Running && next_step < total {
                                        view! {
                                            <button type="button" class="min-h-11 min-w-11 text-button text-persimmon-300 hover:text-persimmon-200 cursor-pointer" on:click=move |_| run_next_step(run_id.clone())>"Run next step"</button>
                                            <button type="button" disabled=auto_run class="min-h-11 min-w-11 text-button text-persimmon-300 disabled:opacity-40 cursor-pointer" on:click=move |_| automation.update(|state| { let _ = state.enable_background(&background_id); })>"Run in background"</button>
                                            <button type="button" class="min-h-11 min-w-11 text-button text-rambutan-400 hover:text-rambutan-200 cursor-pointer" on:click=move |_| automation.update(|state| { let _ = state.pause_job(&pause_id, crate::clock::now_ms()); })>"Pause"</button>
                                        }.into_any()
                                    } else if status == BackgroundJobStatus::Paused {
                                        view! {
                                            <button type="button" class="min-h-11 min-w-11 text-button text-persimmon-300 hover:text-persimmon-200 cursor-pointer" on:click=move |_| automation.update(|state| { let _ = state.resume_job(&resume_id, crate::clock::now_ms()); })>"Resume"</button>
                                        }.into_any()
                                    } else {
                                        view! { <span class="text-label-sm text-pandan-300">"Checkpointed"</span> }.into_any()
                                    };
                                    view! {
                                        <div class="flex flex-wrap items-center justify-between gap-3 rounded-grape bg-longan-950/60 px-3 py-2">
                                            <div>
                                                <p class="text-label text-rambutan-200">{format!("{} of {} steps", next_step, total)}</p>
                                                <p class="text-label-sm text-rambutan-500">{format!("{:?}", status)}</p>
                                            </div>
                                            <div class="flex items-center gap-2">
                                                {actions}
                                            </div>
                                            <super::workflow_payload::WorkflowReview workflow_id=review_id />
                                        </div>
                                    }
                                }).collect_view())}
                            </div>
                        </section>

                        <section class="rounded-grape bg-longan-900/70 p-4 inset-ring inset-ring-longan-700/50" aria-labelledby="audit-title">
                            <h2 id="audit-title" class="text-h4 text-rambutan-100">"Audit timeline"</h2>
                            <div class="mt-3 space-y-2">
                                {move || automation.with(|state| state.audit.iter().rev().take(20).cloned().map(|event| view! {
                                    <div class="border-l border-blueberry-500/30 pl-3">
                                        <p class="text-label-sm text-blueberry-300">{format!("{:?}", event.kind)}</p>
                                        <p class="text-body-sm text-rambutan-300">{event.message}</p>
                                    </div>
                                }).collect_view())}
                            </div>
                            {move || automation.with(|state| state.workflows.iter().rev().filter(|workflow| workflow.can_rollback()).take(3).map(|workflow| {
                                let id = workflow.id.clone();
                                let title = workflow.title.clone();
                                view! { <button type="button" class="mt-3 mr-2 text-button text-lychee-300 hover:text-lychee-200 cursor-pointer" on:click=move |_| rollback(id.clone())>{format!("Undo {title}")}</button> }
                            }).collect_view())}
                        </section>
                    </div>

                    <aside class="space-y-4">
                        <section class="rounded-grape bg-longan-900/70 p-4 inset-ring inset-ring-longan-700/50" aria-labelledby="capabilities-title">
                            <h2 id="capabilities-title" class="text-h4 text-rambutan-100">"Capability grants"</h2>
                            <p class="text-body-sm text-rambutan-500">"Read and Write are independent. Finance and Health start fully off."</p>
                            <div class="mt-3 space-y-2">
                                {AgentDomain::ALL.into_iter().map(|domain| view! {
                                    <div class="flex items-center justify-between gap-3">
                                        <span class="text-label-sm text-rambutan-300">{domain.label()}</span>
                                        <div class="flex gap-1">
                                            {[AgentAccess::Read, AgentAccess::Write].into_iter().map(|access| view! {
                                                <button
                                                    type="button"
                                                    aria-label=format!("Toggle {} {} access", domain.label(), match access { AgentAccess::Read => "Read", AgentAccess::Write => "Write" })
                                                    aria-pressed=move || automation.with(|state| state.permissions.allows(domain, access)).to_string()
                                                    class=move || if automation.with(|state| state.permissions.allows(domain, access)) { "rounded-grape bg-blueberry-500/15 px-2 py-1 text-label-sm text-blueberry-200 inset-ring inset-ring-blueberry-400/20 cursor-pointer" } else { "rounded-grape bg-longan-950/60 px-2 py-1 text-label-sm text-rambutan-400 inset-ring inset-ring-longan-700/40 cursor-pointer hover:text-rambutan-300" }
                                                    on:click=move |_| automation.update(|state| {
                                                        let allowed = state.permissions.allows(domain, access);
                                                        state.permissions.set(domain, access, !allowed);
                                                    })
                                                >{match access { AgentAccess::Read => "Read", AgentAccess::Write => "Write" }}</button>
                                            }).collect_view()}
                                        </div>
                                    </div>
                                }).collect_view()}
                            </div>
                        </section>

                        <section class="rounded-grape bg-longan-900/70 p-4 inset-ring inset-ring-longan-700/50">
                            <p class="text-label-sm uppercase tracking-wide text-rambutan-400">"Safety contract"</p>
                            <p class="mt-2 text-body-sm text-rambutan-300">"Writes never inherit Read access. Every workflow requires a dry run. Undo reverts completed steps, including a partial run, only if their records are unchanged."</p>
                            <p class="mt-2 text-label-sm text-pandan-300">{move || automation.with(|state| format!("{} typed tools · {} workflows", bruhs_models::agent_tool_definitions().len(), state.workflows.len()))}</p>
                            <p class="mt-1 text-label-sm text-rambutan-400">{move || format!("{} local records in connected workflow domains", meeting_notes.with(Vec::len) + threads.with(Vec::len) + work_items.with(Vec::len) + events.with(Vec::len) + captures.with(Vec::len) + documents.with(Vec::len) + conversations.with(Vec::len) + life.with(|life| life.finance.transactions.len() + life.health.workouts.len()))}</p>
                        </section>

                        {move || automation.with(|state| state.workflows.last().cloned().map(|workflow| view! {
                            <section class="rounded-grape bg-longan-900/70 p-4 inset-ring inset-ring-longan-700/50">
                                <p class="text-label-sm uppercase tracking-wide text-rambutan-400">"Latest workflow"</p>
                                <p class="mt-1 text-label text-rambutan-200">{workflow.title}</p>
                                <p class="text-body-sm text-rambutan-500">{status_label(workflow.status)}</p>
                            </section>
                        }))}
                    </aside>
                </div>
                </fieldset>
            </div>
        </section>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn focus_preset_is_cross_domain_and_reversible_by_identity() {
        let operations = focus_operations(1_000);
        assert_eq!(operations.len(), 3);
        assert_eq!(operations[0].domain(), AgentDomain::Tasks);
        assert_eq!(operations[1].domain(), AgentDomain::Calendar);
        assert_eq!(operations[2].domain(), AgentDomain::Capture);
    }

    #[test]
    fn default_automation_state_keeps_sensitive_domains_off() {
        let state = bruhs_models::AutomationState::default();
        for domain in [AgentDomain::Finance, AgentDomain::Health] {
            assert!(!state.permissions.allows(domain, AgentAccess::Read));
            assert!(!state.permissions.allows(domain, AgentAccess::Write));
        }
    }
}

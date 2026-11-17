//! A bruh's own page: its face, name, job and status, Stop or Resume, and
//! tabs for its chat, memory, routines and settings.

use bruhs_models::crew::{status_line, BruhStatus, PAGE_FACE_PX};
use bruhs_models::reactive::ReactiveCommand;
use leptos::prelude::*;

use super::chat::BruhChat;
use super::data::use_crew_snapshot;
use super::face::BruhFace;
use super::settings::BruhSettingsTab;
use crate::components::reactive_automations::{local_time, AgentMemoryPanel, AutomationSetupPanel};
use crate::state::{AppView, SetActiveView};
use crate::ui::{Button, ButtonSize, ButtonVariant};

const TAB_ON: &str = "bruhs-control inline-flex min-h-control items-center rounded-seed bg-longan-700 px-3 text-label-sm text-rambutan-100 focus-visible:outline-2 focus-visible:outline-persimmon-400";
const TAB_OFF: &str = "bruhs-control inline-flex min-h-control items-center rounded-seed px-3 text-label-sm text-rambutan-500 transition-colors hover:text-rambutan-300 focus-visible:outline-2 focus-visible:outline-persimmon-400";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tab {
    Chat,
    Memory,
    Routines,
    Settings,
}

impl Tab {
    const ALL: [Tab; 4] = [Tab::Chat, Tab::Memory, Tab::Routines, Tab::Settings];

    fn label(self) -> &'static str {
        match self {
            Tab::Chat => "Chat",
            Tab::Memory => "Memory",
            Tab::Routines => "Routines",
            Tab::Settings => "Settings",
        }
    }
}

/// The job line under a bruh's name: the first line of its instructions.
fn job_line(instructions: &str) -> &str {
    instructions
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or_default()
}

#[component]
pub fn BruhPage(agent_id: String) -> impl IntoView {
    let data = use_crew_snapshot();
    let id = StoredValue::new(agent_id);
    let tab = RwSignal::new(Tab::Chat);
    let state = Memo::new(move |_| {
        data.snapshot.with(|snapshot| {
            snapshot.as_ref().map(|snapshot| {
                id.with_value(|id| {
                    snapshot
                        .bruhs
                        .iter()
                        .find(|bruh| bruh.agent_id == *id)
                        .cloned()
                })
            })
        })
    });
    let job = Memo::new(move |_| {
        data.snapshot.with(|snapshot| {
            snapshot
                .as_ref()
                .and_then(|snapshot| {
                    id.with_value(|id| snapshot.agents.iter().find(|agent| agent.id == *id))
                })
                .map(|agent| job_line(&agent.instructions).to_string())
                .unwrap_or_default()
        })
    });
    let set_view = use_context::<SetActiveView>().map(|set| set.0);
    let open_workflows = move |_| {
        if let Some(set_view) = set_view {
            set_view.set(AppView::Automations);
        }
    };

    view! {
        <section class="flex min-h-0 flex-1 flex-col bg-longan-950 text-rambutan-300" aria-labelledby="bruh-title">
            {move || match state.get() {
                None => view! {
                    <p role="status" class="p-6 text-body-sm text-rambutan-400">"Loading this bruh…"</p>
                }.into_any(),
                Some(None) => view! {
                    <div class="space-y-2 p-6">
                        <h1 id="bruh-title" class="text-h3 text-rambutan-100">"This bruh no longer exists."</h1>
                        <button type="button" class="bruhs-control text-label text-blueberry-300 underline underline-offset-2 hover:text-blueberry-200 focus-visible:outline-2 focus-visible:outline-persimmon-400" on:click=open_workflows>"Open Agent workflows"</button>
                    </div>
                }.into_any(),
                Some(Some(_)) => ().into_any(),
            }}
            <Show when=move || matches!(state.get(), Some(Some(_)))>
                {move || {
                    let look = Signal::derive(move || state.get().flatten().map(|state| state.look).unwrap_or_default());
                    let status = Signal::derive(move || state.get().flatten().map_or(BruhStatus::Asleep, |state| state.status));
                    let line = Signal::derive(move || state.get().flatten().map(|state| {
                        let wake = state.next_wake_ms.map(local_time);
                        status_line(state.status, wake.as_deref())
                    }).unwrap_or_default());
                    let paused = Signal::derive(move || status.get() == BruhStatus::Paused);
                    let disabled = Signal::derive(move || data.busy.get() || !data.crew_ready());
                    let stop_or_resume = move |_| {
                        let agent_id = id.get_value();
                        data.send.run(if paused.get_untracked() {
                            ReactiveCommand::ResumeBruh { agent_id }
                        } else {
                            ReactiveCommand::StopBruh { agent_id }
                        });
                    };
                    view! {
                        <header class="flex shrink-0 flex-wrap items-center justify-between gap-3 border-b border-longan-800/50 px-4 py-3 sm:px-6">
                            <div class="flex min-w-0 items-center gap-3">
                                <BruhFace size=f64::from(PAGE_FACE_PX) look=look status=status />
                                <div class="min-w-0">
                                    <h1 id="bruh-title" class="break-words text-h3 text-rambutan-100">{move || state.get().flatten().map(|state| state.name).unwrap_or_default()}</h1>
                                    <Show when=move || !job.with(String::is_empty)>
                                        <p class="truncate text-body-sm text-rambutan-400">{move || job.get()}</p>
                                    </Show>
                                    <p class="text-label-sm text-rambutan-500">{move || line.get()}</p>
                                </div>
                            </div>
                            <div class="flex min-w-0 flex-wrap items-center gap-2">
                                {move || data.service_reason().map(|reason| view! { <p class="min-w-0 max-w-xs text-label-sm text-rambutan-400">{reason}</p> })}
                                <Button variant=ButtonVariant::Outline size=ButtonSize::Small disabled=disabled on_click=Callback::new(stop_or_resume)>
                                    {move || if paused.get() { "Resume" } else { "Stop" }}
                                </Button>
                            </div>
                        </header>
                    }
                }}
                <div role="tablist" aria-label="Bruh sections" class="flex shrink-0 gap-1 overflow-x-auto border-b border-longan-800/50 px-4 py-1.5 sm:px-6">
                    {Tab::ALL.into_iter().map(|option| view! {
                        <button
                            type="button"
                            role="tab"
                            aria-selected=move || (tab.get() == option).to_string()
                            class=move || if tab.get() == option { TAB_ON } else { TAB_OFF }
                            on:click=move |_| tab.set(option)
                        >{option.label()}</button>
                    }).collect_view()}
                </div>
                <div role="tabpanel" aria-label=move || tab.get().label() class="flex min-h-0 flex-1 flex-col">
                    {move || {
                        let agent_id = id.get_value();
                        match tab.get() {
                            Tab::Chat => view! { <BruhChat agent_id=agent_id /> }.into_any(),
                            Tab::Memory => view! {
                                <div class="min-h-0 flex-1 overflow-y-auto">
                                    <div class="mx-auto w-full max-w-3xl min-w-0 p-4 sm:px-6">
                                        <AgentMemoryPanel agent_id=agent_id snapshot=data.snapshot send=data.send />
                                    </div>
                                </div>
                            }.into_any(),
                            Tab::Routines => view! {
                                <div class="min-h-0 flex-1 overflow-y-auto">
                                    <div class="mx-auto w-full max-w-3xl min-w-0 space-y-3 p-4 sm:px-6">
                                        {move || data.error.get().map(|message| view! { <p role="alert" class="rounded-grape bg-lychee-500/10 p-3 text-body-sm text-lychee-200">{message}</p> })}
                                        <AutomationSetupPanel snapshot=data.snapshot busy=data.busy error=data.error send=data.send on_chat=Callback::new(move |_| tab.set(Tab::Chat)) agent_filter=agent_id />
                                    </div>
                                </div>
                            }.into_any(),
                            Tab::Settings => view! {
                                <div class="min-h-0 flex-1 overflow-y-auto">
                                    <BruhSettingsTab agent_id=agent_id />
                                </div>
                            }.into_any(),
                        }
                    }}
                </div>
                {move || (tab.get() != Tab::Routines).then(|| data.error.get()).flatten().map(|message| view! {
                    <p role="alert" class="mx-4 mb-3 shrink-0 rounded-grape bg-lychee-500/10 p-3 text-body-sm text-lychee-200 sm:mx-6">{message}</p>
                })}
            </Show>
        </section>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_job_is_the_first_line_with_words() {
        assert_eq!(job_line("\n  Watch bills.\nPay nothing."), "Watch bills.");
        assert_eq!(job_line(""), "");
    }
}

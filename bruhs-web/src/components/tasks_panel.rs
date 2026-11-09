//! Tasks surface for the right panel: running and recent background shells
//! with their log tails. The panel shell owns the tab bar and close button.

use crate::api::dismiss_task;
use crate::ui::{Button, ButtonSize, ButtonVariant};
use bruhs_models::api::TaskStatus;
use bruhs_models::time::{format_relative_ms, RelativeTimeStyle};
use bruhs_models::EpochMs;
use leptos::prelude::*;

/// How long ago `at` was: "now", "3m", "2h", "40d".
fn relative_time(at: EpochMs) -> String {
    format_relative_ms(
        at,
        crate::state::now_epoch_ms(),
        RelativeTimeStyle::CompactDays,
    )
}

#[component]
pub fn TasksSurface() -> impl IntoView {
    let tasks = expect_context::<crate::state::BackgroundTasksSignal>().0;
    let set_tasks = expect_context::<crate::state::SetBackgroundTasksSignal>().0;

    // Track expanded task rows
    let (expanded_id, set_expanded_id) = signal(Option::<String>::None);

    let running_tasks = move || {
        tasks.with(|ts| {
            ts.iter()
                .filter(|t| t.status == TaskStatus::Running)
                .cloned()
                .collect::<Vec<_>>()
        })
    };

    let recent_tasks = move || {
        tasks.with(|ts| {
            ts.iter()
                .filter(|t| t.status != TaskStatus::Running)
                .cloned()
                .collect::<Vec<_>>()
        })
    };

    // `DELETE /api/tasks/{id}`; the row goes once the server has dropped the
    // task (or never had it), so a refused delete does not hide a live one.
    let on_dismiss = move |task_id: String| {
        wasm_bindgen_futures::spawn_local(async move {
            match dismiss_task(&task_id).await {
                Err(error) if !error.is_status(404) => {
                    log::warn!("could not dismiss task {task_id}: {error}");
                }
                _ => {
                    set_tasks.try_update(|ts| ts.retain(|t| t.id != task_id));
                }
            }
        });
    };

    let toggle_expand = move |id: String| {
        set_expanded_id.update(|curr| {
            if curr.as_deref() == Some(id.as_str()) {
                *curr = None;
            } else {
                *curr = Some(id);
            }
        });
    };

    view! {
        <div class="flex h-full flex-col text-rambutan-300">
            <div class="flex-1 overflow-y-auto px-3 py-3">
                {move || {
                    let running = running_tasks();
                    let recent = recent_tasks();

                    if running.is_empty() && recent.is_empty() {
                        view! {
                            <div class="flex items-center justify-center h-full text-label-sm text-rambutan-500 py-8">
                                "No background tasks"
                            </div>
                        }.into_any()
                    } else {
                        view! {
                            <div class="space-y-4">
                                // Running section
                                {if !running.is_empty() {
                                    view! {
                                        <div>
                                            <div class="px-3 py-1.5 text-label-sm text-rambutan-500">
                                                "Running"
                                            </div>
                                            <For
                                                each=move || running_tasks()
                                                key=|t| t.id.clone()
                                                children=move |task| {
                                                    let id = task.id.clone();
                                                    let id_for_expand = id.clone();
                                                    let label = task.label.clone();
                                                    let started_at = relative_time(task.started_at);
                                                    let log_tail = task.log_tail;
                                                    let is_expanded =
                                                        move || expanded_id.get().as_deref() == Some(id.as_str());
                                                    view! {
                                                        <div>
                                                            <button
                                                                on:click=move |_| toggle_expand(id_for_expand.clone())
                                                                class="w-full text-left rounded-grape px-3 py-2 hover:bg-longan-800/50 transition"
                                                            >
                                                                <div class="flex items-center gap-2">
                                                                    // Animated dots indicator
                                                                    <span class="inline-flex gap-0.5 shrink-0">
                                                                        <span class="size-1 rounded-orb bg-pandan-500"></span>
                                                                        <span class="size-1 rounded-orb bg-pandan-500"></span>
                                                                        <span class="size-1 rounded-orb bg-pandan-500"></span>
                                                                    </span>
                                                                    <span class="text-body-sm text-rambutan-200 truncate flex-1">{label}</span>
                                                                    <span class="text-label-sm text-rambutan-500 shrink-0">{started_at}</span>
                                                                </div>
                                                            </button>
                                                            {move || if is_expanded() {
                                                                let logs = log_tail.clone();
                                                                view! {
                                                                    <div class="mt-2 mx-3 rounded-grape bg-longan-950 p-3 inset-ring inset-ring-white/5 text-code-sm text-rambutan-400 max-h-32 overflow-y-auto whitespace-pre-wrap break-all">
                                                                        {if logs.is_empty() {
                                                                            "No output yet...".to_string()
                                                                        } else {
                                                                            logs
                                                                        }}
                                                                    </div>
                                                                }.into_any()
                                                            } else {
                                                                ().into_any()
                                                            }}
                                                        </div>
                                                    }
                                                }
                                            />
                                        </div>
                                    }.into_any()
                                } else {
                                    ().into_any()
                                }}

                                // Recent section
                                {if !recent.is_empty() {
                                    view! {
                                        <div>
                                            <div class="px-3 py-1.5 text-label-sm text-rambutan-500">
                                                "Recent"
                                            </div>
                                            <For
                                                each=move || recent_tasks()
                                                key=|t| t.id.clone()
                                                children=move |task| {
                                                    let id = task.id.clone();
                                                    let id_for_expand = id.clone();
                                                    let id_for_dismiss = id.clone();
                                                    let label = task.label.clone();
                                                    let ended_at = task.ended_at.map(relative_time).unwrap_or_default();
                                                    let exit_code = task.exit_code;
                                                    let log_tail = task.log_tail;
                                                    let success = exit_code == Some(0);
                                                    let is_expanded =
                                                        move || expanded_id.get().as_deref() == Some(id.as_str());
                                                    view! {
                                                        <div>
                                                            <div class="flex items-center rounded-grape px-3 py-2 hover:bg-longan-800/50 transition group">
                                                                <button
                                                                    on:click=move |_| toggle_expand(id_for_expand.clone())
                                                                    class="flex items-center gap-2 flex-1 text-left min-w-0"
                                                                >
                                                                    // Status indicator
                                                                    <span class=move || format!(
                                                                        "text-label-sm shrink-0 {}",
                                                                        if success { "text-pandan-400" } else { "text-lychee-400" }
                                                                    )>
                                                                        {if success { "✓" } else { "✗" }}
                                                                    </span>
                                                                    <span class="text-body-sm text-rambutan-200 truncate flex-1">{label}</span>
                                                                    <span class="text-label-sm text-rambutan-500 shrink-0">{ended_at}</span>
                                                                </button>
                                                                // Dismiss button
                                                                <div class="ml-2 opacity-0 group-hover:opacity-100">
                                                                    <Button
                                                                        variant=ButtonVariant::Ghost
                                                                        size=ButtonSize::IconDefault
                                                                        title="Dismiss".to_string()
                                                                        on_click={
                                                                            let id = id_for_dismiss;
                                                                            Callback::new(move |_| on_dismiss(id.clone()))
                                                                        }
                                                                    >
                                                                        "✕"
                                                                    </Button>
                                                                </div>
                                                            </div>
                                                            {move || if is_expanded() {
                                                                let logs = log_tail.clone();
                                                                view! {
                                                                    <div class="mt-2 mx-3 rounded-grape bg-longan-950 p-3 inset-ring inset-ring-white/5 text-code-sm text-rambutan-400 max-h-32 overflow-y-auto whitespace-pre-wrap break-all">
                                                                        {if logs.is_empty() {
                                                                            "No output.".to_string()
                                                                        } else {
                                                                            logs
                                                                        }}
                                                                    </div>
                                                                }.into_any()
                                                            } else {
                                                                ().into_any()
                                                            }}
                                                        </div>
                                                    }
                                                }
                                            />
                                        </div>
                                    }.into_any()
                                } else {
                                    ().into_any()
                                }}
                            </div>
                        }.into_any()
                    }
                }}
            </div>
        </div>
    }
}

//! The sidebar update pills, the "Update N providers?" confirmation and
//! the provider cards in Settings → Updates.
//!
//! - While a provider updates: "Updating Codex" / "Updating 2 providers".
//! - After it finishes (this page only): "Codex updated: v0.167.0" for three
//!   seconds, or "Codex v0.167.0 update failed" / "Codex still needs an
//!   update" until dismissed.
//! - Otherwise, while something is behind: Update, which confirms first.
//!   When only bruhs is behind (a source checkout bruhs never pulls), it
//!   opens Settings → Updates with the command to copy.

use bruhs_models::agent::AgentProvider;
use bruhs_models::time::{format_relative_ms, RelativeTimeStyle};
use bruhs_models::updates::{
    confirm_title, provider_cli_label, running_label, sidebar_label, AppUpdateState,
    ProviderUpdate, ProviderUpdateState, RunOutcome, SUCCESS_VISIBLE_MS,
};
use leptos::prelude::*;

use crate::components::settings_page::SettingsRow;
use crate::state::{AppView, SetActiveView};
use crate::ui::{
    button_class, Button, ButtonSize, ButtonVariant, CopyButton, Dialog, Icon, Switch,
};
use crate::updates::{now, SidebarRuns, Updates};

const PILL: &str = "flex h-8 w-full min-w-0 items-center gap-2 rounded-grape px-2 text-left text-label transition-colors";
const PILL_ACTION: &str = "bg-persimmon-500/10 text-persimmon-200 inset-ring inset-ring-persimmon-500/25 hover:bg-persimmon-500/15 hover:text-persimmon-100";
const PILL_INFO: &str =
    "bg-longan-800/60 text-rambutan-200 inset-ring inset-ring-white/10 hover:bg-longan-800";
const PILL_FAILED: &str =
    "bg-lychee-500/10 text-lychee-200 inset-ring inset-ring-lychee-500/25 hover:bg-lychee-500/15";

/// Above the sidebar's footer icons.
#[component]
pub fn SidebarUpdateButton() -> impl IntoView {
    let updates = Updates::get();
    let SetActiveView(set_active_view) = expect_context::<SetActiveView>();
    let open_settings = move || {
        updates.reveal_in_settings.set(true);
        set_active_view.set(AppView::Settings);
    };
    let runs = Memo::new(move |_| updates.sidebar_runs(now()));
    // A success shows for three seconds, then leaves on its own.
    Effect::new(move |_| {
        if let SidebarRuns::Finished(run) = runs.get() {
            if run.outcome == Some(RunOutcome::Updated) {
                let left = run
                    .finished_at
                    .map_or(0, |at| SUCCESS_VISIBLE_MS - now().millis_since(at))
                    .clamp(0, SUCCESS_VISIBLE_MS);
                let id = run.id;
                set_timeout(
                    move || updates.dismiss(id),
                    std::time::Duration::from_millis(u64::try_from(left).unwrap_or(0)),
                );
            }
        }
    });
    let report = Memo::new(move |_| updates.report.get().filter(|r| r.has_updates()));

    view! {
        <div class="px-2 pt-1 empty:hidden">
            {move || match runs.get() {
                SidebarRuns::Active(active) => {
                    let title = active
                        .iter()
                        .map(|r| format!("{}: {}", provider_cli_label(r.provider), r.phase_line()))
                        .collect::<Vec<_>>()
                        .join("\n");
                    view! {
                        <button
                            type="button"
                            data-testid="sidebar-update"
                            title=title
                            aria-busy="true"
                            class=format!("{PILL} {PILL_INFO}")
                            on:click=move |_| open_settings()
                        >
                            <Icon name="loader" size=14 class="shrink-0 motion-safe:animate-spin" />
                            <span class="min-w-0 flex-1 truncate">{running_label(&active).unwrap_or_default()}</span>
                        </button>
                    }
                    .into_any()
                }
                SidebarRuns::Finished(run) => {
                    let failed = run.needs_attention();
                    let id = run.id;
                    let title = if failed {
                        run.message.clone().unwrap_or_default()
                    } else {
                        "New sessions will use the updated provider.".to_string()
                    };
                    let tone = if failed { PILL_FAILED } else { PILL_INFO };
                    view! {
                        <div
                            data-testid="sidebar-update"
                            class=format!("relative overflow-hidden pr-1 {PILL} {tone}")
                            title=title
                        >
                            <button
                                type="button"
                                class="flex min-w-0 flex-1 items-center gap-2 text-left"
                                on:click=move |_| open_settings()
                            >
                                {if failed {
                                    view! { <Icon name="alert-triangle" size=14 class="shrink-0" /> }.into_any()
                                } else {
                                    view! { <Icon name="check" size=14 class="shrink-0 text-pandan-400" /> }.into_any()
                                }}
                                <span class="min-w-0 flex-1 truncate">{run.pill_label()}</span>
                            </button>
                            {failed.then(|| view! {
                                <button
                                    type="button"
                                    aria-label="Dismiss"
                                    title="Dismiss until provider status changes"
                                    class="inline-flex size-6 shrink-0 items-center justify-center rounded-seed hover:bg-longan-800/60"
                                    on:click=move |_| updates.dismiss(id)
                                >
                                    <Icon name="x" size=12 />
                                </button>
                            })}
                            {(!failed).then(|| view! {
                                <span
                                    aria-hidden="true"
                                    class="bruhs-update-progress absolute bottom-0 left-0 h-0.5 w-full origin-left bg-pandan-400/60"
                                ></span>
                            })}
                        </div>
                    }
                    .into_any()
                }
                SidebarRuns::None => match report.get() {
                    Some(report) => {
                        let can_update = !report.updatable_providers().is_empty();
                        let count = report.providers.iter().filter(|p| p.is_outdated()).count()
                            + usize::from(report.app.needs_action());
                        view! {
                            <button
                                type="button"
                                data-testid="sidebar-update"
                                title=report.summary()
                                class=format!("{PILL} {PILL_ACTION}")
                                on:click=move |_| {
                                    if can_update {
                                        updates.confirm_open.set(true);
                                    } else {
                                        open_settings();
                                    }
                                }
                            >
                                <Icon name="download" size=14 class="shrink-0" />
                                <span class="min-w-0 flex-1 truncate">{sidebar_label(&report)}</span>
                                {(count > 1).then(|| view! {
                                    <span class="text-label-sm text-persimmon-300/80">{count.to_string()}</span>
                                })}
                            </button>
                        }
                        .into_any()
                    }
                    None => ().into_any(),
                },
            }}
        </div>
        <UpdateConfirmDialog />
    }
}

/// T3's "Update N machine(s)?", for this machine's providers.
#[component]
fn UpdateConfirmDialog() -> impl IntoView {
    let updates = Updates::get();
    let report = Memo::new(move |_| updates.report.get());
    view! {
        <Dialog
            open=Signal::derive(move || updates.confirm_open.get())
            on_close=Callback::new(move |()| updates.confirm_open.set(false))
            max_width="max-w-md".to_string()
            label="Update providers"
        >
            {move || report.get().map(|report| {
                let targets = report.updatable_providers();
                let no_targets = targets.is_empty();
                let skipped = report.skipped_providers();
                let app_behind = report.app.needs_action();
                let skipped_count = skipped.len() + usize::from(app_behind);
                let rows = targets
                    .iter()
                    .filter_map(|p| report.provider(*p).cloned())
                    .map(|p| view! {
                        <li class="flex flex-col gap-0.5 rounded-grape bg-longan-900 px-3 py-2 inset-ring inset-ring-white/5">
                            <span class="flex items-center justify-between gap-2 text-label text-rambutan-100">
                                {provider_cli_label(p.provider)}
                                <span class="text-label-sm text-rambutan-400">
                                    {format!("{} → {}", p.installed.clone().unwrap_or_default(), p.latest.clone().unwrap_or_default())}
                                </span>
                            </span>
                            <code class="truncate text-code-sm text-rambutan-500">{p.command_line().unwrap_or_default()}</code>
                        </li>
                    })
                    .collect_view();
                view! {
                    <div class="space-y-3 p-4" data-testid="update-confirm">
                        <div>
                            <h2 class="text-h4 text-rambutan-100">{confirm_title(targets.len())}</h2>
                            <p class="mt-0.5 text-label-sm text-rambutan-500">
                                "Running chats keep the CLI they started with. New sessions use the update."
                            </p>
                        </div>
                        <ul class="space-y-1.5">{rows}</ul>
                        {(skipped_count > 0).then(|| view! {
                            <details class="text-label-sm text-rambutan-400">
                                <summary class="cursor-pointer select-none">{format!("{skipped_count} skipped")}</summary>
                                <ul class="mt-1.5 space-y-1 pl-4">
                                    {skipped.iter().map(|(p, why)| view! {
                                        <li>{format!("{} · {why}", provider_cli_label(*p))}</li>
                                    }).collect_view()}
                                    {app_behind.then(|| view! {
                                        <li>"bruhs · needs a manual update (Settings → Updates has the command)"</li>
                                    })}
                                </ul>
                            </details>
                        })}
                        <div class="flex justify-end gap-2 pt-1">
                            <Button
                                variant=ButtonVariant::Ghost
                                size=ButtonSize::Small
                                on_click=Callback::new(move |_| updates.confirm_open.set(false))
                            >
                                "Cancel"
                            </Button>
                            <Button
                                variant=ButtonVariant::Primary
                                size=ButtonSize::Small
                                disabled=no_targets
                                on_click=Callback::new(move |_| {
                                    updates.confirm_open.set(false);
                                    updates.update_all();
                                })
                            >
                                "Update"
                            </Button>
                        </div>
                    </div>
                }
            })}
        </Dialog>
    }
}

/// Settings → Updates.
#[component]
pub fn UpdatesSettings() -> impl IntoView {
    let updates = Updates::get();
    let enabled = updates.enabled;
    let checked_line = Signal::derive(move || {
        if let Some(error) = updates.start_error.get() {
            return error;
        }
        if updates.checking.get() {
            return "Checking…".to_string();
        }
        if let Some(error) = updates.error.get() {
            return error;
        }
        match updates.report.get() {
            Some(report) if !report.online => {
                "Update checks are off: installed versions only.".to_string()
            }
            Some(report) => format!(
                "Last checked {}.",
                format_relative_ms(report.checked_at, now(), RelativeTimeStyle::Ago)
            ),
            None => "Not checked yet.".to_string(),
        }
    });
    let updatable = Memo::new(move |_| {
        updates
            .report
            .get()
            .map(|r| r.updatable_providers().len())
            .unwrap_or_default()
    });
    view! {
        <SettingsRow
            title="Check for updates"
            description="Check installed provider CLIs for newer versions, and this checkout's origin for new commits. When off, bruhs still reads the installed versions but asks nothing online. This browser only."
        >
            <Switch
                checked=Signal::derive(move || enabled.get())
                on_change=Callback::new(move |next: bool| enabled.set(next))
                label="Check for updates"
            />
        </SettingsRow>
        <SettingsRow title="Status" live_description=checked_line>
            <Show when=move || { updatable.get() > 0 }>
                <button
                    type="button"
                    class=button_class(ButtonVariant::Primary, ButtonSize::Small, "")
                    on:click=move |_| updates.update_all()
                >
                    {move || if updatable.get() > 1 { "Update all" } else { "Update" }}
                </button>
            </Show>
            <button
                type="button"
                class=button_class(ButtonVariant::Outline, ButtonSize::Small, "")
                disabled=move || updates.checking.get()
                on:click=move |_| {
                    updates.start_error.set(None);
                    updates.check(true);
                }
            >
                <Icon name="refresh-cw" size=12 />
                "Check now"
            </button>
        </SettingsRow>
        <ProviderRow provider=AgentProvider::Claude />
        <ProviderRow provider=AgentProvider::Codex />
        <AppRow />
    }
}

#[component]
fn ProviderRow(provider: AgentProvider) -> impl IntoView {
    let updates = Updates::get();
    let entry = Memo::new(move |_| {
        updates
            .report
            .get()
            .and_then(|report| report.provider(provider).cloned())
    });
    let run = Memo::new(move |_| {
        updates
            .report
            .get()
            .and_then(|report| report.run(provider).cloned())
    });
    let line = Signal::derive(move || match entry.get() {
        Some(entry) => entry.status_line(),
        None if updates.checking.get() => "Checking…".into(),
        None => "Not checked yet.".into(),
    });
    // One block, so the card's dividers fall between providers rather than
    // between a provider and its warning, hint or progress line.
    view! {
        <div>
        // A `SettingsRow` with the binary path as a second, code-styled line.
        <div class="flex flex-col gap-3 px-4 py-3 sm:flex-row sm:items-center sm:justify-between">
            <div class="min-w-0 sm:max-w-[60%]">
                <h3 class="text-label text-rambutan-100">{provider_cli_label(provider)}</h3>
                <p class="mt-0.5 text-label-sm leading-relaxed text-rambutan-500">{move || line.get()}</p>
                {move || entry.get().and_then(|entry| entry.binary_path).map(|path| {
                    let title = path.clone();
                    view! {
                        <p class="mt-0.5 truncate text-code-sm text-rambutan-600" title=title>{path}</p>
                    }
                })}
            </div>
            <div class="flex shrink-0 items-center gap-2 sm:justify-end">
                {move || entry.get().map(|entry| {
                    let running = run.get().is_some_and(|r| r.is_active());
                    provider_control(updates, entry, running)
                })}
            </div>
        </div>
        {move || entry.get().and_then(|entry| entry.compat_warning()).map(|(title, message)| view! {
            <div class="px-4 pb-3">
                <div class="rounded-grape bg-durian-500/10 p-3 inset-ring inset-ring-durian-500/20">
                    <p class="flex items-center gap-1.5 text-label-sm text-durian-200">
                        <Icon name="alert-triangle" size=12 />
                        {title}
                    </p>
                    <p class="mt-1 text-label-sm leading-relaxed text-rambutan-300">{message}</p>
                </div>
            </div>
        })}
        {move || entry.get()
            .filter(|entry| entry.is_outdated() && entry.update_command.is_none())
            .and_then(|entry| entry.manual_hint)
            .map(|hint| view! {
                <div class="px-4 pb-3">
                    <p class="text-label-sm leading-relaxed text-rambutan-400">{hint}</p>
                </div>
            })}
        {move || run.get().map(|run| {
            if run.is_active() {
                view! {
                    <div class="px-4 pb-3">
                        <p class="flex min-w-0 items-center gap-1.5 text-label-sm text-rambutan-300" role="status">
                            <Icon name="loader" size=12 class="shrink-0 motion-safe:animate-spin" />
                            <span class="truncate">{run.phase_line()}</span>
                        </p>
                    </div>
                }
                .into_any()
            } else if run.needs_attention() {
                view! {
                    <div class="px-4 pb-3">
                        <div class="rounded-grape bg-lychee-500/10 p-3 inset-ring inset-ring-lychee-500/20" role="alert">
                            <p class="text-label-sm text-lychee-200">{run.message.clone().unwrap_or_default()}</p>
                            {(!run.output.is_empty()).then(|| view! {
                                <pre class="mt-2 max-h-40 overflow-auto whitespace-pre-wrap text-code-sm text-rambutan-300">{run.output.clone()}</pre>
                            })}
                        </div>
                    </div>
                }
                .into_any()
            } else {
                ().into_any()
            }
        })}
        </div>
    }
}

fn provider_control(updates: Updates, entry: ProviderUpdate, running: bool) -> AnyView {
    let provider = entry.provider;
    if running {
        return view! {
            <span class="inline-flex items-center gap-1.5 text-label-sm text-rambutan-300">
                <Icon name="loader" size=12 class="motion-safe:animate-spin" />
                "Updating…"
            </span>
        }
        .into_any();
    }
    match (entry.state, entry.command_line()) {
        (ProviderUpdateState::Outdated, Some(command)) => view! {
            <button
                type="button"
                title=format!("Runs `{command}`")
                class=button_class(ButtonVariant::Primary, ButtonSize::Small, "")
                on:click=move |_| updates.update(provider)
            >
                "Update now"
            </button>
            <CopyButton
                text=command
                label="Copy update command. Run it in a terminal when you are ready to update."
            />
        }
        .into_any(),
        (ProviderUpdateState::Outdated, None) => view! {
            <span class="text-label-sm text-rambutan-400">"Update by hand"</span>
        }
        .into_any(),
        (ProviderUpdateState::UpToDate, _) => view! {
            <span class="inline-flex items-center gap-1 text-label-sm text-pandan-400">
                <Icon name="check" size=12 />
                "Up to date"
            </span>
        }
        .into_any(),
        _ => ().into_any(),
    }
}

#[component]
fn AppRow() -> impl IntoView {
    let updates = Updates::get();
    let app = Memo::new(move |_| updates.report.get().map(|report| report.app));
    let line = Signal::derive(move || match app.get() {
        Some(app) => app.status_line(),
        None => "Not checked yet.".into(),
    });
    view! {
        <div>
        <SettingsRow title="bruhs" live_description=line>
            {move || app.get().map(|app| match app.state {
                AppUpdateState::UpToDate => view! {
                    <span class="inline-flex items-center gap-1 text-label-sm text-pandan-400">
                        <Icon name="check" size=12 />
                        "Up to date"
                    </span>
                }.into_any(),
                _ => ().into_any(),
            })}
        </SettingsRow>
                {move || app.get()
            .filter(|app| app.state == AppUpdateState::Behind)
            .and_then(|app| app.release)
            .map(|release| view! {
                <div class="px-4 pb-3">
                    <p class="text-label-sm text-rambutan-400">
                        "The Bruhs app downloads the update and asks to restart. "
                        {release.release_url.map(|url| view! {
                            <a href=url target="_blank" rel="noreferrer" class="text-blueberry-300 hover:text-blueberry-200">"Release notes"</a>
                        })}
                    </p>
                </div>
            })}
        {move || app.get().and_then(|app| app.command).map(|command| view! {
            <div class="px-4 pb-3">
                <p class="mb-1.5 text-label-sm text-rambutan-500">
                    "bruhs runs from this checkout, so it does not pull for you. Run this in a terminal, then reload:"
                </p>
                <div class="flex items-center gap-2 rounded-grape bg-longan-950 px-3 py-2 inset-ring inset-ring-white/10">
                    <code class="min-w-0 flex-1 overflow-x-auto whitespace-nowrap text-code-sm text-rambutan-200">{command.clone()}</code>
                    <CopyButton text=command label="Copy update command" />
                </div>
            </div>
        })}
        </div>
    }
}

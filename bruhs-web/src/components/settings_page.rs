//! Settings: a section nav on the left,
//! titled cards of label + control rows on the right. The section
//! components in `SettingsSection` / `SettingsRow` are shared by the panels.

use bruhs_models::agent::{AgentProvider, MODEL_CATALOG};
use bruhs_models::logic::editor::EDITORS;
use leptos::html::Div;
use leptos::prelude::*;
use wasm_bindgen::JsCast;

use crate::components::auto_record_settings::AutoRecordSettingsPanel;
use crate::components::diarize_settings::DiarizeSettingsPanel;
use crate::components::google_calendar_settings::GoogleCalendarSettings;
use crate::components::personality_settings::PersonalitySettings as PersonalitySettingsPanel;
use crate::components::provider::provider_label;
use crate::components::whisper_model_settings::WhisperModelSettings;
use crate::state::{
    AppView, ClaudeAutoCompactSignal, CompactIdleSignal, EditorSignal, FollowUpSteerSignal,
    KeybindingsSignal, PermissionMode, PermissionModeSignal, SetActiveView,
    SetPermissionModeSignal, SidebarChrome, TitleInstructionsSignal,
};
use crate::ui::{Dropdown, DropdownOption, Icon, Switch};

/// `(id, icon, label)` for the section nav, in page order.
pub const SECTIONS: [(&str, &str, &str); 11] = [
    ("general", "settings", "General"),
    ("updates", "download", "Updates"),
    ("beta", "activity", "Beta"),
    ("credentials", "shield", "Credentials"),
    ("calendar", "calendar", "Calendar"),
    ("recording", "mic", "Recording"),
    ("transcription", "activity", "Transcription"),
    ("speakers", "users", "Speakers"),
    ("people", "book-open", "People"),
    ("keyboard", "keyboard", "Keybindings"),
    ("about", "help-circle", "About"),
];

/// Which section owns the viewport: the last one whose top is above the
/// fold line (`top` values are relative to the scroll container's top).
pub fn active_section(tops: &[(String, f64)], fold_px: f64) -> Option<String> {
    tops.iter()
        .rev()
        .find(|(_, top)| *top <= fold_px)
        .or_else(|| tops.first())
        .map(|(id, _)| id.clone())
}

/// A titled card of rows on the panel tone.
#[component]
pub fn SettingsSection(
    #[prop(into)] id: String,
    #[prop(into)] title: String,
    #[prop(optional, into)] description: String,
    children: Children,
) -> impl IntoView {
    view! {
        <section id=id data-settings-section="true" class="scroll-mt-4">
            <div class="mb-2 px-1">
                <h2 class="text-label text-rambutan-100">{title}</h2>
                {(!description.is_empty()).then(|| view! {
                    <p class="mt-0.5 text-label-sm text-rambutan-500">{description}</p>
                })}
            </div>
            <div class="divide-y divide-longan-800/70 rounded-lychee bg-longan-900 inset-ring inset-ring-white/5">
                {children()}
            </div>
        </section>
    }
}

/// One setting: title and description on the left, control on the right.
#[component]
pub fn SettingsRow(
    #[prop(into)] title: String,
    #[prop(optional, into)] description: String,
    /// Reactive description (status lines); rendered under `description`.
    #[prop(optional)]
    live_description: Option<Signal<String>>,
    #[prop(optional, into)] disabled: Signal<bool>,
    children: Children,
) -> impl IntoView {
    view! {
        <div class=move || format!(
            "flex flex-col gap-3 px-4 py-3 transition-opacity sm:flex-row sm:items-center sm:justify-between {}",
            if disabled.get() { "opacity-60" } else { "" }
        )>
            <div class="min-w-0 sm:max-w-[60%]">
                <h3 class="text-label text-rambutan-100">{title}</h3>
                {(!description.is_empty()).then(|| view! {
                    <p class="mt-0.5 text-label-sm leading-relaxed text-rambutan-500">{description}</p>
                })}
                {live_description.map(|d| view! {
                    <p class="mt-0.5 text-label-sm leading-relaxed text-rambutan-500">{move || d.get()}</p>
                })}
            </div>
            <div class="flex shrink-0 items-center gap-2 sm:justify-end">{children()}</div>
        </div>
    }
}

/// Full-width content that belongs to the row above it (lists, sliders).
#[component]
pub fn SettingsRowBody(children: Children) -> impl IntoView {
    view! { <div class="px-4 pb-4">{children()}</div> }
}

/// Width for row controls (dropdowns, steppers).
pub const ROW_CONTROL: &str = "w-full sm:w-60";

#[component]
fn GeneralSettings() -> impl IntoView {
    let crate::state::DefaultModel(default_model) = expect_context::<crate::state::DefaultModel>();
    let PermissionModeSignal(permission_mode) = expect_context::<PermissionModeSignal>();
    let SetPermissionModeSignal(set_permission_mode) = expect_context::<SetPermissionModeSignal>();
    let EditorSignal(editor_id) = expect_context::<EditorSignal>();
    let editor_options: Vec<DropdownOption> = EDITORS
        .iter()
        .map(|e| DropdownOption::new(e.id, e.label).in_group(e.family.label()))
        .collect();

    let mut providers: Vec<AgentProvider> = Vec::new();
    for m in MODEL_CATALOG {
        if !providers.contains(&m.provider) {
            providers.push(m.provider);
        }
    }
    let model_options: Vec<DropdownOption> = providers
        .iter()
        .flat_map(|&p| {
            MODEL_CATALOG
                .iter()
                .filter(move |m| m.provider == p)
                .map(|m| DropdownOption::new(m.id, m.label).in_group(provider_label(m.provider)))
        })
        .collect();
    let mode_options: Vec<DropdownOption> = [
        PermissionMode::Default,
        PermissionMode::AcceptEdits,
        PermissionMode::Plan,
        PermissionMode::BypassPermissions,
    ]
    .into_iter()
    .map(|m| DropdownOption::new(m.as_str(), m.label()).with_description(m.description(None)))
    .collect();

    view! {
        <SettingsRow
            title="Model"
            description="Used for new turns. The composer chip changes it per thread."
        >
            <Dropdown
                options=model_options
                value=Signal::derive(move || default_model.get())
                on_change=Callback::new(move |v: String| default_model.set(v))
                label="Default model"
                class=ROW_CONTROL
                align_end=true
            />
        </SettingsRow>
        <SettingsRow
            title="Access mode"
            description="How much the agent may do without asking. Default asks before shell commands and edits."
        >
            <Dropdown
                options=mode_options
                value=Signal::derive(move || permission_mode.get().as_str().to_string())
                on_change=Callback::new(move |v: String| set_permission_mode.set(PermissionMode::parse_lossy(&v)))
                label="Access mode"
                class=ROW_CONTROL
                align_end=true
            />
        </SettingsRow>
        <SettingsRow
            title="Editor"
            description="Where ⌘O and the diff panel's Open button send the project and files."
        >
            <Dropdown
                options=editor_options
                value=Signal::derive(move || editor_id.get())
                on_change=Callback::new(move |v: String| editor_id.set(v))
                label="Editor"
                class=ROW_CONTROL
                align_end=true
            />
        </SettingsRow>
        <FollowUpBehavior />
        <AutoCompactAfter />
        <CompactIdleThreads />
        <TextGeneration />
    }
}

/// Queue or Steer. Steer writes into a running Claude turn. Codex still queues.
#[component]
fn FollowUpBehavior() -> impl IntoView {
    let FollowUpSteerSignal(steer) = expect_context::<FollowUpSteerSignal>();
    view! {
        <SettingsRow
            title="Follow-up behavior"
            description="Steer sends the next message into a running Claude turn. Codex always queues. The send button names the action. Hold Command or Control and press Enter for the other one."
        >
            <select
                aria-label="Follow-up behavior"
                class="min-h-10 w-full rounded-grape bg-longan-950 px-2 text-body-sm text-rambutan-100 inset-ring inset-ring-white/10 sm:w-60"
                prop:value=move || if steer.get() { "steer" } else { "queue" }
                on:change=move |ev| {
                    steer.set(event_target_value(&ev) == "steer");
                }
            >
                <option value="queue">"Queue"</option>
                <option value="steer">"Steer"</option>
            </select>
        </SettingsRow>
    }
}

/// Claude `--autocompact`. Empty leaves the CLI default. Codex ignores it.
#[component]
fn AutoCompactAfter() -> impl IntoView {
    let ClaudeAutoCompactSignal(tokens) = expect_context::<ClaudeAutoCompactSignal>();
    view! {
        <SettingsRow
            title="Auto-compact after"
            description="Claude compacts a long thread around this many tokens, from 100000 to 1000000. Empty uses Claude's own default. Codex has no auto-compact."
        >
            <input
                type="text"
                inputmode="numeric"
                aria-label="Auto-compact after"
                class="min-h-10 w-full rounded-grape bg-longan-950 px-2 text-body-sm text-rambutan-100 inset-ring inset-ring-white/10 placeholder:text-rambutan-500 sm:w-60"
                placeholder="Claude default"
                prop:value=move || tokens.get().unwrap_or_default()
                on:change=move |ev| {
                    let raw = event_target_value(&ev);
                    match bruhs_models::logic::compact::autocompact_tokens(&raw) {
                        Some(value) => tokens.set(Some(value.to_string())),
                        None if raw.trim().is_empty() => tokens.set(None),
                        None => {}
                    }
                }
            />
        </SettingsRow>
    }
}

/// Idle compact stays in General. bruhs has no Providers page.
#[component]
fn CompactIdleThreads() -> impl IntoView {
    let CompactIdleSignal(mode) = expect_context::<CompactIdleSignal>();
    view! {
        <SettingsRow
            title="Compact idle threads"
            description="When you return, a Compact chip sits beside send after the prompt cache expires on a Claude thread over 200,000 tokens. Enter summarizes, then sends the draft. Before the cache expires compacts that thread while bruhs is open. Never skips both. Manual /compact still works."
        >
            <select
                aria-label="Compact idle threads"
                class="min-h-10 w-full rounded-grape bg-longan-950 px-2 text-body-sm text-rambutan-100 inset-ring inset-ring-white/10 sm:w-60"
                prop:value=move || mode.get().as_str().to_string()
                on:change=move |ev| {
                    mode.set(bruhs_models::logic::compact::IdleCompact::parse(
                        &event_target_value(&ev),
                    ));
                }
            >
                <option value="return">"When you return"</option>
                <option value="before">"Before the cache expires"</option>
                <option value="never">"Never"</option>
            </select>
        </SettingsRow>
    }
}

/// Thread title instructions. `prefix:` and `max N` apply on this device.
/// Other lines are kept for the person who wrote them.
#[component]
fn TextGeneration() -> impl IntoView {
    let TitleInstructionsSignal(instructions) = expect_context::<TitleInstructionsSignal>();
    let threads = expect_context::<crate::state::ThreadsSignal>().0;
    let (project, set_project) = signal(String::new());
    let names = Signal::derive(move || {
        threads.with(|rows| {
            let mut names = rows
                .iter()
                .filter(|thread| !thread.project_name.is_empty())
                .map(|thread| thread.project_name.clone())
                .collect::<Vec<_>>();
            names.sort();
            names.dedup();
            names
        })
    });
    view! {
        <SettingsRow
            title="Thread title instructions"
            description="Applied to new titles and Regenerate title. A prefix: line is added in front. max N caps the length. This build does not send the other lines to a model."
        >
            <textarea
                aria-label="Thread title instructions"
                class="min-h-24 w-full rounded-grape bg-longan-950 px-2 py-1.5 text-body-sm text-rambutan-100 inset-ring inset-ring-white/10 placeholder:text-rambutan-500 sm:w-72"
                placeholder="prefix: Bug\nmax 48"
                prop:value=move || instructions.with(|store| store.global.clone())
                on:input=move |ev| {
                    let value = event_target_value(&ev);
                    instructions.update(|store| store.global = value);
                }
            />
        </SettingsRow>
        <SettingsRow
            title="Project title instructions"
            description="Optional extra lines for one project. They are applied after the instructions above."
        >
            <select
                aria-label="Project for title instructions"
                class="min-h-10 w-full rounded-grape bg-longan-950 px-2 text-body-sm text-rambutan-100 inset-ring inset-ring-white/10 sm:w-60"
                on:change=move |ev| set_project.set(event_target_value(&ev))
            >
                <option value="">"Choose a project"</option>
                {move || names.get().into_iter().map(|name| {
                    let selected = name == project.get();
                    let label = name.clone();
                    view! { <option value=name selected=selected>{label}</option> }
                }).collect_view()}
            </select>
        </SettingsRow>
        <Show when=move || !project.get().is_empty()>
            <SettingsRowBody>
                <textarea
                    aria-label="Project thread title instructions"
                    class="min-h-20 w-full rounded-grape bg-longan-950 px-2 py-1.5 text-body-sm text-rambutan-100 inset-ring inset-ring-white/10"
                    prop:value=move || {
                        let name = project.get();
                        instructions.with(|store| store.by_project.get(&name).cloned().unwrap_or_default())
                    }
                    on:input=move |ev| {
                        let value = event_target_value(&ev);
                        let name = project.get_untracked();
                        instructions.update(|store| {
                            if value.trim().is_empty() {
                                store.by_project.remove(&name);
                            } else {
                                store.by_project.insert(name, value);
                            }
                        });
                    }
                />
            </SettingsRowBody>
        </Show>
    }
}

#[component]
fn BetaSettings() -> impl IntoView {
    let SidebarChrome {
        working_section, ..
    } = expect_context::<SidebarChrome>();
    view! {
        <SettingsRow
            title="Working section"
            description="Threads that are working, or waiting on work they will resume, move into a collapsed section under the active list. A dev server left running does not count. An approval comes back to the active list. This device only."
        >
            <Switch
                checked=Signal::derive(move || working_section.get())
                on_change=Callback::new(move |next: bool| working_section.set(next))
                label="Working section"
            />
        </SettingsRow>
    }
}

fn listed_keybindings(
    rules: RwSignal<Vec<bruhs_models::logic::keybindings::Keybinding>>,
) -> Vec<(usize, bruhs_models::logic::keybindings::Keybinding)> {
    rules.get().into_iter().enumerate().collect()
}

#[component]
fn KeyboardSettings() -> impl IntoView {
    let KeybindingsSignal(rules) = expect_context::<KeybindingsSignal>();
    let defaults = StoredValue::new(bruhs_models::logic::keybindings::default_keybindings());
    let listed = move || listed_keybindings(rules);
    view! {
        <For
            each=listed
            key=|(index, rule)| (*index, rule.command.clone())
            children=move |(index, rule)| {
                let command = rule.command;
                let key_label = command.clone();
                let when_label = command.clone();
                let reset_command = command.clone();
                let command_attr = command.clone();
                view! {
                    <div class="flex items-center justify-between gap-3 px-4 py-2" data-keybinding-command=command_attr>
                        <span class="min-w-0 truncate text-label text-rambutan-200">{command}</span>
                        <span class="flex shrink-0 items-center gap-1">
                            <input
                                class="h-8 w-36 rounded-seed bg-longan-800 px-2 text-label-sm text-rambutan-100 outline-none inset-ring inset-ring-white/10 focus:outline focus:outline-2 focus:outline-persimmon-400"
                                aria-label=format!("Key for {key_label}")
                                spellcheck="false"
                                prop:value=move || {
                                    rules.with(|rows| {
                                        rows.get(index).map(|row| row.key.clone()).unwrap_or_default()
                                    })
                                }
                                on:change=move |event| {
                                    let value = event_target_value(&event);
                                    rules.update(|rows| {
                                        if let Some(row) = rows.get_mut(index) {
                                            row.key = value;
                                        }
                                    });
                                }
                            />
                            <input
                                class=move || {
                                    let broken = rules.with(|rows| {
                                        rows.get(index).and_then(|row| row.when.clone()).is_some_and(|when| {
                                            !when.is_empty()
                                                && !bruhs_models::logic::keybindings::when_syntax_ok(&when)
                                        })
                                    });
                                    if broken {
                                        "h-8 w-40 rounded-seed bg-longan-800 px-2 text-label-sm text-rambutan-100 outline outline-2 outline-persimmon-400"
                                    } else {
                                        "h-8 w-40 rounded-seed bg-longan-800 px-2 text-label-sm text-rambutan-100 outline-none inset-ring inset-ring-white/10 focus:outline focus:outline-2 focus:outline-persimmon-400"
                                    }
                                }
                                aria-label=format!("When for {when_label}")
                                placeholder="when"
                                spellcheck="false"
                                title="Optional when clause. Leave empty to always match."
                                prop:value=move || {
                                    rules.with(|rows| {
                                        rows.get(index)
                                            .and_then(|row| row.when.clone())
                                            .unwrap_or_default()
                                    })
                                }
                                on:change=move |event| {
                                    let value = event_target_value(&event);
                                    let when = if value.trim().is_empty() { None } else { Some(value) };
                                    rules.update(|rows| {
                                        if let Some(row) = rows.get_mut(index) {
                                            row.when = when;
                                        }
                                    });
                                }
                            />
                            <button
                                type="button"
                                class="inline-flex h-8 items-center rounded-seed px-2 text-label-sm text-rambutan-300 hover:bg-longan-800 hover:text-rambutan-100"
                                on:click=move |_| {
                                    let Some(original) = defaults.with_value(|defaults| {
                                        let nth = rules.with_untracked(|rows| {
                                            rows.iter()
                                                .take(index)
                                                .filter(|row| row.command == reset_command)
                                                .count()
                                        });
                                        defaults
                                            .iter()
                                            .filter(|row| row.command == reset_command)
                                            .nth(nth)
                                            .cloned()
                                    }) else {
                                        return;
                                    };
                                    rules.update(|rows| {
                                        if let Some(row) = rows.get_mut(index) {
                                            *row = original;
                                        }
                                    });
                                }
                            >
                                "Reset"
                            </button>
                        </span>
                    </div>
                }
            }
        />
    }
}

#[component]
fn AboutSettings() -> impl IntoView {
    view! {
        <SettingsRow title="Version" description="bruhs-web, built from the workspace crate.">
            <span class="text-code-sm text-rambutan-200">{env!("CARGO_PKG_VERSION")}</span>
        </SettingsRow>
        <SettingsRow
            title="MCP servers"
            description="Configured through Claude Code settings; connected servers appear in each session's init event."
        >
            <span class="text-label-sm text-rambutan-500">"Managed by Claude Code"</span>
        </SettingsRow>
        <SettingsRow title="Design system" description="Bruhs: fruit-named tokens, all-mono type, no shadows.">
            <a
                href="https://design.bruhs.dev"
                target="_blank"
                rel="noreferrer"
                class="inline-flex h-7 items-center gap-1.5 rounded-grape px-2.5 text-label-sm text-rambutan-200 inset-ring inset-ring-white/10 transition-colors hover:bg-longan-800 hover:text-rambutan-100"
            >
                "design.bruhs.dev"
                <Icon name="globe" size=12 class="text-rambutan-500" />
            </a>
        </SettingsRow>
    }
}

#[component]
pub fn SettingsPage() -> impl IntoView {
    let SetActiveView(set_active_view) = expect_context::<SetActiveView>();
    let (current, set_current) = signal(SECTIONS[0].0.to_string());
    let content_ref = NodeRef::<Div>::new();

    let jump = move |id: &'static str| {
        set_current.set(id.to_string());
        if let Some(doc) = web_sys::window().and_then(|w| w.document()) {
            if let Some(el) = doc.get_element_by_id(id) {
                el.scroll_into_view();
            }
        }
    };
    // The sidebar's update button opens Settings at Updates.
    let updates = crate::updates::Updates::get();
    Effect::new(move |_| {
        if updates.reveal_in_settings.get() {
            updates.reveal_in_settings.set(false);
            jump("updates");
        }
    });
    // Scroll spy: highlight the section under the fold line.
    let on_scroll = move |_| {
        let Some(Some(container)) = content_ref.try_get_untracked() else {
            return;
        };
        let container: web_sys::HtmlElement = (*container).clone();
        let base = container.get_bounding_client_rect().top();
        let Some(doc) = web_sys::window().and_then(|w| w.document()) else {
            return;
        };
        let tops: Vec<(String, f64)> = SECTIONS
            .iter()
            .filter_map(|(id, _, _)| {
                doc.get_element_by_id(id).map(|el| {
                    let el: web_sys::Element = el.unchecked_into();
                    (id.to_string(), el.get_bounding_client_rect().top() - base)
                })
            })
            .collect();
        if let Some(id) = active_section(&tops, 96.0) {
            if current.get_untracked() != id {
                set_current.set(id);
            }
        }
    };
    let current_label = move || {
        SECTIONS
            .iter()
            .find(|(id, _, _)| *id == current.get())
            .map_or("", |(_, _, label)| *label)
    };

    view! {
        <section class="flex h-full min-h-0 flex-1 flex-col bg-longan-950">
            <header class="flex h-[52px] shrink-0 items-center gap-2 border-b border-longan-800/60 px-4">
                <h1 class="text-label text-rambutan-100">"Settings"</h1>
                <span class="text-label-sm text-rambutan-500" aria-hidden="true">"/"</span>
                <span class="text-label-sm text-rambutan-300">{current_label}</span>
                <span class="flex-1"></span>
                <button
                    type="button"
                    aria-label="Back to chat"
                    title="Back to chat"
                    class="inline-flex size-7 items-center justify-center rounded-grape text-rambutan-400 transition-colors hover:bg-longan-800/60 hover:text-rambutan-100"
                    on:click=move |_| set_active_view.set(AppView::Chat)
                >
                    <Icon name="x" size=14 />
                </button>
            </header>
            <div class="flex min-h-0 flex-1">
                <nav
                    aria-label="Settings sections"
                    class="hidden w-52 shrink-0 flex-col gap-0.5 overflow-y-auto border-r border-longan-800/60 p-3 md:flex"
                >
                    {SECTIONS.iter().map(|(id, icon, label)| {
                        let id: &'static str = id;
                        let is_on = Signal::derive(move || current.get() == id);
                        view! {
                            <button
                                type="button"
                                aria-current=move || if is_on.get() { "true" } else { "false" }
                                class=move || format!(
                                    "flex h-8 items-center gap-2 rounded-grape px-2 text-left text-label transition-colors {}",
                                    if is_on.get() {
                                        "bg-longan-800/60 text-rambutan-100"
                                    } else {
                                        "text-rambutan-400 hover:bg-longan-800/40 hover:text-rambutan-100"
                                    }
                                )
                                on:click=move |_| jump(id)
                            >
                                <Icon name=*icon size=14 class="shrink-0 text-rambutan-500" />
                                <span>{*label}</span>
                            </button>
                        }
                    }).collect_view()}
                </nav>
                <div node_ref=content_ref class="min-h-0 flex-1 overflow-y-auto" on:scroll=on_scroll>
                    <div class="mx-auto flex w-full max-w-3xl flex-col gap-8 px-5 py-6 md:px-8">
                        <SettingsSection id="general" title="General" description="Defaults for new turns.">
                            <GeneralSettings />
                        </SettingsSection>
                        <SettingsSection id="updates" title="Updates" description="Provider CLIs and this build of bruhs.">
                            <crate::components::updates::UpdatesSettings />
                        </SettingsSection>
                        <SettingsSection id="beta" title="Beta" description="Features that are still settling.">
                            <BetaSettings />
                        </SettingsSection>
                        <SettingsSection id="credentials" title="Credentials" description="Saved credentials stay in your OS keychain. Agents still need approval for every use.">
                            <crate::components::credential_settings::CredentialSettings />
                        </SettingsSection>
                        <SettingsSection id="calendar" title="Calendar" description="Google Calendar sync for the calendar view and meeting auto-record.">
                            <GoogleCalendarSettings />
                        </SettingsSection>
                        <SettingsSection id="recording" title="Recording" description="When bruhs starts recording meetings on its own.">
                            <AutoRecordSettingsPanel />
                        </SettingsSection>
                        <SettingsSection id="transcription" title="Transcription" description="Whisper models for the final transcript and the live preview.">
                            <WhisperModelSettings />
                        </SettingsSection>
                        <SettingsSection id="speakers" title="Speakers" description="Telling voices apart and remembering them across meetings.">
                            <DiarizeSettingsPanel />
                        </SettingsSection>
                        <SettingsSection id="people" title="People" description="What meeting summaries record about each person.">
                            <PersonalitySettingsPanel />
                        </SettingsSection>
                        <SettingsSection id="keyboard" title="Keybindings" description="Each shortcut is a key, a command id, and an optional when clause. A new default is added when its command is not already listed. Chrome keeps some keys for itself.">
                            <KeyboardSettings />
                        </SettingsSection>
                        <SettingsSection id="about" title="About">
                            <AboutSettings />
                        </SettingsSection>
                        <div class="h-24" aria-hidden="true"></div>
                    </div>
                </div>
            </div>
        </section>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn active_section_is_last_above_fold() {
        let tops = vec![
            ("general".to_string(), -400.0),
            ("calendar".to_string(), -20.0),
            ("recording".to_string(), 300.0),
        ];
        assert_eq!(active_section(&tops, 96.0).as_deref(), Some("calendar"));
        let top = vec![("general".to_string(), 200.0)];
        assert_eq!(active_section(&top, 96.0).as_deref(), Some("general"));
        assert_eq!(active_section(&[], 96.0), None);
    }

    #[test]
    fn nav_and_shortcut_tables_are_well_formed() {
        assert_eq!(SECTIONS.len(), 11);
        assert_eq!(SECTIONS[1].0, "updates");
        assert_eq!(SECTIONS[9].2, "Keybindings");
        let rules = bruhs_models::logic::keybindings::default_keybindings();
        assert!(rules.iter().all(|rule| {
            !rule.command.is_empty()
                && (rule.key.is_empty()
                    && bruhs_models::logic::keybindings::UNBOUND_BY_DEFAULT
                        .contains(&rule.command.as_str())
                    || bruhs_models::logic::keybindings::parse_chord(&rule.key, true).is_some())
        }));
        assert!(rules
            .iter()
            .any(|rule| rule.command == "chat.new" && rule.key == "mod+shift+o"));
        assert!(rules
            .iter()
            .any(|rule| rule.command == "thread.pin" && rule.key == "mod+shift+p"));
    }
}

//! Settings rows for roles (named model setups an agent starts a thread
//! with) and for the models of jobs that are not threads. The rules live in
//! `bruhs_models::roles`, shared with desktop.

use bruhs_models::agent::{AgentProvider, PermissionMode, MODEL_CATALOG};
use bruhs_models::logic::run_place::CLAUDE_EFFORTS;
use bruhs_models::roles::{
    Role, RoleOverride, MAX_INSTRUCTIONS_CHARS, MAX_NAME_CHARS, MAX_ROLES, MAX_USE_FOR_CHARS,
};
use leptos::prelude::*;

use crate::components::provider::provider_label;
use crate::components::settings_page::{SettingsRow, SettingsRowBody, ROW_CONTROL};
use crate::state::{RolesSignal, TaskModelsSignal, ThreadsSignal};
use crate::ui::{Dropdown, DropdownOption};

const TEXT_INPUT: &str = "h-8 w-full rounded-grape bg-white/5 px-3 text-body-sm text-rambutan-100 inset-ring inset-ring-white/10 placeholder:text-rambutan-500 transition-colors hover:bg-white/[0.07] focus-visible:outline-2 focus-visible:-outline-offset-1 focus-visible:outline-persimmon-400 sm:w-60";
const TEXT_AREA: &str = "min-h-[6rem] w-full rounded-grape bg-white/5 px-3 py-1.5 text-body-sm text-rambutan-100 inset-ring inset-ring-white/10 placeholder:text-rambutan-500";
const BUTTON: &str = "inline-flex h-8 shrink-0 items-center rounded-grape px-3 text-label-sm text-rambutan-200 inset-ring inset-ring-white/10 transition-colors hover:bg-longan-800/60 hover:text-rambutan-100 disabled:opacity-50 focus-visible:outline-2 focus-visible:outline-persimmon-400 cursor-pointer";
const FIELD: &str = "flex flex-col gap-3 py-2 sm:flex-row sm:items-center sm:justify-between";
const FIELD_LABEL: &str = "text-label-sm text-rambutan-300";

/// Every model, grouped by provider, after one option that keeps `inherit`.
fn model_options(inherit: &str, only: Option<AgentProvider>) -> Vec<DropdownOption> {
    let mut providers: Vec<AgentProvider> = Vec::new();
    for model in MODEL_CATALOG {
        if !providers.contains(&model.provider) && only.is_none_or(|only| only == model.provider) {
            providers.push(model.provider);
        }
    }
    std::iter::once(DropdownOption::new("", inherit))
        .chain(providers.into_iter().flat_map(|provider| {
            MODEL_CATALOG
                .iter()
                .filter(move |model| model.provider == provider)
                .map(|model| {
                    DropdownOption::new(model.id, model.label)
                        .in_group(provider_label(model.provider))
                })
        }))
        .collect()
}

fn effort_options(inherit: &str) -> Vec<DropdownOption> {
    std::iter::once(DropdownOption::new("", inherit))
        .chain(
            CLAUDE_EFFORTS
                .iter()
                .map(|level| DropdownOption::new(*level, *level)),
        )
        .collect()
}

/// A role can only narrow access, so the loosest mode is not offered: it
/// would never change anything.
fn access_options(inherit: &str) -> Vec<DropdownOption> {
    std::iter::once(DropdownOption::new("", inherit))
        .chain(
            [
                PermissionMode::Plan,
                PermissionMode::Default,
                PermissionMode::AcceptEdits,
            ]
            .into_iter()
            .map(|mode| DropdownOption::new(mode.as_str(), mode.label())),
        )
        .collect()
}

fn access_value(access: Option<PermissionMode>) -> String {
    access
        .map(|mode| mode.as_str().to_string())
        .unwrap_or_default()
}

fn parse_access(value: &str) -> Option<PermissionMode> {
    (!value.is_empty()).then(|| PermissionMode::parse_lossy(value))
}

fn some(value: String) -> Option<String> {
    Some(value).filter(|value| !value.is_empty())
}

/// Roles: pick one to edit, add one, and say what a project changes.
#[component]
pub fn RoleSettings() -> impl IntoView {
    let RolesSignal(book) = expect_context::<RolesSignal>();
    let threads = expect_context::<ThreadsSignal>().0;
    // Id of the role being edited, and the project whose override is shown.
    let (selected, set_selected) = signal(String::new());
    let (project, set_project) = signal(String::new());
    let role = Memo::new(move |_| {
        let id = selected.get();
        book.with(|book| book.roles.iter().find(|role| role.id == id).cloned())
    });
    let project_names = Signal::derive(move || {
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
    let edit = move |change: &dyn Fn(&mut Role)| {
        let id = selected.get_untracked();
        book.update(|book| {
            book.update(&id, |role| change(role));
        });
    };
    let change_override = move |change: &dyn Fn(&mut RoleOverride)| {
        let id = selected.get_untracked();
        let name = project.get_untracked();
        book.update(|book| {
            book.update_override(&name, &id, |entry| change(entry));
        });
    };
    let override_now = move || {
        let id = selected.get();
        let name = project.get();
        book.with(|book| book.override_for(&name, &id).cloned())
            .unwrap_or_default()
    };
    let full = Signal::derive(move || book.with(|book| book.roles.len() >= MAX_ROLES));

    view! {
        <SettingsRow
            title="Roles"
            description="Named setups an agent can start a thread with: a model, an effort, an access mode and instructions. An agent picks one by name when it messages a new thread, so one thread can orchestrate while others write or run on the models you chose."
        >
            <Dropdown
                options=Signal::derive(move || {
                    book.with(|book| {
                        std::iter::once(DropdownOption::new("", "Choose a role"))
                            .chain(book.roles.iter().map(|role| {
                                DropdownOption::new(role.id.clone(), role.name.clone())
                            }))
                            .collect::<Vec<_>>()
                    })
                })
                value=selected
                placeholder="Choose a role"
                on_change=Callback::new(move |value: String| set_selected.set(value))
                label="Role to edit"
                class=ROW_CONTROL
                align_end=true
            />
            <button
                type="button"
                class=BUTTON
                disabled=move || full.get()
                on:click=move |_| {
                    let added = book.try_update(|book| book.add("New role")).flatten();
                    if let Some(id) = added {
                        set_selected.set(id);
                    }
                }
            >
                "New role"
            </button>
        </SettingsRow>
        <Show when=move || role.with(Option::is_some)>
            <SettingsRowBody>
                <div class="flex flex-col divide-y divide-longan-800/60" data-role-editor="">
                    <div class=FIELD>
                        <span class=FIELD_LABEL>"Name"</span>
                        <input
                            type="text"
                            autocomplete="off"
                            aria-label="Role name"
                            maxlength=MAX_NAME_CHARS.to_string()
                            class=TEXT_INPUT
                            prop:value=move || role.with(|role| role.as_ref().map(|role| role.name.clone()).unwrap_or_default())
                            on:change=move |ev| {
                                let value = event_target_value(&ev);
                                edit(&|role| role.name.clone_from(&value));
                            }
                        />
                    </div>
                    <div class=FIELD>
                        <span class=FIELD_LABEL>"Use for (shown to agents)"</span>
                        <input
                            type="text"
                            autocomplete="off"
                            aria-label="What to use this role for"
                            placeholder="drafting docs and release notes"
                            maxlength=MAX_USE_FOR_CHARS.to_string()
                            class=TEXT_INPUT
                            prop:value=move || role.with(|role| role.as_ref().map(|role| role.use_for.clone()).unwrap_or_default())
                            on:change=move |ev| {
                                let value = event_target_value(&ev);
                                edit(&|role| role.use_for.clone_from(&value));
                            }
                        />
                    </div>
                    <div class=FIELD>
                        <span class=FIELD_LABEL>"Model"</span>
                        <Dropdown
                            options=model_options("The sender's model", None)
                            value=Signal::derive(move || role.with(|role| role.as_ref().and_then(|role| role.model.clone()).unwrap_or_default()))
                            on_change=Callback::new(move |value: String| edit(&|role| role.model = some(value.clone())))
                            label="Role model"
                            class=ROW_CONTROL
                            align_end=true
                        />
                    </div>
                    <div class=FIELD>
                        <span class=FIELD_LABEL>"Effort (Claude)"</span>
                        <Dropdown
                            options=effort_options("Your default")
                            value=Signal::derive(move || role.with(|role| role.as_ref().and_then(|role| role.effort.clone()).unwrap_or_default()))
                            on_change=Callback::new(move |value: String| edit(&|role| role.effort = some(value.clone())))
                            label="Role effort"
                            class=ROW_CONTROL
                            align_end=true
                        />
                    </div>
                    <div class=FIELD>
                        <span class=FIELD_LABEL>"Access (can only narrow yours)"</span>
                        <Dropdown
                            options=access_options("Your access mode")
                            value=Signal::derive(move || role.with(|role| access_value(role.as_ref().and_then(|role| role.access))))
                            on_change=Callback::new(move |value: String| edit(&|role| role.access = parse_access(&value)))
                            label="Role access"
                            class=ROW_CONTROL
                            align_end=true
                        />
                    </div>
                    <div class="flex flex-col gap-2 py-2">
                        <span class=FIELD_LABEL>"Instructions (added to every turn of the thread)"</span>
                        <textarea
                            aria-label="Role instructions"
                            maxlength=MAX_INSTRUCTIONS_CHARS.to_string()
                            class=TEXT_AREA
                            placeholder="You write user-facing docs. Keep sentences short."
                            prop:value=move || role.with(|role| role.as_ref().map(|role| role.instructions.clone()).unwrap_or_default())
                            on:change=move |ev| {
                                let value = event_target_value(&ev);
                                edit(&|role| role.instructions.clone_from(&value));
                            }
                        />
                    </div>
                    <div class=FIELD>
                        <span class=FIELD_LABEL>"Project override"</span>
                        <Dropdown
                            options=Signal::derive(move || {
                                std::iter::once(DropdownOption::new("", "No project"))
                                    .chain(project_names.get().into_iter().map(|name| DropdownOption::new(name.clone(), name)))
                                    .collect::<Vec<_>>()
                            })
                            value=project
                            placeholder="No project"
                            on_change=Callback::new(move |value: String| set_project.set(value))
                            label="Project that overrides this role"
                            class=ROW_CONTROL
                            align_end=true
                        />
                    </div>
                    <Show when=move || !project.get().is_empty()>
                        <div class="flex flex-col divide-y divide-longan-800/60 ps-4" data-role-override="">
                            <div class=FIELD>
                                <span class=FIELD_LABEL>"Model in this project"</span>
                                <Dropdown
                                    options=model_options("Same as the role", None)
                                    value=Signal::derive(move || override_now().model.unwrap_or_default())
                                    on_change=Callback::new(move |value: String| change_override(&|entry| entry.model = some(value.clone())))
                                    label="Role model in this project"
                                    class=ROW_CONTROL
                                    align_end=true
                                />
                            </div>
                            <div class=FIELD>
                                <span class=FIELD_LABEL>"Effort in this project"</span>
                                <Dropdown
                                    options=effort_options("Same as the role")
                                    value=Signal::derive(move || override_now().effort.unwrap_or_default())
                                    on_change=Callback::new(move |value: String| change_override(&|entry| entry.effort = some(value.clone())))
                                    label="Role effort in this project"
                                    class=ROW_CONTROL
                                    align_end=true
                                />
                            </div>
                            <div class=FIELD>
                                <span class=FIELD_LABEL>"Access in this project"</span>
                                <Dropdown
                                    options=access_options("Same as the role")
                                    value=Signal::derive(move || access_value(override_now().access))
                                    on_change=Callback::new(move |value: String| change_override(&|entry| entry.access = parse_access(&value)))
                                    label="Role access in this project"
                                    class=ROW_CONTROL
                                    align_end=true
                                />
                            </div>
                            <div class="flex flex-col gap-2 py-2">
                                <span class=FIELD_LABEL>"Instructions in this project (empty keeps the role's)"</span>
                                <textarea
                                    aria-label="Role instructions in this project"
                                    maxlength=MAX_INSTRUCTIONS_CHARS.to_string()
                                    class=TEXT_AREA
                                    prop:value=move || override_now().instructions.unwrap_or_default()
                                    on:change=move |ev| {
                                        let value = event_target_value(&ev);
                                        change_override(&|entry| entry.instructions = some(value.clone()));
                                    }
                                />
                            </div>
                        </div>
                    </Show>
                    <div class="flex justify-end py-2">
                        <button
                            type="button"
                            class=BUTTON
                            on:click=move |_| {
                                let id = selected.get_untracked();
                                book.update(|book| {
                                    book.remove(&id);
                                });
                                set_selected.set(String::new());
                            }
                        >
                            "Delete role"
                        </button>
                    </div>
                </div>
            </SettingsRowBody>
        </Show>
    }
}

/// Models for the jobs that are not threads.
#[component]
pub fn TaskModelSettings() -> impl IntoView {
    let TaskModelsSignal(models) = expect_context::<TaskModelsSignal>();
    view! {
        <SettingsRow
            title="Meeting summary model"
            description="The Claude model that writes meeting summaries. Summaries run on Claude."
        >
            <Dropdown
                options=model_options("Built-in default", Some(AgentProvider::Claude))
                value=Signal::derive(move || models.with(|models| models.summary.clone().unwrap_or_default()))
                on_change=Callback::new(move |value: String| models.update(|models| models.summary = some(value)))
                label="Meeting summary model"
                class=ROW_CONTROL
                align_end=true
            />
        </SettingsRow>
        <SettingsRow
            title="Second opinion by Claude"
            description="The model that reviews a Codex thread's answer when you run /second-opinion there."
        >
            <Dropdown
                options=model_options("Built-in default", Some(AgentProvider::Claude))
                value=Signal::derive(move || models.with(|models| models.review_claude.clone().unwrap_or_default()))
                on_change=Callback::new(move |value: String| models.update(|models| models.review_claude = some(value)))
                label="Second opinion by Claude"
                class=ROW_CONTROL
                align_end=true
            />
        </SettingsRow>
        <SettingsRow
            title="Second opinion by Codex"
            description="The model that reviews a Claude thread's answer when you run /second-opinion there."
        >
            <Dropdown
                options=model_options("Your Codex default", Some(AgentProvider::Codex))
                value=Signal::derive(move || models.with(|models| models.review_codex.clone().unwrap_or_default()))
                on_change=Callback::new(move |value: String| models.update(|models| models.review_codex = some(value)))
                label="Second opinion by Codex"
                class=ROW_CONTROL
                align_end=true
            />
        </SettingsRow>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_role_is_never_offered_the_loosest_access() {
        let values: Vec<String> = access_options("Your access mode")
            .into_iter()
            .map(|option| option.value)
            .collect();
        assert_eq!(values, ["", "plan", "default", "acceptEdits"]);
        assert_eq!(parse_access(""), None);
        assert_eq!(parse_access("plan"), Some(PermissionMode::Plan));
        assert_eq!(
            access_value(Some(PermissionMode::AcceptEdits)),
            "acceptEdits"
        );
        assert_eq!(access_value(None), "");
    }

    #[test]
    fn task_models_offer_only_what_the_job_can_run() {
        let claude = model_options("Built-in default", Some(AgentProvider::Claude));
        assert_eq!(claude[0].value, "");
        assert!(claude[1..]
            .iter()
            .all(|option| option.value.starts_with("claude-")));
        let codex = model_options("Your Codex default", Some(AgentProvider::Codex));
        assert!(codex[1..]
            .iter()
            .all(|option| !option.value.starts_with("claude-")));
        let all = model_options("The sender's model", None);
        assert_eq!(all.len(), MODEL_CATALOG.len() + 1);
        assert_eq!(
            effort_options("Your default").len(),
            CLAUDE_EFFORTS.len() + 1
        );
    }
}

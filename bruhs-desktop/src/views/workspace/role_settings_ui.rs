//! Settings rows for roles (named model setups an agent starts a thread
//! with) and for the models of jobs that are not threads. Web:
//! `components/role_settings.rs`. The rules live in `bruhs_models::roles`.

use super::*;
use bruhs_models::agent::{AgentProvider, MODEL_CATALOG};
use bruhs_models::logic::run_place::CLAUDE_EFFORTS;
use bruhs_models::roles::{Role as ThreadRole, RoleOverride, MAX_ROLES};
use gpui_component::input::Input;

pub(super) const SETTINGS_ROLE_DROPDOWN_ID: &str = "desktop-role";
pub(super) const SETTINGS_ROLE_MODEL_DROPDOWN_ID: &str = "desktop-role-model";
pub(super) const SETTINGS_ROLE_EFFORT_DROPDOWN_ID: &str = "desktop-role-effort";
pub(super) const SETTINGS_ROLE_ACCESS_DROPDOWN_ID: &str = "desktop-role-access";
pub(super) const SETTINGS_ROLE_PROJECT_MODEL_DROPDOWN_ID: &str = "desktop-role-project-model";
pub(super) const SETTINGS_ROLE_PROJECT_EFFORT_DROPDOWN_ID: &str = "desktop-role-project-effort";
pub(super) const SETTINGS_ROLE_PROJECT_ACCESS_DROPDOWN_ID: &str = "desktop-role-project-access";
pub(super) const SETTINGS_SUMMARY_MODEL_DROPDOWN_ID: &str = "desktop-summary-model";
pub(super) const SETTINGS_REVIEW_CLAUDE_DROPDOWN_ID: &str = "desktop-review-claude-model";
pub(super) const SETTINGS_REVIEW_CODEX_DROPDOWN_ID: &str = "desktop-review-codex-model";

/// Dropdown value that keeps what the control inherits.
const INHERIT: &str = "inherit";
/// Dropdown values for the roles, by position: a dropdown value is
/// `'static`, and a role id is not.
const ROLE_SLOTS: [&str; MAX_ROLES] =
    ["0", "1", "2", "3", "4", "5", "6", "7", "8", "9", "10", "11"];

/// The text fields of the role editor. One set, shared by whichever role
/// is open.
pub(super) struct RoleInputs {
    name: Entity<InputState>,
    use_for: Entity<InputState>,
    instructions: Entity<InputState>,
    /// Name of the project whose override is shown.
    project: Entity<InputState>,
    project_instructions: Entity<InputState>,
}

/// Writes one text field's value into a role.
type RoleWrite = fn(&mut ThreadRole, String);

/// A role can only narrow access, so the loosest mode is not offered: it
/// would never change anything.
const ROLE_ACCESS: [PermissionMode; 3] = [
    PermissionMode::Plan,
    PermissionMode::Default,
    PermissionMode::AcceptEdits,
];

fn option(value: &'static str, label: impl Into<String>) -> DropdownOption {
    DropdownOption {
        value,
        label: label.into(),
        description: None,
    }
}

fn flat(options: Vec<DropdownOption>) -> Vec<DropdownGroup> {
    vec![DropdownGroup {
        label: None,
        mark: None,
        options,
    }]
}

/// Every model `only` allows, after one option that keeps `inherit`.
fn model_groups(inherit: &str, only: Option<AgentProvider>) -> Vec<DropdownGroup> {
    flat(
        std::iter::once(option(INHERIT, inherit))
            .chain(
                MODEL_CATALOG
                    .iter()
                    .filter(|model| only.is_none_or(|only| only == model.provider))
                    .map(|model| option(model.id, model.label)),
            )
            .collect(),
    )
}

fn effort_groups(inherit: &str) -> Vec<DropdownGroup> {
    flat(
        std::iter::once(option(INHERIT, inherit))
            .chain(CLAUDE_EFFORTS.iter().map(|level| option(level, *level)))
            .collect(),
    )
}

fn access_groups(inherit: &str) -> Vec<DropdownGroup> {
    flat(
        std::iter::once(option(INHERIT, inherit))
            .chain(
                ROLE_ACCESS
                    .iter()
                    .map(|mode| option(mode.as_str(), mode.label())),
            )
            .collect(),
    )
}

/// The dropdown's choice as a stored value: `None` for "inherit".
fn chosen(value: &'static str) -> Option<String> {
    (value != INHERIT).then(|| value.to_string())
}

fn chosen_access(value: &'static str) -> Option<PermissionMode> {
    ROLE_ACCESS
        .iter()
        .copied()
        .find(|mode| mode.as_str() == value)
}

fn settings_button(
    id: &'static str,
    label: &'static str,
    disabled: bool,
    cx: &Context<'_, Workspace>,
    on_press: fn(&mut Workspace, &mut Window, &mut Context<'_, Workspace>),
) -> AnyElement {
    div()
        .id(id)
        .debug_selector(move || id.to_string())
        .h(px(40.0))
        .px(px(12.0))
        .rounded(px(8.0))
        .flex()
        .items_center()
        .justify_center()
        .text_size(px(14.0))
        .border_1()
        .border_color(gpui::white().opacity(0.1))
        .text_color(theme::rambutan_200())
        .tab_index(0)
        .role(Role::Button)
        .aria_label(label)
        .focus_visible(|el| el.border_2().border_color(theme::persimmon_400()))
        .when(disabled, |el| el.opacity(0.5))
        .when(!disabled, |el| {
            el.cursor_pointer()
                .hover(|el| {
                    el.bg(theme::longan_800().opacity(0.6))
                        .text_color(theme::rambutan_100())
                })
                .on_click(cx.listener(move |this, _ev, window, cx| {
                    cx.stop_propagation();
                    on_press(this, window, cx);
                }))
                .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
                    if is_web_button_activation_key(event.keystroke.key.as_str()) {
                        window.prevent_default();
                        cx.stop_propagation();
                        on_press(this, window, cx);
                    }
                }))
        })
        .child(label)
        .into_any_element()
}

impl Workspace {
    pub(super) fn new_role_inputs(
        window: &mut Window,
        cx: &mut Context<'_, Self>,
        subscriptions: &mut Vec<Subscription>,
    ) -> RoleInputs {
        let name = cx.new(|cx| InputState::new(window, cx).placeholder("Role name"));
        let use_for =
            cx.new(|cx| InputState::new(window, cx).placeholder("drafting docs and release notes"));
        let instructions = cx.new(|cx| {
            InputState::new(window, cx)
                .multi_line(true)
                .placeholder("You write user-facing docs. Keep sentences short.")
        });
        let project = cx.new(|cx| InputState::new(window, cx).placeholder("Project name"));
        let project_instructions = cx.new(|cx| {
            InputState::new(window, cx)
                .multi_line(true)
                .placeholder("Empty keeps the role's instructions")
        });
        let edits: [(&Entity<InputState>, RoleWrite); 3] = [
            (&name, |role, value| role.name = value),
            (&use_for, |role, value| role.use_for = value),
            (&instructions, |role, value| role.instructions = value),
        ];
        for (input, write) in edits {
            subscriptions.push(cx.subscribe_in(
                input,
                window,
                move |this, input, event: &InputEvent, _window, cx| {
                    if matches!(event, InputEvent::Change) {
                        let value = input.read(cx).value().to_string();
                        this.edit_role(cx, |role| write(role, value));
                    }
                },
            ));
        }
        subscriptions.push(cx.subscribe_in(
            &project,
            window,
            |this, _input, event: &InputEvent, window, cx| {
                if matches!(event, InputEvent::Change) {
                    this.load_role_project_instructions(window, cx);
                    cx.notify();
                }
            },
        ));
        subscriptions.push(cx.subscribe_in(
            &project_instructions,
            window,
            |this, input, event: &InputEvent, _window, cx| {
                if matches!(event, InputEvent::Change) {
                    let value = input.read(cx).value().to_string();
                    this.edit_role_override(cx, |change| change.instructions = Some(value));
                }
            },
        ));
        RoleInputs {
            name,
            use_for,
            instructions,
            project,
            project_instructions,
        }
    }

    /// The role open in Settings.
    fn selected_role(&self) -> Option<&ThreadRole> {
        let id = self.role_selected.as_deref()?;
        self.roles.roles.iter().find(|role| role.id == id)
    }

    /// The project whose override the editor shows; empty for none.
    fn role_project(&self, cx: &App) -> String {
        self.role_inputs.project.read(cx).value().trim().to_string()
    }

    /// Roles or task models changed: every thread picks up its role as it
    /// stands now, and the change is saved.
    fn roles_changed(&mut self, cx: &mut Context<'_, Self>) {
        crate::server_client::set_summary_model(self.task_models.summary.clone());
        self.stamp_run_prefs_on_threads(cx);
        self.persist_and_notify(cx);
    }

    /// Change the open role. Saves only when something changed, so loading
    /// a role into the fields does not write it back.
    fn edit_role(&mut self, cx: &mut Context<'_, Self>, change: impl FnOnce(&mut ThreadRole)) {
        let Some(id) = self.role_selected.clone() else {
            return;
        };
        let before = self.roles.clone();
        self.roles.update(&id, change);
        if self.roles != before {
            self.roles_changed(cx);
        }
    }

    /// Change what the shown project overrides for the open role.
    fn edit_role_override(
        &mut self,
        cx: &mut Context<'_, Self>,
        change: impl FnOnce(&mut RoleOverride),
    ) {
        let Some(id) = self.role_selected.clone() else {
            return;
        };
        let project = self.role_project(cx);
        let before = self.roles.clone();
        self.roles.update_override(&project, &id, change);
        if self.roles != before {
            self.roles_changed(cx);
        }
    }

    /// Open `id` in the editor, or close the editor.
    fn select_role(&mut self, id: Option<String>, window: &mut Window, cx: &mut Context<'_, Self>) {
        self.role_selected = id;
        let role = self.selected_role().cloned().unwrap_or_default();
        let fields = [
            (self.role_inputs.name.clone(), role.name),
            (self.role_inputs.use_for.clone(), role.use_for),
            (self.role_inputs.instructions.clone(), role.instructions),
        ];
        for (input, value) in fields {
            input.update(cx, |field, cx| field.set_value(value, window, cx));
        }
        self.load_role_project_instructions(window, cx);
        cx.notify();
    }

    /// Show the open role's instructions in the shown project.
    fn load_role_project_instructions(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let project = self.role_project(cx);
        let text = self
            .role_selected
            .as_deref()
            .and_then(|id| self.roles.override_for(&project, id))
            .and_then(|change| change.instructions.clone())
            .unwrap_or_default();
        self.role_inputs
            .project_instructions
            .update(cx, |field, cx| field.set_value(text, window, cx));
    }

    fn add_role(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        if let Some(id) = self.roles.add("New role") {
            self.roles_changed(cx);
            self.select_role(Some(id), window, cx);
        }
    }

    fn delete_role(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let Some(id) = self.role_selected.clone() else {
            return;
        };
        if self.roles.remove(&id) {
            self.roles_changed(cx);
        }
        self.select_role(None, window, cx);
    }

    /// A choice in one of this file's dropdowns. `false` when `id` is not
    /// one of them.
    pub(super) fn select_role_setting(
        &mut self,
        id: &'static str,
        value: &'static str,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> bool {
        match id {
            SETTINGS_ROLE_DROPDOWN_ID => {
                let role = ROLE_SLOTS
                    .iter()
                    .position(|slot| *slot == value)
                    .and_then(|index| self.roles.roles.get(index))
                    .map(|role| role.id.clone());
                self.select_role(role, window, cx);
            }
            SETTINGS_ROLE_MODEL_DROPDOWN_ID => {
                self.edit_role(cx, |role| role.model = chosen(value))
            }
            SETTINGS_ROLE_EFFORT_DROPDOWN_ID => {
                self.edit_role(cx, |role| role.effort = chosen(value));
            }
            SETTINGS_ROLE_ACCESS_DROPDOWN_ID => {
                self.edit_role(cx, |role| role.access = chosen_access(value));
            }
            SETTINGS_ROLE_PROJECT_MODEL_DROPDOWN_ID => {
                self.edit_role_override(cx, |change| change.model = chosen(value));
            }
            SETTINGS_ROLE_PROJECT_EFFORT_DROPDOWN_ID => {
                self.edit_role_override(cx, |change| change.effort = chosen(value));
            }
            SETTINGS_ROLE_PROJECT_ACCESS_DROPDOWN_ID => {
                self.edit_role_override(cx, |change| change.access = chosen_access(value));
            }
            SETTINGS_SUMMARY_MODEL_DROPDOWN_ID => {
                self.set_task_models(cx, |models| models.summary = chosen(value));
            }
            SETTINGS_REVIEW_CLAUDE_DROPDOWN_ID => {
                self.set_task_models(cx, |models| models.review_claude = chosen(value));
            }
            SETTINGS_REVIEW_CODEX_DROPDOWN_ID => {
                self.set_task_models(cx, |models| models.review_codex = chosen(value));
            }
            _ => return false,
        }
        true
    }

    fn set_task_models(
        &mut self,
        cx: &mut Context<'_, Self>,
        change: impl FnOnce(&mut bruhs_models::roles::TaskModels),
    ) {
        let mut models = self.task_models.clone();
        change(&mut models);
        let models = models.sanitized();
        if models != self.task_models {
            self.task_models = models;
            self.roles_changed(cx);
        }
    }

    fn role_dropdown(
        &self,
        id: &'static str,
        label: &str,
        value: &str,
        groups: Vec<DropdownGroup>,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        let (toggle, select) = self.settings_dropdown_handlers(id, cx);
        dropdown(
            id,
            label,
            value,
            groups,
            self.settings_listbox(id),
            toggle,
            select,
        )
    }

    /// The Roles rows: pick or add a role, edit it, override it per project.
    pub(super) fn render_role_settings_rows(&self, cx: &Context<'_, Self>) -> Vec<AnyElement> {
        let selected_slot = self
            .selected_role()
            .and_then(|role| {
                self.roles
                    .roles
                    .iter()
                    .position(|other| other.id == role.id)
            })
            .map_or(INHERIT, |index| ROLE_SLOTS[index]);
        let role_groups = flat(
            std::iter::once(option(INHERIT, "Choose a role"))
                .chain(
                    self.roles
                        .roles
                        .iter()
                        .zip(ROLE_SLOTS)
                        .map(|(role, slot)| option(slot, role.name.clone())),
                )
                .collect(),
        );
        let mut rows = vec![self.settings_row(
            "desktop-settings-row-roles",
            "Roles",
            "Named setups an agent can start a thread with: a model, an effort, an access mode and instructions. An agent picks one by name when it messages a new thread, so one thread can orchestrate while others write or run on the models you chose.",
            settings_ui::control_column(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(self.role_dropdown(
                        SETTINGS_ROLE_DROPDOWN_ID,
                        "Role to edit",
                        selected_slot,
                        role_groups,
                        cx,
                    ))
                    .child(settings_button(
                        "desktop-settings-role-new",
                        "New role",
                        self.roles.roles.len() >= MAX_ROLES,
                        cx,
                        Self::add_role,
                    ))
                    .into_any_element(),
            ),
        )];
        let Some(role) = self.selected_role() else {
            return rows;
        };
        let field = |input: &Entity<InputState>, height: f32| {
            div()
                .h(px(height))
                .child(Input::new(input))
                .into_any_element()
        };
        rows.push(self.settings_row(
            "desktop-settings-row-role-name",
            "Role name",
            "Agents ask for the role by this name.",
            settings_ui::control_column(field(&self.role_inputs.name, 40.0)),
        ));
        rows.push(self.settings_row(
            "desktop-settings-row-role-use-for",
            "Use for",
            "One line shown to agents, so they know what to hand this role.",
            settings_ui::control_column(field(&self.role_inputs.use_for, 40.0)),
        ));
        rows.push(self.settings_row(
            "desktop-settings-row-role-model",
            "Role model",
            "The model a thread with this role starts on.",
            settings_ui::control_column(self.role_dropdown(
                SETTINGS_ROLE_MODEL_DROPDOWN_ID,
                "Role model",
                role.model.as_deref().unwrap_or(INHERIT),
                model_groups("The sender's model", None),
                cx,
            )),
        ));
        rows.push(self.settings_row(
            "desktop-settings-row-role-effort",
            "Role effort",
            "Claude effort for this role's threads. Codex ignores it.",
            settings_ui::control_column(self.role_dropdown(
                SETTINGS_ROLE_EFFORT_DROPDOWN_ID,
                "Role effort",
                role.effort.as_deref().unwrap_or(INHERIT),
                effort_groups("Your default"),
                cx,
            )),
        ));
        rows.push(self.settings_row(
            "desktop-settings-row-role-access",
            "Role access",
            "A role can only narrow your access mode, never widen it.",
            settings_ui::control_column(self.role_dropdown(
                SETTINGS_ROLE_ACCESS_DROPDOWN_ID,
                "Role access",
                role.access.map_or(INHERIT, |mode| mode.as_str()),
                access_groups("Your access mode"),
                cx,
            )),
        ));
        rows.push(
            self.settings_row(
                "desktop-settings-row-role-instructions",
                "Role instructions",
                "Added to every turn of a thread with this role.",
                settings_ui::control_column(
                    div()
                        .w(px(288.0))
                        .child(field(&self.role_inputs.instructions, 96.0))
                        .into_any_element(),
                ),
            ),
        );
        let project = self.role_project(cx);
        let change = self
            .roles
            .override_for(&project, &role.id)
            .cloned()
            .unwrap_or_default();
        let mut override_column = div()
            .w(px(288.0))
            .flex()
            .flex_col()
            .gap_2()
            .child(field(&self.role_inputs.project, 40.0));
        if !project.is_empty() {
            override_column = override_column
                .child(self.role_dropdown(
                    SETTINGS_ROLE_PROJECT_MODEL_DROPDOWN_ID,
                    "Role model in this project",
                    change.model.as_deref().unwrap_or(INHERIT),
                    model_groups("Model: same as the role", None),
                    cx,
                ))
                .child(self.role_dropdown(
                    SETTINGS_ROLE_PROJECT_EFFORT_DROPDOWN_ID,
                    "Role effort in this project",
                    change.effort.as_deref().unwrap_or(INHERIT),
                    effort_groups("Effort: same as the role"),
                    cx,
                ))
                .child(self.role_dropdown(
                    SETTINGS_ROLE_PROJECT_ACCESS_DROPDOWN_ID,
                    "Role access in this project",
                    change.access.map_or(INHERIT, |mode| mode.as_str()),
                    access_groups("Access: same as the role"),
                    cx,
                ))
                .child(field(&self.role_inputs.project_instructions, 80.0));
        }
        rows.push(self.settings_row(
            "desktop-settings-row-role-project",
            "Project override",
            "Type a project's name to change this role there. What you leave alone keeps the role's value.",
            settings_ui::control_column(override_column.into_any_element()),
        ));
        rows.push(self.settings_row(
            "desktop-settings-row-role-delete",
            "Delete role",
            "Threads that had it keep their model and lose its instructions.",
            settings_ui::control_column(settings_button(
                "desktop-settings-role-delete",
                "Delete role",
                false,
                cx,
                Self::delete_role,
            )),
        ));
        rows
    }

    /// Models for the jobs that are not threads.
    pub(super) fn render_task_model_rows(&self, cx: &Context<'_, Self>) -> Vec<AnyElement> {
        vec![
            self.settings_row(
                "desktop-settings-row-summary-model",
                "Meeting summary model",
                "The Claude model that writes meeting summaries. Summaries run on Claude.",
                settings_ui::control_column(self.role_dropdown(
                    SETTINGS_SUMMARY_MODEL_DROPDOWN_ID,
                    "Meeting summary model",
                    self.task_models.summary.as_deref().unwrap_or(INHERIT),
                    model_groups("Built-in default", Some(AgentProvider::Claude)),
                    cx,
                )),
            ),
            self.settings_row(
                "desktop-settings-row-review-claude",
                "Second opinion by Claude",
                "The model that reviews a Codex thread's answer when you run /second-opinion there.",
                settings_ui::control_column(self.role_dropdown(
                    SETTINGS_REVIEW_CLAUDE_DROPDOWN_ID,
                    "Second opinion by Claude",
                    self.task_models.review_claude.as_deref().unwrap_or(INHERIT),
                    model_groups("Built-in default", Some(AgentProvider::Claude)),
                    cx,
                )),
            ),
            self.settings_row(
                "desktop-settings-row-review-codex",
                "Second opinion by Codex",
                "The model that reviews a Claude thread's answer when you run /second-opinion there.",
                settings_ui::control_column(self.role_dropdown(
                    SETTINGS_REVIEW_CODEX_DROPDOWN_ID,
                    "Second opinion by Codex",
                    self.task_models.review_codex.as_deref().unwrap_or(INHERIT),
                    model_groups("Your Codex default", Some(AgentProvider::Codex)),
                    cx,
                )),
            ),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::{
        access_groups, chosen, chosen_access, model_groups, AgentProvider, PermissionMode, INHERIT,
        MAX_ROLES, MODEL_CATALOG, ROLE_SLOTS,
    };

    #[test]
    fn a_role_is_never_offered_the_loosest_access() {
        let values: Vec<&str> = access_groups("Your access mode")[0]
            .options
            .iter()
            .map(|option| option.value)
            .collect();
        assert_eq!(values, [INHERIT, "plan", "default", "acceptEdits"]);
        assert_eq!(chosen_access(INHERIT), None);
        assert_eq!(chosen_access("bypassPermissions"), None);
        assert_eq!(chosen_access("plan"), Some(PermissionMode::Plan));
        assert_eq!(chosen(INHERIT), None);
        assert_eq!(chosen("high").as_deref(), Some("high"));
    }

    #[test]
    fn task_models_offer_only_what_the_job_can_run_and_every_role_has_a_slot() {
        let claude = &model_groups("Built-in default", Some(AgentProvider::Claude))[0].options;
        assert_eq!(claude[0].value, INHERIT);
        assert!(claude[1..]
            .iter()
            .all(|option| option.value.starts_with("claude-")));
        let all = &model_groups("The sender's model", None)[0].options;
        assert_eq!(all.len(), MODEL_CATALOG.len() + 1);
        assert_eq!(ROLE_SLOTS.len(), MAX_ROLES);
    }
}

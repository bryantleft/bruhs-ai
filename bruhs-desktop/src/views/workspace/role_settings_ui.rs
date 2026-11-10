//! Settings rows for roles (named model setups an agent starts a thread
//! with) and for the models of jobs that are not threads. Web:
//! `components/role_settings.rs`, whose layout this follows: the Roles row,
//! then the open role's fields in a body under it. The rules live in
//! `bruhs_models::roles`.

use super::*;
use bruhs_models::agent::{AgentProvider, MODEL_CATALOG};
use bruhs_models::logic::run_place::CLAUDE_EFFORTS;
use bruhs_models::roles::{Role as ThreadRole, RoleOverride, MAX_ROLES};
use gpui_component::input::Input;

pub(super) const SETTINGS_ROLE_DROPDOWN_ID: &str = "desktop-role";
pub(super) const SETTINGS_ROLE_MODEL_DROPDOWN_ID: &str = "desktop-role-model";
pub(super) const SETTINGS_ROLE_EFFORT_DROPDOWN_ID: &str = "desktop-role-effort";
pub(super) const SETTINGS_ROLE_ACCESS_DROPDOWN_ID: &str = "desktop-role-access";
pub(super) const SETTINGS_ROLE_PROJECT_DROPDOWN_ID: &str = "desktop-role-project";
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

/// Web `BUTTON` and `TEXT_INPUT`: `h-8`, and `text-label-sm` on the button.
const CONTROL_HEIGHT_PX: f32 = 32.0;
const LABEL_TEXT_PX: f32 = 12.0;
/// Web `text-body-sm` in a field, with `px-3`.
const FIELD_TEXT_PX: f32 = 14.0;
const FIELD_X_PADDING_PX: f32 = 12.0;
/// Web `TEXT_AREA`: `min-h-[6rem]`.
const TEXT_AREA_HEIGHT_PX: f32 = 96.0;
/// Web `divide-longan-800/60` between a role's fields.
const FIELD_DIVIDER_OPACITY: f32 = 0.6;
/// Web `ps-4` on a project's overrides.
const OVERRIDE_INDENT_PX: f32 = 16.0;

/// The text fields of the role editor. One set, shared by whichever role
/// is open.
pub(super) struct RoleInputs {
    name: Entity<InputState>,
    use_for: Entity<InputState>,
    instructions: Entity<InputState>,
    /// Name of the project whose override is shown; empty for none.
    project: String,
    project_instructions: Entity<InputState>,
}

/// A dropdown value is `'static` and a project's name is not, so each
/// distinct name is kept once for the life of the app.
fn project_value(name: &str) -> &'static str {
    use std::collections::BTreeSet;
    use std::sync::{Mutex, PoisonError};
    static VALUES: Mutex<BTreeSet<&'static str>> = Mutex::new(BTreeSet::new());
    let mut values = VALUES.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some(known) = values.get(name) {
        return known;
    }
    let kept: &'static str = Box::leak(name.to_string().into_boxed_str());
    values.insert(kept);
    kept
}

/// Web `FIELD_LABEL`: `text-label-sm text-rambutan-300`.
fn field_label(text: &'static str) -> Div {
    div()
        .font_family(theme::UI_SMALL_FONT_FAMILY)
        .text_size(px(LABEL_TEXT_PX))
        .font_weight(FontWeight::MEDIUM)
        .line_height(relative(1.5))
        .text_color(theme::rambutan_300())
        .child(text)
}

/// A hairline above every field but the first (web `divide-y`).
fn divided(index: usize, row: Div) -> Div {
    row.when(index > 0, |el| {
        el.border_t_1()
            .border_color(theme::longan_800().opacity(FIELD_DIVIDER_OPACITY))
    })
}

/// Web `FIELD`: the label at the left, a row-width control at the right.
fn field_row(index: usize, label: &'static str, control: AnyElement) -> AnyElement {
    divided(
        index,
        div()
            .py_2()
            .flex()
            .items_center()
            .justify_between()
            .gap_3()
            .child(field_label(label))
            .child(settings_ui::control_column(control)),
    )
    .into_any_element()
}

/// A label over a full-width text area (web: the instructions fields).
fn text_area_row(
    index: usize,
    label: &'static str,
    name: &'static str,
    input: &Entity<InputState>,
) -> AnyElement {
    divided(
        index,
        div()
            .py_2()
            .flex()
            .flex_col()
            .gap_2()
            .child(field_label(label))
            .child(
                Input::new(input)
                    .h(px(TEXT_AREA_HEIGHT_PX))
                    .px(px(FIELD_X_PADDING_PX))
                    .text_size(px(FIELD_TEXT_PX))
                    .aria_label(name),
            ),
    )
    .into_any_element()
}

/// Web `TEXT_INPUT`: one line, the height of a dropdown.
fn text_field(input: &Entity<InputState>, name: &'static str) -> AnyElement {
    // `Input::h` (inherent) only sizes multi-line inputs; the `Styled`
    // height is refined last, so it sizes a single-line field.
    Styled::h(Input::new(input), px(CONTROL_HEIGHT_PX))
        .px(px(FIELD_X_PADDING_PX))
        .text_size(px(FIELD_TEXT_PX))
        .aria_label(name)
        .into_any_element()
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

impl Workspace {
    /// `/role [name]` typed in `chat`: show, set or remove that thread's
    /// role (web: `apply_role`). A role's model becomes the thread's; its
    /// effort, access and instructions are read each turn, so a running turn
    /// keeps what it has.
    pub(super) fn apply_role_command(
        &mut self,
        chat: &Entity<ChatView>,
        argument: &str,
        cx: &mut Context<'_, Self>,
    ) {
        use bruhs_models::roles::{role_command, RoleWrite};
        let (project, current) = {
            let view = chat.read(cx);
            (view.project_name.clone(), view.role.clone())
        };
        let command = role_command(&self.roles, &project, current.as_deref(), argument);
        let notice = command.notice;
        let role = match command.write {
            RoleWrite::Keep => {
                self.meetings_hub
                    .update(cx, |hub, cx| hub.show_notice(notice, false, cx));
                return;
            }
            RoleWrite::Set(role) => Some(role),
            RoleWrite::Remove => None,
        };
        let model = role.as_ref().and_then(|role| role.model.clone());
        chat.update(cx, |chat, cx| {
            chat.role = role.as_ref().map(|role| role.id.clone());
            chat.role_profile = role;
            if let Some(model) = model {
                chat.set_model(model, cx);
            }
            cx.notify();
        });
        self.meetings_hub
            .update(cx, |hub, cx| hub.show_notice(notice, false, cx));
        self.persist_and_notify(cx);
    }
}

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
        .h(px(CONTROL_HEIGHT_PX))
        .px(px(12.0))
        .rounded(px(8.0))
        .flex()
        .flex_shrink_0()
        .items_center()
        .justify_center()
        .font_family(theme::UI_SMALL_FONT_FAMILY)
        .text_size(px(LABEL_TEXT_PX))
        .font_weight(FontWeight::MEDIUM)
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
        let project_instructions = cx.new(|cx| InputState::new(window, cx).multi_line(true));
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
            project: String::new(),
            project_instructions,
        }
    }

    /// The role open in Settings.
    fn selected_role(&self) -> Option<&ThreadRole> {
        let id = self.role_selected.as_deref()?;
        self.roles.roles.iter().find(|role| role.id == id)
    }

    /// The project whose override the editor shows; empty for none.
    fn role_project(&self) -> String {
        self.role_inputs.project.clone()
    }

    /// The projects a role can be overridden in: every project a thread is
    /// in, by name (web: the same list).
    fn role_project_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self
            .threads
            .iter()
            .filter(|thread| !thread.project_name.is_empty())
            .map(|thread| thread.project_name.clone())
            .collect();
        names.sort();
        names.dedup();
        names
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
        let project = self.role_project();
        let before = self.roles.clone();
        self.roles.update_override(&project, &id, change);
        if self.roles != before {
            self.roles_changed(cx);
        }
    }

    /// Open `id` in the editor, or close the editor.
    pub(super) fn select_role(
        &mut self,
        id: Option<String>,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
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
        let project = self.role_project();
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
            SETTINGS_ROLE_PROJECT_DROPDOWN_ID => {
                self.role_inputs.project = chosen(value).unwrap_or_default();
                self.load_role_project_instructions(window, cx);
                cx.notify();
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
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(settings_ui::control_column(self.role_dropdown(
                    SETTINGS_ROLE_DROPDOWN_ID,
                    "Role to edit",
                    selected_slot,
                    role_groups,
                    cx,
                )))
                .child(settings_button(
                    "desktop-settings-role-new",
                    "New role",
                    self.roles.roles.len() >= MAX_ROLES,
                    cx,
                    Self::add_role,
                ))
                .into_any_element(),
        )];
        let Some(role) = self.selected_role() else {
            return rows;
        };
        let project = self.role_project();
        let project_groups = flat(
            std::iter::once(option(INHERIT, "No project"))
                .chain(
                    self.role_project_names()
                        .into_iter()
                        .map(|name| option(project_value(&name), name)),
                )
                .collect(),
        );
        let mut editor = div()
            .debug_selector(|| "desktop-settings-role-editor".to_string())
            .px(px(SETTINGS_ROW_X_PADDING_PX))
            .pb_4()
            .flex()
            .flex_col()
            .child(field_row(
                0,
                "Name",
                text_field(&self.role_inputs.name, "Role name"),
            ))
            .child(field_row(
                1,
                "Use for (shown to agents)",
                text_field(&self.role_inputs.use_for, "What to use this role for"),
            ))
            .child(field_row(
                2,
                "Model",
                self.role_dropdown(
                    SETTINGS_ROLE_MODEL_DROPDOWN_ID,
                    "Role model",
                    role.model.as_deref().unwrap_or(INHERIT),
                    model_groups("The sender's model", None),
                    cx,
                ),
            ))
            .child(field_row(
                3,
                "Effort (Claude)",
                self.role_dropdown(
                    SETTINGS_ROLE_EFFORT_DROPDOWN_ID,
                    "Role effort",
                    role.effort.as_deref().unwrap_or(INHERIT),
                    effort_groups("Your default"),
                    cx,
                ),
            ))
            .child(field_row(
                4,
                "Access (can only narrow yours)",
                self.role_dropdown(
                    SETTINGS_ROLE_ACCESS_DROPDOWN_ID,
                    "Role access",
                    role.access.map_or(INHERIT, |mode| mode.as_str()),
                    access_groups("Your access mode"),
                    cx,
                ),
            ))
            .child(text_area_row(
                5,
                "Instructions (added to every turn of the thread)",
                "Role instructions",
                &self.role_inputs.instructions,
            ))
            .child(field_row(
                6,
                "Project override",
                self.role_dropdown(
                    SETTINGS_ROLE_PROJECT_DROPDOWN_ID,
                    "Project that overrides this role",
                    if project.is_empty() {
                        INHERIT
                    } else {
                        project_value(&project)
                    },
                    project_groups,
                    cx,
                ),
            ));
        if !project.is_empty() {
            let change = self
                .roles
                .override_for(&project, &role.id)
                .cloned()
                .unwrap_or_default();
            editor = editor.child(divided(
                7,
                div()
                    .debug_selector(|| "desktop-settings-role-override".to_string())
                    .pl(px(OVERRIDE_INDENT_PX))
                    .flex()
                    .flex_col()
                    .child(field_row(
                        0,
                        "Model in this project",
                        self.role_dropdown(
                            SETTINGS_ROLE_PROJECT_MODEL_DROPDOWN_ID,
                            "Role model in this project",
                            change.model.as_deref().unwrap_or(INHERIT),
                            model_groups("Same as the role", None),
                            cx,
                        ),
                    ))
                    .child(field_row(
                        1,
                        "Effort in this project",
                        self.role_dropdown(
                            SETTINGS_ROLE_PROJECT_EFFORT_DROPDOWN_ID,
                            "Role effort in this project",
                            change.effort.as_deref().unwrap_or(INHERIT),
                            effort_groups("Same as the role"),
                            cx,
                        ),
                    ))
                    .child(field_row(
                        2,
                        "Access in this project",
                        self.role_dropdown(
                            SETTINGS_ROLE_PROJECT_ACCESS_DROPDOWN_ID,
                            "Role access in this project",
                            change.access.map_or(INHERIT, |mode| mode.as_str()),
                            access_groups("Same as the role"),
                            cx,
                        ),
                    ))
                    .child(text_area_row(
                        3,
                        "Instructions in this project (empty keeps the role's)",
                        "Role instructions in this project",
                        &self.role_inputs.project_instructions,
                    )),
            ));
        }
        rows.push(
            editor
                .child(divided(
                    8,
                    div().py_2().flex().justify_end().child(settings_button(
                        "desktop-settings-role-delete",
                        "Delete role",
                        false,
                        cx,
                        Self::delete_role,
                    )),
                ))
                .into_any_element(),
        );
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

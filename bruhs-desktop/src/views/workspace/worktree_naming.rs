use super::project_defaults::ProjectDefaults;
use super::setting_source::ProjectSetting;
use super::*;
use bruhs_models::logic::run_place::branch_naming::{Mode, Naming};

#[derive(Clone, Copy)]
enum Field {
    Mode,
    Prefix,
    Instructions,
}

impl Field {
    fn value(self, naming: &Naming) -> &str {
        match self {
            Self::Mode => naming.mode.key(),
            Self::Prefix => &naming.prefix,
            Self::Instructions => &naming.instructions,
        }
    }
    fn display(value: &str) -> SharedString {
        if value.is_empty() {
            "Empty".into()
        } else {
            value.to_owned().into()
        }
    }
}

impl ProjectSetting for Field {
    fn project_label(self, project: &ProjectDefaults) -> Option<SharedString> {
        match self {
            Self::Mode => project.branch_mode.map(|m| m.key().into()),
            Self::Prefix => project.branch_prefix.as_deref().map(Self::display),
            Self::Instructions => project.branch_instructions.as_deref().map(Self::display),
        }
    }
    fn clear_project(self, prefs: &mut ProjectDefaults) {
        match self {
            Self::Mode => prefs.branch_mode = None,
            Self::Prefix => prefs.branch_prefix = None,
            Self::Instructions => prefs.branch_instructions = None,
        }
    }
    fn before_reset(self, ws: &mut Workspace, w: &mut Window, cx: &mut Context<'_, Workspace>) {
        ws.commit_branch_fields(w, cx);
    }
    fn after_reset(self, ws: &mut Workspace, w: &mut Window, cx: &mut Context<'_, Workspace>) {
        ws.load_branch_fields(w, cx);
        ws.stamp_run_prefs_on_threads(cx);
        ws.persist_and_notify(cx);
    }
}

impl Workspace {
    pub(super) fn effective_branch_naming(&self, scope: &str) -> Naming {
        let mut naming = self.branch_naming.clone();
        if let Some(prefs) = self.project_defaults.get(scope) {
            if let Some(mode) = prefs.branch_mode {
                naming.mode = mode;
            }
            if let Some(prefix) = &prefs.branch_prefix {
                naming.prefix.clone_from(prefix);
            }
            if let Some(instructions) = &prefs.branch_instructions {
                naming.instructions.clone_from(instructions);
            }
        }
        naming
    }

    pub(super) fn new_branch_fields(
        naming: &Naming,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
        subscriptions: &mut Vec<Subscription>,
    ) -> (Entity<InputState>, Entity<TextareaState>) {
        let prefix = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("No prefix")
                .default_value(naming.prefix.clone())
        });
        let instructions = cx.new(|cx| {
            TextareaState::new(window, cx)
                .rows(4)
                .placeholder("Use team/ followed by the issue ID and a short description.")
                .default_value(naming.instructions.clone())
        });
        subscriptions.push(
            cx.subscribe_in(&prefix, window, |this: &mut Self, _, event, w, cx| {
                if matches!(event, InputEvent::Change) {
                    this.branch_prefix_edited = true;
                }
                if matches!(event, InputEvent::Blur | InputEvent::PressEnter { .. }) {
                    this.commit_branch_fields(w, cx);
                }
            }),
        );
        subscriptions.push(cx.subscribe_in(
            &instructions,
            window,
            |this: &mut Self, _, event, w, cx| {
                if matches!(event, InputEvent::Change) {
                    this.branch_instructions_edited = true;
                }
                if matches!(event, InputEvent::Blur) {
                    this.commit_branch_fields(w, cx);
                }
            },
        ));
        (prefix, instructions)
    }

    pub(super) fn load_branch_fields(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        self.branch_editor_scope
            .clone_from(&self.settings_project_scope);
        let naming = self.effective_branch_naming(&self.branch_editor_scope);
        self.branch_prefix_input
            .update(cx, |field, cx| field.set_value(naming.prefix, window, cx));
        self.branch_instructions_input.update(cx, |field, cx| {
            field.set_value(naming.instructions, window, cx)
        });
    }

    pub(super) fn commit_branch_fields(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let prefix_edited = std::mem::take(&mut self.branch_prefix_edited);
        let instructions_edited = std::mem::take(&mut self.branch_instructions_edited);
        if !prefix_edited && !instructions_edited {
            return;
        }
        let scope = &self.branch_editor_scope;
        let effective = self.effective_branch_naming(scope);
        let raw_prefix = self.branch_prefix_input.read(cx).value();
        let raw_instructions = self.branch_instructions_input.read(cx).value();
        let prefix = if prefix_edited {
            raw_prefix.trim()
        } else {
            &effective.prefix
        };
        let instructions = if instructions_edited {
            raw_instructions.trim()
        } else {
            &effective.instructions
        };
        if prefix != raw_prefix.as_ref() {
            self.branch_prefix_input.update(cx, |field, cx| {
                field.set_value(prefix.to_owned(), window, cx)
            });
        }
        if instructions != raw_instructions.as_ref() {
            self.branch_instructions_input.update(cx, |field, cx| {
                field.set_value(instructions.to_owned(), window, cx)
            });
        }
        if effective.prefix == prefix && effective.instructions == instructions {
            return;
        }
        if scope.is_empty() {
            self.branch_naming.prefix = prefix.to_owned();
            self.branch_naming.instructions = instructions.to_owned();
        } else {
            let prefs = self.project_defaults.entry(scope.clone()).or_default();
            if prefix != effective.prefix {
                prefs.branch_prefix = Some(prefix.to_owned());
            }
            if instructions != effective.instructions {
                prefs.branch_instructions = Some(instructions.to_owned());
            }
        }
        self.stamp_run_prefs_on_threads(cx);
        self.persist_and_notify(cx);
    }

    pub(super) fn choose_branch_naming(&mut self, value: &str, cx: &mut Context<'_, Self>) {
        let Some(mode) = Mode::parse(value) else {
            return;
        };
        if self.settings_project_scope.is_empty() {
            self.branch_naming.mode = mode;
        } else {
            self.project_defaults
                .entry(self.settings_project_scope.clone())
                .or_default()
                .branch_mode = Some(mode);
        }
        self.stamp_run_prefs_on_threads(cx);
        self.persist_and_notify(cx);
    }

    fn branch_source(
        &self,
        field: Field,
        row: &'static str,
        label: &'static str,
        cx: &Context<'_, Self>,
    ) -> Option<AnyElement> {
        let defaults = Naming::default();
        let default = field.value(&defaults);
        let environment = field.value(&self.branch_naming);
        let changed = default != environment;
        let source = self.setting_source(
            field,
            row,
            row,
            label,
            changed.then(|| Field::display(environment)),
            Field::display(default),
            cx,
        );
        let reset = if self.settings_project_scope.is_empty() {
            changed
        } else {
            self.project_defaults
                .get(&self.settings_project_scope)
                .and_then(|p| field.project_label(p))
                .is_some()
        };
        if !reset {
            return Some(source);
        }
        let (id, label) = match field {
            Field::Mode => ("reset-branch-naming", "Reset branch naming to default"),
            Field::Prefix => ("reset-branch-prefix", "Reset branch prefix to default"),
            Field::Instructions => (
                "reset-branch-instructions",
                "Reset branch naming instructions to default",
            ),
        };
        let button = web_button(
            id,
            WebButtonVariant::Ghost,
            WebButtonSize::IconSmall,
            ButtonState::Enabled,
        )
        .debug_selector(move || id.into())
        .aria_label(label)
        .child(
            svg()
                .path(assets::ROTATE_CCW_ICON_PATH)
                .size(px(14.))
                .text_color(theme::rambutan_400()),
        );
        let reset = on_press_view(button, cx, move |this, w, cx| {
            this.commit_branch_fields(w, cx);
            let scope = &this.settings_project_scope;
            if scope.is_empty() {
                let defaults = Naming::default();
                match field {
                    Field::Mode => this.branch_naming.mode = defaults.mode,
                    Field::Prefix => this.branch_naming.prefix = defaults.prefix,
                    Field::Instructions => this.branch_naming.instructions.clear(),
                }
            } else if let Some(p) = this.project_defaults.get_mut(scope) {
                field.clear_project(p);
                if *p == Default::default() {
                    this.project_defaults.remove(scope);
                }
            }
            field.after_reset(this, w, cx);
        });
        Some(
            div()
                .flex()
                .items_center()
                .child(source)
                .child(reset)
                .into_any_element(),
        )
    }

    pub(super) fn branch_naming_rows(&self, cx: &Context<'_, Self>) -> Vec<AnyElement> {
        let naming = self.effective_branch_naming(&self.settings_project_scope);
        let mode = self.choice_control(
            "settings-branch-naming",
            "Worktree branch naming",
            naming.mode.key(),
            &[
                ("static", "Static prefix"),
                ("semantic", "Semantic prefix"),
                ("custom", "Custom instructions"),
                ("thread_id", "Thread ID"),
            ],
            cx,
        );
        let mut rows = vec![
            self.settings_row_with_reset(
                "worktree-naming-mode",
                "Worktree branch naming",
                "Choose how a new thread names its Git branch from the first message.",
                (naming.mode == Mode::Semantic)
                    .then(|| "The model chooses a prefix such as feat/, fix/ or refactor/.".into()),
                false,
                Some(settings_ui::control_column(mode)),
                self.branch_source(
                    Field::Mode,
                    "worktree-naming-mode",
                    "Worktree branch naming",
                    cx,
                ),
            ),
        ];
        match naming.mode {
            Mode::Static => rows.push(
                self.settings_row_with_reset(
                    "worktree-naming-prefix",
                    "Branch prefix",
                    "For example, bruhs/ produces bruhs/add-search. Leave empty for no prefix.",
                    None,
                    false,
                    Some(settings_ui::control_column(
                        Input::new(&self.branch_prefix_input)
                            .aria_label("Branch prefix")
                            .into_any_element(),
                    )),
                    self.branch_source(
                        Field::Prefix,
                        "worktree-naming-prefix",
                        "Branch prefix",
                        cx,
                    ),
                ),
            ),
            Mode::Custom => {
                let heading = self.settings_row_with_reset(
                    "worktree-naming-instructions",
                    "Branch naming instructions",
                    "Guide the model’s complete branch name, including its prefix, spelling and case.",
                    None,
                    false,
                    None,
                    self.branch_source(Field::Instructions, "worktree-naming-instructions", "Branch naming instructions", cx),
                );
                rows.push(
                    div()
                        .child(heading)
                        .child(
                            div()
                                .px_4()
                                .pb_3()
                                .debug_selector(|| "branch-instructions-editor".into())
                                .child(
                                    Textarea::new(&self.branch_instructions_input)
                                        .aria_label("Branch naming instructions"),
                                ),
                        )
                        .into_any_element(),
                );
            }
            _ => {}
        }
        rows
    }
}

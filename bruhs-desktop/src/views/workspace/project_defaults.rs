//! Folder-scoped defaults for new threads. Saved threads keep their actual model and permissions.
use super::*;
use bruhs_models::logic::run_place::WorktreeSubmodules;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub(super) struct ProjectDefaults {
    #[serde(default,skip_serializing_if="crate::views::chat::kiro::KiroChoices::is_empty")]
    pub kiro:crate::views::chat::kiro::KiroChoices,
    #[serde(skip_serializing_if="Option::is_none")]
    pub artwork:Option<super::project_art::Artwork>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if="GenerationOptions::is_empty")]
    pub model_options: GenerationOptions,
    #[serde(skip_serializing_if="Option::is_none")]
    pub text_generation_model: Option<String>,
    /// Zero disables inactivity-based settlement. A missing override inherits the environment.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub idle_settle_days: Option<u16>,
    #[serde(skip_serializing_if="Option::is_none")]
    pub settle_merged: Option<bool>,
    #[serde(skip_serializing_if="Option::is_none")]
    pub cleanup_deleted_worktrees:Option<bool>,
    #[serde(skip_serializing_if="Option::is_none")]
    pub cleanup_mode: Option<worktree_storage::CleanupMode>,
    #[serde(skip_serializing_if="Option::is_none")]
    pub cleanup_keep_when: Option<bruhs_models::coordination::worktree::KeepWhen>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worktree_retention: Option<worktree_storage::WorktreeRetention>,
    #[serde(skip_serializing_if="Option::is_none")]
    pub keep_pull: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auto_pull: Option<bool>,
    #[serde(skip_serializing_if="Option::is_none")]
    pub snooze_limited:Option<bool>,
    #[serde(skip_serializing_if="Option::is_none")]
    pub resume_limited:Option<bool>,
    #[serde(skip_serializing_if="Option::is_none")]
    pub continue_threads:Option<bool>,
    #[serde(skip_serializing_if="Option::is_none")]
    pub response_stream:Option<crate::views::chat::response_stream::Mode>,
    #[serde(skip_serializing_if="Option::is_none")]
    pub snooze_choices: Option<Vec<snooze_options::Choice>>,
    #[serde(
        with = "persisted_enum::optional",
        skip_serializing_if = "Option::is_none"
    )]
    pub permissions: Option<PermissionMode>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branch_mode: Option<bruhs_models::logic::run_place::branch_naming::Mode>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branch_prefix: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branch_instructions: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub submodules: Option<WorktreeSubmodules>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_from_remote: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub placement: Option<WorkspaceChoice>,
}

#[derive(Clone, Copy)]
pub(super) struct DefaultModel;

impl super::setting_source::ProjectSetting for DefaultModel {
    fn project_label(self, prefs: &ProjectDefaults) -> Option<SharedString> {
        prefs.model.as_deref().map(|id| {
            model_info(id)
                .map_or(id, |model| model.label)
                .to_owned()
                .into()
        })
    }

    fn clear_project(self, prefs: &mut ProjectDefaults) {
        prefs.model = None;
        prefs.model_options=Default::default();
    }

    fn after_reset(self, ws: &mut Workspace, _: &mut Window, cx: &mut Context<'_, Workspace>) {
        ws.persist_and_notify(cx);
    }
}

impl Workspace {
    pub(super) fn active_settings_project(&self, cx: &App) -> Option<String> {
        self.active_thread()
            .and_then(|thread| {
                let chat = thread.chat.read(cx);
                chat.project_scoped
                    .then(|| chat.settings_project_path().to_owned())
            })
            .or_else(|| self.active_project.as_ref().map(|p| p.path.clone()))
    }

    fn effective_settle_days(&self, path: &str) -> u16 {
        self.project_defaults
            .get(path)
            .and_then(|p| p.idle_settle_days)
            .unwrap_or(self.idle_settle_days)
            .min(365)
    }

    pub(super) fn idle_settle_after_ms(&self, path: &str) -> Option<i64> {
        let days = self.effective_settle_days(path);
        (days > 0).then(|| i64::from(days) * 86_400_000)
    }

    fn change_settle_days(&mut self, days: Option<u16>, cx: &mut Context<'_, Self>) {
        if self.settings_project_scope.is_empty() {
            if let Some(days) = days {
                self.idle_settle_days = days;
            }
        } else {
            let scope = self.settings_project_scope.clone();
            let prefs = self.project_defaults.entry(scope.clone()).or_default();
            prefs.idle_settle_days = days;
            if *prefs == Default::default() {
                self.project_defaults.remove(&scope);
            }
        }
        self.persist_and_notify(cx);
    }

    pub(super) fn response_stream_row(&self,cx:&Context<'_,Self>)->AnyElement {
        let scoped=!self.settings_project_scope.is_empty();
        let current=if scoped {self.project_defaults.get(&self.settings_project_scope).and_then(|p|p.response_stream).map_or("inherit",|m|m.key())}else{self.response_stream.key()};
        const MODES: &[(&str,&str)]=&[("inherit","Environment default"),("live","As text arrives"),("paragraph","Show finished paragraphs"),("turn","Wait for the full response")];
        self.settings_row("desktop-settings-row-response-stream","Response streaming","When assistant text appears during a turn.",settings_ui::control_column(self.choice_control("settings-response-streaming","Response streaming",current,if scoped {MODES}else{&MODES[1..]},cx)))
    }

    pub(super) fn render_settlement_rows(&self, cx: &Context<'_, Self>) -> Vec<AnyElement> {
        let scoped = !self.settings_project_scope.is_empty();
        let days = self.effective_settle_days(&self.settings_project_scope);
        let controls = if scoped {
            let value = match self
                .project_defaults
                .get(&self.settings_project_scope)
                .and_then(|p| p.idle_settle_days)
            {
                None => "inherit",
                Some(0) => "off",
                Some(_) => "on",
            };
            settings_ui::control_column(self.choice_control(
                "settings-inactive-settlement",
                "Auto-settle inactive threads",
                value,
                &[
                    ("inherit", "Use environment default"),
                    ("on", "On"),
                    ("off", "Off"),
                ],
                cx,
            ))
        } else {
            settings_ui::switch(
                "settings-settle-idle",
                days > 0,
                false,
                "Auto-settle inactive threads",
                cx,
                move |this, _, cx| this.change_settle_days(Some(if days > 0 { 0 } else { 3 }), cx),
            )
        };
        let mut rows = vec![self.settings_row(
            "desktop-settings-row-auto-settle",
            "Auto-settle inactive threads",
            "Move inactive threads to Settled.",
            controls,
        )];
        if days > 0 {
            let button = |id, label, change: u16, symbol| {
                let el = web_button(
                    id,
                    WebButtonVariant::Outline,
                    WebButtonSize::Small,
                    ButtonState::disabled_if(days == change),
                )
                .aria_label(label)
                .child(symbol);
                on_press_view(el, cx, move |this, _, cx| {
                    this.change_settle_days(Some(change), cx)
                })
            };
            let controls = div()
                .flex()
                .items_center()
                .gap_2()
                .child(button(
                    "settings-settle-shorter",
                    "Decrease inactivity period",
                    days.saturating_sub(1).max(1),
                    "−",
                ))
                .child(format!("{days}"))
                .child(button(
                    "settings-settle-longer",
                    "Increase inactivity period",
                    days.saturating_add(1).min(365),
                    "+",
                ));
            rows.push(self.settings_row(
                "desktop-settings-row-settle-days",
                "Days of inactivity before auto-settle",
                "New activity makes a thread active again.",
                controls.into_any_element(),
            ));
        }
        rows.extend(self.pull_settlement_rows(days > 0,cx));
        rows
    }

    pub(super) fn new_thread_defaults(&self, path: &str) -> (String, PermissionMode) {
        let prefs = self.project_defaults.get(path);
        let model = prefs
            .and_then(|p| p.model.as_ref())
            .unwrap_or(&self.current_model);
        (
            self.available_new_thread_model(path, model),
            prefs
                .and_then(|p| p.permissions)
                .unwrap_or(self.permission_mode),
        )
    }

    pub(super) fn available_new_thread_model(&self, path: &str, requested: &str) -> String {
        let preferred = self.model_for_new_thread(requested);
        let Ok(prefs) = bruhs_core::agent_extensions::availability::ProviderAvailability::load_for_project(path.as_ref())
        else { return preferred.to_owned() };
        let instances = std::cell::OnceCell::new();
        let automatic = !self.explicit_model && self.project_defaults.get(path).is_none_or(|p| p.model.is_none());
        self.enabled_new_thread_model(requested, automatic, &prefs, || {
            instances.get_or_init(|| crate::views::acp_models::load().unwrap_or_default()).as_slice()
        }).unwrap_or(requested).to_owned()
    }

    pub(super) fn settings_project_options(&self, cx: &App) -> Vec<DropdownOption> {
        let mut projects = std::collections::BTreeMap::new();
        for project in &self.recent_projects {
            projects.insert(project.path.as_str(), project.name.as_str());
        }
        for thread in &self.threads {
            if !thread.project_name.is_empty() {
                projects.insert(
                    thread.chat.read(cx).settings_project_path(),
                    thread.project_name.as_str(),
                );
            }
        }
        let listed = self.listed_projects();
        for project in &listed {
            projects.insert(&project.path, &project.name);
        }
        let saved_paths = self.project_defaults.keys().chain(self.project_actions.keys()).map(String::as_str);
        for path in saved_paths.chain(self.reactive_automations.read(cx).automation_project_paths())
            .chain(std::iter::once(self.settings_project_scope.as_str())).filter(|p| !p.is_empty())
        {
            projects.entry(path).or_insert_with(|| {
                std::path::Path::new(path)
                    .file_name()
                    .and_then(|x| x.to_str())
                    .unwrap_or(path)
            });
        }
        projects.into_iter().filter(|(path, _)| !path.is_empty()).map(|(path, name)| DropdownOption {
            value: role_settings_ui::project_value(path),
            label: name.into(),
            description: Some(path.into()),
        }).collect()
    }

    pub(super) fn defaults_scope_row(&self, cx: &Context<'_, Self>) -> AnyElement {
        let mut options = self.settings_project_options(cx);
        options.insert(0, DropdownOption {
            value: "",
            label: "This environment".into(),
            description: None,
        });
        let id = "settings-project-defaults-scope";
        let (toggle, select) = self.settings_dropdown_handlers(id, cx);
        let (title, description) = if self.settings_section == SettingsSection::ScheduledTasks {
            ("Project", "Filter tasks by environment or project.")
        } else {
            ("Project defaults and overrides", "Edit environment defaults or one project.")
        };
        self.settings_row(
            "desktop-settings-row-defaults-scope",
            title,
            description,
            settings_ui::control_column(dropdown(
                id,
                "Settings scope",
                &self.settings_project_scope,
                vec![DropdownGroup {
                    label: None,
                    mark: None,
                    options,
                }],
                self.settings_listbox(id),
                toggle,
                select,
            )),
        )
    }

    pub(super) fn select_project_default(
        &mut self,
        id: &str,
        value: &str,
        cx: &mut Context<'_, Self>,
    ) -> bool {
        if self.settings_project_scope.is_empty() {
            return false;
        }
        let scope = self.settings_project_scope.clone();
        let mut prefs = self
            .project_defaults
            .get(&scope)
            .cloned()
            .unwrap_or_default();
        match id {
            "settings-response-streaming" => {
                prefs.response_stream = if value=="inherit" {None} else {let Some(mode)=crate::views::chat::response_stream::Mode::parse(value) else {return false};Some(mode)};
            }
            "settings-auto-pull" => {
                let on = match value {
                    "inherit" => None,
                    "on" => Some(true),
                    "off" => Some(false),
                    _ => return false,
                };
                ScopedSwitch::AutoPull.choose(self, on, cx);
                return true;
            }
            "settings-inactive-settlement" => {
                let days = match value {
                    "inherit" => None,
                    "on" => Some(self.effective_settle_days(&scope).max(1)),
                    "off" => Some(0),
                    _ => return false,
                };
                self.change_settle_days(days, cx);
                return true;
            }
            SETTINGS_ACCESS_MODE_DROPDOWN_ID => {
                prefs.permissions = if value == "inherit" {
                    None
                } else {
                    value.parse().ok().or(prefs.permissions)
                };
            }
            "settings-worktree-remote" => {
                let on = match value {
                    "inherit" => None,
                    "on" => Some(true),
                    "off" => Some(false),
                    _ => return false,
                };
                ScopedSwitch::Remote.choose(self, on, cx);
                return true;
            }
            "settings-worktree-submodules" => {
                prefs.submodules = match value {
                    "inherit" => None,
                    "none" => Some(WorktreeSubmodules::None),
                    "top-level" => Some(WorktreeSubmodules::TopLevel),
                    "recursive" => Some(WorktreeSubmodules::Recursive),
                    _ => return false,
                };
            }
            _ => return false,
        }
        if prefs == ProjectDefaults::default() {
            self.project_defaults.remove(&scope);
        } else {
            self.project_defaults.insert(scope, prefs);
        }
        self.stamp_run_prefs_on_threads(cx);
        self.persist_and_notify(cx);
        true
    }
}

use super::*;
use crate::views::workspace::{project_defaults::ProjectDefaults, setting_source::ProjectSetting};
use bruhs_models::logic::run_place::WorktreeSubmodules as Submodules;

impl ProjectDefaults {
    pub(super) fn worktree_cleanup_mode(&self) -> Option<CleanupMode> {
        self.cleanup_mode.or_else(|| {
            self.cleanup_deleted_worktrees.map(|enabled| {
                if enabled {
                    CleanupMode::Custom
                } else {
                    CleanupMode::Off
                }
            })
        })
    }
}

#[derive(Clone, Copy)]
pub(super) enum WorktreeSetting {
    Placement,
    Submodules,
    Cleanup,
}

impl ProjectSetting for WorktreeSetting {
    fn project_label(self, prefs: &ProjectDefaults) -> Option<SharedString> {
        match self {
            Self::Placement => prefs.placement.map(|choice| choice.label().into()),
            Self::Submodules => prefs.submodules.map(|value| value.label().into()),
            Self::Cleanup => prefs.worktree_cleanup_mode().map(|mode| match mode {
                CleanupMode::Off => "Off".into(),
                CleanupMode::Custom => "Custom".into(),
            }),
        }
    }
    fn clear_project(self, p: &mut ProjectDefaults) {
        match self {
            Self::Placement => p.placement = None,
            Self::Submodules => p.submodules = None,
            Self::Cleanup => {
                p.cleanup_mode = None;
                p.worktree_retention = None;
                p.cleanup_keep_when = None;
                p.cleanup_deleted_worktrees = None;
            }
        }
    }
    fn repository(self, ws: &Workspace) -> Option<(&'static str, Option<SharedString>)> {
        match self {
            Self::Placement => Some((
                "Repository config",
                ws.checkout_defaults
                    .placement
                    .map(|choice| choice.label().into()),
            )),
            Self::Submodules => Some((
                "Repository config",
                ws.checkout_defaults.submodules.map(|v| v.label().into()),
            )),
            Self::Cleanup => None,
        }
    }
    fn after_reset(self, ws: &mut Workspace, _: &mut Window, cx: &mut Context<'_, Workspace>) {
        match self {
            Self::Placement | Self::Submodules => ws.stamp_run_prefs_on_threads(cx),
            Self::Cleanup => {
                ws.retention_editor = None;
                ws.worktree_cleanup_status = None;
            }
        }
        ws.persist_and_notify(cx);
    }
}

impl Workspace {
    pub(in crate::views::workspace) fn watch_checkout_defaults(
        &mut self,
        cx: &mut Context<'_, Self>,
    ) {
        self.checkout_defaults_watch = None;
        self.checkout_defaults = Default::default();
        if self.settings_project_scope.is_empty() {
            return;
        }
        let scope = self.settings_project_scope.clone();
        let mut first_read = true;
        self.checkout_defaults_watch = Some(cx.spawn(async move |this, cx| loop {
            let active = this.update(cx, |ws, _| {
                ws.settings_visible()
                    && matches!(
                        ws.settings_section,
                        SettingsSection::Worktrees
                            | SettingsSection::Projects
                            | SettingsSection::Threads
                    )
                    && ws.settings_project_scope == scope
            });
            if !matches!(active, Ok(true)) {
                return;
            }
            let project = std::path::PathBuf::from(&scope);
            let value = cx
                .background_executor()
                .spawn(async move { bruhs_core::git::project_checkout_defaults(&project) })
                .await;
            if this
                .update(cx, |ws, cx| {
                    if ws.settings_project_scope == scope && (first_read || ws.checkout_defaults != value) {
                        ws.checkout_defaults = value;
                        for thread in &ws.threads {
                            thread.chat.update(cx, |chat, cx| {
                                chat.update_repo_placement(&scope, value.placement, cx)
                            });
                        }
                        cx.notify();
                    }
                })
                .is_err()
            {
                return;
            }
            first_read = false;
            cx.background_executor()
                .timer(std::time::Duration::from_secs(2))
                .await;
        }));
    }

    pub(in crate::views::workspace) fn choose_default_placement(
        &mut self,
        value: &str,
        cx: &mut Context<'_, Self>,
    ) {
        let choice = match value {
            "inherit" => None,
            "checkout" => Some(WorkspaceChoice::Checkout),
            "new_worktree" => Some(WorkspaceChoice::NewWorktree),
            "previous_worktree" if self.previous_worktree.is_some() => {
                Some(WorkspaceChoice::PreviousWorktree)
            }
            _ => return,
        };
        self.set_default_placement(&self.settings_project_scope.clone(), choice, cx);
    }

    pub(in crate::views::workspace) fn set_default_placement(
        &mut self,
        scope: &str,
        choice: Option<WorkspaceChoice>,
        cx: &mut Context<'_, Self>,
    ) {
        if scope.is_empty() {
            self.run_choice = choice;
        } else {
            let prefs = self.project_defaults.entry(scope.to_owned()).or_default();
            prefs.placement = choice;
            if *prefs == Default::default() {
                self.project_defaults.remove(scope);
            }
        }
        self.stamp_run_prefs_on_threads(cx);
        self.persist_and_notify(cx);
    }

    pub(in crate::views::workspace) fn new_thread_placement_row(
        &self,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        let id = "settings-new-thread-placement";
        let row = "desktop-settings-row-new-thread-placement";
        let scoped = !self.settings_project_scope.is_empty();
        let project = self
            .project_defaults
            .get(&self.settings_project_scope)
            .and_then(|p| p.placement);
        let choice = project
            .or(self.run_choice)
            .or(self.checkout_defaults.placement)
            .unwrap_or_default();
        let mut options = vec![
            ("checkout", "Current checkout"),
            ("new_worktree", "New worktree"),
        ];
        if choice == WorkspaceChoice::PreviousWorktree || self.previous_worktree.is_some() {
            options.push(("previous_worktree", "Previous worktree"));
        }
        let control = settings_ui::control_column(self.choice_control(id, "Default workspace", choice.as_str(), &options, cx));
        let actions = self.setting_source_actions(
            WorktreeSetting::Placement,
            id,
            row,
            "Default workspace",
            self.run_choice.map(|choice| choice.label().into()),
            WorkspaceChoice::Checkout.label(),
            if scoped {
                project.is_some()
            } else {
                self.run_choice.is_some()
            },
            cx,
        );
        self.settings_row_with_reset(
            row,
            "Workspace",
            "Where new threads start.",
            None,
            false,
            Some(control),
            Some(actions),
        )
    }

    pub(super) fn submodule_setting(&self, cx: &Context<'_, Self>) -> AnyElement {
        let id = "settings-worktree-submodules";
        let row = "desktop-settings-row-worktree-submodules";
        let label = "Submodules";
        let project = self
            .project_defaults
            .get(&self.settings_project_scope)
            .and_then(|p| p.submodules);
        let environment = self.worktree_prefs.submodules;
        let env_choice = (environment != Submodules::FromProject).then_some(environment);
        let current = project
            .or(env_choice)
            .or(self.checkout_defaults.submodules)
            .unwrap_or(Submodules::Recursive);
        let control = settings_ui::control_column(self.choice_control(
            id,
            label,
            current.key(),
            &[
                ("recursive", "Recursive"),
                ("top-level", "Top level only"),
                ("none", "None"),
            ],
            cx,
        ));
        let changed = if self.settings_project_scope.is_empty() {
            env_choice.is_some()
        } else {
            project.is_some()
        };
        let actions = self.setting_source_actions(
            WorktreeSetting::Submodules,
            id,
            row,
            label,
            env_choice.map(|v| v.label().into()),
            "Recursive",
            changed,
            cx,
        );
        self.settings_row_with_reset(
            row,
            label,
            "Git submodules to initialize in new worktrees.",
            None,
            false,
            Some(control),
            Some(actions),
        )
    }
}

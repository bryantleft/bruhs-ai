//! Edit the host's managed worktree folder from the native workspace.
use super::*;
use bruhs_core::worktrees::locations;
use bruhs_models::coordination::worktree::KeepWhen;

pub(super) use bruhs_models::coordination::worktree::{CleanupMode, ScratchCleanup, WorktreeRetention};
pub(super) mod retention;
mod scoped;
use scoped::WorktreeSetting;

pub(super) struct CleanupTarget {
    pub path: String,
    pub branch: Option<String>,
    pub thread_id: String,
    pub keep: KeepWhen,
}

impl CleanupTarget {
    pub(super) fn new(
        thread_id: &str,
        path: Option<String>,
        branch: Option<String>,
        keep: KeepWhen,
    ) -> Option<Self> {
        let path = path?;
        if branch.is_none()
            && !bruhs_core::worktrees::scratch::is_managed_path(std::path::Path::new(&path))
        {
            return None;
        }
        Some(Self {
            path,
            branch,
            keep,
            thread_id: thread_id.to_owned(),
        })
    }
}

impl Workspace {
    pub(super) fn worktree_location_row(&self, cx: &Context<'_, Self>) -> AnyElement {
        let apply = web_button(
            "settings-apply-worktree-location",
            WebButtonVariant::Outline,
            WebButtonSize::Small,
            ButtonState::disabled_if(self.location_busy),
        )
        .aria_label("Apply worktree location")
        .child("Apply");
        let reset = web_button(
            "settings-reset-worktree-location",
            WebButtonVariant::Outline,
            WebButtonSize::Small,
            ButtonState::disabled_if(self.location_busy),
        )
        .aria_label("Reset worktree location")
        .child("Reset");
        self.settings_row(
            "settings-worktree-location",
            "Worktree location",
            self.location_status.clone().unwrap_or_else(|| {
                "Parent folder for new bruhs-worktrees checkouts.".to_owned()
            }),
            settings_ui::control_column(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(Input::new(&self.location_input).aria_label("Worktree location"))
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .child(on_press_view(apply, cx, |this, _, cx| {
                                let value = this.location_input.read(cx).value().to_string();
                                this.save_worktree_location(value, cx);
                            }))
                            .child(on_press_view(reset, cx, |this, window, cx| {
                                this.location_input
                                    .update(cx, |input, cx| input.set_value("", window, cx));
                                this.save_worktree_location(String::new(), cx);
                            })),
                    )
                    .into_any_element(),
            ),
        )
    }
    pub(super) fn read_worktree_location(&mut self, cx: &mut Context<'_, Self>) {
        if self.location_busy {
            return;
        }
        self.location_busy = true;
        let request = cx.background_spawn(async { locations::active() });
        cx.spawn(async move |this, cx| {
            let result = request.await;
            let _ = this.update(cx, |this, cx| {
                this.location_busy = false;
                this.location_status = Some(match result {
                    Ok(path) => format!("New worktrees are stored in {}.", path.display()),
                    Err(error) => error.to_string(),
                });
                cx.notify();
            });
        })
        .detach();
    }
    fn save_worktree_location(&mut self, value: String, cx: &mut Context<'_, Self>) {
        if self.location_busy {
            return;
        }
        self.location_busy = true;
        let request = cx.background_spawn(async move {
            let value = value.trim();
            if value.is_empty() {
                return locations::choose(None);
            }
            let path = if let Some(rest) = value.strip_prefix("~/") {
                directories::BaseDirs::new()
                    .ok_or_else(|| std::io::Error::other("Home folder unavailable."))?
                    .home_dir()
                    .join(rest)
            } else {
                std::path::PathBuf::from(value)
            };
            if !path.is_absolute() {
                return Err(std::io::Error::other(
                    "Enter an absolute folder path or start with ~/.",
                ));
            }
            locations::choose(Some(&path))
        });
        cx.spawn(async move |this, cx| {
            let result = request.await;
            let _ = this.update(cx, |this, cx| {
                this.location_busy = false;
                this.location_status = Some(match result {
                    Ok(path) => format!("New worktrees are stored in {}.", path.display()),
                    Err(error) => format!("Could not apply worktree location. {error}"),
                });
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
}
impl Workspace {
    pub(super) fn settings_advanced_button(
        &self,
        id: &'static str,
        label: &'static str,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        let expanded = self.settings_advanced.contains(id);
        let button = web_button(
            id,
            WebButtonVariant::Ghost,
            WebButtonSize::Small,
            ButtonState::Enabled,
        )
        .debug_selector(move || id.to_string())
        .aria_label(label)
        .aria_expanded(expanded)
        .w_full()
        .min_h(px(40.))
        .justify_start()
        .px(px(15.))
        .child(
            svg()
                .path(if expanded {
                    assets::CHEVRON_DOWN_ICON_PATH
                } else {
                    assets::CHEVRON_RIGHT_ICON_PATH
                })
                .size(px(14.))
                .text_color(theme::rambutan_400()),
        )
        .child("Advanced");
        on_press_view(button, cx, move |this, _, cx| {
            if !this.settings_advanced.remove(id) {
                this.settings_advanced.insert(id);
            }
            this.set_settings_open_dropdown(None);
            cx.notify();
        })
        .into_any_element()
    }

    pub(super) fn show_settings_advanced(&self, id: &'static str) -> bool {
        self.settings_advanced.contains(id) || self.settings_search_inventory.borrow().is_some()
    }

    pub(super) fn worktree_settings(&self, cx: &Context<'_, Self>) -> AnyElement {
        let scoped = !self.settings_project_scope.is_empty();
        let mut creation = Vec::new();
        if !scoped {
            creation.push(self.worktree_location_row(cx));
        }
        creation.push(self.scoped_switch_row(
            ScopedSwitch::Remote, "settings-worktree-remote", "desktop-settings-row-worktree-remote", "Start from remote",
            "Branch from the latest upstream commit when available.",
            self.worktree_prefs.start_from_remote, cx,
        ));
        creation.push(self.settings_advanced_button(
            "worktree-creation-advanced",
            "Advanced creation settings",
            cx,
        ));
        if self.show_settings_advanced("worktree-creation-advanced") {
            creation.push(self.submodule_setting(cx));
        }
        div()
            .flex()
            .flex_col()
            .gap_4()
            .child(self.settings_card("Settings scope", "", vec![self.defaults_scope_row(cx)]))
            .child(self.settings_card("Creation", "", creation))
            .child(self.settings_card("Branches", "", self.branch_naming_rows(cx)))
            .child(self.settings_card("Cleanup", "", self.worktree_cleanup_rows(cx)))
            .into_any_element()
    }

    fn project_cleanup_mode(&self, project: &str) -> Option<CleanupMode> {
        self.project_defaults.get(project)?.worktree_cleanup_mode()
    }
    pub(super) fn cleanup_rules(&self, project: &str) -> (bool, KeepWhen) {
        let inherited = (self.cleanup_deleted_worktrees, self.cleanup_keep_when);
        match self.project_cleanup_mode(project) {
            None => inherited,
            Some(CleanupMode::Off) => (false, inherited.1),
            Some(CleanupMode::Custom) => {
                let p = &self.project_defaults[project];
                (
                    p.cleanup_deleted_worktrees.unwrap_or(inherited.0),
                    p.cleanup_keep_when.unwrap_or(inherited.1),
                )
            }
        }
    }

    pub(super) fn deleted_checkout(&self, t: &DesktopThread, cx: &App) -> Option<CleanupTarget> {
        let chat = t.chat.read(cx);
        let (enabled, keep) = self.cleanup_rules(chat.settings_project_path());
        if chat.is_streaming || !enabled {
            return None;
        }
        CleanupTarget::new(&t.id, chat.worktree_path.clone(), t.branch.clone(), keep)
    }
    pub(super) fn cleanup_deleted_checkout(
        &mut self,
        checkout: CleanupTarget,
        cx: &mut Context<'_, Self>,
    ) {
        let CleanupTarget {
            path,
            branch,
            keep,
            thread_id,
        } = checkout;
        let scratch = branch.is_none();
        let target = bruhs_core::worktrees::resolve(std::path::Path::new(&path));
        let shared = self
            .threads
            .iter()
            .flat_map(|t| {
                let chat = t.chat.read(cx);
                std::iter::once(chat.project_path.as_str()).chain(chat.worktree_path.as_deref())
            })
            .chain(
                self.archived_threads
                    .iter()
                    .flat_map(|t| t.checkout_paths(cx)),
            )
            .filter(|path| !path.is_empty())
            .any(|other| {
                bruhs_core::worktrees::resolve(std::path::Path::new(other)).starts_with(&target)
            });
        if shared {
            return;
        }
        if scratch {
            if let Ok(id) = bruhs_models::ids::ThreadId::try_from(thread_id.as_str()) {
                self.pending_scratch_cleanups.push(ScratchCleanup {
                    thread_id: id,
                    path,
                    keep_when: keep,
                });
                self.persist(cx);
            }
            return;
        }
        self.send_checkout_cleanup(
            CleanupTarget {
                path,
                branch,
                keep,
                thread_id,
            },
            cx,
        );
    }

    pub(super) fn dispatch_saved_scratch(
        &mut self,
        saved: Vec<ScratchCleanup>,
        cx: &mut Context<'_, Self>,
    ) {
        for entry in saved {
            if self.pending_scratch_cleanups.contains(&entry)
                && self
                    .dispatched_scratch_cleanups
                    .insert(entry.thread_id.to_string())
            {
                self.send_checkout_cleanup(
                    CleanupTarget {
                        path: entry.path,
                        branch: None,
                        keep: entry.keep_when,
                        thread_id: entry.thread_id.to_string(),
                    },
                    cx,
                );
            }
        }
    }

    fn send_checkout_cleanup(&mut self, checkout: CleanupTarget, cx: &mut Context<'_, Self>) {
        let CleanupTarget {
            path,
            branch,
            keep,
            thread_id,
        } = checkout;
        let scratch = branch.is_none();
        let deleted = thread_id.clone();
        let work = crate::app_state::spawn_tokio(cx, async move {
            crate::server_client::ServerClient::workbench_client()
                .clean_deleted_thread(&path, branch.as_deref(), &thread_id, keep)
                .await
        });
        cx.spawn(async move |this, cx| {
            let result = work.await;
            let _ = this.update(cx, |this, cx| {
                if scratch
                    && (result.is_ok()
                        || result
                            .as_ref()
                            .is_err_and(|e| e.conflict_message().is_some()))
                {
                    this.pending_scratch_cleanups
                        .retain(|r| r.thread_id.as_str() != deleted);
                    this.dispatched_scratch_cleanups.remove(&deleted);
                    this.persist(cx);
                }
                this.worktree_cleanup_status = Some(match result {
                    Ok(()) if scratch => "The unused scratch folder was removed.".into(),
                    Ok(()) => "The unused checkout was removed. Its Git branch was kept.".into(),
                    Err(e) if scratch && e.is_transport() => {
                        "Scratch cleanup is pending until the host is available.".into()
                    }
                    Err(e) => {
                        let reason = e.conflict_message().unwrap_or_else(|| e.to_string());
                        if scratch {
                            format!("The scratch folder was kept. {reason}")
                        } else {
                            format!("The worktree was kept. {reason}")
                        }
                    }
                });
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn select_worktree_cleanup(
        &mut self,
        id: &str,
        value: &str,
        cx: &mut Context<'_, Self>,
    ) -> bool {
        let project = self.settings_project_scope.clone();
        match id {
            "settings-worktree-cleanup-mode" if !project.is_empty() => {
                let mode = match value {
                    "inherit" => None,
                    "off" => Some(CleanupMode::Off),
                    "custom" => Some(CleanupMode::Custom),
                    _ => return false,
                };
                if mode == self.project_cleanup_mode(&project) {
                    cx.notify();
                    return true;
                }
                let (deleted, keep) = self.cleanup_rules(&project);
                let retention = self.retention();
                let p = self.project_defaults.entry(project.clone()).or_default();
                self.settings_advanced.remove("worktree-cleanup-advanced");
                p.worktree_retention = (mode == Some(CleanupMode::Custom)).then_some(retention);
                self.retention_editor = None;
                p.cleanup_mode = mode;
                p.cleanup_deleted_worktrees = (mode == Some(CleanupMode::Custom)).then_some(deleted);
                p.cleanup_keep_when = (mode == Some(CleanupMode::Custom)).then_some(keep);
                if *p == Default::default() {
                    self.project_defaults.remove(&project);
                }
            }
            "settings-worktree-keep-when" => {
                let Some((keep, _, _)) = KeepWhen::CHOICES.iter().find(|(_, key, _)| *key == value)
                else {
                    return false;
                };
                if project.is_empty() {
                    self.cleanup_keep_when = *keep;
                } else if self.project_cleanup_mode(&project) == Some(CleanupMode::Custom) {
                    let prefs = self.project_defaults.entry(project).or_default();
                    prefs.cleanup_mode = Some(CleanupMode::Custom);
                    prefs.cleanup_keep_when = Some(*keep);
                }
            }
            _ => return false,
        }
        self.worktree_cleanup_status = None;
        self.persist_and_notify(cx);
        true
    }

    pub(super) fn worktree_cleanup_rows(&self, cx: &Context<'_, Self>) -> Vec<AnyElement> {
        let project = &self.settings_project_scope;
        let (on, keep) = self.cleanup_rules(project);
        let mut rows = Vec::new();
        if !project.is_empty() {
            let mode = self.project_cleanup_mode(project);
            let (value, help) = match mode {
                None => ("inherit", "Use this machine’s worktree cleanup settings."),
                Some(CleanupMode::Off) => (
                    "off",
                    "Keep this project’s worktrees until you delete them manually.",
                ),
                Some(CleanupMode::Custom) => ("custom", "Use these rules for this project."),
            };
            let id = "settings-worktree-cleanup-mode";
            let row = "settings-row-worktree-cleanup-mode";
            let label = "Automatic worktree cleanup";
            let control = settings_ui::control_column(self.choice_control(id, label, value,
                &[("inherit", "Inherit"), ("off", "Off"), ("custom", "Custom")], cx));
            let custom_environment = self.cleanup_deleted_worktrees || self.cleanup_keep_when != KeepWhen::default()
                || self.worktree_retention != WorktreeRetention::default();
            let actions = self.setting_source_actions(WorktreeSetting::Cleanup,id,row,label,custom_environment.then(||"Custom".into()),"Not set",mode.is_some(),cx);
            rows.push(self.settings_row_with_reset(row,label,help,None,false,Some(control),Some(actions)));
            if mode != Some(CleanupMode::Custom) {
                return rows;
            }
        }
        let label = "Delete worktrees with deleted threads";
        rows.push(self.settings_row(
            "settings-row-cleanup-deleted",
            label,
            self.worktree_cleanup_status.clone().unwrap_or_else(|| {
                "Remove an unused worktree or scratch folder when its thread is deleted. Worktree branches are kept.".into()
            }),
            settings_ui::switch("settings-cleanup-deleted", on, false, label, cx, |this, _, cx| {
                let project = &this.settings_project_scope;
                let enabled = !this.cleanup_rules(project).0;
                if project.is_empty() {
                    this.cleanup_deleted_worktrees = enabled;
                } else if this.project_cleanup_mode(project) == Some(CleanupMode::Custom) {
                    let prefs = this.project_defaults.entry(project.clone()).or_default();
                    prefs.cleanup_mode = Some(CleanupMode::Custom);
                    prefs.cleanup_deleted_worktrees = Some(enabled);
                }
                this.worktree_cleanup_status = None;
                this.persist_and_notify(cx);
            }),
        ));
        rows.extend(self.retention_rows(false, cx));
        rows.push(self.settings_advanced_button(
            "worktree-cleanup-advanced",
            "Advanced cleanup settings",
            cx,
        ));
        if !self.show_settings_advanced("worktree-cleanup-advanced") {
            return rows;
        }
        rows.extend(self.retention_rows(true, cx));
        let choices = KeepWhen::CHOICES.map(|(_, key, label)| (key, label));
        rows.push(self.settings_row(
            "settings-row-worktree-keep-when",
            "Keep worktrees with local changes",
            if project.is_empty() {
                let scratch = match keep {
                    KeepWhen::TrackedChanges => "Scratch folders can be removed with their files.",
                    _ => "Files outside node_modules also protect a scratch folder.",
                };
                format!("{} {scratch}", keep.description())
            } else {
                keep.description().to_owned()
            },
            settings_ui::control_column(self.choice_control(
                "settings-worktree-keep-when",
                "Keep worktrees with local changes",
                keep.key(),
                &choices,
                cx,
            )),
        ));
        rows
    }
}

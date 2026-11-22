//! Environment maintenance policies and the desktop operations they control.
mod background;
mod captures;
pub(super) mod restart;
use super::*;
use crate::app_state::spawn_tokio;
use std::collections::BTreeSet;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub(super) struct MaintenancePreferences {
    pub provider_checks: bool,
    pub(super) continue_threads: bool,
    keep_awake: bool,
    fetch_seconds: u32,
    activity:background::Policy,
    pub(super) auto_pull: bool,
    pub retention_days: u16,
}
impl Default for MaintenancePreferences {
    fn default() -> Self {
        Self {
            provider_checks: true,
            continue_threads: true,
            keep_awake: false,
            fetch_seconds: 0,
            activity:Default::default(),
            auto_pull: false,
            retention_days: 0,
        }
    }
}
pub(super) struct Maintenance {
    pub prefs: MaintenancePreferences,
    seconds: Entity<InputState>,
    polling: Option<Task<()>>,
    power:Option<background::Power>,power_timer:Option<Task<()>>,
    sleep: Option<Task<()>>,
    sleep_error: Option<String>,
    git_status: Option<String>,
    cleanup_schedule: Option<Task<()>>,
    cleanup_running: bool,
    cleanup_status: Option<String>,
    captures: captures::Retention,
}
impl Maintenance {
    pub(super) fn new(
        prefs: MaintenancePreferences,
        window: &mut Window,
        cx: &mut Context<'_, Workspace>,
        subscriptions: &mut Vec<Subscription>,
    ) -> Self {
        let seconds =
            cx.new(|cx| InputState::new(window, cx).default_value(prefs.fetch_seconds.to_string()));
        subscriptions.push(cx.subscribe_in(
            &seconds,
            window,
            |this, field, event: &InputEvent, window, cx| {
                if !matches!(event, InputEvent::PressEnter { .. } | InputEvent::Blur) {
                    return;
                }
                let previous = this.maintenance.prefs.fetch_seconds;
                let value = field
                    .read(cx)
                    .value()
                    .trim()
                    .parse::<u32>()
                    .map(|s| if s == 0 { 0 } else { s.clamp(5, 86400) })
                    .unwrap_or(previous);
                if previous != value {
                    this.custom_background();
                    this.maintenance.prefs.fetch_seconds = value;
                    this.schedule_project_refresh(cx);
                    this.persist_and_notify(cx);
                }
                if field.read(cx).value().as_ref() != value.to_string() {
                    field.update(cx, |f, cx| f.set_value(value.to_string(), window, cx));
                }
            },
        ));
        Self {
            prefs,
            seconds,
            polling: None,
            power:None,power_timer:None,
            sleep: None,
            sleep_error: None,
            git_status: None,
            cleanup_schedule: None,
            cleanup_running: false,
            cleanup_status: None,
            captures: captures::Retention::new(window,cx,subscriptions),
        }
    }
}
impl Workspace {
    pub(super) fn schedule_log_cleanup(&mut self, cx: &mut Context<'_, Self>) {
        self.maintenance.cleanup_schedule.take();
        if self.maintenance.prefs.retention_days == 0 {
            return;
        }
        self.maintenance.cleanup_schedule = Some(cx.spawn(async move |this, cx| loop {
            if this
                .update(cx, |this, cx| this.run_maintenance_cleanup(false,cx))
                .is_err()
            {
                break;
            }
            cx.background_executor()
                .timer(std::time::Duration::from_secs(86400))
                .await;
        }));
    }

    fn run_maintenance_cleanup(&mut self, manual: bool, cx: &mut Context<'_, Self>) {
        let days = self.maintenance.prefs.retention_days;
        let captures = manual && self.maintenance.captures.days.unwrap_or_default() > 0;
        if (days == 0 && !captures) || self.maintenance.cleanup_running {return;}
        self.maintenance.cleanup_running = true;
        let work = spawn_tokio(cx, async move {
            let mut results = Vec::new();
            if days != 0 {
                let result = tokio::task::spawn_blocking(move || {
                    let dir = crate::paths::log_dir().ok_or_else(|| std::io::Error::other("The log folder is unavailable."))?;
                    crate::logging::prune_rotated_log(&dir, days, std::time::SystemTime::now())
                }).await;
                results.push(match result {
                    Ok(Ok(true)) => "Deleted one expired desktop log.".into(),
                    Ok(Ok(false)) => "No rotated log met the retention period.".into(),
                    Ok(Err(e)) => format!("Could not clean up rotated logs. {e}"),
                    Err(e) => format!("The log check was interrupted. {e}"),
                });
            }
            if captures {
                results.push(match crate::server_client::ServerClient::workbench_client().prune_browser_captures().await {
                    Ok(0) => "No browser capture met the retention period.".into(),
                    Ok(1) => "Deleted one expired browser capture.".into(),
                    Ok(count) => format!("Deleted {count} expired browser captures."),
                    Err(e) => format!("Could not clean up browser captures. {e}"),
                });
            }
            results.join(" ")
        });
        cx.spawn(async move |this,cx| {
            let message=work.await;
            let _=this.update(cx,|this,cx|{
                this.maintenance.cleanup_running=false;
                this.maintenance.cleanup_status=Some(message);
                cx.notify();
            });
        }).detach();
    }

    pub(super) fn update_sleep_prevention(&mut self, cx: &mut Context<'_, Self>) {
        let keep = self.maintenance.prefs.keep_awake
            && self.threads.iter().any(|t| t.chat.read(cx).is_streaming);
        if !keep {
            self.maintenance.sleep.take();
            self.maintenance.sleep_error = None;
        } else if self.maintenance.sleep.is_none() {
            let acquire = cx.prevent_idle_sleep("bruhs agent turn");
            self.maintenance.sleep = Some(cx.spawn(async move |this, cx| match acquire.await {
                Ok(_guard) => std::future::pending::<()>().await,
                Err(e) => {
                    let _ = this.update(cx, |this, cx| {
                        this.maintenance.sleep_error = Some(e.to_string());
                        cx.notify();
                    });
                }
            }));
        }
    }

    fn auto_pull_for(&self, path: &str) -> bool {
        self.project_defaults
            .get(path)
            .and_then(|p| p.auto_pull)
            .unwrap_or(self.maintenance.prefs.auto_pull)
    }

    pub(super) fn schedule_project_refresh(&mut self, cx: &mut Context<'_, Self>) {
        self.maintenance.polling.take();
        self.maintenance.git_status = None;
        let fetch = self.maintenance.prefs.fetch_seconds;
        let automatic = self.maintenance.prefs.auto_pull
            || self
                .project_defaults
                .values()
                .any(|p| p.auto_pull == Some(true));
        if fetch == 0 && !automatic {
            return;
        }
        let interval =
            std::time::Duration::from_secs(u64::from(if fetch == 0 { 300 } else { fetch.max(5) }));
        self.maintenance.polling = Some(cx.spawn(async move |this, cx| loop {
            cx.background_executor().timer(interval).await;
            let Ok(projects) = this.update(cx, |this, cx| {
                let mut projects = BTreeSet::new();
                if this.background_paused(){return projects;}
                for thread in &this.threads {
                    let chat = thread.chat.read(cx);
                    if chat.project_scoped {
                        projects.insert(chat.settings_project_path().to_owned());
                    }
                }
                if let Some(project) = &this.active_project {
                    projects.insert(project.path.clone());
                }
                for project in &this.recent_projects {
                    projects.insert(project.path.clone());
                }
                projects
                    .retain(|path| !path.is_empty() && (fetch != 0 || this.auto_pull_for(path)));
                projects
            }) else {
                return;
            };
            for path in projects {
                let repo = std::path::PathBuf::from(&path);
                let Ok(load) = this.update(cx, |_, cx| {
                    spawn_tokio(cx, async move {
                        if !repo.join(".git").exists() {
                            return Ok(false);
                        }
                        bruhs_core::git::fetch_project_remote(&repo)
                            .await
                            .map(|()| true)
                    })
                }) else {
                    return;
                };
                let result = match load.await {
                    Ok(false) => continue,
                    Ok(true) => {
                        let Ok(pull) = this.update(cx, |this, cx| {
                            let idle = !this.threads.iter().any(|t| {
                                let chat = t.chat.read(cx);
                                chat.is_streaming && chat.project_path == path
                            });
                            (this.auto_pull_for(&path) && idle).then(|| {
                                let repo = std::path::PathBuf::from(&path);
                                spawn_tokio(cx, async move {
                                    bruhs_core::git::fast_forward_clean_default(&repo).await
                                })
                            })
                        }) else {
                            return;
                        };
                        match pull {
                            Some(update) => update.await,
                            None => Ok(false),
                        }
                    }
                    Err(e) => Err(e),
                };
                let Ok(()) = this.update(cx, |this, cx| {
                    this.maintenance.git_status = Some(match result {
                        Ok(true) => format!("Updated {path} to the remote default branch."),
                        Ok(false) => format!("Fetched remote status for {path}."),
                        Err(e) => format!("Could not refresh {path}. {e}"),
                    });
                    cx.notify();
                }) else {
                    return;
                };
            }
        }));
    }

    pub(super) fn render_source_control(&self, cx: &Context<'_, Self>) -> AnyElement {
        let id = if self.settings_project_scope.is_empty() { "settings-enable-auto-pull" } else { "settings-auto-pull" };
        let mut rows = vec![self.defaults_scope_row(cx), self.scoped_switch_row(
            ScopedSwitch::AutoPull, id, "desktop-settings-row-auto-pull", "Automatically pull",
            "Fast-forward clean default checkouts after fetch.",
            self.maintenance.prefs.auto_pull, cx,
        )];
        let (delivery, template_row) = self.delivery_preference_rows(cx);
        rows.extend(delivery);
        div().flex().flex_col().gap_4()
            .child(self.settings_card("", "", rows))
            .child(self.source_writer_card(template_row, cx)).into_any_element()
    }

    fn environment_maintenance_row(
        &self,
        selector: &'static str,
        label: &'static str,
        description: impl Into<SharedString>,
        read_only_value: impl Into<SharedString>,
        control: AnyElement,
    ) -> AnyElement {
        if self.settings_project_scope.is_empty() {
            self.settings_row(selector, label, description, control)
        } else {
            self.settings_row_opts(
                selector,
                label,
                description,
                Some("Select the environment to change this setting.".into()),
                true,
                div()
                    .text_size(px(13.))
                    .text_color(theme::rambutan_400())
                    .child(read_only_value.into())
                    .into_any_element(),
            )
        }
    }

    pub(super) fn render_maintenance(&self, cx: &Context<'_, Self>) -> AnyElement {
        let awake = self.environment_maintenance_row(
            "desktop-settings-row-keep-awake",
            "Keep awake while agents work",
            self.maintenance
                .sleep_error
                .as_deref()
                .unwrap_or("Prevent idle sleep during active turns.")
                .to_owned(),
            if self.maintenance.prefs.keep_awake {"On"} else {"Off"},
            settings_ui::switch(
                "settings-keep-awake",
                self.maintenance.prefs.keep_awake,
                false,
                "Keep awake while agents work",
                cx,
                |this, _, cx| {
                    this.maintenance.prefs.keep_awake = !this.maintenance.prefs.keep_awake;
                    this.update_sleep_prevention(cx);
                    this.persist_and_notify(cx);
                },
            ),
        );
        let providers = self.environment_maintenance_row(
            "desktop-settings-row-provider-checks",
            "Provider update checks",
            "Check for new provider CLI versions.",
            if self.maintenance.prefs.provider_checks {"On"} else {"Off"},
            settings_ui::switch(
                "settings-provider-checks",
                self.maintenance.prefs.provider_checks,
                false,
                "Provider update checks",
                cx,
                |this, _, cx| {
                    this.maintenance.prefs.provider_checks =
                        !this.maintenance.prefs.provider_checks;
                    let enabled = this.maintenance.prefs.provider_checks;
                    this.updates.update(cx, |updates, cx| {
                        updates.provider_checks = enabled;
                        updates.check(false, cx);
                        cx.notify();
                    });
                    this.persist_and_notify(cx);
                },
            ),
        );
        let git_description = self.maintenance.git_status.as_deref().unwrap_or("Seconds between remote fetches (0 to disable).").to_owned();
        let git = self.environment_maintenance_row("desktop-settings-row-fetch-interval","Git fetch interval",git_description,
            format!("{} seconds", self.maintenance.prefs.fetch_seconds),
            div().w(px(144.)).flex().items_center().gap_2().child(div().flex_1().min_w_0().child(Input::new(&self.maintenance.seconds).aria_label("Git fetch interval in seconds"))).child("seconds").into_any_element());
        div()
            .flex()
            .flex_col()
            .gap(px(24.))
            .child(self.settings_card("Updates", "", vec![self.defaults_scope_row(cx), providers, self.restart_setting(cx)]))
            .child(self.settings_card("Background activity", "", self.background_rows(cx).into_iter().chain([awake,git]).collect()))
            .child(
                self.settings_card(
                    "Artifacts and logs",
                    "",
                    vec![
                        self.capture_retention_row(cx),
                        self.environment_maintenance_row(
                            "desktop-settings-row-log-retention",
                            "Delete old rotated logs",
                            "Remove inactive desktop logs after the selected number of days.",
                            if self.maintenance.prefs.retention_days==0 {"Off".into()} else {format!("{} days",self.maintenance.prefs.retention_days)},
                            settings_ui::control_column(self.choice_control(
                                "settings-log-retention",
                                "Delete old rotated logs",
                                &self.maintenance.prefs.retention_days.to_string(),
                                &[
                                    ("0", "Off"),
                                    ("1", "1 day"),
                                    ("7", "7 days"),
                                    ("14", "14 days"),
                                    ("30", "30 days"),
                                    ("90", "90 days"),
                                ],
                                cx,
                            )),
                        ),
                        self.environment_maintenance_row(
                            "desktop-settings-row-log-cleanup",
                            "Delete now",
                            self.maintenance
                                .cleanup_status
                                .as_deref()
                                .unwrap_or(
                                    "Delete browser captures and rotated logs older than their saved retention periods.",
                                )
                                .to_owned(),
                            "Environment only",
                            on_press_view(
                                web_button(
                                    "settings-delete-expired-logs",
                                    WebButtonVariant::Outline,
                                    WebButtonSize::Small,
                                    ButtonState::disabled_if(
                                        (self.maintenance.prefs.retention_days == 0 && self.maintenance.captures.days.unwrap_or_default() == 0)
                                            || self.maintenance.cleanup_running,
                                    ),
                                )
                                .aria_label("Delete expired captures and logs")
                                .child("Delete now"),
                                cx,
                                |this, _, cx| this.run_maintenance_cleanup(true,cx),
                            )
                            .into_any_element(),
                        ),
                    ],
                ),
            )
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    #[gpui::test]
    fn sleep_prevention_tracks_overlapping_agent_turns_and_releases_on_disable(
        cx: &mut gpui::TestAppContext,
    ) {
        let (workspace, cx, _, _) = super::super::tests::thread_menu_workspace(cx);
        workspace.update_in(cx, |workspace, _, cx| {
            workspace.maintenance.prefs.keep_awake = true;
            for thread in &workspace.threads {
                thread.chat.update(cx, |c, _| c.is_streaming = true);
            }
            workspace.update_sleep_prevention(cx);
        });
        cx.run_until_parked();
        assert_eq!(cx.active_idle_sleep_preventions(), 1);
        workspace.update_in(cx, |workspace, _, cx| {
            workspace.threads[0]
                .chat
                .update(cx, |c, _| c.is_streaming = false);
            workspace.update_sleep_prevention(cx);
        });
        cx.run_until_parked();
        assert_eq!(cx.active_idle_sleep_preventions(), 1);
        workspace.update_in(cx, |workspace, _, cx| {
            workspace.maintenance.prefs.keep_awake = false;
            workspace.update_sleep_prevention(cx);
        });
        cx.run_until_parked();
        assert_eq!(cx.active_idle_sleep_preventions(), 0);
    }
}

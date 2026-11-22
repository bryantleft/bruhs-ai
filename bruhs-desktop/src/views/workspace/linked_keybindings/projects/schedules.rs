use super::*;
use bruhs_models::reactive::ScheduledTaskSummary;
use bruhs_ui::{press::Interactivity, switch::switch};

#[derive(Clone, Copy)]
enum ScheduleEdit {
    SetEnabled(bool),
    Delete,
}

pub(super) enum ScheduleWrite {
    Saving,
    Failed(String),
}

pub(super) async fn schedule_reply<T: serde::de::DeserializeOwned>(
    mut response: reqwest::Response,
    limit: usize,
) -> Result<T, String> {
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| "The host’s response was interrupted.")?
    {
        if bytes.len() + chunk.len() > limit {
            return Err("The host’s response is too large to display.".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&bytes)
        .map_err(|_| "The host returned unreadable scheduled task data.".into())
}

impl ProjectHost {
    pub(super) async fn read_schedules(&self) -> Result<Vec<ScheduledTaskSummary>, String> {
        let mut url = self.endpoint.clone();
        url.set_path("/api/projects/scheduled-tasks");
        let response = self
            .client()?
            .get(url)
            .bearer_auth(&self.token)
            .timeout(Duration::from_secs(30))
            .send()
            .await
            .map_err(|_| format!("Could not reach {}.", self.name()))?;
        if !response.status().is_success() {
            return Err(format!(
                "Could not load scheduled tasks on {} ({}).",
                self.name(),
                response.status()
            ));
        }
        let tasks: Vec<ScheduledTaskSummary> = schedule_reply(response, 8 * 1024 * 1024).await?;
        let mut seen = BTreeSet::new();
        if tasks
            .iter()
            .any(|t| t.id.is_empty() || !t.schedule.is_scheduled_task() || !seen.insert(&t.id))
        {
            return Err("The host returned invalid task identifiers.".into());
        }
        Ok(tasks)
    }
    async fn edit_schedule(
        &self,
        id: &str,
        edit: ScheduleEdit,
    ) -> Result<Option<ScheduledTaskSummary>, String> {
        let mut url = self.endpoint.clone();
        let client = self.client()?;
        let mut path = url
            .path_segments_mut()
            .map_err(|_| "Invalid project host address.")?;
        path.clear()
            .extend(["api", "projects", "scheduled-tasks", id]);
        let request = match edit {
            ScheduleEdit::Delete => {
                drop(path);
                client.delete(url)
            }
            ScheduleEdit::SetEnabled(enabled) => {
                path.push("enabled");
                drop(path);
                client
                    .put(url)
                    .json(&serde_json::json!({"enabled": enabled}))
            }
        };
        let response = request
            .bearer_auth(&self.token)
            .timeout(Duration::from_secs(30))
            .send()
            .await
            .map_err(|_| format!("Could not confirm the change on {}.", self.name()))?;
        match response.status() {
            reqwest::StatusCode::NO_CONTENT if matches!(edit, ScheduleEdit::Delete) => {
                return Ok(None)
            }
            reqwest::StatusCode::OK if matches!(edit, ScheduleEdit::SetEnabled(_)) => {}
            reqwest::StatusCode::UNAUTHORIZED | reqwest::StatusCode::FORBIDDEN => {
                return Err(match edit {
                    ScheduleEdit::Delete => {
                        "This host denied permission to delete scheduled tasks."
                    }
                    ScheduleEdit::SetEnabled(_) => {
                        "This host denied permission to pause or resume tasks."
                    }
                }
                .into());
            }
            reqwest::StatusCode::NOT_FOUND => {
                return Err("This scheduled task is no longer available.".into())
            }
            reqwest::StatusCode::CONFLICT | reqwest::StatusCode::BAD_REQUEST => {
                #[derive(serde::Deserialize)]
                struct Problem {
                    error: String,
                }
                return Err(schedule_reply::<Problem>(response, 8192).await?.error);
            }
            status => {
                return Err(format!(
                    "Could not change the task on {} ({status}).",
                    self.name()
                ))
            }
        }
        let task: ScheduledTaskSummary = schedule_reply(response, 1024 * 1024).await?;
        if task.id != id
            || !matches!(edit, ScheduleEdit::SetEnabled(enabled) if task.enabled == enabled)
            || !task.schedule.is_scheduled_task()
        {
            return Err("The host did not confirm this task’s new state.".into());
        }
        Ok(Some(task))
    }
}

impl Workspace {
    pub(super) fn confirm_delete_linked_schedule(
        &mut self,
        host_id: uuid::Uuid,
        id: &str,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.set_settings_open_dropdown(None);
        cx.notify();
        let Some(host) = self
            .linked_projects
            .hosts
            .iter()
            .find(|h| h.credential == host_id && h.data.can_delete_schedules)
        else {
            return;
        };
        let Some(task) = host
            .data
            .schedules
            .as_ref()
            .and_then(|s| s.as_ref().ok())
            .and_then(|tasks| tasks.iter().find(|t| t.id == id))
        else {
            return;
        };
        let title = task.title.clone();
        let message = format!("Delete “{title}” on {}?", host.name());
        let answer = self.reactive_automations.update(cx, |panel, cx| {
            panel.open_schedule_delete_confirmation(message, window, cx)
        });
        let id = id.to_owned();
        let destination = host.destination();
        let token = host.token.clone();
        cx.spawn(async move |ws, cx| {
            if answer.await.is_err() {
                return;
            }
            let _ = ws.update(cx, |ws, cx| {
                let current = ws
                    .linked_projects
                    .hosts
                    .iter()
                    .find(|h| {
                        h.credential == host_id
                            && h.destination() == destination
                            && h.token == token
                    })
                    .and_then(|h| h.data.schedules.as_ref()?.as_ref().ok())
                    .is_some_and(|tasks| tasks.iter().any(|t| t.id == id && t.title == title));
                if current {
                    ws.change_linked_schedule(host_id, id, ScheduleEdit::Delete, cx);
                }
            });
        })
        .detach();
    }

    fn change_linked_schedule(
        &mut self,
        host_id: uuid::Uuid,
        id: String,
        edit: ScheduleEdit,
        cx: &mut Context<'_, Self>,
    ) {
        let state = &mut self.linked_projects;
        let key = (host_id, id.clone());
        let Some(host) = state
            .hosts
            .iter()
            .find(|h| {
                h.credential == host_id
                    && match edit {
                        ScheduleEdit::Delete => h.data.can_delete_schedules,
                        ScheduleEdit::SetEnabled(_) => h.data.can_toggle_schedules,
                    }
            })
            .cloned()
        else {
            return;
        };
        if matches!(state.schedule_writes.get(&key), Some(ScheduleWrite::Saving)) {
            return;
        }
        state
            .schedule_writes
            .insert(key.clone(), ScheduleWrite::Saving);
        let destination = host.destination();
        let token = host.token.clone();
        let job =
            crate::app_state::spawn_tokio(cx, async move { host.edit_schedule(&id, edit).await });
        cx.spawn(async move |this, cx| {
            let result = job.await;
            let _ = this.update(cx, |ws, cx| {
                let state = &mut ws.linked_projects;
                let Some(host) = state.hosts.iter_mut().find(|h| {
                    h.credential == host_id && h.destination() == destination && h.token == token
                }) else {
                    return;
                };
                state.revision += 1;
                match result {
                    Ok(Some(task)) => {
                        state.schedule_writes.remove(&key);
                        if let Some(Ok(tasks)) = &mut host.data.schedules {
                            if let Some(row) = tasks.iter_mut().find(|r| r.id == task.id) {
                                *row = task;
                            }
                        }
                    }
                    Ok(None) => {
                        state.schedule_writes.remove(&key);
                        if let Some(Ok(tasks)) = &mut host.data.schedules {
                            tasks.retain(|task| task.id != key.1);
                        }
                        if state.runs.forget_task(host_id, &key.1) {
                            ws.persist(cx);
                        }
                    }
                    Err(error) => {
                        state
                            .schedule_writes
                            .insert(key, ScheduleWrite::Failed(error));
                    }
                }
                ws.refresh_linked_projects(false, cx);
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub(super) fn initial_local_task_project(&self, cx: &App) -> String {
        if self.linked_projects.scope.is_none() && !self.settings_project_scope.is_empty() {
            return self.settings_project_scope.clone();
        }
        let projects = self.settings_project_options(cx);
        self.active_project_path(cx)
            .filter(|path| projects.iter().any(|p| p.value == path))
            .or_else(|| projects.first().map(|p| p.value.to_owned()))
            .unwrap_or_default()
    }

    pub(in crate::views::workspace) fn scheduled_task_settings_rows(
        &self,
        width: f32,
        cx: &Context<'_, Self>,
    ) -> Vec<AnyElement> {
        let links = &self.linked_projects;
        let mut rows = vec![self.projects_scope_row(cx)];
        if let Some(message)=&links.editor_error {rows.push(div().id("linked-task-editor-error").px_4().role(Role::Alert).text_sm().text_color(theme::lychee_300()).child(message.clone()).into_any_element());}
        if links.opening_editor.is_some(){rows.push(div().id("linked-task-editor-loading").px_4().role(Role::Status).text_sm().child("Opening task editor…").into_any_element());}
        if !matches!(links.scope, Some(ProjectView::OnHost { .. })) {
            let project = if links.scope.is_some() {
                String::new()
            } else {
                self.settings_project_scope.clone()
            };
            let initial_project = self.initial_local_task_project(cx);
            let initial_model = self
                .project_defaults
                .get(&initial_project)
                .and_then(|p| p.model.clone())
                .unwrap_or_else(|| self.current_model.clone());
            if matches!(links.scope, Some(ProjectView::All)) {
                rows.push(
                    self.settings_heading("This environment", "")
                        .into_any_element(),
                );
            }
            rows.push(
                crate::views::reactive_automations::ScheduledTaskSettings {
                    panel: self.reactive_automations.clone(),
                    card_width: width,
                    project,
                    initial_project,
                    initial_model,
                }
                .into_any_element(),
            );
        }
        for host in &links.hosts {
            let path = match &links.scope {
                Some(ProjectView::All) => None,
                Some(ProjectView::OnHost {
                    host: selected,
                    path,
                }) if *selected == host.credential => path.as_deref(),
                _ => continue,
            };
            let connected = host.data.schedules.is_some() && host.problem.is_none();
            let detail = if host.tunnel.is_some() {
                "SSH"
            } else {
                "HTTPS"
            };
            rows.push(
                div()
                    .px_4()
                    .py_2()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_3()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .min_w_0()
                            .gap_2()
                            .child(
                                svg()
                                    .path(assets::TOOL_TERMINAL_ICON_PATH)
                                    .size(px(16.))
                                    .flex_shrink_0()
                                    .text_color(theme::rambutan_300()),
                            )
                            .child(
                                div()
                                    .min_w_0()
                                    .flex()
                                    .flex_col()
                                    .child(
                                        div()
                                            .id(SharedString::from(format!(
                                                "linked-schedules-heading-{}",
                                                host.credential
                                            )))
                                            .role(Role::Heading)
                                            .aria_level(3)
                                            .text_sm()
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .child(host.name()),
                                    )
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(theme::rambutan_300())
                                            .child(detail),
                                    ),
                            ),
                    )
                    .when(connected && host.data.can_edit_schedules, |row| {
                        let id=host.credential;
                        row.child(super::super::super::provider_editor::editor_button(format!("linked-new-task-{id}"),"New task",WebButtonVariant::Outline,links.opening_editor.is_some(),cx,move |ws,w,cx|ws.open_linked_task_editor(id,None,w,cx)))
                    })
                    .when(connected && !host.data.can_edit_schedules, |row| {
                        row.child(
                            div()
                                .flex_shrink_0()
                                .text_xs()
                                .text_color(theme::rambutan_300())
                                .child(
                                    if host.data.can_toggle_schedules
                                        || host.data.can_run_schedules
                                        || host.data.can_delete_schedules
                                    {
                                        "Connected"
                                    } else {
                                        "Connected · Read only"
                                    },
                                ),
                        )
                    })
                    .into_any_element(),
            );
            let status = if let Some(problem) = &host.problem {
                Some(problem.as_str())
            } else if let Some(result) = &host.data.schedules {
                match result {
                    Err(problem) => Some(problem.as_str()),
                    Ok(tasks) => {
                        let mut filtered = tasks
                            .iter()
                            .filter(|task| path.is_none_or(|p| p == task.project_path))
                            .peekable();
                        let empty = filtered.peek().is_none();
                        for task in filtered {
                            let id =
                                format!("linked-scheduled-task-{}-{}", host.credential, task.id);
                            let host_id = host.credential;
                            let task_id = task.id.clone();
                            let write = links.schedule_writes.get(&(host_id, task_id.clone()));
                            let active = host.data.can_toggle_schedules
                                && !matches!(write, Some(ScheduleWrite::Saving));
                            let enabled = task.enabled;
                            let toggle = switch(
                                format!("{id}-enabled"),
                                format!("Enable {}", task.title),
                                enabled,
                                Interactivity::disabled_if(!active),
                            )
                            .when(active, |el| {
                                on_press_view(el, cx, move |ws, _, cx| {
                                    ws.change_linked_schedule(
                                        host_id,
                                        task_id.clone(),
                                        ScheduleEdit::SetEnabled(!enabled),
                                        cx,
                                    )
                                })
                            });
                            let (menu, status) = self.linked_schedule_actions(
                                host,
                                task,
                                &id,
                                matches!(write, Some(ScheduleWrite::Saving)),
                                cx,
                            );
                            let controls = div()
                                .flex()
                                .items_center()
                                .gap_2()
                                .flex_shrink_0()
                                .child(toggle)
                                .children(menu);
                            rows.push(crate::views::reactive_automations::linked_scheduled_row(
                                id.clone(),
                                task,
                                controls,
                                self.linked_thread_link(host,task,cx),
                            ));
                            rows.extend(status);
                            if let Some(ScheduleWrite::Failed(error)) = write {
                                rows.push(
                                    div()
                                        .id(SharedString::from(format!("{id}-error")))
                                        .debug_selector(move || format!("{id}-error"))
                                        .role(Role::Alert)
                                        .px_4()
                                        .pb_2()
                                        .text_sm()
                                        .text_color(theme::lychee_300())
                                        .child(error.clone())
                                        .into_any_element(),
                                );
                            }
                        }
                        empty.then_some("No scheduled tasks match this selection.")
                    }
                }
            } else if host.data.schedules_allowed != Some(false) {
                Some("Loading scheduled tasks…")
            } else {
                Some("Link a Projects · Read scheduled tasks token in Environments to view tasks on this host.")
            };
            if let Some(status) = status {
                rows.push(
                    div()
                        .id(SharedString::from(format!(
                            "linked-schedules-status-{}",
                            host.credential
                        )))
                        .debug_selector({
                            let id = host.credential;
                            move || format!("linked-schedules-status-{id}")
                        })
                        .role(Role::Status)
                        .text_sm()
                        .text_color(theme::rambutan_300())
                        .px_3()
                        .py_2()
                        .child(status.to_owned())
                        .into_any_element(),
                );
            }
        }
        rows
    }
}

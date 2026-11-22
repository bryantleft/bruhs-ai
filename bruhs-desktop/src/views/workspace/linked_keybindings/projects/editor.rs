//! Bind the editor and its requests to the selected project grant.
mod host_picker;
use super::*;
use crate::views::reactive_automations::ReactiveAutomations;
use bruhs_models::reactive::{LinkedTaskDefinition, LinkedTaskSave, ScheduledTaskSummary};
use std::{collections::HashMap, sync::Arc};

#[derive(Clone)]
pub(crate) struct TaskEditorHost(pub(super) Arc<ProjectHost>);

impl TaskEditorHost {
    pub(crate) fn shares_conversations(&self) -> bool {
        self.0.data.task_conversations
    }
    pub(crate) async fn conversations(
        &self,
        path: &str,
        query: &str,
    ) -> Result<Vec<bruhs_models::reactive::TaskThreadChoice>, String> {
        self.reply(
            self.0
                .client()?
                .get(self.url(&["task-conversations"])?)
                .query(&[("path", path), ("query", query)]),
        )
        .await
    }

    pub(crate) fn name(&self) -> String {
        self.0.name()
    }

    pub(super) async fn reply<T: serde::de::DeserializeOwned>(
        &self,
        request: reqwest::RequestBuilder,
    ) -> Result<T, String> {
        let response = request
            .bearer_auth(&self.0.token)
            .timeout(Duration::from_secs(45))
            .send()
            .await
            .map_err(|_| {
                format!(
                    "Could not confirm the request on {}. Try again.",
                    self.name()
                )
            })?;
        match response.status() {
            reqwest::StatusCode::OK => schedules::schedule_reply(response, 2 * 1024 * 1024).await,
            reqwest::StatusCode::UNAUTHORIZED | reqwest::StatusCode::FORBIDDEN => {
                Err(format!("{} denied access to this request.", self.name()))
            }
            reqwest::StatusCode::CONFLICT
            | reqwest::StatusCode::BAD_REQUEST
            | reqwest::StatusCode::NOT_FOUND => {
                #[derive(serde::Deserialize)]
                struct Problem {
                    error: String,
                }
                Err(schedules::schedule_reply::<Problem>(response, 8192)
                    .await?
                    .error)
            }
            status => Err(format!(
                "{} could not complete this request ({status}).",
                self.name()
            )),
        }
    }
    pub(super) fn url(&self, segments: &[&str]) -> Result<reqwest::Url, String> {
        let mut url = self.0.endpoint.clone();
        url.path_segments_mut()
            .map_err(|_| "Invalid project host address.")?
            .clear()
            .extend(["api", "projects"])
            .extend(segments.iter().copied());
        Ok(url)
    }
    async fn read(&self, id: &str) -> Result<LinkedTaskDefinition, String> {
        let data: LinkedTaskDefinition = self
            .reply(self.0.client()?.get(self.url(&["task-editor", id])?))
            .await?;
        if data.setup.subscription.id != id || !data.setup.source.kind.is_scheduled_task() {
            return Err("The host returned a different task.".into());
        }
        Ok(data)
    }
    pub(crate) async fn save(&self, edit: &LinkedTaskSave) -> Result<(), String> {
        let id = &edit.setup.subscription.id;
        let row: ScheduledTaskSummary = self
            .reply(
                self.0
                    .client()?
                    .put(self.url(&["task-editor", id])?)
                    .json(edit),
            )
            .await?;
        if &row.id != id || row.enabled != edit.enabled {
            return Err("The host did not confirm the saved task.".into());
        }
        Ok(())
    }
    pub(crate) async fn models(
        &self,
        project: &str,
    ) -> Result<bruhs_models::delivery::writer_catalog::WriterCatalog, String> {
        self.reply(
            self.0
                .client()?
                .get(self.url(&["task-models"])?)
                .query(&[("path", project)]),
        )
        .await
    }
    pub(crate) async fn branches(
        &self,
        project: &str,
    ) -> Result<bruhs_core::git::Branches, String> {
        self.reply(
            self.0
                .client()?
                .get(self.url(&["task-branches"])?)
                .query(&[("path", project)]),
        )
        .await
    }
    fn current(&self, ws: &Workspace) -> bool {
        ws.settings_visible()
            && ws.settings_section == SettingsSection::ScheduledTasks
            && ws.linked_projects.hosts.iter().any(|h| {
                h.credential == self.0.credential
                    && h.destination() == self.0.destination()
                    && h.token == self.0.token
                    && h.data.can_edit_schedules
                    && h.problem.is_none()
            })
    }
}

pub(super) struct OpenEditor {
    host: Option<TaskEditorHost>,
    scope: Option<String>,
    panel: Entity<ReactiveAutomations>,
    defaults: Option<bruhs_core::linked_projects::TaskProjects>,
    trigger: Option<FocusHandle>,
    _watch: gpui::Subscription,
}

impl Workspace {
    pub(in crate::views::workspace) fn scheduled_panel(&self) -> &Entity<ReactiveAutomations> {
        self.linked_projects
            .editor
            .as_ref()
            .map(|e| &e.panel)
            .unwrap_or(&self.reactive_automations)
    }
    pub(in crate::views::workspace) fn scheduled_editor_projects(
        &self,
        cx: &App,
    ) -> Vec<DropdownOption> {
        let Some(host) = self
            .linked_projects
            .editor
            .as_ref()
            .and_then(|e| e.host.as_ref())
        else {
            return self.settings_project_options(cx);
        };
        let host = &host.0;
        let mut projects: BTreeMap<String, String> = host
            .data
            .projects
            .iter()
            .map(|p| (p.path.clone(), p.name.clone()))
            .collect();
        if let Some(defaults) = self
            .linked_projects
            .editor
            .as_ref()
            .and_then(|e| e.defaults.as_ref())
        {
            for path in defaults.projects.keys() {
                projects.entry(path.clone()).or_insert_with(|| path.clone());
            }
        }
        if let Some(Ok(tasks)) = &host.data.schedules {
            for t in tasks {
                projects
                    .entry(t.project_path.clone())
                    .or_insert_with(|| t.project_path.clone());
            }
        }
        projects
            .into_iter()
            .map(|(path, label)| DropdownOption {
                value: role_settings_ui::project_value(&path),
                label: label.into(),
                description: Some(path.into()),
            })
            .collect()
    }
    pub(in crate::views::workspace) fn scheduled_editor_models(&self) -> HashMap<String, String> {
        match self
            .linked_projects
            .editor
            .as_ref()
            .and_then(|e| e.defaults.as_ref())
        {
            Some(defaults) => defaults
                .projects
                .iter()
                .filter_map(|(p, m)| Some((p.clone(), m.clone()?)))
                .collect(),
            None => self
                .project_defaults
                .iter()
                .filter_map(|(p, d)| Some((p.clone(), d.model.clone()?)))
                .collect(),
        }
    }
    pub(in crate::views::workspace) fn scheduled_default_model(&self) -> String {
        self.linked_projects
            .editor
            .as_ref()
            .and_then(|e| e.defaults.as_ref())
            .map(|d| d.default_model.clone())
            .unwrap_or_else(|| self.current_model.clone())
    }
    fn task_editor_scope(&self) -> Option<String> {
        self.linked_projects.scope.as_ref().map(ProjectView::key)
    }

    pub(super) fn open_linked_task_editor(
        &mut self,
        id: uuid::Uuid,
        task: Option<String>,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.load_linked_editor(id, task, None, window.focused(cx), window, cx);
    }

    fn load_linked_editor(
        &mut self,
        id: uuid::Uuid,
        task: Option<String>,
        origin: Option<Entity<ReactiveAutomations>>,
        trigger: Option<FocusHandle>,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.set_settings_open_dropdown(None);
        let Some(h) = self
            .linked_projects
            .hosts
            .iter()
            .find(|h| h.credential == id && h.data.can_edit_schedules)
        else {
            return;
        };
        let path = task
            .as_ref()
            .and_then(|id| {
                h.data
                    .schedules
                    .as_ref()?
                    .as_ref()
                    .ok()?
                    .iter()
                    .find(|t| &t.id == id)
                    .map(|t| t.project_path.clone())
            })
            .or_else(|| match &self.linked_projects.scope {
                Some(ProjectView::OnHost { host, path }) if *host == id => path.clone(),
                _ => None,
            })
            .or_else(|| h.data.projects.first().map(|p| p.path.clone()));
        let host = TaskEditorHost(Arc::new(h.clone()));
        if !host.current(self) {
            return;
        }
        let version = uuid::Uuid::new_v4();
        let scope = self.task_editor_scope();
        if let Some(origin) = &origin {
            origin.update(cx, |p, cx| p.select_task_host(Some(version), cx));
        }
        self.linked_projects.opening_editor = Some(version);
        self.linked_projects.editor_error = None;
        let reader = host.clone();
        let read = crate::app_state::spawn_tokio(cx, async move {
            let load_task = async {
                match task {
                    Some(id) => reader.read(&id).await.map(Some),
                    None => Ok(None),
                }
            };
            let defaults = async {
                reader
                    .reply::<bruhs_core::linked_projects::TaskProjects>(
                        reader.0.client()?.get(reader.url(&["task-projects"])?),
                    )
                    .await
            };
            tokio::try_join!(load_task, defaults)
        });
        cx.spawn_in(window, async move |this, cx| {
            let data = read.await;
            let _ = this.update_in(cx, |ws, w, cx| {
                if ws.linked_projects.opening_editor != Some(version) {
                    return;
                }
                ws.linked_projects.opening_editor = None;
                if let Some(origin) = &origin {
                    if ws.scheduled_panel() != origin || !origin.read(cx).awaits_task_host(version)
                    {
                        cx.notify();
                        return;
                    }
                }
                if !host.current(ws) || ws.task_editor_scope() != scope {
                    if let Some(origin) = &origin {
                        origin.update(cx, |p, cx| {
                            p.task_host_failed(
                                "This host is no longer available for task editing.".into(),
                                cx,
                            )
                        });
                    }
                    cx.notify();
                    return;
                }
                match data {
                    Err(e) => {
                        if let Some(origin) = &origin {
                            origin.update(cx, |p, cx| p.task_host_failed(e, cx));
                        } else {
                            ws.linked_projects.editor_error = Some(e);
                        }
                    }
                    Ok((task, defaults)) => {
                        let path = path.or_else(|| defaults.projects.keys().next().cloned());
                        let model = path
                            .as_deref()
                            .and_then(|p| defaults.projects.get(p))
                            .and_then(Option::as_deref)
                            .unwrap_or(&defaults.default_model);
                        let panel = if let Some(origin) = &origin {
                            let local = &ws.reactive_automations;
                            cx.new(|cx| {
                                let mut panel = ReactiveAutomations::switched_task_panel(
                                    Some(host.clone()),
                                    local,
                                    cx,
                                );
                                panel.receive_task_draft(
                                    origin,
                                    path.as_deref().unwrap_or_default(),
                                    model,
                                    w,
                                    cx,
                                );
                                panel
                            })
                        } else {
                            cx.new(|cx| {
                                ReactiveAutomations::linked_editor(
                                    host.clone(),
                                    task,
                                    path.as_deref(),
                                    model,
                                    w,
                                    cx,
                                )
                            })
                        };
                        let watch = cx.observe(&panel, |_, _, cx| cx.notify());
                        ws.linked_projects.editor = Some(OpenEditor {
                            host: Some(host),
                            scope,
                            panel,
                            defaults: Some(defaults),
                            trigger,
                            _watch: watch,
                        });
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    pub(in crate::views::workspace) fn reconcile_linked_task_editor(
        &mut self,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let Some(editor) = &self.linked_projects.editor else {
            return;
        };
        let visible = self.settings_visible()
            && self.settings_section == SettingsSection::ScheduledTasks
            && editor.scope == self.task_editor_scope()
            && editor.host.as_ref().is_none_or(|h| h.current(self));
        if visible && editor.panel.read(cx).scheduled_editor_open() {
            return;
        }
        if editor.host.is_none() {
            self.reactive_automations
                .update(cx, |p, cx| p.refresh_snapshot(cx));
        }
        let trigger = editor.trigger.clone();
        self.linked_projects.editor = None;
        if visible {
            if let Some(focus) = trigger {
                focus.focus(window, cx);
            } else {
                self.focus_modal_dialog(window, cx);
            }
        }
        self.refresh_linked_projects(false, cx);
    }
}

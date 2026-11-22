//! Read a task's delivery history through its owning environment.
use super::*;
use bruhs_models::reactive::SourceKind;
use std::sync::Arc;

#[derive(Clone)]
pub(crate) struct LinkedDeliveries(Arc<ProjectHost>);

impl LinkedDeliveries {
    pub(crate) async fn read<T: serde::de::DeserializeOwned>(
        &self,
        task: &str,
        delivery: Option<&str>,
    ) -> Result<T, String> {
        let host = &self.0;
        let mut url = host.endpoint.clone();
        {
            let mut path = url
                .path_segments_mut()
                .map_err(|_| "Invalid project host address.")?;
            path.clear()
                .extend(["api", "projects", "scheduled-tasks", task, "deliveries"]);
            if let Some(id) = delivery {
                path.push(id);
            }
        }
        let response = host
            .client()?
            .get(url)
            .bearer_auth(&host.token)
            .send()
            .await
            .map_err(|_| {
                format!(
                    "Could not reach {} to read webhook deliveries.",
                    host.name()
                )
            })?;
        match response.status() {
            reqwest::StatusCode::OK => schedules::schedule_reply(response, 1024 * 1024).await,
            reqwest::StatusCode::FORBIDDEN | reqwest::StatusCode::UNAUTHORIZED => Err(format!(
                "{} denied access to webhook deliveries.",
                host.name()
            )),
            reqwest::StatusCode::NOT_FOUND => {
                Err("The task or delivery is no longer available.".into())
            }
            status => Err(format!(
                "Could not read webhook deliveries on {} ({status}).",
                host.name()
            )),
        }
    }

    fn current_title<'a>(&self, projects: &'a LinkedProjects, task: &str) -> Option<&'a str> {
        let host = projects.hosts.iter().find(|h| {
            h.credential == self.0.credential
                && h.token == self.0.token
                && h.destination() == self.0.destination()
                && h.data.can_read_deliveries
                && h.problem.is_none()
        })?;
        let path = match &projects.scope {
            Some(ProjectView::All) => None,
            Some(ProjectView::OnHost {
                host: selected,
                path,
            }) if *selected == host.credential => path.as_deref(),
            _ => return None,
        };
        host.data
            .schedules
            .as_ref()?
            .as_ref()
            .ok()?
            .iter()
            .find(|row| {
                row.id == task
                    && row.schedule == SourceKind::Webhook
                    && path.is_none_or(|p| p == row.project_path)
            })
            .map(|row| row.title.as_str())
    }

    pub(crate) fn name(&self) -> String {
        self.0.name()
    }
}

impl Workspace {
    pub(super) fn open_linked_deliveries(
        &mut self,
        host_id: uuid::Uuid,
        task: &str,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.set_settings_open_dropdown(None);
        cx.notify();
        let Some(host) = self
            .linked_projects
            .hosts
            .iter()
            .find(|h| h.credential == host_id && h.data.can_read_deliveries)
        else {
            return;
        };
        let reader = LinkedDeliveries(Arc::new(host.clone()));
        let Some(title) = reader.current_title(&self.linked_projects, task) else {
            return;
        };
        self.reactive_automations.update(cx, |panel, cx| {
            panel.open_linked_task_deliveries(task.to_owned(), title.to_owned(), reader, window, cx)
        });
    }

    pub(in crate::views::workspace) fn reconcile_linked_delivery_dialog(
        &self,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let panel = self.reactive_automations.read(cx);
        let Some((reader, task, title)) = panel.linked_delivery_origin() else {
            return;
        };
        let visible =
            self.settings_visible() && self.settings_section == SettingsSection::ScheduledTasks;
        let current = visible
            .then(|| reader.current_title(&self.linked_projects, task))
            .flatten();
        if current != Some(title) {
            self.reactive_automations.update(cx, |panel, cx| {
                panel.reconcile_linked_deliveries(current, visible, window, cx)
            });
        }
    }
}

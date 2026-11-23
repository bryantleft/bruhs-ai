//! A manual run keeps its request ID until the owning host acknowledges it.
use super::super::saved::SavedScheduleRun;
use super::*;
use bruhs_models::reactive::{ScheduledTaskSummary, SourceKind};
use bruhs_ui::dropdown::Dropdown;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use uuid::Uuid;

enum Progress {
    WaitingForSave(String),
    Sending,
    Failed(String),
    Accepted,
}

#[derive(Default)]
pub(super) struct ScheduleRuns {
    pub(super) pending: Vec<SavedScheduleRun>,
    progress: BTreeMap<(Uuid, String), Progress>,
}

impl ScheduleRuns {
    pub(super) fn restore(links: Option<&SavedLinks>) -> Self {
        Self {
            pending: links.map(|l| l.pending_runs.clone()).unwrap_or_default(),
            ..Self::default()
        }
    }

    pub(super) fn forget_task(&mut self, host: Uuid, task: &str) -> bool {
        let hash = format!("{:x}", Sha256::digest(task.as_bytes()));
        let before = self.pending.len();
        self.pending
            .retain(|r| r.host != host || r.task_hash != hash);
        self.progress.remove(&(host, hash));
        self.pending.len() != before
    }

    pub(super) fn relinked(&mut self, host: Uuid, changed_destination: bool) {
        self.progress.retain(|(id, _), _| *id != host);
        if changed_destination {
            self.pending.retain(|r| r.host != host);
        }
    }
}

impl ProjectHost {
    async fn run_now(&self, id: &str, request: Uuid) -> Result<(), String> {
        let mut url = self.endpoint.clone();
        url.path_segments_mut()
            .map_err(|_| "Invalid project host address.")?
            .clear()
            .extend([
                "api",
                "projects",
                "scheduled-tasks",
                id,
                "runs",
                &request.to_string(),
            ]);
        let response = self
            .client()?
            .put(url)
            .bearer_auth(&self.token)
            .timeout(Duration::from_secs(30))
            .send()
            .await
            .map_err(|_| {
                format!(
                    "{} did not confirm the run. Retry to check the same request.",
                    self.name()
                )
            })?;
        match response.status() {
            reqwest::StatusCode::NO_CONTENT => Ok(()),
            reqwest::StatusCode::UNAUTHORIZED | reqwest::StatusCode::FORBIDDEN => {
                Err("The host denied Run now access.".into())
            }
            reqwest::StatusCode::NOT_FOUND => {
                Err("This scheduled task is no longer available.".into())
            }
            reqwest::StatusCode::CONFLICT | reqwest::StatusCode::BAD_REQUEST => {
                #[derive(serde::Deserialize)]
                struct Problem {
                    error: String,
                }
                Err(schedules::schedule_reply::<Problem>(response, 8192)
                    .await?
                    .error)
            }
            status => Err(format!(
                "{} did not confirm the run ({status}). Retry to check the same request.",
                self.name()
            )),
        }
    }
}

impl Workspace {
    fn request_linked_run(&mut self, host: Uuid, id: &str, cx: &mut Context<'_, Self>) {
        self.set_settings_open_dropdown(None);
        let links = &mut self.linked_projects;
        let allowed = links
            .hosts
            .iter()
            .find(|h| h.credential == host && h.data.can_run_schedules)
            .and_then(|h| h.data.schedules.as_ref()?.as_ref().ok())
            .is_some_and(|rows| {
                rows.iter()
                    .any(|t| t.id == id && t.schedule != SourceKind::Webhook)
            });
        if !allowed {
            return;
        }
        let task_hash = format!("{:x}", Sha256::digest(id.as_bytes()));
        let key = (host, task_hash.clone());
        if matches!(
            links.runs.progress.get(&key),
            Some(Progress::WaitingForSave(_) | Progress::Sending)
        ) {
            return;
        }
        if !links
            .runs
            .pending
            .iter()
            .any(|r| r.host == host && r.task_hash == task_hash)
        {
            links.runs.pending.push(SavedScheduleRun {
                host,
                task_hash,
                request: Uuid::new_v4(),
            });
        }
        links
            .runs
            .progress
            .insert(key, Progress::WaitingForSave(id.to_owned()));
        self.persist_and_notify(cx);
    }

    pub(in crate::views::workspace) fn linked_run_requests_saved(
        &mut self,
        receipts: Vec<SavedScheduleRun>,
        saved: bool,
        cx: &mut Context<'_, Self>,
    ) {
        let links = &mut self.linked_projects;
        for receipt in receipts {
            let key = (receipt.host, receipt.task_hash.clone());
            if !links.runs.pending.contains(&receipt) {
                continue;
            }
            let Some(Progress::WaitingForSave(id)) = links.runs.progress.get(&key) else {
                continue;
            };
            let id = id.clone();
            if !saved {
                links.runs.progress.insert(
                    key,
                    Progress::Failed(
                        "Could not save the run request on this device. Retry to run the task."
                            .into(),
                    ),
                );
                continue;
            }
            let Some(host) = links
                .hosts
                .iter()
                .find(|h| h.credential == receipt.host && h.data.can_run_schedules)
                .cloned()
            else {
                links.runs.progress.insert(
                    key,
                    Progress::Failed("Reconnect to the host with Run now access and retry.".into()),
                );
                continue;
            };
            links.runs.progress.insert(key.clone(), Progress::Sending);
            let request = receipt.request;
            let job =
                crate::app_state::spawn_tokio(cx, async move { host.run_now(&id, request).await });
            cx.spawn(async move |this, cx| {
                let result = job.await;
                let _ = this.update(cx, |ws, cx| {
                    let runs = &mut ws.linked_projects.runs;
                    if !runs.pending.contains(&receipt) {
                        return;
                    }
                    match result {
                        Ok(()) => {
                            runs.pending.retain(|r| r != &receipt);
                            runs.progress.insert(key, Progress::Accepted);
                            ws.persist(cx);
                        }
                        Err(error) => {
                            runs.progress.insert(key, Progress::Failed(error));
                        }
                    }
                    cx.notify();
                });
            })
            .detach();
        }
    }

    pub(super) fn linked_schedule_actions(
        &self,
        host: &ProjectHost,
        task: &ScheduledTaskSummary,
        row: &str,
        writing: bool,
        cx: &Context<'_, Self>,
    ) -> (Option<AnyElement>, Option<AnyElement>) {
        let can_run = host.data.can_run_schedules && task.schedule != SourceKind::Webhook;
        let host_id = host.credential;
        let hash = format!("{:x}", Sha256::digest(task.id.as_bytes()));
        let pending = self
            .linked_projects
            .runs
            .pending
            .iter()
            .any(|r| r.host == host_id && r.task_hash == hash);
        let progress = self.linked_projects.runs.progress.get(&(host_id, hash));
        let busy = matches!(
            progress,
            Some(Progress::WaitingForSave(_) | Progress::Sending)
        );
        let status = match progress {
            Some(Progress::WaitingForSave(_) | Progress::Sending) => Some("Requesting run…"),
            Some(Progress::Failed(error)) => Some(error.as_str()),
            Some(Progress::Accepted) => Some("Run requested"),
            None if pending => Some("Run unconfirmed. Retry to check the same request."),
            None => None,
        }
        .map(|message| {
            let error = matches!(progress, Some(Progress::Failed(_)));
            let id = format!("{row}-run-status");
            div()
                .id(SharedString::from(id.clone()))
                .debug_selector(move || id)
                .role(if error { Role::Alert } else { Role::Status })
                .px_4()
                .pb_2()
                .text_sm()
                .text_color(if error {
                    theme::lychee_300()
                } else {
                    theme::rambutan_300()
                })
                .child(message.to_owned())
                .into_any_element()
        });
        let name = format!("Actions for {}", task.title);
        let trigger = role_settings_ui::project_value(&format!("{row}-actions"));
        let mut options = Vec::new();
        if host.data.can_edit_schedules {
            options.push(DropdownOption {
                value: "edit",
                label: "Edit task".into(),
                description: None,
            });
        }
        if can_run {
            options.push(DropdownOption {
                value: "run",
                label: if pending { "Retry run" } else { "Run now" }.into(),
                description: None,
            });
        }
        if host.data.can_read_deliveries && task.schedule == SourceKind::Webhook {
            options.push(DropdownOption {
                value: "deliveries",
                label: "Deliveries".into(),
                description: None,
            });
        }
        if host.data.can_delete_schedules {
            options.push(DropdownOption {
                value: "delete",
                label: "Delete".into(),
                description: None,
            });
        }
        let menu = if !options.is_empty() && !busy && !writing {
            let (toggle, _) = self.settings_dropdown_handlers(trigger, cx);
            let ws = cx.entity().downgrade();
            let task_id = task.id.clone();
            let select: SelectHandler = Rc::new(move |command, window, cx| {
                let _ = ws.update(cx, |ws, cx| match command {
                    "run" => ws.request_linked_run(host_id, &task_id, cx),
                    "deliveries" => ws.open_linked_deliveries(host_id, &task_id, window, cx),
                    "edit" => {
                        ws.open_linked_task_editor(host_id, Some(task_id.clone()), window, cx)
                    }
                    "delete" => ws.confirm_delete_linked_schedule(host_id, &task_id, window, cx),
                    _ => {}
                });
            });
            Dropdown::new(
                trigger,
                &name,
                "",
                vec![DropdownGroup {
                    label: None,
                    mark: None,
                    options,
                }],
                self.settings_listbox(trigger),
                toggle,
                select,
            )
            .action_menu(
                bruhs_ui::icons::ELLIPSIS,
                |action| {
                    let (icon, color) = match action {
                        "edit" => (assets::TOOL_PENCIL_ICON_PATH, theme::rambutan_300()),
                        "delete" => (bruhs_ui::icons::IconName::Trash.path(), theme::lychee_300()),
                        "deliveries" => (
                            crate::assets::CLIPBOARD_LIST_ICON_PATH,
                            theme::rambutan_400(),
                        ),
                        _ => (
                            bruhs_ui::icons::IconName::Play.path(),
                            theme::rambutan_400(),
                        ),
                    };
                    svg()
                        .path(icon)
                        .size(px(14.))
                        .text_color(color)
                        .into_any_element()
                },
                Some("delete"),
            )
            .end_aligned_popover(px(150.))
            .into_any_element()
        } else {
            web_button(
                trigger,
                WebButtonVariant::Ghost,
                WebButtonSize::IconSmall,
                ButtonState::Disabled,
            )
            .debug_selector(move || trigger.to_owned())
            .aria_label(name)
            .size(px(28.))
            .child(svg().path(bruhs_ui::icons::ELLIPSIS).size(px(16.)))
            .into_any_element()
        };
        (Some(menu), status)
    }
}

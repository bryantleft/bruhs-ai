//! Read project inventories from paired desktop environments.
use super::controls::shortcut_button as button;
use super::*;
use bruhs_core::linked_projects::Checkout;

mod deliveries;
mod updates;
pub(in crate::views::workspace) use updates::PeerUpdate;
mod editor;
mod thread;
pub(crate) use deliveries::LinkedDeliveries;
pub(crate) use editor::TaskEditorHost;
mod run;
mod schedules;
mod scope;
use scope::ProjectView;

#[derive(Clone, Default, PartialEq, Eq)]
struct ProjectInventory {
    projects: Vec<Checkout>,
    rename: bool,
    schedules_allowed: Option<bool>,
    can_toggle_schedules: bool,
    can_run_schedules: bool,
    can_delete_schedules: bool,
    can_read_deliveries: bool,
    can_edit_schedules: bool,
    task_conversations: bool,
    read_threads: bool,
    reply_threads: bool,
    usage: bool,
    usage_history: bool,
    edit_usage_prices: bool,
    provider_updates: bool,
    schedules: Option<Result<Vec<bruhs_models::reactive::ScheduledTaskSummary>, String>>,
}
type ProjectHost = LinkedHost<ProjectInventory>;

pub(in crate::views::workspace) struct LinkedProjects {
    hosts: Vec<ProjectHost>,
    updates: updates::HostUpdates,
    vault: Vault,
    form: Option<LinkForm>,
    busy: bool,
    reading: bool,
    revision: u64,
    rule_scope: Option<String>,
    scope: Option<ProjectView>,
    editor: Option<editor::OpenEditor>,
    thread: Option<thread::OpenThread>,
    opening_editor: Option<uuid::Uuid>,
    editor_error: Option<String>,
    runs: run::ScheduleRuns,
    schedule_writes: std::collections::BTreeMap<(uuid::Uuid, String), schedules::ScheduleWrite>,
    error: Option<String>,
    watch: Option<Task<()>>,
}

fn checkout_count(count: usize, kind: &str) -> String {
    let noun = if count == 1 { "checkout" } else { "checkouts" };
    format!("{count} {kind} {noun}")
}

impl LinkedProjects {
    pub(in crate::views::workspace) fn restore(saved: Option<&SavedLinks>) -> Self {
        Self {
            updates: Default::default(),
            hosts: saved.map(SavedLinks::restore).unwrap_or_default().0,
            vault: Vault::projects(),
            form: None,
            busy: false,
            reading: false,
            revision: 0,
            rule_scope: None,
            scope: None,
            editor: None,
            thread: None,
            opening_editor: None,
            editor_error: None,
            runs: run::ScheduleRuns::restore(saved),
            schedule_writes: Default::default(),
            error: None,
            watch: None,
        }
    }
    pub(in crate::views::workspace) fn checkouts(
        &self,
    ) -> impl Iterator<Item = (uuid::Uuid, String, &Checkout)> {
        self.hosts.iter().flat_map(|h| {
            h.data
                .projects
                .iter()
                .map(move |p| (h.credential, h.name(), p))
        })
    }
    pub(in crate::views::workspace) fn name_access(
        &self,
        id: uuid::Uuid,
        path: &str,
    ) -> Option<(bool, Option<String>)> {
        let h = self.hosts.iter().find(|h| h.credential == id)?;
        let checkout = h.data.projects.iter().find(|p| p.path == path)?;
        Some((h.data.rename, checkout.title.clone()))
    }

    pub(in crate::views::workspace) fn rename_on_host(
        &self,
        id: uuid::Uuid,
        edit: bruhs_core::linked_projects::names::Rename,
    ) -> Option<impl std::future::Future<Output = Result<(), String>> + Send + use<>> {
        let host = self
            .hosts
            .iter()
            .find(|h| h.credential == id && h.data.rename)?
            .clone();
        Some(async move {
            let mut url = host.endpoint.clone();
            url.set_path("/api/projects/names");
            let response = host
                .client()?
                .post(url)
                .bearer_auth(&host.token)
                .json(&edit)
                .timeout(Duration::from_secs(30))
                .send()
                .await
                .map_err(|_| format!("Could not reach {}.", host.name()))?;
            if response.status().is_success() {
                return Ok(());
            }
            let problem = match response.status() {
                reqwest::StatusCode::CONFLICT => {
                    "The project name changed. Review its current name and retry."
                }
                reqwest::StatusCode::NOT_FOUND => "The checkout is no longer available.",
                reqwest::StatusCode::FORBIDDEN | reqwest::StatusCode::UNAUTHORIZED => {
                    "Project rename access was denied."
                }
                _ => "Could not save the project name.",
            };
            Err(format!("{} · {problem}", host.name()))
        })
    }

    pub(in crate::views::workspace) fn saved(&self) -> SavedLinks {
        let mut links = SavedLinks::capture(&self.hosts, None);
        links.pending_runs = self.runs.pending.clone();
        links
    }
    pub(in crate::views::workspace) fn dismiss(&mut self) {
        self.form = None;
    }
    pub(in crate::views::workspace) fn usage_revision(&self) -> u64 {
        self.revision
    }

    pub(in crate::views::workspace) fn usage_readers(
        &self,
    ) -> impl Iterator<
        Item = (
            String,
            impl std::future::Future<
                    Output = Result<Vec<bruhs_core::usage_hubs::AccountUsage>, String>,
                > + Send
                + use<>,
        ),
    > + '_ {
        self.hosts.iter().filter(|h| h.share_usage).map(|h| {
            let mut host = h.clone();
            let vault = self.vault.clone();
            (host.name(), async move {
                if let Some(t) = host.tunnel.as_mut().filter(|t| t.socket.is_none()) {
                    let snapshot = crate::server_client::ServerClient::workbench_client()
                        .remote_hosts(None)
                        .await
                        .map_err(|_| format!("Reconnect {} in Environments.", t.name))?;
                    t.socket = t.resolve(&snapshot);
                }
                host.restore_token(vault, true).await?;
                let accounts = bruhs_core::usage_hubs::linked::read(
                    host.client()?,
                    host.endpoint,
                    &host.token,
                )
                .await?;
                Ok(accounts.into_iter().map(|a| a.into_account()).collect())
            })
        })
    }

    pub(in crate::views::workspace) fn viewing_inventory(&self) -> bool {
        self.scope.is_some()
    }
    pub(in crate::views::workspace) fn leave_project_view(&mut self) {
        self.scope = None;
        self.rule_scope = None;
    }
    pub(in crate::views::workspace) fn update_tunnels(&mut self, snapshot: &RemoteHostSnapshot) {
        for host in &mut self.hosts {
            if let Some(t) = &mut host.tunnel {
                let socket = t.resolve(snapshot);
                if t.socket != socket {
                    t.socket = socket;
                    host.data = ProjectInventory::default();
                    host.problem = Some(format!("Refresh projects after connecting {}.", t.name));
                    self.revision += 1;
                }
            }
        }
    }
}

impl ProjectHost {
    async fn read_projects(&self, load_schedules: bool) -> Result<ProjectInventory, String> {
        let mut url = self.endpoint.clone();
        url.set_path("/api/projects");
        let mut response = self
            .client()?
            .get(url)
            .timeout(Duration::from_secs(60))
            .bearer_auth(&self.token)
            .send()
            .await
            .map_err(|_| format!("Could not reach {}.", self.name()))?;
        match response.status() {
            reqwest::StatusCode::OK => {}
            reqwest::StatusCode::FORBIDDEN | reqwest::StatusCode::UNAUTHORIZED => {
                return Err(format!(
                    "{} denied project access. Link a current Projects token.",
                    self.name()
                ));
            }
            status => {
                return Err(format!(
                    "{} returned {status} while listing projects.",
                    self.name()
                ));
            }
        }
        let provider_updates = response
            .headers()
            .get("x-bruhs-provider-updates")
            .is_some_and(|v| v == "true");
        let rename = response
            .headers()
            .get("x-bruhs-edit-project-names")
            .is_some_and(|v| v == "true");
        let usage = response
            .headers()
            .get("x-bruhs-read-usage")
            .is_some_and(|v| v == "true");
        let usage_history = response
            .headers()
            .get("x-bruhs-read-usage-history")
            .is_some_and(|v| v == "true");
        let edit_usage_prices = usage_history
            && response
                .headers()
                .get("x-bruhs-edit-usage-prices")
                .is_some_and(|v| v == "true");
        let read_threads = response
            .headers()
            .get("x-bruhs-read-task-threads")
            .is_some_and(|v| v == "true");
        let reply_threads = response
            .headers()
            .get("x-bruhs-reply-task-threads")
            .is_some_and(|v| v == "true");
        let task_conversations = response
            .headers()
            .get("x-bruhs-task-conversations")
            .is_some_and(|v| v == "true");
        let can_edit_schedules = response
            .headers()
            .get("x-bruhs-edit-scheduled-tasks")
            .is_some_and(|v| v == "true");
        let can_read_deliveries = response
            .headers()
            .get("x-bruhs-webhook-deliveries")
            .is_some_and(|v| v == "true");
        let can_delete_schedules = response
            .headers()
            .get("x-bruhs-delete-scheduled-tasks")
            .is_some_and(|v| v == "true");
        let can_run_schedules = response
            .headers()
            .get("x-bruhs-run-scheduled-tasks")
            .is_some_and(|v| v == "true");
        let can_toggle_schedules = response
            .headers()
            .get("x-bruhs-toggle-scheduled-tasks")
            .is_some_and(|v| v == "true");
        let schedules_allowed = response
            .headers()
            .get("x-bruhs-read-scheduled-tasks")
            .is_some_and(|v| v == "true");
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| "The host sent an incomplete project list.")?
        {
            if bytes.len() + chunk.len() > 8 * 1024 * 1024 {
                return Err("The host’s project list is too large to display.".into());
            }
            bytes.extend_from_slice(&chunk);
        }
        let projects: Vec<Checkout> = serde_json::from_slice(&bytes)
            .map_err(|_| "The host returned an unreadable project list.")?;
        let mut seen = BTreeSet::new();
        if projects
            .iter()
            .any(|p| p.path.is_empty() || !seen.insert(&p.path))
        {
            return Err("The host returned an invalid checkout path or duplicate checkout.".into());
        }
        let schedules = if schedules_allowed && load_schedules {
            Some(self.read_schedules().await)
        } else {
            None
        };
        Ok(ProjectInventory {
            projects,
            provider_updates,
            rename,
            schedules_allowed: Some(schedules_allowed),
            can_toggle_schedules,
            can_run_schedules,
            can_delete_schedules,
            can_read_deliveries,
            can_edit_schedules,
            task_conversations,
            read_threads,
            reply_threads,
            usage,
            usage_history,
            edit_usage_prices,
            schedules,
        })
    }
}

impl Workspace {
    pub(in crate::views::workspace) fn open_project_link(
        &mut self,
        host: Option<&RemoteHost>,
        w: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if self.linked_projects.busy {
            return;
        }
        self.linked_projects.form = Some(LinkForm::new(host, "Project access token", w, cx));
        self.linked_projects.error = None;
        self.reveal_settings_row("settings-linked-projects".into(), cx);
        cx.notify();
    }
    fn connect_project_host(&mut self, cx: &mut Context<'_, Self>) {
        let state = &mut self.linked_projects;
        if state.busy {
            return;
        }
        let Some(form) = &state.form else {
            return;
        };
        let mut host = match ProjectHost::new(
            &form.origin.read(cx).value(),
            &form.token.read(cx).value(),
            form.tunnel.clone(),
        ) {
            Ok(h) => h,
            Err(e) => {
                state.error = Some(e);
                cx.notify();
                return;
            }
        };
        let mut changed_destination = false;
        if let Some(existing) = state.hosts.iter().find(|h| h.same_environment(&host)) {
            changed_destination = existing.destination() != host.destination();
            host.credential = existing.credential;
        }
        let vault = state.vault.clone();
        state.busy = true;
        state.revision += 1;
        state.error = None;
        let read = crate::app_state::spawn_tokio(cx, async move {
            host.data = host.read_projects(false).await?;
            host.share_usage = host.data.usage;
            host.share_usage_history = host.data.usage_history;
            host.edit_usage_prices = host.data.edit_usage_prices;
            host.save_token(vault).await?;
            Ok::<_, String>(host)
        });
        cx.spawn(async move |this, cx| {
            let result = read.await;
            let _ = this.update(cx, |this, cx| {
                let state = &mut this.linked_projects;
                state.busy = false;
                match result {
                    Ok(host) => {
                        state
                            .schedule_writes
                            .retain(|(id, _), _| *id != host.credential);
                        state.runs.relinked(host.credential, changed_destination);
                        if changed_destination {
                            this.project_list.grouping.forget_host(host.credential);
                        }
                        if let Some(old) =
                            state.hosts.iter_mut().find(|h| h.same_environment(&host))
                        {
                            *old = host;
                        } else {
                            state.hosts.push(host);
                        }
                        state.form = None;
                        this.persist(cx);
                    }
                    Err(e) => state.error = Some(e),
                }
                this.invalidate_project_search(cx);
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn unlink_project_host(&mut self, id: uuid::Uuid, cx: &mut Context<'_, Self>) {
        let state = &mut self.linked_projects;
        if state.busy {
            return;
        }
        state.busy = true;
        state.revision += 1;
        let vault = state.vault.clone();
        let job = cx.background_spawn(async move { vault.remove(id) });
        cx.spawn(async move |this, cx| {
            let result = job.await;
            let _ = this.update(cx, |this, cx| {
                let state = &mut this.linked_projects;
                state.busy = false;
                match result {
                    Ok(()) => {
                        state.hosts.retain(|h| h.credential != id);
                        state.runs.relinked(id, true);
                        state.schedule_writes.retain(|(host, _), _| *host != id);
                        state.error = None;
                        this.project_list.grouping.forget_host(id);
                        this.invalidate_project_search(cx);
                        this.persist(cx);
                    }
                    Err(e) => state.error = Some(e),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub(in crate::views::workspace) fn watch_linked_projects(
        &mut self,
        section: SettingsSection,
        cx: &mut Context<'_, Self>,
    ) {
        self.linked_projects.watch = None;
        if !matches!(
            section,
            SettingsSection::Projects | SettingsSection::ScheduledTasks
        ) {
            self.linked_projects.leave_project_view();
        }
        if section != SettingsSection::Environments {
            self.linked_projects.dismiss();
        }
        if !matches!(
            section,
            SettingsSection::Projects
                | SettingsSection::Environments
                | SettingsSection::ScheduledTasks
        ) {
            return;
        }
        self.refresh_linked_projects(true, cx);
        self.linked_projects.watch = Some(cx.spawn(async move |this, cx| loop {
            cx.background_executor()
                .timer(Duration::from_secs(15))
                .await;
            if this
                .update(cx, |this, cx| {
                    if !this.settings_visible() || this.settings_section != section {
                        return false;
                    }
                    this.refresh_linked_projects(false, cx);
                    true
                })
                .ok()
                != Some(true)
            {
                break;
            }
        }));
    }

    pub(in crate::views::workspace) fn refresh_after_project_rename(
        &mut self,
        cx: &mut Context<'_, Self>,
    ) {
        self.linked_projects.revision += 1;
        self.refresh_linked_projects(true, cx);
    }

    pub(in crate::views::workspace) fn refresh_linked_projects(
        &mut self,
        manual: bool,
        cx: &mut Context<'_, Self>,
    ) {
        let load_schedules = self.settings_section == SettingsSection::ScheduledTasks;
        let state = &mut self.linked_projects;
        if self.project_names.busy || state.busy || state.reading || state.hosts.is_empty() {
            return;
        }
        let revision = state.revision;
        state.reading = true;
        let hosts = state.hosts.clone();
        let vault = state.vault.clone();
        let job = crate::app_state::spawn_tokio(cx, async move {
            let tunnels = if hosts
                .iter()
                .any(|h| h.tunnel.as_ref().is_some_and(|t| t.socket.is_none()))
            {
                crate::server_client::ServerClient::workbench_client()
                    .remote_hosts(None)
                    .await
                    .ok()
            } else {
                None
            };
            join_all(hosts.into_iter().map(|mut h| {
                let vault = vault.clone();
                let tunnels = tunnels.as_ref();
                async move {
                    if let (Some(t), Some(s)) = (&mut h.tunnel, tunnels) {
                        t.socket = t.resolve(s);
                    }
                    let read = async {
                        h.restore_token(vault, manual).await?;
                        h.read_projects(load_schedules).await
                    }
                    .await;
                    match read {
                        Ok(projects) => {
                            h.share_usage = projects.usage;
                            h.share_usage_history = projects.usage_history;
                            h.edit_usage_prices = projects.edit_usage_prices;
                            h.data = projects;
                            h.problem = None;
                        }
                        Err(e) => {
                            h.data = ProjectInventory::default();
                            h.problem = Some(e);
                            if let Some(t) = &mut h.tunnel {
                                t.socket = None;
                            }
                        }
                    }
                    h
                }
            }))
            .await
        });
        cx.spawn(async move |this, cx| {
            let hosts = job.await;
            let _ = this.update(cx, |this, cx| {
                let state = &mut this.linked_projects;
                state.reading = false;
                if state.revision == revision
                    && load_schedules == (this.settings_section == SettingsSection::ScheduledTasks)
                {
                    let changed = state
                        .hosts
                        .iter()
                        .zip(&hosts)
                        .any(|(a, b)| a.data != b.data || a.problem != b.problem);
                    state.hosts = hosts;
                    this.sync_linked_provider_checks(cx);
                    if changed {
                        this.invalidate_project_search(cx);
                    }
                } else {
                    this.refresh_linked_projects(false, cx);
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn invalidate_project_search(&mut self, cx: &mut Context<'_, Self>) {
        self.settings_search_catalog.clear();
        if self.settings_visible() && !self.settings_search.read(cx).value().trim().is_empty() {
            self.refresh_settings_search(cx);
        }
    }

    pub(in crate::views::workspace) fn project_environment_rows(
        &self,
        cx: &Context<'_, Self>,
    ) -> Vec<AnyElement> {
        let state = &self.linked_projects;
        let mut rows = Vec::new();
        for h in &state.hosts {
            let id = h.credential;
            let detail = h
                .problem
                .clone()
                .unwrap_or_else(|| checkout_count(h.data.projects.len(), "saved"));
            rows.push(
                self.settings_row(
                    format!("settings-project-host-{id}"),
                    h.name(),
                    detail,
                    div()
                        .flex()
                        .flex_wrap()
                        .gap_2()
                        .child(on_press_view(
                            button(format!("view-projects-{id}"), "View projects", state.busy),
                            cx,
                            move |this, _, cx| {
                                this.show_linked_project_scope(id, None, cx);
                            },
                        ))
                        .child(on_press_view(
                            button(format!("unlink-projects-{id}"), "Disconnect", state.busy),
                            cx,
                            move |this, _, cx| this.unlink_project_host(id, cx),
                        ))
                        .into_any_element(),
                ),
            );
        }
        let mut controls = div().flex().flex_col().items_start().gap_2();
        if let Some(form) = &state.form {
            if let Some(t) = &form.tunnel {
                controls = controls.child(t.name.clone());
            } else {
                controls = controls
                    .child(Input::new(&form.origin).aria_label("Project host HTTPS address"));
            }
            controls = controls
                .child(Input::new(&form.token).aria_label("Project access token"))
                .child(
                    div()
                        .flex()
                        .gap_2()
                        .child(on_press_view(
                            button("connect-project-host", "Connect", state.busy),
                            cx,
                            |this, _, cx| this.connect_project_host(cx),
                        ))
                        .child(on_press_view(
                            button("cancel-project-link", "Cancel", state.busy),
                            cx,
                            |this, _, cx| {
                                this.linked_projects.dismiss();
                                this.linked_projects.error = None;
                                cx.notify();
                            },
                        )),
                );
        } else {
            controls = controls.child(on_press_view(
                button("link-project-host", "Link projects", state.busy),
                cx,
                |this, w, cx| this.open_project_link(None, w, cx),
            ));
        }
        controls = controls.child(on_press_view(
            button("manage-project-access", "Manage access", false),
            cx,
            |_, _, cx| {
                if let Ok(url) =
                    crate::server_client::ServerClient::workbench_client().shortcut_access_url()
                {
                    cx.open_url(url.as_str());
                }
            },
        ));
        rows.push(self.settings_row(
            "settings-linked-projects",
            "Linked projects",
            state.error.as_deref().unwrap_or("Link a Projects token from the host. Its access determines whether you can rename checkouts or view scheduled tasks."),
            controls.into_any_element(),
        ));
        rows
    }
}

impl Workspace {
    pub(in crate::views::workspace) fn open_linked_checkout(
        &mut self,
        member: &project_grouping::Member,
        w: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let Some(host) = member.host else { return };
        self.close_transient_surfaces();
        self.open_settings(&OpenSettings, w, cx);
        self.show_linked_project_scope(host, Some(member.path.clone()), cx);
    }

    pub(in crate::views::workspace) fn linked_checkout_cards(
        &self,
        cx: &Context<'_, Self>,
    ) -> Vec<AnyElement> {
        let error = self.removal_error().map(|error| {
            div()
                .id("project-removal-error")
                .debug_selector(|| "project-removal-error".into())
                .px_4()
                .py_2()
                .role(Role::Alert)
                .text_sm()
                .text_color(theme::persimmon_400())
                .child(error.to_owned())
                .into_any_element()
        });
        let all = matches!(self.linked_projects.scope, Some(ProjectView::All));
        if !all
            && (!self.settings_project_scope.is_empty() || !self.linked_projects.hosts.is_empty())
        {
            return error.into_iter().collect();
        }
        let state = &self.linked_projects;
        let mut status = Vec::new();
        let mut count = 0;
        for h in &state.hosts {
            count += h.data.projects.len();
            if let Some(error) = &h.problem {
                status.push(error.clone());
            }
        }
        let summary = if status.is_empty() {
            checkout_count(count, "linked")
        } else {
            status.join(" · ")
        };
        let mut rows = Vec::new();
        if !state.hosts.is_empty() {
            rows.push(
                self.settings_row(
                    "linked-project-inventory",
                    "Linked environments",
                    summary,
                    on_press_view(
                        button(
                            "refresh-linked-projects",
                            "Refresh",
                            state.reading || state.busy,
                        ),
                        cx,
                        |ws, _, cx| ws.refresh_linked_projects(true, cx),
                    )
                    .into_any_element(),
                ),
            );
        }
        rows.extend(error);
        let mut slot = 0;
        for group in self.grouped_projects() {
            rows.push(self.group_name_row(&group, cx));
            // Keep repeated checkouts grouped by machine.
            let multiple_checkouts = group.members.len() > 1;
            let permitted = group.members.iter().all(|p| p.host.is_none());
            let title = if multiple_checkouts {
                "Remove this project everywhere"
            } else {
                "Remove project"
            };
            let key = element_key(&group.key);
            let detail = if permitted {
                "Remove the checkout entries and their conversations. Files stay on disk."
            } else {
                "A linked checkout can be removed on its host."
            };
            let remove_group = self.settings_row(
                format!("remove-group-row-{key}"),
                title,
                detail,
                self.remove_checkout_button(
                    format!("remove-group-{key}"),
                    project_removal::Placement::Section,
                    project_removal::Target::group(&group),
                    permitted,
                    cx,
                ),
            );
            let mut environments = Vec::<Vec<project_grouping::Member>>::new();
            for member in group.members {
                if let Some(bucket) = environments
                    .iter_mut()
                    .find(|members| members[0].host == member.host)
                {
                    bucket.push(member);
                } else {
                    environments.push(vec![member]);
                }
            }
            let multiple = environments.len() > 1;
            for members in environments {
                let nested = multiple && members.len() > 1;
                if nested {
                    rows.push(
                        div()
                            .px_4()
                            .py_2()
                            .text_sm()
                            .text_color(theme::rambutan_300())
                            .child(members[0].environment.clone())
                            .into_any_element(),
                    );
                }
                for member in members {
                    rows.push(self.checkout_member_row(
                        member,
                        slot,
                        nested,
                        multiple_checkouts,
                        cx,
                    ));
                    slot += 1;
                }
            }
            rows.push(remove_group);
        }

        if rows.is_empty() {
            return Vec::new();
        }
        vec![self.settings_card(
            "Checkouts",
            "Choose how each checkout joins project groups on this desktop.",
            rows,
        )]
    }
    fn checkout_member_row(
        &self,
        member: project_grouping::Member,
        slot: usize,
        nested: bool,
        offer_removal: bool,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        let scope = member.scope();
        let id = role_settings_ui::project_value(&format!("linked-checkout-rule-{slot}"));
        let target_scope = scope.clone();
        let workspace = cx.entity().downgrade();
        let toggle: ToggleHandler = Rc::new(move |_, cx| {
            let _ = workspace.update(cx, |ws, cx| {
                let close = ws.settings_open_dropdown() == Some(id)
                    && ws.linked_projects.rule_scope.as_deref() == Some(&target_scope);
                ws.linked_projects.rule_scope = (!close).then(|| target_scope.clone());
                ws.set_settings_open_dropdown((!close).then_some(id));
                cx.notify();
            });
        });
        let entity = cx.entity().downgrade();
        let target = member.clone();
        let select: SelectHandler = Rc::new(move |value, _, cx| {
            let _ = entity.update(cx, |ws, cx| {
                ws.set_settings_open_dropdown(None);
                if let Some(host) = target.host {
                    if !ws
                        .linked_projects
                        .checkouts()
                        .any(|(id, _, p)| id == host && p.path == target.path)
                    {
                        cx.notify();
                        return;
                    }
                }
                ws.set_checkout_grouping(target.scope(), value, cx);
            });
        });
        let label = format!(
            "Grouping rule for {} on {}",
            member.path, member.environment
        );
        let listbox = if self.linked_projects.rule_scope.as_deref() == Some(&scope) {
            self.settings_listbox(id)
        } else {
            Listbox::Closed
        };
        let control = self.checkout_rule_control(id, &scope, &label, listbox, toggle, select);
        let control = if offer_removal {
            div()
                .flex()
                .flex_wrap()
                .items_center()
                .gap_2()
                .min_w_0()
                .max_w_full()
                .child(control)
                .child(self.remove_checkout_button(
                    format!("remove-checkout-{}", element_key(&scope)),
                    project_removal::Placement::MemberRow,
                    project_removal::Target::Checkout(member.path.clone()),
                    member.host.is_none(),
                    cx,
                ))
                .into_any_element()
        } else {
            control
        };
        let row_id = member.row_id();
        let (title, description) = if nested {
            (member.path, String::new())
        } else {
            (member.environment, member.path)
        };
        self.settings_row(row_id, title, description, control)
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::super::tests::HostFixture;
    use super::*;
    use crate::views::workspace::tests::launch;
    use ::core::prelude::v1::test;
    use std::sync::atomic::Ordering;

    #[gpui::test]
    async fn linked_run_menu_retries_the_saved_request_after_relaunch(
        cx: &mut gpui::TestAppContext,
    ) {
        launch::init(cx);
        let mut next_launch = cx.clone();
        let host = HostFixture::new("Studio", "alt+6");
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        let (ws, win) = launch::launch(cx, &path);
        ws.update_in(win, |ws, w, cx| {
            ws.open_settings(&OpenSettings, w, cx);
            ws.set_settings_section(SettingsSection::Environments, cx);
            ws.open_project_link(Some(&host.host), w, cx);
            ws.linked_projects
                .form
                .as_ref()
                .unwrap()
                .token
                .update(cx, |input, cx| input.set_value("d".repeat(64), w, cx));
            ws.connect_project_host(cx);
        });
        win.condition(&ws, |ws, _| ws.linked_projects.hosts.len() == 1)
            .await;
        let (host_id, vault) = ws.read_with(win, |ws, _| {
            (
                ws.linked_projects.hosts[0].credential,
                ws.linked_projects.vault.clone(),
            )
        });
        let show = |ws: &Entity<Workspace>, win: &mut gpui::VisualTestContext| {
            ws.update_in(win, |ws, w, cx| {
                ws.open_settings(&OpenSettings, w, cx);
                ws.set_settings_section(SettingsSection::ScheduledTasks, cx);
                ws.select_settings_option(
                    "settings-project-defaults-scope",
                    role_settings_ui::project_value(&format!("linked:{host_id}:/srv/editor")),
                    w,
                    cx,
                );
            });
        };
        let select = |win: &mut gpui::VisualTestContext, keyboard: bool| {
            let selector = format!("linked-scheduled-task-{host_id}-daily-actions");
            win.run_until_parked();
            let trigger = win
                .debug_bounds(role_settings_ui::project_value(&selector))
                .expect("actions");
            win.simulate_click(trigger.center(), gpui::Modifiers::default());
            win.run_until_parked();
            let item = format!("{selector}-option-run");
            let bounds = win
                .debug_bounds(role_settings_ui::project_value(&item))
                .expect("Run now");
            if keyboard {
                win.simulate_keystrokes("enter");
            } else {
                win.simulate_click(bounds.center(), gpui::Modifiers::default());
            }
            win.run_until_parked();
        };
        show(&ws, win);
        win.condition(&ws, |ws, _| {
            ws.linked_projects.hosts[0].data.schedules.is_some()
        })
        .await;
        select(win, true);
        win.condition(&ws, |_, _| host.run_attempts.lock().unwrap().len() == 1)
            .await;
        launch::saved(&ws, win).await;
        let persisted = std::fs::read_to_string(&path).unwrap();
        let request = host.run_attempts.lock().unwrap()[0].clone();
        assert!(persisted.contains(&request) && !persisted.contains("/srv/editor"));
        launch::quit(&ws, win);
        let (ws, win) = launch::launch(&mut next_launch, &path);
        ws.update_in(win, |ws, _, _| {
            ws.linked_projects.vault = vault;
            ws.linked_projects.update_tunnels(&RemoteHostSnapshot {
                hosts: vec![host.host.clone()],
                error: None,
            });
        });
        show(&ws, win);
        win.condition(&ws, |ws, _| {
            ws.linked_projects.hosts[0].data.schedules.is_some()
        })
        .await;
        assert_eq!(host.run_attempts.lock().unwrap().len(), 1);
        select(win, false);
        win.condition(&ws, |ws, _| ws.linked_projects.runs.pending.is_empty())
            .await;
        assert_eq!(
            host.run_attempts.lock().unwrap().as_slice(),
            [request.clone(), request.clone()]
        );
        launch::saved(&ws, win).await;
        assert!(!std::fs::read_to_string(&path).unwrap().contains(&request));
    }

    async fn pair_schedule_fixture(
        ws: &Entity<Workspace>,
        win: &mut gpui::VisualTestContext,
        host: &HostFixture,
        token: &str,
    ) {
        let before = ws.read_with(win, |ws, _| ws.linked_projects.hosts.len());
        ws.update_in(win, |ws, window, cx| {
            ws.open_project_link(Some(&host.host), window, cx);
            ws.linked_projects
                .form
                .as_ref()
                .expect("project link form open")
                .token
                .update(cx, |field, cx| {
                    field.set_value(token.repeat(64), window, cx);
                });
            ws.connect_project_host(cx);
        });
        win.condition(ws, |ws, _| ws.linked_projects.hosts.len() == before + 1)
            .await;
    }

    #[gpui::test]
    async fn linked_webhook_history_refreshes_and_closes_on_revocation(
        cx: &mut gpui::TestAppContext,
    ) {
        launch::init(cx);
        let hosts = [
            HostFixture::new("Build", "alt+5"),
            HostFixture::new("Studio", "alt+6"),
        ];
        let dir = tempfile::tempdir().unwrap();
        let (ws, win) = launch::launch(cx, &dir.path().join("state.json"));
        ws.update_in(win, |ws, w, cx| {
            ws.open_settings(&OpenSettings, w, cx);
            ws.set_settings_section(SettingsSection::Environments, cx);
        });
        for host in &hosts {
            pair_schedule_fixture(&ws, win, host, "8").await;
        }
        ws.update_in(win, |ws, _, cx| {
            ws.set_settings_section(SettingsSection::ScheduledTasks, cx)
        });
        win.condition(&ws, |ws, _| {
            ws.linked_projects
                .hosts
                .iter()
                .all(|h| matches!(&h.data.schedules, Some(Ok(tasks)) if tasks.len() == 2))
        })
        .await;
        let click = |win: &mut gpui::VisualTestContext, id: &'static str| {
            win.run_until_parked();
            let bounds = win.debug_bounds(id).expect(id);
            win.simulate_click(bounds.center(), gpui::Modifiers::default());
            win.run_until_parked();
        };
        for (index, selector) in ["delivery-row-build-request", "delivery-row-studio-request"]
            .into_iter()
            .enumerate()
        {
            let trigger = ws.update_in(win, |ws, w, cx| {
                let host = ws.linked_projects.hosts[index].credential;
                ws.select_settings_option(
                    "settings-project-defaults-scope",
                    role_settings_ui::project_value(&format!("linked:{host}:/srv/reports")),
                    w,
                    cx,
                );
                role_settings_ui::project_value(&format!(
                    "linked-scheduled-task-{host}-report-actions"
                ))
            });
            click(win, trigger);
            click(
                win,
                role_settings_ui::project_value(&format!("{trigger}-option-deliveries")),
            );
            assert!(win.debug_bounds("delivery-empty").is_some());
            click(win, "reactive-Refresh");
            click(win, selector);
            assert!(win.debug_bounds("delivery-section-Body").is_some());
            if index == 0 {
                click(win, "reactive-Back");
                assert!(win.debug_bounds(selector).is_some());
                click(win, "reactive-Done");
                win.simulate_keystrokes("enter");
                win.run_until_parked();
                assert!(win
                    .debug_bounds(role_settings_ui::project_value(&format!(
                        "{trigger}-option-deliveries"
                    )))
                    .is_some());
                win.simulate_keystrokes("escape");
            } else {
                hosts[index].revoked.store(true, Ordering::SeqCst);
                ws.update_in(win, |ws, _, cx| ws.refresh_linked_projects(true, cx));
                win.condition(&ws, |ws, cx| {
                    ws.reactive_automations
                        .read(cx)
                        .linked_delivery_origin()
                        .is_none()
                })
                .await;
            }
        }
        win.run_until_parked();
        assert!(win.debug_bounds("scheduled-task-dialog").is_none());
    }

    #[gpui::test]
    async fn linked_schedule_delete_confirms_and_targets_its_host(cx: &mut gpui::TestAppContext) {
        launch::init(cx);
        let hosts = [
            HostFixture::new("Build", "alt+5"),
            HostFixture::new("Studio", "alt+6"),
        ];
        let dir = tempfile::tempdir().unwrap();
        let (ws, win) = launch::launch(cx, &dir.path().join("state.json"));
        ws.update_in(win, |ws, window, cx| {
            ws.open_settings(&OpenSettings, window, cx);
            ws.set_settings_section(SettingsSection::Environments, cx);
        });
        pair_schedule_fixture(&ws, win, &hosts[0], "e").await;
        pair_schedule_fixture(&ws, win, &hosts[1], "9").await;
        ws.update_in(win, |ws, _, cx| {
            ws.set_settings_section(SettingsSection::ScheduledTasks, cx)
        });
        win.condition(&ws, |ws, _| {
            ws.linked_projects
                .hosts
                .iter()
                .all(|h| matches!(&h.data.schedules, Some(Ok(tasks)) if tasks.len() == 2))
        })
        .await;
        let select_host = |win: &mut gpui::VisualTestContext, index: usize| {
            ws.update_in(win, |ws, w, cx| {
                let host = ws.linked_projects.hosts[index].credential;
                let scope = role_settings_ui::project_value(&format!("linked:{host}:/srv/reports"));
                ws.select_settings_option("settings-project-defaults-scope", scope, w, cx);
                role_settings_ui::project_value(&format!(
                    "linked-scheduled-task-{host}-report-actions"
                ))
            })
        };
        let click = |win: &mut gpui::VisualTestContext, id: &'static str| {
            win.run_until_parked();
            let bounds = win.debug_bounds(id).expect(id);
            win.simulate_click(bounds.center(), gpui::Modifiers::default());
            win.run_until_parked();
        };
        let readonly = select_host(win, 0);
        click(win, readonly);
        assert!(win
            .debug_bounds(role_settings_ui::project_value(&format!(
                "{readonly}-option-delete"
            )))
            .is_none());
        let writable = select_host(win, 1);
        let delete = role_settings_ui::project_value(&format!("{writable}-option-delete"));
        for choice in ["Cancel", "Delete"] {
            click(win, writable);
            assert!(win.debug_bounds(delete).is_some());
            assert!(win
                .debug_bounds(role_settings_ui::project_value(&format!(
                    "{writable}-option-run"
                )))
                .is_none());
            win.simulate_keystrokes("enter");
            win.run_until_parked();
            assert!(win.debug_bounds("scheduled-task-dialog").is_some());
            assert!(!hosts[1].report_deleted.load(Ordering::SeqCst));
            if choice == "Cancel" {
                win.simulate_keystrokes("escape");
            } else {
                click(win, "schedule-delete-confirm");
            }
            win.run_until_parked();
        }
        win.condition(&ws, |ws, _| matches!(&ws.linked_projects.hosts[1].data.schedules, Some(Ok(tasks)) if tasks.len() == 1)).await;
        assert!(hosts[1].report_deleted.load(Ordering::SeqCst));
        assert!(!hosts[0].report_deleted.load(Ordering::SeqCst));
        assert!(win.debug_bounds(writable).is_none());
        select_host(win, 0);
        win.run_until_parked();
        assert!(win.debug_bounds(readonly).is_some());
    }

    #[gpui::test]
    async fn linked_schedules_filter_by_owning_host_and_project(cx: &mut gpui::TestAppContext) {
        launch::init(cx);
        let hosts = [
            HostFixture::new("Build", "alt+5"),
            HostFixture::new("Studio", "alt+6"),
        ];
        let dir = tempfile::tempdir().unwrap();
        let (ws, win) = launch::launch(cx, &dir.path().join("state.json"));
        ws.update_in(win, |ws, w, cx| {
            ws.open_settings(&OpenSettings, w, cx);
            ws.set_settings_section(SettingsSection::Environments, cx);
        });
        for (i, host) in hosts.iter().enumerate() {
            ws.update_in(win, |ws, w, cx| {
                ws.open_project_link(Some(&host.host), w, cx);
                ws.linked_projects
                    .form
                    .as_ref()
                    .unwrap()
                    .token
                    .update(cx, |field, cx| {
                        field.set_value(if i == 0 { "e" } else { "f" }.repeat(64), w, cx)
                    });
                ws.connect_project_host(cx);
            });
            win.condition(&ws, |ws, _| ws.linked_projects.hosts.len() == i + 1)
                .await;
        }
        ws.update_in(win, |ws, _, cx| {
            ws.set_settings_section(SettingsSection::ScheduledTasks, cx)
        });
        win.condition(&ws, |ws, _| {
            ws.linked_projects
                .hosts
                .iter()
                .all(|h| matches!(&h.data.schedules, Some(Ok(tasks)) if tasks.len() == 2))
        })
        .await;
        let ids = ws.read_with(win, |ws, _| {
            ws.linked_projects
                .hosts
                .iter()
                .map(|h| h.credential)
                .collect::<Vec<_>>()
        });
        let select = |win: &mut gpui::VisualTestContext, scope: &str| {
            win.run_until_parked();
            let trigger = win
                .debug_bounds("settings-project-defaults-scope")
                .expect("scope picker");
            win.simulate_click(trigger.center(), gpui::Modifiers::default());
            win.run_until_parked();
            let selector = format!("settings-project-defaults-scope-option-{scope}");
            let option = win
                .debug_bounds(role_settings_ui::project_value(&selector))
                .expect("scope option");
            win.simulate_click(option.center(), gpui::Modifiers::default());
            win.run_until_parked();
        };
        let row = |i: usize, task: &str| {
            role_settings_ui::project_value(&format!("linked-scheduled-task-{}-{task}", ids[i]))
        };
        select(win, "projects:all");
        assert!(
            win.debug_bounds(row(0, "daily")).is_some()
                && win.debug_bounds(row(1, "daily")).is_some()
        );
        select(win, &format!("linked:{}:/srv/reports", ids[1]));
        assert!(win.debug_bounds(row(1, "report")).is_some());
        assert!(
            win.debug_bounds(row(1, "daily")).is_none()
                && win.debug_bounds(row(0, "report")).is_none()
        );
        let click_switch = |win: &mut gpui::VisualTestContext, i: usize| {
            let selector = format!("{}-enabled", row(i, "report"));
            let bounds = win
                .debug_bounds(role_settings_ui::project_value(&selector))
                .expect("task switch");
            win.simulate_click(bounds.center(), gpui::Modifiers::default());
            win.run_until_parked();
        };
        let key = (ids[1], "report".to_owned());
        click_switch(win, 1);
        win.condition(&ws, |ws, _| {
            matches!(
                ws.linked_projects.schedule_writes.get(&key),
                Some(schedules::ScheduleWrite::Failed(_))
            )
        })
        .await;
        win.run_until_parked();
        assert!(win
            .debug_bounds(role_settings_ui::project_value(&format!(
                "{}-error",
                row(1, "report")
            )))
            .is_some());
        click_switch(win, 1);
        win.condition(&ws, |ws, _| {
            ws.linked_projects.hosts[1]
                .data
                .schedules
                .as_ref()
                .and_then(|tasks| tasks.as_ref().ok())
                .is_some_and(|tasks| tasks.iter().any(|t| t.id == "report" && t.enabled))
        })
        .await;
        assert_eq!(hosts[1].schedule_attempts.load(Ordering::SeqCst), 2);
        select(win, &format!("linked:{}:/srv/reports", ids[0]));
        click_switch(win, 0);
        assert_eq!(hosts[0].schedule_attempts.load(Ordering::SeqCst), 0);
        select(win, &format!("linked:{}:/srv/reports", ids[1]));
        hosts[1].revoked.store(true, Ordering::SeqCst);
        ws.update_in(win, |ws, _, cx| ws.refresh_linked_projects(true, cx));
        win.condition(&ws, |ws, _| ws.linked_projects.hosts[1].problem.is_some())
            .await;
        win.run_until_parked();
        assert!(win.debug_bounds(row(1, "report")).is_none());
        assert!(win
            .debug_bounds(role_settings_ui::project_value(&format!(
                "linked-schedules-status-{}",
                ids[1]
            )))
            .is_some());
        select(win, "projects:all");
        assert!(win.debug_bounds(row(0, "daily")).is_some());
    }

    #[gpui::test]
    async fn linked_projects_pair_restore_group_and_revoke(cx: &mut gpui::TestAppContext) {
        launch::init(cx);
        let mut next_launch = cx.clone();
        let host = HostFixture::new("Development", "alt+8");
        let state_dir = tempfile::tempdir().unwrap();
        let path = state_dir.path().join("state.json");
        let (ws, win) = launch::launch(cx, &path);
        let token = "c".repeat(64);
        ws.update_in(win, |ws, w, cx| {
            ws.open_settings(&OpenSettings, w, cx);
            ws.set_settings_section(SettingsSection::Environments, cx);
            ws.open_project_link(Some(&host.host), w, cx);
            ws.linked_projects
                .form
                .as_ref()
                .unwrap()
                .token
                .update(cx, |i, cx| i.set_value(token.clone(), w, cx));
        });
        let click = |win: &mut gpui::VisualTestContext, id: &'static str| {
            win.run_until_parked();
            let point = win
                .debug_bounds(id)
                .unwrap_or_else(|| panic!("missing {id}"))
                .center();
            win.simulate_click(point, gpui::Modifiers::default());
            win.run_until_parked();
        };
        click(win, "connect-project-host");
        let (id, vault) = ws.read_with(win, |ws, _| {
            let state = &ws.linked_projects;
            assert!(state.error.is_none(), "{:?}", state.error);
            assert_eq!(state.hosts[0].data.projects[0].path, "/srv/editor");
            assert!(ws
                .grouped_projects()
                .iter()
                .any(|g| g.label == "Build editor" && g.members[0].has_custom_name));
            (state.hosts[0].credential, state.vault.clone())
        });
        launch::saved(&ws, win).await;
        let disk = std::fs::read_to_string(&path).unwrap();
        assert!(
            disk.contains("project_links")
                && !disk.contains(&token)
                && !disk.contains("/srv/editor")
        );
        launch::quit(&ws, win);
        let (ws, win) = launch::launch(&mut next_launch, &path);
        win.simulate_resize(gpui::size(px(820.), px(640.)));
        ws.update_in(win, |ws, w, cx| {
            ws.linked_projects.vault = vault.clone();
            assert!(ws.linked_projects.hosts[0].data.projects.is_empty());
            ws.linked_projects.update_tunnels(&RemoteHostSnapshot {
                hosts: vec![host.host.clone()],
                error: None,
            });
            ws.open_project_picker(&OpenProjectPicker, w, cx);
        });
        win.condition(&ws, |ws, _| ws.linked_projects.checkouts().count() == 1)
            .await;
        let scope = format!("linked:{id}:/srv/editor");
        click(
            win,
            role_settings_ui::project_value(&format!(
                "desktop-project-recent-{}",
                element_key(&scope)
            )),
        );
        ws.read_with(win, |ws, _| {
            assert!(ws.settings_visible() && ws.settings_section == SettingsSection::Projects);
            assert!(ws
                .active_project
                .as_ref()
                .is_none_or(|p| p.path != "/srv/editor"));
        });
        assert!(win.debug_bounds("linked-project-view").is_some());
        assert!(win.debug_bounds("settings-project-action-add").is_none());
        assert!(win
            .debug_bounds("desktop-settings-project-folder")
            .is_none());

        // Scroll the rendered row into reach, as in a small Settings window.
        let reveal = |win: &mut gpui::VisualTestContext, ws: &Entity<Workspace>, row: &str| {
            ws.update_in(win, |ws, _, cx| {
                ws.reveal_settings_row(row.to_owned().into(), cx);
                cx.notify();
            });
            win.run_until_parked();
        };
        let row = format!("linked-checkout-{}", element_key(&scope));
        reveal(win, &ws, &row);
        click(win, "linked-checkout-rule-0");
        click(win, "linked-checkout-rule-0-option-separate");
        ws.read_with(win, |ws, _| {
            assert!(ws.grouped_projects().iter().any(|group| group.key == scope));
            assert_eq!(ws.linked_projects.scope.as_ref().unwrap().key(), scope);
        });
        reveal(win, &ws, "settings-remove-linked-checkout-row");
        click(win, "settings-remove-linked-checkout");
        assert!(!win.has_pending_prompt());
        reveal(win, &ws, "linked-project-inventory");
        click(win, "show-all-project-environments");
        assert!(win.debug_bounds("linked-project-view").is_none());
        assert!(win.debug_bounds("all-project-environments").is_some());
        assert!(win.debug_bounds("settings-project-action-add").is_none());

        reveal(win, &ws, "desktop-settings-row-defaults-scope");
        click(win, "settings-project-defaults-scope");
        click(win, "settings-project-defaults-scope-option-");
        assert!(win.debug_bounds("all-project-environments").is_none());
        assert!(win.debug_bounds("settings-project-action-add").is_some());
        ws.read_with(win, |ws, _| assert!(ws.settings_project_scope.is_empty()));

        // A host selected from the dropdown exposes its own checkout list.
        reveal(win, &ws, "desktop-settings-row-defaults-scope");
        click(win, "settings-project-defaults-scope");
        click(
            win,
            role_settings_ui::project_value(&format!(
                "settings-project-defaults-scope-option-linked:{id}"
            )),
        );
        let open_checkout =
            role_settings_ui::project_value(&format!("view-checkout-{}", element_key(&scope)));
        reveal(win, &ws, &row);
        click(win, open_checkout);
        ws.read_with(win, |ws, _| {
            assert_eq!(ws.linked_projects.scope.as_ref().unwrap().key(), scope)
        });
        host.revoked.store(true, Ordering::SeqCst);
        ws.update_in(win, |ws, _, cx| ws.refresh_linked_projects(true, cx));
        win.condition(&ws, |ws, _| {
            ws.linked_projects.hosts[0]
                .problem
                .as_ref()
                .is_some_and(|s| s.contains("denied project access"))
        })
        .await;
        assert!(win.debug_bounds("linked-project-view").is_some());
        assert!(win.debug_bounds("show-all-project-environments").is_some());
        assert!(win.debug_bounds("linked-checkout-name").is_none());
        assert!(win.debug_bounds("settings-project-action-add").is_none());
        ws.update_in(win, |ws, _, cx| {
            assert!(!ws
                .grouped_projects()
                .iter()
                .any(|g| g.members.iter().any(|m| m.host.is_some())));
            ws.unlink_project_host(id, cx);
        });
        win.run_until_parked();
        launch::saved(&ws, win).await;
        let disk = std::fs::read_to_string(&path).unwrap();
        assert!(!disk.contains("project_links") && !disk.contains(&scope));
        assert!(vault
            .read(id, &Destination::from_profile(&host.host.profile))
            .is_err());
    }
}

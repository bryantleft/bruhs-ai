//! Projects navigation follows a physical checkout even when its grouping rule changes.
use super::*;

pub(super) enum ProjectView {
    All,
    OnHost {
        host: uuid::Uuid,
        path: Option<String>,
    },
}

const ALL_ENVIRONMENTS: &str = "projects:all";

impl ProjectView {
    pub(super) fn key(&self) -> String {
        match self {
            Self::All => ALL_ENVIRONMENTS.to_owned(),
            Self::OnHost {
                host,
                path: Some(path),
            } => format!("linked:{host}:{path}"),
            Self::OnHost { host, path: None } => format!("linked:{host}"),
        }
    }
}

impl Workspace {
    pub(super) fn show_linked_project_scope(
        &mut self,
        host: uuid::Uuid,
        path: Option<String>,
        cx: &mut Context<'_, Self>,
    ) {
        self.open_project_view(ProjectView::OnHost { host, path }, cx);
    }

    fn open_project_view(&mut self, view: ProjectView, cx: &mut Context<'_, Self>) {
        self.linked_projects.scope = Some(view);
        self.linked_projects.rule_scope = None;
        self.action_editor.open = false;
        self.action_editor.art_open = false;
        self.set_settings_section(SettingsSection::Projects, cx);
    }

    pub(in crate::views::workspace) fn select_linked_project_scope(
        &mut self,
        value: &str,
        cx: &mut Context<'_, Self>,
    ) -> bool {
        if value == ALL_ENVIRONMENTS {
            self.select_inventory_view(ProjectView::All, cx);
            return true;
        }
        let Some(key) = value.strip_prefix("linked:") else {
            self.linked_projects.scope = None;
            return false;
        };
        let (id, path) = key
            .split_once(':')
            .map_or((key, None), |(id, path)| (id, Some(path)));
        if let Ok(host) = uuid::Uuid::parse_str(id) {
            self.select_inventory_view(
                ProjectView::OnHost {
                    host,
                    path: path.map(str::to_owned),
                },
                cx,
            );
        }
        true
    }

    fn select_inventory_view(&mut self, view: ProjectView, cx: &mut Context<'_, Self>) {
        if self.settings_section == SettingsSection::ScheduledTasks {
            self.linked_projects.scope = Some(view);
            self.set_settings_open_dropdown(None);
            cx.notify();
        } else {
            self.open_project_view(view, cx);
        }
    }

    pub(in crate::views::workspace) fn projects_scope_row(
        &self,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        let links = &self.linked_projects;
        if links.hosts.is_empty() && links.scope.is_none() {
            return self.defaults_scope_row(cx);
        }
        let id = "settings-project-defaults-scope";
        let mut options = self.settings_project_options(cx);
        options.insert(
            0,
            DropdownOption {
                value: "",
                label: "This environment".into(),
                description: None,
            },
        );
        let mut groups = vec![
            DropdownGroup {
                label: None,
                mark: None,
                options: vec![DropdownOption {
                    value: ALL_ENVIRONMENTS,
                    label: "All environments".into(),
                    description: None,
                }],
            },
            DropdownGroup {
                label: Some("This machine"),
                mark: None,
                options,
            },
        ];
        for host in &links.hosts {
            let name = host.name();
            let mut options = Vec::new();
            let mut add = |path: Option<String>, label: String| {
                let scope = ProjectView::OnHost {
                    host: host.credential,
                    path: path.clone(),
                };
                let value = role_settings_ui::project_value(&scope.key());
                options.push(DropdownOption {
                    value,
                    label,
                    description: path,
                });
            };
            add(None, format!("All checkouts on {name}"));
            for checkout in &host.data.projects {
                add(Some(checkout.path.clone()), checkout.name.clone());
            }
            if let Some(Ok(tasks)) = host
                .data
                .schedules
                .as_ref()
                .filter(|_| self.settings_section == SettingsSection::ScheduledTasks)
            {
                let mut paths: BTreeSet<_> =
                    host.data.projects.iter().map(|c| c.path.as_str()).collect();
                for task in tasks {
                    if !task.project_path.is_empty() && paths.insert(&task.project_path) {
                        add(
                            Some(task.project_path.clone()),
                            project_picker_ui::path_display_name(Path::new(&task.project_path)),
                        );
                    }
                }
            }
            groups.push(DropdownGroup {
                label: Some(role_settings_ui::project_value(&name)),
                mark: None,
                options,
            });
        }
        let value = links.scope.as_ref().map(ProjectView::key);
        let (toggle, select) = self.settings_dropdown_handlers(id, cx);
        let mut control = bruhs_ui::dropdown::Dropdown::new(
            id,
            "Project scope",
            value.as_deref().unwrap_or(&self.settings_project_scope),
            groups,
            self.settings_listbox(id),
            toggle,
            select,
        );
        if let Some(ProjectView::OnHost { host, path }) = &links.scope {
            let environment = links.hosts.iter().find(|h| h.credential == *host);
            let label = match path {
                Some(path) => environment
                    .and_then(|h| {
                        h.data
                            .projects
                            .iter()
                            .find(|checkout| &checkout.path == path)
                    })
                    .map_or_else(
                        || project_picker_ui::path_display_name(Path::new(path)),
                        |p| p.name.clone(),
                    ),
                None => environment
                    .map_or_else(|| "Unavailable environment".to_owned(), LinkedHost::name),
            };
            control = control.selection_label(label, None);
        }
        self.settings_row(
            "desktop-settings-row-defaults-scope",
            "Project scope",
            "Choose an environment or checkout.",
            settings_ui::control_column(control.into_any_element()),
        )
    }

    pub(in crate::views::workspace) fn project_inventory_view(
        &self,
        cx: &Context<'_, Self>,
    ) -> Option<AnyElement> {
        let view = self.linked_projects.scope.as_ref()?;
        let ProjectView::OnHost { host: id, path } = view else {
            return Some(
                div()
                    .id("all-project-environments")
                    .debug_selector(|| "all-project-environments".into())
                    .flex()
                    .flex_col()
                    .gap(px(24.))
                    .child(self.settings_card(
                        "Project overview",
                        "",
                        vec![self.projects_scope_row(cx)],
                    ))
                    .children(self.linked_checkout_cards(cx))
                    .into_any_element(),
            );
        };
        let host = self
            .linked_projects
            .hosts
            .iter()
            .find(|h| h.credential == *id);
        let name = host.map_or_else(|| "Linked environment".to_owned(), LinkedHost::name);
        let members: Vec<_> = self
            .grouped_projects()
            .into_iter()
            .flat_map(|g| g.members)
            .filter(|m| m.host == Some(*id) && path.as_ref().is_none_or(|p| p == &m.path))
            .collect();
        let status = match host {
            None => "This environment is no longer linked.".to_owned(),
            Some(h) if h.problem.is_some() => h.problem.clone().unwrap_or_default(),
            Some(_) if members.is_empty() && self.linked_projects.reading => {
                "Loading checkouts…".to_owned()
            }
            Some(_) if members.is_empty() && path.is_some() => {
                "This checkout is no longer available in the selected environment.".to_owned()
            }
            Some(_) if members.is_empty() => "This environment has no saved checkouts.".to_owned(),
            Some(_) => checkout_count(members.len(), "linked"),
        };
        let mut overview = vec![self.projects_scope_row(cx)];
        let refresh_disabled =
            self.linked_projects.reading || self.linked_projects.busy || host.is_none();
        let actions = div()
            .flex()
            .flex_wrap()
            .gap_2()
            .child(on_press_view(
                button("refresh-linked-projects", "Refresh", refresh_disabled),
                cx,
                move |ws, _, cx| {
                    if !refresh_disabled {
                        ws.refresh_linked_projects(true, cx);
                    }
                },
            ))
            .child(on_press_view(
                button(
                    "show-all-project-environments",
                    "Show all environments",
                    false,
                ),
                cx,
                |ws, _, cx| ws.open_project_view(ProjectView::All, cx),
            ));
        overview.push(self.settings_row(
            "linked-project-inventory",
            name,
            status,
            actions.into_any_element(),
        ));
        if let Some(member) = members.first().filter(|_| path.is_some()) {
            overview.push(self.linked_checkout_name_row(member, cx));
        }
        let mut content = div()
            .id("linked-project-view")
            .debug_selector(|| "linked-project-view".into())
            .flex()
            .flex_col()
            .gap(px(24.))
            .child(self.settings_card("Project overview", "", overview));
        let mut rows = Vec::new();
        for (slot, member) in members.into_iter().enumerate() {
            if path.is_some() {
                rows.push(self.checkout_member_row(member, slot, false, false, cx));
            } else {
                let id = *id;
                let path = member.path.clone();
                let open = on_press_view(
                    button(
                        format!("view-checkout-{}", element_key(&member.scope())),
                        "View checkout",
                        false,
                    ),
                    cx,
                    move |ws, _, cx| ws.show_linked_project_scope(id, Some(path.clone()), cx),
                );
                rows.push(self.settings_row(
                    member.row_id(),
                    member.name,
                    member.path,
                    open.into_any_element(),
                ));
            }
        }
        if !rows.is_empty() {
            content = content.child(self.settings_card("Checkouts", "", rows));
            if let Some(path) = path {
                let remove = self.remove_checkout_button(
                    "settings-remove-linked-checkout".into(),
                    project_removal::Placement::Section,
                    project_removal::Target::Checkout(path.clone()),
                    false,
                    cx,
                );
                let row = self.settings_row(
                    "settings-remove-linked-checkout-row",
                    "Remove checkout",
                    "Remove this checkout on its host.",
                    remove,
                );
                content = content.child(self.settings_card("Danger", "", vec![row]));
            }
        }
        Some(content.into_any_element())
    }
}

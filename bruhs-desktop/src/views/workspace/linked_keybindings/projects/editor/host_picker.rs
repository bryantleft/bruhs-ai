use super::*;

impl Workspace {
    pub(in crate::views::workspace) fn scheduled_host_field(&self, cx: &Context<'_, Self>) -> Div {
        use bruhs_ui::dropdown::{DropdownGroup, Listbox, dropdown, saved_option_value};
        let panel = self.scheduled_panel();
        let current = self
            .linked_projects
            .editor
            .as_ref()
            .and_then(|e| e.host.as_ref());
        let field = div().flex().flex_col().gap_1().child(
            div()
                .text_xs()
                .text_color(theme::rambutan_400())
                .child("Runs on"),
        );
        let Some((open, loading)) = panel.read(cx).task_host_menu() else {
            let name = current.map_or_else(|| "This computer".into(), TaskEditorHost::name);
            return field.child(
                div()
                    .id("scheduled-task-fixed-environment")
                    .role(Role::Label)
                    .aria_label(format!("Runs on {name}"))
                    .h(px(bruhs_ui::theme::CONTROL_PX))
                    .px_3()
                    .flex()
                    .items_center()
                    .rounded(px(8.))
                    .border_1()
                    .border_color(theme::longan_700())
                    .bg(theme::longan_900())
                    .font_family(bruhs_ui::theme::FontFace::Code)
                    .text_size(px(bruhs_ui::theme::APP_UI_TEXT_SIZE_PX))
                    .text_color(theme::rambutan_400())
                    .child(name),
            );
        };
        let mut options = vec![DropdownOption {
            value: "local",
            label: "This computer".into(),
            description: Some("Local desktop".into()),
        }];
        for h in &self.linked_projects.hosts {
            if h.data.can_edit_schedules && h.problem.is_none() {
                options.push(DropdownOption {
                    value: saved_option_value(&h.credential.to_string()),
                    label: h.name(),
                    description: match h.tunnel.as_ref().map(|t| &t.destination) {
                        Some(Destination::Ssh {
                            host,
                            ssh_port,
                            remote_port,
                            ..
                        }) => Some(format!("{host}:{ssh_port} · port {remote_port}")),
                        _ => Some(h.endpoint.origin().ascii_serialization()),
                    },
                });
            }
        }
        // Hosts with distinct labels need no secondary line. Profiles can share both a
        // display name and an SSH target, so keep the saved link ID as a final distinction.
        let mut labels = BTreeMap::new();
        let mut addresses = BTreeMap::new();
        for option in &options {
            *labels.entry(option.label.clone()).or_insert(0) += 1;
            *addresses
                .entry((option.label.clone(), option.description.clone()))
                .or_insert(0) += 1;
        }
        for option in &mut options {
            if labels[&option.label] == 1 {
                option.description = None;
            } else if addresses[&(option.label.clone(), option.description.clone())] > 1 {
                option.description = Some(option.value.to_owned());
            }
        }
        let selected = current.map_or_else(|| "local".into(), |h| h.0.credential.to_string());
        let toggle = panel.downgrade();
        let select = cx.weak_entity();
        field
            .child(dropdown(
                "scheduled-task-environment",
                "Runs on",
                &selected,
                vec![DropdownGroup {
                    label: None,
                    mark: None,
                    options,
                }],
                if open {
                    Listbox::Popover
                } else {
                    Listbox::Closed
                },
                Rc::new(move |_, cx| {
                    let _ = toggle.update(cx, |p, cx| p.toggle_task_hosts(cx));
                }),
                Rc::new(move |key, w, cx| {
                    let id = if key == "local" {
                        None
                    } else if let Ok(id) = key.parse() {
                        Some(id)
                    } else {
                        return;
                    };
                    let _ = select.update(cx, |ws, cx| ws.switch_task_host(id, w, cx));
                }),
            ))
            .when(loading, |row| {
                row.child(
                    div()
                        .id("task-host-progress")
                        .role(Role::Status)
                        .text_xs()
                        .child("Loading host projects…"),
                )
            })
    }

    fn switch_task_host(
        &mut self,
        id: Option<uuid::Uuid>,
        w: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let origin = self.scheduled_panel().clone();
        if origin.read(cx).task_host_menu().is_none() {
            return;
        }
        let current = self.linked_projects.editor.as_ref();
        let current_id = current
            .and_then(|e| e.host.as_ref())
            .map(|h| h.0.credential);
        if current_id == id {
            self.linked_projects.opening_editor = None;
            origin.update(cx, |p, cx| p.select_task_host(None, cx));
            return;
        }
        let trigger = current
            .and_then(|e| e.trigger.clone())
            .or_else(|| Some(self.reactive_automations.read(cx).new_task_focus()));
        if let Some(id) = id {
            self.load_linked_editor(id, None, Some(origin), trigger, w, cx);
        } else {
            self.linked_projects.opening_editor = None;
            let local = self.reactive_automations.clone();
            let panel = cx.new(|cx| ReactiveAutomations::switched_task_panel(None, &local, cx));
            let path = self.initial_local_task_project(cx);
            let model = self
                .project_defaults
                .get(&path)
                .and_then(|d| d.model.as_deref())
                .unwrap_or(&self.current_model);
            panel.update(cx, |p, cx| {
                p.receive_task_draft(&origin, &path, model, w, cx)
            });
            let watch = cx.observe(&panel, |_, _, cx| cx.notify());
            self.linked_projects.editor = Some(OpenEditor {
                host: None,
                scope: self.task_editor_scope(),
                panel,
                defaults: None,
                trigger,
                _watch: watch,
            });
            cx.notify();
        }
    }
}

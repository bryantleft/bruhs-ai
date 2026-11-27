//! Manage the local host's SSH profiles through the same service as Workbench.
use super::*;
use crate::{app_state::spawn_tokio, server_client::ServerClient};
use bruhs_models::remote_hosts::{
    RemoteHostCommand, RemoteHostProfile, RemoteHostSnapshot, RemoteHostState,
};
use uuid::Uuid;

const FIELDS: [&str; 6] = [
    "Name",
    "SSH alias or user@host",
    "SSH port",
    "Local port",
    "Remote task port",
    "Connection timeout (seconds)",
];

pub(super) struct Environments {
    snapshot: Option<RemoteHostSnapshot>,
    fields: [Entity<InputState>; 6],
    editing: Option<Uuid>,
    deleting: Option<Uuid>,
    error: Option<String>,
    saving: bool,
    reading: bool,
    revision: u64,
    pub(super) poll: Option<Task<()>>,
}

impl Environments {
    pub(super) fn new(window: &mut Window, cx: &mut Context<'_, Workspace>) -> Self {
        Self {
            snapshot: None,
            fields: FIELDS.map(|label| cx.new(|cx| InputState::new(window, cx).placeholder(label))),
            editing: None,
            deleting: None,
            error: None,
            saving: false,
            reading: false,
            revision: 0,
            poll: None,
        }
    }
    fn profile(&self, cx: &App) -> Result<RemoteHostProfile, String> {
        let [name, host, ssh, local, remote, timeout] = self
            .fields
            .each_ref()
            .map(|f| f.read(cx).value().trim().to_owned());
        let profile = RemoteHostProfile {
            id: self.editing.ok_or("Choose an environment to edit.")?,
            name,
            host,
            ssh_port: ssh
                .parse()
                .map_err(|_| "Enter an SSH port between 1 and 65535.")?,
            local_port: local
                .parse()
                .map_err(|_| "Enter a local port between 1024 and 65535.")?,
            remote_port: remote
                .parse()
                .map_err(|_| "Enter a remote task port between 1 and 65535.")?,
            connect_timeout_seconds: timeout
                .parse()
                .map_err(|_| "Enter a positive timeout in seconds.")?,
        };
        profile.validate().map_err(str::to_owned)?;
        Ok(profile)
    }
}

impl Workspace {
    pub(super) fn open_environments(&mut self, cx: &mut Context<'_, Self>) {
        self.refresh_environments(None, cx);
        self.environments.poll = Some(cx.spawn(async move |this, cx| loop {
            cx.background_executor()
                .timer(std::time::Duration::from_secs(3))
                .await;
            if this
                .update(cx, |this, cx| {
                    if !this.settings_visible()
                        || this.settings_section != SettingsSection::Environments
                    {
                        return false;
                    }
                    if this.environments.error.is_none() {
                        this.refresh_environments(None, cx);
                    }
                    true
                })
                .ok()
                != Some(true)
            {
                break;
            }
        }));
    }

    fn refresh_environments(
        &mut self,
        command: Option<RemoteHostCommand>,
        cx: &mut Context<'_, Self>,
    ) {
        let state = &mut self.environments;
        if state.saving || (command.is_none() && state.reading) {
            return;
        }
        let delete_profile = matches!(command, Some(RemoteHostCommand::Delete { .. }));
        let save_form = matches!(command, Some(RemoteHostCommand::Save { .. }));
        state.saving = command.is_some();
        state.reading = command.is_none();
        state.revision = state.revision.wrapping_add(1);
        let revision = state.revision;
        let request = spawn_tokio(cx, async move {
            ServerClient::workbench_client()
                .remote_hosts(command.as_ref())
                .await
        });
        cx.spawn(async move |this, cx| {
            let result = request.await;
            let _ = this.update(cx, |this, cx| {
                let state = &mut this.environments;
                if state.revision != revision {
                    return;
                }
                state.saving = false;
                state.reading = false;
                match result {
                    Ok(snapshot) => {
                        state.error = snapshot.error.clone();
                        this.linked_keybindings.update_tunnels(&snapshot);
                        this.linked_projects.update_tunnels(&snapshot);
                        state.snapshot = Some(snapshot);
                        if delete_profile {
                            state.deleting = None;
                        }
                        if save_form {
                            state.editing = None;
                        }
                    }
                    Err(error) => {
                        state.error = Some(
                            error
                                .conflict_message()
                                .unwrap_or_else(|| settings_ui::friendly_server_error(&error)),
                        )
                    }
                }
                cx.notify();
            });
        })
        .detach();
        if self.environments.saving {
            cx.notify();
        }
    }

    fn edit_environment(
        &mut self,
        profile: Option<&RemoteHostProfile>,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let state = &mut self.environments;
        state.deleting = None;
        state.error = None;
        state.editing = Some(profile.map_or_else(Uuid::new_v4, |p| p.id));
        let values = profile.map_or_else(
            || {
                let port = (45001..=65535)
                    .find(|port| {
                        !state
                            .snapshot
                            .iter()
                            .flat_map(|s| &s.hosts)
                            .any(|h| h.profile.local_port == *port)
                    })
                    .unwrap_or(45001);
                [
                    String::new(),
                    String::new(),
                    "22".into(),
                    port.to_string(),
                    "45000".into(),
                    "15".into(),
                ]
            },
            |p| {
                [
                    p.name.clone(),
                    p.host.clone(),
                    p.ssh_port.to_string(),
                    p.local_port.to_string(),
                    p.remote_port.to_string(),
                    p.connect_timeout_seconds.to_string(),
                ]
            },
        );
        for (field, value) in state.fields.iter().zip(values) {
            field.update(cx, |f, cx| f.set_value(value, window, cx));
        }
        state.fields[0].update(cx, |f, cx| f.focus(window, cx));
        cx.notify();
    }

    pub(super) fn environment_rows(&self, cx: &Context<'_, Self>) -> Vec<AnyElement> {
        let state = &self.environments;
        let button = |id: SharedString, label: &'static str, disabled: bool| {
            web_button(
                id,
                WebButtonVariant::Outline,
                WebButtonSize::Small,
                ButtonState::disabled_if(disabled),
            )
            .aria_label(label)
            .child(label)
        };
        let mut rows = vec![self.settings_row(
            "settings-environment-local",
            "Local environment",
            format!(
                "This machine, {}, {}",
                std::env::consts::OS,
                std::env::consts::ARCH
            ),
            on_press_view(
                button("environment-sharing".into(), "Sharing & access", false),
                cx,
                |this, _, cx| this.set_settings_section(SettingsSection::Access, cx),
            )
            .into_any_element(),
        )];
        if let Some(error) = &state.error {
            rows.push(
                self.settings_row(
                    "settings-environment-error",
                    "Connection status",
                    error.clone(),
                    on_press_view(
                        button("environment-retry".into(), "Retry", state.saving),
                        cx,
                        |this, _, cx| this.refresh_environments(None, cx),
                    )
                    .into_any_element(),
                ),
            );
        }
        for h in state.snapshot.iter().flat_map(|s| &s.hosts) {
            let p = &h.profile;
            let id = p.id;
            let status = match h.state {
                RemoteHostState::Disconnected => "Disconnected",
                RemoteHostState::Connecting => "Connecting",
                RemoteHostState::Connected => "Connected",
                RemoteHostState::Disconnecting => "Disconnecting",
                RemoteHostState::Interrupted => "Interrupted",
                RemoteHostState::Failed => "Failed",
            };
            let active = h.state.active();
            let name = p.name.clone();
            let mut controls = div().flex().flex_wrap().gap_2();
            if h.state == RemoteHostState::Connected {
                #[cfg(unix)]
                if h.forwarding_socket.is_some() {
                    let project_host = h.clone();
                    controls = controls.child(on_press_view(
                        button(
                            format!("environment-link-projects-{id}").into(),
                            "Link projects",
                            false,
                        ),
                        cx,
                        move |this, w, cx| this.open_project_link(Some(&project_host), w, cx),
                    ));
                    let host = h.clone();
                    controls = controls.child(on_press_view(
                        button(
                            format!("environment-link-shortcuts-{id}").into(),
                            "Link shortcuts",
                            self.linked_keybindings.linking_busy(),
                        )
                        .debug_selector(move || format!("environment-link-shortcuts-{id}")),
                        cx,
                        move |this, w, cx| this.open_shortcut_link(Some(&host), w, cx),
                    ));
                }
                let url = format!("http://127.0.0.1:{}/", p.local_port);
                controls = controls.child(on_press_view(
                    button(format!("environment-open-{id}").into(), "Open", false)
                        .aria_label(format!("Open {name}")),
                    cx,
                    move |_, _, cx| cx.open_url(&url),
                ));
            }
            let action_label = if active { "Disconnect" } else { "Connect" };
            controls = controls.child(on_press_view(
                button(
                    format!("environment-connect-{id}").into(),
                    action_label,
                    state.saving || h.state == RemoteHostState::Disconnecting,
                )
                .aria_label(format!("{action_label} {name}")),
                cx,
                move |this, _, cx| {
                    let action = if active {
                        RemoteHostCommand::Disconnect { id }
                    } else {
                        RemoteHostCommand::Connect { id }
                    };
                    this.refresh_environments(Some(action), cx);
                },
            ));
            if !active {
                let profile = p.clone();
                controls = controls.child(on_press_view(
                    button(
                        format!("environment-edit-{id}").into(),
                        "Edit",
                        state.saving,
                    )
                    .aria_label(format!("Edit {name}")),
                    cx,
                    move |this, w, cx| this.edit_environment(Some(&profile), w, cx),
                ));
                if state.deleting == Some(id) {
                    controls = controls
                        .child(on_press_view(
                            button(
                                format!("environment-confirm-{id}").into(),
                                "Confirm removal",
                                state.saving,
                            ),
                            cx,
                            move |this, _, cx| {
                                this.refresh_environments(
                                    Some(RemoteHostCommand::Delete { id }),
                                    cx,
                                )
                            },
                        ))
                        .child(on_press_view(
                            button(
                                format!("environment-keep-{id}").into(),
                                "Cancel",
                                state.saving,
                            ),
                            cx,
                            |this, _, cx| {
                                this.environments.deleting = None;
                                cx.notify();
                            },
                        ));
                } else {
                    controls = controls.child(on_press_view(
                        button(
                            format!("environment-remove-{id}").into(),
                            "Remove",
                            state.saving,
                        )
                        .aria_label(format!("Remove {name}")),
                        cx,
                        move |this, _, cx| {
                            this.environments.deleting = Some(id);
                            cx.notify();
                        },
                    ));
                }
            }
            let mut detail = format!("{}, {}, local port {}", status, p.host, p.local_port);
            if let Some(message) = &h.error {
                detail.push_str(", ");
                detail.push_str(message);
            }
            rows.push(self.settings_row(
                format!("settings-environment-{id}"),
                name,
                detail,
                controls.into_any_element(),
            ));
        }
        if state.editing.is_some() {
            let mut form = div().flex().flex_col().gap_2();
            for (field, label) in state.fields.iter().zip(FIELDS) {
                form = form.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(label)
                        .child(Input::new(field).aria_label(label)),
                );
            }
            rows.push(
                self.settings_row(
                    "settings-environment-editor",
                    "SSH connection",
                    "Use an existing OpenSSH alias or a host you already trust.",
                    form.child(
                        div()
                            .flex()
                            .gap_2()
                            .child(on_press_view(
                                button("environment-save".into(), "Save", state.saving),
                                cx,
                                |this, _, cx| match this.environments.profile(cx) {
                                    Ok(profile) => this.refresh_environments(
                                        Some(RemoteHostCommand::Save { profile }),
                                        cx,
                                    ),
                                    Err(error) => {
                                        this.environments.error = Some(error);
                                        cx.notify();
                                    }
                                },
                            ))
                            .child(on_press_view(
                                button("environment-cancel".into(), "Cancel", state.saving),
                                cx,
                                |this, _, cx| {
                                    this.environments.editing = None;
                                    this.environments.error = None;
                                    cx.notify();
                                },
                            )),
                    )
                    .into_any_element(),
                ),
            );
        } else {
            rows.push(
                self.settings_row(
                    "settings-remote-environments",
                    "Remote environments",
                    "Connect to a remote bruhs host over SSH.",
                    on_press_view(
                        button("environment-add".into(), "Add environment", state.saving),
                        cx,
                        |this, w, cx| this.edit_environment(None, w, cx),
                    )
                    .into_any_element(),
                ),
            );
        }
        rows.extend(self.project_environment_rows(cx));
        rows.extend(self.shortcut_environment_rows(cx));
        rows
    }
}

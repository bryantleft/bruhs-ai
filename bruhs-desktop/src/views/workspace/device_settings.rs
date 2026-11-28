//! Local tool permissions belong to the selected thread and the running host.
use super::*;
use crate::{
    app_state::{spawn_tokio, DesktopState},
    server_client::ServerClient,
};
use bruhs_models::local_tools::{
    DeviceAction, DeviceCommand, DeviceInventory, DevicePlatform, GrantRequest, LocalToolsSnapshot,
    ToolGrant, ToolScope,
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(super) enum DeviceAccessPolicy {
    #[default]
    Ask,
    ThreadOnly,
    Persistent,
    Off,
}

impl DeviceAccessPolicy {
    pub(super) const CHOICES: &[(&str, &str)] = &[
        ("ask", "Ask per session"),
        ("thread-only", "Active thread only"),
        ("persistent", "Keep access across restarts"),
        ("off", "Off"),
    ];

    pub(super) fn key(self) -> &'static str {
        match self {
            Self::Ask => "ask",
            Self::ThreadOnly => "thread-only",
            Self::Persistent => "persistent",
            Self::Off => "off",
        }
    }

    pub(super) fn to_wire(self) -> bruhs_models::environment_parity::DeviceAccessPolicy {
        match self {
            Self::Ask => bruhs_models::environment_parity::DeviceAccessPolicy::Ask,
            Self::ThreadOnly => bruhs_models::environment_parity::DeviceAccessPolicy::ThreadOnly,
            Self::Persistent => bruhs_models::environment_parity::DeviceAccessPolicy::Persistent,
            Self::Off => bruhs_models::environment_parity::DeviceAccessPolicy::Off,
        }
    }

    pub(super) fn from_key(key: &str) -> Option<Self> {
        match key {
            "ask" => Some(Self::Ask),
            "thread-only" => Some(Self::ThreadOnly),
            "persistent" => Some(Self::Persistent),
            "off" => Some(Self::Off),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct SimlockHostConfig {
    pub id: uuid::Uuid,
    pub label: String,
    pub origin: String,
    pub max_devices: u8,
    pub lease_minutes: u16,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub(super) struct DevicePreferences {
    pub policy: DeviceAccessPolicy,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub simlock_hosts: Vec<SimlockHostConfig>,
}

#[derive(Default)]
pub(super) struct Access {
    snapshot: Option<LocalToolsSnapshot>,
    inventory: Option<(String, DeviceInventory)>,
    busy: bool,
    error: Option<String>,
    adding_simlock: bool,
    simlock_error: Option<String>,
    simlock_probes: std::collections::HashMap<uuid::Uuid, String>,
    simlock_leases: Vec<bruhs_models::environment_parity::SimlockLease>,
}

impl Workspace {
    fn thread_tool_grant(&self, scope: ToolScope) -> Option<&ToolGrant> {
        let id = &self.active_thread()?.id;
        self.local_tools
            .snapshot
            .as_ref()?
            .grants
            .iter()
            .find(|g| g.principal == *id && g.scope == scope)
    }

    pub(super) fn refresh_tool_access(&mut self, cx: &mut Context<'_, Self>) {
        self.update_tool_access(None, cx);
    }

    fn update_tool_access(
        &mut self,
        change: Option<(GrantRequest, Option<String>)>,
        cx: &mut Context<'_, Self>,
    ) {
        if self.local_tools.busy || cx.try_global::<DesktopState>().is_none() {
            return;
        }
        self.local_tools.busy = true;
        self.local_tools.error = None;
        let task = spawn_tokio(cx, async move {
            let client = ServerClient::workbench_client();
            if let Some((request, revoke)) = change {
                client.set_tool_grant(&request, revoke.as_deref()).await?;
            }
            client.tool_snapshot().await
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                let state = &mut this.local_tools;
                state.busy = false;
                match result {
                    Ok(snapshot) => {
                        if state
                            .inventory
                            .as_ref()
                            .is_some_and(|(id, _)| !snapshot.grants.iter().any(|g| &g.id == id))
                        {
                            state.inventory = None;
                        }
                        state.snapshot = Some(snapshot);
                    }
                    Err(error) => {
                        state.error = Some(settings_ui::friendly_server_error(&error));
                        state.snapshot = None;
                        state.inventory = None;
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn scan_devices(&mut self, command: Option<DeviceCommand>, cx: &mut Context<'_, Self>) {
        if self.local_tools.busy {
            return;
        }
        let Some(grant) = self
            .thread_tool_grant(ToolScope::Devices)
            .map(|g| g.id.clone())
        else {
            return;
        };
        if command.as_ref().is_some_and(|c| c.grant_id != grant) {
            return;
        }
        self.local_tools.busy = true;
        self.local_tools.error = None;
        let request_grant = grant.clone();
        let task = spawn_tokio(cx, async move {
            let client = ServerClient::workbench_client();
            if let Some(cmd) = command {
                client.device_power(&cmd).await?;
            }
            client.device_inventory(&request_grant).await
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                this.local_tools.busy = false;
                if this
                    .thread_tool_grant(ToolScope::Devices)
                    .is_some_and(|g| g.id == grant)
                {
                    match result {
                        Ok(devices) => this.local_tools.inventory = Some((grant, devices)),
                        Err(e) => {
                            this.local_tools.inventory = None;
                            this.local_tools.error = Some(settings_ui::friendly_server_error(&e));
                        }
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub(super) fn tool_access_rows(
        &self,
        scope: ToolScope,
        cx: &Context<'_, Self>,
    ) -> Vec<AnyElement> {
        let browser = scope == ToolScope::Browser;
        let label = if browser {
            "Agent browser access"
        } else {
            "Agents can use devices"
        };
        let id = if browser {
            "settings-agent-browser-access"
        } else {
            "settings-agent-device-access"
        };
        let current = self.active_thread();
        let allowed = self.thread_tool_grant(scope).is_some();
        let description = current.map_or_else(
            || "Open a thread to manage its access.".into(),
            |t| {
                format!(
                    "{}. Lasts until revoked or host restart.",
                    t.chat.read(cx).thread_title
                )
            },
        );
        let state = &self.local_tools;
        let switch = settings_ui::switch(
            id,
            allowed,
            state.busy || state.snapshot.is_none() || current.is_none(),
            label,
            cx,
            move |this, _, cx| {
                let Some(t) = this.active_thread() else {
                    return;
                };
                let request = GrantRequest {
                    principal: t.id.clone(),
                    scope,
                };
                let revoke = this.thread_tool_grant(scope).map(|g| g.id.clone());
                this.update_tool_access(Some((request, revoke)), cx);
            },
        );
        let mut rows = vec![self.settings_row(id, label, description, switch)];
        if let Some(e) = &state.error {
            let retry = web_button(
                "tool-settings-retry",
                WebButtonVariant::Outline,
                WebButtonSize::Small,
                ButtonState::disabled_if(state.busy),
            )
            .aria_label("Refresh tool permissions")
            .child("Refresh");
            rows.push(
                self.settings_row(
                    "tool-settings-error",
                    "Local tools",
                    e.clone(),
                    on_press_view(retry, cx, |this, _, cx| this.refresh_tool_access(cx))
                        .into_any_element(),
                ),
            );
        }
        rows
    }

    pub(super) fn select_device_option(
        &mut self,
        id: &str,
        value: &str,
        cx: &mut Context<'_, Self>,
    ) -> bool {
        if id != "settings-device-policy" {
            return false;
        }
        let Some(policy) = DeviceAccessPolicy::from_key(value) else {
            return true;
        };
        self.device_preferences.policy = policy;
        if cx.try_global::<DesktopState>().is_some() {
            let wire = policy.to_wire();
            let task = spawn_tokio(cx, async move {
                ServerClient::workbench_client()
                    .simlock_set_policy(wire)
                    .await
            });
            cx.spawn(async move |this, cx| {
                if let Ok(snap) = task.await {
                    let _ = this.update(cx, |this, cx| {
                        this.local_tools.simlock_leases = snap.active_leases;
                        cx.notify();
                    });
                }
            })
            .detach();
        }
        self.persist_and_notify(cx);
        true
    }

    fn probe_simlock_host(&mut self, host_id: uuid::Uuid, cx: &mut Context<'_, Self>) {
        if cx.try_global::<DesktopState>().is_none() {
            return;
        }
        self.local_tools
            .simlock_probes
            .insert(host_id, "Testing connection…".into());
        cx.notify();
        let task = spawn_tokio(cx, async move {
            ServerClient::workbench_client()
                .simlock_probe_host(host_id)
                .await
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                let message = match result {
                    Ok(probe) => probe.detail,
                    Err(err) => err.to_string(),
                };
                this.local_tools.simlock_probes.insert(host_id, message);
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn device_rows(&self, cx: &Context<'_, Self>) -> Vec<AnyElement> {
        let mut rows = self.tool_access_rows(ToolScope::Devices, cx);
        rows.push(self.settings_row(
            "settings-row-device-policy",
            "Use devices",
            "Control how agent sessions request and retain access to simulators and emulators across restarts.",
            settings_ui::control_column(self.choice_control(
                "settings-device-policy",
                "Use devices",
                self.device_preferences.policy.key(),
                DeviceAccessPolicy::CHOICES,
                cx,
            )),
        ));
        let mut source_labels = vec![format!(
            "This computer ({}, {})",
            std::env::consts::OS,
            std::env::consts::ARCH
        )];
        for (_id, name, connected) in self.linked_projects.host_summaries() {
            let status = if connected { "connected" } else { "offline" };
            source_labels.push(format!("{name} ({status})"));
        }
        let mut source_controls = div().flex().flex_wrap().items_center().gap_2();
        if self.labs_flags.simlock_hosts {
            let add = web_button(
                "device-add-simlock",
                WebButtonVariant::Outline,
                WebButtonSize::Small,
                ButtonState::Enabled,
            )
            .debug_selector(|| "device-add-simlock".into())
            .aria_label("Add Simlock host")
            .child(if self.local_tools.adding_simlock {
                "Close"
            } else {
                "Add Simlock host"
            });
            source_controls = source_controls.child(on_press_view(add, cx, |this, _, cx| {
                this.local_tools.adding_simlock = !this.local_tools.adding_simlock;
                this.local_tools.simlock_error = None;
                cx.notify();
            }));
        }
        let source_badges = div().flex().flex_wrap().items_center().gap_1p5().children(
            source_labels.into_iter().map(|label| {
                div()
                    .px_2()
                    .py_0p5()
                    .rounded(px(999.0))
                    .bg(theme::longan_800())
                    .border_1()
                    .border_color(theme::longan_700().opacity(0.6))
                    .text_size(px(11.0))
                    .text_color(theme::rambutan_300())
                    .child(label)
            }),
        );
        rows.push(
            div()
                .id("settings-device-sources")
                .px_4()
                .py_3()
                .flex()
                .items_center()
                .justify_between()
                .gap_3()
                .child(
                    div()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap_1p5()
                        .child(
                            div()
                                .text_size(px(13.0))
                                .text_color(theme::rambutan_100())
                                .child("Device sources"),
                        )
                        .child(source_badges),
                )
                .child(source_controls)
                .into_any_element(),
        );
        if self.labs_flags.simlock_hosts {
            for host in &self.device_preferences.simlock_hosts {
                let host_id = host.id;
                let probe_note = self.local_tools.simlock_probes.get(&host_id).cloned();
                let test = web_button(
                    SharedString::from(format!("device-probe-simlock-{host_id}")),
                    WebButtonVariant::Outline,
                    WebButtonSize::Small,
                    ButtonState::Enabled,
                )
                .aria_label(format!("Test {}", host.label))
                .child("Test");
                let remove = web_button(
                    SharedString::from(format!("device-remove-simlock-{host_id}")),
                    WebButtonVariant::Ghost,
                    WebButtonSize::Small,
                    ButtonState::Enabled,
                )
                .aria_label(format!("Remove {}", host.label))
                .child("Remove");
                let controls = div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(on_press_view(test, cx, move |this, _, cx| {
                        this.probe_simlock_host(host_id, cx);
                    }))
                    .child(on_press_view(remove, cx, move |this, _, cx| {
                        this.device_preferences
                            .simlock_hosts
                            .retain(|entry| entry.id != host_id);
                        this.local_tools.simlock_probes.remove(&host_id);
                        if cx.try_global::<DesktopState>().is_some() {
                            let task = spawn_tokio(cx, async move {
                                ServerClient::workbench_client()
                                    .simlock_remove_host(host_id)
                                    .await
                            });
                            cx.spawn(async move |this, cx| {
                                if let Ok(snap) = task.await {
                                    let _ = this.update(cx, |this, cx| {
                                        this.local_tools.simlock_leases = snap.active_leases;
                                        cx.notify();
                                    });
                                }
                            })
                            .detach();
                        }
                        this.persist_and_notify(cx);
                    }));
                let host_badges = div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_1p5()
                    .child(
                        div()
                            .px_2()
                            .py_0p5()
                            .rounded(px(999.0))
                            .bg(theme::longan_800())
                            .border_1()
                            .border_color(theme::longan_700().opacity(0.6))
                            .text_size(px(11.0))
                            .text_color(theme::rambutan_300())
                            .child(format!("Max {}", host.max_devices)),
                    )
                    .child(
                        div()
                            .px_2()
                            .py_0p5()
                            .rounded(px(999.0))
                            .bg(theme::longan_800())
                            .border_1()
                            .border_color(theme::longan_700().opacity(0.6))
                            .text_size(px(11.0))
                            .text_color(theme::rambutan_300())
                            .child(format!("{}m lease", host.lease_minutes)),
                    )
                    .when_some(probe_note, |el, note| {
                        el.child(
                            div()
                                .px_2()
                                .py_0p5()
                                .rounded(px(999.0))
                                .bg(theme::longan_800())
                                .border_1()
                                .border_color(theme::longan_700().opacity(0.6))
                                .text_size(px(11.0))
                                .text_color(theme::rambutan_200())
                                .child(note),
                        )
                    });
                rows.push(
                    div()
                        .id(SharedString::from(format!(
                            "settings-simlock-host-{host_id}"
                        )))
                        .px_4()
                        .py_3()
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap_3()
                        .child(
                            div()
                                .min_w_0()
                                .flex()
                                .flex_col()
                                .gap_1()
                                .child(
                                    div()
                                        .flex()
                                        .flex_wrap()
                                        .items_center()
                                        .gap_1p5()
                                        .child(
                                            div()
                                                .text_size(px(13.0))
                                                .text_color(theme::rambutan_100())
                                                .child(host.label.clone()),
                                        )
                                        .child(host_badges),
                                )
                                .child(
                                    div()
                                        .text_size(px(12.0))
                                        .text_color(theme::rambutan_500())
                                        .child(host.origin.clone()),
                                ),
                        )
                        .child(controls)
                        .into_any_element(),
                );
            }
            for lease in &self.local_tools.simlock_leases {
                let lease_id = lease.lease_id;
                let release = web_button(
                    SharedString::from(format!("device-release-lease-{lease_id}")),
                    WebButtonVariant::Ghost,
                    WebButtonSize::Small,
                    ButtonState::Enabled,
                )
                .aria_label(format!("Release {}", lease.device_id))
                .child("Release");
                rows.push(
                    div()
                        .id(SharedString::from(format!(
                            "settings-simlock-lease-{lease_id}"
                        )))
                        .px_4()
                        .py_3()
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap_3()
                        .child(
                            div()
                                .min_w_0()
                                .flex()
                                .flex_wrap()
                                .items_center()
                                .gap_1p5()
                                .child(
                                    div()
                                        .px_2()
                                        .py_0p5()
                                        .rounded(px(999.0))
                                        .bg(theme::blueberry_500().opacity(0.18))
                                        .text_size(px(11.0))
                                        .text_color(theme::blueberry_300())
                                        .child("Leased"),
                                )
                                .child(
                                    div()
                                        .text_size(px(13.0))
                                        .text_color(theme::rambutan_100())
                                        .child(lease.device_id.clone()),
                                )
                                .child(
                                    div()
                                        .px_2()
                                        .py_0p5()
                                        .rounded(px(999.0))
                                        .bg(theme::longan_800())
                                        .border_1()
                                        .border_color(theme::longan_700().opacity(0.6))
                                        .text_size(px(11.0))
                                        .text_color(theme::rambutan_300())
                                        .child(lease.host_label.clone()),
                                )
                                .child(
                                    div()
                                        .px_2()
                                        .py_0p5()
                                        .rounded(px(999.0))
                                        .bg(theme::longan_800())
                                        .border_1()
                                        .border_color(theme::longan_700().opacity(0.6))
                                        .text_size(px(11.0))
                                        .text_color(theme::rambutan_400())
                                        .child(format!("Thread {}", lease.thread_id)),
                                ),
                        )
                        .child(on_press_view(release, cx, move |this, _, cx| {
                            this.local_tools
                                .simlock_leases
                                .retain(|entry| entry.lease_id != lease_id);
                            if cx.try_global::<DesktopState>().is_some() {
                                let task = spawn_tokio(cx, async move {
                                    ServerClient::workbench_client()
                                        .simlock_release_lease(lease_id)
                                        .await
                                });
                                cx.spawn(async move |_, _| {
                                    let _ = task.await;
                                })
                                .detach();
                            }
                            cx.notify();
                        }))
                        .into_any_element(),
                );
            }
            if self.local_tools.adding_simlock {
                let save = web_button(
                    "device-save-simlock",
                    WebButtonVariant::Outline,
                    WebButtonSize::Small,
                    ButtonState::Enabled,
                )
                .debug_selector(|| "device-save-simlock".into())
                .aria_label("Save Simlock host")
                .child("Save host");
                let form = div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(Input::new(&self.simlock_label_input).aria_label("Simlock host label"))
                    .child(
                        Input::new(&self.simlock_origin_input)
                            .aria_label("Simlock gateway origin"),
                    )
                    .when_some(self.local_tools.simlock_error.clone(), |el, err| {
                        el.child(div().text_xs().text_color(theme::lychee_300()).child(err))
                    })
                    .child(on_press_view(save, cx, |this, window, cx| {
                        let label = this.simlock_label_input.read(cx).value().trim().to_owned();
                        let origin = this.simlock_origin_input.read(cx).value().trim().to_owned();
                        if label.is_empty() {
                            this.local_tools.simlock_error = Some("Give the host a label.".into());
                            cx.notify();
                            return;
                        }
                        let valid_origin = reqwest::Url::parse(&origin)
                            .is_ok_and(|u| matches!(u.scheme(), "http" | "https") && u.host().is_some());
                        if !valid_origin {
                            this.local_tools.simlock_error = Some(
                                "Enter the gateway's origin, such as https://gateway.example.ts.net:4700."
                                    .into(),
                            );
                            cx.notify();
                            return;
                        }
                        let spec = SimlockHostConfig {
                            id: uuid::Uuid::new_v4(),
                            label,
                            origin,
                            max_devices: 2,
                            lease_minutes: 30,
                        };
                        if cx.try_global::<DesktopState>().is_some() {
                            let wire = bruhs_models::environment_parity::SimlockHostSpec {
                                id: spec.id,
                                label: spec.label.clone(),
                                origin: spec.origin.clone(),
                                max_devices: spec.max_devices,
                                lease_minutes: spec.lease_minutes,
                            };
                            let task = spawn_tokio(cx, async move {
                                ServerClient::workbench_client()
                                    .simlock_upsert_host(&wire)
                                    .await
                            });
                            cx.spawn(async move |_, _| {
                                let _ = task.await;
                            })
                            .detach();
                        }
                        this.device_preferences.simlock_hosts.push(spec);
                        this.simlock_label_input
                            .update(cx, |i, cx| i.set_value("", window, cx));
                        this.simlock_origin_input
                            .update(cx, |i, cx| i.set_value("", window, cx));
                        this.local_tools.adding_simlock = false;
                        this.local_tools.simlock_error = None;
                        this.persist_and_notify(cx);
                    }));
                rows.push(self.settings_row(
                    "settings-simlock-editor",
                    "Simlock pool",
                    "Lease simulators and emulators from a self-hosted Simlock gateway.",
                    form.into_any_element(),
                ));
            }
        }
        let grant = self.thread_tool_grant(ToolScope::Devices).map(|g| &g.id);
        let state = &self.local_tools;
        let scan = web_button(
            "device-settings-scan",
            WebButtonVariant::Outline,
            WebButtonSize::Small,
            ButtonState::disabled_if(state.busy || grant.is_none()),
        )
        .aria_label("Discover local devices")
        .child("Discover");
        rows.push(self.settings_row(
            "settings-local-simulators",
            "Local simulators and devices",
            "Discover local Apple simulators and Android devices.",
            on_press_view(scan, cx, |this, _, cx| this.scan_devices(None, cx)).into_any_element(),
        ));
        let Some((id, inventory)) = state.inventory.as_ref().filter(|(id, _)| Some(id) == grant)
        else {
            return rows;
        };
        if inventory.devices.is_empty() {
            rows.push(div().p_3().child("No devices found.").into_any_element());
        }
        for d in &inventory.devices {
            let power = match (d.platform, d.state.as_str()) {
                (DevicePlatform::Ios, "Shutdown") => Some(("Start", DeviceAction::Boot)),
                (DevicePlatform::Ios, "Booted") => Some(("Stop", DeviceAction::Shutdown)),
                (DevicePlatform::Android, "device") if d.id.starts_with("emulator-") => {
                    Some(("Stop", DeviceAction::Shutdown))
                }
                _ => None,
            };
            let control = if let Some((action, command)) = power {
                let command = DeviceCommand {
                    grant_id: id.clone(),
                    device_id: d.id.clone(),
                    platform: d.platform,
                    command,
                };
                let btn = web_button(
                    SharedString::from(format!("device-power-{}", d.id)),
                    WebButtonVariant::Outline,
                    WebButtonSize::Small,
                    ButtonState::disabled_if(state.busy),
                )
                .aria_label(format!("{action} {}", d.name))
                .child(action);
                on_press_view(btn, cx, move |this, _, cx| {
                    this.scan_devices(Some(command.clone()), cx)
                })
                .into_any_element()
            } else {
                div().into_any_element()
            };
            let platform = if d.platform == DevicePlatform::Android {
                "Android"
            } else {
                "Apple simulator"
            };
            let is_booted =
                d.state.eq_ignore_ascii_case("booted") || d.state.eq_ignore_ascii_case("running");
            rows.push(
                div()
                    .p_3()
                    .flex()
                    .justify_between()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(d.name.clone())
                            .child(
                                div()
                                    .flex()
                                    .flex_wrap()
                                    .items_center()
                                    .gap_1p5()
                                    .child(
                                        div()
                                            .px_2()
                                            .py_0p5()
                                            .rounded(px(999.0))
                                            .bg(theme::longan_800())
                                            .border_1()
                                            .border_color(theme::longan_700().opacity(0.6))
                                            .text_size(px(11.0))
                                            .text_color(theme::rambutan_300())
                                            .child(platform),
                                    )
                                    .child(
                                        div()
                                            .px_2()
                                            .py_0p5()
                                            .rounded(px(999.0))
                                            .bg(if is_booted {
                                                theme::pandan_500().opacity(0.18)
                                            } else {
                                                theme::longan_800()
                                            })
                                            .text_size(px(11.0))
                                            .text_color(if is_booted {
                                                theme::pandan_300()
                                            } else {
                                                theme::rambutan_400()
                                            })
                                            .child(d.state.clone()),
                                    ),
                            ),
                    )
                    .child(control)
                    .into_any_element(),
            );
        }
        rows.extend(inventory.diagnostics.iter().map(|message| {
            div()
                .p_3()
                .text_xs()
                .child(message.clone())
                .into_any_element()
        }));
        rows
    }
}

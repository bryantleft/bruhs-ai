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

#[derive(Default)]
pub(super) struct Access {
    snapshot: Option<LocalToolsSnapshot>,
    inventory: Option<(String, DeviceInventory)>,
    busy: bool,
    error: Option<String>,
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
                        state.error = Some(error.to_string());
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
                            this.local_tools.error = Some(e.to_string());
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
                    "{} · Permission lasts until revoked or the host restarts.",
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

    pub(super) fn device_rows(&self, cx: &Context<'_, Self>) -> Vec<AnyElement> {
        let mut rows = self.tool_access_rows(ToolScope::Devices, cx);
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
            "Discover Apple simulators and connected Android devices on this host.",
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
            rows.push(
                div()
                    .p_3()
                    .flex()
                    .justify_between()
                    .items_center()
                    .gap_2()
                    .child(
                        div().min_w_0().child(d.name.clone()).child(
                            div()
                                .text_xs()
                                .text_color(theme::rambutan_500())
                                .child(format!("{platform} · {}", d.state)),
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

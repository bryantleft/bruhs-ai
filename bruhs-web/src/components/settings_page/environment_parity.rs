//! T3 Code environment parity Settings sections for `bruhs-web`:
//! - **Environments**: Managed cloud relay & offline webhook mailbox + WSL backend controls.
//! - **Devices**: Durable device access policy + live Simlock remote device pool & active leases.
//! - **Labs**: Feature flags (guest links, linked environments, Simlock remote hosts) persisted locally.

use std::collections::HashMap;

use bruhs_models::environment_parity::{
    DeviceAccessPolicy, RelayConfig, RelaySnapshot, SimlockHostSpec, SimlockPoolSnapshot,
    WslAction, WslBackendConfig, WslBackendStatus, WslDistroState,
};
use leptos::prelude::*;
use leptos::task::spawn_local;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::{SettingsRow, SettingsRowBody, SettingsSection, ROW_CONTROL};
use crate::api;
use crate::ui::{
    Badge, BadgeTone, Button, ButtonSize, ButtonVariant, Dropdown, DropdownOption, Icon, Switch,
};

const LABS_STORAGE_KEY: &str = "bruhs.labsFlags";

fn default_true() -> bool {
    true
}

/// Mirrors `bruhs-desktop`'s `LabsFlags`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct WebLabsFlags {
    #[serde(default = "default_true")]
    pub guest_links: bool,
    #[serde(default = "default_true")]
    pub linked_environments: bool,
    pub simlock_hosts: bool,
}

impl Default for WebLabsFlags {
    fn default() -> Self {
        Self {
            guest_links: true,
            linked_environments: true,
            simlock_hosts: false,
        }
    }
}

pub fn load_labs_flags() -> WebLabsFlags {
    web_sys::window()
        .and_then(|w| w.local_storage().ok().flatten())
        .and_then(|storage| storage.get_item(LABS_STORAGE_KEY).ok().flatten())
        .and_then(|raw| serde_json::from_str::<WebLabsFlags>(&raw).ok())
        .unwrap_or_default()
}

fn save_labs_flags(flags: WebLabsFlags) {
    if let Some(storage) = web_sys::window().and_then(|w| w.local_storage().ok().flatten()) {
        if let Ok(json) = serde_json::to_string(&flags) {
            let _ = storage.set_item(LABS_STORAGE_KEY, &json);
        }
    }
}

/// Environments settings: Managed cloud relay (with offline webhook holding) and WSL backend.
#[component]
pub fn EnvironmentsSettings() -> impl IntoView {
    let relay_snapshot = RwSignal::new(None::<RelaySnapshot>);
    let relay_enabled = RwSignal::new(false);
    let relay_url = RwSignal::new(String::new());
    let relay_label = RwSignal::new(String::new());
    let relay_hold = RwSignal::new(false);
    let relay_hold_hours = RwSignal::new("24".to_string());
    let relay_preview_tunnel = RwSignal::new(true);
    let relay_device_approval = RwSignal::new(true);
    let relay_publish_activity = RwSignal::new(false);
    let relay_saving = RwSignal::new(false);
    let relay_flushing = RwSignal::new(false);
    let relay_status = RwSignal::new(None::<String>);

    let wsl_status = RwSignal::new(None::<WslBackendStatus>);
    let wsl_busy = RwSignal::new(false);
    let wsl_notice = RwSignal::new(None::<String>);

    let apply_relay_snapshot = move |snap: RelaySnapshot| {
        relay_enabled.set(snap.config.enabled);
        relay_url.set(snap.config.relay_url.clone());
        relay_label.set(snap.config.environment_label.clone());
        relay_hold.set(snap.config.hold_webhooks_while_offline);
        relay_hold_hours.set(snap.config.max_hold_hours.to_string());
        relay_preview_tunnel.set(snap.config.preview_tunnel);
        relay_device_approval.set(snap.config.require_device_approval);
        relay_publish_activity.set(snap.config.publish_agent_activity);
        relay_snapshot.set(Some(snap));
    };

    Effect::new(move |_| {
        spawn_local(async move {
            if let Ok(snap) = api::relay_snapshot().await {
                apply_relay_snapshot(snap);
            }
            if let Ok(status) = api::wsl_status().await {
                wsl_status.set(Some(status));
            }
        });
    });

    let save_relay = move || {
        let max_hold_hours = relay_hold_hours
            .get_untracked()
            .trim()
            .parse::<u16>()
            .unwrap_or(24);
        let config = RelayConfig {
            enabled: relay_enabled.get_untracked(),
            relay_url: relay_url.get_untracked().trim().to_string(),
            environment_label: relay_label.get_untracked().trim().to_string(),
            publish_agent_activity: relay_publish_activity.get_untracked(),
            preview_tunnel: relay_preview_tunnel.get_untracked(),
            require_device_approval: relay_device_approval.get_untracked(),
            hold_webhooks_while_offline: relay_hold.get_untracked(),
            max_hold_hours,
        };
        if let Err(err) = config.validate() {
            relay_status.set(Some(err.to_string()));
            return;
        }
        relay_saving.set(true);
        relay_status.set(None);
        spawn_local(async move {
            match api::relay_update(&config).await {
                Ok(snap) => {
                    apply_relay_snapshot(snap);
                    relay_status.set(Some("Saved managed relay settings.".to_string()));
                }
                Err(err) => {
                    relay_status.set(Some(format!("Could not save relay settings ({err}).")));
                }
            }
            relay_saving.set(false);
        });
    };

    let flush_webhooks = move || {
        relay_flushing.set(true);
        relay_status.set(None);
        spawn_local(async move {
            match api::relay_flush_webhooks().await {
                Ok(res) => {
                    relay_status.set(Some(format!(
                        "Flushed {} held webhook(s) ({} expired, {} remaining).",
                        res.flushed, res.expired, res.remaining
                    )));
                    if let Ok(snap) = api::relay_snapshot().await {
                        apply_relay_snapshot(snap);
                    }
                }
                Err(err) => {
                    relay_status.set(Some(format!("Could not flush webhooks ({err}).")));
                }
            }
            relay_flushing.set(false);
        });
    };

    let update_wsl = move |config: WslBackendConfig| {
        wsl_busy.set(true);
        wsl_notice.set(None);
        spawn_local(async move {
            match api::wsl_update_config(&config).await {
                Ok(status) => wsl_status.set(Some(status)),
                Err(err) => wsl_notice.set(Some(format!("Could not update WSL config ({err})."))),
            }
            wsl_busy.set(false);
        });
    };

    let run_wsl_action = move |action: WslAction| {
        let distro = wsl_status
            .get_untracked()
            .and_then(|s| s.config.preferred_distro.or(s.running_distro));
        wsl_busy.set(true);
        wsl_notice.set(None);
        spawn_local(async move {
            match api::wsl_control(action, distro).await {
                Ok(status) => {
                    wsl_status.set(Some(status));
                    wsl_notice.set(Some("WSL command completed.".to_string()));
                }
                Err(err) => wsl_notice.set(Some(format!("WSL action failed ({err})."))),
            }
            wsl_busy.set(false);
        });
    };

    let held_count = Signal::derive(move || {
        relay_snapshot.with(|snap| snap.as_ref().map_or(0, |s| s.held_webhook_count))
    });

    view! {
        <SettingsSection
            id="environments"
            title="Environments"
            description="Managed cloud relay, offline webhook holding, and Windows Subsystem for Linux backend."
        >
            <SettingsRow
                title="Managed cloud relay"
                description="Expose this host through an HTTPS relay for remote threads, preview tunnels, and offline webhook delivery."
            >
                <Switch
                    checked=Signal::from(relay_enabled)
                    on_change=Callback::new(move |val| relay_enabled.set(val))
                    label="Enable managed cloud relay"
                />
            </SettingsRow>
            <SettingsRowBody>
                <div class="grid grid-cols-1 gap-3 sm:grid-cols-2">
                    <label class="flex flex-col gap-1">
                        <span class="text-label-sm text-rambutan-400">"Relay URL (https://)"</span>
                        <input
                            type="url"
                            placeholder="https://relay.bruhs.example"
                            data-testid="settings-relay-url"
                            class="min-h-control rounded-grape bg-longan-950 px-2.5 text-body-sm text-rambutan-100 inset-ring inset-ring-white/10 focus-visible:outline-1 focus-visible:outline-persimmon-400"
                            prop:value=move || relay_url.get()
                            on:input=move |ev| relay_url.set(event_target_value(&ev))
                        />
                    </label>
                    <label class="flex flex-col gap-1">
                        <span class="text-label-sm text-rambutan-400">"Environment label"</span>
                        <input
                            type="text"
                            placeholder="Mac Studio"
                            data-testid="settings-relay-label"
                            class="min-h-control rounded-grape bg-longan-950 px-2.5 text-body-sm text-rambutan-100 inset-ring inset-ring-white/10 focus-visible:outline-1 focus-visible:outline-persimmon-400"
                            prop:value=move || relay_label.get()
                            on:input=move |ev| relay_label.set(event_target_value(&ev))
                        />
                    </label>
                </div>
                <div class="mt-3 flex flex-wrap items-center justify-between gap-3">
                    <div class="flex flex-wrap items-center gap-4 text-label-sm text-rambutan-300">
                        <label class="inline-flex items-center gap-2 cursor-pointer">
                            <input
                                type="checkbox"
                                class="accent-persimmon-400"
                                prop:checked=move || relay_preview_tunnel.get()
                                on:change=move |ev| relay_preview_tunnel.set(event_target_checked(&ev))
                            />
                            <span>"Preview tunnel"</span>
                        </label>
                        <label class="inline-flex items-center gap-2 cursor-pointer">
                            <input
                                type="checkbox"
                                class="accent-persimmon-400"
                                prop:checked=move || relay_device_approval.get()
                                on:change=move |ev| relay_device_approval.set(event_target_checked(&ev))
                            />
                            <span>"Require device approval"</span>
                        </label>
                        <label class="inline-flex items-center gap-2 cursor-pointer">
                            <input
                                type="checkbox"
                                class="accent-persimmon-400"
                                prop:checked=move || relay_publish_activity.get()
                                on:change=move |ev| relay_publish_activity.set(event_target_checked(&ev))
                            />
                            <span>"Publish agent activity"</span>
                        </label>
                    </div>
                    <Button
                        variant=ButtonVariant::Outline
                        size=ButtonSize::Small
                        disabled=Signal::derive(move || relay_saving.get())
                        on_click=Callback::new(move |_| save_relay())
                    >
                        {move || if relay_saving.get() { "Saving…" } else { "Save relay" }}
                    </Button>
                </div>
                <Show when=move || relay_status.get().is_some()>
                    <p class="mt-2 text-label-sm text-rambutan-300">
                        {move || relay_status.get().unwrap_or_default()}
                    </p>
                </Show>
            </SettingsRowBody>

            <SettingsRow
                title="Hold webhooks while offline"
                description="Queue incoming scheduled-task webhooks in the relay mailbox while this host sleeps and replay them on reconnect."
                live_description=Signal::derive(move || {
                    let count = held_count.get();
                    match count {
                        0 => "Mailbox empty.".to_string(),
                        1 => "1 webhook held awaiting delivery.".to_string(),
                        n => format!("{n} webhooks held awaiting delivery."),
                    }
                })
            >
                <div class="flex items-center gap-2">
                    <input
                        type="number"
                        min="1"
                        max="24"
                        aria-label="Max offline hold hours"
                        title="Max offline hold hours (1–24)"
                        class="min-h-control w-16 rounded-grape bg-longan-950 px-2 text-center font-mono text-body-sm text-rambutan-100 inset-ring inset-ring-white/10"
                        prop:value=move || relay_hold_hours.get()
                        on:input=move |ev| relay_hold_hours.set(event_target_value(&ev))
                    />
                    <span class="text-label-sm text-rambutan-500">"h"</span>
                    <Switch
                        checked=Signal::from(relay_hold)
                        on_change=Callback::new(move |val| relay_hold.set(val))
                        label="Hold webhooks while offline"
                    />
                    <Button
                        variant=ButtonVariant::Outline
                        size=ButtonSize::Small
                        disabled=Signal::derive(move || relay_flushing.get() || held_count.get() == 0)
                        on_click=Callback::new(move |_| flush_webhooks())
                    >
                        {move || if relay_flushing.get() { "Flushing…" } else { "Flush now" }}
                    </Button>
                </div>
            </SettingsRow>

            <SettingsRow
                title="WSL backend"
                description="Run the bruhs server inside a Windows Subsystem for Linux distribution on Windows hosts."
                live_description=Signal::derive(move || {
                    wsl_status.with(|status| match status {
                        None => "Checking WSL availability…".to_string(),
                        Some(s) if !s.platform_supported => {
                            "WSL backend is only available on Windows hosts.".to_string()
                        }
                        Some(s) if !s.available => s
                            .diagnostic
                            .clone()
                            .unwrap_or_else(|| "WSL is not installed on this machine.".to_string()),
                        Some(s) => match s.distros.len() {
                            0 => "No WSL distributions installed.".to_string(),
                            1 => "1 distribution detected.".to_string(),
                            n => format!("{n} distributions detected."),
                        },
                    })
                })
            >
                {move || {
                    let supported = wsl_status.with(|s| {
                        s.as_ref().is_some_and(|st| st.platform_supported && st.available)
                    });
                    if !supported {
                        view! {
                            <Badge tone=BadgeTone::Neutral>"Host-managed"</Badge>
                        }
                        .into_any()
                    } else {
                        let enabled = Signal::derive(move || {
                            wsl_status.with(|s| s.as_ref().is_some_and(|st| st.config.enabled))
                        });
                        view! {
                            <div class="flex items-center gap-2">
                                <Switch
                                    checked=enabled
                                    on_change=Callback::new(move |val| {
                                        if let Some(mut cfg) =
                                            wsl_status.get_untracked().map(|s| s.config)
                                        {
                                            cfg.enabled = val;
                                            update_wsl(cfg);
                                        }
                                    })
                                    label="Enable WSL backend"
                                />
                                <Button
                                    variant=ButtonVariant::Outline
                                    size=ButtonSize::Small
                                    disabled=Signal::derive(move || wsl_busy.get())
                                    on_click=Callback::new(move |_| run_wsl_action(WslAction::Start))
                                >
                                    "Start"
                                </Button>
                                <Button
                                    variant=ButtonVariant::Outline
                                    size=ButtonSize::Small
                                    disabled=Signal::derive(move || wsl_busy.get())
                                    on_click=Callback::new(move |_| run_wsl_action(WslAction::Restart))
                                >
                                    "Restart"
                                </Button>
                                <Button
                                    variant=ButtonVariant::Ghost
                                    size=ButtonSize::Small
                                    disabled=Signal::derive(move || wsl_busy.get())
                                    on_click=Callback::new(move |_| run_wsl_action(WslAction::Stop))
                                >
                                    "Stop"
                                </Button>
                            </div>
                        }
                        .into_any()
                    }
                }}
            </SettingsRow>
            <Show when=move || {
                wsl_status.with(|s| s.as_ref().is_some_and(|st| !st.distros.is_empty()))
                    || wsl_notice.get().is_some()
            }>
                <SettingsRowBody>
                    {move || {
                        let distros = wsl_status
                            .with(|s| s.as_ref().map(|st| st.distros.clone()).unwrap_or_default());
                        (!distros.is_empty()).then(|| view! {
                            <div class="flex flex-wrap gap-2">
                                {distros.into_iter().map(|d| {
                                    let (tone, state_label) = match d.state {
                                        WslDistroState::Running => (BadgeTone::Success, "Running"),
                                        WslDistroState::Installing => (BadgeTone::Warning, "Installing"),
                                        WslDistroState::Stopped => (BadgeTone::Neutral, "Stopped"),
                                        WslDistroState::Unknown => (BadgeTone::Neutral, "Unknown"),
                                    };
                                    view! {
                                        <span class="inline-flex items-center gap-1.5 rounded-grape bg-longan-950/70 px-2.5 py-1 text-label-sm inset-ring inset-ring-white/5">
                                            <span class="text-rambutan-100">{d.name}</span>
                                            <Badge tone=BadgeTone::Neutral>{format!("WSL {}", d.version)}</Badge>
                                            <Badge tone=tone>{state_label}</Badge>
                                        </span>
                                    }
                                }).collect_view()}
                            </div>
                        })
                    }}
                    <Show when=move || wsl_notice.get().is_some()>
                        <p class="mt-2 text-label-sm text-rambutan-300">
                            {move || wsl_notice.get().unwrap_or_default()}
                        </p>
                    </Show>
                </SettingsRowBody>
            </Show>
        </SettingsSection>
    }
}

/// Devices settings: durable device access policy and Simlock remote device lease pool.
#[component]
pub fn DevicesSettings(labs: RwSignal<WebLabsFlags>) -> impl IntoView {
    let pool = RwSignal::new(SimlockPoolSnapshot::default());
    let policy_busy = RwSignal::new(false);
    let adding_host = RwSignal::new(false);
    let host_label = RwSignal::new(String::new());
    let host_origin = RwSignal::new(String::new());
    let host_max_devices = RwSignal::new("2".to_string());
    let host_lease_minutes = RwSignal::new("30".to_string());
    let host_error = RwSignal::new(None::<String>);
    let probes = RwSignal::new(HashMap::<Uuid, String>::new());

    Effect::new(move |_| {
        spawn_local(async move {
            if let Ok(snap) = api::simlock_snapshot().await {
                pool.set(snap);
            }
        });
    });

    let policy_options: Vec<DropdownOption> = DeviceAccessPolicy::CHOICES
        .iter()
        .map(|(k, label)| DropdownOption::new(*k, *label))
        .collect();

    let on_policy_change = Callback::new(move |key: String| {
        let Some(policy) = DeviceAccessPolicy::from_key(&key) else {
            return;
        };
        policy_busy.set(true);
        pool.update(|p| p.policy = policy);
        spawn_local(async move {
            if let Ok(snap) = api::simlock_set_policy(policy).await {
                pool.set(snap);
            }
            policy_busy.set(false);
        });
    });

    let save_host = move || {
        let spec = SimlockHostSpec {
            id: Uuid::new_v4(),
            label: host_label.get_untracked().trim().to_string(),
            origin: host_origin.get_untracked().trim().to_string(),
            max_devices: host_max_devices
                .get_untracked()
                .trim()
                .parse::<u8>()
                .unwrap_or(2),
            lease_minutes: host_lease_minutes
                .get_untracked()
                .trim()
                .parse::<u16>()
                .unwrap_or(30),
        };
        if let Err(err) = spec.validate() {
            host_error.set(Some(err.to_string()));
            return;
        }
        host_error.set(None);
        spawn_local(async move {
            match api::simlock_upsert_host(&spec).await {
                Ok(snap) => {
                    pool.set(snap);
                    host_label.set(String::new());
                    host_origin.set(String::new());
                    adding_host.set(false);
                }
                Err(err) => {
                    host_error.set(Some(format!("Could not add Simlock host ({err}).")));
                }
            }
        });
    };

    let probe_host = move |id: Uuid| {
        probes.update(|map| {
            map.insert(id, "Probing…".to_string());
        });
        spawn_local(async move {
            let msg = match api::simlock_probe_host(id).await {
                Ok(res) if res.reachable => {
                    let ms = res
                        .latency_ms
                        .map_or_else(String::new, |v| format!(" ({v} ms)"));
                    format!("Reachable{ms} — {}", res.detail)
                }
                Ok(res) => format!("Unreachable — {}", res.detail),
                Err(err) => format!("Probe failed ({err})"),
            };
            probes.update(|map| {
                map.insert(id, msg);
            });
        });
    };

    let remove_host = move |id: Uuid| {
        spawn_local(async move {
            if let Ok(snap) = api::simlock_remove_host(id).await {
                pool.set(snap);
                probes.update(|map| {
                    map.remove(&id);
                });
            }
        });
    };

    let release_lease = move |lease_id: Uuid| {
        spawn_local(async move {
            if api::simlock_release_lease(lease_id).await.is_ok() {
                pool.update(|p| p.active_leases.retain(|l| l.lease_id != lease_id));
            }
        });
    };

    view! {
        <SettingsSection
            id="devices"
            title="Devices"
            description="Control agent access to local iOS Simulators, Android Emulators, and remote Simlock device pools."
        >
            <SettingsRow
                title="Simulator & emulator access"
                description="Choose how long agent access grants to local simulators and emulators persist."
            >
                <Dropdown
                    options=Signal::derive(move || policy_options.clone())
                    value=Signal::derive(move || pool.get().policy.key().to_string())
                    on_change=on_policy_change
                    label="Simulator and emulator access policy"
                    class=ROW_CONTROL
                />
            </SettingsRow>

            <SettingsRow
                title="Simlock remote hosts"
                description="Lease remote macOS iOS Simulators and Android Emulators from a Simlock pool."
                live_description=Signal::derive(move || {
                    if !labs.get().simlock_hosts {
                        "Enable Simlock remote hosts in Labs below to configure remote pools.".to_string()
                    } else {
                        String::new()
                    }
                })
            >
                <Show
                    when=move || labs.get().simlock_hosts
                    fallback=|| view! { <Badge tone=BadgeTone::Neutral>"Labs"</Badge> }
                >
                    <div class="flex flex-wrap items-center gap-2">
                        <Badge tone=BadgeTone::Neutral>
                            {move || {
                                let count = pool.with(|p| p.hosts.len());
                                if count == 1 { "1 host".to_string() } else { format!("{count} hosts") }
                            }}
                        </Badge>
                        <Show when=move || { pool.with(|p| !p.active_leases.is_empty()) }>
                            <Badge tone=BadgeTone::Info>
                                {move || {
                                    let leases = pool.with(|p| p.active_leases.len());
                                    if leases == 1 { "1 lease".to_string() } else { format!("{leases} leases") }
                                }}
                            </Badge>
                        </Show>
                        <Button
                            variant=ButtonVariant::Outline
                            size=ButtonSize::Small
                            disabled=Signal::derive(move || policy_busy.get())
                            on_click=Callback::new(move |_| {
                                adding_host.update(|open| *open = !*open);
                                host_error.set(None);
                            })
                        >
                            {move || if adding_host.get() { "Cancel" } else { "Add host" }}
                        </Button>
                    </div>
                </Show>
            </SettingsRow>

            <Show when=move || labs.get().simlock_hosts>
                <SettingsRowBody>
                    <Show when=move || adding_host.get()>
                        <div class="mb-3 flex flex-col gap-3 rounded-grape bg-longan-950/70 p-3 inset-ring inset-ring-white/10">
                            <div class="grid grid-cols-1 gap-2.5 sm:grid-cols-2">
                                <input
                                    type="text"
                                    placeholder="Host label (e.g. Lab Studio)"
                                    data-testid="settings-simlock-label"
                                    class="min-h-control rounded-grape bg-longan-950 px-2.5 text-body-sm text-rambutan-100 inset-ring inset-ring-white/10"
                                    prop:value=move || host_label.get()
                                    on:input=move |ev| host_label.set(event_target_value(&ev))
                                />
                                <input
                                    type="url"
                                    placeholder="https://simlock.tailnet.ts.net:4700"
                                    data-testid="settings-simlock-origin"
                                    class="min-h-control rounded-grape bg-longan-950 px-2.5 text-body-sm text-rambutan-100 inset-ring inset-ring-white/10"
                                    prop:value=move || host_origin.get()
                                    on:input=move |ev| host_origin.set(event_target_value(&ev))
                                />
                            </div>
                            <div class="flex flex-wrap items-center justify-between gap-2">
                                <div class="flex items-center gap-3 text-label-sm text-rambutan-400">
                                    <label class="inline-flex items-center gap-1.5">
                                        <span>"Max devices"</span>
                                        <input
                                            type="number"
                                            min="1"
                                            max="16"
                                            class="min-h-control w-16 rounded-grape bg-longan-950 px-2 text-center font-mono text-body-sm text-rambutan-100 inset-ring inset-ring-white/10"
                                            prop:value=move || host_max_devices.get()
                                            on:input=move |ev| host_max_devices.set(event_target_value(&ev))
                                        />
                                    </label>
                                    <label class="inline-flex items-center gap-1.5">
                                        <span>"Lease (min)"</span>
                                        <input
                                            type="number"
                                            min="5"
                                            max="240"
                                            class="min-h-control w-16 rounded-grape bg-longan-950 px-2 text-center font-mono text-body-sm text-rambutan-100 inset-ring inset-ring-white/10"
                                            prop:value=move || host_lease_minutes.get()
                                            on:input=move |ev| host_lease_minutes.set(event_target_value(&ev))
                                        />
                                    </label>
                                </div>
                                <Button
                                    variant=ButtonVariant::Primary
                                    size=ButtonSize::Small
                                    on_click=Callback::new(move |_| save_host())
                                >
                                    "Save host"
                                </Button>
                            </div>
                            <Show when=move || host_error.get().is_some()>
                                <p class="text-label-sm text-lychee-300">
                                    {move || host_error.get().unwrap_or_default()}
                                </p>
                            </Show>
                        </div>
                    </Show>

                    {move || {
                        let snap = pool.get();
                        view! {
                            <div class="flex flex-col gap-2">
                                {snap.hosts.into_iter().map(|host| {
                                    let host_id = host.id;
                                    view! {
                                        <div class="flex flex-wrap items-center justify-between gap-2 rounded-grape bg-longan-950/60 px-3 py-2 inset-ring inset-ring-white/5">
                                            <div class="min-w-0">
                                                <div class="flex flex-wrap items-center gap-1.5 text-label-sm font-medium text-rambutan-100">
                                                    <span>{host.label}</span>
                                                    <Badge tone=BadgeTone::Neutral>
                                                        {format!("Max {}", host.max_devices)}
                                                    </Badge>
                                                    <Badge tone=BadgeTone::Neutral>
                                                        {format!("{}m lease", host.lease_minutes)}
                                                    </Badge>
                                                </div>
                                                <div class="truncate font-mono text-[11px] text-rambutan-400">
                                                    {host.origin}
                                                </div>
                                                {move || probes.with(|map| map.get(&host_id).cloned()).map(|note| view! {
                                                    <div class="mt-0.5 text-[11px] text-rambutan-300">{note}</div>
                                                })}
                                            </div>
                                            <div class="flex shrink-0 items-center gap-1.5">
                                                <Button
                                                    variant=ButtonVariant::Outline
                                                    size=ButtonSize::Small
                                                    on_click=Callback::new(move |_| probe_host(host_id))
                                                >
                                                    "Probe"
                                                </Button>
                                                <Button
                                                    variant=ButtonVariant::Ghost
                                                    size=ButtonSize::Small
                                                    on_click=Callback::new(move |_| remove_host(host_id))
                                                >
                                                    <Icon name="trash" size=14 />
                                                </Button>
                                            </div>
                                        </div>
                                    }
                                }).collect_view()}

                                {snap.active_leases.into_iter().map(|lease| {
                                    let lease_id = lease.lease_id;
                                    let platform_label = match lease.platform {
                                        bruhs_models::local_tools::DevicePlatform::Ios => "iOS",
                                        bruhs_models::local_tools::DevicePlatform::Android => "Android",
                                    };
                                    view! {
                                        <div class="flex flex-wrap items-center justify-between gap-2 rounded-grape bg-longan-950/60 px-3 py-2 inset-ring inset-ring-blueberry-400/20">
                                            <div class="min-w-0 text-label-sm">
                                                <div class="flex flex-wrap items-center gap-1.5 text-rambutan-100">
                                                    <Badge tone=BadgeTone::Info>"Lease"</Badge>
                                                    <span class="font-mono">{lease.device_id}</span>
                                                    <Badge tone=BadgeTone::Neutral>{platform_label}</Badge>
                                                    <Badge tone=BadgeTone::Neutral>{lease.host_label}</Badge>
                                                </div>
                                            </div>
                                            <Button
                                                variant=ButtonVariant::Outline
                                                size=ButtonSize::Small
                                                on_click=Callback::new(move |_| release_lease(lease_id))
                                            >
                                                "Release"
                                            </Button>
                                        </div>
                                    }
                                }).collect_view()}
                            </div>
                        }
                    }}
                </SettingsRowBody>
            </Show>
        </SettingsSection>
    }
}

/// Labs settings: experimental feature flags mirrored with `bruhs-desktop`.
#[component]
pub fn LabsSettings(labs: RwSignal<WebLabsFlags>) -> impl IntoView {
    let update = move |mutate: fn(&mut WebLabsFlags, bool), value: bool| {
        labs.update(|flags| {
            mutate(flags, value);
            save_labs_flags(*flags);
        });
    };
    view! {
        <SettingsSection
            id="labs"
            title="Labs"
            description="Experimental capabilities that match desktop Labs flags."
        >
            <SettingsRow
                title="Guest links"
                description="Allow issuing read-only guest links for shared threads and projects."
            >
                <Switch
                    checked=Signal::derive(move || labs.get().guest_links)
                    on_change=Callback::new(move |val| update(|f, v| f.guest_links = v, val))
                    label="Guest links"
                />
            </SettingsRow>
            <SettingsRow
                title="Linked environments"
                description="Pair remote bruhs hosts for cross-environment delegation and shared usage pools."
            >
                <Switch
                    checked=Signal::derive(move || labs.get().linked_environments)
                    on_change=Callback::new(move |val| update(|f, v| f.linked_environments = v, val))
                    label="Linked environments"
                />
            </SettingsRow>
            <SettingsRow
                title="Simlock remote hosts"
                description="Enable remote simulator and emulator lease pools in Devices."
            >
                <Switch
                    checked=Signal::derive(move || labs.get().simlock_hosts)
                    on_change=Callback::new(move |val| update(|f, v| f.simlock_hosts = v, val))
                    label="Simlock remote hosts"
                />
            </SettingsRow>
        </SettingsSection>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn web_labs_flags_defaults_match_desktop() {
        let flags = WebLabsFlags::default();
        assert!(flags.guest_links);
        assert!(flags.linked_environments);
        assert!(!flags.simlock_hosts);
        let json = serde_json::to_string(&flags).unwrap();
        let back: WebLabsFlags = serde_json::from_str(&json).unwrap();
        assert_eq!(back, flags);
    }
}

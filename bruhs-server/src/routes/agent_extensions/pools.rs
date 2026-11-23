//! Usage from this computer, configured hubs and paired environments.
use super::AppError;
use axum::{Json, extract::Path};
use bruhs_core::usage_hubs::{self as hubs, Hub, Origin, Reading, ResetCredit};
use bruhs_models::agent_extensions::QuotaWindow;
use futures_util::{StreamExt, stream};
use serde::Serialize;
use std::{
    collections::{BTreeMap, HashMap},
    sync::{LazyLock, Mutex},
    time::{Duration, Instant},
};
use uuid::Uuid;

#[derive(Serialize)]
pub struct Snapshot {
    accounts: Vec<Account>,
    notices: Vec<String>,
    external: Vec<UsagePage>,
}
#[derive(Serialize)]
struct UsagePage {
    label: String,
    url: String,
    sources: Vec<String>,
}
#[derive(Serialize)]
struct Account {
    provider: &'static str,
    label: String,
    email: Option<String>,
    workspace: Option<String>,
    on_device: bool,
    via: Vec<String>,
    zero_spend_cap: bool,
    plan: Option<String>,
    sources: Vec<String>,
    windows: Vec<QuotaWindow>,
    credits: usize,
    reset: Option<Choice>,
}
#[derive(Serialize)]
struct Choice {
    handle: Uuid,
    hub: String,
    expires: i64,
}

// The browser receives only an opaque handle. Keep the exact confirmed hub and
// credit available for retries even if a subsequent probe no longer lists it.
struct Target {
    hub: Hub,
    credit: ResetCredit,
    seen: Instant,
}
static TARGETS: LazyLock<Mutex<HashMap<Uuid, Target>>> = LazyLock::new(Default::default);

fn remember(hub: &Hub, credit: &ResetCredit) -> Choice {
    let handle = credit.selection_id(hub);
    let mut targets = TARGETS.lock().unwrap_or_else(|p| p.into_inner());
    targets.retain(|_, t| t.seen.elapsed() < Duration::from_secs(3600));
    if targets.len() >= 2048 && !targets.contains_key(&handle) {
        if let Some(oldest) = targets.iter().min_by_key(|(_, t)| t.seen).map(|(k, _)| *k) {
            targets.remove(&oldest);
        }
    }
    targets.insert(
        handle,
        Target {
            hub: hub.clone(),
            credit: credit.clone(),
            seen: Instant::now(),
        },
    );
    drop(targets);
    Choice {
        handle,
        hub: hub.label.clone(),
        expires: credit.expires_at.timestamp(),
    }
}
pub async fn redeem(
    Path(handle): Path<Uuid>,
    Json(()): Json<()>,
) -> Result<Json<String>, AppError> {
    let (hub, credit) = TARGETS
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .get(&handle)
        .filter(|t| t.seen.elapsed() < Duration::from_secs(3600))
        .map(|t| (t.hub.clone(), t.credit.clone()))
        .ok_or_else(|| {
            AppError::BadRequest(
                "This selection has expired. Refresh usage and choose a credit.".into(),
            )
        })?;
    let result = hubs::redeem(&hub, &credit).await.map_err(|error| {
        if error.kind() == std::io::ErrorKind::InvalidInput {
            AppError::BadRequest(error.to_string())
        } else {
            AppError::bad_gateway(error.to_string(), error)
        }
    })?;
    let mut message = result.message.to_owned();
    if result.cooldown_warning {
        message.push_str(" The hub could not clear its cooldown.");
    }
    Ok(Json(message))
}
pub async fn snapshot(
    hosts: Option<axum::Extension<bruhs_core::remote_hosts::RemoteHostManager>>,
) -> Result<Json<Snapshot>, AppError> {
    let configured = tokio::task::spawn_blocking(hubs::list)
        .await
        .map_err(AppError::internal)?
        .map_err(AppError::internal)?;
    let reports = stream::iter(configured.into_iter().map(|hub| async move {
        let report = hubs::report(&hub).await;
        (hub, report)
    }))
    .buffered(4)
    .collect::<Vec<_>>();
    let linked = hubs::linked::reports(
        bruhs_core::delivery::writer::desktop_state_file(),
        hosts.as_ref().map(|h| &h.0),
    );
    let (local, reports, linked) = tokio::join!(hubs::device_accounts(), reports, linked);
    let mut hub_notices = Vec::new();
    let mut readings: Vec<_> = local
        .iter()
        .map(|account| Reading {
            origin: Origin::Device,
            account,
        })
        .collect();
    for (hub, result) in &reports {
        match result {
            Ok(accounts) if accounts.is_empty() => {
                hub_notices.push(format!("{} · No accounts reported.", hub.label))
            }
            Ok(accounts) => {
                for account in accounts {
                    if let Err(e) = &account.windows {
                        hub_notices.push(format!("{} · {} · {e}", hub.label, account.label));
                    }
                    readings.push(Reading {
                        origin: Origin::Hub(hub),
                        account,
                    });
                }
            }
            Err(e) => hub_notices.push(format!("{} · {e}", hub.label)),
        }
    }
    let linked = match linked {
        Ok(hosts) => hosts,
        Err(_) => {
            hub_notices.push("Could not read the desktop’s linked usage environments.".into());
            vec![]
        }
    };
    let mut linked_accounts = Vec::new();
    for host in linked {
        match host.accounts {
            Ok(accounts) if accounts.is_empty() => {
                hub_notices.push(format!("{} · No accounts reported.", host.name))
            }
            Ok(accounts) => linked_accounts.push((
                host.name,
                accounts
                    .into_iter()
                    .map(|a| a.into_account())
                    .collect::<Vec<_>>(),
            )),
            Err(error) => hub_notices.push(format!("{} · {error}", host.name)),
        }
    }
    for (name, accounts) in &linked_accounts {
        for account in accounts {
            match &account.windows {
                Err(e) => hub_notices.push(format!("{name} · {} · {e}", account.label)),
                Ok(windows) if windows.is_empty() => hub_notices.push(format!(
                    "{name} · {} · No subscription limits reported.",
                    account.label
                )),
                Ok(_) => {}
            }
            readings.push(Reading {
                origin: Origin::Environment(name),
                account,
            });
        }
    }
    let mut notices = Vec::new();
    let mut external = BTreeMap::new();
    for reading in &readings {
        if let (Origin::Device, Err(message)) = (reading.origin, &reading.account.windows) {
            notices.push(format!("{} · {message}", reading.account.provider));
        }
        if matches!(reading.origin, Origin::Device)
            && reading.account.external.is_none()
            && reading.account.windows.as_ref().is_ok_and(Vec::is_empty)
        {
            notices.push(format!(
                "{} · No limits reported.",
                reading.account.provider
            ));
        }
        if let Some(page) = &reading.account.external {
            if reading.account.windows.as_ref().is_ok_and(Vec::is_empty) {
                notices.push(format!("{} · No subscription limits reported.", page.label));
            }
            let entry = external
                .entry(page.url.clone())
                .or_insert_with(|| UsagePage {
                    label: page.label.clone(),
                    url: page.url.clone(),
                    sources: Vec::new(),
                });
            let source = format!("{} on {}", reading.account.provider, reading.origin.label());
            if !entry.sources.contains(&source) {
                entry.sources.push(source);
            }
        }
    }
    let mut accounts = Vec::new();
    for group in hubs::group_accounts(readings) {
        let quota = group.quota.account;
        let Some(windows) = quota.windows.as_ref().ok().filter(|w| !w.is_empty()) else {
            continue;
        };
        let mut sources = Vec::new();
        let mut via = Vec::new();
        for reading in &group.readings {
            let label = reading.origin.label().to_owned();
            if !matches!(reading.origin, Origin::Device) && !via.contains(&label) {
                via.push(label.clone());
            }
            if !sources.contains(&label) {
                sources.push(label);
            }
        }
        let mut label = group.readings[0].account.label.clone();
        if group.show_workspace {
            label.push_str(" · ");
            label.push_str(group.workspace.unwrap_or("Unspecified workspace"));
        }
        let plan = group.latest_plan().map(str::to_owned);
        let mut credits = 0;
        let mut reset = None;
        if let Some(Reading {
            origin: Origin::Hub(hub),
            account,
        }) = group.credits
        {
            if let Some(bank) = &account.credits {
                credits = bank.count;
                reset = bank.next.as_ref().map(|credit| remember(hub, credit));
            }
        }
        accounts.push(Account {
            provider: quota.provider,
            zero_spend_cap: quota.zero_spend_cap,
            via,
            email: group.email().map(str::to_owned),
            workspace: group.show_workspace.then(|| {
                group
                    .workspace
                    .unwrap_or("Unspecified workspace")
                    .to_owned()
            }),
            on_device: group
                .readings
                .iter()
                .any(|r| matches!(r.origin, Origin::Device)),
            label,
            plan,
            sources,
            windows: windows.clone(),
            credits,
            reset,
        });
    }
    notices.extend(hub_notices);
    let mut seen = std::collections::HashSet::new();
    notices.retain(|notice| seen.insert(notice.clone()));
    Ok(Json(Snapshot {
        accounts,
        notices,
        external: external.into_values().collect(),
    }))
}

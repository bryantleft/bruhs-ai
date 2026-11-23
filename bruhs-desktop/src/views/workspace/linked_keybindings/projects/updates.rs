//! Provider version checks and installers are owned by the paired host.
use super::*;
use bruhs_models::agent::AgentProvider;
use bruhs_models::updates::{
    provider_cli_label, LinkedProviderRun, LinkedProviderTarget, LinkedProviderUpdate,
    ProviderUpdateState, RunOutcome, RunPhase,
};
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

#[derive(Default)]
pub(super) struct HostUpdates {
    reports: HashMap<Uuid, Result<Vec<LinkedProviderUpdate>, String>>,
    checking: HashSet<Uuid>,
    checked: HashMap<Uuid, std::time::Instant>,
    recheck: HashSet<Uuid>,
    active: HashSet<(Uuid, AgentProvider)>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::views::workspace) struct PeerUpdate {
    pub host: Uuid,
    pub machine: String,
    pub provider: AgentProvider,
    pub target: LinkedProviderTarget,
}

impl ProjectHost {
    async fn provider_request<T: serde::de::DeserializeOwned>(
        &self,
        suffix: &str,
        target: Option<&LinkedProviderTarget>,
    ) -> Result<T, String> {
        let mut url = self.endpoint.clone();
        url.set_path(&format!("/api/projects/provider-updates{suffix}"));
        if suffix.is_empty() {
            url.set_query(Some("refresh=true"));
        }
        let client = self.client()?;
        let request = match target {
            Some(body) => client.post(url).json(body),
            None => client.get(url),
        };
        let response = request
            .bearer_auth(&self.token)
            .timeout(Duration::from_secs(90))
            .send()
            .await
            .map_err(|_| {
                format!(
                    "Could not reach {}. Check again to see the host’s current state.",
                    self.name()
                )
            })?;
        match response.status() {
            reqwest::StatusCode::OK | reqwest::StatusCode::ACCEPTED => {
                schedules::schedule_reply(response, 64 * 1024).await
            }
            reqwest::StatusCode::CONFLICT => {
                Err("The available update changed or is already running. Check again.".into())
            }
            reqwest::StatusCode::FORBIDDEN | reqwest::StatusCode::UNAUTHORIZED => {
                Err("The host denied provider update access.".into())
            }
            status => Err(format!(
                "{} returned {status} while checking provider updates.",
                self.name()
            )),
        }
    }

    async fn install_provider(&self, plan: &PeerUpdate) -> Result<(), String> {
        let path = format!("/{}", plan.provider.as_str());
        let mut run: LinkedProviderRun = self.provider_request(&path, Some(&plan.target)).await?;
        let id = run.id;
        let path = format!("{path}/runs/{id}");
        loop {
            if run.id != id || run.provider != plan.provider {
                return Err("The host returned a different update run.".into());
            }
            if run.phase == RunPhase::Done {
                return match run.outcome {
                    Some(RunOutcome::Updated | RunOutcome::AlreadyCurrent) => Ok(()),
                    Some(RunOutcome::Unchanged) => {
                        Err("The installed version did not change.".into())
                    }
                    _ => Err("The installer failed on the host.".into()),
                };
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
            run = self.provider_request(&path, None).await?;
        }
    }
}

impl LinkedProjects {
    fn update_host(&self, id: Uuid) -> Option<&ProjectHost> {
        self.hosts.iter().find(|h| {
            h.credential == id
                && h.problem.is_none()
                && h.data.provider_updates
                && !h.token.is_empty()
        })
    }

    pub(in crate::views::workspace) fn skipped_update_hosts(&self) -> Vec<String> {
        self.hosts
            .iter()
            .filter_map(|h| {
                let why = if h.problem.is_some() {
                    "Unavailable"
                } else if h.data.schedules_allowed.is_none() {
                    "Connection pending"
                } else if !h.data.provider_updates {
                    "Provider update access required"
                } else if self.updates.checking.contains(&h.credential) {
                    "Checking for updates…"
                } else if matches!(self.updates.reports.get(&h.credential), Some(Err(_))) {
                    "Update check failed"
                } else if !self.updates.reports.contains_key(&h.credential) {
                    "Updates haven’t been checked"
                } else {
                    return None;
                };
                Some(format!("{} · {why}", h.name()))
            })
            .collect()
    }

    pub(in crate::views::workspace) fn peer_updates(&self) -> Vec<PeerUpdate> {
        let mut targets = Vec::new();
        for host in &self.hosts {
            if self.update_host(host.credential).is_none() {
                continue;
            }
            if let Some(Ok(rows)) = self.updates.reports.get(&host.credential) {
                for row in rows {
                    if row.can_update
                        && !self
                            .updates
                            .active
                            .contains(&(host.credential, row.provider))
                    {
                        if let (Some(installed), Some(latest)) = (&row.installed, &row.latest) {
                            targets.push(PeerUpdate {
                                host: host.credential,
                                machine: host.name(),
                                provider: row.provider,
                                target: LinkedProviderTarget {
                                    installed: installed.clone(),
                                    latest: latest.clone(),
                                },
                            });
                        }
                    }
                }
            }
        }
        targets
    }
}

impl Workspace {
    pub(super) fn sync_linked_provider_checks(&mut self, cx: &mut Context<'_, Self>) {
        let links = &mut self.linked_projects;
        links.updates.reports.retain(|id, _| {
            links
                .hosts
                .iter()
                .any(|h| h.credential == *id && h.problem.is_none() && h.data.provider_updates)
        });
        let updates = self.updates.read(cx);
        if !updates.provider_checks || !updates.enabled {
            return;
        }
        let ids: Vec<_> = links
            .hosts
            .iter()
            .filter(|h| {
                h.data.provider_updates
                    && (!links.updates.reports.contains_key(&h.credential)
                        || links.updates.checked.get(&h.credential).is_none_or(|t| {
                            t.elapsed()
                                >= Duration::from_millis(u64::from(
                                    bruhs_models::updates::POLL_INTERVAL_MS,
                                ))
                        }))
            })
            .map(|h| h.credential)
            .collect();
        for id in ids {
            if !self.linked_projects.updates.checking.contains(&id) {
                self.check_peer_providers(id, cx);
            }
        }
    }

    pub(in crate::views::workspace) fn check_linked_providers(
        &mut self,
        cx: &mut Context<'_, Self>,
    ) {
        let ids: Vec<_> = self
            .linked_projects
            .hosts
            .iter()
            .filter(|h| h.data.provider_updates)
            .map(|h| h.credential)
            .collect();
        for id in ids {
            self.check_peer_providers(id, cx);
        }
    }

    fn check_peer_providers(&mut self, id: Uuid, cx: &mut Context<'_, Self>) {
        let links = &mut self.linked_projects;
        let Some(host) = links.update_host(id).cloned() else {
            return;
        };
        if !links.updates.checking.insert(id) {
            links.updates.recheck.insert(id);
            return;
        }
        let job = crate::app_state::spawn_tokio(cx, async move {
            let result = host.provider_request("", None).await;
            (host, result)
        });
        cx.spawn(async move |this, cx| {
            let (host, result) = job.await;
            let _ = this.update(cx, |ws, cx| {
                let links = &mut ws.linked_projects;
                links.updates.checking.remove(&id);
                if links
                    .update_host(id)
                    .is_some_and(|h| h.token == host.token && h.destination() == host.destination())
                {
                    links.updates.reports.insert(id, result);
                    links.updates.checked.insert(id, std::time::Instant::now());
                }
                if links.updates.recheck.remove(&id) {
                    ws.check_peer_providers(id, cx);
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub(in crate::views::workspace) fn start_peer_update(
        &mut self,
        plan: PeerUpdate,
        cx: &mut Context<'_, Self>,
    ) {
        let links = &mut self.linked_projects;
        if !links.peer_updates().contains(&plan) {
            return;
        }
        let Some(host) = links.update_host(plan.host).cloned() else {
            return;
        };
        let key = (plan.host, plan.provider);
        if !links.updates.active.insert(key) {
            return;
        }
        let job = crate::app_state::spawn_tokio(cx, async move {
            let result = host.install_provider(&plan).await;
            (host, result)
        });
        cx.spawn(async move |this, cx| {
            let (host, result) = job.await;
            let _ = this.update(cx, |ws, cx| {
                let links = &mut ws.linked_projects;
                links.updates.active.remove(&key);
                ws.updates.update(cx, |u, cx| {
                    u.peer_update_finished(key, &host.name(), &result, cx);
                });
                if links
                    .update_host(key.0)
                    .is_some_and(|h| h.token == host.token && h.destination() == host.destination())
                {
                    if let Some(Ok(rows)) = links.updates.reports.get_mut(&key.0) {
                        if let Some(row) = rows.iter_mut().find(|r| r.provider == key.1) {
                            row.can_update = false;
                        }
                    }
                    let failed = result.is_err();
                    let detail = result.err().unwrap_or_else(|| "Update finished.".into());
                    let message =
                        format!("{} on {}. {detail}", provider_cli_label(key.1), host.name());
                    ws.meetings_hub
                        .update(cx, |hub, cx| hub.show_update_notice(message, failed, cx));
                    ws.check_peer_providers(key.0, cx);
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub(in crate::views::workspace) fn linked_provider_rows(
        &self,
        cx: &Context<'_, Self>,
    ) -> Vec<AnyElement> {
        let links = &self.linked_projects;
        let targets = links.peer_updates();
        let mut rows = Vec::new();
        for host in &links.hosts {
            let id = host.credential;
            let checking = links.updates.checking.contains(&id);
            let report = links.updates.reports.get(&id);
            let status = if let Some(problem) = &host.problem {
                problem.as_str()
            } else if host.data.schedules_allowed.is_none() {
                "Checking the connection…"
            } else if !host.data.provider_updates {
                "The host hasn’t granted provider updates."
            } else if checking {
                "Checking installed providers…"
            } else {
                match report {
                    Some(Err(error)) => error.as_str(),
                    Some(Ok(_)) => "Provider versions on this machine.",
                    None => "Check this machine for provider updates.",
                }
            };
            let control = if links.update_host(id).is_some() {
                bruhs_ui::press::on_press_view(
                    button(format!("peer-providers-check-{id}"), "Check now", checking),
                    cx,
                    move |ws, _, cx| ws.check_peer_providers(id, cx),
                )
                .into_any_element()
            } else {
                div().into_any_element()
            };
            rows.push(self.settings_row(
                format!("peer-update-host-{id}"),
                host.name(),
                status.to_owned(),
                control,
            ));
            if links.update_host(id).is_none() {
                continue;
            }
            if let Some(Ok(providers)) = report {
                for row in providers.iter().filter(|r| r.installed.is_some()) {
                    let busy = links.updates.active.contains(&(id, row.provider))
                        || row.run.as_ref().is_some_and(|r| r.phase != RunPhase::Done);
                    let detail = if busy {
                        "Updating…".into()
                    } else {
                        match (&row.installed, &row.latest, row.state) {
                            (Some(a), Some(b), ProviderUpdateState::Outdated) => {
                                format!("{a} → {b}")
                            }
                            (Some(a), _, ProviderUpdateState::UpToDate) => {
                                format!("{a} · Up to date")
                            }
                            (Some(a), _, _) => a.clone(),
                            _ => String::new(),
                        }
                    };
                    let control = if let Some(plan) = targets
                        .iter()
                        .find(|p| p.host == id && p.provider == row.provider)
                        .cloned()
                    {
                        bruhs_ui::press::on_press_view(
                            button(
                                format!("peer-provider-install-{id}-{}", row.provider.as_str()),
                                "Update",
                                busy,
                            ),
                            cx,
                            move |ws, _, cx| ws.start_peer_update(plan.clone(), cx),
                        )
                        .into_any_element()
                    } else {
                        div().into_any_element()
                    };
                    rows.push(self.settings_row(
                        format!("peer-provider-{id}-{}", row.provider.as_str()),
                        provider_cli_label(row.provider),
                        detail,
                        control,
                    ));
                }
            }
        }
        rows
    }
}

//! Desktop shortcut edits are applied separately to each linked environment.
use super::*;
use bruhs_models::logic::keybindings::{EnvironmentKeybindings, Keybinding, KeybindingEdit};
use futures_util::future::join_all;
use std::{collections::BTreeSet, time::Duration};

mod projects;
mod saved;
use bruhs_models::remote_hosts::{RemoteHost, RemoteHostSnapshot};
pub(crate) use projects::{LinkedDeliveries, TaskEditorHost};
pub(super) use projects::{LinkedProjects, PeerUpdate};
pub(super) use saved::SavedLinks;
use saved::{Destination, Vault};
#[derive(Clone)]
struct Tunnel {
    profile: uuid::Uuid,
    name: String,
    destination: Destination,
    socket: Option<std::path::PathBuf>,
}
struct LinkForm {
    origin: Entity<InputState>,
    token: Entity<InputState>,
    tunnel: Option<Tunnel>,
}
impl LinkForm {
    fn new(
        host: Option<&RemoteHost>,
        token_label: &'static str,
        w: &mut Window,
        cx: &mut Context<'_, Workspace>,
    ) -> Self {
        let tunnel = host.and_then(|host| {
            let socket = host.forwarding_socket.clone()?;
            Some(Tunnel {
                profile: host.profile.id,
                name: host.profile.name.clone(),
                destination: Destination::from_profile(&host.profile),
                socket: Some(socket),
            })
        });
        let origin = cx.new(|cx| InputState::new(w, cx).placeholder("https://bruhs.example.com"));
        let token = cx.new(|cx| InputState::new(w, cx).masked(true).placeholder(token_label));
        if tunnel.is_some() {
            token.update(cx, |f, cx| f.focus(w, cx));
        } else {
            origin.update(cx, |f, cx| f.focus(w, cx));
        }
        Self {
            origin,
            token,
            tunnel,
        }
    }
}
#[derive(Clone)]
struct LinkedHost<T = ShortcutRules> {
    credential: uuid::Uuid,
    vault_checked: bool,
    share_usage: bool,
    problem: Option<String>,
    endpoint: reqwest::Url,
    token: String,
    data: T,
    tunnel: Option<Tunnel>,
}

#[derive(Clone, Default)]
struct ShortcutRules {
    editable: bool,
    rules: Vec<Keybinding>,
}

impl<T: Default> LinkedHost<T> {
    fn endpoint(origin: &str, tunneled: bool) -> Result<reqwest::Url, String> {
        bruhs_core::linked_projects::connection::endpoint(origin, tunneled)
    }
    fn new(origin: &str, token: &str, tunnel: Option<Tunnel>) -> Result<Self, String> {
        let endpoint = Self::endpoint(origin, tunnel.is_some())?;
        let token = token.trim();
        if token.len() != 64 || !token.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err("Enter an access token issued by that environment.".into());
        }
        Ok(Self {
            credential: uuid::Uuid::new_v4(),
            vault_checked: true,
            share_usage: false,
            problem: None,
            endpoint,
            token: token.into(),
            data: T::default(),
            tunnel,
        })
    }

    fn name(&self) -> String {
        self.tunnel.as_ref().map_or_else(
            || self.endpoint.origin().ascii_serialization(),
            |t| t.name.clone(),
        )
    }
    fn identity(&self) -> String {
        self.tunnel.as_ref().map_or_else(
            || self.endpoint.origin().ascii_serialization(),
            |t| t.profile.to_string(),
        )
    }
    fn same_environment(&self, other: &Self) -> bool {
        match (&self.tunnel, &other.tunnel) {
            (Some(a), Some(b)) => a.profile == b.profile,
            (None, None) => self.endpoint == other.endpoint,
            _ => false,
        }
    }

    fn client(&self) -> Result<reqwest::Client, String> {
        bruhs_core::linked_projects::connection::client(
            self.tunnel
                .as_ref()
                .map(|t| (t.name.as_str(), t.socket.clone())),
        )
    }
    async fn restore_token(&mut self, vault: Vault, manual: bool) -> Result<(), String> {
        if self.token.is_empty() {
            if self.vault_checked && !manual {
                return Err(self
                    .problem
                    .clone()
                    .unwrap_or_else(|| "Refresh to restore the access token.".into()));
            }
            self.vault_checked = true;
            let id = self.credential;
            let destination = self.destination();
            self.token = tokio::task::spawn_blocking(move || vault.read(id, &destination))
                .await
                .map_err(|_| "Could not restore the access token.")??;
        }
        Ok(())
    }
    async fn save_token(&self, vault: Vault) -> Result<(), String> {
        let id = self.credential;
        let destination = self.destination();
        let token = self.token.clone();
        tokio::task::spawn_blocking(move || vault.write(id, &destination, &token))
            .await
            .map_err(|_| "Could not save the access token.")?
    }
}

impl LinkedHost {
    async fn open_file(&self) -> Result<(), String> {
        let mut url = self.endpoint.clone();
        url.set_path("/api/keybindings/open");
        let reply = self
            .client()?
            .post(url)
            .bearer_auth(&self.token)
            .send()
            .await
            .map_err(|_| format!("Could not reach {}.", self.name()))?;
        if reply.status() != reqwest::StatusCode::NO_CONTENT {
            return Err(format!(
                "{} could not open keybindings.json ({}).",
                self.name(),
                reply.status()
            ));
        }
        Ok(())
    }
    async fn request(
        &self,
        edit: Option<&KeybindingEdit>,
    ) -> Result<EnvironmentKeybindings, String> {
        let http = self.client()?;
        let mut url = self.endpoint.clone();
        url.set_path("/api/keybindings");
        let request = match edit {
            Some(edit) => http.post(url).json(edit),
            None => http.get(url),
        };
        let mut response = request
            .bearer_auth(&self.token)
            .send()
            .await
            .map_err(|_| format!("Could not reach {}.", self.name()))?;
        if matches!(
            response.status(),
            reqwest::StatusCode::FORBIDDEN | reqwest::StatusCode::UNAUTHORIZED
        ) {
            return Err(format!(
                "{} denied shortcut access. Link a new grant in Environments.",
                self.name()
            ));
        }
        if !response.status().is_success() {
            return Err(format!(
                "{} returned {}. Check the shortcut grant.",
                self.name(),
                response.status()
            ));
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| format!("Incomplete reply from {}.", self.name()))?
        {
            if bytes.len() + chunk.len() > 256 * 1024 {
                return Err("The host’s keybindings exceed the size limit.".into());
            }
            bytes.extend_from_slice(&chunk);
        }
        let snapshot: EnvironmentKeybindings = serde_json::from_slice(&bytes)
            .map_err(|_| format!("{} returned an unreadable keybindings file.", self.name()))?;
        bruhs_core::keybinding_file::validate_rules(&snapshot.rules)
            .map_err(|error| format!("{}: {error}", self.name()))?;
        Ok(snapshot)
    }
}

struct PendingEdit {
    edit: KeybindingEdit,
    targets: BTreeSet<uuid::Uuid>,
    acknowledged: BTreeSet<uuid::Uuid>,
}

pub(super) struct LinkedKeybindings {
    hosts: Vec<LinkedHost>,
    vault: Vault,
    revision: u64,
    reading: bool,
    watch: Option<Task<()>>,
    selected: Option<usize>,
    form: Option<LinkForm>,
    busy: bool,
    error: Option<String>,
    retry: Option<PendingEdit>,
}

impl LinkedKeybindings {
    pub(super) fn restore(saved: Option<&SavedLinks>) -> Self {
        let (hosts, selected) = saved.map(SavedLinks::restore).unwrap_or_default();
        Self {
            hosts,
            selected,
            vault: Vault::default(),
            revision: 0,
            watch: None,
            reading: false,
            form: None,
            busy: false,
            error: None,
            retry: None,
        }
    }
    pub(super) fn dismiss_form(&mut self) {
        self.form = None;
    }
    pub(super) fn linking_busy(&self) -> bool {
        self.busy || self.retry.is_some()
    }
    pub(super) fn selected_unavailable(&self) -> bool {
        self.selected
            .and_then(|i| self.hosts.get(i))
            .is_some_and(|h| h.problem.is_some())
    }
    pub(super) fn editable(&self) -> bool {
        !self.busy
            && self.retry.is_none()
            && self
                .selected
                .and_then(|i| self.hosts.get(i))
                .is_none_or(|h| h.data.editable)
            && self
                .hosts
                .iter()
                .all(|h| h.data.editable || (h.problem.is_some() && h.vault_checked))
    }
}

impl Workspace {
    pub(super) fn settings_keybindings(&self) -> &[Keybinding] {
        self.linked_keybindings
            .selected
            .and_then(|i| self.linked_keybindings.hosts.get(i))
            .map_or(&self.keybindings, |h| h.data.rules.as_slice())
    }
    fn view_host_shortcuts(&mut self, index: Option<usize>, cx: &mut Context<'_, Self>) {
        if self.linked_keybindings.busy {
            return;
        }
        self.linked_keybindings.selected = index;
        self.persist(cx);
        self.keybinding_edit_open = false;
        self.keybinding_when_open = None;
        self.keybinding_recording = None;
        self.keybinding_draft = None;
        self.set_settings_section(SettingsSection::Keyboard, cx);
    }
    pub(super) fn remote_file_button_state(&self) -> Option<bool> {
        self.linked_keybindings
            .selected
            .and_then(|i| self.linked_keybindings.hosts.get(i))
            .map(|h| h.data.editable && !self.linked_keybindings.busy)
    }
    pub(super) fn open_selected_shortcut_file(&mut self, cx: &mut Context<'_, Self>) -> bool {
        let Some(host) = self
            .linked_keybindings
            .selected
            .and_then(|i| self.linked_keybindings.hosts.get(i))
            .cloned()
        else {
            return false;
        };
        if !host.data.editable || self.linked_keybindings.busy {
            return true;
        }
        self.linked_keybindings.revision += 1;
        self.linked_keybindings.busy = true;
        let job = crate::app_state::spawn_tokio(cx, async move { host.open_file().await });
        cx.spawn(async move |this, cx| {
            let result = job.await;
            let _ = this.update(cx, |this, cx| {
                this.linked_keybindings.busy = false;
                this.linked_keybindings.error = result.err();
                cx.notify();
            });
        })
        .detach();
        cx.notify();
        true
    }

    pub(super) fn open_shortcut_link(
        &mut self,
        host: Option<&RemoteHost>,
        w: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if self.linked_keybindings.linking_busy() {
            return;
        }
        let form = LinkForm::new(host, "Shortcut access token", w, cx);
        self.reveal_settings_row("settings-linked-shortcuts".into(), cx);
        self.linked_keybindings.form = Some(form);
        if self.linked_keybindings.retry.is_none() {
            self.linked_keybindings.error = None;
        }
        cx.notify();
    }

    fn connect_shortcut_host(&mut self, cx: &mut Context<'_, Self>) {
        if self.linked_keybindings.busy {
            return;
        }
        let Some(form) = &self.linked_keybindings.form else {
            return;
        };
        let host = LinkedHost::new(
            &form.origin.read(cx).value(),
            &form.token.read(cx).value(),
            form.tunnel.clone(),
        );
        let mut host = match host {
            Ok(h) => h,
            Err(e) => {
                self.linked_keybindings.error = Some(e);
                cx.notify();
                return;
            }
        };
        self.linked_keybindings.revision += 1;
        self.linked_keybindings.busy = true;
        self.linked_keybindings.error = None;
        if let Some(existing) = self
            .linked_keybindings
            .hosts
            .iter()
            .find(|h| h.same_environment(&host))
        {
            host.credential = existing.credential;
        }
        let vault = self.linked_keybindings.vault.clone();
        let job = crate::app_state::spawn_tokio(cx, async move {
            let snapshot = host.request(None).await?;
            host.save_token(vault).await?;
            host.data.editable = snapshot.editable;
            host.data.rules = snapshot.rules;
            Ok::<_, String>(host)
        });
        cx.spawn(async move |this, cx| {
            let result = job.await;
            let _ = this.update(cx, |this, cx| {
                let state = &mut this.linked_keybindings;
                state.busy = false;
                match result {
                    Ok(host) => {
                        if let Some(i) = state.hosts.iter().position(|h| h.same_environment(&host))
                        {
                            state.hosts[i] = host;
                            state.selected = Some(i);
                        } else {
                            state.hosts.push(host);
                            state.selected = Some(state.hosts.len() - 1);
                        }
                        this.keybinding_edit_open = false;
                        state.form = None;
                        if !state.editable() {
                            this.keybinding_recording = None;
                            this.keybinding_when_open = None;
                            this.keybinding_edit_open = false;
                        }
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

    pub(super) fn apply_binding_edit(&mut self, edit: KeybindingEdit, cx: &mut Context<'_, Self>) {
        if !self.linked_keybindings.editable() {
            return;
        }
        self.save_linked_edit(
            PendingEdit {
                edit,
                targets: self
                    .linked_keybindings
                    .hosts
                    .iter()
                    .filter(|h| h.data.editable)
                    .map(|h| h.credential)
                    .collect(),
                acknowledged: BTreeSet::new(),
            },
            cx,
        );
    }

    fn save_linked_edit(&mut self, mut pending: PendingEdit, cx: &mut Context<'_, Self>) {
        let hosts: Vec<_> = self
            .linked_keybindings
            .hosts
            .iter()
            .filter(|h| pending.targets.contains(&h.credential))
            .cloned()
            .collect();
        if hosts.is_empty() {
            self.apply_local_binding_edit(&pending.edit, cx);
            return;
        }
        self.linked_keybindings.revision += 1;
        self.linked_keybindings.busy = true;
        self.linked_keybindings.error = None;
        let revision = self.linked_keybindings.revision;
        let job = crate::app_state::spawn_tokio(cx, async move {
            let access = join_all(hosts.iter().map(|h| h.request(None))).await;
            let mut failures = Vec::new();
            for (host, grant) in hosts.iter().zip(&access) {
                match grant {
                    Ok(snapshot) if !snapshot.editable => {
                        failures.push(format!("{} allows viewing shortcuts only.", host.name()))
                    }
                    Err(e) => failures.push(e.clone()),
                    _ => {}
                }
            }
            if !failures.is_empty() {
                return (pending, access, Some(failures.join(" ")));
            }
            // Acknowledged hosts may have received further edits since the
            // failure. Read their current rules without replaying our write.
            let edit = &pending.edit;
            let writes = hosts.iter().zip(access).map(|(host, snapshot)| {
                let done = pending.acknowledged.contains(&host.credential);
                async move {
                    if done {
                        snapshot
                    } else {
                        host.request(Some(edit)).await
                    }
                }
            });
            let results = join_all(writes).await;
            for (host, result) in hosts.iter().zip(&results) {
                match result {
                    Ok(_) => {
                        pending.acknowledged.insert(host.credential);
                    }
                    Err(error) => failures.push(error.clone()),
                }
            }
            let error = (!failures.is_empty()).then(|| {
                format!(
                    "{} of {} linked environments saved this edit. {}",
                    hosts.len() - failures.len(),
                    hosts.len(),
                    failures.join(" ")
                )
            });
            (pending, results, error)
        });
        cx.spawn(async move |this, cx| {
            let (pending, results, error) = job.await;
            let _ = this.update(cx, |this, cx| {
                let state = &mut this.linked_keybindings;
                state.busy = false;
                for (host, result) in state
                    .hosts
                    .iter_mut()
                    .filter(|h| pending.targets.contains(&h.credential))
                    .zip(results)
                {
                    if state.revision != revision {
                        break;
                    }
                    if let Ok(snapshot) = result {
                        host.data.editable = snapshot.editable;
                        host.data.rules = snapshot.rules;
                    }
                }
                if let Some(e) = error {
                    state.error = Some(e);
                    state.retry = Some(pending);
                } else {
                    this.apply_local_binding_edit(&pending.edit, cx);
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn apply_local_binding_edit(&mut self, edit: &KeybindingEdit, cx: &mut Context<'_, Self>) {
        match edit.apply(&mut self.keybindings) {
            Ok(()) => {
                self.keymap_changed(cx);
                self.persist_and_notify(cx);
            }
            Err(e) => {
                self.linked_keybindings.error = Some(e.into());
                cx.notify();
            }
        }
    }
}

mod controls;
mod refresh;
#[cfg(all(test, unix))]
mod tests;

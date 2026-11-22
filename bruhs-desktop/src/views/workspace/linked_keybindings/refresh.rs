//! Refresh linked keymaps while either Settings pane is visible.
use super::*;

impl Tunnel {
    pub(super) fn resolve(&self, snapshot: &RemoteHostSnapshot) -> Option<std::path::PathBuf> {
        self.destination.socket(snapshot)
    }
}
impl LinkedKeybindings {
    pub(in crate::views::workspace) fn update_tunnels(&mut self, snapshot: &RemoteHostSnapshot) {
        for h in &mut self.hosts {
            if let Some(t) = &mut h.tunnel {
                let socket = t.resolve(snapshot);
                if socket != t.socket {
                    t.socket = socket;
                    h.data.editable = false;
                    h.data.rules.clear();
                    h.problem = Some(format!("Connecting to {}…", t.name));
                    self.revision += 1;
                }
            }
        }
    }
}
impl Workspace {
    pub(in crate::views::workspace) fn watch_linked_shortcuts(
        &mut self,
        section: SettingsSection,
        cx: &mut Context<'_, Self>,
    ) {
        self.linked_keybindings.watch = None;
        if !matches!(
            section,
            SettingsSection::Keyboard | SettingsSection::Environments
        ) {
            return;
        }
        self.refresh_linked_shortcuts(cx);
        self.linked_keybindings.watch = Some(cx.spawn(async move |this, cx| loop {
            cx.background_executor().timer(Duration::from_secs(2)).await;
            if this
                .update(cx, |this, cx| {
                    if !this.settings_visible() || this.settings_section != section {
                        return false;
                    }
                    this.read_shortcut_hosts(false, cx);
                    true
                })
                .ok()
                != Some(true)
            {
                break;
            }
        }));
    }
    pub(in crate::views::workspace) fn refresh_linked_shortcuts(
        &mut self,
        cx: &mut Context<'_, Self>,
    ) {
        self.read_shortcut_hosts(true, cx);
    }
    fn read_shortcut_hosts(&mut self, manual: bool, cx: &mut Context<'_, Self>) {
        let state = &mut self.linked_keybindings;
        if state.hosts.is_empty() || state.linking_busy() || state.reading {
            return;
        }
        if manual && state.form.is_none() && state.error.take().is_some() {
            cx.notify();
        }
        state.reading = true;
        let revision = state.revision;
        let hosts = state.hosts.clone();
        let vault = state.vault.clone();
        let job = crate::app_state::spawn_tokio(cx, async move {
            let needs_tunnel = hosts
                .iter()
                .any(|h| h.tunnel.as_ref().is_some_and(|t| t.socket.is_none()));
            let tunnels = if needs_tunnel {
                crate::server_client::ServerClient::workbench_client()
                    .remote_hosts(None)
                    .await
                    .ok()
            } else {
                None
            };
            join_all(hosts.into_iter().map(|mut h| {
                let tunnels = tunnels.as_ref();
                let vault = vault.clone();
                async move {
                    if let (Some(t), Some(snapshot)) = (&mut h.tunnel, tunnels) {
                        t.socket = t.resolve(snapshot);
                    }
                    let result = async {
                        h.restore_token(vault, manual).await?;
                        h.request(None).await
                    }
                    .await;
                    match result {
                        Ok(s) => {
                            h.data.rules = s.rules;
                            h.data.editable = s.editable;
                            h.problem = None;
                        }
                        Err(error) => {
                            h.data.rules.clear();
                            h.data.editable = false;
                            h.problem = Some(error);
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
            let refreshed = job.await;
            let _ = this.update(cx, |this, cx| {
                let state = &mut this.linked_keybindings;
                state.reading = false;
                if state.revision != revision {
                    return;
                }
                let selected_rule = state
                    .selected
                    .and_then(|i| state.hosts.get(i))
                    .and_then(|h| h.data.rules.get(this.keybinding_index))
                    .cloned();
                let changed = state.hosts.iter().zip(&refreshed).any(|(a, b)| {
                    a.data.rules != b.data.rules || a.data.editable != b.data.editable || a.problem != b.problem
                });
                state.hosts = refreshed;
                if !changed {
                    return;
                }
                if this.keybinding_edit_open && state.selected.is_some() {
                    let new_index = selected_rule.and_then(|rule| {
                        this.settings_keybindings().iter().position(|r| r == &rule)
                    });
                    if let Some(i) = new_index.filter(|_| this.linked_keybindings.editable()) {
                        this.keybinding_index = i;
                    } else {
                        this.keybinding_edit_open = false;
                        this.keybinding_when_open = None;
                        this.keybinding_recording = None;
                    }
                }
                if !this.linked_keybindings.editable() {
                    this.keybinding_recording = None;
                    this.keybinding_when_open = None;
                }
                cx.notify();
            });
        })
        .detach();
    }
    pub(super) fn disconnect_shortcut_host(
        &mut self,
        host: &LinkedHost,
        cx: &mut Context<'_, Self>,
    ) {
        let state = &mut self.linked_keybindings;
        if state.busy {
            return;
        }
        let credential = host.credential;
        let vault = state.vault.clone();
        state.revision += 1;
        state.busy = true;
        let job = cx.background_spawn(async move { vault.remove(credential) });
        cx.spawn(async move |this, cx| {
            let result = job.await;
            let _ = this.update(cx, |this, cx| {
                let state = &mut this.linked_keybindings;
                state.busy = false;
                if let Err(e) = result {
                    state.error = Some(e);
                    cx.notify();
                    return;
                }
                let selected = state
                    .selected
                    .and_then(|i| state.hosts.get(i))
                    .map(|h| h.credential);
                state.hosts.retain(|h| h.credential != credential);
                state.selected = state
                    .hosts
                    .iter()
                    .position(|h| Some(h.credential) == selected);
                if state.retry.is_none() {
                    state.error = None;
                }
                this.keybinding_edit_open = false;
                this.persist_and_notify(cx);
            });
        })
        .detach();
        cx.notify();
    }
}

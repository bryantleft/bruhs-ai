//! Owned foreground SSH tunnels; no daemon takeover or automatic reconnection.
use bruhs_models::remote_hosts::*;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    io::{Read, Write},
    path::{Path, PathBuf},
    process::Stdio,
    sync::Arc,
    time::Duration,
};
use tokio::{
    io::AsyncReadExt,
    process::Command,
    sync::{watch, Mutex},
};
use uuid::Uuid;

mod process;
#[cfg(test)]
mod tests;

#[derive(Debug, thiserror::Error)]
pub enum RemoteHostError {
    #[error("{0}")]
    Invalid(String),
    #[error("{0}")]
    Conflict(String),
    #[error("SSH profile not found.")]
    NotFound,
    #[error("SSH profile storage failed: {0}")]
    Storage(#[from] std::io::Error),
}
pub type RemoteHostResult<T> = Result<T, RemoteHostError>;

#[derive(Clone, Default, Serialize, Deserialize)]
struct Store {
    hosts: BTreeMap<Uuid, RemoteHost>,
}
struct Control {
    socket: Option<PathBuf>,
    generation: Uuid,
    cancel: watch::Sender<bool>,
}
struct HostOwner(std::fs::File);
impl Drop for HostOwner {
    fn drop(&mut self) {
        // A forked child can briefly retain this file description before exec.
        // Inner owns this guard until its final manager/worker reference exits.
        if let Err(error) = self.0.unlock() {
            tracing::warn!(%error, "Could not explicitly release the SSH host owner lock");
        }
    }
}
struct Inner {
    data: Mutex<Store>,
    committed: Mutex<Store>,
    path: PathBuf,
    _lock: HostOwner,
    running: std::sync::Mutex<BTreeMap<Uuid, Control>>,
    executable: PathBuf,
    healthy: std::sync::atomic::AtomicBool,
    changes: watch::Sender<()>,
}
#[derive(Clone)]
pub struct RemoteHostManager(Arc<Inner>);

impl RemoteHostManager {
    pub fn default_path() -> std::io::Result<PathBuf> {
        Ok(crate::agent_extensions::directory()?.join("remote-hosts.json"))
    }
    pub async fn open(path: PathBuf) -> RemoteHostResult<Self> {
        Self::open_with_executable(path, PathBuf::from("ssh")).await
    }
    async fn open_with_executable(path: PathBuf, executable: PathBuf) -> RemoteHostResult<Self> {
        let (path, lock, mut data) = tokio::task::spawn_blocking(move || load(path))
            .await
            .map_err(|e| std::io::Error::other(e.to_string()))??;
        for host in data.hosts.values_mut().filter(|host| host.state.active()) {
            host.state = RemoteHostState::Interrupted;
            host.error=Some("Bruhs stopped while this tunnel was active. No connection was replayed and no saved PID was killed. Inspect any occupied local port before reconnecting.".into());
            host.connection_id = None;
        }
        let manager = Self(Arc::new(Inner {
            committed: Mutex::new(data.clone()),
            data: Mutex::new(data),
            path,
            _lock: lock,
            running: std::sync::Mutex::new(BTreeMap::new()),
            executable,
            healthy: std::sync::atomic::AtomicBool::new(true),
            changes: watch::channel(()).0,
        }));
        let mut data = manager.0.data.lock().await;
        manager.persist(&mut data).await?;
        drop(data);
        Ok(manager)
    }
    pub fn changes(&self) -> watch::Receiver<()> {
        self.0.changes.subscribe()
    }
    pub fn shutdown(&self) {
        if let Ok(running) = self.0.running.lock() {
            for control in running.values() {
                control.cancel.send_replace(true);
            }
        }
    }
    pub async fn snapshot(&self) -> RemoteHostSnapshot {
        let data = self.0.data.lock().await;
        let running = self
            .0
            .running
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let hosts = data.hosts.values().cloned().map(|mut host| {
            let control=running.get(&host.profile.id);
            host.forwarding_socket=control.filter(|c|host.state==RemoteHostState::Connected && Some(c.generation)==host.connection_id)
                .and_then(|c|c.socket.clone());
            if host.state.active() && control.is_none(){
                host.state=RemoteHostState::Interrupted;
                host.error=Some("The SSH process ended before its final status could be stored. Repair storage and restart Bruhs before reconnecting.".into());
            }
            host
        }).collect();
        drop(running);
        drop(data);
        RemoteHostSnapshot {hosts,error:(!self.0.healthy.load(std::sync::atomic::Ordering::Acquire)).then(||"SSH profile storage failed. Disconnect any active tunnels, repair storage and restart Bruhs before making another connection.".into())}
    }
    pub async fn command(
        &self,
        command: RemoteHostCommand,
    ) -> RemoteHostResult<RemoteHostSnapshot> {
        if !self.0.healthy.load(std::sync::atomic::Ordering::Acquire)
            && !matches!(command, RemoteHostCommand::Disconnect { .. })
        {
            return Err(conflict("SSH profile storage failed. Repair storage and restart Bruhs. Disconnect remains available."));
        }
        match command {
            RemoteHostCommand::Save { profile } => self.save(profile).await?,
            RemoteHostCommand::Connect { id } => self.connect(id).await?,
            RemoteHostCommand::Disconnect { id } => self.disconnect(id).await?,
            RemoteHostCommand::Delete { id } => {
                let mut data = self.0.data.lock().await;
                let host = data.hosts.get(&id).ok_or(RemoteHostError::NotFound)?;
                if host.state.active() {
                    return Err(conflict(
                        "Disconnect this tunnel before deleting its profile.",
                    ));
                }
                data.hosts.remove(&id);
                self.persist(&mut data).await?;
                drop(data);
            }
        }
        Ok(self.snapshot().await)
    }
    async fn save(&self, profile: RemoteHostProfile) -> RemoteHostResult<()> {
        profile
            .validate()
            .map_err(|e| RemoteHostError::Invalid(e.into()))?;
        let mut data = self.0.data.lock().await;
        if data
            .hosts
            .get(&profile.id)
            .is_some_and(|host| host.state.active())
        {
            return Err(conflict(
                "Disconnect this tunnel before changing its profile.",
            ));
        }
        if data.hosts.values().any(|host| {
            host.profile.id != profile.id
                && host.state.active()
                && host.profile.local_port == profile.local_port
        }) {
            return Err(conflict(
                "An active Bruhs tunnel already owns that local port.",
            ));
        }
        if !data.hosts.contains_key(&profile.id)
            && data.hosts.len() >= bruhs_models::reactive::HISTORY_LIMIT
        {
            return Err(conflict(
                "The host profile page is full. Delete an unused profile before adding another.",
            ));
        }
        data.hosts.insert(
            profile.id,
            RemoteHost {
                local_url: format!("http://127.0.0.1:{}/", profile.local_port),
                profile,
                state: RemoteHostState::Disconnected,
                error: None,
                updated_at_ms: now(),
                connection_id: None,
                forwarding_socket: None,
            },
        );
        self.persist(&mut data).await?;
        drop(data);
        Ok(())
    }
    async fn connect(&self, id: Uuid) -> RemoteHostResult<()> {
        let mut data = self.0.data.lock().await;
        let selected = data.hosts.get(&id).ok_or(RemoteHostError::NotFound)?;
        if selected.state.active() {
            return Err(conflict("This SSH tunnel is already active."));
        }
        let profile = selected.profile.clone();
        profile
            .validate()
            .map_err(|e| RemoteHostError::Invalid(e.into()))?;
        if data.hosts.values().any(|host| {
            host.profile.id != id
                && host.state.active()
                && host.profile.local_port == profile.local_port
        }) {
            return Err(conflict("Another Bruhs tunnel owns this local port."));
        }
        let generation = Uuid::new_v4();
        let (cancel, receiver) = watch::channel(false);
        let host = data.hosts.get_mut(&id).ok_or(RemoteHostError::NotFound)?;
        host.state = RemoteHostState::Connecting;
        host.error = None;
        host.connection_id = Some(generation);
        host.updated_at_ms = now();
        self.persist(&mut data).await?;
        self.0
            .running
            .lock()
            .map_err(|_| conflict("The tunnel process registry is unavailable. Restart Bruhs."))?
            .insert(
                id,
                Control {
                    generation,
                    cancel,
                    socket: None,
                },
            );
        drop(data);
        let manager = self.clone();
        tokio::spawn(async move {
            let outcome = manager.run_tunnel(&profile, generation, receiver).await;
            if let Err(error) = manager.finished(id, generation, outcome).await {
                tracing::error!(%error,"SSH tunnel completion could not be stored");
            }
        });
        Ok(())
    }
    async fn disconnect(&self, id: Uuid) -> RemoteHostResult<()> {
        let mut data = self.0.data.lock().await;
        let host = data.hosts.get_mut(&id).ok_or(RemoteHostError::NotFound)?;
        if let Some(control) = self
            .0
            .running
            .lock()
            .map_err(|_| conflict("The tunnel process registry is unavailable. Restart Bruhs."))?
            .get(&id)
        {
            control.cancel.send_replace(true);
            host.state = RemoteHostState::Disconnecting;
        } else {
            host.state = RemoteHostState::Disconnected;
            host.connection_id = None;
        }
        host.error = None;
        host.updated_at_ms = now();
        self.persist(&mut data).await?;
        drop(data);
        Ok(())
    }
    async fn connected(
        &self,
        id: Uuid,
        generation: Uuid,
        socket: Option<PathBuf>,
    ) -> RemoteHostResult<()> {
        let mut data = self.0.data.lock().await;
        if let Some(host) = data.hosts.get_mut(&id).filter(|host| {
            host.connection_id == Some(generation) && host.state == RemoteHostState::Connecting
        }) {
            if let Some(control) = self
                .0
                .running
                .lock()
                .map_err(|_| conflict("SSH process registry unavailable."))?
                .get_mut(&id)
                .filter(|c| c.generation == generation)
            {
                control.socket = socket;
            }
            host.state = RemoteHostState::Connected;
            host.updated_at_ms = now();
            self.persist(&mut data).await?;
        }
        drop(data);
        Ok(())
    }
    async fn finished(
        &self,
        id: Uuid,
        generation: Uuid,
        outcome: Result<(), String>,
    ) -> RemoteHostResult<()> {
        let mut data = self.0.data.lock().await;
        let mut persisted = Ok(());
        if let Some(host) = data
            .hosts
            .get_mut(&id)
            .filter(|host| host.connection_id == Some(generation))
        {
            let stopped_by_owner = host.state == RemoteHostState::Disconnecting;
            host.state = if stopped_by_owner || outcome.is_ok() {
                RemoteHostState::Disconnected
            } else {
                RemoteHostState::Failed
            };
            host.error = if stopped_by_owner {
                None
            } else {
                outcome.err()
            };
            host.updated_at_ms = now();
            host.connection_id = None;
            persisted = self.persist(&mut data).await;
        }
        drop(data);
        let mut running = self
            .0
            .running
            .lock()
            .map_err(|_| conflict("The tunnel process registry is unavailable."))?;
        if running
            .get(&id)
            .is_some_and(|control| control.generation == generation)
        {
            running.remove(&id);
        }
        drop(running);
        self.0.changes.send_replace(());
        persisted
    }
    async fn persist(&self, data: &mut Store) -> RemoteHostResult<()> {
        let bytes = serde_json::to_vec(data).map_err(std::io::Error::other)?;
        let path = self.0.path.clone();
        let outcome = tokio::task::spawn_blocking(move || {
            let parent = path
                .parent()
                .ok_or_else(|| std::io::Error::other("SSH profiles need a parent directory."))?;
            let mut file = tempfile::NamedTempFile::new_in(parent)?;
            file.write_all(&bytes)?;
            file.as_file().sync_all()?;
            file.persist(&path).map_err(std::io::Error::other)?;
            #[cfg(unix)]
            std::fs::File::open(parent)?.sync_all()?;
            Ok::<(), std::io::Error>(())
        })
        .await
        .map_err(|e| std::io::Error::other(e.to_string()))
        .and_then(std::convert::identity);
        if let Err(error) = outcome {
            self.0
                .healthy
                .store(false, std::sync::atomic::Ordering::Release);
            self.0.changes.send_replace(());
            *data = self.0.committed.lock().await.clone();
            return Err(RemoteHostError::Storage(error));
        }
        *self.0.committed.lock().await = data.clone();
        self.0.changes.send_replace(());
        Ok(())
    }
}

fn load(path: PathBuf) -> std::io::Result<(PathBuf, HostOwner, Store)> {
    let parent = path
        .parent()
        .ok_or_else(|| std::io::Error::other("SSH profiles need a parent directory."))?;
    std::fs::create_dir_all(parent)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700))?;
    }
    let path = parent.canonicalize()?.join(
        path.file_name()
            .ok_or_else(|| std::io::Error::other("SSH profiles need a filename."))?,
    );
    let mut options = std::fs::OpenOptions::new();
    options.create(true).read(true).write(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let lock = options.open(path.with_extension("lock"))?;
    lock.try_lock().map_err(|_| {
        std::io::Error::other(
            "Another process owns these SSH profiles. Use that running companion.",
        )
    })?;
    let lock = HostOwner(lock);
    let mut data: Store = match std::fs::File::open(&path) {
        Ok(mut f) => {
            let mut bytes = Vec::new();
            f.read_to_end(&mut bytes)?;
            serde_json::from_slice(&bytes).map_err(std::io::Error::other)?
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Store::default(),
        Err(e) => return Err(e),
    };
    for (id, host) in &mut data.hosts {
        if *id != host.profile.id {
            return Err(std::io::Error::other("An SSH profile identity does not match its store key. Restore the private host store."));
        }
        host.profile.validate().map_err(std::io::Error::other)?;
        host.local_url = format!("http://127.0.0.1:{}/", host.profile.local_port);
    }
    Ok((path, lock, data))
}
fn conflict(message: &str) -> RemoteHostError {
    RemoteHostError::Conflict(message.into())
}
fn now() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

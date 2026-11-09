//! Values live in keychain; atomically replaced metadata never contains tokens.
use super::{ConnectionError as Error, Tokens};
use bruhs_models::connections::{ConnectionState, ServiceProvider};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    fs::{self, File, OpenOptions},
    io::Write as _,
    path::PathBuf,
    sync::{Arc, Mutex, PoisonError},
};
use tokio::sync::{Mutex as AsyncMutex, OwnedMutexGuard};
use zeroize::Zeroizing;

#[derive(Clone, Serialize, Deserialize)]
pub(super) struct Metadata {
    pub id: String,
    pub account: String,
    #[serde(default)]
    pub account_id: Option<String>,
    pub scopes: Vec<String>,
    pub state: ConnectionState,
    pub registration: String,
    #[serde(default)]
    pub expires_at_ms: Option<i64>,
    #[serde(default)]
    pub refresh_available: bool,
}

type Memory = Arc<Mutex<HashMap<ServiceProvider, (Metadata, Option<Zeroizing<String>>)>>>;

#[derive(Clone)]
pub(super) struct Store {
    memory: Option<Memory>,
    locks: Arc<[Arc<AsyncMutex<()>>; 3]>,
    #[cfg(test)]
    before_write: Option<Arc<tokio::sync::Notify>>,
}

pub(super) struct Lease {
    store: Store,
    provider: ServiceProvider,
    guard: Arc<LeaseGuard>,
}

struct LeaseGuard {
    _file: Option<File>,
    _lock: OwnedMutexGuard<()>,
}

fn directory() -> Result<PathBuf, Error> {
    Ok(directories::ProjectDirs::from("dev", "bruhs", "bruhs-ai")
        .ok_or(Error::Storage)?
        .data_local_dir()
        .join("connections"))
}

fn metadata(provider: ServiceProvider) -> Result<Option<Metadata>, Error> {
    match fs::read(directory()?.join(format!("{}.json", provider.id()))) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|_| Error::Storage),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(_) => Err(Error::Storage),
    }
}

fn persist(provider: ServiceProvider, metadata: &Metadata) -> Result<(), Error> {
    let directory = directory()?;
    let mut file = tempfile::NamedTempFile::new_in(&directory).map_err(|_| Error::Storage)?;
    serde_json::to_writer(&mut file, metadata).map_err(|_| Error::Storage)?;
    file.flush()
        .and_then(|()| file.as_file().sync_all())
        .map_err(|_| Error::Storage)?;
    file.persist(directory.join(format!("{}.json", provider.id())))
        .map_err(|_| Error::Storage)?;
    #[cfg(unix)]
    File::open(&directory)
        .and_then(|file| file.sync_all())
        .map_err(|_| Error::Storage)?;
    Ok(())
}

impl Store {
    pub fn os() -> Self {
        Self {
            memory: None,
            locks: Arc::new(std::array::from_fn(|_| Arc::default())),
            #[cfg(test)]
            before_write: None,
        }
    }
    pub fn memory() -> Self {
        Self {
            memory: Some(Arc::default()),
            locks: Arc::new(std::array::from_fn(|_| Arc::default())),
            #[cfg(test)]
            before_write: None,
        }
    }

    #[cfg(test)]
    pub(super) fn metadata_only(&self, provider: ServiceProvider, metadata: Metadata) {
        self.memory
            .as_ref()
            .unwrap()
            .lock()
            .unwrap()
            .insert(provider, (metadata, None));
    }

    pub async fn metadata(&self, provider: ServiceProvider) -> Result<Option<Metadata>, Error> {
        let memory = self.memory.clone();
        tokio::task::spawn_blocking(move || match memory {
            Some(memory) => Ok(memory
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .get(&provider)
                .map(|(m, _)| m.clone())),
            None => metadata(provider),
        })
        .await
        .map_err(|_| Error::Storage)?
    }

    pub async fn lease(&self, provider: ServiceProvider) -> Result<Lease, Error> {
        let index = match provider {
            ServiceProvider::Google => 0,
            ServiceProvider::Microsoft => 1,
            ServiceProvider::Github => 2,
        };
        let lock = self.locks[index].clone().lock_owned().await;
        let file = if self.memory.is_none() {
            Some(
                tokio::task::spawn_blocking(move || {
                    let directory = directory()?;
                    fs::create_dir_all(&directory).map_err(|_| Error::Storage)?;
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::PermissionsExt as _;
                        fs::set_permissions(&directory, fs::Permissions::from_mode(0o700))
                            .map_err(|_| Error::Storage)?;
                    }
                    let mut options = OpenOptions::new();
                    options.create(true).truncate(false).read(true).write(true);
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::OpenOptionsExt as _;
                        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
                    }
                    let file = options
                        .open(directory.join(format!("{}.lock", provider.id())))
                        .map_err(|_| Error::Storage)?;
                    file.lock().map_err(|_| Error::Storage)?;
                    Ok::<_, Error>(file)
                })
                .await
                .map_err(|_| Error::Storage)??,
            )
        } else {
            None
        };
        Ok(Lease {
            store: self.clone(),
            provider,
            guard: Arc::new(LeaseGuard {
                _file: file,
                _lock: lock,
            }),
        })
    }
}

impl Lease {
    pub async fn metadata(&self) -> Result<Option<Metadata>, Error> {
        self.store.metadata(self.provider).await
    }

    pub async fn read(&self) -> Result<Option<Tokens>, Error> {
        let snapshot = self.metadata().await?;
        if snapshot
            .as_ref()
            .is_some_and(|m| m.state == ConnectionState::Disconnected)
        {
            return Ok(None);
        }
        let memory = self.store.memory.clone();
        let provider = self.provider;
        let guard = self.guard.clone();
        tokio::task::spawn_blocking(move || {
            let _guard = guard;
            let payload = if let Some(memory) = memory {
                memory
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .get(&provider)
                    .and_then(|(_, value)| value.clone())
            } else {
                if metadata(provider)?.is_none() {
                    return Ok(None);
                }
                match keyring::Entry::new("bruhs-ai-connections", provider.id())
                    .map_err(|_| Error::Storage)?
                    .get_password()
                {
                    Ok(value) => Some(Zeroizing::new(value)),
                    Err(keyring::Error::NoEntry) => None,
                    Err(_) => return Err(Error::Storage),
                }
            };
            let tokens: Option<Tokens> = payload
                .map(|value| serde_json::from_str(&value).map_err(|_| Error::Storage))
                .transpose()?;
            Ok(tokens.filter(|tokens| snapshot.as_ref().is_some_and(|m| m.id == tokens.id)))
        })
        .await
        .map_err(|_| Error::Storage)?
    }

    pub async fn write(&self, tokens: &Tokens) -> Result<(), Error> {
        let memory = self.store.memory.clone();
        let provider = self.provider;
        let value = Zeroizing::new(serde_json::to_string(tokens).map_err(|_| Error::Storage)?);
        let mut data = Metadata {
            id: tokens.id.clone(),
            account: tokens.account.clone(),
            account_id: Some(tokens.account_id.clone()),
            scopes: tokens.scopes.clone(),
            state: ConnectionState::Unavailable,
            registration: tokens.registration.clone(),
            expires_at_ms: tokens.expires_at_ms,
            refresh_available: tokens.refresh_token.is_some(),
        };
        let guard = self.guard.clone();
        #[cfg(test)]
        let before_write = self.store.before_write.clone();
        tokio::task::spawn_blocking(move || {
            let _guard = guard;
            #[cfg(test)]
            if let Some(started) = before_write {
                started.notify_one();
            }
            if let Some(memory) = memory {
                data.state = ConnectionState::Connected;
                memory
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .insert(provider, (data, Some(value)));
            } else {
                persist(provider, &data)?;
                keyring::Entry::new("bruhs-ai-connections", provider.id())
                    .and_then(|entry| entry.set_password(&value))
                    .map_err(|_| Error::Storage)?;
                data.state = ConnectionState::Connected;
                persist(provider, &data)?;
            }
            Ok(())
        })
        .await
        .map_err(|_| Error::Storage)?
    }

    pub async fn needs_sign_in(&self) -> Result<(), Error> {
        let memory = self.store.memory.clone();
        let provider = self.provider;
        let guard = self.guard.clone();
        tokio::task::spawn_blocking(move || {
            let _guard = guard;
            if let Some(memory) = memory {
                if let Some((metadata, _)) = memory
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .get_mut(&provider)
                {
                    metadata.state = ConnectionState::NeedsSignIn;
                }
            } else if let Some(mut data) = metadata(provider)? {
                data.state = ConnectionState::NeedsSignIn;
                persist(provider, &data)?;
            }
            Ok(())
        })
        .await
        .map_err(|_| Error::Storage)?
    }

    pub async fn clear(&self) -> Result<(), Error> {
        let memory = self.store.memory.clone();
        let provider = self.provider;
        let guard = self.guard.clone();
        tokio::task::spawn_blocking(move || {
            let _guard = guard;
            // Keep a durable generation fence even if no account was connected.
            let tombstone = Metadata {
                id: uuid::Uuid::new_v4().to_string(),
                account: String::new(),
                account_id: None,
                scopes: Vec::new(),
                state: ConnectionState::Disconnected,
                registration: String::new(),
                expires_at_ms: None,
                refresh_available: false,
            };
            if let Some(memory) = memory {
                memory
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .insert(provider, (tombstone, None));
            } else {
                persist(provider, &tombstone)?;
                match keyring::Entry::new("bruhs-ai-connections", provider.id())
                    .map_err(|_| Error::Storage)?
                    .delete_credential()
                {
                    Ok(()) | Err(keyring::Error::NoEntry) => {}
                    Err(_) => {
                        let mut failed = tombstone;
                        failed.state = ConnectionState::Unavailable;
                        persist(provider, &failed)?;
                        return Err(Error::Storage);
                    }
                }
            }
            Ok(())
        })
        .await
        .map_err(|_| Error::Storage)?
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn aborting_a_write_keeps_the_lease_until_blocking_io_finishes() {
        let started = Arc::new(tokio::sync::Notify::new());
        let mut store = Store::memory();
        store.before_write = Some(started.clone());
        let lease = store.lease(ServiceProvider::Google).await.unwrap();
        let memory = store.memory.clone().unwrap();
        let (held_tx, held_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let gate = std::thread::spawn(move || {
            let _blocked = memory.lock().unwrap();
            held_tx.send(()).unwrap();
            release_rx.recv().unwrap();
        });
        held_rx.recv().unwrap();
        let task = tokio::spawn(async move {
            let tokens = Tokens {
                id: "id".into(),
                access_token: "private".into(),
                refresh_token: None,
                expires_at_ms: None,
                account_id: "subject".into(),
                account: "user".into(),
                scopes: vec![],
                registration: "registration".into(),
            };
            lease.write(&tokens).await
        });
        started.notified().await;
        task.abort();
        let _ = task.await;
        assert!(tokio::time::timeout(
            std::time::Duration::from_millis(20),
            store.lease(ServiceProvider::Google)
        )
        .await
        .is_err());
        release_tx.send(()).unwrap();
        gate.join().unwrap();
        let lease = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            store.lease(ServiceProvider::Google),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(lease.read().await.unwrap().unwrap().access_token, "private");
        drop(lease);
    }

    #[tokio::test]
    async fn a_failed_disconnect_tombstone_cannot_reveal_the_old_bundle() {
        let store = Store::memory();
        let provider = ServiceProvider::Google;
        let lease = store.lease(provider).await.unwrap();
        let tokens = Tokens {
            id: "old".into(),
            access_token: "private".into(),
            refresh_token: None,
            expires_at_ms: None,
            account_id: "subject".into(),
            account: "user".into(),
            scopes: vec![],
            registration: "registration".into(),
        };
        lease.write(&tokens).await.unwrap();
        let memory = store.memory.as_ref().unwrap();
        {
            let mut memory = memory.lock().unwrap();
            let (metadata, _) = memory.get_mut(&provider).unwrap();
            metadata.id = "disconnect-fence".into();
            metadata.state = ConnectionState::Unavailable;
            drop(memory);
        }
        assert!(lease.read().await.unwrap().is_none());
        drop(lease);
    }
}

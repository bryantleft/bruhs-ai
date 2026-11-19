//! Private credential input and execution. Values never enter provider events.

mod api_client;
mod database;
pub(crate) mod mcp;
mod one_password;
pub(crate) mod private;
mod storage;
#[cfg(test)]
mod tests;
mod totp;
mod trackers;
#[cfg(test)]
#[cfg(unix)]
mod vault_tests;

pub use mcp::{is_mcp_helper, run_mcp_helper};
pub use one_password::{OnePassword, OnePasswordError};
pub use storage::SecretStore;

/// A bruh's request it may not have: a key it can't use, or a service
/// connection (D18, F27). No card was shown.
pub(crate) const NOT_GRANTED: &str = "not_granted";

/// Drop a vault create the caller refused before it reached the store,
/// wiping its values as the store wipes a refused one.
pub fn discard(create: bruhs_models::vault::VaultCreate) {
    storage::scrub(create.secret);
}

use crate::{OrchestratorEvent, QueryEvent};
use bruhs_models::{
    ids::ThreadId,
    secrets::{SecretDecision, SecretOperation, SecretRequest, FORGET_SHARED},
    vault::{
        CredentialScope, LoginFields, LoginMeta, SavedSource, SecretField, VaultAttention,
        VaultCheck, VaultEntry, VaultKind, VaultListing, VaultUpdate, CHECK_OK,
    },
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    process::Stdio,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex, MutexGuard, PoisonError,
    },
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::{mpsc, oneshot, watch},
    task::JoinHandle,
};
use zeroize::Zeroizing;

/// Matches the existing local MCP bridge's frame budget. Only metadata crosses it.
const MAX_FRAME_BYTES: usize = 64 * 1024;
const BROKER_ENV: &str = "BRUHS_SECRET_BROKER";
/// Longest a vault read waits on the keychain or 1Password (D16). A
/// keychain prompt nobody answers must not hang a run.
pub const VAULT_READ_WAIT: Duration = Duration::from_secs(30);
const INSTRUCTIONS: &str = "When you need a password, token, or API key, use the bruhs_secrets request_secret tool. Never ask the user to paste credentials into chat. Supply the exact command as an argv array and the credential environment variable. The user reviews that operation in a private form. The harness runs it in this turn's working directory and returns ONLY exit status; stdout/stderr are discarded. Never print, persist, or exfiltrate credentials, or request them as ordinary tool arguments. A shell must be explicit in argv. Saved credentials still require approval. Do not claim success on cancellation or nonzero exit. For Google Workspace, Microsoft 365, and GitHub service access, use request_connection with the provider, reason, exact argv, and access-token variable. The private form offers browser SSO using the user's existing session; connecting an account does not approve its operation. Never ask for SSO passwords, browser cookies, or refresh tokens.";

#[derive(Debug, thiserror::Error)]
pub enum SecretError {
    #[error("{0}")]
    Connection(#[from] crate::connections::ConnectionError),
    #[error("This credential request is no longer pending in this thread.")]
    NotPending,
    #[error("A submission for this credential request is already in progress.")]
    Busy,
    #[error("{0}")]
    Invalid(&'static str),
    #[error("Could not access the OS credential store. Unlock it and retry.")]
    Storage,
    #[error("Could not read or update the credential catalog. Check the app data directory permissions and retry.")]
    Catalog,
    #[error("The saved credential is unavailable. Enter a new value or decline.")]
    NotSaved,
    /// Another save changed the entry first; nothing was written.
    #[error("{}", bruhs_models::vault::ENTRY_CHANGED)]
    Conflict,
    #[error("{0}")]
    OnePassword(#[from] OnePasswordError),
}

impl SecretError {
    /// What a failed vault read shows on its entry (D15).
    fn attention(&self) -> VaultAttention {
        match self {
            Self::OnePassword(error) => error.attention(),
            Self::Storage => VaultAttention::KeychainNeedsYou,
            _ => VaultAttention::Failed,
        }
    }
}

struct Pending {
    scope: Arc<SecretScope>,
    request: SecretRequest,
    /// The vault entry Use saved reads (D17), as matched when the form was
    /// last shown; `request.saved_source` says which kind it is.
    entry: Option<String>,
    /// The answer, and for an approved command the mark that its scope runs
    /// one, made before the answer leaves so nothing slips in between.
    reply: oneshot::Sender<(Approved, Option<Running>)>,
    busy: bool,
    login_cancel: watch::Sender<bool>,
}

impl Pending {
    /// Offer what `request_match` found: an entry and where it comes from,
    /// nothing, or (on `Err`) that saved entries couldn't be checked.
    fn show_match(&mut self, matched: Result<Option<(String, SavedSource)>, ()>) {
        let (entry, storage_unavailable) = match matched {
            Ok(entry) => (entry, false),
            Err(()) => (None, true),
        };
        self.request.saved_available = entry.is_some();
        self.request.saved_source = entry.as_ref().map(|(_, source)| *source);
        self.request.storage_unavailable = storage_unavailable;
        self.entry = entry.map(|(id, _)| id);
    }
}

enum Approved {
    Run(Zeroizing<String>),
    Connected(Zeroizing<String>, oneshot::Sender<()>),
    Decline,
    /// A bruh's request refused with this status before any card: not
    /// granted, or too long to show whole.
    Refused(&'static str),
}

/// Counts one approved command of a scope until dropped: from the
/// approval, through the spawn, until the command and its process group
/// are gone. A bruh's other gated calls wait while the count is above
/// zero (D18).
struct Running(Arc<watch::Sender<usize>>);

impl Running {
    fn start(count: &Arc<watch::Sender<usize>>) -> Self {
        count.send_modify(|running| *running += 1);
        Self(count.clone())
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        self.0
            .send_modify(|running| *running = running.saturating_sub(1));
    }
}

/// Each live scope's count of [`Running`] commands, by its thread.
type RunningFlags = Arc<Mutex<HashMap<ThreadId, Arc<watch::Sender<usize>>>>>;

#[derive(Clone)]
pub struct SecretManager {
    pending: Arc<Mutex<HashMap<String, Pending>>>,
    store: SecretStore,
    connections: crate::connections::Connections,
    one_password: OnePassword,
    /// The last failed read per entry id, cleared by the next success.
    attention: Arc<Mutex<HashMap<String, VaultAttention>>>,
    /// Entries whose keychain item has a read in flight. Each id names one
    /// keychain account. Another read of that item waits for the flight to
    /// end, then reads for itself. A read whose waiter gave up keeps its
    /// blocking thread until the keychain answers; until then another read
    /// of that item is refused at once instead of parking another thread
    /// (F13).
    reading: Flights,
    read_wait: Duration,
    running: RunningFlags,
}

/// Where one value of an entry comes from.
enum Source {
    Keychain(Pick),
    OnePassword(String),
}

/// Which part of a keychain item to hand back.
#[derive(Clone, Copy)]
enum Pick {
    Value,
    Password,
    Code,
}

type Flights = Arc<Mutex<HashMap<String, Arc<Flight>>>>;

/// One keychain read in flight.
struct Flight {
    /// Set when the caller that started the read stopped waiting for it:
    /// the read is stuck behind a prompt nobody answered.
    abandoned: AtomicBool,
    /// Its sender is dropped when the read ends, which wakes the waiters.
    done: watch::Receiver<()>,
}

/// Owned by the reading task: removes the flight when the read itself
/// ends (not when its waiter gives up), then wakes the waiters.
struct InFlight {
    reading: Flights,
    id: String,
    flight: Arc<Flight>,
    _done: watch::Sender<()>,
}

impl Drop for InFlight {
    fn drop(&mut self) {
        let mut reading = self.reading.lock().unwrap_or_else(PoisonError::into_inner);
        if reading
            .get(&self.id)
            .is_some_and(|flight| Arc::ptr_eq(flight, &self.flight))
        {
            reading.remove(&self.id);
        }
        drop(reading);
        // `_done` drops after this, waking the waiters.
    }
}

/// Held by the caller that started a read: marks the flight abandoned if
/// that caller's future is dropped before the read answers.
struct Starter(Option<Arc<Flight>>);

impl Drop for Starter {
    fn drop(&mut self) {
        if let Some(flight) = self.0.take() {
            flight.abandoned.store(true, Ordering::Release);
        }
    }
}

impl Default for SecretManager {
    fn default() -> Self {
        Self::new(SecretStore::os())
    }
}

impl SecretManager {
    /// Every vault entry, value free, sorted by scope then name.
    pub async fn vault_entries(&self) -> Result<Vec<bruhs_models::vault::VaultEntry>, SecretError> {
        self.store.vault_list().await
    }

    pub async fn vault_entry(
        &self,
        id: &str,
    ) -> Result<Option<bruhs_models::vault::VaultEntry>, SecretError> {
        self.store.vault_entry(id).await
    }

    /// Create a vault entry. The caller has canonicalized a project path
    /// and checked that a bruh exists. The store runs
    /// `VaultCreate::validate` first and scrubs a refused input's values.
    pub async fn vault_create(
        &self,
        create: bruhs_models::vault::VaultCreate,
    ) -> Result<bruhs_models::vault::VaultEntry, SecretError> {
        let entry = self.store.vault_create(create).await?;
        self.refresh_saved_requests(&entry.name).await;
        Ok(entry)
    }

    /// Update an entry's values. Open thread forms asking for its name
    /// are matched again.
    pub async fn vault_update(
        &self,
        id: &str,
        update: VaultUpdate,
    ) -> Result<VaultEntry, SecretError> {
        let entry = self.store.vault_update(id, update).await?;
        self.refresh_saved_requests(&entry.name).await;
        Ok(entry)
    }

    /// Delete an entry and its keychain item (D20). Open thread forms that
    /// offered it are matched again, and its last failure is dropped.
    pub async fn vault_delete(&self, id: &str) -> Result<(), SecretError> {
        let name = self.entry_name(id).await;
        self.store.vault_delete(id).await?;
        self.forget_attention([id]);
        if let Some(name) = name {
            self.refresh_saved_requests(&name).await;
        }
        Ok(())
    }

    /// Forget the own entries of the bruhs in `gone`, which were deleted.
    /// No thread form offers a bruh's entry, so none needs a refresh.
    pub async fn forget_bruh_entries(&self, gone: &HashSet<String>) -> Result<usize, SecretError> {
        let owned: Vec<String> = self
            .store
            .vault_list()
            .await
            .unwrap_or_default()
            .into_iter()
            .filter(|entry| {
                matches!(&entry.scope, CredentialScope::Bruh { agent_id } if gone.contains(agent_id))
            })
            .map(|entry| entry.id)
            .collect();
        let forgotten = self.store.forget_bruh_entries(gone).await?;
        self.forget_attention(owned.iter().map(String::as_str));
        Ok(forgotten)
    }

    async fn entry_name(&self, id: &str) -> Option<String> {
        self.store
            .vault_entry(id)
            .await
            .ok()
            .flatten()
            .map(|entry| entry.name)
    }

    /// Drop the last failure of entries that are gone. Public for a process
    /// whose vault changed through another one: the desktop, after a delete
    /// made through bruhs-server.
    pub fn forget_attention<'a>(&self, ids: impl IntoIterator<Item = &'a str>) {
        let mut attention = self
            .attention
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        for id in ids {
            attention.remove(id);
        }
    }

    /// Match every open thread form asking for `name` again, after a vault
    /// change, and show each one what it now offers. A failed match says
    /// saved entries couldn't be checked; one that outlasts the read wait
    /// leaves the form as it was. Use saved reads its entry again either way.
    ///
    /// The manager's own vault changes call this. It is public for a
    /// process whose vault changed through another one: the desktop's
    /// Settings, Vault changes it through bruhs-server, so it calls this on
    /// its own manager after each create, update and delete (D20).
    pub async fn refresh_saved_requests(&self, name: &str) {
        let open: Vec<(String, String)> = self
            .lock()
            .iter()
            .filter(|(_, item)| {
                // A bruh's request is matched by the worker, never here (F30b).
                !item.scope.bruh
                    && item.request.connection.is_none()
                    && item.request.operation.name == name
            })
            .map(|(id, item)| (id.clone(), item.request.cwd.clone()))
            .collect();
        let mut matches = HashMap::new();
        for (id, cwd) in open {
            if !matches.contains_key(&cwd) {
                let matched =
                    tokio::time::timeout(self.read_wait, self.store.request_match(&cwd, name))
                        .await;
                let Ok(matched) = matched else {
                    continue;
                };
                matches.insert(cwd.clone(), matched.map_err(drop));
            }
            let matched = matches[&cwd].clone();
            let update = self.lock().get_mut(&id).map(|item| {
                item.show_match(matched);
                (item.scope.clone(), item.request.clone())
            });
            let Some((scope, request)) = update else {
                continue;
            };
            // UI hints must not delay the mutation.
            if let Some(tx) = scope.tx.upgrade() {
                if tx
                    .try_send(OrchestratorEvent::Query(QueryEvent::SecretRequest(request)))
                    .is_err()
                {
                    tracing::debug!("Credential metadata refresh could not reach the turn stream");
                }
            }
        }
    }

    /// The server and the desktop build theirs here, so both find `op`
    /// (ruling F12).
    pub fn new(store: SecretStore) -> Self {
        Self {
            pending: Arc::default(),
            store,
            connections: crate::connections::Connections::default(),
            one_password: OnePassword::system(),
            attention: Arc::default(),
            reading: Arc::default(),
            read_wait: VAULT_READ_WAIT,
            running: Arc::default(),
        }
    }

    /// While the returned guard lives, every vault lookup of an in-memory
    /// store waits ([`SecretStore::hold_for_tests`]).
    #[cfg(test)]
    pub(crate) fn hold_store_for_tests(&self) -> Option<impl Sized + '_> {
        self.store.hold_for_tests()
    }

    /// Where a bruh turn's broker connection file goes. For the keychain,
    /// the credentials folder, which every bruh run is kept out of (D19),
    /// so the bruh's shell can't read the broker's token there and `ps`
    /// shows only the path. An in-memory store (tests and the acceptance
    /// server) keeps them in a temporary folder of this process instead.
    pub(crate) fn broker_folder(&self) -> std::io::Result<PathBuf> {
        if self.store.is_memory() {
            return Ok(
                std::env::temp_dir().join(format!("bruhs-secret-brokers-{}", std::process::id()))
            );
        }
        storage::catalog_directory()
            .map(|directory| directory.join("brokers"))
            .map_err(std::io::Error::other)
    }

    /// Read 1Password references with this `op` (tests use a stand-in).
    #[must_use]
    pub fn with_one_password(mut self, one_password: OnePassword) -> Self {
        self.one_password = one_password;
        self
    }

    /// Bound vault reads by `wait` instead of [`VAULT_READ_WAIT`].
    #[cfg(any(test, feature = "test-support"))]
    #[must_use]
    pub fn with_read_wait(mut self, wait: Duration) -> Self {
        self.read_wait = wait;
        self
    }

    /// The vault page's listing: every entry with its last failure, and
    /// whether `op` was found (never running it). `bruhs` is left empty for
    /// the server to fill.
    pub async fn vault_listing(&self) -> Result<VaultListing, SecretError> {
        let mut entries = self.store.vault_list().await?;
        let attention = self
            .attention
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        for entry in &mut entries {
            entry.attention = attention.get(&entry.id).copied();
        }
        drop(attention);
        Ok(VaultListing {
            entries,
            bruhs: Vec::new(),
            one_password: self.one_password.installed(),
        })
    }

    /// A key's value, from the keychain or 1Password. A login, or an id no
    /// entry has, is `NotSaved`.
    pub async fn resolve_value(&self, id: &str) -> Result<Zeroizing<String>, SecretError> {
        let entry = self.known(id).await?;
        let source = match entry.kind {
            VaultKind::Value => Source::Keychain(Pick::Value),
            VaultKind::OnePassword { reference } => Source::OnePassword(reference),
            VaultKind::Login(_) => return Err(SecretError::NotSaved),
        };
        let one_password = matches!(source, Source::OnePassword(_));
        self.attended(id, self.read(id, source), one_password).await
    }

    /// One field of a login. A Saved login's username is its metadata, its
    /// password comes from the keychain and its code is made now from the
    /// stored setup; a 1Password login reads that field's reference. A
    /// field the login lacks is `NotSaved`.
    pub async fn resolve_login(
        &self,
        id: &str,
        field: SecretField,
    ) -> Result<Zeroizing<String>, SecretError> {
        let entry = self.known(id).await?;
        let VaultKind::Login(meta) = entry.kind else {
            return Err(SecretError::NotSaved);
        };
        if !meta.fields().contains(&field) {
            return Err(SecretError::NotSaved);
        }
        let source = match (meta.fields, field) {
            // Metadata, not a vault read: it leaves the entry's attention.
            (LoginFields::Saved { username, .. }, SecretField::Username) => {
                return Ok(Zeroizing::new(username));
            }
            (LoginFields::Saved { .. }, SecretField::Password) => Source::Keychain(Pick::Password),
            (LoginFields::Saved { .. }, SecretField::Totp) => Source::Keychain(Pick::Code),
            (LoginFields::OnePassword { username, .. }, SecretField::Username) => {
                Source::OnePassword(username)
            }
            (LoginFields::OnePassword { password, .. }, SecretField::Password) => {
                Source::OnePassword(password)
            }
            (LoginFields::OnePassword { totp, .. }, SecretField::Totp) => {
                Source::OnePassword(totp.ok_or(SecretError::NotSaved)?)
            }
        };
        let one_password = matches!(source, Source::OnePassword(_));
        self.attended(id, self.read(id, source), one_password).await
    }

    /// Read every value of an entry once and drop it: whether that worked,
    /// with `CHECK_OK` or the failure's owner copy, never a value.
    pub async fn vault_check(&self, id: &str) -> Result<VaultCheck, SecretError> {
        let entry = self.known(id).await?;
        let references = match entry.kind {
            VaultKind::Value
            | VaultKind::Login(LoginMeta {
                fields: LoginFields::Saved { .. },
                ..
            }) => None,
            VaultKind::OnePassword { reference } => Some(vec![reference]),
            VaultKind::Login(LoginMeta {
                fields:
                    LoginFields::OnePassword {
                        username,
                        password,
                        totp,
                    },
                ..
            }) => Some([username, password].into_iter().chain(totp).collect()),
        };
        let one_password = references.is_some();
        let read = async {
            match references {
                None => drop(self.keychain(id).await?),
                Some(references) => {
                    for reference in references {
                        drop(self.one_password.read(&reference).await?);
                    }
                }
            }
            Ok(())
        };
        Ok(match self.attended(id, read, one_password).await {
            Ok(()) => VaultCheck {
                ok: true,
                message: CHECK_OK.into(),
            },
            Err(error) => VaultCheck {
                ok: false,
                message: error.attention().message().into(),
            },
        })
    }

    async fn known(&self, id: &str) -> Result<VaultEntry, SecretError> {
        self.store
            .vault_entry(id)
            .await?
            .ok_or(SecretError::NotSaved)
    }

    /// Run a read within the read wait and record its outcome on the
    /// entry. Past the wait, a 1Password read needs the owner (its `op` is
    /// killed); a keychain read means a prompt nobody answered.
    async fn attended<T>(
        &self,
        id: &str,
        read: impl std::future::Future<Output = Result<T, SecretError>>,
        one_password: bool,
    ) -> Result<T, SecretError> {
        let result = match tokio::time::timeout(self.read_wait, read).await {
            Ok(result) => result,
            Err(_) if one_password => Err(OnePasswordError::NeedsOwner.into()),
            Err(_) => Err(SecretError::Storage),
        };
        let mut attention = self
            .attention
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        match &result {
            Ok(_) => attention.remove(id),
            Err(error) => attention.insert(id.to_owned(), error.attention()),
        };
        drop(attention);
        result
    }

    async fn read(&self, id: &str, source: Source) -> Result<Zeroizing<String>, SecretError> {
        let pick = match source {
            Source::OnePassword(reference) => {
                return Ok(self.one_password.read(&reference).await?);
            }
            Source::Keychain(pick) => pick,
        };
        let secret = self.keychain(id).await?;
        match pick {
            Pick::Value => secret.value.ok_or(SecretError::NotSaved),
            Pick::Password => secret.password.ok_or(SecretError::NotSaved),
            Pick::Code => {
                let setup = secret.totp.ok_or(SecretError::NotSaved)?;
                Ok(Zeroizing::new(totp::totp_now(&setup).await))
            }
        }
    }

    /// An entry's keychain item, read on its own task so a waiter that
    /// gives up leaves the read to finish. While one is in flight, another
    /// read of the entry waits for it to end (within the caller's read
    /// wait) and then reads for itself; once the first one's waiter has
    /// given up, it is `Storage` at once.
    async fn keychain(&self, id: &str) -> Result<storage::VaultSecret, SecretError> {
        let (flight, done) = loop {
            let mut done = {
                let mut reading = self.reading.lock().unwrap_or_else(PoisonError::into_inner);
                match reading.get(id) {
                    Some(flight) if flight.abandoned.load(Ordering::Acquire) => {
                        return Err(SecretError::Storage);
                    }
                    Some(flight) => flight.done.clone(),
                    None => {
                        let (done, receiver) = watch::channel(());
                        let flight = Arc::new(Flight {
                            abandoned: AtomicBool::new(false),
                            done: receiver,
                        });
                        reading.insert(id.to_owned(), flight.clone());
                        drop(reading);
                        break (flight, done);
                    }
                }
            };
            // Nothing is ever sent: this ends when the read's sender drops.
            let _ = done.changed().await;
        };
        let mut starter = Starter(Some(flight.clone()));
        let in_flight = InFlight {
            reading: self.reading.clone(),
            id: id.to_owned(),
            flight,
            _done: done,
        };
        let store = self.store.clone();
        let id = id.to_owned();
        let read = tokio::spawn(async move {
            let _in_flight = in_flight;
            store.vault_secret(&id).await
        })
        .await;
        starter.0 = None;
        read.map_err(|_| SecretError::Catalog)??
            .ok_or(SecretError::NotSaved)
    }

    pub fn connections(&self) -> &crate::connections::Connections {
        &self.connections
    }

    pub(crate) fn with_connections(mut self, connections: crate::connections::Connections) -> Self {
        self.connections = connections;
        self
    }

    pub async fn connect_request(
        &self,
        thread: &ThreadId,
        id: &str,
        requested_provider: bruhs_models::connections::ServiceProvider,
    ) -> Result<bruhs_models::connections::ConnectionStart, SecretError> {
        let (provider, cancel) = {
            let pending = self.lock();
            let item = pending
                .get(id)
                .filter(|p| &p.scope.thread_id == thread && p.scope.active.load(Ordering::Acquire))
                .ok_or(SecretError::NotPending)?;
            let connection = item
                .request
                .connection
                .as_ref()
                .ok_or(SecretError::Invalid(
                    "This request does not use a service connection.",
                ))?;
            if connection.provider != requested_provider {
                return Err(SecretError::Invalid(
                    "The service does not match this pending request.",
                ));
            }
            let result = (connection.provider, item.login_cancel.subscribe());
            drop(pending);
            result
        };
        self.connections
            .start_scoped(provider, Some(cancel))
            .await
            .map_err(SecretError::Connection)
    }

    fn lock(&self) -> MutexGuard<'_, HashMap<String, Pending>> {
        self.pending.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The credential scope of one turn. A `bruh` turn's requests are
    /// matched against the vault by the reactive worker, not here (F30b).
    pub(crate) fn turn(
        &self,
        thread_id: ThreadId,
        cwd: PathBuf,
        tx: mpsc::Sender<OrchestratorEvent>,
        bruh: bool,
    ) -> Arc<SecretScope> {
        let (cancel, _) = watch::channel(false);
        let running = Arc::new(watch::channel(0).0);
        self.running_flags()
            .insert(thread_id.clone(), running.clone());
        Arc::new(SecretScope {
            id: uuid::Uuid::new_v4().to_string(),
            thread_id,
            cwd,
            bruh,
            active: AtomicBool::new(true),
            execution: Mutex::new(()),
            cancel,
            running,
            tx: tx.downgrade(),
            manager: self.clone(),
        })
    }

    fn running_flags(&self) -> MutexGuard<'_, HashMap<ThreadId, Arc<watch::Sender<usize>>>> {
        self.running.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// How many approved commands the live turn of `thread_id` is running,
    /// and each change of that. `None` when the thread has no live turn.
    pub(crate) fn running_in(&self, thread_id: &ThreadId) -> Option<watch::Receiver<usize>> {
        self.running_flags()
            .get(thread_id)
            .map(|flag| flag.subscribe())
    }

    /// Take a pending request of `thread_id` for one answer: busy until the
    /// returned claim drops, unless the answer removed it.
    fn claim_pending(&self, thread_id: &ThreadId, id: &str) -> Result<Claimed, SecretError> {
        let mut pending = self.lock();
        let item = pending
            .get_mut(id)
            .filter(|p| &p.scope.thread_id == thread_id && p.scope.active.load(Ordering::Acquire))
            .ok_or(SecretError::NotPending)?;
        if item.busy {
            return Err(SecretError::Busy);
        }
        item.busy = true;
        let claimed = Claimed {
            scope: item.scope.clone(),
            entry: item.entry.clone(),
            request: item.request.clone(),
            claim: SubmissionClaim {
                manager: self.clone(),
                id: id.to_string(),
            },
        };
        drop(pending);
        Ok(claimed)
    }

    /// Hand `approved` to the waiting request, once: rechecked and removed
    /// under the lock. An approved command marks its scope running first.
    fn hand_over(
        &self,
        scope: &Arc<SecretScope>,
        id: &str,
        approved: Approved,
    ) -> Result<(), SecretError> {
        let item = {
            let mut pending = self.lock();
            if !scope.active.load(Ordering::Acquire) {
                return Err(SecretError::NotPending);
            }
            pending.remove(id).ok_or(SecretError::NotPending)?
        };
        let running = matches!(approved, Approved::Run(_) | Approved::Connected(..))
            .then(|| Running::start(&scope.running));
        item.reply
            .send((approved, running))
            .map_err(|_| SecretError::NotPending)
    }

    /// Run a bruh's pending request with vault entry `entry_id`, after the
    /// owner approved this one use (D18). Only the reactive worker calls
    /// this; no route or tool reaches it. When it returns, the scope is
    /// already marked running.
    pub(crate) async fn resolve_from_vault(
        &self,
        thread_id: &ThreadId,
        id: &str,
        entry_id: &str,
    ) -> Result<bool, SecretError> {
        let Claimed {
            scope,
            request,
            claim,
            ..
        } = self.claim_pending(thread_id, id)?;
        if request.connection.is_some() {
            return Err(SecretError::Invalid(
                "This request does not use a vault key.",
            ));
        }
        let value = self.resolve_value(entry_id).await?;
        self.hand_over(&scope, id, Approved::Run(value))?;
        drop(claim);
        scope
            .emit(QueryEvent::SecretResolved { id: id.to_string() })
            .await;
        Ok(true)
    }

    /// Refuse a bruh's pending request with `status` and no card:
    /// [`NOT_GRANTED`] when it named no key the bruh may use or asked for a
    /// service connection, or `VAULT_KEY_TOO_LONG`.
    pub(crate) async fn resolve_refused(
        &self,
        thread_id: &ThreadId,
        id: &str,
        status: &'static str,
    ) -> Result<bool, SecretError> {
        let Claimed { scope, claim, .. } = self.claim_pending(thread_id, id)?;
        self.hand_over(&scope, id, Approved::Refused(status))?;
        drop(claim);
        scope
            .emit(QueryEvent::SecretResolved { id: id.to_string() })
            .await;
        Ok(true)
    }

    /// Only the app UI calls this. Neither MCP nor provider tools can submit values.
    #[expect(
        clippy::significant_drop_tightening,
        reason = "The prepared account lease is moved into submission ownership until the approved spawn is acknowledged."
    )]
    pub async fn resolve(
        &self,
        thread_id: &ThreadId,
        id: &str,
        decision: SecretDecision,
    ) -> Result<bool, SecretError> {
        let Claimed {
            scope,
            entry,
            request,
            claim,
        } = self.claim_pending(thread_id, id)?;
        // A bruh's request runs only through the owner's approval of that
        // use (`resolve_from_vault`); a form answer may only decline it.
        if scope.bruh && !matches!(decision, SecretDecision::Decline) {
            return Err(SecretError::Invalid(
                "A bruh's request is answered by its approval.",
            ));
        }
        if request.connection.is_some()
            && !matches!(
                decision,
                SecretDecision::UseConnection { .. } | SecretDecision::Decline
            )
        {
            return Err(SecretError::Invalid(
                "Approve the connected account or decline this operation.",
            ));
        }
        let mut access = None;
        let mut spawned = None;
        let approved = match decision {
            SecretDecision::UseConnection { id } => {
                let provider = request
                    .connection
                    .as_ref()
                    .ok_or(SecretError::Invalid(
                        "This request requires a private credential.",
                    ))?
                    .provider;
                let current = self
                    .connections
                    .connection(provider)
                    .await
                    .map_err(SecretError::Connection)?;
                if current.id.as_deref() != Some(&id)
                    || current.state != bruhs_models::connections::ConnectionState::Connected
                {
                    return Err(SecretError::Invalid(
                        "The account connection changed. Refresh and approve the current account.",
                    ));
                }
                let prepared = self
                    .connections
                    .access(provider, &id)
                    .await
                    .map_err(SecretError::Connection)?;
                let (started, receive) = oneshot::channel();
                let approved = Approved::Connected(prepared.value.clone(), started);
                access = Some(prepared);
                spawned = Some(receive);
                approved
            }
            SecretDecision::Provide { value, save } => {
                let value = Zeroizing::new(value);
                if value.is_empty() || value.contains('\0') {
                    return Err(SecretError::Invalid(
                        "Enter a nonempty credential without NUL characters.",
                    ));
                }
                if save {
                    self.store
                        .save(&request.cwd, &request.operation.name, value.clone())
                        .await?;
                }
                Approved::Run(value)
            }
            // A 1Password entry runs `op read` now; a failure leaves the
            // request pending.
            SecretDecision::UseSaved => Approved::Run(
                self.resolve_value(entry.as_deref().ok_or(SecretError::NotSaved)?)
                    .await?,
            ),
            SecretDecision::ForgetSaved => {
                // Forgetting a shared entry here would remove it from every
                // project; Settings, Vault manages those.
                if request
                    .saved_source
                    .is_some_and(|source| source != SavedSource::Project)
                {
                    return Err(SecretError::Invalid(FORGET_SHARED));
                }
                let entry = entry.ok_or(SecretError::NotSaved)?;
                self.store.delete_saved(&entry).await?;
                self.forget_attention([entry.as_str()]);
                self.refresh_saved_requests(&request.operation.name).await;
                if !scope.active.load(Ordering::Acquire) {
                    return Err(SecretError::NotPending);
                }
                return Ok(false);
            }
            SecretDecision::Decline => Approved::Decline,
        };
        // Recheck after keychain I/O. Removing under the same lock makes approval one-shot.
        self.hand_over(&scope, id, approved)?;
        // The service lease fences disconnect until this approved spawn is attempted.
        let started = match spawned {
            Some(spawned) => spawned.await.is_ok(),
            None => true,
        };
        drop(access);
        if !started {
            return Err(SecretError::NotPending);
        }
        drop(claim);
        scope
            .emit(QueryEvent::SecretResolved { id: id.to_string() })
            .await;
        Ok(true)
    }
}

/// A pending request taken for one answer.
struct Claimed {
    scope: Arc<SecretScope>,
    entry: Option<String>,
    request: SecretRequest,
    claim: SubmissionClaim,
}

/// Resets a failed or cancelled submission so the form can retry.
struct SubmissionClaim {
    manager: SecretManager,
    id: String,
}
impl Drop for SubmissionClaim {
    fn drop(&mut self) {
        if let Some(item) = self.manager.lock().get_mut(&self.id) {
            item.busy = false;
        }
    }
}

pub(crate) struct SecretScope {
    id: String,
    thread_id: ThreadId,
    cwd: PathBuf,
    /// A bruh's turn: its requests skip `request_match` (F30b).
    bruh: bool,
    active: AtomicBool,
    execution: Mutex<()>,
    cancel: watch::Sender<bool>,
    /// How many approved commands run now ([`Running`]).
    running: Arc<watch::Sender<usize>>,
    tx: mpsc::WeakSender<OrchestratorEvent>,
    manager: SecretManager,
}

pub(crate) struct SecretGuard(Arc<SecretScope>);
impl Drop for SecretGuard {
    fn drop(&mut self) {
        self.0.revoke();
    }
}

pub(crate) struct SecretBroker {
    connection: Connection,
    task: JoinHandle<()>,
    /// A bruh's connection file (`SecretBroker::configure_bruh`), removed
    /// with the broker.
    file: Option<mcp::BrokerFile>,
}
impl Drop for SecretBroker {
    fn drop(&mut self) {
        self.task.abort();
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Connection {
    port: u16,
    token: String,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct BrokerRequest {
    token: String,
    operation: SecretOperation,
    #[serde(default)]
    connection: Option<bruhs_models::connections::ServiceProvider>,
}

#[derive(Debug, Serialize, Deserialize)]
struct OperationResult {
    status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    exit_code: Option<i32>,
}
impl OperationResult {
    fn status(status: &str) -> Self {
        Self {
            status: status.into(),
            exit_code: None,
        }
    }
}

impl SecretBroker {
    /// Ask this broker for `operation`, with a service connection when
    /// `provider` names one, as the helper does: the status and exit code
    /// it answers.
    #[cfg(test)]
    pub(crate) async fn request_for_tests(
        &self,
        operation: SecretOperation,
        provider: Option<bruhs_models::connections::ServiceProvider>,
    ) -> std::io::Result<(String, Option<i32>)> {
        mcp::call_from(&self.connection, operation, provider)
            .await
            .map(|result| (result.status, result.exit_code))
    }
}

impl SecretScope {
    pub(crate) fn guard(self: &Arc<Self>) -> SecretGuard {
        SecretGuard(self.clone())
    }

    fn revoke(&self) {
        let _execution = self
            .execution
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        self.active.store(false, Ordering::Release);
        self.cancel.send_replace(true);
        self.manager
            .lock()
            .retain(|_, item| item.scope.id != self.id);
        let mut running = self.manager.running_flags();
        if running
            .get(&self.thread_id)
            .is_some_and(|flag| Arc::ptr_eq(flag, &self.running))
        {
            running.remove(&self.thread_id);
        }
    }

    async fn emit(&self, event: QueryEvent) {
        if let Some(tx) = self.tx.upgrade() {
            let _ = tx.send(OrchestratorEvent::Query(event)).await;
        }
    }

    pub(crate) async fn start(self: &Arc<Self>) -> std::io::Result<SecretBroker> {
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).await?;
        let connection = Connection {
            port: listener.local_addr()?.port(),
            token: format!(
                "{}{}",
                uuid::Uuid::new_v4().simple(),
                uuid::Uuid::new_v4().simple()
            ),
        };
        let token = connection.token.clone();
        let scope = self.clone();
        let mut cancelled = self.cancel.subscribe();
        let task = tokio::spawn(async move {
            // One credential operation per turn at a time: the UI reviews one private form.
            loop {
                let accepted = tokio::select! {
                    _ = cancelled.changed() => break,
                    accepted = listener.accept() => accepted,
                };
                let Ok((mut stream, _)) = accepted else {
                    break;
                };
                tokio::select! {
                    _ = cancelled.changed() => break,
                    _ = scope.serve_connection(&mut stream, &token) => {}
                }
            }
        });
        Ok(SecretBroker {
            connection,
            task,
            file: None,
        })
    }

    async fn serve_connection(
        self: &Arc<Self>,
        stream: &mut TcpStream,
        token: &str,
    ) -> std::io::Result<()> {
        // This is the existing bridge's 15-second handshake deadline, not a user-input timeout.
        let frame =
            tokio::time::timeout(std::time::Duration::from_secs(15), read_frame(stream)).await??;
        let result = match serde_json::from_slice::<BrokerRequest>(&frame) {
            Ok(request) if same_token(&request.token, token) => {
                let mut probe = [0u8; 1];
                tokio::select! {
                    result = self.request_from(request.operation, request.connection) => result,
                    _ = stream.read(&mut probe) => OperationResult::status("cancelled"),
                }
            }
            _ => OperationResult::status("unauthorized"),
        };
        write_frame(stream, &result).await
    }

    async fn request_from(
        self: &Arc<Self>,
        operation: SecretOperation,
        provider: Option<bruhs_models::connections::ServiceProvider>,
    ) -> OperationResult {
        if operation.validate().is_err() {
            return OperationResult::status("invalid_operation");
        }
        // A service connection is never a bruh's (F27): refused before the
        // connection's account is looked up or a request is shown (ruling
        // A1 M7). Only a caller holding the broker's token gets here.
        if self.bruh && provider.is_some() {
            return OperationResult::status(NOT_GRANTED);
        }
        let Ok(cwd) = tokio::fs::canonicalize(&self.cwd).await else {
            return OperationResult::status("working_directory_unavailable");
        };
        let Some(project) = cwd.to_str() else {
            return OperationResult::status("working_directory_unavailable");
        };
        // A bruh's key is the worker's to find; this lookup could wait on
        // the keychain under the catalog lock (F30b).
        let matched = if provider.is_some() || self.bruh {
            Ok(None)
        } else {
            // Use-once still works; the form explicitly reports unavailable saved credentials.
            self.manager
                .store
                .request_match(project, &operation.name)
                .await
                .map_err(drop)
        };
        let connection = if let Some(provider) = provider {
            match self.manager.connections.connection(provider).await {
                Ok(connection) => Some(connection),
                Err(_) => return OperationResult::status("connection_unavailable"),
            }
        } else {
            None
        };
        if !self.active.load(Ordering::Acquire) {
            return OperationResult::status("cancelled");
        }
        let (reply, receive) = oneshot::channel();
        let mut item = Pending {
            scope: self.clone(),
            request: SecretRequest {
                id: uuid::Uuid::new_v4().to_string(),
                thread_id: self.thread_id.clone(),
                operation,
                cwd: project.to_owned(),
                saved_available: false,
                storage_unavailable: false,
                connection: connection.map(Box::new),
                saved_source: None,
            },
            entry: None,
            reply,
            busy: false,
            login_cancel: watch::channel(false).0,
        };
        item.show_match(matched);
        let request = item.request.clone();
        {
            let mut pending = self.manager.lock();
            if !self.active.load(Ordering::Acquire) {
                return OperationResult::status("cancelled");
            }
            pending.insert(request.id.clone(), item);
        }
        let _pending = PendingGuard {
            scope: self.clone(),
            id: request.id.clone(),
        };
        self.emit(QueryEvent::SecretRequest(request.clone())).await;
        // Dropped last, after the command's process group is gone.
        let (value, spawned, _running) = match receive.await {
            Ok((Approved::Run(value), running)) => (value, None, running),
            Ok((Approved::Connected(value, spawned), running)) => (value, Some(spawned), running),
            Ok((Approved::Decline, _)) => return OperationResult::status("declined"),
            Ok((Approved::Refused(status), _)) => return OperationResult::status(status),
            Err(_) => return OperationResult::status("cancelled"),
        };
        let mut command = tokio::process::Command::new(&request.operation.command[0]);
        if self.bruh {
            // The owner approved one key, not the server's other secrets:
            // a bruh's command gets the same environment its run does
            // (ruling V Task 11 I1).
            command
                .env_clear()
                .envs(crate::agent::unattended_environment(|key| {
                    std::env::var(key).ok()
                }));
        }
        crate::connections::protect_command(&mut command);
        command
            .args(&request.operation.command[1..])
            .current_dir(cwd)
            .env_remove(BROKER_ENV)
            .env(&request.operation.env_var, value.as_str())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt as _;
            command.as_std_mut().process_group(0);
        }
        let child = {
            // Serialize spawn with revocation; approval cannot start a child after teardown.
            let _execution = self
                .execution
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            if !self.active.load(Ordering::Acquire) {
                return OperationResult::status("cancelled");
            }
            command.spawn()
        };
        drop(command);
        drop(value);
        if let Some(spawned) = spawned {
            let _ = spawned.send(());
        }
        let Ok(mut child) = child else {
            return OperationResult::status("command_start_failed");
        };
        #[cfg(unix)]
        let _group = ProcessGroup(child.id().and_then(|id| i32::try_from(id).ok()));
        match child.wait().await {
            Ok(status) => OperationResult {
                status: "completed".into(),
                exit_code: status.code(),
            },
            Err(_) => OperationResult::status("command_wait_failed"),
        }
    }
}

/// Stop descendants still in the approved child's process group as well as the child.
#[cfg(unix)]
struct ProcessGroup(Option<i32>);
#[cfg(unix)]
impl Drop for ProcessGroup {
    fn drop(&mut self) {
        if let Some(id) = self.0 {
            // SAFETY: a positive PID from our own child names its dedicated process group.
            // Negative PID targets that group; no pointers or shared memory are involved.
            unsafe {
                libc::kill(-id, libc::SIGKILL);
            }
        }
    }
}

struct PendingGuard {
    scope: Arc<SecretScope>,
    id: String,
}
impl Drop for PendingGuard {
    fn drop(&mut self) {
        self.scope.manager.lock().remove(&self.id);
        if let Some(tx) = self.scope.tx.upgrade() {
            let _ = tx.try_send(OrchestratorEvent::Query(QueryEvent::SecretResolved {
                id: self.id.clone(),
            }));
        }
    }
}

fn storage_key(cwd: &std::path::Path, name: &str) -> String {
    let mut hash = Sha256::new();
    hash.update(cwd.as_os_str().as_encoded_bytes());
    hash.update([0]);
    hash.update(name.as_bytes());
    format!("{:x}", hash.finalize())
}

fn same_token(left: &str, right: &str) -> bool {
    left.len() == right.len()
        && left
            .bytes()
            .zip(right.bytes())
            .fold(0, |diff, (a, b)| diff | (a ^ b))
            == 0
}

async fn read_frame(stream: &mut TcpStream) -> std::io::Result<Vec<u8>> {
    let length = usize::try_from(stream.read_u32().await?).map_err(std::io::Error::other)?;
    if length > MAX_FRAME_BYTES {
        return Err(std::io::Error::other(
            "Credential metadata frame is too large.",
        ));
    }
    let mut bytes = vec![0u8; length];
    stream.read_exact(&mut bytes).await?;
    Ok(bytes)
}

async fn write_frame(stream: &mut TcpStream, value: &impl Serialize) -> std::io::Result<()> {
    let bytes = serde_json::to_vec(value).map_err(std::io::Error::other)?;
    if bytes.len() > MAX_FRAME_BYTES {
        return Err(std::io::Error::other(
            "Credential metadata frame is too large.",
        ));
    }
    stream
        .write_u32(u32::try_from(bytes.len()).map_err(std::io::Error::other)?)
        .await?;
    stream.write_all(&bytes).await
}

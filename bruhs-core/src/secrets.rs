//! Private credential input and execution. Values never enter provider events.

pub(crate) mod mcp;
mod storage;
#[cfg(test)]
mod tests;

pub use mcp::{is_mcp_helper, run_mcp_helper};
pub use storage::SecretStore;

use crate::{OrchestratorEvent, QueryEvent};
use bruhs_models::{
    ids::ThreadId,
    secrets::{SecretDecision, SecretOperation, SecretRequest},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    path::PathBuf,
    process::Stdio,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex, MutexGuard, PoisonError,
    },
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
const INSTRUCTIONS: &str = "When you need a password, token, or API key, use the bruhs_secrets request_secret tool. Never ask the user to paste credentials into chat. Supply the exact command as an argv array and the credential environment variable. The user reviews that operation in a private form. The harness runs it in this turn's working directory and returns ONLY exit status; stdout/stderr are discarded. Never print, persist, or exfiltrate credentials, or request them as ordinary tool arguments. A shell must be explicit in argv. Saved credentials still require approval. Do not claim success on cancellation or nonzero exit.";

#[derive(Debug, thiserror::Error)]
pub enum SecretError {
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
}

struct Pending {
    scope: Arc<SecretScope>,
    request: SecretRequest,
    key: String,
    reply: oneshot::Sender<Approved>,
    busy: bool,
}

enum Approved {
    Run(Zeroizing<String>),
    Decline,
}

#[derive(Clone)]
pub struct SecretManager {
    pending: Arc<Mutex<HashMap<String, Pending>>>,
    store: SecretStore,
}

impl Default for SecretManager {
    fn default() -> Self {
        Self::new(SecretStore::os())
    }
}

impl SecretManager {
    pub async fn list_credentials(
        &self,
    ) -> Result<Vec<bruhs_models::secrets::SavedCredential>, SecretError> {
        self.store.list().await
    }

    pub async fn replace_credential(&self, id: &str, value: String) -> Result<(), SecretError> {
        self.store.replace(id, value).await?;
        self.refresh_saved_requests(id, true);
        Ok(())
    }

    pub async fn delete_credential(&self, id: &str) -> Result<(), SecretError> {
        self.store.delete_saved(id).await?;
        self.refresh_saved_requests(id, false);
        Ok(())
    }

    fn refresh_saved_requests(&self, key: &str, available: bool) {
        let updates: Vec<_> = self
            .lock()
            .values_mut()
            .filter(|item| item.key == key)
            .map(|item| {
                item.request.saved_available = available;
                item.request.storage_unavailable = false;
                (item.scope.clone(), item.request.clone())
            })
            .collect();
        for (scope, request) in updates {
            // UI hints must not delay the mutation. Saved access is rechecked on every submission.
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

    pub fn new(store: SecretStore) -> Self {
        Self {
            pending: Arc::default(),
            store,
        }
    }

    fn lock(&self) -> MutexGuard<'_, HashMap<String, Pending>> {
        self.pending.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub(crate) fn turn(
        &self,
        thread_id: ThreadId,
        cwd: PathBuf,
        tx: mpsc::Sender<OrchestratorEvent>,
    ) -> Arc<SecretScope> {
        let (cancel, _) = watch::channel(false);
        Arc::new(SecretScope {
            id: uuid::Uuid::new_v4().to_string(),
            thread_id,
            cwd,
            active: AtomicBool::new(true),
            execution: Mutex::new(()),
            cancel,
            tx: tx.downgrade(),
            manager: self.clone(),
        })
    }

    /// Only the app UI calls this. Neither MCP nor provider tools can submit values.
    pub async fn resolve(
        &self,
        thread_id: &ThreadId,
        id: &str,
        decision: SecretDecision,
    ) -> Result<bool, SecretError> {
        let (scope, key, request) = {
            let mut pending = self.lock();
            let item = pending
                .get_mut(id)
                .filter(|p| {
                    &p.scope.thread_id == thread_id && p.scope.active.load(Ordering::Acquire)
                })
                .ok_or(SecretError::NotPending)?;
            if item.busy {
                return Err(SecretError::Busy);
            }
            item.busy = true;
            let claimed = (item.scope.clone(), item.key.clone(), item.request.clone());
            drop(pending);
            claimed
        };
        let claim = SubmissionClaim {
            manager: self.clone(),
            id: id.to_string(),
        };
        let approved = match decision {
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
            SecretDecision::UseSaved => {
                Approved::Run(self.store.get(&key).await?.ok_or(SecretError::NotSaved)?)
            }
            SecretDecision::ForgetSaved => {
                self.store.delete_saved(&key).await?;
                let request = {
                    let mut pending = self.lock();
                    let item = pending
                        .get_mut(id)
                        .filter(|p| p.scope.active.load(Ordering::Acquire))
                        .ok_or(SecretError::NotPending)?;
                    item.request.saved_available = false;
                    let request = item.request.clone();
                    drop(pending);
                    request
                };
                scope.emit(QueryEvent::SecretRequest(request)).await;
                return Ok(false);
            }
            SecretDecision::Decline => Approved::Decline,
        };
        // Recheck after keychain I/O. Removing under the same lock makes approval one-shot.
        let item = {
            let mut pending = self.lock();
            if !scope.active.load(Ordering::Acquire) {
                return Err(SecretError::NotPending);
            }
            pending.remove(id).ok_or(SecretError::NotPending)?
        };
        item.reply
            .send(approved)
            .map_err(|_| SecretError::NotPending)?;
        drop(claim);
        scope
            .emit(QueryEvent::SecretResolved { id: id.to_string() })
            .await;
        Ok(true)
    }
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
    active: AtomicBool,
    execution: Mutex<()>,
    cancel: watch::Sender<bool>,
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
        Ok(SecretBroker { connection, task })
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
                    result = self.request(request.operation) => result,
                    _ = stream.read(&mut probe) => OperationResult::status("cancelled"),
                }
            }
            _ => OperationResult::status("unauthorized"),
        };
        write_frame(stream, &result).await
    }

    async fn request(self: &Arc<Self>, operation: SecretOperation) -> OperationResult {
        if operation.validate().is_err() {
            return OperationResult::status("invalid_operation");
        }
        let Ok(cwd) = tokio::fs::canonicalize(&self.cwd).await else {
            return OperationResult::status("working_directory_unavailable");
        };
        let Some(project) = cwd.to_str() else {
            return OperationResult::status("working_directory_unavailable");
        };
        let key = storage_key(&cwd, &operation.name);
        let (saved_available, storage_unavailable) =
            match self.manager.store.find(project, &operation.name).await {
                Ok(value) => (value.is_some(), false),
                // Use-once still works; the form explicitly reports unavailable saved credentials.
                Err(_) => (false, true),
            };
        if !self.active.load(Ordering::Acquire) {
            return OperationResult::status("cancelled");
        }
        let request = SecretRequest {
            id: uuid::Uuid::new_v4().to_string(),
            thread_id: self.thread_id.clone(),
            operation,
            cwd: project.to_owned(),
            saved_available,
            storage_unavailable,
        };
        let (reply, receive) = oneshot::channel();
        {
            let mut pending = self.manager.lock();
            if !self.active.load(Ordering::Acquire) {
                return OperationResult::status("cancelled");
            }
            pending.insert(
                request.id.clone(),
                Pending {
                    scope: self.clone(),
                    request: request.clone(),
                    key,
                    reply,
                    busy: false,
                },
            );
        }
        let _pending = PendingGuard {
            scope: self.clone(),
            id: request.id.clone(),
        };
        self.emit(QueryEvent::SecretRequest(request.clone())).await;
        let value = match receive.await {
            Ok(Approved::Run(value)) => value,
            Ok(Approved::Decline) => return OperationResult::status("declined"),
            Err(_) => return OperationResult::status("cancelled"),
        };
        let mut command = tokio::process::Command::new(&request.operation.command[0]);
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

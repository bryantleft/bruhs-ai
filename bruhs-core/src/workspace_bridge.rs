//! Opt-in, session-scoped IPC to the running workspace. The transport cannot
//! grant permissions or execute operations: the app owns those decisions.
//! The protocol and the logic apps answer it with live in
//! `bruhs_models::workspace_bridge`.

#[cfg(test)]
mod tests;
mod wire;
pub use bruhs_models::workspace_bridge::{
    BridgeRequest, BridgeResponse, FinanceReadSection, HealthReadSection, MeetingReadSection,
    PersonReadSection, ProposalError, WorkflowProposal, MAX_REQUEST_BYTES, MAX_RESPONSE_BYTES,
};

use rand::TryRngCore;
use serde::{Deserialize, Serialize};
use std::{
    io,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use tokio::{
    net::{UnixListener, UnixStream},
    sync::{mpsc, oneshot},
    task::JoinHandle,
};

const TIMEOUT: Duration = Duration::from_secs(15);

/// The receiver processes requests on its own UI thread and rechecks current
/// grants. A dropped receiver or response cancels the request, never succeeds.
pub struct PendingRequest {
    pub request: BridgeRequest,
    pub respond: oneshot::Sender<BridgeResponse>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Connection {
    socket: PathBuf,
    token: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Envelope {
    token: String,
    request: BridgeRequest,
}

/// Dropping a session revokes its token, stops its listener, and removes only
/// its own ephemeral socket and private connection file. Nothing is auto-enabled.
pub struct BridgeSession {
    directory: PathBuf,
    connection_path: PathBuf,
    task: JoinHandle<()>,
}

impl BridgeSession {
    pub fn start() -> io::Result<(Self, mpsc::Receiver<PendingRequest>)> {
        let directory = wire::private_directory()?;
        match Self::bind(&directory) {
            Ok(session) => Ok(session),
            Err(error) => {
                wire::remove_session_files(&directory);
                Err(error)
            }
        }
    }

    fn bind(directory: &Path) -> io::Result<(Self, mpsc::Receiver<PendingRequest>)> {
        let socket = directory.join("bridge.sock");
        let listener = UnixListener::bind(&socket)?;
        wire::restrict_permissions(&socket)?;
        let mut random = [0u8; 32];
        rand::rngs::OsRng
            .try_fill_bytes(&mut random)
            .map_err(io::Error::other)?;
        let token = random
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        let connection_path = directory.join("connection.json");
        wire::write_connection(
            &connection_path,
            &Connection {
                socket,
                token: token.clone(),
            },
        )?;
        let (sender, receiver) = mpsc::channel(16);
        let task = tokio::spawn(serve(listener, token, sender));
        Ok((
            Self {
                directory: directory.to_owned(),
                connection_path,
                task,
            },
            receiver,
        ))
    }

    pub fn connection_path(&self) -> &Path {
        &self.connection_path
    }
}

impl Drop for BridgeSession {
    fn drop(&mut self) {
        self.task.abort();
        wire::remove_session_files(&self.directory);
    }
}

async fn serve(listener: UnixListener, token: String, sender: mpsc::Sender<PendingRequest>) {
    // JoinSet aborts accepted connections when the session is dropped too.
    let mut connections = tokio::task::JoinSet::new();
    let token = Arc::new(token);
    loop {
        tokio::select! {
            accepted = listener.accept(), if connections.len() < 16 => {
                let Ok((stream, _)) = accepted else { break };
                let (token, sender) = (token.clone(), sender.clone());
                connections.spawn(async move {
                    let _ = tokio::time::timeout(TIMEOUT, serve_connection(stream, &token, sender)).await;
                });
            }
            _ = connections.join_next(), if !connections.is_empty() => {}
            _ = sender.closed() => break,
        }
    }
}

async fn serve_connection(
    mut stream: UnixStream,
    token: &str,
    sender: mpsc::Sender<PendingRequest>,
) -> io::Result<()> {
    let bytes = wire::read_frame(&mut stream, MAX_REQUEST_BYTES).await?;
    let envelope: Envelope = serde_json::from_slice(&bytes).map_err(io::Error::other)?;
    if !wire::same_token(&envelope.token, token) {
        return wire::write_frame(
            &mut stream,
            &BridgeResponse::Error("Workspace connection is not authorized.".into()),
            MAX_RESPONSE_BYTES,
        )
        .await;
    }
    let (respond, response) = oneshot::channel();
    let pending = PendingRequest {
        request: envelope.request,
        respond,
    };
    if sender.try_send(pending).is_err() {
        return wire::write_frame(
            &mut stream,
            &BridgeResponse::Error("Workspace is busy or disconnected.".into()),
            MAX_RESPONSE_BYTES,
        )
        .await;
    }
    let response = response.await.map_err(io::Error::other)?;
    wire::write_frame(&mut stream, &response, MAX_RESPONSE_BYTES).await
}

/// The file is produced only by an explicit app action; it is not discovered
/// automatically, and contains no reusable provider credentials.
#[derive(Clone)]
pub struct BridgeClient {
    connection: Connection,
}

impl BridgeClient {
    pub fn from_file(path: &Path) -> io::Result<Self> {
        Ok(Self {
            connection: wire::read_connection(path)?,
        })
    }

    pub async fn call(&self, request: BridgeRequest) -> io::Result<BridgeResponse> {
        tokio::time::timeout(TIMEOUT, async {
            let mut stream = UnixStream::connect(&self.connection.socket).await?;
            wire::write_frame(
                &mut stream,
                &Envelope {
                    token: self.connection.token.clone(),
                    request,
                },
                MAX_REQUEST_BYTES,
            )
            .await?;
            let bytes = wire::read_frame(&mut stream, MAX_RESPONSE_BYTES).await?;
            serde_json::from_slice(&bytes).map_err(io::Error::other)
        })
        .await
        .map_err(|_| {
            io::Error::new(
                io::ErrorKind::TimedOut,
                "Workspace request timed out; do not assume a proposal was saved.",
            )
        })?
    }
}

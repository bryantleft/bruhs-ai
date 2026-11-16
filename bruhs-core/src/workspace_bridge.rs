//! Opt-in, session-scoped transports to the running workspace. They cannot
//! grant permissions or execute operations: the app owns those decisions.
//!
//! The native app listens on a private Unix socket ([`BridgeSession`]). A web
//! workspace answers through bruhs-server's relay
//! (`bruhs_models::workspace_bridge::relay`); its private connection file is
//! a [`RelayConnectionFile`]. [`BridgeClient`] reads either file.

mod relay;
#[cfg(test)]
mod tests;
mod wire;
pub use bruhs_models::workspace_bridge::{
    BridgeRequest, BridgeResponse, FinanceReadSection, HealthReadSection, MeetingReadSection,
    PersonReadSection, ProposalError, WorkflowProposal, MAX_REQUEST_BYTES, MAX_RESPONSE_BYTES,
};
pub use relay::{new_secret, same_secret, RelayConnectionFile};

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
        let token = new_secret()?;
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
    transport: Transport,
}

#[derive(Clone)]
enum Transport {
    /// The native app's private socket.
    Socket(Connection),
    /// bruhs-server's relay to the web tab that enabled the session.
    Relay(relay::Client),
}

impl BridgeClient {
    pub fn from_file(path: &Path) -> io::Result<Self> {
        let bytes = wire::read_private_file(path)?;
        let transport = if let Ok(connection) = serde_json::from_slice::<Connection>(&bytes) {
            if connection.token.len() != 64 || !connection.socket.is_absolute() {
                return Err(wire::invalid_connection());
            }
            Transport::Socket(connection)
        } else {
            Transport::Relay(relay::Client::from_bytes(&bytes)?)
        };
        Ok(Self { transport })
    }

    pub async fn call(&self, request: BridgeRequest) -> io::Result<BridgeResponse> {
        match &self.transport {
            Transport::Socket(connection) => call_socket(connection, request).await,
            Transport::Relay(client) => client.call(request).await,
        }
    }
}

async fn call_socket(
    connection: &Connection,
    request: BridgeRequest,
) -> io::Result<BridgeResponse> {
    tokio::time::timeout(TIMEOUT, async {
        let mut stream = UnixStream::connect(&connection.socket).await?;
        wire::write_frame(
            &mut stream,
            &Envelope {
                token: connection.token.clone(),
                request,
            },
            MAX_REQUEST_BYTES,
        )
        .await?;
        let bytes = wire::read_frame(&mut stream, MAX_RESPONSE_BYTES).await?;
        serde_json::from_slice(&bytes).map_err(io::Error::other)
    })
    .await
    .map_err(|_| timed_out())?
}

fn timed_out() -> io::Error {
    io::Error::new(
        io::ErrorKind::TimedOut,
        "Workspace request timed out; do not assume a proposal was saved.",
    )
}

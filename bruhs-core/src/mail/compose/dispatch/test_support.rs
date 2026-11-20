//! The fake mail transport and a connected mailbox's cache, shared by the
//! dispatch tests, the apps tests and bruhs-server's tests (ruling F15).
//! Nothing here reaches a provider: the connection is synthetic and the
//! transport records what it was handed.

use super::*;
use crate::{
    connections::Connections,
    mail::{MailboxId, ProviderAcceptance},
};
use bruhs_models::connections::ServiceProvider;
use std::{
    sync::{
        atomic::{AtomicUsize, Ordering},
        Mutex,
    },
    time::Duration,
};
use tokio::sync::Notify;

/// The connection id `Connections::test_connected` saves.
pub const CONNECTION_ID: &str = "synthetic-connection";

/// The sender address the fake transport verifies.
pub fn sender_address() -> MailAddress {
    MailAddress {
        name: String::new(),
        address: "sender@example.test".into(),
    }
}

/// A mail cache in its own folder, bound to a synthetic Gmail connection.
pub struct ConnectedMail {
    pub runtime: MailRuntime,
    pub connections: Connections,
    pub mailbox: MailboxId,
    pub folder: tempfile::TempDir,
}

impl ConnectedMail {
    pub async fn gmail() -> Self {
        let folder = tempfile::tempdir().expect("a test folder");
        let runtime = MailRuntime::open(folder.path().join("mail.sqlite3"))
            .await
            .expect("open the test mail cache");
        let connections =
            Connections::test_connected(ServiceProvider::Gmail, "synthetic-token").await;
        let session = runtime
            .session(&connections, ServiceProvider::Gmail, CONNECTION_ID)
            .await
            .expect("bind the synthetic mailbox");
        let mailbox = session.authorization().mailbox().clone();
        drop(session);
        Self {
            runtime,
            connections,
            mailbox,
            folder,
        }
    }

    pub async fn session(&self) -> MailSession {
        self.runtime
            .session(&self.connections, ServiceProvider::Gmail, CONNECTION_ID)
            .await
            .expect("a synthetic mail session")
    }

    /// Every send journal row as `(draft_id, state)`, read from the file.
    pub fn send_operations(&self) -> Vec<(String, String)> {
        let connection = rusqlite::Connection::open(self.folder.path().join("mail.sqlite3"))
            .expect("read the test mail cache");
        let mut statement = connection
            .prepare("SELECT draft_id,state FROM send_operations ORDER BY rowid")
            .expect("read the send journal");
        statement
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .expect("read the send journal")
            .collect::<Result<_, _>>()
            .expect("read the send journal")
    }
}

#[derive(Clone, Copy)]
pub enum Mode {
    Accepted,
    /// Rate limited, retry after 3 s.
    Rejected,
    /// Refused with this problem.
    Refused(MailSendProblemKind),
    Uncertain,
    Panic,
}

/// A Gmail transport that sends nothing: it counts and keeps each
/// dispatch's MIME, then answers as its [`Mode`] says.
pub struct FakeTransport {
    pub mode: Mode,
    pub count: AtomicUsize,
    pub bytes: Mutex<Vec<Vec<u8>>>,
    pub entered: Notify,
    pub release: Option<Notify>,
    /// How long verifying the sender takes.
    pub profile_delay: Option<Duration>,
    /// How long a send takes after it is recorded.
    pub send_delay: Option<Duration>,
}

impl FakeTransport {
    /// `waiting` holds each send until `release` is notified.
    pub fn new(mode: Mode, waiting: bool) -> Arc<Self> {
        Arc::new(Self::with_mode(mode, waiting))
    }

    /// A send that answers `mode` after `delay`.
    pub fn slow_send(mode: Mode, delay: Duration) -> Arc<Self> {
        Arc::new(Self {
            send_delay: Some(delay),
            ..Self::with_mode(mode, false)
        })
    }

    /// A sender check that takes `delay`, then sends as `mode`.
    pub fn slow_profile(mode: Mode, delay: Duration) -> Arc<Self> {
        Arc::new(Self {
            profile_delay: Some(delay),
            ..Self::with_mode(mode, false)
        })
    }

    fn with_mode(mode: Mode, waiting: bool) -> Self {
        Self {
            mode,
            count: AtomicUsize::new(0),
            bytes: Mutex::new(Vec::new()),
            entered: Notify::new(),
            release: waiting.then(Notify::new),
            profile_delay: None,
            send_delay: None,
        }
    }

    /// How many dispatches reached the transport.
    pub fn sends(&self) -> usize {
        self.count.load(Ordering::SeqCst)
    }

    /// The MIME of each dispatch, in order.
    pub fn sent(&self) -> Vec<Vec<u8>> {
        self.bytes.lock().expect("the sent list").clone()
    }
}

#[async_trait::async_trait]
impl MailSendTransport for FakeTransport {
    fn provider(&self) -> ServiceProvider {
        ServiceProvider::Gmail
    }
    fn mime_limit_bytes(&self) -> usize {
        MAIL_PREPARED_MIME_MAX_BYTES
    }
    async fn verified_sender(
        &self,
        session: &MailSession,
    ) -> Result<VerifiedMailSender, MailSendProblem> {
        if let Some(delay) = self.profile_delay {
            tokio::time::sleep(delay).await;
        }
        VerifiedMailSender::from_profile(session, sender_address())
            .map_err(|_| MailSendProblem::new(MailSendProblemKind::AccountChanged))
    }
    async fn send(&self, session: &MailSession, dispatch: MailDispatch) -> ProviderSendOutcome {
        dispatch
            .validate_session(session)
            .expect("the dispatch's own session");
        self.count.fetch_add(1, Ordering::SeqCst);
        self.bytes
            .lock()
            .expect("the sent list")
            .push(dispatch.mime().bytes().to_vec());
        self.entered.notify_one();
        if let Some(release) = &self.release {
            release.notified().await;
        }
        if let Some(delay) = self.send_delay {
            tokio::time::sleep(delay).await;
        }
        match self.mode {
            Mode::Accepted => ProviderSendOutcome::Accepted(ProviderAcceptance {
                message_id: None,
                thread_id: None,
                request_id: None,
            }),
            Mode::Rejected => ProviderSendOutcome::Rejected(MailSendProblem {
                kind: MailSendProblemKind::RateLimited,
                retry_after_seconds: Some(3),
            }),
            Mode::Refused(kind) => ProviderSendOutcome::Rejected(MailSendProblem::new(kind)),
            Mode::Uncertain => ProviderSendOutcome::Uncertain(MailSendProblem::new(
                MailSendProblemKind::Unconfirmed,
            )),
            Mode::Panic => panic!("synthetic adapter panic"),
        }
    }
}

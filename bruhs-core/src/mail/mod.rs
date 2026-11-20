//! Process-owned connected mail. Local imports remain in their existing stores.
//!
//! SQLite work runs on blocking workers. A session retains current broker access
//! and its connection lease; matching cache identifiers alone never grant access.
//! The send journal records provider acceptance separately from delivery.

mod cache;
pub mod compose;
pub mod device;
pub mod gmail;
pub mod outlook;
mod ownership;
#[cfg(test)]
mod tests;

pub use cache::draft_attachments::{
    DraftAttachment, DraftAttachmentBytes, DraftAttachmentError, DraftAttachmentUpload,
    MAX_DRAFT_ATTACHMENTS, MAX_DRAFT_ATTACHMENT_BYTES,
};
pub use cache::identity::{
    MailAuthorizationRef, MailCacheHostId, MailDraftRef, MailIdentityError, MailboxId,
    RemoteMessageRef, RemoteThreadRef,
};
pub use cache::new_mail::NewMailThread;
pub use cache::send_journal::{
    DispatchClaim, OperationError, ProviderAcceptance, SendOperation, SendReview, SendState,
};
pub use cache::{CacheError, CursorScope, MailDraftRecord, SyncBatch, ThreadWrite};

use crate::connections::{ConnectionAccess, ConnectionError, Connections};
use bruhs_models::mail::{
    ConnectedMailThread, MailBrowseQuery, MailThreadsPage, MailboxesSnapshot,
};
use bruhs_models::{connections::ServiceProvider, EpochMs};
use cache::MailCache;
use std::{path::Path, sync::Arc};
use tokio::sync::{watch, Mutex, OwnedMutexGuard};

#[derive(Debug, thiserror::Error)]
pub enum MailError {
    #[error(transparent)]
    Cache(#[from] CacheError),
    #[error(transparent)]
    Operation(#[from] OperationError),
    #[error(transparent)]
    DraftAttachment(#[from] DraftAttachmentError),
    #[error(transparent)]
    Authorization(#[from] ConnectionError),
    #[error("Another mail runtime owns this cache. Stop that server before opening it here.")]
    AlreadyOwned,
    #[error("The private mail cache could not be opened. Check local storage and retry.")]
    Storage,
    #[error("The mail storage worker stopped. Refresh saved state before retrying.")]
    Worker,
    #[error("The mail runtime is already attached to this process.")]
    AlreadyAttached,
    #[error("Choose a nonempty mail database path.")]
    InvalidPath,
}

#[derive(Clone)]
pub struct MailRuntime {
    cache: Arc<Mutex<MailCache>>,
    // Drop the connection before releasing exclusive ownership.
    owner: Arc<ownership::RuntimeLock>,
    host: MailCacheHostId,
    changes: watch::Sender<u64>,
}

struct ActiveCall {
    cache: OwnedMutexGuard<MailCache>,
    // A cancelled caller must not release ownership during its blocking write.
    _owner: Arc<ownership::RuntimeLock>,
}

/// A nonportable authorization capability. No token or raw account subject is
/// exposed by this type; dropping it releases the broker's connection lease.
pub struct MailSession {
    runtime: MailRuntime,
    authorization: MailAuthorizationRef,
    _access: ConnectionAccess,
}

impl MailRuntime {
    /// Explicit startup only: constructing AppState never opens a mail cache,
    /// touches the keychain, starts sync, or dispatches queued work.
    pub async fn open(path: impl AsRef<Path>) -> Result<Self, MailError> {
        let path = path.as_ref().to_owned();
        let (cache, owner) = tokio::task::spawn_blocking(move || {
            let (path, owner) = ownership::acquire(&path)?;
            let host = MailCacheHostId::from_host_record(&uuid::Uuid::new_v4().to_string())
                .map_err(CacheError::from)?;
            let mut cache = MailCache::open(&path, &host)?;
            cache.recover_interrupted_dispatches(EpochMs::now())?;
            Ok::<_, MailError>((cache, owner))
        })
        .await
        .map_err(|_| MailError::Worker)??;
        let host = cache.host().clone();
        let (changes, _) = watch::channel(0);
        Ok(Self {
            cache: Arc::new(Mutex::new(cache)),
            owner: Arc::new(owner),
            host,
            changes,
        })
    }

    pub fn default_path() -> Result<std::path::PathBuf, MailError> {
        if let Some(path) = std::env::var_os("BRUHS_MAIL_DB") {
            return if path.is_empty() {
                Err(MailError::InvalidPath)
            } else {
                Ok(path.into())
            };
        }
        directories::ProjectDirs::from("dev", "bruhs", "bruhs-ai")
            .map(|dirs| dirs.data_local_dir().join("server/mail.sqlite3"))
            .ok_or(MailError::Storage)
    }

    pub fn host(&self) -> &MailCacheHostId {
        &self.host
    }

    /// Wake hints only, not durable sync revisions. Consumers reread the cache.
    pub fn subscribe(&self) -> watch::Receiver<u64> {
        self.changes.subscribe()
    }

    pub async fn mailboxes(&self) -> Result<MailboxesSnapshot, MailError> {
        let mailboxes = self.call(false, |cache| Ok(cache.mailboxes()?)).await?;
        Ok(MailboxesSnapshot {
            host_id: self.host.as_str().to_owned(),
            mailboxes,
        })
    }

    pub async fn browse(
        &self,
        mailbox_id: String,
        query: MailBrowseQuery,
    ) -> Result<MailThreadsPage, MailError> {
        self.call(false, move |cache| Ok(cache.browse(&mailbox_id, &query)?))
            .await
    }

    pub async fn connected_thread(
        &self,
        host: String,
        mailbox: String,
        external_id: String,
    ) -> Result<Option<ConnectedMailThread>, MailError> {
        self.call(false, move |cache| {
            cache
                .connected_thread(&host, &mailbox, &external_id)?
                .map(|payload| {
                    serde_json::from_str(&payload).map_err(|_| CacheError::CorruptRecord.into())
                })
                .transpose()
        })
        .await
    }

    pub async fn session(
        &self,
        connections: &Connections,
        provider: ServiceProvider,
        connection_id: &str,
    ) -> Result<MailSession, MailError> {
        if !matches!(provider, ServiceProvider::Gmail | ServiceProvider::Outlook) {
            return Err(CacheError::Identity(MailIdentityError::NotMailProvider).into());
        }
        let access = connections.access(provider, connection_id).await?;
        let authorization = access
            .mail_authorization
            .clone()
            .ok_or(ConnectionError::IdentityUnavailable)?;
        let binding = authorization.clone();
        self.call(true, move |cache| Ok(cache.bind(&binding)?))
            .await?;
        Ok(MailSession {
            runtime: self.clone(),
            authorization,
            _access: access,
        })
    }

    /// Up to `limit` of `mailbox`'s unread, unarchived threads whose newest
    /// message came in after `after_ms`, oldest first (D20, ruling F28).
    pub async fn unread_since(
        &self,
        mailbox: &str,
        after_ms: i64,
        limit: usize,
    ) -> Result<Vec<NewMailThread>, MailError> {
        let mailbox = mailbox.to_owned();
        self.call(false, move |cache| {
            Ok(cache.unread_since(&mailbox, after_ms, limit)?)
        })
        .await
    }

    /// The newest `message_at_ms` among `mailbox`'s unread, unarchived
    /// threads, or `None` when it has none.
    pub async fn newest_unread_ms(&self, mailbox: &str) -> Result<Option<i64>, MailError> {
        let mailbox = mailbox.to_owned();
        self.call(false, move |cache| Ok(cache.newest_unread_ms(&mailbox)?))
            .await
    }

    pub async fn cached_thread(
        &self,
        reference: RemoteThreadRef,
    ) -> Result<Option<String>, MailError> {
        self.call(false, move |cache| Ok(cache.thread(&reference)?))
            .await
    }

    pub async fn cached_cursor(
        &self,
        mailbox: MailboxId,
        scope: CursorScope,
    ) -> Result<Option<String>, MailError> {
        self.call(false, move |cache| Ok(cache.cursor(&mailbox, &scope)?))
            .await
    }

    pub async fn cached_draft(
        &self,
        reference: MailDraftRef,
    ) -> Result<Option<(i64, String)>, MailError> {
        self.call(false, move |cache| Ok(cache.mail_draft(&reference)?))
            .await
    }

    /// Editing a local draft requires its owning cache, not provider access.
    /// Reviewing or dispatching it still requires a current MailSession lease.
    pub async fn save_cached_draft(
        &self,
        expected_host: String,
        reference: MailDraftRef,
        revision: i64,
        payload: String,
    ) -> Result<i64, MailError> {
        if expected_host != self.host.as_str() {
            return Err(CacheError::HostChanged.into());
        }
        self.call(true, move |cache| {
            Ok(cache.save_mail_draft(&reference, revision, &payload)?)
        })
        .await
    }

    /// Storage recovery only. The UI pages metadata rather than mounting bodies.
    pub async fn cached_drafts(
        &self,
        mailbox: MailboxId,
    ) -> Result<Vec<MailDraftRecord>, MailError> {
        self.call(false, move |cache| Ok(cache.drafts(&mailbox)?))
            .await
    }

    /// Local draft storage does not authorize a provider operation. Checking the
    /// selected host prevents a client's pending upload from changing servers.
    pub async fn store_draft_attachment(
        &self,
        expected_host: String,
        reference: MailDraftRef,
        upload: DraftAttachmentUpload,
    ) -> Result<DraftAttachment, MailError> {
        if expected_host != self.host.as_str() {
            return Err(CacheError::HostChanged.into());
        }
        self.call(true, move |cache| {
            Ok(cache.store_draft_attachment(&reference, upload)?)
        })
        .await
    }

    pub async fn draft_attachments(
        &self,
        expected_host: String,
        reference: MailDraftRef,
    ) -> Result<Vec<DraftAttachment>, MailError> {
        if expected_host != self.host.as_str() {
            return Err(CacheError::HostChanged.into());
        }
        self.call(false, move |cache| Ok(cache.draft_attachments(&reference)?))
            .await
    }

    pub async fn draft_attachment(
        &self,
        expected_host: String,
        reference: MailDraftRef,
        id: String,
    ) -> Result<Option<DraftAttachmentBytes>, MailError> {
        if expected_host != self.host.as_str() {
            return Err(CacheError::HostChanged.into());
        }
        self.call(false, move |cache| {
            Ok(cache.draft_attachment(&reference, &id)?)
        })
        .await
    }

    pub async fn remove_draft_attachment(
        &self,
        expected_host: String,
        reference: MailDraftRef,
        id: String,
    ) -> Result<bool, MailError> {
        if expected_host != self.host.as_str() {
            return Err(CacheError::HostChanged.into());
        }
        self.call(true, move |cache| {
            Ok(cache.remove_draft_attachment(&reference, &id)?)
        })
        .await
    }

    pub async fn send_operation(
        &self,
        id: String,
        expected_host: MailCacheHostId,
        mailbox: MailboxId,
    ) -> Result<Option<SendOperation>, MailError> {
        self.call(false, move |cache| {
            Ok(cache.send_operation(&id, &expected_host, &mailbox)?)
        })
        .await
    }

    pub async fn cancel_send(
        &self,
        id: String,
        expected_host: MailCacheHostId,
        mailbox: MailboxId,
    ) -> Result<(), MailError> {
        self.call(true, move |cache| {
            Ok(cache.cancel_queued_send(&id, &expected_host, &mailbox, EpochMs::now())?)
        })
        .await
    }

    async fn call<T: Send + 'static>(
        &self,
        changed: bool,
        operation: impl FnOnce(&mut MailCache) -> Result<T, MailError> + Send + 'static,
    ) -> Result<T, MailError> {
        let mut work = ActiveCall {
            cache: self.cache.clone().lock_owned().await,
            _owner: Arc::clone(&self.owner),
        };
        let changes = self.changes.clone();
        tokio::task::spawn_blocking(move || {
            let result = operation(&mut work.cache);
            if changed && result.is_ok() {
                changes.send_modify(|revision| *revision = revision.saturating_add(1));
            }
            drop(work);
            result
        })
        .await
        .map_err(|_| MailError::Worker)?
    }
}

impl MailSession {
    pub fn authorization(&self) -> &MailAuthorizationRef {
        &self.authorization
    }

    pub async fn revision(&self) -> Result<i64, MailError> {
        let auth = self.authorization.clone();
        self.runtime
            .call(false, move |cache| Ok(cache.bind(&auth)?))
            .await
    }

    pub async fn sync_checkpoint(
        &self,
        scope: CursorScope,
    ) -> Result<(i64, Option<String>), MailError> {
        let auth = self.authorization.clone();
        self.runtime
            .call(
                false,
                move |cache| Ok(cache.sync_checkpoint(&auth, &scope)?),
            )
            .await
    }

    pub async fn apply_full_sync_page(
        &self,
        revision: i64,
        batch: SyncBatch,
        run_id: String,
        complete: bool,
    ) -> Result<i64, MailError> {
        let auth = self.authorization.clone();
        self.runtime
            .call(true, move |cache| {
                Ok(cache.apply_full_sync_page(&auth, revision, &batch, &run_id, complete)?)
            })
            .await
    }

    pub async fn apply_sync(&self, revision: i64, batch: SyncBatch) -> Result<i64, MailError> {
        let auth = self.authorization.clone();
        self.runtime
            .call(true, move |cache| {
                Ok(cache.apply_sync(&auth, revision, &batch)?)
            })
            .await
    }

    pub async fn save_draft(
        &self,
        reference: MailDraftRef,
        revision: i64,
        payload: String,
    ) -> Result<i64, MailError> {
        if reference.mailbox != *self.authorization.mailbox() {
            return Err(CacheError::Identity(MailIdentityError::MailboxChanged).into());
        }
        self.runtime
            .call(true, move |cache| {
                Ok(cache.save_mail_draft(&reference, revision, &payload)?)
            })
            .await
    }

    pub async fn review_send(
        &self,
        draft: MailDraftRef,
        revision: i64,
    ) -> Result<SendReview, MailError> {
        let auth = self.authorization.clone();
        self.runtime
            .call(false, move |cache| {
                Ok(cache.review_send(&auth, &draft, revision)?)
            })
            .await
    }

    pub async fn queue_send(
        &self,
        id: String,
        review: SendReview,
    ) -> Result<SendOperation, MailError> {
        let auth = self.authorization.clone();
        self.runtime
            .call(true, move |cache| {
                Ok(cache.queue_send(&id, &review, &auth, EpochMs::now())?)
            })
            .await
    }

    /// Provider adapters call this only after typed recipient/MIME/attachment
    /// preflight. A Start result is a durable dispatch intent, not a send receipt.
    /// Retain this session throughout the provider attempt and its receipt save.
    pub async fn claim_send(&self, id: String) -> Result<DispatchClaim, MailError> {
        let auth = self.authorization.clone();
        self.runtime
            .call(true, move |cache| {
                Ok(cache.claim_send(&id, &auth, EpochMs::now())?)
            })
            .await
    }

    pub async fn checkpoint_provider_draft(
        &self,
        id: String,
        provider_id: String,
    ) -> Result<(), MailError> {
        let auth = self.authorization.clone();
        self.runtime
            .call(true, move |cache| {
                Ok(cache.checkpoint_provider_draft(&id, &auth, &provider_id, EpochMs::now())?)
            })
            .await
    }

    pub async fn record_acceptance(
        &self,
        id: String,
        receipt: ProviderAcceptance,
    ) -> Result<SendOperation, MailError> {
        let auth = self.authorization.clone();
        self.runtime
            .call(true, move |cache| {
                Ok(cache.record_send_acceptance(&id, &auth, &receipt, EpochMs::now())?)
            })
            .await
    }

    /// Timeouts, disconnected transports and ambiguous server errors do not
    /// establish rejection; those attempts need provider reconciliation.
    pub async fn record_definite_rejection(&self, id: String) -> Result<(), MailError> {
        let auth = self.authorization.clone();
        self.runtime
            .call(true, move |cache| {
                Ok(cache.record_definite_send_rejection(&id, &auth, EpochMs::now())?)
            })
            .await
    }
}

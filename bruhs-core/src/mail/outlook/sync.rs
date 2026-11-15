use super::{
    wire, MailSession, MailboxContext, OutlookClient, OutlookError, CONCURRENCY, MAX_BYTES,
};
use crate::mail::{CursorScope, SyncBatch, ThreadWrite};
use futures_util::{stream, StreamExt, TryStreamExt};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Serialize)]
pub struct SyncProgress {
    pub revision: i64,
    pub threads_updated: usize,
    pub full_scan_complete: bool,
    pub caught_up: bool,
}

#[derive(Default)]
struct PageStatus {
    full_scan_complete: bool,
    caught_up: bool,
}

#[derive(Clone, Serialize, Deserialize)]
struct FolderCursor {
    id: String,
    cursor: Option<String>,
}

#[derive(Clone, Serialize, Deserialize)]
struct FolderListing {
    parent: Option<String>,
    next: Option<String>,
    seen: BTreeSet<String>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "phase", rename_all = "snake_case")]
enum Phase {
    Discover {
        pending: Vec<FolderListing>,
        discovered: BTreeSet<String>,
        require_full: bool,
    },
    Anchor {
        index: usize,
        seen: BTreeSet<String>,
    },
    Full {
        run_id: String,
        next: Option<String>,
        seen: BTreeSet<String>,
    },
    Delta {
        index: usize,
        seen: BTreeSet<String>,
    },
}

#[derive(Clone, Serialize, Deserialize)]
struct Checkpoint {
    generation: String,
    context: MailboxContext,
    folders: Vec<FolderCursor>,
    state: Phase,
}

impl Checkpoint {
    fn discovery(&mut self, require_full: bool) {
        self.state = Phase::Discover {
            pending: vec![FolderListing {
                parent: None,
                next: None,
                seen: BTreeSet::new(),
            }],
            discovered: BTreeSet::new(),
            require_full,
        };
    }

    fn full(&mut self) {
        self.state = Phase::Full {
            run_id: uuid::Uuid::new_v4().to_string(),
            next: None,
            seen: BTreeSet::new(),
        };
    }

    fn valid(&self) -> bool {
        let mut ids = BTreeSet::new();
        !self.generation.is_empty()
            && !self.context.account.is_empty()
            && !self.context.inbox.is_empty()
            && !self.context.sent.is_empty()
            && self.folders.iter().all(|folder| {
                !folder.id.is_empty()
                    && ids.insert(&folder.id)
                    && folder.cursor.as_ref().is_none_or(|value| !value.is_empty())
            })
            && match &self.state {
                Phase::Discover { pending, .. } => {
                    !pending.is_empty()
                        && pending
                            .iter()
                            .all(|listing| listing.parent.as_ref().is_none_or(|id| !id.is_empty()))
                }
                Phase::Full { run_id, .. } => !run_id.is_empty(),
                Phase::Anchor { index, .. } | Phase::Delta { index, .. } => {
                    *index < self.folders.len()
                }
            }
    }
}

fn scope() -> CursorScope {
    CursorScope::OutlookMailbox
}

impl OutlookClient {
    async fn begin(&self, session: &MailSession) -> Result<Checkpoint, OutlookError> {
        let context = self.context(session).await?;
        let mut checkpoint = Checkpoint {
            generation: session.authorization().connection_id().into(),
            context,
            folders: vec![],
            state: Phase::Discover {
                pending: vec![],
                discovered: BTreeSet::new(),
                require_full: true,
            },
        };
        checkpoint.discovery(true);
        Ok(checkpoint)
    }

    pub async fn sync_page(&self, session: &MailSession) -> Result<SyncProgress, OutlookError> {
        if session.authorization().mailbox().provider()
            != bruhs_models::connections::ServiceProvider::Outlook
        {
            return Err(OutlookError::WrongProvider);
        }
        let (revision, saved) = session.sync_checkpoint(scope()).await?;
        let checkpoint = saved
            .as_deref()
            .map(serde_json::from_str::<Checkpoint>)
            .transpose()
            .map_err(|_| OutlookError::InvalidCheckpoint)?;
        if checkpoint
            .as_ref()
            .is_some_and(|checkpoint| !checkpoint.valid())
        {
            return Err(OutlookError::InvalidCheckpoint);
        }
        let checkpoint = match checkpoint {
            Some(checkpoint)
                if checkpoint.generation == session.authorization().connection_id() =>
            {
                checkpoint
            }
            _ => self.begin(session).await?,
        };
        self.page(session, revision, checkpoint).await
    }

    /// Explicit restart retains rows/drafts until a new account scan completes.
    pub async fn restart_sync_page(
        &self,
        session: &MailSession,
    ) -> Result<SyncProgress, OutlookError> {
        if session.authorization().mailbox().provider()
            != bruhs_models::connections::ServiceProvider::Outlook
        {
            return Err(OutlookError::WrongProvider);
        }
        let (revision, _) = session.sync_checkpoint(scope()).await?;
        self.page(session, revision, self.begin(session).await?)
            .await
    }

    async fn page(
        &self,
        session: &MailSession,
        revision: i64,
        checkpoint: Checkpoint,
    ) -> Result<SyncProgress, OutlookError> {
        let result = match checkpoint.state.clone() {
            Phase::Discover { .. } => self.discover(session, revision, checkpoint).await,
            Phase::Anchor { .. } => self.delta(session, revision, checkpoint, true).await,
            Phase::Full { .. } => self.full_scan(session, revision, checkpoint).await,
            Phase::Delta { .. } => self.delta(session, revision, checkpoint, false).await,
        };
        match result {
            // Expiration/deleted folders do not establish deletion of cached mail.
            // Restarting anchors avoids replaying unusable provider state.
            Err(OutlookError::SyncExpired) => {
                self.commit(
                    session,
                    revision,
                    self.begin(session).await?,
                    vec![],
                    None,
                    PageStatus::default(),
                )
                .await
            }
            result => result,
        }
    }

    async fn commit(
        &self,
        session: &MailSession,
        revision: i64,
        checkpoint: Checkpoint,
        upserts: Vec<ThreadWrite>,
        full: Option<(String, bool)>,
        status: PageStatus,
    ) -> Result<SyncProgress, OutlookError> {
        let value =
            serde_json::to_string(&checkpoint).map_err(|_| OutlookError::InvalidCheckpoint)?;
        if value.len() > MAX_BYTES {
            return Err(OutlookError::TooLarge);
        }
        let threads_updated = upserts.len();
        let batch = SyncBatch {
            upserts,
            confirmed_deleted: vec![],
            cursor: Some((scope(), value)),
        };
        let revision = if let Some((run, complete)) = full {
            session
                .apply_full_sync_page(revision, batch, run, complete)
                .await?
        } else {
            session.apply_sync(revision, batch).await?
        };
        Ok(SyncProgress {
            revision,
            threads_updated,
            full_scan_complete: status.full_scan_complete,
            caught_up: status.caught_up,
        })
    }

    async fn discover(
        &self,
        session: &MailSession,
        revision: i64,
        mut checkpoint: Checkpoint,
    ) -> Result<SyncProgress, OutlookError> {
        let Phase::Discover {
            mut pending,
            mut discovered,
            require_full,
        } = checkpoint.state.clone()
        else {
            return Err(OutlookError::InvalidCheckpoint);
        };
        let mut current = pending
            .first()
            .cloned()
            .ok_or(OutlookError::InvalidCheckpoint)?;
        let segments = current.parent.as_deref().map_or_else(
            || vec!["mailFolders"],
            |id| vec!["mailFolders", id, "childFolders"],
        );
        let url = match &current.next {
            Some(next) => self.checked_link(next, &segments)?,
            None => self.queried(
                &segments,
                &[
                    ("includeHiddenFolders", "true"),
                    ("$select", "id,childFolderCount"),
                ],
            )?,
        };
        if !current.seen.insert(url.as_str().to_owned()) {
            return Err(OutlookError::InvalidResponse);
        }
        let page: wire::Page<wire::Folder> = self.get(session, url).await.map_err(|error| {
            if matches!(error, OutlookError::NotFound) {
                OutlookError::SyncExpired
            } else {
                error
            }
        })?;
        if page.delta.is_some() {
            return Err(OutlookError::InvalidResponse);
        }
        for folder in page.value {
            if folder.id.is_empty() {
                return Err(OutlookError::InvalidResponse);
            }
            if folder.kind.as_deref() == Some("#microsoft.graph.mailSearchFolder") {
                continue;
            }
            if folder
                .kind
                .as_deref()
                .is_some_and(|kind| kind != "#microsoft.graph.mailFolder")
            {
                return Err(OutlookError::InvalidResponse);
            }
            if discovered.insert(folder.id.clone()) && folder.child_folder_count > 0 {
                pending.push(FolderListing {
                    parent: Some(folder.id),
                    next: None,
                    seen: BTreeSet::new(),
                });
            }
        }
        if let Some(next) = page.next {
            self.checked_link(&next, &segments)?;
            if current.seen.contains(&next) {
                return Err(OutlookError::InvalidResponse);
            }
            current.next = Some(next);
            pending[0] = current;
        } else {
            pending.remove(0);
        }
        if pending.is_empty() {
            let old: BTreeSet<&str> = checkpoint
                .folders
                .iter()
                .map(|folder| folder.id.as_str())
                .collect();
            let changed = old != discovered.iter().map(String::as_str).collect();
            let mut retained: std::collections::BTreeMap<String, Option<String>> = checkpoint
                .folders
                .into_iter()
                .map(|folder| (folder.id, folder.cursor))
                .collect();
            checkpoint.folders = discovered
                .into_iter()
                .map(|id| FolderCursor {
                    cursor: retained.remove(&id).flatten(),
                    id,
                })
                .collect();
            if let Some(index) = checkpoint
                .folders
                .iter()
                .position(|folder| folder.cursor.is_none())
            {
                checkpoint.state = Phase::Anchor {
                    index,
                    seen: BTreeSet::new(),
                };
            } else if require_full || changed || checkpoint.folders.is_empty() {
                checkpoint.full();
            } else {
                checkpoint.state = Phase::Delta {
                    index: 0,
                    seen: BTreeSet::new(),
                };
            }
        } else {
            checkpoint.state = Phase::Discover {
                pending,
                discovered,
                require_full,
            };
        }
        self.commit(
            session,
            revision,
            checkpoint,
            vec![],
            None,
            PageStatus::default(),
        )
        .await
    }

    async fn delta(
        &self,
        session: &MailSession,
        revision: i64,
        mut checkpoint: Checkpoint,
        anchoring: bool,
    ) -> Result<SyncProgress, OutlookError> {
        let (index, mut seen) = match checkpoint.state.clone() {
            Phase::Anchor { index, seen } | Phase::Delta { index, seen } => (index, seen),
            _ => return Err(OutlookError::InvalidCheckpoint),
        };
        let folder = checkpoint
            .folders
            .get(index)
            .ok_or(OutlookError::InvalidCheckpoint)?;
        let segments = ["mailFolders", folder.id.as_str(), "messages", "delta"];
        let url = match &folder.cursor {
            Some(cursor) => self.checked_link(cursor, &segments)?,
            None if anchoring => self.queried(&segments, &[("$select", "id,conversationId")])?,
            None => return Err(OutlookError::InvalidCheckpoint),
        };
        if !seen.insert(url.as_str().to_owned()) {
            return Err(OutlookError::InvalidResponse);
        }
        let page: wire::Page<wire::Change> = self.get(session, url).await.map_err(|error| {
            if matches!(error, OutlookError::NotFound) {
                OutlookError::SyncExpired
            } else {
                error
            }
        })?;
        if page.next.is_some() == page.delta.is_some()
            || page.value.iter().any(|change| change.id.is_empty())
        {
            return Err(OutlookError::InvalidResponse);
        }
        let complete = page.next.is_none();
        let cursor = page
            .next
            .or(page.delta)
            .ok_or(OutlookError::InvalidResponse)?;
        self.checked_link(&cursor, &segments)?;
        if !complete && seen.contains(&cursor) {
            return Err(OutlookError::InvalidResponse);
        }
        checkpoint.folders[index].cursor = Some(cursor);
        let mut upserts = Vec::new();
        let mut needs_scan = page.value.iter().any(|change| change.removed.is_some());
        if !anchoring && !needs_scan {
            let mut ids = BTreeSet::new();
            // Partial delta records are resolved through the immutable message id.
            let changes: Vec<Option<wire::Change>> = stream::iter(page.value)
                .map(|change| async move {
                    if change
                        .conversation_id
                        .as_deref()
                        .is_some_and(|id| !id.is_empty())
                    {
                        return Ok(Some(change));
                    }
                    let url = self.queried(
                        &["messages", &change.id],
                        &[("$select", "id,conversationId")],
                    )?;
                    match self.get::<wire::Change>(session, url).await {
                        Ok(current) if current.id == change.id => Ok(Some(current)),
                        Ok(_) => Err(OutlookError::InvalidResponse),
                        Err(OutlookError::NotFound) => Ok(None),
                        Err(error) => Err(error),
                    }
                })
                .buffer_unordered(CONCURRENCY)
                .try_collect()
                .await?;
            // A disappearance while resolving a page is handled by reconciliation,
            // not by treating the remaining changes as a complete deletion proof.
            for change in changes {
                match change.and_then(|change| change.conversation_id) {
                    Some(id) if !id.is_empty() => {
                        ids.insert(id);
                    }
                    _ => {
                        needs_scan = true;
                    }
                }
            }
            let (writes, missing) = self.writes(session, ids, &checkpoint.context).await?;
            upserts = writes;
            needs_scan |= missing;
        }
        if !anchoring && needs_scan {
            // The offending cursor is already advanced in this same durable state.
            // Retrying after a scan cannot replay the same tombstone forever.
            checkpoint.full();
            return self
                .commit(
                    session,
                    revision,
                    checkpoint,
                    vec![],
                    None,
                    PageStatus::default(),
                )
                .await;
        }
        let mut caught_up = false;
        if !complete {
            checkpoint.state = if anchoring {
                Phase::Anchor { index, seen }
            } else {
                Phase::Delta { index, seen }
            };
        } else if anchoring {
            if let Some(index) = checkpoint
                .folders
                .iter()
                .position(|folder| folder.cursor.is_none())
            {
                checkpoint.state = Phase::Anchor {
                    index,
                    seen: BTreeSet::new(),
                };
            } else {
                checkpoint.full();
            }
        } else if index + 1 < checkpoint.folders.len() {
            checkpoint.state = Phase::Delta {
                index: index + 1,
                seen: BTreeSet::new(),
            };
        } else {
            caught_up = true;
            checkpoint.discovery(false);
        }
        self.commit(
            session,
            revision,
            checkpoint,
            upserts,
            None,
            PageStatus {
                full_scan_complete: !anchoring,
                caught_up,
            },
        )
        .await
    }

    async fn writes(
        &self,
        session: &MailSession,
        ids: BTreeSet<String>,
        context: &MailboxContext,
    ) -> Result<(Vec<ThreadWrite>, bool), OutlookError> {
        stream::iter(ids)
            .map(|id| async move {
                let reference = session
                    .authorization()
                    .mailbox()
                    .thread(&id)
                    .map_err(|_| OutlookError::InvalidResponse)?;
                match self.thread_with_context(session, &id, context).await? {
                    Some(thread) => Ok(Some(ThreadWrite {
                        reference,
                        payload: serde_json::to_string(&thread)
                            .map_err(|_| OutlookError::InvalidResponse)?,
                    })),
                    None => Ok(None),
                }
            })
            .buffer_unordered(CONCURRENCY)
            .try_fold(
                (Vec::new(), 0usize, false),
                |(mut writes, size, mut missing), write| async move {
                    let size =
                        size.saturating_add(write.as_ref().map_or(0, |write| write.payload.len()));
                    if size > MAX_BYTES {
                        return Err(OutlookError::TooLarge);
                    }
                    if let Some(write) = write {
                        writes.push(write);
                    } else {
                        missing = true;
                    }
                    Ok((writes, size, missing))
                },
            )
            .await
            .map(|(writes, _, missing)| (writes, missing))
    }

    async fn full_scan(
        &self,
        session: &MailSession,
        revision: i64,
        mut checkpoint: Checkpoint,
    ) -> Result<SyncProgress, OutlookError> {
        let Phase::Full {
            run_id,
            next,
            mut seen,
        } = checkpoint.state.clone()
        else {
            return Err(OutlookError::InvalidCheckpoint);
        };
        let url = match next {
            Some(next) => self.checked_link(&next, &["messages"])?,
            None => self.queried(
                &["messages"],
                &[("$select", "id,conversationId"), ("$top", "50")],
            )?,
        };
        if !seen.insert(url.as_str().to_owned()) {
            return Err(OutlookError::InvalidResponse);
        }
        let page: wire::Page<wire::Change> = self.get(session, url).await?;
        if page.delta.is_some() {
            return Err(OutlookError::InvalidResponse);
        }
        let mut ids = BTreeSet::new();
        for item in page.value {
            if item.id.is_empty() || item.removed.is_some() {
                return Err(OutlookError::InvalidResponse);
            }
            let id = item
                .conversation_id
                .filter(|id| !id.is_empty())
                .ok_or(OutlookError::InvalidResponse)?;
            ids.insert(id);
        }
        let (upserts, _) = self.writes(session, ids, &checkpoint.context).await?;
        let complete = page.next.is_none();
        if let Some(next) = &page.next {
            self.checked_link(next, &["messages"])?;
            if seen.contains(next) {
                return Err(OutlookError::InvalidResponse);
            }
        }
        let caught_up = complete && checkpoint.folders.is_empty();
        if complete {
            if caught_up {
                checkpoint.discovery(false);
            } else {
                checkpoint.state = Phase::Delta {
                    index: 0,
                    seen: BTreeSet::new(),
                };
            }
        } else {
            checkpoint.state = Phase::Full {
                run_id: run_id.clone(),
                next: page.next,
                seen,
            };
        }
        self.commit(
            session,
            revision,
            checkpoint,
            upserts,
            Some((run_id, complete)),
            PageStatus {
                full_scan_complete: complete,
                caught_up,
            },
        )
        .await
    }
}

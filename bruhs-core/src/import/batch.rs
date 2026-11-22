//! Reading an export into a short-lived, private batch (D3, D5, D7).
//!
//! One read runs at a time. An upload (the web) or the desktop's picked
//! paths take the read slot; the read then runs on its own thread, and the
//! app polls [`ImportHost::job`] until the batch is ready or the read
//! failed.
//!
//! Each batch lives in `<core>/imports/<id>/` (0700):
//! - `incoming/<n>.part`, the uploaded files, deleted when the read ends;
//! - `scratch/`, nested ZIP copies, deleted when the read ends;
//! - `threads/<n>.json`, one normalized `ImportedThread` each;
//! - `index.json`, each thread's `import_key` and its `n`, and when the
//!   batch expires;
//! - `batch.json`, the `ImportBatch`, written last through a temp name
//!   and a rename. A folder without it is a read that never finished.
//!
//! Every file is 0600 and made with `create_new`. A failed or cancelled
//! read deletes its folder; a batch is kept at most [`BATCH_TTL_MS`], and
//! only the newest [`MAX_BATCHES`]. Every write a read makes (uploaded
//! parts, scratch copies, threads) counts toward a free space check every
//! 64 MiB (F14), on unix.
//!
//! Logs carry counts and fixed copy only: never a name, a path or text
//! from an export.

use super::agents::{picked_private, AgentRoots};
use super::archive::{never_opened, walk_with, EntryKind, ExportInput, Hooks, Limits, WalkReport};
use super::json_stream::for_each_record;
use super::text::{clip_part, clip_strings, shown_name, string_chars, TITLE_MAX_CHARS};
use super::{chatgpt, claude_ai, grok, ImportError, ParsedConversation};
use bruhs_models::agent::Role;
use bruhs_models::imports::{
    cut_short_line, import_key, valid_batch_id, AgentScan, BatchCounts, ImportBatch, ImportJob,
    ImportJobState, MemorySuggestion, BATCH_TTL_MS, CLAUDE_MANIFEST, DAMAGED_EXPORT,
    EXPORT_TOO_LARGE, FILE_PRIVATE, FOLDER_MISSING, FOLDER_PRIVATE, GROK_BEST_EFFORT, IMPORT_BUSY,
    IMPORT_FAILED, IMPORT_GONE, MAX_BATCHES, MAX_MESSAGES, MAX_NODES, MAX_PART_CHARS,
    NOT_AN_EXPORT, TOO_MANY_FILES,
};
use bruhs_models::logic::portable::{
    title_from_prompt, ImportCandidate, ImportSource, ImportedThread, PortableMessage,
    PortablePart, PortableToolCall,
};
use bruhs_models::EpochMs;
use rand::TryRngCore as _;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest as _, Sha256};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs::File;
use std::io::{self, Read as _, Write as _};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

const INCOMING: &str = "incoming";
const SCRATCH: &str = "scratch";
const THREADS: &str = "threads";
const INDEX: &str = "index.json";
const BATCH: &str = "batch.json";
const BATCH_TMP: &str = ".batch.json.tmp";

/// How long a failed read's message stays to be polled.
const FAILED_KEPT_MS: i64 = 10 * 60 * 1000;
/// A batch folder without `batch.json` this old is a read that stopped.
const ORPHAN_AGE_MS: i64 = 60 * 60 * 1000;
/// How often a running server sweeps.
const SWEEP_EVERY: Duration = Duration::from_secs(60 * 60);
/// Bytes written between two free space checks.
#[cfg(unix)]
const SPACE_CHECK_BYTES: u64 = 64 << 20;
/// Largest `batch.json` and `index.json` read back: 50 000 rows and their
/// keys fit well within it.
const BATCH_JSON_MAX: u64 = 64 << 20;
/// The read's progress is published every this many conversations.
const PROGRESS_EVERY: usize = 64;

/// Answers the time, so tests can move it.
type Clock = Arc<dyn Fn() -> EpochMs + Send + Sync>;
/// Answers the free bytes on the volume holding a folder.
#[cfg(unix)]
type FreeSpace = Arc<dyn Fn(&Path) -> io::Result<u64> + Send + Sync>;
/// Runs before each record a read handles (tests).
#[cfg(test)]
type RecordHook = Arc<dyn Fn() + Send + Sync>;

/// Reads exports into batches and answers them. Clones share one read
/// slot and one job table.
#[derive(Clone)]
pub struct ImportHost {
    root: PathBuf,
    limits: Limits,
    now: Clock,
    #[cfg(unix)]
    free: FreeSpace,
    #[cfg(test)]
    record_hook: Option<RecordHook>,
    state: Arc<State>,
    /// Where the local agents' folders are (F17).
    agents: AgentRoots,
}

#[derive(Default)]
struct State {
    jobs: Mutex<HashMap<String, JobEntry>>,
    /// The id of the one read that runs, from its upload's start to its
    /// end, and whether it was discarded while it uploaded.
    slot: Mutex<Option<(String, Arc<AtomicBool>)>>,
}

/// A read the job table still answers for. A ready batch is answered from
/// its folder instead.
enum JobEntry {
    Reading {
        cancel: Arc<AtomicBool>,
        files_done: u32,
        files: u32,
        conversations: u32,
    },
    Failed {
        message: String,
        at: EpochMs,
    },
}

impl JobEntry {
    fn state(&self) -> ImportJobState {
        match self {
            Self::Reading {
                files_done,
                files,
                conversations,
                ..
            } => ImportJobState::Reading {
                files_done: *files_done,
                files: *files,
                conversations: *conversations,
            },
            Self::Failed { message, .. } => ImportJobState::Failed {
                message: message.clone(),
            },
        }
    }
}

/// `index.json`: when the batch expires, and each thread's file.
#[derive(Debug, Default, Serialize, Deserialize)]
struct Index {
    expires_at_ms: EpochMs,
    threads: BTreeMap<String, u32>,
}

/// Holds the read slot; dropping it frees the slot.
struct Slot {
    state: Arc<State>,
    /// Set when the upload holding the slot is discarded.
    discarded: Arc<AtomicBool>,
}

impl Drop for Slot {
    fn drop(&mut self) {
        *lock(&self.state.slot) = None;
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

fn to_u64(n: usize) -> u64 {
    u64::try_from(n).unwrap_or(u64::MAX)
}

fn to_u32(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

impl ImportHost {
    /// A host keeping its batches under `root`, with the shipped caps.
    pub fn new(root: PathBuf) -> Self {
        Self {
            limits: Limits::DEFAULT,
            now: Arc::new(EpochMs::now),
            #[cfg(unix)]
            free: Arc::new(|path: &Path| crate::backup::free_bytes(path)),
            #[cfg(test)]
            record_hook: None,
            state: Arc::default(),
            // No agent folders until `from_env` or `with_agent_roots` names
            // them.
            agents: AgentRoots::none(),
            root,
        }
    }

    /// The host for this account: `<core>/imports`, and the agents' folders
    /// in this home. `None` when no home folder can be named.
    pub fn from_env() -> Option<Self> {
        crate::backup::Roots::from_env().map(|roots| Self {
            agents: AgentRoots::from_env(),
            ..Self::new(roots.core.join("imports"))
        })
    }

    /// The same host finding the local agents under `agents` (tests).
    #[cfg(any(test, feature = "test-support"))]
    #[must_use]
    pub fn with_agent_roots(mut self, agents: AgentRoots) -> Self {
        self.agents = agents;
        self
    }

    /// The personas `source` has, from `picked` or its default places
    /// ([`super::agents::scan`]).
    ///
    /// # Errors
    ///
    /// As [`super::agents::scan`].
    pub fn scan(
        &self,
        source: ImportSource,
        picked: Option<&Path>,
    ) -> Result<AgentScan, ImportError> {
        super::agents::scan(&self.agents, source, picked)
    }

    /// The same host with other caps (tests).
    #[cfg(any(test, feature = "test-support"))]
    #[must_use]
    pub fn with_limits(mut self, limits: Limits) -> Self {
        self.limits = limits;
        self
    }

    /// The same host on another clock (tests).
    #[cfg(any(test, feature = "test-support"))]
    #[must_use]
    pub fn with_clock(mut self, now: Arc<dyn Fn() -> EpochMs + Send + Sync>) -> Self {
        self.now = now;
        self
    }

    /// The same host reading free space from `free` (tests).
    #[cfg(all(unix, any(test, feature = "test-support")))]
    #[must_use]
    pub fn with_free_space(mut self, free: FreeSpace) -> Self {
        self.free = free;
        self
    }

    /// The same host calling `hook` before each record a read handles.
    #[cfg(test)]
    #[must_use]
    pub(crate) fn with_record_hook(mut self, hook: RecordHook) -> Self {
        self.record_hook = Some(hook);
        self
    }

    fn now(&self) -> EpochMs {
        (self.now)()
    }

    /// Starts an upload: it takes the read slot and writes its files to a
    /// new batch's `incoming/`.
    ///
    /// # Errors
    ///
    /// [`ImportError::Busy`] while another read holds the slot, else
    /// when the batch folder can't be made.
    pub fn begin_upload(&self) -> Result<ImportUpload, ImportError> {
        let id = random_id();
        let slot = self.take_slot(&id)?;
        let dir = self.new_batch_dir(&id)?;
        let incoming = dir.join(INCOMING);
        if let Err(error) = private_dir(&incoming) {
            let _ = std::fs::remove_dir_all(&dir);
            return Err(error);
        }
        Ok(ImportUpload {
            space: self.space_check(&dir),
            host: self.clone(),
            id,
            dir,
            incoming,
            parts: 0,
            file: None,
            bytes: 0,
            slot: Some(slot),
            started: false,
        })
    }

    /// Reads files or folders the desktop's picker chose, in place: they
    /// are only opened to read, never moved or deleted. `~` and `~/…` are
    /// read against the home, as a scan reads them.
    ///
    /// # Errors
    ///
    /// - `Refused(NOT_AN_EXPORT)` for no paths, `TOO_MANY_FILES` past
    ///   `upload_files`, `FOLDER_MISSING` for a path that isn't absolute
    ///   once expanded (nothing resolves against the working folder) or
    ///   isn't a regular file or a folder, `FOLDER_PRIVATE` for a folder
    ///   and `FILE_PRIVATE` for a file inside a folder never opened below
    ///   the home (D4, as typed or resolved), `EXPORT_TOO_LARGE` when the
    ///   files pass `upload_bytes`. A file whose own name is never opened
    ///   is left out and counted.
    /// - [`ImportError::Busy`] while another read holds the slot.
    pub fn read_paths(&self, paths: Vec<PathBuf>) -> Result<ImportJob, ImportError> {
        if paths.is_empty() {
            return Err(ImportError::Refused(NOT_AN_EXPORT));
        }
        if paths.len() > self.limits.upload_files {
            return Err(ImportError::Refused(TOO_MANY_FILES));
        }
        let mut inputs = Vec::new();
        let mut bytes: u64 = 0;
        for path in paths {
            let missing = || ImportError::Refused(FOLDER_MISSING);
            let path = self
                .agents
                .expand(&path)
                .filter(|path| path.is_absolute())
                .ok_or_else(missing)?;
            let real = std::fs::canonicalize(&path).map_err(|_| missing())?;
            let meta = std::fs::metadata(&real).map_err(|_| missing())?;
            if !meta.is_dir() && !meta.is_file() {
                return Err(missing());
            }
            // Every folder of the picked path and of the one it resolves
            // to, below the home, as a scan checks its places, and a
            // picked folder's own name (F33).
            if meta.is_dir() {
                if picked_private(&self.agents, &path, &real) {
                    return Err(ImportError::Refused(FOLDER_PRIVATE));
                }
                inputs.push(ExportInput::Folder(real));
                continue;
            }
            let parent = |path: &Path| path.parent().map(Path::to_path_buf).unwrap_or_default();
            if picked_private(&self.agents, &parent(&path), &parent(&real)) {
                return Err(ImportError::Refused(FILE_PRIVATE));
            }
            // A file private by its own name only (an export's
            // `users.json`) is left out and counted, as an entry is.
            let named_private = [path.as_path(), real.as_path()].iter().any(|path| {
                path.file_name()
                    .is_some_and(|name| never_opened(&name.to_string_lossy()))
            });
            if named_private {
                inputs.push(ExportInput::Skipped);
            } else {
                bytes = bytes.saturating_add(meta.len());
                inputs.push(ExportInput::File(real));
            }
        }
        if bytes > self.limits.upload_bytes {
            return Err(ImportError::Refused(EXPORT_TOO_LARGE));
        }
        let id = random_id();
        let slot = self.take_slot(&id)?;
        let dir = self.new_batch_dir(&id)?;
        self.start_read(id, dir, inputs, slot)
    }

    /// Where the read `id` stands: the job table while it reads or after
    /// it failed, else its batch folder. `None` for an id that isn't one,
    /// and for a batch that is gone or past its time.
    pub fn job(&self, id: &str) -> Option<ImportJob> {
        if !valid_batch_id(id) {
            return None;
        }
        let now = self.now();
        {
            let mut jobs = lock(&self.state.jobs);
            prune_failed(&mut jobs, now);
            if let Some(entry) = jobs.get(id) {
                return Some(ImportJob {
                    id: id.to_string(),
                    state: entry.state(),
                });
            }
        }
        let index = self.live_index(id, now)?;
        let mut batch: ImportBatch =
            serde_json::from_slice(&read_own(&self.root.join(id).join(BATCH), BATCH_JSON_MAX)?)
                .ok()?;
        batch.expires_at_ms = index.expires_at_ms;
        Some(ImportJob {
            id: id.to_string(),
            state: ImportJobState::Ready {
                batch: Box::new(batch),
            },
        })
    }

    /// Ends the read `id` and deletes what it wrote, or deletes its batch.
    ///
    /// # Errors
    ///
    /// [`ImportError::Gone`] when there is no such read or batch.
    pub fn discard(&self, id: &str) -> Result<(), ImportError> {
        if !valid_batch_id(id) {
            return Err(ImportError::Gone);
        }
        let dir = self.root.join(id);
        {
            // Held while deciding, so an upload's finish can't start its
            // read in between (it checks the flag under this lock).
            let mut jobs = lock(&self.state.jobs);
            match jobs.remove(id) {
                // The read deletes its own folder once it sees this.
                Some(JobEntry::Reading { cancel, .. }) => {
                    cancel.store(true, Ordering::SeqCst);
                    return Ok(());
                }
                Some(JobEntry::Failed { .. }) => return Ok(()),
                None => {}
            }
            // A ready batch goes even while its read still holds the slot
            // on the way out (M5); that read touches it no more.
            let ready = std::fs::symlink_metadata(dir.join(BATCH)).is_ok_and(|meta| meta.is_file());
            let upload = lock(&self.state.slot)
                .as_ref()
                .filter(|(held, _)| !ready && held == id)
                .map(|(_, discarded)| Arc::clone(discarded));
            if let Some(discarded) = upload {
                // An upload still being written stops, and its drop
                // deletes its folder.
                discarded.store(true, Ordering::SeqCst);
                return Ok(());
            }
            drop(jobs);
        }
        if std::fs::symlink_metadata(&dir).is_ok_and(|meta| meta.is_dir()) {
            std::fs::remove_dir_all(&dir).map_err(|error| ImportError::io(&dir, error))
        } else {
            Err(ImportError::Gone)
        }
    }

    /// The conversation `id` from `source` in the batch `batch`.
    ///
    /// # Errors
    ///
    /// [`ImportError::Gone`] when the batch isn't one, is missing or is
    /// past its time; [`ImportError::NotFound`] when it has no such
    /// conversation.
    pub fn thread(
        &self,
        batch: &str,
        source: ImportSource,
        id: &str,
    ) -> Result<ImportedThread, ImportError> {
        if !valid_batch_id(batch) {
            return Err(ImportError::Gone);
        }
        let index = self
            .live_index(batch, self.now())
            .ok_or(ImportError::Gone)?;
        let n = index
            .threads
            .get(&import_key(source, id))
            .ok_or_else(|| ImportError::not_found(source, id))?;
        let path = self
            .root
            .join(batch)
            .join(THREADS)
            .join(format!("{n}.json"));
        // Every thread a read writes fits `record` (`thread_bytes`).
        let bytes = read_own(&path, self.limits.record).ok_or(ImportError::Gone)?;
        serde_json::from_slice(&bytes).map_err(|_| ImportError::Refused(DAMAGED_EXPORT))
    }

    /// Removes batches past their time, folders a read left without
    /// `batch.json` more than an hour ago, a ready batch's leftover
    /// `incoming/` and `scratch/`, and failed reads older than 10
    /// minutes. A read that runs is left alone. Blocking file I/O: call
    /// it off the async runtime.
    pub fn sweep(&self) {
        let now = self.now();
        let mut busy: HashSet<String> = {
            let mut jobs = lock(&self.state.jobs);
            prune_failed(&mut jobs, now);
            jobs.keys().cloned().collect()
        };
        busy.extend(lock(&self.state.slot).as_ref().map(|(id, _)| id.clone()));
        for (id, dir) in self.batch_dirs() {
            if busy.contains(&id) {
                continue;
            }
            let ready = std::fs::symlink_metadata(dir.join(BATCH)).is_ok_and(|meta| meta.is_file());
            match self.index(&id).filter(|_| ready) {
                Some(index) if index.expires_at_ms > now => {
                    for leftover in [INCOMING, SCRATCH] {
                        let _ = std::fs::remove_dir_all(dir.join(leftover));
                    }
                }
                Some(_) => {
                    let _ = std::fs::remove_dir_all(&dir);
                }
                None => {
                    let stale = modified_ms(&dir)
                        .is_none_or(|modified| now.millis_since(modified) > ORPHAN_AGE_MS);
                    if stale {
                        let _ = std::fs::remove_dir_all(&dir);
                    }
                }
            }
        }
    }

    /// Sweeps every hour for as long as the runtime runs, each sweep off
    /// the async workers. The server's start sweeps once itself.
    pub fn keep_sweeping(&self) {
        let host = self.clone();
        tokio::spawn(async move {
            let start = tokio::time::Instant::now() + SWEEP_EVERY;
            let mut ticks = tokio::time::interval_at(start, SWEEP_EVERY);
            ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                ticks.tick().await;
                let sweeper = host.clone();
                if tokio::task::spawn_blocking(move || sweeper.sweep())
                    .await
                    .is_err()
                {
                    tracing::warn!("An import sweep stopped early");
                }
            }
        });
    }

    fn take_slot(&self, id: &str) -> Result<Slot, ImportError> {
        let mut slot = lock(&self.state.slot);
        if slot.is_some() {
            return Err(ImportError::Busy);
        }
        let discarded = Arc::new(AtomicBool::new(false));
        *slot = Some((id.to_string(), Arc::clone(&discarded)));
        drop(slot);
        Ok(Slot {
            state: Arc::clone(&self.state),
            discarded,
        })
    }

    /// Makes `<root>` (0700) and the new batch folder `<root>/<id>`
    /// (0700).
    fn new_batch_dir(&self, id: &str) -> Result<PathBuf, ImportError> {
        if let Some(parent) = self.root.parent() {
            std::fs::create_dir_all(parent).map_err(|error| ImportError::io(parent, error))?;
        }
        private_dir(&self.root)?;
        let dir = self.root.join(id);
        dir_builder()
            .create(&dir)
            .map_err(|error| ImportError::io(&dir, error))?;
        Ok(dir)
    }

    fn space_check(&self, dir: &Path) -> SpaceCheck {
        #[cfg(not(unix))]
        let _ = dir;
        SpaceCheck {
            #[cfg(unix)]
            free: Arc::clone(&self.free),
            #[cfg(unix)]
            dir: dir.to_path_buf(),
            #[cfg(unix)]
            written: std::cell::Cell::new(0),
            #[cfg(unix)]
            next: std::cell::Cell::new(0),
        }
    }

    /// Records the read and starts it on its own thread, which keeps the
    /// slot until it ends.
    fn start_read(
        &self,
        id: String,
        dir: PathBuf,
        inputs: Vec<ExportInput>,
        slot: Slot,
    ) -> Result<ImportJob, ImportError> {
        let cancel = Arc::new(AtomicBool::new(false));
        let files = to_u32(inputs.len());
        {
            let mut jobs = lock(&self.state.jobs);
            if slot.discarded.load(Ordering::SeqCst) {
                drop(jobs);
                let _ = std::fs::remove_dir_all(&dir);
                return Err(ImportError::Gone);
            }
            jobs.insert(
                id.clone(),
                JobEntry::Reading {
                    cancel: Arc::clone(&cancel),
                    files_done: 0,
                    files,
                    conversations: 0,
                },
            );
        }
        let host = self.clone();
        let (job_id, job_dir) = (id.clone(), dir.clone());
        let spawned = std::thread::Builder::new()
            .name("bruhs-import".into())
            .spawn(move || {
                let _slot = slot;
                host.run_read(&job_id, &job_dir, &inputs, &cancel);
            });
        if let Err(error) = spawned {
            lock(&self.state.jobs).remove(&id);
            let _ = std::fs::remove_dir_all(&dir);
            return Err(ImportError::io(&dir, error));
        }
        Ok(ImportJob {
            id,
            state: ImportJobState::Reading {
                files_done: 0,
                files,
                conversations: 0,
            },
        })
    }

    /// The read itself, then its cleanup: the uploaded parts and scratch
    /// copies always go; a failed or cancelled read takes its folder too.
    fn run_read(&self, id: &str, dir: &Path, inputs: &[ExportInput], cancel: &AtomicBool) {
        let result = self.read_into(id, dir, inputs, cancel);
        for leftover in [INCOMING, SCRATCH] {
            let _ = std::fs::remove_dir_all(dir.join(leftover));
        }
        // Older batches go before this one is answered as ready, and a
        // failed or discarded read's folder goes before the job table is
        // locked (M3).
        let kept = result.is_ok() && !cancel.load(Ordering::SeqCst);
        if kept {
            self.keep_newest(id);
        } else {
            let _ = std::fs::remove_dir_all(dir);
        }
        let mut jobs = lock(&self.state.jobs);
        let cancelled = cancel.load(Ordering::SeqCst);
        match result {
            Ok(counts) if !cancelled => {
                jobs.remove(id);
                drop(jobs);
                tracing::info!(
                    conversations = counts.conversations,
                    files = counts.files_read,
                    "An export was read"
                );
            }
            outcome => {
                if cancelled {
                    // `discard` already took its entry.
                    jobs.remove(id);
                    drop(jobs);
                    if kept {
                        // Discarded after the folder was kept.
                        let _ = std::fs::remove_dir_all(dir);
                    }
                    tracing::info!("An export read was discarded");
                } else if let Err(error) = outcome {
                    let message = failed_copy(&error);
                    jobs.insert(
                        id.to_string(),
                        JobEntry::Failed {
                            message: message.clone(),
                            at: self.now(),
                        },
                    );
                    drop(jobs);
                    tracing::warn!(error = %message, "An export read failed");
                }
            }
        }
    }

    /// Holds the read slot for `id`, as a read on its way out does
    /// (tests).
    #[cfg(test)]
    pub(crate) fn hold_slot_for_test(&self, id: &str) -> impl Drop {
        self.take_slot(id).expect("the slot is free")
    }

    fn read_into(
        &self,
        id: &str,
        dir: &Path,
        inputs: &[ExportInput],
        cancel: &AtomicBool,
    ) -> Result<BatchCounts, ImportError> {
        let scratch = dir.join(SCRATCH);
        private_dir(&scratch)?;
        let threads = dir.join(THREADS);
        private_dir(&threads)?;
        let space = self.space_check(dir);
        let mut reader = BatchReader {
            host: self,
            id,
            threads: &threads,
            space: &space,
            cancel,
            candidates: Vec::new(),
            index: BTreeMap::new(),
            memory: MemoryTexts::default(),
            counts: BatchCounts::default(),
            stored: 0,
            claude_manifest: false,
        };
        let wrote = |bytes: u64| space.add(bytes);
        let input_done = |done: u32| self.progress(id, Some(done), None);
        let hooks = Hooks {
            wrote: &wrote,
            input_done: &input_done,
        };
        let limits = self.limits;
        let report = walk_with(
            inputs,
            &scratch,
            &limits,
            cancel,
            &hooks,
            &mut |kind, entry, len| {
                for_each_record(entry, kind, len, &limits, &mut |value| {
                    reader.record(kind, value)
                })
            },
        )?;
        if cancel.load(Ordering::SeqCst) {
            return Err(ImportError::Gone);
        }
        reader.finish(dir, report, self.now())
    }

    /// Publishes a running read's progress.
    fn progress(&self, id: &str, files_done: Option<u32>, chats: Option<u32>) {
        if let Some(JobEntry::Reading {
            files_done: done,
            conversations,
            ..
        }) = lock(&self.state.jobs).get_mut(id)
        {
            if let Some(files_done) = files_done {
                *done = files_done;
            }
            if let Some(chats) = chats {
                *conversations = chats;
            }
        }
    }

    /// Every folder under the root named like a batch.
    fn batch_dirs(&self) -> Vec<(String, PathBuf)> {
        let Ok(listing) = std::fs::read_dir(&self.root) else {
            return Vec::new();
        };
        listing
            .flatten()
            .filter_map(|entry| {
                let id = entry.file_name().to_str()?.to_string();
                let dir = entry.path();
                let is_dir = std::fs::symlink_metadata(&dir).is_ok_and(|meta| meta.is_dir());
                (valid_batch_id(&id) && is_dir).then_some((id, dir))
            })
            .collect()
    }

    fn index(&self, id: &str) -> Option<Index> {
        let bytes = read_own(&self.root.join(id).join(INDEX), BATCH_JSON_MAX)?;
        serde_json::from_slice(&bytes).ok()
    }

    /// The index of the ready batch `id`, unless it is past its time.
    fn live_index(&self, id: &str, now: EpochMs) -> Option<Index> {
        let ready = std::fs::symlink_metadata(self.root.join(id).join(BATCH))
            .is_ok_and(|meta| meta.is_file());
        self.index(id)
            .filter(|index| ready && index.expires_at_ms > now)
    }

    /// Keeps the batch `newest` and the newest others, [`MAX_BATCHES`] in
    /// all, and removes the rest.
    fn keep_newest(&self, newest: &str) {
        let busy: HashSet<String> = lock(&self.state.jobs).keys().cloned().collect();
        let mut others: Vec<(EpochMs, String, PathBuf)> = self
            .batch_dirs()
            .into_iter()
            .filter(|(id, dir)| id != newest && !busy.contains(id) && dir.join(BATCH).is_file())
            .filter_map(|(id, dir)| Some((self.index(&id)?.expires_at_ms, id, dir)))
            .collect();
        others.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| b.1.cmp(&a.1)));
        for (_, _, dir) in others.into_iter().skip(MAX_BATCHES.saturating_sub(1)) {
            let _ = std::fs::remove_dir_all(dir);
        }
    }
}

/// The fixed copy a failed read answers.
fn failed_copy(error: &ImportError) -> String {
    match error {
        ImportError::Refused(copy) => (*copy).to_string(),
        ImportError::Space(_) => error.to_string(),
        ImportError::Busy => IMPORT_BUSY.to_string(),
        ImportError::Gone => IMPORT_GONE.to_string(),
        _ => IMPORT_FAILED.to_string(),
    }
}

/// Drops failed reads older than [`FAILED_KEPT_MS`].
fn prune_failed(jobs: &mut HashMap<String, JobEntry>, now: EpochMs) {
    jobs.retain(|_, entry| match entry {
        JobEntry::Reading { .. } => true,
        JobEntry::Failed { at, .. } => now.millis_since(*at) <= FAILED_KEPT_MS,
    });
}

/// An upload being written: one file per [`ImportUpload::next_file`].
/// Dropped before [`ImportUpload::finish`], it deletes its batch folder
/// and frees the read slot.
pub struct ImportUpload {
    host: ImportHost,
    id: String,
    dir: PathBuf,
    incoming: PathBuf,
    parts: usize,
    file: Option<File>,
    bytes: u64,
    space: SpaceCheck,
    slot: Option<Slot>,
    /// Set once the read took the files over.
    started: bool,
}

impl ImportUpload {
    /// The read's id, which its batch keeps.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Starts the next uploaded file.
    ///
    /// # Errors
    ///
    /// `Refused(TOO_MANY_FILES)` past `upload_files`, else when the file
    /// can't be made.
    pub fn next_file(&mut self) -> Result<(), ImportError> {
        self.check_discarded()?;
        if self.parts >= self.host.limits.upload_files {
            return Err(ImportError::Refused(TOO_MANY_FILES));
        }
        self.close_part()?;
        let path = self.incoming.join(format!("{}.part", self.parts));
        let file = new_private_file(&path).map_err(|error| ImportError::io(&path, error))?;
        self.parts += 1;
        self.file = Some(file);
        Ok(())
    }

    /// Appends one chunk to the current file, starting the first file if
    /// none is open.
    ///
    /// # Errors
    ///
    /// `Refused(EXPORT_TOO_LARGE)` past `upload_bytes` in all,
    /// [`ImportError::Space`] once less than `FREE_MARGIN` would be left
    /// (checked every 64 MiB), else when the write fails.
    pub fn write(&mut self, chunk: &[u8]) -> Result<(), ImportError> {
        self.check_discarded()?;
        if self.file.is_none() {
            self.next_file()?;
        }
        let added = to_u64(chunk.len());
        let total = self.bytes.saturating_add(added);
        if total > self.host.limits.upload_bytes {
            return Err(ImportError::Refused(EXPORT_TOO_LARGE));
        }
        self.space.add(added)?;
        let file = self.file.as_mut().ok_or(ImportError::Gone)?;
        file.write_all(chunk)
            .map_err(|error| ImportError::io(&self.incoming, error))?;
        self.bytes = total;
        Ok(())
    }

    /// Ends the upload and starts reading it.
    ///
    /// # Errors
    ///
    /// `Refused(NOT_AN_EXPORT)` when nothing was uploaded, else when the
    /// last file can't be closed or the read can't start.
    pub fn finish(mut self) -> Result<ImportJob, ImportError> {
        self.check_discarded()?;
        self.close_part()?;
        if self.parts == 0 {
            return Err(ImportError::Refused(NOT_AN_EXPORT));
        }
        let slot = self.slot.take().ok_or(ImportError::Gone)?;
        let inputs = (0..self.parts)
            .map(|n| ExportInput::File(self.incoming.join(format!("{n}.part"))))
            .collect();
        self.started = true;
        self.host
            .start_read(self.id.clone(), self.dir.clone(), inputs, slot)
    }

    /// Whether [`ImportHost::discard`] ended this upload: the caller
    /// should stop waiting for more bytes and drop it, which frees the
    /// read slot. A plain flag read, safe on an async worker.
    pub fn is_discarded(&self) -> bool {
        self.slot
            .as_ref()
            .is_some_and(|slot| slot.discarded.load(Ordering::SeqCst))
    }

    /// [`ImportError::Gone`] once the upload was discarded.
    fn check_discarded(&self) -> Result<(), ImportError> {
        if self.is_discarded() {
            Err(ImportError::Gone)
        } else {
            Ok(())
        }
    }

    fn close_part(&mut self) -> Result<(), ImportError> {
        if let Some(file) = self.file.take() {
            file.sync_all()
                .map_err(|error| ImportError::io(&self.incoming, error))?;
        }
        Ok(())
    }
}

impl Drop for ImportUpload {
    fn drop(&mut self) {
        if !self.started {
            // Closed first: Windows can't delete an open file.
            drop(self.file.take());
            let _ = std::fs::remove_dir_all(&self.dir);
        }
        // The slot is freed after the folder is gone.
        drop(self.slot.take());
    }
}

/// Counts what one read or upload writes, and checks the free space
/// every [`SPACE_CHECK_BYTES`] (unix; skipped elsewhere, D18).
struct SpaceCheck {
    #[cfg(unix)]
    free: FreeSpace,
    #[cfg(unix)]
    dir: PathBuf,
    #[cfg(unix)]
    written: std::cell::Cell<u64>,
    #[cfg(unix)]
    next: std::cell::Cell<u64>,
}

impl SpaceCheck {
    /// Counts `bytes` about to be written.
    ///
    /// # Errors
    ///
    /// [`ImportError::Space`] when less than `FREE_MARGIN` would be left
    /// after the next 64 MiB (or `bytes`, when more).
    fn add(&self, bytes: u64) -> Result<(), ImportError> {
        #[cfg(unix)]
        {
            use crate::backup::FREE_MARGIN;
            let written = self.written.get().saturating_add(bytes);
            self.written.set(written);
            if written >= self.next.get() {
                let free =
                    (self.free)(&self.dir).map_err(|error| ImportError::io(&self.dir, error))?;
                let need = FREE_MARGIN.saturating_add(SPACE_CHECK_BYTES.max(bytes));
                if free < need {
                    return Err(ImportError::Space(need - free));
                }
                self.next.set(written.saturating_add(SPACE_CHECK_BYTES));
            }
        }
        #[cfg(not(unix))]
        let _ = bytes;
        Ok(())
    }
}

/// Classifies each record a read gives and stores its conversations.
struct BatchReader<'r> {
    host: &'r ImportHost,
    id: &'r str,
    threads: &'r Path,
    space: &'r SpaceCheck,
    cancel: &'r AtomicBool,
    candidates: Vec<ImportCandidate>,
    /// Each stored conversation's `import_key`, and its file's number.
    index: BTreeMap<String, u32>,
    memory: MemoryTexts,
    counts: BatchCounts,
    stored: u64,
    claude_manifest: bool,
}

impl BatchReader<'_> {
    /// One record from `kind`. A record from Grok's own file is only ever
    /// read as Grok: one that isn't counts as unrecognised.
    fn record(&mut self, kind: EntryKind, value: Value) -> Result<(), ImportError> {
        #[cfg(test)]
        if let Some(hook) = &self.host.record_hook {
            hook();
        }
        if self.cancel.load(Ordering::SeqCst) {
            return Err(ImportError::Gone);
        }
        if kind == EntryKind::GrokBackend {
            return if grok::is_conversation(&value) {
                self.conversation(&value, grok::conversation(&value))
            } else {
                self.unknown();
                Ok(())
            };
        }
        if self.classify(&value)? {
            return Ok(());
        }
        // A file read whole (a bare upload, a manifest or memory file)
        // may hold a list of conversations, as Grok's does.
        match value.get("conversations").and_then(Value::as_array) {
            Some(records) => {
                for record in records {
                    if self.cancel.load(Ordering::SeqCst) {
                        return Err(ImportError::Gone);
                    }
                    if !self.classify(record)? {
                        self.unknown();
                    }
                }
            }
            None => self.unknown(),
        }
        Ok(())
    }

    /// Reads `v` as the first kind it matches, in D4's order: a ChatGPT
    /// conversation, a Claude conversation, a Grok conversation, Claude
    /// memory, a manifest. `false` when it is none of them.
    fn classify(&mut self, v: &Value) -> Result<bool, ImportError> {
        if chatgpt::is_conversation(v) {
            self.conversation(v, chatgpt::conversation(v))?;
        } else if claude_ai::is_conversation(v) {
            self.conversation(v, claude_ai::conversation(v))?;
        } else if grok::is_conversation(v) {
            self.conversation(v, grok::conversation(v))?;
        } else if claude_ai::is_memory(v) {
            // Memory keys with no text offer nothing.
            // The same file in two parts of a split export comes once.
            if let Some(MemorySuggestion { text, .. }) = claude_ai::memory(v) {
                self.memory.add(text);
            }
        } else if claude_ai::is_manifest(v) {
            self.claude_manifest = true;
        } else if !chatgpt::is_manifest(v) {
            return Ok(false);
        }
        Ok(true)
    }

    fn unknown(&mut self) {
        self.counts.skipped_unknown = self.counts.skipped_unknown.saturating_add(1);
    }

    /// Stores one parsed conversation, unless it is empty, came before or
    /// is past the read's limit.
    fn conversation(
        &mut self,
        raw: &Value,
        parsed: Option<ParsedConversation>,
    ) -> Result<(), ImportError> {
        let counts = &mut self.counts;
        let Some(parsed) = parsed else {
            counts.skipped_empty = counts.skipped_empty.saturating_add(1);
            return Ok(());
        };
        let mut candidate = parsed.candidate;
        let key = import_key(candidate.source, &candidate.id);
        if self.index.contains_key(&key) {
            counts.duplicates = counts.duplicates.saturating_add(1);
            return Ok(());
        }
        if self.candidates.len() >= self.host.limits.conversations {
            counts.over_limit = counts.over_limit.saturating_add(1);
            return Ok(());
        }
        let mut cut =
            nodes(candidate.source, raw) > MAX_NODES || parsed.messages.len() >= MAX_MESSAGES;
        candidate.title = row_title(&candidate.title, &parsed.messages);
        let Some((bytes, fitted)) =
            thread_bytes(candidate.clone(), parsed.messages, self.host.limits.record)?
        else {
            counts.cut_short = counts.cut_short.saturating_add(1);
            return Ok(());
        };
        cut |= fitted;
        let size = to_u64(bytes.len());
        if self.stored.saturating_add(size) > self.host.limits.stored {
            return Err(ImportError::Refused(EXPORT_TOO_LARGE));
        }
        self.space.add(size)?;
        let n = to_u32(self.candidates.len());
        write_private(&self.threads.join(format!("{n}.json")), &bytes)?;
        self.stored = self.stored.saturating_add(size);
        self.index.insert(key, n);
        self.candidates.push(candidate);
        let counts = &mut self.counts;
        counts.dropped_branches = counts
            .dropped_branches
            .saturating_add(parsed.dropped_branches);
        if cut {
            counts.cut_short = counts.cut_short.saturating_add(1);
        }
        if self.candidates.len().is_multiple_of(PROGRESS_EVERY) {
            let chats = to_u32(self.candidates.len());
            self.host.progress(self.id, None, Some(chats));
        }
        Ok(())
    }

    /// Writes `index.json`, then `batch.json` through a temp name, and
    /// answers the batch's counts.
    fn finish(
        mut self,
        dir: &Path,
        report: WalkReport,
        now: EpochMs,
    ) -> Result<BatchCounts, ImportError> {
        let expires_at_ms =
            EpochMs::saturating_from_i64(now.as_millis().saturating_add(BATCH_TTL_MS));
        self.counts.conversations = to_u32(self.candidates.len());
        self.counts.files_read = report.files_read;
        self.counts.skipped_files = report.skipped;
        let best_effort = self
            .candidates
            .iter()
            .any(|candidate| candidate.source == ImportSource::Grok);
        let mut warnings = Vec::new();
        if best_effort {
            warnings.push(GROK_BEST_EFFORT.to_string());
        }
        let claude_chats = self
            .candidates
            .iter()
            .any(|candidate| candidate.source == ImportSource::ClaudeAi);
        if self.claude_manifest && !claude_chats {
            warnings.push(CLAUDE_MANIFEST.to_string());
        }
        let memory = std::mem::take(&mut self.memory)
            .suggestion()
            .into_iter()
            .collect();
        let mut candidates = std::mem::take(&mut self.candidates);
        candidates.sort_by_key(|candidate| std::cmp::Reverse(candidate.updated_at));
        let index = Index {
            expires_at_ms,
            threads: std::mem::take(&mut self.index),
        };
        let index = serde_json::to_vec(&index).map_err(|_| ImportError::Refused(IMPORT_FAILED))?;
        readable_back(&index)?;
        self.space.add(to_u64(index.len()))?;
        write_synced(&dir.join(INDEX), &index)?;
        let batch = ImportBatch {
            id: self.id.to_string(),
            candidates,
            memory,
            counts: self.counts,
            warnings,
            best_effort,
            expires_at_ms,
        };
        let bytes = serde_json::to_vec(&batch).map_err(|_| ImportError::Refused(IMPORT_FAILED))?;
        readable_back(&bytes)?;
        self.space.add(to_u64(bytes.len()))?;
        // On disk before the rename, so a crash never leaves an empty
        // `batch.json` in place (M4).
        let tmp = dir.join(BATCH_TMP);
        write_synced(&tmp, &bytes)?;
        let path = dir.join(BATCH);
        std::fs::rename(&tmp, &path).map_err(|error| ImportError::io(&path, error))?;
        Ok(self.counts)
    }
}

/// The thread `candidate` and `messages` make, serialized within `cap`
/// bytes, and whether anything was left out to fit (final review I1). A
/// reply merges every assistant node until the next prompt, and the
/// context repeats the newest message whole, so a thread can outgrow its
/// record. Past `cap`, each round keeps the messages within half of what
/// is left to try ([`trim`]); once one part is all that is left, that
/// part is cut from its own text, by halves. `None` when even that
/// doesn't fit: the chat is left out and counted as cut.
fn thread_bytes(
    candidate: ImportCandidate,
    messages: Vec<PortableMessage>,
    cap: u64,
) -> Result<Option<(Vec<u8>, bool)>, ImportError> {
    let fits = |candidate, messages| -> Result<Result<Vec<u8>, ImportedThread>, ImportError> {
        let thread = ImportedThread::new(candidate, messages);
        let bytes = serde_json::to_vec(&thread).map_err(|_| ImportError::Refused(IMPORT_FAILED))?;
        Ok(if to_u64(bytes.len()) <= cap {
            Ok(bytes)
        } else {
            Err(thread)
        })
    };
    let (mut candidate, mut messages) = (candidate, messages);
    let mut budget = cap / 2;
    let mut cut = false;
    loop {
        let thread = match fits(candidate, messages)? {
            Ok(bytes) => return Ok(Some((bytes, cut))),
            Err(thread) => thread,
        };
        ImportedThread {
            candidate,
            messages,
            ..
        } = thread;
        let one_part = match messages.as_slice() {
            [only] => only.parts.len() <= 1,
            _ => messages.is_empty(),
        };
        if one_part {
            break;
        }
        trim(&mut messages, budget);
        cut = true;
        budget /= 2;
    }
    let Some(part) = messages.first_mut().and_then(|message| message.parts.pop()) else {
        return Ok(None);
    };
    let mut limit = part_chars(&part) / 2;
    while limit > 0 {
        if let Some(message) = messages.first_mut() {
            message.parts.push(clipped(&part, limit));
        }
        let thread = match fits(candidate, messages)? {
            Ok(bytes) => return Ok(Some((bytes, true))),
            Err(thread) => thread,
        };
        ImportedThread {
            candidate,
            messages,
            ..
        } = thread;
        if let Some(message) = messages.first_mut() {
            message.parts.pop();
        }
        limit /= 2;
    }
    Ok(None)
}

/// `messages` cut to about `budget` bytes serialized, keeping the newest.
/// The newest message keeps its newest parts within half the budget
/// first, so a huge last reply loses its own oldest parts before any
/// prompt goes; then the oldest messages go, then the oldest parts of the
/// first one left. The newest message and its newest part always stay.
fn trim(messages: &mut Vec<PortableMessage>, budget: u64) {
    if let Some(last) = messages.last_mut() {
        drop_oldest_parts(&mut last.parts, budget / 2);
    }
    let sizes: Vec<u64> = messages.iter().map(serialized_len).collect();
    let mut total = sizes
        .iter()
        .fold(0_u64, |sum, size| sum.saturating_add(*size));
    let mut first = 0;
    while total > budget && first + 1 < messages.len() {
        total = total.saturating_sub(sizes[first]);
        first += 1;
    }
    messages.drain(..first);
    if let Some(message) = messages.first_mut() {
        drop_oldest_parts(&mut message.parts, budget);
    }
}

/// `parts` without their oldest ones until they take at most `budget`
/// bytes serialized; the newest always stays.
fn drop_oldest_parts(parts: &mut Vec<PortablePart>, budget: u64) {
    let sizes: Vec<u64> = parts.iter().map(serialized_len).collect();
    let mut total = sizes
        .iter()
        .fold(0_u64, |sum, size| sum.saturating_add(*size));
    let mut first = 0;
    while total > budget && first + 1 < parts.len() {
        total = total.saturating_sub(sizes[first]);
        first += 1;
    }
    parts.drain(..first);
}

/// Characters of text `part` holds: its text, or the larger of a tool
/// call's input strings and output.
fn part_chars(part: &PortablePart) -> usize {
    match part {
        PortablePart::Text(text) | PortablePart::Thinking(text) => text.chars().count(),
        PortablePart::ToolCall(call) => string_chars(&call.input).max(
            call.output
                .as_deref()
                .map_or(0, |output| output.chars().count()),
        ),
    }
}

/// `part` with its text cut to `limit` characters, each cut ending with
/// its cut line.
fn clipped(part: &PortablePart, limit: usize) -> PortablePart {
    match part {
        PortablePart::Text(text) => PortablePart::Text(clip_part(text.clone(), limit)),
        PortablePart::Thinking(text) => PortablePart::Thinking(clip_part(text.clone(), limit)),
        PortablePart::ToolCall(call) => PortablePart::ToolCall(PortableToolCall {
            input: clip_strings(&call.input, limit),
            output: call.output.clone().map(|output| clip_part(output, limit)),
            ..call.clone()
        }),
    }
}

/// Bytes `value` takes as JSON, counted without keeping them.
fn serialized_len(value: &impl Serialize) -> u64 {
    struct Count(u64);
    impl io::Write for Count {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0 = self.0.saturating_add(to_u64(bytes.len()));
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    let mut count = Count(0);
    serde_json::to_writer(&mut count, value).map_or(u64::MAX, |()| count.0)
}

/// Claude's memory as a read collects it: each distinct text once, by
/// its digest, joined with a blank line and cut to [`MAX_PART_CHARS`].
/// Texts are held only until the joined text reaches the cut, and past it
/// only their characters are counted, so a read holds about two parts'
/// worth however much memory an export repeats (final review C2).
#[derive(Default)]
pub(super) struct MemoryTexts {
    held: Vec<String>,
    seen: HashSet<[u8; 32]>,
    /// Characters of every distinct text joined, separators included.
    chars: usize,
}

impl MemoryTexts {
    /// Adds `text` unless it came before.
    pub(super) fn add(&mut self, text: String) {
        if !self.seen.insert(Sha256::digest(text.as_bytes()).into()) {
            return;
        }
        let before = self.chars;
        let separator = if self.seen.len() > 1 { 2 } else { 0 };
        self.chars = before
            .saturating_add(separator)
            .saturating_add(text.chars().count());
        if before < MAX_PART_CHARS {
            self.held.push(text);
        }
    }

    /// Characters the held texts take, joined (tests).
    #[cfg(test)]
    pub(super) fn held_chars(&self) -> usize {
        self.held
            .iter()
            .map(|text| text.chars().count())
            .sum::<usize>()
            .saturating_add(2 * self.held.len().saturating_sub(1))
    }

    /// The joined memory as one suggestion, cut as
    /// [`clip_part`](super::text::clip_part) cuts the whole join: what was
    /// left out is counted, held or not.
    pub(super) fn suggestion(self) -> Option<MemorySuggestion> {
        if self.held.is_empty() {
            return None;
        }
        let mut text = self.held.join("\n\n");
        if self.chars > MAX_PART_CHARS {
            if let Some((cut, _)) = text.char_indices().nth(MAX_PART_CHARS) {
                text.truncate(cut);
            }
            text.push('\n');
            text.push_str(&cut_short_line(self.chars - MAX_PART_CHARS));
        }
        Some(MemorySuggestion {
            source: ImportSource::ClaudeAi,
            text,
        })
    }
}

/// The nodes a conversation record holds before a walk caps them at
/// `MAX_NODES`: ChatGPT's mapping, Claude's messages, Grok's responses.
fn nodes(source: ImportSource, raw: &Value) -> usize {
    let len = |key: &str| match raw.get(key) {
        Some(Value::Object(map)) => map.len(),
        Some(Value::Array(items)) => items.len(),
        _ => 0,
    };
    match source {
        ImportSource::ChatGpt => len("mapping"),
        ImportSource::ClaudeAi => len("chat_messages").max(len("messages")),
        ImportSource::Grok => len("responses"),
        _ => 0,
    }
}

/// A provider's title as a row shows it: one clean line of at most
/// [`TITLE_MAX_CHARS`]. A reader already names a chat whose title cleans
/// to nothing by its branch's first prompt (M7, D6); one still empty
/// falls back to the first prompt kept.
fn row_title(title: &str, messages: &[PortableMessage]) -> String {
    let shown = shown_name(title, TITLE_MAX_CHARS);
    if !shown.is_empty() {
        return shown;
    }
    let prompt = messages
        .iter()
        .find(|message| message.role == Role::User)
        .map(PortableMessage::text)
        .unwrap_or_default();
    title_from_prompt(&shown_name(&prompt, TITLE_MAX_CHARS))
}

/// 16 fresh OS-random bytes as 32 lowercase hex characters.
fn random_id() -> String {
    let mut bytes = [0_u8; 16];
    rand::rngs::OsRng
        .try_fill_bytes(&mut bytes)
        .expect("the operating system's random source answers");
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn dir_builder() -> std::fs::DirBuilder {
    #[cfg_attr(not(unix), expect(unused_mut, reason = "modes are unix only"))]
    let mut builder = std::fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt as _;
        builder.mode(0o700);
    }
    builder
}

/// Makes `path` a folder only this user can open, or makes the existing
/// one so. A link or a file there is refused.
fn private_dir(path: &Path) -> Result<(), ImportError> {
    let io_error = |error: io::Error| ImportError::io(path, error);
    match dir_builder().create(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            let meta = std::fs::symlink_metadata(path).map_err(io_error)?;
            if !meta.is_dir() {
                return Err(io_error(io::Error::other("not a folder")));
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt as _;
                std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
                    .map_err(io_error)?;
            }
            Ok(())
        }
        Err(error) => Err(io_error(error)),
    }
}

/// A new 0600 file, never replacing one or following a link.
fn new_private_file(path: &Path) -> io::Result<File> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    options.open(path)
}

fn write_private(path: &Path, bytes: &[u8]) -> Result<(), ImportError> {
    new_private_file(path)
        .and_then(|mut file| file.write_all(bytes))
        .map_err(|error| ImportError::io(path, error))
}

/// [`write_private`], then flushed to disk.
fn write_synced(path: &Path, bytes: &[u8]) -> Result<(), ImportError> {
    new_private_file(path)
        .and_then(|mut file| {
            file.write_all(bytes)?;
            file.sync_all()
        })
        .map_err(|error| ImportError::io(path, error))
}

/// Refuses an `index.json` or `batch.json` that [`read_own`] couldn't read
/// back (M4).
fn readable_back(bytes: &[u8]) -> Result<(), ImportError> {
    if to_u64(bytes.len()) > BATCH_JSON_MAX {
        Err(ImportError::Refused(EXPORT_TOO_LARGE))
    } else {
        Ok(())
    }
}

/// One of a batch's own files, read without following a link or waiting
/// on a FIFO; `None` when it is missing, not a regular file or over
/// `max` bytes.
fn read_own(path: &Path, max: u64) -> Option<Vec<u8>> {
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let file = options.open(path).ok()?;
    let meta = file.metadata().ok()?;
    if !meta.is_file() || meta.len() > max {
        return None;
    }
    let mut bytes = Vec::new();
    file.take(max).read_to_end(&mut bytes).ok()?;
    Some(bytes)
}

/// When `path` was last changed, as epoch ms.
fn modified_ms(path: &Path) -> Option<EpochMs> {
    let modified = std::fs::symlink_metadata(path).ok()?.modified().ok()?;
    let ms = modified
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_millis();
    Some(EpochMs::saturating_from_i64(i64::try_from(ms).ok()?))
}

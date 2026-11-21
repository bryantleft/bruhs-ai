//! The backup host (D6, D8, D20, ruling F25): one job at a time (making or
//! opening a backup), the web's uploads, the downloads waiting in
//! `backups/outgoing`, the last backup record, the sweep and the overview.
//!
//! Nothing here logs or returns a password, a keychain value or a path
//! inside a backup.

use std::fs::{File, OpenOptions};
use std::future::Future;
use std::io::{self, Read, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, SystemTime};

use bruhs_backup::{Header, KdfParams, Kind, HEADER_LEN};
use bruhs_models::backup::{
    no_space, valid_id, validate_password, BackupJob, BackupOptions, BackupOverview, BackupSizes,
    JobState, JobStep, LastBackup, PendingRestore, PreRestoreInfo, StagedRestore, BACKUP_BUSY,
    INVALID_REQUEST, KEYCHAIN_MAX_BYTES, MANIFEST_MAX_BYTES, MAX_ENTRIES, NOT_A_BACKUP,
    RECORDING_FIRST, RESTORE_PENDING, UNSUPPORTED_PLATFORM, WEB_TOO_LARGE, WEB_UPLOAD_MAX,
};
use futures_util::future::BoxFuture;
use rand::TryRngCore as _;
use tokio::sync::watch;

use super::create::{self, CancelFlag, CreateRequest};
use super::{free_bytes, inventory, BackupError, RootKey, Roots, FREE_MARGIN};
use crate::local_tools::LocalToolsHost;
use crate::reactive::ReactiveRuntime;
use crate::secrets::{keychain_inventory, SecretStore, VAULT_READ_WAIT};

/// Folders inside `<core>/backups`.
const OUTGOING: &str = "outgoing";
const INCOMING: &str = "incoming";
const TMP: &str = "tmp";
/// The restore a start finishes (Task 8 writes it).
pub(super) const PENDING: &str = "pending.json";

/// The last backup made here.
const LAST: &str = "last.json";
/// A download or upload left this long is swept.
const TRANSFER_TTL: Duration = Duration::from_secs(30 * 60);
/// A restore upload checks the free space each time this much more
/// arrived.
const SPACE_CHECK_BYTES: u64 = 64 << 20;
/// `last.json` is a few dozen bytes; anything larger isn't ours.
const LAST_MAX_BYTES: u64 = 4096;
/// How long a bruh's browser may take to close before the backup leaves
/// its profile out (D6).
const BROWSER_WAIT: Duration = Duration::from_secs(30);
/// How long the capture daemon may take to say whether it records.
const RECORDING_WAIT: Duration = Duration::from_secs(2);

/// Whether the capture daemon is recording a meeting now (ruling F24).
type RecordingProbe = Arc<dyn Fn() -> BoxFuture<'static, bool> + Send + Sync>;

/// Makes backups and takes uploads. Clones share one job slot.
#[derive(Clone)]
pub struct BackupHost {
    config: Arc<Config>,
    shared: Arc<Shared>,
}

/// What a job reads; fixed once the host is built.
#[derive(Clone)]
pub(super) struct Config {
    pub(super) roots: Roots,
    pub(super) store: SecretStore,
    pub(super) tools: Option<Arc<LocalToolsHost>>,
    pub(super) reactive: Option<ReactiveRuntime>,
    recording: RecordingProbe,
    pub(super) kdf: KdfParams,
    pub(super) keychain_wait: Duration,
    pub(super) browser_wait: Duration,
    /// The release written into each manifest.
    pub(super) app_version: String,
    /// The most entries and manifest bytes a backup may hold, checked
    /// alike by making and opening one.
    pub(super) limits: Limits,
    /// Called on the writer's thread at each [`Hook`] of a data file, so a
    /// test can hold a job or change a file mid-write.
    #[cfg(test)]
    pub(super) before_file: Option<TestHook>,
    /// Answers in place of the volume's free space, so a test can fill the
    /// disk.
    #[cfg(test)]
    pub(super) free_space: Option<FreeSpace>,
}

#[cfg(test)]
pub(super) type FreeSpace = Arc<dyn Fn(&Path) -> u64 + Send + Sync>;

/// What one backup may hold (B Task 7 I1). Making a backup refuses one
/// that wouldn't open, so the two sides never disagree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Limits {
    /// Tar entries: files, folders and the three parts beside the data.
    /// GNU long-name headers don't count.
    pub(super) entries: u64,
    /// `manifest.json`'s bytes.
    pub(super) manifest_bytes: u64,
    /// `keychain.json`'s bytes (B Task 7 N1).
    pub(super) keychain_bytes: u64,
}

impl Limits {
    pub(super) const BACKUP: Self = Self {
        entries: MAX_ENTRIES,
        manifest_bytes: MANIFEST_MAX_BYTES,
        keychain_bytes: KEYCHAIN_MAX_BYTES,
    };
}

impl Config {
    /// The free bytes on the volume of `folder` (a test's answer when it
    /// set one).
    pub(super) fn free_bytes(&self, folder: &Path) -> io::Result<u64> {
        #[cfg(test)]
        if let Some(free) = &self.free_space {
            return Ok(free(folder));
        }
        free_bytes(folder)
    }
}

/// Where the writer calls a test's hook, with the entry's path.
#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Hook {
    /// Before the file is opened.
    BeforeFile,
    /// Between a first open whose size differs from the walk's and the
    /// second open.
    Reopening,
    /// After it was opened and its size taken, before it is read.
    Opened,
    /// After it was read, before its size is checked again.
    Read,
}

#[cfg(test)]
pub(super) type TestHook = Arc<dyn Fn(Hook, &str) + Send + Sync>;

#[derive(Default)]
struct Shared {
    /// The running job, or the last one until the next starts.
    job: Mutex<Option<JobSlot>>,
}

struct JobSlot {
    id: String,
    state: watch::Receiver<JobState>,
    cancel: CancelFlag,
}

impl JobSlot {
    fn running(&self) -> bool {
        matches!(*self.state.borrow(), JobState::Running { .. })
    }
}

impl BackupHost {
    pub fn new(roots: Roots, store: SecretStore) -> Self {
        let recording: RecordingProbe = Arc::new(|| Box::pin(daemon_recording()));
        Self {
            config: Arc::new(Config {
                roots,
                store,
                tools: None,
                reactive: None,
                recording,
                kdf: KdfParams::NATIVE,
                keychain_wait: VAULT_READ_WAIT,
                browser_wait: BROWSER_WAIT,
                app_version: env!("CARGO_PKG_VERSION").to_owned(),
                limits: Limits::BACKUP,
                #[cfg(test)]
                before_file: None,
                #[cfg(test)]
                free_space: None,
            }),
            shared: Arc::default(),
        }
    }

    /// Closes the bruhs' browsers before a backup that takes their
    /// profiles.
    pub fn with_computers(mut self, tools: Arc<LocalToolsHost>) -> Self {
        Arc::make_mut(&mut self.config).tools = Some(tools);
        self
    }

    /// Names bruhs in warnings and counts them in the manifest.
    pub fn with_reactive(mut self, runtime: ReactiveRuntime) -> Self {
        Arc::make_mut(&mut self.config).reactive = Some(runtime);
        self
    }

    /// The release each manifest names, as the app reports it (its
    /// release version, else its crate version). Bruhs-core's own crate
    /// version until set.
    pub fn with_app_version(mut self, version: String) -> Self {
        Arc::make_mut(&mut self.config).app_version = version;
        self
    }

    /// Fast key derivation, short waits and a daemon that answers as told.
    #[cfg(any(test, feature = "test-support"))]
    pub(super) fn for_tests(mut self, recording: bool) -> Self {
        let config = Arc::make_mut(&mut self.config);
        config.kdf = KdfParams::FLOOR;
        config.keychain_wait = Duration::from_millis(300);
        config.browser_wait = Duration::from_millis(300);
        config.recording = Arc::new(move || Box::pin(async move { recording }));
        self
    }

    #[cfg(test)]
    pub(super) fn with_before_file(mut self, hook: TestHook) -> Self {
        Arc::make_mut(&mut self.config).before_file = Some(hook);
        self
    }

    /// Smaller limits, so a test can reach them.
    #[cfg(test)]
    pub(super) fn with_limits(mut self, limits: Limits) -> Self {
        Arc::make_mut(&mut self.config).limits = limits;
        self
    }

    #[cfg(test)]
    pub(super) fn with_free_space(mut self, free: FreeSpace) -> Self {
        Arc::make_mut(&mut self.config).free_space = Some(free);
        self
    }

    pub fn roots(&self) -> &Roots {
        &self.config.roots
    }

    pub(super) fn config(&self) -> &Config {
        &self.config
    }

    /// 16 fresh OS-random bytes as 32 lowercase hex characters.
    pub fn random_id(&self) -> String {
        random_id()
    }

    /// Starts making a backup. Refused while a job runs (`BACKUP_BUSY`),
    /// while a restore waits (`RESTORE_PENDING`), while a meeting records
    /// (`RECORDING_FIRST`, ruling F24) and for a password or destination
    /// that can't work.
    pub async fn start_create(&self, request: CreateRequest) -> Result<BackupJob, BackupError> {
        if !cfg!(any(target_os = "macos", target_os = "linux")) {
            return Err(BackupError::Refused(UNSUPPORTED_PLATFORM));
        }
        validate_password(&request.password).map_err(BackupError::Refused)?;
        if request.web_part.is_some() && request.web_db_version.is_none() {
            return Err(BackupError::Refused(INVALID_REQUEST));
        }
        if let create::Output::File { path, overwrite } = &request.output {
            create::check_destination(self.roots(), path, *overwrite)?;
        }
        create::backups_are_folders(self.roots())?;
        if self.running() {
            return Err(BackupError::Busy(BACKUP_BUSY));
        }
        if self.restore_pending() {
            return Err(BackupError::Refused(RESTORE_PENDING));
        }
        if (self.config.recording)().await {
            return Err(BackupError::Refused(RECORDING_FIRST));
        }

        let host = self.clone();
        self.launch(JobStep::Collecting, move |_, progress, cancel| async move {
            match create::create(&host, request, &progress, &cancel).await {
                Ok(file) => JobState::Created { file },
                Err(error) => JobState::Failed {
                    message: error.message(),
                },
            }
        })
    }

    /// Runs `work` as the one job, unless a job runs (`BACKUP_BUSY`).
    /// `work` gets the job's id, its progress and its cancel flag, and
    /// answers the job's end state; a job whose task stops without one
    /// ends `Failed`.
    pub(super) fn launch<W, F>(&self, step: JobStep, work: W) -> Result<BackupJob, BackupError>
    where
        W: FnOnce(String, watch::Sender<JobState>, CancelFlag) -> F,
        F: Future<Output = JobState> + Send + 'static,
    {
        let id = random_id();
        let first = JobState::Running {
            done_bytes: 0,
            total_bytes: 0,
            step,
        };
        let (progress, state) = watch::channel(first.clone());
        let cancel = CancelFlag::default();
        {
            let mut job = self.lock_job();
            if job.as_ref().is_some_and(JobSlot::running) {
                return Err(BackupError::Busy(BACKUP_BUSY));
            }
            *job = Some(JobSlot {
                id: id.clone(),
                state,
                cancel: cancel.clone(),
            });
        }
        let work = work(id.clone(), progress.clone(), cancel);
        tokio::spawn(async move {
            let state = tokio::spawn(work)
                .await
                .unwrap_or_else(|_| JobState::Failed {
                    message: BackupError::Io(io::Error::other("the backup job stopped")).message(),
                });
            progress.send_replace(state);
        });
        Ok(BackupJob { id, state: first })
    }

    /// A job's state, while it runs and after, until the next job starts.
    pub fn job(&self, id: &str) -> Option<BackupJob> {
        self.lock_job()
            .as_ref()
            .filter(|slot| slot.id == id)
            .map(|slot| BackupJob {
                id: slot.id.clone(),
                state: slot.state.borrow().clone(),
            })
    }

    /// Stops a running job; it removes its partial file and ends `Failed`.
    pub fn cancel(&self, id: &str) -> bool {
        let job = self.lock_job();
        match job.as_ref() {
            Some(slot) if slot.id == id && slot.running() => {
                slot.cancel.cancel();
                true
            }
            _ => false,
        }
    }

    /// Whether a job runs now.
    pub fn running(&self) -> bool {
        self.lock_job().as_ref().is_some_and(JobSlot::running)
    }

    /// The running job's id.
    pub(super) fn running_id(&self) -> Option<String> {
        self.lock_job()
            .as_ref()
            .filter(|slot| slot.running())
            .map(|slot| slot.id.clone())
    }

    fn lock_job(&self) -> std::sync::MutexGuard<'_, Option<JobSlot>> {
        self.shared
            .job
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// `<core>/backups`, or one of its folders, when it is a real folder:
    /// nothing is read or removed through a link (B Task 5 I1 ruling).
    pub(super) fn private_folder(&self, folder: Option<&str>) -> Option<PathBuf> {
        self.private_folder_in(RootKey::Core, folder)
    }

    /// [`Self::private_folder`] in either root.
    pub(super) fn private_folder_in(&self, root: RootKey, folder: Option<&str>) -> Option<PathBuf> {
        let real =
            |path: &Path| std::fs::symlink_metadata(path).is_ok_and(|metadata| metadata.is_dir());
        let backups = self.roots().backups(root);
        if !real(&backups) {
            return None;
        }
        match folder {
            None => Some(backups),
            Some(name) => Some(backups.join(name)).filter(|path| real(path)),
        }
    }

    /// Whether a restore waits. A `pending.json` seen through a linked
    /// `backups` folder doesn't count; the job then refuses that folder.
    pub(super) fn restore_pending(&self) -> bool {
        self.private_folder(None)
            .is_some_and(|backups| std::fs::symlink_metadata(backups.join(PENDING)).is_ok())
    }

    /// A finished web download's file.
    pub fn outgoing(&self, id: &str) -> Option<PathBuf> {
        if !valid_id(id) {
            return None;
        }
        let path = self
            .private_folder(Some(OUTGOING))?
            .join(format!("{id}.bruhsbak"));
        std::fs::symlink_metadata(&path)
            .is_ok_and(|metadata| metadata.is_file())
            .then_some(path)
    }

    /// Removes a download once it was sent in full.
    pub fn finish_download(&self, id: &str) {
        if let Some(path) = self.outgoing(id) {
            // Best effort: the sweep removes it later otherwise.
            let _ = std::fs::remove_file(path);
        }
    }

    pub(super) fn outgoing_dir(&self) -> PathBuf {
        self.roots().backups(RootKey::Core).join(OUTGOING)
    }

    pub(super) fn tmp_dir(&self) -> PathBuf {
        self.roots().backups(RootKey::Core).join(TMP)
    }

    fn incoming_dir(&self) -> PathBuf {
        self.roots().backups(RootKey::Core).join(INCOMING)
    }

    /// Starts an upload into `backups/incoming`. Feed it the body's frames.
    pub fn begin_upload(&self, kind: UploadKind) -> Result<Upload, BackupError> {
        let backups = self.roots().backups(RootKey::Core);
        create::private_dir(&backups)?;
        let folder = self.incoming_dir();
        create::private_dir(&folder)?;
        let id = random_id();
        let path = upload_path(&folder, kind, &id);
        let file = new_private_file(&path)?;
        Ok(Upload {
            kind,
            id,
            path,
            folder,
            file: Some(file),
            bytes: 0,
            next_check: 0,
            head: Vec::with_capacity(HEADER_LEN),
        })
    }

    /// A finished upload's file.
    pub fn uploaded(&self, kind: UploadKind, id: &str) -> Option<PathBuf> {
        if !valid_id(id) {
            return None;
        }
        let path = upload_path(&self.private_folder(Some(INCOMING))?, kind, id);
        std::fs::symlink_metadata(&path)
            .is_ok_and(|metadata| metadata.is_file())
            .then_some(path)
    }

    /// Removes downloads and uploads older than 30 minutes and, unless a
    /// job runs, every snapshot left in `backups/tmp`, the partial files a
    /// stopped process left in chosen folders, and staged restores that
    /// never finished or weren't applied within 24 hours, with their seal
    /// keys (ruling F25).
    ///
    /// The restore's part takes `restore.lock` and may remove keychain
    /// items (seal keys whose set is gone), so it can wait on the
    /// keychain: call it off the async runtime.
    pub fn sweep(&self) {
        let now = SystemTime::now();
        for folder in [OUTGOING, INCOMING] {
            if let Some(folder) = self.private_folder(Some(folder)) {
                sweep_folder(
                    &folder,
                    |modified| {
                        now.duration_since(modified)
                            .is_ok_and(|age| age > TRANSFER_TTL)
                    },
                    |_| true,
                );
            }
        }
        if self.running() {
            return;
        }
        if let Some(tmp) = self.private_folder(Some(TMP)) {
            remove_marked_partials(&tmp);
            sweep_folder(&tmp, |_| true, |_| true);
        }
        self.sweep_staged();
        // A `last.json` record that a stopped process never renamed.
        if let Some(backups) = self.private_folder(None) {
            let leftover = format!(".{LAST}.partial-");
            sweep_folder(
                &backups,
                |_| true,
                |name| name.strip_prefix(leftover.as_str()).is_some_and(valid_id),
            );
        }
        // What a stopped apply, rollback or delete left (B Task 8).
        super::apply::sweep_restore(self.roots(), &self.config().store);
    }

    /// The last backup made on this computer.
    pub fn last(&self) -> Option<LastBackup> {
        let path = self.private_folder(None)?.join(LAST);
        serde_json::from_slice(&read_small(&path, LAST_MAX_BYTES)?).ok()
    }

    /// Records the backup just made, replacing the record atomically.
    pub(super) fn record_last(&self, last: &LastBackup) -> Result<(), BackupError> {
        create::private_dir(&self.roots().backups(RootKey::Core))?;
        let backups = self
            .private_folder(None)
            .ok_or_else(|| BackupError::Io(io::Error::other("backups isn't a real folder")))?;
        let partial = backups.join(format!(".{LAST}.partial-{}", random_id()));
        let mut file = new_private_file(&partial)?;
        let written = serde_json::to_writer(&mut file, last)
            .map_err(io::Error::other)
            .and_then(|()| file.sync_all())
            .and_then(|()| std::fs::rename(&partial, backups.join(LAST)));
        if written.is_err() {
            let _ = std::fs::remove_file(&partial);
        }
        written.map_err(BackupError::from)
    }

    /// What Settings shows (D23, ruling F25): sizes, keychain items,
    /// outside roots, the last backup, the job, a waiting or staged
    /// restore, the kept sets and whether a browser part waits.
    pub async fn overview(&self) -> BackupOverview {
        let roots = self.roots().clone();
        let measured = tokio::task::spawn_blocking(move || {
            let sizes = inventory(&roots, &BackupOptions::default())
                .map(|taken| BackupSizes {
                    data_bytes: taken.data_bytes,
                    audio_bytes: taken.audio_bytes,
                    browser_bytes: taken.browser_bytes,
                })
                .unwrap_or_default();
            let items = keychain_inventory(
                &roots.core.join("credentials"),
                &roots.core.join("connections"),
                &roots.config.join("calendar").join("google"),
            )
            .map_or(0, |items| items.len());
            let restore = Restore {
                pending: super::pending(&roots),
                pre_restores: super::pre_restores(&roots),
                web_part_waiting: super::waiting_web_part(&roots).is_some(),
            };
            (sizes, u32::try_from(items).unwrap_or(u32::MAX), restore)
        })
        .await
        .unwrap_or_default();
        let job = self.lock_job().as_ref().map(|slot| BackupJob {
            id: slot.id.clone(),
            state: slot.state.borrow().clone(),
        });
        BackupOverview {
            supported: cfg!(any(target_os = "macos", target_os = "linux")),
            sizes: measured.0,
            keychain_items: measured.1,
            outside_roots: self
                .roots()
                .outside_overrides(|name| std::env::var_os(name)),
            last: self.last(),
            job,
            pending: measured.2.pending,
            staged: self.staged().map(|preview| StagedRestore { preview }),
            pre_restores: measured.2.pre_restores,
            web_part_waiting: measured.2.web_part_waiting,
        }
    }
}

/// The overview's restore fields, read off the async runtime.
#[derive(Default)]
struct Restore {
    pending: Option<PendingRestore>,
    pre_restores: Vec<PreRestoreInfo>,
    web_part_waiting: bool,
}

/// What an upload is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UploadKind {
    /// The browser's encrypted part, before it makes a backup.
    WebPart,
    /// A backup file to restore.
    Restore,
    /// The browser's data from before its own restore.
    PreviousWebPart,
}

impl UploadKind {
    fn extension(self) -> &'static str {
        match self {
            Self::WebPart => "web-part",
            Self::Restore => "bruhsbak",
            Self::PreviousWebPart => "previous-web-part",
        }
    }

    fn web(self) -> bool {
        matches!(self, Self::WebPart | Self::PreviousWebPart)
    }
}

fn upload_path(folder: &Path, kind: UploadKind, id: &str) -> PathBuf {
    folder.join(format!("{id}.{}", kind.extension()))
}

/// An upload being written. Dropped before [`Upload::finish`], it deletes
/// its file.
pub struct Upload {
    kind: UploadKind,
    id: String,
    path: PathBuf,
    folder: PathBuf,
    file: Option<File>,
    bytes: u64,
    next_check: u64,
    /// The first bytes, for a web part's header check.
    head: Vec<u8>,
}

impl Upload {
    /// Appends one body frame. A web part over `WEB_UPLOAD_MAX` is
    /// refused; a restore upload stops with `NO_SPACE` once less than
    /// `FREE_MARGIN` would be left (checked every 64 MiB).
    pub fn write(&mut self, chunk: &[u8]) -> Result<(), BackupError> {
        let added = u64::try_from(chunk.len()).map_err(io::Error::other)?;
        self.bytes = self.bytes.saturating_add(added);
        if self.kind.web() && self.bytes > WEB_UPLOAD_MAX {
            return Err(BackupError::Refused(WEB_TOO_LARGE));
        }
        if self.kind == UploadKind::Restore && self.bytes >= self.next_check {
            let free = free_bytes(&self.folder)?;
            if free < FREE_MARGIN.saturating_add(added) {
                return Err(BackupError::Space(no_space(
                    FREE_MARGIN.saturating_add(added).saturating_sub(free),
                )));
            }
            self.next_check = self.bytes.saturating_add(SPACE_CHECK_BYTES);
        }
        let room = HEADER_LEN.saturating_sub(self.head.len());
        self.head.extend_from_slice(&chunk[..room.min(chunk.len())]);
        let file = self
            .file
            .as_mut()
            .ok_or_else(|| io::Error::other("the upload has ended"))?;
        file.write_all(chunk)?;
        Ok(())
    }

    /// Ends the upload and answers its id. A web part must start with a
    /// web part header.
    pub fn finish(mut self) -> Result<String, BackupError> {
        if self.kind.web() && !web_part_header(&self.head) {
            return Err(BackupError::Format(NOT_A_BACKUP));
        }
        let file = self
            .file
            .take()
            .ok_or_else(|| io::Error::other("the upload has ended"))?;
        file.sync_all()?;
        drop(file);
        Ok(std::mem::take(&mut self.id))
    }
}

impl Drop for Upload {
    fn drop(&mut self) {
        if self.file.take().is_some() {
            // Best effort: the sweep removes it later otherwise.
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

/// Whether `head` is a web part's header.
pub(super) fn web_part_header(head: &[u8]) -> bool {
    <&[u8; HEADER_LEN]>::try_from(head)
        .ok()
        .and_then(|bytes| Header::parse(bytes).ok())
        .is_some_and(|header| header.kind == Kind::WebPart)
}

/// A small regular file's bytes, read without following a link or waiting
/// on a FIFO; `None` when it is missing, not a regular file or over `max`
/// bytes.
pub(super) fn read_small(path: &Path, max: u64) -> Option<Vec<u8>> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
        .ok()?;
    let metadata = file.metadata().ok()?;
    if !metadata.is_file() || metadata.len() > max {
        return None;
    }
    let mut text = Vec::new();
    file.take(max).read_to_end(&mut text).ok()?;
    Some(text)
}

/// Creates a new 0600 file, never replacing one or following a link.
pub(super) fn new_private_file(path: &Path) -> io::Result<File> {
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
}

/// 16 fresh OS-random bytes as 32 lowercase hex characters.
pub(super) fn random_id() -> String {
    let mut bytes = [0_u8; 16];
    random_bytes(&mut bytes);
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Fills `bytes` from the operating system's random source.
pub(super) fn random_bytes(bytes: &mut [u8]) {
    rand::rngs::OsRng
        .try_fill_bytes(bytes)
        .expect("the operating system's random source answers");
}

/// Each marker in `tmp` names a partial backup file a job was writing
/// (`<id>.partial-path`), with that file's device and inode. A job that
/// ended removes its marker; one left means the process stopped. The file
/// it names is removed only when it still is a regular file with the
/// partial name a job gives it and the very inode the job made (ruling B
/// Task 6 N1). A marker is read without following a link or waiting on a
/// FIFO, and only when it is a small regular file.
fn remove_marked_partials(tmp: &Path) {
    use std::os::unix::fs::MetadataExt as _;
    let Ok(entries) = std::fs::read_dir(tmp) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let marker = name
            .to_str()
            .and_then(|name| name.strip_suffix(PARTIAL_MARKER))
            .is_some_and(valid_id);
        if !marker {
            continue;
        }
        let Some((dev, ino, partial)) = read_marker(&entry.path()) else {
            continue;
        };
        let ours = partial.is_absolute()
            && partial.file_name().is_some_and(partial_name)
            && std::fs::symlink_metadata(&partial).is_ok_and(|metadata| {
                metadata.is_file() && metadata.dev() == dev && metadata.ino() == ino
            });
        if ours {
            let _ = std::fs::remove_file(&partial);
        }
        let _ = std::fs::remove_file(entry.path());
    }
}

/// The longest marker read: a path of up to `PATH_MAX` and its identity.
const MARKER_MAX_BYTES: u64 = 4096 + 64;

/// A marker's text: `<dev> <ino>`, a newline, then the partial file's path
/// as raw bytes.
pub(super) fn marker_text(dev: u64, ino: u64, partial: &Path) -> Vec<u8> {
    use std::os::unix::ffi::OsStrExt as _;
    let mut text = format!("{dev} {ino}\n").into_bytes();
    text.extend_from_slice(partial.as_os_str().as_bytes());
    text
}

/// Reads a marker: a regular file, opened without following a link or
/// blocking, at most [`MARKER_MAX_BYTES`].
fn read_marker(path: &Path) -> Option<(u64, u64, PathBuf)> {
    use std::os::unix::ffi::OsStrExt as _;
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
        .ok()?;
    let metadata = file.metadata().ok()?;
    if !metadata.is_file() || metadata.len() > MARKER_MAX_BYTES {
        return None;
    }
    let mut text = Vec::new();
    file.take(MARKER_MAX_BYTES).read_to_end(&mut text).ok()?;
    let split = text.iter().position(|byte| *byte == b'\n')?;
    let (identity, partial) = (
        std::str::from_utf8(&text[..split]).ok()?,
        &text[split + 1..],
    );
    let (dev, ino) = identity.split_once(' ')?;
    Some((
        dev.parse().ok()?,
        ino.parse().ok()?,
        PathBuf::from(std::ffi::OsStr::from_bytes(partial)),
    ))
}

/// A marker's suffix in `backups/tmp`.
pub(super) const PARTIAL_MARKER: &str = ".partial-path";

/// Whether `name` is `.<name>.partial-<32 hex>`, as a job names the file
/// it writes beside a chosen destination.
fn partial_name(name: &std::ffi::OsStr) -> bool {
    name.to_str().is_some_and(|name| {
        name.starts_with('.')
            && name
                .rsplit_once(".partial-")
                .is_some_and(|(head, id)| head.len() > 1 && valid_id(id))
    })
}

/// Removes the regular files in `folder` whose UTF-8 name `named` and
/// change time `expired` accept. Links, FIFOs and folders stay.
pub(super) fn sweep_folder(
    folder: &Path,
    expired: impl Fn(SystemTime) -> bool,
    named: impl Fn(&str) -> bool,
) {
    let Ok(entries) = std::fs::read_dir(folder) else {
        return;
    };
    for entry in entries.flatten() {
        if !entry.file_name().to_str().is_some_and(&named) {
            continue;
        }
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        let changed = metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH);
        if metadata.is_file() && expired(changed) {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

/// Whether the capture daemon records now. One that can't be reached in
/// time isn't recording (ruling F24).
async fn daemon_recording() -> bool {
    tokio::time::timeout(RECORDING_WAIT, crate::capture_client::fetch_status())
        .await
        .is_ok_and(|status| status.is_ok_and(|status| status.recording))
}

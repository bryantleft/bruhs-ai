//! Opening a backup to restore (D8, D9, D11, D13; rulings F16, F25, F32
//! and the B Task 3 notes): check the password, every chunk, every entry's
//! path and type, the tar end, every size and hash and the manifest, while
//! writing nothing outside a fresh `backups/staging/<id>/` in each root.
//! Then seal the keychain items and answer what the backup holds.
//!
//! The backup file is hostile input: anyone who knows its password can
//! craft one. No size it names decides an allocation, nothing follows or
//! makes a link, and `tar::Entry::unpack` is never used. Any failure
//! removes the staging folders and changes nothing else.
//!
//! Staged data files are plain, as sensitive as the live data they replace
//! (ruling F32). Keychain values never are: they wait sealed in
//! `meta/keychain.sealed`, the key at `bruhs-ai-backup`/`staged:<id>`.

use std::cell::Cell;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fs::{DirBuilder, File, OpenOptions, Permissions};
use std::io::{self, Read, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::path::{Component, Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, SystemTime};

use bruhs_backup::{Decryptor, FormatError, Kind, Secret, HEADER_LEN};
use bruhs_models::backup::{
    excluded, folded_entry_key, no_space, normalize_password, safe_entry_path, valid_id,
    validate_password, BackupJob, BackupManifest, EntryPath, Excluded, JobState, JobStep,
    PendingRestore, RestorePreview, RestoreSource, BACKUP_BUSY, BACKUP_STOPPED, DAMAGED,
    DESKTOP_ENTRIES, GONE, KEYCHAIN_NEEDS_YOU, MAX_ENTRIES, NAMES_CLASH, NEWER_BACKUP,
    NOT_A_BACKUP, RESTORE_PENDING, ROOTS_OVERLAP, SOURCE_MISSING, UNSAFE_ENTRY,
    UNSUPPORTED_PLATFORM, WRONG_PASSWORD,
};
use sha2::{Digest, Sha256};
use tokio::sync::watch;
use zeroize::Zeroizing;

use super::create::{backups_are_folders, private_dir, CancelFlag};
use super::host::{
    new_private_file, random_bytes, read_small, web_part_header, BackupHost, PENDING,
};
use super::{BackupError, RootKey, Roots, FREE_MARGIN};
use crate::secrets::{forget_seal_key, seal_items, store_seal_key, KeychainItem};

/// The folder inside each root's `backups` that holds staged restores.
const STAGING: &str = "staging";
/// The folder inside the core staging that holds what isn't data. A backup
/// entry can't take its name.
const META: &str = "meta";
const PREVIEW_FILE: &str = "preview.json";
const MANIFEST_FILE: &str = "manifest.json";
const SEALED_FILE: &str = "keychain.sealed";
const WEB_PART_FILE: &str = "web-part.bin";
/// `keychain.json` is held in memory, so it has a cap (D8).
const KEYCHAIN_MAX: u64 = 4 << 20;
/// `manifest.json` is held in memory, so it has a cap (D8).
const MANIFEST_MAX: u64 = 64 << 20;
/// A GNU long name holds one entry path and its NUL.
const LONG_NAME_MAX: u64 = 4096 + 1;
/// A tar block.
const BLOCK: usize = 512;
/// What may follow the tar's first end block: the second, and the padding
/// of a writer that fills whole records (ruling F16).
const TAIL_MAX: usize = 10 * 1024;
/// A staged restore not applied in this time goes at the next start (D8).
const STAGED_TTL: Duration = Duration::from_secs(24 * 60 * 60);
const PREVIEW_MAX: u64 = 1 << 20;
const PENDING_MAX: u64 = 64 << 10;
/// Bytes read between two progress updates.
const PROGRESS_STEP: u64 = 1 << 20;
const COPY_BUFFER: usize = 64 << 10;
/// The modes a staged entry gets: a file, an owner-runnable file (ruling
/// F32) and a folder. Nothing else of the header's mode carries over.
const PRIVATE: u32 = 0o600;
const RUNNABLE: u32 = 0o700;
const FOLDER: u32 = 0o700;

/// A verified backup in its staging folders, before its items are sealed.
/// Holds keychain values: deliberately neither `Debug` nor `Clone`.
pub(crate) struct Staged {
    pub id: String,
    pub manifest: BackupManifest,
    pub items: Vec<KeychainItem>,
}

/// `<root>/backups/staging/<id>`.
pub(super) fn staging_dir(roots: &Roots, root: RootKey, id: &str) -> PathBuf {
    roots.backups(root).join(STAGING).join(id)
}

/// The roots that get a staging folder: the core root, and the desktop
/// root unless both are one folder.
fn staging_roots(roots: &Roots) -> &'static [RootKey] {
    if roots.same() {
        &[RootKey::Core]
    } else {
        &[RootKey::Core, RootKey::Desktop]
    }
}

/// The keychain account of a staged restore's seal key.
fn seal_account(id: &str) -> String {
    format!("staged:{id}")
}

fn damaged() -> BackupError {
    BackupError::Format(DAMAGED)
}

fn unsafe_entry() -> BackupError {
    BackupError::Format(UNSAFE_ENTRY)
}

fn stopped() -> BackupError {
    BackupError::Refused(BACKUP_STOPPED)
}

impl BackupHost {
    /// Starts opening the backup at `source` with `password` (D8): one job
    /// at a time with making one (`BACKUP_BUSY`), none while a restore
    /// waits (`RESTORE_PENDING`). The job ends `Opened` with the preview,
    /// or `Failed` with nothing staged. With `remove_after`, the file (an
    /// upload) is removed when the job ends either way.
    pub fn start_open(
        &self,
        source: PathBuf,
        password: Zeroizing<String>,
        remove_after: bool,
    ) -> Result<BackupJob, BackupError> {
        if !cfg!(any(target_os = "macos", target_os = "linux")) {
            return Err(BackupError::Refused(UNSUPPORTED_PLATFORM));
        }
        validate_password(&password).map_err(BackupError::Refused)?;
        backups_are_folders(self.roots())?;
        if self.running() {
            return Err(BackupError::Busy(BACKUP_BUSY));
        }
        if self.restore_pending() {
            return Err(BackupError::Refused(RESTORE_PENDING));
        }
        let host = self.clone();
        self.launch(JobStep::Checking, move |id, progress, cancel| async move {
            match open(
                &host,
                &id,
                source,
                password,
                remove_after,
                &progress,
                &cancel,
            )
            .await
            {
                Ok(preview) => JobState::Opened {
                    preview: Box::new(preview),
                },
                Err(error) => JobState::Failed {
                    message: error.message(),
                },
            }
        })
    }

    /// The staged restore waiting for the owner's confirmation: the newest
    /// whose staging finished.
    pub fn staged(&self) -> Option<RestorePreview> {
        let staging = self.private_folder(Some(STAGING))?;
        self.staged_ids()
            .into_iter()
            .filter_map(|id| finished_staging(&staging.join(&id), &id))
            .max_by_key(|(changed, _)| *changed)
            .map(|(_, preview)| preview)
    }

    /// Removes a staged restore: both roots' folders and its seal key.
    /// Refused while its job runs or a restore waits on it; an id that
    /// isn't staged is `GONE`.
    pub fn discard_staged(&self, id: &str) -> Result<(), BackupError> {
        if !valid_id(id) {
            return Err(BackupError::Refused(GONE));
        }
        if self.running_id().as_deref() == Some(id) {
            return Err(BackupError::Busy(BACKUP_BUSY));
        }
        if self.pending_staged().keeps(id) {
            return Err(BackupError::Refused(RESTORE_PENDING));
        }
        let found = self.remove_staged(id);
        forget_seal_key(&self.config().store, &seal_account(id))
            .map_err(|_| BackupError::Keychain(KEYCHAIN_NEEDS_YOU))?;
        if found {
            Ok(())
        } else {
            Err(BackupError::Refused(GONE))
        }
    }

    /// Removes every staged restore that never finished staging or wasn't
    /// applied within 24 hours, with its seal key (D8, ruling F25). Called
    /// from [`BackupHost::sweep`] when no job runs. A restore that
    /// `pending.json` names stays.
    pub(super) fn sweep_staged(&self) {
        let pending = self.pending_staged();
        let now = SystemTime::now();
        let staging = self.private_folder(Some(STAGING));
        for id in self.staged_ids() {
            if pending.keeps(&id) {
                continue;
            }
            let fresh = staging
                .as_ref()
                .and_then(|staging| finished_staging(&staging.join(&id), &id))
                .is_some_and(|(changed, _)| {
                    now.duration_since(changed)
                        .map_or(true, |age| age <= STAGED_TTL)
                });
            if !fresh {
                self.forget_staged(&id);
            }
        }
    }

    /// Removes every staged restore but `keep`, so one waits at a time.
    fn discard_others(&self, keep: &str) {
        let pending = self.pending_staged();
        for id in self.staged_ids() {
            if id != keep && !pending.keeps(&id) {
                self.forget_staged(&id);
            }
        }
    }

    /// Removes a staged restore's folders and key, as far as it can.
    fn forget_staged(&self, id: &str) {
        self.remove_staged(id);
        if forget_seal_key(&self.config().store, &seal_account(id)).is_err() {
            tracing::warn!("Bruhs couldn't remove a staged restore's key");
        }
    }

    /// The ids with a staging folder in either root.
    fn staged_ids(&self) -> BTreeSet<String> {
        let mut ids = BTreeSet::new();
        for root in staging_roots(self.roots()) {
            let Some(staging) = self.private_folder_in(*root, Some(STAGING)) else {
                continue;
            };
            let Ok(entries) = std::fs::read_dir(staging) else {
                continue;
            };
            for entry in entries.flatten() {
                let folder = entry.file_type().is_ok_and(|kind| kind.is_dir());
                if let Some(id) = entry.file_name().to_str().filter(|id| valid_id(id)) {
                    if folder {
                        ids.insert(id.to_owned());
                    }
                }
            }
        }
        ids
    }

    /// Removes `id`'s staging folder in each root, never through a link.
    /// Answers whether one was there.
    fn remove_staged(&self, id: &str) -> bool {
        let mut found = false;
        for root in staging_roots(self.roots()) {
            let Some(staging) = self.private_folder_in(*root, Some(STAGING)) else {
                continue;
            };
            let folder = staging.join(id);
            if std::fs::symlink_metadata(&folder).is_ok_and(|metadata| metadata.is_dir()) {
                found = true;
                if std::fs::remove_dir_all(&folder).is_err() {
                    tracing::warn!("Bruhs couldn't remove a staged restore");
                }
            }
        }
        found
    }

    /// The staged restore `pending.json` names.
    fn pending_staged(&self) -> Pending {
        let Some(path) = self
            .private_folder(None)
            .map(|backups| backups.join(PENDING))
        else {
            return Pending::None;
        };
        if std::fs::symlink_metadata(&path).is_err() {
            return Pending::None;
        }
        let pending = read_small(&path, PENDING_MAX)
            .and_then(|text| serde_json::from_slice::<PendingRestore>(&text).ok());
        match pending.map(|pending| pending.source) {
            Some(RestoreSource::Staged { id }) => Pending::Staged(id),
            Some(RestoreSource::PreRestore { .. }) => Pending::None,
            // A restore this build can't read keeps every staged set.
            Some(RestoreSource::Other) | None => Pending::Unknown,
        }
    }
}

/// What a waiting restore needs kept.
enum Pending {
    None,
    Staged(String),
    Unknown,
}

impl Pending {
    fn keeps(&self, id: &str) -> bool {
        match self {
            Self::None => false,
            Self::Staged(pending) => pending == id,
            Self::Unknown => true,
        }
    }
}

/// A staging folder whose staging finished: its preview, and when the
/// preview was written.
fn finished_staging(folder: &Path, id: &str) -> Option<(SystemTime, RestorePreview)> {
    let meta = folder.join(META);
    if !std::fs::symlink_metadata(&meta).is_ok_and(|metadata| metadata.is_dir()) {
        return None;
    }
    let path = meta.join(PREVIEW_FILE);
    let text = read_small(&path, PREVIEW_MAX)?;
    let changed = std::fs::symlink_metadata(&path).ok()?.modified().ok()?;
    let preview: RestorePreview = serde_json::from_slice(&text).ok()?;
    (preview.id == id).then_some((changed, preview))
}

/// Opens the backup at `source` into staging `id` and answers its preview.
/// Every failure removes what it staged; the upload goes either way when
/// `remove_after`.
async fn open(
    host: &BackupHost,
    id: &str,
    source: PathBuf,
    password: Zeroizing<String>,
    remove_after: bool,
    progress: &watch::Sender<JobState>,
    cancel: &CancelFlag,
) -> Result<RestorePreview, BackupError> {
    let _upload = remove_after.then(|| RemoveWhenDone(source.clone()));
    let config = host.config().clone();
    let roots = config.roots.clone();

    // D8: room for the file's size plus the margin, before anything is
    // staged.
    let bytes = match std::fs::metadata(&source) {
        Ok(metadata) if metadata.is_file() => metadata.len(),
        Ok(_) => return Err(BackupError::Format(NOT_A_BACKUP)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Err(BackupError::Refused(SOURCE_MISSING))
        }
        Err(error) => return Err(error.into()),
    };
    backups_are_folders(&roots)?;
    let needed = bytes.saturating_add(FREE_MARGIN);
    for root in staging_roots(&roots) {
        let backups = roots.backups(*root);
        private_dir(&backups)?;
        let free = config.free_bytes(&backups)?;
        if free < needed {
            return Err(BackupError::Space(no_space(needed - free)));
        }
    }

    let discard = Discard {
        host: host.clone(),
        id: id.to_owned(),
        armed: true,
    };
    let staged = {
        let (roots, id, cancel, progress) = (
            roots.clone(),
            id.to_owned(),
            cancel.clone(),
            progress.clone(),
        );
        tokio::task::spawn_blocking(move || {
            let report = |done_bytes, total_bytes| {
                progress.send_replace(JobState::Running {
                    done_bytes,
                    total_bytes,
                    step: JobStep::Checking,
                });
            };
            extract(&roots, &id, &source, &password, &cancel, &report)
        })
        .await
        .map_err(|_| worker_stopped())??
    };
    // Task 9 prepares the staged data for this computer here.
    let preview = tokio::task::spawn_blocking(move || seal(&config, &staged))
        .await
        .map_err(|_| worker_stopped())??;
    discard.disarm();
    host.discard_others(id);
    Ok(preview)
}

fn worker_stopped() -> BackupError {
    BackupError::Io(io::Error::other("the open worker stopped"))
}

/// Removes an upload once its open ended.
struct RemoveWhenDone(PathBuf);

impl Drop for RemoveWhenDone {
    fn drop(&mut self) {
        // Best effort: the sweep removes an upload later otherwise.
        let _ = std::fs::remove_file(&self.0);
    }
}

/// Removes a staged restore and its key unless the open finished.
struct Discard {
    host: BackupHost,
    id: String,
    armed: bool,
}

impl Discard {
    fn disarm(mut self) {
        self.armed = false;
    }
}

impl Drop for Discard {
    fn drop(&mut self) {
        if self.armed {
            self.host.forget_staged(&self.id);
        }
    }
}

/// Seals the staged items (D13) under a fresh key kept at `staged:<id>`,
/// writes the manifest and, last, the preview, and answers the preview.
fn seal(config: &super::host::Config, staged: &Staged) -> Result<RestorePreview, BackupError> {
    let folder = staging_dir(&config.roots, RootKey::Core, &staged.id);
    let meta = folder.join(META);
    private_dir(&meta)?;
    if staged.manifest.options.keychain {
        let mut key = Zeroizing::new([0_u8; 32]);
        random_bytes(&mut *key);
        let mut random = [0_u8; 23];
        random_bytes(&mut random);
        let blob = seal_items(&staged.items, random, *key).map_err(|_| damaged())?;
        write_new(&meta.join(SEALED_FILE), &blob)?;
        store_seal_key(&config.store, &seal_account(&staged.id), &key)
            .map_err(|_| BackupError::Keychain(KEYCHAIN_NEEDS_YOU))?;
    }
    let manifest = serde_json::to_vec(&staged.manifest).map_err(io::Error::other)?;
    write_new(&meta.join(MANIFEST_FILE), &manifest)?;
    let preview = preview_of(&staged.id, &staged.manifest);
    let text = serde_json::to_vec(&preview).map_err(io::Error::other)?;
    write_new(&meta.join(PREVIEW_FILE), &text)?;
    for synced in [&meta, &folder] {
        // Best effort: makes the new names durable where folders can sync.
        let _ = File::open(synced).and_then(|folder| folder.sync_all());
    }
    Ok(preview)
}

/// Writes a new 0600 file and syncs it.
fn write_new(path: &Path, bytes: &[u8]) -> Result<(), BackupError> {
    let mut file = new_private_file(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}

/// What the manifest says the backup holds. Task 9 adds the rewrites and
/// the outside folders.
fn preview_of(id: &str, manifest: &BackupManifest) -> RestorePreview {
    RestorePreview {
        id: id.to_owned(),
        made_at_ms: manifest.made_at_ms,
        made_on: manifest.made_on,
        app_version: manifest.app_version.clone(),
        source_home: manifest.home.clone(),
        rewrites: Vec::new(),
        outside_home: Vec::new(),
        counts: manifest.counts,
        bytes: manifest
            .files
            .iter()
            .map(|file| file.bytes)
            .fold(0, u64::saturating_add),
        has_web_part: manifest.web_db_version.is_some(),
        has_keychain: manifest.options.keychain,
        has_audio: manifest.options.audio,
        has_browser_profiles: manifest.options.browser_profiles,
        skipped_links: manifest.skipped_links,
    }
}

/// Decrypts and checks the backup at `source` into fresh staging folders
/// `id` (D8, D9, D11, ruling F16) and answers it with its keychain items.
/// `progress` hears the bytes read of the file's size. On any failure the
/// staging folders are gone and nothing else changed.
pub(crate) fn extract(
    roots: &Roots,
    id: &str,
    source: &Path,
    password: &str,
    cancel: &CancelFlag,
    progress: &dyn Fn(u64, u64),
) -> Result<Staged, BackupError> {
    if cancel.cancelled() {
        return Err(stopped());
    }
    let folders = Folders::create(roots, id)?;
    let staged = Extraction::new(roots, &folders, id, cancel).run(source, password, progress)?;
    folders.keep();
    Ok(staged)
}

/// The staging folders of one open. Dropped before [`Folders::keep`], they
/// are removed.
struct Folders {
    core: PathBuf,
    desktop: PathBuf,
    made: Vec<PathBuf>,
    kept: bool,
}

impl Folders {
    /// Makes `<root>/backups/staging/<id>` in each root, 0700, refusing a
    /// `backups` or `staging` that is a link and an id folder that exists.
    fn create(roots: &Roots, id: &str) -> Result<Self, BackupError> {
        backups_are_folders(roots)?;
        let mut folders = Self {
            core: staging_dir(roots, RootKey::Core, id),
            desktop: staging_dir(roots, RootKey::Desktop, id),
            made: Vec::new(),
            kept: false,
        };
        for root in staging_roots(roots) {
            let backups = roots.backups(*root);
            private_dir(&backups)?;
            private_dir(&backups.join(STAGING))?;
            let folder = staging_dir(roots, *root, id);
            DirBuilder::new().mode(FOLDER).create(&folder)?;
            folders.made.push(folder);
        }
        Ok(folders)
    }

    fn keep(mut self) {
        self.kept = true;
    }
}

impl Drop for Folders {
    fn drop(&mut self) {
        if self.kept {
            return;
        }
        for folder in &self.made {
            // Staged entries are only ever files and folders made here.
            if std::fs::remove_dir_all(folder).is_err() {
                tracing::warn!("Bruhs couldn't remove a staged restore");
            }
        }
    }
}

/// What went wrong under the tar reader, which hides the decryptor's error.
#[derive(Debug, Clone, Copy)]
enum Fault {
    Damaged,
    Io(io::ErrorKind),
}

/// The decrypted stream, remembering why it first failed.
struct Plain<R: Read> {
    inner: Decryptor<R>,
    fault: Rc<Cell<Option<Fault>>>,
}

impl<R: Read> Read for Plain<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.inner.read(buf).inspect_err(|error| {
            let fault = match error
                .get_ref()
                .and_then(|inner| inner.downcast_ref::<FormatError>())
            {
                Some(FormatError::Io(inner)) => Fault::Io(inner.kind()),
                Some(_) => Fault::Damaged,
                None => Fault::Io(error.kind()),
            };
            if self.fault.get().is_none() {
                self.fault.set(Some(fault));
            }
        })
    }
}

/// The backup file, counting what was read for progress and stopping when
/// the job is cancelled.
struct Counted<'a> {
    file: File,
    read: u64,
    sent: u64,
    total: u64,
    cancel: &'a CancelFlag,
    progress: &'a dyn Fn(u64, u64),
}

impl Read for Counted<'_> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.cancel.cancelled() {
            return Err(io::Error::other("the open was stopped"));
        }
        let read = self.file.read(buf)?;
        self.read = self
            .read
            .saturating_add(u64::try_from(read).map_err(io::Error::other)?);
        if self.read.saturating_sub(self.sent) >= PROGRESS_STEP {
            self.sent = self.read;
            (self.progress)(self.read, self.total.max(self.read));
        }
        Ok(read)
    }
}

/// Opens the chosen file for reading without waiting on a FIFO; anything
/// but a regular file isn't a backup.
fn open_backup_file(source: &Path) -> Result<File, BackupError> {
    match OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK)
        .open(source)
    {
        Ok(file) if file.metadata()?.is_file() => Ok(file),
        Ok(_) => Err(BackupError::Format(NOT_A_BACKUP)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            Err(BackupError::Refused(SOURCE_MISSING))
        }
        Err(error) => Err(error.into()),
    }
}

/// Why a stream that opened can't be read on: a cancelled job, an io
/// error, or a damaged backup.
fn read_failure(cancel: &CancelFlag, fault: Option<Fault>) -> BackupError {
    if cancel.cancelled() {
        return stopped();
    }
    match fault {
        Some(Fault::Io(kind)) => BackupError::Io(kind.into()),
        Some(Fault::Damaged) | None => damaged(),
    }
}

/// One backup being read into its staging folders.
struct Extraction<'a> {
    same: bool,
    core: &'a Path,
    desktop: &'a Path,
    id: &'a str,
    cancel: &'a CancelFlag,
    fault: Rc<Cell<Option<Fault>>>,
    entries: u64,
    /// Every path in a staging folder, whether it is a folder.
    placed: HashMap<(RootKey, String), bool>,
    /// Paths an entry of their own named.
    given: HashSet<(RootKey, String)>,
    /// Each placed path's folded key and the path holding it.
    folded: HashMap<(RootKey, String), String>,
    /// On one root: the archive root each top-level name came from.
    tops: HashMap<String, RootKey>,
    /// Folders made in the staging folders.
    made: HashSet<PathBuf>,
    /// Every file that arrived, with its size and SHA-256.
    received: BTreeMap<String, (u64, String)>,
    keychain: Option<Zeroizing<Vec<u8>>>,
    web_part: bool,
    manifest: Option<Vec<u8>>,
    desktop_entries: bool,
}

impl<'a> Extraction<'a> {
    fn new(roots: &Roots, folders: &'a Folders, id: &'a str, cancel: &'a CancelFlag) -> Self {
        let same = roots.same();
        Self {
            same,
            core: &folders.core,
            desktop: if same {
                &folders.core
            } else {
                &folders.desktop
            },
            id,
            cancel,
            fault: Rc::default(),
            entries: 0,
            placed: HashMap::new(),
            given: HashSet::new(),
            folded: HashMap::new(),
            tops: HashMap::new(),
            made: HashSet::new(),
            received: BTreeMap::new(),
            keychain: None,
            web_part: false,
            manifest: None,
            desktop_entries: false,
        }
    }

    fn broken(&self) -> BackupError {
        read_failure(self.cancel, self.fault.get())
    }

    fn run(
        mut self,
        source: &Path,
        password: &str,
        progress: &dyn Fn(u64, u64),
    ) -> Result<Staged, BackupError> {
        let file = open_backup_file(source)?;
        let total = file.metadata()?.len();
        let counted = Counted {
            file,
            read: 0,
            sent: 0,
            total,
            cancel: self.cancel,
            progress,
        };
        let decryptor = {
            let password = Zeroizing::new(normalize_password(password));
            Decryptor::open(counted, Secret::Password(&password), Kind::Full)
                .map_err(|error| self.opening(error))?
        };
        let mut plain = Plain {
            inner: decryptor,
            fault: self.fault.clone(),
        };
        {
            let mut archive = tar::Archive::new(&mut plain);
            // Raw: this reader takes GNU long names itself, with a cap, and
            // refuses pax headers, so no header size decides an allocation.
            let entries = archive.entries().map_err(|_| self.broken())?.raw(true);
            let mut long_name = None;
            for entry in entries {
                let mut entry = entry.map_err(|_| self.broken())?;
                if self.cancel.cancelled() {
                    return Err(stopped());
                }
                self.entries = self.entries.saturating_add(1);
                if self.entries > MAX_ENTRIES {
                    return Err(unsafe_entry());
                }
                if entry.header().entry_type().is_gnu_longname() {
                    if long_name.is_some() {
                        return Err(damaged());
                    }
                    long_name = Some(self.long_name(&mut entry)?);
                    continue;
                }
                let name = long_name
                    .take()
                    .unwrap_or_else(|| entry.path_bytes().into_owned());
                self.entry(&mut entry, name)?;
            }
            if long_name.is_some() {
                return Err(damaged());
            }
        }
        self.tail(&mut plain)?;
        plain
            .inner
            .finish()
            .map_err(|error| read_failure(self.cancel, Some(fault_of(&error))))?;
        self.checked()
    }

    /// The decryptor's first answer: the header and the first chunk.
    fn opening(&self, error: FormatError) -> BackupError {
        if self.cancel.cancelled() {
            return stopped();
        }
        match error {
            FormatError::NotABackup => BackupError::Format(NOT_A_BACKUP),
            FormatError::NewerVersion => BackupError::Format(NEWER_BACKUP),
            FormatError::WrongPasswordOrDamaged => BackupError::Format(WRONG_PASSWORD),
            FormatError::Io(error) => BackupError::Io(error),
            FormatError::BadHeader
            | FormatError::BadParams
            | FormatError::Truncated
            | FormatError::TrailingData => damaged(),
        }
    }

    /// A GNU long name: the next entry's path, at most 4 KiB.
    fn long_name<R: Read>(&self, entry: &mut tar::Entry<'_, R>) -> Result<Vec<u8>, BackupError> {
        let size = entry.size();
        if size > LONG_NAME_MAX {
            return Err(unsafe_entry());
        }
        let mut name = vec![0; usize::try_from(size).map_err(io::Error::other)?];
        entry.read_exact(&mut name).map_err(|_| self.broken())?;
        if name.last() == Some(&0) {
            name.pop();
        }
        Ok(name)
    }

    /// Checks one entry and stages it, or holds it when it is the
    /// manifest or `keychain.json`.
    fn entry<R: Read>(
        &mut self,
        entry: &mut tar::Entry<'_, R>,
        name: Vec<u8>,
    ) -> Result<(), BackupError> {
        // The manifest is the last entry.
        if self.manifest.is_some() {
            return Err(damaged());
        }
        let folder = match entry.header().entry_type() {
            tar::EntryType::Regular => false,
            tar::EntryType::Directory => true,
            // Links, devices, FIFOs, sparse files, pax headers and the rest.
            _ => return Err(unsafe_entry()),
        };
        let path = String::from_utf8(name).map_err(|_| unsafe_entry())?;
        let size = entry.size();
        let target = safe_entry_path(&path).map_err(BackupError::Format)?;
        if folder && !matches!(target, EntryPath::Core(_) | EntryPath::Desktop(_)) {
            return Err(unsafe_entry());
        }
        match target {
            EntryPath::Manifest => {
                if size > MANIFEST_MAX {
                    return Err(damaged());
                }
                let mut text = vec![0; usize::try_from(size).map_err(io::Error::other)?];
                entry.read_exact(&mut text).map_err(|_| self.broken())?;
                self.manifest = Some(text);
            }
            EntryPath::Keychain => {
                if self.keychain.is_some() {
                    return Err(unsafe_entry());
                }
                if size > KEYCHAIN_MAX {
                    return Err(damaged());
                }
                // Sized once from the capped size, so it never moves.
                let mut json =
                    Zeroizing::new(vec![0; usize::try_from(size).map_err(io::Error::other)?]);
                entry.read_exact(&mut json).map_err(|_| self.broken())?;
                self.received
                    .insert(path, (size, hex(&Sha256::digest(&*json))));
                self.keychain = Some(json);
            }
            EntryPath::WebPart => {
                if self.web_part {
                    return Err(unsafe_entry());
                }
                self.web_part = true;
                let meta = self.core.join(META);
                DirBuilder::new().mode(FOLDER).create(&meta)?;
                let (bytes, sha256, head) =
                    self.write_file(entry, &meta.join(WEB_PART_FILE), PRIVATE, size)?;
                if !web_part_header(&head) {
                    return Err(damaged());
                }
                self.received.insert(path, (bytes, sha256));
            }
            EntryPath::Core(rel) => self.data(entry, path, RootKey::Core, &rel, folder, size)?,
            EntryPath::Desktop(rel) => {
                self.desktop_entries = true;
                self.data(entry, path, RootKey::Desktop, &rel, folder, size)?;
            }
        }
        Ok(())
    }

    /// Stages a data file or folder from `origin`'s part of the archive.
    /// On one root, `desktop/` entries go into the core staging (D11).
    fn data<R: Read>(
        &mut self,
        entry: &mut tar::Entry<'_, R>,
        path: String,
        origin: RootKey,
        rel: &str,
        folder: bool,
        size: u64,
    ) -> Result<(), BackupError> {
        // Deeper excluded folders and files too (B Task 3 note).
        if excluded(rel, folder) == Excluded::Always {
            return Err(unsafe_entry());
        }
        let root = if self.same { RootKey::Core } else { origin };
        self.claim(origin, root, rel, folder)?;
        if folder {
            if size != 0 {
                return Err(damaged());
            }
            return self.make_folders(root, rel, true);
        }
        self.make_folders(root, rel, false)?;
        let mode = entry.header().mode().map_err(|_| damaged())?;
        let mode = if mode & 0o100 == 0 { PRIVATE } else { RUNNABLE };
        let target = self.staging(root).join(rel);
        let (bytes, sha256, _) = self.write_file(entry, &target, mode, size)?;
        self.received.insert(path, (bytes, sha256));
        Ok(())
    }

    fn staging(&self, root: RootKey) -> &Path {
        match root {
            RootKey::Core => self.core,
            RootKey::Desktop => self.desktop,
        }
    }

    /// Takes `rel` and every folder above it in `root`'s staging (D9):
    /// refused when a path comes twice, a file sits under a file or where a
    /// folder was, two names differ only in case or accents, the core
    /// staging's `meta` is named, or, on one root, both archive roots name
    /// one top-level entry (D11, `ROOTS_OVERLAP`).
    fn claim(
        &mut self,
        origin: RootKey,
        root: RootKey,
        rel: &str,
        folder: bool,
    ) -> Result<(), BackupError> {
        let top = rel.split('/').next().unwrap_or_default();
        if root == RootKey::Core && folded_entry_key(top) == META {
            return Err(unsafe_entry());
        }
        if self.same {
            let first = *self.tops.entry(folded_entry_key(top)).or_insert(origin);
            if first != origin {
                return Err(BackupError::Format(ROOTS_OVERLAP));
            }
        }
        for (end, _) in rel.match_indices('/') {
            self.place(root, &rel[..end], true, false)?;
        }
        self.place(root, rel, folder, true)
    }

    fn place(
        &mut self,
        root: RootKey,
        path: &str,
        folder: bool,
        given: bool,
    ) -> Result<(), BackupError> {
        let key = (root, path.to_owned());
        if given && !self.given.insert(key.clone()) {
            return Err(unsafe_entry());
        }
        match self.placed.get(&key) {
            // A file is there: a file under it, or the same path again.
            Some(false) => return Err(unsafe_entry()),
            Some(true) if !folder => return Err(unsafe_entry()),
            Some(true) => return Ok(()),
            None => {}
        }
        let holder = self
            .folded
            .entry((root, folded_entry_key(path)))
            .or_insert_with(|| path.to_owned());
        if holder != path {
            return Err(BackupError::Format(NAMES_CLASH));
        }
        self.placed.insert(key, folder);
        Ok(())
    }

    /// Makes the folders above `rel` in `root`'s staging, and `rel` itself
    /// when `whole`, each 0700. A name that is already there means the
    /// volume merged two names.
    fn make_folders(&mut self, root: RootKey, rel: &str, whole: bool) -> Result<(), BackupError> {
        let parts: Vec<&str> = rel.split('/').collect();
        let upto = if whole { parts.len() } else { parts.len() - 1 };
        let mut folder = self.staging(root).to_path_buf();
        for part in &parts[..upto] {
            folder.push(part);
            if self.made.contains(&folder) {
                continue;
            }
            match DirBuilder::new().mode(FOLDER).create(&folder) {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                    return Err(BackupError::Format(NAMES_CLASH));
                }
                Err(error) => return Err(error.into()),
            }
            self.made.insert(folder.clone());
        }
        Ok(())
    }

    /// Writes an entry's bytes to a new file at `target` (never replacing
    /// one or following a link) with `mode`, hashing them. Answers the size,
    /// the SHA-256 and the first bytes.
    fn write_file<R: Read>(
        &self,
        entry: &mut tar::Entry<'_, R>,
        target: &Path,
        mode: u32,
        size: u64,
    ) -> Result<(u64, String, Vec<u8>), BackupError> {
        let mut out = match OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(mode)
            .custom_flags(libc::O_NOFOLLOW)
            .open(target)
        {
            Ok(out) => out,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                return Err(BackupError::Format(NAMES_CLASH));
            }
            Err(error) => return Err(error.into()),
        };
        // The umask may have taken bits; the mode is exactly this.
        out.set_permissions(Permissions::from_mode(mode))?;
        let mut hasher = Sha256::new();
        let mut head = Vec::with_capacity(HEADER_LEN);
        let mut written = 0_u64;
        let mut buffer = vec![0; COPY_BUFFER];
        loop {
            let read = match entry.read(&mut buffer) {
                Ok(0) => break,
                Ok(read) => read,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(_) => return Err(self.broken()),
            };
            let chunk = &buffer[..read];
            hasher.update(chunk);
            let room = HEADER_LEN.saturating_sub(head.len());
            head.extend_from_slice(&chunk[..room.min(read)]);
            out.write_all(chunk)?;
            written = written.saturating_add(u64::try_from(read).map_err(io::Error::other)?);
        }
        if written != size {
            return Err(damaged());
        }
        Ok((written, hex(&hasher.finalize()), head))
    }

    /// The tar's end (ruling F16): after the first end block, which the
    /// tar reader took, only zero blocks: at least the second, at most
    /// 10 KiB.
    fn tail<R: Read>(&self, plain: &mut Plain<R>) -> Result<(), BackupError> {
        let mut rest = Vec::new();
        let limit = u64::try_from(TAIL_MAX + 1).map_err(io::Error::other)?;
        plain
            .take(limit)
            .read_to_end(&mut rest)
            .map_err(|_| self.broken())?;
        let zero_blocks = rest.len() >= BLOCK
            && rest.len() <= TAIL_MAX
            && rest.len() % BLOCK == 0
            && rest.iter().all(|byte| *byte == 0);
        if zero_blocks {
            Ok(())
        } else {
            Err(damaged())
        }
    }

    /// The manifest's checks (D3, D8): readable by this build, plain
    /// absolute roots, every listed file arrived with its size and hash and
    /// nothing else did, and the keychain and web parts are there exactly
    /// when it says so. Then a one-root backup's desktop entries move to
    /// the desktop staging (D11).
    fn checked(mut self) -> Result<Staged, BackupError> {
        let text = self.manifest.take().ok_or_else(damaged)?;
        let manifest: BackupManifest = serde_json::from_slice(&text).map_err(|_| damaged())?;
        manifest.readable().map_err(BackupError::Format)?;
        let plain_absolute = |path: &str| {
            let path = Path::new(path);
            path.is_absolute()
                && path
                    .components()
                    .all(|part| matches!(part, Component::RootDir | Component::Normal(_)))
        };
        let roots = [&manifest.home, &manifest.core_root, &manifest.desktop_root];
        if !roots.iter().all(|root| plain_absolute(root))
            || manifest.same_root != (manifest.core_root == manifest.desktop_root)
            || (manifest.same_root && self.desktop_entries)
        {
            return Err(damaged());
        }

        let mut listed = BTreeMap::new();
        for file in &manifest.files {
            if listed
                .insert(file.path.as_str(), (file.bytes, file.sha256.as_str()))
                .is_some()
            {
                return Err(damaged());
            }
        }
        let all_arrived = listed.len() == self.received.len()
            && self.received.iter().all(|(path, (bytes, sha256))| {
                listed.get(path.as_str()) == Some(&(*bytes, sha256.as_str()))
            });
        if !all_arrived
            || manifest.options.keychain != self.keychain.is_some()
            || manifest.web_db_version.is_some() != self.web_part
        {
            return Err(damaged());
        }

        let items = match self.keychain.take() {
            Some(json) => {
                serde_json::from_slice::<KeychainFile>(&json)
                    .map_err(|_| damaged())?
                    .items
            }
            None => Vec::new(),
        };
        if u32::try_from(items.len()).ok() != Some(manifest.keychain_items) {
            return Err(damaged());
        }
        if manifest.same_root && !self.same {
            for name in DESKTOP_ENTRIES {
                let from = self.core.join(name);
                if std::fs::symlink_metadata(&from).is_ok() {
                    std::fs::rename(&from, self.desktop.join(name))?;
                }
            }
        }
        Ok(Staged {
            id: self.id.to_owned(),
            manifest,
            items,
        })
    }
}

/// `keychain.json` as Task 6 writes it: base64 values, no escapes.
#[derive(serde::Deserialize)]
struct KeychainFile {
    items: Vec<KeychainItem>,
}

fn fault_of(error: &FormatError) -> Fault {
    match error {
        FormatError::Io(error) => Fault::Io(error.kind()),
        _ => Fault::Damaged,
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

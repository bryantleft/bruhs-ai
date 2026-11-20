//! Making a backup (D3 to D6, rulings F9, F15, F16, F24 and the B Task 5
//! notes): both data roots, Bruhs' keychain items and the browser's own
//! part, streamed as one tar into the encrypted container, the manifest
//! last.
//!
//! Database files are never read with a plain descriptor: each one is
//! snapshotted through SQLite into `backups/tmp/` and streamed from there.
//! Every other file is opened through its folder handles without
//! following a link. Keychain values exist only in wiped buffers and in
//! the ciphertext.

use std::collections::{BTreeMap, HashSet};
use std::fs::{File, Metadata, OpenOptions};
use std::io::{self, BufWriter, Read, Seek, Write};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use bruhs_backup::{Encryptor, Kdf, Key, Kind};
use bruhs_models::backup::{
    backup_file_name, browser_busy_line, changed_files_line, missing_items_line, no_space,
    normalize_password, skipped_names_line, validate_password, BackupApp, BackupCounts, BackupFile,
    BackupManifest, BackupOptions, JobState, JobStep, LastBackup, ManifestFile, BACKUP_STOPPED,
    BACKUP_TOO_MANY, BRUHS_UNCOUNTED, DESTINATION_EXISTS, DESTINATION_INSIDE, DESTINATION_MISSING,
    DESTINATION_NAME_LONG, FILE_CHANGED, INVALID_REQUEST, KEYCHAIN_ITEM_REFUSED,
    KEYCHAIN_NEEDS_YOU, KEYCHAIN_READ_FAILED, MANIFEST_FORMAT, NOT_A_BACKUP, ROOT_NAME_REFUSED,
    WEB_TOO_LARGE, WEB_UPLOAD_MAX,
};
use bruhs_models::crew::CREW_PROTOCOL;
use bruhs_models::EpochMs;
use sha2::{Digest, Sha256};
use tokio::sync::watch;
use zeroize::Zeroizing;

use super::host::{
    marker_text, new_private_file, random_bytes, random_id, web_part_header, BackupHost, Config,
    Limits, PARTIAL_MARKER,
};
use super::{
    free_bytes, inventory, open_source, snapshot_sqlite, BackupError, RootId, RootKey, Roots,
    SourceFile, FREE_MARGIN,
};
use crate::secrets::{keychain_inventory, read_items, ItemsError, KeychainItem};

/// A backup to make. Holds the password: deliberately no `Debug` or
/// `Clone`.
pub struct CreateRequest {
    pub password: Zeroizing<String>,
    pub options: BackupOptions,
    pub app: BackupApp,
    /// The browser's encrypted part, a finished `UploadKind::WebPart`
    /// upload.
    pub web_part: Option<PathBuf>,
    /// The IndexedDB version the browser dumped its part from, as its
    /// caller names it.
    pub web_db_version: Option<u32>,
    pub output: Output,
}

/// Where a backup goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Output {
    /// The file the desktop's save dialog chose. `overwrite` only after
    /// the dialog's own confirmation.
    File { path: PathBuf, overwrite: bool },
    /// `backups/outgoing/<id>.bruhsbak`, for the web to download.
    Outgoing,
}

/// Stops a job between files and between chunks.
#[derive(Debug, Clone, Default)]
pub struct CancelFlag(Arc<AtomicBool>);

impl CancelFlag {
    pub fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }

    pub fn cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }

    fn check(&self) -> Result<(), BackupError> {
        if self.cancelled() {
            Err(BackupError::Refused(BACKUP_STOPPED))
        } else {
            Ok(())
        }
    }
}

/// The file extension every backup has.
const EXTENSION: &str = "bruhsbak";
/// The longest file name the volumes Bruhs runs on take, in bytes.
const NAME_MAX: usize = 255;
/// What a partial file adds to its final name: `.`, `.partial-` and 32
/// hex characters.
const PARTIAL_EXTRA: usize = 1 + 9 + 32;
/// Bytes read between two progress updates.
const PROGRESS_STEP: u64 = 1 << 20;

/// Checks a destination the desktop chose (D6) and answers the final path:
/// absolute, in a folder that exists, outside every data root, with the
/// `.bruhsbak` extension (appended when missing), and not an existing
/// file unless `overwrite`. A folder is never replaced.
pub fn check_destination(
    roots: &Roots,
    path: &Path,
    overwrite: bool,
) -> Result<PathBuf, BackupError> {
    let missing = || BackupError::Refused(DESTINATION_MISSING);
    if !path.is_absolute() {
        return Err(missing());
    }
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(missing)?;
    let name = if Path::new(name)
        .extension()
        .is_some_and(|extension| extension == EXTENSION)
    {
        name.to_owned()
    } else {
        format!("{name}.{EXTENSION}")
    };
    // `.<name>.partial-<32 hex>` must fit a 255-byte file name.
    if name.len().saturating_add(PARTIAL_EXTRA) > NAME_MAX {
        return Err(BackupError::Refused(DESTINATION_NAME_LONG));
    }
    let folder = path.parent().ok_or_else(missing)?;
    let folder = std::fs::canonicalize(folder).map_err(|_| missing())?;
    if !folder.is_dir() {
        return Err(missing());
    }
    let target = folder.join(name);
    for root in [&roots.core, &roots.desktop, &roots.config] {
        let resolved = std::fs::canonicalize(root).unwrap_or_else(|_| root.clone());
        if target.starts_with(&resolved) || target.starts_with(root) {
            return Err(BackupError::Refused(DESTINATION_INSIDE));
        }
    }
    match std::fs::symlink_metadata(&target) {
        Ok(metadata) if metadata.is_dir() || !overwrite => {
            Err(BackupError::Refused(DESTINATION_EXISTS))
        }
        Ok(_) => Ok(target),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(target),
        Err(error) => Err(error.into()),
    }
}

/// Creates `folder` with mode 0700 when missing. An existing one must be a
/// real folder, never a link, and is made 0700 through its own handle.
pub(super) fn private_dir(folder: &Path) -> Result<(), BackupError> {
    match std::fs::symlink_metadata(folder) {
        Ok(metadata) if metadata.is_dir() => return tighten(folder),
        Ok(_) => return Err(not_a_folder()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    let mut builder = std::fs::DirBuilder::new();
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    match builder.create(folder) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            match std::fs::symlink_metadata(folder) {
                Ok(metadata) if metadata.is_dir() => Ok(()),
                _ => Err(not_a_folder()),
            }
        }
        Err(error) => Err(error.into()),
    }
}

/// Makes the real folder `folder` 0700, opened without following a link.
fn tighten(folder: &Path) -> Result<(), BackupError> {
    let opened = File::from(
        rustix::fs::open(
            folder,
            rustix::fs::OFlags::RDONLY
                | rustix::fs::OFlags::DIRECTORY
                | rustix::fs::OFlags::NOFOLLOW
                | rustix::fs::OFlags::CLOEXEC,
            rustix::fs::Mode::empty(),
        )
        .map_err(|_| not_a_folder())?,
    );
    if opened.metadata()?.permissions().mode() & 0o777 != 0o700 {
        opened.set_permissions(std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

/// Refuses when either root's `backups` exists but isn't a real folder (a
/// link, a file). A missing one is fine: a job makes it.
pub(super) fn backups_are_folders(roots: &Roots) -> Result<(), BackupError> {
    let keys: &[RootKey] = if roots.same() {
        &[RootKey::Core]
    } else {
        &[RootKey::Core, RootKey::Desktop]
    };
    for key in keys {
        match std::fs::symlink_metadata(roots.backups(*key)) {
            Ok(metadata) if !metadata.is_dir() => return Err(not_a_folder()),
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

/// A `backups` folder that is a link or a file.
fn not_a_folder() -> BackupError {
    BackupError::Io(io::Error::other("a backups folder isn't a real folder"))
}

/// Makes the backup `request` asks for and answers the finished file.
/// Every failure removes the partial file and the snapshots it made.
pub(crate) async fn create(
    host: &BackupHost,
    request: CreateRequest,
    progress: &watch::Sender<JobState>,
    cancel: &CancelFlag,
) -> Result<BackupFile, BackupError> {
    let config = host.config().clone();
    let roots = config.roots.clone();
    let CreateRequest {
        password,
        options,
        app,
        web_part,
        web_db_version,
        output,
    } = request;
    let info = utf8_roots(&roots)?;
    validate_password(&password).map_err(BackupError::Refused)?;
    if web_part.is_some() && web_db_version.is_none() {
        return Err(BackupError::Refused(INVALID_REQUEST));
    }
    let made_at = EpochMs::now();

    // Neither root's `backups` may be a link (B Task 5 I1 ruling).
    backups_are_folders(&roots)?;
    private_dir(&roots.backups(RootKey::Core))?;
    let tmp = host.tmp_dir();
    private_dir(&tmp)?;
    let (target, overwrite, download) = match &output {
        Output::File { path, overwrite } => (
            check_destination(&roots, path, *overwrite)?,
            *overwrite,
            None,
        ),
        Output::Outgoing => {
            let folder = host.outgoing_dir();
            private_dir(&folder)?;
            let id = random_id();
            (folder.join(format!("{id}.{EXTENSION}")), false, Some(id))
        }
    };

    send(progress, 0, 0, JobStep::Collecting);
    let bruhs = bruhs_here(&config).await;
    let busy = if options.browser_profiles {
        close_browsers(&config, &roots).await
    } else {
        Vec::new()
    };
    let mut warnings: Vec<String> = busy
        .iter()
        .map(|id| browser_busy_line(bruhs.names.get(id).map_or(id.as_str(), String::as_str)))
        .collect();
    if bruhs.count.is_none() {
        warnings.push(BRUHS_UNCOUNTED.to_owned());
    }
    cancel.check()?;

    let walked = {
        let roots = roots.clone();
        tokio::task::spawn_blocking(move || inventory(&roots, &options))
            .await
            .map_err(|_| stopped())??
    };
    let mut files = walked.files;
    files.retain(|file| {
        file.root != RootKey::Core
            || !busy
                .iter()
                .any(|id| file.rel.starts_with(&format!("server/bruhs/{id}/browser/")))
    });
    warnings.extend(skipped_names_line(walked.skipped_names));
    let web = web_part.as_deref().map(open_web_part).transpose()?;
    let beside = [
        options.keychain.then_some("keychain.json"),
        web.as_ref().map(|_| "web/part.bin"),
    ];
    fits(
        &files,
        &beside.into_iter().flatten().collect::<Vec<_>>(),
        config.limits,
    )?;

    let total = files
        .iter()
        .map(|file| file.bytes)
        .chain(web.as_ref().map(|(_, bytes)| *bytes))
        .fold(0_u64, u64::saturating_add);
    let folder = target
        .parent()
        .ok_or(BackupError::Refused(DESTINATION_MISSING))?;
    // Snapshots wait in `backups/tmp` one at a time, so the largest must
    // fit there too, beside the output when both share a volume.
    let snapshot = files
        .iter()
        .filter(|file| file.sqlite)
        .map(|file| file.bytes)
        .max()
        .unwrap_or(0);
    let shared = std::fs::metadata(folder)?.dev() == std::fs::metadata(&tmp)?.dev();
    let needed = total
        .saturating_add(if shared { snapshot } else { 0 })
        .saturating_add(FREE_MARGIN);
    enough_space(folder, needed)?;
    if !shared {
        enough_space(&tmp, snapshot.saturating_add(FREE_MARGIN))?;
    }

    let keychain = if options.keychain {
        send(progress, 0, total, JobStep::Keychain);
        Some(read_keychain(&config, &roots).await?)
    } else {
        None
    };
    cancel.check()?;

    let keychain_items = keychain.as_ref().map_or(0, |(_, count, _)| *count);
    if let Some((_, _, missing)) = &keychain {
        warnings.extend(missing_items_line(*missing));
    }
    let manifest = BackupManifest {
        format: MANIFEST_FORMAT,
        app_version: config.app_version.clone(),
        made_at_ms: made_at,
        made_on: app,
        reactive_protocol: CREW_PROTOCOL,
        web_db_version: web.as_ref().and(web_db_version),
        os: std::env::consts::OS.to_owned(),
        home: info.home,
        core_root: info.core,
        desktop_root: info.desktop,
        same_root: roots.same(),
        options,
        files: Vec::new(),
        keychain_items,
        skipped_links: walked.skipped_links,
        skipped_names: walked.skipped_names,
        changed_files: 0,
        counts: BackupCounts {
            bruhs: bruhs.count.unwrap_or(0),
            vault_entries: vault_entries(&roots),
            keychain_items,
            files: u64::try_from(files.len()).unwrap_or(u64::MAX),
        },
    };
    let writer = Writer {
        files,
        keychain: keychain.map(|(json, ..)| json),
        web,
        password,
        kdf: config.kdf,
        limits: config.limits,
        tmp,
        cancel: cancel.clone(),
        changed: 0,
        progress: Progress {
            sender: progress.clone(),
            total,
            done: 0,
            sent: 0,
        },
        manifest,
        #[cfg(test)]
        before_file: config.before_file.clone(),
    };
    let place = target.clone();
    let (bytes, changed) = tokio::task::spawn_blocking(move || writer.run(&place, overwrite))
        .await
        .map_err(|_| stopped())??;
    warnings.extend(changed_files_line(changed));

    // The backup is made either way; Settings only misses its last line.
    if host
        .record_last(&LastBackup {
            made_at_ms: made_at,
            app,
            bytes,
        })
        .is_err()
    {
        tracing::warn!("Bruhs couldn't record the last backup");
    }
    let (name, path, download) = match download {
        Some(id) => (
            backup_file_name(made_at),
            None,
            Some(format!("/api/backup/files/{id}")),
        ),
        None => (
            target
                .file_name()
                .map_or_else(String::new, |name| name.to_string_lossy().into_owned()),
            Some(target.to_string_lossy().into_owned()),
            None,
        ),
    };
    Ok(BackupFile {
        name,
        bytes,
        path,
        download,
        warnings,
    })
}

/// The fewest bytes one listed file takes in the manifest beside its path:
/// `{"path":"` `","bytes":` one digit `,"sha256":"` 64 hex `"}`.
const LISTED_FILE_MIN_BYTES: u64 = 97;

/// Refuses, before anything is written, a backup a restore wouldn't take
/// (B Task 7 I1): more tar entries than `limits.entries` (the files, every
/// folder above them, the parts `beside` the data and the manifest), or a
/// manifest that would be over `limits.manifest_bytes` even at its
/// smallest. The writer checks the exact count and size again before the
/// manifest goes in.
pub(super) fn fits(
    files: &[SourceFile],
    beside: &[&str],
    limits: Limits,
) -> Result<(), BackupError> {
    let mut folders = HashSet::new();
    let mut manifest = 0_u64;
    let mut listed = |path_bytes: usize| {
        let path_bytes = u64::try_from(path_bytes).unwrap_or(u64::MAX);
        manifest = manifest
            .saturating_add(path_bytes)
            .saturating_add(LISTED_FILE_MIN_BYTES);
    };
    for file in files {
        let prefix = match file.root {
            RootKey::Core => "core/",
            RootKey::Desktop => "desktop/",
        };
        for (end, _) in file.rel.match_indices('/') {
            folders.insert((file.root, &file.rel[..end]));
        }
        listed(prefix.len() + file.rel.len());
    }
    for part in beside {
        listed(part.len());
    }
    let entries = [files.len(), folders.len(), beside.len(), 1]
        .into_iter()
        .map(|count| u64::try_from(count).unwrap_or(u64::MAX))
        .fold(0, u64::saturating_add);
    if entries > limits.entries || manifest > limits.manifest_bytes {
        Err(BackupError::Refused(BACKUP_TOO_MANY))
    } else {
        Ok(())
    }
}

/// The job's worker stopped without an answer.
fn stopped() -> BackupError {
    BackupError::Io(io::Error::other("the backup worker stopped"))
}

fn send(progress: &watch::Sender<JobState>, done_bytes: u64, total_bytes: u64, step: JobStep) {
    progress.send_replace(JobState::Running {
        done_bytes,
        total_bytes,
        step,
    });
}

/// The home and roots as text. A root that isn't UTF-8 can't be named in
/// the manifest or rewritten on restore, so no backup is made (B Task 5
/// ruling).
fn utf8_roots(roots: &Roots) -> Result<bruhs_models::backup::RootsInfo, BackupError> {
    let all_text = [&roots.home, &roots.core, &roots.desktop, &roots.config]
        .iter()
        .all(|path| path.to_str().is_some());
    if all_text {
        Ok(roots.info())
    } else {
        Err(BackupError::Refused(ROOT_NAME_REFUSED))
    }
}

/// The bruhs the reactive store knows: their names for warnings and how
/// many there are.
#[derive(Default)]
struct Bruhs {
    names: BTreeMap<String, String>,
    /// `None` when the store didn't answer, which the result says.
    count: Option<u32>,
}

async fn bruhs_here(config: &Config) -> Bruhs {
    let Some(runtime) = &config.reactive else {
        return Bruhs {
            count: Some(0),
            ..Bruhs::default()
        };
    };
    let Ok(snapshot) = runtime.snapshot().await else {
        return Bruhs::default();
    };
    Bruhs {
        names: snapshot
            .agents
            .into_iter()
            .map(|agent| (agent.id, agent.name))
            .collect(),
        count: Some(u32::try_from(snapshot.bruhs.len()).unwrap_or(u32::MAX)),
    }
}

/// `NO_SPACE` unless `needed` bytes are free on the volume of `folder`.
fn enough_space(folder: &Path, needed: u64) -> Result<(), BackupError> {
    let free = free_bytes(folder)?;
    if free < needed {
        Err(BackupError::Space(no_space(needed - free)))
    } else {
        Ok(())
    }
}

/// Closes every bruh browser whose profile a backup takes (D6), each
/// waiting at most `browser_wait` for a running step. Answers the bruhs
/// whose browser was still busy: their profiles stay out.
async fn close_browsers(config: &Config, roots: &Roots) -> Vec<String> {
    let Some(tools) = &config.tools else {
        return Vec::new();
    };
    let ids = profile_owners(&roots.core.join("server").join("bruhs"));
    let closing = ids.iter().map(|id| async move {
        match tokio::time::timeout(config.browser_wait, tools.close_computer(id)).await {
            // Closed, or a bruh whose computer is off.
            Ok(_) => None,
            Err(_) => Some(id.clone()),
        }
    });
    futures_util::future::join_all(closing)
        .await
        .into_iter()
        .flatten()
        .collect()
}

/// The ids under `server/bruhs` that hold a browser profile.
fn profile_owners(trees: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(trees) else {
        return Vec::new();
    };
    let mut ids: Vec<String> = entries
        .flatten()
        .filter(|entry| {
            std::fs::symlink_metadata(entry.path().join("browser"))
                .is_ok_and(|metadata| metadata.is_dir())
        })
        .filter_map(|entry| entry.file_name().into_string().ok())
        .collect();
    ids.sort();
    ids
}

/// Reads every keychain item Bruhs' catalogs name (D5) and answers them
/// as `keychain.json`, `{"items": [...]}` with base64 values (ruling B
/// Task 4 I2), in a wiped buffer sized once, with how many there are and
/// how many the keychain didn't hold (D5). A read that waits or fails
/// stops the backup.
async fn read_keychain(
    config: &Config,
    roots: &Roots,
) -> Result<(Zeroizing<Vec<u8>>, u32, u32), BackupError> {
    let refs = keychain_inventory(
        &roots.core.join("credentials"),
        &roots.core.join("connections"),
        &roots.config.join("calendar").join("google"),
    )
    .map_err(|_| BackupError::Keychain(KEYCHAIN_READ_FAILED))?;
    let (items, missing) = read_items(&config.store, &refs, config.keychain_wait)
        .await
        .map_err(|error| {
            BackupError::Keychain(match error {
                ItemsError::NeedsOwner => KEYCHAIN_NEEDS_YOU,
                ItemsError::Storage => KEYCHAIN_READ_FAILED,
                ItemsError::Refused => KEYCHAIN_ITEM_REFUSED,
            })
        })?;
    let count = u32::try_from(items.len()).unwrap_or(u32::MAX);
    Ok((keychain_json(&items)?, count, missing))
}

#[derive(serde::Serialize)]
struct KeychainFile<'a> {
    items: &'a [KeychainItem],
}

/// Counts bytes, so a secret buffer is sized once and never moves.
struct Counter(usize);

impl Write for Counter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0 = self.0.saturating_add(bytes.len());
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn keychain_json(items: &[KeychainItem]) -> Result<Zeroizing<Vec<u8>>, BackupError> {
    let file = KeychainFile { items };
    let mut counter = Counter(0);
    serde_json::to_writer(&mut counter, &file).map_err(io::Error::other)?;
    let mut json = Zeroizing::new(Vec::with_capacity(counter.0));
    serde_json::to_writer(&mut *json, &file).map_err(io::Error::other)?;
    Ok(json)
}

/// How many vault entries and legacy saved values the catalogs hold.
fn vault_entries(roots: &Roots) -> u32 {
    let credentials = roots.core.join("credentials");
    ["catalog.json", "vault.json"]
        .iter()
        .filter_map(|name| std::fs::read(credentials.join(name)).ok())
        .filter_map(|text| serde_json::from_slice::<serde_json::Value>(&text).ok())
        .filter_map(|catalog| catalog.get("entries")?.as_object().map(|map| map.len()))
        .map(|count| u32::try_from(count).unwrap_or(u32::MAX))
        .fold(0, u32::saturating_add)
}

/// Opens the browser's stored part: a regular file, at most
/// `WEB_UPLOAD_MAX`, starting with a web part header.
fn open_web_part(path: &Path) -> Result<(File, u64), BackupError> {
    let mut file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() {
        return Err(BackupError::Format(NOT_A_BACKUP));
    }
    if metadata.len() > WEB_UPLOAD_MAX {
        return Err(BackupError::Refused(WEB_TOO_LARGE));
    }
    let mut head = [0_u8; bruhs_backup::HEADER_LEN];
    if file.read_exact(&mut head).is_err() || !web_part_header(&head) {
        return Err(BackupError::Format(NOT_A_BACKUP));
    }
    file.rewind()?;
    Ok((file, metadata.len()))
}

/// The job's progress, sent every megabyte.
struct Progress {
    sender: watch::Sender<JobState>,
    total: u64,
    done: u64,
    sent: u64,
}

impl Progress {
    fn add(&mut self, bytes: u64) {
        self.done = self.done.saturating_add(bytes);
        if self.done.saturating_sub(self.sent) >= PROGRESS_STEP {
            self.flush();
        }
    }

    fn flush(&mut self) {
        self.sent = self.done;
        send(
            &self.sender,
            self.done,
            self.total.max(self.done),
            JobStep::Writing,
        );
    }
}

/// The blocking part of a job: the key, the walk's files and the tar.
struct Writer {
    files: Vec<SourceFile>,
    keychain: Option<Zeroizing<Vec<u8>>>,
    web: Option<(File, u64)>,
    password: Zeroizing<String>,
    kdf: bruhs_backup::KdfParams,
    limits: Limits,
    tmp: PathBuf,
    cancel: CancelFlag,
    /// Files that changed while they were read (ruling B Task 6 I2).
    changed: u32,
    progress: Progress,
    manifest: BackupManifest,
    #[cfg(test)]
    before_file: Option<super::host::TestHook>,
}

type Sink = Encryptor<BufWriter<File>>;

/// The partial file a job writes beside its target, and the marker in
/// `backups/tmp` that names it, so the next start removes a partial file
/// a stopped process left (`BackupHost::sweep`). Dropped unplaced, both
/// go.
struct Partial {
    path: PathBuf,
    marker: PathBuf,
}

impl Partial {
    /// Creates the partial file, then its marker with the file's device
    /// and inode, so the sweep removes only this very file (ruling B Task
    /// 6 N1).
    fn new(folder: &Path, name: &std::ffi::OsStr, tmp: &Path) -> Result<(Self, File), BackupError> {
        let path = folder.join(format!(
            ".{}.partial-{}",
            name.to_string_lossy(),
            random_id()
        ));
        let file = new_private_file(&path)?;
        let partial = Self {
            path,
            marker: tmp.join(format!("{}{PARTIAL_MARKER}", random_id())),
        };
        let identity = file.metadata()?;
        let mut note = new_private_file(&partial.marker)?;
        note.write_all(&marker_text(identity.dev(), identity.ino(), &partial.path))?;
        note.sync_all()?;
        Ok((partial, file))
    }
}

impl Drop for Partial {
    /// Removes the partial file, which a placed backup no longer has (or,
    /// after a hard link, still has as a second name), then the marker.
    /// A partial file that can't be removed keeps its marker for the sweep.
    fn drop(&mut self) {
        match std::fs::remove_file(&self.path) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(_) => return,
        }
        let _ = std::fs::remove_file(&self.marker);
    }
}

impl Writer {
    #[cfg(test)]
    fn hook(&self, stage: super::host::Hook, path: &str) {
        if let Some(hook) = &self.before_file {
            hook(stage, path);
        }
    }

    /// Writes the backup beside `target` and moves it into place. Answers
    /// its size and how many files changed while they were read.
    fn run(self, target: &Path, overwrite: bool) -> Result<(u64, u32), BackupError> {
        let (Some(folder), Some(name)) = (target.parent(), target.file_name()) else {
            return Err(BackupError::Refused(DESTINATION_MISSING));
        };
        let (partial, file) = Partial::new(folder, name, &self.tmp)?;
        let cancel = self.cancel.clone();
        let written = self.write(file).and_then(|written| {
            place(&partial.path, target, overwrite)?;
            Ok(written)
        });
        drop(partial);
        match written {
            Err(_) if cancel.cancelled() => Err(BackupError::Refused(BACKUP_STOPPED)),
            written => written,
        }
    }

    fn write(mut self, file: File) -> Result<(u64, u32), BackupError> {
        let mut random = [0_u8; 23];
        random_bytes(&mut random);
        let header = bruhs_backup::Header::new(Kind::Full, Kdf::Argon2id(self.kdf), random);
        let key = {
            let password = Zeroizing::new(normalize_password(&self.password));
            Key::derive(&password, &header).map_err(io::Error::other)?
        };
        self.password = Zeroizing::new(String::new());
        let sink = Encryptor::new(BufWriter::new(file), &header, &key)?;
        drop(key);
        let mut archive = Archive {
            tar: tar::Builder::new(sink),
            folders: HashSet::new(),
            listed: Vec::new(),
        };

        let files = std::mem::take(&mut self.files);
        for file in &files {
            self.cancel.check()?;
            let prefix = match file.root {
                RootKey::Core => "core",
                RootKey::Desktop => "desktop",
            };
            let path = format!("{prefix}/{}", file.rel);
            #[cfg(test)]
            self.hook(super::host::Hook::BeforeFile, &path);
            archive.folders_for(&path)?;
            if file.sqlite {
                self.snapshot(&mut archive, file, &path)?;
            } else {
                self.copy(&mut archive, file, &path)?;
            }
        }
        // Only the data files written: one gone since the walk isn't in.
        self.manifest.counts.files = u64::try_from(archive.listed.len()).unwrap_or(u64::MAX);
        if let Some(json) = self.keychain.take() {
            archive.secret("keychain.json", &json)?;
        }
        if let Some((part, bytes)) = self.web.take() {
            self.stream(&mut archive, "web/part.bin", part, bytes, PRIVATE)?;
        }
        // The exact count and size, so nothing made here fails to open (B
        // Task 7 I1): the folders, the listed files and the manifest.
        let entries = archive.folders.len() + archive.listed.len() + 1;
        self.manifest.files = std::mem::take(&mut archive.listed);
        self.manifest.changed_files = self.changed;
        let manifest = serde_json::to_vec(&self.manifest).map_err(io::Error::other)?;
        if u64::try_from(entries).map_or(true, |entries| entries > self.limits.entries)
            || u64::try_from(manifest.len())
                .map_or(true, |bytes| bytes > self.limits.manifest_bytes)
        {
            return Err(BackupError::Refused(BACKUP_TOO_MANY));
        }
        archive.plain("manifest.json", &manifest)?;
        self.progress.flush();

        let sink = archive.tar.into_inner()?;
        let file = sink
            .finish()?
            .into_inner()
            .map_err(io::IntoInnerError::into_error)?;
        file.sync_all()?;
        Ok((file.metadata()?.len(), self.changed))
    }

    /// A database of Bruhs' own: snapshotted through SQLite into `tmp`,
    /// streamed, then deleted (ruling F15).
    fn snapshot(
        &mut self,
        archive: &mut Archive,
        file: &SourceFile,
        path: &str,
    ) -> Result<(), BackupError> {
        if RootId::of(&file.base)? != file.base_id {
            return Err(root_changed());
        }
        let copy = Snapshot(self.tmp.join(format!("{}.sqlite3", random_id())));
        snapshot_sqlite(&file.path, &copy.0)?;
        let opened = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&copy.0)?;
        let bytes = opened.metadata()?.len();
        self.stream(archive, path, opened, bytes, PRIVATE)
    }

    /// Any other file, opened through its folder handles, with the owner's
    /// execute bit kept (ruling B Task 6 I1). The header carries the size
    /// taken before the read (D6, as amended by ruling B Task 6 I2): a file
    /// that grows or whose time changes while it is read keeps that much
    /// and is counted; only one that shrinks below it mid-read fails the
    /// backup with `FILE_CHANGED`.
    fn copy(
        &mut self,
        archive: &mut Archive,
        file: &SourceFile,
        path: &str,
    ) -> Result<(), BackupError> {
        #[cfg(test)]
        let reopening = || self.hook(super::host::Hook::Reopening, path);
        #[cfg(not(test))]
        let reopening = || {};
        let Some(Settled {
            file: opened,
            bytes,
            metadata: before,
            moving,
        }) = open_settled(file, &reopening)?
        else {
            return Ok(());
        };
        let mode = if before.permissions().mode() & 0o100 == 0 {
            PRIVATE
        } else {
            RUNNABLE
        };
        #[cfg(test)]
        self.hook(super::host::Hook::Opened, path);
        let check = opened.try_clone()?;
        self.stream(archive, path, opened, bytes, mode)?;
        #[cfg(test)]
        self.hook(super::host::Hook::Read, path);
        let after = check.metadata()?;
        if moving || after.len() != bytes || after.modified().ok() != before.modified().ok() {
            self.changed = self.changed.saturating_add(1);
        }
        Ok(())
    }

    /// Appends `bytes` bytes of `source` at `path` with `mode`, hashing
    /// them. Fewer bytes than that is `FILE_CHANGED`.
    fn stream(
        &mut self,
        archive: &mut Archive,
        path: &str,
        source: File,
        bytes: u64,
        mode: u32,
    ) -> Result<(), BackupError> {
        let mut reader = Hashing {
            inner: source.take(bytes),
            hasher: Sha256::new(),
            read: 0,
            cancel: &self.cancel,
            progress: &mut self.progress,
        };
        let mut header = file_header(bytes, mode);
        match archive.tar.append_data(&mut header, path, &mut reader) {
            Ok(()) if reader.read == bytes => {}
            // A short entry leaves the tar unusable: the job ends here.
            Ok(()) => return Err(BackupError::Refused(FILE_CHANGED)),
            Err(error) => return Err(error.into()),
        }
        let sha256 = hex(&reader.hasher.finalize());
        archive.listed.push(ManifestFile {
            path: path.to_owned(),
            bytes,
            sha256,
        });
        Ok(())
    }
}

/// The mode of every entry but an owner-runnable file.
const PRIVATE: u32 = 0o600;
/// A file the owner could run: a restore keeps it runnable (ruling F32).
const RUNNABLE: u32 = 0o700;

/// A snapshot in `backups/tmp`, deleted when dropped.
struct Snapshot(PathBuf);

impl Drop for Snapshot {
    fn drop(&mut self) {
        // Best effort: the sweep removes it at the next start otherwise.
        let _ = std::fs::remove_file(&self.0);
    }
}

fn root_changed() -> BackupError {
    BackupError::Io(io::Error::other("a data root changed since the walk"))
}

/// Opens a listed file and answers it with the size to read and its
/// metadata then, or `None` when it is gone or became a link since the
/// walk. A size that differs from the walk's opens the file again once:
/// a size that holds is read whole and isn't counted. A size still moving
/// is read at the smaller of the second open's size and the walk's, so a
/// file that has become smaller is read at that smaller size, and the file
/// counts as changed (ruling B Task 6 N2). Only a shrink during the read
/// fails, with `FILE_CHANGED`.
fn open_settled(file: &SourceFile, reopening: &dyn Fn()) -> Result<Option<Settled>, BackupError> {
    let open = || match open_source(file) {
        Ok(opened) => {
            let metadata = opened.metadata()?;
            Ok(Some((opened, metadata)))
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) if matches!(error.raw_os_error(), Some(libc::ELOOP | libc::ENOTDIR)) => Ok(None),
        Err(error) => Err(BackupError::from(error)),
    };
    let Some((opened, first)) = open()? else {
        return Ok(None);
    };
    if first.len() == file.bytes {
        return Ok(Some(Settled {
            file: opened,
            bytes: file.bytes,
            metadata: first,
            moving: false,
        }));
    }
    drop(opened);
    reopening();
    let Some((opened, second)) = open()? else {
        return Ok(None);
    };
    let moving = second.len() != first.len();
    Ok(Some(Settled {
        file: opened,
        // Still moving: at most the walk's size, never more than is there.
        bytes: if moving {
            second.len().min(file.bytes)
        } else {
            second.len()
        },
        metadata: second,
        moving,
    }))
}

/// A listed file opened for reading.
struct Settled {
    file: File,
    /// The size its entry gets.
    bytes: u64,
    /// Its metadata when it was opened.
    metadata: Metadata,
    /// Its size still moved between the two opens: counted as changed.
    moving: bool,
}

/// Whether a failed `link` means the volume has no hard links (FAT and
/// exFAT drives, some network shares), not some other refusal.
fn no_hard_links(error: &io::Error) -> bool {
    error.raw_os_error().is_some_and(|code| {
        [libc::EXDEV, libc::EPERM, libc::ENOTSUP, libc::EOPNOTSUPP].contains(&code)
    })
}

/// Moves the finished partial file into place. Without `overwrite` an
/// existing file is never replaced, even one that appeared during the job.
/// The partial file is already 0600.
fn place(partial: &Path, target: &Path, overwrite: bool) -> Result<(), BackupError> {
    if overwrite {
        std::fs::rename(partial, target)?;
    } else {
        match std::fs::hard_link(partial, target) {
            // The backup is in place; a partial name that stays is the
            // sweep's (ruling B Task 6, outside the findings).
            Ok(()) => {
                if std::fs::remove_file(partial).is_err() {
                    tracing::warn!("Bruhs couldn't remove a finished backup's partial name");
                }
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                return Err(BackupError::Refused(DESTINATION_EXISTS));
            }
            // A volume without hard links: check, then rename.
            Err(error) if no_hard_links(&error) => {
                if std::fs::symlink_metadata(target).is_ok() {
                    return Err(BackupError::Refused(DESTINATION_EXISTS));
                }
                std::fs::rename(partial, target)?;
            }
            Err(error) => return Err(error.into()),
        }
    }
    if let Some(folder) = target.parent() {
        // Best effort: makes the rename durable where folders can sync.
        let _ = File::open(folder).and_then(|folder| folder.sync_all());
    }
    Ok(())
}

/// The tar being written.
struct Archive {
    tar: tar::Builder<Sink>,
    /// Folders already written.
    folders: HashSet<String>,
    /// Every file written, for the manifest.
    listed: Vec<ManifestFile>,
}

impl Archive {
    /// Writes a folder entry for each folder above `path` that isn't
    /// written yet, never `core` or `desktop` itself (ruling F16) and
    /// without a trailing slash.
    fn folders_for(&mut self, path: &str) -> io::Result<()> {
        let mut ends = path.match_indices('/').map(|(at, _)| at).skip(1);
        for end in ends.by_ref() {
            let folder = &path[..end];
            if self.folders.insert(folder.to_owned()) {
                let mut header = tar::Header::new_gnu();
                header.set_entry_type(tar::EntryType::Directory);
                header.set_mode(0o700);
                header.set_size(0);
                header.set_mtime(0);
                self.tar.append_data(&mut header, folder, io::empty())?;
            }
        }
        Ok(())
    }

    /// A small file that holds no secret, such as the manifest.
    fn plain(&mut self, path: &str, data: &[u8]) -> io::Result<()> {
        let mut header = file_header(
            u64::try_from(data.len()).map_err(io::Error::other)?,
            PRIVATE,
        );
        self.tar.append_data(&mut header, path, data)
    }

    /// `keychain.json`: written straight into the encryptor, whose chunk
    /// buffer is wiped, so no copy buffer holds a value. Listed with its
    /// hash (ruling F16).
    fn secret(&mut self, path: &str, data: &[u8]) -> io::Result<()> {
        let bytes = u64::try_from(data.len()).map_err(io::Error::other)?;
        let mut header = file_header(bytes, PRIVATE);
        header.set_path(path)?;
        header.set_cksum();
        let out = self.tar.get_mut();
        out.write_all(header.as_bytes())?;
        out.write_all(data)?;
        let padding = (512 - data.len() % 512) % 512;
        out.write_all(&[0; 512][..padding])?;
        self.listed.push(ManifestFile {
            path: path.to_owned(),
            bytes,
            sha256: hex(&Sha256::digest(data)),
        });
        Ok(())
    }
}

/// A regular file's header: the size, `mode`, and no owner or time.
fn file_header(bytes: u64, mode: u32) -> tar::Header {
    let mut header = tar::Header::new_gnu();
    header.set_entry_type(tar::EntryType::Regular);
    header.set_mode(mode);
    header.set_size(bytes);
    header.set_mtime(0);
    header
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Reads a file's bytes while hashing them, counting progress and
/// stopping when the job is cancelled.
struct Hashing<'a, R> {
    inner: R,
    hasher: Sha256,
    read: u64,
    cancel: &'a CancelFlag,
    progress: &'a mut Progress,
}

impl<R: Read> Read for Hashing<'_, R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.cancel.cancelled() {
            return Err(io::Error::other("the backup was stopped"));
        }
        let read = self.inner.read(buf)?;
        self.hasher.update(&buf[..read]);
        let read_bytes = u64::try_from(read).map_err(io::Error::other)?;
        self.read = self.read.saturating_add(read_bytes);
        self.progress.add(read_bytes);
        Ok(read)
    }
}

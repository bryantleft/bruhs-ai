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
use std::fs::{File, OpenOptions};
use std::io::{self, BufWriter, Read, Seek, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use bruhs_backup::{Encryptor, Kdf, Key, Kind};
use bruhs_models::backup::{
    backup_file_name, browser_busy_line, no_space, normalize_password, skipped_names_line,
    validate_password, BackupApp, BackupCounts, BackupFile, BackupManifest, BackupOptions,
    JobState, JobStep, LastBackup, ManifestFile, BACKUP_STOPPED, DESTINATION_EXISTS,
    DESTINATION_INSIDE, DESTINATION_MISSING, FILE_CHANGED, KEYCHAIN_ITEM_REFUSED,
    KEYCHAIN_NEEDS_YOU, KEYCHAIN_READ_FAILED, MANIFEST_FORMAT, NOT_A_BACKUP, ROOT_NAME_REFUSED,
    WEB_TOO_LARGE, WEB_UPLOAD_MAX,
};
use bruhs_models::crew::CREW_PROTOCOL;
use bruhs_models::EpochMs;
use sha2::{Digest, Sha256};
use tokio::sync::watch;
use zeroize::Zeroizing;

use super::host::{new_private_file, random_bytes, random_id, web_part_header, BackupHost, Config};
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
/// real folder, never a link.
pub(super) fn private_dir(folder: &Path) -> Result<(), BackupError> {
    match std::fs::symlink_metadata(folder) {
        Ok(metadata) if metadata.is_dir() => return Ok(()),
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
    let made_at = EpochMs::now();

    // Neither root's `backups` may be a link (B Task 5 I1 ruling).
    private_dir(&roots.backups(RootKey::Core))?;
    if !roots.same() {
        match std::fs::symlink_metadata(roots.backups(RootKey::Desktop)) {
            Ok(metadata) if !metadata.is_dir() => return Err(not_a_folder()),
            _ => {}
        }
    }
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

    let total = files
        .iter()
        .map(|file| file.bytes)
        .chain(web.as_ref().map(|(_, bytes)| *bytes))
        .fold(0_u64, u64::saturating_add);
    let folder = target
        .parent()
        .ok_or(BackupError::Refused(DESTINATION_MISSING))?;
    let free = free_bytes(folder)?;
    let needed = total.saturating_add(FREE_MARGIN);
    if free < needed {
        return Err(BackupError::Space(no_space(needed - free)));
    }

    let keychain = if options.keychain {
        send(progress, 0, total, JobStep::Keychain);
        Some(read_keychain(&config, &roots).await?)
    } else {
        None
    };
    cancel.check()?;

    let keychain_items = keychain.as_ref().map_or(0, |(_, count)| *count);
    let manifest = BackupManifest {
        format: MANIFEST_FORMAT,
        app_version: env!("CARGO_PKG_VERSION").to_owned(),
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
        counts: BackupCounts {
            bruhs: bruhs.count,
            vault_entries: vault_entries(&roots),
            keychain_items,
            files: u64::try_from(files.len()).unwrap_or(u64::MAX),
        },
    };
    let writer = Writer {
        files,
        keychain: keychain.map(|(json, _)| json),
        web,
        password,
        kdf: config.kdf,
        tmp,
        cancel: cancel.clone(),
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
    let bytes = tokio::task::spawn_blocking(move || writer.run(&place, overwrite))
        .await
        .map_err(|_| stopped())??;

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
    count: u32,
}

async fn bruhs_here(config: &Config) -> Bruhs {
    let Some(runtime) = &config.reactive else {
        return Bruhs::default();
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
        count: u32::try_from(snapshot.bruhs.len()).unwrap_or(u32::MAX),
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
/// Task 4 I2), in a wiped buffer sized once, and how many there are. A
/// read that waits or fails stops the backup.
async fn read_keychain(
    config: &Config,
    roots: &Roots,
) -> Result<(Zeroizing<Vec<u8>>, u32), BackupError> {
    let refs = keychain_inventory(
        &roots.core.join("credentials"),
        &roots.core.join("connections"),
        &roots.config.join("calendar").join("google"),
    )
    .map_err(|_| BackupError::Keychain(KEYCHAIN_READ_FAILED))?;
    let (items, _missing) = read_items(&config.store, &refs, config.keychain_wait)
        .await
        .map_err(|error| {
            BackupError::Keychain(match error {
                ItemsError::NeedsOwner => KEYCHAIN_NEEDS_YOU,
                ItemsError::Storage => KEYCHAIN_READ_FAILED,
                ItemsError::Refused => KEYCHAIN_ITEM_REFUSED,
            })
        })?;
    let count = u32::try_from(items.len()).unwrap_or(u32::MAX);
    Ok((keychain_json(&items)?, count))
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
    tmp: PathBuf,
    cancel: CancelFlag,
    progress: Progress,
    manifest: BackupManifest,
    #[cfg(test)]
    before_file: Option<Arc<dyn Fn() + Send + Sync>>,
}

type Sink = Encryptor<BufWriter<File>>;

impl Writer {
    /// Writes the backup beside `target` and moves it into place. Answers
    /// its size.
    fn run(self, target: &Path, overwrite: bool) -> Result<u64, BackupError> {
        let (Some(folder), Some(name)) = (target.parent(), target.file_name()) else {
            return Err(BackupError::Refused(DESTINATION_MISSING));
        };
        let partial = folder.join(format!(
            ".{}.partial-{}",
            name.to_string_lossy(),
            random_id()
        ));
        let file = new_private_file(&partial)?;
        let cancel = self.cancel.clone();
        let written = self
            .write(file)
            .and_then(|bytes| place(&partial, target, overwrite).map(|()| bytes));
        match written {
            Ok(bytes) => Ok(bytes),
            Err(error) => {
                // Best effort: the file is incomplete and 0600.
                let _ = std::fs::remove_file(&partial);
                if cancel.cancelled() {
                    Err(BackupError::Refused(BACKUP_STOPPED))
                } else {
                    Err(error)
                }
            }
        }
    }

    fn write(mut self, file: File) -> Result<u64, BackupError> {
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
            #[cfg(test)]
            if let Some(hook) = &self.before_file {
                hook();
            }
            let prefix = match file.root {
                RootKey::Core => "core",
                RootKey::Desktop => "desktop",
            };
            let path = format!("{prefix}/{}", file.rel);
            archive.folders_for(&path)?;
            if file.sqlite {
                self.snapshot(&mut archive, file, &path)?;
            } else {
                self.copy(&mut archive, file, &path)?;
            }
        }
        if let Some(json) = self.keychain.take() {
            archive.secret("keychain.json", &json)?;
        }
        if let Some((part, bytes)) = self.web.take() {
            self.stream(&mut archive, "web/part.bin", part, bytes)?;
        }
        self.manifest.files = std::mem::take(&mut archive.listed);
        let manifest = serde_json::to_vec(&self.manifest).map_err(io::Error::other)?;
        archive.plain("manifest.json", &manifest)?;
        self.progress.flush();

        let sink = archive.tar.into_inner()?;
        let file = sink
            .finish()?
            .into_inner()
            .map_err(io::IntoInnerError::into_error)?;
        file.sync_all()?;
        Ok(file.metadata()?.len())
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
        self.stream(archive, path, opened, bytes)
    }

    /// Any other file, opened through its folder handles. One whose size
    /// differs from the walk's is opened again once; a size that still
    /// moves, or changes while it is read, is `FILE_CHANGED`.
    fn copy(
        &mut self,
        archive: &mut Archive,
        file: &SourceFile,
        path: &str,
    ) -> Result<(), BackupError> {
        let Some((opened, bytes)) = open_settled(file)? else {
            return Ok(());
        };
        let check = opened.try_clone()?;
        self.stream(archive, path, opened, bytes)?;
        if check.metadata()?.len() != bytes {
            return Err(BackupError::Refused(FILE_CHANGED));
        }
        Ok(())
    }

    /// Appends `bytes` bytes of `source` at `path`, hashing them.
    fn stream(
        &mut self,
        archive: &mut Archive,
        path: &str,
        source: File,
        bytes: u64,
    ) -> Result<(), BackupError> {
        let mut reader = Hashing {
            inner: source.take(bytes),
            hasher: Sha256::new(),
            read: 0,
            cancel: &self.cancel,
            progress: &mut self.progress,
        };
        let mut header = file_header(bytes);
        archive.tar.append_data(&mut header, path, &mut reader)?;
        if reader.read != bytes {
            return Err(BackupError::Refused(FILE_CHANGED));
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

/// Opens a listed file and answers it with the size to read, or `None`
/// when it is gone or became a link since the walk.
fn open_settled(file: &SourceFile) -> Result<Option<(File, u64)>, BackupError> {
    let mut expected = file.bytes;
    for attempt in 0..2 {
        let opened = match open_source(file) {
            Ok(opened) => opened,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) if matches!(error.raw_os_error(), Some(libc::ELOOP | libc::ENOTDIR)) => {
                return Ok(None)
            }
            Err(error) => return Err(error.into()),
        };
        let bytes = opened.metadata()?.len();
        if bytes == expected {
            return Ok(Some((opened, bytes)));
        }
        if attempt == 1 {
            break;
        }
        expected = bytes;
    }
    Err(BackupError::Refused(FILE_CHANGED))
}

/// Moves the finished partial file into place. Without `overwrite` an
/// existing file is never replaced, even one that appeared during the job.
fn place(partial: &Path, target: &Path, overwrite: bool) -> Result<(), BackupError> {
    if overwrite {
        std::fs::rename(partial, target)?;
    } else {
        match std::fs::hard_link(partial, target) {
            Ok(()) => std::fs::remove_file(partial)?,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                return Err(BackupError::Refused(DESTINATION_EXISTS));
            }
            // A volume without hard links: check, then rename.
            Err(_) => {
                if std::fs::symlink_metadata(target).is_ok() {
                    return Err(BackupError::Refused(DESTINATION_EXISTS));
                }
                std::fs::rename(partial, target)?;
            }
        }
    }
    let mode = std::fs::Permissions::from_mode(0o600);
    std::fs::set_permissions(target, mode)?;
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
        let mut header = file_header(u64::try_from(data.len()).map_err(io::Error::other)?);
        self.tar.append_data(&mut header, path, data)
    }

    /// `keychain.json`: written straight into the encryptor, whose chunk
    /// buffer is wiped, so no copy buffer holds a value. Listed with its
    /// hash (ruling F16).
    fn secret(&mut self, path: &str, data: &[u8]) -> io::Result<()> {
        let bytes = u64::try_from(data.len()).map_err(io::Error::other)?;
        let mut header = file_header(bytes);
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

/// A regular file's header: the size, mode 0600, and no owner or time.
fn file_header(bytes: u64) -> tar::Header {
    let mut header = tar::Header::new_gnu();
    header.set_entry_type(tar::EntryType::Regular);
    header.set_mode(0o600);
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

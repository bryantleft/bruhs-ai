//! Applying a restore offline (D16, D18; rulings F10 c, F11, F20, F21 b,
//! F33): swapping a staged restore or a kept pre-restore set into both
//! roots, with today's data and keychain items kept aside.
//!
//! The apply runs only where nothing else writes: it takes
//! `backups/restore.lock` first and holds it throughout, then the reactive
//! database's lock and the desktop's instance lock, none of them waiting.
//! It follows the journal in `backups/apply.json` (see [`super::journal`]),
//! so a stop at any point resumes at the next start and ends with either
//! today's data or the restored data in place, never a mix:
//!
//! 1. seal today's keychain items into the pre-restore set (D13);
//! 2. root by root, rename today's top-level entries the backup covers into
//!    `backups/pre-restore-<stamp>/`, then the source's entries into the
//!    root (one pass when both roots are one folder, ruling F33);
//! 3. write the source's items, then remove today's items the restored
//!    catalogs don't name (they are in the sealed set);
//! 4. move the source's browser part to `backups/web-part/<id>.bin`;
//! 5. write the set's `meta/info.json` and `backups/result.json`;
//! 6. remove the source, its seal key, `pending.json` and the journal.
//!
//! Before step 3, a rename that fails for good undoes the renames and
//! answers `RESTORE_ROLLED_BACK` ([`rollback`]); from step 3 on, the apply
//! only goes forward, since only keychain errors remain and they pass on a
//! retry.
//!
//! Every rename stays on one volume and is followed by an fsync of both
//! folders before the journal records it. Staged data was synced when it
//! was staged. Nothing here logs a path, a name from inside a backup or a
//! keychain value.

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::fs::{DirBuilder, File, OpenOptions, TryLockError};
use std::io::{self, Write};
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

use bruhs_models::backup::{
    backup_file_name, desktop_entry, folded_entry_key, included, items_stayed_line, valid_id,
    valid_stamp, BackupManifest, BackupOptions, PathRewrite, PendingRestore, RestorePreview,
    RestoreResult, RestoreSource, WebPartInfo, BACKUP_BUSY, DAMAGED, EXCLUDED_TOP, GONE,
    INVALID_REQUEST, KEEP_ITEMS_TOO_LARGE, KEYCHAIN_ITEM_REFUSED, KEYCHAIN_NEEDS_YOU,
    KEYCHAIN_READ_FAILED, KEYCHAIN_SET_UNREADABLE, MANIFEST_MAX_BYTES, PARKED_STAYED,
    REACTIVE_DB_RELATIVE, RESERVED_TOP, RESTORE_CANT_MOVE, RESTORE_PENDING,
};
use bruhs_models::EpochMs;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use zeroize::Zeroizing;

use super::create::{backups_are_folders, private_dir};
use super::host::{new_private_file, random_bytes, random_id, read_small, PENDING};
use super::journal::{
    full_sync, single_component, sync_dir, write_atomically, Journal, Pass, Step, JOURNAL,
};
use super::open::staging_dir;
use super::{BackupError, RootKey, Roots};
use crate::secrets::{
    keychain_inventory, read_seal_key, seal_items, store_seal_key, unseal_items, ItemRef,
    KeychainItem, SecretError, SecretStore, BACKUP_SERVICE,
};

mod confirm;
mod kept;
mod rollback;

pub use confirm::{
    check_restore, desktop_running, finish_web_part, unschedule, waiting_web_part_file,
};
use kept::{check_kept, hold_restore_lock, kept_account, kept_dir, kept_info, park};
pub use kept::{delete_pre_restore, keep_previous_web_part, pre_restores};
pub(super) use kept::{forget_key, sweep_restore};

/// `<core>/logs/restore.log` (see [`open_restore_log`]).
const RESTORE_LOG: &str = "restore.log";
/// The apply's own lock, inside `<core>/backups`, which no apply moves.
pub(super) const RESTORE_LOCK: &str = "restore.lock";
/// The desktop app's instance lock, at the top of the desktop root (D17).
const INSTANCE_LOCK: &str = "desktop.instance.lock";
/// Any running server holds this lock beside its reactive database.
const REACTIVE_LOCK: &str = "reactive.sqlite3.lock";
const REACTIVE_DB: &str = "reactive.sqlite3";
const REACTIVE_DB_ENV: &str = "BRUHS_REACTIVE_DB";
const SERVER: &str = "server";
pub(super) const RESULT: &str = "result.json";
/// `<core>/backups/web-part/<id>.bin`: a restored browser part waiting for
/// the browser (D19).
const WEB_PARTS: &str = "web-part";
const KEPT_PREFIX: &str = "pre-restore-";
/// The folder of a staged restore or kept set that isn't data.
const META: &str = RESERVED_TOP;
const INFO: &str = "info.json";
const PREVIEW: &str = "preview.json";
const MANIFEST: &str = "manifest.json";
const SEALED: &str = "keychain.sealed";
const WEB_PART: &str = "web-part.bin";
/// The top-level folder that, on Linux, lives in the config root.
const CALENDAR: &str = "calendar";
const CREDENTIALS: &str = "credentials";
const CONNECTIONS: &str = "connections";
const GOOGLE: &str = "google";
/// `pending.json`, `result.json`, `info.json` and a preview are small.
const SMALL_MAX: u64 = 1 << 20;
/// A sealed set holds at most a backup's `keychain.json` and its framing.
const SEALED_MAX: u64 = 16 << 20;
/// What a backup could take from a root: D4's rule with every option on.
const EVERYTHING: BackupOptions = BackupOptions {
    keychain: true,
    audio: true,
    browser_profiles: true,
};
/// How long a lock that reads as held is tried again before it counts as
/// held (see [`lock`]).
const LOCK_GRACE: std::time::Duration = std::time::Duration::from_millis(250);
/// How long [`take_service_restart`] waits for an apply's lock.
const RESTORE_LOCK_WAIT: std::time::Duration = std::time::Duration::from_secs(5);
/// The minute of a stamp gets a random suffix; this many tries find a
/// free one.
const STAMP_TRIES: usize = 64;

/// Who holds a lock the apply needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Holder {
    /// A running server (the reactive database's lock, wherever
    /// `BRUHS_REACTIVE_DB` puts it, or, for an apply a stop cut short, a
    /// relative override whose lock can't be found).
    Server,
    /// A running desktop app (its instance lock).
    Desktop,
    /// Another apply (`backups/restore.lock`).
    Restore,
}

/// What [`apply_pending`] did.
#[derive(Debug, Clone, PartialEq)]
pub enum ApplyOutcome {
    /// No restore waits.
    NothingPending,
    /// The restore is in place.
    Applied(RestoreResult),
    /// A lock is held; nothing was changed.
    Busy(Holder),
}

/// Where an apply asks a test's hook whether to stop, as a kill would.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Point {
    /// Before a rename's work. A yes fails the rename for good (`EACCES`),
    /// as a folder the owner can't move would.
    Before,
    /// After a step's work, before the journal records it.
    Acted,
    /// Right after the journal records it.
    Recorded,
    /// After a rollback undid a recorded rename.
    Undone,
}

/// `pending.json`: the restore a start finishes.
#[derive(Serialize, Deserialize)]
struct PendingFile {
    source: RestoreSource,
    created_at_ms: EpochMs,
    #[serde(default)]
    service_to_restart: bool,
}

/// `result.json`: the last restore applied here.
#[derive(Serialize, Deserialize, Default)]
#[serde(default)]
struct ResultFile {
    applied_at_ms: EpochMs,
    stamp: String,
    warnings: Vec<String>,
    service_to_restart: bool,
    web_part: Option<WebPartInfo>,
    /// The kept set the restore that brought `web_part` made: the
    /// browser's data from before that restore joins it, whatever sets
    /// came after (final review B minor).
    #[serde(default)]
    web_part_stamp: Option<String>,
}

impl ResultFile {
    fn result(&self) -> RestoreResult {
        RestoreResult {
            applied_at_ms: self.applied_at_ms,
            stamp: self.stamp.clone(),
            warnings: self.warnings.clone(),
        }
    }
}

/// What a source brings, once checked.
struct SourceSet {
    places: Vec<(RootKey, Vec<String>)>,
    keychain: bool,
    web_part: bool,
    rewrites: Vec<PathRewrite>,
    /// The crew protocol of a backup's data; `None` for a kept set, which
    /// is this computer's own.
    protocol: Option<u32>,
}

/// `<desktop root>/desktop.instance.lock` (D17, ruling F21 b): the same
/// file whatever `BRUHS_DESKTOP_STATE_FILE` says.
pub fn instance_lock_path(desktop_root: &Path) -> PathBuf {
    desktop_root.join(INSTANCE_LOCK)
}

/// Takes the lock at `path` without waiting, creating the file (0600,
/// never through a link). `None` when another handle holds it. Keep the
/// file for as long as the lock should last.
pub fn take_instance_lock(path: &Path) -> io::Result<Option<File>> {
    match lock(open_lock_file(path)?)? {
        Lock::Free(file) => Ok(file),
        Lock::Held => Ok(None),
    }
}

/// Schedules `source` to be applied at the next start (D15): writes and
/// syncs `backups/pending.json`. Refused while a restore waits or runs
/// (`RESTORE_PENDING`), while an apply holds `restore.lock`
/// (`BACKUP_BUSY`), for a source that isn't a finished staged restore or
/// a whole kept set (`GONE`), and for a staging that holds data its
/// manifest doesn't name (`DAMAGED`).
pub fn schedule(
    roots: &Roots,
    source: &RestoreSource,
    service_to_restart: bool,
) -> Result<PendingRestore, BackupError> {
    backups_are_folders(roots)?;
    let backups = roots.backups(RootKey::Core);
    new_dir(&backups)?;
    let Lock::Free(_restore_lock) = lock(open_lock_file(&backups.join(RESTORE_LOCK))?)? else {
        return Err(BackupError::Busy(BACKUP_BUSY));
    };
    if exists(&backups.join(PENDING))? || exists(&backups.join(JOURNAL))? {
        return Err(BackupError::Refused(RESTORE_PENDING));
    }
    if check_source(roots, source)?.is_none() {
        return Err(BackupError::Refused(GONE));
    }
    let pending = PendingFile {
        source: source.clone(),
        created_at_ms: EpochMs::now(),
        service_to_restart,
    };
    write_atomically(&backups, PENDING, &to_json(&pending)?)?;
    Ok(PendingRestore {
        source: pending.source,
        created_at_ms: pending.created_at_ms,
    })
}

/// Applies the waiting restore, or finishes the one a stop interrupted
/// (D16, ruling F20). `desktop_lock` is the caller's own instance lock
/// when the caller is the desktop app. A held lock is `Busy` and changes
/// nothing. With `BRUHS_REACTIVE_DB` set, the server's lock is checked
/// beside the database it names (final review C-I1); with a relative
/// override, whose lock this can't find, a fresh apply is refused
/// (`REACTIVE_DB_RELATIVE`) and one a stop cut short is `Busy`. A `backups`
/// folder that is a link is never acted on.
///
/// A restore that can never be applied (its source isn't there whole, its
/// sealed items don't open, a rename can't happen, or `pending.json` is
/// damaged) is refused and `pending.json` removed, so it doesn't block
/// backups (B Task 8 I2). A rename that fails for good mid-apply puts
/// today's data back and answers `RESTORE_ROLLED_BACK` (I1).
///
/// It blocks on the file system and the keychain: call it before the
/// stores open, or from a blocking task.
pub fn apply_pending(
    roots: &Roots,
    store: &SecretStore,
    desktop_lock: Option<&File>,
) -> Result<ApplyOutcome, BackupError> {
    apply_pending_with(
        roots,
        store,
        desktop_lock,
        &|name| std::env::var_os(name),
        &|_, _| false,
    )
}

/// [`apply_pending`], reading the environment through `env` and asking
/// `stop` at each [`Point`] whether a test stops or fails the apply there.
pub(super) fn apply_pending_with(
    roots: &Roots,
    store: &SecretStore,
    desktop_lock: Option<&File>,
    env: &dyn Fn(&str) -> Option<OsString>,
    stop: &dyn Fn(&Step, Point) -> bool,
) -> Result<ApplyOutcome, BackupError> {
    backups_are_folders(roots)?;
    let backups = roots.backups(RootKey::Core);
    if !real_dir(&backups) {
        return Ok(ApplyOutcome::NothingPending);
    }
    // F20 a: the apply's own lock first, held for the whole apply.
    let Lock::Free(_restore_lock) = lock(open_lock_file(&backups.join(RESTORE_LOCK))?)? else {
        return Ok(ApplyOutcome::Busy(Holder::Restore));
    };
    let mut journal = Journal::load(roots)?;
    if let Some(unmoved) = journal.take_if(|journal| moved_nothing(roots, journal)) {
        // Nothing moved yet (final review B minor): today's data is whole,
        // so the apply starts over from `pending.json` with a fresh plan.
        drop_unmoved(roots, store, &backups, &unmoved)?;
    }
    let work = match journal {
        Some(journal) => Work::Resume(journal),
        None => match read_pending(&backups) {
            Ok(Some(pending)) => Work::Start(pending),
            Ok(None) => return Ok(ApplyOutcome::NothingPending),
            // Damaged, or a source this build can't read.
            // Its service flag can't be read either: a service it stopped
            // stays down until something starts it (a recorded limit).
            Err(error @ (BackupError::Format(_) | BackupError::Refused(_))) => {
                tracing::warn!("a damaged pending restore was removed: {}", error.message());
                return Err(refuse(&backups, error, false));
            }
            Err(error) => return Err(error),
        },
    };
    // Final review C-I1: the server's lock where its database is, even
    // when `BRUHS_REACTIVE_DB` moved it; the swap never touches that file.
    // A relative override's lock can't be found: a fresh apply is refused
    // for good, keeping the service flag, rather than wait forever; one a
    // stop cut short waits, since today's data may be half placed.
    let Some(server_lock) = reactive_lock_path(roots, env) else {
        return match work {
            Work::Start(pending) => Err(refuse(
                &backups,
                BackupError::Refused(REACTIVE_DB_RELATIVE),
                pending.service_to_restart,
            )),
            Work::Resume(_) => Ok(ApplyOutcome::Busy(Holder::Server)),
        };
    };
    let Lock::Free(_server_lock) = lock_existing(&server_lock)? else {
        return Ok(ApplyOutcome::Busy(Holder::Server));
    };
    let desktop = instance_lock_path(&roots.desktop);
    let _desktop_lock = if same_file(desktop_lock, &desktop) {
        None
    } else {
        match lock_existing(&desktop)? {
            Lock::Free(file) => file,
            Lock::Held => return Ok(ApplyOutcome::Busy(Holder::Desktop)),
        }
    };
    let journal = match work {
        Work::Resume(journal) => journal,
        Work::Start(pending) => plan(roots, store, &backups, pending)?,
    };
    Apply {
        roots,
        store,
        backups: &backups,
        journal,
        stop,
    }
    .run()
}

/// The lock a running server holds on its reactive database: beside
/// `<core>/server/reactive.sqlite3`, or beside the database a
/// `BRUHS_REACTIVE_DB` override names, resolved as the store resolves it
/// (final review C-I1). `None` for a relative override, whose folder
/// depends on where the server started: its lock can't be checked here.
pub fn reactive_lock_path(
    roots: &Roots,
    env: &dyn Fn(&str) -> Option<OsString>,
) -> Option<PathBuf> {
    let Some(path) = env(REACTIVE_DB_ENV).filter(|path| !path.is_empty()) else {
        return Some(roots.core.join(SERVER).join(REACTIVE_LOCK));
    };
    let path = PathBuf::from(path);
    if path == roots.core.join(SERVER).join(REACTIVE_DB) {
        return Some(roots.core.join(SERVER).join(REACTIVE_LOCK));
    }
    if !path.is_absolute() {
        return None;
    }
    // As the store does: the file itself when it is there, else its
    // folder and its name. A folder that isn't there holds no lock.
    let resolved = std::fs::canonicalize(&path).ok().or_else(|| {
        let name = path.file_name()?;
        let folder = std::fs::canonicalize(path.parent()?).ok()?;
        Some(folder.join(name))
    });
    let mut lock = resolved.unwrap_or(path).into_os_string();
    lock.push(".lock");
    Some(PathBuf::from(lock))
}

/// The restore waiting to be applied.
pub fn pending(roots: &Roots) -> Option<PendingRestore> {
    let backups = backups_dir(roots)?;
    let pending = read_pending(&backups).ok()??;
    Some(PendingRestore {
        source: pending.source,
        created_at_ms: pending.created_at_ms,
    })
}

/// Whether a restore waits to be applied or is being applied: its
/// `pending.json` or the journal of an apply a stop interrupted. An app's
/// start that finds one and can't apply it opens no store (ruling F20 b).
/// A linked `backups` folder holds nothing an apply acts on.
pub fn restore_waits(roots: &Roots) -> bool {
    backups_dir(roots).is_some_and(|backups| {
        [PENDING, JOURNAL]
            .iter()
            .any(|name| std::fs::symlink_metadata(backups.join(name)).is_ok())
    })
}

/// Whether an apply has begun moving data and not ended: its journal is
/// there and something moved, or it can't be told. Data in the roots may
/// then be half placed, so nothing opens a store. A journal that moved
/// nothing yet leaves today's data whole (final review B minor): the next
/// apply plans again.
pub fn applying(roots: &Roots) -> bool {
    let Some(backups) = backups_dir(roots) else {
        return false;
    };
    if std::fs::symlink_metadata(backups.join(JOURNAL)).is_err() {
        return false;
    }
    match Journal::load(roots) {
        Ok(Some(journal)) => !moved_nothing(roots, &journal),
        Ok(None) => false,
        Err(_) => true,
    }
}

/// Whether `journal`'s apply moved nothing yet: no step past the seal is
/// recorded, and on disk too, since a stop may fall between a rename and
/// its record. Every entry the plan moves is still in its root, every one
/// it places still in the source, and the new set holds nothing but its
/// `meta`. Anything that can't be read counts as moved.
fn moved_nothing(roots: &Roots, journal: &Journal) -> bool {
    if journal.steps.iter().any(|step| *step != Step::Sealed) {
        return false;
    }
    journal.passes.iter().all(|pass| {
        let there = |path: PathBuf| matches!(exists(&path), Ok(true));
        let moves_live = pass
            .moves
            .iter()
            .all(|name| there(live(roots, pass.root, name)));
        let places_waiting = source_dir(roots, &journal.source, pass.root)
            .is_ok_and(|source| pass.places.iter().all(|name| there(source.join(name))));
        let kept = kept_dir(roots, pass.root, &journal.stamp);
        let kept_empty = only_names(&kept, &[META]).is_some_and(|names| {
            names.is_empty()
                || (pass.root == RootKey::Core && only_names(&kept.join(META), &[SEALED]).is_some())
        });
        moves_live && places_waiting && kept_empty
    })
}

/// The names in `folder` when each is one of `allowed` (none when it isn't
/// there), else `None`, as for anything that can't be read.
fn only_names(folder: &Path, allowed: &[&str]) -> Option<Vec<std::ffi::OsString>> {
    match std::fs::read_dir(folder) {
        Ok(entries) => entries
            .map(|entry| {
                let name = entry.ok()?.file_name();
                allowed
                    .iter()
                    .any(|allowed| name == *allowed)
                    .then_some(name)
            })
            .collect(),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Some(Vec::new()),
        Err(_) => None,
    }
}

/// Drops the journal of an apply that moved nothing: the new set's
/// folders (only its seal) and key, then the journal. `pending.json`
/// stays, so the apply plans again.
fn drop_unmoved(
    roots: &Roots,
    store: &SecretStore,
    backups: &Path,
    journal: &Journal,
) -> Result<(), BackupError> {
    tracing::info!("A restore that moved nothing yet starts over");
    for pass in &journal.passes {
        remove_tree(&kept_dir(roots, pass.root, &journal.stamp))?;
    }
    forget_key(store, backups, &kept_account(&journal.stamp));
    Journal::remove(backups)
}

/// `<core>/logs/restore.log`, opened for appending: where the restore's
/// applier and the apps' start write outcomes (never paths or values).
/// Never through a link or onto anything but a plain file, and 0600 even
/// when it was there before (B Task 10 I6).
pub fn open_restore_log(core: &Path) -> io::Result<File> {
    use rustix::fs::{Mode, OFlags};
    use std::os::unix::fs::PermissionsExt;
    let logs = core.join("logs");
    DirBuilder::new().recursive(true).mode(0o700).create(core)?;
    match DirBuilder::new().mode(0o700).create(&logs) {
        Err(error) if error.kind() != io::ErrorKind::AlreadyExists => return Err(error),
        _ => {}
    }
    // The folder itself never through a link (B Task 10 N3), and the file
    // made inside the folder that was opened.
    let folder = rustix::fs::open(
        &logs,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )?;
    let file = File::from(rustix::fs::openat(
        &folder,
        RESTORE_LOG,
        OFlags::WRONLY
            | OFlags::APPEND
            | OFlags::CREATE
            | OFlags::NOFOLLOW
            | OFlags::NONBLOCK
            | OFlags::CLOEXEC,
        Mode::from_raw_mode(0o600),
    )?);
    if !file.metadata()?.is_file() {
        return Err(io::Error::other("the restore log isn't a plain file"));
    }
    file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    Ok(file)
}

/// The last restore applied on this computer. A record that only keeps a
/// refused restore's service flag names none.
pub fn result(roots: &Roots) -> Option<RestoreResult> {
    read_result(&backups_dir(roots)?)
        .filter(|result| result.applied_at_ms != EpochMs::default())
        .map(|result| result.result())
}

/// The browser part the last restore brought, while it waits for the
/// browser in `backups/web-part/<id>.bin` (D19).
pub fn waiting_web_part(roots: &Roots) -> Option<WebPartInfo> {
    let backups = backups_dir(roots)?;
    let mut part = read_result(&backups)?.web_part?;
    if !valid_id(&part.id) {
        return None;
    }
    let path = backups.join(WEB_PARTS).join(format!("{}.bin", part.id));
    let metadata = std::fs::symlink_metadata(path).ok()?;
    if !metadata.is_file() {
        return None;
    }
    part.bytes = metadata.len();
    Some(part)
}

/// Whether the background service should start again after the last
/// apply. Clears the flag, so only one caller starts it. Takes
/// `restore.lock`, waiting up to 5 seconds for an apply that holds it:
/// call it off the async runtime.
pub fn take_service_restart(roots: &Roots) -> bool {
    let Some(backups) = backups_dir(roots) else {
        return false;
    };
    // Under the apply's lock, so the sweep never takes the record mid-write
    // (N8); an apply still holding it after a few seconds wins.
    let Some(_restore_lock) = hold_restore_lock(&backups, RESTORE_LOCK_WAIT) else {
        tracing::warn!("Bruhs couldn't take the restore lock to read the service restart");
        return false;
    };
    let Some(mut result) = read_result(&backups) else {
        return false;
    };
    if !result.service_to_restart {
        return false;
    }
    result.service_to_restart = false;
    let cleared = to_json(&result).and_then(|text| write_atomically(&backups, RESULT, &text));
    if cleared.is_err() {
        tracing::warn!("Bruhs couldn't clear the restore's service restart");
    }
    true
}

/// Whether `name` is a top-level name an apply may move: one plain
/// component, never a name a backup always leaves out or the reserved
/// `meta`, in any case or form a volume would fold together.
pub(super) fn movable(name: &str) -> bool {
    let folded = folded_entry_key(name);
    single_component(name) && folded != META && !EXCLUDED_TOP.contains(&folded.as_str())
}

/// What a locked apply does next.
enum Work {
    Resume(Journal),
    Start(PendingFile),
}

/// Plans a fresh apply of `pending` and saves the journal. A restore that
/// can never be applied is refused with `pending.json` removed (B Task 8
/// I2): a source that isn't there whole (`GONE`) or holds data its
/// manifest doesn't name (`DAMAGED`), sealed items that don't open
/// (`KEYCHAIN_SET_UNREADABLE`), or a rename that can't happen. A keychain
/// that needs the owner, or another read error, keeps it for a retry.
fn plan(
    roots: &Roots,
    store: &SecretStore,
    backups: &Path,
    pending: PendingFile,
) -> Result<Journal, BackupError> {
    let restart = pending.service_to_restart;
    let set = match check_source(roots, &pending.source) {
        Ok(Some(set)) => set,
        Ok(None) => return Err(refuse(backups, BackupError::Refused(GONE), restart)),
        Err(error @ (BackupError::Format(_) | BackupError::Refused(_))) => {
            return Err(refuse(backups, error, restart))
        }
        Err(error) => return Err(error),
    };
    // The source's items open before anything moves; only their accounts
    // are kept.
    let written: Vec<ItemRef> = if set.keychain {
        match open_items(roots, store, &pending.source) {
            Ok(items) => items
                .iter()
                .map(|item| ItemRef {
                    service: item.service.clone(),
                    account: item.account.clone(),
                })
                .collect(),
            Err(BackupError::Keychain(message)) if message == KEYCHAIN_SET_UNREADABLE => {
                return Err(refuse(backups, BackupError::Keychain(message), restart))
            }
            Err(error) => return Err(error),
        }
    } else {
        Vec::new()
    };
    // What the restored catalogs name, read before anything moves (final
    // review B-I2): catalogs that don't read never get past the plan.
    let restored = if set.keychain {
        let source = source_dir(roots, &pending.source, RootKey::Core)?;
        match inventory_at(
            &source.join(CREDENTIALS),
            &source.join(CONNECTIONS),
            &source.join(CALENDAR),
        ) {
            Ok(restored) => restored,
            Err(_) => return Err(refuse(backups, BackupError::Format(DAMAGED), restart)),
        }
    } else {
        Vec::new()
    };
    let stamp = new_stamp(roots)?;
    let mut passes = Vec::new();
    for (root, places) in set.places {
        let moves = live_names(roots, root, &places)?;
        passes.push(Pass {
            root,
            moves,
            places,
        });
    }
    if !renames_can_happen(roots, &pending.source, &passes)? {
        return Err(refuse(backups, cant_move(), restart));
    }
    let web_part = set.web_part.then(|| match &pending.source {
        RestoreSource::Staged { id } => id.clone(),
        _ => random_id(),
    });
    let journal = Journal {
        stamp,
        source: pending.source,
        made_at_ms: EpochMs::now(),
        service_to_restart: pending.service_to_restart,
        passes,
        keychain: set.keychain,
        written,
        restored,
        sealed: Vec::new(),
        items_stayed: 0,
        web_part,
        rewrites: set.rewrites,
        steps: Vec::new(),
    };
    journal.save(backups)?;
    Ok(journal)
}

/// One apply following its journal.
struct Apply<'a> {
    roots: &'a Roots,
    store: &'a SecretStore,
    backups: &'a Path,
    journal: Journal,
    stop: &'a dyn Fn(&Step, Point) -> bool,
}

impl Apply<'_> {
    fn run(mut self) -> Result<ApplyOutcome, BackupError> {
        if self.journal.done(&Step::RollingBack) {
            return self.roll_back();
        }
        if !self.journal.done(&Step::Sealed) {
            // A plan never names a kept set that is there (M3); a journal
            // that does would lose that set's sealed items.
            if kept_info(self.roots, &self.journal.stamp).is_some() {
                return Err(BackupError::Format(DAMAGED));
            }
            match self.seal() {
                Ok(sealed) => self.journal.sealed = sealed,
                Err(error) => {
                    return Err(if seal_permanent(&error) {
                        self.abandon(error)
                    } else {
                        error
                    })
                }
            }
            self.step(Step::Sealed)?;
        }
        // Before the keychain writes, a rename that fails for good undoes
        // the renames; from them on, the apply only goes forward (I1).
        if let Err((step, error)) = self.renames() {
            if !permanent(&error) || self.journal.done(&Step::KeysWritten) {
                return Err(error);
            }
            tracing::warn!("A restore couldn't move its data, so Bruhs is putting it back");
            // The failed step may have half happened: the undo judges each
            // rename by what is on disk.
            if !self.journal.done(&step) {
                self.journal.steps.push(step);
            }
            return self.roll_back();
        }
        if !self.journal.done(&Step::KeysWritten) {
            self.write_keys()?;
            self.step(Step::KeysWritten)?;
        }
        if !self.journal.done(&Step::KeysPruned) {
            self.journal.items_stayed = self.prune_keys();
            self.step(Step::KeysPruned)?;
        }
        if !self.journal.done(&Step::WebPart) {
            self.web_part()?;
            self.step(Step::WebPart)?;
        }
        if !self.journal.done(&Step::Done) {
            self.finish()?;
            self.step(Step::Done)?;
        }
        self.clean_up()?;
        let result = read_result(self.backups).map_or_else(
            || RestoreResult {
                applied_at_ms: EpochMs::now(),
                stamp: self.journal.stamp.clone(),
                warnings: Vec::new(),
            },
            |result| result.result(),
        );
        Ok(ApplyOutcome::Applied(result))
    }

    /// Each root's renames: today's entries aside, then the source's in.
    /// On a failure, answers the step it failed at.
    fn renames(&mut self) -> Result<(), (Step, BackupError)> {
        for pass in self.journal.passes.clone() {
            let moves = pass.moves.iter().map(|name| Step::Moved {
                root: pass.root,
                name: name.clone(),
            });
            let places = pass.places.iter().map(|name| Step::Placed {
                root: pass.root,
                name: name.clone(),
            });
            for step in moves.chain(places) {
                if self.journal.done(&step) {
                    continue;
                }
                let done = if (self.stop)(&step, Point::Before) {
                    Err(io::Error::from_raw_os_error(libc::EACCES).into())
                } else {
                    match &step {
                        Step::Moved { root, name } => self.move_aside(*root, name),
                        Step::Placed { root, name } => self.place(*root, name),
                        _ => Ok(()),
                    }
                };
                if let Err(error) = done.and_then(|()| self.step(step.clone())) {
                    return Err((step, error));
                }
            }
        }
        Ok(())
    }

    /// Records `step`, stopping where a test asks.
    fn step(&mut self, step: Step) -> Result<(), BackupError> {
        if (self.stop)(&step, Point::Acted) {
            return Err(stopped());
        }
        self.journal.record(self.backups, step.clone())?;
        if (self.stop)(&step, Point::Recorded) {
            return Err(stopped());
        }
        Ok(())
    }

    fn kept(&self, root: RootKey) -> PathBuf {
        kept_dir(self.roots, root, &self.journal.stamp)
    }

    /// Makes the pre-restore folders (0700) and seals today's items into
    /// `meta/keychain.sealed` under a fresh key at `pre-restore:<stamp>`
    /// (D13). A seal a stop left behind is made again. Answers the items
    /// sealed, without values.
    fn seal(&self) -> Result<Vec<ItemRef>, BackupError> {
        for pass in &self.journal.passes {
            new_dir(&self.roots.backups(pass.root))?;
            new_dir(&self.kept(pass.root))?;
        }
        let kept = self.kept(RootKey::Core);
        let meta = kept.join(META);
        new_dir(&meta)?;
        let sealed = meta.join(SEALED);
        remove_if_there(&sealed)?;
        let items = current_items(self.roots, self.store)?;
        // Fresh OS-random bytes only.
        let mut key = Zeroizing::new([0_u8; 32]);
        random_bytes(&mut *key);
        let mut random = [0_u8; 23];
        random_bytes(&mut random);
        let blob = seal_items(&items, random, &key)
            .map_err(|_| BackupError::Keychain(KEYCHAIN_ITEM_REFUSED))?;
        // A set whose seal a put back couldn't read is never made (final
        // review B minor): refused before anything moves.
        if u64::try_from(blob.len()).map_or(true, |bytes| bytes > SEALED_MAX) {
            return Err(BackupError::Keychain(KEEP_ITEMS_TOO_LARGE));
        }
        let refs = items
            .iter()
            .map(|item| ItemRef {
                service: item.service.clone(),
                account: item.account.clone(),
            })
            .collect();
        drop(items);
        let mut file = new_private_file(&sealed)?;
        file.write_all(&blob)?;
        file.sync_all()?;
        sync_dir(&meta)?;
        sync_dir(&kept)?;
        store_seal_key(self.store, &kept_account(&self.journal.stamp), &key)
            .map_err(|_| BackupError::Keychain(KEYCHAIN_NEEDS_YOU))?;
        Ok(refs)
    }

    /// Renames today's `name` into the pre-restore set. Done already when
    /// it is gone from the root.
    fn move_aside(&self, root: RootKey, name: &str) -> Result<(), BackupError> {
        let from = live(self.roots, root, name);
        let to = self.kept(root).join(name);
        match (exists(&from)?, exists(&to)?) {
            (true, false) => rename_retrying(&from, &to),
            (false, _) => Ok(()),
            (true, true) => Err(in_the_way()),
        }
    }

    /// Renames the source's `name` into the root. Done already when it is
    /// gone from the source. Something that took the name since today's
    /// entry moved aside is in the way, for good.
    fn place(&self, root: RootKey, name: &str) -> Result<(), BackupError> {
        let from = source_dir(self.roots, &self.journal.source, root)?.join(name);
        let to = live(self.roots, root, name);
        match (exists(&from)?, exists(&to)?) {
            (true, false) => {
                if let Some(parent) = to.parent() {
                    make_root(parent)?;
                }
                rename_retrying(&from, &to)
            }
            (false, _) => Ok(()),
            (true, true) => Err(in_the_way()),
        }
    }

    /// Writes the source's items under their accounts. Writing one again
    /// after a stop changes nothing.
    fn write_keys(&self) -> Result<(), BackupError> {
        if !self.journal.keychain {
            return Ok(());
        }
        let items = open_items(self.roots, self.store, &self.journal.source)?;
        for item in &items {
            self.store
                .write_item(&item.service, &item.account, &item.value)
                .map_err(|_| BackupError::Keychain(KEYCHAIN_NEEDS_YOU))?;
        }
        Ok(())
    }

    /// Removes today's sealed items that the restored catalogs don't name
    /// and the source didn't write; the sealed set keeps them (D16). A
    /// source without items keeps today's (D5). Never a seal key (ruling
    /// F11). Best effort (final review B-I2): an item that won't go stays,
    /// harmless since the seal holds it, and is counted for the result.
    /// Reads no catalog: both lists are in the journal.
    fn prune_keys(&self) -> u32 {
        if !self.journal.keychain {
            return 0;
        }
        let keep: BTreeSet<&ItemRef> = self
            .journal
            .restored
            .iter()
            .chain(&self.journal.written)
            .collect();
        let mut stayed = 0_u32;
        for item in &self.journal.sealed {
            if item.service == BACKUP_SERVICE || keep.contains(item) {
                continue;
            }
            if self
                .store
                .remove_item(&item.service, &item.account)
                .is_err()
            {
                stayed = stayed.saturating_add(1);
            }
        }
        if stayed > 0 {
            tracing::warn!("{stayed} keychain items from before the restore stayed");
        }
        stayed
    }

    /// Moves the source's browser part to `backups/web-part/<id>.bin`.
    fn web_part(&self) -> Result<(), BackupError> {
        let Some(id) = &self.journal.web_part else {
            return Ok(());
        };
        let from = source_dir(self.roots, &self.journal.source, RootKey::Core)?
            .join(META)
            .join(WEB_PART);
        let folder = self.backups.join(WEB_PARTS);
        new_dir(&folder)?;
        if exists(&from)? {
            rename_synced(&from, &folder.join(format!("{id}.bin")))?;
        }
        Ok(())
    }

    /// Writes the kept set's `meta/info.json` from what it holds, then
    /// `result.json`, then makes it all durable with one full sync (M5).
    /// Both files are replaced whole, so doing it again is safe.
    fn finish(&self) -> Result<(), BackupError> {
        // Past the keychain writes nothing rolls back, so a carry that
        // fails is skipped, not repeated: the entries stay in the set put
        // back, which `clean_up` keeps (R2-M3).
        let mut warnings = Vec::new();
        if self.carry_parked().is_err() {
            tracing::warn!(
                "Bruhs couldn't carry kept files into the new set; they stay where they were"
            );
            warnings.push(PARKED_STAYED.to_owned());
        }
        warnings.extend(items_stayed_line(self.journal.items_stayed));
        self.write_info(true)?;
        let web_part = self.journal.web_part.as_ref().and_then(|id| {
            let path = self.backups.join(WEB_PARTS).join(format!("{id}.bin"));
            let metadata = std::fs::symlink_metadata(path).ok()?;
            metadata.is_file().then(|| WebPartInfo {
                id: id.clone(),
                bytes: metadata.len(),
                rewrites: self.journal.rewrites.clone(),
            })
        });
        let result = ResultFile {
            applied_at_ms: EpochMs::now(),
            stamp: self.journal.stamp.clone(),
            warnings,
            service_to_restart: self.journal.service_to_restart,
            web_part_stamp: web_part.as_ref().map(|_| self.journal.stamp.clone()),
            web_part,
        };
        write_atomically(self.backups, RESULT, &to_json(&result)?)?;
        full_sync(self.backups)
    }

    /// Removes the source and its seal key, then `pending.json`, then the
    /// journal.
    fn clean_up(&self) -> Result<(), BackupError> {
        let account = match &self.journal.source {
            RestoreSource::Staged { id } => format!("staged:{id}"),
            RestoreSource::PreRestore { stamp } => kept_account(stamp),
            RestoreSource::Other => return Err(BackupError::Refused(INVALID_REQUEST)),
        };
        if !self.keep_parked_source()? {
            for pass in &self.journal.passes {
                remove_tree(&source_dir(self.roots, &self.journal.source, pass.root)?)?;
            }
            forget_key(self.store, self.backups, &account);
        }
        remove_if_there(&self.backups.join(PENDING))?;
        sync_dir(self.backups)?;
        Journal::remove(self.backups)
    }

    /// Drops an apply whose seal failed for good (N4). Nothing has moved:
    /// the kept folders, the seal key, `pending.json` and the journal go,
    /// and the answer is the seal's own error.
    fn abandon(&self, error: BackupError) -> BackupError {
        tracing::warn!("A restore couldn't keep today's keychain items aside, so it was dropped");
        let dropped = (|| {
            for pass in &self.journal.passes {
                remove_tree(&self.kept(pass.root))?;
            }
            forget_key(self.store, self.backups, &kept_account(&self.journal.stamp));
            remove_if_there(&self.backups.join(PENDING))?;
            sync_dir(self.backups)?;
            Journal::remove(self.backups)
        })();
        match dropped {
            Ok(()) => error,
            Err(failed) => failed,
        }
    }
}

/// Whether a seal failed for good: a refused item, catalogs that don't
/// read, or a folder or file it can't make. A keychain that needs the
/// owner may answer later.
fn seal_permanent(error: &BackupError) -> bool {
    match error {
        BackupError::Keychain(message) => {
            *message == KEYCHAIN_ITEM_REFUSED
                || *message == KEYCHAIN_READ_FAILED
                || *message == KEEP_ITEMS_TOO_LARGE
        }
        other => permanent(other),
    }
}

/// Checks that `source` is there whole (D16, the Task 8 rulings): a staged
/// restore whose staging finished (its preview names it, its manifest
/// reads, every top-level entry its files name is staged, and nothing
/// staged is a link or an odd name), or a kept set whose info names its
/// stamp and whose entries are all there. `None` when it isn't.
fn check_source(roots: &Roots, source: &RestoreSource) -> Result<Option<SourceSet>, BackupError> {
    match source {
        RestoreSource::Staged { id } if valid_id(id) => check_staged(roots, id),
        RestoreSource::PreRestore { stamp } if valid_stamp(stamp) => check_kept(roots, stamp),
        _ => Ok(None),
    }
}

/// The root Task 7 staged a backup's `origin` entry `top` in (D11).
fn staged_root(roots: &Roots, manifest: &BackupManifest, origin: RootKey, top: &str) -> RootKey {
    if roots.same() {
        RootKey::Core
    } else if manifest.same_root && desktop_entry(top) {
        RootKey::Desktop
    } else {
        origin
    }
}

/// The paths below `root` that a staged restore's manifest names as
/// files, as they sit in the root once placed. `None` when the staging's
/// manifest is gone or unreadable.
fn manifest_files(roots: &Roots, id: &str, root: RootKey) -> Option<BTreeSet<String>> {
    let path = staging_dir(roots, RootKey::Core, id)
        .join(META)
        .join(MANIFEST);
    let manifest = read_json::<BackupManifest>(&path, MANIFEST_MAX_BYTES)?;
    Some(
        manifest
            .files
            .iter()
            .filter_map(|file| {
                let (origin, rel) = if let Some(rel) = file.path.strip_prefix("core/") {
                    (RootKey::Core, rel)
                } else {
                    (RootKey::Desktop, file.path.strip_prefix("desktop/")?)
                };
                let top = rel.split('/').next().unwrap_or_default();
                (staged_root(roots, &manifest, origin, top) == root).then(|| rel.to_owned())
            })
            .collect(),
    )
}

fn check_staged(roots: &Roots, id: &str) -> Result<Option<SourceSet>, BackupError> {
    let meta = staging_dir(roots, RootKey::Core, id).join(META);
    if !real_dir(&meta) {
        return Ok(None);
    }
    let Some(preview) = read_json::<RestorePreview>(&meta.join(PREVIEW), SMALL_MAX)
        .filter(|preview| preview.id == id)
    else {
        return Ok(None);
    };
    let Some(manifest) = read_json::<BackupManifest>(&meta.join(MANIFEST), MANIFEST_MAX_BYTES)
    else {
        return Ok(None);
    };
    let mut expected: Vec<(RootKey, BTreeSet<&str>)> = pass_roots(roots)
        .iter()
        .map(|root| (*root, BTreeSet::new()))
        .collect();
    for file in &manifest.files {
        let (origin, rel) = if let Some(rel) = file.path.strip_prefix("core/") {
            (RootKey::Core, rel)
        } else if let Some(rel) = file.path.strip_prefix("desktop/") {
            (RootKey::Desktop, rel)
        } else {
            continue;
        };
        let top = rel.split('/').next().unwrap_or_default();
        let root = staged_root(roots, &manifest, origin, top);
        if let Some((_, names)) = expected.iter_mut().find(|(key, _)| *key == root) {
            names.insert(top);
        }
    }
    let mut places = Vec::new();
    for (root, names) in &expected {
        let folder = staging_dir(roots, *root, id);
        let Some(staged) = staged_names(&folder)? else {
            return Ok(None);
        };
        if !names
            .iter()
            .all(|name| staged.iter().any(|staged| staged == name))
        {
            return Ok(None);
        }
        // Data the manifest doesn't name is never placed (M4): a file or a
        // folder holding one is damage; a folder holding none comes from
        // an empty folder in the backup and stays behind.
        let mut named = Vec::new();
        for name in staged {
            if names.contains(name.as_str()) {
                named.push(name);
            } else if holds_files(&folder.join(&name))? {
                return Err(BackupError::Format(DAMAGED));
            }
        }
        places.push((*root, named));
    }
    let keychain = manifest.options.keychain;
    let web_part = manifest.web_db_version.is_some();
    if keychain != is_file(&meta.join(SEALED)) || web_part != is_file(&meta.join(WEB_PART)) {
        return Ok(None);
    }
    Ok(Some(SourceSet {
        places,
        keychain,
        web_part,
        rewrites: preview.rewrites,
        protocol: Some(manifest.reactive_protocol),
    }))
}

/// A staged root's top-level names, `meta` aside. `None` when the folder
/// isn't a real one, or holds a link, an odd entry or a name no backup
/// stages.
fn staged_names(folder: &Path) -> Result<Option<Vec<String>>, BackupError> {
    if !real_dir(folder) {
        return Ok(None);
    }
    let mut names = Vec::new();
    for entry in std::fs::read_dir(folder)? {
        let entry = entry?;
        let Ok(name) = entry.file_name().into_string() else {
            return Ok(None);
        };
        if folded_entry_key(&name) == META {
            continue;
        }
        let kind = entry.file_type()?;
        if !movable(&name) || !(kind.is_dir() || kind.is_file()) {
            return Ok(None);
        }
        names.push(name);
    }
    names.sort();
    Ok(Some(names))
}

/// Today's top-level names in `root` that the apply moves aside: every
/// entry D4's rule would take with every option on, never one a backup
/// always leaves out or `meta`, plus any name the source brings that is
/// there today. On a computer with its own config root, `calendar` is the
/// config root's, and the core root's own is left alone (ruling F10 a).
/// Names that fold together on the volume move once.
fn live_names(roots: &Roots, root: RootKey, places: &[String]) -> Result<Vec<String>, BackupError> {
    let separate = root == RootKey::Core && roots.config != roots.core;
    let mut names: BTreeMap<String, String> = BTreeMap::new();
    match std::fs::read_dir(roots.root(root)) {
        Ok(entries) => {
            for entry in entries {
                let entry = entry?;
                let Ok(name) = entry.file_name().into_string() else {
                    continue;
                };
                if !movable(&name) || (separate && name == CALENDAR) {
                    continue;
                }
                if included(&name, entry.file_type()?.is_dir(), &EVERYTHING) {
                    names.entry(folded_entry_key(&name)).or_insert(name);
                }
            }
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    if separate && exists(&roots.config.join(CALENDAR))? {
        names
            .entry(folded_entry_key(CALENDAR))
            .or_insert_with(|| CALENDAR.to_owned());
    }
    for name in places {
        if exists(&live(roots, root, name))? {
            names
                .entry(folded_entry_key(name))
                .or_insert_with(|| name.clone());
        }
    }
    Ok(names.into_values().collect())
}

/// Whether every rename the plan makes can happen (B Task 8 I1 f, M2), so
/// a rollback stays for the rare case: each entry is on the volume it goes
/// to, by its own device (a mount point is another), this user may write
/// the folders it leaves and enters, and a folder itself, since moving it
/// to another parent rewrites its `..`.
fn renames_can_happen(
    roots: &Roots,
    source: &RestoreSource,
    passes: &[Pass],
) -> Result<bool, BackupError> {
    for pass in passes {
        let backups = roots.backups(pass.root);
        new_dir(&backups)?;
        let kept = device(&backups)?;
        let from = source_dir(roots, source, pass.root)?;
        if !writable(&backups) || !writable(&from) {
            return Ok(false);
        }
        for name in &pass.moves {
            let parent = live_parent(roots, pass.root, name);
            if !entry_can_move(&parent.join(name), &parent, kept)? {
                return Ok(false);
            }
        }
        for name in &pass.places {
            let parent = live_parent(roots, pass.root, name);
            if exists(&parent)? && !writable(&parent) {
                return Ok(false);
            }
            if !entry_can_move(&from.join(name), &from, nearest_device(&parent)?)? {
                return Ok(false);
            }
        }
    }
    Ok(true)
}

/// Whether `entry` (in `parent`) can be renamed to a folder on `device`.
/// One that isn't there has nothing to move.
fn entry_can_move(entry: &Path, parent: &Path, device: u64) -> Result<bool, BackupError> {
    let metadata = match std::fs::symlink_metadata(entry) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(true),
        Err(error) => return Err(error.into()),
    };
    Ok(metadata.dev() == device && writable(parent) && (!metadata.is_dir() || writable(entry)))
}

/// Whether this user may write `path`, judged by the system, not by the
/// mode bits alone.
fn writable(path: &Path) -> bool {
    rustix::fs::access(path, rustix::fs::Access::WRITE_OK).is_ok()
}

/// Whether a file is at or under `path`, never following a link.
fn holds_files(path: &Path) -> Result<bool, BackupError> {
    let metadata = std::fs::symlink_metadata(path)?;
    if !metadata.is_dir() {
        return Ok(true);
    }
    let mut folders = vec![path.to_path_buf()];
    while let Some(folder) = folders.pop() {
        for entry in std::fs::read_dir(&folder)? {
            let entry = entry?;
            if entry.file_type()?.is_dir() {
                folders.push(entry.path());
            } else {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

fn device(path: &Path) -> Result<u64, BackupError> {
    Ok(std::fs::metadata(path)?.dev())
}

/// The device of `path`, or of its nearest folder that exists.
fn nearest_device(path: &Path) -> Result<u64, BackupError> {
    let mut at = Some(path);
    while let Some(path) = at {
        match std::fs::metadata(path) {
            Ok(metadata) => return Ok(metadata.dev()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => at = path.parent(),
            Err(error) => return Err(error.into()),
        }
    }
    Err(io::Error::from(io::ErrorKind::NotFound).into())
}

/// The source's folder in `root`.
fn source_dir(
    roots: &Roots,
    source: &RestoreSource,
    root: RootKey,
) -> Result<PathBuf, BackupError> {
    match source {
        RestoreSource::Staged { id } => Ok(staging_dir(roots, root, id)),
        RestoreSource::PreRestore { stamp } => Ok(kept_dir(roots, root, stamp)),
        RestoreSource::Other => Err(BackupError::Refused(INVALID_REQUEST)),
    }
}

/// Where today's top-level `name` of `root` lives: the root, except the
/// core root's `calendar` on a computer with its own config root.
fn live(roots: &Roots, root: RootKey, name: &str) -> PathBuf {
    live_parent(roots, root, name).join(name)
}

fn live_parent(roots: &Roots, root: RootKey, name: &str) -> PathBuf {
    if root == RootKey::Core && name == CALENDAR && roots.config != roots.core {
        roots.config.clone()
    } else {
        roots.root(root).to_path_buf()
    }
}

/// The source's items, opened from its sealed set with the key it keeps.
fn open_items(
    roots: &Roots,
    store: &SecretStore,
    source: &RestoreSource,
) -> Result<Vec<KeychainItem>, BackupError> {
    let account = match source {
        RestoreSource::Staged { id } => format!("staged:{id}"),
        RestoreSource::PreRestore { stamp } => kept_account(stamp),
        RestoreSource::Other => return Err(BackupError::Refused(INVALID_REQUEST)),
    };
    let unreadable = || BackupError::Keychain(KEYCHAIN_SET_UNREADABLE);
    let sealed = source_dir(roots, source, RootKey::Core)?
        .join(META)
        .join(SEALED);
    let blob = read_small(&sealed, SEALED_MAX).ok_or_else(unreadable)?;
    let key = read_seal_key(store, &account).map_err(|error| match error {
        SecretError::NotSaved => unreadable(),
        _ => BackupError::Keychain(KEYCHAIN_NEEDS_YOU),
    })?;
    unseal_items(&blob, &key).map_err(|_| unreadable())
}

/// Today's items, as today's catalogs name them (D5). A missing item is
/// skipped. Blocks on the keychain.
fn current_items(roots: &Roots, store: &SecretStore) -> Result<Vec<KeychainItem>, BackupError> {
    let refs = inventory_at(
        &roots.core.join(CREDENTIALS),
        &roots.core.join(CONNECTIONS),
        &roots.config.join(CALENDAR),
    )?;
    let mut items = Vec::with_capacity(refs.len());
    for item in refs {
        match store.read_item(&item.service, &item.account) {
            Ok(Some(value)) => items.push(KeychainItem {
                service: item.service,
                account: item.account,
                value,
            }),
            Ok(None) => {}
            Err(_) => return Err(BackupError::Keychain(KEYCHAIN_NEEDS_YOU)),
        }
    }
    Ok(items)
}

/// The items the catalogs in `credentials` and `connections`, and the
/// calendar state under `calendar`, name.
fn inventory_at(
    credentials: &Path,
    connections: &Path,
    calendar: &Path,
) -> Result<Vec<ItemRef>, BackupError> {
    keychain_inventory(credentials, connections, &calendar.join(GOOGLE))
        .map_err(|_| BackupError::Keychain(KEYCHAIN_READ_FAILED))
}

/// A fresh stamp (D18): this minute in UTC and 4 random hex, free in both
/// roots.
fn new_stamp(roots: &Roots) -> Result<String, BackupError> {
    let name = backup_file_name(EpochMs::now());
    let minute = name
        .strip_prefix("bruhs-backup-")
        .and_then(|rest| rest.strip_suffix(".bruhsbak"))
        .ok_or_else(|| io::Error::other("the backup file name changed shape"))?;
    for _ in 0..STAMP_TRIES {
        let mut suffix = [0_u8; 2];
        random_bytes(&mut suffix);
        let stamp = format!("{minute}-{:02x}{:02x}", suffix[0], suffix[1]);
        let free = [RootKey::Core, RootKey::Desktop]
            .iter()
            .all(|root| matches!(exists(&kept_dir(roots, *root, &stamp)), Ok(false)));
        if valid_stamp(&stamp) && free {
            return Ok(stamp);
        }
    }
    Err(io::Error::other("no free pre-restore stamp").into())
}

/// The roots one apply passes over: the core root, and the desktop root
/// unless both are one folder (ruling F33).
pub(super) fn pass_roots(roots: &Roots) -> &'static [RootKey] {
    if roots.same() {
        &[RootKey::Core]
    } else {
        &[RootKey::Core, RootKey::Desktop]
    }
}

/// `<core>/backups` when both roots' `backups` folders are real folders
/// (or missing) and the core one is there: nothing is read through a link
/// (the B Task 6 ruling).
fn backups_dir(roots: &Roots) -> Option<PathBuf> {
    backups_are_folders(roots).ok()?;
    let backups = roots.backups(RootKey::Core);
    real_dir(&backups).then_some(backups)
}

fn read_pending(backups: &Path) -> Result<Option<PendingFile>, BackupError> {
    let path = backups.join(PENDING);
    if !exists(&path)? {
        return Ok(None);
    }
    match read_json::<PendingFile>(&path, SMALL_MAX) {
        Some(PendingFile {
            source: RestoreSource::Other,
            ..
        }) => Err(BackupError::Refused(INVALID_REQUEST)),
        Some(pending) => Ok(Some(pending)),
        None => Err(BackupError::Format(DAMAGED)),
    }
}

fn read_result(backups: &Path) -> Option<ResultFile> {
    read_json(&backups.join(RESULT), SMALL_MAX)
}

/// A small JSON file, read without following a link or waiting on a FIFO.
fn read_json<T: DeserializeOwned>(path: &Path, max: u64) -> Option<T> {
    serde_json::from_slice(&read_small(path, max)?).ok()
}

fn to_json<T: Serialize>(value: &T) -> Result<Vec<u8>, BackupError> {
    Ok(serde_json::to_vec(value).map_err(io::Error::other)?)
}

/// A folder's top-level names, `meta` aside, sorted.
fn top_names(folder: &Path) -> Result<Vec<String>, BackupError> {
    let mut names = Vec::new();
    for entry in std::fs::read_dir(folder)? {
        if let Ok(name) = entry?.file_name().into_string() {
            if folded_entry_key(&name) != META {
                names.push(name);
            }
        }
    }
    names.sort();
    Ok(names)
}

/// The bytes of the files at and under `path`, never following a link.
/// Best effort (final review B-I1): what can't be read counts nothing, so
/// a size never fails an apply or a rollback.
fn tree_bytes(path: &Path) -> u64 {
    let mut bytes = 0_u64;
    let mut folders = Vec::new();
    let Ok(metadata) = std::fs::symlink_metadata(path) else {
        return 0;
    };
    if metadata.is_dir() {
        folders.push(path.to_path_buf());
    } else if metadata.is_file() {
        bytes = metadata.len();
    }
    while let Some(folder) = folders.pop() {
        let Ok(entries) = std::fs::read_dir(&folder) else {
            continue;
        };
        for entry in entries.flatten() {
            let Ok(metadata) = entry.metadata() else {
                continue;
            };
            if metadata.is_dir() {
                folders.push(entry.path());
            } else if metadata.is_file() {
                bytes = bytes.saturating_add(metadata.len());
            }
        }
    }
    bytes
}

/// Renames `from` to `to`, then syncs the folder it went into and the one
/// it left.
fn rename_synced(from: &Path, to: &Path) -> Result<(), BackupError> {
    std::fs::rename(from, to)?;
    for folder in [to.parent(), from.parent()].into_iter().flatten() {
        sync_dir(folder)?;
    }
    Ok(())
}

/// [`rename_synced`], tried once more when a folder target briefly isn't
/// empty (`ENOTEMPTY`).
fn rename_retrying(from: &Path, to: &Path) -> Result<(), BackupError> {
    match rename_synced(from, to) {
        Err(BackupError::Io(error)) if error.raw_os_error() == Some(libc::ENOTEMPTY) => {
            rename_synced(from, to)
        }
        done => done,
    }
}

/// Makes a missing root folder (and its parents) 0700, syncing the folder
/// above each one made (M5).
fn make_root(folder: &Path) -> Result<(), BackupError> {
    let mut missing = Vec::new();
    let mut at = Some(folder);
    while let Some(path) = at {
        if exists(path)? {
            break;
        }
        missing.push(path);
        at = path.parent();
    }
    if missing.is_empty() {
        return Ok(());
    }
    DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(folder)?;
    for made in missing.iter().rev() {
        if let Some(parent) = made.parent() {
            sync_dir(parent)?;
        }
    }
    Ok(())
}

/// [`private_dir`] (0700, never a link), syncing the folder above when it
/// made one (M5).
fn new_dir(folder: &Path) -> Result<(), BackupError> {
    let made = !exists(folder)?;
    private_dir(folder)?;
    if let (true, Some(parent)) = (made, folder.parent()) {
        sync_dir(parent)?;
    }
    Ok(())
}

/// Removes `pending.json` and answers `error` (B Task 8 I2): a restore
/// that can never be applied doesn't block backups and restores. A
/// service the restore stopped (`service_to_restart`) is first recorded in
/// `result.json`, where [`take_service_restart`] finds it (B Task 10 I5).
/// The caller holds `restore.lock`.
fn refuse(backups: &Path, error: BackupError, service_to_restart: bool) -> BackupError {
    let kept = if service_to_restart {
        keep_service_restart(backups)
    } else {
        Ok(())
    };
    match kept
        .and_then(|()| remove_if_there(&backups.join(PENDING)))
        .and_then(|()| sync_dir(backups))
    {
        Ok(()) => error,
        Err(failed) => failed,
    }
}

/// Sets `result.json`'s service flag, keeping the rest of the record (the
/// last restore, a browser part that waits). With no record yet, the new
/// one holds only the flag, which [`result`] doesn't report.
fn keep_service_restart(backups: &Path) -> Result<(), BackupError> {
    let mut result = read_result(backups).unwrap_or_default();
    if result.service_to_restart {
        return Ok(());
    }
    result.service_to_restart = true;
    write_atomically(backups, RESULT, &to_json(&result)?)
}

/// Whether a rename failed for good (B Task 8 I1): a name something else
/// took, no permission, another volume, a busy mount point or a folder
/// that isn't empty or isn't one. Anything else may pass on a retry.
fn permanent(error: &BackupError) -> bool {
    let BackupError::Io(error) = error else {
        return false;
    };
    error.kind() == io::ErrorKind::AlreadyExists
        || error.raw_os_error().is_some_and(|code| {
            [
                libc::EACCES,
                libc::EPERM,
                libc::EXDEV,
                libc::EBUSY,
                libc::ENOTEMPTY,
                libc::EEXIST,
                libc::EISDIR,
                libc::ENOTDIR,
            ]
            .contains(&code)
        })
}

/// Removes a real folder and everything in it, never through a link, then
/// syncs its parent. Answers whether it was there.
fn remove_tree(folder: &Path) -> Result<bool, BackupError> {
    if !real_dir(folder) {
        return Ok(false);
    }
    std::fs::remove_dir_all(folder)?;
    if let Some(parent) = folder.parent() {
        sync_dir(parent)?;
    }
    Ok(true)
}

fn remove_if_there(path: &Path) -> Result<(), BackupError> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

/// Whether anything is at `path`, a link included.
fn exists(path: &Path) -> Result<bool, BackupError> {
    match std::fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.into()),
    }
}

fn real_dir(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|metadata| metadata.is_dir())
}

fn is_file(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|metadata| metadata.is_file())
}

enum Lock {
    /// Taken (the file to keep while it lasts), or no lock file there.
    Free(Option<File>),
    Held,
}

/// Opens or makes a lock file (0600, never through a link).
fn open_lock_file(path: &Path) -> io::Result<File> {
    OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
}

/// Takes `file`'s lock without waiting on a holder, trying again for a
/// moment first. A process another thread forks holds a copy of every
/// open file, and with it the lock, until it execs, so a lock released a
/// moment ago can read as held; a real holder still holds it after
/// [`LOCK_GRACE`].
fn lock(file: File) -> io::Result<Lock> {
    let until = std::time::Instant::now() + LOCK_GRACE;
    loop {
        match file.try_lock() {
            Ok(()) => return Ok(Lock::Free(Some(file))),
            Err(TryLockError::WouldBlock) if std::time::Instant::now() < until => {
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            Err(TryLockError::WouldBlock) => return Ok(Lock::Held),
            Err(TryLockError::Error(error)) => return Err(error),
        }
    }
}

/// Takes the lock of an existing lock file; a missing one is free.
fn lock_existing(path: &Path) -> io::Result<Lock> {
    match File::open(path) {
        Ok(file) => lock(file),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Lock::Free(None)),
        Err(error) => Err(error),
    }
}

/// Whether `file` is the file at `path`.
fn same_file(file: Option<&File>, path: &Path) -> bool {
    let Some(file) = file else {
        return false;
    };
    match (file.metadata(), std::fs::symlink_metadata(path)) {
        (Ok(held), Ok(there)) => held.dev() == there.dev() && held.ino() == there.ino(),
        _ => false,
    }
}

fn stopped() -> BackupError {
    BackupError::Io(io::Error::other("the apply stopped at a crash point"))
}

/// Something took a name the apply needs: permanent.
fn in_the_way() -> BackupError {
    BackupError::Io(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "an entry is in the way of the restore",
    ))
}

/// A rename the plan makes can't happen: another volume, or no permission
/// (N5).
fn cant_move() -> BackupError {
    BackupError::Refused(RESTORE_CANT_MOVE)
}

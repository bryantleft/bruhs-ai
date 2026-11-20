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
    backup_file_name, desktop_entry, folded_entry_key, included, valid_id, valid_stamp,
    BackupManifest, BackupOptions, PathRewrite, PendingRestore, PreRestoreInfo, RestorePreview,
    RestoreResult, RestoreSource, WebPartInfo, BACKUP_BUSY, DAMAGED, EXCLUDED_TOP, GONE,
    INVALID_REQUEST, KEYCHAIN_ITEM_REFUSED, KEYCHAIN_NEEDS_YOU, KEYCHAIN_READ_FAILED,
    KEYCHAIN_SET_UNREADABLE, MANIFEST_MAX_BYTES, RESERVED_TOP, RESTORE_PENDING,
};
use bruhs_models::EpochMs;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use zeroize::Zeroizing;

use super::create::{backups_are_folders, private_dir};
use super::host::{new_private_file, random_bytes, random_id, read_small, PENDING};
use super::journal::{single_component, sync_dir, write_atomically, Journal, Pass, Step, JOURNAL};
use super::open::staging_dir;
use super::{BackupError, RootKey, Roots};
use crate::secrets::{
    forget_seal_key, keychain_inventory, read_seal_key, seal_items, store_seal_key, unseal_items,
    ItemRef, KeychainItem, SecretError, SecretStore, BACKUP_SERVICE,
};

/// The apply's own lock, inside `<core>/backups`, which no apply moves.
const RESTORE_LOCK: &str = "restore.lock";
/// The desktop app's instance lock, at the top of the desktop root (D17).
const INSTANCE_LOCK: &str = "desktop.instance.lock";
/// Any running server holds this lock beside its reactive database.
const REACTIVE_LOCK: &str = "reactive.sqlite3.lock";
const REACTIVE_DB: &str = "reactive.sqlite3";
const REACTIVE_DB_ENV: &str = "BRUHS_REACTIVE_DB";
const SERVER: &str = "server";
const RESULT: &str = "result.json";
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
/// The minute of a stamp gets a random suffix; this many tries find a
/// free one.
const STAMP_TRIES: usize = 64;

/// Who holds a lock the apply needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Holder {
    /// A running server (the reactive database's lock, or a reactive
    /// database moved elsewhere by `BRUHS_REACTIVE_DB`).
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

/// Where a test stops an apply, as a kill would: after a step's work but
/// before the journal records it, or right after it is recorded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Crash {
    Acted,
    Recorded,
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

/// A kept set's `meta/info.json`: its manifest. A put back acts only on a
/// set whose info names its stamp and whose entries are all there.
#[derive(Serialize, Deserialize)]
struct KeptInfo {
    stamp: String,
    made_at_ms: EpochMs,
    bytes: u64,
    /// Whether `meta/keychain.sealed` holds the day's items.
    keychain: bool,
    roots: Vec<KeptRoot>,
}

#[derive(Serialize, Deserialize)]
struct KeptRoot {
    root: RootKey,
    names: Vec<String>,
}

/// What a source brings, once checked.
struct SourceSet {
    places: Vec<(RootKey, Vec<String>)>,
    keychain: bool,
    web_part: bool,
    rewrites: Vec<PathRewrite>,
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
/// (`RESTORE_PENDING`) and for a source that isn't a finished staged
/// restore or a whole kept set (`GONE`).
pub fn schedule(
    roots: &Roots,
    source: &RestoreSource,
    service_to_restart: bool,
) -> Result<PendingRestore, BackupError> {
    backups_are_folders(roots)?;
    let backups = roots.backups(RootKey::Core);
    private_dir(&backups)?;
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
/// nothing; so does a `BRUHS_REACTIVE_DB` that moves the reactive database
/// out of the core root, whose lock this can't check (ruling F10 c). A
/// `backups` folder that is a link is never acted on.
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
/// `stop` after each step whether a test stops the apply there.
pub(super) fn apply_pending_with(
    roots: &Roots,
    store: &SecretStore,
    desktop_lock: Option<&File>,
    env: &dyn Fn(&str) -> Option<OsString>,
    stop: &dyn Fn(&Step, Crash) -> bool,
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
    let work = match Journal::load(&backups)? {
        Some(journal) => Work::Resume(journal),
        None => match read_pending(&backups)? {
            Some(pending) => Work::Start(pending),
            None => return Ok(ApplyOutcome::NothingPending),
        },
    };
    let moved_db = env(REACTIVE_DB_ENV)
        .filter(|path| !path.is_empty())
        .is_some_and(|path| Path::new(&path) != roots.core.join(SERVER).join(REACTIVE_DB));
    if moved_db {
        return Ok(ApplyOutcome::Busy(Holder::Server));
    }
    let Lock::Free(_server_lock) = lock_existing(&roots.core.join(SERVER).join(REACTIVE_LOCK))?
    else {
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

/// The restore waiting to be applied.
pub fn pending(roots: &Roots) -> Option<PendingRestore> {
    let backups = backups_dir(roots)?;
    let pending = read_pending(&backups).ok()??;
    Some(PendingRestore {
        source: pending.source,
        created_at_ms: pending.created_at_ms,
    })
}

/// The last restore applied on this computer.
pub fn result(roots: &Roots) -> Option<RestoreResult> {
    read_result(&backups_dir(roots)?).map(|result| result.result())
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
/// apply. Clears the flag, so only one caller starts it.
pub fn take_service_restart(roots: &Roots) -> bool {
    let Some(backups) = backups_dir(roots) else {
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

/// The sets kept from earlier restores, newest first (D18). A set a put
/// back is consuming isn't listed.
pub fn pre_restores(roots: &Roots) -> Vec<PreRestoreInfo> {
    let Some(backups) = backups_dir(roots) else {
        return Vec::new();
    };
    let consuming = match Journal::load(&backups) {
        Ok(Some(Journal {
            source: RestoreSource::PreRestore { stamp },
            ..
        })) => Some(stamp),
        _ => None,
    };
    let Ok(entries) = std::fs::read_dir(&backups) else {
        return Vec::new();
    };
    let mut kept: Vec<PreRestoreInfo> = entries
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().into_string().ok()?;
            let stamp = name.strip_prefix(KEPT_PREFIX).filter(|s| valid_stamp(s))?;
            if consuming.as_deref() == Some(stamp) {
                return None;
            }
            let info = kept_info(roots, stamp)?;
            Some(PreRestoreInfo {
                stamp: info.stamp,
                made_at_ms: info.made_at_ms,
                bytes: info.bytes,
                has_web_part: is_file(
                    &kept_dir(roots, RootKey::Core, stamp)
                        .join(META)
                        .join(WEB_PART),
                ),
            })
        })
        .collect();
    kept.sort_by(|a, b| {
        b.made_at_ms
            .cmp(&a.made_at_ms)
            .then_with(|| b.stamp.cmp(&a.stamp))
    });
    kept
}

/// Deletes a kept set: both roots' folders and its seal key. An invalid
/// or unknown stamp is `GONE`; a set a waiting or running restore uses is
/// `RESTORE_PENDING`. Blocks on the keychain.
pub fn delete_pre_restore(
    roots: &Roots,
    store: &SecretStore,
    stamp: &str,
) -> Result<(), BackupError> {
    if !valid_stamp(stamp) {
        return Err(BackupError::Refused(GONE));
    }
    let backups = backups_dir(roots).ok_or(BackupError::Refused(GONE))?;
    let Lock::Free(_restore_lock) = lock(open_lock_file(&backups.join(RESTORE_LOCK))?)? else {
        return Err(BackupError::Busy(BACKUP_BUSY));
    };
    let named = |source: &RestoreSource| matches!(source, RestoreSource::PreRestore { stamp: used } if used == stamp);
    let journal = Journal::load(&backups)?;
    let in_use = journal
        .as_ref()
        .is_some_and(|journal| journal.stamp == stamp || named(&journal.source))
        || read_pending(&backups)?.is_some_and(|pending| named(&pending.source));
    if in_use {
        return Err(BackupError::Refused(RESTORE_PENDING));
    }
    let mut found = false;
    for root in pass_roots(roots) {
        found |= remove_tree(&kept_dir(roots, *root, stamp))?;
    }
    forget_seal_key(store, &kept_account(stamp))
        .map_err(|_| BackupError::Keychain(KEYCHAIN_NEEDS_YOU))?;
    if found {
        Ok(())
    } else {
        Err(BackupError::Refused(GONE))
    }
}

/// Moves a finished upload of the browser's data from before its own
/// restore into the newest kept set's `meta/web-part.bin` (D18, D19),
/// replacing one kept before.
pub fn keep_previous_web_part(roots: &Roots, upload: &Path) -> Result<(), BackupError> {
    let gone = || BackupError::Refused(GONE);
    backups_dir(roots).ok_or_else(gone)?;
    if !is_file(upload) {
        return Err(gone());
    }
    let newest = pre_restores(roots).into_iter().next().ok_or_else(gone)?;
    let meta = kept_dir(roots, RootKey::Core, &newest.stamp).join(META);
    if !real_dir(&meta) {
        return Err(gone());
    }
    rename_synced(upload, &meta.join(WEB_PART))
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

/// Plans a fresh apply of `pending` and saves the journal. A source that
/// isn't there whole, or whose manifest doesn't match its folders, is
/// never applied: `pending.json` is removed and the answer is `GONE`.
fn plan(
    roots: &Roots,
    store: &SecretStore,
    backups: &Path,
    pending: PendingFile,
) -> Result<Journal, BackupError> {
    let Some(set) = check_source(roots, &pending.source)? else {
        remove_if_there(&backups.join(PENDING))?;
        sync_dir(backups)?;
        return Err(BackupError::Refused(GONE));
    };
    // The source's items open before anything moves; only their accounts
    // are kept.
    let written: Vec<ItemRef> = if set.keychain {
        open_items(roots, store, &pending.source)?
            .iter()
            .map(|item| ItemRef {
                service: item.service.clone(),
                account: item.account.clone(),
            })
            .collect()
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
    one_volume(roots, &pending.source, &passes)?;
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
    stop: &'a dyn Fn(&Step, Crash) -> bool,
}

impl Apply<'_> {
    fn run(mut self) -> Result<ApplyOutcome, BackupError> {
        if !self.journal.done(&Step::Sealed) {
            self.seal()?;
            self.step(Step::Sealed)?;
        }
        for pass in self.journal.passes.clone() {
            for name in &pass.moves {
                let step = Step::Moved {
                    root: pass.root,
                    name: name.clone(),
                };
                if !self.journal.done(&step) {
                    self.move_aside(pass.root, name)?;
                    self.step(step)?;
                }
            }
            let source = source_dir(self.roots, &self.journal.source, pass.root)?;
            for name in &pass.places {
                let step = Step::Placed {
                    root: pass.root,
                    name: name.clone(),
                };
                if !self.journal.done(&step) {
                    self.place(&source, pass.root, name)?;
                    self.step(step)?;
                }
            }
        }
        if !self.journal.done(&Step::KeysWritten) {
            self.write_keys()?;
            self.step(Step::KeysWritten)?;
        }
        if !self.journal.done(&Step::KeysPruned) {
            self.prune_keys()?;
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

    /// Records `step`, stopping where a test asks.
    fn step(&mut self, step: Step) -> Result<(), BackupError> {
        if (self.stop)(&step, Crash::Acted) {
            return Err(stopped());
        }
        self.journal.record(self.backups, step.clone())?;
        if (self.stop)(&step, Crash::Recorded) {
            return Err(stopped());
        }
        Ok(())
    }

    fn kept(&self, root: RootKey) -> PathBuf {
        kept_dir(self.roots, root, &self.journal.stamp)
    }

    /// Makes the pre-restore folders (0700) and seals today's items into
    /// `meta/keychain.sealed` under a fresh key at `pre-restore:<stamp>`
    /// (D13). A seal a stop left behind is made again.
    fn seal(&self) -> Result<(), BackupError> {
        for pass in &self.journal.passes {
            private_dir(&self.roots.backups(pass.root))?;
            private_dir(&self.kept(pass.root))?;
        }
        let kept = self.kept(RootKey::Core);
        let meta = kept.join(META);
        private_dir(&meta)?;
        let sealed = meta.join(SEALED);
        remove_if_there(&sealed)?;
        let items = current_items(self.roots, self.store)?;
        // Fresh OS-random bytes only.
        let mut key = Zeroizing::new([0_u8; 32]);
        random_bytes(&mut *key);
        let mut random = [0_u8; 23];
        random_bytes(&mut random);
        let blob = seal_items(&items, random, *key)
            .map_err(|_| BackupError::Keychain(KEYCHAIN_ITEM_REFUSED))?;
        drop(items);
        let mut file = new_private_file(&sealed)?;
        file.write_all(&blob)?;
        file.sync_all()?;
        sync_dir(&meta)?;
        sync_dir(&kept)?;
        store_seal_key(self.store, &kept_account(&self.journal.stamp), &key)
            .map_err(|_| BackupError::Keychain(KEYCHAIN_NEEDS_YOU))
    }

    /// Renames today's `name` into the pre-restore set. Done already when
    /// it is gone from the root.
    fn move_aside(&self, root: RootKey, name: &str) -> Result<(), BackupError> {
        let from = live(self.roots, root, name);
        let to = self.kept(root).join(name);
        match (exists(&from)?, exists(&to)?) {
            (true, false) => rename_synced(&from, &to),
            (false, _) => Ok(()),
            (true, true) => Err(in_the_way()),
        }
    }

    /// Renames the source's `name` into the root. Done already when it is
    /// gone from the source.
    fn place(&self, source: &Path, root: RootKey, name: &str) -> Result<(), BackupError> {
        let from = source.join(name);
        let to = live(self.roots, root, name);
        match (exists(&from)?, exists(&to)?) {
            (true, false) => {
                if let Some(parent) = to.parent() {
                    make_root(parent)?;
                }
                rename_synced(&from, &to)
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

    /// Removes today's items that the restored catalogs don't name and the
    /// source didn't write; the sealed set keeps them (D16). A source
    /// without items keeps today's (D5). Never a seal key (ruling F11).
    fn prune_keys(&self) -> Result<(), BackupError> {
        if !self.journal.keychain {
            return Ok(());
        }
        let kept = self.kept(RootKey::Core);
        let before = inventory_at(
            &kept.join(CREDENTIALS),
            &kept.join(CONNECTIONS),
            &kept.join(CALENDAR),
        )?;
        let mut after: BTreeSet<ItemRef> = inventory_at(
            &self.roots.core.join(CREDENTIALS),
            &self.roots.core.join(CONNECTIONS),
            &self.roots.config.join(CALENDAR),
        )?
        .into_iter()
        .collect();
        after.extend(self.journal.written.iter().cloned());
        for item in before {
            if item.service == BACKUP_SERVICE || after.contains(&item) {
                continue;
            }
            self.store
                .remove_item(&item.service, &item.account)
                .map_err(|_| BackupError::Keychain(KEYCHAIN_NEEDS_YOU))?;
        }
        Ok(())
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
        private_dir(&folder)?;
        if exists(&from)? {
            rename_synced(&from, &folder.join(format!("{id}.bin")))?;
        }
        Ok(())
    }

    /// Writes the kept set's `meta/info.json` from what it holds, then
    /// `result.json`. Both are replaced whole, so doing it again is safe.
    fn finish(&self) -> Result<(), BackupError> {
        let mut roots = Vec::new();
        let mut bytes = 0_u64;
        for pass in &self.journal.passes {
            let folder = self.kept(pass.root);
            let names = top_names(&folder)?;
            for name in &names {
                bytes = bytes.saturating_add(tree_bytes(&folder.join(name))?);
            }
            roots.push(KeptRoot {
                root: pass.root,
                names,
            });
        }
        let info = KeptInfo {
            stamp: self.journal.stamp.clone(),
            made_at_ms: self.journal.made_at_ms,
            bytes,
            keychain: true,
            roots,
        };
        write_atomically(&self.kept(RootKey::Core).join(META), INFO, &to_json(&info)?)?;
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
            warnings: Vec::new(),
            service_to_restart: self.journal.service_to_restart,
            web_part,
        };
        write_atomically(self.backups, RESULT, &to_json(&result)?)
    }

    /// Removes the source and its seal key, then `pending.json`, then the
    /// journal.
    fn clean_up(&self) -> Result<(), BackupError> {
        let account = match &self.journal.source {
            RestoreSource::Staged { id } => format!("staged:{id}"),
            RestoreSource::PreRestore { stamp } => kept_account(stamp),
            RestoreSource::Other => return Err(BackupError::Refused(INVALID_REQUEST)),
        };
        for pass in &self.journal.passes {
            remove_tree(&source_dir(self.roots, &self.journal.source, pass.root)?)?;
        }
        if forget_seal_key(self.store, &account).is_err() {
            tracing::warn!("Bruhs couldn't remove a restored set's key");
        }
        remove_if_there(&self.backups.join(PENDING))?;
        sync_dir(self.backups)?;
        Journal::remove(self.backups)
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
        // Where Task 7 staged it (D11).
        let root = if roots.same() {
            RootKey::Core
        } else if manifest.same_root && desktop_entry(top) {
            RootKey::Desktop
        } else {
            origin
        };
        if let Some((_, names)) = expected.iter_mut().find(|(key, _)| *key == root) {
            names.insert(top);
        }
    }
    let mut places = Vec::new();
    for (root, names) in &expected {
        let Some(staged) = staged_names(&staging_dir(roots, *root, id))? else {
            return Ok(None);
        };
        if !names
            .iter()
            .all(|name| staged.iter().any(|staged| staged == name))
        {
            return Ok(None);
        }
        places.push((*root, staged));
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
    }))
}

fn check_kept(roots: &Roots, stamp: &str) -> Result<Option<SourceSet>, BackupError> {
    let Some(info) = kept_info(roots, stamp) else {
        return Ok(None);
    };
    if info
        .roots
        .iter()
        .any(|kept| !pass_roots(roots).contains(&kept.root))
    {
        return Ok(None);
    }
    let mut places = Vec::new();
    for root in pass_roots(roots) {
        let folder = kept_dir(roots, *root, stamp);
        if !real_dir(&folder) {
            return Ok(None);
        }
        let names = info
            .roots
            .iter()
            .find(|kept| kept.root == *root)
            .map(|kept| kept.names.clone())
            .unwrap_or_default();
        for name in &names {
            if !movable(name) || !exists(&folder.join(name))? {
                return Ok(None);
            }
        }
        places.push((*root, names));
    }
    let meta = kept_dir(roots, RootKey::Core, stamp).join(META);
    if info.keychain && !is_file(&meta.join(SEALED)) {
        return Ok(None);
    }
    Ok(Some(SourceSet {
        places,
        keychain: info.keychain,
        web_part: is_file(&meta.join(WEB_PART)),
        rewrites: Vec::new(),
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

/// Refuses a plan whose renames would cross volumes: a rename can't, and
/// an apply that stopped there could never finish.
fn one_volume(roots: &Roots, source: &RestoreSource, passes: &[Pass]) -> Result<(), BackupError> {
    for pass in passes {
        let backups = roots.backups(pass.root);
        private_dir(&backups)?;
        let kept = device(&backups)?;
        let staged = device(&source_dir(roots, source, pass.root)?)?;
        for name in &pass.moves {
            if device(&live_parent(roots, pass.root, name))? != kept {
                return Err(other_volume());
            }
        }
        for name in &pass.places {
            if nearest_device(&live_parent(roots, pass.root, name))? != staged {
                return Err(other_volume());
            }
        }
    }
    Ok(())
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
    unseal_items(&blob, *key).map_err(|_| unreadable())
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

/// The keychain account of a kept set's seal key.
fn kept_account(stamp: &str) -> String {
    format!("pre-restore:{stamp}")
}

/// `<root>/backups/pre-restore-<stamp>`.
fn kept_dir(roots: &Roots, root: RootKey, stamp: &str) -> PathBuf {
    roots.backups(root).join(format!("{KEPT_PREFIX}{stamp}"))
}

/// A kept set's info, when it names the set's stamp.
fn kept_info(roots: &Roots, stamp: &str) -> Option<KeptInfo> {
    let meta = kept_dir(roots, RootKey::Core, stamp).join(META);
    if !real_dir(&kept_dir(roots, RootKey::Core, stamp)) || !real_dir(&meta) {
        return None;
    }
    read_json::<KeptInfo>(&meta.join(INFO), SMALL_MAX).filter(|info| info.stamp == stamp)
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
fn pass_roots(roots: &Roots) -> &'static [RootKey] {
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
fn tree_bytes(path: &Path) -> Result<u64, BackupError> {
    let mut bytes = 0_u64;
    let mut folders = Vec::new();
    let metadata = std::fs::symlink_metadata(path)?;
    if metadata.is_dir() {
        folders.push(path.to_path_buf());
    } else if metadata.is_file() {
        bytes = metadata.len();
    }
    while let Some(folder) = folders.pop() {
        for entry in std::fs::read_dir(&folder)? {
            let entry = entry?;
            let metadata = entry.metadata()?;
            if metadata.is_dir() {
                folders.push(entry.path());
            } else if metadata.is_file() {
                bytes = bytes.saturating_add(metadata.len());
            }
        }
    }
    Ok(bytes)
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

/// Makes a missing root folder (and its parents) 0700.
fn make_root(folder: &Path) -> Result<(), BackupError> {
    if !exists(folder)? {
        DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(folder)?;
    }
    Ok(())
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

fn lock(file: File) -> io::Result<Lock> {
    match file.try_lock() {
        Ok(()) => Ok(Lock::Free(Some(file))),
        Err(TryLockError::WouldBlock) => Ok(Lock::Held),
        Err(TryLockError::Error(error)) => Err(error),
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

fn in_the_way() -> BackupError {
    BackupError::Io(io::Error::other("an entry is in the way of the restore"))
}

fn other_volume() -> BackupError {
    BackupError::Io(io::Error::other("a restore can't move data across volumes"))
}

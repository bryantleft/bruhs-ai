//! Kept pre-restore sets (D18): listing, deleting and checking them, the
//! browser part kept with one, what a rollback parks in one, and the
//! sweep of what a stopped apply or delete left behind.
//!
//! A set is `<root>/backups/pre-restore-<stamp>/` in each root. Its
//! manifest is the core folder's `meta/info.json`: a folder without one
//! isn't a set, and the sweep removes it once no apply can be making it.
//! A set may hold `.in-the-way-<n>/` folders: entries something made
//! while a restore ran, kept so they aren't lost. A put back leaves them
//! in the set it consumes and carries them into the new one.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use bruhs_models::backup::{
    valid_id, valid_stamp, PreRestoreInfo, RestoreSource, BACKUP_BUSY, DAMAGED, GONE,
    KEYCHAIN_NEEDS_YOU, NOTHING_TO_PUT_BACK, RESTORE_PENDING,
};
use bruhs_models::EpochMs;
use serde::{Deserialize, Serialize};

use super::{
    backups_dir, exists, in_the_way, is_file, lock, make_root, movable, new_dir, open_lock_file,
    pass_roots, read_json, read_pending, real_dir, remove_if_there, remove_tree, rename_retrying,
    rename_synced, to_json, top_names, tree_bytes, write_atomically, Apply, Journal, Lock,
    SourceSet, INFO, KEPT_PREFIX, META, RESTORE_LOCK, SEALED, SMALL_MAX, WEB_PART,
};
use crate::backup::journal::{sync_dir, JOURNAL};
use crate::backup::{BackupError, RootKey, Roots};
use crate::secrets::{forget_seal_key, SecretStore};

/// What a rollback parks things in the way under: `.in-the-way-<n>`.
const PARKED: &str = ".in-the-way";
/// The free `.in-the-way-<n>` a park looks for among this many.
const PARK_TRIES: u32 = 1000;
/// A kept set being deleted: `<root>/backups/.deleting-<stamp>`.
const DELETING: &str = ".deleting-";
/// Seal keys a removal failed for, retried by the sweep.
const ORPHAN_KEYS: &str = "keys-to-forget.json";
/// Tries of a seal key removal before it waits for the sweep.
const FORGET_TRIES: usize = 3;

/// A kept set's `meta/info.json`: its manifest. A put back acts only on a
/// set whose info names its stamp and whose entries are all there.
/// Every field but the stamp has a default, so a set a newer Bruhs wrote
/// still reads here (R2-I1).
#[derive(Serialize, Deserialize)]
pub(super) struct KeptInfo {
    stamp: String,
    #[serde(default)]
    made_at_ms: EpochMs,
    #[serde(default)]
    bytes: u64,
    /// Whether `meta/keychain.sealed` holds the day's items. When an info
    /// doesn't say, a sealed file being there does (R3-M3).
    #[serde(default)]
    keychain: Option<bool>,
    /// What each root's folder holds. An info without it is damaged, never
    /// a set with nothing to put back (R3-M3).
    #[serde(default)]
    roots: Option<Vec<KeptRoot>>,
}

#[derive(Serialize, Deserialize)]
pub(super) struct KeptRoot {
    root: RootKey,
    #[serde(default)]
    names: Vec<String>,
}

/// The sets kept from earlier restores, newest first (D18). A set a put
/// back is consuming isn't listed.
pub fn pre_restores(roots: &Roots) -> Vec<PreRestoreInfo> {
    let Some(backups) = backups_dir(roots) else {
        return Vec::new();
    };
    let consuming = match Journal::load(roots) {
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

/// Deletes a kept set: its seal key, then both roots' folders. An invalid
/// or unknown stamp is `GONE`; a set a waiting or running restore uses is
/// `RESTORE_PENDING`; one an apply holds `restore.lock` over,
/// `BACKUP_BUSY`. Blocks on the keychain.
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
    let journal = Journal::load(roots)?;
    let in_use = journal
        .as_ref()
        .is_some_and(|journal| journal.stamp == stamp || named(&journal.source))
        || read_pending(&backups)?.is_some_and(|pending| named(&pending.source));
    if in_use {
        return Err(BackupError::Refused(RESTORE_PENDING));
    }
    if !pass_roots(roots)
        .iter()
        .any(|root| real_dir(&kept_dir(roots, *root, stamp)))
    {
        return Err(BackupError::Refused(GONE));
    }
    // The key goes first: a keychain that refuses leaves the set whole, and
    // a stop after it leaves a set whose key is gone, never a key without
    // its set (M8). Each folder is then renamed to `.deleting-<stamp>`,
    // which unlists it at once; a stop before it is removed leaves a name
    // the sweep removes without judging any set (R2-I1).
    forget_seal_key(store, &kept_account(stamp))
        .map_err(|_| BackupError::Keychain(KEYCHAIN_NEEDS_YOU))?;
    for root in pass_roots(roots) {
        let folder = kept_dir(roots, *root, stamp);
        if !real_dir(&folder) {
            continue;
        }
        let doomed = roots.backups(*root).join(format!("{DELETING}{stamp}"));
        remove_tree(&doomed)?;
        rename_synced(&folder, &doomed)?;
        remove_tree(&doomed)?;
    }
    Ok(())
}

/// Moves a finished upload of the browser's data from before its own
/// restore into the newest kept set's `meta/web-part.bin` (D18, D19).
/// The first upload stays: a later one is dropped, since a browser restore
/// retried after its write would send the restored data, not the data from
/// before (B Task 14 I2). `BACKUP_BUSY` while an apply holds
/// `restore.lock`.
pub fn keep_previous_web_part(roots: &Roots, upload: &Path) -> Result<(), BackupError> {
    let gone = || BackupError::Refused(GONE);
    let backups = backups_dir(roots).ok_or_else(gone)?;
    let Lock::Free(_restore_lock) = lock(open_lock_file(&backups.join(RESTORE_LOCK))?)? else {
        return Err(BackupError::Busy(BACKUP_BUSY));
    };
    if !is_file(upload) {
        return Err(gone());
    }
    let newest = pre_restores(roots).into_iter().next().ok_or_else(gone)?;
    let meta = kept_dir(roots, RootKey::Core, &newest.stamp).join(META);
    if !real_dir(&meta) {
        return Err(gone());
    }
    let kept = meta.join(WEB_PART);
    if exists(&kept)? {
        return remove_if_there(upload);
    }
    rename_synced(upload, &kept)
}

pub(super) fn check_kept(roots: &Roots, stamp: &str) -> Result<Option<SourceSet>, BackupError> {
    let Some(info) = kept_info(roots, stamp) else {
        return Ok(None);
    };
    let Some(listed) = &info.roots else {
        return Err(BackupError::Format(DAMAGED));
    };
    if listed
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
        // What a rollback parked stays in the set (N7).
        let names: Vec<String> = listed
            .iter()
            .find(|kept| kept.root == *root)
            .map(|kept| kept.names.clone())
            .unwrap_or_default()
            .into_iter()
            .filter(|name| !parked_folder(&folder, name))
            .collect();
        for name in &names {
            if !movable(name) || !exists(&folder.join(name))? {
                return Ok(None);
            }
        }
        places.push((*root, names));
    }
    // A set that holds only what a rollback parked is no state to go back
    // to: putting it back would empty the roots.
    if places.iter().all(|(_, names)| names.is_empty()) {
        return Err(BackupError::Refused(NOTHING_TO_PUT_BACK));
    }
    let meta = kept_dir(roots, RootKey::Core, stamp).join(META);
    let sealed = is_file(&meta.join(SEALED));
    let keychain = info.keychain.unwrap_or(sealed);
    if keychain && !sealed {
        return Ok(None);
    }
    Ok(Some(SourceSet {
        places,
        keychain,
        web_part: is_file(&meta.join(WEB_PART)),
        rewrites: Vec::new(),
        protocol: None,
    }))
}

/// The keychain account of a kept set's seal key.
pub(super) fn kept_account(stamp: &str) -> String {
    format!("pre-restore:{stamp}")
}

/// `<root>/backups/pre-restore-<stamp>`.
pub(super) fn kept_dir(roots: &Roots, root: RootKey, stamp: &str) -> PathBuf {
    roots.backups(root).join(format!("{KEPT_PREFIX}{stamp}"))
}

/// A kept set's info, when it names the set's stamp.
pub(super) fn kept_info(roots: &Roots, stamp: &str) -> Option<KeptInfo> {
    let meta = kept_dir(roots, RootKey::Core, stamp).join(META);
    if !real_dir(&kept_dir(roots, RootKey::Core, stamp)) || !real_dir(&meta) {
        return None;
    }
    read_json::<KeptInfo>(&meta.join(INFO), SMALL_MAX).filter(|info| info.stamp == stamp)
}

/// Writes the set `stamp`'s `meta/info.json` from what its folders in
/// `roots_of` hold: its manifest, which a put back checks.
fn write_set_info(
    roots: &Roots,
    stamp: &str,
    made_at_ms: EpochMs,
    keychain: bool,
    roots_of: &[RootKey],
) -> Result<(), BackupError> {
    let mut listed = Vec::new();
    let mut bytes = 0_u64;
    for root in roots_of {
        let folder = kept_dir(roots, *root, stamp);
        let names = if real_dir(&folder) {
            top_names(&folder)?
        } else {
            Vec::new()
        };
        for name in &names {
            bytes = bytes.saturating_add(tree_bytes(&folder.join(name)));
        }
        listed.push(KeptRoot { root: *root, names });
    }
    let info = KeptInfo {
        stamp: stamp.to_owned(),
        made_at_ms,
        bytes,
        keychain: Some(keychain),
        roots: Some(listed),
    };
    let core = kept_dir(roots, RootKey::Core, stamp);
    let meta = core.join(META);
    new_dir(&core)?;
    new_dir(&meta)?;
    write_atomically(&meta, INFO, &to_json(&info)?)
}

impl Apply<'_> {
    fn pass_keys(&self) -> Vec<RootKey> {
        self.journal.passes.iter().map(|pass| pass.root).collect()
    }

    /// Writes the new kept set's `meta/info.json` from what it holds.
    pub(super) fn write_info(&self, keychain: bool) -> Result<(), BackupError> {
        write_set_info(
            self.roots,
            &self.journal.stamp,
            self.journal.made_at_ms,
            keychain,
            &self.pass_keys(),
        )
    }

    /// After a put back whose carry of parked entries failed (R2-M3), the
    /// set put back keeps those entries as a set with nothing to put back:
    /// its seal and key go and its info lists only them. Answers whether
    /// the set stays. Safe to repeat.
    pub(super) fn keep_parked_source(&self) -> Result<bool, BackupError> {
        let RestoreSource::PreRestore { stamp } = &self.journal.source else {
            return Ok(false);
        };
        // A parked folder the carry emptied holds nothing to keep.
        let mut held = false;
        for root in self.pass_keys() {
            let folder = kept_dir(self.roots, root, stamp);
            if !real_dir(&folder) {
                continue;
            }
            for name in top_names(&folder)? {
                if parked_folder(&folder, &name)
                    && std::fs::read_dir(folder.join(&name))?.next().is_some()
                {
                    held = true;
                }
            }
        }
        if !held {
            return Ok(false);
        }
        let meta = kept_dir(self.roots, RootKey::Core, stamp).join(META);
        remove_if_there(&meta.join(SEALED))?;
        remove_if_there(&meta.join(WEB_PART))?;
        forget_key(self.store, self.backups, &kept_account(stamp));
        let made_at_ms =
            kept_info(self.roots, stamp).map_or(self.journal.made_at_ms, |info| info.made_at_ms);
        write_set_info(self.roots, stamp, made_at_ms, false, &self.pass_keys())?;
        Ok(true)
    }
}

/// Whether `name` in the kept folder `folder` is one a rollback parked
/// things in (N2, N7): named `.in-the-way-<digits>` and a real folder,
/// never an owner's own entry that only looks alike (R2-M1, R3-M1).
pub(super) fn parked_folder(folder: &Path, name: &str) -> bool {
    let named = name
        .strip_prefix(PARKED)
        .and_then(|rest| rest.strip_prefix('-'))
        .is_some_and(|digits| !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()));
    named && real_dir(&folder.join(name))
}

/// Moves `from` into the first `.in-the-way-<n>/<rel>` of the kept folder
/// `kept` that is free (N2): a park left by an earlier stop never blocks
/// the next one.
pub(super) fn park(kept: &Path, rel: &Path, from: &Path) -> Result<(), BackupError> {
    for n in 1..=PARK_TRIES {
        let folder = kept.join(format!("{PARKED}-{n}"));
        let target = folder.join(rel);
        if free(&folder, &target)? {
            if let Some(parent) = target.parent() {
                make_root(parent)?;
            }
            return rename_retrying(from, &target);
        }
    }
    Err(in_the_way())
}

/// Whether `target` under `folder` is free: nothing there, and nothing
/// but folders on the way down to it.
fn free(folder: &Path, target: &Path) -> Result<bool, BackupError> {
    if exists(target)? {
        return Ok(false);
    }
    let mut at = target.parent();
    while let Some(path) = at {
        if !path.starts_with(folder) {
            break;
        }
        match std::fs::symlink_metadata(path) {
            Ok(metadata) => return Ok(metadata.is_dir()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => at = path.parent(),
            Err(error) => return Err(error.into()),
        }
    }
    Ok(true)
}

/// Removes a seal key, a few times over (B Task 8 out of scope). One the
/// keychain won't remove goes on the sweep's list, so no key outlives its
/// set for good. Blocks on the keychain.
pub(in crate::backup) fn forget_key(store: &SecretStore, backups: &Path, account: &str) {
    for _ in 0..FORGET_TRIES {
        if forget_seal_key(store, account).is_ok() {
            return;
        }
    }
    tracing::warn!("Bruhs couldn't remove a restore's key; it tries again later");
    let mut list: Vec<String> =
        read_json(&backups.join(ORPHAN_KEYS), SMALL_MAX).unwrap_or_default();
    if !list.iter().any(|known| known == account) {
        list.push(account.to_owned());
        let listed = to_json(&list).and_then(|text| write_atomically(backups, ORPHAN_KEYS, &text));
        if listed.is_err() {
            tracing::warn!("Bruhs couldn't note a restore's key to remove later");
        }
    }
}

/// The restore side of the sweep (B Task 8 N6, M11, R2-I1, out of scope),
/// only while no apply runs (`restore.lock` free, held throughout):
/// - the partial records a stopped process left in `backups`;
/// - `.deleting-<stamp>` folders, a delete that stopped part way;
///
/// and, while no journal waits either:
///
/// - `pre-restore-*` folders whose manifest is not there at all (never
///   one that only fails to read), with their keys;
/// - seal keys a removal failed for, once their set is gone.
///
/// Blocks on the keychain.
pub(in crate::backup) fn sweep_restore(roots: &Roots, store: &SecretStore) {
    let Some(backups) = backups_dir(roots) else {
        return;
    };
    let Ok(Lock::Free(_restore_lock)) = open_lock_file(&backups.join(RESTORE_LOCK)).and_then(lock)
    else {
        return;
    };
    for record in [super::PENDING, JOURNAL, super::RESULT, ORPHAN_KEYS] {
        let leftover = format!(".{record}.partial-");
        super::super::host::sweep_folder(
            &backups,
            |_| true,
            |name| name.strip_prefix(leftover.as_str()).is_some_and(valid_id),
        );
    }
    // A delete that stopped part way.
    for root in pass_roots(roots) {
        let Ok(entries) = std::fs::read_dir(roots.backups(*root)) else {
            continue;
        };
        for entry in entries.flatten() {
            let doomed = entry
                .file_name()
                .to_str()
                .and_then(|name| name.strip_prefix(DELETING))
                .is_some_and(valid_stamp);
            if doomed && remove_tree(&entry.path()).is_err() {
                tracing::warn!("Bruhs couldn't finish deleting a kept restore set");
            }
        }
    }
    if !matches!(exists(&backups.join(JOURNAL)), Ok(false)) {
        return;
    }
    for root in pass_roots(roots) {
        let Ok(entries) = std::fs::read_dir(roots.backups(*root)) else {
            continue;
        };
        for entry in entries.flatten() {
            let Ok(name) = entry.file_name().into_string() else {
                continue;
            };
            let Some(stamp) = name.strip_prefix(KEPT_PREFIX).filter(|s| valid_stamp(s)) else {
                continue;
            };
            if !real_dir(&entry.path()) {
                continue;
            }
            // Only a set whose manifest is plainly not there goes: one that
            // can't be read or parsed may be the owner's only undo (R2-I1).
            let info = kept_dir(roots, RootKey::Core, stamp).join(META).join(INFO);
            match std::fs::symlink_metadata(&info) {
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Ok(_) => {
                    if kept_info(roots, stamp).is_none() {
                        tracing::warn!("A kept restore set's info doesn't read; Bruhs leaves it");
                    }
                    continue;
                }
                Err(_) => {
                    tracing::warn!("A kept restore set's info doesn't read; Bruhs leaves it");
                    continue;
                }
            }
            if remove_tree(&entry.path()).is_err() {
                tracing::warn!("Bruhs couldn't remove a restore's leftover folder");
            }
            forget_key(store, &backups, &kept_account(stamp));
        }
    }
    forget_orphan_keys(roots, store, &backups);
}

/// Retries the seal keys on the sweep's list whose set is gone. A key
/// whose set is there again stays: it is that set's.
fn forget_orphan_keys(roots: &Roots, store: &SecretStore, backups: &Path) {
    let path = backups.join(ORPHAN_KEYS);
    let Some(list) = read_json::<Vec<String>>(&path, SMALL_MAX) else {
        return;
    };
    let set_there = |folder: &dyn Fn(RootKey) -> PathBuf| {
        pass_roots(roots)
            .iter()
            .any(|root| real_dir(&folder(*root)))
    };
    let mut left = Vec::new();
    for account in list {
        let gone = if let Some(id) = account.strip_prefix("staged:").filter(|id| valid_id(id)) {
            !set_there(&|root| super::staging_dir(roots, root, id))
        } else if let Some(stamp) = account
            .strip_prefix("pre-restore:")
            .filter(|stamp| valid_stamp(stamp))
        {
            !set_there(&|root| kept_dir(roots, root, stamp))
        } else {
            continue;
        };
        // A key whose set is still there stays listed until the set goes
        // (R2-M2).
        if !gone || forget_seal_key(store, &account).is_err() {
            left.push(account);
        }
    }
    let written = if left.is_empty() {
        remove_if_there(&path).and_then(|()| sync_dir(backups))
    } else {
        to_json(&left).and_then(|text| write_atomically(backups, ORPHAN_KEYS, &text))
    };
    if written.is_err() {
        tracing::warn!("Bruhs couldn't update the restore keys to remove");
    }
}

/// Takes `restore.lock`, waiting up to `wait` for an apply that holds it.
pub(super) fn hold_restore_lock(backups: &Path, wait: Duration) -> Option<std::fs::File> {
    let until = Instant::now() + wait;
    loop {
        match open_lock_file(&backups.join(RESTORE_LOCK)).and_then(lock) {
            Ok(Lock::Free(file)) => return file,
            Ok(Lock::Held) if Instant::now() < until => {
                std::thread::sleep(Duration::from_millis(50));
            }
            _ => return None,
        }
    }
}

impl Apply<'_> {
    /// A put back takes along what its set had parked, into the new set
    /// (N7), so consuming the set never drops it. Each move is judged by
    /// what is on disk, so repeating it is safe.
    pub(super) fn carry_parked(&self) -> Result<(), BackupError> {
        let RestoreSource::PreRestore { stamp } = &self.journal.source else {
            return Ok(());
        };
        for pass in &self.journal.passes {
            let from = kept_dir(self.roots, pass.root, stamp);
            if !real_dir(&from) {
                continue;
            }
            for name in top_names(&from)? {
                if !parked_folder(&from, &name) {
                    continue;
                }
                let folder = from.join(&name);
                for entry in std::fs::read_dir(&folder)? {
                    let entry = entry?;
                    let rel = PathBuf::from(entry.file_name());
                    park(&self.kept(pass.root), &rel, &entry.path())?;
                }
            }
        }
        Ok(())
    }
}

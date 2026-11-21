//! What the server's confirmation of a restore checks before anything
//! stops, and the steps around it (D15, D18, D19; rulings F21, F22): the
//! source's check, the desktop's lock, taking back a schedule the server
//! couldn't hand over, and the browser part that waits after an apply.
//!
//! Nothing here logs a path, a name from inside a backup or a value.

use std::path::PathBuf;

use bruhs_models::backup::{
    valid_id, RestoreSource, WebPartInfo, BACKUP_BUSY, GONE, RESTORE_PENDING,
};

use super::{
    backups_dir, check_source, delete_pre_restore, exists, hold_restore_lock, instance_lock_path,
    lock_existing, open_items, read_result, real_dir, remove_if_there, to_json, waiting_web_part,
    write_atomically, Lock, RESTORE_LOCK_WAIT, RESULT, WEB_PARTS,
};
use crate::backup::host::{BackupHost, PENDING};
use crate::backup::journal::{sync_dir, JOURNAL};
use crate::backup::{BackupError, Roots};
use crate::secrets::SecretStore;

/// Checks that `source` can be applied, before the server stops anything
/// (D15): a finished staged restore or a whole kept set (`GONE`,
/// `DAMAGED`, `NOTHING_TO_PUT_BACK`), whose sealed items open with the key
/// it keeps (`KEYCHAIN_SET_UNREADABLE`, as for a set whose delete stopped
/// after its key went; `KEYCHAIN_NEEDS_YOU`). Answers the crew protocol of
/// the data it brings: a backup's, or `None` for a kept set, which is this
/// computer's own. The items are dropped at once. Blocks on the keychain.
pub fn check_restore(
    roots: &Roots,
    store: &SecretStore,
    source: &RestoreSource,
) -> Result<Option<u32>, BackupError> {
    let set = check_source(roots, source)?.ok_or(BackupError::Refused(GONE))?;
    if set.keychain {
        drop(open_items(roots, store, source)?);
    }
    Ok(set.protocol)
}

/// Whether the desktop app runs: another handle holds its instance lock
/// (D17, ruling F21 b). A lock file that isn't there is free; none is made.
pub fn desktop_running(roots: &Roots) -> Result<bool, BackupError> {
    Ok(matches!(
        lock_existing(&instance_lock_path(&roots.desktop))?,
        Lock::Held
    ))
}

/// Takes back a restore this server scheduled but couldn't hand over
/// (ruling F22 a): removes `pending.json` while no apply has begun.
/// `BACKUP_BUSY` while an apply holds `restore.lock` past a short wait,
/// `RESTORE_PENDING` once one began.
pub fn unschedule(roots: &Roots) -> Result<(), BackupError> {
    let Some(backups) = backups_dir(roots) else {
        return Ok(());
    };
    let _restore_lock =
        hold_restore_lock(&backups, RESTORE_LOCK_WAIT).ok_or(BackupError::Busy(BACKUP_BUSY))?;
    if exists(&backups.join(JOURNAL))? {
        return Err(BackupError::Refused(RESTORE_PENDING));
    }
    remove_if_there(&backups.join(PENDING))?;
    sync_dir(&backups)?;
    Ok(())
}

/// The browser part that waits for the browser (D19), with its file in
/// `backups/web-part/`.
pub fn waiting_web_part_file(roots: &Roots) -> Option<(WebPartInfo, PathBuf)> {
    let part = waiting_web_part(roots)?;
    let path = backups_dir(roots)?
        .join(WEB_PARTS)
        .join(format!("{}.bin", part.id));
    Some((part, path))
}

/// Removes the waiting browser part once the browser restored it (D19):
/// its file, then its place in `result.json`. `GONE` when none waits,
/// `BACKUP_BUSY` while an apply holds `restore.lock`.
pub fn finish_web_part(roots: &Roots) -> Result<(), BackupError> {
    let backups = backups_dir(roots).ok_or(BackupError::Refused(GONE))?;
    let _restore_lock =
        hold_restore_lock(&backups, RESTORE_LOCK_WAIT).ok_or(BackupError::Busy(BACKUP_BUSY))?;
    let mut result = read_result(&backups).ok_or(BackupError::Refused(GONE))?;
    let part = result.web_part.take().ok_or(BackupError::Refused(GONE))?;
    let folder = backups.join(WEB_PARTS);
    if valid_id(&part.id) && real_dir(&folder) {
        remove_if_there(&folder.join(format!("{}.bin", part.id)))?;
        sync_dir(&folder)?;
    }
    write_atomically(&backups, RESULT, &to_json(&result)?)
}

impl BackupHost {
    /// [`check_restore`] with this host's keychain. Blocks on it.
    pub fn check_restore(&self, source: &RestoreSource) -> Result<Option<u32>, BackupError> {
        check_restore(self.roots(), &self.config().store, source)
    }

    /// Deletes a kept set with this host's keychain
    /// ([`delete_pre_restore`]). Blocks on it.
    pub fn delete_kept(&self, stamp: &str) -> Result<(), BackupError> {
        delete_pre_restore(self.roots(), &self.config().store, stamp)
    }
}

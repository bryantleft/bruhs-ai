//! Putting today's data back when a restore can't finish (B Task 8 I1).
//!
//! Before the keychain writes, a rename that fails for good (something
//! took a name the apply needs, no permission, another volume) records
//! `RollingBack` and undoes every rename in reverse. Each undo is judged by
//! what is on disk, as the forward renames are: an entry already back is
//! done, so a stop anywhere in the rollback resumes into the rollback at
//! the next start. Then the new pre-restore set's seal and key go,
//! `pending.json` and the journal go, and the answer is
//! `RESTORE_ROLLED_BACK`. The source stays as it was: a staged restore is
//! left to the 24-hour sweep, a kept set keeps its entries.
//!
//! Nothing made mid-apply is thrown away (N2, N3). Something that took a
//! name today's entry left (the capture daemon making `audio/`, say), and
//! a file written inside a restored folder that the backup's manifest
//! doesn't name (a transcript in the restored `audio/`), move into a free
//! `.in-the-way-<n>/` of the pre-restore set, which then stays, listed,
//! with no keychain items. The staging the rest goes back to is swept
//! after 24 hours, so it must hold nothing but the backup.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use bruhs_models::backup::{RestoreSource, RESTORE_ROLLED_BACK};
use bruhs_models::EpochMs;

use super::{
    exists, full_sync, kept_account, live, make_root, manifest_files, park, read_result, real_dir,
    remove_if_there, remove_tree, rename_retrying, source_dir, stopped, to_json, top_names,
    write_atomically, Apply, ApplyOutcome, Journal, Point, ResultFile, Step, KEYCHAIN_NEEDS_YOU,
    META, PENDING, RESULT, SEALED,
};
use crate::backup::{BackupError, RootKey};
use crate::secrets::forget_seal_key;

impl Apply<'_> {
    /// Undoes every rename, then removes the new set's seal, `pending.json`
    /// and the journal. Always answers `RESTORE_ROLLED_BACK` once done.
    pub(super) fn roll_back(&mut self) -> Result<ApplyOutcome, BackupError> {
        if !self.journal.done(&Step::RollingBack) {
            self.step(Step::RollingBack)?;
        }
        let undo: Vec<Step> = self.journal.steps.iter().rev().cloned().collect();
        for step in &undo {
            match step {
                Step::Placed { root, name } => self.unplace(*root, name)?,
                Step::Moved { root, name } => self.unmove(*root, name)?,
                _ => continue,
            }
            if (self.stop)(step, Point::Undone) {
                return Err(stopped());
            }
        }
        // The seal held today's items, which never changed.
        remove_if_there(&self.kept(RootKey::Core).join(META).join(SEALED))?;
        forget_seal_key(self.store, &kept_account(&self.journal.stamp))
            .map_err(|_| BackupError::Keychain(KEYCHAIN_NEEDS_YOU))?;
        let mut held = false;
        for pass in &self.journal.passes {
            let folder = self.kept(pass.root);
            held |= real_dir(&folder) && !top_names(&folder)?.is_empty();
        }
        if held {
            self.write_info(false)?;
        } else {
            for pass in &self.journal.passes {
                remove_tree(&self.kept(pass.root))?;
            }
        }
        // The status shows the rollback, and the service still starts
        // again. A browser part an earlier restore left waiting still
        // waits (N1).
        let previous = read_result(self.backups);
        let result = ResultFile {
            applied_at_ms: EpochMs::now(),
            stamp: if held {
                self.journal.stamp.clone()
            } else {
                previous
                    .as_ref()
                    .map(|previous| previous.stamp.clone())
                    .unwrap_or_default()
            },
            warnings: vec![RESTORE_ROLLED_BACK.to_owned()],
            service_to_restart: self.journal.service_to_restart,
            web_part_stamp: previous
                .as_ref()
                .and_then(|previous| previous.web_part_stamp.clone()),
            web_part: previous.and_then(|previous| previous.web_part),
        };
        write_atomically(self.backups, RESULT, &to_json(&result)?)?;
        remove_if_there(&self.backups.join(PENDING))?;
        full_sync(self.backups)?;
        Journal::remove(self.backups)?;
        Err(BackupError::Refused(RESTORE_ROLLED_BACK))
    }

    /// Puts a placed entry back into the source. Nothing to do when the
    /// source holds it (never placed, or undone already) or the root
    /// doesn't.
    fn unplace(&self, root: RootKey, name: &str) -> Result<(), BackupError> {
        let back = source_dir(self.roots, &self.journal.source, root)?.join(name);
        let placed = live(self.roots, root, name);
        if exists(&back)? || !exists(&placed)? {
            return Ok(());
        }
        if let RestoreSource::Staged { id } = &self.journal.source {
            if let Some(named) = manifest_files(self.roots, id, root) {
                self.park_extras(root, name, &placed, &named)?;
            }
        }
        rename_retrying(&placed, &back)
    }

    /// Parks every file under the placed folder `placed` that the
    /// manifest doesn't name (N3): written there after it was placed.
    fn park_extras(
        &self,
        root: RootKey,
        name: &str,
        placed: &Path,
        named: &BTreeSet<String>,
    ) -> Result<(), BackupError> {
        if !std::fs::symlink_metadata(placed)?.is_dir() {
            return Ok(());
        }
        let mut folders = vec![(placed.to_path_buf(), PathBuf::from(name))];
        while let Some((folder, rel)) = folders.pop() {
            for entry in std::fs::read_dir(&folder)? {
                let entry = entry?;
                let rel = rel.join(entry.file_name());
                if entry.file_type()?.is_dir() {
                    folders.push((entry.path(), rel));
                } else if !rel.to_str().is_some_and(|text| named.contains(text)) {
                    park(&self.kept(root), &rel, &entry.path())?;
                }
            }
        }
        Ok(())
    }

    /// Puts today's entry back into the root. Nothing to do when the kept
    /// set no longer holds it. Whatever took its name meanwhile moves into
    /// a free `.in-the-way-<n>/` of the kept set (N2).
    fn unmove(&self, root: RootKey, name: &str) -> Result<(), BackupError> {
        let kept = self.kept(root).join(name);
        if !exists(&kept)? {
            return Ok(());
        }
        let back = live(self.roots, root, name);
        if exists(&back)? {
            park(&self.kept(root), Path::new(name), &back)?;
        }
        if let Some(parent) = back.parent() {
            make_root(parent)?;
        }
        rename_retrying(&kept, &back)
    }
}

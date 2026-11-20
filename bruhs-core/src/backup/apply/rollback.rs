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
//! Something that took a name mid-apply (the capture daemon making
//! `audio/`, say) isn't thrown away: it moves into the pre-restore set's
//! `.in-the-way/`, and that set stays, listed, with no keychain items.

use bruhs_models::backup::RESTORE_ROLLED_BACK;
use bruhs_models::EpochMs;

use super::{
    exists, full_sync, in_the_way, kept_account, live, make_root, new_dir, real_dir,
    remove_if_there, remove_tree, rename_retrying, source_dir, stopped, to_json, top_names,
    write_atomically, Apply, ApplyOutcome, Journal, Point, ResultFile, Step, KEYCHAIN_NEEDS_YOU,
    META, PENDING, RESULT, SEALED,
};
use crate::backup::{BackupError, RootKey};
use crate::secrets::forget_seal_key;

/// Where an entry that took a name mid-apply waits in the pre-restore set.
const IN_THE_WAY: &str = ".in-the-way";

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
        // The status shows the rollback, and the service still starts again.
        let result = ResultFile {
            applied_at_ms: EpochMs::now(),
            stamp: if held {
                self.journal.stamp.clone()
            } else {
                String::new()
            },
            warnings: vec![RESTORE_ROLLED_BACK.to_owned()],
            service_to_restart: self.journal.service_to_restart,
            web_part: None,
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
        rename_retrying(&placed, &back)
    }

    /// Puts today's entry back into the root. Nothing to do when the kept
    /// set no longer holds it. Whatever took its name meanwhile moves into
    /// the kept set's `.in-the-way/`.
    fn unmove(&self, root: RootKey, name: &str) -> Result<(), BackupError> {
        let kept = self.kept(root).join(name);
        if !exists(&kept)? {
            return Ok(());
        }
        let back = live(self.roots, root, name);
        if exists(&back)? {
            let aside = self.kept(root).join(IN_THE_WAY);
            new_dir(&aside)?;
            let parked = aside.join(name);
            if exists(&parked)? {
                return Err(in_the_way());
            }
            rename_retrying(&back, &parked)?;
        }
        if let Some(parent) = back.parent() {
            make_root(parent)?;
        }
        rename_retrying(&kept, &back)
    }
}

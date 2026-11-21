//! The apply's journal (D16): `<core>/backups/apply.json`, the plan of one
//! restore and the steps done so far.
//!
//! The plan is fixed before the first step: which top-level names move
//! aside and which come in, root by root. A resumed apply follows it and
//! never lists the roots again, since by then they hold restored entries.
//! Each step is recorded after it is done, so every step is written to be
//! safe to repeat: a rename whose source is gone and whose target is there
//! is done, and keychain writes and removals repeat harmlessly.
//!
//! The file is written to a partial name, synced and renamed into place,
//! then its folder is synced. It holds names and keychain accounts, never
//! a value.

use std::io::{self, Write};
use std::path::{Component, Path};

use bruhs_models::backup::{valid_id, valid_stamp, PathRewrite, RestoreSource, DAMAGED};
use bruhs_models::EpochMs;
use serde::{Deserialize, Serialize};

use super::apply::{movable, pass_roots};
use super::host::{new_private_file, random_id, read_small};
use super::{BackupError, RootKey, Roots};
use crate::secrets::ItemRef;

/// The journal's name inside `<core>/backups`.
pub(super) const JOURNAL: &str = "apply.json";
/// A journal names top-level entries and keychain accounts only.
const JOURNAL_MAX: u64 = 16 << 20;

/// One restore being applied.
#[derive(Serialize, Deserialize)]
pub(super) struct Journal {
    /// The pre-restore set today's data goes into.
    pub stamp: String,
    /// What comes in.
    pub source: RestoreSource,
    /// When the apply started: the pre-restore set's date.
    pub made_at_ms: EpochMs,
    /// Whether the background service starts again after the apply.
    pub service_to_restart: bool,
    /// The names each root moves aside and takes in. One pass when both
    /// roots are one folder (ruling F33).
    pub passes: Vec<Pass>,
    /// Whether the source holds keychain items to write.
    pub keychain: bool,
    /// The items the source writes, without values, so the pruning never
    /// removes one of them.
    pub written: Vec<ItemRef>,
    /// The items the source's catalogs name, read when the apply was
    /// planned (final review B-I2): the pruning keeps them too, and never
    /// reads a catalog after the swap.
    pub restored: Vec<ItemRef>,
    /// Today's items the seal holds, recorded with [`Step::Sealed`]: the
    /// only items the pruning may remove.
    pub sealed: Vec<ItemRef>,
    /// Today's items the pruning couldn't remove, recorded with
    /// [`Step::KeysPruned`], for the result's warning.
    pub items_stayed: u32,
    /// The id the source's browser part waits under, when it has one.
    pub web_part: Option<String>,
    /// The path rewrites the browser applies to its part.
    pub rewrites: Vec<PathRewrite>,
    /// The steps done, in order.
    pub steps: Vec<Step>,
}

/// One root's share of the plan.
#[derive(Serialize, Deserialize, Clone)]
pub(super) struct Pass {
    pub root: RootKey,
    /// Today's top-level entries that move into the pre-restore set.
    pub moves: Vec<String>,
    /// The source's top-level entries that move into the root.
    pub places: Vec<String>,
}

/// A step of the apply, recorded once it is done (D16's order).
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(tag = "step", rename_all = "snake_case")]
pub(super) enum Step {
    /// Today's keychain items are sealed into the pre-restore set.
    Sealed,
    /// Today's entry `name` is in the pre-restore set.
    Moved { root: RootKey, name: String },
    /// The source's entry `name` is in the root.
    Placed { root: RootKey, name: String },
    /// The source's items are in the keychain.
    KeysWritten,
    /// Today's items the restored catalogs don't name are removed.
    KeysPruned,
    /// The source's browser part waits in `backups/web-part`.
    WebPart,
    /// The result and the pre-restore set's info are written.
    Done,
    /// A rename failed for good before any keychain write: the renames are
    /// being undone, and a resume finishes undoing them (B Task 8 I1).
    RollingBack,
}

impl Journal {
    /// The journal in `<core>/backups` of `roots`, if one is there. One
    /// that isn't a small regular file, doesn't parse, names anything
    /// outside its plan or plans other roots than this computer's (one
    /// pass on one root, ruling F33) is `DAMAGED`: an apply never acts on
    /// it.
    pub(super) fn load(roots: &Roots) -> Result<Option<Self>, BackupError> {
        let backups = roots.backups(RootKey::Core);
        let path = backups.join(JOURNAL);
        match std::fs::symlink_metadata(&path) {
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        }
        let damaged = || BackupError::Format(DAMAGED);
        let text = read_small(&path, JOURNAL_MAX).ok_or_else(damaged)?;
        let journal: Self = serde_json::from_slice(&text).map_err(|_| damaged())?;
        let planned: Vec<RootKey> = journal.passes.iter().map(|pass| pass.root).collect();
        if journal.valid() && planned == pass_roots(roots) {
            Ok(Some(journal))
        } else {
            Err(damaged())
        }
    }

    /// Whether every name and id is one this build writes, every step
    /// belongs to the plan, and no root comes twice.
    fn valid(&self) -> bool {
        let source = match &self.source {
            RestoreSource::Staged { id } => valid_id(id),
            RestoreSource::PreRestore { stamp } => valid_stamp(stamp) && *stamp != self.stamp,
            RestoreSource::Other => false,
        };
        let distinct = match self.passes.as_slice() {
            [_] => true,
            [first, second] => first.root != second.root,
            _ => false,
        };
        let names = self.passes.iter().all(|pass| {
            pass.moves
                .iter()
                .chain(&pass.places)
                .all(|name| movable(name))
        });
        let planned = |root: &RootKey, name: &String, moved: bool| {
            self.passes.iter().any(|pass| {
                pass.root == *root
                    && if moved {
                        pass.moves.contains(name)
                    } else {
                        pass.places.contains(name)
                    }
            })
        };
        let steps = self.steps.iter().all(|step| match step {
            Step::Moved { root, name } => planned(root, name, true),
            Step::Placed { root, name } => planned(root, name, false),
            _ => true,
        });
        valid_stamp(&self.stamp)
            && source
            && distinct
            && names
            && steps
            && self.web_part.as_deref().is_none_or(valid_id)
    }

    /// Whether `step` is recorded.
    pub(super) fn done(&self, step: &Step) -> bool {
        self.steps.contains(step)
    }

    /// Writes the journal into `backups`, replacing the last one.
    pub(super) fn save(&self, backups: &Path) -> Result<(), BackupError> {
        let text = serde_json::to_vec(self).map_err(io::Error::other)?;
        write_atomically(backups, JOURNAL, &text)
    }

    /// Records `step` as done.
    pub(super) fn record(&mut self, backups: &Path, step: Step) -> Result<(), BackupError> {
        self.steps.push(step);
        self.save(backups)
    }

    /// Removes the journal once the apply finished.
    pub(super) fn remove(backups: &Path) -> Result<(), BackupError> {
        match std::fs::remove_file(backups.join(JOURNAL)) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        sync_dir(backups)
    }
}

/// Whether `name` is one plain path component.
pub(super) fn single_component(name: &str) -> bool {
    let mut parts = Path::new(name).components();
    !name.contains(['/', '\0'])
        && matches!(parts.next(), Some(Component::Normal(_)))
        && parts.next().is_none()
}

/// Replaces `folder/name` with `bytes` atomically: a new 0600 partial file
/// (never following a link), synced, renamed into place, then the folder
/// synced. A partial left by a failure is removed.
pub(super) fn write_atomically(folder: &Path, name: &str, bytes: &[u8]) -> Result<(), BackupError> {
    let partial = folder.join(format!(".{name}.partial-{}", random_id()));
    let written = new_private_file(&partial).and_then(|mut file| {
        file.write_all(bytes)?;
        file.sync_all()
    });
    let renamed = written.and_then(|()| std::fs::rename(&partial, folder.join(name)));
    if let Err(error) = renamed {
        let _ = std::fs::remove_file(&partial);
        return Err(error.into());
    }
    sync_dir(folder)
}

/// fsyncs a folder, opened without following a link, so the names made or
/// removed in it last. A volume that can't sync folders (`EINVAL`) is left
/// as it is. On macOS this reaches the drive's cache only at the next full
/// sync ([`full_sync`]); each journal record is one, since `sync_all` is
/// `F_FULLFSYNC` there.
pub(super) fn sync_dir(folder: &Path) -> Result<(), BackupError> {
    match rustix::fs::fsync(open_dir(folder)?) {
        Ok(()) | Err(rustix::io::Errno::INVAL) => Ok(()),
        Err(error) => Err(io::Error::from(error).into()),
    }
}

/// A full sync through `folder` (M5): `F_FULLFSYNC` on macOS, which also
/// flushes the drive's cache and so makes every earlier sync durable; a
/// plain fsync elsewhere, where it already reaches the drive.
pub(super) fn full_sync(folder: &Path) -> Result<(), BackupError> {
    let opened = open_dir(folder)?;
    #[cfg(target_os = "macos")]
    let synced = rustix::fs::fcntl_fullfsync(&opened);
    #[cfg(not(target_os = "macos"))]
    let synced = rustix::fs::fsync(&opened);
    match synced {
        Ok(()) | Err(rustix::io::Errno::INVAL | rustix::io::Errno::NOTSUP) => sync_dir(folder),
        Err(error) => Err(io::Error::from(error).into()),
    }
}

fn open_dir(folder: &Path) -> Result<std::os::fd::OwnedFd, BackupError> {
    Ok(rustix::fs::open(
        folder,
        rustix::fs::OFlags::RDONLY
            | rustix::fs::OFlags::DIRECTORY
            | rustix::fs::OFlags::NOFOLLOW
            | rustix::fs::OFlags::CLOEXEC,
        rustix::fs::Mode::empty(),
    )
    .map_err(io::Error::from)?)
}

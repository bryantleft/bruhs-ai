//! Backup and restore of everything Bruhs keeps on this computer (crew
//! project 6): the data roots, what a backup takes from them, consistent
//! SQLite copies, the free space a job needs, opening a backup into
//! staging folders before a restore, preparing the staged data for this
//! computer, and applying it offline with today's data kept aside.
//!
//! The container format lives in `bruhs-backup`, the wire types, rules and
//! copy in `bruhs_models::backup`. Nothing here is reachable from a bruh:
//! both roots' `backups` folders are private to every bruh run
//! (`secrets::private`).

use std::ffi::OsString;
use std::path::{Component, Path, PathBuf};

use bruhs_models::backup::{excluded, Excluded, RootsInfo};

// Backup runs on macOS and Linux only (D27).
#[cfg(unix)]
mod apply;
#[cfg(unix)]
mod collect;
#[cfg(unix)]
mod create;
#[cfg(unix)]
mod disk;
#[cfg(unix)]
mod host;
#[cfg(unix)]
mod journal;
#[cfg(unix)]
mod open;
#[cfg(unix)]
mod prepare;
#[cfg(all(unix, any(test, feature = "test-support")))]
pub mod test_support;

#[cfg(unix)]
pub use apply::{
    apply_pending, applying, check_restore, delete_pre_restore, desktop_running, finish_web_part,
    instance_lock_path, keep_previous_web_part, open_restore_log, pending, pre_restores,
    reactive_lock_path, restore_waits, result, schedule, take_instance_lock, take_service_restart,
    unschedule, waiting_web_part, waiting_web_part_file, ApplyOutcome, Holder,
};
#[cfg(all(test, unix))]
use collect::snapshot_sqlite_with;
#[cfg(unix)]
pub use collect::{inventory, open_source, snapshot_sqlite, Inventory, RootId, SourceFile};
#[cfg(unix)]
pub use create::{check_destination, CancelFlag, CreateRequest, Output};
#[cfg(unix)]
pub use disk::{free_bytes, FREE_MARGIN};
#[cfg(unix)]
pub use host::{BackupHost, JobHold, Upload, UploadKind};

#[cfg(test)]
mod apply_tests;
#[cfg(test)]
mod collect_tests;
#[cfg(test)]
mod create_tests;
#[cfg(test)]
mod open_tests;
#[cfg(test)]
mod prepare_tests;
#[cfg(test)]
mod reach_tests;

/// The app and organisation names `directories` resolves the roots from.
const QUALIFIER: &str = "dev";
const ORG: &str = "bruhs";
const APP: &str = "bruhs-ai";
/// The folder inside each root that holds backups, staging and pre-restore
/// sets. A backup never holds it.
const BACKUPS: &str = "backups";

/// The `BRUHS_*` path overrides that can move data out of the roots (D23,
/// ruling F10 b), in the order the overview names them.
const PATH_OVERRIDES: [&str; 10] = [
    "BRUHS_REACTIVE_DB",
    "BRUHS_MEETING_DB",
    "BRUHS_MAIL_DB",
    "BRUHS_DESKTOP_STATE_FILE",
    "BRUHS_ATTACHMENTS_DIR",
    "BRUHS_RICH_RESPONSE_DIR",
    "BRUHS_WORKBENCH_DIR",
    "BRUHS_DELIVERY_DIR",
    "BRUHS_WORKTREE_METER",
    "BRUHS_GENERATORS_DB",
];

/// Where this computer keeps Bruhs' data (D4, D11, ruling F10 a).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Roots {
    /// The owner's home folder.
    pub home: PathBuf,
    /// The server's and the core's data: `ProjectDirs::data_local_dir()`.
    pub core: PathBuf,
    /// The desktop app's data: `BaseDirs::data_dir()/bruhs-ai`. The same
    /// folder as `core` on Linux.
    pub desktop: PathBuf,
    /// `ProjectDirs::config_dir()`, whose [`CONFIG_ENTRIES`] (`calendar/`
    /// and `secret-sources.json`) a backup takes. The same folder as
    /// `core` on macOS; `~/.config/bruhs-ai` on Linux.
    ///
    /// [`CONFIG_ENTRIES`]: bruhs_models::backup::CONFIG_ENTRIES
    pub config: PathBuf,
}

/// One of the two data roots, as a backup's `core/` and `desktop/` name
/// them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RootKey {
    Core,
    Desktop,
}

impl Roots {
    /// This account's roots, or `None` when no home folder can be named.
    pub fn from_env() -> Option<Self> {
        let project = directories::ProjectDirs::from(QUALIFIER, ORG, APP)?;
        let base = directories::BaseDirs::new()?;
        Some(Self::new(
            base.home_dir().to_path_buf(),
            project.data_local_dir().to_path_buf(),
            base.data_dir().join(APP),
            project.config_dir().to_path_buf(),
        ))
    }

    /// Roots named by their path components, so a `HOME` with a trailing
    /// slash or an `XDG_*` path with `//` or `/./` reads as the plain path
    /// a manifest can hold (final re-review N-I1). A `..` stays, and a
    /// backup still refuses it.
    pub fn new(home: PathBuf, core: PathBuf, desktop: PathBuf, config: PathBuf) -> Self {
        let plain = |path: PathBuf| path.components().collect::<PathBuf>();
        Self {
            home: plain(home),
            core: plain(core),
            desktop: plain(desktop),
            config: plain(config),
        }
    }

    /// Whether the core and desktop roots are one folder (Linux): one path,
    /// or two paths that resolve to one folder, as the backup's walk tells
    /// them apart (final review A minor). Every step that splits the work
    /// by root asks this, so none of them passes over one folder twice.
    pub fn same(&self) -> bool {
        if self.core == self.desktop {
            return true;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt as _;
            match (
                std::fs::metadata(&self.core),
                std::fs::metadata(&self.desktop),
            ) {
                (Ok(core), Ok(desktop)) => {
                    core.dev() == desktop.dev() && core.ino() == desktop.ino()
                }
                _ => false,
            }
        }
        #[cfg(not(unix))]
        false
    }

    /// The home and roots as a backup's manifest names them. A path that
    /// isn't UTF-8 is named lossily. One folder is named once, by the core
    /// root's path, as a one-root manifest says.
    pub fn info(&self) -> RootsInfo {
        let text = |path: &Path| path.to_string_lossy().into_owned();
        RootsInfo {
            home: text(&self.home),
            core: text(&self.core),
            desktop: if self.same() {
                text(&self.core)
            } else {
                text(&self.desktop)
            },
        }
    }

    /// The folder `root` is.
    pub fn root(&self, root: RootKey) -> &Path {
        match root {
            RootKey::Core => &self.core,
            RootKey::Desktop => &self.desktop,
        }
    }

    /// `<root>/backups`.
    pub fn backups(&self, root: RootKey) -> PathBuf {
        self.root(root).join(BACKUPS)
    }

    /// Each `BRUHS_*` path override, read through `env`, whose data a
    /// backup doesn't hold (D23, ruling F10 b): one that points outside
    /// both roots, into a folder a backup leaves out, or that isn't a
    /// plain absolute path. An empty or unset override is ignored.
    pub fn outside_overrides(&self, env: impl Fn(&str) -> Option<OsString>) -> Vec<String> {
        PATH_OVERRIDES
            .iter()
            .filter(|name| {
                env(name)
                    .filter(|value| !value.is_empty())
                    .is_some_and(|value| !self.holds(Path::new(&value)))
            })
            .map(|name| (*name).to_owned())
            .collect()
    }

    /// Whether a backup holds `path`: a plain absolute path under one of
    /// the roots that the backup rules don't always leave out.
    fn holds(&self, path: &Path) -> bool {
        let plain = path.is_absolute()
            && path
                .components()
                .all(|part| matches!(part, Component::RootDir | Component::Normal(_)));
        plain
            && [&self.core, &self.desktop].into_iter().any(|root| {
                path.strip_prefix(root).is_ok_and(|rel| {
                    let rel = rel.to_string_lossy();
                    !rel.is_empty() && excluded(&rel, false) != Excluded::Always
                })
            })
    }
}

/// Why a backup or restore step stopped. Every message is fixed copy from
/// `bruhs_models::backup` (or, for [`BackupError::Space`], that copy with
/// a size filled in): never a path, a name from inside a backup or a
/// system error's text.
#[derive(Debug, thiserror::Error)]
pub enum BackupError {
    /// The request can't be done as asked.
    #[error("{0}")]
    Refused(&'static str),
    /// Not enough free disk space; the filled `NO_SPACE` copy.
    #[error("{0}")]
    Space(String),
    /// The keychain didn't answer, or refused.
    #[error("{0}")]
    Keychain(&'static str),
    /// Something else holds what this step needs.
    #[error("{0}")]
    Busy(&'static str),
    /// A file couldn't be read or written. The source may name a path, so
    /// the message never shows it and nothing logs it.
    #[error("Bruhs couldn't read or write its data.")]
    Io(#[from] std::io::Error),
    /// The backup itself is wrong: a bad header, password or entry.
    #[error("{0}")]
    Format(&'static str),
}

impl BackupError {
    /// The fixed copy the apps show.
    pub fn message(&self) -> String {
        self.to_string()
    }
}

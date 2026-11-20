//! What a backup takes from the roots (D4, rulings F9, F10, F15): a walk
//! that never follows a link, and consistent copies of Bruhs' own SQLite
//! databases.

use std::fs::{File, Metadata, OpenOptions};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::time::Duration;

use bruhs_models::backup::{
    excluded, safe_entry_path, BackupOptions, Excluded, CONTENT_FOLDERS, EXCLUDED_TOP,
};
use rusqlite::{Connection, OpenFlags};

use super::{BackupError, RootKey, Roots};

/// The first 16 bytes of every SQLite database file.
const SQLITE_MAGIC: &[u8; 16] = b"SQLite format 3\0";
/// The top-level folder that, on Linux, comes from the config root.
const CALENDAR: &str = "calendar";
/// How long a snapshot waits for a writer that holds its lock mid-commit.
const SNAPSHOT_WAIT: Duration = Duration::from_secs(5);

/// One file a backup takes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceFile {
    /// The root it goes under in the archive (`core/` or `desktop/`).
    pub root: RootKey,
    /// Its path below that root, `/`-separated. Always one
    /// `safe_entry_path` accepts once prefixed with the root's name.
    pub rel: String,
    /// Where it is on disk.
    pub path: PathBuf,
    /// Its size when it was listed.
    pub bytes: u64,
    /// A database of Bruhs' own, copied with [`snapshot_sqlite`] rather
    /// than byte for byte (ruling F15).
    pub sqlite: bool,
}

/// What a backup made with some options takes, and what each option adds.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Inventory {
    /// The files that go in, root by root, depth first, by name.
    pub files: Vec<SourceFile>,
    /// Links found where a file would go in. They are never followed or
    /// stored.
    pub skipped_links: u32,
    /// The bytes that go in whatever the options say.
    pub data_bytes: u64,
    /// The meeting audio files, whether or not they go in.
    pub audio_bytes: u64,
    /// The bruhs' browser profiles without their caches, whether or not
    /// they go in.
    pub browser_bytes: u64,
}

/// Lists what a backup made with `options` takes from `roots` (D4): the
/// core root, then the desktop root unless it is the same folder. On a
/// computer whose config root differs from the core root (Linux), the
/// core root's `calendar/` comes from the config root, where Bruhs keeps
/// it, and nothing else of the config root goes in (ruling F10 a).
///
/// Nothing is followed: a link is counted in `skipped_links` where its
/// path would go in, and a socket, FIFO or device is left out. A name the
/// restore would refuse (not UTF-8, or one `safe_entry_path` refuses)
/// fails the inventory with [`io::ErrorKind::InvalidData`], whose message
/// names no path: a backup that can't be restored is worse than none.
pub fn inventory(roots: &Roots, options: &BackupOptions) -> io::Result<Inventory> {
    let mut walk = Walk {
        options,
        inventory: Inventory::default(),
    };
    let mut core = top_entries(&roots.core)?;
    if roots.config != roots.core {
        core.retain(|(name, _)| name != CALENDAR);
        let calendar = roots.config.join(CALENDAR);
        if exists(&calendar)? {
            core.push((CALENDAR.to_owned(), calendar));
            core.sort_by(|a, b| a.0.cmp(&b.0));
        }
    }
    walk.entries(RootKey::Core, "", core)?;
    if !roots.same() {
        let desktop = top_entries(&roots.desktop)?;
        walk.entries(RootKey::Desktop, "", desktop)?;
    }
    Ok(walk.inventory)
}

struct Walk<'a> {
    options: &'a BackupOptions,
    inventory: Inventory,
}

impl Walk<'_> {
    /// Visits `entries`, the sorted children of the folder at `parent`.
    fn entries(
        &mut self,
        root: RootKey,
        parent: &str,
        entries: Vec<(String, PathBuf)>,
    ) -> io::Result<()> {
        for (name, path) in entries {
            if parent.is_empty()
                && EXCLUDED_TOP
                    .iter()
                    .any(|excluded| name.eq_ignore_ascii_case(excluded))
            {
                continue;
            }
            let metadata = match std::fs::symlink_metadata(&path) {
                Ok(metadata) => metadata,
                // Gone since the folder was read.
                Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
                Err(error) => return Err(error),
            };
            let rel = if parent.is_empty() {
                name
            } else {
                format!("{parent}/{name}")
            };
            self.entry(root, rel, path, &metadata)?;
        }
        Ok(())
    }

    fn entry(
        &mut self,
        root: RootKey,
        rel: String,
        path: PathBuf,
        metadata: &Metadata,
    ) -> io::Result<()> {
        let kind = metadata.file_type();
        let why = excluded(&rel, kind.is_dir());
        if why == Excluded::Always {
            return Ok(());
        }
        let goes_in = match why {
            Excluded::No => true,
            Excluded::Always => false,
            Excluded::Audio => self.options.audio,
            Excluded::Browser => self.options.browser_profiles,
        };
        if kind.is_symlink() {
            if goes_in {
                self.inventory.skipped_links = self.inventory.skipped_links.saturating_add(1);
            }
        } else if kind.is_dir() {
            // A folder an option leaves out is still walked for its size.
            let children = children(&path)?;
            self.entries(root, &rel, children)?;
        } else if kind.is_file() {
            let bytes = metadata.len();
            let total = match why {
                Excluded::Audio => &mut self.inventory.audio_bytes,
                Excluded::Browser => &mut self.inventory.browser_bytes,
                Excluded::No | Excluded::Always => &mut self.inventory.data_bytes,
            };
            *total = total.saturating_add(bytes);
            if goes_in {
                if safe_entry_path(&format!("core/{rel}")).is_err() {
                    return Err(refused_name());
                }
                let sqlite = snapshot_candidate(&rel) && has_sqlite_magic(&path)?;
                self.inventory.files.push(SourceFile {
                    root,
                    rel,
                    path,
                    bytes,
                    sqlite,
                });
            }
        }
        // Sockets, FIFOs and devices never go in.
        Ok(())
    }
}

/// The sorted children of a root, or none when the root doesn't exist
/// yet. The root itself may be a link the owner made; nothing below it is
/// followed.
fn top_entries(root: &Path) -> io::Result<Vec<(String, PathBuf)>> {
    match children(root) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Vec::new()),
        other => other,
    }
}

/// The children of `folder` with their names, sorted by name.
fn children(folder: &Path) -> io::Result<Vec<(String, PathBuf)>> {
    let mut children = Vec::new();
    for entry in std::fs::read_dir(folder)? {
        let entry = entry?;
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| refused_name())?;
        children.push((name, entry.path()));
    }
    children.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(children)
}

fn exists(path: &Path) -> io::Result<bool> {
    match std::fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

fn refused_name() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "a file name can't go in a backup",
    )
}

/// Whether a file at `rel` may be one of Bruhs' own databases (ruling
/// F15): not in a content folder, nor in a bruh's `work`, `browser` or
/// `browser-cache` folder, which hold the owner's own files and are
/// copied byte for byte.
fn snapshot_candidate(rel: &str) -> bool {
    let parts: Vec<&str> = rel.split('/').collect();
    let owner = match parts.as_slice() {
        [top, ..] if CONTENT_FOLDERS.contains(top) => true,
        ["server", "bruhs", _, folder, ..] => {
            matches!(*folder, "work" | "browser" | "browser-cache")
        }
        _ => false,
    };
    !owner
}

/// Whether the regular file at `path` starts with the SQLite header. It is
/// opened without following a link and without blocking on a FIFO that
/// took its place.
fn has_sqlite_magic(path: &Path) -> io::Result<bool> {
    let file = match open_no_follow(path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error),
    };
    if !file.metadata()?.is_file() {
        return Ok(false);
    }
    let mut head = Vec::with_capacity(SQLITE_MAGIC.len());
    file.take(16).read_to_end(&mut head)?;
    Ok(head == SQLITE_MAGIC)
}

fn open_no_follow(path: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    options.open(path)
}

/// Copies the SQLite database at `source` into the new file `into` with
/// `VACUUM INTO` and returns the copy's size. The copy is one consistent
/// snapshot, committed WAL frames included.
///
/// The live database is opened read-only (no link followed, schema code
/// untrusted), so the snapshot takes only a read transaction: a writer in
/// WAL mode keeps committing while it runs, and the live file is never
/// written. `into` is created first with mode 0600, never replacing a
/// file or following a link; on failure it is removed. Errors carry no
/// path (ruling F15).
pub fn snapshot_sqlite(source: &Path, into: &Path) -> Result<u64, BackupError> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    drop(options.open(into)?);
    match vacuum_into(source, into) {
        Ok(()) => Ok(std::fs::symlink_metadata(into)?.len()),
        Err(error) => {
            // Best effort: the copy is incomplete and in a 0600 file.
            let _ = std::fs::remove_file(into);
            Err(error)
        }
    }
}

fn vacuum_into(source: &Path, into: &Path) -> Result<(), BackupError> {
    // SQLite's `NOFOLLOW` refuses a link anywhere in the path (`/var` is
    // one on macOS), so both folders are resolved first; a link in the
    // final name is still refused.
    let source = resolved_folder(source)?;
    let into = resolved_folder(into)?;
    let target = into
        .to_str()
        .ok_or_else(|| snapshot_failed("the snapshot path isn't UTF-8"))?;
    let live = Connection::open_with_flags(
        &source,
        OpenFlags::SQLITE_OPEN_READ_ONLY
            | OpenFlags::SQLITE_OPEN_NO_MUTEX
            | OpenFlags::SQLITE_OPEN_NOFOLLOW,
    )
    .map_err(sqlite_failed)?;
    live.busy_timeout(SNAPSHOT_WAIT).map_err(sqlite_failed)?;
    live.pragma_update(None, "trusted_schema", false)
        .map_err(sqlite_failed)?;
    live.execute("VACUUM INTO ?1", [target])
        .map_err(sqlite_failed)?;
    live.close().map_err(|(_, error)| sqlite_failed(error))
}

/// `path` with its folder resolved and its final name kept as it is.
fn resolved_folder(path: &Path) -> Result<PathBuf, BackupError> {
    let (Some(folder), Some(name)) = (path.parent(), path.file_name()) else {
        return Err(snapshot_failed("a snapshot path has no folder"));
    };
    let folder = if folder.as_os_str().is_empty() {
        Path::new(".")
    } else {
        folder
    };
    Ok(std::fs::canonicalize(folder)?.join(name))
}

/// A SQLite failure as fixed text: its result codes, never its message,
/// which can name a file.
fn sqlite_failed(error: rusqlite::Error) -> BackupError {
    match error {
        rusqlite::Error::SqliteFailure(failure, _) => snapshot_failed(&format!(
            "a SQLite snapshot failed: {:?} ({})",
            failure.code, failure.extended_code
        )),
        _ => snapshot_failed("a SQLite snapshot failed"),
    }
}

fn snapshot_failed(why: &str) -> BackupError {
    BackupError::Io(io::Error::other(why.to_owned()))
}

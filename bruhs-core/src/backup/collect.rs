//! What a backup takes from the roots (D4, rulings F9, F10, F15 and the
//! B Task 5 rulings): a walk through folder handles that never follows a
//! link, and consistent copies of Bruhs' own SQLite databases.
//!
//! Nothing here opens a database file with a plain descriptor: closing one
//! drops every POSIX lock this process holds on the file, including the
//! locks of the server's own connections. Databases are found and copied
//! through SQLite: when one of its connections closes a file that another
//! connection in this process still holds a lock on, its unix VFS keeps
//! the descriptor open until that lock goes (`setPendingFd`), so no lock is
//! dropped.

use std::collections::HashSet;
use std::ffi::{CStr, CString};
use std::fs::{File, OpenOptions};
use std::io;
use std::os::fd::OwnedFd;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use bruhs_models::backup::{
    excluded, folded_entry_key, safe_entry_path, BackupOptions, Excluded, CONTENT_FOLDERS,
    DATA_NAME_REFUSED, EXCLUDED_TOP, MAX_ENTRY_DEPTH,
};
use rusqlite::{Connection, OpenFlags};
use rustix::fs::{AtFlags, Dir, FileType, Mode, OFlags};
use rustix::io::Errno;

use super::{BackupError, RootKey, Roots};

/// The top-level folder that, on Linux, comes from the config root.
const CALENDAR: &str = "calendar";
/// How long a snapshot or a check waits for a writer that holds its lock
/// mid-commit.
const SQLITE_WAIT: Duration = Duration::from_secs(5);
/// SQLite's extended result code for a link in a path it was told not to
/// follow (`SQLITE_CANTOPEN_SYMLINK`).
const CANTOPEN_SYMLINK: i32 = 1550;

/// One file a backup takes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceFile {
    /// The root it goes under in the archive (`core/` or `desktop/`).
    pub root: RootKey,
    /// Its path below that root, `/`-separated, one `safe_entry_path`
    /// accepts under the root's own prefix.
    pub rel: String,
    /// `base` joined with `rel`. It names no link unless one was swapped
    /// in after the walk; open it with [`open_source`], never by path.
    pub path: PathBuf,
    /// The resolved folder `rel` is relative to: the root, or for the core
    /// root's `calendar/` on Linux, the config root.
    pub base: PathBuf,
    /// `base`'s identity when the walk opened it. [`open_source`] refuses
    /// a `base` that is another folder now (B Task 5 N3).
    pub base_id: RootId,
    /// Its size when it was listed.
    pub bytes: u64,
    /// A database of Bruhs' own, copied with [`snapshot_sqlite`] rather
    /// than byte for byte (ruling F15). Never read with a plain descriptor.
    pub sqlite: bool,
}

/// What a backup made with some options takes, and what each option adds.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Inventory {
    /// The files that go in, root by root, depth first, by name.
    pub files: Vec<SourceFile>,
    /// Links found where a file would go in, including an entry that
    /// became a link during the walk. They are never followed or stored.
    pub skipped_links: u32,
    /// Owner files and folders left out because a restore would refuse
    /// their names; a folder counts once for its whole tree.
    pub skipped_names: u32,
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
/// The walk goes through folder handles (`openat` with `O_NOFOLLOW`,
/// `fstatat` without following), so a link is never followed, even one
/// swapped in mid-walk; it is counted in `skipped_links` where its path
/// would go in. Sockets, FIFOs and devices are left out.
///
/// Names are judged where they would go in. One the restore would refuse
/// (not UTF-8, or one `safe_entry_path` refuses under the root's prefix,
/// including more than `MAX_ENTRY_DEPTH` levels, so the walk never holds
/// more than that many folder handles) is
/// left out and counted in `skipped_names` in the owner's own files: the
/// content folders and a bruh's `work` and `browser` folders. Anywhere else
/// it refuses the backup with [`DATA_NAME_REFUSED`]. The same goes for the
/// second of two names that differ only in case or Unicode normalization
/// (`Makefile` and `makefile`), which a restore refuses because one would
/// land on the other (B Task 7 I2); the walk takes names in byte order.
pub fn inventory(roots: &Roots, options: &BackupOptions) -> Result<Inventory, BackupError> {
    inventory_with(roots, options, &mut |_| {})
}

/// [`inventory`], calling `before_open` with each folder's `rel` between
/// reading its type and opening it, so a test can swap it for a link.
pub(super) fn inventory_with(
    roots: &Roots,
    options: &BackupOptions,
    before_open: &mut dyn FnMut(&str),
) -> Result<Inventory, BackupError> {
    let mut walk = Walk {
        options,
        inventory: Inventory::default(),
        before_open,
        names: Names::default(),
    };
    let core = opened_root(&roots.core)?;
    let desktop = opened_root(&roots.desktop)?;
    let config = if roots.config == roots.core {
        None
    } else {
        opened_root(&roots.config)?
    };

    if let Some(core) = &core {
        let mut top: Vec<(CString, &OpenedRoot)> = names(&core.fd)?
            .into_iter()
            .filter(|name| config.is_none() || name.as_bytes() != CALENDAR.as_bytes())
            .map(|name| (name, core))
            .collect();
        if let Some(config) = &config {
            let calendar = CString::new(CALENDAR).map_err(io::Error::other)?;
            match rustix::fs::statat(&config.fd, calendar.as_c_str(), AtFlags::SYMLINK_NOFOLLOW) {
                Ok(_) => top.push((calendar, config)),
                Err(Errno::NOENT) => {}
                Err(error) => return Err(io::Error::from(error).into()),
            }
            top.sort_by(|a, b| a.0.cmp(&b.0));
        }
        for (name, from) in &top {
            walk.entry(RootKey::Core, from, &from.fd, "", name)?;
        }
    }
    let separate = match (&core, &desktop) {
        (Some(core), Some(desktop)) => !roots.same() && core.base != desktop.base,
        _ => !roots.same(),
    };
    if let (true, Some(desktop)) = (separate, &desktop) {
        for name in names(&desktop.fd)? {
            walk.entry(RootKey::Desktop, desktop, &desktop.fd, "", &name)?;
        }
    }
    Ok(walk.inventory)
}

/// A root folder's identity (device and inode) when it was opened, so a
/// later step can tell that the folder at its path is still the same one
/// (B Task 5 N3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RootId {
    pub dev: u64,
    pub ino: u64,
}

impl RootId {
    /// The identity of the resolved folder `base` now, opened without
    /// following a link in its last name. Compare it with
    /// [`SourceFile::base_id`] before reading through `base` by path.
    pub fn of(base: &Path) -> io::Result<Self> {
        open_base(base).map(|(_, id)| id)
    }
}

/// Opens the resolved folder `base` (no link in its last name) and reads
/// its identity from the open handle.
fn open_base(base: &Path) -> io::Result<(OwnedFd, RootId)> {
    let fd = rustix::fs::open(
        base,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )?;
    let folder = File::from(fd);
    let metadata = folder.metadata()?;
    let id = RootId {
        dev: metadata.dev(),
        ino: metadata.ino(),
    };
    Ok((OwnedFd::from(folder), id))
}

/// A root folder, resolved and open.
struct OpenedRoot {
    base: PathBuf,
    fd: OwnedFd,
    id: RootId,
}

/// Opens a root, or `None` when it doesn't exist yet. The path to the root
/// may hold a link the owner made; it is resolved once, then the resolved
/// folder is opened without following a link and its identity recorded.
/// Nothing below it is followed.
fn opened_root(root: &Path) -> io::Result<Option<OpenedRoot>> {
    let base = match std::fs::canonicalize(root) {
        Ok(base) => base,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let (fd, id) = open_base(&base)?;
    Ok(Some(OpenedRoot { base, fd, id }))
}

/// The names in the folder `fd`, without `.` and `..`, sorted by bytes.
fn names(fd: &OwnedFd) -> io::Result<Vec<CString>> {
    let mut names = Vec::new();
    for entry in Dir::read_from(fd)? {
        let name = entry?.file_name().to_owned();
        if name.as_bytes() != b"." && name.as_bytes() != b".." {
            names.push(name);
        }
    }
    names.sort();
    Ok(names)
}

struct Walk<'a> {
    options: &'a BackupOptions,
    inventory: Inventory,
    before_open: &'a mut dyn FnMut(&str),
    names: Names,
}

/// What the walk does with a name that would go in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Verdict {
    Take,
    /// Left out and counted in `skipped_names`: owner content.
    Skip,
    /// The backup is refused with `DATA_NAME_REFUSED`: Bruhs' own folders.
    Refuse,
}

/// The folded key (`folded_entry_key`) of every path that goes in, root
/// by root.
#[derive(Default)]
pub(super) struct Names(HashSet<(RootKey, String)>);

impl Names {
    /// Takes `rel` (its last component `name`) when a restore would: a
    /// name `safe_entry_path` accepts that is the first in its root with
    /// its folded key, so a volume that ignores case or normalization
    /// keeps both (B Task 7 I2). Otherwise owner content skips it and
    /// Bruhs' own folders refuse the backup.
    pub(super) fn judge(&mut self, key: RootKey, rel: &str, name: &CStr) -> Verdict {
        if restorable(key, rel, name) && self.0.insert((key, folded_entry_key(rel))) {
            Verdict::Take
        } else if owner_content(rel) {
            Verdict::Skip
        } else {
            Verdict::Refuse
        }
    }
}

impl Walk<'_> {
    /// Visits `name` in the folder `parent`, which sits at `parent_rel` (a
    /// lossy text when the walk is only measuring) below `root.base`.
    fn entry(
        &mut self,
        key: RootKey,
        root: &OpenedRoot,
        parent: &OwnedFd,
        parent_rel: &str,
        name: &CStr,
    ) -> Result<(), BackupError> {
        let shown = String::from_utf8_lossy(name.to_bytes());
        if parent_rel.is_empty()
            && EXCLUDED_TOP
                .iter()
                .any(|excluded| shown.eq_ignore_ascii_case(excluded))
        {
            return Ok(());
        }
        let rel = if parent_rel.is_empty() {
            shown.into_owned()
        } else {
            format!("{parent_rel}/{shown}")
        };
        let stat = match rustix::fs::statat(parent, name, AtFlags::SYMLINK_NOFOLLOW) {
            Ok(stat) => stat,
            // Gone since the folder was read.
            Err(Errno::NOENT) => return Ok(()),
            Err(error) => return Err(io::Error::from(error).into()),
        };
        let kind = FileType::from_raw_mode(stat.st_mode);
        let why = excluded(&rel, kind == FileType::Directory);
        let goes_in = match why {
            Excluded::No => true,
            Excluded::Always => return Ok(()),
            Excluded::Audio => self.options.audio,
            Excluded::Browser => self.options.browser_profiles,
        };
        // The walk holds a folder handle a level: a measuring walk stops
        // where a restore would refuse the depth (B Task 5 N1).
        if !goes_in && rel.split('/').count() > MAX_ENTRY_DEPTH {
            return Ok(());
        }
        match kind {
            FileType::Symlink => {
                self.skipped_link(goes_in);
                return Ok(());
            }
            FileType::Directory | FileType::RegularFile => {}
            // Sockets, FIFOs and devices never go in.
            _ => return Ok(()),
        }
        if goes_in {
            match self.names.judge(key, &rel, name) {
                Verdict::Take => {}
                Verdict::Skip => {
                    self.inventory.skipped_names = self.inventory.skipped_names.saturating_add(1);
                    return Ok(());
                }
                Verdict::Refuse => return Err(BackupError::Refused(DATA_NAME_REFUSED)),
            }
        }
        if kind == FileType::Directory {
            (self.before_open)(&rel);
            let folder = match rustix::fs::openat(
                parent,
                name,
                OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
            ) {
                Ok(folder) => folder,
                Err(Errno::NOENT) => return Ok(()),
                // It became a link (or a file) since it was read.
                Err(Errno::LOOP | Errno::NOTDIR) => {
                    self.skipped_link(goes_in);
                    return Ok(());
                }
                Err(error) => return Err(io::Error::from(error).into()),
            };
            // A folder an option leaves out is still walked for its size.
            for child in names(&folder)? {
                self.entry(key, root, &folder, &rel, &child)?;
            }
            return Ok(());
        }

        let bytes = u64::try_from(stat.st_size).map_err(io::Error::other)?;
        let path = root.base.join(&rel);
        let mut sqlite = false;
        if goes_in && bytes > 0 && snapshot_candidate(&rel) {
            match found(&path) {
                Found::Database => sqlite = true,
                Found::Other => {}
                Found::Link => {
                    self.skipped_link(true);
                    return Ok(());
                }
            }
        }
        let total = match why {
            Excluded::Audio => &mut self.inventory.audio_bytes,
            Excluded::Browser => &mut self.inventory.browser_bytes,
            Excluded::No | Excluded::Always => &mut self.inventory.data_bytes,
        };
        *total = total.saturating_add(bytes);
        if goes_in {
            self.inventory.files.push(SourceFile {
                root: key,
                rel,
                path,
                base: root.base.clone(),
                base_id: root.id,
                bytes,
                sqlite,
            });
        }
        Ok(())
    }

    fn skipped_link(&mut self, goes_in: bool) {
        if goes_in {
            self.inventory.skipped_links = self.inventory.skipped_links.saturating_add(1);
        }
    }
}

/// Whether a restore accepts `rel` under `key`'s prefix, `name` (its last
/// component) included: UTF-8 and every `safe_entry_path` rule, its length
/// limits measured with the real prefix.
pub(super) fn restorable(key: RootKey, rel: &str, name: &CStr) -> bool {
    let prefix = match key {
        RootKey::Core => "core",
        RootKey::Desktop => "desktop",
    };
    name.to_str().is_ok() && safe_entry_path(&format!("{prefix}/{rel}")).is_ok()
}

/// Whether `rel` sits in the owner's own files: a content folder, or a
/// bruh's `work` folder or `browser` profile (whose names Chromium picks).
fn owner_content(rel: &str) -> bool {
    match rel.split('/').collect::<Vec<_>>().as_slice() {
        [top, ..] if CONTENT_FOLDERS.contains(top) => true,
        ["server", "bruhs", _, folder, ..] => matches!(*folder, "work" | "browser"),
        _ => false,
    }
}

/// Whether a file at `rel` may be one of Bruhs' own databases (ruling
/// F15): not in the owner's files nor a bruh's `browser-cache`, all of
/// which are copied byte for byte.
fn snapshot_candidate(rel: &str) -> bool {
    !owner_content(rel)
        && !matches!(
            rel.split('/').collect::<Vec<_>>().as_slice(),
            ["server", "bruhs", _, "browser-cache", ..]
        )
}

/// What SQLite makes of a file.
enum Found {
    /// A database, or a file SQLite can't yet tell apart from one (busy,
    /// locked, a hot journal): never read raw.
    Database,
    /// Not a database.
    Other,
    /// A link took the file's place.
    Link,
}

/// Asks SQLite whether the file at `path` is a database (B Task 5 C1):
/// read-only, no link anywhere in the path, then `PRAGMA schema_version`.
/// Closing this connection drops no lock another connection in this
/// process holds on the file: SQLite keeps its descriptor open until those
/// locks go. Its no-follow check is by path, done before the open, and the
/// open doesn't use `O_NONBLOCK` (recorded limits, N2).
fn found(path: &Path) -> Found {
    let checked = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY
            | OpenFlags::SQLITE_OPEN_NO_MUTEX
            | OpenFlags::SQLITE_OPEN_NOFOLLOW,
    )
    .and_then(|connection| {
        connection.busy_timeout(SQLITE_WAIT)?;
        connection.query_row("PRAGMA schema_version", [], |row| row.get::<_, i64>(0))
    });
    match checked {
        Ok(_) => Found::Database,
        Err(rusqlite::Error::SqliteFailure(failure, _))
            if failure.code == rusqlite::ErrorCode::NotADatabase =>
        {
            Found::Other
        }
        Err(rusqlite::Error::SqliteFailure(failure, _))
            if failure.extended_code == CANTOPEN_SYMLINK =>
        {
            Found::Link
        }
        Err(_) => Found::Database,
    }
}

/// Opens a listed file for reading through folder handles from its `base`,
/// following no link on the way (B Task 5 I1 ruling): a folder or the file
/// swapped for a link since the walk fails with `ELOOP` or `ENOTDIR`, a
/// `base` that is another folder than the walk's, or anything but a
/// regular file, fails with `InvalidData`. A [`SourceFile::sqlite`] file is
/// refused with `InvalidInput`: a plain descriptor on a live database
/// drops this process's locks on it when it closes (C1, N4).
pub fn open_source(file: &SourceFile) -> io::Result<File> {
    if file.sqlite {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "a database is copied through SQLite, never opened directly",
        ));
    }
    let (mut folder, id) = open_base(&file.base)?;
    if id != file.base_id {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "a data root changed since the walk",
        ));
    }
    let mut parts = file.rel.split('/').peekable();
    while let Some(part) = parts.next() {
        if parts.peek().is_none() {
            let opened = rustix::fs::openat(
                &folder,
                part,
                OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
                Mode::empty(),
            )?;
            if FileType::from_raw_mode(rustix::fs::fstat(&opened)?.st_mode) != FileType::RegularFile
            {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "a listed file isn't a regular file anymore",
                ));
            }
            return Ok(File::from(opened));
        }
        folder = rustix::fs::openat(
            &folder,
            part,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )?;
    }
    Err(io::Error::new(
        io::ErrorKind::InvalidInput,
        "a listed file has no name",
    ))
}

/// Copies the SQLite database at `source` into the new file `into` and
/// returns the copy's size. The copy is one consistent snapshot,
/// committed WAL frames included.
///
/// The live database is opened read-only through SQLite, with no link
/// anywhere in `source` (pass a [`SourceFile::path`], which is resolved)
/// and schema code untrusted, and is never written. A WAL database is
/// copied with `VACUUM INTO`, whose one read transaction never stops a
/// writer. A rollback-mode database (`server/mail.sqlite3`,
/// `database-connections.sqlite3`) is copied with the online backup API a
/// few pages a step, so a writer waits at most one step for its lock
/// (B Task 5 I3); a write between steps starts the copy again.
/// `into` is created first with mode 0600, never replacing a file or
/// following a link, and its folder is resolved; on failure it is
/// removed. Errors carry no path (ruling F15).
pub fn snapshot_sqlite(source: &Path, into: &Path) -> Result<u64, BackupError> {
    snapshot_sqlite_with(source, into, &mut || {})
}

/// [`snapshot_sqlite`], calling `between_steps` after each step of a
/// stepped copy, so a test can write to the source mid-copy.
pub(super) fn snapshot_sqlite_with(
    source: &Path,
    into: &Path,
    between_steps: &mut dyn FnMut(),
) -> Result<u64, BackupError> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    drop(options.open(into)?);
    match copy_database(source, into, between_steps) {
        Ok(()) => Ok(std::fs::symlink_metadata(into)?.len()),
        Err(error) => {
            // Best effort: the copy is incomplete and in a 0600 file.
            let _ = std::fs::remove_file(into);
            Err(error)
        }
    }
}

fn copy_database(
    source: &Path,
    into: &Path,
    between_steps: &mut dyn FnMut(),
) -> Result<(), BackupError> {
    // SQLite's `NOFOLLOW` refuses a link anywhere in a path (`/var` is one
    // on macOS), so the target's folder, which Bruhs made, is resolved.
    let into = resolved_folder(into)?;
    let target = into
        .to_str()
        .ok_or_else(|| snapshot_failed("the snapshot path isn't UTF-8"))?;
    let live = Connection::open_with_flags(
        source,
        OpenFlags::SQLITE_OPEN_READ_ONLY
            | OpenFlags::SQLITE_OPEN_NO_MUTEX
            | OpenFlags::SQLITE_OPEN_NOFOLLOW,
    )
    .map_err(sqlite_failed)?;
    live.busy_timeout(SQLITE_WAIT).map_err(sqlite_failed)?;
    live.pragma_update(None, "trusted_schema", false)
        .map_err(sqlite_failed)?;
    let mode: String = live
        .query_row("PRAGMA journal_mode", [], |row| row.get(0))
        .map_err(sqlite_failed)?;
    if mode.eq_ignore_ascii_case("wal") {
        live.execute("VACUUM INTO ?1", [target])
            .map_err(sqlite_failed)?;
    } else {
        backup_in_steps(&live, &into, between_steps)?;
    }
    live.close().map_err(|(_, error)| sqlite_failed(error))
}

/// Pages one online backup step copies; SQLite's default page is 4 KiB.
const BACKUP_STEP_PAGES: i32 = 64;
/// The pause between two steps, when a writer may take its lock.
const BACKUP_STEP_PAUSE: Duration = Duration::from_millis(2);
/// How long a backup may keep starting again under a busy writer.
const BACKUP_DEADLINE: Duration = Duration::from_secs(120);

/// Copies `live` into the empty file `into` with SQLite's online backup
/// API, a few pages at a time. The source's lock is held only during a
/// step. A write by another connection between steps starts the copy
/// again, so a database written more often than one full pass takes is
/// refused at the deadline with fixed copy.
fn backup_in_steps(
    live: &Connection,
    into: &Path,
    between_steps: &mut dyn FnMut(),
) -> Result<(), BackupError> {
    use rusqlite::backup::{Backup, StepResult};
    let mut copy = Connection::open_with_flags(
        into,
        OpenFlags::SQLITE_OPEN_READ_WRITE
            | OpenFlags::SQLITE_OPEN_NO_MUTEX
            | OpenFlags::SQLITE_OPEN_NOFOLLOW,
    )
    .map_err(sqlite_failed)?;
    {
        let backup = Backup::new(live, &mut copy).map_err(sqlite_failed)?;
        let started = std::time::Instant::now();
        loop {
            match backup.step(BACKUP_STEP_PAGES).map_err(sqlite_failed)? {
                StepResult::Done => break,
                StepResult::More | StepResult::Busy | StepResult::Locked => {}
                _ => return Err(snapshot_failed("a SQLite backup step failed")),
            }
            if started.elapsed() > BACKUP_DEADLINE {
                return Err(snapshot_failed("a SQLite backup never caught up"));
            }
            between_steps();
            std::thread::sleep(BACKUP_STEP_PAUSE);
        }
    }
    copy.close().map_err(|(_, error)| sqlite_failed(error))
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

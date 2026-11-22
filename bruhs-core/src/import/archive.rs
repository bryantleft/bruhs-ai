//! Opening an export within fixed budgets: the uploaded or picked files,
//! the ZIP entries inside them, and the files of a picked folder.
//!
//! Every input is hostile until read:
//! - A file is classified by its first bytes (D4): `PK\x03\x04` is a
//!   ZIP, `[` or `{` after an optional byte order mark and whitespace is
//!   bare JSON, anything else adds nothing.
//! - Inside a ZIP or a folder only [`entry_kind`]'s allow-list is opened.
//!   An entry name is only matched, never joined to a path, and nothing
//!   is extracted by name.
//! - Before `ZipArchive::new` parses a central directory, its end
//!   records (Zip64 included) are checked against the entry and size
//!   caps (F10).
//! - Every byte read counts against the entry's budget and the read's
//!   (`Capped`), on the bytes actually read, never the sizes an archive
//!   declares.
//! - A nested ZIP is copied, still counted, to the caller's scratch
//!   folder under a counter name, read at most `nesting` levels deep,
//!   and deleted.
//! - A folder is walked through folder handles, never following a link.
//!
//! A refusal's copy travels through a shared slot (F11), checked before
//! any other error.

use super::json_stream::{refuse, Refusal};
use super::ImportError;
use bruhs_models::imports::{
    self as caps, ARCHIVE_REFUSED, DAMAGED_EXPORT, ENCRYPTED_EXPORT, NOT_AN_EXPORT,
};
use std::cell::Cell;
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use zip::ZipArchive;

/// The caps one read keeps to (D5), as numbers the tests can lower.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    /// Bytes uploaded or picked in total.
    pub upload_bytes: u64,
    /// Files uploaded or picked.
    pub upload_files: usize,
    /// Entries one ZIP, or one picked folder, may hold.
    pub entries: usize,
    /// Levels of ZIP read inside an uploaded or picked one.
    pub nesting: u8,
    /// An entry may inflate to this many times its compressed size...
    pub ratio: u64,
    /// ...or to this many bytes, whichever is more...
    pub ratio_floor: u64,
    /// ...and never past this many.
    pub json_entry: u64,
    /// Bytes one read inflates in total, nested copies included.
    pub inflated: u64,
    /// Bytes of a file read whole: a manifest, a memory file or a bare
    /// object.
    pub small_json: u64,
    /// Bytes of one conversation record.
    pub record: u64,
    /// Values (counted at each `[`, `{` and `,`) one record may hold, and
    /// a file read whole: what bounds the memory of the value built from
    /// it.
    pub values: u64,
    /// Conversations one read stores.
    pub conversations: usize,
    /// Bytes of normalized threads one batch stores.
    pub stored: u64,
    /// Folder levels walked under a picked folder.
    pub folder_depth: usize,
}

impl Limits {
    /// The shipped caps, from `bruhs_models::imports`.
    pub const DEFAULT: Self = Self {
        upload_bytes: caps::MAX_UPLOAD_BYTES,
        upload_files: caps::MAX_UPLOAD_FILES,
        entries: caps::MAX_ARCHIVE_ENTRIES,
        nesting: levels(caps::MAX_ZIP_NESTING),
        ratio: caps::MAX_INFLATE_RATIO,
        ratio_floor: caps::RATIO_FLOOR_BYTES,
        json_entry: caps::MAX_JSON_ENTRY_BYTES,
        inflated: caps::MAX_INFLATED_BYTES,
        small_json: caps::MAX_SMALL_JSON_BYTES,
        record: caps::MAX_RECORD_BYTES,
        values: MAX_RECORD_VALUES,
        conversations: caps::MAX_CONVERSATIONS,
        stored: caps::MAX_STORED_BYTES,
        folder_depth: caps::MAX_FOLDER_DEPTH,
    };

    /// Caps small enough for a test to pass them quickly.
    #[cfg(any(test, feature = "test-support"))]
    pub const TEST: Self = Self {
        ratio_floor: 1 << 20,
        json_entry: 8 << 20,
        inflated: 32 << 20,
        record: 1 << 20,
        values: 200_000,
        entries: 1_000,
        upload_bytes: 16 << 20,
        stored: 8 << 20,
        ..Self::DEFAULT
    };

    /// The bytes one entry may give: `compressed × ratio`, at least
    /// `ratio_floor` and at most `json_entry`.
    fn entry_budget(&self, compressed: u64) -> u64 {
        compressed
            .saturating_mul(self.ratio)
            .max(self.ratio_floor)
            .min(self.json_entry)
    }
}

/// Values one record may hold: about 128 MiB of `serde_json::Value` on
/// the server (I Task 3 I1).
const MAX_RECORD_VALUES: u64 = 4 << 20;

/// `n` as a `u8`, in a constant (a cast could truncate).
const fn levels(n: usize) -> u8 {
    let mut out: u8 = 0;
    let mut left = n;
    while left > 0 {
        out = match out.checked_add(1) {
            Some(next) => next,
            None => panic!("MAX_ZIP_NESTING fits a u8"),
        };
        left -= 1;
    }
    out
}

/// One file or folder of an export: an uploaded part, or a path the
/// desktop picked.
#[derive(Debug, Clone)]
pub(crate) enum ExportInput {
    File(PathBuf),
    Folder(PathBuf),
}

/// What an opened entry holds, by its name (D4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EntryKind {
    /// ChatGPT's `conversations.json`, or one numbered shard of it.
    Conversations { shard: Option<u32> },
    /// Grok's `prod-grok-backend.json`.
    GrokBackend,
    /// `export_manifest.json` or `manifest-*.json`.
    Manifest,
    /// `memories.json`, or a `.json` directly under `memories/`.
    Memory,
    /// A nested ZIP.
    Zip,
    /// An uploaded or picked JSON file, classified by structure alone.
    Bare,
}

/// What a walk read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct WalkReport {
    /// Entries and files handed to the visitor.
    pub files_read: u32,
    /// Allowed entries left out: an unreadable folder or file, or a
    /// `.zip` that isn't a ZIP.
    pub skipped: u32,
}

/// Names never opened, in a ZIP, a folder or a picked path (D4, D10),
/// matched against each part of a name, ignoring ASCII case.
const NEVER_OPEN: [&str; 11] = [
    "users.json",
    "user.json",
    "login_history.json",
    "user_settings.json",
    "credentials",
    "secrets",
    "state.db",
    "config.yaml",
    "openclaw.json",
    "clawdbot.json",
    "moltbot.json",
];

/// Whether one part of a name is never opened: [`NEVER_OPEN`],
/// `prod-mc-*`, `.env*`, `auth*.json`, `*.sqlite*`, and Windows' reserved
/// device names.
pub(crate) fn never_opened(part: &str) -> bool {
    let lower = part.to_ascii_lowercase();
    NEVER_OPEN.contains(&lower.as_str())
        || lower.starts_with("prod-mc-")
        || lower.starts_with(".env")
        || (lower.starts_with("auth") && lower.ends_with(".json"))
        || lower.contains(".sqlite")
        || is_device_name(&lower)
}

/// Windows' reserved device names (`CON`, `PRN`, `AUX`, `NUL`, `COM1` to
/// `COM9`, `LPT1` to `LPT9`), with or without an extension: opening one
/// opens the device. `lower` is already lower case.
fn is_device_name(lower: &str) -> bool {
    let stem = lower
        .split('.')
        .next()
        .unwrap_or(lower)
        .trim_end_matches([' ', ':']);
    match stem.as_bytes() {
        b"con" | b"prn" | b"aux" | b"nul" => true,
        [b'c', b'o', b'm', digit] | [b'l', b'p', b't', digit] => (b'1'..=b'9').contains(digit),
        _ => false,
    }
}

/// Whether a nested ZIP is never opened by its own name (M8): its stem
/// is never opened, or starts with `credentials` or `secrets`.
fn zip_never_opened(stem: &str) -> bool {
    never_opened(stem) || stem.starts_with("credentials") || stem.starts_with("secrets")
}

/// The allow-list (D4): what an entry named `name` inside a ZIP or a
/// folder holds, or `None` when it is never opened. `\` reads as `/`,
/// and base names match ignoring ASCII case.
pub(crate) fn entry_kind(name: &str) -> Option<EntryKind> {
    let name = name.replace('\\', "/");
    let mut parts: Vec<&str> = name.split('/').collect();
    let base = parts.pop()?;
    if base.is_empty() || never_opened(base) || parts.iter().any(|part| never_opened(part)) {
        return None;
    }
    let base = base.to_ascii_lowercase();
    match base.as_str() {
        "conversations.json" => return Some(EntryKind::Conversations { shard: None }),
        "prod-grok-backend.json" => return Some(EntryKind::GrokBackend),
        "export_manifest.json" => return Some(EntryKind::Manifest),
        "memories.json" => return Some(EntryKind::Memory),
        _ => {}
    }
    if let Some(digits) = base
        .strip_prefix("conversations-")
        .and_then(|rest| rest.strip_suffix(".json"))
    {
        let numbered =
            (1..=4).contains(&digits.len()) && digits.bytes().all(|b| b.is_ascii_digit());
        return numbered
            .then(|| digits.parse().ok())
            .flatten()
            .map(|n| EntryKind::Conversations { shard: Some(n) });
    }
    if base.starts_with("manifest-") && base.ends_with(".json") {
        return Some(EntryKind::Manifest);
    }
    let parent = parts.last().map(|part| part.to_ascii_lowercase());
    if base.ends_with(".json") && parent.as_deref() == Some("memories") {
        return Some(EntryKind::Memory);
    }
    if let Some(stem) = base.strip_suffix(".zip").filter(|stem| !stem.is_empty()) {
        return (!zip_never_opened(stem)).then_some(EntryKind::Zip);
    }
    None
}

/// What [`walk`] hands each opened entry to: its kind, a reader within
/// its budgets, and a bare file's length.
pub(crate) type Visit<'v> =
    dyn FnMut(EntryKind, &mut dyn Read, Option<u64>) -> Result<(), ImportError> + 'v;

/// What a walk tells its caller as it goes (F14).
pub(crate) struct Hooks<'h> {
    /// Called before each chunk a nested ZIP's copy writes to scratch,
    /// with its size: an error (the free space check) ends the walk.
    pub wrote: &'h dyn Fn(u64) -> Result<(), ImportError>,
    /// Called once each input is read, with how many are done.
    pub input_done: &'h dyn Fn(u32),
}

#[cfg(test)]
fn wrote_nothing(_: u64) -> Result<(), ImportError> {
    Ok(())
}

#[cfg(test)]
fn no_progress(_: u32) {}

#[cfg(test)]
impl Hooks<'static> {
    /// Hooks that check nothing and report nothing.
    pub(crate) const NONE: Self = Self {
        wrote: &wrote_nothing,
        input_done: &no_progress,
    };
}

/// Bytes a nested ZIP's copy writes between two [`Hooks::wrote`] calls.
const COPY_CHUNK: usize = 64 * 1024;

/// Nested ZIPs one read copies out, across every level.
const MAX_NESTED_ZIPS: u32 = 256;
/// Bytes of central directory a ZIP may have (F10).
const MAX_CENTRAL_DIRECTORY: u64 = 64 << 20;
/// Leading bytes searched for the first JSON byte of a bare file.
const SNIFF_BYTES: u64 = 64 * 1024;

/// Reads every input in order and hands each opened entry to `visit`:
/// its kind, a reader within its budgets, and a bare file's length.
/// `visit` reads it with `json_stream::for_each_record`.
///
/// `scratch` must exist; nested ZIP copies go there and are deleted
/// before this returns. `cancel` is checked between entries and on each
/// read.
///
/// # Errors
///
/// - [`ImportError::Gone`] once `cancel` is set.
/// - `Refused` with `ARCHIVE_REFUSED` past a budget, `ENCRYPTED_EXPORT`
///   for a password-protected entry, `DAMAGED_EXPORT` for a ZIP that
///   can't be read, and `NOT_AN_EXPORT` when nothing was opened.
/// - `visit`'s own error, unless a budget refused first.
/// - [`ImportError::Io`] when an input can't be opened.
#[cfg(test)]
pub(crate) fn walk(
    inputs: &[ExportInput],
    scratch: &Path,
    limits: &Limits,
    cancel: &AtomicBool,
    visit: &mut Visit<'_>,
) -> Result<WalkReport, ImportError> {
    walk_with(inputs, scratch, limits, cancel, &Hooks::NONE, visit)
}

/// [`walk`], telling `hooks` what a nested copy writes and when each input
/// is done.
///
/// # Errors
///
/// As [`walk`], and the `wrote` hook's own error.
pub(crate) fn walk_with(
    inputs: &[ExportInput],
    scratch: &Path,
    limits: &Limits,
    cancel: &AtomicBool,
    hooks: &Hooks<'_>,
    visit: &mut Visit<'_>,
) -> Result<WalkReport, ImportError> {
    let mut walker = Walker {
        scratch,
        limits,
        cancel,
        hooks,
        visit,
        refusal: Refusal::default(),
        inflated_left: Rc::new(Cell::new(limits.inflated)),
        walk_id: format!("{:016x}", rand::random::<u64>()),
        nested: 0,
        files_read: 0,
        skipped: Cell::new(0),
    };
    sweep_stale_copies(scratch);
    let result = inputs.iter().zip(1_u32..).try_for_each(|(input, done)| {
        walker.check_cancel()?;
        match input {
            ExportInput::File(path) => walker.file(path),
            ExportInput::Folder(path) => walker.folder(path),
        }?;
        (walker.hooks.input_done)(done);
        Ok(())
    });
    if let Err(error) = result {
        return Err(walker.settle(error));
    }
    // A visitor that swallowed a refusal still ends the read (M2).
    walker.check_cancel()?;
    if let Some(copy) = walker.refusal.get() {
        return Err(ImportError::Refused(copy));
    }
    if walker.files_read == 0 {
        return Err(ImportError::Refused(NOT_AN_EXPORT));
    }
    Ok(WalkReport {
        files_read: walker.files_read,
        skipped: walker.skipped.get(),
    })
}

struct Walker<'a> {
    scratch: &'a Path,
    limits: &'a Limits,
    cancel: &'a AtomicBool,
    hooks: &'a Hooks<'a>,
    visit: &'a mut Visit<'a>,
    refusal: Refusal,
    /// Bytes the read may still inflate.
    inflated_left: Rc<Cell<u64>>,
    /// Names this walk's scratch copies apart from any other's (M4).
    walk_id: String,
    nested: u32,
    files_read: u32,
    skipped: Cell<u32>,
}

fn damaged() -> ImportError {
    ImportError::Refused(DAMAGED_EXPORT)
}

fn to_u64(n: usize) -> u64 {
    u64::try_from(n).unwrap_or(u64::MAX)
}

impl<'a> Walker<'a> {
    /// Counts one allowed entry left out.
    fn skip(&self) {
        self.skipped.set(self.skipped.get().saturating_add(1));
    }

    fn check_cancel(&self) -> Result<(), ImportError> {
        if self.cancel.load(Ordering::SeqCst) {
            Err(ImportError::Gone)
        } else {
            Ok(())
        }
    }

    /// The error a failed walk answers: cancelling first, then a
    /// budget's refusal, then the error itself (F11).
    fn settle(&self, error: ImportError) -> ImportError {
        if self.cancel.load(Ordering::SeqCst) {
            ImportError::Gone
        } else if let Some(copy) = self.refusal.get() {
            ImportError::Refused(copy)
        } else {
            error
        }
    }

    fn capped<R: Read>(&self, inner: R, budget: u64) -> Capped<'a, R> {
        Capped {
            inner,
            left: budget,
            inflated_left: Rc::clone(&self.inflated_left),
            cancel: self.cancel,
            refusal: Rc::clone(&self.refusal),
        }
    }

    /// Hands one entry to the visitor within `budget` bytes.
    fn read_entry(
        &mut self,
        kind: EntryKind,
        inner: impl Read,
        budget: u64,
        len: Option<u64>,
    ) -> Result<(), ImportError> {
        let mut reader = self.capped(inner, budget);
        self.files_read = self.files_read.saturating_add(1);
        (self.visit)(kind, &mut reader, len)
    }

    /// An uploaded part or a picked file, classified by its first bytes.
    fn file(&mut self, path: &Path) -> Result<(), ImportError> {
        let Some(mut file) = open_input(path).map_err(|error| ImportError::io(path, error))? else {
            return Ok(());
        };
        match sniff(&mut file).map_err(|error| ImportError::io(path, error))? {
            Sniffed::Zip => self.zip(file, 0),
            Sniffed::Json(len) => {
                self.read_entry(EntryKind::Bare, file, self.limits.json_entry, Some(len))
            }
            Sniffed::Other => Ok(()),
        }
    }

    /// One ZIP, `depth` levels inside the uploaded or picked one.
    fn zip(&mut self, mut file: File, depth: u8) -> Result<(), ImportError> {
        let len = file.metadata().map_err(|_| damaged())?.len();
        check_end_records(&mut file, len, self.limits)?;
        file.seek(SeekFrom::Start(0)).map_err(|_| damaged())?;
        let mut archive = ZipArchive::new(file).map_err(|_| damaged())?;
        if archive.len() > self.limits.entries {
            return Err(ImportError::Refused(ARCHIVE_REFUSED));
        }
        if archive.has_duplicate_names() {
            return Err(damaged());
        }
        let mut picked = Vec::new();
        for index in 0..archive.len() {
            self.check_cancel()?;
            let kind = match archive.name_for_index(index) {
                Some(Ok(name)) => entry_kind(&name),
                _ => None,
            };
            let Some(kind) = kind else {
                continue;
            };
            let entry = archive.by_index_raw(index).map_err(|_| damaged())?;
            if entry.is_symlink() || !entry.is_file() {
                continue;
            }
            if entry.encrypted() {
                return Err(ImportError::Refused(ENCRYPTED_EXPORT));
            }
            // A declared size is only a multiplier, and never past the
            // archive's own length.
            picked.push(((index, entry.compressed_size().min(len)), kind));
        }
        for ((index, compressed), kind) in in_reading_order(picked) {
            self.check_cancel()?;
            let budget = self.limits.entry_budget(compressed);
            let entry = archive.by_index(index).map_err(|_| damaged())?;
            if kind == EntryKind::Zip {
                self.nested_zip(entry, budget, depth)?;
            } else {
                self.read_entry(kind, entry, budget, None)?;
            }
        }
        Ok(())
    }

    /// Copies a nested ZIP to scratch within `budget`, reads it one level
    /// deeper, then deletes the copy.
    fn nested_zip(&mut self, entry: impl Read, budget: u64, depth: u8) -> Result<(), ImportError> {
        let child = depth.saturating_add(1);
        if child > self.limits.nesting || self.nested >= MAX_NESTED_ZIPS {
            return Err(ImportError::Refused(ARCHIVE_REFUSED));
        }
        let mut reader = self.capped(entry, budget);
        let mut head = [0_u8; 4];
        let mut got = 0;
        while got < head.len() {
            match reader.read(&mut head[got..]).map_err(|_| damaged())? {
                0 => break,
                n => got += n,
            }
        }
        if !is_zip_magic(&head[..got]) {
            // Named .zip, but not one (M9).
            self.skip();
            return Ok(());
        }
        self.nested += 1;
        let path = self.scratch.join(format!(
            "{NESTED_PREFIX}{}-{}.zip",
            self.walk_id, self.nested
        ));
        let mut copy = ScratchCopy::create(path)?;
        let file = copy.file.as_mut().ok_or_else(damaged)?;
        (self.hooks.wrote)(to_u64(got))?;
        io::Write::write_all(file, &head[..got]).map_err(|_| damaged())?;
        let mut chunk = vec![0_u8; COPY_CHUNK];
        loop {
            let n = match reader.read(&mut chunk) {
                Ok(0) => break,
                Ok(n) => n,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(_) => return Err(damaged()),
            };
            (self.hooks.wrote)(to_u64(n))?;
            io::Write::write_all(file, &chunk[..n]).map_err(|_| damaged())?;
        }
        let file = file.try_clone().map_err(|_| damaged())?;
        // `copy` deletes the file once this level is read.
        self.zip(file, child)
    }

    /// A ZIP found in a picked folder, read in place as the top level. One
    /// that isn't a ZIP is left out and counted (M9).
    fn folder_zip(&mut self, mut file: File) -> Result<(), ImportError> {
        let mut head = Vec::new();
        let peeked = Read::by_ref(&mut file).take(4).read_to_end(&mut head);
        if peeked.is_err() || !is_zip_magic(&head) {
            self.skip();
            return Ok(());
        }
        self.zip(file, 0)
    }

    #[cfg(unix)]
    fn folder(&mut self, path: &Path) -> Result<(), ImportError> {
        use rustix::fs::{Mode, OFlags};
        let io_error = |error: io::Error| ImportError::io(path, error);
        let root = rustix::fs::open(
            path,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(|error| io_error(error.into()))?;
        let mut found = Vec::new();
        let mut seen = 0;
        self.list_folder(&root, &mut Vec::new(), 0, &mut found, &mut seen)
            .map_err(|error| match error {
                Listed::Stop(error) => error,
                Listed::Io(error) => io_error(error),
            })?;
        for (parts, kind) in in_reading_order(found) {
            self.check_cancel()?;
            let file = match folder_unix::open_under(&root, &parts) {
                Ok(Some(file)) => file,
                Ok(None) => continue,
                // Unreadable: left out and counted (M5).
                Err(_) => {
                    self.skip();
                    continue;
                }
            };
            if kind == EntryKind::Zip {
                self.folder_zip(file)?;
            } else {
                self.read_entry(kind, file, self.limits.json_entry, None)?;
            }
        }
        Ok(())
    }

    /// Lists `dir` and its folders down to `folder_depth`, through
    /// handles, never following a link, counting every entry.
    #[cfg(unix)]
    fn list_folder(
        &self,
        dir: &std::os::fd::OwnedFd,
        rel: &mut Vec<std::ffi::CString>,
        depth: usize,
        found: &mut Vec<(Vec<std::ffi::CString>, EntryKind)>,
        seen: &mut usize,
    ) -> Result<(), Listed> {
        use rustix::fs::{AtFlags, FileType, Mode, OFlags};
        use rustix::io::Errno;
        let names = match folder_unix::names(dir, seen, self.limits.entries) {
            Ok(names) => names,
            // An unreadable folder below the picked one is left out (M5).
            Err(Listed::Io(_)) if depth > 0 => {
                self.skip();
                return Ok(());
            }
            Err(error) => return Err(error),
        };
        for name in names {
            self.check_cancel().map_err(Listed::Stop)?;
            let shown = String::from_utf8_lossy(name.to_bytes()).into_owned();
            if never_opened(&shown) {
                continue;
            }
            let stat = match rustix::fs::statat(dir, name.as_c_str(), AtFlags::SYMLINK_NOFOLLOW) {
                Ok(stat) => stat,
                Err(Errno::NOENT) => continue,
                Err(_) => {
                    self.skip();
                    continue;
                }
            };
            match FileType::from_raw_mode(stat.st_mode) {
                FileType::Directory if depth < self.limits.folder_depth => {
                    let folder = match rustix::fs::openat(
                        dir,
                        name.as_c_str(),
                        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                        Mode::empty(),
                    ) {
                        Ok(folder) => folder,
                        // Gone, or a link now.
                        Err(Errno::NOENT | Errno::LOOP | Errno::NOTDIR) => continue,
                        Err(_) => {
                            self.skip();
                            continue;
                        }
                    };
                    rel.push(name);
                    let listed = self.list_folder(&folder, rel, depth + 1, found, seen);
                    rel.pop();
                    listed?;
                }
                FileType::RegularFile => {
                    let mut shown_path: Vec<String> = rel
                        .iter()
                        .map(|part| String::from_utf8_lossy(part.to_bytes()).into_owned())
                        .collect();
                    shown_path.push(shown);
                    if let Some(kind) = entry_kind(&shown_path.join("/")) {
                        let mut parts = rel.clone();
                        parts.push(name);
                        found.push((parts, kind));
                    }
                }
                // Links, FIFOs, sockets and devices are never opened.
                _ => {}
            }
        }
        Ok(())
    }

    /// Windows has no handle-relative walk: each name is checked with
    /// `symlink_metadata`, then opened (D10's recorded race).
    #[cfg(not(unix))]
    fn folder(&mut self, path: &Path) -> Result<(), ImportError> {
        let io_error = |error: io::Error| ImportError::io(path, error);
        let mut found = Vec::new();
        let mut seen = 0;
        self.list_folder(path, &mut Vec::new(), 0, &mut found, &mut seen)
            .map_err(|error| match error {
                Listed::Stop(error) => error,
                Listed::Io(error) => io_error(error),
            })?;
        for (parts, kind) in in_reading_order(found) {
            self.check_cancel()?;
            let mut full = path.to_path_buf();
            let mut linked = false;
            for part in &parts {
                full.push(part);
                match std::fs::symlink_metadata(&full) {
                    Ok(meta) if !meta.file_type().is_symlink() => {}
                    _ => linked = true,
                }
            }
            if linked || !std::fs::symlink_metadata(&full).is_ok_and(|meta| meta.is_file()) {
                continue;
            }
            let Ok(file) = File::open(&full) else {
                // Unreadable: left out and counted (M5).
                self.skip();
                continue;
            };
            if !file.metadata().is_ok_and(|meta| meta.is_file()) {
                continue;
            }
            if kind == EntryKind::Zip {
                self.folder_zip(file)?;
            } else {
                self.read_entry(kind, file, self.limits.json_entry, None)?;
            }
        }
        Ok(())
    }

    #[cfg(not(unix))]
    fn list_folder(
        &self,
        dir: &Path,
        rel: &mut Vec<std::ffi::OsString>,
        depth: usize,
        found: &mut Vec<(Vec<std::ffi::OsString>, EntryKind)>,
        seen: &mut usize,
    ) -> Result<(), Listed> {
        let mut entries = Vec::new();
        let listing = match std::fs::read_dir(dir) {
            Ok(listing) => listing,
            // An unreadable folder below the picked one is left out (M5).
            Err(_) if depth > 0 => {
                self.skip();
                return Ok(());
            }
            Err(error) => return Err(Listed::Io(error)),
        };
        for entry in listing {
            *seen += 1;
            if *seen > self.limits.entries {
                return Err(Listed::Stop(ImportError::Refused(ARCHIVE_REFUSED)));
            }
            match entry.and_then(|entry| Ok((entry.file_name(), entry.file_type()?))) {
                Ok(found) => entries.push(found),
                Err(_) => self.skip(),
            }
        }
        entries.sort_by(|a, b| a.0.cmp(&b.0));
        for (name, kind) in entries {
            self.check_cancel().map_err(Listed::Stop)?;
            let shown = name.to_string_lossy().into_owned();
            if never_opened(&shown) || kind.is_symlink() {
                continue;
            }
            if kind.is_dir() && depth < self.limits.folder_depth {
                rel.push(name.clone());
                let listed = self.list_folder(&dir.join(&name), rel, depth + 1, found, seen);
                rel.pop();
                listed?;
            } else if kind.is_file() {
                let mut shown_path: Vec<String> = rel
                    .iter()
                    .map(|part| part.to_string_lossy().into_owned())
                    .collect();
                shown_path.push(shown);
                if let Some(entry_kind) = entry_kind(&shown_path.join("/")) {
                    let mut parts = rel.clone();
                    parts.push(name);
                    found.push((parts, entry_kind));
                }
            }
        }
        Ok(())
    }
}

/// Why listing a folder stopped.
enum Listed {
    /// A refusal or cancelling, answered as it is.
    Stop(ImportError),
    Io(io::Error),
}

#[cfg(unix)]
mod folder_unix {
    use super::{ImportError, Listed, ARCHIVE_REFUSED};
    use rustix::fs::{Dir, FileType, Mode, OFlags};
    use rustix::io::Errno;
    use std::ffi::CString;
    use std::fs::File;
    use std::io;
    use std::os::fd::{AsFd, OwnedFd};

    /// The names in `dir`, sorted, each counted in `seen`; past
    /// `entries` the walk is refused.
    pub(super) fn names(
        dir: &OwnedFd,
        seen: &mut usize,
        entries: usize,
    ) -> Result<Vec<CString>, Listed> {
        let mut names = Vec::new();
        for entry in Dir::read_from(dir).map_err(|error| Listed::Io(error.into()))? {
            let entry = entry.map_err(|error| Listed::Io(error.into()))?;
            let name = entry.file_name();
            if name.to_bytes() == b"." || name.to_bytes() == b".." {
                continue;
            }
            *seen += 1;
            if *seen > entries {
                return Err(Listed::Stop(ImportError::Refused(ARCHIVE_REFUSED)));
            }
            names.push(name.to_owned());
        }
        names.sort();
        Ok(names)
    }

    /// Opens the listed file `parts` under `root`, a folder handle at a
    /// time with no link followed. `None` when it is gone or no longer a
    /// regular file.
    pub(super) fn open_under(root: &OwnedFd, parts: &[CString]) -> io::Result<Option<File>> {
        let Some((last, folders)) = parts.split_last() else {
            return Ok(None);
        };
        let mut folder: Option<OwnedFd> = None;
        for part in folders {
            let parent = folder.as_ref().map_or_else(|| root.as_fd(), AsFd::as_fd);
            match rustix::fs::openat(
                parent,
                part.as_c_str(),
                OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
            ) {
                Ok(next) => folder = Some(next),
                Err(Errno::NOENT | Errno::LOOP | Errno::NOTDIR) => return Ok(None),
                Err(error) => return Err(error.into()),
            }
        }
        let parent = folder.as_ref().map_or_else(|| root.as_fd(), AsFd::as_fd);
        let opened = match rustix::fs::openat(
            parent,
            last.as_c_str(),
            OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
            Mode::empty(),
        ) {
            Ok(opened) => opened,
            Err(Errno::NOENT | Errno::LOOP | Errno::NOTDIR) => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        if FileType::from_raw_mode(rustix::fs::fstat(&opened)?.st_mode) != FileType::RegularFile {
            return Ok(None);
        }
        Ok(Some(File::from(opened)))
    }
}

/// Numbered ChatGPT shards first, by number; everything else after, in
/// the order found.
fn in_reading_order<T>(found: Vec<(T, EntryKind)>) -> Vec<(T, EntryKind)> {
    let (mut shards, rest): (Vec<_>, Vec<_>) = found
        .into_iter()
        .partition(|(_, kind)| matches!(kind, EntryKind::Conversations { shard: Some(_) }));
    shards.sort_by_key(|(_, kind)| match kind {
        EntryKind::Conversations { shard } => *shard,
        _ => None,
    });
    shards.extend(rest);
    shards
}

/// Opens an input read-only, never through a link in its last name.
/// `None` for a link or anything but a regular file.
fn open_input(path: &Path) -> io::Result<Option<File>> {
    #[cfg(unix)]
    let opened = {
        use std::os::unix::fs::OpenOptionsExt as _;
        match std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC)
            .open(path)
        {
            Ok(file) => file,
            Err(error) if error.raw_os_error() == Some(libc::ELOOP) => return Ok(None),
            Err(error) => return Err(error),
        }
    };
    #[cfg(not(unix))]
    let opened = {
        // A link, a folder, or a reserved device name adds nothing (M6).
        let device = path
            .file_name()
            .is_some_and(|name| is_device_name(&name.to_string_lossy().to_ascii_lowercase()));
        if device || !std::fs::symlink_metadata(path)?.is_file() {
            return Ok(None);
        }
        File::open(path)?
    };
    Ok(opened.metadata()?.is_file().then_some(opened))
}

/// A local file header: a ZIP with an entry to read. An empty ZIP's end
/// record (`PK\x05\x06`) holds nothing, so a nested or found one is left
/// out and counted like any `.zip` that isn't one (M9).
fn is_zip_magic(head: &[u8]) -> bool {
    head == b"PK\x03\x04"
}

/// Scratch copies are named `nested-<walk>-<n>.zip`.
const NESTED_PREFIX: &str = "nested-";
/// A scratch copy this old was left by a read that never finished.
const STALE_COPY: std::time::Duration = std::time::Duration::from_secs(60 * 60);

/// Deletes scratch copies a crashed read left behind (M4): regular files
/// named `nested-*.zip` and older than [`STALE_COPY`]. Best effort.
fn sweep_stale_copies(scratch: &Path) {
    let Ok(listing) = std::fs::read_dir(scratch) else {
        return;
    };
    for entry in listing.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !(name.starts_with(NESTED_PREFIX) && name.ends_with(".zip")) {
            continue;
        }
        let path = entry.path();
        let stale = std::fs::symlink_metadata(&path).is_ok_and(|meta| {
            meta.is_file()
                && meta
                    .modified()
                    .ok()
                    .and_then(|modified| modified.elapsed().ok())
                    .is_some_and(|age| age > STALE_COPY)
        });
        if stale {
            let _ = std::fs::remove_file(&path);
        }
    }
}

enum Sniffed {
    Zip,
    /// Bare JSON, with the file's length.
    Json(u64),
    Other,
}

/// What a file holds, by its first bytes (D4). Leaves it at its start.
fn sniff(file: &mut File) -> io::Result<Sniffed> {
    let len = file.metadata()?.len();
    let mut head = Vec::new();
    Read::by_ref(file)
        .take(SNIFF_BYTES)
        .read_to_end(&mut head)?;
    file.seek(SeekFrom::Start(0))?;
    if head.starts_with(b"PK\x03\x04") {
        return Ok(Sniffed::Zip);
    }
    let body = head.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(&head);
    let first = body
        .iter()
        .find(|byte| !matches!(byte, b' ' | b'\t' | b'\n' | b'\r'));
    Ok(match first {
        Some(b'[' | b'{') => Sniffed::Json(len),
        _ => Sniffed::Other,
    })
}

/// A nested ZIP's copy in scratch, deleted when dropped.
struct ScratchCopy {
    path: PathBuf,
    file: Option<File>,
}

impl ScratchCopy {
    /// A new file (never an existing one), readable only by this user.
    fn create(path: PathBuf) -> Result<Self, ImportError> {
        let mut options = std::fs::OpenOptions::new();
        options.read(true).write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            options.mode(0o600);
        }
        let file = options
            .open(&path)
            .map_err(|error| ImportError::io(&path, error))?;
        Ok(Self {
            path,
            file: Some(file),
        })
    }
}

impl Drop for ScratchCopy {
    fn drop(&mut self) {
        // Closed first: Windows can't delete an open file.
        drop(self.file.take());
        let _ = std::fs::remove_file(&self.path);
    }
}

/// Counts the bytes actually read against one entry's budget and the
/// read's, and hands out no byte past either.
struct Capped<'a, R> {
    inner: R,
    left: u64,
    inflated_left: Rc<Cell<u64>>,
    cancel: &'a AtomicBool,
    refusal: Refusal,
}

impl<R: Read> Read for Capped<'_, R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.cancel.load(Ordering::Relaxed) {
            return Err(io::Error::other("the read was cancelled"));
        }
        let allowed = self.left.min(self.inflated_left.get());
        let want =
            usize::try_from(allowed.saturating_add(1)).map_or(buf.len(), |cap| cap.min(buf.len()));
        let n = self.inner.read(&mut buf[..want])?;
        let got = to_u64(n);
        if got > allowed {
            return Err(refuse(&self.refusal, ARCHIVE_REFUSED));
        }
        self.left -= got;
        self.inflated_left.set(self.inflated_left.get() - got);
        Ok(n)
    }
}

// ── The end records (F10) ──────────────────────────────────────────────

const EOCD_SIG: &[u8; 4] = b"PK\x05\x06";
const LOCATOR_SIG: &[u8; 4] = b"PK\x06\x07";
const EOCD64_SIG: &[u8; 4] = b"PK\x06\x06";
const EOCD_LEN: usize = 22;
const LOCATOR_LEN: u64 = 20;
const EOCD64_LEN: usize = 56;
/// The end record, its longest comment and some slack: everywhere the
/// zip crate looks for an end record.
const TAIL: u64 = 22 + 65_535 + 64;
/// Bytes searched for a Zip64 end record before its locator. A real one
/// sits right before it.
const MAX_ZIP64_SEARCH: u64 = 1 << 20;

fn read_at(file: &mut File, at: u64, len: u64) -> Result<Vec<u8>, ImportError> {
    let mut bytes = vec![0; usize::try_from(len).map_err(|_| damaged())?];
    file.seek(SeekFrom::Start(at)).map_err(|_| damaged())?;
    file.read_exact(&mut bytes).map_err(|_| damaged())?;
    Ok(bytes)
}

fn field<const N: usize>(bytes: &[u8], at: usize) -> [u8; N] {
    let mut out = [0; N];
    if let Some(slice) = bytes.get(at..at + N) {
        out.copy_from_slice(slice);
    }
    out
}

fn u16_at(bytes: &[u8], at: usize) -> u16 {
    u16::from_le_bytes(field(bytes, at))
}

fn u32_at(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(field(bytes, at))
}

fn u64_at(bytes: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(field(bytes, at))
}

/// Refuses a central directory that claims more than the caps: over
/// `entries` entries, a declared size over [`MAX_CENTRAL_DIRECTORY`], or
/// more than that between where it starts and its end record (all the
/// zip crate could decode).
fn within(limits: &Limits, entries: u64, declared: u64, span: u64) -> Result<(), ImportError> {
    if entries > to_u64(limits.entries)
        || declared > MAX_CENTRAL_DIRECTORY
        || span > MAX_CENTRAL_DIRECTORY
    {
        Err(ImportError::Refused(ARCHIVE_REFUSED))
    } else {
        Ok(())
    }
}

/// Checks every end record the zip crate could use before it parses
/// the central directory (F10): `ZipArchive::new` decodes every entry an
/// end record claims, with no cap of its own. A candidate the crate
/// would pass over is passed over here too.
///
/// # Errors
///
/// `ARCHIVE_REFUSED` past a cap, `DAMAGED_EXPORT` with no usable end
/// record.
fn check_end_records(file: &mut File, len: u64, limits: &Limits) -> Result<(), ImportError> {
    let tail_len = len.min(TAIL);
    let tail_start = len - tail_len;
    let tail = read_at(file, tail_start, tail_len)?;
    let mut usable = false;
    for p in (0..tail.len().saturating_sub(EOCD_LEN - 1)).rev() {
        if &tail[p..p + 4] != EOCD_SIG {
            continue;
        }
        let at = tail_start + to_u64(p);
        let disk = u16_at(&tail, p + 4);
        let directory_disk = u16_at(&tail, p + 6);
        let on_disk = u16_at(&tail, p + 8);
        let total = u16_at(&tail, p + 10);
        let size = u32_at(&tail, p + 12);
        let offset = u32_at(&tail, p + 16);
        let comment = u16_at(&tail, p + 20);
        if at + to_u64(EOCD_LEN) + u64::from(comment) > len {
            continue;
        }
        let zip64 = total == u16::MAX || size == u32::MAX || offset == u32::MAX;
        if let (true, Some(locator_at)) = (zip64, at.checked_sub(LOCATOR_LEN)) {
            let locator = read_at(file, locator_at, LOCATOR_LEN)?;
            if &locator[..4] == LOCATOR_SIG {
                usable |= check_zip64(file, locator_at, &locator, limits)?;
                continue;
            }
        }
        if disk != 0 || directory_disk != 0 || on_disk != total {
            continue;
        }
        if total > 0 && u64::from(offset) >= at {
            continue;
        }
        within(
            limits,
            u64::from(total),
            u64::from(size),
            at.saturating_sub(u64::from(offset)),
        )?;
        usable = true;
    }
    if usable {
        Ok(())
    } else {
        Err(damaged())
    }
}

/// The Zip64 end records a locator at `locator_at` could lead to:
/// whether any is usable, after checking each against the caps.
///
/// Recorded limit (final review): each locator reads up to
/// [`MAX_ZIP64_SEARCH`] before it is refused, and a forged 64 KiB tail
/// can name about 2,900 locators, so a hostile ZIP can cost about 3 GB
/// of reads. They are bounded, held one region at a time, and refuse.
fn check_zip64(
    file: &mut File,
    locator_at: u64,
    locator: &[u8],
    limits: &Limits,
) -> Result<bool, ImportError> {
    let from = u64_at(locator, 8);
    let disks = u32_at(locator, 16);
    if from >= locator_at || disks > 1 {
        return Ok(false);
    }
    let span = locator_at - from;
    if span > MAX_ZIP64_SEARCH {
        return Err(damaged());
    }
    let region = read_at(file, from, span)?;
    let mut usable = false;
    for p in 0..region.len().saturating_sub(EOCD64_LEN - 1) {
        if &region[p..p + 4] != EOCD64_SIG {
            continue;
        }
        let at = from + to_u64(p);
        let record_size = u64_at(&region, p + 4);
        if record_size < 44 || record_size.checked_add(12) != Some(locator_at - at) {
            continue;
        }
        let on_disk = u64_at(&region, p + 24);
        let total = u64_at(&region, p + 32);
        let size = u64_at(&region, p + 40);
        let offset = u64_at(&region, p + 48);
        if offset > at {
            continue;
        }
        within(limits, on_disk.max(total), size, at - offset)?;
        usable = true;
    }
    Ok(usable)
}

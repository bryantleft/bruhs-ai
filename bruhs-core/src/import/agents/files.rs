//! Reading a persona's files by name, never through a link (D10, F22).
//!
//! On unix every name opens relative to a folder handle: each folder with
//! `O_DIRECTORY | O_NOFOLLOW`, the file with `O_NOFOLLOW | O_NONBLOCK`,
//! then the handle is checked as a regular file within
//! [`MAX_PERSONA_FILE_BYTES`]. Elsewhere each part is checked with
//! `symlink_metadata` before it is opened, a race D18 records. A name in
//! [`NEVER_OPEN`], or like `.env.*` or `*.sqlite*`, is never opened.

use bruhs_models::imports::{FileUse, SkipReason, MAX_PERSONA_FILE_BYTES};
use std::io::Read as _;
use std::path::Path;

/// Names never opened in an agent folder, in any part of a name and
/// ignoring ASCII case. `.env.*` and `*.sqlite*` are refused too, along
/// with everything an export never opens.
pub(crate) const NEVER_OPEN: [&str; 10] = [
    ".env",
    "auth.json",
    "auth-profiles.json",
    "credentials",
    "state.db",
    "config.yaml",
    "openclaw.json",
    "clawdbot.json",
    "moltbot.json",
    "secrets",
];

/// Whether one part of a name is never opened.
pub(crate) fn never_open(part: &str) -> bool {
    let lower = part.to_ascii_lowercase();
    NEVER_OPEN.contains(&lower.as_str())
        || lower.starts_with(".env.")
        || lower.contains(".sqlite")
        || super::super::archive::never_opened(part)
}

/// One read of a persona file.
pub(crate) enum FileRead {
    /// Its text, lossy UTF-8.
    Text(String),
    /// Not there, or a name that is never opened.
    Missing,
    Skipped(SkipReason),
}

/// What a listed name is, as seen without following it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    Folder,
    File {
        bytes: u64,
    },
    Link,
    /// A FIFO, socket or device.
    Other,
}

/// The names in `dir`, at most `max` of them, sorted, and whether each is
/// a real folder: a link counts as a link, not a folder. Never-opened
/// names and names that aren't UTF-8 are left out. A folder that is a
/// link or can't be read lists nothing.
pub(crate) fn list_names(dir: &Path, max: usize) -> Vec<(String, bool)> {
    entries(dir, max)
        .into_iter()
        .map(|(name, kind)| (name, kind == Kind::Folder))
        .collect()
}

/// [`list_names`] with what each name is.
pub(crate) fn entries(dir: &Path, max: usize) -> Vec<(String, Kind)> {
    let mut found = listed(dir, max);
    found.retain(|(name, _)| !never_open(name));
    found.sort_by(|a, b| a.0.cmp(&b.0));
    found
}

#[cfg(unix)]
fn listed(dir: &Path, max: usize) -> Vec<(String, Kind)> {
    use rustix::fs::{AtFlags, Dir, FileType, Mode, OFlags};
    let Ok(folder) = rustix::fs::open(
        dir,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    ) else {
        return Vec::new();
    };
    let Ok(entries) = Dir::read_from(&folder) else {
        return Vec::new();
    };
    let mut found = Vec::new();
    let mut seen = 0;
    for entry in entries {
        let Ok(entry) = entry else { break };
        let bytes = entry.file_name().to_bytes();
        if bytes == b"." || bytes == b".." {
            continue;
        }
        seen += 1;
        if seen > max {
            break;
        }
        let Ok(name) = std::str::from_utf8(bytes) else {
            continue;
        };
        let Ok(stat) = rustix::fs::statat(&folder, entry.file_name(), AtFlags::SYMLINK_NOFOLLOW)
        else {
            continue;
        };
        let kind = match FileType::from_raw_mode(stat.st_mode) {
            FileType::Directory => Kind::Folder,
            FileType::RegularFile => Kind::File {
                bytes: u64::try_from(stat.st_size).unwrap_or(0),
            },
            FileType::Symlink => Kind::Link,
            _ => Kind::Other,
        };
        found.push((name.to_string(), kind));
    }
    found
}

#[cfg(not(unix))]
fn listed(dir: &Path, max: usize) -> Vec<(String, Kind)> {
    if !std::fs::symlink_metadata(dir).is_ok_and(|meta| meta.is_dir()) {
        return Vec::new();
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut found = Vec::new();
    for entry in entries.take(max) {
        let Ok(entry) = entry else { break };
        let Ok(name) = entry.file_name().into_string() else {
            continue;
        };
        // Neither follows a link.
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        let kind = if kind.is_symlink() {
            Kind::Link
        } else if kind.is_dir() {
            Kind::Folder
        } else if kind.is_file() {
            Kind::File {
                bytes: entry.metadata().map_or(0, |meta| meta.len()),
            }
        } else {
            Kind::Other
        };
        found.push((name, kind));
    }
    found
}

/// The text of `name` under `dir`. `name` may hold folders, split on `/`;
/// each part is checked against the never-opened names before anything
/// is opened, and none of them is followed when it is a link.
pub(crate) fn read_text(dir: &Path, name: &str) -> FileRead {
    let parts: Vec<&str> = name.split('/').collect();
    if parts
        .iter()
        .any(|part| part.is_empty() || *part == "." || *part == ".." || never_open(part))
    {
        return FileRead::Missing;
    }
    match open(dir, &parts) {
        Ok(file) => {
            let mut bytes = Vec::new();
            match file.take(MAX_PERSONA_FILE_BYTES).read_to_end(&mut bytes) {
                Ok(_) => FileRead::Text(String::from_utf8_lossy(&bytes).into_owned()),
                Err(_) => FileRead::Skipped(SkipReason::Unreadable),
            }
        }
        Err(read) => read,
    }
}

/// Opens `parts` under `dir` as a regular file within the cap.
#[cfg(unix)]
fn open(dir: &Path, parts: &[&str]) -> Result<std::fs::File, FileRead> {
    use rustix::fs::{AtFlags, FileType, Mode, OFlags};
    use rustix::io::Errno;
    use std::os::fd::{AsFd, BorrowedFd};
    // Opening a linked folder with `O_NOFOLLOW` fails as `ENOTDIR` on some
    // systems and `ELOOP` on others: the name itself says which it is.
    let failed = |error: Errno, parent: Option<BorrowedFd<'_>>, name: &str| match error {
        Errno::LOOP => FileRead::Skipped(SkipReason::Link),
        Errno::NOENT => FileRead::Missing,
        Errno::NOTDIR => {
            let linked = parent.is_some_and(|parent| {
                rustix::fs::statat(parent, name, AtFlags::SYMLINK_NOFOLLOW)
                    .is_ok_and(|stat| FileType::from_raw_mode(stat.st_mode) == FileType::Symlink)
            });
            if linked {
                FileRead::Skipped(SkipReason::Link)
            } else {
                FileRead::Missing
            }
        }
        _ => FileRead::Skipped(SkipReason::Unreadable),
    };
    let folder_flags = OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC;
    let (last, folders) = parts.split_last().ok_or(FileRead::Missing)?;
    let mut folder = rustix::fs::open(dir, folder_flags, Mode::empty()).map_err(|error| {
        let linked = std::fs::symlink_metadata(dir).is_ok_and(|meta| meta.file_type().is_symlink());
        if linked {
            FileRead::Skipped(SkipReason::Link)
        } else {
            failed(error, None, "")
        }
    })?;
    for part in folders {
        folder = rustix::fs::openat(&folder, *part, folder_flags, Mode::empty())
            .map_err(|error| failed(error, Some(folder.as_fd()), part))?;
    }
    let file = rustix::fs::openat(
        &folder,
        *last,
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(|error| failed(error, Some(folder.as_fd()), last))?;
    let stat = rustix::fs::fstat(&file).map_err(|error| failed(error, None, ""))?;
    if FileType::from_raw_mode(stat.st_mode) != FileType::RegularFile {
        return Err(FileRead::Skipped(SkipReason::NotAFile));
    }
    if u64::try_from(stat.st_size).unwrap_or(u64::MAX) > MAX_PERSONA_FILE_BYTES {
        return Err(FileRead::Skipped(SkipReason::TooLarge));
    }
    Ok(std::fs::File::from(file))
}

/// Windows has no handle-relative open: each part is checked with
/// `symlink_metadata`, then the file is opened and checked again.
#[cfg(not(unix))]
fn open(dir: &Path, parts: &[&str]) -> Result<std::fs::File, FileRead> {
    let failed = |error: std::io::Error| match error.kind() {
        std::io::ErrorKind::NotFound => FileRead::Missing,
        _ => FileRead::Skipped(SkipReason::Unreadable),
    };
    let mut path = dir.to_path_buf();
    let mut meta = std::fs::symlink_metadata(&path).map_err(failed)?;
    for part in parts {
        if meta.file_type().is_symlink() {
            return Err(FileRead::Skipped(SkipReason::Link));
        }
        if !meta.is_dir() {
            return Err(FileRead::Missing);
        }
        path.push(part);
        meta = std::fs::symlink_metadata(&path).map_err(failed)?;
    }
    if meta.file_type().is_symlink() {
        return Err(FileRead::Skipped(SkipReason::Link));
    }
    let checked = |meta: &std::fs::Metadata| {
        if !meta.is_file() {
            Err(FileRead::Skipped(SkipReason::NotAFile))
        } else if meta.len() > MAX_PERSONA_FILE_BYTES {
            Err(FileRead::Skipped(SkipReason::TooLarge))
        } else {
            Ok(())
        }
    };
    checked(&meta)?;
    let file = std::fs::File::open(&path).map_err(failed)?;
    checked(&file.metadata().map_err(failed)?)?;
    Ok(file)
}

/// What a listed file named for `use_` becomes before it is opened: a
/// link, a folder or a FIFO in its place, or a file over the cap, is
/// skipped.
pub(crate) fn planned(kind: Kind, use_: FileUse) -> FileUse {
    let reason = match kind {
        Kind::File { bytes } if bytes > MAX_PERSONA_FILE_BYTES => SkipReason::TooLarge,
        Kind::File { .. } => return use_,
        Kind::Link => SkipReason::Link,
        Kind::Folder | Kind::Other => SkipReason::NotAFile,
    };
    FileUse::Skipped { reason }
}

//! Cross-process compare-and-save. File I/O stays on the background writer.

use sha2::{Digest, Sha256};
use std::fs::{self, File, OpenOptions, TryLockError};
use std::io::{self, Read};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) enum DiskVersion {
    #[default]
    Missing,
    Present([u8; 32]),
    Untrusted,
}

impl DiskVersion {
    pub(super) fn of_bytes(bytes: &[u8]) -> Self {
        Self::Present(Sha256::digest(bytes).into())
    }

    pub(super) fn read(path: &Path) -> io::Result<Self> {
        reject_non_file(path)?;
        let mut file = match File::open(path) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Self::Missing),
            Err(error) => return Err(error),
        };
        let mut hash = Sha256::new();
        let mut buffer = [0; 16 * 1024];
        loop {
            let count = file.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            hash.update(&buffer[..count]);
        }
        Ok(Self::Present(hash.finalize().into()))
    }
}

#[derive(Debug)]
pub(super) enum SaveError {
    Conflict,
    Busy,
    Io(String),
}

impl std::fmt::Display for SaveError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Conflict => formatter.write_str("Another instance changed the saved workspace. Copy recovery before reloading; retry cannot overwrite the newer version."),
            Self::Busy => formatter.write_str("Another instance is saving this workspace. Retry when that save finishes."),
            Self::Io(message) => formatter.write_str(message),
        }
    }
}

impl std::error::Error for SaveError {}

impl From<io::Error> for SaveError {
    fn from(error: io::Error) -> Self {
        Self::Io(error.to_string())
    }
}

impl From<&str> for SaveError {
    fn from(message: &str) -> Self {
        Self::Io(message.into())
    }
}

pub(super) fn reject_non_file(path: &Path) -> io::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if !metadata.is_file() => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "The saved-state path must be a regular file, not a symlink or directory.",
        )),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

fn lock_path(path: &Path) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(".lock");
    path.with_file_name(name)
}

struct StateLock {
    file: File,
}

impl Drop for StateLock {
    fn drop(&mut self) {
        // Process creation on another thread can briefly inherit this open
        // description. Closing only our descriptor does not release that lock.
        if let Err(error) = self.file.unlock() {
            tracing::warn!("explicit state unlock failed; closing handle: {error}");
        }
    }
}

fn acquire_lock(path: &Path) -> Result<StateLock, SaveError> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::create_dir_all(parent)?;
    let lock_path = lock_path(path);
    reject_non_file(&lock_path)?;
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let lock = options.open(lock_path)?;
    lock.try_lock().map_err(|error| match error {
        TryLockError::WouldBlock => SaveError::Busy,
        TryLockError::Error(error) => error.into(),
    })?;
    // Keep the sidecar inode stable; deleting it permits independent locks.
    // Explicit unlock handles inherited descriptors; process death still
    // releases the lock when all owning descriptors close.
    Ok(StateLock { file: lock })
}

pub(super) fn guarded_write(
    path: &Path,
    expected: &mut DiskVersion,
    bytes: &[u8],
    write: impl FnOnce(&[u8]) -> io::Result<()>,
) -> Result<(), SaveError> {
    let candidate = DiskVersion::of_bytes(bytes);
    let _lock = acquire_lock(path)?;
    if *expected == DiskVersion::Untrusted || DiskVersion::read(path)? != *expected {
        return Err(SaveError::Conflict);
    }
    let result = write(bytes);
    if result.is_ok() || DiskVersion::read(path).is_ok_and(|actual| actual == candidate) {
        // A directory sync can fail after rename. Remember our installed bytes,
        // but keep the UI failed until a retry actually acknowledges durability.
        *expected = candidate;
    }
    result.map_err(Into::into)
}

#[cfg(test)]
mod tests;

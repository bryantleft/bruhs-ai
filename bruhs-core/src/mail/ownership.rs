use super::MailError;
use std::{
    fs::File,
    path::{Path, PathBuf},
};

pub(super) struct RuntimeLock(File);

impl Drop for RuntimeLock {
    fn drop(&mut self) {
        if let Err(error) = self.0.unlock() {
            tracing::error!(%error, "Mail runtime ownership could not be released");
        }
    }
}

pub(super) fn acquire(path: &Path) -> Result<(PathBuf, RuntimeLock), MailError> {
    if path.as_os_str().is_empty() || path.file_name().is_none() {
        return Err(MailError::InvalidPath);
    }
    let parent = path
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let mut directories = std::fs::DirBuilder::new();
    directories.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        directories.mode(0o700);
    }
    directories.create(parent).map_err(|_| MailError::Storage)?;
    let path = if path.exists() {
        std::fs::canonicalize(path)
    } else {
        std::fs::canonicalize(parent)
            .map(|parent| parent.join(path.file_name().unwrap_or_default()))
    }
    .map_err(|_| MailError::Storage)?;
    let mut options = std::fs::OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    let file = options.open(&path).map_err(|_| MailError::Storage)?;
    if !file.metadata().map_err(|_| MailError::Storage)?.is_file() {
        return Err(MailError::InvalidPath);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        // SQLite's journal path is tied to its filename. Hard links are not a
        // supported alias for a database, even after the previous owner stops.
        if file.metadata().map_err(|_| MailError::Storage)?.nlink() != 1 {
            return Err(MailError::InvalidPath);
        }
        file.set_permissions(std::fs::Permissions::from_mode(0o600))
            .map_err(|_| MailError::Storage)?;
    }
    drop(file);
    // A separate lock must not contend with SQLite's own database locks on macOS.
    let mut lock_path = path.as_os_str().to_owned();
    lock_path.push(".lock");
    let lock = options.open(lock_path).map_err(|_| MailError::Storage)?;
    lock.try_lock().map_err(|_| MailError::AlreadyOwned)?;
    let owner = RuntimeLock(lock);
    Ok((path, owner))
}

//! Fixed repository path, bounded reads, optimistic revisions, atomic replacement.
use super::{ApiClientError, ApiResult};
use bruhs_models::api_client::*;
use sha2::{Digest, Sha256};
use std::{
    fs::File,
    io::{Read, Write},
    path::Path,
};

pub(super) fn load(root: &Path) -> ApiResult<ApiCollectionSnapshot> {
    match Directory::project(root, false)? {
        Some(directory) => read(&directory),
        None => Ok(empty()),
    }
}
pub(super) fn save(
    root: &Path,
    collection: ApiCollection,
    revision: Option<String>,
) -> ApiResult<ApiCollectionSnapshot> {
    let directory = Directory::project(root, true)?.ok_or_else(storage_error)?;
    let _lock = directory.lock()?;
    let previous = read(&directory)?;
    if previous.revision != revision {
        return Err(ApiClientError::Conflict);
    }
    let mut bytes = serde_json::to_vec_pretty(&collection).map_err(|_| storage_error())?;
    bytes.push(b'\n');
    if bytes.len() > PAYLOAD_BYTES {
        return Err(ApiClientError::Invalid(
            "The formatted collection exceeds 32 KiB. Split or reduce its requests.".into(),
        ));
    }
    directory.replace(&bytes)?;
    Ok(ApiCollectionSnapshot {
        collection,
        revision: Some(hash(&bytes)),
    })
}
fn empty() -> ApiCollectionSnapshot {
    ApiCollectionSnapshot {
        collection: ApiCollection::default(),
        revision: None,
    }
}
fn read(directory: &Directory) -> ApiResult<ApiCollectionSnapshot> {
    let Some(mut file) = directory.read_file()? else {
        return Ok(empty());
    };
    if !file.metadata().map_err(|_| storage_error())?.is_file() {
        return Err(unsafe_path());
    }
    let mut bytes = Vec::new();
    Read::by_ref(&mut file)
        .take(u64::try_from(PAYLOAD_BYTES).unwrap_or(u64::MAX) + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| storage_error())?;
    if bytes.len() > PAYLOAD_BYTES {
        return Err(ApiClientError::Invalid("The saved collection exceeds 32 KiB. Reduce it externally before reopening; nothing was overwritten.".into()));
    }
    let collection:ApiCollection=serde_json::from_slice(&bytes).map_err(|_|ApiClientError::Invalid("The collection JSON is invalid. Repair or restore .bruhs/api/collection.json; nothing was overwritten.".into()))?;
    super::validation::collection(&collection)?;
    Ok(ApiCollectionSnapshot {
        collection,
        revision: Some(hash(&bytes)),
    })
}
fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn storage_error() -> ApiClientError {
    ApiClientError::Storage("Could not read or atomically save .bruhs/api/collection.json. Check this working folder's permissions and retry.".into())
}
fn unsafe_path() -> ApiClientError {
    ApiClientError::Invalid("The API collection path contains a link or a non-regular file. Use real .bruhs/api directories and a regular collection.json inside this project.".into())
}

#[cfg(unix)]
struct Directory {
    file: File,
}
#[cfg(unix)]
impl Directory {
    fn project(root: &Path, create: bool) -> ApiResult<Option<Self>> {
        use std::os::unix::fs::OpenOptionsExt;
        let root = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(root)
            .map_err(|_| unsafe_path())?;
        let Some(first) = (Self { file: root }).child(c".bruhs", create)? else {
            return Ok(None);
        };
        first.child(c"api", create)
    }
    fn child(&self, name: &std::ffi::CStr, create: bool) -> ApiResult<Option<Self>> {
        use std::os::fd::{AsRawFd, FromRawFd};
        if create {
            // SAFETY: valid owned directory descriptor and NUL-terminated literal component.
            let result = unsafe { libc::mkdirat(self.file.as_raw_fd(), name.as_ptr(), 0o700) };
            if result < 0
                && std::io::Error::last_os_error().kind() != std::io::ErrorKind::AlreadyExists
            {
                return Err(storage_error());
            }
        }
        // Parent descriptors keep replacement/rename races confined to the opened directory.
        // SAFETY: valid descriptor, component C string, no pointer escapes this call.
        let fd = unsafe {
            libc::openat(
                self.file.as_raw_fd(),
                name.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        if fd < 0 {
            if !create && std::io::Error::last_os_error().kind() == std::io::ErrorKind::NotFound {
                return Ok(None);
            }
            return Err(unsafe_path());
        }
        // SAFETY: openat returned a new descriptor now exclusively owned by File.
        let file = unsafe { File::from_raw_fd(fd) };
        Ok(Some(Self { file }))
    }
    fn lock(&self) -> ApiResult<File> {
        use std::os::fd::{AsRawFd, FromRawFd};
        // A stable lock inode serializes cooperating processes, independent of atomic JSON renames.
        // SAFETY: owned dir descriptor and static filename, newly created fd is uniquely owned.
        let fd = unsafe {
            libc::openat(
                self.file.as_raw_fd(),
                c".collection.lock".as_ptr(),
                libc::O_RDWR | libc::O_CREAT | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                0o600,
            )
        };
        if fd < 0 {
            return Err(unsafe_path());
        }
        // SAFETY: fd is new and valid; ownership transfers once.
        let file = unsafe { File::from_raw_fd(fd) };
        if !file.metadata().map_err(|_| storage_error())?.is_file() {
            return Err(unsafe_path());
        }
        file.try_lock().map_err(|_| ApiClientError::Busy)?;
        Ok(file)
    }
    fn read_file(&self) -> ApiResult<Option<File>> {
        use std::os::fd::{AsRawFd, FromRawFd};
        // SAFETY: read-only no-follow open relative to our owned directory descriptor.
        let fd = unsafe {
            libc::openat(
                self.file.as_raw_fd(),
                c"collection.json".as_ptr(),
                libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK,
            )
        };
        if fd < 0 {
            if std::io::Error::last_os_error().kind() == std::io::ErrorKind::NotFound {
                return Ok(None);
            }
            return Err(unsafe_path());
        }
        // SAFETY: successful openat returned this uniquely owned descriptor.
        Ok(Some(unsafe { File::from_raw_fd(fd) }))
    }
    fn replace(&self, bytes: &[u8]) -> ApiResult<()> {
        use std::os::fd::{AsRawFd, FromRawFd};
        let temporary = std::ffi::CString::new(format!(".collection-{}.tmp", uuid::Uuid::new_v4()))
            .map_err(|_| storage_error())?;
        // SAFETY: fixed directory fd and generated filename; exclusive create returns a new fd.
        let fd = unsafe {
            libc::openat(
                self.file.as_raw_fd(),
                temporary.as_ptr(),
                libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                0o600,
            )
        };
        if fd < 0 {
            return Err(storage_error());
        }
        // SAFETY: this function owns the newly opened descriptor exactly once.
        let mut file = unsafe { File::from_raw_fd(fd) };
        let result = (|| {
            file.write_all(bytes)
                .and_then(|()| file.flush())
                .and_then(|()| file.sync_all())
                .map_err(|_| storage_error())?;
            // SAFETY: both paths are single components relative to the same owned directory.
            if unsafe {
                libc::renameat(
                    self.file.as_raw_fd(),
                    temporary.as_ptr(),
                    self.file.as_raw_fd(),
                    c"collection.json".as_ptr(),
                )
            } < 0
            {
                return Err(storage_error());
            }
            self.file.sync_all().map_err(|_| storage_error())
        })();
        // SAFETY: remove only our random temporary filename. After rename it no longer exists.
        unsafe { libc::unlinkat(self.file.as_raw_fd(), temporary.as_ptr(), 0) };
        result
    }
}

#[cfg(not(unix))]
struct Directory {
    path: std::path::PathBuf,
}
#[cfg(not(unix))]
impl Directory {
    fn project(root: &Path, create: bool) -> ApiResult<Option<Self>> {
        let mut path = root.to_path_buf();
        for component in [".bruhs", "api"] {
            path.push(component);
            if create && !path.exists() {
                std::fs::create_dir(&path).map_err(|_| storage_error())?;
            }
            match std::fs::symlink_metadata(&path) {
                Ok(meta) if meta.is_dir() && !meta.file_type().is_symlink() => {}
                Err(error) if !create && error.kind() == std::io::ErrorKind::NotFound => {
                    return Ok(None)
                }
                _ => return Err(unsafe_path()),
            }
            if !path
                .canonicalize()
                .map_err(|_| unsafe_path())?
                .starts_with(root)
            {
                return Err(unsafe_path());
            }
        }
        Ok(Some(Self { path }))
    }
    fn checked_file(&self, name: &str) -> ApiResult<std::path::PathBuf> {
        let path = self.path.join(name);
        match std::fs::symlink_metadata(&path) {
            Ok(meta) if meta.is_file() && !meta.file_type().is_symlink() => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            _ => return Err(unsafe_path()),
        }
        Ok(path)
    }
    fn lock(&self) -> ApiResult<File> {
        let file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(self.checked_file(".collection.lock")?)
            .map_err(|_| storage_error())?;
        file.try_lock().map_err(|_| ApiClientError::Busy)?;
        Ok(file)
    }
    fn read_file(&self) -> ApiResult<Option<File>> {
        match File::open(self.checked_file("collection.json")?) {
            Ok(file) => Ok(Some(file)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(_) => Err(storage_error()),
        }
    }
    fn replace(&self, bytes: &[u8]) -> ApiResult<()> {
        let path = self.checked_file("collection.json")?;
        let mut temporary =
            tempfile::NamedTempFile::new_in(&self.path).map_err(|_| storage_error())?;
        temporary
            .write_all(bytes)
            .and_then(|()| temporary.flush())
            .and_then(|()| temporary.as_file().sync_all())
            .map_err(|_| storage_error())?;
        temporary.persist(path).map_err(|_| storage_error())?;
        Ok(())
    }
}

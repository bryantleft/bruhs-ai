//! Fingerprints expose only a digest and never follow project symlinks.
use super::*;
#[cfg(unix)]
fn source_file(project: &Path, relative: &str) -> std::io::Result<File> {
    use std::{
        ffi::CString,
        os::{
            fd::{AsRawFd as _, FromRawFd as _},
            unix::fs::OpenOptionsExt as _,
        },
    };
    let mut directory = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_CLOEXEC)
        .open(project)?;
    let components = relative.split('/').collect::<Vec<_>>();
    for (index, part) in components.iter().enumerate() {
        let name = CString::new(*part)
            .map_err(|_| std::io::Error::from(std::io::ErrorKind::InvalidInput))?;
        let flags = libc::O_RDONLY
            | libc::O_CLOEXEC
            | libc::O_NOFOLLOW
            | libc::O_NONBLOCK
            | if index + 1 < components.len() {
                libc::O_DIRECTORY
            } else {
                0
            };
        // SAFETY: directory owns a live descriptor and name is NUL-terminated.
        // openat borrows both and returns a new descriptor; no creation mode is used.
        let fd = unsafe { libc::openat(directory.as_raw_fd(), name.as_ptr(), flags) };
        if fd < 0 {
            return Err(std::io::Error::last_os_error());
        }
        // SAFETY: openat succeeded and this is the sole owner of its new descriptor.
        directory = unsafe { File::from_raw_fd(fd) };
    }
    Ok(directory)
}
#[cfg(not(unix))]
fn source_file(project: &Path, relative: &str) -> std::io::Result<File> {
    let root = project.canonicalize()?;
    let mut path = root.clone();
    for part in relative.split('/') {
        path.push(part);
        if fs::symlink_metadata(&path)?.file_type().is_symlink() {
            return Err(std::io::Error::from(std::io::ErrorKind::PermissionDenied));
        }
    }
    if !path.canonicalize()?.starts_with(root) {
        return Err(std::io::Error::from(std::io::ErrorKind::PermissionDenied));
    }
    File::open(path)
}
pub fn source_fingerprint(project: &Path, relative: &str) -> Result<String> {
    validate_relative_path(relative)?;
    let file = source_file(project, relative)?;
    if !file.metadata()?.is_file() || file.metadata()?.len() > MAX_BOARD_BYTES {
        return Err(BoardError::Invalid(
            "Source fingerprints require a regular project file within the board content budget."
                .into(),
        )
        .into());
    }
    let mut hasher = Sha256::new();
    let mut reader = file.take(MAX_BOARD_BYTES + 1);
    let mut total = 0u64;
    let mut buffer = [0u8; 16384];
    loop {
        let count = reader.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        total += u64::try_from(count).unwrap_or(u64::MAX);
        if total > MAX_BOARD_BYTES {
            return Err(BoardError::Invalid(
                "The source grew beyond the fingerprint budget.".into(),
            )
            .into());
        }
        hasher.update(&buffer[..count]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fingerprints_are_confined_and_change_with_source() {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir(root.path().join("src")).unwrap();
        fs::write(root.path().join("src/a.rs"), "one").unwrap();
        let one = source_fingerprint(root.path(), "src/a.rs").unwrap();
        fs::write(root.path().join("src/a.rs"), "two").unwrap();
        assert_ne!(one, source_fingerprint(root.path(), "src/a.rs").unwrap());
        assert!(source_fingerprint(root.path(), "../outside").is_err());
        #[cfg(unix)]
        {
            let outside = tempfile::tempdir().unwrap();
            fs::write(outside.path().join("secret"), "secret").unwrap();
            std::os::unix::fs::symlink(outside.path(), root.path().join("escape")).unwrap();
            assert!(source_fingerprint(root.path(), "escape/secret").is_err());
        }
    }
}

//! Copy-on-write clones of files and directory trees: `clonefile(2)` on
//! macOS (APFS) and `FICLONE` reflinks on Linux (Btrfs, XFS with reflink).
//! A clone shares the source's disk blocks until either side writes, so it
//! costs almost no space.
//!
//! Nothing here falls back to copying bytes. A filesystem or platform that
//! cannot clone yields an [`std::io::ErrorKind::Unsupported`] error, and the
//! caller decides what to do instead.

use std::io;
use std::path::Path;

/// Clone `src` (a file, a symlink, or a whole directory tree, never
/// following symlinks) to `dst`. `dst` must not exist; its parent must.
/// On failure nothing is left at `dst`.
pub fn clone_path(src: &Path, dst: &Path) -> io::Result<()> {
    if dst.symlink_metadata().is_ok() {
        return Err(io::Error::from(io::ErrorKind::AlreadyExists));
    }
    let result = imp::clone_path(src, dst);
    if result.is_err() {
        // Both platforms can stop part-way through a tree (`clonefile`
        // clones a directory "as if each item was cloned individually"),
        // and everything at `dst` was created by this call.
        if let Err(error) = remove_partial(dst) {
            tracing::debug!(
                "could not remove a partial clone {}: {error}",
                dst.display()
            );
        }
    }
    result
}

/// Remove what a failed clone left at `path`, first making its directories
/// writable, since read-only source folders are cloned read-only.
fn remove_partial(path: &Path) -> io::Result<()> {
    let meta = match path.symlink_metadata() {
        Ok(meta) => meta,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    if !meta.is_dir() {
        return std::fs::remove_file(path);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut dirs = vec![path.to_path_buf()];
        while let Some(dir) = dirs.pop() {
            let mode = std::fs::symlink_metadata(&dir)?.permissions().mode();
            std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(mode | 0o700))?;
            for entry in std::fs::read_dir(&dir)?.flatten() {
                if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
                    dirs.push(entry.path());
                }
            }
        }
    }
    std::fs::remove_dir_all(path)
}

/// `error` reclassified as `Unsupported` when the errno says this
/// filesystem, volume pair or platform cannot clone.
#[cfg(unix)]
fn classify(error: io::Error) -> io::Error {
    let unsupported = error.raw_os_error().is_some_and(|code| {
        code == libc::ENOTSUP
            || code == libc::EOPNOTSUPP
            || code == libc::EXDEV
            || code == libc::ENOSYS
            || code == libc::ENOTTY
            || (cfg!(target_os = "linux") && code == libc::EINVAL)
    });
    if unsupported {
        io::Error::new(io::ErrorKind::Unsupported, error)
    } else {
        error
    }
}

#[cfg(target_os = "macos")]
mod imp {
    use std::ffi::CString;
    use std::io;
    use std::os::unix::ffi::OsStrExt;
    use std::path::Path;

    /// `<sys/clonefile.h>`: clone a symlink itself, not its target.
    const CLONE_NOFOLLOW: u32 = 0x0001;
    /// `<sys/clonefile.h>`: the clone belongs to the caller, not the source's owner.
    const CLONE_NOOWNERCOPY: u32 = 0x0002;

    pub(super) fn clone_path(src: &Path, dst: &Path) -> io::Result<()> {
        let src = CString::new(src.as_os_str().as_bytes())?;
        let dst = CString::new(dst.as_os_str().as_bytes())?;
        // SAFETY: both pointers come from CStrings that live until the call
        // returns, and clonefile only reads them.
        let rc = unsafe {
            libc::clonefile(
                src.as_ptr(),
                dst.as_ptr(),
                CLONE_NOFOLLOW | CLONE_NOOWNERCOPY,
            )
        };
        if rc == 0 {
            Ok(())
        } else {
            Err(super::classify(io::Error::last_os_error()))
        }
    }
}

#[cfg(target_os = "linux")]
mod imp {
    use std::fs::{self, File, FileTimes, OpenOptions};
    use std::io;
    use std::os::fd::AsRawFd;
    use std::path::Path;

    pub(super) fn clone_path(src: &Path, dst: &Path) -> io::Result<()> {
        clone_entry(src, dst)
    }

    fn clone_entry(src: &Path, dst: &Path) -> io::Result<()> {
        let meta = fs::symlink_metadata(src)?;
        let kind = meta.file_type();
        if kind.is_symlink() {
            std::os::unix::fs::symlink(fs::read_link(src)?, dst)
        } else if kind.is_dir() {
            fs::create_dir(dst)?;
            for entry in fs::read_dir(src)? {
                let entry = entry?;
                clone_entry(&entry.path(), &dst.join(entry.file_name()))?;
            }
            fs::set_permissions(dst, meta.permissions())?;
            // Build tools compare modification times; a directory's is only
            // informational, so failing to keep it is not an error.
            if let Err(error) = File::open(dst).and_then(|dir| dir.set_times(times(&meta)?)) {
                tracing::debug!("kept a new mtime on {}: {error}", dst.display());
            }
            Ok(())
        } else if kind.is_file() {
            let from = File::open(src)?;
            let to = OpenOptions::new().write(true).create_new(true).open(dst)?;
            if meta.len() > 0 {
                // SAFETY: both descriptors stay open for the call; FICLONE
                // takes the source descriptor as its integer argument and
                // reads no memory.
                let rc = unsafe { libc::ioctl(to.as_raw_fd(), libc::FICLONE, from.as_raw_fd()) };
                if rc != 0 {
                    return Err(super::classify(io::Error::last_os_error()));
                }
            }
            to.set_permissions(meta.permissions())?;
            // Cargo and package managers decide freshness from mtimes.
            to.set_times(times(&meta)?)
        } else {
            // Sockets and FIFOs carry no build output.
            Ok(())
        }
    }

    fn times(meta: &fs::Metadata) -> io::Result<FileTimes> {
        Ok(FileTimes::new()
            .set_accessed(meta.accessed()?)
            .set_modified(meta.modified()?))
    }
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
mod imp {
    use std::io;
    use std::path::Path;

    pub(super) fn clone_path(_src: &Path, _dst: &Path) -> io::Result<()> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "copy-on-write clones need macOS or Linux",
        ))
    }
}

/// Whether `dir` can clone at all (APFS yes; tmpfs and ext4 no). Tests that
/// need a clone skip themselves otherwise.
#[cfg(test)]
pub(crate) fn clones_supported(dir: &Path) -> bool {
    let src = dir.join(".probe-src");
    let dst = dir.join(".probe-dst");
    std::fs::write(&src, b"probe").unwrap();
    let supported = clone_path(&src, &dst).is_ok();
    let _ = std::fs::remove_file(&src);
    let _ = std::fs::remove_file(&dst);
    supported
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clones_a_tree_with_symlinks_and_refuses_an_existing_destination() {
        let dir = tempfile::tempdir().unwrap();
        if !clones_supported(dir.path()) {
            eprintln!("skipped: this filesystem cannot clone");
            return;
        }
        let src = dir.path().join("src");
        std::fs::create_dir_all(src.join("pkg/bin")).unwrap();
        std::fs::write(src.join("pkg/index.js"), b"module.exports = 1;\n").unwrap();
        std::fs::write(src.join("empty"), b"").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink("../index.js", src.join("pkg/bin/run")).unwrap();

        let dst = dir.path().join("dst");
        clone_path(&src, &dst).unwrap();
        assert_eq!(
            std::fs::read(dst.join("pkg/index.js")).unwrap(),
            b"module.exports = 1;\n"
        );
        assert_eq!(std::fs::read(dst.join("empty")).unwrap(), b"");
        #[cfg(unix)]
        assert_eq!(
            std::fs::read_link(dst.join("pkg/bin/run")).unwrap(),
            Path::new("../index.js")
        );

        // Writes stay private to each side.
        std::fs::write(dst.join("pkg/index.js"), b"changed\n").unwrap();
        assert_eq!(
            std::fs::read(src.join("pkg/index.js")).unwrap(),
            b"module.exports = 1;\n"
        );

        assert!(clone_path(&src, &dst).is_err());
        assert_eq!(
            std::fs::read(dst.join("pkg/index.js")).unwrap(),
            b"changed\n"
        );
    }

    /// A failure part-way through a tree with read-only folders still
    /// leaves nothing behind.
    #[cfg(unix)]
    #[test]
    fn partial_read_only_trees_are_removed() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let partial = dir.path().join("partial");
        std::fs::create_dir_all(partial.join("locked/inner")).unwrap();
        std::fs::write(partial.join("locked/inner/file"), b"x").unwrap();
        for folder in ["locked/inner", "locked"] {
            std::fs::set_permissions(partial.join(folder), std::fs::Permissions::from_mode(0o555))
                .unwrap();
        }
        remove_partial(&partial).unwrap();
        assert!(partial.symlink_metadata().is_err());
    }

    #[test]
    fn a_missing_source_leaves_nothing_behind() {
        let dir = tempfile::tempdir().unwrap();
        let dst = dir.path().join("dst");
        assert!(clone_path(&dir.path().join("missing"), &dst).is_err());
        assert!(dst.symlink_metadata().is_err());
    }
}

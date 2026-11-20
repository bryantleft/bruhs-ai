//! Free space on a volume, for a job's space check (D6, D8).

use std::io;
use std::path::Path;

/// A job leaves at least this much free on the volume it writes to: 2 GiB.
pub const FREE_MARGIN: u64 = 2 << 30;

/// The bytes an unprivileged process may still write on the volume that
/// holds `path` (`f_bavail * f_frsize`). `path` must exist.
#[cfg(unix)]
pub fn free_bytes(path: &Path) -> io::Result<u64> {
    use std::ffi::CString;
    use std::mem::MaybeUninit;
    use std::os::unix::ffi::OsStrExt;

    let path = CString::new(path.as_os_str().as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "a path holds a NUL byte"))?;
    // SAFETY: `path` is a NUL-terminated string that outlives the call, and
    // `stat` is writable memory the size and alignment of one `statvfs`,
    // which the call fills on success. `statvfs` holds only integers, so
    // the zeroed value is initialized even where the call leaves a field
    // alone, and it is read only after the call returned 0.
    let stat = unsafe {
        let mut stat = MaybeUninit::<libc::statvfs>::zeroed();
        if libc::statvfs(path.as_ptr(), stat.as_mut_ptr()) != 0 {
            return Err(io::Error::last_os_error());
        }
        stat.assume_init()
    };
    widen(stat.f_bavail)
        .checked_mul(widen(stat.f_frsize))
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "free space overflows"))
}

/// Backup runs on macOS and Linux only (D27).
#[cfg(not(unix))]
pub fn free_bytes(_path: &Path) -> io::Result<u64> {
    Err(io::Error::from(io::ErrorKind::Unsupported))
}

/// A block count or size as `u64`: `u32` on some platforms, `u64` on
/// others.
#[cfg(unix)]
fn widen(value: impl Into<u64>) -> u64 {
    value.into()
}

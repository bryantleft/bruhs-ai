//! Free space on a volume, for a job's space check (D6, D8).

use std::io;
use std::path::Path;

/// A job leaves at least this much free on the volume it writes to: 2 GiB.
pub const FREE_MARGIN: u64 = 2 << 30;

/// The bytes an unprivileged process may still write on the volume that
/// holds `path`. `path` must exist.
///
/// macOS reads `statfs` (`f_bavail * f_bsize`), whose block counts are 64
/// bits wide; its `statvfs` has 32-bit counts that cap out above 16 TiB.
#[cfg(target_os = "macos")]
pub fn free_bytes(path: &Path) -> io::Result<u64> {
    let stat = rustix::fs::statfs(path)?;
    product(stat.f_bavail, stat.f_bsize)
}

/// The bytes an unprivileged process may still write on the volume that
/// holds `path` (`statvfs`: `f_bavail * f_frsize`). `path` must exist.
#[cfg(not(target_os = "macos"))]
pub fn free_bytes(path: &Path) -> io::Result<u64> {
    let stat = rustix::fs::statvfs(path)?;
    product(stat.f_bavail, stat.f_frsize)
}

/// `blocks * size` as `u64`. Each is `u32` or `u64` depending on the
/// platform.
fn product(blocks: impl Into<u64>, size: impl Into<u64>) -> io::Result<u64> {
    blocks
        .into()
        .checked_mul(size.into())
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "free space overflows"))
}

//! What deleting a worktree folder would free, from a replaceable meter.
//!
//! `du` and Finder count a copy-on-write clone at its full size, so they
//! overstate worktrees that share build folders with the checkout. A meter
//! reports what deletion actually frees:
//!
//! - `pith` (the macOS default): Pith's scanner, linked in, reads APFS
//!   private sizes and clone counts.
//! - `command`: any program that takes the folder as its last argument and
//!   prints Pith's `frees` JSON (`{"bytes":…,"shared":…,"snapshots":…,
//!   "unreadable":…}`; extra fields are ignored). Pith's own
//!   `storage-scanner frees` qualifies, as does a script around another tool.
//! - `allocated` (the Linux default): each file's allocated blocks, counted
//!   once per inode. Exact where files cannot share blocks (ext4 and the
//!   like); it refuses Btrfs, XFS, ZFS and APFS, where clones would make it
//!   overstate.
//! - `off` (the default elsewhere): no size is shown, rather than a wrong one.
//!
//! The choice lives in `worktree-meter.json` in bruhs-ai's data directory
//! (`BRUHS_WORKTREE_METER` names another file); see `docs/worktree-disk.md`.

use std::path::{Path, PathBuf};
use std::time::Duration;

use bruhs_models::coordination::worktree::WorktreeFrees;
use serde::{Deserialize, Serialize};
use tokio::io::AsyncReadExt;

/// Bytes of configuration read.
const CONFIG_LIMIT: u64 = 16 * 1024;
/// Bytes of a command meter's output read.
const OUTPUT_LIMIT: u64 = 64 * 1024;
/// A command meter that takes longer is stopped and reports nothing.
pub const COMMAND_TIMEOUT: Duration = Duration::from_secs(120);
/// At most this many extra arguments for a command meter.
const MAX_ARGS: usize = 16;

/// Which meter measures worktrees.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "meter", rename_all = "snake_case", deny_unknown_fields)]
pub enum MeterConfig {
    /// Pith's scanner, built in on macOS.
    Pith {},
    /// `command args… <folder>`, printing Pith's `frees` JSON.
    Command {
        command: PathBuf,
        args: Vec<String>,
    },
    /// Allocated blocks per inode; only where files cannot share blocks.
    Allocated {},
    // Empty braces, not unit variants: serde only rejects unknown fields
    // of an internally tagged enum's struct variants.
    Off {},
}

impl Default for MeterConfig {
    fn default() -> Self {
        if cfg!(target_os = "macos") {
            Self::Pith {}
        } else if cfg!(target_os = "linux") {
            Self::Allocated {}
        } else {
            Self::Off {}
        }
    }
}

impl MeterConfig {
    /// A command meter names an absolute program and plain arguments.
    pub fn validate(&self) -> Result<(), String> {
        let Self::Command { command, args } = self else {
            return Ok(());
        };
        if !command.is_absolute() {
            return Err("The meter command must be an absolute path.".into());
        }
        if args.len() > MAX_ARGS || args.iter().any(|arg| arg.contains('\0')) {
            return Err(format!(
                "The meter takes at most {MAX_ARGS} arguments, without NUL bytes."
            ));
        }
        Ok(())
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::Pith {} => "pith",
            Self::Command { .. } => "command",
            Self::Allocated {} => "allocated",
            Self::Off {} => "off",
        }
    }
}

/// The configuration file: `BRUHS_WORKTREE_METER`, else
/// `worktree-meter.json` in bruhs-ai's data directory.
pub fn config_path() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("BRUHS_WORKTREE_METER") {
        return Some(PathBuf::from(path));
    }
    directories::ProjectDirs::from("dev", "bruhs", "bruhs-ai")
        .map(|dirs| dirs.data_dir().join("worktree-meter.json"))
}

/// The configured meter. The platform default when there is no file; `Err`
/// when the file exists but cannot be used, so a typo is reported instead of
/// silently measuring with something else.
pub async fn load(path: Option<&Path>) -> Result<MeterConfig, String> {
    let Some(path) = path else {
        return Ok(MeterConfig::default());
    };
    let meta = match tokio::fs::metadata(path).await {
        Ok(meta) => meta,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(MeterConfig::default());
        }
        Err(error) => return Err(format!("Cannot read {}: {error}", path.display())),
    };
    if !meta.is_file() || meta.len() > CONFIG_LIMIT {
        return Err(format!(
            "{} must be a regular file of at most 16 KiB.",
            path.display()
        ));
    }
    let mut bytes = Vec::new();
    tokio::fs::File::open(path)
        .await
        .map_err(|error| format!("Cannot read {}: {error}", path.display()))?
        .take(CONFIG_LIMIT + 1)
        .read_to_end(&mut bytes)
        .await
        .map_err(|error| format!("Cannot read {}: {error}", path.display()))?;
    let config: MeterConfig = serde_json::from_slice(&bytes)
        .map_err(|error| format!("{} is invalid: {error}", path.display()))?;
    config.validate()?;
    Ok(config)
}

/// Pith's `frees` line; other meters may print only `bytes`.
#[derive(Deserialize)]
struct FreesLine {
    bytes: u64,
    #[serde(default)]
    shared: u64,
    #[serde(default)]
    snapshots: u64,
    #[serde(default)]
    unreadable: u64,
}

impl From<FreesLine> for WorktreeFrees {
    fn from(line: FreesLine) -> Self {
        Self {
            bytes: line.bytes,
            shared: line.shared,
            snapshots: line.snapshots,
            unreadable: line.unreadable,
        }
    }
}

/// What deleting `folder` frees, by `config`. `Ok(None)` when the meter is
/// off or unavailable on this platform.
pub async fn measure(config: &MeterConfig, folder: &Path) -> Result<Option<WorktreeFrees>, String> {
    match config {
        MeterConfig::Off {} => Ok(None),
        MeterConfig::Pith {} => pith(folder).await,
        MeterConfig::Allocated {} => {
            let folder = folder.to_path_buf();
            tokio::task::spawn_blocking(move || allocated(&folder).map(Some))
                .await
                .map_err(|error| format!("The allocated meter stopped: {error}"))?
        }
        MeterConfig::Command { command, args } => {
            run_command(command, args, folder).await.map(Some)
        }
    }
}

#[cfg(target_os = "macos")]
async fn pith(folder: &Path) -> Result<Option<WorktreeFrees>, String> {
    let paths = vec![folder.to_path_buf()];
    let frees = tokio::task::spawn_blocking(move || storage_scanner::frees::measure(&paths))
        .await
        .map_err(|error| format!("The Pith meter stopped: {error}"))?;
    Ok(Some(WorktreeFrees {
        bytes: frees.bytes,
        shared: frees.shared,
        snapshots: frees.snapshots,
        unreadable: frees.unreadable,
    }))
}

#[cfg(not(target_os = "macos"))]
async fn pith(_folder: &Path) -> Result<Option<WorktreeFrees>, String> {
    Ok(None)
}

async fn run_command(
    command: &Path,
    args: &[String],
    folder: &Path,
) -> Result<WorktreeFrees, String> {
    let mut builder = tokio::process::Command::new(command);
    builder
        .args(args)
        .arg(folder)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true);
    // Its own process group, so a timeout stops whatever a wrapper started.
    #[cfg(unix)]
    builder.process_group(0);
    let mut child = builder
        .spawn()
        .map_err(|error| format!("Cannot start {}: {error}", command.display()))?;
    // Stops the whole group if this future ends before the meter is reaped
    // (timeout, or the request was cancelled). Disarmed once it is reaped,
    // when its group id could belong to someone else.
    let mut guard = GroupGuard(child.id());
    let mut stdout = Vec::new();
    let read = async {
        if let Some(out) = child.stdout.take() {
            out.take(OUTPUT_LIMIT + 1).read_to_end(&mut stdout).await?;
        }
        child.wait().await
    };
    let status = tokio::time::timeout(COMMAND_TIMEOUT, read)
        .await
        .map_err(|_| {
            format!(
                "{} took longer than {}s.",
                command.display(),
                COMMAND_TIMEOUT.as_secs()
            )
        })?
        .map_err(|error| format!("{} failed: {error}", command.display()))?;
    guard.0 = None;
    if !status.success() {
        return Err(format!("{} exited with {status}.", command.display()));
    }
    if u64::try_from(stdout.len()).unwrap_or(u64::MAX) > OUTPUT_LIMIT {
        return Err(format!("{} printed more than 64 KiB.", command.display()));
    }
    // The last non-empty line is the result; a meter may print progress first.
    let line = String::from_utf8_lossy(&stdout);
    let line = line
        .lines()
        .rev()
        .find(|line| !line.trim().is_empty())
        .ok_or_else(|| format!("{} printed nothing.", command.display()))?;
    serde_json::from_str::<FreesLine>(line)
        .map(Into::into)
        .map_err(|error| format!("{} printed no frees result: {error}", command.display()))
}

/// Kills the meter's process group when dropped while armed.
struct GroupGuard(Option<u32>);

impl Drop for GroupGuard {
    fn drop(&mut self) {
        kill_group(self.0.take());
    }
}

/// Stop the meter's whole process group after a timeout.
#[cfg(unix)]
fn kill_group(group: Option<u32>) {
    let Some(group) = group.and_then(|pid| libc::pid_t::try_from(pid).ok()) else {
        return;
    };
    // SAFETY: killpg takes plain integers. The group was created for this
    // meter by `process_group(0)`, so it holds only the meter and what it
    // started.
    if unsafe { libc::killpg(group, libc::SIGKILL) } != 0 {
        tracing::debug!("meter process group {group} was already gone");
    }
}

#[cfg(not(unix))]
fn kill_group(_group: Option<u32>) {}

/// Entries the allocated meter reads before giving up.
#[cfg(unix)]
const ALLOCATED_ENTRIES: u32 = 5_000_000;

/// Allocated blocks under `folder`, once per inode, without crossing into
/// other volumes. A hard-linked file with a link outside counts as shared.
#[cfg(unix)]
fn allocated(folder: &Path) -> Result<WorktreeFrees, String> {
    use std::os::unix::fs::MetadataExt;
    refuse_shared_blocks(folder)?;
    let top = std::fs::symlink_metadata(folder)
        .map_err(|error| format!("Cannot read {}: {error}", folder.display()))?;
    let device = top.dev();
    let mut frees = WorktreeFrees {
        bytes: top.blocks().saturating_mul(512),
        ..WorktreeFrees::default()
    };
    // (device, inode) → (links seen here, links in all, bytes).
    let mut links = std::collections::HashMap::<(u64, u64), (u64, u64, u64)>::new();
    let mut dirs = vec![folder.to_path_buf()];
    let mut seen = 0u32;
    while let Some(dir) = dirs.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            frees.unreadable += 1;
            continue;
        };
        for entry in entries.flatten() {
            seen += 1;
            if seen > ALLOCATED_ENTRIES {
                return Err("This worktree holds too many files to measure.".into());
            }
            // `DirEntry::metadata` does not follow symlinks.
            let Ok(meta) = entry.metadata() else {
                continue;
            };
            if meta.dev() != device {
                continue;
            }
            let bytes = meta.blocks().saturating_mul(512);
            if meta.is_dir() {
                frees.bytes = frees.bytes.saturating_add(bytes);
                dirs.push(entry.path());
            } else if meta.nlink() > 1 {
                let seen =
                    links
                        .entry((meta.dev(), meta.ino()))
                        .or_insert((0, meta.nlink(), bytes));
                seen.0 += 1;
            } else {
                frees.bytes = frees.bytes.saturating_add(bytes);
            }
        }
    }
    for (seen, all, bytes) in links.into_values() {
        if seen >= all {
            frees.bytes = frees.bytes.saturating_add(bytes);
        } else {
            frees.shared = frees.shared.saturating_add(bytes);
        }
    }
    Ok(frees)
}

#[cfg(not(unix))]
fn allocated(_folder: &Path) -> Result<WorktreeFrees, String> {
    Err("The allocated meter needs macOS or Linux.".into())
}

/// Filesystems whose files can share blocks, where allocated sizes overstate
/// what deleting frees.
#[cfg(unix)]
fn refuse_shared_blocks(folder: &Path) -> Result<(), String> {
    if cfg!(target_os = "macos") {
        return Err("APFS shares blocks between clones, so allocated sizes overstate what removing frees. Use the pith meter.".into());
    }
    let mountinfo = std::fs::read_to_string("/proc/self/mountinfo").unwrap_or_default();
    match filesystem_of(&mountinfo, &std::fs::canonicalize(folder).unwrap_or_default()) {
        Some(kind) if ["btrfs", "xfs", "zfs", "bcachefs"].contains(&kind.as_str()) => Err(format!(
            "{kind} can share blocks between files, so allocated sizes overstate what removing frees. Set a command meter instead."
        )),
        _ => Ok(()),
    }
}

/// A mountinfo path with its octal escapes (`\\040` space, `\\011` tab,
/// `\\012` newline, `\\134` backslash) decoded.
fn unescape_mount(field: &str) -> String {
    let bytes = field.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let octal = bytes.get(i + 1..i + 4).filter(|digits| {
            bytes[i] == b'\\' && digits.iter().all(|digit| (b'0'..=b'7').contains(digit))
        });
        match octal.and_then(|digits| u8::from_str_radix(std::str::from_utf8(digits).ok()?, 8).ok())
        {
            Some(byte) => {
                out.push(byte);
                i += 4;
            }
            None => {
                out.push(bytes[i]);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The filesystem type of the mount holding `path`, from
/// `/proc/self/mountinfo`: the longest mount point that contains it.
fn filesystem_of(mountinfo: &str, path: &Path) -> Option<String> {
    mountinfo
        .lines()
        .filter_map(|line| {
            let (left, right) = line.split_once(" - ")?;
            let mount = unescape_mount(left.split(' ').nth(4)?);
            let kind = right.split(' ').next()?;
            Some((PathBuf::from(mount), kind.to_string()))
        })
        .filter(|(mount, _)| path.starts_with(mount))
        .max_by_key(|(mount, _)| mount.as_os_str().len())
        .map(|(_, kind)| kind)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn config_defaults_per_platform_and_rejects_typos() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("worktree-meter.json");
        assert_eq!(load(Some(&file)).await.unwrap(), MeterConfig::default());
        assert_eq!(
            MeterConfig::default().name(),
            if cfg!(target_os = "macos") {
                "pith"
            } else if cfg!(target_os = "linux") {
                "allocated"
            } else {
                "off"
            }
        );

        std::fs::write(&file, r#"{"meter":"off"}"#).unwrap();
        assert_eq!(load(Some(&file)).await.unwrap(), MeterConfig::Off {});
        std::fs::write(
            &file,
            r#"{"meter":"command","command":"/usr/bin/true","args":["frees"]}"#,
        )
        .unwrap();
        assert!(matches!(
            load(Some(&file)).await.unwrap(),
            MeterConfig::Command { .. }
        ));

        for bad in [
            r#"{"meter":"du"}"#,
            r#"{"meter":"off","extra":1}"#,
            r#"{"meter":"command","command":"relative/meter","args":[]}"#,
            "{ not json",
        ] {
            std::fs::write(&file, bad).unwrap();
            assert!(load(Some(&file)).await.is_err(), "{bad}");
        }
    }

    /// Any program printing Pith's `frees` line can be the meter; it gets
    /// the folder as its last argument.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_command_meter_reads_the_last_line_and_its_failures_are_reported() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("meter");
        std::fs::write(
            &script,
            "#!/bin/sh\necho progress\n[ -d \"$2\" ] || exit 3\necho '{\"type\":\"frees\",\"bytes\":42,\"shared\":7,\"files\":1,\"elapsedMs\":0}'\n",
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        let config = MeterConfig::Command {
            command: script.clone(),
            args: vec!["frees".into()],
        };
        let frees = measure(&config, dir.path()).await.unwrap().unwrap();
        assert_eq!(
            frees,
            WorktreeFrees {
                bytes: 42,
                shared: 7,
                snapshots: 0,
                unreadable: 0
            }
        );
        assert!(measure(&config, &dir.path().join("missing")).await.is_err());
        assert_eq!(
            measure(&MeterConfig::Off {}, dir.path()).await.unwrap(),
            None
        );
    }

    /// The built-in Pith meter counts a clone shared with another folder as
    /// shared, not freed.
    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn pith_counts_clones_as_shared() {
        let dir = tempfile::tempdir().unwrap();
        let (checkout, worktree) = (dir.path().join("checkout"), dir.path().join("worktree"));
        std::fs::create_dir_all(&checkout).unwrap();
        std::fs::write(checkout.join("big.bin"), vec![7u8; 4 * 1024 * 1024]).unwrap();
        let status = std::process::Command::new("cp")
            .args(["-cR"])
            .arg(&checkout)
            .arg(&worktree)
            .status()
            .unwrap();
        assert!(status.success());
        let frees = measure(&MeterConfig::Pith {}, &worktree)
            .await
            .unwrap()
            .unwrap();
        if frees.shared == 0 {
            eprintln!("skipped: this volume did not clone");
            return;
        }
        assert!(frees.bytes < 1024 * 1024, "{frees:?}");
        assert!(frees.shared >= 4 * 1024 * 1024, "{frees:?}");
    }

    #[test]
    fn mount_paths_decode_every_octal_escape() {
        assert_eq!(unescape_mount("/mnt/a\\040b\\011c\\134d"), "/mnt/a b\tc\\d");
        assert_eq!(unescape_mount("/mnt/plain\\9"), "/mnt/plain\\9");
    }

    #[test]
    fn mountinfo_names_the_closest_mount() {
        let info = "22 1 8:1 / / rw - ext4 /dev/sda1 rw\n40 22 0:40 / /home/me/wt rw - btrfs /dev/sdb rw\n";
        assert_eq!(
            filesystem_of(info, Path::new("/home/me/wt/proj")).as_deref(),
            Some("btrfs")
        );
        assert_eq!(
            filesystem_of(info, Path::new("/srv/x")).as_deref(),
            Some("ext4")
        );
    }

    /// Where files cannot share blocks, allocated sizes are exact, and a hard
    /// link reaching outside the folder counts as shared.
    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn allocated_counts_each_inode_once() {
        let dir = tempfile::tempdir().unwrap();
        let (folder, outside) = (dir.path().join("wt"), dir.path().join("outside"));
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join("a"), vec![1u8; 1 << 20]).unwrap();
        std::fs::hard_link(folder.join("a"), folder.join("a-again")).unwrap();
        std::fs::write(folder.join("b"), vec![2u8; 1 << 20]).unwrap();
        std::fs::hard_link(folder.join("b"), &outside).unwrap();
        match measure(&MeterConfig::Allocated {}, &folder).await {
            Ok(Some(frees)) => {
                assert!(frees.bytes >= 1 << 20 && frees.bytes < 2 << 20, "{frees:?}");
                assert!(frees.shared >= 1 << 20, "{frees:?}");
            }
            Err(refused) => assert!(refused.contains("share blocks"), "{refused}"),
            Ok(None) => panic!("the allocated meter returned nothing"),
        }
    }

    /// A meter that hangs and started a child of its own is stopped whole.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_timed_out_meter_takes_its_children_with_it() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let pid_file = dir.path().join("child.pid");
        let script = dir.path().join("meter");
        std::fs::write(
            &script,
            format!(
                "#!/bin/sh\nsleep 300 &\necho $! > {}\nwait\n",
                pid_file.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        let mut child = tokio::process::Command::new(&script);
        child.process_group(0).stdout(std::process::Stdio::piped());
        let mut child = child.spawn().unwrap();
        let group = child.id();
        for _ in 0..100 {
            if pid_file.exists()
                && !std::fs::read_to_string(&pid_file)
                    .unwrap()
                    .trim()
                    .is_empty()
            {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        let grandchild: libc::pid_t = std::fs::read_to_string(&pid_file)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        kill_group(group);
        child.wait().await.unwrap();
        let mut gone = false;
        for _ in 0..100 {
            // SAFETY: signal 0 only checks whether the process exists.
            if unsafe { libc::kill(grandchild, 0) } != 0 {
                gone = true;
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        assert!(gone, "the meter's child survived");
    }
}

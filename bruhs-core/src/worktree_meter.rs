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
    // Empty braces, not unit variants: serde only rejects unknown fields
    // of an internally tagged enum's struct variants.
    Off {},
}

impl Default for MeterConfig {
    fn default() -> Self {
        if cfg!(target_os = "macos") {
            Self::Pith {}
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
    let mut child = tokio::process::Command::new(command)
        .args(args)
        .arg(folder)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .map_err(|error| format!("Cannot start {}: {error}", command.display()))?;
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
}

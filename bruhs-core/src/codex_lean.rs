//! Which Codex sessions bruhs started lean (no plugins).
//!
//! A Codex session must keep the plugin setting it started with: resuming a
//! session that began with plugins without them made Codex insert ~11k
//! tokens of instructions, and every later turn cost 35k instead of 24k
//! (measured, codex-cli 0.166). Codex records nothing about the setting, so
//! bruhs keeps the ids of the sessions it started lean in one small file in
//! its local data dir, shared by the server and the desktop app: a resumed
//! session is lean exactly when it is listed.

use std::io::Write as _;
use std::path::{Path, PathBuf};

/// Overrides where the list lives (tests, side-by-side profiles).
const FILE_ENV: &str = "BRUHS_CODEX_LEAN_SESSIONS_FILE";
const FILE_NAME: &str = "codex-lean-sessions";
/// Ids kept; older ones fall off when the file outgrows this. A forgotten
/// session resumes with plugins: one costlier turn per old thread, never a
/// failure.
const MAX_IDS: usize = 4096;

/// The list's path: `BRUHS_CODEX_LEAN_SESSIONS_FILE`, else
/// `<local data dir>/codex-lean-sessions`.
pub(crate) fn default_file() -> Option<PathBuf> {
    std::env::var_os(FILE_ENV)
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            directories::ProjectDirs::from("dev", "bruhs", "bruhs-ai")
                .map(|dirs| dirs.data_local_dir().join(FILE_NAME))
        })
}

/// Whether a Codex turn runs lean. A new session follows `lean`, but only
/// when there is a list to remember it in; a resumed one keeps what it
/// started with, whatever `lean` says now.
pub(crate) fn turn_is_lean(file: Option<&Path>, resume: Option<&str>, lean: bool) -> bool {
    match (file, resume) {
        (None, _) => false,
        (Some(_), None) => lean,
        (Some(file), Some(id)) => contains(file, id),
    }
}

/// Whether `id` is listed in `file`. A missing or unreadable file lists
/// nothing.
pub(crate) fn contains(file: &Path, id: &str) -> bool {
    std::fs::read_to_string(file).is_ok_and(|text| text.lines().any(|line| line == id))
}

/// List `id` in `file`, keeping the newest [`MAX_IDS`].
///
/// The server and the desktop app may record at the same moment, so the
/// check, the append and the prune run under an exclusive lock on a sidecar
/// file (not the list itself, which prune replaces).
///
/// # Errors
///
/// When the file, its lock or its directory cannot be written.
pub(crate) fn record(file: &Path, id: &str) -> std::io::Result<()> {
    if id.is_empty() || id.contains('\n') {
        return Ok(());
    }
    let dir = file.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(dir)?;
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(file.with_extension("lock"))?;
    lock.lock()?;
    if contains(file, id) {
        return Ok(());
    }
    let mut out = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(file)?;
    out.write_all(format!("{id}\n").as_bytes())?;
    drop(out);
    prune(file, dir)
    // `lock` drops here, releasing the lock.
}

/// Drop the oldest ids once there are more than [`MAX_IDS`] (written to a
/// fresh temporary file and renamed, so a reader never sees half a list).
/// Called with the lock held.
fn prune(file: &Path, dir: &Path) -> std::io::Result<()> {
    let text = std::fs::read_to_string(file)?;
    let ids: Vec<&str> = text.lines().filter(|line| !line.is_empty()).collect();
    if ids.len() <= MAX_IDS {
        return Ok(());
    }
    let kept = ids[ids.len() - MAX_IDS..].join("\n") + "\n";
    let mut temp = tempfile::NamedTempFile::new_in(dir)?;
    temp.write_all(kept.as_bytes())?;
    temp.persist(file).map_err(|error| error.error)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_resumed_session_keeps_the_mode_it_started_with() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("nested").join(FILE_NAME);
        assert!(turn_is_lean(Some(&file), None, true), "new: follows lean");
        assert!(!turn_is_lean(Some(&file), None, false));
        assert!(
            !turn_is_lean(Some(&file), Some("old"), true),
            "started before bruhs ran Codex lean: stays full"
        );

        record(&file, "s-1").unwrap();
        record(&file, "s-1").unwrap();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "s-1\n", "once");
        assert!(turn_is_lean(Some(&file), Some("s-1"), true));
        assert!(
            turn_is_lean(Some(&file), Some("s-1"), false),
            "a lean session stays lean even when lean turns are turned off"
        );
        assert!(!turn_is_lean(None, Some("s-1"), true), "no file, no list");
        assert!(
            !turn_is_lean(None, None, true),
            "a session that cannot be remembered never starts lean"
        );
    }

    /// The server and the desktop record at once, with the list full: no
    /// id may be lost (unlocked, 2 of 400 were).
    #[test]
    fn concurrent_records_keep_every_id() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join(FILE_NAME);
        let old: String = (0..MAX_IDS).map(|i| format!("old-{i}\n")).collect();
        std::fs::write(&file, old).unwrap();
        let writers: Vec<_> = ["a", "b"]
            .into_iter()
            .map(|name| {
                let file = file.clone();
                std::thread::spawn(move || {
                    for i in 0..100 {
                        record(&file, &format!("{name}-{i}")).unwrap();
                    }
                })
            })
            .collect();
        for writer in writers {
            writer.join().unwrap();
        }
        let text = std::fs::read_to_string(&file).unwrap();
        let ids: Vec<&str> = text.lines().collect();
        assert_eq!(ids.len(), MAX_IDS);
        for name in ["a", "b"] {
            for i in 0..100 {
                assert!(ids.contains(&format!("{name}-{i}").as_str()), "{name}-{i}");
            }
        }
    }

    #[test]
    fn the_list_keeps_the_newest_ids() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join(FILE_NAME);
        let old: String = (0..MAX_IDS).map(|i| format!("id-{i}\n")).collect();
        std::fs::write(&file, old).unwrap();
        record(&file, "newest").unwrap();
        assert!(!contains(&file, "id-0"));
        assert!(contains(&file, "id-1") && contains(&file, "newest"));
        assert_eq!(
            std::fs::read_to_string(&file).unwrap().lines().count(),
            MAX_IDS
        );
    }
}

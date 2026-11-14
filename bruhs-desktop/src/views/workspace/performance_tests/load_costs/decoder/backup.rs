//! Recovery must decode a backup without adopting or calculating its disk version.
use super::super::super::super::{load_desktop_state_from_path, SaveError};
use super::*;

mod reference;

fn assert_path_readers_match(path: &Path) {
    assert_eq!(
        load_desktop_state_from_path(path).map(|state| state.map(state_value)),
        reference::load_original_state_from_path(path).map(|state| state.map(state_value))
    );
    assert_eq!(
        read_desktop_state_snapshot(path).map(|(state, version)| (state.map(state_value), version)),
        reference::read_original_state_snapshot(path)
            .map(|(state, version)| (state.map(state_value), version))
    );
}

fn assert_recovery_matches(path: &Path) -> DiskVersion {
    let expected = reference::load_original_state_with_backup(path);
    let actual = load_desktop_state_with_backup(path);
    assert_eq!(actual.0.map(state_value), expected.0.map(state_value));
    assert_eq!(actual.1, expected.1);
    assert_eq!(actual.2, expected.2);
    actual.2
}

#[test]
fn unversioned_load_retains_full_state_and_primary_raw_byte_versions() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("saved café.json");
    for label in [
        "thread-rich-small",
        "unicode-pretty",
        "ascii-compact",
        "escaped-pretty",
    ] {
        let bytes = fixture_bytes(label);
        fs::write(&path, &bytes).unwrap();
        assert_path_readers_match(&path);
        let actual = load_desktop_state_from_path(&path).unwrap().unwrap();
        let expected: PersistedWorkspaceState = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(
            serde_json::to_value(actual).unwrap(),
            serde_json::to_value(expected).unwrap()
        );
        assert_eq!(
            read_desktop_state_snapshot(&path).unwrap().1,
            DiskVersion::of_bytes(&bytes)
        );
        assert_eq!(fs::read(&path).unwrap(), bytes);
    }
    let state = fixture_state("thread-rich-small");
    let compact = serde_json::to_vec(&state).unwrap();
    let pretty = serde_json::to_vec_pretty(&state).unwrap();
    fs::write(&path, &compact).unwrap();
    let compact_version = read_desktop_state_snapshot(&path).unwrap().1;
    fs::write(&path, &pretty).unwrap();
    let pretty_version = read_desktop_state_snapshot(&path).unwrap().1;
    assert_ne!(
        compact_version, pretty_version,
        "version hashes the exact saved bytes"
    );
}

#[test]
fn backup_recovery_remains_untrusted_and_cannot_authorize_an_overwrite() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("workspace.json");
    let backup = desktop_state_backup_path(&path);
    let bytes = fixture_bytes("thread-rich-small");
    fs::write(&backup, &bytes).unwrap();
    for primary in [
        &b"{ invalid primary"[..],
        &b""[..],
        &b"{\"current_model\":\"\xff\"}"[..],
    ] {
        fs::write(&path, primary).unwrap();
        assert_path_readers_match(&path);
        let mut version = assert_recovery_matches(&path);
        assert_eq!(version, DiskVersion::Untrusted);
        assert!(matches!(
            state_file::guarded_write(&path, &mut version, b"candidate", |_| panic!(
                "recovery must not authorize writing"
            )),
            Err(SaveError::Conflict)
        ));
        assert_eq!(version, DiskVersion::Untrusted);
        assert_eq!(fs::read(&path).unwrap(), primary);
        assert_eq!(fs::read(&backup).unwrap(), bytes);
    }
}

#[test]
fn state_readers_preserve_missing_primary_preference_and_combined_errors() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("workspace.json");
    let backup = desktop_state_backup_path(&path);
    let bytes = fixture_bytes("thread-rich-small");
    fs::write(&path, &bytes).unwrap();
    fs::write(&backup, b"{ bad backup").unwrap();
    assert_eq!(
        assert_recovery_matches(&path),
        DiskVersion::of_bytes(&bytes)
    );
    fs::remove_file(&path).unwrap();
    fs::write(&backup, &bytes).unwrap();
    assert_path_readers_match(&path);
    assert_eq!(assert_recovery_matches(&path), DiskVersion::Missing);
    assert!(load_desktop_state_with_backup(&path).0.is_none());
    fs::write(&path, b"{ bad primary").unwrap();
    fs::remove_file(&backup).unwrap();
    assert_eq!(assert_recovery_matches(&path), DiskVersion::Untrusted);
    fs::write(&backup, b"{ bad backup").unwrap();
    assert_eq!(assert_recovery_matches(&path), DiskVersion::Untrusted);
    let result = load_desktop_state_with_backup(&path);
    assert!(result.0.is_none());
    let message = result.1.unwrap();
    assert!(
        message.contains("workspace.json is invalid")
            && message.contains("workspace.json.bak is invalid")
    );
    assert_eq!(fs::read(&path).unwrap(), b"{ bad primary");
    assert_eq!(fs::read(&backup).unwrap(), b"{ bad backup");
}

#[test]
fn state_readers_reject_directories_and_never_follow_recovery_symlinks() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("workspace.json");
    let backup = desktop_state_backup_path(&path);
    let bytes = fixture_bytes("thread-rich-small");
    fs::create_dir(&path).unwrap();
    fs::write(&backup, &bytes).unwrap();
    assert_path_readers_match(&path);
    assert_eq!(assert_recovery_matches(&path), DiskVersion::Untrusted);
    fs::remove_dir(&path).unwrap();
    fs::write(&path, b"{").unwrap();
    fs::remove_file(&backup).unwrap();
    fs::create_dir(&backup).unwrap();
    assert_eq!(assert_recovery_matches(&path), DiskVersion::Untrusted);
    fs::remove_dir(&backup).unwrap();
    #[cfg(unix)]
    {
        let target = dir.path().join("retained.json");
        fs::write(&target, &bytes).unwrap();
        fs::remove_file(&path).unwrap();
        std::os::unix::fs::symlink(&target, &path).unwrap();
        fs::write(&backup, &bytes).unwrap();
        assert_path_readers_match(&path);
        assert_eq!(assert_recovery_matches(&path), DiskVersion::Untrusted);
        assert!(fs::symlink_metadata(&path)
            .unwrap()
            .file_type()
            .is_symlink());
        assert_eq!(fs::read(&target).unwrap(), bytes);
        fs::remove_file(&path).unwrap();
        fs::write(&path, b"{").unwrap();
        fs::remove_file(&backup).unwrap();
        std::os::unix::fs::symlink(&target, &backup).unwrap();
        assert_eq!(assert_recovery_matches(&path), DiskVersion::Untrusted);
        assert!(load_desktop_state_with_backup(&path).0.is_none());
        assert!(fs::symlink_metadata(&backup)
            .unwrap()
            .file_type()
            .is_symlink());
        assert_eq!(fs::read(&target).unwrap(), bytes);
    }
}

#[gpui::test]
#[ignore = "manual optimized complete-file backup hash comparison through shared construction"]
fn complete_workspace_backup_hash_load_cost(cx: &mut TestAppContext) {
    initialize(cx);
    let (variant, load): (&str, LoadState) =
        match std::env::var("BRUHS_PERF_BACKUP_HASH").as_deref() {
            Ok("original") => ("original", reference::load_original_state_with_backup),
            Ok("current") | Err(_) => ("current", load_desktop_state_with_backup),
            other => panic!("invalid backup hash selection {other:?}"),
        };
    let selected = std::env::var("BRUHS_PERF_BACKUP_CASE").ok();
    measure_load_cases(
        cx,
        variant,
        load,
        reference::load_original_state_with_backup,
        selected.as_deref(),
        "BACKUP_LOAD_COST",
    );
}

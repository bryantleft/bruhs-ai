use super::*;

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("bruhs-state-lock-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
    fn state(&self) -> PathBuf {
        self.0.join("state.json")
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn stale_save_preserves_newer_bytes_and_its_original_version() {
    let fixture = Fixture::new();
    let path = fixture.state();
    fs::write(&path, b"initial").unwrap();
    let mut first = DiskVersion::read(&path).unwrap();
    let mut stale = first.clone();
    guarded_write(&path, &mut first, b"newer", |bytes| fs::write(&path, bytes)).unwrap();
    let result = guarded_write(&path, &mut stale, b"stale", |bytes| fs::write(&path, bytes));
    assert!(matches!(result, Err(SaveError::Conflict)));
    assert_eq!(fs::read(&path).unwrap(), b"newer");
    assert_eq!(stale, DiskVersion::of_bytes(b"initial"));
}

#[test]
fn deleted_state_cannot_be_recreated_from_a_stale_snapshot() {
    let fixture = Fixture::new();
    let path = fixture.state();
    fs::write(&path, b"initial").unwrap();
    let mut version = DiskVersion::read(&path).unwrap();
    fs::remove_file(&path).unwrap();
    assert!(matches!(
        guarded_write(&path, &mut version, b"stale", |bytes| fs::write(
            &path, bytes
        )),
        Err(SaveError::Conflict)
    ));
    assert!(!path.exists());
}

#[test]
fn error_after_replacement_remains_an_error_but_retry_recognizes_own_bytes() {
    let fixture = Fixture::new();
    let path = fixture.state();
    let mut version = DiskVersion::Missing;
    let result = guarded_write(&path, &mut version, b"candidate", |bytes| {
        fs::write(&path, bytes)?;
        Err(io::Error::other("directory sync failed"))
    });
    assert!(matches!(result, Err(SaveError::Io(_))));
    assert_eq!(version, DiskVersion::of_bytes(b"candidate"));
    guarded_write(&path, &mut version, b"retry", |bytes| {
        fs::write(&path, bytes)
    })
    .unwrap();
    assert_eq!(fs::read(&path).unwrap(), b"retry");
}

#[test]
fn held_lock_rejects_a_second_handle_without_running_the_write() {
    let fixture = Fixture::new();
    let path = fixture.state();
    let held = acquire_lock(&path).unwrap();
    assert!(matches!(
        guarded_write(&path, &mut DiskVersion::Missing, b"candidate", |_| panic!(
            "must not write"
        )),
        Err(SaveError::Busy)
    ));
    drop(held);
    guarded_write(&path, &mut DiskVersion::Missing, b"candidate", |bytes| {
        fs::write(&path, bytes)
    })
    .unwrap();
}

#[test]
fn releasing_writer_does_not_leave_its_lock_on_an_inherited_descriptor() {
    let fixture = Fixture::new();
    let path = fixture.state();
    let held = acquire_lock(&path).unwrap();
    // Model a descriptor duplicated during another thread's process spawn.
    let inherited = held.file.try_clone().unwrap();
    drop(held);
    let next = acquire_lock(&path).expect("completed writer must release its lock explicitly");
    drop(next);
    drop(inherited);
}

#[test]
#[cfg(unix)]
fn symlink_state_is_not_followed_or_replaced() {
    let fixture = Fixture::new();
    let target = fixture.0.join("original");
    fs::write(&target, b"original").unwrap();
    let path = fixture.state();
    std::os::unix::fs::symlink(&target, &path).unwrap();
    assert!(
        guarded_write(&path, &mut DiskVersion::Missing, b"candidate", |_| panic!(
            "must not write"
        ))
        .is_err()
    );
    assert_eq!(fs::read(&target).unwrap(), b"original");
    assert!(fs::symlink_metadata(&path)
        .unwrap()
        .file_type()
        .is_symlink());
}

/// Runs in a separate test-harness process when requested by ProcessFixture.
#[test]
fn subprocess_fixture() {
    use super::super::{
        read_desktop_state_snapshot, save_desktop_state, PersistedWorkspaceState,
        PersistenceCoordinator,
    };
    use std::io::{BufRead, Write};
    let Some(path) = std::env::var_os("BRUHS_STATE_TEST_PATH") else {
        return;
    };
    let path = PathBuf::from(path);
    let mode = std::env::var("BRUHS_STATE_TEST_MODE").unwrap();
    let held = (mode == "hold").then(|| acquire_lock(&path).unwrap());
    let (state, version) = read_desktop_state_snapshot(&path).unwrap();
    let coordinator = std::sync::Arc::new(PersistenceCoordinator {
        latest_generation: std::sync::atomic::AtomicU64::new(1),
        write_lock: std::sync::Mutex::new(version),
    });
    println!("READY");
    io::stdout().flush().unwrap();
    let mut command = String::new();
    io::stdin().lock().read_line(&mut command).unwrap();
    if mode == "hold" {
        drop(held);
        return;
    }
    let state = PersistedWorkspaceState {
        current_model: Some(mode),
        ..state.unwrap_or_default()
    };
    let result = save_desktop_state(Some(path), state, coordinator, 1);
    println!(
        "RESULT:{}",
        match result {
            Ok(()) => "saved",
            Err(SaveError::Conflict) => "conflict",
            Err(SaveError::Busy) => "busy",
            Err(error) => panic!("unexpected writer error: {error}"),
        }
    );
}

struct ProcessFixture {
    child: std::process::Child,
    output: std::sync::mpsc::Receiver<String>,
}

impl ProcessFixture {
    fn start(path: &Path, mode: &str) -> Self {
        use std::{
            io::BufRead,
            process::{Command, Stdio},
        };
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "views::workspace::state_file::tests::subprocess_fixture",
                "--nocapture",
            ])
            .env("BRUHS_STATE_TEST_PATH", path)
            .env("BRUHS_STATE_TEST_MODE", mode)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        let stdout = child.stdout.take().unwrap();
        let (tx, output) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            for line in io::BufReader::new(stdout).lines() {
                if tx.send(line.unwrap()).is_err() {
                    break;
                }
            }
        });
        let fixture = Self { child, output };
        fixture.message("READY");
        fixture
    }

    fn message(&self, prefix: &str) -> String {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            let line = self
                .output
                .recv_timeout(deadline.saturating_duration_since(std::time::Instant::now()))
                .expect("child must respond within 10 seconds");
            if line.starts_with(prefix) {
                return line;
            }
        }
    }

    fn release(&mut self) {
        use std::io::Write;
        self.child
            .stdin
            .as_mut()
            .unwrap()
            .write_all(b"write\n")
            .unwrap();
    }
}

impl Drop for ProcessFixture {
    fn drop(&mut self) {
        // Only this fixture's child, including cleanup after a failed assertion.
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
fn simultaneous_processes_commit_only_one_complete_snapshot() {
    let fixture = Fixture::new();
    let path = fixture.state();
    let mut first = ProcessFixture::start(&path, "first");
    let mut second = ProcessFixture::start(&path, "second");
    first.release();
    second.release();
    let outcomes = [first.message("RESULT:"), second.message("RESULT:")];
    assert_eq!(
        outcomes
            .iter()
            .filter(|result| *result == "RESULT:saved")
            .count(),
        1
    );
    assert!(outcomes
        .iter()
        .any(|result| result == "RESULT:busy" || result == "RESULT:conflict"));
    let bytes = fs::read(&path).unwrap();
    let state: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert!(state["current_model"] == "first" || state["current_model"] == "second");
}

#[test]
fn separate_stale_process_cannot_overwrite_or_rotate_backup() {
    let fixture = Fixture::new();
    let path = fixture.state();
    let mut stale = ProcessFixture::start(&path, "stale");
    let mut first = ProcessFixture::start(&path, "newer");
    first.release();
    assert_eq!(first.message("RESULT:"), "RESULT:saved");
    let newer = fs::read(&path).unwrap();
    stale.release();
    assert_eq!(stale.message("RESULT:"), "RESULT:conflict");
    assert_eq!(fs::read(&path).unwrap(), newer);
    assert!(!path.with_file_name("state.json.bak").exists());
}

#[test]
fn terminated_process_releases_lock_without_removing_the_sidecar() {
    let fixture = Fixture::new();
    let path = fixture.state();
    let held = ProcessFixture::start(&path, "hold");
    assert!(matches!(acquire_lock(&path), Err(SaveError::Busy)));
    drop(held);
    assert!(lock_path(&path).is_file());
    let mut next = ProcessFixture::start(&path, "after-exit");
    next.release();
    assert_eq!(next.message("RESULT:"), "RESULT:saved");
}

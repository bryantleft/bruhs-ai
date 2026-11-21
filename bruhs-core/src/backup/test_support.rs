//! A restore waiting to be applied, for the apps' own tests (the server's
//! applier, the desktop's instance lock). Built with the real make, open
//! and schedule steps over the caller's temporary roots, with fast key
//! derivation and a capture daemon that isn't recording, so nothing here
//! reads the owner's data, keychain or daemon.

use std::path::Path;
use std::time::Duration;

use bruhs_models::backup::{
    BackupApp, BackupOptions, JobState, PendingRestore, RestoreSource, DAMAGED,
};
use zeroize::Zeroizing;

use super::{schedule, BackupError, BackupHost, CreateRequest, Output, Roots};
use crate::secrets::SecretStore;

/// `<core>/notes/note.txt` holds this once the restore is applied.
pub const RESTORED: &str = "restored";
/// And this until then.
pub const TODAY: &str = "today";
/// The file both texts live in, under the core root.
pub const NOTE: &str = "notes/note.txt";

const PASSWORD: &str = "correct horse 1";
const BACKUP_FILE: &str = "restore-for-tests.bruhsbak";

/// Backs up [`RESTORED`] in `<core>/notes/note.txt` (and a desktop state
/// file), writes [`TODAY`] over it, stages the backup and schedules it.
/// The roots must exist; the backup file goes in `roots.home`. Keychain
/// items stay out. Builds its own runtime, so call it outside one.
pub fn stage_restore(
    roots: &Roots,
    store: &SecretStore,
    service_to_restart: bool,
) -> Result<PendingRestore, BackupError> {
    write_data(roots, RESTORED)?;
    let host = BackupHost::new(roots.clone(), store.clone()).for_tests(false);
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let path = roots.home.join(BACKUP_FILE);
    let job = runtime.block_on(host.start_create(CreateRequest {
        password: Zeroizing::new(PASSWORD.into()),
        options: BackupOptions {
            keychain: false,
            audio: true,
            browser_profiles: false,
        },
        app: BackupApp::Desktop,
        web_part: None,
        web_db_version: None,
        output: Output::File {
            path: path.clone(),
            overwrite: true,
        },
    }))?;
    let JobState::Created { .. } = runtime.block_on(finished(&host, &job.id)) else {
        return Err(BackupError::Format(DAMAGED));
    };
    write_data(roots, TODAY)?;
    let job = runtime
        .block_on(async { host.start_open(path, Zeroizing::new(PASSWORD.into()), false) })?;
    let JobState::Opened { .. } = runtime.block_on(finished(&host, &job.id)) else {
        return Err(BackupError::Format(DAMAGED));
    };
    schedule(
        roots,
        &RestoreSource::Staged { id: job.id },
        service_to_restart,
    )
}

/// What `<core>/notes/note.txt` holds now.
pub fn note(roots: &Roots) -> Option<String> {
    std::fs::read_to_string(roots.core.join(NOTE)).ok()
}

fn write_data(roots: &Roots, text: &str) -> std::io::Result<()> {
    write(&roots.core.join(NOTE), text)?;
    write(
        &roots.desktop.join("desktop-state.json"),
        &format!("{{\"note\":\"{text}\"}}"),
    )
}

fn write(path: &Path, text: &str) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, text)
}

async fn finished(host: &BackupHost, id: &str) -> JobState {
    loop {
        let Some(job) = host.job(id) else {
            return JobState::Other;
        };
        if !matches!(job.state, JobState::Running { .. }) {
            return job.state;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

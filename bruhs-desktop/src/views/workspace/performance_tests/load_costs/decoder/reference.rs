//! Frozen saved-file reader from before SIMD UTF-8 validation.
use super::*;

pub(super) fn load_original_state_from_path(
    path: &Path,
) -> Result<Option<PersistedWorkspaceState>, String> {
    read_original_state_snapshot(path).map(|(state, _)| state)
}

pub(super) fn read_original_state_snapshot(
    path: &Path,
) -> Result<(Option<PersistedWorkspaceState>, DiskVersion), String> {
    state_file::reject_non_file(path).map_err(|error| error.to_string())?;
    let contents = match fs::read(path) {
        Ok(contents) => contents,
        Err(err) if err.kind() == io::ErrorKind::NotFound => {
            return Ok((None, DiskVersion::Missing))
        }
        Err(err) => {
            return Err(format!(
                "Could not read saved desktop state at {}: {err}",
                path.display()
            ));
        }
    };

    serde_json::from_slice(&contents)
        .map(|state| (Some(state), DiskVersion::of_bytes(&contents)))
        .map_err(|err| {
            format!(
                "Saved desktop state at {} is invalid: {err}",
                path.display()
            )
        })
}

pub(super) fn load_original_state_with_backup(
    path: &Path,
) -> (Option<PersistedWorkspaceState>, Option<String>, DiskVersion) {
    match read_original_state_snapshot(path) {
        Ok((state, version)) => (state, None, version),
        Err(err) => {
            tracing::error!("{err}");
            let backup_path = desktop_state_backup_path(path);
            match load_original_state_from_path(&backup_path) {
                Ok(Some(state)) => {
                    let recovery = format!(
                        "{err} Recovered the previous state from {}.",
                        backup_path.display()
                    );
                    tracing::warn!("{recovery}");
                    (Some(state), Some(recovery), DiskVersion::Untrusted)
                }
                Ok(None) => (None, Some(err), DiskVersion::Untrusted),
                Err(backup_err) => {
                    tracing::error!("{backup_err}");
                    (
                        None,
                        Some(format!("{err} {backup_err}")),
                        DiskVersion::Untrusted,
                    )
                }
            }
        }
    }
}

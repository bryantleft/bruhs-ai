//! "Open in editor": resolves an editor from the shared table in
//! `bruhs_models::logic::editor` and spawns it detached. Shared by the server
//! route and the desktop.

use std::path::Path;

// The editor table and argument shapes live in `bruhs-models`; this module
// keeps the PATH lookup and the spawn.
use bruhs_models::logic::editor::{editor_args, editor_by_id, EditorDefinition};

fn find_on_path(command: &str) -> bool {
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&path).any(|dir| dir.join(command).is_file())
}

/// Resolve the first installed CLI for `editor`; on macOS fall back to
/// `open -a <App> <path>` when none is on `PATH`.
pub fn launch_command(
    editor: &EditorDefinition,
    path: &str,
    line: Option<u32>,
) -> Option<(String, Vec<String>)> {
    if let Some(cmd) = editor.commands.iter().find(|c| find_on_path(c)) {
        return Some(editor_args(editor, cmd, path, line));
    }
    if cfg!(target_os = "macos") {
        if let Some(app) = editor.mac_app {
            return Some((
                "open".to_string(),
                vec!["-a".to_string(), app.to_string(), path.to_string()],
            ));
        }
    }
    None
}

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum EditorError {
    #[error("unknown editor")]
    UnknownEditor,
    #[error("Editor not found")]
    NotInstalled,
    #[error("could not launch editor: {0}")]
    Spawn(#[source] std::io::Error),
}

/// Open `path` (a file or directory) in the editor `editor_id`, detached.
pub fn open_in_editor(
    editor_id: &str,
    path: &Path,
    line: Option<u32>,
) -> Result<&'static EditorDefinition, EditorError> {
    let editor = editor_by_id(editor_id).ok_or(EditorError::UnknownEditor)?;
    let path_str = path.to_string_lossy();
    let (cmd, args) = launch_command(editor, &path_str, line).ok_or(EditorError::NotInstalled)?;
    let mut command = std::process::Command::new(cmd);
    command
        .args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    spawn_reaped(command).map_err(EditorError::Spawn)?;
    Ok(editor)
}

/// Spawn `command` detached and wait for it on a small thread so it is
/// reaped when it exits instead of lingering as a zombie. The handle yields
/// the exit status; callers may drop it. `None` when the editor started but
/// the waiter thread could not (it is logged; the launch still succeeded).
fn spawn_reaped(
    mut command: std::process::Command,
) -> std::io::Result<Option<std::thread::JoinHandle<std::io::Result<std::process::ExitStatus>>>> {
    let mut child = command.spawn()?;
    match std::thread::Builder::new()
        .name("bruhs-editor-reaper".to_string())
        .spawn(move || child.wait())
    {
        Ok(waiter) => Ok(Some(waiter)),
        Err(e) => {
            tracing::warn!("editor started but its reaper thread did not: {e}");
            Ok(None)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Editor CLIs (`code`, `zed`, `open`) exit quickly after handing off;
    /// without a wait they stayed behind as zombies.
    #[test]
    fn launched_processes_are_reaped() {
        let mut command = std::process::Command::new("/bin/sh");
        command.args(["-c", "exit 7"]);
        let waiter = spawn_reaped(command).unwrap().unwrap();
        let status = waiter.join().unwrap().unwrap();
        assert_eq!(status.code(), Some(7));
    }

    #[test]
    fn unknown_editor_is_rejected() {
        assert!(matches!(
            open_in_editor("emacs-of-doom", Path::new("/tmp"), None),
            Err(EditorError::UnknownEditor)
        ));
    }
}

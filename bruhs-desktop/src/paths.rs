//! Where the desktop app keeps its files.
//!
//! Everything lives under one per-user data directory: on macOS
//! `~/Library/Application Support/bruhs-ai` (where earlier versions already
//! wrote, so existing state keeps loading), on Linux
//! `$XDG_DATA_HOME/bruhs-ai` (usually `~/.local/share/bruhs-ai`), on Windows
//! `%APPDATA%\bruhs-ai`.

use std::path::PathBuf;

const APP_DIR: &str = "bruhs-ai";
/// Overrides the state file (tests, side-by-side profiles); attachments then
/// live beside it.
const DESKTOP_STATE_FILE_ENV: &str = "BRUHS_DESKTOP_STATE_FILE";
const STATE_FILE_NAME: &str = "desktop-state.json";
const ATTACHMENTS_DIR: &str = "knowledge-attachments";
const LOGS_DIR: &str = "logs";

/// The platform's per-user data directory for bruhs-ai.
pub fn app_data_dir() -> Option<PathBuf> {
    directories::BaseDirs::new().map(|dirs| dirs.data_dir().join(APP_DIR))
}

/// The desktop's saved workspace state.
pub fn desktop_state_file() -> Option<PathBuf> {
    match std::env::var_os(DESKTOP_STATE_FILE_ENV) {
        Some(path) => Some(PathBuf::from(path)),
        None => app_data_dir().map(|dir| dir.join(STATE_FILE_NAME)),
    }
}

/// Where knowledge attachment payloads are copied (beside the state file).
pub fn knowledge_attachments_dir() -> Option<PathBuf> {
    desktop_state_file().and_then(|state| state.parent().map(|parent| parent.join(ATTACHMENTS_DIR)))
}

/// The desktop's log files.
pub fn log_dir() -> Option<PathBuf> {
    app_data_dir().map(|dir| dir.join(LOGS_DIR))
}

#[cfg(test)]
mod tests {
    use super::{app_data_dir, APP_DIR};

    #[test]
    fn the_data_dir_is_the_platform_one() {
        let dir = app_data_dir().expect("a home directory in tests");
        assert!(dir.ends_with(APP_DIR));
        #[cfg(target_os = "macos")]
        assert!(
            dir.ends_with("Library/Application Support/bruhs-ai"),
            "macOS keeps the path earlier versions wrote: {}",
            dir.display()
        );
    }
}

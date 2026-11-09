//! Filesystem layout for Google Calendar state. On macOS this resolves
//! under `~/Library/Application Support/dev.bruhs.bruhs-ai/calendar/google/`;
//! on Linux it follows `XDG_CONFIG_HOME` (default `~/.config/bruhs-ai/...`).
//!
//! Tokens never live here; they go to the OS keychain. This directory holds
//! the per-calendar sync cursors and the calendar list (see `cursor.rs`).

use directories::ProjectDirs;
use std::path::PathBuf;

const QUALIFIER: &str = "dev";
const ORG: &str = "bruhs";
const APP: &str = "bruhs-ai";

/// The calendar/google state directory, or `None` when no home directory
/// can be resolved. Pure: nothing is created here.
pub fn default_google_calendar_dir() -> Option<PathBuf> {
    ProjectDirs::from(QUALIFIER, ORG, APP)
        .map(|dirs| dirs.config_dir().join("calendar").join("google"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_dir_is_absolute_and_scoped_to_the_app() {
        let dir = default_google_calendar_dir().expect("ProjectDirs should resolve in test env");
        assert!(dir.is_absolute(), "{dir:?} should be absolute");
        assert!(dir.ends_with(std::path::PathBuf::from("calendar/google")));
    }
}

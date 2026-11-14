//! Explicit native fixtures; ordinary application startup never installs this mode.
use gpui::{App, Global};
use std::cell::Cell;

mod auth_fixture;
pub(crate) use auth_fixture::{
    auth_fixture_original, auth_fixture_program, auth_probe_started, auth_status_delivered,
};

#[cfg(target_os = "macos")]
mod clocks;
#[cfg(target_os = "macos")]
mod runner;
#[cfg(target_os = "macos")]
mod startup_auth;

#[derive(Clone, Copy)]
pub(crate) enum Service {
    Auth,
    Meetings,
    Preview,
    Updates,
}

impl Service {
    const ALL: [Self; 4] = [Self::Auth, Self::Meetings, Self::Preview, Self::Updates];

    fn index(self) -> usize {
        match self {
            Self::Auth => 0,
            Self::Meetings => 1,
            Self::Preview => 2,
            Self::Updates => 3,
        }
    }
}

#[derive(Default)]
struct FixtureMode {
    blocked: [Cell<u64>; Service::ALL.len()],
}

impl Global for FixtureMode {}

pub(crate) fn block_service(cx: &App, service: Service) -> bool {
    let Some(mode) = cx.try_global::<FixtureMode>() else {
        return false;
    };
    let count = &mode.blocked[service.index()];
    count.set(count.get().saturating_add(1));
    true
}

pub(crate) fn is_executable() -> bool {
    option_env!("CARGO_BIN_NAME") == Some("bruhs-native-performance")
}

pub(crate) fn run() -> Result<(), String> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if args.first().map(String::as_str) != Some("--native-performance") {
        println!("NATIVE_PERFORMANCE_SKIPPED explicit --native-performance arguments required");
        return Ok(());
    }
    if cfg!(test) {
        return Err(
            "Build the normal native binary; test targets change GPUI frame flushing".into(),
        );
    }
    #[cfg(target_os = "macos")]
    return runner::run(&args[1..]);
    #[cfg(not(target_os = "macos"))]
    Err("The native performance harness currently supports macOS".into())
}

#[cfg(test)]
mod tests {
    #[test]
    fn regular_application_test_target_never_selects_the_native_entry() {
        assert!(!super::is_executable());
    }
}

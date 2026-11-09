//! `AppState::new` used to `tokio::spawn` the Google Calendar loop, which
//! panics outside a runtime. The desktop app builds its state before
//! entering one, so it crashed at startup whenever `BRUHS_GOOGLE_*` was set.
//!
//! This is its own test binary so setting the env vars cannot race other
//! tests reading the environment.

use bruhs_core::AppState;

#[test]
fn app_state_builds_outside_a_runtime_with_google_configured() {
    std::env::set_var("BRUHS_GOOGLE_CLIENT_ID", "test-client-id");
    std::env::set_var("BRUHS_GOOGLE_CLIENT_SECRET", "test-client-secret");

    // No runtime is entered here.
    assert!(tokio::runtime::Handle::try_current().is_err());
    let state = AppState::new();
    assert!(state.google_calendar().start_oauth().is_ok());
}

//! Hold file decoding, owned state and layout constant while comparing chat construction.
use super::*;

#[cfg(target_os = "macos")]
mod diagnostics;

#[gpui::test]
#[ignore = "manual optimized complete saved-file loading with known or discarded default project path"]
fn complete_workspace_project_path_load_cost(cx: &mut TestAppContext) {
    initialize(cx);
    let (variant, restore): (&str, ThreadRestorer) =
        match std::env::var("BRUHS_PERF_PROJECT_PATH").as_deref() {
            Ok("original") => (
                "original",
                super::super::project_path_reference::default_threads,
            ),
            Ok("known") | Err(_) => ("known", threads::current_threads),
            other => panic!("invalid project path construction {other:?}"),
        };
    let selected = std::env::var("BRUHS_PERF_PROJECT_PATH_CASE").ok();
    measure_load_cases_with_threads(
        cx,
        variant,
        load_desktop_state_with_backup,
        load_desktop_state_with_backup,
        (
            restore,
            super::super::project_path_reference::default_threads,
        ),
        selected.as_deref(),
        "PROJECT_PATH_LOAD_COST",
    );
}

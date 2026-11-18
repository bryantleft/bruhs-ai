//! Web `Local suggestions` on desktop: the section's own dry run, and a
//! card per suggestion with its evidence, Dismiss and "Preview N changes".

use super::super::{performance_tests::fixture, AppView};
use bruhs_models::{suggestion_copy, EpochMs, WorkflowStatus};

fn click(cx: &mut gpui::VisualTestContext, selector: &'static str) {
    let bounds = cx
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("{selector} is drawn"));
    cx.simulate_click(bounds.center(), gpui::Modifiers::default());
    cx.run_until_parked();
}

#[gpui::test]
fn local_suggestions_preview_dismiss_and_dry_run_like_web(cx: &mut gpui::TestAppContext) {
    let (workspace, cx) = fixture(cx);
    // A waiting preview gets its own card above Local suggestions; room for both.
    cx.simulate_resize(gpui::size(gpui::px(1920.0), gpui::px(3200.0)));
    workspace.update(cx, |workspace, cx| {
        let now = EpochMs::now().as_millis();
        workspace.automation_state.suggestions =
            vec![bruhs_models::protect_focus_suggestion(3, 3, now)];
        workspace.active_view = AppView::Automations;
        cx.notify();
    });
    cx.run_until_parked();
    assert!(cx.debug_bounds("desktop-local-suggestions").is_some());
    assert!(
        cx.debug_bounds("desktop-suggestion-0").is_some(),
        "a card per suggestion"
    );
    let operations = workspace.read_with(cx, |workspace, _| {
        workspace.automation_state.suggestions[0].operations.len()
    });
    assert_eq!(suggestion_copy::preview(operations), "Preview 3 changes");

    // New dry run previews the focus workflow and leaves the suggestion
    // uncounted, as the web's does.
    click(cx, "desktop-automation-new-dry-run");
    workspace.read_with(cx, |workspace, _| {
        let latest = workspace
            .automation_state
            .workflows
            .last()
            .expect("a workflow");
        assert_eq!(latest.status, WorkflowStatus::Previewed);
        assert_eq!(workspace.automation_state.suggestions[0].shown_count, 0);
    });

    // Preview N changes counts it as shown and previews its operations.
    click(cx, "desktop-automation-suggestion-preview-0");
    workspace.read_with(cx, |workspace, _| {
        let latest = workspace
            .automation_state
            .workflows
            .last()
            .expect("a workflow");
        assert_eq!(latest.title, "Protect a focus block");
        assert_eq!(latest.status, WorkflowStatus::Previewed);
        assert_eq!(workspace.automation_state.suggestions[0].shown_count, 1);
    });

    // Once shown it waits a day, as on the web.
    assert!(
        cx.debug_bounds("desktop-suggestion-0").is_none(),
        "shown today"
    );

    workspace.update(cx, |workspace, cx| {
        let now = EpochMs::now().as_millis();
        workspace.automation_state.suggestions =
            vec![bruhs_models::protect_focus_suggestion(3, 3, now)];
        cx.notify();
    });
    cx.run_until_parked();
    click(cx, "desktop-automation-suggestion-dismiss-0");
    workspace.read_with(cx, |workspace, _| {
        assert!(workspace.automation_state.suggestions[0].dismissed);
    });
    assert!(
        cx.debug_bounds("desktop-suggestion-0").is_none(),
        "dismissed"
    );
}

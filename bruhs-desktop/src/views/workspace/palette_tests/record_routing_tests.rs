use super::*;
use bruhs_models::{FinanceTransaction, SchedulingLink, WorkoutLog};

#[derive(Clone, Copy)]
enum Fixture {
    Finance,
    Health,
    SchedulingLink,
    ExternalEvent,
}

fn assert_opens_detail(fixture: Fixture, cx: &mut TestAppContext) {
    cx.update(|cx| {
        cx.bind_keys([gpui::KeyBinding::new(
            "escape",
            super::super::CloseOverlays,
            None,
        )])
    });
    let (workspace, cx) = workspace(cx);
    let path = std::env::temp_dir().join(format!(
        "bruhs-record-navigation-{}.json",
        uuid::Uuid::new_v4()
    ));
    let before = workspace.update_in(cx, |workspace, window, cx| {
        match fixture {
            Fixture::Finance => workspace.life_context.finance.transactions.push(
                FinanceTransaction::manual("routequartz", "Fixture category", -1234, 1),
            ),
            Fixture::Health => workspace.life_context.health.workouts.push(
                WorkoutLog::manual("routequartz", 37, 1),
            ),
            Fixture::SchedulingLink => workspace.scheduling_preferences.links.push(
                SchedulingLink::new("routequartz", 45),
            ),
            Fixture::ExternalEvent => workspace.meetings_hub.update(cx, |hub, cx| {
                hub.google_events.push(serde_json::from_value(serde_json::json!({
                    "id": "routequartz-event", "title": "routequartz", "start_at_ms": 1789120800000_f64,
                    "end_at_ms": 1789124400000_f64, "all_day": false,
                    "color": null, "notes": "Exact remote event", "created_at": 1, "updated_at": 1,
                    "source": "google", "location": "Fixture room"
                })).expect("calendar fixture"));
                cx.notify();
            }),
        }
        workspace.persistence_test_path = Some(path.clone());
        workspace.focus_command_bar(&FocusCommandBar, window, cx);
        record_snapshot(workspace, cx)
    });
    cx.run_until_parked();
    cx.simulate_input("routequartz");
    cx.run_until_parked();
    let selected_id = workspace.update_in(cx, |workspace, _, cx| {
        assert_eq!(workspace.filtered_palette_items(cx).len(), 1);
        workspace.universal_search("routequartz", cx).1[0]
            .document
            .id
            .clone()
    });
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert!(
        cx.debug_bounds("desktop-search-record-detail").is_some(),
        "The matching command must open a visible exact-record detail"
    );
    workspace.update_in(cx, |workspace, window, cx| {
        assert!(workspace.search_record_focus.is_focused(window));
        assert_eq!(
            workspace
                .selected_search_record
                .as_ref()
                .map(|record| record.id()),
            Some(selected_id.as_str())
        );
        let detail = workspace
            .selected_search_record_detail(cx)
            .expect("selected record");
        assert_eq!(detail.title, "routequartz");
        let expected = match fixture {
            Fixture::Finance => ("Category", "Fixture category"),
            Fixture::Health => ("Duration", "37 minutes"),
            Fixture::SchedulingLink => ("Duration", "45 minutes"),
            Fixture::ExternalEvent => ("Location", "Fixture room"),
        };
        assert!(detail
            .fields
            .iter()
            .any(|(label, value)| (*label, value.as_str()) == expected));
        assert_eq!(record_snapshot(workspace, cx), before);
        assert_eq!(
            workspace
                .persistence
                .latest_generation
                .load(std::sync::atomic::Ordering::Acquire),
            0
        );
    });
    let back = cx
        .debug_bounds("desktop-close-search-record")
        .expect("Back control");
    cx.simulate_click(back.center(), gpui::Modifiers::default());
    cx.run_until_parked();
    workspace.update_in(cx, |workspace, window, _| {
        assert!(workspace.selected_search_record.is_none());
        assert!(workspace.workspace_focus_handle.is_focused(window));
    });
    workspace.update_in(cx, |workspace, window, cx| {
        workspace.focus_command_bar(&FocusCommandBar, window, cx)
    });
    cx.run_until_parked();
    cx.simulate_input("routequartz");
    cx.run_until_parked();
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    workspace.update_in(cx, |workspace, window, _| {
        assert!(workspace.selected_search_record.is_none());
        assert!(workspace.workspace_focus_handle.is_focused(window));
    });
    assert!(!path.exists(), "Read-only navigation wrote the workspace");
}

fn record_snapshot(workspace: &Workspace, cx: &gpui::Context<'_, Workspace>) -> serde_json::Value {
    serde_json::to_value((
        &workspace.life_context,
        &workspace.scheduling_preferences,
        &workspace.automation_state,
        &workspace.meetings_hub.read(cx).google_events,
    ))
    .expect("record snapshot")
}

#[gpui::test]
fn native_finance_command_opens_exact_detail(cx: &mut TestAppContext) {
    assert_opens_detail(Fixture::Finance, cx);
}

#[gpui::test]
fn native_health_command_opens_exact_detail(cx: &mut TestAppContext) {
    assert_opens_detail(Fixture::Health, cx);
}

#[gpui::test]
fn native_scheduling_link_command_opens_exact_detail(cx: &mut TestAppContext) {
    assert_opens_detail(Fixture::SchedulingLink, cx);
}

#[gpui::test]
fn native_external_event_command_opens_exact_detail(cx: &mut TestAppContext) {
    assert_opens_detail(Fixture::ExternalEvent, cx);
}

#[gpui::test]
fn stale_record_hit_keeps_its_id_instead_of_opening_another_record(cx: &mut TestAppContext) {
    let (workspace, cx) = workspace(cx);
    let selected_id = workspace.update_in(cx, |workspace, window, cx| {
        workspace.life_context.finance.transactions = vec![
            FinanceTransaction::manual("routequartz", "Selected", -100, 1),
            FinanceTransaction::manual("Other merchant", "Unrelated", -200, 2),
        ];
        let hit = workspace.universal_search("routequartz", cx).1.remove(0);
        let selected_id = hit.document.id.clone();
        workspace.life_context.finance.transactions.remove(0);
        workspace.open_universal_search_hit(hit, window, cx);
        selected_id
    });
    cx.run_until_parked();
    assert!(cx.debug_bounds("desktop-search-record-detail").is_some());
    workspace.update_in(cx, |workspace, _, cx| {
        assert!(workspace.selected_search_record_detail(cx).is_none());
        assert_eq!(
            workspace
                .selected_search_record
                .as_ref()
                .map(|record| record.id()),
            Some(selected_id.as_str())
        );
        assert_eq!(
            workspace.life_context.finance.transactions[0].merchant,
            "Other merchant"
        );
        workspace.life_tab = bruhs_models::LifeSection::Health;
        cx.notify();
    });
    cx.run_until_parked();
    assert!(
        cx.debug_bounds("desktop-search-record-detail").is_none(),
        "Finance selection leaked into the Health tab"
    );
}

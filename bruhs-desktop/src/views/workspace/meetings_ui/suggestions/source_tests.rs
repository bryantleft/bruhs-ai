//! Promote, retain and navigate a suggestion through the real workspace UI.

use crate::views::workspace::persistence::{
    load_desktop_state_from_path, write_desktop_state_atomically, RestoredRecords,
};
use crate::{
    app_state::DesktopState,
    theme,
    views::workspace::{AppView, MeetingPrefs, Workspace},
};
use crate::{server_client::ServerClient, views::meetings_hub::MeetingsHub};
use bruhs_models::meeting_suggestions::{Suggestion, SuggestionState, ThoughtKind};
use bruhs_models::Meeting;
use gpui::AppContext;
use std::{cell::RefCell, rc::Rc, sync::Arc};

#[gpui::test]
fn a_kept_suggestion_preserves_its_own_meeting_through_restore_and_both_navigation_directions(
    cx: &mut gpui::TestAppContext,
) {
    // Opening a meeting requests a fresh transcript through the Tokio bridge.
    // Port zero isolates this UI case from any running companion server.
    cx.executor().allow_parking();
    cx.update(|cx| {
        bruhs_ui::init(cx);
        bruhs_ui::Theme::change(bruhs_ui::ThemeMode::Dark, None, cx);
        theme::apply_bruhs_theme(cx);
        cx.set_global(DesktopState::new().expect("initialize isolated test desktop"));
    });
    let cell = Rc::new(RefCell::new(None));
    let target = cell.clone();
    let (_root, cx) = cx.add_window_view(|window, cx| {
        let workspace = cx.new(|cx| Workspace::new(window, cx));
        *target.borrow_mut() = Some(workspace.clone());
        bruhs_ui::Root::new(workspace, window, cx)
    });
    let workspace = cell.borrow().clone().expect("capture test workspace");
    let source = Meeting::new("Repeated title", 0.0);
    let other = Meeting::new("Repeated title", 1.0);
    let source_id = source.id.clone();
    let task_id = workspace.update_in(cx, |workspace, window, cx| {
        let runtime = Arc::clone(&cx.global::<DesktopState>().runtime);
        workspace.meetings_hub = cx.new(|cx| {
            MeetingsHub::new(
                ServerClient::new("http://127.0.0.1:0", "http://127.0.0.1:0").unwrap(),
                runtime,
                MeetingPrefs::default(),
                cx,
            )
        });
        workspace.work_items.clear();
        workspace.meetings_hub.update(cx, |hub, _cx| {
            hub.meetings = vec![source.clone(), other.clone()];
            hub.live_meeting_id = Some(other.id.clone());
            hub.segments.insert(source.id.clone(), Vec::new());
        });
        workspace.meeting_suggestions.live.follow(Some(&source.id));
        workspace.meeting_suggestions.live.items.push(Suggestion {
            id: "suggestion".into(),
            kind: ThoughtKind::ActionItem,
            text: "Prepare the launch plan".into(),
            reason: "A commitment".into(),
            score: 4,
            source: None,
            created_at_ms: 0.0,
            state: SuggestionState::New,
        });
        workspace.decide_meeting_suggestion("suggestion", SuggestionState::Added, cx);
        assert_eq!(workspace.work_items.len(), 1);
        assert!(workspace.work_items[0]
            .source
            .as_ref()
            .unwrap()
            .is_meeting(&source_id));
        workspace.work_items[0].title = "Edited task".into();
        workspace.work_items[0].description = Some("Replaced description".into());
        workspace.work_items[0].planning.recurrence = Some(bruhs_models::RecurrenceRule::Daily);
        workspace.work_items[0].due_date = Some("2026-11-16".into());
        let next = crate::views::workspace::tasks_ui::next_recurring_desktop_item(
            &workspace.work_items[0],
        )
        .unwrap();
        assert_eq!(next.source, workspace.work_items[0].source);
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("desktop-state.json");
        write_desktop_state_atomically(&path, &workspace.snapshot(cx), 1).unwrap();
        let mut restored = load_desktop_state_from_path(&path).unwrap().unwrap();
        let records = RestoredRecords::take_from_persisted(Some(&mut restored));
        workspace.work_items = records.work_items;
        let id = workspace.work_items[0].id.clone();
        assert!(workspace.work_items[0]
            .source
            .as_ref()
            .unwrap()
            .is_meeting(&source_id));
        workspace.active_view = AppView::Tasks;
        workspace.select_work_item_by_id(&id, window, cx);
        id
    });
    cx.run_until_parked();
    let bounds = cx
        .debug_bounds("desktop-task-source-meeting")
        .expect("source action is visible");
    cx.simulate_click(bounds.center(), gpui::Modifiers::default());
    cx.run_until_parked();
    workspace.update_in(cx, |workspace, _window, _cx| {
        assert_eq!(
            workspace.active_view,
            AppView::MeetingDetail {
                meeting_id: source_id.clone()
            }
        );
        assert!(workspace.selected_work_item_id.is_none());
    });
    let bounds = cx
        .debug_bounds("desktop-meeting-linked-task")
        .expect("related task is visible");
    cx.simulate_click(bounds.center(), gpui::Modifiers::default());
    cx.run_until_parked();
    workspace.update_in(cx, |workspace, _window, _cx| {
        assert_eq!(workspace.active_view, AppView::Tasks);
        assert_eq!(
            workspace.selected_work_item_id.as_deref(),
            Some(task_id.as_str())
        );
    });
    for key in ["enter", "space"] {
        let source = cx.debug_bounds("desktop-task-source-meeting").unwrap();
        // Focus the control without a click. The key must perform navigation.
        cx.simulate_mouse_down(
            source.center(),
            gpui::MouseButton::Left,
            gpui::Modifiers::default(),
        );
        cx.simulate_mouse_up(
            gpui::point(gpui::px(1.0), gpui::px(1.0)),
            gpui::MouseButton::Left,
            gpui::Modifiers::default(),
        );
        workspace.update_in(cx, |workspace, _, _| {
            assert_eq!(workspace.active_view, AppView::Tasks)
        });
        cx.simulate_keystrokes(key);
        cx.run_until_parked();
        workspace.update_in(cx, |workspace, _, _| {
            assert_eq!(
                workspace.active_view,
                AppView::MeetingDetail {
                    meeting_id: source_id.clone()
                },
                "{key}"
            );
        });
        let task = cx.debug_bounds("desktop-meeting-linked-task").unwrap();
        cx.simulate_mouse_down(
            task.center(),
            gpui::MouseButton::Left,
            gpui::Modifiers::default(),
        );
        cx.simulate_mouse_up(
            gpui::point(gpui::px(1.0), gpui::px(1.0)),
            gpui::MouseButton::Left,
            gpui::Modifiers::default(),
        );
        cx.simulate_keystrokes(key);
        cx.run_until_parked();
        workspace.update_in(cx, |workspace, _, _| {
            assert_eq!(workspace.active_view, AppView::Tasks, "{key}");
            assert_eq!(
                workspace.selected_work_item_id.as_deref(),
                Some(task_id.as_str())
            );
        });
    }
    workspace.update_in(cx, |workspace, _window, cx| {
        workspace.meetings_hub.update(cx, |hub, cx| {
            hub.meetings.retain(|meeting| meeting.id != source_id);
            cx.notify();
        });
        cx.notify();
    });
    cx.run_until_parked();
    let unavailable = cx
        .debug_bounds("desktop-task-source-meeting")
        .expect("source hint remains after the meeting is removed");
    cx.simulate_click(unavailable.center(), gpui::Modifiers::default());
    workspace.update_in(cx, |workspace, _window, _cx| {
        assert_eq!(workspace.active_view, AppView::Tasks);
        assert_eq!(workspace.work_items.len(), 1);
        assert!(workspace.work_items[0]
            .source
            .as_ref()
            .unwrap()
            .is_meeting(&source_id));
    });
}

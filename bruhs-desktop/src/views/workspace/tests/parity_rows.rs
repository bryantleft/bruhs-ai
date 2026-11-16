//! Capability matrix rows closed on desktop
//! (docs/parity-capability-matrix-2026-11-16.md), checked through GPUI test
//! frames the way a user reaches them.

use super::*;
use bruhs_models::{Meeting, MeetingSummary, ObservationKind, PersonObservation, PersonProfile};

/// A workspace in a window, with the app's theme and dialogs root. Its hub
/// talks to a closed loopback port (9), never a bruhs-server running on
/// this machine; opening a meeting reads through the real Tokio adapter, so
/// the scheduler allows that I/O.
fn parity_workspace(
    cx: &mut gpui::TestAppContext,
) -> (Entity<Workspace>, &mut gpui::VisualTestContext) {
    cx.executor().allow_parking();
    cx.update(|cx| {
        gpui_component::init(cx);
        gpui_component::Theme::change(gpui_component::ThemeMode::Dark, None, cx);
        theme::apply_bruhs_theme(cx);
        cx.set_global(DesktopState::new().expect("desktop state should initialize"));
    });
    let workspace_cell: Rc<RefCell<Option<Entity<Workspace>>>> = Rc::new(RefCell::new(None));
    let workspace_for_window = workspace_cell.clone();
    let (_root, cx) = cx.add_window_view(|window, cx| {
        let workspace = cx.new(|cx| Workspace::new(window, cx));
        *workspace_for_window.borrow_mut() = Some(workspace.clone());
        gpui_component::Root::new(workspace, window, cx)
    });
    let workspace = workspace_cell
        .borrow()
        .clone()
        .expect("workspace entity should be captured");
    workspace.update_in(cx, |workspace, _window, cx| {
        workspace.meetings_hub.update(cx, |hub, _| {
            hub.set_people_test_client(
                crate::server_client::ServerClient::new("http://127.0.0.1:9", "http://127.0.0.1:9")
                    .expect("a loopback client"),
            );
        });
    });
    (workspace, cx)
}

fn show_view(workspace: &Entity<Workspace>, cx: &mut gpui::VisualTestContext, view: AppView) {
    workspace.update_in(cx, |workspace, _window, cx| {
        workspace.active_view = view;
        cx.notify();
    });
    cx.run_until_parked();
}

/// PM-21, PM-22 and PM-24: Today's decisions open their meeting, its
/// follow-ups open the person's profile (even past a People search that
/// hides it), and "Calendar →" opens the calendar.
#[gpui::test]
fn today_rows_open_their_meeting_person_and_calendar(cx: &mut gpui::TestAppContext) {
    let (workspace, cx) = parity_workspace(cx);
    let hour_ago = EpochMs::now().as_f64() - 3_600_000.0;
    workspace.update_in(cx, |workspace, window, cx| {
        workspace.meetings_hub.update(cx, |hub, _| {
            let mut meeting = Meeting::new("Roadmap review", hour_ago);
            meeting.id = "meeting-roadmap".into();
            meeting.summary = Some(MeetingSummary {
                tldr: String::new(),
                action_items: vec![],
                decisions: vec!["Ship the beta on Friday".into()],
                topics: vec![],
                generated_by: "fixture".into(),
                generated_at: 1,
            });
            hub.meetings = vec![meeting];
            let mut ada = PersonProfile::new("Ada");
            ada.id = "person-ada".into();
            ada.observations.push(PersonObservation {
                at_meeting_id: "meeting-roadmap".into(),
                kind: ObservationKind::ActionItemOwner,
                text: "Send the launch notes".into(),
                created_at: EpochMs::saturating_from_f64(hour_ago).as_millis(),
            });
            hub.people = vec![ada.into(), PersonProfile::new("Grace").into()];
        });
        workspace
            .people_page
            .search_input
            .update(cx, |input, cx| input.set_value("Grace", window, cx));
    });
    show_view(&workspace, cx, AppView::Today);

    click_debug_selector(cx, "desktop-today-follow-up-0");
    cx.run_until_parked();
    workspace.update_in(cx, |workspace, window, cx| {
        assert_eq!(workspace.active_view, AppView::People);
        assert_eq!(workspace.expanded_person_id.as_deref(), Some("person-ada"));
        assert_eq!(
            workspace.people_page.search_input.read(cx).value().as_ref(),
            "",
            "a search that hides the profile is cleared"
        );
        assert!(workspace.workspace_focus_handle.is_focused(window));
    });

    show_view(&workspace, cx, AppView::Today);
    click_debug_selector(cx, "desktop-today-decision-0");
    cx.run_until_parked();
    workspace.update_in(cx, |workspace, _window, _cx| {
        assert_eq!(
            workspace.active_view,
            AppView::MeetingDetail {
                meeting_id: "meeting-roadmap".into()
            }
        );
    });

    show_view(&workspace, cx, AppView::Today);
    click_debug_selector(cx, "desktop-today-calendar-link");
    cx.run_until_parked();
    workspace.update_in(cx, |workspace, _window, _cx| {
        assert_eq!(workspace.active_view, AppView::Calendar);
    });
}

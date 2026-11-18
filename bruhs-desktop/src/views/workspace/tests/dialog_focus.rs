//! Where focus lands when a dialog opens (calendar record, "Desktop dialog
//! focus"; INBOX-11), checked against the web `Dialog` in
//! `bruhs-web/src/ui/dialog.rs`: it focuses the element marked
//! `data-autofocus`, else its first focusable element, else itself. Web
//! dialogs have no close button, so a desktop dialog lands on its first
//! control after the header; Tasks' Projects lands on its close button,
//! which is the web dialog's own Close. Settings is a page on the web, so it
//! keeps focus on itself until the first Tab.

use super::keys::{focus_handle_of, focus_page, focused, in_dialog, press, tab_to};
use super::launch::{init, launch, saved};
use super::*;
use crate::views::workspace::OpenCapture;
use bruhs_models::{InboxConversation, InboxMessage, InboxParticipant, InboxSource};
use chrono::TimeZone as _;
use gpui::{Context, Window};

fn conversation(id: &str) -> InboxConversation {
    let mut conversation = InboxConversation::new(
        InboxSource::Email,
        "Lunch plans",
        vec![InboxParticipant {
            id: format!("participant-{id}"),
            display_name: "Ada".into(),
            address: String::new(),
            person_id: None,
        }],
        1,
    );
    conversation.id = id.into();
    conversation.receive(InboxMessage::incoming("Ada", "Friday works?", 1));
    conversation
}

const GOOGLE_LINK: &str = "https://calendar.google.com/calendar/event?eid=synthetic";

fn google_event(id: &str, date: NaiveDate) -> bruhs_models::CalendarEvent {
    let start = chrono::Local
        .from_local_datetime(&date.and_hms_opt(11, 0, 0).expect("a time"))
        .earliest()
        .expect("a local time")
        .timestamp_millis();
    bruhs_models::CalendarEvent {
        id: id.into(),
        title: "Provider sync".into(),
        start_at_ms: EpochMs::saturating_from_i64(start).as_f64(),
        end_at_ms: EpochMs::saturating_from_i64(start + 3_600_000).as_f64(),
        all_day: false,
        color: None,
        notes: Some("Provider notes".into()),
        created_at: 1,
        updated_at: 1,
        source: bruhs_models::EventSource::Google,
        google_event_id: Some(format!("g-{id}")),
        calendar_id: Some("primary".into()),
        attendees: Vec::new(),
        organizer: None,
        location: None,
        conferencing: None,
        recurring_event_id: None,
        html_link: Some(GOOGLE_LINK.into()),
    }
}

/// Whether the dialog itself has focus rather than one of its controls.
fn on_dialog_itself(workspace: &Entity<Workspace>, cx: &mut gpui::VisualTestContext) -> bool {
    workspace.update_in(cx, |workspace, window, _cx| {
        workspace.modal_start_focus_handle.is_focused(window)
    })
}

fn overlay(workspace: &Entity<Workspace>, cx: &mut gpui::VisualTestContext) -> Option<Overlay> {
    workspace.read_with(cx, |workspace, _| workspace.overlay.clone())
}

/// Tab stops in one full turn of the open dialog, from the focused one;
/// Tab never leaves the dialog.
fn dialog_tab_stops(workspace: &Entity<Workspace>, cx: &mut gpui::VisualTestContext) -> usize {
    let first = focused(cx).expect("a focused control");
    let mut stops = 1;
    loop {
        cx.simulate_keystrokes("tab");
        assert!(in_dialog(workspace, cx), "Tab stays in the dialog");
        if focused(cx).as_ref() == Some(&first) {
            return stops;
        }
        stops += 1;
        assert!(stops < 20, "Tab never wrapped inside the dialog");
    }
}

/// Opens a dialog with `open` and checks that focus is on one of its
/// controls, not on the dialog itself.
fn open_on_a_control(
    workspace: &Entity<Workspace>,
    cx: &mut gpui::VisualTestContext,
    name: &str,
    open: impl FnOnce(&mut Workspace, &mut Window, &mut Context<'_, Workspace>),
) {
    focus_page(workspace, cx);
    workspace.update_in(cx, |workspace, window, cx| open(workspace, window, cx));
    cx.run_until_parked();
    assert!(overlay(workspace, cx).is_some(), "{name} is open");
    assert!(in_dialog(workspace, cx), "{name}: focus is in the dialog");
    assert!(
        !on_dialog_itself(workspace, cx),
        "{name}: focus is on a control, not on the dialog itself"
    );
}

/// The conversation's delete dialog opened by Enter on Delete lands on
/// Cancel, as the web's does: Enter there keeps the conversation. Tab
/// cycles Cancel, Delete and the close button; Shift-Tab from Cancel
/// reaches the close button.
#[gpui::test]
async fn the_delete_dialog_opens_on_cancel(cx: &mut gpui::TestAppContext) {
    init(cx);
    let files = tempfile::tempdir().expect("a temporary directory");
    let path = files.path().join("state.json");
    let (workspace, cx) = launch(cx, &path);
    workspace.update_in(cx, |workspace, _window, cx| {
        workspace.inbox_conversations = vec![conversation("lunch")];
        workspace.persist_and_notify(cx);
    });
    saved(&workspace, cx).await;
    workspace.update_in(cx, |workspace, window, cx| {
        workspace.select_inbox_conversation("lunch", window, cx);
    });
    cx.run_until_parked();
    let delete = focus_handle_of(cx, "desktop-inbox-delete");
    let open_by_enter = |cx: &mut gpui::VisualTestContext| {
        focus_page(&workspace, cx);
        tab_to(cx, &delete, "Delete");
        press(cx, "enter");
        assert!(matches!(
            overlay(&workspace, cx),
            Some(Overlay::RecordDeleteConfirm { .. })
        ));
    };

    open_by_enter(cx);
    assert!(in_dialog(&workspace, cx), "focus moved into the dialog");
    assert!(
        !on_dialog_itself(&workspace, cx),
        "focus is on the dialog's first control, as on the web"
    );
    press(cx, "enter");
    assert_eq!(overlay(&workspace, cx), None, "Enter on Cancel closes it");
    workspace.read_with(cx, |workspace, _| {
        assert_eq!(workspace.inbox_conversations.len(), 1, "and keeps it");
    });
    assert!(focused(cx).as_ref() == Some(&delete), "back on Delete");

    // Tab: Cancel, Delete and the close button, then Cancel again.
    open_by_enter(cx);
    assert_eq!(
        dialog_tab_stops(&workspace, cx),
        3,
        "Cancel, Delete and the close button"
    );
    // Shift-Tab from Cancel reaches the close button, which closes it.
    cx.simulate_keystrokes("shift-tab");
    press(cx, "enter");
    assert_eq!(overlay(&workspace, cx), None, "the close button closed it");
    workspace.read_with(cx, |workspace, _| {
        assert_eq!(workspace.inbox_conversations.len(), 1);
    });
}

/// Every other dialog lands where its web counterpart does; Enter there
/// shows which control it is.
#[gpui::test]
async fn dialogs_open_on_their_first_control(cx: &mut gpui::TestAppContext) {
    init(cx);
    let files = tempfile::tempdir().expect("a temporary directory");
    let path = files.path().join("state.json");
    let (workspace, cx) = launch(cx, &path);
    let date = NaiveDate::from_ymd_opt(2026, 5, 12).expect("a date");
    workspace.update_in(cx, |workspace, window, cx| {
        workspace.projects = vec![DesktopProject {
            id: "project-1".into(),
            name: "Alpha".into(),
        }];
        let mut task = work_item("task-1", DesktopWorkItemStatus::Todo);
        task.project_id = Some("project-1".into());
        workspace.work_items = vec![task];
        workspace.calendar_events = vec![event(
            "standup",
            date.and_hms_opt(9, 0, 0).expect("a time"),
            date.and_hms_opt(9, 30, 0).expect("a time"),
        )];
        let thread = Workspace::create_thread_with_title(
            String::new(),
            String::new(),
            workspace.current_model.clone(),
            workspace.permission_mode,
            "General chat",
            window,
            cx,
        );
        workspace.active_thread_id = Some(thread.id.clone());
        workspace.threads = vec![thread];
        workspace.persist_and_notify(cx);
    });
    saved(&workspace, cx).await;

    // Web task detail: Cancel, which keeps the task.
    open_on_a_control(&workspace, cx, "Delete task", |workspace, window, cx| {
        workspace.request_delete_work_item("task-1", window, cx);
    });
    press(cx, "enter");
    assert_eq!(overlay(&workspace, cx), None, "Cancel closed Delete task");
    workspace.read_with(cx, |workspace, _| assert_eq!(workspace.work_items.len(), 1));

    // Web project sidebar: Cancel, which keeps the project.
    open_on_a_control(&workspace, cx, "Delete project", |workspace, window, cx| {
        workspace.request_delete_project_by_id("project-1", window, cx);
    });
    press(cx, "enter");
    assert_eq!(
        overlay(&workspace, cx),
        None,
        "Cancel closed Delete project"
    );
    workspace.read_with(cx, |workspace, _| assert_eq!(workspace.projects.len(), 1));

    // Web Calendar day: the first record, which opens it.
    open_on_a_control(&workspace, cx, "Calendar day", |workspace, window, cx| {
        workspace.open_calendar_day_list(date, window, cx);
    });
    press(cx, "enter");
    workspace.read_with(cx, |workspace, _| {
        assert!(workspace.event_create_visible(), "the event opened");
        assert_eq!(
            workspace.selected_calendar_event_id.as_deref(),
            Some("standup")
        );
    });
    workspace.update_in(cx, |workspace, _window, cx| {
        workspace.close_transient_surfaces();
        cx.notify();
    });
    cx.run_until_parked();

    // A read-only Google event: its fields are disabled, so the web lands
    // on "Open in Google Calendar", which opens the provider's page.
    workspace.update_in(cx, |workspace, _window, cx| {
        workspace.calendar_anchor = date;
        workspace.meetings_hub.update(cx, |hub, _cx| {
            hub.google_events = vec![google_event("provider", date)];
        });
        cx.notify();
    });
    cx.run_until_parked();
    open_on_a_control(&workspace, cx, "Google event", |workspace, window, cx| {
        workspace.open_event_edit("provider".into(), window, cx);
    });
    workspace.read_with(cx, |workspace, _| {
        assert!(workspace.event_dialog_read_only(), "read only");
    });
    // Its disabled fields and Save are not Tab stops, as on the web: the
    // link, Cancel and the close button.
    assert_eq!(dialog_tab_stops(&workspace, cx), 3);
    press(cx, "enter");
    assert_eq!(cx.opened_url().as_deref(), Some(GOOGLE_LINK));
    workspace.update_in(cx, |workspace, _window, cx| {
        workspace.close_transient_surfaces();
        cx.notify();
    });
    cx.run_until_parked();

    // Web "Snooze until": the date and time picker, which opens.
    let thread_id = workspace.read_with(cx, |workspace, _| {
        workspace.active_thread_id.clone().expect("a thread")
    });
    open_on_a_control(&workspace, cx, "Snooze until", |workspace, window, cx| {
        workspace.open_snooze_custom(&thread_id, window, cx);
    });
    press(cx, "enter");
    workspace.read_with(cx, |workspace, _| {
        assert!(workspace.date_picker.is_some(), "the date picker opened");
    });
    workspace.update_in(cx, |workspace, _window, cx| {
        workspace.close_transient_surfaces();
        cx.notify();
    });
    cx.run_until_parked();

    // Web "Update providers": Cancel.
    open_on_a_control(
        &workspace,
        cx,
        "Update providers",
        |workspace, window, cx| {
            workspace.activate_sidebar_update(true, window, cx);
        },
    );
    press(cx, "enter");
    assert_eq!(overlay(&workspace, cx), None, "Cancel closed the update");

    // "Restart to update" (desktop only): Later, the first control.
    open_on_a_control(
        &workspace,
        cx,
        "Restart to update",
        |workspace, window, cx| {
            workspace.open_restart_confirm(window, cx);
        },
    );
    press(cx, "enter");
    assert_eq!(overlay(&workspace, cx), None, "Later closed it");

    // Web Tasks "Projects": its Close comes first; here the close button.
    open_on_a_control(&workspace, cx, "Projects", |workspace, window, cx| {
        workspace.open_task_projects(window, cx);
    });
    press(cx, "enter");
    assert_eq!(overlay(&workspace, cx), None, "the close button closed it");

    // Web capture: its text field.
    focus_page(&workspace, cx);
    workspace.update_in(cx, |workspace, window, cx| {
        workspace.open_capture_dialog(&OpenCapture, window, cx);
    });
    cx.run_until_parked();
    let capture = workspace.read_with(cx, |workspace, cx| {
        workspace.capture_input.read(cx).focus_handle(cx)
    });
    assert!(focused(cx).as_ref() == Some(&capture), "capture's field");
    workspace.update_in(cx, |workspace, _window, cx| {
        workspace.close_transient_surfaces();
        cx.notify();
    });
    cx.run_until_parked();

    // Settings is a page on the web: nothing inside is focused until the
    // first Tab, which reaches its first control.
    focus_page(&workspace, cx);
    workspace.update_in(cx, |workspace, window, cx| {
        workspace.open_settings(&OpenSettings, window, cx);
    });
    cx.run_until_parked();
    assert!(on_dialog_itself(&workspace, cx), "Settings itself");
    cx.simulate_keystrokes("tab");
    assert!(in_dialog(&workspace, cx));
    assert!(!on_dialog_itself(&workspace, cx), "a control in Settings");
}

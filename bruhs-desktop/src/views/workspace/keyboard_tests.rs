//! Keyboard reach, checked by what the keys do: the app starts with focus in
//! the workspace, as a web page starts with the document focused; Tab leaves
//! a multi-line field as it leaves a web textarea; and a project's Delete is
//! a Tab stop, as on the web.

use super::{
    AppView, DesktopProject, DiskVersion, PersistedWorkspaceState, RestoredRecords, Workspace,
};
use crate::{app_state::DesktopState, theme};
use bruhs_models::{InboxConversation, InboxMessage, InboxSource};
use bruhs_ui::zoom::window_px;
use gpui::{
    size, AppContext, Entity, FocusHandle, Focusable as _, TestAppContext, VisualTestContext,
};
use std::{cell::RefCell, rc::Rc};

fn open(
    cx: &mut TestAppContext,
    state: Option<PersistedWorkspaceState>,
) -> (Entity<Workspace>, &mut VisualTestContext) {
    cx.update(|cx| {
        bruhs_ui::init(cx);
        bruhs_ui::Theme::change(bruhs_ui::ThemeMode::Dark, None, cx);
        theme::apply_bruhs_theme(cx);
        cx.set_global(DesktopState::new().expect("desktop state"));
    });
    let entity = Rc::new(RefCell::new(None));
    let captured = entity.clone();
    let (_, cx) = cx.add_window_view(|window, cx| {
        let workspace = cx.new(|cx| {
            Workspace::from_loaded_state(
                (state, None, DiskVersion::Missing),
                window,
                cx,
                RestoredRecords::take_from_persisted,
                Workspace::restore_threads,
            )
        });
        *captured.borrow_mut() = Some(workspace.clone());
        bruhs_ui::Root::new(workspace, window, cx)
    });
    let workspace = entity.borrow().clone().expect("workspace");
    cx.simulate_resize(size(window_px(1280.0), window_px(800.0)));
    cx.run_until_parked();
    (workspace, cx)
}

/// A key press as the app gets it: gpui clicks a focused button on key up.
fn press(cx: &mut VisualTestContext, key: &'static str) {
    let keystroke = gpui::Keystroke::parse(key).expect("keystroke");
    cx.simulate_event(gpui::KeyDownEvent {
        keystroke: keystroke.clone(),
        is_held: false,
        prefer_character_input: false,
    });
    cx.simulate_event(gpui::KeyUpEvent { keystroke });
    cx.run_until_parked();
}

fn focused(cx: &mut VisualTestContext) -> Option<FocusHandle> {
    cx.update(|window, cx| window.focused(cx))
}

/// Tab stops in one full turn of the Tab order.
fn tab_cycle(cx: &mut VisualTestContext) -> usize {
    cx.simulate_keystrokes("tab");
    let first = focused(cx).expect("a Tab stop");
    for stops in 1..500 {
        cx.simulate_keystrokes("tab");
        if focused(cx).as_ref() == Some(&first) {
            return stops;
        }
    }
    panic!("Tab never came back around");
}

/// Reopened on a page other than Chat, the app used to start with nothing
/// focused, so Tab and every shortcut did nothing until a click.
#[gpui::test]
fn the_app_starts_with_the_keyboard_in_the_workspace(cx: &mut TestAppContext) {
    let state: PersistedWorkspaceState = serde_json::from_value(serde_json::json!({
        "active_view": AppView::Today.route(),
    }))
    .expect("state");
    let (_workspace, cx) = open(cx, Some(state));
    let start = focused(cx).expect("focus at launch");
    cx.simulate_keystrokes("tab");
    let next = focused(cx).expect("Tab reaches a control");
    assert!(start != next, "Tab moves focus without a click first");
}

/// Web: Tab leaves the reply textarea for Save draft; Shift-Tab returns.
#[gpui::test]
fn tab_leaves_a_multi_line_field_like_web(cx: &mut TestAppContext) {
    let mut conversation = InboxConversation::new(InboxSource::Email, "Plans", Vec::new(), 1);
    conversation.receive(InboxMessage::incoming("Ada", "Lunch?", 1));
    let view = AppView::ConversationDetail {
        conversation_id: conversation.id.clone(),
    };
    let state: PersistedWorkspaceState = serde_json::from_value(serde_json::json!({
        "active_view": view.route(),
        "inbox_conversations": [conversation],
    }))
    .expect("state");
    let (workspace, cx) = open(cx, Some(state));
    let reply = workspace.update_in(cx, |workspace, window, cx| {
        workspace.inbox_reply_input.update(cx, |input, cx| {
            input.set_value("Friday works", window, cx);
            input.focus(window, cx);
        });
        workspace.inbox_reply_input.read(cx).focus_handle(cx)
    });
    cx.run_until_parked();
    assert!(focused(cx).as_ref() == Some(&reply), "the reply has focus");
    cx.simulate_keystrokes("tab");
    assert!(focused(cx).as_ref() != Some(&reply), "Tab left the reply");
    // The next stop is Save draft, as on the web: Enter there saves the reply.
    press(cx, "enter");
    workspace.read_with(cx, |workspace, cx| {
        assert_eq!(
            workspace.inbox_reply_input.read(cx).value().as_ref(),
            "Friday works",
            "Tab moved focus instead of typing"
        );
        assert_eq!(
            workspace.inbox_conversations[0].draft_body(),
            "Friday works",
            "Enter after Tab saved the draft"
        );
    });
    cx.simulate_keystrokes("shift-tab");
    assert!(focused(cx).as_ref() == Some(&reply), "Shift-Tab returns");
}

/// Web: a project row's Delete is hidden until hover or focus, but the
/// keyboard reaches it, so each project adds two Tab stops, its row and its
/// Delete.
#[gpui::test]
fn the_keyboard_reaches_a_projects_delete_button_like_web(cx: &mut TestAppContext) {
    let state: PersistedWorkspaceState = serde_json::from_value(serde_json::json!({
        "active_view": AppView::Tasks.route(),
        "projects": [{ "id": "project-launch", "name": "Launch" }],
    }))
    .expect("state");
    let (workspace, cx) = open(cx, Some(state));
    let one = tab_cycle(cx);
    workspace.update(cx, |workspace, cx| {
        workspace.projects.push(DesktopProject {
            id: "project-hiring".into(),
            name: "Hiring".into(),
        });
        cx.notify();
    });
    cx.run_until_parked();
    let two = tab_cycle(cx);
    assert_eq!(two, one + 2, "a project's row and its Delete");
}

/// gpui clicks a focused button on key up, after its Enter/Space key-down
/// handler has run, so a button that answered both moved Previous by two
/// months for one Enter.
#[gpui::test]
fn one_enter_on_previous_moves_the_calendar_back_once(cx: &mut TestAppContext) {
    let state: PersistedWorkspaceState = serde_json::from_value(serde_json::json!({
        "active_view": AppView::Calendar.route(),
    }))
    .expect("state");
    let (workspace, cx) = open(cx, Some(state));
    let previous = cx
        .debug_bounds("desktop-calendar-prev")
        .expect("Previous is drawn")
        .center();
    // A click focuses the button and moves back one month.
    cx.simulate_click(previous, gpui::Modifiers::default());
    cx.run_until_parked();
    let after_click = workspace.read_with(cx, |workspace, _| workspace.calendar_anchor);
    press(cx, "enter");
    let after_enter = workspace.read_with(cx, |workspace, _| workspace.calendar_anchor);
    assert_eq!(
        super::calendar_ui::shift_month(after_click, -1),
        after_enter,
        "one Enter, one month"
    );
}

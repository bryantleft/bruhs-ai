//! Keyboard reach, read through the accessibility tree a screen reader gets:
//! the app starts with focus in the workspace, as a web page starts with
//! the document focused, and Tab leaves a multi-line field as it leaves a
//! web textarea.

use super::{AppView, DiskVersion, PersistedWorkspaceState, RestoredRecords, Workspace};
use crate::{app_state::DesktopState, theme};
use bruhs_models::{InboxConversation, InboxMessage, InboxSource};
use gpui::{px, size, AppContext, Entity, Role, TestAppContext, VisualTestContext};
use std::{cell::RefCell, rc::Rc};

fn open(
    cx: &mut TestAppContext,
    state: Option<PersistedWorkspaceState>,
) -> (Entity<Workspace>, &mut VisualTestContext) {
    cx.update(|cx| {
        gpui_component::init(cx);
        gpui_component::Theme::change(gpui_component::ThemeMode::Dark, None, cx);
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
        gpui_component::Root::new(workspace, window, cx)
    });
    let workspace = entity.borrow().clone().expect("workspace");
    cx.simulate_resize(size(px(1280.0), px(800.0)));
    cx.run_until_parked();
    cx.enable_accessibility();
    (workspace, cx)
}

fn focused(cx: &mut VisualTestContext) -> (Role, String) {
    let (role, label) = cx.accessibility_focus().expect("a focused node");
    (role, label.unwrap_or_default())
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
    assert!(
        cx.update(|window, cx| window.focused(cx).is_some()),
        "focus at launch"
    );
    let mut reached = Vec::new();
    for _ in 0..80 {
        cx.simulate_keystrokes("tab");
        let (role, label) = focused(cx);
        if role == Role::Button {
            reached.push(label);
        }
    }
    assert!(
        reached
            .iter()
            .any(|label| label.starts_with("Toggle sidebar")),
        "Tab walks the page: {reached:?}"
    );
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
    workspace.update_in(cx, |workspace, window, cx| {
        workspace.inbox_reply_input.update(cx, |input, cx| {
            input.set_value("Friday works", window, cx);
            input.focus(window, cx);
        });
    });
    cx.run_until_parked();
    assert_eq!(focused(cx).1, "Reply draft");
    cx.simulate_keystrokes("tab");
    assert_eq!(focused(cx), (Role::Button, "Save draft".to_string()));
    workspace.read_with(cx, |workspace, cx| {
        assert_eq!(
            workspace.inbox_reply_input.read(cx).value().as_ref(),
            "Friday works",
            "Tab moved focus instead of typing"
        );
    });
    cx.simulate_keystrokes("shift-tab");
    assert_eq!(focused(cx).1, "Reply draft");
}

/// Web: the project row's delete button is hidden until hover or focus,
/// but the keyboard reaches it.
#[gpui::test]
fn the_keyboard_reaches_a_projects_delete_button_like_web(cx: &mut TestAppContext) {
    let state: PersistedWorkspaceState = serde_json::from_value(serde_json::json!({
        "active_view": AppView::Tasks.route(),
        "projects": [{ "id": "project-launch", "name": "Launch" }],
    }))
    .expect("state");
    let (_workspace, cx) = open(cx, Some(state));
    let mut reached = Vec::new();
    for _ in 0..60 {
        cx.simulate_keystrokes("tab");
        reached.push(focused(cx));
    }
    let select = reached
        .iter()
        .position(|(_, label)| label == "Select project Launch")
        .expect("the project row");
    assert_eq!(
        reached[select + 1],
        (Role::Button, "Delete project".to_string()),
        "{reached:?}"
    );
}

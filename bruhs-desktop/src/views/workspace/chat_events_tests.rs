//! How the workspace reacts to a chat: what saves (typed events, never
//! spinner frames) and which failures reach the banner.

use super::Workspace;
use crate::{app_state::DesktopState, theme, views::chat::ChatView};
use gpui::{AppContext, Entity, TestAppContext, VisualTestContext};
use std::{cell::RefCell, rc::Rc, time::Duration};

fn fixture(cx: &mut TestAppContext) -> (Entity<Workspace>, &mut VisualTestContext) {
    cx.update(|cx| {
        gpui_component::init(cx);
        gpui_component::Theme::change(gpui_component::ThemeMode::Dark, None, cx);
        theme::apply_bruhs_theme(cx);
        cx.set_global(DesktopState::new().expect("desktop state"));
    });
    let captured = Rc::new(RefCell::new(None));
    let capture = captured.clone();
    let (_, cx) = cx.add_window_view(|window, cx| {
        let workspace = cx.new(|cx| Workspace::new(window, cx));
        *capture.borrow_mut() = Some(workspace.clone());
        gpui_component::Root::new(workspace, window, cx)
    });
    let workspace = captured.borrow().clone().expect("workspace");
    cx.run_until_parked();
    (workspace, cx)
}

fn active_chat(workspace: &Entity<Workspace>, cx: &mut VisualTestContext) -> Entity<ChatView> {
    workspace.read_with(cx, |workspace, _| {
        workspace
            .active_thread()
            .map(|thread| thread.chat.clone())
            .expect("an active thread")
    })
}

fn persist_requests(workspace: &Entity<Workspace>, cx: &mut VisualTestContext) -> usize {
    workspace.read_with(cx, |workspace, _| workspace.persist_requests)
}

#[gpui::test]
fn spinner_frames_never_save_or_drop_the_search_index(cx: &mut TestAppContext) {
    let (workspace, cx) = fixture(cx);
    let chat = active_chat(&workspace, cx);
    workspace.update(cx, |workspace, cx| {
        // Built lazily; a save throws it away.
        workspace.universal_search("", cx);
    });
    let before = persist_requests(&workspace, cx);

    chat.update(cx, |chat, cx| chat.start_spinner_for_test(cx));
    let frames = 25;
    for _ in 0..frames {
        cx.executor().advance_clock(Duration::from_millis(80));
        cx.run_until_parked();
    }

    let frame = chat.read_with(cx, |chat, _| chat.spinner_frame);
    assert_ne!(frame, 0, "the spinner must actually have ticked");
    assert_eq!(persist_requests(&workspace, cx), before);
    workspace.read_with(cx, |workspace, _| {
        assert!(
            workspace.universal_search_index.borrow().is_some(),
            "spinner frames must not invalidate the search index"
        );
    });
    chat.update(cx, |chat, _| chat.is_streaming = false);
}

#[gpui::test]
fn a_slash_command_that_changes_messages_saves(cx: &mut TestAppContext) {
    let (workspace, cx) = fixture(cx);
    let chat = active_chat(&workspace, cx);
    let before = persist_requests(&workspace, cx);

    chat.update(cx, |chat, cx| chat.run_slash_command_for_test("/help", cx));
    cx.run_until_parked();

    assert!(persist_requests(&workspace, cx) > before);
}

fn notice(workspace: &Entity<Workspace>, cx: &mut VisualTestContext) -> Option<String> {
    workspace.read_with(cx, |workspace, cx| {
        workspace
            .meetings_hub
            .read(cx)
            .notice
            .as_ref()
            .map(|notice| notice.message.clone())
    })
}

#[gpui::test]
fn a_failed_editor_launch_is_shown_not_just_logged(cx: &mut TestAppContext) {
    let (workspace, cx) = fixture(cx);
    workspace.update(cx, |workspace, cx| {
        workspace.editor_id = "no-such-editor".to_string();
        workspace.open_path_in_editor("src/main.rs", Some(3), cx);
    });
    cx.run_until_parked();
    let shown = notice(&workspace, cx).expect("a notice");
    assert!(shown.starts_with("Open in editor failed"), "{shown}");
}

#[gpui::test]
fn a_chat_notice_reaches_the_banner(cx: &mut TestAppContext) {
    use crate::views::chat::ChatViewEvent;
    let (workspace, cx) = fixture(cx);
    let chat = active_chat(&workspace, cx);
    chat.update(cx, |_, cx| {
        cx.emit(ChatViewEvent::Notice(
            "The agent did not get your answer".into(),
        ))
    });
    cx.run_until_parked();
    assert_eq!(
        notice(&workspace, cx).as_deref(),
        Some("The agent did not get your answer")
    );
}

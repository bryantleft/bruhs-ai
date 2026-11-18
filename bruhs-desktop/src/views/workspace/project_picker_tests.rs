//! The project picker reads the disk off the UI thread.

use super::Workspace;
use crate::{app_state::DesktopState, theme};
use gpui::{AppContext, TestAppContext};
use std::{cell::RefCell, rc::Rc};

#[gpui::test]
fn listing_a_directory_does_not_block_the_ui_thread(cx: &mut TestAppContext) {
    cx.update(|cx| {
        bruhs_ui::init(cx);
        theme::apply_bruhs_theme(cx);
        cx.set_global(DesktopState::new().expect("desktop state"));
    });
    let captured = Rc::new(RefCell::new(None));
    let capture = captured.clone();
    let (_, cx) = cx.add_window_view(|window, cx| {
        let workspace = cx.new(|cx| Workspace::new(window, cx));
        *capture.borrow_mut() = Some(workspace.clone());
        bruhs_ui::Root::new(workspace, window, cx)
    });
    let workspace = captured.borrow().clone().expect("workspace");
    cx.run_until_parked();
    let home = std::fs::canonicalize(std::env::var_os("HOME").expect("HOME in tests"))
        .expect("home resolves");

    workspace.update(cx, |workspace, cx| {
        workspace.project_picker_current_path = "unchanged".into();
        workspace.load_project_directory(Some(home.to_string_lossy().into_owned()), cx);
        // Nothing was read synchronously.
        assert_eq!(workspace.project_picker_current_path, "unchanged");
        assert!(workspace.project_picker_task.is_some());
    });
    cx.run_until_parked();

    workspace.read_with(cx, |workspace, _| {
        assert_eq!(
            workspace.project_picker_current_path,
            home.to_string_lossy()
        );
        assert!(workspace.project_picker_error.is_none());
    });
}

#[gpui::test]
fn project_attachment_preserves_conversation_permissions_and_draft(cx: &mut TestAppContext) {
    use super::{
        DesktopProjectLocation, Message, NewThread, OpenProjectPicker, PermissionMode, TopLayer,
    };
    cx.update(|cx| {
        bruhs_ui::init(cx);
        theme::apply_bruhs_theme(cx);
        cx.set_global(DesktopState::new().expect("desktop state"));
    });
    let captured = Rc::new(RefCell::new(None));
    let capture = captured.clone();
    let (_, cx) = cx.add_window_view(|window, cx| {
        let workspace = cx.new(|cx| Workspace::new(window, cx));
        *capture.borrow_mut() = Some(workspace.clone());
        bruhs_ui::Root::new(workspace, window, cx)
    });
    let workspace = captured.borrow().clone().expect("workspace");
    workspace.update_in(cx, |workspace, window, cx| {
        workspace.active_project = Some(DesktopProjectLocation {
            name: "Stale project".into(),
            path: "/fixture/stale".into(),
        });
        workspace.permission_mode = PermissionMode::Plan;
        workspace.new_general_thread(window, cx);
        let id = workspace.active_thread_id.clone().expect("thread");
        let chat = workspace.active_thread().unwrap().chat.clone();
        assert!(
            !chat.read(cx).project_scoped,
            "General threads do not inherit the terminal's folder"
        );
        chat.update(cx, |chat, cx| {
            chat.thread_title = "Architecture discussion".into();
            chat.messages.push(Message::user("Keep this decision"));
            chat.session_id = Some("earlier-session".into());
            chat.permission_mode = PermissionMode::Plan;
            chat.set_composer_draft("Build this design".into(), window, cx);
        });
        let count = workspace.threads.len();
        workspace.open_project_picker(&OpenProjectPicker, window, cx);
        workspace.apply_thread_project(
            Some(DesktopProjectLocation {
                name: "Chosen".into(),
                path: "/fixture/chosen".into(),
            }),
            window,
            cx,
        );
        assert_eq!(workspace.threads.len(), count);
        assert_eq!(workspace.active_thread_id, Some(id.clone()));
        assert_eq!(
            workspace.active_thread().unwrap().chat.entity_id(),
            chat.entity_id()
        );
        {
            let view = chat.read(cx);
            assert_eq!(view.project_path, "/fixture/chosen");
            assert_eq!(view.project_name, "Chosen");
            assert!(view.project_scoped && view.fresh_session && view.session_id.is_none());
            assert_eq!(view.messages[0].content, "Keep this decision");
            assert!(view
                .import_context
                .as_deref()
                .unwrap()
                .contains("Keep this decision"));
            assert_eq!(view.permission_mode, PermissionMode::Plan);
            assert_eq!(
                view.input_state.read(cx).value().as_str(),
                "Build this design"
            );
        }
        let saved = workspace
            .snapshot(cx)
            .threads
            .into_iter()
            .find(|thread| thread.id == id)
            .unwrap();
        let restored = Workspace::create_thread_from_persisted(
            saved,
            &workspace.current_model.clone(),
            workspace.permission_mode,
            window,
            cx,
        );
        assert_eq!(
            restored.chat.read(cx).messages[0].content,
            "Keep this decision"
        );
        assert!(restored
            .chat
            .read(cx)
            .import_context
            .as_deref()
            .unwrap()
            .contains("Keep this decision"));
        assert_eq!(restored.chat.read(cx).permission_mode, PermissionMode::Plan);

        workspace.new_thread(&NewThread, window, cx);
        assert_eq!(workspace.active_thread().unwrap().project_name, "Chosen");
        assert_eq!(
            workspace
                .active_thread()
                .unwrap()
                .chat
                .read(cx)
                .thread_title,
            "New thread"
        );
        workspace.select_thread_by_id(&id, cx);
        workspace.top_layer = Some(TopLayer::ProjectPicker {
            for_terminal: false,
            thread: Some(chat.entity_id()),
        });
        chat.update(cx, |chat, _| chat.is_streaming = true);
        workspace.apply_thread_project(None, window, cx);
        assert!(workspace.project_picker_error.is_some());
        assert_eq!(workspace.active_thread().unwrap().project_name, "Chosen");
        chat.update(cx, |chat, _| chat.is_streaming = false);
        workspace.apply_thread_project(None, window, cx);
        assert!(workspace.active_thread().unwrap().project_name.is_empty());
        assert!(!chat.read(cx).project_scoped);
        assert!(workspace.active_project_path(cx).is_none());
        assert_eq!(chat.read(cx).messages[0].content, "Keep this decision");
    });
}

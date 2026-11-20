//! Actual GPUI activation and HTTP serialization; the endpoint is an owned
//! loopback fixture. The shared host/browser suite covers durable execution.
use super::{HandoffUi, WorkItemHandoffRequest};
use crate::views::workspace::{
    AppView, DesktopPriority, DesktopProjectLocation, DesktopWorkItem, DesktopWorkItemStatus,
    SaveStatus, Workspace,
};
use crate::{app_state::DesktopState, server_client::ServerClient, theme};
use bruhs_models::workbench_handoff::WorkItemHandoff;
use bruhs_models::{EpochMs, PermissionMode, WorkItemPlanning, WorkItemSource};
use gpui::{point, px, AppContext, Modifiers, MouseButton, TestAppContext, VisualTestContext};
use std::{
    cell::RefCell,
    io::{Read, Write},
    net::TcpListener,
    rc::Rc,
    sync::mpsc,
    time::{Duration as StdDuration, Instant},
};

fn drive_until(cx: &mut VisualTestContext, mut ready: impl FnMut(&mut VisualTestContext) -> bool) {
    let deadline = Instant::now() + StdDuration::from_secs(60);
    while !ready(cx) {
        assert!(
            Instant::now() < deadline,
            "Native handoff must settle within the existing Workbench request deadline"
        );
        cx.run_until_parked();
        std::thread::yield_now();
    }
}

#[gpui::test]
fn native_work_item_handoff_pointer_keyboard_and_late_response_preserve_the_personal_task(
    cx: &mut TestAppContext,
) {
    cx.executor().allow_parking();
    cx.update(|cx| {
        bruhs_ui::init(cx);
        bruhs_ui::Theme::change(bruhs_ui::ThemeMode::Dark, None, cx);
        theme::apply_bruhs_theme(cx);
        cx.set_global(DesktopState::new().unwrap());
    });
    let captured = Rc::new(RefCell::new(None));
    let target = captured.clone();
    let (_root, cx) = cx.add_window_view(|window, cx| {
        let workspace = cx.new(|cx| Workspace::new(window, cx));
        *target.borrow_mut() = Some(workspace.clone());
        bruhs_ui::Root::new(workspace, window, cx)
    });
    let workspace = captured.borrow().clone().unwrap();
    for activation in ["pointer", "Enter", "Space", "late-edit"] {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let (received, requests) = mpsc::channel();
        let endpoint = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(StdDuration::from_secs(60)))
                .unwrap();
            let mut bytes = Vec::new();
            let mut chunk = [0u8; 4096];
            let (head, body_start, length) = loop {
                let read = socket.read(&mut chunk).unwrap();
                assert_ne!(read, 0);
                bytes.extend_from_slice(&chunk[..read]);
                if let Some(end) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
                    let head = String::from_utf8(bytes[..end].to_vec()).unwrap();
                    let length: usize = head
                        .lines()
                        .find_map(|line| {
                            line.split_once(':')
                                .filter(|(key, _)| key.eq_ignore_ascii_case("content-length"))
                                .map(|(_, value)| value.trim().parse().unwrap())
                        })
                        .unwrap();
                    break (head, end + 4, length);
                }
            };
            while bytes.len() - body_start < length {
                let read = socket.read(&mut chunk).unwrap();
                assert_ne!(read, 0);
                bytes.extend_from_slice(&chunk[..read]);
            }
            assert!(head.starts_with("POST /api/coordination/handoffs HTTP/1.1"));
            let request: WorkItemHandoffRequest =
                serde_json::from_slice(&bytes[body_start..body_start + length]).unwrap();
            received.send(request.clone()).unwrap();
            let handoff = WorkItemHandoff {
                id: bruhs_models::ids::ThreadId::new_v4(),
                request,
                created_at: EpochMs::new(100).unwrap(),
                started: None,
            };
            let body = serde_json::to_vec(&handoff).unwrap();
            write!(socket,"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",body.len()).unwrap();
            socket.write_all(&body).unwrap();
            handoff.id
        });
        let original = workspace.update_in(cx, |workspace, window, cx| {
            workspace.work_items = vec![DesktopWorkItem {
                id: "native-personal-task".into(),
                project_id: None,
                title: "Implement the reviewed meeting action".into(),
                description: Some("Keep the accepted context".into()),
                status: DesktopWorkItemStatus::Todo,
                priority: DesktopPriority::High,
                due_date: None,
                planning: WorkItemPlanning {
                    time_block_event_ids: vec!["native-planned-block".into()],
                    ..Default::default()
                },
                source: Some(WorkItemSource::Meeting {
                    meeting_id: "native-recording".into(),
                    title: "Planning".into(),
                }),
                updated_at: EpochMs::new(42).unwrap(),
            }];
            workspace.current_model = "codex/fixture-model".into();
            workspace.permission_mode = PermissionMode::Plan;
            workspace.active_project = Some(DesktopProjectLocation {
                path: "/selected/native/project".into(),
                name: "Project".into(),
            });
            workspace.work_item_handoff = HandoffUi {
                client: Some(
                    ServerClient::new(format!("http://{address}"), "http://127.0.0.1:0").unwrap(),
                ),
                ..Default::default()
            };
            workspace.active_view = AppView::Tasks;
            workspace.claude_effort = Some("high".into());
            workspace.save_status = SaveStatus::Saved;
            workspace.select_work_item_by_id("native-personal-task", window, cx);
            workspace.work_items[0].to_work_item()
        });
        cx.run_until_parked();
        let bounds = cx
            .debug_bounds("desktop-task-agent-review")
            .expect("review action is visible");
        if activation == "late-edit" {
            workspace.update_in(cx, |workspace, _, cx| {
                workspace.prepare_work_item_handoff("native-personal-task", cx);
                workspace.work_items[0].description = Some("Owner's newer edit".into());
                cx.notify();
            });
        } else if activation == "pointer" {
            cx.simulate_click(bounds.center(), Modifiers::default());
        } else {
            cx.simulate_mouse_down(bounds.center(), MouseButton::Left, Modifiers::default());
            cx.simulate_mouse_up(
                point(px(1.0), px(1.0)),
                MouseButton::Left,
                Modifiers::default(),
            );
            cx.simulate_keystrokes(if activation == "Enter" {
                "enter"
            } else {
                "space"
            });
        }
        drive_until(cx, |cx| {
            workspace.update_in(cx, |workspace, _, _| {
                workspace.work_item_handoff.pending.is_none()
            })
        });
        let request = requests.try_recv().unwrap();
        assert_eq!(request.item, original);
        assert_eq!(request.project_path, "/selected/native/project");
        assert_eq!(request.model, "codex/fixture-model");
        assert_eq!(request.effort.as_deref(), Some("high"));
        assert_eq!(request.permission_mode, PermissionMode::Plan);
        let id = endpoint.join().unwrap();
        workspace.update_in(cx, |workspace, _, cx| {
            assert!(
                workspace.work_item_handoff.error.is_none(),
                "{:?}",
                workspace.work_item_handoff.error
            );
            assert_eq!(workspace.work_items[0].source, original.source);
            assert_eq!(workspace.work_items[0].planning, original.planning);
            if activation == "late-edit" {
                assert_eq!(workspace.active_view, AppView::Tasks);
                assert_eq!(
                    workspace.work_items[0].description.as_deref(),
                    Some("Owner's newer edit")
                );
            } else {
                assert_eq!(workspace.active_view, AppView::Workbench);
                assert_eq!(workspace.work_items[0].to_work_item(), original);
                let panel = workspace.workbench_panel.as_ref().unwrap().read(cx);
                assert!(panel
                    .current_url(cx)
                    .unwrap()
                    .ends_with(&format!("#handoff={id}")));
            }
        });
    }
}

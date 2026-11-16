//! One modal dialog at a time, and Escape's "was anything open" agrees with
//! what Escape closes.

use super::{date_picker_ui::DatePickerTarget, AppView, Overlay, TopLayer, Workspace};
use crate::{app_state::DesktopState, theme};
use gpui::{AppContext, Entity, TestAppContext, VisualTestContext};
use std::{cell::RefCell, rc::Rc};

fn workspace(cx: &mut TestAppContext) -> (Entity<Workspace>, &mut VisualTestContext) {
    cx.update(|cx| {
        gpui_component::init(cx);
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

#[gpui::test]
fn event_color_menu_scrolls_to_its_last_option(cx: &mut TestAppContext) {
    let (workspace, cx) = workspace(cx);
    cx.simulate_resize(gpui::size(gpui::px(640.0), gpui::px(480.0)));
    workspace.update_in(cx, |workspace, window, cx| {
        workspace.open_event_create(None, window, cx);
        workspace.toggle_event_color_menu(cx);
    });
    cx.run_until_parked();
    workspace.update(cx, |workspace, cx| {
        workspace
            .modal_scroll
            .set_offset(-workspace.modal_scroll.max_offset());
        cx.notify();
    });
    cx.run_until_parked();
    let menu = cx.debug_bounds("desktop-event-color-options").unwrap();
    let last_before = cx
        .debug_bounds("desktop-event-color-option-dragonfruit")
        .unwrap();
    cx.simulate_event(gpui::ScrollWheelEvent {
        position: menu.center(),
        delta: gpui::ScrollDelta::Pixels(gpui::point(gpui::px(0.0), gpui::px(-1000.0))),
        ..Default::default()
    });
    cx.run_until_parked();
    let last = cx
        .debug_bounds("desktop-event-color-option-dragonfruit")
        .unwrap();
    assert!(
        last.top() < last_before.top(),
        "The list must scroll independently of the form"
    );
    assert!(
        last.bottom() <= menu.bottom(),
        "The final color must be visible"
    );
}

#[gpui::test]
fn date_popovers_stay_inside_the_clear_modal_area(cx: &mut TestAppContext) {
    let (workspace, cx) = workspace(cx);
    cx.simulate_resize(gpui::size(gpui::px(640.0), gpui::px(480.0)));
    for target in [
        DatePickerTarget::TaskDue,
        DatePickerTarget::EventStart,
        DatePickerTarget::EventEnd,
    ] {
        workspace.update_in(cx, |workspace, window, cx| {
            if target == DatePickerTarget::TaskDue {
                workspace.open_task_create(window, cx);
                workspace
                    .task_create_due_input
                    .update(cx, |input, cx| input.set_value("2026-08-01", window, cx));
            } else {
                let start = chrono::NaiveDate::from_ymd_opt(2026, 8, 1)
                    .unwrap()
                    .and_hms_opt(9, 0, 0)
                    .unwrap();
                workspace.open_event_create(Some(start), window, cx);
            }
        });
        cx.run_until_parked();
        workspace.update_in(cx, |workspace, window, cx| {
            workspace
                .modal_scroll
                .set_offset(-workspace.modal_scroll.max_offset());
            workspace.toggle_date_picker(target, window, cx);
        });
        cx.run_until_parked();
        let selector = if target == DatePickerTarget::TaskDue {
            "desktop-task-create-dialog"
        } else {
            "desktop-event-create-dialog"
        };
        let dialog = cx.debug_bounds(selector).unwrap();
        let popup = cx.debug_bounds("desktop-date-picker-popover").unwrap();
        let clear = workspace.read_with(cx, |workspace, _| {
            workspace.blur_scrim.clear_bounds().unwrap()
        });
        assert_eq!(
            clear.union(&popup),
            clear,
            "{target:?} must stay outside native blur panes"
        );
        assert!(popup.left() >= gpui::px(0.0));
        assert!(popup.right() <= gpui::px(640.0));
        assert!(popup.top() >= gpui::px(theme::APP_HEADER_HEIGHT_PX));
        assert!(popup.bottom() <= gpui::px(480.0));
        workspace.update(cx, |workspace, cx| workspace.close_date_picker(cx));
        cx.run_until_parked();
        let reset = workspace.read_with(cx, |workspace, _| {
            workspace.blur_scrim.clear_bounds().unwrap()
        });
        assert!(reset.size.width <= dialog.size.width);
        assert!(reset.size.height <= dialog.size.height);
    }
}

#[gpui::test]
fn task_detail_keeps_its_footer_visible_and_planning_reachable(cx: &mut TestAppContext) {
    let (workspace, cx) = workspace(cx);
    for (width, height) in [(640.0, 480.0), (1280.0, 800.0)] {
        cx.simulate_resize(gpui::size(gpui::px(width), gpui::px(height)));
        workspace.update_in(cx, |workspace, window, cx| {
            let item = super::DesktopWorkItem {
                id: "layout-fixture".into(),
                project_id: None,
                title: "Review planning controls".into(),
                description: None,
                status: super::DesktopWorkItemStatus::Todo,
                priority: super::DesktopPriority::Medium,
                due_date: None,
                planning: super::WorkItemPlanning::default(),
                updated_at: super::EpochMs::UNIX_EPOCH,
            };
            let id = item.id.clone();
            workspace.work_items = vec![item];
            workspace.active_view = AppView::Tasks;
            workspace.select_work_item_by_id(&id, window, cx);
        });
        cx.run_until_parked();
        let delete = cx.debug_bounds("desktop-task-detail-delete").unwrap();
        assert!(
            delete.bottom() <= gpui::px(height),
            "Delete task is below the window at {height}px"
        );
        let body = cx.debug_bounds("desktop-task-detail-body").unwrap();
        cx.simulate_event(gpui::ScrollWheelEvent {
            position: body.center(),
            delta: gpui::ScrollDelta::Pixels(gpui::point(gpui::px(0.0), gpui::px(-5000.0))),
            ..Default::default()
        });
        cx.run_until_parked();
        let planning = cx.debug_bounds("desktop-task-detail-planning").unwrap();
        assert!(planning.top() >= body.top());
        assert!(
            planning.bottom() <= body.bottom(),
            "Planning must be reachable by scrolling"
        );
        assert_eq!(cx.debug_bounds("desktop-task-detail-delete"), Some(delete));
    }
}

#[gpui::test]
fn life_privacy_controls_fit_inside_the_card(cx: &mut TestAppContext) {
    let (workspace, cx) = workspace(cx);
    for (width, height) in [(640.0, 480.0), (1280.0, 800.0)] {
        cx.simulate_resize(gpui::size(gpui::px(width), gpui::px(height)));
        for finance in [true, false] {
            workspace.update(cx, |workspace, cx| {
                workspace.active_view = AppView::Life;
                workspace.life_tab = if finance {
                    super::DesktopLifeTab::Finance
                } else {
                    super::DesktopLifeTab::Health
                };
                cx.notify();
            });
            cx.run_until_parked();
            let card = cx.debug_bounds("desktop-life-privacy").unwrap();
            let selectors = if finance {
                &["desktop-life-agent-access", "desktop-life-exact-amounts"][..]
            } else {
                &["desktop-life-agent-access"][..]
            };
            for selector in selectors {
                let control = cx.debug_bounds(selector).unwrap();
                assert!(control.left() >= card.left());
                assert!(
                    control.right() <= card.right(),
                    "{selector} clipped at {width}px"
                );
                assert!(control.top() >= card.top());
                assert!(control.bottom() <= card.bottom());
            }
        }
    }
}

#[gpui::test]
fn settings_scrollbar_drag_reaches_the_end_of_the_page(cx: &mut TestAppContext) {
    let (workspace, cx) = workspace(cx);
    for (width, height) in [(640.0, 480.0), (1280.0, 800.0)] {
        cx.simulate_resize(gpui::size(gpui::px(width), gpui::px(height)));
        workspace.update_in(cx, |workspace, window, cx| {
            workspace.open_settings(&super::OpenSettings, window, cx);
        });
        cx.run_until_parked();
        let bounds = cx
            .debug_bounds("desktop-settings-content")
            .expect("settings content");
        let start = gpui::point(bounds.right() - gpui::px(8.0), bounds.top() + gpui::px(8.0));
        let end = gpui::point(start.x, bounds.bottom() - gpui::px(8.0));
        cx.simulate_mouse_move(start, None, gpui::Modifiers::default());
        cx.simulate_mouse_down(start, gpui::MouseButton::Left, gpui::Modifiers::default());
        cx.simulate_mouse_move(
            end,
            Some(gpui::MouseButton::Left),
            gpui::Modifiers::default(),
        );
        cx.simulate_mouse_up(end, gpui::MouseButton::Left, gpui::Modifiers::default());
        cx.run_until_parked();
        workspace.read_with(cx, |workspace, _| {
            assert!(workspace.settings_scroll.max_offset().y > gpui::px(0.0));
            assert_eq!(
                workspace.settings_scroll.offset().y,
                -workspace.settings_scroll.max_offset().y,
                "dragging the visible settings scrollbar must reach the page end"
            );
            assert_eq!(workspace.settings_section, super::SettingsSection::About);
        });
    }
}

#[gpui::test]
fn event_form_actions_stay_visible_at_compact_and_roomy_sizes(cx: &mut TestAppContext) {
    let (workspace, cx) = workspace(cx);
    for (width, height) in [(640.0, 480.0), (1280.0, 800.0)] {
        cx.simulate_resize(gpui::size(gpui::px(width), gpui::px(height)));
        for editing in [false, true] {
            workspace.update_in(cx, |workspace, window, cx| {
                workspace.open_event_create(None, window, cx);
                if editing {
                    workspace.selected_calendar_event_id = Some("layout-fixture".into());
                }
            });
            cx.run_until_parked();
            let assert_actions_visible = |cx: &mut VisualTestContext| {
                let dialog = cx.debug_bounds("desktop-event-create-dialog").unwrap();
                for selector in ["desktop-event-create-cancel", "desktop-event-create-submit"] {
                    let bounds = cx.debug_bounds(selector).expect("event form action");
                    assert!(bounds.top() >= dialog.top(), "{selector} above dialog");
                    assert!(
                        bounds.bottom() <= dialog.bottom(),
                        "{selector} {bounds:?} clipped by dialog {dialog:?}"
                    );
                    assert!(
                        bounds.bottom() <= gpui::px(height),
                        "{selector} below viewport"
                    );
                }
                if editing {
                    let bounds = cx.debug_bounds("desktop-event-create-delete").unwrap();
                    assert!(
                        bounds.bottom() <= dialog.bottom(),
                        "Delete clipped by dialog"
                    );
                }
            };
            assert_actions_visible(cx);
            let footer = cx.debug_bounds("desktop-event-create-submit").unwrap();
            workspace.update(cx, |workspace, cx| workspace.toggle_event_color_menu(cx));
            cx.run_until_parked();
            assert_actions_visible(cx);
            if height == 480.0 {
                assert_eq!(cx.debug_bounds("desktop-event-create-submit"), Some(footer));
                scroll_form_body_to_end(&workspace, cx, "desktop-event-create-body");
                assert_eq!(cx.debug_bounds("desktop-event-create-submit"), Some(footer));
            }
        }
    }
}

fn scroll_form_body_to_end(
    workspace: &Entity<Workspace>,
    cx: &mut VisualTestContext,
    selector: &'static str,
) {
    let bounds = cx.debug_bounds(selector).expect("form scroll viewport");
    let extent = workspace.read_with(cx, |workspace, _| workspace.modal_scroll.max_offset().y);
    assert!(extent > gpui::px(0.0), "expanded form must overflow");
    cx.simulate_event(gpui::ScrollWheelEvent {
        position: gpui::point(bounds.right() - gpui::px(12.0), bounds.center().y),
        delta: gpui::ScrollDelta::Pixels(gpui::point(gpui::px(0.0), -extent)),
        ..Default::default()
    });
    cx.run_until_parked();
    workspace.read_with(cx, |workspace, _| {
        assert!(workspace.modal_scroll.offset().y < gpui::px(0.0));
    });
}

#[gpui::test]
fn task_form_actions_stay_visible_while_the_body_scrolls(cx: &mut TestAppContext) {
    let (workspace, cx) = workspace(cx);
    for (width, height) in [(640.0, 480.0), (1280.0, 800.0)] {
        cx.simulate_resize(gpui::size(gpui::px(width), gpui::px(height)));
        workspace.update_in(cx, |workspace, window, cx| {
            workspace.open_task_create(window, cx);
        });
        cx.run_until_parked();
        let assert_actions_visible = |cx: &mut VisualTestContext| {
            let dialog = cx.debug_bounds("desktop-task-create-dialog").unwrap();
            for selector in ["desktop-task-create-cancel", "desktop-task-create-submit"] {
                let bounds = cx.debug_bounds(selector).expect("task form action");
                assert!(bounds.top() >= dialog.top());
                assert!(
                    bounds.bottom() <= dialog.bottom(),
                    "{selector} clipped by dialog"
                );
                assert!(bounds.bottom() <= gpui::px(height));
            }
        };
        assert_actions_visible(cx);
        let footer = cx.debug_bounds("desktop-task-create-submit").unwrap();
        workspace.update(cx, |workspace, cx| {
            workspace.toggle_task_create_menu(super::TaskCreateMenu::Status, cx);
        });
        cx.run_until_parked();
        assert_actions_visible(cx);
        if height == 480.0 {
            assert_eq!(cx.debug_bounds("desktop-task-create-submit"), Some(footer));
            scroll_form_body_to_end(&workspace, cx, "desktop-task-create-body");
            assert_eq!(cx.debug_bounds("desktop-task-create-submit"), Some(footer));
        }
    }
}

#[gpui::test]
fn event_date_popover_keeps_the_final_day_reachable_in_a_short_window(cx: &mut TestAppContext) {
    let (workspace, cx) = workspace(cx);
    cx.simulate_resize(gpui::size(gpui::px(640.0), gpui::px(480.0)));
    workspace.update_in(cx, |workspace, window, cx| {
        let start = chrono::NaiveDate::from_ymd_opt(2026, 8, 1)
            .unwrap()
            .and_hms_opt(9, 0, 0)
            .unwrap();
        workspace.open_event_create(Some(start), window, cx);
        workspace.toggle_date_picker(DatePickerTarget::EventStart, window, cx);
    });
    cx.run_until_parked();
    let popup = cx.debug_bounds("desktop-date-picker-popover").unwrap();
    assert!(popup.top() >= gpui::px(theme::APP_HEADER_HEIGHT_PX));
    assert!(popup.bottom() <= gpui::px(480.0));
    let last_day = cx
        .debug_bounds("desktop-date-picker-day-2026-08-31")
        .unwrap();
    assert!(last_day.top() >= popup.top());
    assert!(last_day.bottom() <= popup.bottom());
    assert!(last_day.bottom() <= gpui::px(480.0));
    cx.simulate_click(last_day.center(), gpui::Modifiers::default());
    cx.run_until_parked();
    workspace.read_with(cx, |workspace, cx| {
        assert_eq!(
            workspace.event_create_start_input.read(cx).value().as_ref(),
            "2026-08-31T09:00"
        );
    });
}

#[gpui::test]
fn opening_a_dialog_replaces_the_one_showing(cx: &mut TestAppContext) {
    let (workspace, cx) = workspace(cx);
    workspace.update_in(cx, |workspace, window, cx| {
        workspace.open_task_create(window, cx);
        workspace.toggle_task_create_menu(super::TaskCreateMenu::Status, cx);
        assert_eq!(
            workspace.overlay,
            Some(Overlay::TaskCreate {
                menu: Some(super::TaskCreateMenu::Status)
            })
        );
        workspace.open_settings(&super::OpenSettings, window, cx);
        // The task dialog and its menu went with it; nothing is stacked.
        assert_eq!(
            workspace.overlay,
            Some(Overlay::Settings {
                open_dropdown: None
            })
        );
        assert!(!workspace.task_create_visible());
        assert_eq!(workspace.task_create_menu_open(), None);
        workspace.close_transient_surfaces();
        assert_eq!(workspace.overlay, None);
        assert!(!workspace.overlay_open());
    });
}

#[gpui::test]
fn an_open_date_picker_counts_as_an_overlay(cx: &mut TestAppContext) {
    let (workspace, cx) = workspace(cx);
    workspace.update_in(cx, |workspace, window, cx| {
        workspace.toggle_date_picker(DatePickerTarget::TaskDetailDue, window, cx);
        assert!(workspace.date_picker.is_some());
        // Escape closes it, so Escape must not treat the screen as clear.
        assert!(workspace.overlay_open());
        workspace.close_transient_surfaces();
        assert!(workspace.date_picker.is_none());
        assert!(!workspace.overlay_open());
    });
}

/// Web mounts the command bar and project picker over everything: ⌘K over
/// an open form leaves the form open behind it, and Escape closes both.
#[gpui::test]
fn the_command_bar_opens_over_a_form_and_escape_closes_both(cx: &mut TestAppContext) {
    let (workspace, cx) = workspace(cx);
    workspace.update_in(cx, |workspace, window, cx| {
        workspace.open_task_create(window, cx);
        workspace.focus_command_bar(&super::FocusCommandBar, window, cx);
        assert!(workspace.task_create_visible());
        assert_eq!(workspace.top_layer, Some(TopLayer::CommandBar));
        // ⌘K again closes only the bar.
        workspace.focus_command_bar(&super::FocusCommandBar, window, cx);
        assert!(workspace.task_create_visible());
        assert_eq!(workspace.top_layer, None);
        workspace.focus_command_bar(&super::FocusCommandBar, window, cx);
        workspace.close_overlays(&super::CloseOverlays, window, cx);
        assert_eq!(workspace.overlay, None);
        assert_eq!(workspace.top_layer, None);
        assert!(!workspace.overlay_open());
    });
}

#[gpui::test]
fn the_project_picker_opens_over_a_form_and_escape_closes_both(cx: &mut TestAppContext) {
    let (workspace, cx) = workspace(cx);
    workspace.update_in(cx, |workspace, window, cx| {
        workspace.open_event_create(None, window, cx);
        workspace.open_project_picker(&super::OpenProjectPicker, window, cx);
        assert!(workspace.event_create_visible());
        assert!(workspace.project_picker_visible());
        assert!(workspace.overlay_open());
        workspace.close_overlays(&super::CloseOverlays, window, cx);
        assert!(!workspace.event_create_visible());
        assert!(!workspace.project_picker_visible());
    });
}

/// Both layers draw: the form under the bar.
#[gpui::test]
fn both_layers_render(cx: &mut TestAppContext) {
    let (workspace, cx) = workspace(cx);
    workspace.update_in(cx, |workspace, window, cx| {
        workspace.open_task_create(window, cx);
        workspace.focus_command_bar(&super::FocusCommandBar, window, cx);
    });
    cx.run_until_parked();
    assert!(cx.debug_bounds("desktop-command-bar").is_some());
    assert!(cx.debug_bounds("desktop-task-create-dialog").is_some());
}

#[gpui::test]
fn closing_project_selection_cannot_open_a_thread_later(cx: &mut TestAppContext) {
    let (workspace, cx) = workspace(cx);
    let directory = tempfile::tempdir().expect("project fixture");
    let count = workspace.read_with(cx, |workspace, _| workspace.threads.len());
    workspace.update_in(cx, |workspace, window, cx| {
        workspace.open_project_picker(&super::OpenProjectPicker, window, cx);
        workspace.select_project_location(
            directory.path().to_string_lossy().into_owned(),
            "Canceled project".into(),
            window,
            cx,
        );
        workspace.close_transient_surfaces();
        workspace.activate_sidebar_view(AppView::Today, window, cx);
        // Reopening a picker must not revive the earlier selection.
        workspace.open_project_picker(&super::OpenProjectPicker, window, cx);
    });
    cx.run_until_parked();
    workspace.read_with(cx, |workspace, _| {
        assert_eq!(workspace.threads.len(), count);
        assert_eq!(workspace.active_view, AppView::Today);
        assert!(workspace.project_picker_visible());
    });
}

#[gpui::test]
fn record_deletion_requires_confirmation_for_the_same_record(cx: &mut TestAppContext) {
    let (workspace, cx) = workspace(cx);
    workspace.update_in(cx, |workspace, window, cx| {
        let document = bruhs_models::KnowledgeDocument::new("Keep this draft", "Body", 0);
        let second = bruhs_models::KnowledgeDocument::new("Another document", "Body", 0);
        let id = document.id.clone();
        let second_id = second.id.clone();
        workspace.knowledge_documents = vec![document, second];
        workspace.active_view = AppView::DocumentDetail {
            document_id: id.clone(),
        };
        workspace.request_record_delete(window, cx);
        assert_eq!(workspace.knowledge_documents.len(), 2);
        workspace.close_transient_surfaces();
        assert_eq!(workspace.knowledge_documents.len(), 2);

        workspace.request_record_delete(window, cx);
        workspace.active_view = AppView::DocumentDetail {
            document_id: second_id,
        };
        workspace.confirm_record_delete(window, cx);
        assert_eq!(workspace.knowledge_documents.len(), 2);

        workspace.active_view = AppView::DocumentDetail {
            document_id: id.clone(),
        };
        workspace.request_record_delete(window, cx);
        workspace.confirm_record_delete(window, cx);
        assert_eq!(workspace.knowledge_documents.len(), 1);
        assert_ne!(workspace.knowledge_documents[0].id, id);
        assert_eq!(workspace.active_view, AppView::Knowledge);
        assert!(!workspace.overlay_open());
    });
}

#[gpui::test]
fn desktop_palette_fits_short_window_with_scrollable_last_action(cx: &mut TestAppContext) {
    let (workspace, cx) = workspace(cx);
    cx.simulate_resize(gpui::size(gpui::px(640.0), gpui::px(480.0)));
    workspace.update_in(cx, |workspace, window, cx| {
        workspace.focus_command_bar(&super::FocusCommandBar, window, cx);
    });
    cx.run_until_parked();
    let bounds = cx
        .debug_bounds("desktop-command-bar")
        .expect("visible command bar");
    assert!(bounds.bottom() <= gpui::px(480.0));
    cx.simulate_keystrokes("up");
    cx.run_until_parked();
    workspace.update(cx, |workspace, cx| {
        assert!(workspace.command_scroll.offset().y < gpui::px(0.0));
        assert_eq!(
            workspace.command_selected_index,
            workspace.filtered_palette_items(cx).len() - 1
        );
    });
}

#[gpui::test]
fn capture_actions_stay_visible_when_tabbed_to_in_a_short_window(cx: &mut TestAppContext) {
    let (workspace, cx) = workspace(cx);
    cx.simulate_resize(gpui::size(gpui::px(640.0), gpui::px(480.0)));
    workspace.update_in(cx, |workspace, window, cx| {
        workspace.open_capture_dialog(&super::OpenCapture, window, cx);
        workspace.capture_input.update(cx, |input, cx| {
            input.set_value("Synthetic keyboard capture", window, cx);
        });
    });
    cx.run_until_parked();
    let mut reached_submit = false;
    // Five type tabs, the textarea, close, cancel, submit, and a scrollbar.
    for _ in 0..bruhs_models::CaptureKind::ALL.len() + 5 {
        cx.simulate_keystrokes("tab");
        cx.run_until_parked();
        reached_submit = workspace.update_in(cx, |workspace, window, _| {
            workspace.capture_submit_focus.is_focused(window)
        });
        if reached_submit {
            break;
        }
    }
    assert!(reached_submit, "Tab must reach the capture action");
    let submit = cx
        .debug_bounds("desktop-capture-submit")
        .expect("capture action");
    let dialog = cx
        .debug_bounds("desktop-modal-footer")
        .expect("pinned actions");
    assert!(submit.top() >= dialog.top());
    assert!(submit.bottom() <= gpui::px(480.0));
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    workspace.read_with(cx, |workspace, _| {
        assert!(workspace
            .work_items
            .iter()
            .any(|item| item.title == "Synthetic keyboard capture"));
        assert!(!workspace.overlay_open());
    });
}

#[gpui::test]
fn scheduling_drawer_overlays_calendar_and_has_real_scroll_extent(cx: &mut TestAppContext) {
    let (workspace, cx) = workspace(cx);
    cx.simulate_resize(gpui::size(gpui::px(640.0), gpui::px(480.0)));
    workspace.update_in(cx, |workspace, window, cx| {
        workspace.activate_primary_view(AppView::Calendar, window, cx);
    });
    cx.run_until_parked();
    let before = cx
        .debug_bounds("desktop-calendar-page")
        .expect("calendar page");
    workspace.update_in(cx, |workspace, window, cx| {
        workspace.toggle_scheduling_panel(window, cx);
    });
    cx.run_until_parked();
    let after = cx
        .debug_bounds("desktop-calendar-page")
        .expect("calendar page");
    let drawer = cx
        .debug_bounds("desktop-scheduling-scroll")
        .expect("scheduling drawer");
    assert_eq!(before, after, "opening the drawer must not shrink its page");
    assert!(drawer.right() <= after.right());
    assert!(drawer.bottom() <= after.bottom());
    workspace.read_with(cx, |workspace, _| {
        assert!(workspace.scheduling_scroll.max_offset().y > gpui::px(0.0));
    });
}

/// Dialogs sit in the middle of the window (shadcn `Dialog`), not hung from
/// the top. The command bar and the pickers whose lists change length stay
/// anchored at the top so they do not jump while you type or browse.
#[gpui::test]
fn dialogs_are_centered_in_the_window(cx: &mut TestAppContext) {
    let (workspace, cx) = workspace(cx);
    for (width, height) in [(640.0, 480.0), (1280.0, 800.0)] {
        cx.simulate_resize(gpui::size(gpui::px(width), gpui::px(height)));
        cx.run_until_parked();
        for (selector, open) in [
            ("desktop-task-create-dialog", 0),
            ("desktop-event-create-dialog", 1),
            ("desktop-modal-dialog", 2),
        ] {
            workspace.update_in(cx, |workspace, window, cx| match open {
                0 => workspace.open_task_create(window, cx),
                1 => workspace.open_event_create(None, window, cx),
                _ => workspace.open_capture_dialog(&super::OpenCapture, window, cx),
            });
            cx.run_until_parked();
            let dialog = cx.debug_bounds(selector).expect(selector);
            let above = dialog.top();
            let below = gpui::px(height) - dialog.bottom();
            assert!(
                (above - below).abs() <= gpui::px(1.0),
                "{selector} at {width}x{height} is not centered: {above:?} above, {below:?} below"
            );
            assert!(above >= gpui::px(0.0) && below >= gpui::px(0.0));
            workspace.update(cx, |workspace, cx| {
                workspace.close_transient_surfaces();
                cx.notify();
            });
            cx.run_until_parked();
        }
    }
}

/// Like web, a document connects a record or cites a source through a
/// search picker: the trigger opens it, typing narrows the matches, the
/// arrow keys move the highlight, and Enter adds the highlighted match.
#[gpui::test]
fn knowledge_record_picker_searches_and_adds_like_web(cx: &mut TestAppContext) {
    let (workspace, cx) = workspace(cx);
    let id = workspace.update_in(cx, |workspace, _window, cx| {
        let document = bruhs_models::KnowledgeDocument::new("Plan", "Body", 0);
        let id = document.id.clone();
        workspace.knowledge_documents = vec![
            document,
            bruhs_models::KnowledgeDocument::new("Café notes", "Body", 0),
            bruhs_models::KnowledgeDocument::new("Roadmap", "Body", 0),
        ];
        workspace.source_clips = vec![
            bruhs_models::SourceClip::new("Pricing study", "https://example.com/a", "", 0),
            bruhs_models::SourceClip::new("Survey", "https://example.com/b", "", 0),
        ];
        workspace.active_view = AppView::DocumentDetail {
            document_id: id.clone(),
        };
        cx.notify();
        id
    });
    let click = |cx: &mut VisualTestContext, selector: &'static str| {
        cx.run_until_parked();
        let bounds = cx
            .debug_bounds(selector)
            .unwrap_or_else(|| panic!("{selector} should be rendered"));
        cx.simulate_click(bounds.center(), gpui::Modifiers::default());
        cx.run_until_parked();
    };

    click(cx, "desktop-record-picker-trigger-link");
    assert!(cx.debug_bounds("desktop-record-picker").is_some());
    cx.simulate_input("road");
    cx.run_until_parked();
    assert!(cx.debug_bounds("desktop-record-picker-option-0").is_some());
    assert!(
        cx.debug_bounds("desktop-record-picker-option-1").is_none(),
        "only Roadmap matches"
    );
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();

    click(cx, "desktop-record-picker-trigger-citation");
    cx.simulate_keystrokes("down enter");
    cx.run_until_parked();

    workspace.read_with(cx, |workspace, _| {
        assert!(workspace.top_layer.is_none(), "choosing closes the picker");
        let document = workspace
            .knowledge_documents
            .iter()
            .find(|document| document.id == id)
            .expect("the open document");
        let links: Vec<_> = document
            .links
            .iter()
            .map(|link| link.label.as_str())
            .collect();
        assert_eq!(links, ["Roadmap"]);
        let cited = workspace
            .source_clips
            .iter()
            .find(|clip| clip.id == document.citations[0].clip_id)
            .map(|clip| clip.title.as_str());
        assert_eq!(cited, Some("Survey"), "Down moved to the second source");
    });
}

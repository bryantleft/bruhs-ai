use super::{
    DesktopPriority, DesktopWorkItem, DesktopWorkItemStatus, FocusCommandBar, PaletteItem,
    Workspace,
};
use crate::{app_state::DesktopState, theme};
use bruhs_models::{AppView, WorkItemPlanning};
use gpui::{AppContext, Entity, EntityInputHandler, TestAppContext, VisualTestContext};
use std::{cell::RefCell, collections::HashSet, rc::Rc};

mod meeting_note_search_tests;
mod record_routing_tests;

fn workspace(cx: &mut TestAppContext) -> (Entity<Workspace>, &mut VisualTestContext) {
    cx.update(|cx| {
        gpui_component::init(cx);
        gpui_component::Theme::change(gpui_component::ThemeMode::Dark, None, cx);
        theme::apply_bruhs_theme(cx);
        cx.set_global(DesktopState::new().expect("desktop state"));
    });
    let entity = Rc::new(RefCell::new(None));
    let captured = entity.clone();
    let (_, cx) = cx.add_window_view(|window, cx| {
        let workspace = cx.new(|cx| Workspace::new(window, cx));
        *captured.borrow_mut() = Some(workspace.clone());
        gpui_component::Root::new(workspace, window, cx)
    });
    let workspace = entity.borrow().clone().expect("workspace");
    (workspace, cx)
}

fn task(id: &str, title: &str) -> DesktopWorkItem {
    DesktopWorkItem {
        id: id.into(),
        title: title.into(),
        project_id: None,
        description: None,
        status: DesktopWorkItemStatus::Todo,
        priority: DesktopPriority::None,
        due_date: None,
        planning: WorkItemPlanning::default(),
        updated_at: bruhs_models::EpochMs::UNIX_EPOCH,
    }
}

#[gpui::test]
fn native_palette_should_find_and_activate_task_from_keyboard(cx: &mut TestAppContext) {
    let (workspace, cx) = workspace(cx);
    workspace.update_in(cx, |workspace, window, cx| {
        workspace.work_items = vec![task("palette-task", "uniquequartz task")];
        workspace.focus_command_bar(&FocusCommandBar, window, cx);
    });
    cx.run_until_parked();
    cx.simulate_input("uniquequartz");
    cx.run_until_parked();
    workspace.update_in(cx, |workspace, _, cx| {
        let labels: Vec<_> = workspace
            .filtered_palette_items(cx)
            .iter()
            .map(PaletteItem::label)
            .collect();
        assert_eq!(labels, ["Task: uniquequartz task"]);
    });
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    workspace.update_in(cx, |workspace, _, cx| {
        assert_eq!(workspace.active_view, AppView::Tasks);
        assert_eq!(
            workspace.selected_work_item_id.as_deref(),
            Some("palette-task")
        );
        assert!(!workspace.command_bar_visible());
        assert_eq!(
            workspace.task_detail_title_input.read(cx).value().as_ref(),
            "uniquequartz task"
        );
    });
}

#[gpui::test]
fn native_search_should_refresh_after_same_count_task_edit(cx: &mut TestAppContext) {
    let (workspace, cx) = workspace(cx);
    workspace.update_in(cx, |workspace, _, cx| {
        workspace.work_items = vec![task("edited-task", "oldquartz")];
        workspace.selected_work_item_id = Some("edited-task".into());
        assert_eq!(workspace.universal_search("oldquartz", cx).1.len(), 1);
        workspace.update_selected_work_item_title("newquartz".into(), cx);
        assert_eq!(workspace.universal_search("newquartz", cx).1.len(), 1);
        assert!(workspace.universal_search("oldquartz", cx).1.is_empty());
    });
}

#[test]
fn native_palette_empty_results_should_not_consume_editing_keys() {
    for key in ["Backspace", "Escape", "Enter", "a"] {
        assert_eq!(
            Workspace::move_command_selected_index(0, 0, key),
            None,
            "{key}"
        );
    }
}

#[gpui::test]
fn native_palette_keyboard_indices_should_follow_visible_groups(cx: &mut TestAppContext) {
    let (workspace, cx) = workspace(cx);
    workspace.update_in(cx, |workspace, _, cx| {
        let mut visited = HashSet::new();
        let mut previous = None;
        for item in workspace.filtered_palette_items(cx) {
            let group = item.group();
            if previous != Some(group) {
                assert!(visited.insert(group), "noncontiguous group: {group}");
                previous = Some(group);
            }
        }
    });
}

#[gpui::test]
fn native_palette_should_search_hydrated_domains(cx: &mut TestAppContext) {
    use bruhs_models::{
        FinanceTransaction, InboxConversation, InboxSource, KnowledgeDocument, Meeting,
        PersonProfile, SchedulingLink, SearchKind, SourceClip, WorkoutLog,
    };
    let (workspace, cx) = workspace(cx);
    workspace.update_in(cx, |workspace, window, cx| {
        workspace.work_items = vec![task("task", "quartz task")];
        workspace.projects = vec![super::DesktopProject {
            id: "project".into(),
            name: "quartz project".into(),
        }];
        let start = chrono::NaiveDate::from_ymd_opt(2026, 9, 11)
            .unwrap()
            .and_hms_opt(12, 0, 0)
            .unwrap();
        workspace.calendar_events = vec![super::DesktopCalendarEvent {
            id: "event".into(),
            title: "quartz event".into(),
            start_at: start,
            end_at: start + chrono::Duration::hours(1),
            all_day: false,
            color: None,
            notes: None,
        }];
        workspace.scheduling_preferences.links = vec![SchedulingLink::new("quartz booking", 30)];
        workspace.knowledge_documents = vec![KnowledgeDocument::new("quartz document", "body", 1)];
        workspace.source_clips = vec![SourceClip::new(
            "quartz clip",
            "https://example.test",
            "body",
            1,
        )];
        workspace.inbox_conversations = vec![InboxConversation::new(
            InboxSource::Email,
            "quartz conversation",
            vec![],
            1,
        )];
        workspace.life_context.finance.transactions = vec![FinanceTransaction::manual(
            "quartz merchant",
            "food",
            -100,
            1,
        )];
        workspace.life_context.health.workouts = vec![WorkoutLog::manual("quartz workout", 30, 1)];
        workspace.meetings_hub.update(cx, |hub, _| {
            hub.people = vec![PersonProfile::new("quartz person")];
            hub.meetings = vec![Meeting::new("quartz meeting", 1.0)];
            let meeting_id = hub.meetings[0].id.clone();
            hub.segments.insert(
                meeting_id.clone(),
                vec![bruhs_models::TranscriptSegment {
                    id: "segment".into(),
                    meeting_id,
                    speaker_id: None,
                    start_ms: 0,
                    end_ms: 1000,
                    text: "quartz transcript".into(),
                    confidence: 1.0,
                    partial: false,
                    speaker_confidence: None,
                }],
            );
        });
        workspace
            .command_input
            .update(cx, |input, cx| input.set_value("quartz", window, cx));
        let kinds: HashSet<_> = workspace
            .filtered_palette_items(cx)
            .into_iter()
            .filter_map(|item| match item {
                PaletteItem::Record(hit) => Some(hit.document.kind),
                _ => None,
            })
            .collect();
        assert_eq!(
            kinds,
            HashSet::from([
                SearchKind::Task,
                SearchKind::Project,
                SearchKind::Event,
                SearchKind::SchedulingLink,
                SearchKind::Document,
                SearchKind::Clip,
                SearchKind::Conversation,
                SearchKind::Finance,
                SearchKind::Health,
                SearchKind::Person,
                SearchKind::Meeting,
                SearchKind::Transcript,
            ])
        );
    });
}

#[gpui::test]
fn native_palette_should_scroll_keyboard_selection_into_view(cx: &mut TestAppContext) {
    let (workspace, cx) = workspace(cx);
    workspace.update_in(cx, |workspace, window, cx| {
        workspace.work_items = (0..10_000)
            .map(|i| task(&format!("task-{i}"), &format!("quartz {i:03}")))
            .collect();
        workspace.focus_command_bar(&FocusCommandBar, window, cx);
    });
    cx.run_until_parked();
    cx.simulate_input("quartz");
    cx.run_until_parked();
    cx.simulate_keystrokes("up");
    cx.run_until_parked();
    let selector = workspace.update_in(cx, |workspace, _, cx| {
        let items = workspace.filtered_palette_items(cx);
        assert_eq!(items.len(), 20);
        assert_eq!(workspace.command_selected_index, 19);
        items[19].selector()
    });
    let row = cx
        .debug_bounds(Box::leak(selector.into_boxed_str()))
        .expect("selected record bounds");
    let footer = cx
        .debug_bounds("desktop-command-menu")
        .expect("list bounds");
    assert!(
        row.origin.y >= footer.origin.y
            && row.origin.y + row.size.height <= footer.origin.y + footer.size.height,
        "selected row must be inside the palette: {row:?} vs {footer:?}"
    );
    cx.simulate_click(row.center(), gpui::Modifiers::default());
    cx.run_until_parked();
    workspace.update_in(cx, |workspace, _, _| {
        assert_eq!(workspace.selected_work_item_id.as_deref(), Some("task-19"));
        assert!(!workspace.command_bar_visible());
    });
}

#[gpui::test]
fn native_palette_should_leave_ime_composition_unactivated(cx: &mut TestAppContext) {
    let (workspace, cx) = workspace(cx);
    workspace.update_in(cx, |workspace, window, cx| {
        workspace.work_items = vec![task("task", "quartz")];
        workspace.focus_command_bar(&FocusCommandBar, window, cx);
    });
    cx.run_until_parked();
    workspace.update_in(cx, |workspace, window, cx| {
        workspace.command_input.update(cx, |input, cx| {
            input.replace_and_mark_text_in_range(None, "quartz", None, window, cx);
            assert!(input.marked_text_range(window, cx).is_some());
        });
    });
    cx.run_until_parked();
    workspace.update_in(cx, |workspace, window, cx| {
        workspace.command_input.update(cx, |input, cx| {
            assert!(
                input.marked_text_range(window, cx).is_some(),
                "composition must survive render"
            )
        });
    });
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    workspace.update_in(cx, |workspace, _, _| {
        assert!(workspace.command_bar_visible())
    });
    workspace.update_in(cx, |workspace, window, cx| {
        workspace
            .command_input
            .update(cx, |input, cx| input.unmark_text(window, cx));
    });
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    workspace.update_in(cx, |workspace, _, _| {
        assert!(!workspace.command_bar_visible());
        assert_eq!(workspace.selected_work_item_id.as_deref(), Some("task"));
    });
}

#[gpui::test]
fn native_palette_stationary_pointer_should_not_reset_arrow_navigation(cx: &mut TestAppContext) {
    let (workspace, cx) = workspace(cx);
    workspace.update_in(cx, |workspace, window, cx| {
        workspace.focus_command_bar(&FocusCommandBar, window, cx);
    });
    cx.run_until_parked();
    let row = cx
        .debug_bounds("desktop-command-action-nav-chat")
        .expect("chat action");
    cx.simulate_mouse_move(row.center(), None, gpui::Modifiers::default());
    cx.run_until_parked();
    for _ in 0..40 {
        let expected = workspace.update_in(cx, |workspace, _, cx| {
            (workspace.command_selected_index + 1) % workspace.filtered_palette_items(cx).len()
        });
        cx.simulate_keystrokes("down");
        cx.run_until_parked();
        workspace.update_in(cx, |workspace, _, _| {
            assert_eq!(workspace.command_selected_index, expected)
        });
    }
}

#[gpui::test]
fn native_palette_no_results_should_remain_editable_and_dismissible(cx: &mut TestAppContext) {
    let (workspace, cx) = workspace(cx);
    workspace.update_in(cx, |workspace, window, cx| {
        workspace.focus_command_bar(&FocusCommandBar, window, cx)
    });
    cx.run_until_parked();
    cx.simulate_input("zzabsentquery");
    cx.run_until_parked();
    cx.simulate_keystrokes("backspace");
    cx.run_until_parked();
    workspace.update_in(cx, |workspace, _, cx| {
        assert_eq!(
            workspace.command_input.read(cx).value().as_ref(),
            "zzabsentquer"
        )
    });
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    workspace.update_in(cx, |workspace, _, _| {
        assert!(!workspace.command_bar_visible())
    });
}

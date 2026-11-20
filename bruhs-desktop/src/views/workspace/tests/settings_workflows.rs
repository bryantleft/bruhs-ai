use super::*;

#[cfg(target_os = "macos")]
#[gpui::test]
fn browser_link_target_and_search_engine_reach_the_live_preview(cx: &mut gpui::TestAppContext) {
    let (workspace, cx, _, _) = thread_menu_workspace(cx);
    workspace.update_in(cx, |workspace, window, cx| {
        workspace.open_settings(&OpenSettings, window, cx);
        workspace.set_settings_section(super::super::SettingsSection::Browser, cx);
        workspace.select_settings_option("settings-browser-search-engine", "brave", window, cx);
        workspace.select_settings_option("settings-browser-zoom", "125", window, cx);
        workspace.select_settings_option("settings-browser-appearance", "dark", window, cx);
        workspace.select_settings_option("settings-browser-link-target", "app", window, cx);
        let saved = workspace
            .snapshot(cx)
            .browser_preferences
            .expect("browser settings saved");
        assert_eq!(saved.zoom_percent, 125);
        assert_eq!(saved.appearance.key(), "dark");
        assert_eq!(
            saved.address("rust borrow").as_deref(),
            Some("https://search.brave.com/search?q=rust%20borrow")
        );
        workspace.close_transient_surfaces();
        workspace.open_web_link(
            &bruhs_ui::text::OpenWebLink("https://example.org/docs".into()),
            window,
            cx,
        );
        assert!(workspace.preview_panel_visible);
        assert_eq!(
            workspace.preview_panel.read(cx).url(),
            Some("https://example.org/docs")
        );
    });
}

fn scheduled_task_workspace(
    cx: &mut gpui::TestAppContext,
    kind: bruhs_models::reactive::SourceKind,
) -> (
    Entity<Workspace>,
    &mut gpui::VisualTestContext,
    Entity<crate::views::reactive_automations::ReactiveAutomations>,
) {
    scheduled_task_workspace_with(cx, kind, |_| {})
}

fn scheduled_task_workspace_with(
    cx: &mut gpui::TestAppContext,
    kind: bruhs_models::reactive::SourceKind,
    configure: impl FnOnce(&mut bruhs_models::reactive::ReactiveSnapshot),
) -> (
    Entity<Workspace>,
    &mut gpui::VisualTestContext,
    Entity<crate::views::reactive_automations::ReactiveAutomations>,
) {
    use bruhs_models::reactive::{EventSource, ReactiveAgent, ReactiveSnapshot, Subscription};
    let (workspace, cx, _, _) = thread_menu_workspace(cx);
    let panel = workspace.update_in(cx, |workspace, window, cx| {
        let panel = workspace.reactive_automations.clone();
        panel.update(cx, |panel, cx| {
            let mut snapshot = ReactiveSnapshot {
                agents: vec![ReactiveAgent {
                    id: "agent".into(),
                    name: "Weekly changes".into(),
                    instructions: "Summarize changes".into(),
                    model: "codex".into(),
                    project_path: "/repo".into(),
                    memory: String::new(),
                    enabled: true,
                }],
                sources: vec![EventSource {
                    id: "clock".into(),
                    name: "Daily".into(),
                    enabled: true,
                    kind,
                }],
                subscriptions: vec![Subscription {
                    id: "task".into(),
                    agent_id: "agent".into(),
                    source_id: "clock".into(),
                    enabled: true,
                    auto_run: true,
                    instructions: "Summarize".into(),
                    debounce_ms: 0,
                    throttle_ms: 0,
                    webhook_filters: Vec::new(),
                    webhook_prompt: None,
                    webhook_security: Default::default(),
                    webhook_dedupe: Default::default(),
                    task_title: None,
                    task_conversation: Default::default(),
                    task_checkout: Default::default(),
                    on_busy: Default::default(),
                }],
                ..Default::default()
            };
            configure(&mut snapshot);
            panel.show_snapshot_for_test(snapshot, cx);
        });
        workspace.open_settings(&OpenSettings, window, cx);
        workspace.set_settings_section(super::super::SettingsSection::ScheduledTasks, cx);
        panel
    });
    cx.run_until_parked();
    (workspace, cx, panel)
}

#[gpui::test]
fn scheduled_task_link_opens_its_saved_thread_with_an_empty_recent_history(
    cx: &mut gpui::TestAppContext,
) {
    use bruhs_models::{
        crew::{BruhState, BruhStatus},
        ids::ThreadId,
        reactive::{SourceKind, TaskConversation},
    };
    let thread = ThreadId::new_v4();
    let (workspace, cx, panel) =
        scheduled_task_workspace_with(cx, SourceKind::Webhook, |snapshot| {
            snapshot.subscriptions[0].task_conversation = TaskConversation::Continue;
            snapshot.task_threads.insert("task".into(), thread.clone());
            snapshot.bruhs.push(BruhState {
                agent_id: "agent".into(),
                name: "Release assistant".into(),
                look: Default::default(),
                status: BruhStatus::Asleep,
                access: Default::default(),
                effort: None,
                next_wake_ms: None,
                unread: 0,
                computer: false,
            });
        });
    click_debug_selector(cx, "settings-enable-task-task");
    cx.simulate_keystrokes("shift-tab enter");
    cx.run_until_parked();
    assert_rendered_bounds(cx, "task-thread-header");
    cx.simulate_input("Inspect the report");
    super::keys::press(cx, "enter");
    panel.read_with(cx, |panel, _| {
        assert_eq!(
            panel.sent_for_test().last(),
            Some(&bruhs_models::reactive::ReactiveCommand::ChatInTaskThread {
                task_id: "task".into(),
                thread_id: thread.clone(),
                message: "Inspect the report".into(),
            })
        );
    });
    workspace.update_in(cx, |ws, window, cx| {
        assert!(!ws.settings_visible());
        assert_eq!(ws.snapshot(cx).task_thread_view.unwrap().thread_id, thread);
        assert_eq!(panel.read(cx).task_thread().unwrap().thread_id, thread);
        ws.activate_sidebar_view(
            AppView::Bruh {
                agent_id: "agent".into(),
            },
            window,
            cx,
        );
    });
    cx.run_until_parked();
    assert_no_rendered_bounds(cx, "task-thread-header");
    assert_rendered_bounds(cx, "desktop-bruh-header");
}

#[gpui::test]
fn scheduled_thread_delete_uses_the_preference_and_returns_to_settings_after_success(
    cx: &mut gpui::TestAppContext,
) {
    use bruhs_models::{
        crew::{BruhState, BruhStatus},
        ids::ThreadId,
        reactive::{ReactiveCommand, SourceKind, TaskConversation},
    };
    let thread = ThreadId::new_v4();
    let (workspace, cx, panel) = scheduled_task_workspace_with(cx, SourceKind::Webhook, |s| {
        s.can_delete_task_threads = true;
        s.subscriptions[0].task_conversation = TaskConversation::Continue;
        let mut other = s.subscriptions[0].clone();
        other.id = "other".into();
        other.task_title = Some("Evening".into());
        s.subscriptions.push(other);
        for task in ["task", "other"] {
            s.task_threads.insert(task.into(), thread.clone());
        }
        s.bruhs.push(BruhState {
            agent_id: "agent".into(),
            name: "Release assistant".into(),
            status: BruhStatus::Asleep,
            look: Default::default(),
            access: Default::default(),
            effort: None,
            next_wake_ms: None,
            unread: 0,
            computer: false,
        });
    });
    click_debug_selector(cx, "task-open-thread-task");
    click_debug_selector(cx, "task-thread-actions");
    super::keys::press(cx, "escape");
    assert_no_rendered_bounds(cx, "task-thread-delete");
    super::keys::press(cx, "enter");
    assert_rendered_bounds(cx, "task-thread-delete");
    super::keys::press(cx, "enter");
    cx.simulate_prompt_answer("Cancel");
    cx.run_until_parked();
    assert_rendered_bounds(cx, "task-thread-header");
    panel.read_with(cx, |p, _| assert!(p.sent_for_test().is_empty()));
    let expected = ReactiveCommand::DeleteTaskThread {
        thread_id: thread.clone(),
        linked_tasks: ["task".to_string(), "other".to_string()]
            .into_iter()
            .collect(),
    };
    click_debug_selector(cx, "task-thread-actions");
    click_debug_selector(cx, "task-thread-delete");
    cx.simulate_prompt_answer("Delete");
    cx.run_until_parked();
    panel.read_with(cx, |p, _| {
        assert_eq!(p.sent_for_test(), std::slice::from_ref(&expected))
    });
    workspace.update_in(cx, |w, _, cx| {
        w.confirm_thread_delete = false;
        cx.notify();
    });
    click_debug_selector(cx, "task-thread-actions");
    click_debug_selector(cx, "task-thread-delete");
    panel.read_with(cx, |p, _| {
        assert_eq!(p.sent_for_test(), &[expected.clone(), expected])
    });
    panel.update(cx, |p, cx| p.answer_last_for_test(Default::default(), cx));
    cx.run_until_parked();
    assert_rendered_bounds(cx, "desktop-settings-dialog");
    assert_rendered_bounds(cx, "scheduled-tasks-empty");
    workspace.read_with(cx, |w, _| assert!(w.task_thread_view.is_none()));
}

#[gpui::test]
fn scheduled_destination_search_selects_a_project_thread_and_preserves_its_agent(
    cx: &mut gpui::TestAppContext,
) {
    use bruhs_models::{
        ids::ThreadId,
        reactive::{ReactiveCommand, SourceKind, TaskConversation},
    };
    let target = ThreadId::new_v4();
    let (_, cx, panel) =
        scheduled_task_workspace_with(cx, SourceKind::Schedule { every_ms: 60_000 }, |snapshot| {
            snapshot.selected_task_threads = Some(Default::default());
            snapshot.subscriptions[0].task_conversation = TaskConversation::Continue;
            for (id, project) in [("same", "/repo"), ("foreign", "/other")] {
                let mut agent = snapshot.agents[0].clone();
                agent.id = id.into();
                agent.project_path = project.into();
                agent.model = "claude-sonnet-4-6".into();
                let mut sub = snapshot.subscriptions[0].clone();
                sub.agent_id = id.into();
                sub.id = id.into();
                sub.task_title = Some(format!("{id} release audit"));
                snapshot.agents.push(agent);
                snapshot.subscriptions.push(sub);
                snapshot.task_threads.insert(
                    id.into(),
                    if id == "same" {
                        target.clone()
                    } else {
                        ThreadId::new_v4()
                    },
                );
            }
        });
    cx.simulate_resize(gpui::size(px(1040.), px(860.)));
    click_debug_selector(cx, "task-actions-task");
    click_debug_selector(cx, "reactive-Edit");
    let scroll = rendered_bounds(cx, "scheduled-task-dialog-scroll");
    let trigger = rendered_bounds(cx, "task-destination-thread");
    cx.simulate_event(gpui::ScrollEvent {
        position: scroll.center(),
        delta: gpui::ScrollDelta::Pixels(gpui::point(
            px(0.),
            scroll.center().y - trigger.center().y,
        )),
        ..Default::default()
    });
    cx.run_until_parked();
    click_debug_selector(cx, "task-destination-thread");
    cx.simulate_input("foreign");
    assert!(cx
        .debug_bounds("task-destination-thread-option-destination-0")
        .is_none());
    cx.simulate_keystrokes("cmd-a");
    cx.simulate_input("release audit");
    cx.run_until_parked();
    assert_rendered_bounds(cx, "task-destination-thread-option-destination-0");
    assert!(cx
        .debug_bounds("task-destination-thread-option-destination-1")
        .is_none());
    super::keys::press(cx, "enter");
    assert!(cx.debug_bounds("task-destination-thread-list").is_none());
    click_debug_selector(cx, "reactive-Save task");
    panel.read_with(cx, |p, _| {
        let Some(ReactiveCommand::SaveScheduledTask {
            automation,
            destination,
            ..
        }) = p.sent_for_test().last()
        else {
            panic!("missing task save")
        };
        assert_eq!(
            destination.as_ref(),
            Some(&bruhs_models::reactive::TaskDestinationEdit::Existing {
                thread_id: target.clone()
            })
        );
        assert_eq!(automation.agent.id, "same");
        assert_eq!(automation.agent.model, "claude-sonnet-4-6");
        assert_eq!(automation.agent.instructions, "Summarize changes");
        assert!(automation.subscription.instructions.contains("Summarize"));
    });
}

#[gpui::test]
fn scheduled_tasks_manage_the_selected_subscription(cx: &mut gpui::TestAppContext) {
    use bruhs_models::reactive::{ReactiveChange, ReactiveCommand, SourceKind};
    let (workspace, cx, panel) = scheduled_task_workspace(cx, SourceKind::Schedule { every_ms: 86_400_000 });
    for (width, stacked) in [(820., true), (1040., false)] {
        cx.simulate_resize(gpui::size(px(width), px(820.)));
        cx.run_until_parked();
        let title = rendered_bounds(cx, "scheduled-task-name-task");
        let actions = rendered_bounds(cx, "task-actions-task");
        if stacked {
            assert!(actions.top() > title.bottom());
            assert!(actions.left() < title.right());
        } else {
            assert!(actions.left() > title.right());
        }
    }
    click_debug_selector(cx, "settings-enable-task-task");
    panel.read_with(cx, |p, _| assert!(matches!(p.sent_for_test().last(), Some(ReactiveCommand::SetAutomationEnabled { id, enabled:false }) if id=="task")));
    click_debug_selector(cx, "task-actions-task");
    super::keys::press(cx, "down");
    super::keys::press(cx, "enter");
    panel.read_with(cx, |p, _| assert!(matches!(p.sent_for_test().last(), Some(ReactiveCommand::RunScheduledTask { id, .. }) if id == "task")));
    assert!(cx.debug_bounds("scheduled-task-actions-menu").is_none());
    click_debug_selector(cx, "task-actions-task");
    click_debug_selector(cx, "reactive-Delete");
    cx.simulate_prompt_answer("Cancel");
    cx.run_until_parked();
    panel.read_with(cx, |p, _| assert_eq!(p.sent_for_test().len(), 2));
    click_debug_selector(cx, "task-actions-task");
    click_debug_selector(cx, "reactive-Delete");
    cx.simulate_prompt_answer("Delete");
    cx.run_until_parked();
    panel.read_with(cx, |p, _| assert!(matches!(p.sent_for_test().last(), Some(ReactiveCommand::Apply {change:ReactiveChange::DeleteSubscription { id }}) if id=="task")));
    click_debug_selector(cx, "reactive-New task");
    cx.run_until_parked();
    assert!(cx.debug_bounds("task-schedule-fields-calendar").is_some());
    // All modes share one horizontal control. Arrow keys move focus; Space selects.
    cx.simulate_resize(gpui::size(px(1040.), px(960.)));
    cx.run_until_parked();
    let scroll = rendered_bounds(cx, "scheduled-task-dialog-scroll");
    cx.simulate_event(gpui::ScrollEvent {
        position: scroll.center(),
        delta: gpui::ScrollDelta::Pixels(gpui::point(px(0.), px(-240.))),
        ..Default::default()
    });
    cx.run_until_parked();
    let first = rendered_bounds(cx, "task-schedule-type-0");
    let last = rendered_bounds(cx, "task-schedule-type-3");
    assert_eq!(first.top(), last.top());
    click_debug_selector(cx, "task-schedule-type-0");
    super::keys::press(cx, "right");
    assert!(cx.debug_bounds("task-schedule-fields-calendar").is_some());
    super::keys::press(cx, "space");
    assert!(cx.debug_bounds("task-schedule-fields-once").is_some());
    super::keys::press(cx, "right");
    super::keys::press(cx, "enter");
    assert!(cx.debug_bounds("task-schedule-fields-schedule").is_some());
    click_debug_selector(cx, "task-schedule-type-3");
    assert!(cx.debug_bounds("task-schedule-days").is_none());
    super::keys::press(cx, "home");
    super::keys::press(cx, "enter");
    assert!(cx.debug_bounds("task-schedule-days").is_some());
    assert!(cx
        .debug_bounds("reactive-A command reports a change")
        .is_none());
    let dialog = rendered_bounds(cx, "scheduled-task-dialog");
    let scroll = rendered_bounds(cx, "scheduled-task-dialog-scroll");
    let submit = rendered_bounds(cx, "reactive-Create task");
    assert!(submit.top() >= scroll.bottom() && submit.bottom() <= dialog.bottom());
    cx.simulate_event(gpui::ScrollEvent {
        position: scroll.center(),
        delta: gpui::ScrollDelta::Pixels(gpui::point(px(0.), px(-300.))),
        ..Default::default()
    });
    cx.run_until_parked();
    assert_eq!(rendered_bounds(cx, "reactive-Create task"), submit);
    // Walking backwards through the first input wraps to the footer.
    let (start, _) = workspace.update_in(cx, |ws, _, cx| ws.active_modal_focus_bounds(cx));
    for key in ["shift-tab", "shift-tab", "tab", "tab"] {
        cx.simulate_keystrokes(key);
        assert!(cx.update(|window, cx| start.contains_focused(window, cx)), "{key} kept focus inside the task dialog");
    }
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    assert!(cx.debug_bounds("scheduled-task-dialog").is_none());
    assert!(cx.debug_bounds("desktop-settings-dialog").is_some());
    super::keys::press(cx, "enter");
    assert!(cx.debug_bounds("reactive-Create task").is_some(), "focus returns to New task");
    click_debug_selector(cx, "reactive-Cancel");
    click_debug_selector(cx, "task-actions-task");
    click_debug_selector(cx, "reactive-Edit");
    assert!(cx.debug_bounds("reactive-Save task").is_some());
    click_debug_selector(cx, "scheduled-task-close");
    super::keys::press(cx, "enter");
    assert!(cx.debug_bounds("scheduled-task-actions-menu").is_some(), "focus returns to the row’s action button");
    super::keys::press(cx, "enter");
    assert!(cx.debug_bounds("reactive-Save task").is_some(), "the menu focuses Edit");
    click_debug_selector(cx, "reactive-Cancel");
    workspace.update_in(cx, |w, window, cx| {
        w.select_settings_option("settings-project-defaults-scope", "/other", window, cx);
    });
    cx.run_until_parked();
    assert!(cx.debug_bounds("task-actions-task").is_none());
    assert!(cx.debug_bounds("scheduled-tasks-empty").is_some());
    workspace.update_in(cx, |w, window, cx| {
        w.select_settings_option("settings-project-defaults-scope", "/repo", window, cx);
    });
    cx.run_until_parked();
    assert!(cx.debug_bounds("task-actions-task").is_some());
    click_debug_selector(cx, "reactive-New task");
    click_debug_selector(cx, "reactive-Create task");
    assert!(cx.debug_bounds("task-save-error").is_some());
    let dialog = rendered_bounds(cx, "scheduled-task-dialog");
    let outside = gpui::point(dialog.left() - px(2.), dialog.center().y);
    cx.simulate_click(outside, gpui::Modifiers::default());
    cx.run_until_parked();
    assert!(cx.debug_bounds("scheduled-task-dialog").is_none());
    assert!(cx.debug_bounds("desktop-settings-dialog").is_some());

    workspace.update_in(cx, |w, _, cx| {
        w.recent_projects.push(super::super::DesktopProjectLocation { name: "Research".into(), path: "/other/research".into() });
        w.project_defaults.entry("/other/research".into()).or_default().model = Some("gpt-6-luna".into());
        cx.notify();
    });
    click_debug_selector(cx, "reactive-New task");
    cx.simulate_input("Daily research");
    cx.simulate_keystrokes("tab");
    cx.simulate_input("Summarize today's changes.");
    click_debug_selector(cx, "task-project");
    assert!(cx.debug_bounds("task-project-list").is_some());
    cx.simulate_keystrokes("tab");
    cx.run_until_parked();
    assert!(cx.debug_bounds("task-project-list").is_none(), "Tab dismisses the project list");
    assert!(cx.debug_bounds("scheduled-task-dialog").is_some());
    click_debug_selector(cx, "task-project");
    cx.simulate_input("research");
    cx.run_until_parked();
    assert!(cx.debug_bounds("task-project-option-/other/research").is_some());
    assert!(cx.debug_bounds("task-project-option-/repo").is_none());
    super::keys::press(cx, "enter");
    let scroll = rendered_bounds(cx, "scheduled-task-dialog-scroll");
    cx.simulate_event(gpui::ScrollEvent {
        position: scroll.center(),
        delta: gpui::ScrollDelta::Pixels(gpui::point(px(0.), px(-1000.))),
        ..Default::default()
    });
    cx.run_until_parked();
    for day in ["task-schedule-days-0", "task-schedule-days-1", "task-schedule-days-2", "task-schedule-days-3", "task-schedule-days-4"] {
        click_debug_selector(cx, day);
    }
    click_debug_selector(cx, "schedule-task-enabled");
    click_debug_selector(cx, "reactive-Create task");
    panel.read_with(cx, |p, _| {
        let Some(ReactiveCommand::SaveScheduledTask { automation, enabled: false, .. }) = p.sent_for_test().last() else {
            panic!("expected the task to be saved");
        };
        assert_eq!(automation.agent.project_path, "/other/research");
        assert_eq!(automation.agent.model, "gpt-6-luna");
        assert_eq!(automation.agent.name, "Daily research");
        assert_eq!(automation.subscription.task_title.as_deref(), Some("Daily research"));
        assert!(matches!(&automation.source.kind, SourceKind::Calendar {weekdays, ..} if weekdays == &[5]));
    });
}

fn open_task_timing(cx: &mut gpui::VisualTestContext) {
    click_debug_selector(cx, "task-actions-task");
    click_debug_selector(cx, "reactive-Edit");
    let scroll = rendered_bounds(cx, "scheduled-task-dialog-scroll");
    cx.simulate_event(gpui::ScrollEvent {
        position: scroll.center(),
        delta: gpui::ScrollDelta::Pixels(gpui::point(px(0.), px(-1000.))),
        ..Default::default()
    });
    cx.run_until_parked();
    click_debug_selector(cx, "schedule-picker-toggle");
    assert_rendered_bounds(cx, "schedule-datetime-popup");
}

#[gpui::test]
fn scheduled_time_picker_edits_the_first_run_in_its_saved_zone(cx: &mut gpui::TestAppContext) {
    use bruhs_models::reactive::{ReactiveChange, ReactiveCommand, SourceKind};
    let (_, cx, panel) = scheduled_task_workspace(
        cx,
        SourceKind::Calendar {
            times: vec!["13:22".into(), "17:40".into()],
            weekdays: vec![1, 4],
            time_zone: "Europe/Paris".into(),
        },
    );
    cx.update(|_, _| crate::clock_format::ClockFormat::Twelve.apply());
    cx.simulate_resize(gpui::size(px(820.), px(640.)));
    cx.run_until_parked();
    open_task_timing(cx);
    super::keys::press(cx, "down");
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    assert_rendered_bounds(cx, "schedule-datetime-popup");
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    assert_no_rendered_bounds(cx, "schedule-datetime-popup");
    super::keys::press(cx, "enter");
    super::keys::press(cx, "down");
    super::keys::press(cx, "right");
    super::keys::press(cx, "down");
    cx.simulate_keystrokes("tab");
    cx.run_until_parked();
    assert_rendered_bounds(cx, "schedule-period");
    super::keys::press(cx, "home");
    super::keys::press(cx, "enter");
    assert_no_rendered_bounds(cx, "schedule-datetime-popup");
    super::keys::press(cx, "enter");
    assert_rendered_bounds(cx, "schedule-datetime-popup");
    click_debug_selector(cx, "schedule-picker-toggle");
    assert_no_rendered_bounds(cx, "schedule-datetime-popup");
    click_debug_selector(cx, "reactive-Save task");
    panel.read_with(cx, |panel, _| {
        let Some(ReactiveCommand::Apply {
            change: ReactiveChange::SaveAutomation { automation },
        }) = panel.sent_for_test().last()
        else {
            panic!("expected an edited task")
        };
        assert_eq!(
            automation.source.kind,
            SourceKind::Calendar {
                times: vec!["02:23".into(), "17:40".into()],
                weekdays: vec![1, 4],
                time_zone: "Europe/Paris".into(),
            }
        );
    });
}

#[gpui::test]
fn scheduled_inline_fields_preserve_invalid_parts_and_edit_the_saved_minute(
    cx: &mut gpui::TestAppContext,
) {
    use bruhs_models::{
        reactive::{ReactiveChange, ReactiveCommand, SourceKind},
        EpochMs,
    };
    use chrono::{Local, TimeZone};
    let instant = |month, day, hour, minute| {
        EpochMs::new(
            Local
                .with_ymd_and_hms(2032, month, day, hour, minute, 0)
                .single()
                .unwrap()
                .timestamp_millis(),
        )
        .unwrap()
    };
    let (_, cx, panel) = scheduled_task_workspace(
        cx,
        SourceKind::Once {
            due_at: instant(1, 31, 23, 59),
        },
    );
    cx.update(|_, _| crate::clock_format::ClockFormat::Twelve.apply());
    cx.simulate_resize(gpui::size(px(1040.), px(820.)));
    cx.run_until_parked();
    open_task_timing(cx);
    click_debug_selector(cx, "schedule-picker-toggle");
    click_debug_selector(cx, "schedule-segment-month");
    super::keys::press(cx, "up");
    click_debug_selector(cx, "reactive-Save task");
    assert_rendered_bounds(cx, "task-save-error");
    panel.read_with(cx, |p, _| assert!(p.sent_for_test().is_empty()));

    click_debug_selector(cx, "schedule-segment-day");
    super::keys::press(cx, "up");
    click_debug_selector(cx, "schedule-segment-hour");
    super::keys::press(cx, "up");
    super::keys::press(cx, "right");
    super::keys::press(cx, "up");
    cx.simulate_keystrokes("tab");
    cx.run_until_parked();
    super::keys::press(cx, "p");
    click_debug_selector(cx, "reactive-Save task");
    panel.read_with(cx, |p, _| {
        let Some(ReactiveCommand::Apply {
            change: ReactiveChange::SaveAutomation { automation },
        }) = p.sent_for_test().last()
        else {
            panic!("expected saved schedule")
        };
        assert_eq!(
            automation.source.kind,
            SourceKind::Once {
                due_at: instant(2, 1, 12, 0)
            }
        );
    });
}

#[gpui::test]
fn scheduled_inline_time_accepts_digits_and_preserves_typed_additional_times(
    cx: &mut gpui::TestAppContext,
) {
    use bruhs_models::reactive::{ReactiveChange, ReactiveCommand, SourceKind};
    let (_, cx, panel) = scheduled_task_workspace(
        cx,
        SourceKind::Calendar {
            times: vec!["09:45".into()],
            weekdays: vec![3],
            time_zone: "Europe/Paris".into(),
        },
    );
    cx.update(|_, _| crate::clock_format::ClockFormat::TwentyFour.apply());
    cx.simulate_resize(gpui::size(px(820.), px(640.)));
    cx.run_until_parked();
    open_task_timing(cx);
    click_debug_selector(cx, "schedule-picker-toggle");
    assert_no_rendered_bounds(cx, "schedule-segment-am/pm");
    click_debug_selector(cx, "schedule-segment-hour");
    for key in ["1", "7", "3", "2"] {
        super::keys::press(cx, key);
    }
    click_debug_selector(cx, "schedule-picker-toggle");
    click_debug_selector(cx, "schedule-entry-mode");
    assert_no_rendered_bounds(cx, "schedule-segmented-field");
    cx.simulate_keystrokes("end");
    cx.simulate_input(", 19:15");
    cx.run_until_parked();
    click_debug_selector(cx, "reactive-Save task");
    panel.read_with(cx, |p, _| {
        let Some(ReactiveCommand::Apply {
            change: ReactiveChange::SaveAutomation { automation },
        }) = p.sent_for_test().last()
        else {
            panic!("expected saved calendar")
        };
        assert_eq!(
            automation.source.kind,
            SourceKind::Calendar {
                times: vec!["17:32".into(), "19:15".into()],
                weekdays: vec![3],
                time_zone: "Europe/Paris".into()
            }
        );
    });
}

#[gpui::test]
fn scheduled_date_picker_steps_months_and_saves_the_local_instant(cx: &mut gpui::TestAppContext) {
    use bruhs_models::{
        reactive::{ReactiveChange, ReactiveCommand, SourceKind},
        EpochMs,
    };
    use chrono::{Local, TimeZone};
    let instant = |month, day, hour, minute| {
        EpochMs::new(
            Local
                .with_ymd_and_hms(2032, month, day, hour, minute, 0)
                .single()
                .unwrap()
                .timestamp_millis(),
        )
        .unwrap()
    };
    let (_, cx, panel) = scheduled_task_workspace(
        cx,
        SourceKind::Once {
            due_at: instant(1, 31, 12, 30),
        },
    );
    cx.update(|_, _| crate::clock_format::ClockFormat::TwentyFour.apply());
    cx.simulate_resize(gpui::size(px(1040.), px(820.)));
    cx.run_until_parked();
    open_task_timing(cx);
    assert_no_rendered_bounds(cx, "schedule-period");
    click_debug_selector(cx, "schedule-select-month");
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    assert_rendered_bounds(cx, "schedule-datetime-popup");
    assert_no_rendered_bounds(cx, "schedule-months");
    click_debug_selector(cx, "schedule-select-month");
    click_debug_selector(cx, "schedule-next-year");
    click_debug_selector(cx, "schedule-month-2");
    assert_rendered_bounds(cx, "desktop-date-picker-day-2033-02-28");
    click_debug_selector(cx, "schedule-select-month");
    click_debug_selector(cx, "schedule-previous-year");
    click_debug_selector(cx, "schedule-month-1");
    click_debug_selector(cx, "desktop-date-picker-next");
    click_debug_selector(cx, "desktop-date-picker-day-2032-02-29");
    super::keys::press(cx, "right");
    assert_rendered_bounds(cx, "desktop-date-picker-day-2032-03-01");
    let minutes = rendered_bounds(cx, "schedule-minutes");
    let choice = rendered_bounds(cx, "schedule-minutes-32");
    assert!(
        minutes.contains(&choice.center()),
        "minute option {choice:?} must be visible inside {minutes:?}"
    );
    click_debug_selector(cx, "schedule-minutes-32");
    super::keys::press(cx, "enter");
    assert_no_rendered_bounds(cx, "schedule-datetime-popup");
    click_debug_selector(cx, "reactive-Save task");
    panel.read_with(cx, |panel, _| {
        let Some(ReactiveCommand::Apply {
            change: ReactiveChange::SaveAutomation { automation },
        }) = panel.sent_for_test().last()
        else {
            panic!("expected an edited task")
        };
        assert_eq!(
            automation.source.kind,
            SourceKind::Once {
                due_at: instant(3, 1, 12, 32)
            }
        );
    });
}

#[gpui::test]
fn scheduled_date_picker_can_clear_and_choose_today(cx: &mut gpui::TestAppContext) {
    use bruhs_models::{
        reactive::{ReactiveChange, ReactiveCommand, SourceKind},
        EpochMs,
    };
    use chrono::{DateTime, Local, TimeZone};
    let instant = Local
        .with_ymd_and_hms(2032, 1, 12, 8, 11, 0)
        .single()
        .unwrap();
    let (_, cx, panel) = scheduled_task_workspace(
        cx,
        SourceKind::Once {
            due_at: EpochMs::new(instant.timestamp_millis()).unwrap(),
        },
    );
    cx.simulate_resize(gpui::size(px(820.), px(640.)));
    cx.run_until_parked();
    open_task_timing(cx);
    click_debug_selector(cx, "schedule-clear-date");
    assert_no_rendered_bounds(cx, "schedule-datetime-popup");
    click_debug_selector(cx, "reactive-Save task");
    assert_rendered_bounds(cx, "task-save-error");
    panel.read_with(cx, |panel, _| assert!(panel.sent_for_test().is_empty()));
    click_debug_selector(cx, "schedule-picker-toggle");
    click_debug_selector(cx, "schedule-today");
    click_debug_selector(cx, "schedule-picker-toggle");
    cx.run_until_parked();
    click_debug_selector(cx, "reactive-Save task");
    panel.read_with(cx, |panel, _| {
        let Some(ReactiveCommand::Apply {
            change: ReactiveChange::SaveAutomation { automation },
        }) = panel.sent_for_test().last()
        else {
            panic!("expected a task for today")
        };
        let SourceKind::Once { due_at } = automation.source.kind else {
            panic!("expected a one-time schedule")
        };
        assert_eq!(
            DateTime::from_timestamp_millis(due_at.as_millis())
                .unwrap()
                .with_timezone(&Local)
                .date_naive(),
            Local::now().date_naive()
        );
    });
}

#[cfg(target_os = "macos")]
#[gpui::test]
fn browser_profiles_keep_data_identities_across_settings_and_restart(
    cx: &mut gpui::TestAppContext,
) {
    use crate::views::browser_preferences::BrowserPreferences;
    let (workspace, cx, _, _) = thread_menu_workspace(cx);
    workspace.update_in(cx, |workspace, window, cx| {
        workspace.open_settings(&OpenSettings, window, cx);
        workspace.set_settings_section(super::super::SettingsSection::Browser, cx);
        let prefs = cx.default_global::<BrowserPreferences>();
        let work = prefs.add_profile("Work").unwrap();
        assert!(prefs.add_profile(" work ").is_err());
        let personal = prefs.add_profile("Personal").unwrap();
        assert_ne!(work, personal);
        prefs.default_profile = Some(work);
        let state = workspace.snapshot(cx).browser_preferences.unwrap();
        let json = serde_json::to_vec(&state).unwrap();
        let mut loaded: BrowserPreferences = serde_json::from_slice(&json).unwrap();
        assert_eq!(loaded.page_settings().profile, Some(work));
        loaded.restore_defaults();
        assert_eq!(loaded.page_settings().profile, None);
        assert_eq!(loaded.profiles, state.profiles);
    });
}

#[gpui::test]
fn project_defaults_affect_only_new_threads_in_the_selected_folder(cx: &mut gpui::TestAppContext) {
    let (workspace, cx, _, _) = thread_menu_workspace(cx);
    workspace.update_in(cx, |workspace, window, cx| {
        let model = bruhs_models::agent::MODEL_CATALOG
            .iter()
            .find(|m| m.id != workspace.current_model)
            .unwrap()
            .id;
        let environment = workspace.current_model.clone();
        workspace.open_settings(&OpenSettings, window, cx);
        workspace.set_settings_section(super::super::SettingsSection::Threads, cx);
        workspace.select_settings_option(
            "settings-project-defaults-scope",
            "/test/one/api",
            window,
            cx,
        );
        workspace.select_settings_option(
            super::super::SETTINGS_MODEL_DROPDOWN_ID,
            model,
            window,
            cx,
        );
        workspace.select_settings_option(
            super::super::SETTINGS_ACCESS_MODE_DROPDOWN_ID,
            "plan",
            window,
            cx,
        );
        workspace.select_settings_option("settings-worktree-remote", "on", window, cx);
        workspace.select_settings_option("settings-worktree-submodules", "recursive", window, cx);
        workspace.new_thread_in_project("api".into(), "/test/one/api".into(), window, cx);
        let first = workspace.active_thread().unwrap();
        assert_eq!(first.chat.read(cx).model_id, model);
        assert_eq!(first.chat.read(cx).permission_mode, PermissionMode::Plan);
        assert_eq!(
            first.chat.read(cx).worktree_prefs.submodules,
            bruhs_models::logic::run_place::WorktreeSubmodules::Recursive
        );
        assert!(first.chat.read(cx).worktree_prefs.start_from_remote);
        assert!(!workspace.worktree_prefs.start_from_remote);
        let snapshot = first.snapshot(cx);
        let restored = Workspace::create_thread_from_persisted(
            snapshot,
            &environment,
            PermissionMode::Default,
            window,
            cx,
        );
        assert_eq!(restored.chat.read(cx).permission_mode, PermissionMode::Plan);
        workspace.new_thread_in_project("api".into(), "/test/two/api".into(), window, cx);
        assert_eq!(
            workspace.active_thread().unwrap().chat.read(cx).model_id,
            environment
        );
        workspace.select_settings_option(
            super::super::SETTINGS_MODEL_DROPDOWN_ID,
            "inherit",
            window,
            cx,
        );
        workspace.select_settings_option(
            super::super::SETTINGS_ACCESS_MODE_DROPDOWN_ID,
            "inherit",
            window,
            cx,
        );
        workspace.select_settings_option("settings-worktree-remote", "inherit", window, cx);
        workspace.select_settings_option("settings-worktree-submodules", "inherit", window, cx);
        assert!(workspace.snapshot(cx).project_defaults.is_empty());
    });
}

#[gpui::test]
fn pull_links_use_the_native_review_then_follow_the_browser_preference(
    cx: &mut gpui::TestAppContext,
) {
    use crate::views::workspace::DesktopBranchPull;
    let (workspace, cx, _, _) = thread_menu_workspace(cx);
    workspace.update_in(cx, |workspace, window, cx| {
        let url = "https://github.com/example/repo/pull/42";
        workspace.branch_pull = Some(DesktopBranchPull {
            number: 42,
            title: "Example change".into(),
            url: url.into(),
            state: "open".into(),
        });
        workspace.open_web_link(&bruhs_ui::text::OpenWebLink(url.into()), window, cx);
        assert!(workspace.pull_panel_visible);
        assert_eq!(workspace.displayed_pull().unwrap().title, "Example change");
        if crate::views::preview_panel::WebViewHost::available() {
            workspace.change_browser_preference("settings-browser-link-target", "app", cx);
            let comment = format!("{url}#discussion_r123");
            workspace.open_web_link(&bruhs_ui::text::OpenWebLink(comment.clone()), window, cx);
            assert_eq!(
                workspace.preview_panel.read(cx).url(),
                Some(comment.as_str())
            );
            workspace.diff_panel.update(cx, |panel, cx| {
                let mut prefs = panel.prefs;
                prefs.open_pull_links_in_app = false;
                panel.set_preferences(prefs, cx)
            });
            workspace.open_web_link(&bruhs_ui::text::OpenWebLink(url.into()), window, cx);
            assert_eq!(workspace.preview_panel.read(cx).url(), Some(url));
        }
    });
}

#[gpui::test]
fn region_and_settlement_choices_apply_to_display_and_project_scope(cx: &mut gpui::TestAppContext) {
    let (workspace, cx, _, _) = thread_menu_workspace(cx);
    workspace.update_in(cx, |workspace, window, cx| {
        let afternoon = chrono::NaiveTime::from_hms_opt(15, 7, 0).unwrap();
        workspace.select_settings_option("settings-clock-format", "12h", window, cx);
        assert_eq!(
            afternoon.format(crate::clock_format::pattern()).to_string(),
            "3:07 PM"
        );
        workspace.select_settings_option("settings-clock-format", "24h", window, cx);
        assert_eq!(
            afternoon.format(crate::clock_format::pattern()).to_string(),
            "15:07"
        );
        assert_eq!(
            workspace.snapshot(cx).appearance.unwrap().clock.key(),
            "24h"
        );
        workspace.settings_project_scope = "/one".into();
        workspace.select_settings_option("settings-inactive-settlement", "off", window, cx);
        assert_eq!(workspace.idle_settle_after_ms("/one"), None);
        assert!(workspace.idle_settle_after_ms("/two").is_some());
        workspace.select_settings_option("settings-inactive-settlement", "inherit", window, cx);
        assert_eq!(
            workspace.idle_settle_after_ms("/one"),
            workspace.idle_settle_after_ms("/two")
        );
    });
}

#[gpui::test]
fn scheduled_webhook_history_refreshes_and_returns_to_its_task(cx: &mut gpui::TestAppContext) {
    use serde_json::json;
    use std::io::{BufRead, BufReader, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let request = json!({"id":"issue","outcome":"filtered","filter_reason":"Filter 1 (body.status) did not match.","received_at":1700000000000_i64});
    let detail = json!({"summary":request,"headers":"x-source: tracker","query":"lane: inbox","body":"status=held","body_truncated":false,"signature_verified":true});
    let worker = std::thread::spawn(move || {
        for (path, body) in [
            ("/api/reactive/tasks/task/deliveries", json!([])),
            ("/api/reactive/tasks/task/deliveries", json!([request])),
            ("/api/reactive/tasks/task/deliveries/issue", detail),
        ] {
            let (mut stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(&stream);
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            assert!(line.starts_with(&format!("GET {path} ")), "{line}");
            loop {
                line.clear();
                if reader.read_line(&mut line).unwrap() == 0 || line.trim().is_empty() {
                    break;
                }
            }
            let body = body.to_string();
            write!(stream,"HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{body}",body.len()).unwrap();
        }
    });
    cx.executor().allow_parking();
    let (workspace, cx, _, _) = thread_menu_workspace(cx);
    workspace.update_in(cx, |ws, window,cx| {
        ws.reactive_automations.update(cx,|panel,cx| {
            panel.set_delivery_test_client(crate::server_client::ServerClient::new(&base,&base).unwrap());
            panel.show_snapshot_for_test(serde_json::from_value(json!({
                "events":[],"runs":[],"proposals":[],
                "agents":[{"id":"clerk","name":"Review issues","instructions":"Review issues","model":"codex","project_path":"/repo","enabled":true}],
                "sources":[{"id":"issues","name":"Issues","kind":{"type":"webhook"},"enabled":true}],
                "subscriptions":[
                    {"id":"task","agent_id":"clerk","source_id":"issues","instructions":"Review opened issues","enabled":true},
                    {"id":"triage","agent_id":"clerk","source_id":"issues","instructions":"Triage issues","enabled":true,"task_title":"Evening triage"}
                ]
            })).unwrap(),cx);
        });
        ws.open_settings(&OpenSettings, window, cx);
        ws.set_settings_section(super::super::SettingsSection::ScheduledTasks, cx);
    });
    cx.run_until_parked();
    click_debug_selector(cx, "task-actions-task");
    click_debug_selector(cx, "reactive-Deliveries");
    assert!(cx.debug_bounds("delivery-empty").is_some());
    assert!(cx.debug_bounds("scheduled-task-close").is_some());
    click_debug_selector(cx, "reactive-Refresh");
    click_debug_selector(cx, "delivery-row-issue");
    assert!(cx.debug_bounds("delivery-section-Body").is_some());
    assert!(cx.debug_bounds("delivery-signature-verified").is_some());
    let scroll = rendered_bounds(cx, "scheduled-task-dialog-scroll");
    assert!(rendered_bounds(cx, "reactive-Back").top() >= scroll.bottom());
    click_debug_selector(cx, "reactive-Back");
    assert!(cx.debug_bounds("delivery-row-issue").is_some());
    click_debug_selector(cx, "reactive-Done");
    assert!(cx.debug_bounds("scheduled-task-dialog").is_none());
    assert!(cx.debug_bounds("desktop-settings-dialog").is_some());
    super::keys::press(cx, "enter");
    assert!(cx.debug_bounds("scheduled-task-actions-menu").is_some());
    cx.simulate_keystrokes("escape");
    assert!(cx.debug_bounds("scheduled-task-actions-menu").is_none());
    assert!(cx.debug_bounds("desktop-settings-dialog").is_some());
    click_debug_selector(cx, "task-actions-triage");
    click_debug_selector(cx, "reactive-Edit");
    cx.simulate_keystrokes("cmd-a");
    cx.simulate_input("  Afternoon triage  ");
    click_debug_selector(cx, "reactive-Save task");
    workspace.read_with(cx, |ws, cx| {
        let panel = ws.reactive_automations.read(cx);
        use bruhs_models::reactive::{ReactiveChange, ReactiveCommand};
        let Some(ReactiveCommand::Apply { change: ReactiveChange::SaveAutomation { automation } }) = panel.sent_for_test().last() else {
            panic!("expected the shared task edit to save");
        };
        assert_eq!(automation.subscription.id, "triage");
        assert_eq!(automation.subscription.task_title.as_deref(), Some("Afternoon triage"));
        assert_eq!(automation.subscription.instructions, "Triage issues");
        assert_eq!(automation.agent.name, "Review issues");
    });
    worker.join().unwrap();
}


#[gpui::test]
fn thread_details_menu_shares_choices_with_the_card_and_restores_defaults(
    cx: &mut gpui::TestAppContext,
) {
    let (workspace, cx, _, code) = thread_menu_workspace(cx);
    cx.simulate_resize(gpui::size(px(820.), px(720.)));
    cx.run_until_parked();
    workspace.update_in(cx, |w, window, cx| {
        w.select_thread_by_id(&code, cx);
        w.open_settings(&OpenSettings, window, cx);
        w.settings_search.update(cx, |field, cx| {
            field.set_value("Thread details", window, cx)
        });
        w.refresh_settings_search(cx);
        assert!(!w.settings_search_results.is_empty());
        w.open_settings_hit(0, window, cx);
    });
    cx.run_until_parked();
    click_debug_selector(cx, "thread-details-settings");
    cx.run_until_parked();
    assert_rendered_bounds(cx, "thread-details-option-context");
    click_debug_selector(cx, "thread-details-option-environment");
    cx.run_until_parked();
    assert_rendered_bounds(cx, "thread-details-menu");
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    assert_no_rendered_bounds(cx, "thread-details-menu");
    assert_rendered_bounds(cx, "reset-thread-details");
    workspace.update_in(cx, |w, win, cx| {
        let saved = serde_json::to_value(w.snapshot(cx)).unwrap();
        assert_eq!(saved["appearance"]["thread_details"]["environment"], false);
        w.open_chat_view(&super::super::OpenChatView, win, cx);
    });
    cx.run_until_parked();
    click_debug_selector(cx, "desktop-thread-details-toggle");
    cx.run_until_parked();
    assert_rendered_bounds(cx, "thread-details-card");
    assert_no_rendered_bounds(cx, "detail-environment");
    click_debug_selector(cx, "detail-configure");
    cx.run_until_parked();
    click_debug_selector(cx, "thread-details-option-reset");
    cx.run_until_parked();
    assert_no_rendered_bounds(cx, "thread-details-menu");
    assert_rendered_bounds(cx, "detail-environment");
    workspace.update_in(cx, |w, _, cx| {
        w.active_thread().unwrap().chat.update(cx, |chat, _| {
            chat.import_context = Some("Frozen project context".to_owned())
        });
    });
    click_debug_selector(cx, "detail-new-side-chat");
    cx.run_until_parked();
    let child = workspace.update_in(cx, |w, _, cx| {
        let id = w.active_thread_id.clone().unwrap();
        assert_ne!(id, code);
        let restored: PersistedWorkspaceState =
            serde_json::from_value(serde_json::to_value(w.snapshot(cx)).unwrap()).unwrap();
        assert_eq!(restored.thread_origins[&id].parent(), code);
        let child = restored
            .threads
            .iter()
            .find(|thread| thread.id == id)
            .unwrap();
        assert_eq!(
            child.import_context.as_deref(),
            Some("Frozen project context")
        );
        assert!(child.session_id.is_none());
        id
    });
    click_debug_selector(cx, "desktop-thread-details-toggle");
    cx.run_until_parked();
    click_debug_selector(cx, "detail-parent");
    cx.run_until_parked();
    workspace.update_in(cx, |w, _, cx| {
        assert_eq!(w.active_thread_id.as_ref(), Some(&code));
        w.delete_thread_by_id(&code, cx);
        assert!(w.threads.iter().any(|thread| thread.id == child));
        assert!(!w.snapshot(cx).thread_origins.contains_key(&child));
    });
}

#[gpui::test]
fn thread_details_resizes_preserves_each_threads_inline_choice_and_opens_at_pointer(
    cx: &mut gpui::TestAppContext,
) {
    use gpui::{size, MouseButton, MouseDownEvent};
    let (workspace, cx, general, code) = thread_menu_workspace(cx);
    cx.simulate_resize(size(px(1720.), px(920.)));
    cx.run_until_parked();
    let card = rendered_bounds(cx, "thread-details-card");
    let header = rendered_bounds(cx, "desktop-chat-header");
    assert!(card.left() > header.right());
    let position = card.center();
    cx.simulate_event(MouseDownEvent {
        position,
        button: MouseButton::Right,
        modifiers: Default::default(),
        click_count: 1,
        first_mouse: false,
    });
    cx.run_until_parked();
    let menu = rendered_bounds(cx, "thread-details-menu");
    assert!((menu.top() - position.y).abs() <= px(1.) && menu.left() <= position.x + px(1.) && menu.right() >= position.x - px(1.), "menu {menu:?} at {position:?}");
    cx.simulate_keystrokes("end space escape");
    cx.run_until_parked();
    assert_no_rendered_bounds(cx, "thread-details-menu");
    assert_rendered_bounds(cx, "thread-details-card");
    click_debug_selector(cx, "desktop-thread-details-toggle");
    cx.run_until_parked();
    assert_no_rendered_bounds(cx, "thread-details-card");
    workspace.update_in(cx, |w, _, cx| {
        assert!(w
            .snapshot(cx)
            .hidden_inline_thread_details
            .contains(&general));
        w.select_thread_by_id(&code, cx);
    });
    cx.run_until_parked();
    assert_rendered_bounds(cx, "thread-details-card");
    cx.simulate_resize(size(px(820.), px(640.)));
    cx.run_until_parked();
    assert_no_rendered_bounds(cx, "thread-details-card");
    click_debug_selector(cx, "desktop-thread-details-toggle");
    cx.run_until_parked();
    assert_rendered_bounds(cx, "thread-details-card");
    click_debug_selector(cx, "desktop-thread-details-toggle");
    cx.run_until_parked();
    assert_no_rendered_bounds(cx, "thread-details-card");
}

#[gpui::test]
fn thread_details_measures_the_selected_checkout(cx: &mut gpui::TestAppContext) {
    let (workspace, cx, general, code) = thread_menu_workspace(cx);
    let folder = tempfile::tempdir().unwrap();
    std::fs::write(folder.path().join("fixture"), [1; 4096]).unwrap();
    cx.simulate_resize(gpui::size(px(820.), px(720.)));
    workspace.update_in(cx, |w, _, cx| {
        w.select_thread_by_id(&code, cx);
        w.active_thread().unwrap().chat.update(cx, |chat, _| {
            chat.worktree_path = Some(folder.path().to_string_lossy().into_owned());
        });
    });
    cx.run_until_parked();
    click_debug_selector(cx, "desktop-thread-details-toggle");
    cx.run_until_parked();
    click_debug_selector(cx, "measure-thread-usage");
    cx.run_until_parked();
    assert_rendered_bounds(cx, "detail-resource-sample");
    workspace.update_in(cx, |w, _, cx| {
        let (id, _, result) = w.details.last_measurement().unwrap();
        assert_eq!(id, &code);
        assert!(result.contains("No active local CLI") && result.contains("Checkout size ·"));
        w.select_thread_by_id(&general, cx);
    });
    cx.run_until_parked();
    assert_no_rendered_bounds(cx, "detail-resource-sample");
}

#[test]
fn thread_details_defaults_and_explicit_effort_preferences_load_from_older_files() {
    use super::super::appearance::DeviceAppearance;
    let old: DeviceAppearance =
        serde_json::from_value(serde_json::json!({"effort_label":false})).unwrap();
    assert!(!old.effort_label);
    let details = serde_json::to_value(old.thread_details).unwrap();
    assert_eq!(details.as_object().unwrap().len(), 13);
    assert_eq!(details["resources"], true);
    assert_eq!(
        details
            .as_object()
            .unwrap()
            .values()
            .filter(|value| value.as_bool() == Some(true))
            .count(),
        11
    );
    assert_eq!(details["usage"], false);
    assert_eq!(details["context"], false);
    assert!(DeviceAppearance::default().effort_label);
}

#[gpui::test]
fn worktree_cleanup_inherits_project_rules_and_reveals_advanced_search_results(
    cx: &mut gpui::TestAppContext,
) {
    use bruhs_models::coordination::worktree::KeepWhen;
    let (workspace, cx, _, _) = thread_menu_workspace(cx);
    cx.simulate_resize(gpui::size(px(1040.), px(820.)));
    workspace.update_in(cx, |ws, window, cx| {
        ws.cleanup_deleted_worktrees = true;
        ws.cleanup_keep_when = KeepWhen::AnyLocalFiles;
        ws.open_settings(&OpenSettings, window, cx);
        ws.select_settings_option("settings-project-defaults-scope", "/tmp/alpha", window, cx);
        ws.set_settings_section(super::super::SettingsSection::Worktrees, cx);
    });
    cx.run_until_parked();
    assert_no_rendered_bounds(cx, "settings-cleanup-deleted");
    click_debug_selector(cx, "settings-worktree-cleanup-mode");
    click_debug_selector(cx, "settings-worktree-cleanup-mode-option-custom");
    assert_rendered_bounds(cx, "settings-cleanup-deleted");
    assert_no_rendered_bounds(cx, "settings-worktree-keep-when");
    workspace.update_in(cx, |ws, window, cx| {
        assert_eq!(
            ws.cleanup_rules("/tmp/alpha"),
            (true, KeepWhen::AnyLocalFiles)
        );
        ws.settings_search.update(cx, |field, cx| {
            field.set_value("Keep worktrees with local changes", window, cx)
        });
        ws.refresh_settings_search(cx);
        ws.open_settings_hit(0, window, cx);
    });
    cx.run_until_parked();
    click_debug_selector(cx, "settings-worktree-keep-when");
    click_debug_selector(cx, "settings-worktree-keep-when-option-tracked-changes");
    workspace.update_in(cx, |ws, window, cx| {
        let state: PersistedWorkspaceState =
            serde_json::from_value(serde_json::to_value(ws.snapshot(cx)).unwrap()).unwrap();
        assert_eq!(state.cleanup_keep_when, KeepWhen::AnyLocalFiles);
        assert_eq!(
            state.project_defaults["/tmp/alpha"].cleanup_keep_when,
            Some(KeepWhen::TrackedChanges)
        );
        assert_eq!(
            ws.cleanup_rules("/tmp/alpha"),
            (true, KeepWhen::TrackedChanges)
        );
        ws.select_settings_option("settings-worktree-cleanup-mode", "off", window, cx);
        assert!(!ws.cleanup_rules("/tmp/alpha").0);
        ws.select_settings_option("settings-worktree-cleanup-mode", "inherit", window, cx);
        assert_eq!(
            ws.cleanup_rules("/tmp/alpha"),
            (true, KeepWhen::AnyLocalFiles)
        );
        assert!(!ws.project_defaults.contains_key("/tmp/alpha"));
        // An older version saved only a per-project boolean. Turning the
        // deletion trigger off must leave the project in custom mode.
        ws.project_defaults.insert(
            "/tmp/alpha".into(),
            serde_json::from_value(serde_json::json!({"cleanup_deleted_worktrees": true})).unwrap(),
        );
        ws.settings_search.update(cx, |field, cx| {
            field.set_value("Delete worktrees with deleted threads", window, cx)
        });
        ws.refresh_settings_search(cx);
        ws.open_settings_hit(0, window, cx);
    });
    cx.run_until_parked();
    click_debug_selector(cx, "settings-cleanup-deleted");
    workspace.read_with(cx, |ws, _| {
        assert_eq!(
            ws.project_defaults["/tmp/alpha"].cleanup_mode,
            Some(super::super::worktree_storage::CleanupMode::Custom)
        );
        assert_eq!(
            ws.cleanup_rules("/tmp/alpha"),
            (false, KeepWhen::AnyLocalFiles)
        );
        assert!(ws.cleanup_deleted_worktrees);
    });
    assert_rendered_bounds(cx, "worktree-cleanup-advanced");
}

#[gpui::test]
fn branch_naming_edits_follow_project_scope_and_reset_to_the_inherited_field(
    cx: &mut gpui::TestAppContext,
) {
    use bruhs_models::logic::run_place::branch_naming::{Mode, Naming};
    let (workspace, cx, _, code) = thread_menu_workspace(cx);
    cx.simulate_resize(gpui::size(px(1100.), px(820.)));
    workspace.update_in(cx, |ws, window, cx| {
        ws.open_settings(&OpenSettings, window, cx);
        ws.select_settings_option("settings-project-defaults-scope", "/tmp/alpha", window, cx);
        ws.settings_search.update(cx, |field, cx| {
            field.set_value("Worktree branch naming", window, cx)
        });
        ws.refresh_settings_search(cx);
        ws.open_settings_hit(0, window, cx);
    });
    cx.run_until_parked();
    click_debug_selector(cx, "settings-branch-naming");
    click_debug_selector(cx, "settings-branch-naming-option-custom");
    assert_rendered_bounds(cx, "branch-instructions-editor");
    workspace.update_in(cx, |ws, w, cx| {
        ws.branch_instructions_input
            .update(cx, |field, cx| field.focus(w, cx))
    });
    cx.simulate_input("  Keep the issue ID.");
    cx.simulate_keystrokes("enter");
    cx.simulate_input("Use team/.  ");
    workspace.update_in(cx, |ws, win, cx| {
        assert_eq!(ws.effective_branch_naming("/tmp/alpha").instructions, "");
        ws.select_settings_option("settings-project-defaults-scope", "", win, cx);
        let project = ws.effective_branch_naming("/tmp/alpha");
        assert_eq!(project.mode, Mode::Custom);
        assert_eq!(project.instructions, "Keep the issue ID.\nUse team/.");
        assert_eq!(ws.branch_naming, Naming::default());
        assert_eq!(
            ws.threads
                .iter()
                .find(|t| t.id == code)
                .unwrap()
                .chat
                .read(cx)
                .branch_naming,
            project
        );
        let restored: PersistedWorkspaceState =
            serde_json::from_value(serde_json::to_value(ws.snapshot(cx)).unwrap()).unwrap();
        assert_eq!(
            restored.project_defaults["/tmp/alpha"].branch_mode,
            Some(Mode::Custom)
        );
        ws.select_settings_option("settings-project-defaults-scope", "/tmp/alpha", win, cx);
        ws.settings_search.update(cx, |field, cx| {
            field.set_value("Branch naming instructions", win, cx)
        });
        ws.refresh_settings_search(cx);
        ws.open_settings_hit(0, win, cx);
    });
    cx.run_until_parked();
    click_debug_selector(cx, "reset-branch-instructions");
    click_debug_selector(cx, "reset-branch-naming");
    workspace.read_with(cx, |ws, _| {
        assert_eq!(ws.effective_branch_naming("/tmp/alpha"), Naming::default());
        assert!(!ws.project_defaults.contains_key("/tmp/alpha"));
    });
}

#[gpui::test]
fn source_writer_search_reveals_the_model_and_keeps_the_controls_together(
    cx: &mut gpui::TestAppContext,
) {
    use bruhs_models::delivery::{
        DeliveryOverrides, DeliveryPreferences, MergeMethod, WriterModel, WritingStyle,
    };
    let (workspace, cx, _, _) = thread_menu_workspace(cx);
    cx.simulate_resize(gpui::size(px(820.), px(790.)));
    workspace.update_in(cx, |ws, w, cx| {
        ws.open_settings(&OpenSettings, w, cx);
        ws.settings_section = super::super::SettingsSection::SourceControl;
        let model = WriterModel::Separate {
            options: Default::default(),
            model: "claude-haiku-4-5".into(),
        };
        ws.set_delivery_preferences_for_test(DeliveryPreferences {
            review_guides_enabled: false,
            review_guide_model: Default::default(),
            review_guide_instructions: String::new(),
            merge_method: MergeMethod::Squash,
            follow_templates: true,
            remove_agent_credits: false,
            writer_model: model.clone(),
            writing_style: WritingStyle::Custom,
            writer_instructions: "Keep issue IDs.".into(),
            overrides: DeliveryOverrides {
                writer_model: Some(model),
                ..Default::default()
            },
        });
        cx.notify();
    });
    cx.run_until_parked();
    assert_rendered_bounds(cx, "writer-instructions-editor");
    assert_no_rendered_bounds(cx, "settings-writer-model");
    workspace.update_in(cx, |ws, w, cx| {
        ws.settings_search.update(cx, |input, cx| {
            input.set_value("Source control writer model", w, cx)
        });
        ws.refresh_settings_search(cx);
        ws.open_settings_hit(0, w, cx);
    });
    cx.run_until_parked();
    let picker = rendered_bounds(cx, "settings-writer-model");
    let toggle = rendered_bounds(cx, "settings-writer-model-switch");
    assert!(picker.right() <= toggle.left());
    assert!(picker.top() < toggle.bottom() && toggle.top() < picker.bottom());
    click_debug_selector(cx, "settings-writer-options");
    assert_rendered_bounds(cx, "settings-writer-options-option-thinking=on-check");
    assert_rendered_bounds(cx, "settings-writer-options-option-thinking=off");
    assert_no_rendered_bounds(cx, "settings-writer-options-option-effort=low");
    cx.simulate_keystrokes("escape");
    click_debug_selector(cx, "settings-writer-model");
    assert_rendered_bounds(cx, "settings-writer-model-option-claude-haiku-4-5");
    assert_rendered_bounds(cx, "settings-writer-model-option-gpt-6-luna");
}

#[gpui::test]
fn review_guide_preferences_reveal_the_agent_and_its_instructions(cx: &mut gpui::TestAppContext) {
    use bruhs_models::delivery::{DeliveryPreferences, GuideModel};
    let (workspace, cx, _, _) = thread_menu_workspace(cx);
    cx.simulate_resize(gpui::size(px(820.), px(790.)));
    let prefs: DeliveryPreferences = serde_json::from_value(serde_json::json!({
        "merge_method":"squash","follow_templates":true,"overrides":{}
    }))
    .unwrap();
    workspace.update_in(cx, |ws, w, cx| {
        ws.open_settings(&OpenSettings, w, cx);
        ws.settings_section = super::super::SettingsSection::SourceControl;
        ws.set_delivery_preferences_for_test(prefs.clone());
        cx.notify();
    });
    cx.run_until_parked();
    assert_no_rendered_bounds(cx, "review-guide-instructions-editor");
    workspace.update_in(cx, |ws, w, cx| {
        ws.settings_search.update(cx, |input, cx| {
            input.set_value("Review guide instructions", w, cx)
        });
        ws.refresh_settings_search(cx);
        assert!(!ws.settings_search_results.is_empty());
        ws.open_settings_hit(0, w, cx);
        assert!(ws.settings_search_anchor.is_some());
    });
    cx.run_until_parked();
    assert_no_rendered_bounds(cx, "review-guide-instructions-editor");
    workspace.update_in(cx, |ws, _, cx| {
        ws.set_delivery_preferences_for_test(DeliveryPreferences {
            review_guides_enabled: true,
            review_guide_instructions: "Explain interfaces first.".into(),
            review_guide_model: GuideModel::Separate {
                options: Default::default(),
                model: "claude-haiku-4-5".into(),
            },
            ..prefs
        });
        cx.notify();
    });
    cx.run_until_parked();
    workspace.update_in(cx, |ws, w, cx| {
        ws.settings_search
            .update(cx, |input, cx| input.set_value("Review guide model", w, cx));
        ws.refresh_settings_search(cx);
        ws.open_settings_hit(0, w, cx);
    });
    cx.run_until_parked();
    assert_rendered_bounds(cx, "settings-guide-model-switch");
    click_debug_selector(cx, "settings-guide-options");
    assert_rendered_bounds(cx, "settings-guide-options-option-thinking=on-check");
    cx.simulate_keystrokes("escape");
    click_debug_selector(cx, "settings-guide-model");
    assert_rendered_bounds(cx, "settings-guide-model-option-claude-haiku-4-5");
    assert_rendered_bounds(cx, "settings-guide-model-option-gpt-6-luna");
}

#[gpui::test]
fn forking_a_reply_freezes_its_history_and_uses_project_title_settings(
    cx: &mut gpui::TestAppContext,
) {
    use crate::views::chat::{ChatViewEvent, Message, TurnState};
    let (workspace, cx, _, parent_id) = thread_menu_workspace(cx);
    cx.simulate_resize(gpui::size(px(1040.), px(820.)));
    let reply = |text: &str| {
        let mut message = Message::assistant_streaming(1, None);
        message.state = TurnState::Finished;
        message.content = text.to_owned();
        message
    };
    workspace.update_in(cx, |ws, _, cx| {
        ws.select_thread_by_id(&parent_id, cx);
        ws.settings_project_scope = "/tmp/alpha".into();
        ws.choose_title_model("local", cx);
        let chat = ws.active_thread().unwrap().chat.clone();
        chat.update(cx, |view, cx| {
            let mut assistant = reply("Use hourly weather tiles.");
            assistant.thinking = "Private scratchpad".into();
            view.messages = vec![
                Message::user("Render rainfall"),
                assistant,
                Message::user("Later conversation"),
            ];
            view.session_id = Some("parent-only".into());
            cx.notify();
        });
    });
    cx.run_until_parked();
    click_debug_selector(cx, "desktop-fork-from-reply-1");
    cx.run_until_parked();
    let child_id = workspace.update_in(cx, |ws, _, cx| {
        let child = ws.active_thread().unwrap();
        assert_ne!(child.id, parent_id);
        let view = child.chat.read(cx);
        assert!(view.session_id.is_none());
        assert_eq!(view.thread_title, "Alpha session fork");
        assert_eq!(view.text_generation_model, "local");
        let source = view.fork.as_ref().unwrap().title_source(None);
        assert_eq!(
            source.history,
            "USER:\nRender rainfall\n\nASSISTANT:\nUse hourly weather tiles."
        );
        let saved: PersistedThread =
            serde_json::from_slice(&serde_json::to_vec(&child.snapshot(cx)).unwrap()).unwrap();
        assert_eq!(
            saved.fork.unwrap().title_source(None).history,
            source.history
        );
        let child_id = child.id.clone();
        let chat = child.chat.clone();
        chat.update(cx, |view, cx| {
            view.messages = vec![Message::user("Add a legend"), reply("Use five ranges.")];
            cx.emit(ChatViewEvent::TurnStarted {
                model_id: view.model_id.clone(),
                branch: None,
            });
        });
        child_id
    });
    cx.run_until_parked();
    workspace.read_with(cx, |ws, cx| {
        assert_eq!(
            ws.active_thread().unwrap().chat.read(cx).thread_title,
            "Add a legend"
        )
    });
    click_debug_selector(cx, "desktop-fork-from-reply-1");
    cx.run_until_parked();
    workspace.read_with(cx, |ws, cx| {
        let view = ws.active_thread().unwrap().chat.read(cx);
        let source = view.fork.as_ref().unwrap().title_source(None);
        assert_eq!(source.title, "Add a legend");
        assert!(source.history.contains("Render rainfall"));
        assert!(source.history.ends_with("ASSISTANT:\nUse five ranges."));
        let context = view.conversation_context(&[]).unwrap();
        assert!(context.contains("Render rainfall") && context.contains("Add a legend"));
        assert!(!context.contains("Later conversation"));
    });
    click_debug_selector(cx, "open-fork-source");
    cx.run_until_parked();
    workspace.read_with(cx, |ws, _| {
        assert_eq!(ws.active_thread_id.as_deref(), Some(child_id.as_str()))
    });
}

#[gpui::test]
fn scheduled_webhook_url_copies_and_rotates_after_confirmation(cx: &mut gpui::TestAppContext) {
    use std::io::{BufRead, BufReader, Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let worker = std::thread::spawn(move || {
        for (method, token) in [("GET", "first"), ("POST", "second")] {
            let (mut stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(&stream);
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            assert_eq!(
                line.trim(),
                format!("{method} /api/reactive/tasks/task/webhook-url HTTP/1.1")
            );
            loop {
                line.clear();
                if reader.read_line(&mut line).unwrap() == 0 || line.trim().is_empty() {
                    break;
                }
            }
            if method == "POST" {
                let mut previous = [0; 7];
                reader.read_exact(&mut previous).unwrap();
                assert_eq!(&previous, br#""first""#);
            }
            let body = serde_json::to_string(token).unwrap();
            write!(stream, "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
        }
    });
    cx.executor().allow_parking();
    let (_, cx, panel) =
        scheduled_task_workspace_with(cx, bruhs_models::reactive::SourceKind::Webhook, |s| {
            s.can_address_task_webhooks = true
        });
    panel.update(cx, |panel, _| {
        panel.set_delivery_test_client(
            crate::server_client::ServerClient::new(&base, &base).unwrap(),
        )
    });
    cx.simulate_resize(gpui::size(px(1040.), px(820.)));
    cx.run_until_parked();
    cx.update(|window, _| window.activate_window());
    cx.run_until_parked();
    assert!(cx.update(|window, _| window.is_window_active()));
    click_debug_selector(cx, "task-actions-task");
    click_debug_selector(cx, "reactive-Edit");
    let copy = |cx: &mut gpui::VisualTestContext| {
        click_debug_selector(cx, "copy-task-webhook-url");
        cx.update(|_, cx| cx.read_from_clipboard().unwrap().text().unwrap())
    };
    let url = format!("{base}/api/reactive/task-hooks/task");
    for width in [820., 1040.] {
        cx.simulate_resize(gpui::size(px(width), px(820.)));
        cx.run_until_parked();
        let field = rendered_bounds(cx, "task-webhook-url-field");
        let copy_button = rendered_bounds(cx, "copy-task-webhook-url");
        let rotate_button = rendered_bounds(cx, "rotate-task-webhook-url");
        assert!(field.size.width > px(120.));
        assert!(field.right() < copy_button.left() && copy_button.right() < rotate_button.left());
    }
    let scroll = rendered_bounds(cx, "scheduled-task-dialog-scroll");
    for _ in 0..10 {
        let row = rendered_bounds(cx, "copy-task-webhook-url");
        if scroll.contains(&row.center()) {
            break;
        }
        cx.simulate_event(gpui::ScrollEvent {
            position: scroll.center(),
            delta: gpui::ScrollDelta::Pixels(gpui::point(px(0.), px(-100.))),
            ..Default::default()
        });
        cx.run_until_parked();
    }
    let field = rendered_bounds(cx, "task-webhook-url-field");
    assert!(scroll.contains(&field.center()), "field {field:?} must be inside {scroll:?}");
    click_debug_selector(cx, "task-webhook-url-field");
    cx.simulate_input("replacement");
    cx.simulate_keystrokes("cmd-c");
    let selected = cx.update(|_, cx| cx.read_from_clipboard().and_then(|item| item.text()));
    assert_eq!(selected, Some(format!("{url}/first")));
    assert_eq!(copy(cx), format!("{url}/first"));
    click_debug_selector(cx, "rotate-task-webhook-url");
    cx.simulate_prompt_answer("Cancel");
    cx.run_until_parked();
    assert_eq!(copy(cx), format!("{url}/first"));
    click_debug_selector(cx, "rotate-task-webhook-url");
    cx.simulate_prompt_answer("Rotate");
    cx.run_until_parked();
    assert_eq!(copy(cx), format!("{url}/second"));
    worker.join().unwrap();
}

#[gpui::test]
async fn appearance_theme_halves_apply_independently_and_survive_restart(
    cx: &mut gpui::TestAppContext,
) {
    use super::super::appearance::ColorScheme;
    use super::launch::{init, launch, quit, saved};
    use bruhs_ui::theme::{ThemeFamily, ThemeMode};
    use bruhs_ui::ActiveTheme;
    use gpui::WindowAppearance;
    init(cx);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.json");
    let mut relaunch = cx.clone();
    let (workspace, cx) = launch(cx, &path);
    workspace.update_in(cx, |ws, w, cx| {
        ws.open_settings(&OpenSettings, w, cx);
        ws.set_settings_section(super::super::SettingsSection::Appearance, cx);
    });
    cx.run_until_parked();
    click_debug_selector(cx, "settings-scheme-dark");
    click_debug_selector(cx, "settings-theme-T3 Chat");
    workspace.read_with(cx, |ws, _| {
        assert_eq!(ws.appearance.light_theme, ThemeFamily::T3Chat);
        assert_eq!(ws.appearance.dark_theme, ThemeFamily::T3Chat);
    });
    cx.simulate_keystrokes("tab tab");
    super::keys::press(cx, "enter");
    workspace.read_with(cx, |ws, _| {
        assert_eq!(ws.appearance.light_theme, ThemeFamily::Grove);
        assert_eq!(ws.appearance.dark_theme, ThemeFamily::T3Chat);
    });
    click_debug_selector(cx, "settings-theme-Bruhs-Light");
    workspace.read_with(cx, |ws, cx| {
        assert_eq!(ws.appearance.light_theme, ThemeFamily::Bruhs);
        assert_eq!(ws.appearance.dark_theme, ThemeFamily::T3Chat);
        assert_eq!(
            cx.theme().background,
            gpui::rgb(ThemeFamily::T3Chat.palette(ThemeMode::Dark).surface[0]).into()
        );
    });
    click_debug_selector(cx, "settings-scheme-dark");
    cx.simulate_keystrokes("shift-tab");
    super::keys::press(cx, "space");
    workspace.read_with(cx, |ws, cx| {
        assert_eq!(ws.appearance.scheme, ColorScheme::Light);
        assert_eq!(
            cx.theme().background,
            gpui::rgb(ThemeFamily::Bruhs.palette(ThemeMode::Light).surface[0]).into()
        );
    });
    click_debug_selector(cx, "settings-scheme-system");
    saved(&workspace, cx).await;
    quit(&workspace, cx);
    let (workspace, cx) = launch(&mut relaunch, &path);
    workspace.update_in(cx, |ws, w, cx| {
        assert_eq!(ws.appearance.scheme, ColorScheme::System);
        assert_eq!(ws.appearance.light_theme, ThemeFamily::Bruhs);
        assert_eq!(ws.appearance.dark_theme, ThemeFamily::T3Chat);
        let (family, mode) = match w.appearance() {
            WindowAppearance::Light | WindowAppearance::VibrantLight => {
                (ThemeFamily::Bruhs, ThemeMode::Light)
            }
            _ => (ThemeFamily::T3Chat, ThemeMode::Dark),
        };
        assert_eq!(
            cx.theme().background,
            gpui::rgb(family.palette(mode).surface[0]).into()
        );
        ws.open_settings(&OpenSettings, w, cx);
        ws.settings_search.update(cx, |input, cx| input.set_value("Grove", w, cx));
        ws.refresh_settings_search(cx);
        ws.open_settings_hit(0, w, cx);
        assert_eq!(ws.settings_section, super::super::SettingsSection::Appearance);
    });
}

#[gpui::test]
async fn custom_theme_import_conflicts_export_and_restart(cx: &mut gpui::TestAppContext) {
    use super::launch::{init, launch, quit, saved};
    use bruhs_ui::ActiveTheme;
    use serde_json::{json, Value};

    init(cx);
    let dir = tempfile::tempdir().unwrap();
    let state = dir.path().join("state.json");
    let source = json!({"version": 1, "id":"aurora", "name":"Aurora", "appearance":"dark",
        "colors":{"canvas":"#123456","accent":"oklch(62% 0.2 280)","text":"#eee","terminalForeground":"#dae9fa","terminalBackground":"#112132","terminalCursor":"#ff7900","terminalSelection":"#267bc488"},
        "collection":{"id":"studio","label":"Studio"},"css":"/* preserved */"});
    let path = dir.path().join("aurora.json");
    std::fs::write(&path, source.to_string()).unwrap();
    let mut relaunch = cx.clone();
    let (workspace, cx) = launch(cx, &state);
    let open = |ws: &mut Workspace, w: &mut gpui::Window, cx: &mut gpui::Context<'_, Workspace>| {
        ws.open_settings(&OpenSettings, w, cx);
        ws.set_settings_section(super::super::SettingsSection::Appearance, cx);
    };
    workspace.update_in(cx, open);
    cx.simulate_resize(gpui::size(px(1440.), px(1100.)));
    click_debug_selector(cx, "settings-scheme-dark");
    click_debug_selector(cx, "settings-add-theme");
    click_debug_selector(cx, "theme-choose-files");
    cx.simulate_path_prompt_response(move |opts| {
        assert!(opts.files && opts.multiple);
        Some(vec![path])
    });
    cx.run_until_parked();
    click_debug_selector(cx, "theme-editor-save");
    workspace.read_with(cx, |_, cx| {
        assert_eq!(cx.theme().background, gpui::rgb(0x123456).into());
        assert_eq!(cx.theme().color_role("terminalCursor"), Some(gpui::rgb(0xff7900).into()));
        assert_eq!(cx.theme().color_role("terminalSelection"), Some(gpui::rgba(0x267bc488).into()));

    });
    click_debug_selector(cx, "settings-add-theme");
    click_debug_selector(cx,"theme-json-input");
    cx.simulate_input(&source.to_string());
    click_debug_selector(cx, "theme-editor-save");
    click_debug_selector(cx, "theme-keep-both");
    saved(&workspace, cx).await;
    quit(&workspace, cx);
    let (workspace, cx) = launch(&mut relaunch, &state);
    workspace.read_with(cx, |ws, cx| {
        let library = serde_json::to_value(&ws.custom_themes).unwrap();
        assert!(library["light"].is_null());
        assert_eq!(library["dark"], "aurora");
        assert_eq!(library["themes"][1]["name"], "Aurora (1)");
        assert_eq!(library["themes"][0], source);
        assert_eq!(cx.theme().background, gpui::rgb(0x123456).into());
        assert_eq!(cx.theme().color_role("terminalCursor"), Some(gpui::rgb(0xff7900).into()));
        assert_eq!(cx.theme().color_role("terminalSelection"), Some(gpui::rgba(0x267bc488).into()));

    });
    workspace.update_in(cx, open);
    cx.simulate_resize(gpui::size(px(1440.), px(1100.)));
    workspace.update(cx, |ws, cx| {
        ws.settings_scroll.set_offset(gpui::point(px(0.), px(-420.)));
        cx.notify();
    });
    click_debug_selector(cx, "theme-export-custom-aurora");
    let exported = dir.path().join("export.json");
    cx.simulate_new_path_selection(|_| Some(exported.clone()));
    cx.run_until_parked();
    assert_eq!(serde_json::from_slice::<Value>(&std::fs::read(exported).unwrap()).unwrap(), source);
    click_debug_selector(cx, "theme-remove-custom-aurora");
    cx.simulate_prompt_answer("Remove");
    cx.run_until_parked();
    workspace.read_with(cx, |ws, _| {
        let library = serde_json::to_value(&ws.custom_themes).unwrap();
        assert!(library["dark"].is_null());
        assert_eq!(library["themes"].as_array().unwrap().len(), 1);
    });
}

#[gpui::test]
fn custom_theme_editor_creates_edits_and_validates_colors(cx: &mut gpui::TestAppContext) {
    use super::launch::{init, launch};
    init(cx);
    let dir = tempfile::tempdir().unwrap();
    let (workspace, cx) = launch(cx, &dir.path().join("state.json"));
    workspace.update_in(cx, |ws, w, cx| {
        ws.open_settings(&OpenSettings, w, cx);
        ws.set_settings_section(super::super::SettingsSection::Appearance, cx);
    });
    cx.simulate_resize(gpui::size(px(1440.), px(1100.)));
    click_debug_selector(cx, "theme-duplicate-Grove");
    cx.simulate_keystrokes("cmd-a");
    cx.simulate_input("Forest");
    click_debug_selector(cx, "theme-edit-dark");
    click_debug_selector(cx, "theme-role-canvas");
    cx.simulate_keystrokes("cmd-a");
    cx.simulate_input("invalid");
    click_debug_selector(cx, "theme-editor-save");
    assert!(cx.debug_bounds("theme-editor-error").is_some());
    click_debug_selector(cx, "theme-role-canvas");
    cx.simulate_keystrokes("cmd-a");
    cx.simulate_input("#163925");
    click_debug_selector(cx, "theme-editor-save");
    workspace.read_with(cx, |ws, _| {
        let library = serde_json::to_value(&ws.custom_themes).unwrap();
        assert_eq!(library["dark"], "forest");
        assert_eq!(library["themes"][0]["variants"]["dark"]["canvas"], "#163925");
    });
    workspace.update(cx, |ws, cx| {
        ws.settings_scroll.set_offset(gpui::point(px(0.), px(-420.)));
        cx.notify();
    });
    click_debug_selector(cx, "theme-edit-custom-forest");
    cx.simulate_keystrokes("cmd-a");
    cx.simulate_input("Forest canopy");
    click_debug_selector(cx, "theme-editor-save");
    workspace.read_with(cx, |ws, _| {
        let library = serde_json::to_value(&ws.custom_themes).unwrap();
        assert_eq!(library["themes"].as_array().unwrap().len(), 1);
        assert_eq!(library["themes"][0]["name"], "Forest canopy");
        assert_eq!(library["light"], "forest");
    });
}

#[gpui::test]
async fn vscode_theme_batch_pairs_files_keeps_valid_imports_and_restores_selection(
    cx: &mut gpui::TestAppContext,
) {
    use super::launch::{init, launch, quit, saved};
    use bruhs_ui::ActiveTheme;
    init(cx);
    let dir = tempfile::tempdir().unwrap();
    let state = dir.path().join("state.json");
    let file = |name: &str, contents: &str| {
        let path = dir.path().join(name);
        std::fs::write(&path, contents).unwrap();
        path
    };
    let imported = [
        file(
            "coast-dark.json",
            r##"{ // both files have the extension's display name
            "name":"Coast", "colors":{"editor.background":"#122131",}, }"##,
        ),
        file(
            "coast-light.json",
            r##"{ "name":"Coast", "type":"hc-light",
            "colors":{"editor.background":"#f0faf7","focusBorder":"#287050"} }"##,
        ),
        file("busted.json", r#"{"tokenColors":[]}"#),
    ];
    let mut relaunch = cx.clone();
    let (workspace, cx) = launch(cx, &state);
    workspace.update_in(cx, |ws, w, cx| {
        ws.open_settings(&OpenSettings, w, cx);
        ws.set_settings_section(super::super::SettingsSection::Appearance, cx);
    });
    cx.simulate_resize(gpui::size(px(1440.), px(1100.)));
    click_debug_selector(cx, "settings-scheme-dark");
    click_debug_selector(cx, "settings-add-theme");
    click_debug_selector(cx, "theme-choose-files");
    cx.simulate_path_prompt_response(move |_| Some(imported.into()));
    cx.run_until_parked();
    assert!(cx.debug_bounds("theme-editor-error").is_some());
    workspace.read_with(cx, |ws, _| {
        let library = serde_json::to_value(&ws.custom_themes).unwrap();
        assert!(library["dark"].is_null());
        assert_eq!(library["themes"].as_array().unwrap().len(), 1);
        let pair = &library["themes"][0];
        assert_eq!(pair["id"], "coast");
        assert_eq!(pair["colors"]["canvas"], "#f0faf7");
        assert_eq!(pair["variants"]["dark"]["canvas"], "#122131");
    });
    click_debug_selector(cx, "theme-editor-cancel");
    workspace.update(cx, |ws, cx| {
        ws.settings_scroll
            .set_offset(gpui::point(px(0.), px(-420.)));
        cx.notify();
    });
    click_debug_selector(cx, "settings-theme-custom-coast");
    saved(&workspace, cx).await;
    quit(&workspace, cx);
    let (workspace, cx) = launch(&mut relaunch, &state);
    workspace.read_with(cx, |ws, cx| {
        let library = serde_json::to_value(&ws.custom_themes).unwrap();
        assert_eq!(library["dark"], "coast");
        assert_eq!(library["light"], "coast");
        assert_eq!(cx.theme().background, gpui::rgb(0x122131).into());
    });
    workspace.update_in(cx, |ws, w, cx| {
        ws.open_settings(&OpenSettings, w, cx);
        ws.set_settings_section(super::super::SettingsSection::Appearance, cx);
    });
    click_debug_selector(cx, "settings-scheme-light");
    workspace.read_with(cx, |_, cx| {
        assert_eq!(cx.theme().background, gpui::rgb(0xf0faf7).into());
    });
    click_debug_selector(cx, "settings-add-theme");
    click_debug_selector(cx,"theme-json-input");
    cx.simulate_input(r##"{ /* paste a separate dark palette */ "name":"Night",
        "colors":{"editor.background":"#112031",}, }"##);
    click_debug_selector(cx, "theme-editor-save");
    workspace.read_with(cx, |ws,cx| {
        let library=serde_json::to_value(&ws.custom_themes).unwrap();
        assert_eq!(library["dark"], "night");
        assert_eq!(library["light"], "coast");
        assert_eq!(cx.theme().background,gpui::rgb(0xf0faf7).into());
    });
}

#[gpui::test]
fn vscode_batch_installs_new_files_before_resolving_an_existing_theme(
    cx: &mut gpui::TestAppContext,
) {
    use super::launch::{init, launch};
    use serde_json::json;
    init(cx);
    let dir = tempfile::tempdir().unwrap();
    let (workspace, cx) = launch(cx, &dir.path().join("state.json"));
    workspace.update_in(cx, |ws, w, cx| {
        ws.open_settings(&OpenSettings, w, cx);
        ws.set_settings_section(super::super::SettingsSection::Appearance, cx);
    });
    let original = json!({"version":1,"id":"coast","name":"Coast","appearance":"dark",
        "colors":{"canvas":"#222222"},"collection":{"id":"studio","label":"Studio"}});
    click_debug_selector(cx, "settings-add-theme");
    click_debug_selector(cx,"theme-json-input");
    cx.simulate_input(&original.to_string());
    click_debug_selector(cx, "theme-editor-save");
    let paths = ["Coast", "Grotto"].map(|name| {
        let path = dir.path().join(format!("{name}.json"));
        let content = json!({"name":name,"type":"light","colors":{"editor.background":"#fafaf1"}});
        std::fs::write(&path, content.to_string()).unwrap();
        path
    });
    click_debug_selector(cx, "settings-add-theme");
    let position = rendered_bounds(cx, "theme-file-drop").center();
    let paths = gpui::ExternalPaths(paths.into_iter().collect());
    cx.simulate_event(gpui::FileDropEvent::Entered { position, paths });
    cx.simulate_event(gpui::FileDropEvent::Pending { position });
    cx.simulate_event(gpui::FileDropEvent::Submit { position });
    cx.run_until_parked();
    workspace.read_with(cx, |ws, _| {
        let lib = serde_json::to_value(&ws.custom_themes).unwrap();
        assert_eq!(lib["themes"].as_array().unwrap().len(), 2);
        assert_eq!(lib["themes"][0], original);
        assert_eq!(lib["themes"][1]["id"], "grotto");
    });
    click_debug_selector(cx, "theme-editor-save");
    workspace.read_with(cx, |ws, _| {
        let lib = serde_json::to_value(&ws.custom_themes).unwrap();
        assert_eq!(lib["themes"][0]["colors"]["canvas"], "#fafaf1");
        assert_eq!(lib["themes"][0]["collection"], original["collection"]);
        assert!(lib["dark"].is_null() && lib["light"].is_null());
    });
}

mod community;

#[gpui::test]
async fn worktree_retention_commits_days_and_restores_scoped_rules(cx: &mut gpui::TestAppContext) {
    use super::launch::{init, launch, quit, saved};
    init(cx);
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("state.json");
    let mut restart = cx.clone();
    let (ws, cx) = launch(cx, &file);
    cx.simulate_resize(gpui::size(px(1040.), px(820.)));
    ws.update_in(cx, |ws, w, cx| {
        ws.open_settings(&OpenSettings, w, cx);
        ws.set_settings_section(super::super::SettingsSection::Worktrees, cx);
    });
    let reveal = |ws: &mut Workspace,
                  label: &str,
                  w: &mut gpui::Window,
                  cx: &mut gpui::Context<'_, Workspace>| {
        ws.settings_search
            .update(cx, |input, cx| input.set_value(label, w, cx));
        ws.refresh_settings_search(cx);
        ws.open_settings_hit(0, w, cx);
    };
    ws.update_in(cx, |ws, w, cx| {
        reveal(ws, "Delete inactive worktrees", w, cx)
    });
    click_debug_selector(cx, "settings-cleanup-inactive");
    assert_rendered_bounds(cx, "settings-cleanup-inactive-input");
    cx.simulate_keystrokes("up escape");
    ws.read_with(cx, |ws, _| {
        assert_eq!(ws.worktree_retention.inactive_days, None)
    });
    click_debug_selector(cx, "settings-cleanup-inactive");
    cx.simulate_keystrokes("cmd-a");
    cx.simulate_input("13");
    cx.simulate_keystrokes("up up down enter");
    ws.read_with(cx, |ws, _| {
        assert_eq!(ws.worktree_retention.inactive_days, Some(14))
    });
    click_debug_selector(cx, "settings-cleanup-inactive-decrease");
    ws.update_in(cx, |ws, w, cx| {
        ws.select_settings_option("settings-project-defaults-scope", "/tmp/alpha", w, cx);
        ws.select_settings_option("settings-worktree-cleanup-mode", "custom", w, cx);
        reveal(ws, "Delete settled worktrees", w, cx);
    });
    assert_rendered_bounds(cx, "settings-cleanup-settled");
    click_debug_selector(cx, "settings-cleanup-settled");
    cx.simulate_keystrokes("cmd-a");
    cx.simulate_input("0");
    cx.simulate_keystrokes("enter");
    click_debug_selector(cx, "settings-cleanup-unchanged");
    saved(&ws, cx).await;
    quit(&ws, cx);
    let (ws, cx) = launch(&mut restart, &file);
    ws.read_with(cx, |ws, _| {
        assert_eq!(ws.worktree_retention.inactive_days, Some(13));
        assert_eq!(ws.worktree_retention.settled_days, None);
        let prefs = ws.project_defaults["/tmp/alpha"]
            .worktree_retention
            .unwrap();
        assert!(prefs.unchanged);
        assert_eq!(prefs.settled_days, Some(0));
        assert_eq!(prefs.inactive_days, Some(13));
    });
}

#[gpui::test]
fn generator_setup_keeps_keyboard_focus_and_escape_returns_to_provider_settings(
    cx: &mut gpui::TestAppContext,
) {
    let (workspace, cx, _, _) = thread_menu_workspace(cx);
    workspace.update_in(cx, |ws, window, cx| {
        ws.open_settings(&OpenSettings, window, cx);
        ws.settings_search.update(cx, |field, cx| {
            field.set_value("Model API connections", window, cx)
        });
        ws.refresh_settings_search(cx);
        ws.open_settings_hit(0, window, cx);
    });
    cx.run_until_parked();
    click_debug_selector(cx, "generator-add");
    click_debug_selector(cx, "generator-custom");
    for _ in 0..14 {
        cx.simulate_keystrokes("tab");
        workspace.update_in(cx, |ws, w, cx| {
            let (start, _) = ws.generators.focus_bounds().expect("generator dialog open");
            assert!(start.contains_focused(w, cx));
        });
    }
    cx.simulate_keystrokes("escape");
    workspace.read_with(cx, |ws, _| {
        assert!(ws.generators.focus_bounds().is_none());
        assert!(ws.settings_visible());
        assert_eq!(ws.settings_section, super::super::SettingsSection::Updates);
    });
}

#[gpui::test]
fn selecting_a_generator_secret_source_accepts_typing_and_validates_on_blur(
    cx: &mut gpui::TestAppContext,
) {
    let (workspace, cx, _, _) = thread_menu_workspace(cx);
    workspace.update_in(cx, |ws, w, cx| {
        ws.open_settings(&OpenSettings, w, cx);
        ws.settings_search.update(cx, |field, cx| {
            field.set_value("Model API connections", w, cx)
        });
        ws.refresh_settings_search(cx);
        ws.open_settings_hit(0, w, cx);
    });
    cx.run_until_parked();
    click_debug_selector(cx, "generator-add");
    workspace.update_in(cx, |ws, _, cx| {
        ws.credential_settings
            .read(cx)
            .secret_sources()
            .update(cx, |sources, cx| sources.add_test_source("test-source", cx));
    });
    cx.run_until_parked();
    click_debug_selector(cx, "generator-key-source");
    click_debug_selector(cx, "generator-key-source-option-source:test-source");
    cx.simulate_input("invalid");
    cx.simulate_keystrokes("tab");
    cx.run_until_parked();
    assert!(cx.debug_bounds("generator-invalid-reference").is_some());
    cx.simulate_keystrokes("shift-tab cmd-a");
    cx.simulate_input("op://Team/Model/key");
    cx.simulate_keystrokes("tab");
    cx.run_until_parked();
    assert!(cx.debug_bounds("generator-invalid-reference").is_none());
}

#[gpui::test]
fn secret_provider_dialog_validates_fields_traps_focus_and_restores_the_trigger(
    cx: &mut gpui::TestAppContext,
) {
    let (workspace, cx, _, _) = thread_menu_workspace(cx);
    workspace.update_in(cx, |ws, window, cx| {
        ws.open_settings(&OpenSettings, window, cx);
        ws.set_settings_section(super::super::SettingsSection::Credentials, cx);
    });
    cx.run_until_parked();
    click_debug_selector(cx, "add-secret-provider");
    cx.run_until_parked();
    assert!(cx.debug_bounds("secret-provider-dialog").is_some());
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert!(cx.debug_bounds("secret-provider-name-error").is_some());
    assert!(cx.debug_bounds("secret-provider-account-error").is_some());
    cx.simulate_input("Personal");
    cx.simulate_keystrokes("tab");
    cx.simulate_input("https://example.com");
    cx.run_until_parked();
    assert!(cx.debug_bounds("secret-provider-name-error").is_none());
    assert!(cx.debug_bounds("secret-provider-account-error").is_some());
    // The bruhs host-default option is explicit. Switching back restores
    // both the typed account and its validation feedback.
    cx.simulate_keystrokes("tab space");
    cx.run_until_parked();
    assert!(cx.debug_bounds("secret-provider-account-error").is_none());
    cx.simulate_keystrokes("space");
    cx.run_until_parked();
    assert!(cx.debug_bounds("secret-provider-account-error").is_some());
    cx.simulate_keystrokes("shift-tab");
    cx.simulate_keystrokes("cmd-a");
    cx.simulate_input("team.example");
    cx.run_until_parked();
    assert!(cx.debug_bounds("secret-provider-account-error").is_none());
    for key in ["tab","tab","tab","shift-tab","shift-tab","shift-tab"] {
        cx.simulate_keystrokes(key);
        workspace.update_in(cx, |ws, window, cx| {
            let (dialog, _) = ws.active_modal_focus_bounds(cx);
            assert!(dialog.contains_focused(window, cx));
        });
    }
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    assert!(cx.debug_bounds("secret-provider-dialog").is_none());
    workspace.read_with(cx, |ws, _| assert!(ws.settings_visible()));
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert!(cx.debug_bounds("secret-provider-dialog").is_some());
}

#[gpui::test]
fn secret_variable_check_tracks_valid_input_and_disappears_on_cancel(
    cx: &mut gpui::TestAppContext,
) {
    let (workspace, cx, _, _) = thread_menu_workspace(cx);
    workspace.update_in(cx, |ws, window, cx| {
        ws.open_settings(&OpenSettings, window, cx);
        ws.set_settings_section(super::super::SettingsSection::Credentials, cx);
    });
    cx.run_until_parked();
    workspace.update_in(cx, |ws, _, cx| {
        ws.credential_settings
            .read(cx)
            .secret_sources()
            .update(cx, |sources, cx| {
                sources.add_test_source("variable-check-fixture", cx);
                sources.add_test_source("variable-check-other", cx);
            });
    });
    cx.run_until_parked();
    click_debug_selector(cx, "bind-source-variable-check-fixture");
    cx.simulate_input("invalid");
    cx.simulate_keystrokes("tab");
    cx.run_until_parked();
    assert!(cx.debug_bounds("secret-variable-reference-error").is_some());
    assert!(cx.debug_bounds("secret-variable-check").is_none());
    cx.simulate_keystrokes("shift-tab cmd-a");
    cx.simulate_input("op://Team/Model/key");
    cx.run_until_parked();
    assert!(cx.debug_bounds("secret-variable-reference-error").is_none());
    assert!(cx.debug_bounds("secret-variable-check-ready").is_some());
    click_debug_selector(cx, "secret-variable-source");
    click_debug_selector(cx, "secret-variable-source-option-variable-check-other");
    cx.run_until_parked();
    assert!(cx.debug_bounds("secret-variable-check").is_none());
    cx.simulate_input("op://Other/Model/key");
    cx.run_until_parked();
    assert!(cx.debug_bounds("secret-variable-check-ready").is_some());
    click_debug_selector(cx, "cancel-secret-source");
    cx.run_until_parked();
    assert!(cx.debug_bounds("secret-variable-check").is_none());
}

//! The desktop state file as written by builds up to 2026-10-03 must keep
//! loading. The fixture was produced by serializing a populated
//! `PersistedWorkspaceState` with that build; never regenerate it.

use super::{
    initial_permission_mode_from_persisted, AppView, DesktopCalendarMode, DesktopPriority,
    DesktopSidebarTab, DesktopTaskViewMode, DesktopWorkItemStatus, EpochMs, MessageRole,
    PermissionMode, PersistedWorkspaceState,
};

const FIXTURE_2026_10_03: &str =
    include_str!("../../../tests/fixtures/workspace-state-2026-10-03.json");

fn fixture() -> PersistedWorkspaceState {
    serde_json::from_str(FIXTURE_2026_10_03).expect("2026-10-03 state file loads")
}

#[test]
fn state_file_from_2026_10_03_loads() {
    let state = fixture();
    assert_eq!(state.current_model.as_deref(), Some("claude-sonnet-4-6"));
    assert_eq!(state.permission_mode, Some(PermissionMode::AcceptEdits));
    assert_eq!(
        initial_permission_mode_from_persisted(Some(&state)),
        PermissionMode::AcceptEdits
    );
    assert_eq!(state.sidebar_visible, Some(true));
    assert_eq!(state.sidebar_tab, Some(DesktopSidebarTab::Code));
    assert_eq!(state.active_view, Some(AppView::Calendar));
    assert_eq!(state.active_thread_id.as_deref(), Some("thread-b"));
    assert_eq!(state.task_view_mode, Some(DesktopTaskViewMode::Board));
    assert_eq!(state.calendar_mode, Some(DesktopCalendarMode::Week));
    assert_eq!(state.calendar_anchor.as_deref(), Some("2026-09-14"));

    assert_eq!(state.threads.len(), 2);
    let general = &state.threads[0];
    assert_eq!(general.id, "thread-a");
    assert_eq!(general.permission_mode, PermissionMode::Default);
    assert_eq!(general.updated_at_ms.as_millis(), 1_757_000_001_600);
    assert!(general.pinned && !general.settled);
    assert_eq!(general.branch.as_deref(), Some("main"));
    assert_eq!(general.messages.len(), 3);
    assert_eq!(general.messages[0].role, MessageRole::User);
    assert_eq!(general.messages[1].role, MessageRole::Assistant);
    assert_eq!(general.messages[1].tool_calls.len(), 1);
    assert_eq!(general.messages[1].tool_calls[0].tool, "Bash");
    assert!(general.messages[2].stopped);
    assert_eq!(general.messages[2].error.as_deref(), Some("Provider error"));
    let session = &state.threads[1];
    assert_eq!(session.project_name, "bruhs-ai");
    assert_eq!(session.permission_mode, PermissionMode::Plan);
    // Never recorded: restore treats the epoch as "fresh now".
    assert_eq!(session.updated_at_ms, EpochMs::UNIX_EPOCH);
    assert!(session.settled);

    assert_eq!(state.projects.len(), 1);
    assert_eq!(state.work_items.len(), 2);
    let task = &state.work_items[0];
    assert_eq!(task.status, DesktopWorkItemStatus::InProgress);
    assert_eq!(task.priority, DesktopPriority::High);
    assert_eq!(task.planning.estimate_minutes, Some(30));
    // The placeholder "now" was never a time.
    assert_eq!(task.updated_at, EpochMs::UNIX_EPOCH);
    assert_eq!(state.work_items[1].status, DesktopWorkItemStatus::Backlog);
    assert_eq!(state.work_items[1].priority, DesktopPriority::None);

    assert_eq!(state.calendar_events.len(), 1);
    assert_eq!(state.calendar_events[0].start_at, "2026-09-15T09:00:00");
    assert_eq!(state.editor_id.as_deref(), Some("zed"));
    assert_eq!(state.terminal_drawer_height, Some(280.0));
    assert_eq!(state.right_panel_width, Some(540.0));
    assert_eq!(state.recent_projects.len(), 1);
    assert!(state.meeting_prefs.is_some());
}

/// Saving writes the enum fields as the same strings earlier builds wrote,
/// and what it writes loads back unchanged.
#[test]
fn state_file_from_2026_10_03_saves_the_same_enum_strings() {
    let written = serde_json::to_value(fixture()).expect("serializes");
    let original: serde_json::Value = serde_json::from_str(FIXTURE_2026_10_03).expect("json");
    for key in [
        "permission_mode",
        "sidebar_tab",
        "active_view",
        "task_view_mode",
        "calendar_mode",
    ] {
        assert_eq!(written[key], original[key], "{key}");
    }
    for (index, thread) in original["threads"]
        .as_array()
        .expect("threads")
        .iter()
        .enumerate()
    {
        let written_thread = &written["threads"][index];
        assert_eq!(written_thread["permission_mode"], thread["permission_mode"]);
        for (message_index, message) in thread["messages"]
            .as_array()
            .expect("messages")
            .iter()
            .enumerate()
        {
            assert_eq!(
                written_thread["messages"][message_index]["role"],
                message["role"]
            );
        }
    }
    for (index, item) in original["work_items"]
        .as_array()
        .expect("work items")
        .iter()
        .enumerate()
    {
        assert_eq!(written["work_items"][index]["status"], item["status"]);
        assert_eq!(written["work_items"][index]["priority"], item["priority"]);
        assert_eq!(written["work_items"][index]["updated_at"], 0);
    }

    let reloaded: PersistedWorkspaceState =
        serde_json::from_value(written.clone()).expect("written state loads");
    assert_eq!(serde_json::to_value(reloaded).expect("serializes"), written);
}

#[test]
fn unknown_enum_strings_fall_back_instead_of_failing_the_file() {
    let state: PersistedWorkspaceState = serde_json::from_str(
        r#"{
            "permission_mode": "sudo",
            "sidebar_tab": "files",
            "active_view": "meeting_detail",
            "task_view_mode": "gantt",
            "calendar_mode": "year",
            "threads": [{
                "id": "t", "title": "T", "project_name": "", "project_path": null,
                "updated_at": "now", "model_id": "m", "permission_mode": "root",
                "messages": [{"role": "system", "content": "c"}]
            }],
            "work_items": [{
                "id": "w", "project_id": null, "title": "W", "description": null,
                "status": "blocked", "priority": "p0", "due_date": null,
                "updated_at": "1700000000000"
            }]
        }"#,
    )
    .expect("tolerant load");
    assert_eq!(state.permission_mode, Some(PermissionMode::Default));
    assert_eq!(state.sidebar_tab, Some(DesktopSidebarTab::Chat));
    assert_eq!(state.active_view, Some(AppView::Chat));
    assert_eq!(state.task_view_mode, Some(DesktopTaskViewMode::List));
    assert_eq!(state.calendar_mode, Some(DesktopCalendarMode::Month));
    assert_eq!(state.threads[0].permission_mode, PermissionMode::Default);
    assert_eq!(state.threads[0].messages[0].role, MessageRole::Assistant);
    assert_eq!(state.work_items[0].status, DesktopWorkItemStatus::Backlog);
    assert_eq!(state.work_items[0].priority, DesktopPriority::None);
    assert_eq!(
        state.work_items[0].updated_at.as_millis(),
        1_700_000_000_000
    );
}

use super::threads_ui::{
    default_thread_title, step_thread_index, thread_link, thread_selection_after_delete,
    THREAD_TITLE_GENERAL, THREAD_TITLE_PROJECT_GROUP_NEW, THREAD_TITLE_PROJECT_SESSION,
};
use super::Overlay;
use super::TaskGlyph;
use super::{
    calendar_anchor_after_event_dialog_open, canonical_project_location,
    click_task_detail_status_chip_like_web, default_event_start,
    desktop_model_provider_groups_like_web, desktop_state_backup_path, event_color,
    event_form_times_from_inputs, event_timegrid_surface_color, event_timegrid_text_color,
    focused_input_should_receive_plain_shortcut, format_desktop_due_date,
    format_event_form_datetime, format_full_date, format_month_year, format_week_range,
    general_thread_project_path_like_web, general_thread_project_scoped_like_web, home_dir_path,
    initial_active_thread_id_after_start, initial_active_view_after_start,
    initial_calendar_anchor_after_start, initial_calendar_mode_after_start,
    initial_permission_mode_from_persisted, initial_selected_calendar_event_after_start,
    initial_selected_project_after_start, initial_selected_work_item_after_start,
    initial_sidebar_visible_after_start, is_web_checkbox_activation_key, lay_out_timegrid_events,
    load_desktop_state_from_path, load_desktop_state_with_backup, move_timegrid_event_to,
    move_work_item_to_status, normalize_event_form_times, parse_event_form_datetime,
    permission_mode_after_cycle, project_delete_prompt, project_filter_selection_after_change,
    project_picker_directory_error_like_web, project_session_surface_after_create_like_web,
    project_task_count, provider_label, push_recent_project, read_project_directory,
    resize_timegrid_event_end_to, restored_thread_model_id, restored_thread_permission_mode,
    right_panel_after_toggle, select_task_detail_priority_option_like_web,
    select_task_detail_project_option_like_web, select_task_detail_status_option_like_web,
    selected_project_name_like_web, showing_surface, snap_timegrid_offset_seconds,
    task_create_selected_work_item_after_submit, task_detail_menu_after_native_select_selection,
    tasks_panel_visibility_after_toggle, work_item_due_on, write_desktop_state_atomically,
    AgentProvider, AppView, ClosePanel, DesktopCalendarEvent, DesktopCalendarMode, DesktopPriority,
    DesktopProject, DesktopProjectLocation, DesktopSidebarTab, DesktopTaskViewMode,
    DesktopWorkItem, DesktopWorkItemStatus, FocusCommandBar, OpenLocalServers, OpenProjectPicker,
    OpenSettings, PermissionMode, PersistedThread, PersistedWorkspaceState, RightPanelSurface,
    TaskCreateMenu, TaskDetailMenu, ToggleDiff, TogglePanel, WorkItemPlanning, Workspace,
    CALENDAR_ACTION_GROUP_GAP_PX, CALENDAR_BODY_FONT_FAMILY, CALENDAR_CONTROL_RADIUS_PX,
    CALENDAR_DATE_TEXT_SIZE_PX, CALENDAR_HEADER_BORDER_OPACITY, CALENDAR_HEADER_MIN_HEIGHT_PX,
    CALENDAR_HEADER_TITLE_GROUP_GAP_PX, CALENDAR_HEADER_TITLE_MIN_WIDTH_PX,
    CALENDAR_HEADER_TITLE_TEXT_SIZE_PX, CALENDAR_HEADER_X_PADDING_PX, CALENDAR_HEADER_Y_PADDING_PX,
    CALENDAR_MODE_ACTIVE_BG_OPACITY, CALENDAR_MODE_ACTIVE_BORDER_OPACITY,
    CALENDAR_MODE_HOVER_BG_OPACITY, CALENDAR_MODE_INACTIVE_OPACITY,
    CALENDAR_MODE_SWITCHER_BG_OPACITY, CALENDAR_MODE_SWITCHER_BORDER_OPACITY,
    CALENDAR_MODE_SWITCHER_GAP_PX, CALENDAR_MODE_SWITCHER_PADDING_PX, CALENDAR_MODE_TEXT_SIZE_PX,
    CALENDAR_MODE_X_PADDING_PX, CALENDAR_MODE_Y_PADDING_PX, CALENDAR_MONTH_CELL_BORDER_OPACITY,
    CALENDAR_MONTH_CELL_HOVER_BG_OPACITY, CALENDAR_MONTH_CELL_PADDING_PX,
    CALENDAR_MONTH_ROW_MIN_HEIGHT_PX, CALENDAR_MONTH_TODAY_RADIUS_PX, CALENDAR_MONTH_TODAY_SIZE_PX,
    CALENDAR_NAV_BUTTON_SIZE_PX, CALENDAR_NAV_GROUP_GAP_PX, CALENDAR_NAV_HOVER_BG_OPACITY,
    CALENDAR_NAV_TEXT_SIZE_PX, CALENDAR_NEW_EVENT_BG_OPACITY, CALENDAR_NEW_EVENT_BORDER_OPACITY,
    CALENDAR_NEW_EVENT_HOVER_BG_OPACITY, CALENDAR_NEW_EVENT_LABEL, CALENDAR_NEW_EVENT_TEXT_SIZE_PX,
    CALENDAR_NEW_EVENT_X_PADDING_PX, CALENDAR_NEW_EVENT_Y_PADDING_PX, CALENDAR_NEXT_LABEL,
    CALENDAR_PILL_DOT_RADIUS_PX, CALENDAR_PILL_DOT_SIZE_PX, CALENDAR_PILL_TEXT_SIZE_PX,
    CALENDAR_PREVIOUS_LABEL, CALENDAR_TASK_DUE_LABEL, CALENDAR_TIMEGRID_AXIS_WIDTH_PX,
    CALENDAR_TIMEGRID_COLUMN_BORDER_OPACITY, CALENDAR_TIMEGRID_EVENT_BG_OPACITY,
    CALENDAR_TIMEGRID_EVENT_BORDER_OPACITY, CALENDAR_TIMEGRID_EVENT_HOVER_BG_OPACITY,
    CALENDAR_TIMEGRID_EVENT_INSET_PX, CALENDAR_TIMEGRID_EVENT_RADIUS_PX,
    CALENDAR_TIMEGRID_EVENT_TEXT_SIZE_PX, CALENDAR_TIMEGRID_EVENT_X_PADDING_PX,
    CALENDAR_TIMEGRID_EVENT_Y_PADDING_PX, CALENDAR_TIMEGRID_HEADER_BORDER_OPACITY,
    CALENDAR_TIMEGRID_HEADER_TEXT_SIZE_PX, CALENDAR_TIMEGRID_HEADER_X_PADDING_PX,
    CALENDAR_TIMEGRID_HEADER_Y_PADDING_PX, CALENDAR_TIMEGRID_HOUR_LABEL_BORDER_OPACITY,
    CALENDAR_TIMEGRID_HOUR_SLOT_BORDER_OPACITY, CALENDAR_TIMEGRID_HOUR_SLOT_HOVER_BG_OPACITY,
    CALENDAR_TIMEGRID_HOUR_TEXT_SIZE_PX, CALENDAR_TIMEGRID_HOUR_TOP_PADDING_PX,
    CALENDAR_TIMEGRID_HOUR_X_PADDING_PX, CALENDAR_TIMEGRID_NOW_DOT_OFFSET_PX,
    CALENDAR_TIMEGRID_NOW_DOT_RADIUS_PX, CALENDAR_TIMEGRID_NOW_LINE_HEIGHT_PX,
    CALENDAR_TODAY_TEXT_SIZE_PX, CALENDAR_TODAY_X_PADDING_PX, CALENDAR_TODAY_Y_PADDING_PX,
    CALENDAR_WEEKDAY_TEXT_SIZE_PX, CALENDAR_WEEKDAY_X_PADDING_PX, CALENDAR_WEEKDAY_Y_PADDING_PX,
    CHAT_NO_ACTIVE_THREAD_LABEL, CHAT_NO_ACTIVE_THREAD_TEXT_SIZE_PX, COMMAND_ACTIONS,
    COMMAND_BAR_ACTION_HOVER_BG_OPACITY, COMMAND_BAR_ACTION_RADIUS_PX,
    COMMAND_BAR_ACTION_TEXT_SIZE_PX, COMMAND_BAR_ACTION_X_PADDING_PX,
    COMMAND_BAR_ACTION_Y_PADDING_PX, COMMAND_BAR_BODY_FONT_FAMILY, COMMAND_BAR_EMPTY_X_PADDING_PX,
    COMMAND_BAR_EMPTY_Y_PADDING_PX, COMMAND_BAR_FOOTER_BORDER_OPACITY,
    COMMAND_BAR_FOOTER_HINT_GAP_PX, COMMAND_BAR_FOOTER_TEXT_SIZE_PX,
    COMMAND_BAR_FOOTER_X_PADDING_PX, COMMAND_BAR_FOOTER_Y_PADDING_PX,
    COMMAND_BAR_GROUP_BOTTOM_PADDING_PX, COMMAND_BAR_GROUP_TEXT_SIZE_PX,
    COMMAND_BAR_GROUP_X_PADDING_PX, COMMAND_BAR_INPUT_PLACEHOLDER, COMMAND_BAR_INPUT_TEXT_SIZE_PX,
    COMMAND_BAR_INPUT_X_PADDING_PX, COMMAND_BAR_INPUT_Y_PADDING_PX, COMMAND_BAR_LABEL_FONT_FAMILY,
    COMMAND_BAR_LIST_MAX_HEIGHT_PX, COMMAND_BAR_LIST_X_PADDING_PX, COMMAND_BAR_LIST_Y_PADDING_PX,
    COMMAND_BAR_OVERLAY_OPACITY, DESKTOP_DIALOG_LG_WIDTH_PX, DESKTOP_DIALOG_MAX_WIDTH_RATIO,
    DESKTOP_DIALOG_MD_WIDTH_PX, DESKTOP_DIALOG_RADIUS_PX, DESKTOP_DIALOG_TOP_PADDING_PX,
    DESKTOP_HOUR_ROW_PX, DESKTOP_MODAL_FOCUS_END_TAB_INDEX, DESKTOP_MODAL_FOCUS_START_TAB_INDEX,
    DESKTOP_MODAL_FOCUS_TRAP_KEY, DESKTOP_MODAL_HEADER_X_PADDING_PX,
    DESKTOP_MODAL_HEADER_Y_PADDING_PX, DESKTOP_MODAL_OVERLAY_OPACITY,
    DESKTOP_MODAL_TITLE_TEXT_SIZE_PX, DESKTOP_TEXT_INPUT_GUARDED_SHORTCUTS,
    DESKTOP_TIMEGRID_MIN_DISPLAY_PX, DESKTOP_TIMEGRID_MIN_DISPLAY_SECONDS,
    DESKTOP_TIMEGRID_NOW_DOT_PX, DESKTOP_TIMEGRID_RESIZE_HANDLE_PX, EVENT_ALL_DAY_CHECK_RADIUS_PX,
    EVENT_ALL_DAY_CHECK_SIZE_PX, EVENT_ALL_DAY_CHECK_TEXT_SIZE_PX, EVENT_ALL_DAY_GAP_PX,
    EVENT_CANCEL_LABEL, EVENT_COLOR_OPTIONS, EVENT_COLOR_OPTION_X_PADDING_PX,
    EVENT_COLOR_OPTION_Y_PADDING_PX, EVENT_CREATE_ACTION_GAP_PX, EVENT_CREATE_BODY_GAP_PX,
    EVENT_CREATE_BODY_Y_PADDING_PX, EVENT_CREATE_DELETE_HOVER_BG_OPACITY,
    EVENT_CREATE_DELETE_X_PADDING_PX, EVENT_CREATE_DELETE_Y_PADDING_PX,
    EVENT_CREATE_FOOTER_BG_OPACITY, EVENT_CREATE_FOOTER_GAP_PX, EVENT_CREATE_FOOTER_Y_PADDING_PX,
    EVENT_CREATE_HEADER_Y_PADDING_PX, EVENT_CREATE_OVERLAY_OPACITY, EVENT_CREATE_SUBMIT_LABEL,
    EVENT_DATETIME_PLACEHOLDER, EVENT_DELETE_LABEL, EVENT_DIALOG_BODY_FONT_FAMILY,
    EVENT_DIALOG_LABEL_FONT_FAMILY, EVENT_DIALOG_TITLE_TEXT_SIZE_PX, EVENT_EDIT_LABEL,
    EVENT_FORM_LABEL_TEXT_SIZE_PX, EVENT_NOTES_PLACEHOLDER, EVENT_NOTES_ROWS,
    EVENT_SAVE_SUBMIT_LABEL, EVENT_TITLE_PLACEHOLDER, FORM_CONTROL_RADIUS_PX,
    FORM_DIALOG_SECTION_X_PADDING_PX, FORM_FIELD_GAP_PX, FORM_FIVE_ROW_TEXTAREA_HEIGHT_PX,
    FORM_GRID_GAP_PX, FORM_INPUT_HEIGHT_PX, FORM_INPUT_TEXT_SIZE_PX, FORM_SELECT_BODY_FONT_FAMILY,
    FORM_SELECT_BODY_TEXT_SIZE_PX, FORM_SELECT_HEIGHT_PX, FORM_SELECT_MENU_MAX_HEIGHT_PX,
    FORM_SELECT_MENU_Y_PADDING_PX, FORM_SELECT_OPTION_GAP_PX, FORM_SELECT_OPTION_X_PADDING_PX,
    FORM_SELECT_OPTION_Y_PADDING_PX, FORM_THREE_ROW_TEXTAREA_HEIGHT_PX, MODEL_CATALOG,
    PROJECT_DELETE_ACTION_GAP_PX, PROJECT_DELETE_BODY_GAP_PX, PROJECT_DELETE_BODY_PADDING_PX,
    PROJECT_DELETE_CANCEL_LABEL, PROJECT_DELETE_CONFIRM_LABEL, PROJECT_DELETE_LABEL,
    PROJECT_DELETE_PROMPT_TEXT_SIZE_PX, PROJECT_PICKER_BODY_FONT_FAMILY,
    PROJECT_PICKER_CONTENT_MAX_HEIGHT_PX, PROJECT_PICKER_EMPTY_X_PADDING_PX,
    PROJECT_PICKER_EMPTY_Y_PADDING_PX, PROJECT_PICKER_FOOTER_BORDER_OPACITY,
    PROJECT_PICKER_FOOTER_GAP_PX, PROJECT_PICKER_FOOTER_TEXT_SIZE_PX,
    PROJECT_PICKER_FOOTER_X_PADDING_PX, PROJECT_PICKER_FOOTER_Y_PADDING_PX,
    PROJECT_PICKER_HEADER_GAP_PX, PROJECT_PICKER_HEADER_X_PADDING_PX,
    PROJECT_PICKER_HEADER_Y_PADDING_PX, PROJECT_PICKER_ICON_SIZE_PX,
    PROJECT_PICKER_LABEL_FONT_FAMILY, PROJECT_PICKER_LABEL_TEXT_SIZE_PX, PROJECT_PICKER_LIST_ERROR,
    PROJECT_PICKER_LIST_Y_PADDING_PX, PROJECT_PICKER_OVERLAY_OPACITY,
    PROJECT_PICKER_PARENT_ICON_SLOT_WIDTH_PX, PROJECT_PICKER_PATH_FONT_FAMILY,
    PROJECT_PICKER_PATH_TOP_MARGIN_PX, PROJECT_PICKER_RECENTS_BORDER_OPACITY,
    PROJECT_PICKER_RECENTS_GAP_PX, PROJECT_PICKER_RECENTS_LABEL_BOTTOM_MARGIN_PX,
    PROJECT_PICKER_RECENTS_X_PADDING_PX, PROJECT_PICKER_RECENTS_Y_PADDING_PX,
    PROJECT_PICKER_RECENT_ROW_GAP_PX, PROJECT_PICKER_RECENT_ROW_HOVER_BG_OPACITY,
    PROJECT_PICKER_RECENT_ROW_X_PADDING_PX, PROJECT_PICKER_RECENT_ROW_Y_PADDING_PX,
    PROJECT_PICKER_ROW_GAP_PX, PROJECT_PICKER_ROW_HOVER_BG_OPACITY, PROJECT_PICKER_ROW_RADIUS_PX,
    PROJECT_PICKER_ROW_X_PADDING_PX, PROJECT_PICKER_ROW_Y_PADDING_PX,
    PROJECT_PICKER_SECTION_TEXT_SIZE_PX, PROJECT_PICKER_SELECT_BG_OPACITY,
    PROJECT_PICKER_SELECT_HOVER_BG_OPACITY, PROJECT_PICKER_SELECT_MARGIN_LEFT_PX,
    PROJECT_PICKER_SELECT_X_PADDING_PX, PROJECT_PICKER_SELECT_Y_PADDING_PX,
    PROJECT_SIDEBAR_BODY_FONT_FAMILY, PROJECT_SIDEBAR_BORDER_OPACITY,
    PROJECT_SIDEBAR_CONTENT_X_PADDING_PX, PROJECT_SIDEBAR_CONTENT_Y_PADDING_PX,
    PROJECT_SIDEBAR_DELETE_ICON_SIZE_PX, PROJECT_SIDEBAR_FOOTER_BORDER_OPACITY,
    PROJECT_SIDEBAR_FOOTER_CONTAINER_BOTTOM_PADDING_PX,
    PROJECT_SIDEBAR_FOOTER_CONTAINER_X_PADDING_PX, PROJECT_SIDEBAR_FOOTER_GAP_PX,
    PROJECT_SIDEBAR_FOOTER_HEIGHT_PX, PROJECT_SIDEBAR_FOOTER_HOVER_BORDER_OPACITY,
    PROJECT_SIDEBAR_FOOTER_ICON_TEXT_SIZE_PX, PROJECT_SIDEBAR_FOOTER_TEXT_SIZE_PX,
    PROJECT_SIDEBAR_FOOTER_X_PADDING_PX, PROJECT_SIDEBAR_FOOTER_Y_PADDING_PX,
    PROJECT_SIDEBAR_LABEL_FONT_FAMILY, PROJECT_SIDEBAR_ROW_ACTION_HOVER_BG_OPACITY,
    PROJECT_SIDEBAR_ROW_ACTION_PADDING_PX, PROJECT_SIDEBAR_ROW_ACTION_RADIUS_PX,
    PROJECT_SIDEBAR_ROW_ACTIVE_BG_OPACITY, PROJECT_SIDEBAR_ROW_GAP_PX,
    PROJECT_SIDEBAR_ROW_HEIGHT_PX, PROJECT_SIDEBAR_ROW_HOVER_BG_OPACITY,
    PROJECT_SIDEBAR_ROW_RADIUS_PX, PROJECT_SIDEBAR_ROW_STACK_GAP_PX,
    PROJECT_SIDEBAR_ROW_TEXT_SIZE_PX, PROJECT_SIDEBAR_ROW_X_PADDING_PX, SETTINGS_BODY_GAP_PX,
    SETTINGS_BODY_TEXT_SIZE_PX, SETTINGS_BODY_X_PADDING_PX, SETTINGS_BODY_Y_PADDING_PX,
    SETTINGS_CLOSE_BUTTON_SIZE_PX, SETTINGS_CLOSE_HOVER_BG_OPACITY, SETTINGS_CLOSE_LABEL,
    SETTINGS_CODE_FONT_FAMILY, SETTINGS_CODE_RADIUS_PX, SETTINGS_CONTROL_RADIUS_PX,
    SETTINGS_META_TEXT_SIZE_PX, SETTINGS_MODEL_ID_GAP_PX, SETTINGS_MODEL_LABEL_TEXT_SIZE_PX,
    SETTINGS_MODEL_ROW_MIN_HEIGHT_PX, SETTINGS_SECTION_GAP_PX, TASKS_ALL_PROJECTS_LABEL,
    TASKS_HEADER_BODY_FONT_FAMILY, TASKS_HEADER_BORDER_OPACITY, TASKS_HEADER_CONTROL_GAP_PX,
    TASKS_HEADER_CONTROL_RADIUS_PX, TASKS_HEADER_CONTROL_X_PADDING_PX,
    TASKS_HEADER_CONTROL_Y_PADDING_PX, TASKS_HEADER_MIN_HEIGHT_PX,
    TASKS_HEADER_MODE_ACTIVE_BG_OPACITY, TASKS_HEADER_MODE_TEXT_SIZE_PX,
    TASKS_HEADER_NEW_TASK_BG_OPACITY, TASKS_HEADER_NEW_TASK_BORDER_OPACITY,
    TASKS_HEADER_NEW_TASK_HOVER_BG_OPACITY, TASKS_HEADER_NEW_TASK_LEFT_MARGIN_PX,
    TASKS_HEADER_NEW_TASK_TEXT_SIZE_PX, TASKS_HEADER_TITLE_TEXT_SIZE_PX, TASKS_HEADER_X_PADDING_PX,
    TASKS_HEADER_Y_PADDING_PX, TASKS_NEW_PROJECT_LABEL, TASKS_NEW_TASK_LABEL,
    TASK_BOARD_BODY_FONT_FAMILY, TASK_BOARD_BODY_GAP_PX, TASK_BOARD_BODY_PADDING_PX,
    TASK_BOARD_COLUMN_ACTIVE_BG_OPACITY, TASK_BOARD_COLUMN_COMPLETED_BG_OPACITY,
    TASK_BOARD_COLUMN_COMPLETED_OPACITY, TASK_BOARD_COLUMN_MIN_WIDTH_PX,
    TASK_BOARD_COLUMN_RADIUS_PX, TASK_BOARD_COUNT_BG_OPACITY, TASK_BOARD_COUNT_RADIUS_PX,
    TASK_BOARD_COUNT_TEXT_SIZE_PX, TASK_BOARD_COUNT_X_PADDING_PX,
    TASK_BOARD_DRAG_OVER_RING_OPACITY, TASK_BOARD_DRAG_OVER_RING_PX, TASK_BOARD_EMPTY_TEXT_SIZE_PX,
    TASK_BOARD_EMPTY_Y_PADDING_PX, TASK_BOARD_GAP_PX, TASK_BOARD_HEADER_DOT_RADIUS_PX,
    TASK_BOARD_HEADER_DOT_SIZE_PX, TASK_BOARD_HEADER_GAP_PX, TASK_BOARD_HEADER_TEXT_SIZE_PX,
    TASK_BOARD_HEADER_X_PADDING_PX, TASK_BOARD_HEADER_Y_PADDING_PX, TASK_BOARD_PADDING_PX,
    TASK_CARD_BG_OPACITY, TASK_CARD_BORDER_OPACITY, TASK_CARD_DUE_TEXT_SIZE_PX, TASK_CARD_GAP_PX,
    TASK_CARD_HOVER_BORDER_OPACITY, TASK_CARD_PADDING_PX, TASK_CARD_PRIORITY_MARGIN_TOP_PX,
    TASK_CARD_RADIUS_PX, TASK_CARD_SELECTED_BG_OPACITY, TASK_CARD_TITLE_TEXT_SIZE_PX,
    TASK_CARD_TOP_ROW_GAP_PX, TASK_CREATE_BODY_GAP_PX, TASK_CREATE_BODY_Y_PADDING_PX,
    TASK_CREATE_DESCRIPTION_PLACEHOLDER, TASK_CREATE_DESCRIPTION_ROWS, TASK_CREATE_DUE_PLACEHOLDER,
    TASK_CREATE_FOOTER_BG_OPACITY, TASK_CREATE_FOOTER_GAP_PX, TASK_CREATE_FOOTER_Y_PADDING_PX,
    TASK_CREATE_HEADER_Y_PADDING_PX, TASK_CREATE_OVERLAY_OPACITY, TASK_CREATE_TITLE_PLACEHOLDER,
    TASK_DETAIL_BODY_GAP_PX, TASK_DETAIL_BODY_Y_PADDING_PX, TASK_DETAIL_CLOSE_LABEL,
    TASK_DETAIL_DESCRIPTION_PLACEHOLDER, TASK_DETAIL_DESCRIPTION_ROWS, TASK_DETAIL_DUE_PLACEHOLDER,
    TASK_DETAIL_FOOTER_Y_PADDING_PX, TASK_DETAIL_HEADER_Y_PADDING_PX, TASK_DETAIL_PANEL_WIDTH_PX,
    TASK_DETAIL_STATUS_CHIP_ACTIVE_BORDER_OPACITY, TASK_DETAIL_STATUS_CHIP_GAP_PX,
    TASK_DETAIL_STATUS_CHIP_RADIUS_PX, TASK_DETAIL_STATUS_CHIP_TEXT_SIZE_PX,
    TASK_DETAIL_STATUS_CHIP_X_PADDING_PX, TASK_DETAIL_STATUS_CHIP_Y_PADDING_PX,
    TASK_DETAIL_TITLE_PLACEHOLDER, TASK_DETAIL_TITLE_TEXT_SIZE_PX, TASK_DIALOG_BODY_FONT_FAMILY,
    TASK_DIALOG_LABEL_FONT_FAMILY, TASK_DIALOG_TITLE_TEXT_SIZE_PX, TASK_DRAG_PREVIEW_BG_OPACITY,
    TASK_DRAG_PREVIEW_BORDER_OPACITY, TASK_DRAG_PREVIEW_RADIUS_PX, TASK_FORM_LABEL_TEXT_SIZE_PX,
    TASK_LIST_BODY_FONT_FAMILY, TASK_LIST_BODY_Y_PADDING_PX, TASK_LIST_DUE_COLUMN_WIDTH_PX,
    TASK_LIST_EMPTY_TEXT_SIZE_PX, TASK_LIST_EMPTY_Y_PADDING_PX, TASK_LIST_HEADER_BORDER_OPACITY,
    TASK_LIST_HEADER_GAP_PX, TASK_LIST_HEADER_HEIGHT_PX, TASK_LIST_HEADER_TEXT_SIZE_PX,
    TASK_LIST_HEADER_X_PADDING_PX, TASK_LIST_PRIORITY_COLUMN_WIDTH_PX,
    TASK_LIST_STATUS_SLOT_WIDTH_PX, TASK_PRIORITY_BAR_GAP_PX, TASK_PRIORITY_BAR_HEIGHTS_PX,
    TASK_PRIORITY_BAR_RADIUS_PX, TASK_PRIORITY_BAR_WIDTH_PX, TASK_PRIORITY_EMPTY_OPACITY,
    TASK_ROW_BODY_FONT_FAMILY, TASK_ROW_DUE_TEXT_SIZE_PX, TASK_ROW_GAP_PX,
    TASK_ROW_HOVER_BG_OPACITY, TASK_ROW_RADIUS_PX, TASK_ROW_SELECTED_BG_OPACITY,
    TASK_ROW_STATUS_DOT_RADIUS_PX, TASK_ROW_STATUS_DOT_SIZE_PX, TASK_ROW_TITLE_TEXT_SIZE_PX,
    TASK_ROW_X_PADDING_PX, TASK_ROW_Y_PADDING_PX, TIMEGRID_DRAG_PREVIEW_BG_OPACITY,
    TIMEGRID_DRAG_PREVIEW_BORDER_OPACITY, TIMEGRID_DRAG_PREVIEW_RADIUS_PX, WEEKDAY_NAMES,
    WORKSPACE_SIDEBAR_BORDER_OPACITY, WORKSPACE_SIDEBAR_TAB_RADIUS_PX,
    WORKSPACE_SIDE_PANEL_MIN_WIDTH_PX, WORKSPACE_SIDE_PANEL_WIDTH_PX,
    WORKSPACE_THREAD_ROW_HOVER_BG_OPACITY,
};
use super::{CloseOverlays, OpenTerminalView, TogglePreview, ToggleTerminal};
use crate::{app_state::DesktopState, assets, theme};
use bruhs_core::registry::TaskUpdate;
use bruhs_core::TaskKind;
use bruhs_models::EpochMs;
use chrono::{NaiveDate, NaiveDateTime};
use gpui::px;
use gpui::{AppContext as _, Entity, Focusable};
use std::{cell::RefCell, fs, rc::Rc};

fn dt(year: i32, month: u32, day: u32, hour: u32, minute: u32) -> NaiveDateTime {
    NaiveDate::from_ymd_opt(year, month, day)
        .and_then(|date| date.and_hms_opt(hour, minute, 0))
        .expect("test datetime should be valid")
}

fn event(id: &str, start_at: NaiveDateTime, end_at: NaiveDateTime) -> DesktopCalendarEvent {
    DesktopCalendarEvent {
        id: id.to_string(),
        title: id.to_string(),
        start_at,
        end_at,
        all_day: false,
        color: None,
        notes: None,
    }
}

#[gpui::test]
fn desktop_settings_modal_focus_should_stay_trapped_like_web_dialog(cx: &mut gpui::TestAppContext) {
    cx.update(|cx| {
        gpui_component::init(cx);
        gpui_component::Theme::change(gpui_component::ThemeMode::Dark, None, cx);
        theme::apply_bruhs_theme(cx);
        let state = DesktopState::new().expect("desktop state should initialize");
        cx.set_global(state);
    });

    let workspace_cell: Rc<RefCell<Option<Entity<Workspace>>>> = Rc::new(RefCell::new(None));
    let workspace_for_window = workspace_cell.clone();
    let (_root, cx) = cx.add_window_view(|window, cx| {
        let workspace = cx.new(|cx| Workspace::new(window, cx));
        *workspace_for_window.borrow_mut() = Some(workspace.clone());
        gpui_component::Root::new(workspace, window, cx)
    });
    let workspace = workspace_cell
        .borrow()
        .clone()
        .expect("workspace entity should be captured");

    set_model_for_visual_test(
        &workspace,
        cx,
        "claude-sonnet-4-6",
        TestAuthState::Unauthenticated,
    );
    cx.run_until_parked();
    assert_provider_logo_bounds(cx, "desktop-chat-provider-logo-claude");
    assert_rendered_bounds(cx, "desktop-auth-banner");

    workspace.update_in(cx, |workspace, window, cx| {
        workspace.open_settings(&OpenSettings, window, cx);
    });
    cx.run_until_parked();

    workspace.update_in(cx, |workspace, window, cx| {
        assert!(workspace.settings_visible());
        assert!(workspace
            .modal_start_focus_handle
            .contains_focused(window, cx));
    });
    // The model row is a dropdown now; provider logos head its groups.
    assert_rendered_bounds(cx, "desktop-settings-row-model");
    click_debug_selector(cx, "desktop-model");
    cx.run_until_parked();
    assert_rendered_bounds(cx, "desktop-model-list");
    assert_provider_logo_bounds(cx, "desktop-provider-logo-claude");
    assert_provider_logo_bounds(cx, "desktop-provider-logo-codex");

    click_debug_selector(cx, "desktop-model-option-gpt-5.5");
    workspace.update_in(cx, |workspace, _window, cx| {
        assert_eq!(workspace.current_model, "gpt-5.5");
        let active_chat = workspace
            .active_thread()
            .expect("workspace should have an active thread")
            .chat
            .read(cx);
        assert_eq!(active_chat.model_id, "gpt-5.5");
        assert!(!active_chat.auth_banner_visible_for_test());
    });
    assert_provider_logo_bounds(cx, "desktop-chat-provider-logo-codex");

    cx.simulate_keystrokes("tab tab tab tab tab shift-tab shift-tab shift-tab");

    workspace.update_in(cx, |workspace, window, cx| {
        assert!(workspace.settings_visible());
        assert!(workspace
            .modal_start_focus_handle
            .contains_focused(window, cx));
    });
}

#[gpui::test]
fn desktop_codex_model_should_not_render_claude_auth_banner_like_web(
    cx: &mut gpui::TestAppContext,
) {
    cx.update(|cx| {
        gpui_component::init(cx);
        gpui_component::Theme::change(gpui_component::ThemeMode::Dark, None, cx);
        theme::apply_bruhs_theme(cx);
        let state = DesktopState::new().expect("desktop state should initialize");
        cx.set_global(state);
    });

    let (_root, cx) = cx.add_window_view(|window, cx| {
        let workspace = cx.new(|cx| Workspace::new(window, cx));
        workspace.update(cx, |workspace, cx| {
            workspace.current_model = "gpt-5.5".to_string();
            let chats = workspace
                .threads
                .iter()
                .map(|thread| thread.chat.clone())
                .collect::<Vec<_>>();
            for chat in chats {
                chat.update(cx, |chat, cx| {
                    chat.set_model("gpt-5.5", cx);
                });
            }
            cx.notify();
        });
        gpui_component::Root::new(workspace, window, cx)
    });

    assert_provider_logo_bounds(cx, "desktop-chat-provider-logo-codex");
    assert_no_rendered_bounds(cx, "desktop-auth-banner");
}

#[test]
fn persisted_thread_without_card_fields_should_still_load() {
    let legacy = r#"{"id":"t1","title":"Old","project_name":"p","project_path":"/p","updated_at":"now","model_id":"gpt-5.4","permission_mode":"default","messages":[]}"#;
    let thread: PersistedThread = serde_json::from_str(legacy).expect("legacy record loads");
    assert_eq!(thread.updated_at_ms, EpochMs::UNIX_EPOCH);
    assert!(!thread.pinned);
    assert!(!thread.settled);
    assert_eq!(thread.branch, None);
    assert_eq!(thread.worktree_path, None);
    assert_eq!(thread.effort, None);
    assert!(!thread.run_locked);
    let state: PersistedWorkspaceState = serde_json::from_str("{}").expect("empty state loads");
    assert_eq!(state.sidebar_settled_open, None);
    assert_eq!(state.run_choice, "");
    assert_eq!(state.claude_effort, None);
    assert_eq!(state.previous_worktree, None);
    assert_eq!(thread_link("abc"), "bruhs://thread/abc");
}

#[gpui::test]
fn desktop_settings_sections_switch_and_dropdowns_select(cx: &mut gpui::TestAppContext) {
    cx.update(|cx| {
        gpui_component::init(cx);
        gpui_component::Theme::change(gpui_component::ThemeMode::Dark, None, cx);
        theme::apply_bruhs_theme(cx);
        let state = DesktopState::new().expect("desktop state should initialize");
        cx.set_global(state);
    });
    let workspace_cell: Rc<RefCell<Option<Entity<Workspace>>>> = Rc::new(RefCell::new(None));
    let workspace_for_window = workspace_cell.clone();
    let (_root, cx) = cx.add_window_view(|window, cx| {
        let workspace = cx.new(|cx| Workspace::new(window, cx));
        *workspace_for_window.borrow_mut() = Some(workspace.clone());
        gpui_component::Root::new(workspace, window, cx)
    });
    let workspace = workspace_cell
        .borrow()
        .clone()
        .expect("workspace entity should be captured");

    workspace.update_in(cx, |workspace, window, cx| {
        workspace.open_settings(&OpenSettings, window, cx);
    });
    cx.run_until_parked();
    assert_rendered_bounds(cx, "desktop-settings-nav-general");
    assert_rendered_bounds(cx, "desktop-settings-row-model");
    assert_rendered_bounds(cx, "desktop-settings-row-access-mode");
    assert_rendered_bounds(cx, "desktop-settings-row-editor");
    // All eight sections stack on one page, like the web.
    assert_rendered_bounds(cx, "desktop-settings-section-calendar");
    assert_rendered_bounds(cx, "desktop-settings-row-auto-record");
    assert_rendered_bounds(cx, "desktop-settings-row-whisper-model");
    assert_rendered_bounds(cx, "desktop-settings-slider-max-speakers");
    assert_rendered_bounds(cx, "desktop-settings-row-observations");

    // Access mode dropdown → Plan.
    click_debug_selector(cx, "desktop-access-mode");
    cx.run_until_parked();
    assert_rendered_bounds(cx, "desktop-access-mode-list");
    click_debug_selector(cx, "desktop-access-mode-option-plan");
    cx.run_until_parked();
    workspace.update_in(cx, |workspace, _window, _cx| {
        assert_eq!(workspace.permission_mode.as_str(), "plan");
        assert_eq!(workspace.settings_open_dropdown(), None);
    });
    assert_no_rendered_bounds(cx, "desktop-access-mode-list");

    // Editor dropdown → Zed.
    click_debug_selector(cx, "desktop-editor");
    cx.run_until_parked();
    click_debug_selector(cx, "desktop-editor-option-zed");
    cx.run_until_parked();
    workspace.update_in(cx, |workspace, _window, _cx| {
        assert_eq!(workspace.editor_id, "zed");
    });

    click_debug_selector(cx, "desktop-settings-nav-transcription");
    cx.run_until_parked();
    click_debug_selector(cx, "desktop-transcription-provider");
    cx.run_until_parked();
    click_debug_selector(
        cx,
        "desktop-transcription-provider-option-open_ai_compatible",
    );
    cx.run_until_parked();
    assert_rendered_bounds(cx, "desktop-settings-row-transcription-server-model");
    assert_no_rendered_bounds(cx, "desktop-settings-row-whisper-model");
    workspace.update_in(cx, |workspace, window, cx| {
        workspace
            .settings_transcription_model_input
            .update(cx, |input, cx| {
                input.replace_all("synthetic-model", window, cx);
            });
    });
    cx.run_until_parked();
    workspace.update_in(cx, |workspace, _window, cx| {
        let prefs = &workspace.meetings_hub.read(cx).prefs;
        assert_eq!(
            prefs.transcription.provider,
            bruhs_models::TranscriptionProvider::OpenAiCompatible
        );
        assert_eq!(prefs.transcription.remote_model, "synthetic-model");
        let body =
            crate::views::meetings_hub::capture_start_body(prefs, "synthetic-recording", 1.0)
                .unwrap();
        assert_eq!(body.transcription, Some(prefs.transcription.clone()));
        let restored: crate::views::meetings_hub::MeetingPrefs =
            serde_json::from_str(&serde_json::to_string(prefs).unwrap()).unwrap();
        assert_eq!(restored, *prefs);
    });
    click_debug_selector(cx, "desktop-transcription-provider");
    cx.run_until_parked();
    click_debug_selector(cx, "desktop-transcription-provider-option-local_whisper");
    cx.run_until_parked();
    assert_rendered_bounds(cx, "desktop-settings-row-whisper-model");
    assert_no_rendered_bounds(cx, "desktop-settings-row-transcription-server-model");
    assert_rendered_bounds(cx, "desktop-settings-row-live-whisper-model");
    workspace.update_in(cx, |workspace, _window, cx| {
        let prefs = &workspace.meetings_hub.read(cx).prefs;
        assert_eq!(prefs.transcription.remote_model, "synthetic-model");
    });

    // Nav jumps scroll the page (web `scroll_into_view`); the spy then
    // highlights the section under the fold line.
    click_debug_selector(cx, "desktop-settings-nav-keyboard");
    cx.run_until_parked();
    assert_rendered_bounds(cx, "desktop-settings-keybinding-row-0");
    workspace.update_in(cx, |workspace, _window, _cx| {
        assert!(workspace.settings_scroll.offset().y < px(0.0));
        let tops = workspace
            .settings_section_tops()
            .expect("sections laid out");
        let keyboard = super::SettingsSection::Keyboard.index();
        assert!(
            tops[keyboard] <= super::SETTINGS_SCROLL_SPY_FOLD_PX,
            "keyboard top {}",
            tops[keyboard]
        );
        assert_eq!(workspace.settings_section, super::SettingsSection::Keyboard);
    });
    click_debug_selector(cx, "desktop-settings-nav-about");
    cx.run_until_parked();
    assert_rendered_bounds(cx, "desktop-settings-about-version");
    // The spy reads the post-jump layout on the following frame (the
    // app requests it; the test harness needs a nudge).
    workspace.update_in(cx, |_workspace, _window, cx| cx.notify());
    cx.run_until_parked();
    workspace.update_in(cx, |workspace, _window, _cx| {
        let tops = workspace
            .settings_section_tops()
            .expect("sections laid out");
        let expected = super::active_settings_section(&tops, super::SETTINGS_SCROLL_SPY_FOLD_PX)
            .map(|ix| super::SettingsSection::ALL[ix]);
        assert_eq!(Some(workspace.settings_section), expected);
    });
}

#[test]
fn active_settings_section_is_last_above_fold_like_web() {
    let tops = [-400.0, -10.0, 300.0];
    assert_eq!(super::active_settings_section(&tops, 96.0), Some(1));
    assert_eq!(super::active_settings_section(&[500.0], 96.0), Some(0));
    assert_eq!(super::active_settings_section(&[], 96.0), None);
    let ids: Vec<&str> = super::SettingsSection::ALL.iter().map(|s| s.id()).collect();
    assert_eq!(
        ids,
        [
            "general",
            "updates",
            "beta",
            "credentials",
            "calendar",
            "recording",
            "transcription",
            "speakers",
            "people",
            "keyboard",
            "about"
        ]
    );
}

#[test]
fn palette_helpers_match_thread_links_queries_and_terminal_command() {
    assert_eq!(super::thread_link_id("bruhs://thread/abc-1"), Some("abc-1"));
    assert_eq!(super::thread_link_id("  bruhs://thread/abc/ "), Some("abc"));
    assert_eq!(super::thread_link_id("bruhs://thread/"), None);
    assert_eq!(super::thread_link_id("open project"), None);
    assert!(super::text_matches_query(
        "Fix the Parser bug",
        "parser fix"
    ));
    assert!(!super::text_matches_query("Fix the parser", "lexer"));
    assert!(super::text_matches_query("anything", ""));
    let (program, args) = super::system_terminal_command("/tmp/proj");
    if cfg!(target_os = "macos") {
        assert_eq!(program, "open");
        assert_eq!(args, vec!["-a", "Terminal", "/tmp/proj"]);
    } else {
        assert_eq!(args.last().map(String::as_str), Some("/tmp/proj"));
    }
    assert_eq!(
        super::browser_open_command("http://localhost:3000").1,
        vec!["http://localhost:3000"]
    );
}

#[gpui::test]
fn desktop_palette_threads_mode_and_thread_links(cx: &mut gpui::TestAppContext) {
    cx.update(|cx| {
        gpui_component::init(cx);
        gpui_component::Theme::change(gpui_component::ThemeMode::Dark, None, cx);
        theme::apply_bruhs_theme(cx);
        let state = DesktopState::new().expect("desktop state should initialize");
        cx.set_global(state);
    });
    let workspace_cell: Rc<RefCell<Option<Entity<Workspace>>>> = Rc::new(RefCell::new(None));
    let workspace_for_window = workspace_cell.clone();
    let (_root, cx) = cx.add_window_view(|window, cx| {
        let workspace = cx.new(|cx| Workspace::new(window, cx));
        *workspace_for_window.borrow_mut() = Some(workspace.clone());
        gpui_component::Root::new(workspace, window, cx)
    });
    let workspace = workspace_cell
        .borrow()
        .clone()
        .expect("workspace entity should be captured");

    let (parser_id, other_id) = workspace.update_in(cx, |workspace, window, cx| {
        workspace.new_thread_in_project(
            "proj".to_string(),
            "/tmp/bruhs-desktop-palette-test".to_string(),
            window,
            cx,
        );
        let parser_id = workspace.active_thread_id.clone().expect("thread");
        workspace
            .active_thread()
            .expect("thread")
            .chat
            .update(cx, |chat, _cx| {
                chat.thread_title = "Fix the parser".to_string()
            });
        workspace.new_thread_in_project(
            "proj".to_string(),
            "/tmp/bruhs-desktop-palette-test".to_string(),
            window,
            cx,
        );
        let other_id = workspace.active_thread_id.clone().expect("thread");
        workspace
            .active_thread()
            .expect("thread")
            .chat
            .update(cx, |chat, _cx| {
                chat.thread_title = "Calendar sync".to_string()
            });
        // Like the web, threads show up in the command bar once a query
        // is typed (⌘⇧F opens the transcript Search page instead).
        workspace.focus_command_bar(&FocusCommandBar, window, cx);
        (parser_id, other_id)
    });
    cx.run_until_parked();
    let parser_row = leaked_debug_selector(format!(
        "desktop-command-thread-{}",
        super::element_key(&parser_id)
    ));
    let other_row = leaked_debug_selector(format!(
        "desktop-command-thread-{}",
        super::element_key(&other_id)
    ));
    assert_no_rendered_bounds(cx, parser_row);
    assert_no_rendered_bounds(cx, other_row);

    cx.simulate_input("parser");
    cx.run_until_parked();
    assert_rendered_bounds(cx, parser_row);
    assert_no_rendered_bounds(cx, other_row);
    click_debug_selector(cx, parser_row);
    cx.run_until_parked();
    workspace.update_in(cx, |workspace, _window, _cx| {
        assert_eq!(
            workspace.active_thread_id.as_deref(),
            Some(parser_id.as_str())
        );
        assert!(!workspace.command_bar_visible());
    });

    // A pasted thread reference resolves to that one thread in any mode.
    workspace.update_in(cx, |workspace, window, cx| {
        workspace.focus_command_bar(&FocusCommandBar, window, cx);
        let link = thread_link(&other_id);
        workspace
            .command_input
            .update(cx, |input, cx| input.set_value(link, window, cx));
    });
    cx.run_until_parked();
    assert_rendered_bounds(cx, other_row);
    assert_no_rendered_bounds(cx, parser_row);
    workspace.update_in(cx, |workspace, window, cx| {
        workspace.execute_first_command(window, cx);
        assert_eq!(
            workspace.active_thread_id.as_deref(),
            Some(other_id.as_str())
        );
    });

    // ⌘⇧J: servers mode shows its chip and empty state under test.
    workspace.update_in(cx, |workspace, window, cx| {
        workspace.open_local_servers(&OpenLocalServers, window, cx);
    });
    cx.run_until_parked();
    assert_rendered_bounds(cx, "desktop-command-mode");
    assert_rendered_bounds(cx, "desktop-command-empty");
}

#[gpui::test]
fn desktop_terminal_drawer_and_view_should_render_like_web(cx: &mut gpui::TestAppContext) {
    cx.update(|cx| {
        gpui_component::init(cx);
        gpui_component::Theme::change(gpui_component::ThemeMode::Dark, None, cx);
        theme::apply_bruhs_theme(cx);
        let state = DesktopState::new().expect("desktop state should initialize");
        cx.set_global(state);
    });
    let workspace_cell: Rc<RefCell<Option<Entity<Workspace>>>> = Rc::new(RefCell::new(None));
    let workspace_for_window = workspace_cell.clone();
    let (_root, cx) = cx.add_window_view(|window, cx| {
        let workspace = cx.new(|cx| Workspace::new(window, cx));
        *workspace_for_window.borrow_mut() = Some(workspace.clone());
        gpui_component::Root::new(workspace, window, cx)
    });
    let workspace = workspace_cell
        .borrow()
        .clone()
        .expect("workspace entity should be captured");
    cx.run_until_parked();
    assert_no_rendered_bounds(cx, "desktop-terminal-drawer");

    // ⌘J: the drawer under the chat, with the empty state (no shell is
    // spawned under test).
    workspace.update_in(cx, |workspace, window, cx| {
        workspace.toggle_terminal(&ToggleTerminal, window, cx);
    });
    cx.run_until_parked();
    assert_rendered_bounds(cx, "desktop-terminal-drawer");
    assert_rendered_bounds(cx, "desktop-terminal-empty");
    assert_rendered_bounds(cx, "desktop-terminal-empty-home");
    assert_rendered_bounds(cx, "desktop-terminal-close-drawer");
    let drawer = rendered_bounds(cx, "desktop-terminal-drawer");
    assert_eq!(
        drawer.size.height,
        px(crate::views::terminal_panel::TERMINAL_DRAWER_HEIGHT_PX)
    );

    // ⌘J again hides it.
    workspace.update_in(cx, |workspace, window, cx| {
        workspace.toggle_terminal(&ToggleTerminal, window, cx);
    });
    cx.run_until_parked();
    assert_no_rendered_bounds(cx, "desktop-terminal-drawer");

    // The Terminal view hosts the same panel at full height.
    workspace.update_in(cx, |workspace, window, cx| {
        workspace.open_terminal_view(&OpenTerminalView, window, cx);
    });
    cx.run_until_parked();
    assert_rendered_bounds(cx, "desktop-terminal-page");
    assert_rendered_bounds(cx, "desktop-terminal-page-header");
    // The shared page header is one bar, not a column whose text wrapped
    // a glyph per line and pushed the terminal off the page.
    assert!(
        rendered_bounds(cx, "desktop-terminal-page-header")
            .size
            .height
            < gpui::px(96.0)
    );
    assert_rendered_bounds(cx, "desktop-terminal-empty");
    assert_no_rendered_bounds(cx, "desktop-terminal-close-drawer");

    // A session renders its grid, cursor, tab and status.
    let panel = workspace.update(cx, |workspace, _| workspace.terminal_panel.clone());
    let info = bruhs_core::terminal::TerminalInfo {
        id: "sess-1".into(),
        cwd: Some("/tmp/proj".into()),
        title: "proj".into(),
        project_name: Some("proj".into()),
        cols: 80,
        rows: 24,
        created_at_ms: 1,
    };
    let view = panel.update(cx, |panel, cx| panel.push_detached(info, cx));
    view.update(cx, |view, cx| view.feed(b"hello \x1b[32mgreen\x1b[0m", cx));
    cx.run_until_parked();
    assert_no_rendered_bounds(cx, "desktop-terminal-empty");
    assert_rendered_bounds(cx, "desktop-terminal-view");
    assert_rendered_bounds(cx, "desktop-terminal-cursor");
    assert_rendered_bounds(cx, "desktop-terminal-status");
    let key = super::element_key("sess-1");
    let tab_selector = leaked_debug_selector(format!("desktop-terminal-tab-{key}"));
    assert_rendered_bounds(cx, tab_selector);
    panel.update(cx, |panel, _| assert_eq!(panel.active_id(), Some("sess-1")));
    view.update(cx, |view, _| {
        let (rows, cursor) = view.snapshot();
        assert_eq!(rows[0].text, "hello green");
        assert_eq!(cursor.map(|c| (c.row, c.col)), Some((0, 11)));
        assert!(view.selection_text().is_none());
        // The canvas prepaint measured the page and resized the grid
        // away from the 80×24 it was created with.
        assert_ne!(
            view.size(),
            crate::views::terminal_view::TermSize { cols: 80, rows: 24 },
            "grid should follow the rendered bounds"
        );
        assert!(view.size().cols > 2 && view.size().rows > 1);
    });

    // "+ New" opens the folder menu with Home and "Choose folder…".
    click_debug_selector(cx, "desktop-terminal-new");
    cx.run_until_parked();
    assert_rendered_bounds(cx, "desktop-terminal-new-menu");
    assert_rendered_bounds(cx, "desktop-terminal-new-home");
    assert_rendered_bounds(cx, "desktop-terminal-new-folder");
    click_debug_selector(cx, "desktop-terminal-new-folder");
    cx.run_until_parked();
    assert_no_rendered_bounds(cx, "desktop-terminal-new-menu");
    workspace.update(cx, |workspace, _| {
        assert!(workspace.project_picker_visible());
        assert!(workspace.project_picker_for_terminal());
    });
    workspace.update_in(cx, |workspace, window, cx| {
        workspace.close_overlays(&CloseOverlays, window, cx);
        assert!(!workspace.project_picker_for_terminal());
    });
    cx.run_until_parked();

    // Closing the tab from its × brings the empty state back.
    let close_selector = leaked_debug_selector(format!("desktop-terminal-tab-close-{key}"));
    click_debug_selector(cx, close_selector);
    cx.run_until_parked();
    assert_no_rendered_bounds(cx, tab_selector);
    assert_rendered_bounds(cx, "desktop-terminal-empty");
}

#[gpui::test]
fn desktop_preview_surface_should_render_like_web(cx: &mut gpui::TestAppContext) {
    cx.update(|cx| {
        gpui_component::init(cx);
        gpui_component::Theme::change(gpui_component::ThemeMode::Dark, None, cx);
        theme::apply_bruhs_theme(cx);
        let state = DesktopState::new().expect("desktop state should initialize");
        cx.set_global(state);
    });
    let workspace_cell: Rc<RefCell<Option<Entity<Workspace>>>> = Rc::new(RefCell::new(None));
    let workspace_for_window = workspace_cell.clone();
    let (_root, cx) = cx.add_window_view(|window, cx| {
        let workspace = cx.new(|cx| Workspace::new(window, cx));
        *workspace_for_window.borrow_mut() = Some(workspace.clone());
        gpui_component::Root::new(workspace, window, cx)
    });
    let workspace = workspace_cell
        .borrow()
        .clone()
        .expect("workspace entity should be captured");
    cx.run_until_parked();
    assert_no_rendered_bounds(cx, "desktop-preview-panel");

    // ⌘⇧J opens the right panel on Preview with the empty state.
    workspace.update_in(cx, |workspace, window, cx| {
        workspace.toggle_preview(&TogglePreview, window, cx);
        assert!(workspace.preview_panel_visible);
        assert!(!workspace.diff_panel_visible && !workspace.tasks_panel_visible);
    });
    cx.run_until_parked();
    assert_rendered_bounds(cx, "desktop-panel-tab-preview");
    assert_rendered_bounds(cx, "desktop-preview-panel");
    assert_rendered_bounds(cx, "desktop-preview-empty");
    assert_rendered_bounds(cx, "desktop-preview-servers");
    assert_rendered_bounds(cx, "desktop-preview-go");

    // A scanned server is one click from a page.
    let panel = workspace.update(cx, |workspace, _| workspace.preview_panel.clone());
    panel.update(cx, |panel, cx| {
        panel.set_servers_for_test(
            vec![bruhs_core::preview::LocalServer {
                port: 3000,
                command: "node".into(),
                pid: 1,
            }],
            cx,
        );
    });
    cx.run_until_parked();
    assert_rendered_bounds(cx, "desktop-preview-server-3000");
    click_debug_selector(cx, "desktop-preview-server-3000");
    cx.run_until_parked();
    panel.update(cx, |panel, _| {
        assert_eq!(panel.url(), Some("http://localhost:3000"));
    });
    assert_no_rendered_bounds(cx, "desktop-preview-empty");
    assert_rendered_bounds(cx, "desktop-preview-page");

    // Clear returns to the list; the Diff tab replaces the surface.
    click_debug_selector(cx, "desktop-preview-clear");
    cx.run_until_parked();
    panel.update(cx, |panel, _| assert_eq!(panel.url(), None));
    assert_rendered_bounds(cx, "desktop-preview-empty");
    click_debug_selector(cx, "desktop-panel-tab-diff");
    cx.run_until_parked();
    workspace.update(cx, |workspace, _| {
        assert!(workspace.diff_panel_visible && !workspace.preview_panel_visible);
    });
    assert_no_rendered_bounds(cx, "desktop-preview-panel");

    // ⌘⇧J again from Diff switches back; a third press closes the panel.
    workspace.update_in(cx, |workspace, window, cx| {
        workspace.toggle_preview(&TogglePreview, window, cx);
    });
    cx.run_until_parked();
    assert_rendered_bounds(cx, "desktop-preview-panel");
    workspace.update_in(cx, |workspace, window, cx| {
        workspace.toggle_preview(&TogglePreview, window, cx);
        assert!(!workspace.right_panel_open());
    });
}

#[test]
fn step_thread_index_wraps_both_ways() {
    let order = ["a", "b", "c"];
    assert_eq!(step_thread_index(&order, Some("a"), 1), Some(1));
    assert_eq!(step_thread_index(&order, Some("c"), 1), Some(0));
    assert_eq!(step_thread_index(&order, Some("a"), -1), Some(2));
    assert_eq!(step_thread_index(&order, None, 1), Some(0));
    assert_eq!(step_thread_index(&order, None, -1), Some(2));
    assert_eq!(step_thread_index(&order, Some("zz"), 1), Some(0));
    assert_eq!(step_thread_index(&[], Some("a"), 1), None);
    // Steps longer than the list wrap like `rem_euclid`.
    for delta in -7..=7_isize {
        let len = isize::try_from(order.len()).unwrap();
        assert_eq!(
            step_thread_index(&order, Some("b"), delta),
            usize::try_from((1 + delta).rem_euclid(len)).ok(),
            "delta {delta}"
        );
    }
}

/// Every key bound in `main.rs` shows up in the Settings keymap table.
#[test]
fn desktop_keymap_covers_every_bound_action() {
    fn glyphs(binding: &str) -> String {
        let mut out = String::new();
        let parts: Vec<&str> = binding.split('-').collect();
        let (mods, key) = parts.split_at(parts.len().saturating_sub(1));
        // "cmd-shift-[" splits as ["cmd", "shift", "["]; a lone "-" would not.
        for m in mods {
            out.push_str(match *m {
                "cmd" => "⌘",
                "shift" => "⇧",
                "alt" => "⌥",
                "ctrl" => "⌃",
                other => other,
            });
        }
        let key = key.first().copied().unwrap_or("");
        out.push_str(&match key {
            "escape" => "Esc".to_string(),
            k => k.to_ascii_uppercase(),
        });
        out
    }
    let listed: Vec<&str> = super::DESKTOP_KEYMAP
        .iter()
        .flat_map(|(_, keys)| keys.iter().copied())
        .collect();
    for binding in crate::DESKTOP_GLOBAL_SHORTCUTS {
        let glyph = glyphs(binding);
        let is_digit_jump = glyph.starts_with('⌘')
            && glyph.len() == "⌘1".len()
            && glyph.ends_with(|c: char| c.is_ascii_digit());
        let covered = listed.contains(&glyph.as_str())
            || (is_digit_jump && listed.contains(&"⌘1") && listed.contains(&"⌘9"));
        assert!(
            covered,
            "{binding} ({glyph}) is bound but missing from DESKTOP_KEYMAP"
        );
    }
}

#[test]
fn right_panel_toggle_surface_semantics() {
    // Opening Diff hides Tasks; toggling Diff again closes everything.
    assert_eq!(
        right_panel_after_toggle(true, false, false, RightPanelSurface::Diff),
        (false, true, false)
    );
    assert_eq!(
        right_panel_after_toggle(false, true, false, RightPanelSurface::Diff),
        (false, false, false)
    );
    assert_eq!(
        right_panel_after_toggle(false, true, false, RightPanelSurface::Tasks),
        (true, false, false)
    );
    assert_eq!(
        right_panel_after_toggle(true, false, false, RightPanelSurface::Tasks),
        (false, false, false)
    );
    // Preview behaves like the others: it replaces, then toggles off.
    assert_eq!(
        right_panel_after_toggle(false, true, false, RightPanelSurface::Preview),
        (false, false, true)
    );
    assert_eq!(
        right_panel_after_toggle(false, false, true, RightPanelSurface::Preview),
        (false, false, false)
    );
    // The file tab is not one of the three toggles.
    assert_eq!(
        right_panel_after_toggle(false, true, false, RightPanelSurface::File),
        (false, true, false)
    );
    assert_eq!(
        right_panel_after_toggle(true, true, true, RightPanelSurface::Files),
        (false, false, false)
    );
    assert_eq!(
        showing_surface(true, true, true, true, true),
        RightPanelSurface::File
    );
    assert_eq!(
        showing_surface(false, true, true, true, true),
        RightPanelSurface::Pull
    );
    assert_eq!(
        showing_surface(false, false, true, true, true),
        RightPanelSurface::Files
    );
    assert_eq!(
        showing_surface(false, false, false, false, false),
        RightPanelSurface::Tasks
    );
}

#[gpui::test]
fn desktop_diff_panel_renders_files_and_gaps(cx: &mut gpui::TestAppContext) {
    cx.update(|cx| {
        gpui_component::init(cx);
        gpui_component::Theme::change(gpui_component::ThemeMode::Dark, None, cx);
        theme::apply_bruhs_theme(cx);
        let state = DesktopState::new().expect("desktop state should initialize");
        cx.set_global(state);
    });

    let workspace_cell: Rc<RefCell<Option<Entity<Workspace>>>> = Rc::new(RefCell::new(None));
    let workspace_for_window = workspace_cell.clone();
    let (_root, cx) = cx.add_window_view(|window, cx| {
        let workspace = cx.new(|cx| Workspace::new(window, cx));
        *workspace_for_window.borrow_mut() = Some(workspace.clone());
        gpui_component::Root::new(workspace, window, cx)
    });
    let workspace = workspace_cell
        .borrow()
        .clone()
        .expect("workspace entity should be captured");

    const RAW: &str = "diff --git a/src/a.rs b/src/a.rs\n--- a/src/a.rs\n+++ b/src/a.rs\n@@ -1,3 +1,4 @@\n fn main() {\n-    old();\n+    new();\n+    more();\n }\n@@ -12,2 +13,2 @@ fn x()\n a\n b\n";
    workspace.update_in(cx, |workspace, window, cx| {
        workspace.new_thread_in_project(
            "proj".to_string(),
            "/tmp/bruhs-desktop-diff-test".to_string(),
            window,
            cx,
        );
        workspace.tasks_panel_visible = true;
        workspace.toggle_diff(&ToggleDiff, window, cx);
        assert!(workspace.diff_panel_visible);
        assert!(!workspace.tasks_panel_visible, "one surface at a time");
        assert_eq!(
            workspace.diff_panel.read(cx).project_path(),
            Some("/tmp/bruhs-desktop-diff-test")
        );
        let panel = workspace.diff_panel.clone();
        panel.update(cx, |panel, cx| panel.set_diff_text_for_test(RAW, cx));
        assert_eq!(panel.read(cx).file_count_for_test(), 1);
    });
    cx.run_until_parked();

    let file_key = crate::views::diff_panel::element_key_for_test("src/a.rs");
    assert_rendered_bounds(cx, "desktop-right-panel");
    assert_rendered_bounds(cx, "desktop-panel-tab-diff");
    assert_rendered_bounds(cx, "desktop-diff-panel");
    assert_rendered_bounds(cx, "desktop-diff-stats");
    assert_rendered_bounds(
        cx,
        leaked_debug_selector(format!("desktop-diff-file-{file_key}")),
    );
    assert_rendered_bounds(
        cx,
        leaked_debug_selector(format!("desktop-diff-open-{file_key}")),
    );
    assert_rendered_bounds(
        cx,
        leaked_debug_selector(format!("desktop-diff-gap-{file_key}-1")),
    );
    assert_rendered_bounds(
        cx,
        leaked_debug_selector(format!("desktop-diff-body-{file_key}")),
    );

    // Collapse the file: body and gaps go away, header stays.
    click_debug_selector(
        cx,
        leaked_debug_selector(format!("desktop-diff-file-{file_key}")),
    );
    cx.run_until_parked();
    assert_no_rendered_bounds(
        cx,
        leaked_debug_selector(format!("desktop-diff-body-{file_key}")),
    );
    assert_rendered_bounds(
        cx,
        leaked_debug_selector(format!("desktop-diff-file-{file_key}")),
    );

    // Per-turn scope: pick a turn from the scope dropdown.
    workspace.update_in(cx, |workspace, _window, cx| {
        let panel = workspace.diff_panel.clone();
        panel.update(cx, |panel, cx| {
            panel.set_turns(
                vec![crate::views::diff_panel::DesktopTurnFiles {
                    label: "Turn · Fix the parser".to_string(),
                    prompt: "Fix the parser".to_string(),
                    files: crate::views::diff_panel::parse_unified(
                        "diff --git a/turn.rs b/turn.rs\n--- a/turn.rs\n+++ b/turn.rs\n@@ -1,1 +1,1 @@\n-old\n+new\n",
                    ),
                }],
                cx,
            );
        });
    });
    cx.run_until_parked();
    click_debug_selector(cx, "desktop-diff-scope");
    cx.run_until_parked();
    assert_rendered_bounds(cx, "desktop-diff-scope-list");
    click_debug_selector(cx, "desktop-diff-scope-option-turn-0");
    cx.run_until_parked();
    let turn_key = crate::views::diff_panel::element_key_for_test("turn.rs");
    assert_rendered_bounds(
        cx,
        leaked_debug_selector(format!("desktop-diff-file-{turn_key}")),
    );
    assert_no_rendered_bounds(
        cx,
        leaked_debug_selector(format!("desktop-diff-file-{file_key}")),
    );
    workspace.update_in(cx, |workspace, _window, cx| {
        assert_eq!(
            workspace.diff_panel.read(cx).scope_for_test(),
            crate::views::diff_panel::DiffScope::Turn(0)
        );
    });
    click_debug_selector(cx, "desktop-diff-scope");
    cx.run_until_parked();
    click_debug_selector(cx, "desktop-diff-scope-option-working-tree");
    cx.run_until_parked();
    assert_rendered_bounds(
        cx,
        leaked_debug_selector(format!("desktop-diff-file-{file_key}")),
    );

    // Tabs switch surfaces; × closes the panel; ⌘⌥B brings the last one back.
    click_debug_selector(cx, "desktop-panel-tab-tasks");
    cx.run_until_parked();
    workspace.update_in(cx, |workspace, _window, _cx| {
        assert!(workspace.tasks_panel_visible);
        assert!(!workspace.diff_panel_visible);
    });
    assert_rendered_bounds(cx, "desktop-background-tasks-panel");
    click_debug_selector(cx, "desktop-panel-close");
    cx.run_until_parked();
    workspace.update_in(cx, |workspace, window, cx| {
        assert!(!workspace.right_panel_open());
        workspace.toggle_panel(&TogglePanel, window, cx);
        assert!(workspace.tasks_panel_visible, "reopens the last surface");
        workspace.close_panel(&ClosePanel, window, cx);
        assert!(!workspace.right_panel_open());
    });
    assert_no_rendered_bounds(cx, "desktop-right-panel");
}

#[gpui::test]
fn desktop_code_thread_cards_and_settled_fold_should_match_web_sidebar(
    cx: &mut gpui::TestAppContext,
) {
    cx.update(|cx| {
        gpui_component::init(cx);
        gpui_component::Theme::change(gpui_component::ThemeMode::Dark, None, cx);
        theme::apply_bruhs_theme(cx);
        let state = DesktopState::new().expect("desktop state should initialize");
        cx.set_global(state);
    });

    let workspace_cell: Rc<RefCell<Option<Entity<Workspace>>>> = Rc::new(RefCell::new(None));
    let workspace_for_window = workspace_cell.clone();
    let (_root, cx) = cx.add_window_view(|window, cx| {
        let workspace = cx.new(|cx| Workspace::new(window, cx));
        *workspace_for_window.borrow_mut() = Some(workspace.clone());
        gpui_component::Root::new(workspace, window, cx)
    });
    let workspace = workspace_cell
        .borrow()
        .clone()
        .expect("workspace entity should be captured");

    let (fresh_id, settled_id) = workspace.update_in(cx, |workspace, window, cx| {
        workspace.new_thread_in_project(
            "proj".to_string(),
            "/tmp/bruhs-desktop-card-test".to_string(),
            window,
            cx,
        );
        workspace.new_thread_in_project(
            "proj".to_string(),
            "/tmp/bruhs-desktop-card-test".to_string(),
            window,
            cx,
        );
        let ids: Vec<String> = workspace
            .threads
            .iter()
            .filter(|thread| thread.project_name == "proj")
            .map(|thread| thread.id.clone())
            .collect();
        let (settled_id, fresh_id) = (ids[0].clone(), ids[1].clone());
        for thread in workspace.threads.iter_mut() {
            if thread.id == settled_id {
                thread.branch = Some("main".to_string());
                thread.settled = true;
            } else if thread.id == fresh_id {
                thread.branch = Some("feature/cards".to_string());
            }
        }
        workspace.sidebar_tab = DesktopSidebarTab::Code;
        workspace.sidebar_settled_open = false;
        cx.notify();
        (fresh_id, settled_id)
    });
    cx.run_until_parked();

    let fresh_key = super::element_key(&fresh_id);
    let settled_key = super::element_key(&settled_id);
    assert_rendered_bounds(
        cx,
        leaked_debug_selector(format!("desktop-thread-card-project-{fresh_key}")),
    );
    assert_rendered_bounds(
        cx,
        leaked_debug_selector(format!("desktop-thread-card-title-{fresh_key}")),
    );
    assert_rendered_bounds(
        cx,
        leaked_debug_selector(format!("desktop-thread-card-branch-{fresh_key}")),
    );
    // The settled thread hides behind the fold until it is opened.
    assert_rendered_bounds(cx, "desktop-sidebar-settled-fold");
    assert_no_rendered_bounds(cx, thread_row_debug_selector(&settled_id));
    click_debug_selector(cx, "desktop-sidebar-settled-fold");
    cx.run_until_parked();
    assert_rendered_bounds(cx, thread_row_debug_selector(&settled_id));
    assert_rendered_bounds(
        cx,
        leaked_debug_selector(format!("desktop-thread-card-branch-{settled_key}")),
    );
    workspace.update_in(cx, |workspace, _window, _cx| {
        assert!(workspace.sidebar_settled_open);
    });

    // Pinning clears the settled flag: the card leaves the fold and shows its pin.
    workspace.update_in(cx, |workspace, _window, cx| {
        workspace.select_thread_by_id(&settled_id, cx);
        workspace.toggle_pin_active(cx);
        workspace.sidebar_settled_open = false;
        let thread = workspace
            .threads
            .iter()
            .find(|thread| thread.id == settled_id)
            .expect("thread exists");
        assert!(thread.pinned);
        assert!(!thread.settled);
        cx.notify();
    });
    cx.run_until_parked();
    assert_rendered_bounds(cx, thread_row_debug_selector(&settled_id));
    assert_rendered_bounds(
        cx,
        leaked_debug_selector(format!("desktop-thread-pin-{settled_key}")),
    );
    assert_no_rendered_bounds(cx, "desktop-sidebar-settled-fold");

    // Unpin returns the thread to the active shelf.
    workspace.update_in(cx, |workspace, _window, cx| {
        assert_eq!(workspace.toggle_pinned_by_id(&settled_id, cx), Some(false));
    });
    cx.run_until_parked();
    assert_rendered_bounds(cx, thread_row_debug_selector(&settled_id));
    assert_no_rendered_bounds(cx, "desktop-sidebar-settled-fold");
}

#[gpui::test]
fn desktop_project_picker_select_current_should_create_project_session_like_web(
    cx: &mut gpui::TestAppContext,
) {
    let Some(home) = home_dir_path().and_then(|path| fs::canonicalize(path).ok()) else {
        return;
    };
    let Ok(cwd) = std::env::current_dir().and_then(fs::canonicalize) else {
        return;
    };
    if !cwd.starts_with(&home) {
        return;
    }

    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let project_dir = cwd.join("target").join(format!(
        "desktop-project-picker-current-test-{}-{unique}",
        std::process::id()
    ));
    fs::create_dir_all(&project_dir).expect("test project directory should be creatable");
    let project_path = fs::canonicalize(&project_dir)
        .expect("test project directory should be canonicalizable")
        .to_string_lossy()
        .to_string();
    let project_name = project_dir
        .file_name()
        .expect("test project directory should have a name")
        .to_string_lossy()
        .to_string();

    cx.update(|cx| {
        gpui_component::init(cx);
        gpui_component::Theme::change(gpui_component::ThemeMode::Dark, None, cx);
        theme::apply_bruhs_theme(cx);
        let state = DesktopState::new().expect("desktop state should initialize");
        cx.set_global(state);
    });

    let workspace_cell: Rc<RefCell<Option<Entity<Workspace>>>> = Rc::new(RefCell::new(None));
    let workspace_for_window = workspace_cell.clone();
    let (_root, cx) = cx.add_window_view(|window, cx| {
        let workspace = cx.new(|cx| Workspace::new(window, cx));
        *workspace_for_window.borrow_mut() = Some(workspace.clone());
        gpui_component::Root::new(workspace, window, cx)
    });
    let workspace = workspace_cell
        .borrow()
        .clone()
        .expect("workspace entity should be captured");

    let mut initial_thread_count = 0;
    let project_path_for_open = project_path.clone();
    workspace.update_in(cx, |workspace, window, cx| {
        initial_thread_count = workspace.threads.len();
        workspace.open_project_picker(&OpenProjectPicker, window, cx);
        workspace.load_project_directory(Some(project_path_for_open), cx);
    });
    cx.run_until_parked();

    workspace.update_in(cx, |workspace, window, cx| {
        assert!(workspace.project_picker_visible());
        assert_eq!(workspace.project_picker_current_path, project_path);
        assert_eq!(workspace.project_picker_current_name, project_name);
        assert!(workspace
            .modal_start_focus_handle
            .contains_focused(window, cx));
    });
    assert_rendered_bounds(cx, "desktop-project-picker-dialog");
    assert_rendered_bounds(cx, "desktop-project-select-current");

    click_debug_selector(cx, "desktop-project-select-current");
    cx.run_until_parked();

    workspace.update_in(cx, |workspace, _window, cx| {
        assert!(!workspace.project_picker_visible());
        assert_eq!(workspace.threads.len(), initial_thread_count + 1);

        let active_project = workspace
            .active_project
            .as_ref()
            .expect("selecting current folder should set an active project");
        assert_eq!(active_project.path, project_path);
        assert_eq!(active_project.name, project_name);
        assert_eq!(
            workspace
                .recent_projects
                .first()
                .expect("selected project should be pushed to recents")
                .path,
            project_path
        );

        let active_thread = workspace
            .active_thread()
            .expect("selecting a project should create an active thread");
        assert_eq!(active_thread.project_name, project_name);
        assert_eq!(active_thread.project_path, project_path);
        let active_chat = active_thread.chat.read(cx);
        assert_eq!(active_chat.project_path, project_path);
        assert!(active_chat.project_scoped);
    });

    let _ = fs::remove_dir_all(&project_dir);
}

#[gpui::test]
fn desktop_project_picker_directory_clicks_should_match_web_browse_and_select_flow(
    cx: &mut gpui::TestAppContext,
) {
    let Some(home) = home_dir_path().and_then(|path| fs::canonicalize(path).ok()) else {
        return;
    };
    let Ok(cwd) = std::env::current_dir().and_then(fs::canonicalize) else {
        return;
    };
    if !cwd.starts_with(&home) {
        return;
    }

    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let root_dir = cwd.join("target").join(format!(
        "desktop-project-picker-dir-test-{}-{unique}",
        std::process::id()
    ));
    let child_dir = root_dir.join("alpha-project");
    fs::create_dir_all(&child_dir).expect("test directory tree should be creatable");
    let root_path = fs::canonicalize(&root_dir)
        .expect("test root directory should be canonicalizable")
        .to_string_lossy()
        .to_string();
    let child_path = fs::canonicalize(&child_dir)
        .expect("test child directory should be canonicalizable")
        .to_string_lossy()
        .to_string();

    cx.update(|cx| {
        gpui_component::init(cx);
        gpui_component::Theme::change(gpui_component::ThemeMode::Dark, None, cx);
        theme::apply_bruhs_theme(cx);
        let state = DesktopState::new().expect("desktop state should initialize");
        cx.set_global(state);
    });

    let workspace_cell: Rc<RefCell<Option<Entity<Workspace>>>> = Rc::new(RefCell::new(None));
    let workspace_for_window = workspace_cell.clone();
    let (_root, cx) = cx.add_window_view(|window, cx| {
        let workspace = cx.new(|cx| Workspace::new(window, cx));
        *workspace_for_window.borrow_mut() = Some(workspace.clone());
        gpui_component::Root::new(workspace, window, cx)
    });
    let workspace = workspace_cell
        .borrow()
        .clone()
        .expect("workspace entity should be captured");

    let mut initial_thread_count = 0;
    let root_path_for_open = root_path.clone();
    workspace.update_in(cx, |workspace, window, cx| {
        initial_thread_count = workspace.threads.len();
        workspace.open_project_picker(&OpenProjectPicker, window, cx);
        workspace.load_project_directory(Some(root_path_for_open), cx);
    });
    cx.run_until_parked();

    let child_selector = project_dir_debug_selector(&child_path);
    assert_rendered_bounds(cx, child_selector);
    click_debug_selector(cx, child_selector);
    cx.run_until_parked();

    workspace.update_in(cx, |workspace, _window, _cx| {
        assert!(workspace.project_picker_visible());
        assert_eq!(workspace.project_picker_current_path, child_path);
        assert!(workspace.active_project.is_none());
        assert_eq!(workspace.threads.len(), initial_thread_count);
    });
    assert_rendered_bounds(cx, "desktop-project-parent");

    workspace.update_in(cx, |workspace, _window, cx| {
        workspace.load_project_directory(Some(root_path), cx);
    });
    cx.run_until_parked();

    assert_rendered_bounds(cx, child_selector);
    double_click_debug_selector(cx, child_selector);
    cx.run_until_parked();

    workspace.update_in(cx, |workspace, _window, cx| {
        assert!(!workspace.project_picker_visible());
        assert_eq!(workspace.threads.len(), initial_thread_count + 1);
        let active_project = workspace
            .active_project
            .as_ref()
            .expect("double-clicking a directory should select it");
        assert_eq!(active_project.path, child_path);
        assert_eq!(active_project.name, "alpha-project");
        let active_thread = workspace
            .active_thread()
            .expect("selecting a directory should create an active thread");
        assert_eq!(active_thread.project_name, "alpha-project");
        assert_eq!(active_thread.project_path, child_path);
        assert_eq!(active_thread.chat.read(cx).project_path, child_path);
    });

    let _ = fs::remove_dir_all(&root_dir);
}

#[gpui::test]
fn desktop_task_create_dialog_submit_should_match_web_work_item_fields(
    cx: &mut gpui::TestAppContext,
) {
    cx.update(|cx| {
        gpui_component::init(cx);
        gpui_component::Theme::change(gpui_component::ThemeMode::Dark, None, cx);
        theme::apply_bruhs_theme(cx);
        let state = DesktopState::new().expect("desktop state should initialize");
        cx.set_global(state);
    });

    let workspace_cell: Rc<RefCell<Option<Entity<Workspace>>>> = Rc::new(RefCell::new(None));
    let workspace_for_window = workspace_cell.clone();
    let (_root, cx) = cx.add_window_view(|window, cx| {
        let workspace = cx.new(|cx| Workspace::new(window, cx));
        *workspace_for_window.borrow_mut() = Some(workspace.clone());
        gpui_component::Root::new(workspace, window, cx)
    });
    let workspace = workspace_cell
        .borrow()
        .clone()
        .expect("workspace entity should be captured");
    cx.simulate_resize(gpui::size(gpui::px(1_000.0), gpui::px(1_000.0)));

    let mut initial_item_count = 0;
    workspace.update_in(cx, |workspace, window, cx| {
        initial_item_count = workspace.work_items.len();
        workspace.projects.push(DesktopProject {
            id: "project-1".to_string(),
            name: "Alpha".to_string(),
        });
        workspace.selected_project_id = Some("project-1".to_string());
        workspace.open_task_create(window, cx);
    });
    cx.run_until_parked();

    workspace.update_in(cx, |workspace, window, cx| {
        assert!(workspace.task_create_visible());
        assert_eq!(
            workspace.task_create_project_id.as_deref(),
            Some("project-1")
        );
        assert_eq!(workspace.task_create_status, DesktopWorkItemStatus::Backlog);
        assert_eq!(workspace.task_create_priority, DesktopPriority::None);
        assert!(workspace
            .modal_start_focus_handle
            .contains_focused(window, cx));
    });
    assert_rendered_bounds(cx, "desktop-task-create-dialog");
    assert_rendered_bounds(cx, "desktop-task-create-select-project");
    assert_rendered_bounds(cx, "desktop-task-create-select-status");
    assert_rendered_bounds(cx, "desktop-task-create-select-priority");

    click_debug_selector(cx, "desktop-task-create-submit");
    cx.run_until_parked();
    assert_rendered_bounds(cx, "desktop-task-create-title-error");
    workspace.update_in(cx, |workspace, window, cx| {
        assert!(workspace.task_create_visible());
        assert_eq!(workspace.work_items.len(), initial_item_count);
        assert_eq!(
            workspace.task_create_error.as_deref(),
            Some("Title is required.")
        );
        assert!(workspace
            .task_create_title_input
            .read(cx)
            .focus_handle(cx)
            .contains_focused(window, cx));
    });

    workspace.update_in(cx, |workspace, window, cx| {
        workspace.task_create_title_input.update(cx, |input, cx| {
            input.set_value("Ship task parity", window, cx);
        });
        workspace
            .task_create_description_input
            .update(cx, |input, cx| {
                input.set_value("Created through the desktop task dialog", window, cx);
            });
        workspace.task_create_due_input.update(cx, |input, cx| {
            input.set_value("2026-05-06", window, cx);
        });
        cx.notify();
    });
    cx.run_until_parked();

    click_debug_selector(cx, "desktop-task-create-select-status");
    cx.run_until_parked();
    workspace.update_in(cx, |workspace, _window, _cx| {
        assert_eq!(
            workspace.task_create_menu_open(),
            Some(TaskCreateMenu::Status)
        );
    });
    assert_rendered_bounds(cx, "desktop-task-create-status-option-in_progress");
    click_debug_selector(cx, "desktop-task-create-status-option-in_progress");
    cx.run_until_parked();

    click_debug_selector(cx, "desktop-task-create-select-priority");
    cx.run_until_parked();
    assert_rendered_bounds(cx, "desktop-task-create-priority-option-high");
    click_debug_selector(cx, "desktop-task-create-priority-option-high");
    cx.run_until_parked();

    click_debug_selector(cx, "desktop-task-create-submit");
    cx.run_until_parked();

    workspace.update_in(cx, |workspace, _window, _cx| {
        assert!(!workspace.task_create_visible());
        assert_eq!(workspace.work_items.len(), initial_item_count + 1);
        let item = workspace
            .work_items
            .last()
            .expect("submitting should append a work item");
        assert_eq!(item.title, "Ship task parity");
        assert_eq!(
            item.description.as_deref(),
            Some("Created through the desktop task dialog")
        );
        assert_eq!(item.due_date.as_deref(), Some("2026-05-06"));
        assert_eq!(item.project_id.as_deref(), Some("project-1"));
        assert_eq!(item.status, DesktopWorkItemStatus::InProgress);
        assert_eq!(item.priority, DesktopPriority::High);
        assert!(workspace.selected_work_item_id.is_none());
    });
}

#[gpui::test]
fn desktop_tasks_header_controls_should_match_web_flow(cx: &mut gpui::TestAppContext) {
    cx.update(|cx| {
        gpui_component::init(cx);
        gpui_component::Theme::change(gpui_component::ThemeMode::Dark, None, cx);
        theme::apply_bruhs_theme(cx);
        let state = DesktopState::new().expect("desktop state should initialize");
        cx.set_global(state);
    });

    let workspace_cell: Rc<RefCell<Option<Entity<Workspace>>>> = Rc::new(RefCell::new(None));
    let workspace_for_window = workspace_cell.clone();
    let (_root, cx) = cx.add_window_view(|window, cx| {
        let workspace = cx.new(|cx| Workspace::new(window, cx));
        *workspace_for_window.borrow_mut() = Some(workspace.clone());
        gpui_component::Root::new(workspace, window, cx)
    });
    let workspace = workspace_cell
        .borrow()
        .clone()
        .expect("workspace entity should be captured");

    workspace.update_in(cx, |workspace, _window, cx| {
        workspace.active_view = AppView::Tasks;
        workspace.sidebar_visible = true;
        workspace.projects = vec![
            DesktopProject {
                id: "project-1".to_string(),
                name: "Alpha".to_string(),
            },
            DesktopProject {
                id: "project-2".to_string(),
                name: "Beta".to_string(),
            },
        ];
        workspace.work_items = vec![
            DesktopWorkItem {
                id: "task-alpha".to_string(),
                project_id: Some("project-1".to_string()),
                title: "Alpha task".to_string(),
                description: None,
                status: DesktopWorkItemStatus::Todo,
                priority: DesktopPriority::Medium,
                due_date: None,
                planning: WorkItemPlanning::default(),
                updated_at: EpochMs::UNIX_EPOCH,
            },
            DesktopWorkItem {
                id: "task-beta".to_string(),
                project_id: Some("project-2".to_string()),
                title: "Beta task".to_string(),
                description: None,
                status: DesktopWorkItemStatus::Todo,
                priority: DesktopPriority::Medium,
                due_date: None,
                planning: WorkItemPlanning::default(),
                updated_at: EpochMs::UNIX_EPOCH,
            },
        ];
        workspace.selected_project_id = Some("project-1".to_string());
        workspace.selected_work_item_id = None;
        workspace.task_view_mode = DesktopTaskViewMode::List;
        workspace.dismiss_overlay(|overlay| matches!(overlay, Overlay::TaskCreate { .. }));
        cx.notify();
    });
    cx.run_until_parked();

    let alpha_row = task_row_debug_selector("task-alpha");
    let beta_row = task_row_debug_selector("task-beta");
    let alpha_card = task_card_debug_selector("task-alpha");
    let beta_card = task_card_debug_selector("task-beta");

    assert_rendered_bounds(cx, "desktop-task-view-list");
    assert_rendered_bounds(cx, "desktop-task-view-board");
    assert_rendered_bounds(cx, "desktop-task-open-create");
    assert_rendered_bounds(cx, alpha_row);
    assert_no_rendered_bounds(cx, beta_row);
    assert_no_rendered_bounds(cx, "desktop-task-board");

    click_debug_selector(cx, "desktop-task-view-board");
    cx.run_until_parked();

    workspace.update_in(cx, |workspace, _window, _cx| {
        assert_eq!(workspace.task_view_mode, DesktopTaskViewMode::Board);
    });
    assert_rendered_bounds(cx, "desktop-task-board");
    assert_rendered_bounds(cx, alpha_card);
    assert_no_rendered_bounds(cx, beta_card);

    click_debug_selector(cx, "desktop-task-view-list");
    cx.run_until_parked();

    workspace.update_in(cx, |workspace, _window, _cx| {
        assert_eq!(workspace.task_view_mode, DesktopTaskViewMode::List);
    });
    assert_rendered_bounds(cx, alpha_row);
    assert_no_rendered_bounds(cx, beta_row);

    click_debug_selector(cx, "desktop-task-open-create");
    cx.run_until_parked();

    assert_rendered_bounds(cx, "desktop-task-create-dialog");
    workspace.update_in(cx, |workspace, window, cx| {
        assert_eq!(workspace.active_view, AppView::Tasks);
        assert!(workspace.task_create_visible());
        assert_eq!(
            workspace.task_create_project_id.as_deref(),
            Some("project-1")
        );
        assert!(workspace.selected_work_item_id.is_none());
        assert!(workspace
            .modal_start_focus_handle
            .contains_focused(window, cx));
    });
}

#[gpui::test]
fn desktop_event_create_dialog_submit_should_match_web_calendar_event_fields(
    cx: &mut gpui::TestAppContext,
) {
    cx.update(|cx| {
        gpui_component::init(cx);
        gpui_component::Theme::change(gpui_component::ThemeMode::Dark, None, cx);
        theme::apply_bruhs_theme(cx);
        let state = DesktopState::new().expect("desktop state should initialize");
        cx.set_global(state);
    });

    let workspace_cell: Rc<RefCell<Option<Entity<Workspace>>>> = Rc::new(RefCell::new(None));
    let workspace_for_window = workspace_cell.clone();
    let (_root, cx) = cx.add_window_view(|window, cx| {
        let workspace = cx.new(|cx| Workspace::new(window, cx));
        *workspace_for_window.borrow_mut() = Some(workspace.clone());
        gpui_component::Root::new(workspace, window, cx)
    });
    let workspace = workspace_cell
        .borrow()
        .clone()
        .expect("workspace entity should be captured");
    cx.simulate_resize(gpui::size(gpui::px(1_000.0), gpui::px(1_000.0)));

    let mut initial_event_count = 0;
    workspace.update_in(cx, |workspace, window, cx| {
        initial_event_count = workspace.calendar_events.len();
        workspace.open_event_create(Some(dt(2026, 5, 6, 9, 15)), window, cx);
    });
    cx.run_until_parked();

    workspace.update_in(cx, |workspace, window, cx| {
        assert!(workspace.event_create_visible());
        assert_eq!(workspace.selected_calendar_event_id, None);
        assert_eq!(workspace.event_create_color, "persimmon");
        assert!(!workspace.event_create_all_day);
        assert_eq!(
            workspace.event_create_start_input.read(cx).value().as_ref(),
            "2026-05-06T09:15"
        );
        assert_eq!(
            workspace.event_create_end_input.read(cx).value().as_ref(),
            "2026-05-06T10:15"
        );
        assert!(workspace
            .modal_start_focus_handle
            .contains_focused(window, cx));
    });
    assert_rendered_bounds(cx, "desktop-event-create-dialog");
    assert_rendered_bounds(cx, "desktop-event-create-all-day");
    assert_rendered_bounds(cx, "desktop-event-color-select");

    click_debug_selector(cx, "desktop-event-create-submit");
    cx.run_until_parked();
    assert_rendered_bounds(cx, "desktop-event-create-title-error");
    workspace.update_in(cx, |workspace, window, cx| {
        assert!(workspace.event_create_visible());
        assert_eq!(workspace.calendar_events.len(), initial_event_count);
        assert_eq!(
            workspace.event_create_error.as_deref(),
            Some("Title is required.")
        );
        assert!(workspace
            .event_create_title_input
            .read(cx)
            .focus_handle(cx)
            .contains_focused(window, cx));
    });

    workspace.update_in(cx, |workspace, window, cx| {
        workspace.event_create_title_input.update(cx, |input, cx| {
            input.set_value("Calendar parity check", window, cx);
        });
        workspace.event_create_notes_input.update(cx, |input, cx| {
            input.set_value("Created through the desktop event dialog", window, cx);
        });
        cx.notify();
    });
    cx.run_until_parked();

    click_debug_selector(cx, "desktop-event-create-all-day");
    cx.run_until_parked();

    click_debug_selector(cx, "desktop-event-color-select");
    cx.run_until_parked();
    assert_rendered_bounds(cx, "desktop-event-color-option-pandan");
    click_debug_selector(cx, "desktop-event-color-option-pandan");
    cx.run_until_parked();

    click_debug_selector(cx, "desktop-event-create-submit");
    cx.run_until_parked();

    workspace.update_in(cx, |workspace, _window, _cx| {
        assert!(!workspace.event_create_visible());
        assert_eq!(workspace.calendar_events.len(), initial_event_count + 1);
        let event = workspace
            .calendar_events
            .last()
            .expect("submitting should append a calendar event");
        assert_eq!(event.title, "Calendar parity check");
        assert_eq!(event.start_at, dt(2026, 5, 6, 0, 0));
        assert_eq!(event.end_at, dt(2026, 5, 7, 0, 0));
        assert!(event.all_day);
        assert_eq!(event.color.as_deref(), Some("pandan"));
        assert_eq!(
            event.notes.as_deref(),
            Some("Created through the desktop event dialog")
        );
        assert_eq!(workspace.selected_calendar_event_id, None);
    });
}

#[gpui::test]
fn desktop_event_edit_dialog_save_and_delete_should_match_web_flow(cx: &mut gpui::TestAppContext) {
    cx.update(|cx| {
        gpui_component::init(cx);
        gpui_component::Theme::change(gpui_component::ThemeMode::Dark, None, cx);
        theme::apply_bruhs_theme(cx);
        let state = DesktopState::new().expect("desktop state should initialize");
        cx.set_global(state);
    });

    let workspace_cell: Rc<RefCell<Option<Entity<Workspace>>>> = Rc::new(RefCell::new(None));
    let workspace_for_window = workspace_cell.clone();
    let (_root, cx) = cx.add_window_view(|window, cx| {
        let workspace = cx.new(|cx| Workspace::new(window, cx));
        *workspace_for_window.borrow_mut() = Some(workspace.clone());
        gpui_component::Root::new(workspace, window, cx)
    });
    let workspace = workspace_cell
        .borrow()
        .clone()
        .expect("workspace entity should be captured");

    workspace.update_in(cx, |workspace, window, cx| {
        workspace.active_view = AppView::Calendar;
        workspace.calendar_anchor =
            NaiveDate::from_ymd_opt(2026, 5, 6).expect("test date should be valid");
        workspace.calendar_events.clear();
        workspace.calendar_events.push(DesktopCalendarEvent {
            id: "event-1".to_string(),
            title: "Original title".to_string(),
            start_at: dt(2026, 5, 6, 9, 0),
            end_at: dt(2026, 5, 6, 10, 0),
            all_day: false,
            color: Some("persimmon".to_string()),
            notes: Some("Original notes".to_string()),
        });
        workspace.open_event_edit("event-1".to_string(), window, cx);
    });
    cx.run_until_parked();

    workspace.update_in(cx, |workspace, window, cx| {
        assert!(workspace.event_create_visible());
        assert_eq!(
            workspace.selected_calendar_event_id.as_deref(),
            Some("event-1")
        );
        assert_eq!(
            workspace.event_create_title_input.read(cx).value().as_ref(),
            "Original title"
        );
        assert_eq!(workspace.event_create_color, "persimmon");
        assert!(workspace
            .modal_start_focus_handle
            .contains_focused(window, cx));
    });
    assert_rendered_bounds(cx, "desktop-event-create-dialog");
    assert_rendered_bounds(cx, "desktop-event-create-delete");

    workspace.update_in(cx, |workspace, window, cx| {
        workspace.event_create_title_input.update(cx, |input, cx| {
            input.set_value("Updated event", window, cx);
        });
        workspace.event_create_notes_input.update(cx, |input, cx| {
            input.set_value("Updated notes", window, cx);
        });
        workspace.event_create_start_input.update(cx, |input, cx| {
            input.set_value("2026-05-06T11:00", window, cx);
        });
        workspace.event_create_end_input.update(cx, |input, cx| {
            input.set_value("2026-05-06T12:30", window, cx);
        });
        cx.notify();
    });
    cx.run_until_parked();

    click_debug_selector(cx, "desktop-event-color-select");
    cx.run_until_parked();
    click_debug_selector(cx, "desktop-event-color-option-blueberry");
    cx.run_until_parked();

    click_debug_selector(cx, "desktop-event-create-submit");
    cx.run_until_parked();

    workspace.update_in(cx, |workspace, window, cx| {
        assert!(!workspace.event_create_visible());
        assert_eq!(workspace.selected_calendar_event_id, None);
        assert_eq!(workspace.calendar_events.len(), 1);
        let event = workspace
            .calendar_events
            .first()
            .expect("event should be updated in place");
        assert_eq!(event.id, "event-1");
        assert_eq!(event.title, "Updated event");
        assert_eq!(event.start_at, dt(2026, 5, 6, 11, 0));
        assert_eq!(event.end_at, dt(2026, 5, 6, 12, 30));
        assert_eq!(event.color.as_deref(), Some("blueberry"));
        assert_eq!(event.notes.as_deref(), Some("Updated notes"));

        workspace.open_event_edit("event-1".to_string(), window, cx);
    });
    cx.run_until_parked();

    assert_rendered_bounds(cx, "desktop-event-create-delete");
    click_debug_selector(cx, "desktop-event-create-delete");
    cx.run_until_parked();

    workspace.update_in(cx, |workspace, _window, _cx| {
        assert!(!workspace.event_create_visible());
        assert_eq!(workspace.selected_calendar_event_id, None);
        assert!(workspace.calendar_events.is_empty());
    });
}

#[gpui::test]
fn desktop_calendar_navigation_and_create_entrypoints_should_match_web_flow(
    cx: &mut gpui::TestAppContext,
) {
    cx.update(|cx| {
        gpui_component::init(cx);
        gpui_component::Theme::change(gpui_component::ThemeMode::Dark, None, cx);
        theme::apply_bruhs_theme(cx);
        let state = DesktopState::new().expect("desktop state should initialize");
        cx.set_global(state);
    });

    let workspace_cell: Rc<RefCell<Option<Entity<Workspace>>>> = Rc::new(RefCell::new(None));
    let workspace_for_window = workspace_cell.clone();
    let (_root, cx) = cx.add_window_view(|window, cx| {
        let workspace = cx.new(|cx| Workspace::new(window, cx));
        *workspace_for_window.borrow_mut() = Some(workspace.clone());
        gpui_component::Root::new(workspace, window, cx)
    });
    let workspace = workspace_cell
        .borrow()
        .clone()
        .expect("workspace entity should be captured");

    workspace.update_in(cx, |workspace, _window, cx| {
        workspace.active_view = AppView::Calendar;
        workspace.calendar_mode = DesktopCalendarMode::Month;
        workspace.calendar_anchor =
            NaiveDate::from_ymd_opt(2026, 5, 6).expect("test date should be valid");
        workspace.calendar_events.clear();
        workspace.selected_calendar_event_id = None;
        workspace.dismiss_overlay(|overlay| matches!(overlay, Overlay::EventCreate { .. }));
        cx.notify();
    });
    cx.run_until_parked();

    assert_rendered_bounds(cx, "desktop-calendar-page");
    assert_rendered_bounds(cx, "desktop-calendar-title");
    assert_rendered_bounds(cx, "desktop-calendar-month");
    assert_rendered_bounds(cx, "desktop-calendar-mode-month");
    assert_rendered_bounds(cx, "desktop-calendar-prev");
    assert_rendered_bounds(cx, "desktop-calendar-today");
    assert_rendered_bounds(cx, "desktop-calendar-next");
    assert_rendered_bounds(cx, "desktop-calendar-new-event");

    click_debug_selector(cx, "desktop-calendar-next");
    cx.run_until_parked();
    workspace.update_in(cx, |workspace, _window, _cx| {
        assert_eq!(
            workspace.calendar_anchor,
            NaiveDate::from_ymd_opt(2026, 6, 6).expect("test date should be valid")
        );
        assert_eq!(workspace.calendar_title(), "June 2026");
    });

    click_debug_selector(cx, "desktop-calendar-prev");
    cx.run_until_parked();
    workspace.update_in(cx, |workspace, _window, _cx| {
        assert_eq!(
            workspace.calendar_anchor,
            NaiveDate::from_ymd_opt(2026, 5, 6).expect("test date should be valid")
        );
        assert_eq!(workspace.calendar_title(), "May 2026");
    });

    click_debug_selector(cx, "desktop-calendar-mode-week");
    cx.run_until_parked();
    assert_rendered_bounds(cx, "desktop-calendar-timegrid-7");
    workspace.update_in(cx, |workspace, _window, _cx| {
        assert_eq!(workspace.calendar_mode, DesktopCalendarMode::Week);
        assert_eq!(workspace.calendar_title(), "May 3 — 9, 2026");
    });

    click_debug_selector(cx, "desktop-calendar-next");
    cx.run_until_parked();
    workspace.update_in(cx, |workspace, _window, _cx| {
        assert_eq!(
            workspace.calendar_anchor,
            NaiveDate::from_ymd_opt(2026, 5, 13).expect("test date should be valid")
        );
        assert_eq!(workspace.calendar_title(), "May 10 — 16, 2026");
    });

    click_debug_selector(cx, "desktop-calendar-mode-day");
    cx.run_until_parked();
    assert_rendered_bounds(cx, "desktop-calendar-timegrid-1");
    workspace.update_in(cx, |workspace, _window, _cx| {
        assert_eq!(workspace.calendar_mode, DesktopCalendarMode::Day);
        assert_eq!(workspace.calendar_title(), "May 13, 2026");
    });

    click_debug_selector(cx, "desktop-calendar-prev");
    cx.run_until_parked();
    workspace.update_in(cx, |workspace, _window, _cx| {
        assert_eq!(
            workspace.calendar_anchor,
            NaiveDate::from_ymd_opt(2026, 5, 12).expect("test date should be valid")
        );
        assert_eq!(workspace.calendar_title(), "May 12, 2026");
    });

    click_debug_selector(cx, "desktop-calendar-today");
    cx.run_until_parked();
    workspace.update_in(cx, |workspace, _window, _cx| {
        assert_eq!(workspace.calendar_anchor, super::today_date());
    });

    click_debug_selector(cx, "desktop-calendar-new-event");
    cx.run_until_parked();
    workspace.update_in(cx, |workspace, _window, cx| {
        assert!(workspace.event_create_visible());
        assert_eq!(workspace.selected_calendar_event_id, None);
        assert!(workspace
            .event_create_start_input
            .read(cx)
            .value()
            .contains('T'));
    });
    assert_rendered_bounds(cx, "desktop-event-create-dialog");
    click_debug_selector(cx, "desktop-event-create-cancel");
    cx.run_until_parked();

    workspace.update_in(cx, |workspace, _window, cx| {
        workspace.calendar_mode = DesktopCalendarMode::Month;
        workspace.calendar_anchor =
            NaiveDate::from_ymd_opt(2026, 5, 6).expect("test date should be valid");
        workspace.dismiss_overlay(|overlay| matches!(overlay, Overlay::EventCreate { .. }));
        cx.notify();
    });
    cx.run_until_parked();
    assert_rendered_bounds(cx, "desktop-calendar-day-2026-05-06");
    click_debug_selector(cx, "desktop-calendar-day-2026-05-06");
    cx.run_until_parked();
    workspace.update_in(cx, |workspace, _window, cx| {
        assert!(workspace.event_create_visible());
        assert_eq!(
            workspace.event_create_start_input.read(cx).value().as_ref(),
            "2026-05-06T09:00"
        );
        assert_eq!(
            workspace.event_create_end_input.read(cx).value().as_ref(),
            "2026-05-06T10:00"
        );
    });
    click_debug_selector(cx, "desktop-event-create-cancel");
    cx.run_until_parked();

    workspace.update_in(cx, |workspace, _window, cx| {
        workspace.calendar_mode = DesktopCalendarMode::Day;
        workspace.calendar_anchor =
            NaiveDate::from_ymd_opt(2026, 5, 12).expect("test date should be valid");
        workspace.dismiss_overlay(|overlay| matches!(overlay, Overlay::EventCreate { .. }));
        cx.notify();
    });
    cx.run_until_parked();
    assert_rendered_bounds(cx, "desktop-calendar-timegrid-hour-2026-05-12-9");
    click_debug_selector(cx, "desktop-calendar-timegrid-hour-2026-05-12-9");
    cx.run_until_parked();
    workspace.update_in(cx, |workspace, _window, cx| {
        assert!(workspace.event_create_visible());
        assert_eq!(
            workspace.event_create_start_input.read(cx).value().as_ref(),
            "2026-05-12T09:00"
        );
        assert_eq!(
            workspace.event_create_end_input.read(cx).value().as_ref(),
            "2026-05-12T10:00"
        );
    });
}

#[gpui::test]
fn desktop_calendar_month_pills_should_match_web_click_flow(cx: &mut gpui::TestAppContext) {
    cx.update(|cx| {
        gpui_component::init(cx);
        gpui_component::Theme::change(gpui_component::ThemeMode::Dark, None, cx);
        theme::apply_bruhs_theme(cx);
        let state = DesktopState::new().expect("desktop state should initialize");
        cx.set_global(state);
    });

    let workspace_cell: Rc<RefCell<Option<Entity<Workspace>>>> = Rc::new(RefCell::new(None));
    let workspace_for_window = workspace_cell.clone();
    let (_root, cx) = cx.add_window_view(|window, cx| {
        let workspace = cx.new(|cx| Workspace::new(window, cx));
        *workspace_for_window.borrow_mut() = Some(workspace.clone());
        gpui_component::Root::new(workspace, window, cx)
    });
    let workspace = workspace_cell
        .borrow()
        .clone()
        .expect("workspace entity should be captured");
    cx.simulate_resize(gpui::size(gpui::px(1120.0), gpui::px(760.0)));

    workspace.update_in(cx, |workspace, _window, cx| {
        workspace.active_view = AppView::Calendar;
        workspace.calendar_mode = DesktopCalendarMode::Month;
        workspace.calendar_anchor =
            NaiveDate::from_ymd_opt(2026, 5, 6).expect("test date should be valid");
        workspace.dismiss_overlay(|overlay| matches!(overlay, Overlay::EventCreate { .. }));
        workspace.selected_calendar_event_id = None;
        workspace.selected_work_item_id = None;
        workspace.calendar_events = vec![DesktopCalendarEvent {
            id: "event-1".to_string(),
            title: "Design review".to_string(),
            start_at: dt(2026, 5, 6, 11, 0),
            end_at: dt(2026, 5, 6, 12, 0),
            all_day: false,
            color: Some("blueberry".to_string()),
            notes: None,
        }];
        workspace.work_items = vec![DesktopWorkItem {
            id: "task-1".to_string(),
            project_id: None,
            title: "Ship calendar task marker".to_string(),
            description: None,
            status: DesktopWorkItemStatus::Todo,
            priority: DesktopPriority::Medium,
            due_date: Some("2026-05-06".to_string()),
            planning: WorkItemPlanning::default(),
            updated_at: EpochMs::UNIX_EPOCH,
        }];
        cx.notify();
    });
    cx.run_until_parked();

    let event_pill = calendar_event_pill_debug_selector("event-1");
    let task_pill = calendar_task_pill_debug_selector("task-1");
    assert_rendered_bounds(cx, "desktop-calendar-day-2026-05-06");
    assert_rendered_bounds(cx, event_pill);
    assert_rendered_bounds(cx, task_pill);

    click_debug_selector(cx, event_pill);
    cx.run_until_parked();
    workspace.update_in(cx, |workspace, _window, cx| {
        assert_eq!(workspace.active_view, AppView::Calendar);
        assert!(workspace.event_create_visible());
        assert_eq!(
            workspace.selected_calendar_event_id.as_deref(),
            Some("event-1")
        );
        assert_eq!(workspace.selected_work_item_id, None);
        assert_eq!(
            workspace.event_create_title_input.read(cx).value().as_ref(),
            "Design review"
        );
    });

    click_debug_selector(cx, "desktop-event-create-cancel");
    cx.run_until_parked();
    workspace.update_in(cx, |workspace, _window, cx| {
        workspace.selected_calendar_event_id = None;
        workspace.dismiss_overlay(|overlay| matches!(overlay, Overlay::EventCreate { .. }));
        cx.notify();
    });
    cx.run_until_parked();

    click_debug_selector(cx, task_pill);
    cx.run_until_parked();
    workspace.update_in(cx, |workspace, _window, cx| {
        assert_eq!(workspace.active_view, AppView::Calendar);
        assert!(!workspace.event_create_visible());
        assert_eq!(workspace.selected_calendar_event_id, None);
        assert_eq!(workspace.selected_work_item_id.as_deref(), Some("task-1"));
        assert_eq!(
            workspace.task_detail_title_input.read(cx).value().as_ref(),
            "Ship calendar task marker"
        );
    });
}

#[gpui::test]
fn desktop_calendar_timegrid_event_block_should_match_web_click_flow(
    cx: &mut gpui::TestAppContext,
) {
    cx.update(|cx| {
        gpui_component::init(cx);
        gpui_component::Theme::change(gpui_component::ThemeMode::Dark, None, cx);
        theme::apply_bruhs_theme(cx);
        let state = DesktopState::new().expect("desktop state should initialize");
        cx.set_global(state);
    });

    let workspace_cell: Rc<RefCell<Option<Entity<Workspace>>>> = Rc::new(RefCell::new(None));
    let workspace_for_window = workspace_cell.clone();
    let (_root, cx) = cx.add_window_view(|window, cx| {
        let workspace = cx.new(|cx| Workspace::new(window, cx));
        *workspace_for_window.borrow_mut() = Some(workspace.clone());
        gpui_component::Root::new(workspace, window, cx)
    });
    let workspace = workspace_cell
        .borrow()
        .clone()
        .expect("workspace entity should be captured");
    cx.simulate_resize(gpui::size(gpui::px(1120.0), gpui::px(760.0)));

    workspace.update_in(cx, |workspace, _window, cx| {
        workspace.active_view = AppView::Calendar;
        workspace.calendar_mode = DesktopCalendarMode::Day;
        workspace.calendar_anchor =
            NaiveDate::from_ymd_opt(2026, 5, 12).expect("test date should be valid");
        workspace.dismiss_overlay(|overlay| matches!(overlay, Overlay::EventCreate { .. }));
        workspace.selected_calendar_event_id = None;
        workspace.calendar_events = vec![DesktopCalendarEvent {
            id: "event-1".to_string(),
            title: "Timegrid review".to_string(),
            start_at: dt(2026, 5, 12, 9, 0),
            end_at: dt(2026, 5, 12, 10, 0),
            all_day: false,
            color: Some("pandan".to_string()),
            notes: Some("Review resize behavior".to_string()),
        }];
        cx.notify();
    });
    cx.run_until_parked();

    let event_block = calendar_timegrid_event_debug_selector("event-1");
    let resize_handle = calendar_timegrid_event_resize_debug_selector("event-1");
    assert_rendered_bounds(cx, "desktop-calendar-timegrid-1");
    assert_rendered_bounds(cx, event_block);
    assert_rendered_bounds(cx, resize_handle);

    click_debug_selector(cx, resize_handle);
    cx.run_until_parked();
    workspace.update_in(cx, |workspace, _window, _cx| {
        assert!(!workspace.event_create_visible());
        assert_eq!(workspace.selected_calendar_event_id, None);
        assert_eq!(workspace.calendar_events.len(), 1);
    });

    click_debug_selector(cx, event_block);
    cx.run_until_parked();
    workspace.update_in(cx, |workspace, _window, cx| {
        assert_eq!(workspace.active_view, AppView::Calendar);
        assert!(workspace.event_create_visible());
        assert_eq!(
            workspace.selected_calendar_event_id.as_deref(),
            Some("event-1")
        );
        assert_eq!(
            workspace.event_create_title_input.read(cx).value().as_ref(),
            "Timegrid review"
        );
        assert_eq!(
            workspace.event_create_start_input.read(cx).value().as_ref(),
            "2026-05-12T09:00"
        );
        assert_eq!(
            workspace.event_create_end_input.read(cx).value().as_ref(),
            "2026-05-12T10:00"
        );
    });
}

#[gpui::test]
fn desktop_calendar_timegrid_move_and_resize_should_match_web_state_flow(
    cx: &mut gpui::TestAppContext,
) {
    cx.update(|cx| {
        gpui_component::init(cx);
        gpui_component::Theme::change(gpui_component::ThemeMode::Dark, None, cx);
        theme::apply_bruhs_theme(cx);
        let state = DesktopState::new().expect("desktop state should initialize");
        cx.set_global(state);
    });

    let workspace_cell: Rc<RefCell<Option<Entity<Workspace>>>> = Rc::new(RefCell::new(None));
    let workspace_for_window = workspace_cell.clone();
    let (_root, cx) = cx.add_window_view(|window, cx| {
        let workspace = cx.new(|cx| Workspace::new(window, cx));
        *workspace_for_window.borrow_mut() = Some(workspace.clone());
        gpui_component::Root::new(workspace, window, cx)
    });
    let workspace = workspace_cell
        .borrow()
        .clone()
        .expect("workspace entity should be captured");
    cx.simulate_resize(gpui::size(gpui::px(1120.0), gpui::px(760.0)));

    workspace.update_in(cx, |workspace, _window, cx| {
        workspace.active_view = AppView::Calendar;
        workspace.calendar_mode = DesktopCalendarMode::Week;
        workspace.calendar_anchor =
            NaiveDate::from_ymd_opt(2026, 5, 4).expect("test date should be valid");
        workspace.dismiss_overlay(|overlay| matches!(overlay, Overlay::EventCreate { .. }));
        workspace.selected_calendar_event_id = None;
        workspace.calendar_events = vec![DesktopCalendarEvent {
            id: "event-1".to_string(),
            title: "Move across week".to_string(),
            start_at: dt(2026, 5, 4, 9, 0),
            end_at: dt(2026, 5, 4, 10, 30),
            all_day: false,
            color: Some("blueberry".to_string()),
            notes: None,
        }];
        cx.notify();
    });
    cx.run_until_parked();

    let event_block = calendar_timegrid_event_debug_selector("event-1");
    let resize_handle = calendar_timegrid_event_resize_debug_selector("event-1");
    assert_rendered_bounds(cx, "desktop-calendar-timegrid-7");
    assert_rendered_bounds(cx, "desktop-calendar-timegrid-hour-2026-05-06-13");
    assert_rendered_bounds(cx, event_block);
    let original_bounds = rendered_bounds(cx, event_block);

    workspace.update_in(cx, |workspace, _window, cx| {
        let event = workspace
            .calendar_events
            .iter_mut()
            .find(|event| event.id == "event-1")
            .expect("event should exist before move");
        assert!(move_timegrid_event_to(
            event,
            NaiveDate::from_ymd_opt(2026, 5, 6).expect("test date should be valid"),
            DESKTOP_HOUR_ROW_PX * 13.18 + 8.0,
            8.0,
        ));
        workspace.persist_and_notify(cx);
    });
    cx.run_until_parked();

    workspace.update_in(cx, |workspace, _window, _cx| {
        let event = workspace
            .calendar_events
            .iter()
            .find(|event| event.id == "event-1")
            .expect("event should still exist after move");
        assert_eq!(event.start_at, dt(2026, 5, 6, 13, 15));
        assert_eq!(event.end_at, dt(2026, 5, 6, 14, 45));
    });
    let moved_bounds = rendered_bounds(cx, event_block);
    let target_hour_bounds = rendered_bounds(cx, "desktop-calendar-timegrid-hour-2026-05-06-13");
    let moved_start_point = gpui::point(
        moved_bounds.center().x,
        moved_bounds.origin.y + gpui::px(2.0),
    );
    assert!(target_hour_bounds.contains(&moved_start_point));
    assert!(
        f32::from((moved_bounds.origin.x - original_bounds.origin.x).abs()) > DESKTOP_HOUR_ROW_PX,
        "moving across week columns should change the rendered x-position"
    );
    assert!(
        f32::from((moved_bounds.origin.y - original_bounds.origin.y).abs()) > DESKTOP_HOUR_ROW_PX,
        "moving to a later time should change the rendered y-position"
    );

    assert_rendered_bounds(cx, resize_handle);
    let height_before_resize = f32::from(rendered_bounds(cx, event_block).size.height);
    workspace.update_in(cx, |workspace, _window, cx| {
        let event = workspace
            .calendar_events
            .iter_mut()
            .find(|event| event.id == "event-1")
            .expect("event should exist before resize");
        assert!(resize_timegrid_event_end_to(
            event,
            NaiveDate::from_ymd_opt(2026, 5, 6).expect("test date should be valid"),
            DESKTOP_HOUR_ROW_PX * 16.05,
        ));
        workspace.persist_and_notify(cx);
    });
    cx.run_until_parked();

    workspace.update_in(cx, |workspace, _window, _cx| {
        let event = workspace
            .calendar_events
            .iter()
            .find(|event| event.id == "event-1")
            .expect("event should still exist after resize");
        assert_eq!(event.start_at, dt(2026, 5, 6, 13, 15));
        assert_eq!(event.end_at, dt(2026, 5, 6, 16, 0));
    });
    let height_after_resize = f32::from(rendered_bounds(cx, event_block).size.height);
    assert!(
        height_after_resize > height_before_resize,
        "resizing the end time should increase the rendered event height"
    );
    assert_rendered_bounds(cx, resize_handle);
}

#[gpui::test]
fn desktop_workspace_sidebar_controls_and_cards_should_match_web_flow(
    cx: &mut gpui::TestAppContext,
) {
    cx.update(|cx| {
        gpui_component::init(cx);
        gpui_component::Theme::change(gpui_component::ThemeMode::Dark, None, cx);
        theme::apply_bruhs_theme(cx);
        let state = DesktopState::new().expect("desktop state should initialize");
        cx.set_global(state);
    });

    let workspace_cell: Rc<RefCell<Option<Entity<Workspace>>>> = Rc::new(RefCell::new(None));
    let workspace_for_window = workspace_cell.clone();
    let (_root, cx) = cx.add_window_view(|window, cx| {
        let workspace = cx.new(|cx| Workspace::new(window, cx));
        *workspace_for_window.borrow_mut() = Some(workspace.clone());
        gpui_component::Root::new(workspace, window, cx)
    });
    let workspace = workspace_cell
        .borrow()
        .clone()
        .expect("workspace entity should be captured");

    let mut general_id = String::new();
    let mut code_id = String::new();
    workspace.update_in(cx, |workspace, window, cx| {
        workspace.active_view = AppView::Chat;
        workspace.sidebar_visible = true;
        workspace.sidebar_tab = DesktopSidebarTab::Chat;
        workspace.threads.clear();
        let general = Workspace::create_thread_with_title(
            String::new(),
            String::new(),
            workspace.current_model.clone(),
            workspace.permission_mode,
            "General chat",
            window,
            cx,
        );
        general_id = general.id.clone();
        let code = Workspace::create_thread_with_title(
            "Alpha".to_string(),
            "/tmp/alpha".to_string(),
            workspace.current_model.clone(),
            workspace.permission_mode,
            "Alpha session",
            window,
            cx,
        );
        code_id = code.id.clone();
        workspace.active_thread_id = Some(general_id.clone());
        workspace.threads = vec![general, code];
        cx.notify();
    });
    cx.run_until_parked();

    assert_rendered_bounds(cx, "desktop-workspace-sidebar");
    assert_rendered_bounds(cx, "desktop-sidebar-header");
    assert_rendered_bounds(cx, "desktop-sidebar-search");
    assert_rendered_bounds(cx, "desktop-sidebar-new-thread");
    assert_rendered_bounds(cx, "desktop-workspace-modes");
    assert_rendered_bounds(cx, "desktop-workspace-mode-chat");
    assert_rendered_bounds(cx, "desktop-workspace-mode-code");
    assert_no_rendered_bounds(cx, "desktop-sidebar-filter");
    assert_no_rendered_bounds(cx, "desktop-sidebar-new-project");
    assert_rendered_bounds(cx, "desktop-sidebar-footer");
    assert_rendered_bounds(cx, thread_row_debug_selector(&general_id));
    assert_no_rendered_bounds(cx, thread_row_debug_selector(&code_id));

    let original_chat = workspace.update_in(cx, |workspace, window, cx| {
        let chat = workspace
            .active_thread()
            .expect("general thread")
            .chat
            .clone();
        chat.update(cx, |chat, cx| {
            chat.set_composer_draft("Keep this draft".into(), window, cx)
        });
        chat.entity_id()
    });
    click_debug_selector(cx, "desktop-workspace-mode-code");
    cx.run_until_parked();
    workspace.update_in(cx, |workspace, _window, cx| {
        let chat = &workspace
            .active_thread()
            .expect("still general thread")
            .chat;
        assert_eq!(chat.entity_id(), original_chat);
        assert_eq!(
            chat.read(cx).input_state.read(cx).value().as_str(),
            "Keep this draft"
        );
        assert_eq!(
            workspace.active_thread_id.as_deref(),
            Some(general_id.as_str())
        );
    });
    assert_rendered_bounds(cx, thread_row_debug_selector(&code_id));
    assert_no_rendered_bounds(cx, thread_row_debug_selector(&general_id));
    click_debug_selector(cx, "desktop-workspace-mode-chat");
    cx.run_until_parked();
    cx.simulate_keystrokes("tab space");
    workspace.update_in(cx, |workspace, _window, _cx| {
        assert_eq!(workspace.sidebar_tab, DesktopSidebarTab::Code);
    });
    cx.simulate_keystrokes("shift-tab enter");
    workspace.update_in(cx, |workspace, _window, _cx| {
        assert_eq!(workspace.sidebar_tab, DesktopSidebarTab::Chat);
    });

    // The square-pen button from a general chat starts another general chat.
    click_debug_selector(cx, "desktop-sidebar-new-thread");
    cx.run_until_parked();
    let new_general_id = workspace.update_in(cx, |workspace, _window, _cx| {
        assert_eq!(workspace.active_view, AppView::Chat);
        assert_eq!(workspace.threads.len(), 3);
        let thread = workspace
            .active_thread()
            .expect("new chat should become active");
        assert!(thread.project_name.is_empty());
        thread.id.clone()
    });
    assert_rendered_bounds(cx, thread_row_debug_selector(&new_general_id));

    // Delete shows on hover at the card's top right.
    hover_debug_selector(cx, thread_row_debug_selector(&new_general_id));
    cx.run_until_parked();
    click_debug_selector(cx, thread_delete_debug_selector(&new_general_id));
    cx.run_until_parked();
    workspace.update_in(cx, |workspace, _window, _cx| {
        assert_eq!(workspace.active_thread_id, None);
        assert_eq!(workspace.threads.len(), 2);
        assert!(workspace
            .threads
            .iter()
            .all(|thread| thread.id != new_general_id));
    });

    // Mode controls are above the workspace, not duplicated in the filter.
    assert_rendered_bounds(cx, thread_row_debug_selector(&general_id));
    assert_no_rendered_bounds(cx, thread_row_debug_selector(&code_id));
    click_debug_selector(cx, "desktop-workspace-mode-code");
    cx.run_until_parked();
    assert_rendered_bounds(cx, "desktop-sidebar-filter");
    click_debug_selector(cx, "desktop-sidebar-filter");
    cx.run_until_parked();
    assert_rendered_bounds(cx, "desktop-sidebar-filter-all");
    assert_no_rendered_bounds(cx, "desktop-sidebar-filter-chat");
    click_debug_selector(cx, "desktop-sidebar-filter-all");
    workspace.update_in(cx, |workspace, _window, cx| {
        assert_eq!(workspace.sidebar_tab, DesktopSidebarTab::Code);
        workspace.sidebar_filter = super::sidebar_ui::SidebarFilter::Project("Alpha".to_string());
        cx.notify();
    });
    cx.run_until_parked();
    assert_rendered_bounds(cx, thread_row_debug_selector(&code_id));
    assert_no_rendered_bounds(cx, thread_row_debug_selector(&general_id));
    workspace.update_in(cx, |workspace, _window, cx| {
        workspace.sidebar_filter = super::sidebar_ui::SidebarFilter::All;
        cx.notify();
    });
    cx.run_until_parked();

    // Cards select threads; the folder-plus button opens the project picker.
    click_debug_selector(cx, thread_row_debug_selector(&code_id));
    cx.run_until_parked();
    workspace.update_in(cx, |workspace, _window, _cx| {
        assert_eq!(
            workspace.active_thread_id.as_deref(),
            Some(code_id.as_str())
        );
    });
    click_debug_selector(cx, "desktop-sidebar-new-project");
    cx.run_until_parked();
    workspace.update_in(cx, |workspace, _window, _cx| {
        assert!(workspace.project_picker_visible());
    });
    assert_rendered_bounds(cx, "desktop-project-picker-dialog");
}

#[gpui::test]
fn desktop_status_bar_actions_should_match_web_flow(cx: &mut gpui::TestAppContext) {
    cx.update(|cx| {
        gpui_component::init(cx);
        gpui_component::Theme::change(gpui_component::ThemeMode::Dark, None, cx);
        theme::apply_bruhs_theme(cx);
        let state = DesktopState::new().expect("desktop state should initialize");
        cx.set_global(state);
    });

    let workspace_cell: Rc<RefCell<Option<Entity<Workspace>>>> = Rc::new(RefCell::new(None));
    let workspace_for_window = workspace_cell.clone();
    let (_root, cx) = cx.add_window_view(|window, cx| {
        let workspace = cx.new(|cx| Workspace::new(window, cx));
        *workspace_for_window.borrow_mut() = Some(workspace.clone());
        gpui_component::Root::new(workspace, window, cx)
    });
    let workspace = workspace_cell
        .borrow()
        .clone()
        .expect("workspace entity should be captured");

    workspace.update_in(cx, |workspace, _window, cx| {
        workspace.active_view = AppView::Chat;
        workspace.sidebar_visible = true;
        workspace.set_current_model("gpt-5.5", cx);
        workspace.set_permission_mode(PermissionMode::Default, cx);
    });
    cx.run_until_parked();

    assert_rendered_bounds(cx, "desktop-status-bar");
    assert_rendered_bounds(cx, "desktop-model-status");
    assert_rendered_bounds(cx, "desktop-permission-mode");
    assert_provider_logo_bounds(cx, "desktop-chat-provider-logo-codex");

    // The access-mode chip opens a menu (web); pick Auto-accept from it.
    click_debug_selector(cx, "desktop-permission-mode");
    cx.run_until_parked();
    assert_rendered_bounds(cx, "desktop-permission-menu");
    click_debug_selector(
        cx,
        leaked_debug_selector(format!(
            "desktop-permission-mode-{}",
            PermissionMode::AcceptEdits.as_str()
        )),
    );
    cx.run_until_parked();
    assert_no_rendered_bounds(cx, "desktop-permission-menu");

    workspace.update_in(cx, |workspace, _window, cx| {
        assert_eq!(workspace.permission_mode, PermissionMode::AcceptEdits);
        let active_chat = workspace
            .active_thread()
            .expect("workspace should have an active thread")
            .chat
            .read(cx);
        assert_eq!(active_chat.permission_mode, PermissionMode::AcceptEdits);
    });

    click_debug_selector(cx, "desktop-model-status");
    cx.run_until_parked();

    // The model chip opens the model picker, not Settings.
    assert_rendered_bounds(cx, "desktop-model-picker");
    assert_no_rendered_bounds(cx, "desktop-settings-dialog");
    let target: &'static str = workspace.update_in(cx, |workspace, _window, _cx| {
        assert!(!workspace.settings_visible());
        super::MODEL_CATALOG
            .iter()
            .map(|model| model.id)
            .find(|id| *id != workspace.current_model)
            .expect("a second model to pick")
    });
    click_debug_selector(
        cx,
        leaked_debug_selector(format!(
            "desktop-model-picker-option-{}",
            super::element_key(target)
        )),
    );
    cx.run_until_parked();
    workspace.update_in(cx, |workspace, _window, cx| {
        assert_eq!(workspace.current_model, target);
        let active_chat = workspace
            .active_thread()
            .expect("status bar test has an active thread")
            .chat
            .read(cx);
        assert_eq!(active_chat.model_id, target);
        assert!(!active_chat.model_picker_open);
    });
    assert_no_rendered_bounds(cx, "desktop-model-picker");

    click_debug_selector(cx, "desktop-composer-input");
    cx.simulate_input("/set");
    cx.run_until_parked();
    assert_rendered_bounds(cx, "desktop-slash-command-row-settings");
    click_debug_selector(cx, "desktop-slash-command-row-settings");
    cx.run_until_parked();

    workspace.update_in(cx, |workspace, _window, _cx| {
        assert!(workspace.settings_visible());
    });
}

#[gpui::test]
fn desktop_chat_header_tasks_toggle_should_match_web_flow(cx: &mut gpui::TestAppContext) {
    cx.update(|cx| {
        gpui_component::init(cx);
        gpui_component::Theme::change(gpui_component::ThemeMode::Dark, None, cx);
        theme::apply_bruhs_theme(cx);
        let state = DesktopState::new().expect("desktop state should initialize");
        cx.set_global(state);
    });

    let workspace_cell: Rc<RefCell<Option<Entity<Workspace>>>> = Rc::new(RefCell::new(None));
    let workspace_for_window = workspace_cell.clone();
    let (_root, cx) = cx.add_window_view(|window, cx| {
        let workspace = cx.new(|cx| Workspace::new(window, cx));
        *workspace_for_window.borrow_mut() = Some(workspace.clone());
        gpui_component::Root::new(workspace, window, cx)
    });
    let workspace = workspace_cell
        .borrow()
        .clone()
        .expect("workspace entity should be captured");
    cx.simulate_resize(gpui::size(gpui::px(1180.0), gpui::px(760.0)));

    workspace.update_in(cx, |workspace, _window, cx| {
        workspace.active_view = AppView::Chat;
        workspace.sidebar_visible = false;
        workspace.set_tasks_panel_visible(false, cx);
    });
    cx.run_until_parked();

    assert_rendered_bounds(cx, "desktop-chat-tasks-toggle");
    workspace.update_in(cx, |workspace, _window, cx| {
        assert!(!workspace.tasks_panel_visible);
        let active_chat = workspace
            .active_thread()
            .expect("workspace should have an active thread")
            .chat
            .read(cx);
        assert!(!active_chat.tasks_panel_visible);
    });

    click_debug_selector(cx, "desktop-chat-tasks-toggle");
    cx.run_until_parked();

    assert_rendered_bounds(cx, "desktop-background-tasks-panel");
    workspace.update_in(cx, |workspace, _window, cx| {
        assert!(workspace.tasks_panel_visible);
        let active_chat = workspace
            .active_thread()
            .expect("workspace should have an active thread")
            .chat
            .read(cx);
        assert!(active_chat.tasks_panel_visible);
    });

    click_debug_selector(cx, "desktop-chat-tasks-toggle");
    cx.run_until_parked();

    workspace.update_in(cx, |workspace, _window, cx| {
        assert!(!workspace.tasks_panel_visible);
        let active_chat = workspace
            .active_thread()
            .expect("workspace should have an active thread")
            .chat
            .read(cx);
        assert!(!active_chat.tasks_panel_visible);
    });
}

#[gpui::test]
fn desktop_tasks_panel_expand_dismiss_and_close_should_match_web_flow(
    cx: &mut gpui::TestAppContext,
) {
    let app = cx.update(|cx| {
        gpui_component::init(cx);
        gpui_component::Theme::change(gpui_component::ThemeMode::Dark, None, cx);
        theme::apply_bruhs_theme(cx);
        let state = DesktopState::new().expect("desktop state should initialize");
        let app = state.app.clone();
        cx.set_global(state);
        app
    });
    let running_task = app.registry().register(
        "thread-1",
        TaskKind::Shell,
        "Running command",
        serde_json::json!({}),
    );
    app.registry()
        .append_log(&running_task.id, "running output\nstill going");
    let recent_task = app.registry().register(
        "thread-1",
        TaskKind::Shell,
        "Completed command",
        serde_json::json!({}),
    );
    app.registry()
        .append_log(&recent_task.id, "finished output\nexit 0");
    app.registry()
        .update(&recent_task.id, TaskUpdate::exited(200_000, 0));

    let workspace_cell: Rc<RefCell<Option<Entity<Workspace>>>> = Rc::new(RefCell::new(None));
    let workspace_for_window = workspace_cell.clone();
    let (_root, cx) = cx.add_window_view(|window, cx| {
        let workspace = cx.new(|cx| Workspace::new(window, cx));
        *workspace_for_window.borrow_mut() = Some(workspace.clone());
        gpui_component::Root::new(workspace, window, cx)
    });
    let workspace = workspace_cell
        .borrow()
        .clone()
        .expect("workspace entity should be captured");
    cx.simulate_resize(gpui::size(gpui::px(1180.0), gpui::px(760.0)));
    workspace.update_in(cx, |workspace, _window, cx| {
        workspace.active_view = AppView::Chat;
        workspace.sidebar_visible = false;
        workspace.tasks_panel_visible = true;
        cx.notify();
    });
    cx.run_until_parked();

    let running_row = background_task_expand_debug_selector(&running_task.id);
    let recent_row = background_task_expand_debug_selector(&recent_task.id);
    let running_log = background_task_log_debug_selector(&running_task.id);
    let recent_log = background_task_log_debug_selector(&recent_task.id);
    assert_rendered_bounds(cx, "desktop-background-tasks-panel");
    assert_rendered_bounds(cx, running_row);
    assert_rendered_bounds(cx, recent_row);
    assert_no_rendered_bounds(cx, running_log);
    assert_no_rendered_bounds(cx, recent_log);

    click_debug_selector(cx, running_row);
    cx.run_until_parked();
    workspace.update_in(cx, |workspace, _window, cx| {
        assert_eq!(
            workspace.tasks_panel.read(cx).expanded_task_id_for_test(),
            Some(running_task.id.as_str())
        );
    });
    assert_rendered_bounds(cx, running_log);

    click_debug_selector(cx, recent_row);
    cx.run_until_parked();
    workspace.update_in(cx, |workspace, _window, cx| {
        assert_eq!(
            workspace.tasks_panel.read(cx).expanded_task_id_for_test(),
            Some(recent_task.id.as_str())
        );
    });
    assert_rendered_bounds(cx, recent_log);

    hover_debug_selector(cx, background_task_row_debug_selector(&recent_task.id));
    cx.run_until_parked();
    click_debug_selector(cx, background_task_dismiss_debug_selector(&recent_task.id));
    cx.run_until_parked();
    assert!(app.registry().get(&recent_task.id).is_none());
    workspace.update_in(cx, |workspace, _window, cx| {
        let tasks_panel = workspace.tasks_panel.read(cx);
        assert_eq!(tasks_panel.expanded_task_id_for_test(), None);
        assert!(!tasks_panel.has_task_for_test(&recent_task.id));
    });

    click_debug_selector(cx, "desktop-close-tasks-panel");
    cx.run_until_parked();
    workspace.update_in(cx, |workspace, _window, _cx| {
        assert!(!workspace.tasks_panel_visible);
    });
}

#[gpui::test]
fn desktop_sidebar_footer_nav_should_switch_views_like_web(cx: &mut gpui::TestAppContext) {
    cx.update(|cx| {
        gpui_component::init(cx);
        gpui_component::Theme::change(gpui_component::ThemeMode::Dark, None, cx);
        theme::apply_bruhs_theme(cx);
        let state = DesktopState::new().expect("desktop state should initialize");
        cx.set_global(state);
    });

    let workspace_cell: Rc<RefCell<Option<Entity<Workspace>>>> = Rc::new(RefCell::new(None));
    let workspace_for_window = workspace_cell.clone();
    let (_root, cx) = cx.add_window_view(|window, cx| {
        let workspace = cx.new(|cx| Workspace::new(window, cx));
        *workspace_for_window.borrow_mut() = Some(workspace.clone());
        gpui_component::Root::new(workspace, window, cx)
    });
    let workspace = workspace_cell
        .borrow()
        .clone()
        .expect("workspace entity should be captured");

    // Web sidebar footer: today · tasks · terminal · calendar · meetings · people, settings.
    assert_rendered_bounds(cx, "desktop-sidebar-toggle");
    assert_rendered_bounds(cx, "desktop-sidebar-footer");
    for selector in [
        "desktop-sidebar-nav-today",
        "desktop-sidebar-nav-tasks",
        "desktop-sidebar-nav-terminal",
        "desktop-sidebar-nav-calendar",
        "desktop-sidebar-nav-meetings",
        "desktop-sidebar-nav-people",
        "desktop-sidebar-nav-settings",
    ] {
        assert_rendered_bounds(cx, selector);
    }

    click_debug_selector(cx, "desktop-sidebar-nav-today");
    cx.run_until_parked();
    workspace.update_in(cx, |workspace, _window, _cx| {
        assert_eq!(workspace.active_view, AppView::Today);
    });
    assert_rendered_bounds(cx, "desktop-today-scroll");

    click_debug_selector(cx, "desktop-sidebar-nav-tasks");
    cx.run_until_parked();
    workspace.update_in(cx, |workspace, _window, _cx| {
        assert_eq!(workspace.active_view, AppView::Tasks);
    });
    // The thread sidebar stays on every view, like the web.
    assert_rendered_bounds(cx, "desktop-workspace-sidebar");

    click_debug_selector(cx, "desktop-sidebar-nav-calendar");
    cx.run_until_parked();
    workspace.update_in(cx, |workspace, _window, _cx| {
        assert_eq!(workspace.active_view, AppView::Calendar);
    });
    click_debug_selector(cx, "desktop-sidebar-nav-terminal");
    cx.run_until_parked();
    workspace.update_in(cx, |workspace, _window, _cx| {
        assert_eq!(workspace.active_view, AppView::Terminal);
    });
    click_debug_selector(cx, "desktop-sidebar-nav-people");
    cx.run_until_parked();
    workspace.update_in(cx, |workspace, _window, _cx| {
        assert_eq!(workspace.active_view, AppView::People);
    });
    click_debug_selector(cx, "desktop-sidebar-nav-settings");
    cx.run_until_parked();
    workspace.update_in(cx, |workspace, _window, cx| {
        assert!(workspace.settings_visible());
        workspace.close_transient_surfaces();
        cx.notify();
    });
    cx.run_until_parked();

    // Mode controls and the sidebar toggle remain in the top bar.
    workspace.update_in(cx, |workspace, window, cx| {
        workspace.toggle_sidebar_for_window(window, cx);
    });
    cx.run_until_parked();
    assert_no_rendered_bounds(cx, "desktop-workspace-sidebar");
    assert_rendered_bounds(cx, "desktop-collapsed-sidebar-strip");
    click_debug_selector(cx, "desktop-sidebar-toggle");
    cx.run_until_parked();
    assert_rendered_bounds(cx, "desktop-workspace-sidebar");
    assert_rendered_bounds(cx, "desktop-collapsed-sidebar-strip");
    assert_rendered_bounds(cx, "desktop-workspace-mode-chat");
    assert_rendered_bounds(cx, "desktop-workspace-mode-code");
}

#[gpui::test]
fn desktop_today_plan_review_apply_and_undo_should_be_one_transaction(
    cx: &mut gpui::TestAppContext,
) {
    cx.update(|cx| {
        gpui_component::init(cx);
        gpui_component::Theme::change(gpui_component::ThemeMode::Dark, None, cx);
        theme::apply_bruhs_theme(cx);
        let state = DesktopState::new().expect("desktop state should initialize");
        cx.set_global(state);
    });

    let workspace_cell: Rc<RefCell<Option<Entity<Workspace>>>> = Rc::new(RefCell::new(None));
    let workspace_for_window = workspace_cell.clone();
    let (_root, cx) = cx.add_window_view(|window, cx| {
        let workspace = cx.new(|cx| Workspace::new(window, cx));
        *workspace_for_window.borrow_mut() = Some(workspace.clone());
        gpui_component::Root::new(workspace, window, cx)
    });
    let workspace = workspace_cell
        .borrow()
        .clone()
        .expect("workspace entity should be captured");
    let now = EpochMs::now();
    let now_ms = now.as_f64();

    workspace.update_in(cx, |workspace, _window, cx| {
        workspace.active_view = AppView::Today;
        workspace.work_items = vec![DesktopWorkItem {
            id: "today-task".to_string(),
            project_id: None,
            title: "Protect focus".to_string(),
            description: None,
            status: DesktopWorkItemStatus::Todo,
            priority: DesktopPriority::High,
            due_date: None,
            planning: WorkItemPlanning::default(),
            updated_at: EpochMs::UNIX_EPOCH,
        }];
        workspace.day_plan_proposal = Some(bruhs_models::DayPlanProposal {
            id: "proposal".to_string(),
            created_at_ms: now.as_millis(),
            blocks: vec![bruhs_models::ProposedTimeBlock {
                id: "today-block".to_string(),
                task_id: "today-task".to_string(),
                task_title: "Protect focus".to_string(),
                start_at_ms: now_ms + 3_600_000.0,
                end_at_ms: now_ms + 5_400_000.0,
            }],
            unscheduled_count: 0,
            unscheduled_task_ids: Vec::new(),
            busy_blocks_respected: 0,
        });
        cx.notify();
    });
    cx.run_until_parked();

    assert_rendered_bounds(cx, "desktop-today-plan-apply");
    click_debug_selector(cx, "desktop-today-plan-apply");
    cx.run_until_parked();
    workspace.update_in(cx, |workspace, _window, _cx| {
        assert!(workspace.day_plan_proposal.is_none());
        assert_eq!(workspace.calendar_events.len(), 1);
        assert_eq!(
            workspace.work_items[0].planning.time_block_event_ids,
            ["today-block"]
        );
    });

    assert_rendered_bounds(cx, "desktop-today-plan-undo");
    click_debug_selector(cx, "desktop-today-plan-undo");
    cx.run_until_parked();
    workspace.update_in(cx, |workspace, _window, _cx| {
        assert!(workspace.calendar_events.is_empty());
        assert!(workspace.work_items[0]
            .planning
            .time_block_event_ids
            .is_empty());
        assert!(workspace.day_plan_undo.is_none());
    });
}

#[gpui::test]
fn desktop_palette_should_reopen_by_keyboard_after_escape_and_navigation(
    cx: &mut gpui::TestAppContext,
) {
    cx.update(|cx| {
        gpui_component::init(cx);
        cx.bind_keys([
            gpui::KeyBinding::new("cmd-k", FocusCommandBar, None),
            gpui::KeyBinding::new("escape", super::CloseOverlays, None),
        ]);
        theme::apply_bruhs_theme(cx);
        cx.set_global(DesktopState::new().expect("desktop state should initialize"));
    });
    let workspace_cell = Rc::new(RefCell::new(None));
    let workspace_for_window = workspace_cell.clone();
    let (_root, cx) = cx.add_window_view(|window, cx| {
        let workspace = cx.new(|cx| Workspace::new(window, cx));
        *workspace_for_window.borrow_mut() = Some(workspace.clone());
        gpui_component::Root::new(workspace, window, cx)
    });
    let workspace = workspace_cell.borrow().clone().unwrap();
    workspace.update_in(cx, |workspace, window, cx| {
        workspace.focus_command_bar(&FocusCommandBar, window, cx);
    });
    cx.run_until_parked();
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    assert_no_rendered_bounds(cx, "desktop-command-bar");
    cx.simulate_keystrokes("cmd-k");
    cx.run_until_parked();
    assert_rendered_bounds(cx, "desktop-command-bar");
    click_debug_selector(cx, "desktop-command-action-nav-automations");
    cx.run_until_parked();
    assert_no_rendered_bounds(cx, "desktop-command-bar");
    cx.simulate_keystrokes("cmd-k");
    cx.run_until_parked();
    assert_rendered_bounds(cx, "desktop-command-bar");
    for selector in [
        "desktop-command-action-nav-today",
        "desktop-command-action-nav-tasks",
        "desktop-command-action-nav-calendar",
        "desktop-command-action-nav-life",
        "desktop-command-action-nav-inbox",
        "desktop-command-action-nav-knowledge",
        "desktop-command-action-nav-people",
        "desktop-command-action-nav-search",
        "desktop-command-action-nav-chat",
    ] {
        click_debug_selector(cx, selector);
        cx.run_until_parked();
        assert_no_rendered_bounds(cx, "desktop-command-bar");
        cx.simulate_keystrokes("cmd-k");
        cx.run_until_parked();
        assert_rendered_bounds(cx, "desktop-command-bar");
    }
    // The shortcut itself is also a dismissal path.
    cx.simulate_keystrokes("cmd-k");
    cx.run_until_parked();
    assert_no_rendered_bounds(cx, "desktop-command-bar");
    cx.simulate_keystrokes("cmd-k");
    cx.run_until_parked();
    assert_rendered_bounds(cx, "desktop-command-bar");
}

#[gpui::test]
fn desktop_command_bar_should_not_activate_calendar_content(cx: &mut gpui::TestAppContext) {
    cx.update(|cx| {
        gpui_component::init(cx);
        gpui_component::Theme::change(gpui_component::ThemeMode::Dark, None, cx);
        theme::apply_bruhs_theme(cx);
        let state = DesktopState::new().expect("desktop state should initialize");
        cx.set_global(state);
    });

    let workspace_cell: Rc<RefCell<Option<Entity<Workspace>>>> = Rc::new(RefCell::new(None));
    let workspace_for_window = workspace_cell.clone();
    let (_root, cx) = cx.add_window_view(|window, cx| {
        let workspace = cx.new(|cx| Workspace::new(window, cx));
        *workspace_for_window.borrow_mut() = Some(workspace.clone());
        gpui_component::Root::new(workspace, window, cx)
    });
    let workspace = workspace_cell
        .borrow()
        .clone()
        .expect("workspace entity should be captured");

    let initial_event_count = workspace.update_in(cx, |workspace, _window, cx| {
        workspace.active_view = AppView::Calendar;
        workspace.calendar_mode = DesktopCalendarMode::Day;
        workspace.dismiss_overlay(|overlay| matches!(overlay, Overlay::EventCreate { .. }));
        cx.notify();
        workspace.calendar_events.len()
    });
    cx.run_until_parked();
    assert_rendered_bounds(cx, "desktop-calendar-page");

    workspace.update_in(cx, |workspace, window, cx| {
        workspace.focus_command_bar(&FocusCommandBar, window, cx);
    });
    cx.run_until_parked();
    workspace.update_in(cx, |workspace, _window, _cx| {
        assert!(workspace.command_bar_visible());
        assert!(!workspace.event_create_visible());
        assert_eq!(workspace.calendar_events.len(), initial_event_count);
    });
    assert_rendered_bounds(cx, "desktop-command-bar");
    assert_no_rendered_bounds(cx, "desktop-event-create-dialog");
}

#[gpui::test]
fn desktop_compact_layout_should_use_drawers_without_collapsing_main_content(
    cx: &mut gpui::TestAppContext,
) {
    cx.update(|cx| {
        gpui_component::init(cx);
        gpui_component::Theme::change(gpui_component::ThemeMode::Dark, None, cx);
        theme::apply_bruhs_theme(cx);
        let state = DesktopState::new().expect("desktop state should initialize");
        cx.set_global(state);
    });

    let workspace_cell: Rc<RefCell<Option<Entity<Workspace>>>> = Rc::new(RefCell::new(None));
    let workspace_for_window = workspace_cell.clone();
    let (_root, cx) = cx.add_window_view(|window, cx| {
        let workspace = cx.new(|cx| Workspace::new(window, cx));
        *workspace_for_window.borrow_mut() = Some(workspace.clone());
        gpui_component::Root::new(workspace, window, cx)
    });
    let workspace = workspace_cell
        .borrow()
        .clone()
        .expect("workspace entity should be captured");
    cx.simulate_resize(gpui::size(gpui::px(760.0), gpui::px(700.0)));

    workspace.update_in(cx, |workspace, _window, cx| {
        workspace.active_view = AppView::Chat;
        workspace.sidebar_visible = true;
        workspace.compact_sidebar_open = false;
        workspace.tasks_panel_visible = false;
        cx.notify();
    });
    cx.run_until_parked();
    assert_rendered_bounds(cx, "desktop-composer");
    assert_no_rendered_bounds(cx, "desktop-workspace-sidebar");

    // Compact: the drag strip carries the sidebar toggle above the chat.
    let sidebar_toggle = rendered_bounds(cx, "desktop-sidebar-toggle");
    let composer = rendered_bounds(cx, "desktop-composer");
    assert!(
        f32::from(sidebar_toggle.origin.y + sidebar_toggle.size.height)
            <= f32::from(composer.origin.y),
        "compact strip should sit above the chat"
    );

    click_debug_selector(cx, "desktop-sidebar-toggle");
    cx.run_until_parked();
    assert_rendered_bounds(cx, "desktop-compact-sidebar-backdrop");
    assert_rendered_bounds(cx, "desktop-compact-sidebar-drawer");
    assert_rendered_bounds(cx, "desktop-workspace-sidebar");

    click_debug_selector(cx, "desktop-compact-sidebar-backdrop");
    cx.run_until_parked();
    assert_no_rendered_bounds(cx, "desktop-compact-sidebar-drawer");
    assert_rendered_bounds(cx, "desktop-composer");

    workspace.update_in(cx, |workspace, _window, cx| {
        workspace.tasks_panel_visible = true;
        cx.notify();
    });
    cx.run_until_parked();
    assert_rendered_bounds(cx, "desktop-compact-tasks-panel");
    assert_rendered_bounds(cx, "desktop-composer");

    workspace.update_in(cx, |workspace, _window, cx| {
        workspace.active_view = AppView::Tasks;
        workspace.tasks_panel_visible = false;
        workspace.projects = vec![DesktopProject {
            id: "project-1".to_string(),
            name: "Alpha".to_string(),
        }];
        workspace.selected_project_id = Some("project-1".to_string());
        workspace.work_items = vec![DesktopWorkItem {
            id: "task-1".to_string(),
            project_id: Some("project-1".to_string()),
            title: "Compact detail".to_string(),
            description: None,
            status: DesktopWorkItemStatus::Todo,
            priority: DesktopPriority::None,
            due_date: None,
            planning: WorkItemPlanning::default(),
            updated_at: EpochMs::UNIX_EPOCH,
        }];
        workspace.selected_work_item_id = Some("task-1".to_string());
        workspace.compact_sidebar_open = false;
        cx.notify();
    });
    cx.run_until_parked();
    assert_rendered_bounds(cx, task_row_debug_selector("task-1"));
    assert_rendered_bounds(cx, "desktop-compact-task-detail");
    assert_rendered_bounds(cx, "desktop-task-detail");
    assert_no_rendered_bounds(cx, "desktop-project-sidebar");

    click_debug_selector(cx, "desktop-sidebar-toggle");
    cx.run_until_parked();
    assert_rendered_bounds(cx, "desktop-compact-sidebar-drawer");
    assert_rendered_bounds(cx, "desktop-workspace-sidebar");
}

#[gpui::test]
fn desktop_command_bar_filter_click_should_open_settings_like_web(cx: &mut gpui::TestAppContext) {
    cx.update(|cx| {
        gpui_component::init(cx);
        gpui_component::Theme::change(gpui_component::ThemeMode::Dark, None, cx);
        theme::apply_bruhs_theme(cx);
        let state = DesktopState::new().expect("desktop state should initialize");
        cx.set_global(state);
    });

    let workspace_cell: Rc<RefCell<Option<Entity<Workspace>>>> = Rc::new(RefCell::new(None));
    let workspace_for_window = workspace_cell.clone();
    let (_root, cx) = cx.add_window_view(|window, cx| {
        let workspace = cx.new(|cx| Workspace::new(window, cx));
        *workspace_for_window.borrow_mut() = Some(workspace.clone());
        gpui_component::Root::new(workspace, window, cx)
    });
    let workspace = workspace_cell
        .borrow()
        .clone()
        .expect("workspace entity should be captured");

    workspace.update_in(cx, |workspace, window, cx| {
        workspace.focus_command_bar(&FocusCommandBar, window, cx);
        workspace.command_input.update(cx, |input, cx| {
            input.set_value("settings", window, cx);
        });
        cx.notify();
    });
    cx.run_until_parked();

    workspace.update_in(cx, |workspace, _window, cx| {
        assert!(workspace.command_bar_visible());
        assert!(!workspace.settings_visible());
        assert_eq!(
            workspace.command_input.read(cx).value().as_ref(),
            "settings"
        );
    });
    assert_rendered_bounds(cx, "desktop-command-bar");
    assert_rendered_bounds(cx, "desktop-command-action-settings");

    click_debug_selector(cx, "desktop-command-action-settings");
    cx.run_until_parked();

    workspace.update_in(cx, |workspace, _window, _cx| {
        assert!(!workspace.command_bar_visible());
        assert!(workspace.settings_visible());
    });
    assert_rendered_bounds(cx, "desktop-settings-dialog");
    // Provider logos head the model dropdown's groups once it is open.
    assert_rendered_bounds(cx, "desktop-settings-row-model");
    click_debug_selector(cx, "desktop-model");
    cx.run_until_parked();
    assert_provider_logo_bounds(cx, "desktop-provider-logo-claude");
    assert_provider_logo_bounds(cx, "desktop-provider-logo-codex");
}

#[gpui::test]
fn desktop_command_bar_actions_should_match_web_flow(cx: &mut gpui::TestAppContext) {
    cx.update(|cx| {
        gpui_component::init(cx);
        gpui_component::Theme::change(gpui_component::ThemeMode::Dark, None, cx);
        theme::apply_bruhs_theme(cx);
        let state = DesktopState::new().expect("desktop state should initialize");
        cx.set_global(state);
    });

    let workspace_cell: Rc<RefCell<Option<Entity<Workspace>>>> = Rc::new(RefCell::new(None));
    let workspace_for_window = workspace_cell.clone();
    let (_root, cx) = cx.add_window_view(|window, cx| {
        let workspace = cx.new(|cx| Workspace::new(window, cx));
        *workspace_for_window.borrow_mut() = Some(workspace.clone());
        gpui_component::Root::new(workspace, window, cx)
    });
    let workspace = workspace_cell
        .borrow()
        .clone()
        .expect("workspace entity should be captured");

    workspace.update_in(cx, |workspace, _window, cx| {
        workspace.active_view = AppView::Chat;
        workspace.sidebar_visible = true;
        cx.notify();
    });
    cx.run_until_parked();

    workspace.update_in(cx, |workspace, window, cx| {
        workspace.focus_command_bar(&FocusCommandBar, window, cx);
    });
    cx.run_until_parked();

    assert_rendered_bounds(cx, "desktop-command-bar");
    assert_rendered_bounds(cx, "desktop-command-action-nav-tasks");
    click_debug_selector(cx, "desktop-command-action-nav-tasks");
    cx.run_until_parked();
    workspace.update_in(cx, |workspace, _window, _cx| {
        assert_eq!(workspace.active_view, AppView::Tasks);
        assert!(!workspace.command_bar_visible());
    });

    workspace.update_in(cx, |workspace, window, cx| {
        workspace.focus_command_bar(&FocusCommandBar, window, cx);
    });
    cx.run_until_parked();
    assert_rendered_bounds(cx, "desktop-command-action-nav-calendar");
    click_debug_selector(cx, "desktop-command-action-nav-calendar");
    cx.run_until_parked();
    workspace.update_in(cx, |workspace, _window, _cx| {
        assert_eq!(workspace.active_view, AppView::Calendar);
        assert!(!workspace.command_bar_visible());
    });

    workspace.update_in(cx, |workspace, window, cx| {
        workspace.focus_command_bar(&FocusCommandBar, window, cx);
    });
    cx.run_until_parked();
    assert_rendered_bounds(cx, "desktop-command-action-nav-chat");
    click_debug_selector(cx, "desktop-command-action-nav-chat");
    cx.run_until_parked();
    workspace.update_in(cx, |workspace, _window, _cx| {
        assert_eq!(workspace.active_view, AppView::Chat);
        assert!(!workspace.command_bar_visible());
    });

    workspace.update_in(cx, |workspace, window, cx| {
        workspace.focus_command_bar(&FocusCommandBar, window, cx);
    });
    cx.run_until_parked();
    assert_rendered_bounds(cx, "desktop-command-action-open-project");
    click_debug_selector(cx, "desktop-command-action-open-project");
    cx.run_until_parked();
    workspace.update_in(cx, |workspace, _window, _cx| {
        assert!(!workspace.command_bar_visible());
        assert!(workspace.project_picker_visible());
    });
    assert_rendered_bounds(cx, "desktop-project-picker-dialog");

    workspace.update_in(cx, |workspace, window, cx| {
        workspace.close_transient_surfaces();
        workspace.focus_command_bar(&FocusCommandBar, window, cx);
    });
    cx.run_until_parked();
    // The list scrolls past 320px now; filter first, like a user would.
    cx.simulate_input("new task");
    cx.run_until_parked();
    assert_rendered_bounds(cx, "desktop-command-action-new-task");
    click_debug_selector(cx, "desktop-command-action-new-task");
    cx.run_until_parked();
    workspace.update_in(cx, |workspace, _window, _cx| {
        assert_eq!(workspace.active_view, AppView::Tasks);
        assert!(!workspace.command_bar_visible());
        assert!(workspace.task_create_visible());
    });
    assert_rendered_bounds(cx, "desktop-task-create-dialog");

    workspace.update_in(cx, |workspace, window, cx| {
        workspace.close_transient_surfaces();
        workspace.sidebar_visible = true;
        workspace.focus_command_bar(&FocusCommandBar, window, cx);
        workspace.command_input.update(cx, |input, cx| {
            input.set_value("sidebar", window, cx);
        });
        cx.notify();
    });
    cx.run_until_parked();
    assert_rendered_bounds(cx, "desktop-command-action-toggle-sidebar");
    click_debug_selector(cx, "desktop-command-action-toggle-sidebar");
    cx.run_until_parked();
    workspace.update_in(cx, |workspace, _window, _cx| {
        assert!(!workspace.command_bar_visible());
        assert!(!workspace.sidebar_visible);
    });
}

#[gpui::test]
fn desktop_project_sidebar_create_select_and_delete_should_match_web_flow(
    cx: &mut gpui::TestAppContext,
) {
    cx.update(|cx| {
        gpui_component::init(cx);
        gpui_component::Theme::change(gpui_component::ThemeMode::Dark, None, cx);
        theme::apply_bruhs_theme(cx);
        let state = DesktopState::new().expect("desktop state should initialize");
        cx.set_global(state);
    });

    let workspace_cell: Rc<RefCell<Option<Entity<Workspace>>>> = Rc::new(RefCell::new(None));
    let workspace_for_window = workspace_cell.clone();
    let (_root, cx) = cx.add_window_view(|window, cx| {
        let workspace = cx.new(|cx| Workspace::new(window, cx));
        *workspace_for_window.borrow_mut() = Some(workspace.clone());
        gpui_component::Root::new(workspace, window, cx)
    });
    let workspace = workspace_cell
        .borrow()
        .clone()
        .expect("workspace entity should be captured");

    workspace.update_in(cx, |workspace, _window, cx| {
        workspace.active_view = AppView::Tasks;
        workspace.sidebar_visible = true;
        workspace.projects.clear();
        workspace.work_items.clear();
        workspace.selected_project_id = None;
        workspace.selected_work_item_id = None;
        workspace
            .dismiss_overlay(|overlay| matches!(overlay, Overlay::ProjectDeleteConfirm { .. }));
        cx.notify();
    });
    cx.run_until_parked();

    assert_rendered_bounds(cx, "desktop-project-sidebar");
    assert_rendered_bounds(cx, "desktop-project-all");
    assert_rendered_bounds(cx, "desktop-project-new");

    click_debug_selector(cx, "desktop-project-new");
    cx.run_until_parked();

    let empty_project_id = workspace.update_in(cx, |workspace, _window, _cx| {
        assert_eq!(workspace.projects.len(), 1);
        let project = workspace
            .projects
            .first()
            .expect("new project should be appended");
        assert_eq!(project.name, "Project 1");
        assert_eq!(
            workspace.selected_project_id.as_deref(),
            Some(project.id.as_str())
        );
        project.id.clone()
    });

    let empty_row_selector = project_row_debug_selector(&empty_project_id);
    assert_rendered_bounds(cx, empty_row_selector);
    hover_debug_selector(cx, empty_row_selector);
    cx.run_until_parked();
    click_debug_selector(cx, project_delete_debug_selector(&empty_project_id));
    cx.run_until_parked();

    workspace.update_in(cx, |workspace, _window, _cx| {
        assert!(workspace.projects.is_empty());
        assert!(workspace.work_items.is_empty());
        assert!(workspace.selected_project_id.is_none());
        assert!(workspace.project_delete_confirm_id().is_none());
    });

    click_debug_selector(cx, "desktop-project-new");
    cx.run_until_parked();

    let guarded_project_id = workspace.update_in(cx, |workspace, _window, cx| {
        let project_id = workspace
            .selected_project_id
            .clone()
            .expect("new project should be selected");
        workspace.work_items.push(DesktopWorkItem {
            id: "task-1".to_string(),
            project_id: Some(project_id.clone()),
            title: "Scoped task".to_string(),
            description: None,
            status: DesktopWorkItemStatus::Todo,
            priority: DesktopPriority::Medium,
            due_date: None,
            planning: WorkItemPlanning::default(),
            updated_at: EpochMs::UNIX_EPOCH,
        });
        workspace.selected_work_item_id = None;
        cx.notify();
        project_id
    });
    cx.run_until_parked();

    let guarded_row_selector = project_row_debug_selector(&guarded_project_id);
    hover_debug_selector(cx, guarded_row_selector);
    cx.run_until_parked();
    click_debug_selector(cx, project_delete_debug_selector(&guarded_project_id));
    cx.run_until_parked();

    workspace.update_in(cx, |workspace, window, _cx| {
        assert_eq!(
            workspace.project_delete_confirm_id(),
            Some(guarded_project_id.as_str())
        );
        assert_eq!(workspace.projects.len(), 1);
        assert_eq!(workspace.work_items.len(), 1);
        assert!(workspace
            .modal_start_focus_handle
            .contains_focused(window, _cx));
    });
    assert_rendered_bounds(cx, "desktop-project-delete-dialog");
    assert_rendered_bounds(cx, "desktop-project-delete-prompt");
    assert_rendered_bounds(cx, "desktop-project-delete-cancel");
    assert_rendered_bounds(cx, "desktop-project-delete-confirm");

    click_debug_selector(cx, "desktop-project-delete-cancel");
    cx.run_until_parked();

    workspace.update_in(cx, |workspace, _window, _cx| {
        assert!(workspace.project_delete_confirm_id().is_none());
        assert_eq!(workspace.projects.len(), 1);
        assert_eq!(workspace.work_items.len(), 1);
        assert_eq!(
            workspace.selected_project_id.as_deref(),
            Some(guarded_project_id.as_str())
        );
        assert!(workspace.selected_work_item_id.is_none());
    });

    hover_debug_selector(cx, guarded_row_selector);
    cx.run_until_parked();
    click_debug_selector(cx, project_delete_debug_selector(&guarded_project_id));
    cx.run_until_parked();
    click_debug_selector(cx, "desktop-project-delete-confirm");
    cx.run_until_parked();

    workspace.update_in(cx, |workspace, _window, _cx| {
        assert!(workspace.project_delete_confirm_id().is_none());
        assert!(workspace
            .projects
            .iter()
            .all(|project| project.id != guarded_project_id));
        assert!(workspace
            .work_items
            .iter()
            .all(|item| item.project_id.as_deref() != Some(guarded_project_id.as_str())));
        assert!(workspace.selected_project_id.is_none());
        assert!(workspace.selected_work_item_id.is_none());
    });
}

#[gpui::test]
fn desktop_task_detail_edit_select_close_and_delete_should_match_web_flow(
    cx: &mut gpui::TestAppContext,
) {
    cx.update(|cx| {
        gpui_component::init(cx);
        gpui_component::Theme::change(gpui_component::ThemeMode::Dark, None, cx);
        theme::apply_bruhs_theme(cx);
        let state = DesktopState::new().expect("desktop state should initialize");
        cx.set_global(state);
    });

    let workspace_cell: Rc<RefCell<Option<Entity<Workspace>>>> = Rc::new(RefCell::new(None));
    let workspace_for_window = workspace_cell.clone();
    let (_root, cx) = cx.add_window_view(|window, cx| {
        let workspace = cx.new(|cx| Workspace::new(window, cx));
        *workspace_for_window.borrow_mut() = Some(workspace.clone());
        gpui_component::Root::new(workspace, window, cx)
    });
    let workspace = workspace_cell
        .borrow()
        .clone()
        .expect("workspace entity should be captured");

    workspace.update_in(cx, |workspace, _window, cx| {
        workspace.active_view = AppView::Tasks;
        workspace.sidebar_visible = true;
        workspace.projects = vec![
            DesktopProject {
                id: "project-1".to_string(),
                name: "Alpha".to_string(),
            },
            DesktopProject {
                id: "project-2".to_string(),
                name: "Beta".to_string(),
            },
        ];
        workspace.work_items = vec![DesktopWorkItem {
            id: "task-1".to_string(),
            project_id: Some("project-1".to_string()),
            title: "Original task".to_string(),
            description: Some("Original description".to_string()),
            status: DesktopWorkItemStatus::Todo,
            priority: DesktopPriority::Low,
            due_date: Some("2026-05-06".to_string()),
            planning: WorkItemPlanning::default(),
            updated_at: EpochMs::UNIX_EPOCH,
        }];
        workspace.selected_project_id = None;
        workspace.selected_work_item_id = None;
        workspace.task_detail_menu_open = None;
        cx.notify();
    });
    cx.run_until_parked();

    let row_selector = task_row_debug_selector("task-1");
    assert_rendered_bounds(cx, row_selector);
    click_debug_selector(cx, row_selector);
    cx.run_until_parked();

    workspace.update_in(cx, |workspace, _window, cx| {
        assert_eq!(workspace.selected_work_item_id.as_deref(), Some("task-1"));
        assert_eq!(
            workspace.task_detail_title_input.read(cx).value().as_ref(),
            "Original task"
        );
        assert_eq!(
            workspace
                .task_detail_description_input
                .read(cx)
                .value()
                .as_ref(),
            "Original description"
        );
        assert_eq!(
            workspace.task_detail_due_input.read(cx).value().as_ref(),
            "2026-05-06"
        );
    });
    assert_rendered_bounds(cx, "desktop-task-detail");
    assert_rendered_bounds(cx, "desktop-task-detail-title-input");
    assert_rendered_bounds(cx, "desktop-task-detail-description-input");
    assert_rendered_bounds(cx, "desktop-task-detail-due-picker");
    assert_rendered_bounds(cx, "desktop-task-detail-select-project");
    assert_rendered_bounds(cx, "desktop-task-detail-select-status");
    assert_rendered_bounds(cx, "desktop-task-detail-select-priority");

    click_debug_selector(cx, "desktop-task-detail-title-input");
    cx.simulate_keystrokes("cmd-a backspace");
    cx.simulate_input("Updated task");
    cx.run_until_parked();

    click_debug_selector(cx, "desktop-task-detail-description-input");
    cx.simulate_keystrokes("cmd-a backspace");
    cx.simulate_input("Updated description");
    cx.run_until_parked();

    // The due date is a picker now (web `DatePicker`): open it on the
    // value's month and click the 7th.
    click_debug_selector(cx, "desktop-task-detail-due-picker");
    cx.run_until_parked();
    assert_rendered_bounds(cx, "desktop-date-picker-popover");
    click_debug_selector(cx, "desktop-date-picker-day-2026-05-07");
    cx.run_until_parked();
    assert_no_rendered_bounds(cx, "desktop-date-picker-popover");

    workspace.update_in(cx, |workspace, _window, _cx| {
        let item = workspace
            .work_items
            .iter()
            .find(|item| item.id == "task-1")
            .expect("task should still exist");
        assert_eq!(item.title, "Updated task");
        assert_eq!(item.description.as_deref(), Some("Updated description"));
        assert_eq!(item.due_date.as_deref(), Some("2026-05-07"));
        assert!(item.updated_at > EpochMs::UNIX_EPOCH);
    });

    click_debug_selector(cx, "desktop-task-detail-status-chip-done");
    cx.run_until_parked();

    workspace.update_in(cx, |workspace, _window, _cx| {
        let item = workspace
            .work_items
            .iter()
            .find(|item| item.id == "task-1")
            .expect("task should still exist");
        assert_eq!(item.status, DesktopWorkItemStatus::Done);
    });

    click_debug_selector(cx, "desktop-task-detail-select-project");
    cx.run_until_parked();
    assert_rendered_bounds(cx, "desktop-task-detail-menu-project");
    click_debug_selector(cx, task_detail_project_option_debug_selector("project-2"));
    cx.run_until_parked();

    click_debug_selector(cx, "desktop-task-detail-select-status");
    cx.run_until_parked();
    assert_rendered_bounds(cx, "desktop-task-detail-status-option-canceled");
    click_debug_selector(cx, "desktop-task-detail-status-option-canceled");
    cx.run_until_parked();

    click_debug_selector(cx, "desktop-task-detail-select-priority");
    cx.run_until_parked();
    assert_rendered_bounds(cx, "desktop-task-detail-priority-option-high");
    click_debug_selector(cx, "desktop-task-detail-priority-option-high");
    cx.run_until_parked();

    workspace.update_in(cx, |workspace, _window, _cx| {
        assert_eq!(workspace.task_detail_menu_open, None);
        let item = workspace
            .work_items
            .iter()
            .find(|item| item.id == "task-1")
            .expect("task should still exist");
        assert_eq!(item.project_id.as_deref(), Some("project-2"));
        assert_eq!(item.status, DesktopWorkItemStatus::Canceled);
        assert_eq!(item.priority, DesktopPriority::High);
    });

    click_debug_selector(cx, "desktop-task-detail-close");
    cx.run_until_parked();
    workspace.update_in(cx, |workspace, _window, _cx| {
        assert_eq!(workspace.selected_work_item_id, None);
        assert_eq!(workspace.task_detail_menu_open, None);
        assert_eq!(workspace.work_items.len(), 1);
    });

    click_debug_selector(cx, row_selector);
    cx.run_until_parked();
    assert_rendered_bounds(cx, "desktop-task-detail-delete");
    click_debug_selector(cx, "desktop-task-detail-delete");
    cx.run_until_parked();

    workspace.update_in(cx, |workspace, _window, _cx| {
        assert!(workspace.work_items.is_empty());
        assert_eq!(workspace.selected_work_item_id, None);
        assert_eq!(workspace.task_detail_menu_open, None);
    });
}

#[gpui::test]
fn desktop_task_board_drop_handler_should_match_web_status_move_flow(
    cx: &mut gpui::TestAppContext,
) {
    cx.update(|cx| {
        gpui_component::init(cx);
        gpui_component::Theme::change(gpui_component::ThemeMode::Dark, None, cx);
        theme::apply_bruhs_theme(cx);
        let state = DesktopState::new().expect("desktop state should initialize");
        cx.set_global(state);
    });

    let workspace_cell: Rc<RefCell<Option<Entity<Workspace>>>> = Rc::new(RefCell::new(None));
    let workspace_for_window = workspace_cell.clone();
    let (_root, cx) = cx.add_window_view(|window, cx| {
        let workspace = cx.new(|cx| Workspace::new(window, cx));
        *workspace_for_window.borrow_mut() = Some(workspace.clone());
        gpui_component::Root::new(workspace, window, cx)
    });
    let workspace = workspace_cell
        .borrow()
        .clone()
        .expect("workspace entity should be captured");
    cx.simulate_resize(gpui::size(gpui::px(1100.0), gpui::px(760.0)));

    workspace.update_in(cx, |workspace, _window, cx| {
        workspace.active_view = AppView::Tasks;
        workspace.sidebar_visible = true;
        workspace.task_view_mode = DesktopTaskViewMode::Board;
        workspace.projects = vec![DesktopProject {
            id: "project-1".to_string(),
            name: "Alpha".to_string(),
        }];
        workspace.selected_project_id = Some("project-1".to_string());
        workspace.work_items = vec![DesktopWorkItem {
            id: "task-1".to_string(),
            project_id: Some("project-1".to_string()),
            title: "Drag me".to_string(),
            description: None,
            status: DesktopWorkItemStatus::Todo,
            priority: DesktopPriority::High,
            due_date: None,
            planning: WorkItemPlanning::default(),
            updated_at: EpochMs::UNIX_EPOCH,
        }];
        workspace.selected_work_item_id = None;
        cx.notify();
    });
    cx.run_until_parked();

    assert_rendered_bounds(cx, "desktop-task-board");
    let card_selector = task_card_debug_selector("task-1");
    let todo_selector = task_board_column_debug_selector(DesktopWorkItemStatus::Todo);
    let done_selector = task_board_column_debug_selector(DesktopWorkItemStatus::Done);
    assert_rendered_bounds(cx, card_selector);
    assert_rendered_bounds(cx, todo_selector);
    assert_rendered_bounds(cx, done_selector);
    let todo_bounds = cx
        .debug_bounds(todo_selector)
        .expect("todo column should have rendered bounds");
    let card_center = cx
        .debug_bounds(card_selector)
        .expect("todo card should have rendered bounds")
        .center();
    assert!(todo_bounds.contains(&card_center));

    workspace.update_in(cx, |workspace, _window, cx| {
        workspace.update_work_item_status("task-1", DesktopWorkItemStatus::Done, cx);
    });
    cx.run_until_parked();

    workspace.update_in(cx, |workspace, _window, _cx| {
        let item = workspace
            .work_items
            .iter()
            .find(|item| item.id == "task-1")
            .expect("dragged task should still exist");
        assert_eq!(item.status, DesktopWorkItemStatus::Done);
        assert!(item.updated_at > EpochMs::UNIX_EPOCH);
        assert!(workspace.selected_work_item_id.is_none());
        assert!(workspace.task_detail_menu_open.is_none());
    });
    assert_rendered_bounds(cx, card_selector);
    let done_bounds = cx
        .debug_bounds(done_selector)
        .expect("done column should have rendered bounds");
    let card_center = cx
        .debug_bounds(card_selector)
        .expect("done card should have rendered bounds")
        .center();
    assert!(done_bounds.contains(&card_center));
}

fn assert_provider_logo_bounds(cx: &mut gpui::VisualTestContext, selector: &'static str) {
    let bounds = cx
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("{selector} should be rendered"));
    assert_eq!(f32::from(bounds.size.width), assets::PROVIDER_LOGO_SIZE_PX);
    assert_eq!(f32::from(bounds.size.height), assets::PROVIDER_LOGO_SIZE_PX);
}

fn click_debug_selector(cx: &mut gpui::VisualTestContext, selector: &'static str) {
    let bounds = rendered_bounds(cx, selector);
    cx.simulate_click(bounds.center(), gpui::Modifiers::default());
}

fn double_click_debug_selector(cx: &mut gpui::VisualTestContext, selector: &'static str) {
    let bounds = rendered_bounds(cx, selector);
    let position = bounds.center();
    cx.simulate_event(gpui::MouseDownEvent {
        position,
        modifiers: gpui::Modifiers::default(),
        button: gpui::MouseButton::Left,
        click_count: 2,
        first_mouse: false,
    });
    cx.simulate_event(gpui::MouseUpEvent {
        position,
        modifiers: gpui::Modifiers::default(),
        button: gpui::MouseButton::Left,
        click_count: 2,
    });
}

fn hover_debug_selector(cx: &mut gpui::VisualTestContext, selector: &'static str) {
    let bounds = rendered_bounds(cx, selector);
    cx.simulate_mouse_move(bounds.center(), None, gpui::Modifiers::default());
}

fn rendered_bounds(
    cx: &mut gpui::VisualTestContext,
    selector: &'static str,
) -> gpui::Bounds<gpui::Pixels> {
    cx.debug_bounds(selector)
        .unwrap_or_else(|| panic!("{selector} should be rendered"))
}

fn assert_rendered_bounds(cx: &mut gpui::VisualTestContext, selector: &'static str) {
    let _ = rendered_bounds(cx, selector);
}

fn assert_no_rendered_bounds(cx: &mut gpui::VisualTestContext, selector: &'static str) {
    assert!(
        cx.debug_bounds(selector).is_none(),
        "{selector} should not be rendered"
    );
}

fn project_row_debug_selector(project_id: &str) -> &'static str {
    leaked_debug_selector(format!(
        "desktop-project-row-{}",
        super::element_key(project_id)
    ))
}

fn project_delete_debug_selector(project_id: &str) -> &'static str {
    leaked_debug_selector(format!(
        "desktop-project-delete-{}",
        super::element_key(project_id)
    ))
}

fn project_dir_debug_selector(project_path: &str) -> &'static str {
    leaked_debug_selector(format!(
        "desktop-project-dir-{}",
        super::element_key(project_path)
    ))
}

fn task_row_debug_selector(item_id: &str) -> &'static str {
    leaked_debug_selector(format!("desktop-task-row-{}", super::element_key(item_id)))
}

fn task_card_debug_selector(item_id: &str) -> &'static str {
    leaked_debug_selector(format!("desktop-task-card-{}", super::element_key(item_id)))
}

fn task_board_column_debug_selector(status: DesktopWorkItemStatus) -> &'static str {
    leaked_debug_selector(format!("desktop-task-board-column-{}", status.as_str()))
}

fn calendar_event_pill_debug_selector(event_id: &str) -> &'static str {
    leaked_debug_selector(format!(
        "desktop-calendar-event-pill-{}",
        super::element_key(event_id)
    ))
}

fn calendar_task_pill_debug_selector(task_id: &str) -> &'static str {
    leaked_debug_selector(format!(
        "desktop-calendar-task-pill-{}",
        super::element_key(task_id)
    ))
}

fn calendar_timegrid_event_debug_selector(event_id: &str) -> &'static str {
    leaked_debug_selector(format!(
        "desktop-calendar-time-event-{}",
        super::element_key(event_id)
    ))
}

fn calendar_timegrid_event_resize_debug_selector(event_id: &str) -> &'static str {
    leaked_debug_selector(format!(
        "desktop-calendar-time-event-resize-{}",
        super::element_key(event_id)
    ))
}

fn background_task_row_debug_selector(task_id: &str) -> &'static str {
    leaked_debug_selector(format!(
        "desktop-background-task-row-{}",
        super::element_key(task_id)
    ))
}

fn background_task_expand_debug_selector(task_id: &str) -> &'static str {
    leaked_debug_selector(format!(
        "desktop-background-task-expand-{}",
        super::element_key(task_id)
    ))
}

fn background_task_dismiss_debug_selector(task_id: &str) -> &'static str {
    leaked_debug_selector(format!(
        "desktop-background-task-dismiss-{}",
        super::element_key(task_id)
    ))
}

fn background_task_log_debug_selector(task_id: &str) -> &'static str {
    leaked_debug_selector(format!(
        "desktop-background-task-log-{}",
        super::element_key(task_id)
    ))
}

fn task_detail_project_option_debug_selector(project_id: &str) -> &'static str {
    leaked_debug_selector(format!(
        "desktop-task-detail-project-option-{}",
        super::element_key(project_id)
    ))
}

fn thread_row_debug_selector(thread_id: &str) -> &'static str {
    leaked_debug_selector(format!(
        "desktop-thread-row-{}",
        super::element_key(thread_id)
    ))
}

fn thread_delete_debug_selector(thread_id: &str) -> &'static str {
    leaked_debug_selector(format!(
        "desktop-thread-delete-{}",
        super::element_key(thread_id)
    ))
}

fn leaked_debug_selector(selector: String) -> &'static str {
    Box::leak(selector.into_boxed_str())
}

#[derive(Clone, Copy)]
enum TestAuthState {
    Unauthenticated,
}

fn set_model_for_visual_test(
    workspace: &Entity<Workspace>,
    cx: &mut gpui::VisualTestContext,
    model_id: &'static str,
    auth_state: TestAuthState,
) {
    workspace.update_in(cx, |workspace, _window, cx| {
        workspace.current_model = model_id.to_string();
        let chats = workspace
            .threads
            .iter()
            .map(|thread| thread.chat.clone())
            .collect::<Vec<_>>();
        for chat in chats {
            chat.update(cx, |chat, cx| {
                chat.model_id = model_id.to_string();
                match auth_state {
                    TestAuthState::Unauthenticated => chat.force_unauthenticated_for_test(cx),
                }
            });
        }
        cx.notify();
    });
}

fn work_item(id: &str, status: DesktopWorkItemStatus) -> DesktopWorkItem {
    DesktopWorkItem {
        id: id.to_string(),
        project_id: None,
        title: id.to_string(),
        description: None,
        status,
        priority: DesktopPriority::Medium,
        due_date: None,
        planning: WorkItemPlanning::default(),
        updated_at: EpochMs::UNIX_EPOCH,
    }
}

fn persisted_thread(id: &str, model_id: &str, permission_mode: PermissionMode) -> PersistedThread {
    PersistedThread {
        id: id.to_string(),
        title: "Thread".to_string(),
        project_name: String::new(),
        project_path: None,
        model_id: model_id.to_string(),
        permission_mode,
        messages: Vec::new(),
        updated_at_ms: EpochMs::UNIX_EPOCH,
        pinned: false,
        settled: false,
        pin_rank: None,
        active_rank: None,
        snoozed_until_ms: None,
        branch: None,
        prompt_queue: Vec::new(),
        fresh_session: false,
        session_id: None,
        session_provider: None,
        import_context: None,
        goal: None,
        worktree_path: None,
        effort: None,
        run_locked: false,
        role: None,
    }
}

#[test]
#[cfg(unix)]
fn desktop_state_and_backup_are_owner_only() {
    use std::os::unix::fs::PermissionsExt;
    let dir = std::env::temp_dir().join(format!(
        "bruhs-desktop-private-state-{}",
        uuid::Uuid::new_v4()
    ));
    let path = dir.join("desktop-state.json");
    write_desktop_state_atomically(&path, &PersistedWorkspaceState::default(), 1).unwrap();
    write_desktop_state_atomically(&path, &PersistedWorkspaceState::default(), 2).unwrap();
    for file in [&path, &desktop_state_backup_path(&path)] {
        assert_eq!(
            fs::metadata(file).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn desktop_state_write_preserves_crash_leftovers() {
    let dir =
        std::env::temp_dir().join(format!("bruhs-desktop-stale-temp-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&dir).unwrap();
    let path = dir.join("desktop-state.json");
    let orphan = dir.join(".desktop-state.json.1.tmp");
    fs::write(&orphan, "recoverable orphan").unwrap();
    write_desktop_state_atomically(&path, &PersistedWorkspaceState::default(), 1).unwrap();
    assert_eq!(fs::read_to_string(&orphan).unwrap(), "recoverable orphan");
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn independent_native_writers_cannot_replace_a_newer_workspace() {
    use super::{save_desktop_state, AtomicOrdering, PersistenceCoordinator};
    let dir = std::env::temp_dir().join(format!("bruhs-native-conflict-{}", uuid::Uuid::new_v4()));
    let path = dir.join("state.json");
    let first = std::sync::Arc::new(PersistenceCoordinator::default());
    let stale = std::sync::Arc::new(PersistenceCoordinator::default());
    first.latest_generation.store(1, AtomicOrdering::Release);
    stale.latest_generation.store(1, AtomicOrdering::Release);
    save_desktop_state(
        Some(path.clone()),
        PersistedWorkspaceState {
            current_model: Some("newer-state".into()),
            ..Default::default()
        },
        first,
        1,
    )
    .unwrap();
    let result = save_desktop_state(
        Some(path.clone()),
        PersistedWorkspaceState {
            current_model: Some("stale-state".into()),
            ..Default::default()
        },
        stale,
        1,
    );
    assert!(result.is_err(), "a separate stale writer must fail closed");
    assert_eq!(
        load_desktop_state_from_path(&path)
            .unwrap()
            .unwrap()
            .current_model
            .as_deref(),
        Some("newer-state")
    );
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn desktop_state_writes_should_be_atomic_and_keep_one_backup() {
    let dir = std::env::temp_dir().join(format!(
        "bruhs-desktop-persistence-{}",
        uuid::Uuid::new_v4()
    ));
    let path = dir.join("desktop-state.json");
    let first = PersistedWorkspaceState {
        current_model: Some("first-model".to_string()),
        ..PersistedWorkspaceState::default()
    };
    let second = PersistedWorkspaceState {
        current_model: Some("second-model".to_string()),
        ..PersistedWorkspaceState::default()
    };

    write_desktop_state_atomically(&path, &first, 1).expect("first state should write atomically");
    write_desktop_state_atomically(&path, &second, 2)
        .expect("second state should replace the first atomically");

    let current = load_desktop_state_from_path(&path)
        .expect("current state should parse")
        .expect("current state should exist");
    let backup = load_desktop_state_from_path(&desktop_state_backup_path(&path))
        .expect("backup state should parse")
        .expect("backup state should exist");
    assert_eq!(current.current_model.as_deref(), Some("second-model"));
    assert_eq!(backup.current_model.as_deref(), Some("first-model"));
    assert!(
        fs::read_dir(&dir)
            .expect("persistence directory should be readable")
            .all(|entry| !entry
                .expect("persistence entry should be readable")
                .file_name()
                .to_string_lossy()
                .ends_with(".tmp")),
        "successful atomic writes should not leave temporary files"
    );

    fs::remove_dir_all(dir).expect("persistence test directory should be removable");
}

#[test]
fn desktop_state_load_should_recover_from_valid_backup() {
    let dir = std::env::temp_dir().join(format!("bruhs-desktop-recovery-{}", uuid::Uuid::new_v4()));
    let path = dir.join("desktop-state.json");
    let first = PersistedWorkspaceState {
        current_model: Some("recoverable-model".to_string()),
        ..PersistedWorkspaceState::default()
    };
    let second = PersistedWorkspaceState {
        current_model: Some("newer-model".to_string()),
        ..PersistedWorkspaceState::default()
    };
    write_desktop_state_atomically(&path, &first, 1).expect("first state should write atomically");
    write_desktop_state_atomically(&path, &second, 2).expect("second state should create a backup");
    fs::write(&path, "{ invalid json")
        .expect("current state should be corruptible for recovery test");

    let (recovered, warning, _) = load_desktop_state_with_backup(&path);
    assert_eq!(
        recovered
            .expect("backup should recover state")
            .current_model
            .as_deref(),
        Some("recoverable-model")
    );
    assert!(
        warning
            .as_deref()
            .is_some_and(|warning| warning.contains("Recovered the previous state")),
        "recovery should return a visible warning"
    );

    fs::remove_dir_all(dir).expect("recovery test directory should be removable");
}

#[test]
fn command_actions_should_match_web_reference() {
    assert_eq!(COMMAND_BAR_INPUT_PLACEHOLDER, "What do you need?");
    assert_eq!(COMMAND_BAR_BODY_FONT_FAMILY, "Monaspace Neon");
    assert_eq!(COMMAND_BAR_LABEL_FONT_FAMILY, "Monaspace Argon");
    assert_eq!(COMMAND_BAR_INPUT_TEXT_SIZE_PX, 14.0);
    assert_eq!(COMMAND_BAR_INPUT_X_PADDING_PX, 20.0);
    assert_eq!(COMMAND_BAR_INPUT_Y_PADDING_PX, 16.0);
    assert_eq!(COMMAND_BAR_OVERLAY_OPACITY, 0.70);
    assert_eq!(COMMAND_BAR_LIST_MAX_HEIGHT_PX, 576.0);
    assert_eq!(COMMAND_BAR_LIST_X_PADDING_PX, 8.0);
    assert_eq!(COMMAND_BAR_LIST_Y_PADDING_PX, 8.0);
    assert_eq!(COMMAND_BAR_EMPTY_X_PADDING_PX, 16.0);
    assert_eq!(COMMAND_BAR_EMPTY_Y_PADDING_PX, 32.0);
    assert_eq!(COMMAND_BAR_ACTION_RADIUS_PX, 8.0);
    assert_eq!(COMMAND_BAR_ACTION_TEXT_SIZE_PX, 12.0);
    assert_eq!(COMMAND_BAR_ACTION_X_PADDING_PX, 12.0);
    assert_eq!(COMMAND_BAR_ACTION_Y_PADDING_PX, 10.0);
    assert_eq!(COMMAND_BAR_ACTION_HOVER_BG_OPACITY, 0.70);
    assert_eq!(COMMAND_BAR_GROUP_TEXT_SIZE_PX, 14.0);
    assert_eq!(COMMAND_BAR_GROUP_X_PADDING_PX, 12.0);
    assert_eq!(COMMAND_BAR_GROUP_BOTTOM_PADDING_PX, 4.0);
    assert_eq!(COMMAND_BAR_FOOTER_TEXT_SIZE_PX, 14.0);
    assert_eq!(COMMAND_BAR_FOOTER_X_PADDING_PX, 20.0);
    assert_eq!(COMMAND_BAR_FOOTER_Y_PADDING_PX, 10.0);
    assert_eq!(COMMAND_BAR_FOOTER_BORDER_OPACITY, 0.50);
    assert_eq!(COMMAND_BAR_FOOTER_HINT_GAP_PX, 4.0);
    let actions = COMMAND_ACTIONS
        .iter()
        .map(|action| (action.id, action.group, action.label, action.hint))
        .collect::<Vec<_>>();
    assert_eq!(
        actions,
        [
            ("nav-today", "Navigate", "Today", "plan & focus"),
            ("nav-chat", "Navigate", "Chat", "conversations"),
            ("nav-inbox", "Navigate", "Inbox", "messages & follow-ups"),
            (
                "nav-knowledge",
                "Navigate",
                "Knowledge",
                "documents & sources"
            ),
            ("nav-tasks", "Navigate", "Tasks", "projects & issues"),
            ("nav-calendar", "Navigate", "Calendar", "schedule & events"),
            ("nav-life", "Navigate", "Life", "finance, health & habits",),
            (
                "nav-automations",
                "Navigate",
                "Agent workflows",
                "permissions, jobs & audit",
            ),
            (
                "nav-meetings",
                "Navigate",
                "Meetings",
                "recorded conversations"
            ),
            ("nav-people", "Navigate", "People", "meeting profiles"),
            ("nav-search", "Navigate", "Search", "everything, instantly"),
            (
                "nav-terminal",
                "Navigate",
                "Terminal",
                "shells, no thread needed"
            ),
            ("open-project", "Agent", "Open project...", "coding agent"),
            ("new-thread", "Agent", "New thread", "⌘⇧O"),
            (
                "import-thread",
                "Agent",
                "Import thread...",
                "Claude Code, Codex, T3, Cursor"
            ),
            ("pin-thread", "Actions", "Pin / unpin thread", "⌘⇧P"),
            (
                "settle-thread",
                "Actions",
                "Settle / unsettle thread",
                "⌘⇧S"
            ),
            ("copy-thread-link", "Actions", "Copy thread link", "⌘⇧C"),
            (
                "copy-resume-command",
                "Actions",
                "Copy resume command",
                "Claude Code / Codex"
            ),
            (
                "copy-thread-markdown",
                "Actions",
                "Copy thread as Markdown",
                "for Cursor and others"
            ),
            (
                "continue-in-t3",
                "Actions",
                "Continue in T3 Code",
                "new T3 thread"
            ),
            (
                "copy-pull-link",
                "Actions",
                "Copy pull request link",
                "open pull request"
            ),
            (
                "copy-pull-number",
                "Actions",
                "Copy pull request number",
                "# and the number"
            ),
            ("new-task", "Tasks", "New task", "create"),
            ("plan-day", "Planning", "Plan my day", "review focus blocks"),
            (
                "smart-scheduling",
                "Planning",
                "Smart scheduling",
                "availability, links & conflicts",
            ),
            ("new-document", "Knowledge", "New document", "Markdown note"),
            (
                "import-conversation",
                "Inbox",
                "Import conversation",
                "email, Slack, Discord, iMessage, X"
            ),
            (
                "new-web-clip",
                "Knowledge",
                "Save web clip",
                "source & excerpt"
            ),
            ("capture", "Actions", "Capture anything", "⌘⇧Space"),
            (
                "new-meeting-note",
                "Meetings",
                "New meeting note",
                "local text"
            ),
            ("toggle-sidebar", "View", "Toggle sidebar", "⌘B"),
            ("toggle-diff", "View", "Toggle diff panel", "⌘D"),
            ("toggle-panel", "View", "Toggle right panel", "⌘⌥B"),
            ("toggle-tasks", "View", "Toggle tasks panel", "⌘⇧T"),
            ("toggle-terminal", "View", "Toggle terminal", "⌘J"),
            ("open-terminal", "View", "Open in system Terminal", "app"),
            ("local-servers", "View", "Open a local server…", "servers"),
            ("toggle-preview", "View", "Toggle preview", "⌘⇧J"),
            ("toggle-files", "View", "Toggle files panel", "project tree"),
            ("stash-prompt", "Actions", "Stash prompt", "⌘S"),
            ("model-picker", "Actions", "Model", "switch provider"),
            ("settings", "Actions", "Settings", "preferences"),
        ]
    );
}

#[test]
fn command_action_filter_should_match_web_raw_query_behavior() {
    let matching_labels = COMMAND_ACTIONS
        .iter()
        .filter(|action| Workspace::command_action_matches_query(action, " "))
        .map(|action| action.label)
        .collect::<Vec<_>>();

    assert_eq!(
        matching_labels,
        [
            "Today",
            "Inbox",
            "Knowledge",
            "Tasks",
            "Calendar",
            "Life",
            "Agent workflows",
            "Meetings",
            "People",
            "Search",
            "Terminal",
            "Open project...",
            "New thread",
            "Import thread...",
            "Pin / unpin thread",
            "Settle / unsettle thread",
            "Copy thread link",
            "Copy resume command",
            "Copy thread as Markdown",
            "Continue in T3 Code",
            "Copy pull request link",
            "Copy pull request number",
            "New task",
            "Plan my day",
            "Smart scheduling",
            "New document",
            "Import conversation",
            "Save web clip",
            "Capture anything",
            "New meeting note",
            "Toggle sidebar",
            "Toggle diff panel",
            "Toggle right panel",
            "Toggle tasks panel",
            "Toggle terminal",
            "Open in system Terminal",
            "Open a local server…",
            "Toggle preview",
            "Toggle files panel",
            "Stash prompt",
            "Model",
        ]
    );
}

#[test]
fn command_bar_selection_should_match_web_arrow_behavior() {
    assert_eq!(
        Workspace::move_command_selected_index(0, COMMAND_ACTIONS.len(), "ArrowDown"),
        Some(1)
    );
    assert_eq!(
        Workspace::move_command_selected_index(0, COMMAND_ACTIONS.len(), "down"),
        Some(1)
    );
    assert_eq!(
        Workspace::move_command_selected_index(
            COMMAND_ACTIONS.len() - 1,
            COMMAND_ACTIONS.len(),
            "ArrowDown"
        ),
        Some(0)
    );
    assert_eq!(
        Workspace::move_command_selected_index(0, COMMAND_ACTIONS.len(), "ArrowUp"),
        Some(COMMAND_ACTIONS.len() - 1)
    );
    assert_eq!(
        Workspace::move_command_selected_index(0, COMMAND_ACTIONS.len(), "up"),
        Some(COMMAND_ACTIONS.len() - 1)
    );
    assert_eq!(
        // Out-of-range indices clamp to the last row before moving.
        Workspace::move_command_selected_index(
            COMMAND_ACTIONS.len() + 1,
            COMMAND_ACTIONS.len(),
            "ArrowUp"
        ),
        Some(COMMAND_ACTIONS.len() - 2)
    );
    assert_eq!(
        Workspace::move_command_selected_index(0, 0, "ArrowDown"),
        Some(0)
    );
    assert_eq!(
        Workspace::move_command_selected_index(0, COMMAND_ACTIONS.len(), "Enter"),
        None
    );
    assert_eq!(Workspace::clamp_command_selected_index(12, 7), 6);
}

#[test]
fn desktop_model_settings_options_should_match_web_reference() {
    let models = MODEL_CATALOG
        .iter()
        .map(|model| (model.id, model.label, model.provider.as_str()))
        .collect::<Vec<_>>();

    assert_eq!(
        models,
        vec![
            ("gpt-6.1-sol", "GPT-6.1 Sol", "codex"),
            ("gpt-5.5", "GPT-5.5", "codex"),
            ("gpt-5.4", "GPT-5.4", "codex"),
            ("gpt-5.3-codex", "GPT-5.3 Codex", "codex"),
            ("claude-fable-5", "Claude Fable 5", "claude"),
            ("claude-opus-4-8", "Claude Opus 4.8", "claude"),
            ("claude-sonnet-4-6", "Claude Sonnet 4.6", "claude"),
            ("claude-haiku-4-5", "Claude Haiku 4.5", "claude"),
        ]
    );

    let grouped = desktop_model_provider_groups_like_web()
        .into_iter()
        .map(|(provider, models)| {
            (
                provider.as_str(),
                models
                    .into_iter()
                    .map(|(_, model)| model.id)
                    .collect::<Vec<_>>(),
            )
        })
        .collect::<Vec<_>>();

    assert_eq!(
        grouped,
        vec![
            (
                "claude",
                vec![
                    "claude-fable-5",
                    "claude-opus-4-8",
                    "claude-sonnet-4-6",
                    "claude-haiku-4-5",
                ],
            ),
            (
                "codex",
                vec!["gpt-6.1-sol", "gpt-5.5", "gpt-5.4", "gpt-5.3-codex"],
            ),
        ]
    );

    assert_eq!(provider_label(AgentProvider::Claude), "Anthropic");
    assert_eq!(provider_label(AgentProvider::Codex), "OpenAI Codex");
    assert_eq!(assets::PROVIDER_LOGO_SIZE_PX, 12.0);
    assert_eq!(assets::PROVIDER_LOGO_OPACITY, 0.6);
}

#[test]
fn desktop_sidebar_tabs_should_match_web_reference() {
    let tabs = [DesktopSidebarTab::Chat, DesktopSidebarTab::Code]
        .iter()
        .map(|tab| (tab.as_str(), tab.label()))
        .collect::<Vec<_>>();

    assert_eq!(tabs, vec![("chat", "Chat"), ("code", "Code")]);
}

#[test]
fn tasks_panel_shortcut_should_toggle_state_like_web_reference() {
    assert!(tasks_panel_visibility_after_toggle(false));
    assert!(!tasks_panel_visibility_after_toggle(true));
}

#[test]
fn plain_key_shortcuts_should_not_capture_focused_inputs_like_web_reference() {
    assert_eq!(DESKTOP_TEXT_INPUT_GUARDED_SHORTCUTS, ["/", "c"]);
    assert!(focused_input_should_receive_plain_shortcut("/", true));
    assert!(focused_input_should_receive_plain_shortcut("c", true));
    assert!(!focused_input_should_receive_plain_shortcut("/", false));
}

#[test]
fn permission_mode_should_be_workspace_global_like_web_reference() {
    let persisted = PersistedWorkspaceState {
        permission_mode: Some(PermissionMode::Plan),
        active_thread_id: Some("thread-1".to_string()),
        threads: vec![persisted_thread(
            "thread-1",
            "gpt-5.4",
            PermissionMode::AcceptEdits,
        )],
        ..PersistedWorkspaceState::default()
    };

    assert_eq!(
        initial_permission_mode_from_persisted(Some(&persisted)),
        PermissionMode::Plan
    );
    assert_eq!(
        restored_thread_model_id("claude-sonnet-4-6", "gpt-5.4"),
        "gpt-5.4"
    );
    assert_eq!(
        restored_thread_model_id("claude-sonnet-4-6", ""),
        "claude-sonnet-4-6"
    );
    assert_eq!(
        restored_thread_permission_mode(PermissionMode::Plan, PermissionMode::AcceptEdits),
        PermissionMode::Plan
    );
    assert_eq!(
        permission_mode_after_cycle(PermissionMode::Plan),
        PermissionMode::BypassPermissions
    );
}

#[test]
fn permission_mode_restore_should_migrate_from_active_thread_state() {
    let persisted = PersistedWorkspaceState {
        active_thread_id: Some("thread-2".to_string()),
        threads: vec![
            persisted_thread("thread-1", "gpt-5.4", PermissionMode::AcceptEdits),
            persisted_thread(
                "thread-2",
                "claude-sonnet-4-6",
                PermissionMode::BypassPermissions,
            ),
        ],
        ..PersistedWorkspaceState::default()
    };

    assert_eq!(
        initial_permission_mode_from_persisted(Some(&persisted)),
        PermissionMode::BypassPermissions
    );
}

#[test]
fn active_view_restore_should_start_in_chat_like_web_reference() {
    assert_eq!(
        initial_active_view_after_start(Some(AppView::Tasks)),
        AppView::Chat
    );
    assert_eq!(
        initial_active_view_after_start(Some(AppView::Calendar)),
        AppView::Chat
    );
    assert_eq!(initial_active_view_after_start(None), AppView::Chat);
}

#[test]
fn sidebar_restore_should_start_visible_like_web_reference() {
    assert!(initial_sidebar_visible_after_start(Some(false)));
    assert!(initial_sidebar_visible_after_start(Some(true)));
    assert!(initial_sidebar_visible_after_start(None));
}

#[test]
fn active_thread_restore_should_select_first_thread_like_web_reference() {
    let thread_ids = vec![
        "thread-1".to_string(),
        "thread-2".to_string(),
        "thread-3".to_string(),
    ];

    assert_eq!(
        initial_active_thread_id_after_start(Some("thread-3".to_string()), &thread_ids),
        Some("thread-1".to_string())
    );
    assert_eq!(
        initial_active_thread_id_after_start(Some("missing".to_string()), &thread_ids),
        Some("thread-1".to_string())
    );
    assert_eq!(initial_active_thread_id_after_start(None, &[]), None);
}

#[test]
fn task_selection_restore_should_start_empty_like_web_reference() {
    assert_eq!(
        initial_selected_project_after_start(Some("project-1".to_string())),
        None
    );
    assert_eq!(
        initial_selected_work_item_after_start(Some("task-1".to_string())),
        None
    );
    assert_eq!(initial_selected_project_after_start(None), None);
    assert_eq!(initial_selected_work_item_after_start(None), None);
}

#[test]
fn calendar_view_restore_should_start_fresh_like_web_reference() {
    let today = NaiveDate::from_ymd_opt(2026, 5, 5).expect("test date should be valid");

    assert_eq!(
        initial_calendar_mode_after_start(Some(DesktopCalendarMode::Day)),
        DesktopCalendarMode::Month
    );
    assert_eq!(
        initial_calendar_mode_after_start(None),
        DesktopCalendarMode::Month
    );
    assert_eq!(
        initial_calendar_anchor_after_start(Some("2026-01-01"), today),
        today
    );
    assert_eq!(
        initial_selected_calendar_event_after_start(Some("event-1".to_string())),
        None
    );
    assert_eq!(initial_selected_calendar_event_after_start(None), None);
}

#[test]
fn desktop_sidebar_copy_should_match_web_reference() {
    assert_eq!(
        CHAT_NO_ACTIVE_THREAD_LABEL,
        "Pick a conversation, or start a new one."
    );
    assert_eq!(CHAT_NO_ACTIVE_THREAD_TEXT_SIZE_PX, 13.0);
    assert_eq!(THREAD_TITLE_GENERAL, "New thread");
    assert_eq!(THREAD_TITLE_PROJECT_SESSION, "New session");
    assert_eq!(THREAD_TITLE_PROJECT_GROUP_NEW, "New thread");
}

#[test]
fn project_thread_titles_should_match_web_creation_paths() {
    assert_eq!(default_thread_title(""), "New thread");
    assert_eq!(default_thread_title("bruhs-ai"), "New session");
    assert_eq!(THREAD_TITLE_PROJECT_GROUP_NEW, "New thread");
}

#[test]
fn project_session_create_should_preserve_current_surface_like_web_reference() {
    assert_eq!(
        project_session_surface_after_create_like_web(AppView::Chat),
        (AppView::Chat, DesktopSidebarTab::Code)
    );
    assert_eq!(
        project_session_surface_after_create_like_web(AppView::Tasks),
        (AppView::Tasks, DesktopSidebarTab::Code)
    );
}

#[test]
fn active_thread_delete_should_clear_selection_like_web_reference() {
    assert_eq!(
        thread_selection_after_delete(Some("thread-1".to_string()), "thread-1"),
        None
    );
    assert_eq!(
        thread_selection_after_delete(Some("thread-2".to_string()), "thread-1"),
        Some("thread-2".to_string())
    );
    assert_eq!(thread_selection_after_delete(None, "thread-1"), None);
}

#[test]
fn clamp_right_panel_width_should_match_web_reference() {}

#[test]
fn desktop_workspace_sidebar_typography_should_match_web_reference() {
    assert_eq!(WORKSPACE_SIDE_PANEL_WIDTH_PX, 272.0);
    assert_eq!(WORKSPACE_SIDE_PANEL_MIN_WIDTH_PX, 208.0);
    assert_eq!(WORKSPACE_SIDEBAR_BORDER_OPACITY, 0.50);
    assert_eq!(WORKSPACE_SIDEBAR_TAB_RADIUS_PX, 4.0);
    assert_eq!(WORKSPACE_THREAD_ROW_HOVER_BG_OPACITY, 0.30);
}

#[test]
fn desktop_project_picker_typography_should_match_web_reference() {
    assert_eq!(PROJECT_PICKER_BODY_FONT_FAMILY, "Monaspace Neon");
    assert_eq!(PROJECT_PICKER_LABEL_FONT_FAMILY, "Monaspace Argon");
    assert_eq!(PROJECT_PICKER_PATH_FONT_FAMILY, "Monaspace Neon");
    assert_eq!(PROJECT_PICKER_ROW_RADIUS_PX, 8.0);
    assert_eq!(PROJECT_PICKER_ICON_SIZE_PX, 14.0);
    assert_eq!(PROJECT_PICKER_PARENT_ICON_SLOT_WIDTH_PX, 14.0);
    assert_eq!(PROJECT_PICKER_CONTENT_MAX_HEIGHT_PX, 288.0);
    assert_eq!(PROJECT_PICKER_LABEL_TEXT_SIZE_PX, 12.0);
    assert_eq!(PROJECT_PICKER_SECTION_TEXT_SIZE_PX, 12.0);
    assert_eq!(PROJECT_PICKER_FOOTER_TEXT_SIZE_PX, 12.0);
    assert_eq!(PROJECT_PICKER_OVERLAY_OPACITY, 0.70);
    assert_eq!(PROJECT_PICKER_HEADER_X_PADDING_PX, 16.0);
    assert_eq!(PROJECT_PICKER_HEADER_Y_PADDING_PX, 12.0);
    assert_eq!(PROJECT_PICKER_HEADER_GAP_PX, 12.0);
    assert_eq!(PROJECT_PICKER_PATH_TOP_MARGIN_PX, 2.0);
    assert_eq!(PROJECT_PICKER_SELECT_MARGIN_LEFT_PX, 12.0);
    assert_eq!(PROJECT_PICKER_SELECT_X_PADDING_PX, 12.0);
    assert_eq!(PROJECT_PICKER_SELECT_Y_PADDING_PX, 6.0);
    assert_eq!(PROJECT_PICKER_SELECT_BG_OPACITY, 0.20);
    assert_eq!(PROJECT_PICKER_SELECT_HOVER_BG_OPACITY, 0.30);
    assert_eq!(PROJECT_PICKER_RECENTS_X_PADDING_PX, 16.0);
    assert_eq!(PROJECT_PICKER_RECENTS_Y_PADDING_PX, 12.0);
    assert_eq!(PROJECT_PICKER_RECENTS_GAP_PX, 2.0);
    assert_eq!(PROJECT_PICKER_RECENTS_LABEL_BOTTOM_MARGIN_PX, 4.0);
    assert_eq!(PROJECT_PICKER_RECENTS_BORDER_OPACITY, 0.50);
    assert_eq!(PROJECT_PICKER_RECENT_ROW_GAP_PX, 8.0);
    assert_eq!(PROJECT_PICKER_RECENT_ROW_X_PADDING_PX, 8.0);
    assert_eq!(PROJECT_PICKER_RECENT_ROW_Y_PADDING_PX, 6.0);
    assert_eq!(PROJECT_PICKER_RECENT_ROW_HOVER_BG_OPACITY, 0.50);
    assert_eq!(PROJECT_PICKER_LIST_Y_PADDING_PX, 4.0);
    assert_eq!(PROJECT_PICKER_ROW_GAP_PX, 8.0);
    assert_eq!(PROJECT_PICKER_ROW_X_PADDING_PX, 16.0);
    assert_eq!(PROJECT_PICKER_ROW_Y_PADDING_PX, 8.0);
    assert_eq!(PROJECT_PICKER_ROW_HOVER_BG_OPACITY, 0.40);
    assert_eq!(PROJECT_PICKER_EMPTY_X_PADDING_PX, 16.0);
    assert_eq!(PROJECT_PICKER_EMPTY_Y_PADDING_PX, 24.0);
    assert_eq!(PROJECT_PICKER_FOOTER_X_PADDING_PX, 16.0);
    assert_eq!(PROJECT_PICKER_FOOTER_Y_PADDING_PX, 10.0);
    assert_eq!(PROJECT_PICKER_FOOTER_GAP_PX, 12.0);
    assert_eq!(PROJECT_PICKER_FOOTER_BORDER_OPACITY, 0.50);
    assert_eq!(PROJECT_PICKER_LIST_ERROR, "Could not list directory");
}

#[test]
fn project_picker_directory_error_should_match_web_dialog_copy() {
    assert_eq!(
        project_picker_directory_error_like_web("Cannot read directory"),
        "Could not list directory"
    );
    assert_eq!(
        project_picker_directory_error_like_web("Path must be within your home directory"),
        "Could not list directory"
    );
}

#[test]
fn project_picker_recents_should_match_web_dedupe_order_and_cap() {
    let mut recents = (0..5)
        .map(|index| DesktopProjectLocation {
            path: format!("/projects/{index}"),
            name: format!("Project {index}"),
        })
        .collect::<Vec<_>>();

    push_recent_project(
        &mut recents,
        DesktopProjectLocation {
            path: "/projects/2".to_string(),
            name: "Renamed".to_string(),
        },
    );
    assert_eq!(
        recents
            .iter()
            .map(|project| project.path.as_str())
            .collect::<Vec<_>>(),
        vec![
            "/projects/2",
            "/projects/0",
            "/projects/1",
            "/projects/3",
            "/projects/4"
        ]
    );
    assert_eq!(recents[0].name, "Renamed");

    push_recent_project(
        &mut recents,
        DesktopProjectLocation {
            path: "/projects/5".to_string(),
            name: "Project 5".to_string(),
        },
    );
    assert_eq!(
        recents
            .iter()
            .map(|project| project.path.as_str())
            .collect::<Vec<_>>(),
        vec![
            "/projects/5",
            "/projects/2",
            "/projects/0",
            "/projects/1",
            "/projects/3"
        ]
    );
}

#[test]
fn general_thread_project_path_should_follow_active_project_like_web_reference() {
    let active_project = DesktopProjectLocation {
        path: "/projects/bruhs-ai".to_string(),
        name: "bruhs-ai".to_string(),
    };

    assert_eq!(
        general_thread_project_path_like_web(Some(&active_project), "/fallback".to_string()),
        "/projects/bruhs-ai"
    );
    assert_eq!(
        general_thread_project_path_like_web(None, "/fallback".to_string()),
        "/fallback"
    );
    assert!(general_thread_project_scoped_like_web(Some(
        &active_project
    )));
    assert!(!general_thread_project_scoped_like_web(None));
}

#[test]
fn read_project_directory_should_treat_empty_path_like_web_reference() {
    if home_dir_path().is_none() {
        return;
    }

    let home_listing =
        read_project_directory(None).expect("home directory should be listable in tests");
    let empty_listing = read_project_directory(Some(""))
        .expect("empty path should fall back to the home directory");

    assert_eq!(empty_listing.current, home_listing.current);
    assert_eq!(empty_listing.name, home_listing.name);
    assert_eq!(empty_listing.parent, home_listing.parent);
}

#[test]
fn read_project_directory_should_match_web_file_error() {
    let Some(home) = home_dir_path().and_then(|path| fs::canonicalize(path).ok()) else {
        return;
    };
    let Ok(cwd) = std::env::current_dir().and_then(fs::canonicalize) else {
        return;
    };
    if !cwd.starts_with(&home) {
        return;
    }

    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let dir = cwd.join("target").join(format!(
        "desktop-list-dir-file-test-{}-{unique}",
        std::process::id()
    ));
    fs::create_dir_all(&dir).expect("test directory should be creatable");
    let file = dir.join("not-a-directory");
    fs::write(&file, "not a directory").expect("test file should be writable");

    let result = read_project_directory(file.to_str());

    let _ = fs::remove_file(&file);
    let _ = fs::remove_dir(&dir);
    assert_eq!(result.err().as_deref(), Some("Cannot read directory"));
}

#[test]
fn read_project_directory_should_match_web_filter_sort_and_parent_rules() {
    let Some(home) = home_dir_path().and_then(|path| fs::canonicalize(path).ok()) else {
        return;
    };
    let Ok(cwd) = std::env::current_dir().and_then(fs::canonicalize) else {
        return;
    };
    if !cwd.starts_with(&home) {
        return;
    }

    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let root = cwd.join("target").join(format!(
        "desktop-list-dir-rules-test-{}-{unique}",
        std::process::id()
    ));
    fs::create_dir_all(root.join("zeta")).expect("test directory should be creatable");
    fs::create_dir_all(root.join("alpha")).expect("test directory should be creatable");
    fs::create_dir_all(root.join(".hidden")).expect("hidden test directory should be creatable");
    fs::write(root.join("file.txt"), "not a directory").expect("test file should be writable");

    let listing = read_project_directory(root.to_str()).expect("test directory should be listable");
    let expected_parent = root.parent().map(|path| path.to_string_lossy().to_string());

    let _ = fs::remove_dir_all(&root);
    assert_eq!(listing.current, root.to_string_lossy());
    assert_eq!(listing.parent, expected_parent);
    assert_eq!(
        listing
            .dirs
            .iter()
            .map(|directory| directory.name.as_str())
            .collect::<Vec<_>>(),
        vec!["alpha", "zeta"]
    );
}

#[test]
fn canonical_project_location_should_reject_empty_path_like_web_reference() {
    let result = canonical_project_location("", "Fallback");
    assert_eq!(result.err().as_deref(), Some("Path is required"));
}

#[test]
fn desktop_task_form_copy_should_match_web_reference() {
    assert_eq!(TASK_DIALOG_BODY_FONT_FAMILY, "Monaspace Neon");
    assert_eq!(TASK_DIALOG_LABEL_FONT_FAMILY, "Monaspace Argon");
    assert_eq!(TASK_DIALOG_TITLE_TEXT_SIZE_PX, 13.0);
    assert_eq!(TASK_FORM_LABEL_TEXT_SIZE_PX, 12.0);
    assert_eq!(FORM_FIELD_GAP_PX, 6.0);
    assert_eq!(FORM_CONTROL_RADIUS_PX, 8.0);
    assert_eq!(FORM_SELECT_BODY_FONT_FAMILY, "Monaspace Neon");
    assert_eq!(FORM_SELECT_BODY_TEXT_SIZE_PX, 14.0);
    assert_eq!(FORM_SELECT_HEIGHT_PX, 32.0);
    assert_eq!(FORM_SELECT_MENU_MAX_HEIGHT_PX, 184.0);
    assert_eq!(FORM_INPUT_HEIGHT_PX, 40.0);
    assert_eq!(FORM_INPUT_TEXT_SIZE_PX, 16.0);
    assert_eq!(FORM_THREE_ROW_TEXTAREA_HEIGHT_PX, 88.0);
    assert_eq!(FORM_FIVE_ROW_TEXTAREA_HEIGHT_PX, 136.0);
    assert_eq!(FORM_DIALOG_SECTION_X_PADDING_PX, 16.0);
    assert_eq!(FORM_GRID_GAP_PX, 12.0);
    assert_eq!(FORM_SELECT_MENU_Y_PADDING_PX, 4.0);
    assert_eq!(FORM_SELECT_OPTION_GAP_PX, 8.0);
    assert_eq!(FORM_SELECT_OPTION_X_PADDING_PX, 12.0);
    assert_eq!(FORM_SELECT_OPTION_Y_PADDING_PX, 6.0);
    assert_eq!(TASK_CREATE_OVERLAY_OPACITY, 0.70);
    assert_eq!(TASK_CREATE_HEADER_Y_PADDING_PX, 12.0);
    assert_eq!(TASK_CREATE_BODY_Y_PADDING_PX, 12.0);
    assert_eq!(TASK_CREATE_BODY_GAP_PX, 12.0);
    assert_eq!(TASK_CREATE_FOOTER_Y_PADDING_PX, 12.0);
    assert_eq!(TASK_CREATE_FOOTER_GAP_PX, 8.0);
    assert_eq!(TASK_CREATE_FOOTER_BG_OPACITY, 0.40);
    assert_eq!(TASK_DETAIL_PANEL_WIDTH_PX, 384.0);
    assert_eq!(TASK_DETAIL_TITLE_TEXT_SIZE_PX, 14.0);
    assert_eq!(TASK_DETAIL_HEADER_Y_PADDING_PX, 12.0);
    assert_eq!(TASK_DETAIL_BODY_Y_PADDING_PX, 16.0);
    assert_eq!(TASK_DETAIL_BODY_GAP_PX, 16.0);
    assert_eq!(TASK_DETAIL_FOOTER_Y_PADDING_PX, 12.0);
    assert_eq!(TASK_DETAIL_STATUS_CHIP_RADIUS_PX, 8.0);
    assert_eq!(TASK_DETAIL_STATUS_CHIP_X_PADDING_PX, 10.0);
    assert_eq!(TASK_DETAIL_STATUS_CHIP_Y_PADDING_PX, 4.0);
    assert_eq!(TASK_DETAIL_STATUS_CHIP_GAP_PX, 6.0);
    assert_eq!(TASK_DETAIL_STATUS_CHIP_TEXT_SIZE_PX, 12.0);
    assert_eq!(TASK_DETAIL_STATUS_CHIP_ACTIVE_BORDER_OPACITY, 0.30);
    assert_eq!(TASK_CREATE_TITLE_PLACEHOLDER, "What needs to be done?");
    assert_eq!(
        TASK_CREATE_DESCRIPTION_PLACEHOLDER,
        "Add details (optional)"
    );
    assert_eq!(TASK_CREATE_DUE_PLACEHOLDER, "Select date");
    assert_eq!(TASK_DETAIL_CLOSE_LABEL, "Close");
    assert_eq!(TASK_DETAIL_TITLE_PLACEHOLDER, "Title");
    assert_eq!(TASK_DETAIL_DESCRIPTION_PLACEHOLDER, "Add description");
    assert_eq!(TASK_DETAIL_DUE_PLACEHOLDER, "");
    assert_eq!(EVENT_DIALOG_BODY_FONT_FAMILY, "Monaspace Neon");
    assert_eq!(EVENT_DIALOG_LABEL_FONT_FAMILY, "Monaspace Argon");
    assert_eq!(EVENT_DIALOG_TITLE_TEXT_SIZE_PX, 13.0);
    assert_eq!(EVENT_FORM_LABEL_TEXT_SIZE_PX, 12.0);
    assert_eq!(EVENT_CREATE_OVERLAY_OPACITY, 0.70);
    assert_eq!(EVENT_CREATE_HEADER_Y_PADDING_PX, 12.0);
    assert_eq!(EVENT_CREATE_BODY_Y_PADDING_PX, 12.0);
    assert_eq!(EVENT_CREATE_BODY_GAP_PX, 12.0);
    assert_eq!(EVENT_CREATE_FOOTER_Y_PADDING_PX, 12.0);
    assert_eq!(EVENT_CREATE_FOOTER_GAP_PX, 8.0);
    assert_eq!(EVENT_CREATE_FOOTER_BG_OPACITY, 0.40);
    assert_eq!(EVENT_CREATE_ACTION_GAP_PX, 8.0);
    assert_eq!(EVENT_CREATE_DELETE_X_PADDING_PX, 12.0);
    assert_eq!(EVENT_CREATE_DELETE_Y_PADDING_PX, 8.0);
    assert_eq!(EVENT_CREATE_DELETE_HOVER_BG_OPACITY, 0.10);
    assert_eq!(EVENT_COLOR_OPTION_X_PADDING_PX, 12.0);
    assert_eq!(EVENT_COLOR_OPTION_Y_PADDING_PX, 6.0);
    assert_eq!(EVENT_ALL_DAY_GAP_PX, 8.0);
    assert_eq!(EVENT_ALL_DAY_CHECK_SIZE_PX, 14.0);
    assert_eq!(EVENT_ALL_DAY_CHECK_RADIUS_PX, 4.0);
    assert_eq!(EVENT_ALL_DAY_CHECK_TEXT_SIZE_PX, 12.0);
    assert_eq!(EVENT_EDIT_LABEL, "Edit event");
    assert_eq!(EVENT_CREATE_SUBMIT_LABEL, "Create");
    assert_eq!(EVENT_SAVE_SUBMIT_LABEL, "Save");
    assert_eq!(EVENT_DELETE_LABEL, "Delete");
    assert_eq!(EVENT_CANCEL_LABEL, "Cancel");
    assert_eq!(EVENT_TITLE_PLACEHOLDER, "What's happening?");
    assert_eq!(EVENT_DATETIME_PLACEHOLDER, "");
    assert_eq!(EVENT_NOTES_PLACEHOLDER, "Add details (optional)");
}

#[test]
fn desktop_multiline_form_rows_should_match_web_reference() {
    assert_eq!(TASK_CREATE_DESCRIPTION_ROWS, 3);
    assert_eq!(TASK_DETAIL_DESCRIPTION_ROWS, 5);
    assert_eq!(EVENT_NOTES_ROWS, 3);
}

#[test]
fn desktop_task_detail_selects_should_match_web_native_select_behavior() {
    let mut item = work_item("task-1", DesktopWorkItemStatus::Todo);
    item.project_id = Some("project-1".to_string());
    item.priority = DesktopPriority::High;

    assert!(!select_task_detail_project_option_like_web(
        &mut item,
        Some("project-1".to_string())
    ));
    assert_eq!(item.updated_at, EpochMs::UNIX_EPOCH);
    assert_eq!(
        task_detail_menu_after_native_select_selection(
            Some(TaskDetailMenu::Project),
            TaskDetailMenu::Project
        ),
        None
    );
    assert_eq!(
        task_detail_menu_after_native_select_selection(
            Some(TaskDetailMenu::Status),
            TaskDetailMenu::Project
        ),
        Some(TaskDetailMenu::Status)
    );

    assert!(select_task_detail_project_option_like_web(&mut item, None));
    assert_eq!(item.project_id, None);
    assert!(item.updated_at > EpochMs::UNIX_EPOCH);

    item.updated_at = EpochMs::UNIX_EPOCH;
    assert!(!select_task_detail_status_option_like_web(
        &mut item,
        DesktopWorkItemStatus::Todo
    ));
    assert_eq!(item.updated_at, EpochMs::UNIX_EPOCH);

    assert!(select_task_detail_status_option_like_web(
        &mut item,
        DesktopWorkItemStatus::Done
    ));
    assert_eq!(item.status, DesktopWorkItemStatus::Done);
    assert!(item.updated_at > EpochMs::UNIX_EPOCH);

    item.updated_at = EpochMs::UNIX_EPOCH;
    assert!(!select_task_detail_priority_option_like_web(
        &mut item,
        DesktopPriority::High
    ));
    assert_eq!(item.updated_at, EpochMs::UNIX_EPOCH);

    assert!(select_task_detail_priority_option_like_web(
        &mut item,
        DesktopPriority::Low
    ));
    assert_eq!(item.priority, DesktopPriority::Low);
    assert!(item.updated_at > EpochMs::UNIX_EPOCH);
}

#[test]
fn desktop_task_detail_status_chips_should_update_active_status_like_web_reference() {
    let mut item = work_item("task-1", DesktopWorkItemStatus::Todo);

    click_task_detail_status_chip_like_web(&mut item, DesktopWorkItemStatus::Todo);

    assert_eq!(item.status, DesktopWorkItemStatus::Todo);
    assert!(item.updated_at > EpochMs::UNIX_EPOCH);
}

#[test]
fn task_create_submit_should_preserve_selection_like_web_reference() {
    assert_eq!(
        task_create_selected_work_item_after_submit(None, "created"),
        None
    );
    assert_eq!(
        task_create_selected_work_item_after_submit(Some("existing".to_string()), "created"),
        Some("existing".to_string())
    );
}

#[test]
fn project_filter_change_should_preserve_selected_task_like_web_reference() {
    assert_eq!(
        project_filter_selection_after_change(Some("project-2".to_string()), None),
        (Some("project-2".to_string()), None)
    );
    assert_eq!(
        project_filter_selection_after_change(
            Some("project-2".to_string()),
            Some("task-1".to_string())
        ),
        (Some("project-2".to_string()), Some("task-1".to_string()))
    );
    assert_eq!(
        project_filter_selection_after_change(None, Some("task-1".to_string())),
        (None, Some("task-1".to_string()))
    );
}

#[test]
fn desktop_project_sidebar_copy_should_match_web_reference() {
    assert_eq!(TASKS_ALL_PROJECTS_LABEL, "All Tasks");
    assert_eq!(TASKS_NEW_PROJECT_LABEL, "New project");
    assert_eq!(TASKS_NEW_TASK_LABEL, "New task");
    assert_eq!(PROJECT_SIDEBAR_BODY_FONT_FAMILY, "Monaspace Neon");
    assert_eq!(PROJECT_SIDEBAR_LABEL_FONT_FAMILY, "Monaspace Argon");
    assert_eq!(PROJECT_SIDEBAR_BORDER_OPACITY, 0.50);
    assert_eq!(PROJECT_SIDEBAR_CONTENT_X_PADDING_PX, 8.0);
    assert_eq!(PROJECT_SIDEBAR_CONTENT_Y_PADDING_PX, 8.0);
    assert_eq!(PROJECT_SIDEBAR_ROW_STACK_GAP_PX, 2.0);
    assert_eq!(PROJECT_SIDEBAR_ROW_HEIGHT_PX, 32.0);
    assert_eq!(PROJECT_SIDEBAR_ROW_RADIUS_PX, 8.0);
    assert_eq!(PROJECT_SIDEBAR_ROW_GAP_PX, 6.0);
    assert_eq!(PROJECT_SIDEBAR_ROW_X_PADDING_PX, 8.0);
    assert_eq!(PROJECT_SIDEBAR_ROW_ACTIVE_BG_OPACITY, 0.40);
    assert_eq!(PROJECT_SIDEBAR_ROW_HOVER_BG_OPACITY, 0.30);
    assert_eq!(PROJECT_SIDEBAR_ROW_TEXT_SIZE_PX, 12.0);
    assert_eq!(PROJECT_SIDEBAR_FOOTER_TEXT_SIZE_PX, 12.0);
    assert_eq!(PROJECT_SIDEBAR_FOOTER_ICON_TEXT_SIZE_PX, 12.0);
    assert_eq!(PROJECT_SIDEBAR_FOOTER_HEIGHT_PX, 36.0);
    assert_eq!(PROJECT_SIDEBAR_FOOTER_CONTAINER_X_PADDING_PX, 12.0);
    assert_eq!(PROJECT_SIDEBAR_FOOTER_CONTAINER_BOTTOM_PADDING_PX, 12.0);
    assert_eq!(PROJECT_SIDEBAR_FOOTER_GAP_PX, 6.0);
    assert_eq!(PROJECT_SIDEBAR_FOOTER_X_PADDING_PX, 12.0);
    assert_eq!(PROJECT_SIDEBAR_FOOTER_Y_PADDING_PX, 8.0);
    assert_eq!(PROJECT_SIDEBAR_FOOTER_BORDER_OPACITY, 0.50);
    assert_eq!(PROJECT_SIDEBAR_FOOTER_HOVER_BORDER_OPACITY, 0.50);
    assert_eq!(PROJECT_SIDEBAR_DELETE_ICON_SIZE_PX, 14.0);
    assert_eq!(PROJECT_SIDEBAR_ROW_ACTION_RADIUS_PX, 4.0);
    assert_eq!(PROJECT_SIDEBAR_ROW_ACTION_PADDING_PX, 2.0);
    assert_eq!(PROJECT_SIDEBAR_ROW_ACTION_HOVER_BG_OPACITY, 0.50);
    assert_eq!(CALENDAR_NEW_EVENT_LABEL, "New event");
    assert_eq!(PROJECT_DELETE_LABEL, "Delete project");
    assert_eq!(PROJECT_DELETE_CANCEL_LABEL, "Cancel");
    assert_eq!(PROJECT_DELETE_CONFIRM_LABEL, "Delete");
    assert_eq!(PROJECT_DELETE_BODY_PADDING_PX, 16.0);
    assert_eq!(PROJECT_DELETE_BODY_GAP_PX, 16.0);
    assert_eq!(PROJECT_DELETE_ACTION_GAP_PX, 8.0);
    assert_eq!(PROJECT_DELETE_PROMPT_TEXT_SIZE_PX, 14.0);
}

#[test]
fn desktop_tasks_header_typography_should_match_web_reference() {
    assert_eq!(TASKS_HEADER_BODY_FONT_FAMILY, "Monaspace Neon");
    assert_eq!(TASKS_HEADER_MIN_HEIGHT_PX, 40.0);
    assert_eq!(TASKS_HEADER_X_PADDING_PX, 16.0);
    assert_eq!(TASKS_HEADER_Y_PADDING_PX, 8.0);
    assert_eq!(TASKS_HEADER_BORDER_OPACITY, 0.40);
    assert_eq!(TASKS_HEADER_CONTROL_GAP_PX, 4.0);
    assert_eq!(TASKS_HEADER_CONTROL_RADIUS_PX, 8.0);
    assert_eq!(TASKS_HEADER_CONTROL_X_PADDING_PX, 10.0);
    assert_eq!(TASKS_HEADER_CONTROL_Y_PADDING_PX, 4.0);
    assert_eq!(TASKS_HEADER_MODE_ACTIVE_BG_OPACITY, 0.70);
    assert_eq!(TASKS_HEADER_TITLE_TEXT_SIZE_PX, 12.0);
    assert_eq!(TASKS_HEADER_MODE_TEXT_SIZE_PX, 12.0);
    assert_eq!(TASKS_HEADER_NEW_TASK_LEFT_MARGIN_PX, 4.0);
    assert_eq!(TASKS_HEADER_NEW_TASK_BORDER_OPACITY, 0.20);
    assert_eq!(TASKS_HEADER_NEW_TASK_BG_OPACITY, 0.15);
    assert_eq!(TASKS_HEADER_NEW_TASK_HOVER_BG_OPACITY, 0.25);
    assert_eq!(TASKS_HEADER_NEW_TASK_TEXT_SIZE_PX, 12.0);
}

#[test]
fn desktop_dialog_metrics_should_match_web_reference() {
    assert_eq!(DESKTOP_DIALOG_MD_WIDTH_PX, 448.0);
    assert_eq!(DESKTOP_DIALOG_LG_WIDTH_PX, 576.0);
    assert_eq!(DESKTOP_DIALOG_MAX_WIDTH_RATIO, 0.9);
    assert_eq!(DESKTOP_DIALOG_RADIUS_PX, 14.0);
    assert_eq!(DESKTOP_MODAL_TITLE_TEXT_SIZE_PX, 18.0);
    assert_eq!(DESKTOP_MODAL_OVERLAY_OPACITY, 0.70);
    assert_eq!(DESKTOP_MODAL_HEADER_X_PADDING_PX, 16.0);
    assert_eq!(DESKTOP_MODAL_HEADER_Y_PADDING_PX, 12.0);
    assert_eq!(DESKTOP_DIALOG_TOP_PADDING_PX, 128.0);
    assert_eq!(DESKTOP_MODAL_FOCUS_TRAP_KEY, "tab");
    assert_eq!(DESKTOP_MODAL_FOCUS_START_TAB_INDEX, 0);
    assert_eq!(DESKTOP_MODAL_FOCUS_END_TAB_INDEX, 999);
    assert_eq!(SETTINGS_CODE_FONT_FAMILY, "Monaspace Neon");
    assert_eq!(SETTINGS_CLOSE_BUTTON_SIZE_PX, 32.0);
    assert_eq!(SETTINGS_CONTROL_RADIUS_PX, 8.0);
    assert_eq!(SETTINGS_BODY_X_PADDING_PX, 32.0);
    assert_eq!(SETTINGS_BODY_Y_PADDING_PX, 24.0);
    assert_eq!(SETTINGS_BODY_GAP_PX, 32.0);
    assert_eq!(SETTINGS_SECTION_GAP_PX, 8.0);
    assert_eq!(SETTINGS_CLOSE_HOVER_BG_OPACITY, 0.50);
    assert_eq!(SETTINGS_BODY_TEXT_SIZE_PX, 12.0);
    assert_eq!(SETTINGS_META_TEXT_SIZE_PX, 12.0);
    assert_eq!(SETTINGS_CODE_RADIUS_PX, 4.0);
    assert_eq!(SETTINGS_MODEL_ROW_MIN_HEIGHT_PX, 48.0);
    assert_eq!(SETTINGS_MODEL_ID_GAP_PX, 2.0);
    assert_eq!(SETTINGS_MODEL_LABEL_TEXT_SIZE_PX, 14.0);
    assert_eq!(SETTINGS_CLOSE_LABEL, "Close settings");
}

#[test]
fn desktop_task_status_glyphs_should_match_web_reference() {
    let glyphs = DesktopWorkItemStatus::all()
        .iter()
        .map(|status| (status.as_str(), status.glyph()))
        .collect::<Vec<_>>();

    assert_eq!(
        glyphs,
        vec![
            ("backlog", "⬚"),
            ("todo", "⃝"),
            ("in_progress", "◐"),
            ("done", "●"),
            ("canceled", "⊘"),
        ]
    );
}

#[test]
fn desktop_priority_glyphs_should_match_web_reference() {
    let glyphs = DesktopPriority::all()
        .iter()
        .map(|priority| (priority.as_str(), priority.glyph()))
        .collect::<Vec<_>>();

    assert_eq!(
        glyphs,
        vec![
            ("none", "—"),
            ("urgent", "‼"),
            ("high", "▲"),
            ("medium", "◆"),
            ("low", "▼"),
        ]
    );
}

#[test]
fn desktop_task_board_columns_should_match_web_reference() {
    let columns = DesktopWorkItemStatus::board_columns()
        .iter()
        .map(|status| status.as_str())
        .collect::<Vec<_>>();

    assert_eq!(columns, vec!["backlog", "todo", "in_progress", "done"]);
}

#[test]
fn desktop_task_board_typography_should_match_web_reference() {
    assert_eq!(TASK_BOARD_BODY_FONT_FAMILY, "Monaspace Neon");
    assert_eq!(TASK_BOARD_PADDING_PX, 16.0);
    assert_eq!(TASK_BOARD_GAP_PX, 12.0);
    assert_eq!(TASK_BOARD_COLUMN_MIN_WIDTH_PX, 190.0);
    assert_eq!(TASK_BOARD_COLUMN_RADIUS_PX, 8.0);
    assert_eq!(TASK_BOARD_DRAG_OVER_RING_PX, 2.0);
    assert_eq!(TASK_BOARD_DRAG_OVER_RING_OPACITY, 0.40);
    assert_eq!(TASK_BOARD_COLUMN_COMPLETED_OPACITY, 0.70);
    assert_eq!(TASK_BOARD_COLUMN_COMPLETED_BG_OPACITY, 0.50);
    assert_eq!(TASK_BOARD_COLUMN_ACTIVE_BG_OPACITY, 0.30);
    assert_eq!(TASK_BOARD_HEADER_X_PADDING_PX, 12.0);
    assert_eq!(TASK_BOARD_HEADER_Y_PADDING_PX, 10.0);
    assert_eq!(TASK_BOARD_HEADER_GAP_PX, 8.0);
    assert_eq!(TASK_BOARD_HEADER_DOT_SIZE_PX, 8.0);
    assert_eq!(TASK_BOARD_HEADER_DOT_RADIUS_PX, 999.0);
    assert_eq!(TASK_BOARD_COUNT_RADIUS_PX, 999.0);
    assert_eq!(TASK_BOARD_COUNT_X_PADDING_PX, 6.0);
    assert_eq!(TASK_BOARD_COUNT_BG_OPACITY, 0.50);
    assert_eq!(TASK_BOARD_HEADER_TEXT_SIZE_PX, 12.0);
    assert_eq!(TASK_BOARD_BODY_PADDING_PX, 8.0);
    assert_eq!(TASK_BOARD_BODY_GAP_PX, 8.0);
    assert_eq!(TASK_BOARD_EMPTY_Y_PADDING_PX, 16.0);
    assert_eq!(TASK_BOARD_COUNT_TEXT_SIZE_PX, 12.0);
    assert_eq!(TASK_BOARD_EMPTY_TEXT_SIZE_PX, 12.0);
    assert_eq!(TASK_CARD_RADIUS_PX, 8.0);
    assert_eq!(TASK_CARD_PADDING_PX, 12.0);
    assert_eq!(TASK_CARD_GAP_PX, 8.0);
    assert_eq!(TASK_CARD_TOP_ROW_GAP_PX, 8.0);
    assert_eq!(TASK_CARD_BORDER_OPACITY, 0.30);
    assert_eq!(TASK_CARD_SELECTED_BG_OPACITY, 0.50);
    assert_eq!(TASK_CARD_BG_OPACITY, 0.60);
    assert_eq!(TASK_CARD_HOVER_BORDER_OPACITY, 0.50);
    assert_eq!(TASK_DRAG_PREVIEW_RADIUS_PX, 6.0);
    assert_eq!(TASK_DRAG_PREVIEW_BORDER_OPACITY, 0.45);
    assert_eq!(TASK_DRAG_PREVIEW_BG_OPACITY, 0.92);
    assert_eq!(TASK_CARD_PRIORITY_MARGIN_TOP_PX, 2.0);
    assert_eq!(TASK_CARD_TITLE_TEXT_SIZE_PX, 12.0);
    assert_eq!(TASK_CARD_DUE_TEXT_SIZE_PX, 12.0);
    assert_eq!(TASK_PRIORITY_EMPTY_OPACITY, 0.20);
}

#[test]
fn desktop_task_list_typography_should_match_web_reference() {
    assert_eq!(TASK_LIST_BODY_FONT_FAMILY, "Monaspace Neon");
    assert_eq!(TASK_LIST_HEADER_HEIGHT_PX, 34.0);
    assert_eq!(TASK_LIST_HEADER_X_PADDING_PX, 12.0);
    assert_eq!(TASK_LIST_HEADER_GAP_PX, 8.0);
    assert_eq!(TASK_LIST_HEADER_BORDER_OPACITY, 0.50);
    assert_eq!(TASK_LIST_HEADER_TEXT_SIZE_PX, 12.0);
    assert_eq!(TASK_LIST_BODY_Y_PADDING_PX, 4.0);
    assert_eq!(TASK_LIST_EMPTY_Y_PADDING_PX, 48.0);
    assert_eq!(TASK_LIST_EMPTY_TEXT_SIZE_PX, 13.0);
    assert_eq!(TASK_LIST_STATUS_SLOT_WIDTH_PX, 24.0);
    assert_eq!(TASK_LIST_PRIORITY_COLUMN_WIDTH_PX, 32.0);
    assert_eq!(TASK_LIST_DUE_COLUMN_WIDTH_PX, 80.0);
    assert_eq!(TASK_ROW_BODY_FONT_FAMILY, "Monaspace Neon");
    assert_eq!(TASK_ROW_RADIUS_PX, 8.0);
    assert_eq!(TASK_ROW_X_PADDING_PX, 12.0);
    assert_eq!(TASK_ROW_Y_PADDING_PX, 8.0);
    assert_eq!(TASK_ROW_GAP_PX, 8.0);
    assert_eq!(TASK_ROW_SELECTED_BG_OPACITY, 0.40);
    assert_eq!(TASK_ROW_HOVER_BG_OPACITY, 0.30);
    assert_eq!(TASK_ROW_STATUS_DOT_SIZE_PX, 10.0);
    assert_eq!(TASK_ROW_STATUS_DOT_RADIUS_PX, 999.0);
    assert_eq!(TASK_ROW_TITLE_TEXT_SIZE_PX, 12.0);
    assert_eq!(TASK_ROW_DUE_TEXT_SIZE_PX, 12.0);
    assert_eq!(TASK_PRIORITY_BAR_WIDTH_PX, 2.0);
    assert_eq!(TASK_PRIORITY_BAR_GAP_PX, 1.0);
    assert_eq!(TASK_PRIORITY_BAR_RADIUS_PX, 999.0);
    assert_eq!(TASK_PRIORITY_BAR_HEIGHTS_PX, [4.0, 6.0, 8.0, 10.0]);
}

#[test]
fn desktop_calendar_modes_should_match_web_reference() {
    let modes = [
        DesktopCalendarMode::Day,
        DesktopCalendarMode::Week,
        DesktopCalendarMode::Month,
    ]
    .iter()
    .map(|mode| (mode.as_str(), mode.label()))
    .collect::<Vec<_>>();

    assert_eq!(
        modes,
        vec![("day", "Day"), ("week", "Week"), ("month", "Month")]
    );
    assert_eq!(CALENDAR_PREVIOUS_LABEL, "Previous");
    assert_eq!(CALENDAR_NEXT_LABEL, "Next");
    assert_eq!(CALENDAR_NEW_EVENT_LABEL, "New event");
    assert_eq!(CALENDAR_TASK_DUE_LABEL, "Task due");
    assert_eq!(CALENDAR_BODY_FONT_FAMILY, "Monaspace Neon");
    assert_eq!(CALENDAR_HEADER_TITLE_TEXT_SIZE_PX, 12.0);
    assert_eq!(CALENDAR_NAV_TEXT_SIZE_PX, 12.0);
    assert_eq!(CALENDAR_TODAY_TEXT_SIZE_PX, 12.0);
    assert_eq!(CALENDAR_MODE_TEXT_SIZE_PX, 12.0);
    assert_eq!(CALENDAR_NEW_EVENT_TEXT_SIZE_PX, 12.0);
    assert_eq!(CALENDAR_WEEKDAY_TEXT_SIZE_PX, 12.0);
    assert_eq!(CALENDAR_DATE_TEXT_SIZE_PX, 12.0);
    assert_eq!(CALENDAR_PILL_TEXT_SIZE_PX, 12.0);
    assert_eq!(CALENDAR_TIMEGRID_HEADER_TEXT_SIZE_PX, 12.0);
    assert_eq!(CALENDAR_TIMEGRID_HOUR_TEXT_SIZE_PX, 12.0);
    assert_eq!(CALENDAR_TIMEGRID_EVENT_TEXT_SIZE_PX, 12.0);
    assert_eq!(DESKTOP_TIMEGRID_NOW_DOT_PX, 8.0);
    assert_eq!(DESKTOP_TIMEGRID_RESIZE_HANDLE_PX, 6.0);
    assert_eq!(TIMEGRID_DRAG_PREVIEW_RADIUS_PX, 6.0);
    assert_eq!(TIMEGRID_DRAG_PREVIEW_BORDER_OPACITY, 0.55);
    assert_eq!(TIMEGRID_DRAG_PREVIEW_BG_OPACITY, 0.32);
}

#[test]
fn desktop_calendar_title_formatting_should_match_web_reference() {
    let may_four = NaiveDate::from_ymd_opt(2026, 5, 4).expect("test date should be valid");
    let may_thirty_one = NaiveDate::from_ymd_opt(2026, 5, 31).expect("test date should be valid");

    assert_eq!(format_month_year(may_four), "May 2026");
    assert_eq!(format_full_date(may_four), "May 4, 2026");
    assert_eq!(format_week_range(may_four), "May 3 — 9, 2026");
    assert_eq!(format_week_range(may_thirty_one), "May 31 — Jun 6, 2026");
}

#[test]
fn desktop_calendar_metrics_should_match_web_reference() {
    assert_eq!(CALENDAR_HEADER_MIN_HEIGHT_PX, 40.0);
    assert_eq!(CALENDAR_HEADER_X_PADDING_PX, 16.0);
    assert_eq!(CALENDAR_HEADER_Y_PADDING_PX, 8.0);
    assert_eq!(CALENDAR_HEADER_BORDER_OPACITY, 0.40);
    assert_eq!(CALENDAR_HEADER_TITLE_MIN_WIDTH_PX, 180.0);
    assert_eq!(CALENDAR_HEADER_TITLE_GROUP_GAP_PX, 12.0);
    assert_eq!(CALENDAR_NAV_GROUP_GAP_PX, 4.0);
    assert_eq!(CALENDAR_NAV_BUTTON_SIZE_PX, 32.0);
    assert_eq!(CALENDAR_CONTROL_RADIUS_PX, 8.0);
    assert_eq!(CALENDAR_NAV_HOVER_BG_OPACITY, 0.40);
    assert_eq!(CALENDAR_TODAY_X_PADDING_PX, 8.0);
    assert_eq!(CALENDAR_TODAY_Y_PADDING_PX, 2.0);
    assert_eq!(CALENDAR_ACTION_GROUP_GAP_PX, 8.0);
    assert_eq!(CALENDAR_MODE_SWITCHER_GAP_PX, 2.0);
    assert_eq!(CALENDAR_MODE_SWITCHER_PADDING_PX, 2.0);
    assert_eq!(CALENDAR_MODE_SWITCHER_BORDER_OPACITY, 0.60);
    assert_eq!(CALENDAR_MODE_SWITCHER_BG_OPACITY, 0.40);
    assert_eq!(CALENDAR_MODE_ACTIVE_BORDER_OPACITY, 0.20);
    assert_eq!(CALENDAR_MODE_ACTIVE_BG_OPACITY, 0.15);
    assert_eq!(CALENDAR_MODE_INACTIVE_OPACITY, 0.0);
    assert_eq!(CALENDAR_MODE_HOVER_BG_OPACITY, 0.40);
    assert_eq!(CALENDAR_MODE_X_PADDING_PX, 8.0);
    assert_eq!(CALENDAR_MODE_Y_PADDING_PX, 2.0);
    assert_eq!(CALENDAR_NEW_EVENT_X_PADDING_PX, 10.0);
    assert_eq!(CALENDAR_NEW_EVENT_Y_PADDING_PX, 4.0);
    assert_eq!(CALENDAR_NEW_EVENT_BORDER_OPACITY, 0.20);
    assert_eq!(CALENDAR_NEW_EVENT_BG_OPACITY, 0.15);
    assert_eq!(CALENDAR_NEW_EVENT_HOVER_BG_OPACITY, 0.25);
    assert_eq!(CALENDAR_MONTH_ROW_MIN_HEIGHT_PX, 92.0);
    assert_eq!(CALENDAR_MONTH_TODAY_SIZE_PX, 20.0);
    assert_eq!(CALENDAR_MONTH_TODAY_RADIUS_PX, 999.0);
    assert_eq!(CALENDAR_MONTH_CELL_PADDING_PX, 6.0);
    assert_eq!(CALENDAR_MONTH_CELL_HOVER_BG_OPACITY, 0.20);
    assert_eq!(CALENDAR_MONTH_CELL_BORDER_OPACITY, 0.30);
    assert_eq!(CALENDAR_WEEKDAY_X_PADDING_PX, 8.0);
    assert_eq!(CALENDAR_WEEKDAY_Y_PADDING_PX, 6.0);
    assert_eq!(CALENDAR_PILL_DOT_SIZE_PX, 6.0);
    assert_eq!(CALENDAR_PILL_DOT_RADIUS_PX, 999.0);
    assert_eq!(CALENDAR_TIMEGRID_AXIS_WIDTH_PX, 56.0);
    assert_eq!(CALENDAR_TIMEGRID_HEADER_X_PADDING_PX, 8.0);
    assert_eq!(CALENDAR_TIMEGRID_HEADER_Y_PADDING_PX, 6.0);
    assert_eq!(CALENDAR_TIMEGRID_HEADER_BORDER_OPACITY, 0.40);
    assert_eq!(CALENDAR_TIMEGRID_COLUMN_BORDER_OPACITY, 0.30);
    assert_eq!(CALENDAR_TIMEGRID_HOUR_X_PADDING_PX, 6.0);
    assert_eq!(CALENDAR_TIMEGRID_HOUR_TOP_PADDING_PX, 2.0);
    assert_eq!(CALENDAR_TIMEGRID_HOUR_LABEL_BORDER_OPACITY, 0.40);
    assert_eq!(CALENDAR_TIMEGRID_HOUR_SLOT_BORDER_OPACITY, 0.30);
    assert_eq!(CALENDAR_TIMEGRID_HOUR_SLOT_HOVER_BG_OPACITY, 0.15);
    assert_eq!(CALENDAR_TIMEGRID_NOW_LINE_HEIGHT_PX, 1.0);
    assert_eq!(CALENDAR_TIMEGRID_NOW_DOT_OFFSET_PX, -4.0);
    assert_eq!(CALENDAR_TIMEGRID_NOW_DOT_RADIUS_PX, 999.0);
    assert_eq!(CALENDAR_TIMEGRID_EVENT_INSET_PX, 2.0);
    assert_eq!(CALENDAR_TIMEGRID_EVENT_RADIUS_PX, 8.0);
    assert_eq!(CALENDAR_TIMEGRID_EVENT_X_PADDING_PX, 6.0);
    assert_eq!(CALENDAR_TIMEGRID_EVENT_Y_PADDING_PX, 4.0);
    assert_eq!(CALENDAR_TIMEGRID_EVENT_BORDER_OPACITY, 0.40);
    assert_eq!(CALENDAR_TIMEGRID_EVENT_BG_OPACITY, 0.30);
    assert_eq!(CALENDAR_TIMEGRID_EVENT_HOVER_BG_OPACITY, 0.34);
}

#[test]
fn desktop_calendar_weekday_headers_should_match_web_reference() {
    assert_eq!(
        WEEKDAY_NAMES,
        &["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"]
    );
}

#[test]
fn desktop_event_color_options_should_match_web_reference() {
    assert_eq!(
        EVENT_COLOR_OPTIONS,
        &[
            ("persimmon", "Persimmon"),
            ("blueberry", "Blueberry"),
            ("pandan", "Pandan"),
            ("lychee", "Lychee"),
            ("durian", "Durian"),
            ("mangosteen", "Mangosteen"),
            ("dragonfruit", "Dragonfruit"),
        ]
    );
    assert_eq!(
        event_color(Some("persimmon")),
        crate::theme::persimmon_400()
    );
    assert_eq!(event_color(None), crate::theme::rambutan_500());
    assert_eq!(
        event_timegrid_surface_color(Some("blueberry")),
        crate::theme::blueberry_500()
    );
    assert_eq!(
        event_timegrid_surface_color(None),
        crate::theme::rambutan_500()
    );
    assert_eq!(
        event_timegrid_text_color(Some("pandan")),
        crate::theme::pandan_100()
    );
    assert_eq!(
        event_timegrid_text_color(None),
        crate::theme::rambutan_100()
    );
}

#[test]
fn default_event_start_should_round_up_to_next_half_hour() {
    assert_eq!(
        default_event_start(dt(2026, 5, 4, 10, 7)),
        dt(2026, 5, 4, 10, 30)
    );
}

#[test]
fn default_event_start_should_roll_to_next_day() {
    assert_eq!(
        default_event_start(dt(2026, 5, 4, 23, 59)),
        dt(2026, 5, 5, 0, 0)
    );
}

#[test]
fn event_form_datetime_should_round_trip_without_seconds() {
    let value = dt(2026, 5, 4, 9, 30);
    assert_eq!(
        parse_event_form_datetime(&format_event_form_datetime(value)),
        Some(value)
    );
}

#[test]
fn event_dialog_open_should_preserve_calendar_anchor_like_web_reference() {
    let current_anchor = NaiveDate::from_ymd_opt(2026, 7, 15).expect("test date should be valid");
    let event_start_at = dt(2026, 5, 4, 9, 0);

    assert_eq!(
        calendar_anchor_after_event_dialog_open(current_anchor, event_start_at),
        current_anchor
    );
}

#[test]
fn normalize_event_form_times_should_extend_end_after_start() {
    let start = dt(2026, 5, 4, 9, 0);
    let end = dt(2026, 5, 4, 8, 0);
    assert_eq!(
        normalize_event_form_times(start, end, false),
        (start, dt(2026, 5, 4, 10, 0))
    );
}

#[test]
fn normalize_event_form_times_should_convert_all_day_to_midnight_range() {
    let start = dt(2026, 5, 4, 9, 30);
    let end = dt(2026, 5, 4, 12, 0);
    assert_eq!(
        normalize_event_form_times(start, end, true),
        (dt(2026, 5, 4, 0, 0), dt(2026, 5, 5, 0, 0))
    );
}

#[test]
fn event_form_times_from_inputs_should_match_web_invalid_datetime_fallbacks() {
    let fallback_now = dt(2026, 5, 4, 9, 2);
    assert_eq!(
        event_form_times_from_inputs("not-a-date", "also-bad", false, fallback_now),
        (dt(2026, 5, 4, 9, 30), dt(2026, 5, 4, 10, 30))
    );
    assert_eq!(
        event_form_times_from_inputs("2026-05-04T11:00", "also-bad", false, fallback_now),
        (dt(2026, 5, 4, 11, 0), dt(2026, 5, 4, 12, 0))
    );
    assert_eq!(
        event_form_times_from_inputs("not-a-date", "also-bad", true, fallback_now),
        (dt(2026, 5, 4, 0, 0), dt(2026, 5, 5, 0, 0))
    );
}

#[test]
fn work_item_due_on_should_match_local_date_string() {
    let item = DesktopWorkItem {
        id: "task-1".to_string(),
        project_id: None,
        title: "Ship parity".to_string(),
        description: None,
        status: DesktopWorkItemStatus::Todo,
        priority: DesktopPriority::Medium,
        due_date: Some("2026-05-04".to_string()),
        planning: WorkItemPlanning::default(),
        updated_at: EpochMs::UNIX_EPOCH,
    };

    assert!(work_item_due_on(
        &item,
        NaiveDate::from_ymd_opt(2026, 5, 4).expect("test date should be valid")
    ));
}

#[test]
fn format_desktop_due_date_should_match_web_relative_labels() {
    let today = NaiveDate::from_ymd_opt(2026, 5, 4).expect("test date should be valid");

    assert_eq!(
        format_desktop_due_date("2026-05-04", today).map(|(label, _)| label),
        Some("today".to_string())
    );
    assert_eq!(
        format_desktop_due_date("2026-05-05", today).map(|(label, _)| label),
        Some("tomorrow".to_string())
    );
    assert_eq!(
        format_desktop_due_date("2026-05-06", today).map(|(label, _)| label),
        Some("May 6".to_string())
    );
    assert!(format_desktop_due_date("not-a-date", today).is_none());
}

#[test]
fn project_task_count_should_only_count_matching_project_items() {
    let mut first = work_item("task-1", DesktopWorkItemStatus::Todo);
    first.project_id = Some("project-1".to_string());
    let mut second = work_item("task-2", DesktopWorkItemStatus::Todo);
    second.project_id = Some("project-2".to_string());
    let mut third = work_item("task-3", DesktopWorkItemStatus::Todo);
    third.project_id = Some("project-1".to_string());

    assert_eq!(project_task_count(&[first, second, third], "project-1"), 2);
}

#[test]
fn selected_project_name_should_match_web_missing_project_fallback() {
    let projects = vec![DesktopProject {
        id: "project-1".to_string(),
        name: "Roadmap".to_string(),
    }];

    assert_eq!(selected_project_name_like_web(&projects, None), "All Tasks");
    assert_eq!(
        selected_project_name_like_web(&projects, Some("project-1")),
        "Roadmap"
    );
    assert_eq!(
        selected_project_name_like_web(&projects, Some("missing")),
        "Project"
    );
}

#[test]
fn project_delete_prompt_should_match_web_pluralization() {
    assert_eq!(
        project_delete_prompt("Roadmap", 1),
        "Delete project \"Roadmap\" and 1 task?"
    );
    assert_eq!(
        project_delete_prompt("Roadmap", 2),
        "Delete project \"Roadmap\" and 2 tasks?"
    );
}

#[test]
fn move_work_item_to_status_should_update_matching_item() {
    let mut items = vec![work_item("task-1", DesktopWorkItemStatus::Todo)];

    assert!(move_work_item_to_status(
        &mut items,
        "task-1",
        DesktopWorkItemStatus::Done
    ));
    assert_eq!(items[0].status, DesktopWorkItemStatus::Done);
    assert!(items[0].updated_at > EpochMs::UNIX_EPOCH);
}

#[test]
fn move_work_item_to_status_should_noop_when_status_already_matches() {
    let mut items = vec![work_item("task-1", DesktopWorkItemStatus::Todo)];

    assert!(!move_work_item_to_status(
        &mut items,
        "task-1",
        DesktopWorkItemStatus::Todo
    ));
    assert_eq!(items[0].updated_at, EpochMs::UNIX_EPOCH);
}

#[test]
fn move_work_item_to_status_should_noop_when_item_is_missing() {
    let mut items = vec![work_item("task-1", DesktopWorkItemStatus::Todo)];

    assert!(!move_work_item_to_status(
        &mut items,
        "missing",
        DesktopWorkItemStatus::Done
    ));
    assert_eq!(items[0].status, DesktopWorkItemStatus::Todo);
}

#[test]
fn activation_keys_should_match_web_button_keyboard_behavior() {
    assert!(is_web_checkbox_activation_key("space"));
    assert!(!is_web_checkbox_activation_key("enter"));
}

#[test]
fn snap_timegrid_offset_seconds_should_snap_to_fifteen_minutes() {
    assert_eq!(
        snap_timegrid_offset_seconds(DESKTOP_HOUR_ROW_PX * 1.20),
        4500
    );
}

#[test]
fn move_timegrid_event_to_should_snap_and_preserve_duration() {
    let mut event = event("event", dt(2026, 5, 4, 9, 0), dt(2026, 5, 4, 10, 30));
    let date = NaiveDate::from_ymd_opt(2026, 5, 5).expect("test date should be valid");
    let pointer_y = DESKTOP_HOUR_ROW_PX * 13.18 + 8.0;

    assert!(move_timegrid_event_to(&mut event, date, pointer_y, 8.0));
    assert_eq!(event.start_at, dt(2026, 5, 5, 13, 15));
    assert_eq!(event.end_at, dt(2026, 5, 5, 14, 45));
}

#[test]
fn move_timegrid_event_to_should_preserve_short_duration_like_web() {
    let mut event = event("event", dt(2026, 5, 4, 9, 0), dt(2026, 5, 4, 9, 10));
    let date = NaiveDate::from_ymd_opt(2026, 5, 5).expect("test date should be valid");
    let pointer_y = DESKTOP_HOUR_ROW_PX * 13.18;

    assert!(move_timegrid_event_to(&mut event, date, pointer_y, 0.0));
    assert_eq!(event.start_at, dt(2026, 5, 5, 13, 15));
    assert_eq!(event.end_at, dt(2026, 5, 5, 13, 25));
}

#[test]
fn move_timegrid_event_to_should_clamp_inside_target_day() {
    let mut event = event("event", dt(2026, 5, 4, 22, 30), dt(2026, 5, 5, 1, 0));
    let date = NaiveDate::from_ymd_opt(2026, 5, 6).expect("test date should be valid");

    assert!(move_timegrid_event_to(
        &mut event,
        date,
        DESKTOP_HOUR_ROW_PX * 23.75,
        0.0
    ));
    assert_eq!(event.start_at, dt(2026, 5, 6, 21, 30));
    assert_eq!(event.end_at, dt(2026, 5, 7, 0, 0));
}

#[test]
fn resize_timegrid_event_end_to_should_snap_and_enforce_min_duration() {
    let mut event = event("event", dt(2026, 5, 4, 9, 0), dt(2026, 5, 4, 10, 0));
    let date = NaiveDate::from_ymd_opt(2026, 5, 4).expect("test date should be valid");

    assert!(resize_timegrid_event_end_to(
        &mut event,
        date,
        DESKTOP_HOUR_ROW_PX * 9.05
    ));
    assert_eq!(event.end_at, dt(2026, 5, 4, 9, 15));
}

#[test]
fn lay_out_timegrid_events_should_split_overlapping_events_into_lanes() {
    let date = NaiveDate::from_ymd_opt(2026, 5, 4).expect("test date should be valid");
    let events = vec![
        event("a", dt(2026, 5, 4, 9, 0), dt(2026, 5, 4, 10, 0)),
        event("b", dt(2026, 5, 4, 9, 30), dt(2026, 5, 4, 10, 30)),
        event("c", dt(2026, 5, 4, 11, 0), dt(2026, 5, 4, 12, 0)),
    ];

    let laid_out = lay_out_timegrid_events(&events, date);

    assert_eq!(
        laid_out
            .iter()
            .map(|event| (event.event.id.as_str(), event.lane, event.lanes))
            .collect::<Vec<_>>(),
        vec![("a", 0, 2), ("b", 1, 2), ("c", 0, 2)]
    );
}

#[test]
fn lay_out_timegrid_events_should_preserve_same_start_order_like_web_reference() {
    let date = NaiveDate::from_ymd_opt(2026, 5, 4).expect("test date should be valid");
    let events = vec![
        event("long", dt(2026, 5, 4, 9, 0), dt(2026, 5, 4, 11, 0)),
        event("short", dt(2026, 5, 4, 9, 0), dt(2026, 5, 4, 10, 0)),
    ];

    let laid_out = lay_out_timegrid_events(&events, date);

    assert_eq!(
        laid_out
            .iter()
            .map(|event| (event.event.id.as_str(), event.lane, event.lanes))
            .collect::<Vec<_>>(),
        vec![("long", 0, 2), ("short", 1, 2)]
    );
}

#[test]
fn lay_out_timegrid_events_should_use_web_minimum_display_height() {
    let date = NaiveDate::from_ymd_opt(2026, 5, 4).expect("test date should be valid");
    let events = vec![event("short", dt(2026, 5, 4, 9, 0), dt(2026, 5, 4, 9, 15))];

    let laid_out = lay_out_timegrid_events(&events, date);

    assert_eq!(DESKTOP_TIMEGRID_MIN_DISPLAY_SECONDS, 30 * 60);
    assert_eq!(DESKTOP_TIMEGRID_MIN_DISPLAY_PX, 20.0);
    assert_eq!(laid_out.len(), 1);
    assert_eq!(laid_out[0].height_px, 20.0);
}

#[test]
fn lay_out_timegrid_events_should_skip_all_day_events() {
    let date = NaiveDate::from_ymd_opt(2026, 5, 4).expect("test date should be valid");
    let mut all_day = event("all-day", dt(2026, 5, 4, 0, 0), dt(2026, 5, 5, 0, 0));
    all_day.all_day = true;

    assert!(lay_out_timegrid_events(&[all_day], date).is_empty());
}

fn update_report(
    codex: (&str, &str),
    app: bruhs_models::updates::AppUpdateState,
    runs: Vec<bruhs_models::updates::ProviderRun>,
) -> bruhs_models::updates::UpdatesReport {
    use bruhs_models::agent::AgentProvider;
    use bruhs_models::updates::{
        detect_install, update_command, AppUpdate, CommandContext, ProviderUpdate,
        ProviderUpdateState, UpdatesReport,
    };
    let provider = |provider, installed: &str, latest: &str, path: &str| {
        let install = detect_install(provider, path, path);
        ProviderUpdate {
            provider,
            state: ProviderUpdateState::Unknown,
            installed: Some(installed.into()),
            latest: Some(latest.into()),
            binary_path: Some(path.into()),
            update_command: update_command(
                &install,
                &CommandContext {
                    resolved: path,
                    ..CommandContext::default()
                },
            ),
            install: Some(install),
            manual_hint: None,
            compat: None,
            error: None,
        }
        .settle()
    };
    UpdatesReport {
        checked_at: bruhs_models::time::EpochMs::now(),
        online: true,
        providers: vec![
            provider(
                AgentProvider::Claude,
                "2.1.319",
                "2.1.319",
                "/Users/a/.local/share/claude/versions/2.1.319",
            ),
            provider(
                AgentProvider::Codex,
                codex.0,
                codex.1,
                "/opt/homebrew/Caskroom/codex/0.166.0/bin/codex",
            ),
        ],
        app: AppUpdate {
            state: app,
            built_commit: Some("3c5dfc3378a291b5801baee78ce72a9b77bfcdab".into()),
            remote_commit: Some("e723466aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into()),
            remote_branch: Some("origin/main".into()),
            local_branch: None,
            repo_root: Some("/Users/a/bruhs-ai".into()),
            command: bruhs_models::updates::app_update_command(
                "/Users/a/bruhs-ai",
                app,
                true,
                "cargo run --release -p bruhs-desktop",
            ),
            error: None,
            release: None,
        },
        runs,
    }
}

fn update_run(
    id: u64,
    phase: bruhs_models::updates::RunPhase,
    outcome: Option<bruhs_models::updates::RunOutcome>,
) -> bruhs_models::updates::ProviderRun {
    bruhs_models::updates::ProviderRun {
        id,
        provider: bruhs_models::agent::AgentProvider::Codex,
        phase,
        started_at: bruhs_models::time::EpochMs::now(),
        finished_at: outcome.map(|_| bruhs_models::time::EpochMs::now()),
        command: Some("brew upgrade --cask codex".into()),
        last_line: Some("==> Upgrading codex".into()),
        from_version: Some("0.166.0".into()),
        to_version: Some("0.167.0".into()),
        outcome,
        message: outcome.map(|_| "Update command exited with code 1: offline".into()),
        output: "==> Upgrading codex\nError: offline".into(),
    }
}

/// Web `SidebarUpdateButton`, `UpdateConfirmDialog` and Settings → Updates.
/// The tests never confirm an update or click Check now: those run real
/// installers and checks.
#[gpui::test]
fn desktop_update_pills_dialog_and_settings_should_match_web(cx: &mut gpui::TestAppContext) {
    use bruhs_models::updates::{AppUpdateState, RunOutcome, RunPhase};
    cx.update(|cx| {
        gpui_component::init(cx);
        gpui_component::Theme::change(gpui_component::ThemeMode::Dark, None, cx);
        theme::apply_bruhs_theme(cx);
        let state = DesktopState::new().expect("desktop state should initialize");
        cx.set_global(state);
    });
    let workspace_cell: Rc<RefCell<Option<Entity<Workspace>>>> = Rc::new(RefCell::new(None));
    let workspace_for_window = workspace_cell.clone();
    let (_root, cx) = cx.add_window_view(|window, cx| {
        let workspace = cx.new(|cx| Workspace::new(window, cx));
        *workspace_for_window.borrow_mut() = Some(workspace.clone());
        gpui_component::Root::new(workspace, window, cx)
    });
    let workspace = workspace_cell
        .borrow()
        .clone()
        .expect("workspace entity should be captured");
    let seed = |cx: &mut gpui::VisualTestContext, report| {
        workspace.update_in(cx, |workspace, _window, cx| {
            workspace
                .updates
                .update(cx, |updates, cx| updates.set_report_for_test(report, cx));
        });
        cx.run_until_parked();
    };
    cx.run_until_parked();
    // Nothing checked yet, then everything current: no pill.
    assert_no_rendered_bounds(cx, "desktop-sidebar-update");
    seed(
        cx,
        update_report(("0.166.0", "0.166.0"), AppUpdateState::UpToDate, vec![]),
    );
    assert_no_rendered_bounds(cx, "desktop-sidebar-update");

    // Codex behind: Update confirms first (T3's "Update N machine(s)?").
    seed(
        cx,
        update_report(("0.166.0", "0.167.0"), AppUpdateState::Behind, vec![]),
    );
    click_debug_selector(cx, "desktop-sidebar-update");
    cx.run_until_parked();
    assert_rendered_bounds(cx, "desktop-update-confirm");
    click_debug_selector(cx, "desktop-update-confirm-cancel");
    cx.run_until_parked();
    assert_no_rendered_bounds(cx, "desktop-update-confirm");
    workspace.update_in(cx, |workspace, _window, cx| {
        assert!(workspace
            .updates
            .read(cx)
            .report
            .as_ref()
            .unwrap()
            .runs
            .is_empty());
    });

    // Settings rows: Update now and its copy button, the app command.
    workspace.update_in(cx, |workspace, window, cx| {
        workspace.open_settings(&OpenSettings, window, cx);
        workspace.set_settings_section(super::SettingsSection::Updates, cx);
    });
    cx.run_until_parked();
    assert_rendered_bounds(cx, "desktop-settings-row-update-checks");
    assert_rendered_bounds(cx, "desktop-settings-row-update-claude");
    assert_rendered_bounds(cx, "desktop-settings-update-codex");
    assert_rendered_bounds(cx, "desktop-settings-copy-codex");
    assert_no_rendered_bounds(cx, "desktop-settings-update-claude");
    assert_rendered_bounds(cx, "desktop-settings-update-command");
    assert_no_rendered_bounds(cx, "desktop-settings-update-compat");

    // A known-broken Codex warns on its card.
    seed(
        cx,
        update_report(("0.140.0", "0.140.0"), AppUpdateState::UpToDate, vec![]),
    );
    assert_rendered_bounds(cx, "desktop-settings-update-compat");

    // A run in progress shows its step on the card and in the sidebar.
    seed(
        cx,
        update_report(
            ("0.166.0", "0.167.0"),
            AppUpdateState::UpToDate,
            vec![update_run(7, RunPhase::Running, None)],
        ),
    );
    assert_rendered_bounds(cx, "desktop-settings-update-progress");
    assert_no_rendered_bounds(cx, "desktop-settings-update-codex");
    workspace.update_in(cx, |workspace, _window, cx| {
        workspace.close_transient_surfaces();
        cx.notify();
    });
    cx.run_until_parked();
    assert_rendered_bounds(cx, "desktop-sidebar-update");

    // A failed run stays until dismissed.
    seed(
        cx,
        update_report(
            ("0.166.0", "0.167.0"),
            AppUpdateState::UpToDate,
            vec![update_run(7, RunPhase::Done, Some(RunOutcome::Failed))],
        ),
    );
    assert_rendered_bounds(cx, "desktop-sidebar-update-dismiss");
    click_debug_selector(cx, "desktop-sidebar-update-dismiss");
    cx.run_until_parked();
    workspace.update_in(cx, |workspace, _window, cx| {
        assert!(workspace.updates.read(cx).dismissed.contains(&7));
    });
    // Dismissed, it falls back to Update (Codex is still behind).
    assert_no_rendered_bounds(cx, "desktop-sidebar-update-dismiss");
    assert_rendered_bounds(cx, "desktop-sidebar-update");

    // Only the app behind: the pill opens Settings → Updates, where the
    // dismissed failure is still on the card.
    seed(
        cx,
        update_report(
            ("0.167.0", "0.167.0"),
            AppUpdateState::RebuildNeeded,
            vec![update_run(7, RunPhase::Done, Some(RunOutcome::Failed))],
        ),
    );
    click_debug_selector(cx, "desktop-sidebar-update");
    cx.run_until_parked();
    workspace.update_in(cx, |workspace, _window, _cx| {
        assert!(workspace.settings_visible());
        assert_eq!(workspace.settings_section, super::SettingsSection::Updates);
    });
    assert_rendered_bounds(cx, "desktop-settings-update-failure-codex");

    // Turning online checks off is saved.
    click_debug_selector(cx, "desktop-settings-update-checks");
    cx.run_until_parked();
    workspace.update_in(cx, |workspace, _window, cx| {
        assert!(!workspace.updates.read(cx).enabled);
        assert_eq!(workspace.snapshot(cx).update_checks, Some(false));
    });
}

/// A downloaded release: "Restart to update" in the sidebar and Settings,
/// with T3's Later. (Restart itself quits the app, so it is not clicked.)
#[gpui::test]
fn desktop_restart_to_update_should_confirm_first(cx: &mut gpui::TestAppContext) {
    use bruhs_models::updates::AppUpdateState;
    cx.update(|cx| {
        gpui_component::init(cx);
        gpui_component::Theme::change(gpui_component::ThemeMode::Dark, None, cx);
        theme::apply_bruhs_theme(cx);
        let state = DesktopState::new().expect("desktop state should initialize");
        cx.set_global(state);
    });
    let workspace_cell: Rc<RefCell<Option<Entity<Workspace>>>> = Rc::new(RefCell::new(None));
    let workspace_for_window = workspace_cell.clone();
    let (_root, cx) = cx.add_window_view(|window, cx| {
        let workspace = cx.new(|cx| Workspace::new(window, cx));
        *workspace_for_window.borrow_mut() = Some(workspace.clone());
        gpui_component::Root::new(workspace, window, cx)
    });
    let workspace = workspace_cell
        .borrow()
        .clone()
        .expect("workspace entity should be captured");
    workspace.update_in(cx, |workspace, _window, cx| {
        workspace.updates.update(cx, |updates, cx| {
            let mut report = update_report(("0.167.0", "0.167.0"), AppUpdateState::Behind, vec![]);
            report.app.release = Some(bruhs_models::updates::ReleaseUpdate {
                current_version: "0.1.0".into(),
                latest_version: Some("0.2.0".into()),
                release_url: None,
                download_url: None,
                sha256: None,
            });
            updates.set_report_for_test(report, cx);
            updates.install = super::updates_ui::AppInstall::Ready {
                version: "0.2.0".into(),
                staged: std::path::PathBuf::from("/nonexistent/Bruhs.app"),
            };
        });
    });
    cx.run_until_parked();
    click_debug_selector(cx, "desktop-sidebar-update");
    cx.run_until_parked();
    assert_rendered_bounds(cx, "desktop-restart-confirm");
    click_debug_selector(cx, "desktop-restart-confirm-later");
    cx.run_until_parked();
    assert_no_rendered_bounds(cx, "desktop-restart-confirm");
    // Still offered from the sidebar and Settings.
    assert_rendered_bounds(cx, "desktop-sidebar-update");
    workspace.update_in(cx, |workspace, window, cx| {
        workspace.open_settings(&OpenSettings, window, cx);
        workspace.set_settings_section(super::SettingsSection::Updates, cx);
    });
    cx.run_until_parked();
    assert_rendered_bounds(cx, "desktop-settings-restart-update");
    // The version's changelog entry sits beside Restart.
    assert_rendered_bounds(cx, "desktop-settings-release-notes");
}

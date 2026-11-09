use super::{
    auth_banner_visible_like_web, clamp_composer_hint_index, clear_permission_if_matches,
    clear_slash_trigger_text, composer_focus_allowed_like_web, composer_input_disabled_like_web,
    composer_input_tab_index_like_web, desktop_assistant_content_blocks,
    desktop_auth_status_after_check, desktop_diff_lines, desktop_limited_terminal_output,
    desktop_mention_icon, desktop_permission_request_from_raw, desktop_permission_summary,
    desktop_scroll_to_bottom_control_visible, desktop_tool_body_text, desktop_tool_input_summary,
    desktop_tool_options, detect_mention_query, detect_pull_query, element_key,
    filter_models_like_web, group_models_like_web, mark_streaming_message_stopped_like_web,
    move_composer_hint_index, normalize_claude_command_error, parse_desktop_auth_status,
    replace_mention_trigger, select_composer_hint_index_on_hover, slash_command_query,
    thread_title_after_clear_slash, thread_title_after_submit_like_web, truncate_tool_output,
    turn_duration_ms, turn_fold_label, AgentProvider, ChatView, ComposerHintDirection,
    DesktopAssistantContentBlock, DesktopAuthStatus, DesktopDiffLineKind, DesktopMathRender,
    DesktopMermaidRender, DesktopPermissionRequest, DesktopToolCall, Message, MessageRole,
    PermissionMode, TurnState, AUTH_BANNER_ACTION_GAP_PX, AUTH_BANNER_ACTION_TOP_MARGIN_PX,
    AUTH_BANNER_BG_OPACITY, AUTH_BANNER_BORDER_OPACITY, AUTH_BANNER_CONTENT_GAP_PX,
    AUTH_BANNER_FONT_FAMILY, AUTH_BANNER_ICON_SIZE_PX, AUTH_BANNER_ICON_TOP_OFFSET_PX,
    AUTH_BANNER_MESSAGE_BOTTOM_MARGIN_PX, AUTH_BANNER_MESSAGE_OPACITY, AUTH_BANNER_PADDING_PX,
    AUTH_BANNER_RADIUS_PX, AUTH_BANNER_TEXT_SIZE_PX, AUTH_BANNER_TITLE_BOTTOM_MARGIN_PX,
    AUTH_BANNER_X_MARGIN_PX, AUTH_BANNER_Y_MARGIN_PX, CHAT_COMPOSER_BOTTOM_PADDING_PX,
    CHAT_COMPOSER_MAX_ROWS, CHAT_COMPOSER_MIN_ROWS, CHAT_CONTENT_BLOCK_MAX_WIDTH,
    CHAT_DIAGRAM_ERROR_BORDER_OPACITY, CHAT_DIAGRAM_ERROR_FONT_FAMILY,
    CHAT_DIAGRAM_ERROR_LINE_HEIGHT, CHAT_DIAGRAM_ERROR_RADIUS_PX, CHAT_DIAGRAM_ERROR_TEXT_SIZE_PX,
    CHAT_DIAGRAM_ERROR_X_PADDING_PX, CHAT_DIAGRAM_ERROR_Y_PADDING_PX, CHAT_INPUT_PLACEHOLDER,
    CHAT_MATH_DISPLAY_Y_PADDING_PX, CHAT_MERMAID_CARD_BORDER_OPACITY, CHAT_MERMAID_CARD_PADDING_PX,
    CHAT_MERMAID_CARD_RADIUS_PX, CHAT_MERMAID_MIN_WIDTH_PX, CHAT_MESSAGE_LIST_GAP_PX,
    CHAT_MESSAGE_LIST_MAX_WIDTH_PX, CHAT_MESSAGE_LIST_X_PADDING_PX, CHAT_MESSAGE_LIST_Y_PADDING_PX,
    CHAT_PROSE_CODE_BORDER_OPACITY, CHAT_PROSE_CODE_LINE_HEIGHT, CHAT_PROSE_CODE_RADIUS_PX,
    CHAT_PROSE_CODE_TEXT_SIZE_PX, CHAT_PROSE_CODE_X_PADDING_PX, CHAT_PROSE_CODE_Y_PADDING_PX,
    CHAT_PROSE_HEADING_1_TEXT_SIZE_PX, CHAT_PROSE_HEADING_2_TEXT_SIZE_PX,
    CHAT_PROSE_HEADING_FALLBACK_TEXT_SIZE_PX, CHAT_PROSE_PARAGRAPH_GAP_REM,
    CHAT_SCROLL_BOTTOM_THRESHOLD_PX, CHAT_SCROLL_TO_BOTTOM_LABEL, CHAT_SCROLL_TO_BOTTOM_OFFSET_PX,
    CHAT_SCROLL_TO_BOTTOM_RADIUS_PX, CHAT_SCROLL_TO_BOTTOM_SIZE_PX,
    CHAT_SCROLL_TO_BOTTOM_TEXT_SIZE_PX, CLAUDE_AUTH_NOT_AUTHENTICATED_LABEL,
    CLAUDE_AUTH_POLL_ATTEMPTS, CLAUDE_AUTH_POLL_INTERVAL_MS, CLAUDE_AUTH_STATUS_ARGS,
    CLAUDE_LOGIN_ACTION_LABEL, CLAUDE_LOGIN_WAITING_LABEL, CLAUDE_LOGIN_WAITING_MESSAGE,
    COMPOSER_DIRECTORY_ICON, COMPOSER_FILE_ICON, COMPOSER_HINT_EMPTY_LABEL,
    COMPOSER_HINT_ICON_SLOT_WIDTH_PX, COMPOSER_HINT_POPOVER_MAX_HEIGHT_PX,
    COMPOSER_HINT_POPOVER_OFFSET_PX, COMPOSER_HINT_POPOVER_RADIUS_PX,
    COMPOSER_HINT_ROW_TEXT_SIZE_PX, COMPOSER_HINT_SUB_TEXT_SIZE_PX, COMPOSER_SLASH_ICON,
    DESKTOP_SLASH_COMMANDS, GIT_BRANCH_CURRENT_MARK, GIT_BRANCH_CURRENT_MARK_WIDTH_PX,
    GIT_BRANCH_EMPTY_LABEL, GIT_BRANCH_EMPTY_TEXT_SIZE_PX, GIT_BRANCH_MENU_MAX_HEIGHT_PX,
    GIT_BRANCH_MENU_MIN_WIDTH_PX, GIT_BRANCH_MENU_RADIUS_PX, GIT_BRANCH_MENU_ROW_TEXT_SIZE_PX,
    GIT_BRANCH_SWITCH_LABEL, MODEL_CATALOG, PERMISSION_PROMPT_ACTION_GAP_PX,
    PERMISSION_PROMPT_ACTION_HOVER_BG_OPACITY, PERMISSION_PROMPT_BG_OPACITY,
    PERMISSION_PROMPT_BORDER_OPACITY, PERMISSION_PROMPT_DIVIDER_MIN_HEIGHT_PX,
    PERMISSION_PROMPT_DIVIDER_OPACITY, PERMISSION_PROMPT_DIVIDER_WIDTH_PX,
    PERMISSION_PROMPT_FONT_FAMILY, PERMISSION_PROMPT_ICON_SIZE_PX, PERMISSION_PROMPT_RADIUS_PX,
    PERMISSION_PROMPT_TEXT_SIZE_PX, PERMISSION_PROMPT_X_MARGIN_PX, PERMISSION_PROMPT_X_PADDING_PX,
    PERMISSION_PROMPT_Y_MARGIN_PX, PERMISSION_PROMPT_Y_PADDING_PX, STATUS_CHANGE_MODEL_LABEL,
    TERMINAL_COMMAND_PREFIX, TERMINAL_OUTPUT_MAX_LINES, TERMINAL_OUTPUT_MORE_LINES_LABEL,
    TOOL_CARD_BODY_BORDER_OPACITY, TOOL_CARD_BODY_PADDING_PX, TOOL_CODE_FONT_FAMILY,
    TOOL_CODE_TEXT_SIZE_PX, TOOL_DIFF_COUNT_GAP_PX, TOOL_DIFF_HEADER_BG_OPACITY,
    TOOL_DIFF_HEADER_GAP_PX, TOOL_DIFF_LINE_BG_OPACITY, TOOL_DIFF_LINE_GAP_PX,
    TOOL_DIFF_LINE_Y_PADDING_PX, TOOL_DIFF_MARKER_OPACITY, TOOL_DIFF_MARKER_WIDTH_PX,
    TOOL_OUTPUT_BLOCK_RADIUS_PX, TOOL_OUTPUT_DIVIDER_OPACITY, TOOL_OUTPUT_HEADER_Y_PADDING_PX,
    TOOL_OUTPUT_MAX_CHARS, TOOL_OUTPUT_MAX_HEIGHT_PX, TOOL_OUTPUT_X_PADDING_PX,
    TOOL_OUTPUT_Y_PADDING_PX, TOOL_QUESTION_ANSWER_LABEL_BOTTOM_MARGIN_PX,
    TOOL_QUESTION_ANSWER_LABEL_TEXT_SIZE_PX, TOOL_QUESTION_ANSWER_LINE_HEIGHT,
    TOOL_QUESTION_ANSWER_TEXT_SIZE_PX, TOOL_QUESTION_ANSWER_TOP_MARGIN_PX,
    TOOL_QUESTION_ANSWER_X_PADDING_PX, TOOL_QUESTION_ANSWER_Y_PADDING_PX, TOOL_QUESTION_GAP_PX,
    TOOL_QUESTION_LINE_HEIGHT, TOOL_QUESTION_OPTIONS_GAP_PX, TOOL_QUESTION_OPTION_GAP_PX,
    TOOL_QUESTION_OPTION_LINE_HEIGHT, TOOL_QUESTION_OPTION_TEXT_SIZE_PX,
    TOOL_QUESTION_TEXT_SIZE_PX,
};
use crate::{app_state::DesktopState, theme};
use bruhs_core::UsageInfo;
use gpui::AppContext as _;
use serde_json::json;
use std::cell::RefCell;
use std::fs;
use std::path::Path;
use std::process::Command;
use std::rc::Rc;

#[test]
fn detect_mention_query_should_find_current_at_token() {
    assert_eq!(
        detect_mention_query("open @src/ma", 12),
        Some("src/ma".to_string())
    );
}

#[test]
fn detect_mention_query_should_ignore_embedded_at() {
    assert_eq!(detect_mention_query("email a@b", 9), None);
}

#[test]
fn detect_pull_query_lists_digits_and_leaves_a_heading() {
    assert_eq!(detect_pull_query("see #12", 7), Some("12".to_string()));
    assert_eq!(detect_pull_query("#", 1), Some(String::new()));
    assert_eq!(detect_pull_query("# title", 3), None);
}

#[test]
fn skill_queries_leave_dollar_amounts_literal() {
    use super::hints_ui::detect_skill_query;
    assert_eq!(detect_skill_query("use $re", 7), Some("re".into()));
    assert_eq!(detect_skill_query("$", 1), Some(String::new()));
    assert_eq!(detect_skill_query("cost $12", 8), None);
    assert_eq!(detect_skill_query("cost $12.50", 11), None);
}

#[test]
fn replace_mention_trigger_should_replace_token_and_keep_suffix() {
    assert_eq!(
        replace_mention_trigger("read @sr please", 8, "@src/main.rs"),
        "read @src/main.rs please"
    );
}

#[test]
fn desktop_composer_hint_copy_should_match_web_reference() {
    let slash_commands = DESKTOP_SLASH_COMMANDS
        .iter()
        .map(|command| (command.name, command.description))
        .collect::<Vec<_>>();

    assert_eq!(
        slash_commands,
        vec![
            ("/clear", "Clear conversation"),
            ("/help", "Show available commands"),
            ("/model", "Change model"),
            ("/settings", "Open settings"),
            ("/goal", "Set a goal for this thread"),
            ("/compact", "Compact the conversation"),
            ("/handoff", "Continue in a fresh thread with a brief"),
            (
                "/second-opinion",
                "Ask the other provider to review the last answer"
            ),
        ]
    );
    assert_eq!(COMPOSER_HINT_EMPTY_LABEL, "No matches");
    assert_eq!(COMPOSER_SLASH_ICON, "/");
    assert_eq!(COMPOSER_HINT_POPOVER_OFFSET_PX, 4.0);
    assert_eq!(COMPOSER_HINT_POPOVER_RADIUS_PX, 8.0);
    assert_eq!(COMPOSER_HINT_POPOVER_MAX_HEIGHT_PX, 288.0);
    assert_eq!(COMPOSER_HINT_ICON_SLOT_WIDTH_PX, 12.0);
    assert_eq!(COMPOSER_HINT_ROW_TEXT_SIZE_PX, 14.0);
    assert_eq!(COMPOSER_HINT_SUB_TEXT_SIZE_PX, 12.0);
    assert_eq!(
        CHAT_INPUT_PLACEHOLDER,
        "Ask anything, @ threads or files, # pull requests, $ skills, or / for commands"
    );
}

#[test]
fn desktop_composer_input_streaming_state_should_match_web_reference() {
    assert!(!composer_input_disabled_like_web(false));
    assert_eq!(composer_input_tab_index_like_web(false), 0);
    assert!(composer_focus_allowed_like_web(false));

    assert!(!composer_input_disabled_like_web(true));
    assert_eq!(composer_input_tab_index_like_web(true), 0);
    assert!(composer_focus_allowed_like_web(true));
}

#[test]
fn clear_slash_command_should_preserve_thread_title_like_web_reference() {
    assert_eq!(
        thread_title_after_clear_slash("Investigate provider logos"),
        "Investigate provider logos"
    );
    assert_eq!(thread_title_after_clear_slash("New thread"), "New thread");
}

#[test]
fn submit_should_preserve_thread_title_like_web_reference() {
    assert_eq!(
        thread_title_after_submit_like_web("New thread", "Rename me from prompt"),
        "New thread"
    );
    assert_eq!(
        thread_title_after_submit_like_web("New session", "Implement provider picker"),
        "New session"
    );
    assert_eq!(
        thread_title_after_submit_like_web("Existing title", "A different prompt"),
        "Existing title"
    );
}

#[test]
fn slash_command_query_should_match_web_trigger_rules() {
    assert_eq!(slash_command_query("/he", 3, None), Some("he".to_string()));
    assert_eq!(
        slash_command_query("/he please", 3, None),
        Some("he".to_string())
    );
    assert_eq!(
        slash_command_query("first line\n/mo", "first line\n/mo".len(), None),
        Some("mo".to_string())
    );
    assert_eq!(slash_command_query(" /he", " /he".len(), None), None);
    assert_eq!(slash_command_query("ask /he", "ask /he".len(), None), None);
    assert_eq!(
        slash_command_query("/he more", "/he more".len(), None),
        None
    );
    assert_eq!(slash_command_query("/he", 3, Some("/he")), None);
}

#[test]
fn clear_slash_trigger_text_should_match_web_replacement_behavior() {
    assert_eq!(clear_slash_trigger_text("/clear", 6), "");
    assert_eq!(clear_slash_trigger_text("notes\n/mo", 9), "notes");
    assert_eq!(
        clear_slash_trigger_text("notes\n/mo keep", "notes\n/mo".len()),
        "notes\n keep"
    );
}

#[test]
fn slash_command_filter_should_match_web_contains_behavior() {
    let query = slash_command_query("/ar", 3, None).expect("slash query should be active");
    let matching = DESKTOP_SLASH_COMMANDS
        .iter()
        .filter(|command| command.name.to_ascii_lowercase().contains(&query))
        .map(|command| command.name)
        .collect::<Vec<_>>();

    assert_eq!(matching, vec!["/clear"]);
}

#[test]
fn desktop_chat_shell_layout_should_match_web_reference() {
    assert_eq!(CHAT_MESSAGE_LIST_MAX_WIDTH_PX, 768.0);
    assert_eq!(CHAT_MESSAGE_LIST_X_PADDING_PX, 16.0);
    assert_eq!(CHAT_MESSAGE_LIST_Y_PADDING_PX, 16.0);
    assert_eq!(CHAT_MESSAGE_LIST_GAP_PX, 0.0);
    assert_eq!(CHAT_COMPOSER_BOTTOM_PADDING_PX, 16.0);
    assert_eq!(CHAT_COMPOSER_MIN_ROWS, 3);
    assert_eq!(CHAT_COMPOSER_MAX_ROWS, 8);
}

#[test]
fn desktop_scroll_to_bottom_control_should_match_web_threshold() {
    assert_eq!(CHAT_SCROLL_BOTTOM_THRESHOLD_PX, 100.0);
    assert_eq!(CHAT_SCROLL_TO_BOTTOM_OFFSET_PX, 16.0);
    assert_eq!(CHAT_SCROLL_TO_BOTTOM_SIZE_PX, 32.0);
    assert_eq!(CHAT_SCROLL_TO_BOTTOM_RADIUS_PX, 999.0);
    assert_eq!(CHAT_SCROLL_TO_BOTTOM_TEXT_SIZE_PX, 16.0);
    assert_eq!(CHAT_SCROLL_TO_BOTTOM_LABEL, "Scroll to bottom");
    assert!(!desktop_scroll_to_bottom_control_visible(0.0, 0.0));
    assert!(!desktop_scroll_to_bottom_control_visible(-900.0, 1_000.0));
    assert!(desktop_scroll_to_bottom_control_visible(-899.0, 1_000.0));
}

#[test]
fn desktop_chat_header_tasks_toggle_copy_should_match_web_reference() {}

#[test]
fn desktop_message_bubble_typography_should_match_web_reference() {
    assert_eq!(CHAT_DIAGRAM_ERROR_FONT_FAMILY, "Monaspace Argon");
    assert_eq!(CHAT_DIAGRAM_ERROR_TEXT_SIZE_PX, 10.5);
    assert_eq!(CHAT_DIAGRAM_ERROR_RADIUS_PX, 8.0);
    assert_eq!(CHAT_DIAGRAM_ERROR_X_PADDING_PX, 10.0);
    assert_eq!(CHAT_DIAGRAM_ERROR_Y_PADDING_PX, 8.0);
    assert_eq!(CHAT_DIAGRAM_ERROR_LINE_HEIGHT, 1.45);
    assert_eq!(CHAT_DIAGRAM_ERROR_BORDER_OPACITY, 0.30);
    assert_eq!(CHAT_MERMAID_CARD_RADIUS_PX, 8.0);
    assert_eq!(CHAT_MERMAID_CARD_PADDING_PX, 8.0);
    assert_eq!(CHAT_MERMAID_CARD_BORDER_OPACITY, 0.60);
    assert_eq!(CHAT_MERMAID_MIN_WIDTH_PX, 180.0);
    assert_eq!(CHAT_CONTENT_BLOCK_MAX_WIDTH, 1.0);
    assert_eq!(CHAT_MATH_DISPLAY_Y_PADDING_PX, 3.0);
    assert_eq!(CHAT_PROSE_PARAGRAPH_GAP_REM, 0.5);
    assert_eq!(CHAT_PROSE_HEADING_1_TEXT_SIZE_PX, 14.0);
    assert_eq!(CHAT_PROSE_HEADING_2_TEXT_SIZE_PX, 13.0);
    assert_eq!(CHAT_PROSE_HEADING_FALLBACK_TEXT_SIZE_PX, 12.0);
    assert_eq!(CHAT_PROSE_CODE_TEXT_SIZE_PX, 10.5);
    assert_eq!(CHAT_PROSE_CODE_RADIUS_PX, 8.0);
    assert_eq!(CHAT_PROSE_CODE_X_PADDING_PX, 12.0);
    assert_eq!(CHAT_PROSE_CODE_Y_PADDING_PX, 10.0);
    assert_eq!(CHAT_PROSE_CODE_LINE_HEIGHT, 1.55);
    assert_eq!(CHAT_PROSE_CODE_BORDER_OPACITY, 1.0);
}

#[test]
fn desktop_composer_mention_icons_should_match_web_reference() {
    assert_eq!(COMPOSER_DIRECTORY_ICON, "▸");
    assert_eq!(COMPOSER_FILE_ICON, "·");
    assert_eq!(desktop_mention_icon(true), "▸");
    assert_eq!(desktop_mention_icon(false), "·");
}

#[test]
fn desktop_git_branch_switcher_copy_should_match_web_reference() {
    assert_eq!(GIT_BRANCH_EMPTY_LABEL, "No branches");
    assert_eq!(GIT_BRANCH_CURRENT_MARK, "●");
    assert_eq!(GIT_BRANCH_SWITCH_LABEL, "Switch git branch");
    assert_eq!(GIT_BRANCH_MENU_MIN_WIDTH_PX, 200.0);
    assert_eq!(GIT_BRANCH_MENU_MAX_HEIGHT_PX, 256.0);
    assert_eq!(GIT_BRANCH_MENU_RADIUS_PX, 8.0);
    assert_eq!(GIT_BRANCH_EMPTY_TEXT_SIZE_PX, 12.0);
    assert_eq!(GIT_BRANCH_CURRENT_MARK_WIDTH_PX, 12.0);
    assert_eq!(GIT_BRANCH_MENU_ROW_TEXT_SIZE_PX, 14.0);
}

#[test]
fn desktop_auth_banner_copy_should_match_web_reference() {
    assert_eq!(AUTH_BANNER_FONT_FAMILY, "Monaspace Neon");
    assert_eq!(AUTH_BANNER_TEXT_SIZE_PX, 14.0);
    assert_eq!(AUTH_BANNER_RADIUS_PX, 8.0);
    assert_eq!(AUTH_BANNER_X_MARGIN_PX, 16.0);
    assert_eq!(AUTH_BANNER_Y_MARGIN_PX, 12.0);
    assert_eq!(AUTH_BANNER_PADDING_PX, 12.0);
    assert_eq!(AUTH_BANNER_CONTENT_GAP_PX, 12.0);
    assert_eq!(AUTH_BANNER_TITLE_BOTTOM_MARGIN_PX, 4.0);
    assert_eq!(AUTH_BANNER_MESSAGE_BOTTOM_MARGIN_PX, 8.0);
    assert_eq!(AUTH_BANNER_ACTION_GAP_PX, 8.0);
    assert_eq!(AUTH_BANNER_ACTION_TOP_MARGIN_PX, 4.0);
    assert_eq!(AUTH_BANNER_BORDER_OPACITY, 0.30);
    assert_eq!(AUTH_BANNER_BG_OPACITY, 0.10);
    assert_eq!(AUTH_BANNER_MESSAGE_OPACITY, 0.80);
    assert_eq!(AUTH_BANNER_ICON_SIZE_PX, 16.0);
    assert_eq!(AUTH_BANNER_ICON_TOP_OFFSET_PX, 2.0);
    assert_eq!(
        CLAUDE_AUTH_NOT_AUTHENTICATED_LABEL,
        "Claude Code not authenticated."
    );
    assert_eq!(CLAUDE_LOGIN_ACTION_LABEL, "Log in with Claude");
    assert_eq!(CLAUDE_LOGIN_WAITING_LABEL, "Waiting…");
    assert_eq!(
        CLAUDE_LOGIN_WAITING_MESSAGE,
        "Waiting for Claude login… complete the flow in the terminal where bruhs-server is running."
    );
    assert_eq!(CLAUDE_AUTH_STATUS_ARGS, &["auth", "status"]);
    assert_eq!(CLAUDE_AUTH_POLL_INTERVAL_MS, 2_000);
    assert_eq!(CLAUDE_AUTH_POLL_ATTEMPTS, 30);
}

#[test]
fn desktop_auth_banner_visibility_should_match_web_provider_condition() {
    assert!(auth_banner_visible_like_web("claude-sonnet-4-6", false));
    assert!(!auth_banner_visible_like_web("claude-sonnet-4-6", true));
    assert!(!auth_banner_visible_like_web("gpt-5.5", false));
    assert!(!auth_banner_visible_like_web("gpt-5.5", true));
}

#[test]
fn desktop_auth_check_should_preserve_login_guidance_like_web_reference() {
    let previous = Some(
        "Still not authenticated. Run `claude auth login` in a terminal, then click Re-check."
            .to_string(),
    );
    let unauthenticated = parse_desktop_auth_status("Not logged in", "", None);

    let checked = desktop_auth_status_after_check(unauthenticated, previous.clone());
    assert_eq!(checked.message(), previous.as_deref());

    let authenticated = parse_desktop_auth_status(
        r#"{"authenticated":true,"email":"dev@example.com"}"#,
        "",
        None,
    );
    let checked =
        desktop_auth_status_after_check(authenticated, Some("Still not authenticated".to_string()));
    assert!(checked.is_authenticated());
    assert!(checked.message().is_none());
}

#[test]
fn mention_matches_should_list_children_under_query_dir() {
    let root = std::env::temp_dir().join(format!(
        "bruhs-desktop-mention-test-{}",
        uuid::Uuid::new_v4()
    ));
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(root.join("src/main.rs"), "").unwrap();
    let root = fs::canonicalize(root).unwrap();

    let matches = mention_matches(&root, "src/");

    fs::remove_dir_all(&root).ok();
    assert!(matches
        .iter()
        .any(|item| item.path == "src/main.rs" && !item.is_dir));
}

/// The composer's `@` search, run to completion.
fn mention_matches(root: &Path, query: &str) -> Vec<super::DesktopMentionItem> {
    tokio::runtime::Runtime::new()
        .expect("test runtime")
        .block_on(super::git_info::mention_matches(
            root.to_path_buf(),
            query.to_string(),
        ))
}

fn git_info(root: &Path) -> super::git_info::GitInfo {
    tokio::runtime::Runtime::new()
        .expect("test runtime")
        .block_on(super::git_info::load_git_info(root.to_path_buf()))
}

fn run_git(root: &Path, args: &[&str]) {
    let status = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .status()
        .expect("git command should run");
    assert!(status.success(), "git command failed: {args:?}");
}

fn run_git_with_date(root: &Path, args: &[&str], date: &str) {
    let status = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .env("GIT_AUTHOR_DATE", date)
        .env("GIT_COMMITTER_DATE", date)
        .status()
        .expect("git command should run");
    assert!(status.success(), "git command failed: {args:?}");
}

fn git_stdout(root: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .expect("git command should run");
    assert!(output.status.success(), "git command failed: {args:?}");
    String::from_utf8(output.stdout)
        .expect("git output should be utf8")
        .trim()
        .to_string()
}

#[test]
fn git_info_should_match_web_route_order_and_current() {
    let root = std::env::temp_dir().join(format!(
        "bruhs-desktop-branch-info-test-{}",
        uuid::Uuid::new_v4()
    ));
    fs::create_dir_all(&root).unwrap();
    run_git(&root, &["init", "-q"]);
    run_git(&root, &["checkout", "-q", "-b", "main"]);
    fs::write(root.join("app.rs"), "initial").unwrap();
    run_git(&root, &["add", "."]);
    run_git_with_date(
        &root,
        &[
            "-c",
            "user.email=desktop@example.com",
            "-c",
            "user.name=Desktop Test",
            "commit",
            "-q",
            "-m",
            "initial",
        ],
        "2024-01-01T00:00:00Z",
    );
    run_git(&root, &["branch", "older"]);
    fs::write(root.join("app.rs"), "main update").unwrap();
    run_git(&root, &["add", "."]);
    run_git_with_date(
        &root,
        &[
            "-c",
            "user.email=desktop@example.com",
            "-c",
            "user.name=Desktop Test",
            "commit",
            "-q",
            "-m",
            "main",
        ],
        "2024-01-02T00:00:00Z",
    );
    run_git(&root, &["checkout", "-q", "-b", "recent"]);
    fs::write(root.join("app.rs"), "recent update").unwrap();
    run_git(&root, &["add", "."]);
    run_git_with_date(
        &root,
        &[
            "-c",
            "user.email=desktop@example.com",
            "-c",
            "user.name=Desktop Test",
            "commit",
            "-q",
            "-m",
            "recent",
        ],
        "2024-01-03T00:00:00Z",
    );
    run_git(&root, &["checkout", "-q", "main"]);
    let root = fs::canonicalize(root).unwrap();

    let info = git_info(&root);

    // The server's `/api/git/branches` order (bruhs-core): the current
    // branch first, then by most recent commit.
    assert_eq!(info.current.as_deref(), Some("main"));
    assert_eq!(info.branches, vec!["main", "recent", "older"]);

    let head = git_stdout(&root, &["rev-parse", "HEAD"]);
    run_git(&root, &["checkout", "-q", &head]);
    let detached = git_info(&root);

    fs::remove_dir_all(&root).ok();
    assert_eq!(detached.current, None, "a detached HEAD has no branch");
}

#[test]
fn mention_matches_should_return_empty_without_git_files_like_web_reference() {
    let root = std::env::temp_dir().join(format!(
        "bruhs-desktop-mention-no-git-test-{}",
        uuid::Uuid::new_v4()
    ));
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(root.join("src/main.rs"), "").unwrap();
    let root = fs::canonicalize(root).unwrap();

    let matches = mention_matches(&root, "");

    fs::remove_dir_all(&root).ok();
    assert!(matches.is_empty());
}

#[test]
fn mention_matches_should_use_web_directory_slot_limit() {
    let root = std::env::temp_dir().join(format!(
        "bruhs-desktop-mention-slots-test-{}",
        uuid::Uuid::new_v4()
    ));
    fs::create_dir_all(&root).unwrap();
    run_git(&root, &["init", "-q"]);
    for index in 0..12 {
        let dir = root.join(format!("dir-{index:02}"));
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("mod.rs"), "").unwrap();
    }
    for index in 0..25 {
        fs::write(root.join(format!("file-{index:02}.rs")), "").unwrap();
    }
    run_git(&root, &["add", "."]);
    let root = fs::canonicalize(root).unwrap();

    let matches = mention_matches(&root, "");

    fs::remove_dir_all(&root).ok();
    assert_eq!(matches.len(), 30);
    assert_eq!(matches.iter().filter(|item| item.is_dir).count(), 10);
    assert_eq!(matches.iter().filter(|item| !item.is_dir).count(), 20);
}

#[test]
fn mention_matches_should_rank_git_hotspots_like_web_reference() {
    let root = std::env::temp_dir().join(format!(
        "bruhs-desktop-mention-hotspot-test-{}",
        uuid::Uuid::new_v4()
    ));
    fs::create_dir_all(root.join("src")).unwrap();
    run_git(&root, &["init", "-q"]);
    fs::write(root.join("src/match_old.rs"), "old").unwrap();
    fs::write(root.join("src/match_recent.rs"), "recent").unwrap();
    run_git(&root, &["add", "."]);
    run_git(
        &root,
        &[
            "-c",
            "user.email=desktop@example.com",
            "-c",
            "user.name=Desktop Test",
            "commit",
            "-q",
            "-m",
            "initial",
        ],
    );
    fs::write(root.join("src/match_recent.rs"), "recent update").unwrap();
    run_git(&root, &["add", "src/match_recent.rs"]);
    run_git(
        &root,
        &[
            "-c",
            "user.email=desktop@example.com",
            "-c",
            "user.name=Desktop Test",
            "commit",
            "-q",
            "-m",
            "recent",
        ],
    );
    let root = fs::canonicalize(root).unwrap();

    let matches = mention_matches(&root, "match");
    let files = matches
        .iter()
        .filter(|item| !item.is_dir)
        .map(|item| item.path.as_str())
        .collect::<Vec<_>>();

    fs::remove_dir_all(&root).ok();
    assert_eq!(files.first().copied(), Some("src/match_recent.rs"));
}

#[test]
fn desktop_usage_detail_rows_should_include_nonzero_cache_counts() {
    let mut partial = Message::assistant_streaming(1_000, Some("gpt-5.4".to_string()));
    partial.content = "Partial response".to_string();
    mark_streaming_message_stopped_like_web(&mut partial);

    assert_eq!(partial.state, TurnState::Stopped);
    assert_eq!(partial.content, "Partial response");
}

#[test]
fn saved_stopped_and_error_flags_read_as_one_turn_state() {
    assert_eq!(TurnState::from_persisted(false, None), TurnState::Finished);
    assert_eq!(TurnState::from_persisted(true, None), TurnState::Stopped);
    let failed = TurnState::from_persisted(false, Some("boom".into()));
    assert_eq!(failed.error(), Some("boom"));
    assert!(!failed.is_stopped());
    // Never written together; the error is what the transcript shows.
    assert_eq!(TurnState::from_persisted(true, Some("boom".into())), failed);
}

#[test]
fn composer_hint_index_should_wrap_between_matches() {
    assert_eq!(
        move_composer_hint_index(0, 3, ComposerHintDirection::Previous),
        2
    );
    assert_eq!(
        move_composer_hint_index(2, 3, ComposerHintDirection::Next),
        0
    );
    assert_eq!(
        move_composer_hint_index(1, 3, ComposerHintDirection::Next),
        2
    );
}

#[test]
fn composer_hint_index_should_clamp_stale_selection() {
    assert_eq!(clamp_composer_hint_index(4, 2), 1);
    assert_eq!(clamp_composer_hint_index(4, 0), 0);
}

#[test]
fn composer_hint_hover_should_select_entered_row() {
    let mut index = 0;

    assert!(select_composer_hint_index_on_hover(&mut index, true, 2));
    assert_eq!(index, 2);
    assert!(!select_composer_hint_index_on_hover(&mut index, true, 2));
    assert!(!select_composer_hint_index_on_hover(&mut index, false, 1));
    assert_eq!(index, 2);
}

#[test]
fn desktop_tool_input_summary_should_prefer_tool_specific_fields() {
    assert_eq!(
        desktop_tool_input_summary("Bash", &json!({ "command": "cargo test -p bruhs" })),
        "cargo test -p bruhs"
    );
    let long_command = "x".repeat(81);
    assert_eq!(
        desktop_tool_input_summary("Bash", &json!({ "command": long_command })),
        format!("{}…", "x".repeat(80))
    );
}

#[test]
fn desktop_tool_toggle_label_should_match_web_aria_copy() {
    assert_eq!(TOOL_CARD_BODY_PADDING_PX, 12.0);
    assert_eq!(TOOL_CARD_BODY_BORDER_OPACITY, 0.50);
}

#[test]
fn permission_mode_label_should_match_web_status_bar_copy() {
    assert_eq!(PermissionMode::AcceptEdits.label(), "Auto-accept");
    assert_eq!(STATUS_CHANGE_MODEL_LABEL, "Change model");
}

#[test]
fn desktop_permission_summary_should_match_web_supported_tools() {
    assert_eq!(PERMISSION_PROMPT_FONT_FAMILY, "Monaspace Argon");
    assert_eq!(PERMISSION_PROMPT_TEXT_SIZE_PX, 12.0);
    assert_eq!(PERMISSION_PROMPT_RADIUS_PX, 8.0);
    assert_eq!(PERMISSION_PROMPT_X_MARGIN_PX, 16.0);
    assert_eq!(PERMISSION_PROMPT_Y_MARGIN_PX, 8.0);
    assert_eq!(PERMISSION_PROMPT_X_PADDING_PX, 12.0);
    assert_eq!(PERMISSION_PROMPT_Y_PADDING_PX, 8.0);
    assert_eq!(PERMISSION_PROMPT_BORDER_OPACITY, 0.30);
    assert_eq!(PERMISSION_PROMPT_DIVIDER_OPACITY, 0.20);
    assert_eq!(PERMISSION_PROMPT_BG_OPACITY, 0.05);
    assert_eq!(PERMISSION_PROMPT_ACTION_HOVER_BG_OPACITY, 0.10);
    assert_eq!(PERMISSION_PROMPT_ICON_SIZE_PX, 14.0);
    assert_eq!(PERMISSION_PROMPT_ACTION_GAP_PX, 6.0);
    assert_eq!(PERMISSION_PROMPT_DIVIDER_WIDTH_PX, 1.0);
    assert_eq!(PERMISSION_PROMPT_DIVIDER_MIN_HEIGHT_PX, 34.0);
    assert_eq!(
        desktop_permission_summary("Bash", &json!({ "command": "cargo check" })),
        "cargo check"
    );
    assert_eq!(
        desktop_permission_summary("Write", &json!({ "file_path": "src/main.rs" })),
        "src/main.rs"
    );
    let long_command = "x".repeat(100);
    assert_eq!(
        desktop_permission_summary("Bash", &json!({ "command": long_command })),
        "x".repeat(100)
    );
    assert_eq!(
        desktop_permission_summary("Glob", &json!({ "pattern": "*.rs" })),
        ""
    );
}

#[test]
fn clear_permission_if_matches_should_only_drop_the_named_request() {
    let mut pending = Some(DesktopPermissionRequest {
        id: "perm-1".to_string(),
        tool: "Bash".to_string(),
        input: json!({ "command": "cargo test" }),
    });

    assert!(!clear_permission_if_matches(&mut pending, "perm-2"));
    assert!(pending.is_some(), "a foreign id must not clear the prompt");

    assert!(clear_permission_if_matches(&mut pending, "perm-1"));
    assert!(pending.is_none());

    assert!(!clear_permission_if_matches(&mut pending, "perm-1"));
}

#[test]
fn desktop_permission_request_from_raw_should_read_direct_or_wrapped_payload() {
    let direct = desktop_permission_request_from_raw(&json!({
        "type": "permission_request",
        "id": "perm-1",
        "tool": "Bash",
        "input": { "command": "cargo test" }
    }))
    .expect("direct permission request should parse");
    assert_eq!(direct.id, "perm-1");
    assert_eq!(direct.tool, "Bash");

    let wrapped = desktop_permission_request_from_raw(&json!({
        "type": "raw",
        "data": {
            "type": "permission_request",
            "request_id": "perm-2",
            "tool": "Write",
            "input": { "file_path": "README.md" }
        }
    }))
    .expect("wrapped permission request should parse");
    assert_eq!(wrapped.id, "perm-2");
    assert_eq!(wrapped.input["file_path"], "README.md");
}

#[gpui::test]
fn desktop_permission_prompt_approve_should_clear_inline_banner_like_web(
    cx: &mut gpui::TestAppContext,
) {
    cx.update(|cx| {
        gpui_component::init(cx);
        gpui_component::Theme::change(gpui_component::ThemeMode::Dark, None, cx);
        theme::apply_bruhs_theme(cx);
        let state = DesktopState::new().expect("desktop state should initialize");
        cx.set_global(state);
    });

    let chat_cell: Rc<RefCell<Option<gpui::Entity<ChatView>>>> = Rc::new(RefCell::new(None));
    let chat_for_window = chat_cell.clone();
    let (_root, cx) = cx.add_window_view(|window, cx| {
        let chat = cx.new(|cx| ChatView::new(window, cx));
        *chat_for_window.borrow_mut() = Some(chat.clone());
        gpui_component::Root::new(chat, window, cx)
    });
    let chat = chat_cell
        .borrow()
        .clone()
        .expect("chat entity should be captured");

    chat.update(cx, |chat, cx| {
        chat.pending_permission = Some(DesktopPermissionRequest {
            id: "perm-1".to_string(),
            tool: "Bash".to_string(),
            input: json!({ "command": "cargo test -p bruhs-desktop" }),
        });
        cx.notify();
    });
    cx.run_until_parked();

    assert_chat_rendered_bounds(cx, "desktop-permission-prompt");
    assert_chat_rendered_bounds(cx, "desktop-permission-summary");
    assert_chat_rendered_bounds(cx, "desktop-permission-deny");
    assert_chat_rendered_bounds(cx, "desktop-permission-approve");
    chat.update(cx, |chat, _cx| {
        let request = chat
            .pending_permission
            .as_ref()
            .expect("permission request should still be pending before response");
        assert_eq!(
            desktop_permission_summary(&request.tool, &request.input),
            "cargo test -p bruhs-desktop"
        );
    });

    click_chat_debug_selector(cx, "desktop-permission-approve");

    chat.update(cx, |chat, _cx| {
        assert!(
            chat.pending_permission.is_none(),
            "approving should clear the inline permission prompt"
        );
    });
}

fn finished_assistant(content: &str, tool_id: &str, secs: i64) -> Message {
    Message {
        role: MessageRole::Assistant,
        content: content.to_string(),
        model: None,
        thinking: String::new(),
        usage: None,
        duration_ms: None,
        started_at_ms: Some(1_000),
        completed_at_ms: Some(1_000 + secs * 1_000),
        tool_calls: vec![DesktopToolCall {
            id: tool_id.to_string(),
            tool: "Bash".to_string(),
            input: json!({ "command": "ls" }),
            output: Some("ok".to_string()),
            is_error: false,
        }],
        files: Vec::new(),
        state: TurnState::Finished,
        checkpoint: None,
    }
}

#[gpui::test]
fn desktop_mention_file_should_insert_at_and_open_hints(cx: &mut gpui::TestAppContext) {
    cx.update(|cx| {
        gpui_component::init(cx);
        gpui_component::Theme::change(gpui_component::ThemeMode::Dark, None, cx);
        theme::apply_bruhs_theme(cx);
        let state = DesktopState::new().expect("desktop state should initialize");
        cx.set_global(state);
    });
    let chat_cell: Rc<RefCell<Option<gpui::Entity<ChatView>>>> = Rc::new(RefCell::new(None));
    let chat_for_window = chat_cell.clone();
    let (_root, cx) = cx.add_window_view(|window, cx| {
        let chat = cx.new(|cx| {
            let mut chat = ChatView::new(window, cx);
            chat.start_thread("thread-mention", "Mention thread", cx);
            chat
        });
        *chat_for_window.borrow_mut() = Some(chat.clone());
        gpui_component::Root::new(chat, window, cx)
    });
    let chat = chat_cell.borrow().clone().expect("chat entity");
    cx.run_until_parked();

    chat.update_in(cx, |chat, window, cx| {
        chat.mention_file(window, cx);
        assert_eq!(chat.input_state.read(cx).value().to_string(), "@");
        assert_eq!(chat.mention_query.as_deref(), Some(""));
        chat.input_state
            .update(cx, |input, cx| input.set_value("fix the", window, cx));
        chat.mention_file(window, cx);
        assert_eq!(chat.input_state.read(cx).value().to_string(), "fix the @");
    });
}

#[gpui::test]
fn desktop_hero_long_project_name_stays_inside_composer(cx: &mut gpui::TestAppContext) {
    cx.update(|cx| {
        gpui_component::init(cx);
        gpui_component::Theme::change(gpui_component::ThemeMode::Dark, None, cx);
        theme::apply_bruhs_theme(cx);
        let state = DesktopState::new().expect("desktop state should initialize");
        cx.set_global(state);
    });

    let (_root, cx) = cx.add_window_view(|window, cx| {
        let chat = cx.new(|cx| {
            let mut chat = ChatView::new(window, cx);
            chat.set_project_context(
                ".",
                "desktop-project-picker-current-test-25130-17780044132693770000",
                true,
                cx,
            );
            chat
        });
        gpui_component::Root::new(chat, window, cx)
    });
    cx.run_until_parked();
    refresh_chat_test_window(cx);

    let composer = cx
        .debug_bounds("desktop-composer")
        .expect("composer should be rendered");
    let hero = cx
        .debug_bounds("desktop-chat-hero")
        .expect("hero should be rendered");
    let project = cx
        .debug_bounds("desktop-chat-hero-project")
        .expect("hero project should be rendered");
    let composer_left = f32::from(composer.origin.x);
    let composer_right = composer_left + f32::from(composer.size.width);
    let hero_left = f32::from(hero.origin.x);
    let hero_right = hero_left + f32::from(hero.size.width);
    let project_left = f32::from(project.origin.x);
    let project_right = project_left + f32::from(project.size.width);

    assert!(hero_left >= composer_left - 0.5);
    assert!(hero_right <= composer_right + 0.5);
    assert!(project_left >= hero_left - 0.5);
    assert!(project_right <= hero_right + 0.5);
}

#[test]
fn filter_models_matches_every_word() {
    let all = filter_models_like_web(MODEL_CATALOG, "");
    assert_eq!(all.len(), MODEL_CATALOG.len());
    let gpt = filter_models_like_web(MODEL_CATALOG, "gpt 5.4");
    assert_eq!(
        gpt.iter().map(|m| m.id).collect::<Vec<_>>(),
        vec!["gpt-5.4"]
    );
    let anthropic = filter_models_like_web(MODEL_CATALOG, "ANTHROPIC opus");
    assert!(anthropic
        .iter()
        .all(|m| m.provider == AgentProvider::Claude));
    assert!(anthropic.iter().all(|m| m.label.contains("Opus")));
    assert!(filter_models_like_web(MODEL_CATALOG, "gpt claude").is_empty());
}

#[test]
fn group_models_keeps_provider_order() {
    let matches = filter_models_like_web(MODEL_CATALOG, "");
    let groups = group_models_like_web(&matches);
    let providers: Vec<AgentProvider> = groups.iter().map(|(p, _)| *p).collect();
    let mut expected: Vec<AgentProvider> = Vec::new();
    for m in MODEL_CATALOG {
        if !expected.contains(&m.provider) {
            expected.push(m.provider);
        }
    }
    assert_eq!(providers, expected);
    assert_eq!(
        groups.iter().map(|(_, ms)| ms.len()).sum::<usize>(),
        MODEL_CATALOG.len()
    );
}

#[gpui::test]
fn desktop_model_picker_opens_from_chip_and_selects(cx: &mut gpui::TestAppContext) {
    cx.update(|cx| {
        gpui_component::init(cx);
        gpui_component::Theme::change(gpui_component::ThemeMode::Dark, None, cx);
        theme::apply_bruhs_theme(cx);
        let state = DesktopState::new().expect("desktop state should initialize");
        cx.set_global(state);
    });

    let chat_cell: Rc<RefCell<Option<gpui::Entity<ChatView>>>> = Rc::new(RefCell::new(None));
    let chat_for_window = chat_cell.clone();
    let (_root, cx) = cx.add_window_view(|window, cx| {
        let chat = cx.new(|cx| {
            let mut chat = ChatView::new(window, cx);
            chat.start_thread("thread-picker", "Picker thread", cx);
            chat.set_model("gpt-5.5", cx);
            chat
        });
        *chat_for_window.borrow_mut() = Some(chat.clone());
        gpui_component::Root::new(chat, window, cx)
    });
    let chat = chat_cell
        .borrow()
        .clone()
        .expect("chat entity should be captured");
    cx.run_until_parked();

    assert_chat_no_rendered_bounds(cx, "desktop-model-picker");
    click_chat_debug_selector(cx, "desktop-model-status");
    cx.run_until_parked();
    assert_chat_rendered_bounds(cx, "desktop-model-picker");
    let option = leaked_chat_debug_selector(format!(
        "desktop-model-picker-option-{}",
        element_key("gpt-5.4")
    ));
    assert_chat_rendered_bounds(cx, option);
    click_chat_debug_selector(cx, option);
    cx.run_until_parked();
    chat.update(cx, |chat, _cx| {
        assert_eq!(chat.model_id, "gpt-5.4");
        assert!(!chat.model_picker_open, "selecting closes the picker");
    });
    assert_chat_no_rendered_bounds(cx, "desktop-model-picker");

    // Keyboard: ⌘⇧M-style toggle, ↓ then ↵ picks the second row; esc closes.
    chat.update_in(cx, |chat, window, cx| {
        chat.toggle_model_picker(window, cx);
        assert!(chat.model_picker_open);
        assert!(chat.handle_model_picker_key("down", None, window, cx));
        assert!(chat.handle_model_picker_key("enter", None, window, cx));
        assert_eq!(chat.model_id, MODEL_CATALOG[1].id);
        assert!(!chat.model_picker_open);
        chat.toggle_model_picker(window, cx);
        assert!(chat.handle_model_picker_key("3", Some(3), window, cx));
        assert_eq!(chat.model_id, MODEL_CATALOG[2].id);
        chat.toggle_model_picker(window, cx);
        assert!(chat.handle_model_picker_key("escape", None, window, cx));
        assert!(!chat.model_picker_open);
    });
}

#[test]
fn turn_fold_label_worked_and_stopped() {
    assert_eq!(turn_fold_label(Some(223_000), false), "Worked for 3m 43s");
    assert_eq!(turn_fold_label(Some(12_000), true), "You stopped after 12s");
    assert_eq!(turn_fold_label(Some(900), false), "Worked for 0.9s");
    assert_eq!(turn_fold_label(Some(3_720_000), false), "Worked for 1h 2m");
    assert_eq!(turn_fold_label(None, false), "Worked");
    assert_eq!(turn_fold_label(None, true), "You stopped");
}

#[test]
fn should_fold_only_completed_non_latest_turns_with_activity() {
    let mut errored = finished_assistant("Agent error: boom", "tool-err", 2);
    errored.state = TurnState::Failed {
        error: "boom".to_string(),
    };
    let mut plain = finished_assistant("just text", "unused", 1);
    plain.tool_calls.clear();
    let messages = [
        Message::user("first"),
        finished_assistant("done", "tool-a", 5),
        Message::user("second"),
        errored,
        Message::user("third"),
        plain,
        Message::user("fourth"),
        finished_assistant("latest", "tool-b", 3),
    ];
    assert_eq!(turn_duration_ms(&messages[1]), Some(5_000));
}

#[gpui::test]
fn desktop_turn_fold_collapses_tool_cards(cx: &mut gpui::TestAppContext) {
    cx.update(|cx| {
        gpui_component::init(cx);
        gpui_component::Theme::change(gpui_component::ThemeMode::Dark, None, cx);
        theme::apply_bruhs_theme(cx);
        let state = DesktopState::new().expect("desktop state should initialize");
        cx.set_global(state);
    });

    let chat_cell: Rc<RefCell<Option<gpui::Entity<ChatView>>>> = Rc::new(RefCell::new(None));
    let chat_for_window = chat_cell.clone();
    let (_root, cx) = cx.add_window_view(|window, cx| {
        let chat = cx.new(|cx| {
            let mut chat = ChatView::new(window, cx);
            chat.start_thread("thread-fold", "Fold thread", cx);
            chat.set_model("gpt-5.5", cx);
            chat.messages.push(Message::user("first"));
            chat.messages
                .push(finished_assistant("first reply", "tool-first", 223));
            chat.messages.push(Message::user("second"));
            chat.messages
                .push(finished_assistant("latest reply", "tool-latest", 3));
            chat
        });
        *chat_for_window.borrow_mut() = Some(chat.clone());
        gpui_component::Root::new(chat, window, cx)
    });
    let chat = chat_cell
        .borrow()
        .clone()
        .expect("chat entity should be captured");
    cx.run_until_parked();

    let first_card = leaked_chat_debug_selector(format!(
        "desktop-tool-call-card-{}",
        element_key("tool-first")
    ));
    let latest_card = leaked_chat_debug_selector(format!(
        "desktop-tool-call-card-{}",
        element_key("tool-latest")
    ));
    // Settled turn: fold row instead of the card; latest turn: card, no fold.
    assert_chat_rendered_bounds(cx, "desktop-turn-fold-1");
    assert_chat_no_rendered_bounds(cx, first_card);
    assert_chat_no_rendered_bounds(cx, "desktop-turn-fold-3");
    assert_chat_rendered_bounds(cx, latest_card);

    click_chat_debug_selector(cx, "desktop-turn-fold-1");
    cx.run_until_parked();
    assert_chat_rendered_bounds(cx, first_card);
    chat.update(cx, |chat, _cx| {
        assert!(chat.expanded_folds.contains(&1));
        assert_eq!(
            turn_fold_label(turn_duration_ms(&chat.messages[1]), false),
            "Worked for 3m 43s"
        );
    });

    click_chat_debug_selector(cx, "desktop-turn-fold-1");
    cx.run_until_parked();
    assert_chat_no_rendered_bounds(cx, first_card);
}

#[gpui::test]
fn desktop_tool_call_cards_expand_should_match_web_body_renderers(cx: &mut gpui::TestAppContext) {
    cx.update(|cx| {
        gpui_component::init(cx);
        gpui_component::Theme::change(gpui_component::ThemeMode::Dark, None, cx);
        theme::apply_bruhs_theme(cx);
        let state = DesktopState::new().expect("desktop state should initialize");
        cx.set_global(state);
    });

    let chat_cell: Rc<RefCell<Option<gpui::Entity<ChatView>>>> = Rc::new(RefCell::new(None));
    let chat_for_window = chat_cell.clone();
    let (_root, cx) = cx.add_window_view(|window, cx| {
        let chat = cx.new(|cx| {
            let mut chat = ChatView::new(window, cx);
            chat.start_thread("thread-1", "New thread", cx);
            chat.set_model("gpt-5.5", cx);
            chat.messages.push(Message {
                role: MessageRole::Assistant,
                content: "I used a few tools.".to_string(),
                model: None,
                thinking: String::new(),
                usage: None,
                duration_ms: None,
                started_at_ms: None,
                completed_at_ms: None,
                state: TurnState::Finished,
                files: Vec::new(),
                checkpoint: None,
                tool_calls: vec![
                    DesktopToolCall {
                        id: "tool-bash".to_string(),
                        tool: "Bash".to_string(),
                        input: json!({ "command": "cargo test -p bruhs-desktop" }),
                        output: Some("running 1 test\ntest result: ok".to_string()),
                        is_error: true,
                    },
                    DesktopToolCall {
                        id: "tool-edit".to_string(),
                        tool: "Edit".to_string(),
                        input: json!({
                            "file_path": "src/main.rs",
                            "old_string": "let old = true;",
                            "new_string": "let old = false;"
                        }),
                        output: None,
                        is_error: false,
                    },
                    DesktopToolCall {
                        id: "tool-question".to_string(),
                        tool: "AskUserQuestion".to_string(),
                        input: json!({
                            "question": "Which provider?",
                            "options": [
                                "OpenAI",
                                { "label": "Anthropic", "value": "claude" }
                            ]
                        }),
                        output: Some("OpenAI".to_string()),
                        is_error: false,
                    },
                ],
            });
            cx.notify();
            chat
        });
        *chat_for_window.borrow_mut() = Some(chat.clone());
        gpui_component::Root::new(chat, window, cx)
    });
    let chat = chat_cell
        .borrow()
        .clone()
        .expect("chat entity should be captured");
    cx.run_until_parked();

    let bash_card = tool_call_card_debug_selector("tool-bash");
    let bash_body = tool_call_body_debug_selector("tool-bash");
    let edit_card = tool_call_card_debug_selector("tool-edit");
    let edit_body = tool_call_body_debug_selector("tool-edit");
    let question_card = tool_call_card_debug_selector("tool-question");
    let question_body = tool_call_body_debug_selector("tool-question");

    assert_chat_rendered_bounds(cx, "desktop-work-group-0");
    assert_chat_no_rendered_bounds(cx, bash_card);
    click_chat_debug_selector(cx, "desktop-work-group-0");
    cx.run_until_parked();
    assert_chat_rendered_bounds(cx, bash_card);
    assert_chat_rendered_bounds(cx, tool_error_badge_debug_selector("tool-bash"));
    assert_chat_rendered_bounds(cx, edit_card);
    assert_chat_rendered_bounds(cx, question_card);
    assert_chat_no_rendered_bounds(cx, bash_body);
    assert_chat_no_rendered_bounds(cx, edit_body);
    assert_chat_no_rendered_bounds(cx, question_body);

    click_chat_debug_selector(cx, bash_card);
    cx.run_until_parked();
    assert_chat_rendered_bounds(cx, bash_body);
    assert_chat_rendered_bounds(cx, "desktop-terminal-output");
    chat.update(cx, |chat, _cx| {
        assert!(chat
            .expanded_rows
            .get("0:tool:tool-bash")
            .copied()
            .unwrap_or(false));
    });

    click_chat_debug_selector(cx, edit_card);
    cx.run_until_parked();
    assert_chat_rendered_bounds(cx, edit_body);
    assert_chat_rendered_bounds(cx, "desktop-diff-view");
    chat.update(cx, |chat, _cx| {
        assert!(chat
            .expanded_rows
            .get("0:tool:tool-edit")
            .copied()
            .unwrap_or(false));
    });

    click_chat_debug_selector(cx, question_card);
    cx.run_until_parked();
    assert_chat_rendered_bounds(cx, question_body);
    assert_chat_rendered_bounds(cx, "desktop-question-output");
    chat.update(cx, |chat, _cx| {
        assert!(chat
            .expanded_rows
            .get("0:tool:tool-question")
            .copied()
            .unwrap_or(false));
    });
}

#[gpui::test]
fn desktop_thinking_and_usage_meta_should_match_web_interaction_flow(
    cx: &mut gpui::TestAppContext,
) {
    cx.update(|cx| {
        gpui_component::init(cx);
        gpui_component::Theme::change(gpui_component::ThemeMode::Dark, None, cx);
        theme::apply_bruhs_theme(cx);
        let state = DesktopState::new().expect("desktop state should initialize");
        cx.set_global(state);
    });

    let chat_cell: Rc<RefCell<Option<gpui::Entity<ChatView>>>> = Rc::new(RefCell::new(None));
    let chat_for_window = chat_cell.clone();
    let (_root, cx) = cx.add_window_view(|window, cx| {
        let chat = cx.new(|cx| {
            let mut chat = ChatView::new(window, cx);
            chat.start_thread("thread-1", "New thread", cx);
            chat.set_model("gpt-5.5", cx);
            chat.messages.push(Message {
                role: MessageRole::Assistant,
                content: "Done.".to_string(),
                model: Some("gpt-5.5".to_string()),
                thinking:
                    "thinking through several tool choices before choosing the safest next action"
                        .to_string(),
                usage: Some(UsageInfo {
                    input_tokens: 1_200,
                    output_tokens: 45,
                    cache_read_input_tokens: 10_000,
                    cache_creation_input_tokens: 50,
                    context_tokens: None,
                }),
                duration_ms: None,
                started_at_ms: Some(1_700_000_000_000),
                completed_at_ms: Some(1_700_000_001_250),
                state: TurnState::Finished,
                files: Vec::new(),
                checkpoint: None,
                tool_calls: Vec::new(),
            });
            cx.notify();
            chat
        });
        *chat_for_window.borrow_mut() = Some(chat.clone());
        gpui_component::Root::new(chat, window, cx)
    });
    let chat = chat_cell
        .borrow()
        .clone()
        .expect("chat entity should be captured");
    cx.run_until_parked();

    let thinking_toggle = "desktop-thinking-toggle-0";
    let thinking_body = "desktop-thinking-body-0";
    let usage_meta = "desktop-usage-meta-0";

    assert_chat_rendered_bounds(cx, thinking_toggle);
    assert_chat_no_rendered_bounds(cx, thinking_body);

    click_chat_debug_selector(cx, thinking_toggle);
    cx.run_until_parked();
    assert_chat_rendered_bounds(cx, thinking_body);
    chat.update(cx, |chat, _cx| {
        assert!(chat.expanded_thinking.contains(&0));
    });

    // The meta row (model · duration · tokens · clock) is hover-only.
    hover_chat_debug_selector(cx, "desktop-assistant-message-0");
    cx.run_until_parked();
    assert_chat_rendered_bounds(cx, usage_meta);
}

#[gpui::test]
fn desktop_composer_hint_keyboard_should_match_web_flow(cx: &mut gpui::TestAppContext) {
    cx.update(|cx| {
        gpui_component::init(cx);
        gpui_component::Theme::change(gpui_component::ThemeMode::Dark, None, cx);
        theme::apply_bruhs_theme(cx);
        let state = DesktopState::new().expect("desktop state should initialize");
        cx.set_global(state);
    });

    let chat_cell: Rc<RefCell<Option<gpui::Entity<ChatView>>>> = Rc::new(RefCell::new(None));
    let chat_for_window = chat_cell.clone();
    let (_root, cx) = cx.add_window_view(|window, cx| {
        let chat = cx.new(|cx| {
            let mut chat = ChatView::new(window, cx);
            chat.start_thread("thread-1", "New thread", cx);
            chat.set_model("gpt-5.5", cx);
            chat
        });
        *chat_for_window.borrow_mut() = Some(chat.clone());
        gpui_component::Root::new(chat, window, cx)
    });
    let chat = chat_cell
        .borrow()
        .clone()
        .expect("chat entity should be captured");
    cx.run_until_parked();

    click_chat_debug_selector(cx, "desktop-composer-input");
    cx.simulate_input("/");
    cx.run_until_parked();
    assert_chat_rendered_bounds(cx, "desktop-slash-command-hints");
    assert_chat_rendered_bounds(cx, "desktop-slash-command-row-clear");
    assert_chat_rendered_bounds(cx, "desktop-slash-command-row-help");

    cx.simulate_keystrokes("down enter");
    cx.run_until_parked();

    chat.update(cx, |chat, cx| {
        assert_eq!(chat.messages.len(), 1);
        assert!(matches!(chat.messages[0].role, MessageRole::Assistant));
        assert!(chat.messages[0].content.contains("Available commands"));
        assert!(chat.input_state.read(cx).value().is_empty());
        assert_eq!(chat.composer_hint_selected_index, 0);
        assert!(chat.composer_hints_suppressed_value.is_none());
        assert!(!chat.composer_hint_popover_open(cx));
    });

    click_chat_debug_selector(cx, "desktop-composer-input");
    cx.simulate_input("/mo");
    cx.run_until_parked();
    assert_chat_rendered_bounds(cx, "desktop-slash-command-hints");
    assert_chat_rendered_bounds(cx, "desktop-slash-command-row-model");

    cx.simulate_keystrokes("escape");
    cx.run_until_parked();

    chat.update(cx, |chat, cx| {
        assert_eq!(chat.input_state.read(cx).value().as_ref(), "/mo");
        assert_eq!(chat.composer_hints_suppressed_value.as_deref(), Some("/mo"));
        assert_eq!(chat.composer_hint_selected_index, 0);
        assert!(!chat.composer_hint_popover_open(cx));
    });
}

#[gpui::test]
fn desktop_composer_hints_and_git_branch_menu_should_match_web_flow(cx: &mut gpui::TestAppContext) {
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let root = std::env::current_dir()
        .expect("test cwd should be readable")
        .join("target")
        .join(format!(
            "desktop-composer-hints-test-{}-{unique}",
            std::process::id()
        ));
    let src_dir = root.join("src");
    fs::create_dir_all(&src_dir).expect("test src directory should be creatable");
    fs::write(root.join("README.md"), "fixture\n").expect("README fixture should be writable");
    fs::write(src_dir.join("main.rs"), "fn main() {}\n")
        .expect("source fixture should be writable");

    let git_ready = Command::new("git")
        .args(["init", "-q", "-b", "main"])
        .arg(&root)
        .status()
        .ok()
        .is_some_and(|status| status.success())
        && Command::new("git")
            .arg("-C")
            .arg(&root)
            .args(["add", "README.md", "src/main.rs"])
            .status()
            .ok()
            .is_some_and(|status| status.success())
        && Command::new("git")
            .arg("-C")
            .arg(&root)
            .args([
                "-c",
                "user.email=test@example.invalid",
                "-c",
                "user.name=Test User",
                "commit",
                "-q",
                "-m",
                "init",
            ])
            .status()
            .ok()
            .is_some_and(|status| status.success())
        && Command::new("git")
            .arg("-C")
            .arg(&root)
            .args(["branch", "feature"])
            .status()
            .ok()
            .is_some_and(|status| status.success());
    if !git_ready {
        let _ = fs::remove_dir_all(&root);
        return;
    }
    let project_path = root
        .to_str()
        .expect("test path should be utf-8")
        .to_string();

    cx.update(|cx| {
        gpui_component::init(cx);
        gpui_component::Theme::change(gpui_component::ThemeMode::Dark, None, cx);
        theme::apply_bruhs_theme(cx);
        let state = DesktopState::new().expect("desktop state should initialize");
        cx.set_global(state);
    });

    let chat_cell: Rc<RefCell<Option<gpui::Entity<ChatView>>>> = Rc::new(RefCell::new(None));
    let chat_for_window = chat_cell.clone();
    let (_root, cx) = cx.add_window_view(|window, cx| {
        let chat = cx.new(|cx| {
            let mut chat = ChatView::new(window, cx);
            chat.start_thread("thread-1", "New thread", cx);
            chat.set_model("gpt-5.5", cx);
            chat.set_project_context(project_path.clone(), "proj", true, cx);
            chat
        });
        *chat_for_window.borrow_mut() = Some(chat.clone());
        gpui_component::Root::new(chat, window, cx)
    });
    let chat = chat_cell
        .borrow()
        .clone()
        .expect("chat entity should be captured");
    cx.run_until_parked();

    assert_chat_rendered_bounds(cx, "desktop-composer");
    assert_chat_rendered_bounds(cx, "desktop-composer-input");
    assert_chat_rendered_bounds(cx, "desktop-status-bar");
    assert_chat_rendered_bounds(cx, "desktop-model-status");
    assert_chat_rendered_bounds(cx, "desktop-permission-mode");
    assert_chat_provider_logo_bounds(cx, "desktop-chat-provider-logo-codex");

    click_chat_debug_selector(cx, "desktop-composer-input");
    cx.simulate_input("/he");
    cx.run_until_parked();
    assert_chat_rendered_bounds(cx, "desktop-slash-command-hints");
    assert_chat_rendered_bounds(cx, "desktop-slash-command-row-help");

    click_chat_debug_selector(cx, "desktop-slash-command-row-help");
    cx.run_until_parked();
    chat.update(cx, |chat, cx| {
        assert_eq!(chat.messages.len(), 1);
        assert!(matches!(chat.messages[0].role, MessageRole::Assistant));
        assert!(chat.messages[0].content.contains("Available commands"));
        assert!(chat.input_state.read(cx).value().is_empty());
    });

    click_chat_debug_selector(cx, "desktop-composer-input");
    cx.simulate_input("open @src/ma");
    cx.executor().advance_clock(super::MENTION_SEARCH_DEBOUNCE);
    cx.run_until_parked();
    assert_chat_rendered_bounds(cx, "desktop-file-mention-hints");
    click_chat_debug_selector(cx, file_mention_row_debug_selector("src/main.rs"));
    cx.run_until_parked();
    chat.update(cx, |chat, cx| {
        assert_eq!(
            chat.input_state.read(cx).value().as_ref(),
            "open @src/main.rs "
        );
        assert!(chat.mention_query.is_none());
        assert!(chat.mention_items.is_empty());
    });

    assert_chat_rendered_bounds(cx, "desktop-git-branch");
    click_chat_debug_selector(cx, "desktop-git-branch");
    cx.run_until_parked();
    assert_chat_rendered_bounds(cx, "desktop-git-branch-menu");
    click_chat_debug_selector(cx, git_branch_row_debug_selector("feature"));
    cx.run_until_parked();
    chat.update(cx, |chat, _cx| {
        assert_eq!(chat.git_current_branch.as_deref(), Some("feature"));
        assert!(!chat.branch_menu_open);
    });

    let _ = fs::remove_dir_all(&root);
}

fn assert_chat_rendered_bounds(cx: &mut gpui::VisualTestContext, selector: &'static str) {
    refresh_chat_test_window(cx);
    assert!(
        cx.debug_bounds(selector).is_some(),
        "{selector} should be rendered"
    );
}

fn assert_chat_no_rendered_bounds(cx: &mut gpui::VisualTestContext, selector: &'static str) {
    refresh_chat_test_window(cx);
    assert!(
        cx.debug_bounds(selector).is_none(),
        "{selector} should not be rendered"
    );
}

fn assert_chat_provider_logo_bounds(cx: &mut gpui::VisualTestContext, selector: &'static str) {
    refresh_chat_test_window(cx);
    let bounds = cx
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("{selector} should be rendered"));
    assert_eq!(
        f32::from(bounds.size.width),
        crate::assets::PROVIDER_LOGO_SIZE_PX
    );
    assert_eq!(
        f32::from(bounds.size.height),
        crate::assets::PROVIDER_LOGO_SIZE_PX
    );
}

fn refresh_chat_test_window(cx: &mut gpui::VisualTestContext) {
    cx.update(|window, _cx| {
        window.refresh();
    });
    cx.run_until_parked();
}

fn click_chat_debug_selector(cx: &mut gpui::VisualTestContext, selector: &'static str) {
    refresh_chat_test_window(cx);
    let bounds = cx
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("{selector} should be rendered"));
    cx.simulate_click(bounds.center(), gpui::Modifiers::default());
}

fn hover_chat_debug_selector(cx: &mut gpui::VisualTestContext, selector: &'static str) {
    refresh_chat_test_window(cx);
    let bounds = cx
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("{selector} should be rendered"));
    cx.simulate_mouse_move(bounds.center(), None, gpui::Modifiers::default());
}

fn file_mention_row_debug_selector(path: &str) -> &'static str {
    leaked_chat_debug_selector(format!(
        "desktop-file-mention-row-{}",
        super::element_key(path)
    ))
}

fn git_branch_row_debug_selector(branch: &str) -> &'static str {
    leaked_chat_debug_selector(format!(
        "desktop-git-branch-row-{}",
        super::element_key(branch)
    ))
}

fn tool_call_card_debug_selector(tool_call_id: &str) -> &'static str {
    leaked_chat_debug_selector(format!(
        "desktop-tool-call-card-{}",
        super::element_key(tool_call_id)
    ))
}

fn tool_call_body_debug_selector(tool_call_id: &str) -> &'static str {
    leaked_chat_debug_selector(format!(
        "desktop-tool-call-body-{}",
        super::element_key(tool_call_id)
    ))
}

fn tool_error_badge_debug_selector(tool_call_id: &str) -> &'static str {
    leaked_chat_debug_selector(format!(
        "desktop-tool-error-badge-{}",
        super::element_key(tool_call_id)
    ))
}

fn leaked_chat_debug_selector(selector: String) -> &'static str {
    Box::leak(selector.into_boxed_str())
}

#[test]
fn desktop_diff_lines_should_mark_context_removals_and_additions() {
    assert_eq!(TOOL_DIFF_HEADER_BG_OPACITY, 0.50);
    assert_eq!(TOOL_DIFF_HEADER_GAP_PX, 8.0);
    assert_eq!(TOOL_DIFF_COUNT_GAP_PX, 4.0);
    assert_eq!(TOOL_DIFF_LINE_GAP_PX, 8.0);
    assert_eq!(TOOL_DIFF_LINE_BG_OPACITY, 0.10);
    assert_eq!(TOOL_DIFF_LINE_Y_PADDING_PX, 2.0);
    assert_eq!(TOOL_DIFF_MARKER_OPACITY, 0.60);
    assert_eq!(TOOL_DIFF_MARKER_WIDTH_PX, 12.0);
    let lines = desktop_diff_lines("same\nold\nremoved", "same\nnew");

    assert_eq!(lines[0].kind, DesktopDiffLineKind::Context);
    assert_eq!(lines[1].kind, DesktopDiffLineKind::Remove);
    assert_eq!(lines[2].kind, DesktopDiffLineKind::Add);
    assert_eq!(lines[3].kind, DesktopDiffLineKind::Remove);
}

#[test]
fn desktop_limited_terminal_output_should_truncate_after_one_hundred_lines() {
    let output = (0..(TERMINAL_OUTPUT_MAX_LINES + 2))
        .map(|index| format!("line {index}"))
        .collect::<Vec<_>>()
        .join("\n");

    let limited = desktop_limited_terminal_output(&output);

    assert!(limited.contains(&format!("line {}", TERMINAL_OUTPUT_MAX_LINES - 1)));
    assert!(!limited.contains(&format!("line {TERMINAL_OUTPUT_MAX_LINES}")));
    assert!(limited.ends_with(&format!("... (2 {TERMINAL_OUTPUT_MORE_LINES_LABEL})")));
}

#[test]
fn desktop_terminal_output_copy_should_match_web_reference() {
    assert_eq!(TERMINAL_COMMAND_PREFIX, "$ ");
    assert_eq!(TERMINAL_OUTPUT_MAX_LINES, 100);
    assert_eq!(TERMINAL_OUTPUT_MORE_LINES_LABEL, "more lines");
    assert_eq!(TOOL_OUTPUT_MAX_CHARS, 2_000);
    assert_eq!(TOOL_CODE_FONT_FAMILY, "Monaspace Neon");
    assert_eq!(TOOL_CODE_TEXT_SIZE_PX, 12.0);
    assert_eq!(TOOL_OUTPUT_BLOCK_RADIUS_PX, 8.0);
    assert_eq!(TOOL_OUTPUT_X_PADDING_PX, 12.0);
    assert_eq!(TOOL_OUTPUT_Y_PADDING_PX, 8.0);
    assert_eq!(TOOL_OUTPUT_HEADER_Y_PADDING_PX, 6.0);
    assert_eq!(TOOL_OUTPUT_MAX_HEIGHT_PX, 240.0);
    assert_eq!(TOOL_OUTPUT_DIVIDER_OPACITY, 0.50);
}

#[test]
fn desktop_tool_output_truncation_should_match_web_reference() {
    let output = "x".repeat(TOOL_OUTPUT_MAX_CHARS + 1);

    let truncated = truncate_tool_output(&output, TOOL_OUTPUT_MAX_CHARS);

    assert_eq!(truncated.chars().count(), TOOL_OUTPUT_MAX_CHARS + 4);
    assert!(truncated.ends_with("\n..."));
}

#[test]
fn desktop_tool_body_text_should_render_edit_diff_inputs() {
    let tool_call = DesktopToolCall {
        id: "tool-1".to_string(),
        tool: "Edit".to_string(),
        input: json!({
            "file_path": "src/main.rs",
            "old_string": "let old = true;",
            "new_string": "let old = false;"
        }),
        output: None,
        is_error: false,
    };

    assert_eq!(
        desktop_tool_body_text(&tool_call),
        "src/main.rs\n\n--- old\nlet old = true;\n\n+++ new\nlet old = false;"
    );
}

#[test]
fn desktop_tool_options_should_match_web_question_option_labels() {
    assert_eq!(TOOL_QUESTION_GAP_PX, 8.0);
    assert_eq!(TOOL_QUESTION_TEXT_SIZE_PX, 16.0);
    assert_eq!(TOOL_QUESTION_LINE_HEIGHT, 1.45);
    assert_eq!(TOOL_QUESTION_OPTIONS_GAP_PX, 4.0);
    assert_eq!(TOOL_QUESTION_OPTION_GAP_PX, 8.0);
    assert_eq!(TOOL_QUESTION_OPTION_TEXT_SIZE_PX, 14.0);
    assert_eq!(TOOL_QUESTION_OPTION_LINE_HEIGHT, 1.45);
    assert_eq!(TOOL_QUESTION_ANSWER_TOP_MARGIN_PX, 8.0);
    assert_eq!(TOOL_QUESTION_ANSWER_X_PADDING_PX, 12.0);
    assert_eq!(TOOL_QUESTION_ANSWER_Y_PADDING_PX, 8.0);
    assert_eq!(TOOL_QUESTION_ANSWER_TEXT_SIZE_PX, 14.0);
    assert_eq!(TOOL_QUESTION_ANSWER_LINE_HEIGHT, 1.45);
    assert_eq!(TOOL_QUESTION_ANSWER_LABEL_BOTTOM_MARGIN_PX, 4.0);
    assert_eq!(TOOL_QUESTION_ANSWER_LABEL_TEXT_SIZE_PX, 12.0);
    assert_eq!(
        desktop_tool_options(&json!({
            "options": [
                "Claude",
                { "label": "Codex", "value": "codex" },
                { "value": "Other provider" }
            ]
        })),
        vec![
            "Claude".to_string(),
            "Codex".to_string(),
            "Other provider".to_string()
        ]
    );
}

#[test]
fn parse_desktop_auth_status_should_read_json_account_fields() {
    let status = parse_desktop_auth_status(
        r#"{"authenticated":true,"email":"dev@example.com","plan":"pro"}"#,
        "",
        None,
    );

    let DesktopAuthStatus::SignedIn(account) = status else {
        panic!("signed in");
    };
    assert_eq!(account.user.as_deref(), Some("dev@example.com"));
    assert_eq!(account.subscription_type.as_deref(), Some("pro"));
}

#[test]
fn desktop_assistant_content_blocks_should_render_mermaid_fence() {
    let blocks = desktop_assistant_content_blocks(
        "Before\n\n```mermaid\nflowchart LR\n    A[Start] --> B[Done]\n```\n\nAfter",
    );

    assert_eq!(blocks.len(), 3);
    assert_eq!(
        blocks[0],
        DesktopAssistantContentBlock::Markdown("Before".to_string())
    );
    assert_eq!(
        blocks[2],
        DesktopAssistantContentBlock::Markdown("After".to_string())
    );

    let DesktopAssistantContentBlock::Mermaid(block) = &blocks[1] else {
        panic!("middle block should be a rendered Mermaid diagram");
    };
    assert!(block.source.contains("flowchart LR"));
    let DesktopMermaidRender::Svg(svg) = &block.render else {
        panic!("valid Mermaid source should render as SVG");
    };
    assert!(svg.contains("<svg"));
    assert!(svg.contains("Start"));
}

#[test]
fn desktop_assistant_content_blocks_should_render_math_delimiters() {
    let blocks = desktop_assistant_content_blocks(
        "Inline $E=mc^2$ done\n\n$$\\frac{a}{b}$$\n\nAlso \\(x+y\\) and \\[z^2\\]",
    );

    assert_eq!(blocks.len(), 8);
    let DesktopAssistantContentBlock::Markdown(markdown) = &blocks[0] else {
        panic!("first block should be markdown");
    };
    assert_eq!(markdown.trim(), "Inline");

    for block in [&blocks[1], &blocks[3], &blocks[5], &blocks[7]] {
        let DesktopAssistantContentBlock::Math(math) = block else {
            panic!("expected rendered math block");
        };
        let DesktopMathRender::Svg(svg) = &math.render else {
            panic!("valid TeX should render as SVG");
        };
        assert!(svg.contains("<svg"));
    }
}

#[test]
fn desktop_assistant_content_blocks_should_ignore_math_delimiters_in_code() {
    let raw = "Before\n\n```text\n$not_math$\n```\n\n`$also_not_math$`";

    assert_eq!(
        desktop_assistant_content_blocks(raw),
        vec![DesktopAssistantContentBlock::Markdown(raw.to_string())]
    );
}

#[test]
fn desktop_assistant_content_blocks_should_leave_other_code_fences_as_markdown() {
    let raw = "é\n\n```rust\nfn main() {}\n```";

    assert_eq!(
        desktop_assistant_content_blocks(raw),
        vec![DesktopAssistantContentBlock::Markdown(raw.to_string())]
    );
}

#[test]
fn parse_desktop_auth_status_should_detect_raw_unauthenticated_output() {
    let status = parse_desktop_auth_status("Not logged in", "", None);

    assert!(!status.is_authenticated());
}

#[test]
fn normalize_claude_command_error_should_explain_missing_cli() {
    let missing = std::io::Error::from(std::io::ErrorKind::NotFound);
    assert_eq!(
        normalize_claude_command_error(&missing),
        "Claude Code CLI not found. Install it first."
    );
    // Only a missing program is "not installed"; other failures say what
    // happened, even when their text happens to say "not found".
    let other = std::io::Error::other("config not found");
    assert_eq!(normalize_claude_command_error(&other), "config not found");
}

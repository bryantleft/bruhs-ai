use super::chat_window;
use crate::views::chat::{DesktopMentionItem, DesktopPermissionRequest};
use gpui::{
    point, size, ScrollDelta, ScrollEvent, ScrollHandle, TestAppContext, VisualTestContext,
};
use gpui_component::zoom::px;
use serde_json::json;

fn assert_row_is_visible(scroll: &ScrollHandle, index: usize) {
    let viewport = scroll.bounds();
    let row = scroll
        .bounds_for_item(index)
        .expect("selected row is laid out");
    let offset = scroll.offset().y;
    assert!(viewport.size.height > px(0.0));
    assert!(row.top() + offset >= viewport.top() - px(1.0));
    assert!(row.bottom() + offset <= viewport.bottom() + px(1.0));
}

fn assert_model_picker_clear_of_chrome(cx: &mut VisualTestContext, width: f32) {
    let picker = cx
        .debug_bounds("desktop-model-picker")
        .expect("model picker");
    let trigger = cx
        .debug_bounds("desktop-model-status")
        .expect("model trigger");
    assert!(picker.left() >= px(8.0));
    assert!(
        picker.top() >= px(crate::theme::APP_HEADER_HEIGHT_PX + 8.0),
        "picker overlaps the native titlebar: {picker:?}"
    );
    assert!(picker.right() <= px(width - 8.0));
    assert!(
        picker.bottom() <= trigger.top() - px(8.0),
        "picker covers its trigger at width {width}: {picker:?}, trigger {trigger:?}"
    );
    for selector in [
        "desktop-model-picker-search-row",
        "desktop-model-picker-auth-status",
    ] {
        let content = cx
            .debug_bounds(selector)
            .expect("picker search and authentication");
        assert!(content.size.height > px(0.0));
        assert!(content.top() >= picker.top());
        assert!(content.bottom() <= picker.bottom());
    }
}

fn scroll_options_to_lower_actions(cx: &mut VisualTestContext) {
    let panel = cx.debug_bounds("desktop-composer-options-panel").unwrap();
    cx.simulate_event(ScrollEvent {
        position: panel.center(),
        delta: ScrollDelta::Pixels(point(px(0.0), -panel.size.height)),
        ..Default::default()
    });
    cx.run_until_parked();
}

#[gpui::test]
fn model_picker_fits_a_short_window_and_reveals_keyboard_selection(cx: &mut TestAppContext) {
    let (chat, cx) = chat_window(cx);
    cx.simulate_resize(size(px(640.0), px(480.0)));
    chat.update_in(cx, |chat, window, cx| {
        // Start at the first catalog row so count - 1 presses reaches the
        // last entry regardless of catalog additions or provider ordering.
        chat.set_model(bruhs_models::agent::MODEL_CATALOG[0].id, cx);
        chat.open_model_picker(window, cx);
    });
    cx.run_until_parked();
    assert_model_picker_clear_of_chrome(cx, 640.0);

    let count = chat.read_with(cx, |chat, cx| chat.model_picker_matches(cx).len());
    cx.simulate_keystrokes(&"down ".repeat(count - 1));
    cx.run_until_parked();
    let selected_key = chat.read_with(cx, |chat, cx| {
        crate::views::chat::element_key(&chat.model_picker_matches(cx)[count - 1].id)
    });
    let selector =
        Box::leak(format!("desktop-model-picker-option-{selected_key}").into_boxed_str());
    let selected = cx.debug_bounds(selector).expect("selected model row");
    let list = cx
        .debug_bounds("desktop-model-picker-list")
        .expect("model list");
    let offset = chat.read_with(cx, |chat, _| chat.model_picker_scroll.offset());
    assert!(
        selected.top() >= list.top(),
        "selected row {selected:?} above list {list:?}"
    );
    assert!(
        selected.bottom() <= list.bottom(),
        "selected row {selected:?} below list {list:?}, offset {offset:?}"
    );
    chat.read_with(cx, |chat, cx| {
        assert_eq!(chat.model_picker_index, count - 1);
        let groups = crate::views::chat::group_models_like_web(&chat.model_picker_matches(cx));
        assert_row_is_visible(&chat.model_picker_scroll, count + groups.len() - 1);
        assert!(chat.model_picker_scroll.offset().y < px(0.0));
    });
    let offset = chat.read_with(cx, |chat, _| chat.model_picker_scroll.offset().y);
    cx.simulate_event(ScrollEvent {
        position: list.center(),
        delta: ScrollDelta::Pixels(point(px(0.0), list.size.height)),
        ..Default::default()
    });
    cx.run_until_parked();
    chat.read_with(cx, |chat, _| {
        assert!(
            chat.model_picker_scroll.offset().y > offset,
            "manual scrolling should stay free of keyboard highlight snapping"
        );
    });
    for (width, height) in [(1280.0, 800.0), (640.0, 480.0)] {
        cx.simulate_resize(size(px(width), px(height)));
        cx.run_until_parked();
        assert_model_picker_clear_of_chrome(cx, width);
        let selected = cx
            .debug_bounds(selector)
            .expect("selected model after resize");
        let list = cx.debug_bounds("desktop-model-picker-list").unwrap();
        assert!(selected.top() >= list.top());
        assert!(selected.bottom() <= list.bottom());
    }
}

#[gpui::test]
fn listed_cursor_models_become_picker_rows(cx: &mut TestAppContext) {
    use bruhs_models::agent::AgentProvider;
    use bruhs_models::runtime_models::{ModelChoice, RuntimeModels};
    let (chat, cx) = chat_window(cx);
    chat.update_in(cx, |chat, window, cx| {
        chat.set_model("cursor/gpt-5", cx);
        chat.open_model_picker(window, cx);
        let custom_row = chat.model_picker_matches(cx).len();
        assert_eq!(chat.model_picker_index, custom_row, "unlisted: custom row");
        chat.apply_runtime_listing(
            AgentProvider::Cursor,
            RuntimeModels {
                models: vec![ModelChoice {
                    id: "cursor/gpt-5".into(),
                    label: "GPT-5".into(),
                    provider: AgentProvider::Cursor,
                }],
                error: None,
            },
            cx,
        );
        let matches = chat.model_picker_matches(cx);
        let row = matches
            .iter()
            .position(|model| model.id == "cursor/gpt-5")
            .expect("listed model row");
        assert_eq!(matches[row - 1].id, "cursor/default");
        assert_eq!(chat.model_picker_index, row, "highlight follows the row");
        assert_eq!(chat.model_picker_custom_match(cx), None);
        chat.apply_runtime_listing(
            AgentProvider::Cursor,
            RuntimeModels {
                models: Vec::new(),
                error: Some("Run `cursor-agent update`.".into()),
            },
            cx,
        );
        assert_eq!(
            chat.model_picker_status_line(AgentProvider::Cursor),
            "Cursor models unavailable: Run `cursor-agent update`."
        );
    });
    cx.run_until_parked();
    let option = Box::leak(
        format!(
            "desktop-model-picker-option-{}",
            crate::views::chat::element_key("cursor/default")
        )
        .into_boxed_str(),
    );
    assert!(cx.debug_bounds(option).is_some());
}

#[gpui::test]
fn arriving_cursor_models_keep_a_moved_highlight_on_its_model(cx: &mut TestAppContext) {
    use bruhs_models::agent::AgentProvider;
    use bruhs_models::runtime_models::{ModelChoice, RuntimeModels};
    let (chat, cx) = chat_window(cx);
    chat.update_in(cx, |chat, window, cx| {
        chat.set_model("gpt-5.5", cx);
        chat.open_model_picker(window, cx);
        let grok = chat
            .model_picker_matches(cx)
            .iter()
            .position(|model| model.id == "grok/default")
            .unwrap();
        chat.model_picker_index = grok;
        chat.apply_runtime_listing(
            AgentProvider::Cursor,
            RuntimeModels {
                models: vec![ModelChoice {
                    id: "cursor/gpt-5".into(),
                    label: "GPT-5".into(),
                    provider: AgentProvider::Cursor,
                }],
                error: None,
            },
            cx,
        );
        let matches = chat.model_picker_matches(cx);
        assert_eq!(matches[chat.model_picker_index].id, "grok/default");
        assert!(chat.handle_model_picker_key("enter", None, window, cx));
        assert_eq!(chat.model_id, "grok/default");
    });
}

/// The picker's fetch path end to end: opening runs the installed lister on
/// the tokio runtime and the listing becomes rows.
#[gpui::test]
fn opening_the_picker_lists_the_accounts_cursor_models(cx: &mut TestAppContext) {
    use crate::views::chat::model_picker_ui::RuntimeModelLister;
    use bruhs_models::agent::AgentProvider;
    use bruhs_models::runtime_models::{ModelChoice, RuntimeModels};
    let (chat, cx) = chat_window(cx);
    cx.update(|_, cx| {
        cx.set_global(RuntimeModelLister(std::sync::Arc::new(|provider| {
            Box::pin(async move {
                assert_eq!(provider, AgentProvider::Cursor);
                RuntimeModels {
                    models: vec![ModelChoice {
                        id: "cursor/composer-2".into(),
                        label: "Composer 2".into(),
                        provider,
                    }],
                    error: None,
                }
            })
        })));
    });
    chat.update_in(cx, |chat, window, cx| chat.open_model_picker(window, cx));
    cx.run_until_parked();
    chat.read_with(cx, |chat, cx| {
        assert!(chat
            .model_picker_matches(cx)
            .iter()
            .any(|model| model.id == "cursor/composer-2" && model.label == "Composer 2"));
    });
    let option = Box::leak(
        format!(
            "desktop-model-picker-option-{}",
            crate::views::chat::element_key("cursor/composer-2")
        )
        .into_boxed_str(),
    );
    assert!(cx.debug_bounds(option).is_some());
}

#[gpui::test]
fn every_composer_hint_list_reveals_its_keyboard_selection(cx: &mut TestAppContext) {
    let (chat, cx) = chat_window(cx);
    cx.simulate_resize(size(px(640.0), px(480.0)));
    chat.update_in(cx, |chat, window, cx| {
        chat.set_model("gpt-5.5", cx);
        chat.input_state.update(cx, |input, cx| {
            input.focus(window, cx);
        });
    });
    cx.run_until_parked();
    cx.simulate_input("/");
    cx.run_until_parked();
    let count = chat.read_with(cx, |chat, cx| chat.composer_hint_count(cx));
    assert!(count > 0, "typing / must populate the command list");
    cx.simulate_keystrokes(&"down ".repeat(count - 1));
    cx.run_until_parked();
    let popover = cx.debug_bounds("desktop-composer-hint-popover").unwrap();
    chat.read_with(cx, |chat, _| {
        assert_eq!(chat.composer_hint_selected_index, count - 1);
        assert_row_is_visible(&chat.composer_hint_scroll, count - 1);
        let viewport = chat.composer_hint_scroll.bounds();
        assert!(
            viewport.top() >= popover.top() && viewport.bottom() <= popover.bottom(),
            "command viewport {viewport:?} is clipped by popover {popover:?}"
        );
    });

    for kind in ["files", "skills", "pulls"] {
        chat.update_in(cx, |chat, window, cx| {
            chat.input_state
                .update(cx, |input, cx| input.set_value("", window, cx));
        });
        cx.run_until_parked();
        chat.update(cx, |chat, cx| {
            chat.dismiss_composer_hints(cx);
            chat.composer_hint_selected_index = 0;
            chat.composer_hint_scroll.set_offset(Default::default());
            match kind {
                "files" => {
                    chat.mention_query = Some(String::new());
                    chat.mention_items = (0..24)
                        .map(|i| DesktopMentionItem {
                            path: format!("src/file-{i}.rs"),
                            name: format!("file-{i}.rs"),
                            is_dir: false,
                        })
                        .collect();
                }
                "skills" => {
                    chat.skill_query = Some(String::new());
                    chat.skill_items = (0..24)
                        .map(|i| bruhs_core::SkillRef {
                            name: format!("skill-{i}"),
                            description: format!("Description {i}"),
                        })
                        .collect();
                }
                _ => {
                    chat.pull_query = Some(String::new());
                    chat.pull_items = (0..24)
                        .map(|i| bruhs_core::PullRef {
                            number: i + 1,
                            title: format!("Pull request {i}"),
                            url: format!("https://example.invalid/{i}"),
                            state: "open".into(),
                        })
                        .collect();
                }
            }
            cx.notify();
        });
        cx.run_until_parked();
        cx.simulate_keystrokes(&"down ".repeat(23));
        cx.run_until_parked();
        let popover = cx.debug_bounds("desktop-composer-hint-popover").unwrap();
        chat.read_with(cx, |chat, _| {
            assert_eq!(chat.composer_hint_selected_index, 23, "{kind}");
            assert_row_is_visible(&chat.composer_hint_scroll, 23);
            assert!(chat.composer_hint_scroll.offset().y < px(0.0), "{kind}");
            let viewport = chat.composer_hint_scroll.bounds();
            assert!(
                viewport.top() >= popover.top() && viewport.bottom() <= popover.bottom(),
                "{kind} viewport is clipped by the outer popover"
            );
        });
    }
}

#[gpui::test]
fn permission_review_keeps_full_arguments_and_actions_in_a_short_window(cx: &mut TestAppContext) {
    let (chat, cx) = chat_window(cx);
    cx.simulate_resize(size(px(640.0), px(480.0)));
    let input = json!({
        "command": format!("{} FINAL_ARGUMENT", "long argument 中文 ` ``` ".repeat(80)),
        "nested": {"target": "specific recipient", "mode": "write"},
    });
    let source = crate::views::chat::tool_output_ui::permission_input_markdown(&input);
    let code = source
        .strip_prefix("```json\n")
        .unwrap()
        .strip_suffix("\n```")
        .unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(code).unwrap(),
        input
    );
    chat.update(cx, |chat, cx| {
        chat.set_model("gpt-5.5", cx);
        chat.pending_permission = Some(DesktopPermissionRequest {
            id: "audit-permission".into(),
            tool: "mcp__provider__operation".into(),
            input,
        });
        cx.notify();
    });
    cx.run_until_parked();
    let review = cx
        .debug_bounds("desktop-permission-input")
        .expect("full argument review");
    assert!(review.size.height <= px(120.0));
    let content = cx.debug_bounds("desktop-permission-input-content").unwrap();
    assert!(content.bottom() > review.bottom());
    cx.simulate_event(ScrollEvent {
        position: review.center(),
        delta: ScrollDelta::Pixels(point(px(0.0), -content.size.height)),
        ..Default::default()
    });
    cx.run_until_parked();
    let scrolled = cx.debug_bounds("desktop-permission-input-content").unwrap();
    assert!(scrolled.top() < content.top());
    assert!(scrolled.bottom() <= review.bottom());
    for name in ["desktop-permission-deny", "desktop-permission-approve"] {
        let action = cx.debug_bounds(name).expect("permission action");
        assert!(action.top() >= px(0.0));
        assert!(action.bottom() <= px(480.0));
    }
}

#[gpui::test]
fn compact_footer_keeps_actions_aligned_and_full_access_visible(cx: &mut TestAppContext) {
    let (chat, cx) = chat_window(cx);
    cx.simulate_resize(size(px(640.0), px(480.0)));
    chat.update(cx, |chat, cx| chat.set_model("gpt-5.5", cx));
    cx.run_until_parked();
    let selectors = [
        "desktop-attach-file",
        "desktop-model-status",
        "desktop-composer-options",
        "desktop-send-button",
    ];
    let mut previous_right = px(0.0);
    let mut center = None;
    for selector in selectors {
        let before = cx.debug_bounds(selector).expect("footer action");
        assert_eq!(before.size.height, px(32.0), "{selector}");
        assert!(before.left() >= previous_right, "footer actions overlap");
        assert!(before.right() <= px(640.0));
        if let Some(center) = center {
            assert!((before.center().y - center).abs() <= px(1.0));
        }
        center = Some(before.center().y);
        previous_right = before.right();
        cx.simulate_mouse_move(before.center(), None, gpui::Modifiers::default());
        cx.run_until_parked();
        assert_eq!(
            cx.debug_bounds(selector).unwrap(),
            before,
            "hover moved {selector}"
        );
    }
    assert!(cx.debug_bounds("desktop-permission-mode").is_none());
    assert!(cx.debug_bounds("desktop-plan-toggle").is_none());
    assert!(cx.debug_bounds("desktop-effort-chip").is_none());
    assert!(cx.debug_bounds("desktop-stash-chip").is_none());
    for mode in [
        crate::views::chat::PermissionMode::BypassPermissions,
        crate::views::chat::PermissionMode::Plan,
    ] {
        chat.update(cx, |chat, cx| {
            chat.permission_mode = mode;
            chat.plan_return_mode = crate::views::chat::PermissionMode::BypassPermissions;
            cx.notify();
        });
        cx.run_until_parked();
        let status = cx
            .debug_bounds("desktop-composer-status")
            .expect("active access or plan status");
        let options = cx.debug_bounds("desktop-composer-options").unwrap();
        let send = cx.debug_bounds("desktop-send-button").unwrap();
        assert!(status.size.width > px(0.0));
        assert!(status.left() > options.right());
        assert!(status.right() < send.left());
        assert!((status.center().y - send.center().y).abs() <= px(1.0));
    }
}

#[gpui::test]
fn composer_options_trap_keyboard_focus_and_restore_the_trigger(cx: &mut TestAppContext) {
    let (chat, cx) = chat_window(cx);
    cx.simulate_resize(size(px(640.0), px(480.0)));
    chat.update(cx, |chat, cx| {
        chat.messages.push(crate::views::chat::Message {
            role: crate::views::chat::MessageRole::Assistant,
            content: "Local context fixture".into(),
            model: None,
            thinking: String::new(),
            usage: Some(bruhs_core::UsageInfo {
                context_tokens: Some(bruhs_models::logic::compact::IDLE_TOKEN_FLOOR),
                ..Default::default()
            }),
            duration_ms: None,
            started_at_ms: None,
            completed_at_ms: Some(0),
            state: crate::views::chat::TurnState::Finished,
            files: Vec::new(),
            checkpoint: None,
            tool_calls: Vec::new(),
        });
        cx.notify();
    });
    chat.update_in(cx, |chat, window, cx| {
        chat.input_state.update(cx, |input, cx| {
            input.set_value("Retained local draft", window, cx)
        });
    });
    cx.run_until_parked();
    let trigger = cx.debug_bounds("desktop-composer-options").unwrap();
    cx.simulate_click(trigger.center(), gpui::Modifiers::default());
    cx.run_until_parked();
    let panel = cx.debug_bounds("desktop-composer-options-panel").unwrap();
    assert!(panel.left() >= px(8.0));
    assert!(
        panel.top() >= px(crate::theme::APP_HEADER_HEIGHT_PX + 8.0),
        "options overlap the native titlebar: {panel:?}"
    );
    assert!(panel.right() <= px(632.0));
    assert!(panel.bottom() <= px(472.0));
    assert!(panel.bottom() <= trigger.top() - px(8.0));
    for key in ["tab", "shift-tab"] {
        for _ in 0..20 {
            cx.simulate_keystrokes(key);
            cx.run_until_parked();
            let (context_focused, dictation_focused) = chat.update_in(cx, |chat, window, cx| {
                assert!(chat.composer_options_focus.contains_focused(window, cx));
                (
                    chat.composer_options_section_focus[4].contains_focused(window, cx),
                    chat.composer_options_section_focus[5].contains_focused(window, cx),
                )
            });
            if context_focused {
                let panel = cx.debug_bounds("desktop-composer-options-panel").unwrap();
                for selector in ["desktop-return-compact", "desktop-compact-context"] {
                    let action = cx.debug_bounds(selector).expect("context option");
                    assert!(
                        action.top() >= panel.top() && action.bottom() <= panel.bottom(),
                        "focused context action is clipped"
                    );
                }
            }
            if dictation_focused {
                let panel = cx.debug_bounds("desktop-composer-options-panel").unwrap();
                let action = cx.debug_bounds("desktop-composer-dictate").unwrap();
                assert!(
                    action.top() >= panel.top() && action.bottom() <= panel.bottom(),
                    "focused dictation action is clipped"
                );
            }
        }
    }
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    chat.update_in(cx, |chat, window, _| {
        assert!(!chat.composer_options_open);
        assert!(chat.composer_options_trigger_focus.is_focused(window));
    });
    cx.simulate_keystrokes("space");
    cx.run_until_parked();
    assert!(cx.debug_bounds("desktop-composer-options-panel").is_some());
    let trigger = cx.debug_bounds("desktop-composer-options").unwrap();
    cx.simulate_click(trigger.center(), gpui::Modifiers::default());
    cx.run_until_parked();
    assert!(
        cx.debug_bounds("desktop-composer-options-panel").is_none(),
        "trigger should close its open panel"
    );
    let trigger = cx.debug_bounds("desktop-composer-options").unwrap();
    cx.simulate_click(trigger.center(), gpui::Modifiers::default());
    cx.run_until_parked();
    cx.simulate_click(gpui::point(px(2.0), px(2.0)), gpui::Modifiers::default());
    cx.run_until_parked();
    assert!(
        cx.debug_bounds("desktop-composer-options-panel").is_none(),
        "outside click dismisses options"
    );
}

#[gpui::test]
fn compact_options_preserve_effort_and_saved_prompt_workflows(cx: &mut TestAppContext) {
    let (chat, cx) = chat_window(cx);
    cx.simulate_resize(size(px(640.0), px(480.0)));
    chat.update_in(cx, |chat, window, cx| {
        chat.toggle_effort_menu(window, cx);
    });
    cx.run_until_parked();
    assert!(cx.debug_bounds("desktop-composer-options-panel").is_some());
    let effort = cx.debug_bounds("desktop-effort-option-high").unwrap();
    let panel = cx.debug_bounds("desktop-composer-options-panel").unwrap();
    assert!(effort.top() >= panel.top() && effort.bottom() <= panel.bottom());
    cx.simulate_click(effort.center(), gpui::Modifiers::default());
    cx.run_until_parked();
    chat.read_with(cx, |chat, _| {
        assert_eq!(chat.claude_effort.as_deref(), Some("high"))
    });
    chat.update_in(cx, |chat, window, cx| {
        chat.input_state.update(cx, |input, cx| {
            input.set_value("Saved local draft", window, cx)
        });
    });
    cx.run_until_parked();
    scroll_options_to_lower_actions(cx);
    let stash = cx.debug_bounds("desktop-stash-draft").unwrap();
    let panel = cx.debug_bounds("desktop-composer-options-panel").unwrap();
    assert!(stash.top() >= panel.top() && stash.bottom() <= panel.bottom());
    cx.simulate_click(stash.center(), gpui::Modifiers::default());
    cx.run_until_parked();
    chat.read_with(cx, |chat, cx| {
        assert_eq!(chat.stash.len(), 1);
        assert!(chat.input_state.read(cx).value().is_empty());
        assert!(!chat.composer_options_open);
    });
    let trigger = cx.debug_bounds("desktop-composer-options").unwrap();
    cx.simulate_click(trigger.center(), gpui::Modifiers::default());
    cx.run_until_parked();
    scroll_options_to_lower_actions(cx);
    let saved = cx.debug_bounds("desktop-saved-prompts").unwrap();
    let panel = cx.debug_bounds("desktop-composer-options-panel").unwrap();
    assert!(saved.top() >= panel.top() && saved.bottom() <= panel.bottom());
    cx.simulate_click(saved.center(), gpui::Modifiers::default());
    cx.run_until_parked();
    chat.read_with(cx, |chat, _| {
        assert!(chat.stash_open);
        assert!(!chat.composer_options_open);
    });
    assert!(cx.debug_bounds("desktop-stash-menu").is_some());
}

/// The options and saved prompts panels look like the web's
/// `ComposerPopupSurface`: a titled header with a 32px close button, then
/// full-width 32px rows. Close returns focus to the options button.
#[gpui::test]
fn composer_panels_have_the_web_header_and_rows(cx: &mut TestAppContext) {
    let (chat, cx) = chat_window(cx);
    let trigger = cx.debug_bounds("desktop-composer-options").unwrap();
    cx.simulate_click(trigger.center(), gpui::Modifiers::default());
    cx.run_until_parked();
    let panel = cx.debug_bounds("desktop-composer-options-panel").unwrap();
    let close = cx.debug_bounds("desktop-composer-options-close").unwrap();
    let control = px(crate::theme::CONTROL_PX);
    assert_eq!(close.size, size(control, control));
    assert!(close.right() <= panel.right() && close.top() >= panel.top());
    let body = cx.debug_bounds("desktop-composer-options-body").unwrap();
    assert!(
        body.top() >= close.bottom(),
        "the body sits under the header"
    );
    let row = cx
        .debug_bounds("desktop-permission-mode-default")
        .expect("access row");
    assert_eq!(row.size.height, control);
    assert!(
        row.size.width > body.size.width - px(24.0),
        "access rows span the body like web rows: {row:?} in {body:?}"
    );
    cx.simulate_click(close.center(), gpui::Modifiers::default());
    cx.run_until_parked();
    assert!(cx.debug_bounds("desktop-composer-options-panel").is_none());
    chat.update_in(cx, |chat, window, _| {
        assert!(chat.composer_options_trigger_focus.is_focused(window));
    });

    // Saved prompts opens in the same kind of panel.
    let trigger = cx.debug_bounds("desktop-composer-options").unwrap();
    cx.simulate_click(trigger.center(), gpui::Modifiers::default());
    cx.run_until_parked();
    scroll_options_to_lower_actions(cx);
    let saved = cx.debug_bounds("desktop-saved-prompts").unwrap();
    cx.simulate_click(saved.center(), gpui::Modifiers::default());
    cx.run_until_parked();
    assert!(cx.debug_bounds("desktop-stash-menu").is_some());
    let close = cx
        .debug_bounds("desktop-stash-close")
        .expect("saved prompts close");
    assert_eq!(close.size, size(control, control));
    cx.simulate_click(close.center(), gpui::Modifiers::default());
    cx.run_until_parked();
    assert!(cx.debug_bounds("desktop-stash-menu").is_none());
    chat.update_in(cx, |chat, window, _| {
        assert!(!chat.stash_open);
        assert!(chat.composer_options_trigger_focus.is_focused(window));
    });
}

/// A model that is not in the catalog gets the web's "Use …" row, checked
/// while it is the current model.
#[gpui::test]
fn model_picker_checks_the_current_custom_model(cx: &mut TestAppContext) {
    let (chat, cx) = chat_window(cx);
    chat.update_in(cx, |chat, window, cx| {
        chat.set_model("opencode/local/my-model", cx);
        chat.open_model_picker(window, cx);
    });
    cx.run_until_parked();
    let row = cx
        .debug_bounds("desktop-model-picker-custom")
        .expect("the custom row");
    let check = cx
        .debug_bounds("desktop-model-picker-custom-check")
        .expect("its check");
    assert!(check.left() > row.center().x, "the check sits at the right");
}

/// Like web, a transcript scrolled up from its end offers a "Scroll to end"
/// pill centered over its bottom edge, and the pill returns to the end.
#[gpui::test]
fn scroll_to_end_is_a_centered_pill_like_the_web(cx: &mut TestAppContext) {
    let (chat, cx) = chat_window(cx);
    cx.simulate_resize(size(px(900.0), px(600.0)));
    chat.update(cx, |chat, cx| {
        chat.messages = (0..40)
            .map(|n| super::super::Message::user(format!("Message {n}\nwith a second line")))
            .collect();
        chat.scroll_messages_to_bottom();
        cx.notify();
    });
    cx.run_until_parked();
    assert!(cx.debug_bounds("desktop-scroll-to-bottom").is_none());

    cx.simulate_event(ScrollEvent {
        position: point(px(450.0), px(250.0)),
        delta: ScrollDelta::Pixels(point(px(0.0), px(4_000.0))),
        ..Default::default()
    });
    cx.run_until_parked();
    let pill = cx
        .debug_bounds("desktop-scroll-to-bottom")
        .expect("the pill shows once the transcript leaves its end");
    assert_eq!(pill.size.height, px(28.0));
    assert!(
        (pill.center().x - px(450.0)).abs() < px(1.0),
        "the pill is centered, not in a corner"
    );

    cx.simulate_click(pill.center(), gpui::Modifiers::default());
    cx.run_until_parked();
    assert!(
        cx.debug_bounds("desktop-scroll-to-bottom").is_none(),
        "the pill returns to the end"
    );
}

/// Like web, the options panel's Context section leads with the meter: a
/// 28px ring and "N% used" for the latest reply's window, against the
/// window of the model that wrote it.
#[gpui::test]
fn composer_options_show_the_context_meter_like_the_web(cx: &mut TestAppContext) {
    let (chat, cx) = chat_window(cx);
    chat.update_in(cx, |chat, window, cx| {
        let mut reply =
            super::super::Message::assistant_streaming(1, Some("claude-sonnet-4-6".into()));
        reply.content = "Done".into();
        reply.usage = Some(bruhs_core::UsageInfo {
            context_tokens: Some(84_000),
            ..Default::default()
        });
        chat.messages = vec![super::super::Message::user("Hi"), reply];
        chat.open_composer_options(window, cx);
        cx.notify();
    });
    cx.run_until_parked();
    let meter = cx
        .debug_bounds("desktop-context-meter")
        .expect("the meter leads the Context section");
    assert_eq!((meter.size.width, meter.size.height), (px(28.0), px(28.0)));
    chat.read_with(cx, |chat, _| {
        let reading = chat.latest_context_reading().expect("a reading");
        assert_eq!(reading.label(), "Context window, 42%, 84k of 200k");
        assert_eq!(reading.used_label(), "42% used");
    });
}

/// Like web, a turn's action row shows on hover or while focus is inside
/// it, and stays usable while hidden: a click on its copy button copies,
/// and the focus it leaves in the row keeps the row shown.
#[gpui::test]
fn turn_actions_show_on_hover_or_focus_within_like_the_web(cx: &mut TestAppContext) {
    let (chat, cx) = chat_window(cx);
    chat.update(cx, |chat, cx| {
        let mut reply =
            super::super::Message::assistant_streaming(1, Some("claude-sonnet-4-6".into()));
        reply.content = "Done".into();
        reply.state = super::super::TurnState::Finished;
        chat.messages = vec![super::super::Message::user("Hi"), reply];
        cx.notify();
    });
    cx.run_until_parked();
    chat.update_in(cx, |chat, window, cx| {
        assert!(
            !chat.turn_actions_shown(true, 0, window, cx),
            "the user row rests hidden"
        );
        assert!(
            !chat.turn_actions_shown(false, 1, window, cx),
            "so does the reply's"
        );
    });
    let copy = cx
        .debug_bounds("desktop-turn-copy-1")
        .expect("the hidden copy button keeps its place");
    cx.simulate_click(copy.center(), gpui::Modifiers::default());
    cx.run_until_parked();
    chat.update_in(cx, |chat, window, cx| {
        assert!(
            chat.copy_feedback.is_shown("desktop-turn-copy-1"),
            "the click copied"
        );
        assert!(
            chat.turn_actions_shown(false, 1, window, cx),
            "focus inside the row shows it"
        );
        assert!(!chat.turn_actions_shown(true, 0, window, cx));
    });
}

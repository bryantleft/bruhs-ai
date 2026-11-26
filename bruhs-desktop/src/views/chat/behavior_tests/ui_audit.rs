use super::chat_window;
use crate::views::chat::{DesktopMentionItem, DesktopPermissionRequest};
use bruhs_ui::zoom::px;
use gpui::{
    point, size, ScrollDelta, ScrollEvent, ScrollHandle, TestAppContext, VisualTestContext,
};
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
    for selector in ["desktop-model-picker-search-row", "model-options-footer"] {
        let content = cx.debug_bounds(selector).expect("picker search and footer");
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
    use bruhs_models::{
        agent::AgentProvider,
        delivery::writer_catalog::{WriterCatalog, WriterCliModel},
    };
    let (chat, cx) = chat_window(cx);
    cx.simulate_resize(size(px(640.0), px(480.0)));
    chat.update_in(cx, |chat, window, cx| {
        // Start at the first catalog row so count - 1 presses reaches the
        // last entry regardless of catalog additions or provider ordering.
        let first = bruhs_models::agent::MODEL_CATALOG[0].id;
        let provider = AgentProvider::for_model(first);
        let mut catalog = WriterCatalog::default();
        catalog.models.extend((0..16).map(|index| WriterCliModel {
            id: format!("{}fixture-{index}", provider.model_prefix()),
            label: format!("Fixture {index}"),
            provider,
            options: Vec::new(),
            is_legacy: false,
        }));
        chat.replace_model_catalog(std::sync::Arc::new(catalog), cx);
        chat.set_model(first, cx);
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
    chat.read_with(cx, |chat, _| {
        assert_eq!(chat.model_picker_index, count - 1);
        assert_row_is_visible(&chat.model_picker_scroll, count - 1);
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
fn model_picker_refresh_uses_live_catalog_order_and_preserves_highlight(cx: &mut TestAppContext) {
    use bruhs_models::{
        agent::AgentProvider,
        delivery::writer_catalog::{WriterCatalog, WriterCliModel},
    };
    let (chat, cx) = chat_window(cx);
    cx.simulate_resize(size(px(1200.), px(780.)));
    chat.update_in(cx, |chat, window, cx| {
        chat.set_model("gpt-5.4", cx);
        chat.open_model_picker(window, cx);
        let catalog = WriterCatalog {
            models: vec![WriterCliModel {
                id: "codex/fixture-live".into(),
                label: "Fixture live model".into(),
                provider: AgentProvider::Codex,
                options: Vec::new(),
                is_legacy: false,
            }],
        };
        chat.replace_model_catalog(std::sync::Arc::new(catalog), cx);
        let rows = chat.model_picker_matches(cx);
        assert_eq!(
            rows.iter().map(|row| row.id.as_str()).collect::<Vec<_>>(),
            ["codex/fixture-live", "gpt-5.4"]
        );
        assert_eq!(rows[chat.model_picker_index].id, chat.model_id);
    });
    cx.run_until_parked();
    let picker = cx.debug_bounds("desktop-model-picker").unwrap();
    let chip = cx.debug_bounds("desktop-model-status").unwrap();
    let right_limit = px(1200. - 9.) - picker.size.width;
    let anchor = cx.debug_bounds("desktop-model-popover-anchor").unwrap();
    let leading_edge = anchor.left().min(right_limit).max(px(9.));
    let right_aligned = (chip.right() - picker.size.width)
        .min(right_limit)
        .max(leading_edge);
    assert!((picker.left() - right_aligned).abs() <= px(1.));
    let row = cx.debug_bounds("desktop-model-picker-list").unwrap();
    assert!(row.size.height > px(0.));
}

#[gpui::test]
fn model_picker_discloses_older_models_without_hiding_favorites_or_search_results(
    cx: &mut TestAppContext,
) {
    use crate::views::model_browser::favorite_key;
    use bruhs_models::delivery::writer_catalog::WriterCatalog;

    let (chat, cx) = chat_window(cx);
    cx.simulate_resize(size(px(1200.), px(780.)));
    chat.update_in(cx, |chat, window, cx| {
        let mut catalog = WriterCatalog::default();
        for model in &mut catalog.models {
            model.is_legacy = matches!(model.id.as_str(), "claude-sonnet-4-6" | "claude-haiku-4-5");
        }
        chat.replace_model_catalog(std::sync::Arc::new(catalog), cx);
        chat.set_model("claude-fable-5", cx);
        chat.open_model_picker(window, cx);
    });
    let visible = |chat: &crate::views::chat::ChatView, cx: &gpui::App| {
        chat.model_picker_matches(cx)
            .into_iter()
            .map(|row| row.id)
            .collect::<Vec<_>>()
    };
    cx.run_until_parked();
    assert_eq!(
        chat.read_with(cx, visible),
        ["claude-fable-5", "claude-opus-4-8"]
    );
    let disclosure = cx.debug_bounds("desktop-model-picker-disclosure").unwrap();
    cx.simulate_keystrokes("down down enter");
    cx.run_until_parked();
    assert_eq!(
        chat.read_with(cx, visible),
        [
            "claude-fable-5",
            "claude-opus-4-8",
            "claude-sonnet-4-6",
            "claude-haiku-4-5"
        ]
    );
    chat.read_with(cx, |chat, _| assert_eq!(chat.model_id, "claude-fable-5"));
    let collapse = cx.debug_bounds("desktop-model-picker-disclosure").unwrap();
    assert!(collapse.top() > disclosure.top());
    cx.simulate_click(collapse.center(), gpui::Modifiers::default());
    cx.run_until_parked();
    chat.update_in(cx, |chat, window, cx| {
        chat.handle_model_picker_key("3", Some(3), window, cx);
        assert_eq!(
            chat.model_id, "claude-fable-5",
            "hidden models and the disclosure have no jump shortcut"
        );
        chat.replace_picker_favorites(&[favorite_key("claude-haiku-4-5")].into(), cx);
        assert_eq!(
            visible(chat, cx),
            ["claude-haiku-4-5", "claude-fable-5", "claude-opus-4-8"]
        );
        chat.model_picker_query
            .update(cx, |input, cx| input.set_value("sonnet 4.6", window, cx));
        assert_eq!(visible(chat, cx), ["claude-sonnet-4-6"]);
    });
    cx.run_until_parked();
    assert!(cx.debug_bounds("desktop-model-picker-disclosure").is_none());
    chat.update_in(cx, |chat, window, cx| {
        chat.replace_picker_favorites(&Default::default(), cx);
        chat.set_model("claude-sonnet-4-6", cx);
        chat.close_model_picker(window, cx);
        chat.open_model_picker(window, cx);
        let models = visible(chat, cx);
        assert_eq!(models[chat.model_picker_index], "claude-sonnet-4-6");
        assert!(models.iter().any(|id| id == "claude-haiku-4-5"));
    });
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
            chat.model_picker_notice(AgentProvider::Cursor).as_deref(),
            Some("Cursor models unavailable: Run `cursor-agent update`.")
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
        chat.model_picker_query
            .update(cx, |field, cx| field.set_value("default", window, cx));
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
                    label: "Default GPT-5".into(),
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
        cx.set_global(RuntimeModelLister(std::sync::Arc::new(|provider, _| {
            Box::pin(async move {
                if provider != AgentProvider::Cursor {
                    return RuntimeModels::default();
                }
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
    let cursor_tab = cx.debug_bounds("desktop-model-picker-tab-cursor").unwrap();
    cx.simulate_click(cursor_tab.center(), gpui::Modifiers::default());
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
            can_answer: false,
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
    let panel = cx.debug_bounds("desktop-model-picker").unwrap();
    let column = cx.debug_bounds("model-options-column").unwrap();
    let effort = cx.debug_bounds("desktop-effort-option-high").unwrap();
    assert!(
        effort.top() >= column.top()
            && effort.bottom() <= column.bottom()
            && column.right() <= panel.right()
    );
    cx.simulate_click(effort.center(), gpui::Modifiers::default());
    cx.run_until_parked();
    chat.read_with(cx, |chat, _| {
        assert!(chat.model_picker_open);
        assert_eq!(chat.claude_send_effort().as_deref(), Some("high"));
    });
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    let trigger = cx.debug_bounds("desktop-composer-options").unwrap();
    cx.simulate_click(trigger.center(), gpui::Modifiers::default());
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

#[gpui::test]
fn model_picker_option_column_follows_keyboard_and_window_size(cx: &mut TestAppContext) {
    use bruhs_models::delivery::generation::OptionKind::{Effort, ServiceTier};
    let (chat, cx) = chat_window(cx);
    cx.simulate_resize(size(px(960.), px(720.)));
    chat.update_in(cx, |chat, window, cx| {
        chat.set_model("gpt-6.1-sol", cx);
        chat.open_model_picker(window, cx);
    });
    cx.run_until_parked();
    let list = cx.debug_bounds("desktop-model-picker-list").unwrap();
    let column = cx.debug_bounds("model-options-column").unwrap();
    assert!(column.left() >= list.right() - px(1.));
    let search = cx.debug_bounds("desktop-model-picker-search-row").unwrap();
    let divider = cx
        .debug_bounds("desktop-model-picker-favorites-divider")
        .unwrap();
    let heading = cx
        .debug_bounds("desktop-model-picker-options-heading")
        .unwrap();
    assert_eq!(search.bottom(), divider.bottom());
    assert_eq!(search.bottom(), heading.bottom());
    cx.simulate_keystrokes("right");
    cx.run_until_parked();
    chat.update_in(cx, |chat, window, _| {
        assert!(chat.composer_effort_focus.is_focused(window))
    });
    cx.simulate_keystrokes("down enter");
    cx.run_until_parked();
    chat.read_with(cx, |chat, _| {
        assert_eq!(chat.model_options.configured(Effort), Some("medium"))
    });
    cx.simulate_keystrokes("left");
    cx.run_until_parked();
    chat.update_in(cx, |chat, window, cx| {
        use gpui::Focusable;
        assert!(chat.model_picker_query.focus_handle(cx).is_focused(window));
    });
    let tier = cx
        .debug_bounds("desktop-model-service_tier-priority")
        .unwrap();
    cx.simulate_click(tier.center(), gpui::Modifiers::default());
    cx.run_until_parked();
    chat.read_with(cx, |chat, _| {
        assert_eq!(chat.model_options.configured(ServiceTier), Some("priority"))
    });
    cx.simulate_resize(size(px(580.), px(660.)));
    cx.run_until_parked();
    let list = cx.debug_bounds("desktop-model-picker-list").unwrap();
    let column = cx.debug_bounds("model-options-column").unwrap();
    let footer = cx.debug_bounds("model-options-footer").unwrap();
    assert!(column.top() >= list.bottom() - px(1.) && footer.top() >= column.bottom() - px(1.));
    let permissions = cx.debug_bounds("model-picker-permissions").unwrap();
    cx.simulate_click(permissions.center(), gpui::Modifiers::default());
    cx.run_until_parked();
    let allow_edits = cx
        .debug_bounds("model-picker-permissions-option-acceptEdits")
        .unwrap();
    cx.simulate_click(allow_edits.center(), gpui::Modifiers::default());
    cx.run_until_parked();
    chat.read_with(cx, |chat, _| {
        assert!(chat.model_picker_open);
        assert!(chat.model_picker_dropdown.is_none());
        assert_eq!(chat.plan_return_mode.as_str(), "acceptEdits");
    });
    let medium = cx.debug_bounds("desktop-effort-option-medium").unwrap();
    cx.simulate_click(medium.center(), gpui::Modifiers::default());
    cx.simulate_keystrokes("left");
    cx.run_until_parked();
    chat.update_in(cx, |chat, window, cx| {
        use gpui::Focusable;
        assert!(chat.model_picker_query.focus_handle(cx).is_focused(window));
    });
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    assert!(cx.debug_bounds("desktop-model-picker").is_none());
    let chip = cx.debug_bounds("desktop-model-status").unwrap();
    cx.simulate_click(chip.center(), gpui::Modifiers::default());
    cx.run_until_parked();
    cx.simulate_click(point(px(2.), px(2.)), gpui::Modifiers::default());
    cx.run_until_parked();
    assert!(cx.debug_bounds("desktop-model-picker").is_none());
}

#[gpui::test]
fn model_picker_shows_claude_speed_as_standard_and_fast_segments(cx: &mut TestAppContext) {
    use bruhs_models::delivery::generation::OptionKind::FastMode;

    let (chat, cx) = chat_window(cx);
    chat.update_in(cx, |chat, window, cx| {
        chat.set_model("claude-opus-4-8", cx);
        chat.open_model_picker(window, cx);
    });
    cx.run_until_parked();
    let standard = cx.debug_bounds("desktop-model-fast_mode-off").unwrap();
    let fast = cx.debug_bounds("desktop-model-fast_mode-on").unwrap();
    let footer = cx.debug_bounds("model-options-footer").unwrap();
    assert!(standard.right() <= fast.left());
    assert!(footer.contains(&standard.center()) && footer.contains(&fast.center()));

    for (segment, value) in [(fast, "on"), (standard, "off")] {
        cx.simulate_click(segment.center(), gpui::Modifiers::default());
        cx.run_until_parked();
        chat.read_with(cx, |chat, _| {
            assert_eq!(chat.model_options.configured(FastMode), Some(value));
            assert!(chat.model_picker_open);
        });
    }
}

#[gpui::test]
fn model_picker_provider_keys_restore_search_and_keep_options_open(cx: &mut TestAppContext) {
    use gpui::Focusable;

    let (chat, cx) = chat_window(cx);
    chat.update_in(cx, |chat, window, cx| {
        chat.set_model("claude-sonnet-4-6", cx);
        chat.open_model_picker(window, cx);
    });
    cx.run_until_parked();
    cx.simulate_keystrokes("shift-tab");
    cx.run_until_parked();
    chat.update_in(cx, |chat, window, _| {
        assert!(chat.model_picker_rail.is_focused(window));
    });
    cx.simulate_keystrokes("right");
    cx.run_until_parked();
    chat.update_in(cx, |chat, window, cx| {
        assert!(chat.model_picker_query.focus_handle(cx).is_focused(window));
        chat.model_picker_query.update(cx, |input, cx| {
            input.set_value("gpt-5.4", window, cx);
        });
    });
    cx.run_until_parked();
    assert!(cx.debug_bounds("desktop-model-picker-rail").is_none());
    cx.simulate_keystrokes("cmd-shift-down");
    cx.run_until_parked();
    chat.update_in(cx, |chat, window, cx| {
        assert!(chat.model_picker_browsing(cx));
        assert_ne!(chat.model_browser_tab.as_deref(), Some("claude"));
        assert!(chat.model_picker_query.focus_handle(cx).is_focused(window));
        chat.model_picker_query.update(cx, |input, cx| {
            input.set_value("gpt-5.4", window, cx);
        });
    });
    cx.run_until_parked();
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    chat.update_in(cx, |chat, window, _| {
        assert_eq!(chat.model_id, "gpt-5.4");
        assert!(chat.model_picker_open);
        assert!(chat.composer_effort_focus.is_focused(window));
    });
}

#[gpui::test]
fn model_picker_secondary_options_close_independently(cx: &mut TestAppContext) {
    use bruhs_models::delivery::generation::OptionKind::{ContextWindow, Thinking};
    let (chat, cx) = chat_window(cx);
    cx.simulate_resize(size(px(1280.), px(800.)));
    chat.update_in(cx, |chat, window, cx| {
        // An installed provider can offer thinking on a model with context options.
        let mut catalog = (*chat.model_catalog).clone();
        let thinking = catalog
            .model("claude-haiku-4-5")
            .unwrap()
            .options
            .iter()
            .find(|option| option.kind == Thinking)
            .unwrap()
            .clone();
        catalog
            .models
            .iter_mut()
            .find(|model| model.id == "claude-sonnet-4-6")
            .unwrap()
            .options
            .push(thinking);
        chat.replace_model_catalog(std::sync::Arc::new(catalog), cx);
        chat.set_model("claude-sonnet-4-6", cx);
        chat.open_model_picker(window, cx);
    });
    cx.run_until_parked();
    let trigger = cx.debug_bounds("context_window").unwrap();
    let thinking_row = cx.debug_bounds("thinking-switch-row").unwrap();
    assert_eq!(trigger.left(), thinking_row.left());
    assert_eq!(trigger.size, thinking_row.size);
    assert!(trigger.size.height >= px(crate::theme::CONTROL_DETAIL_PX));
    let context_caption = cx.debug_bounds("context_window-caption").unwrap();
    let thinking_caption = cx.debug_bounds("thinking-caption").unwrap();
    assert_eq!(context_caption.left(), thinking_caption.left());
    let context_chevron = cx.debug_bounds("context_window-chevron").unwrap();
    let switch = cx.debug_bounds("thinking").unwrap();
    assert_eq!(context_chevron.right(), switch.right());
    assert!((context_chevron.center().y - trigger.center().y).abs() <= px(1.));
    assert!((switch.center().y - thinking_row.center().y).abs() <= px(1.));
    cx.simulate_keystrokes("right c right");
    cx.run_until_parked();
    let flyout = cx.debug_bounds("context_window-list").unwrap();
    assert!(flyout.left() >= trigger.right() || flyout.right() <= trigger.left());
    assert!((flyout.top() - trigger.top()).abs() <= px(1.));
    cx.simulate_keystrokes("left");
    cx.run_until_parked();
    assert!(cx.debug_bounds("context_window-list").is_none());
    assert!(cx.debug_bounds("desktop-model-picker").is_some());
    cx.simulate_keystrokes("space");
    cx.run_until_parked();
    let million = cx.debug_bounds("context_window-option-1m").unwrap();
    cx.simulate_click(million.center(), gpui::Modifiers::default());
    cx.run_until_parked();
    chat.read_with(cx, |chat, _| {
        assert!(chat.model_picker_open);
        assert!(chat.model_picker_dropdown.is_none());
        assert_eq!(chat.model_options.configured(ContextWindow), Some("1m"));
    });
    cx.simulate_resize(size(px(480.), px(690.)));
    cx.run_until_parked();
    cx.simulate_keystrokes("space");
    cx.run_until_parked();
    let trigger = cx.debug_bounds("context_window").unwrap();
    let flyout = cx.debug_bounds("context_window-list").unwrap();
    assert!(flyout.left() >= px(0.) && flyout.right() <= px(480.));
    assert!(
        flyout.left() >= trigger.right()
            || flyout.right() <= trigger.left()
            || flyout.top() >= trigger.bottom()
            || flyout.bottom() <= trigger.top()
    );
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    assert!(cx.debug_bounds("context_window-list").is_none());
    assert!(cx.debug_bounds("desktop-model-picker").is_some());
}

#[gpui::test]
fn model_picker_thinking_switch_keeps_keyboard_focus_in_place(cx: &mut TestAppContext) {
    use bruhs_models::delivery::generation::OptionKind::Thinking;
    let (chat, cx) = chat_window(cx);
    chat.update_in(cx, |chat, window, cx| {
        chat.set_model("claude-haiku-4-5", cx);
        chat.open_model_picker(window, cx);
    });
    cx.run_until_parked();
    assert!(cx.debug_bounds("thinking").is_some());
    cx.simulate_keystrokes("right space");
    cx.run_until_parked();
    chat.update_in(cx, |chat, window, _| {
        assert_eq!(chat.model_options.configured(Thinking), Some("off"));
        assert!(chat.model_picker_open && chat.model_picker_dropdown.is_none());
        assert!(chat.composer_effort_focus.is_focused(window));
    });
    assert!(cx.debug_bounds("thinking-list").is_none());
    cx.simulate_keystrokes("space");
    cx.run_until_parked();
    chat.read_with(cx, |chat, _| {
        assert_eq!(chat.model_options.configured(Thinking), Some("on"));
    });
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

#[gpui::test]
fn composer_status_pill_context_ring_and_queued_edit_banner_work(cx: &mut TestAppContext) {
    let (chat, cx) = chat_window(cx);
    cx.simulate_resize(size(px(640.0), px(480.0)));
    chat.update(cx, |chat, cx| {
        chat.permission_mode = crate::views::chat::PermissionMode::Plan;
        let mut reply =
            super::super::Message::assistant_streaming(1, Some("claude-sonnet-4-6".into()));
        reply.content = "Done".into();
        reply.usage = Some(bruhs_core::UsageInfo {
            context_tokens: Some(160_000),
            ..Default::default()
        });
        chat.messages = vec![super::super::Message::user("Hi"), reply];
        cx.notify();
    });
    cx.run_until_parked();

    // Clicking the Plan status pill opens Composer options.
    let status = cx
        .debug_bounds("desktop-composer-status")
        .expect("interactive Plan status pill");
    cx.simulate_click(status.center(), gpui::Modifiers::default());
    cx.run_until_parked();
    assert!(cx.debug_bounds("desktop-composer-options-panel").is_some());
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    assert!(cx.debug_bounds("desktop-composer-options-panel").is_none());

    // Footer context ring appears when usage exists and opens Composer options.
    let ring = cx
        .debug_bounds("desktop-composer-context-ring")
        .expect("footer context ring button");
    let send = cx.debug_bounds("desktop-send-button").unwrap();
    assert!(ring.right() <= send.left());
    assert!((ring.center().y - send.center().y).abs() <= px(1.0));
    cx.simulate_click(ring.center(), gpui::Modifiers::default());
    cx.run_until_parked();
    assert!(cx.debug_bounds("desktop-composer-options-panel").is_some());
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();

    // Editing a queued message displays the banner and Escape cancels cleanly.
    chat.update_in(cx, |chat, window, cx| {
        let tid = bruhs_models::ids::ThreadId::new_v4();
        chat.thread_id = Some(tid.to_string());
        chat.input_state.update(cx, |input, cx| {
            input.set_value("Original draft", window, cx)
        });
        chat.prompt_queue
            .push(tid.as_str(), "Queued follow up", 0.0);
        let item = chat
            .prompt_queue
            .latest_for(tid.as_str())
            .expect("pushed queued item");
        chat.begin_queue_edit(item, window, cx);
        chat.input_state
            .update(cx, |input, cx| input.focus(window, cx));
        cx.notify();
    });
    cx.run_until_parked();
    assert!(
        cx.debug_bounds("desktop-queued-edit-banner").is_some(),
        "editing a queued message shows the banner"
    );
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    assert!(
        cx.debug_bounds("desktop-queued-edit-banner").is_none(),
        "Escape cancels queued edit"
    );
    chat.read_with(cx, |chat, cx| {
        assert!(chat.queue_edit.is_none());
        assert_eq!(chat.input_state.read(cx).value(), "Original draft");
    });
}

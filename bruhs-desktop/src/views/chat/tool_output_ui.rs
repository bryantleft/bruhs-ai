//! Tool call bodies (terminal, diff, read, question), the permission prompt, and the scroll-to-bottom control.

use super::*;
pub(super) const TERMINAL_COMMAND_PREFIX: &str = "$ ";
pub(super) const TERMINAL_OUTPUT_MAX_LINES: usize = 100;
pub(super) const TERMINAL_OUTPUT_MORE_LINES_LABEL: &str = "more lines";
pub(super) const TOOL_OUTPUT_MAX_CHARS: usize = 2_000;
pub(super) const TOOL_CARD_BODY_PADDING_PX: f32 = 12.0;
pub(super) const TOOL_CARD_BODY_BORDER_OPACITY: f32 = 0.50;
pub(super) const TOOL_QUESTION_GAP_PX: f32 = 8.0;
pub(super) const TOOL_QUESTION_TEXT_SIZE_PX: f32 = 16.0;
pub(super) const TOOL_QUESTION_LINE_HEIGHT: f32 = 1.45;
pub(super) const TOOL_QUESTION_OPTIONS_GAP_PX: f32 = 4.0;
pub(super) const TOOL_QUESTION_OPTION_GAP_PX: f32 = 8.0;
pub(super) const TOOL_QUESTION_OPTION_TEXT_SIZE_PX: f32 = 14.0;
pub(super) const TOOL_QUESTION_OPTION_LINE_HEIGHT: f32 = 1.45;
pub(super) const TOOL_QUESTION_ANSWER_TOP_MARGIN_PX: f32 = 8.0;
pub(super) const TOOL_QUESTION_ANSWER_X_PADDING_PX: f32 = 12.0;
pub(super) const TOOL_QUESTION_ANSWER_Y_PADDING_PX: f32 = 8.0;
pub(super) const TOOL_QUESTION_ANSWER_TEXT_SIZE_PX: f32 = 14.0;
pub(super) const TOOL_QUESTION_ANSWER_LINE_HEIGHT: f32 = 1.45;
pub(super) const TOOL_QUESTION_ANSWER_LABEL_BOTTOM_MARGIN_PX: f32 = 4.0;
pub(super) const TOOL_QUESTION_ANSWER_LABEL_TEXT_SIZE_PX: f32 = 12.0;
pub(super) const TOOL_CODE_FONT_FAMILY: &str = "Monaspace Neon";
pub(super) const TOOL_CODE_TEXT_SIZE_PX: f32 = 12.0;
pub(super) const TOOL_OUTPUT_BLOCK_RADIUS_PX: f32 = 8.0;
pub(super) const TOOL_OUTPUT_X_PADDING_PX: f32 = 12.0;
pub(super) const TOOL_OUTPUT_Y_PADDING_PX: f32 = 8.0;
pub(super) const TOOL_OUTPUT_HEADER_Y_PADDING_PX: f32 = 6.0;
pub(super) const TOOL_OUTPUT_MAX_HEIGHT_PX: f32 = 240.0;
pub(super) const TOOL_OUTPUT_DIVIDER_OPACITY: f32 = 0.50;
pub(super) const TOOL_DIFF_HEADER_BG_OPACITY: f32 = 0.50;
pub(super) const TOOL_DIFF_HEADER_GAP_PX: f32 = 8.0;
pub(super) const TOOL_DIFF_COUNT_GAP_PX: f32 = 4.0;
pub(super) const TOOL_DIFF_LINE_GAP_PX: f32 = 8.0;
pub(super) const TOOL_DIFF_LINE_BG_OPACITY: f32 = 0.10;
pub(super) const TOOL_DIFF_LINE_Y_PADDING_PX: f32 = 2.0;
pub(super) const TOOL_DIFF_MARKER_OPACITY: f32 = 0.60;
pub(super) const TOOL_DIFF_MARKER_WIDTH_PX: f32 = 12.0;
pub(super) const PERMISSION_PROMPT_FONT_FAMILY: &str = "Monaspace Argon";
pub(super) const PERMISSION_PROMPT_TEXT_SIZE_PX: f32 = 12.0;
pub(super) const PERMISSION_PROMPT_RADIUS_PX: f32 = 8.0;
pub(super) const PERMISSION_PROMPT_X_MARGIN_PX: f32 = 16.0;
pub(super) const PERMISSION_PROMPT_Y_MARGIN_PX: f32 = 8.0;
pub(super) const PERMISSION_PROMPT_X_PADDING_PX: f32 = 12.0;
pub(super) const PERMISSION_PROMPT_Y_PADDING_PX: f32 = 8.0;
pub(super) const PERMISSION_PROMPT_BORDER_OPACITY: f32 = 0.30;
pub(super) const PERMISSION_PROMPT_DIVIDER_OPACITY: f32 = 0.20;
pub(super) const PERMISSION_PROMPT_BG_OPACITY: f32 = 0.05;
pub(super) const PERMISSION_PROMPT_ACTION_HOVER_BG_OPACITY: f32 = 0.10;
pub(super) const PERMISSION_PROMPT_ICON_SIZE_PX: f32 = 14.0;
pub(super) const PERMISSION_PROMPT_ACTION_GAP_PX: f32 = 6.0;
pub(super) const PERMISSION_PROMPT_DIVIDER_WIDTH_PX: f32 = 1.0;
pub(super) const PERMISSION_PROMPT_DIVIDER_MIN_HEIGHT_PX: f32 = 34.0;
pub(super) const CHAT_SCROLL_BOTTOM_THRESHOLD_PX: f32 = 100.0;
pub(super) const CHAT_SCROLL_TO_BOTTOM_OFFSET_PX: f32 = 16.0;
pub(super) const CHAT_SCROLL_TO_BOTTOM_SIZE_PX: f32 = 32.0;
pub(super) const CHAT_SCROLL_TO_BOTTOM_RADIUS_PX: f32 = 999.0;
pub(super) const CHAT_SCROLL_TO_BOTTOM_TEXT_SIZE_PX: f32 = 16.0;
pub(super) const CHAT_SCROLL_TO_BOTTOM_LABEL: &str = "Scroll to bottom";

pub(super) fn desktop_scroll_to_bottom_control_visible(
    offset_y_px: f32,
    max_offset_height_px: f32,
) -> bool {
    max_offset_height_px > 0.0
        && max_offset_height_px + offset_y_px > CHAT_SCROLL_BOTTOM_THRESHOLD_PX
}

pub(super) fn truncate_chars(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        return value.to_string();
    }

    let mut truncated = value.chars().take(max_chars).collect::<String>();
    truncated.push('…');
    truncated
}

pub(super) fn truncate_tool_output(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        return value.to_string();
    }

    let mut truncated = value.chars().take(max_chars).collect::<String>();
    truncated.push_str("\n...");
    truncated
}

pub(super) fn desktop_tool_pick_str<'a>(
    input: &'a serde_json::Value,
    key: &str,
) -> Option<&'a str> {
    input.get(key).and_then(serde_json::Value::as_str)
}

pub(super) fn desktop_tool_input_summary(tool: &str, input: &serde_json::Value) -> String {
    let raw = match tool {
        "Read" | "Edit" | "Write" => desktop_tool_pick_str(input, "file_path"),
        "Bash" => desktop_tool_pick_str(input, "command"),
        "Glob" | "Grep" => desktop_tool_pick_str(input, "pattern"),
        "WebSearch" | "WebFetch" => {
            desktop_tool_pick_str(input, "query").or_else(|| desktop_tool_pick_str(input, "url"))
        }
        "AskUserQuestion" => desktop_tool_pick_str(input, "question"),
        _ => None,
    };

    if let Some(raw) = raw {
        return truncate_chars(raw, 80);
    }

    for key in ["command", "path", "file_path", "pattern", "query", "url"] {
        if let Some(value) = desktop_tool_pick_str(input, key) {
            return truncate_chars(value, 80);
        }
    }

    truncate_chars(&input.to_string(), 80)
}

pub(super) fn desktop_permission_summary(tool: &str, input: &serde_json::Value) -> String {
    let raw = match tool {
        "Bash" => desktop_tool_pick_str(input, "command"),
        "Edit" | "Write" | "Read" => desktop_tool_pick_str(input, "file_path"),
        _ => None,
    };
    raw.unwrap_or_default().to_string()
}

pub(super) fn desktop_diff_lines(old_text: &str, new_text: &str) -> Vec<DesktopDiffLine> {
    let old_lines = old_text.split('\n').collect::<Vec<_>>();
    let new_lines = new_text.split('\n').collect::<Vec<_>>();
    let mut lines = Vec::new();
    let mut old_index = 0;
    let mut new_index = 0;

    while old_index < old_lines.len() || new_index < new_lines.len() {
        if old_index < old_lines.len() && new_index < new_lines.len() {
            if old_lines[old_index] == new_lines[new_index] {
                lines.push(DesktopDiffLine {
                    kind: DesktopDiffLineKind::Context,
                    content: old_lines[old_index].to_string(),
                });
                old_index += 1;
                new_index += 1;
            } else {
                lines.push(DesktopDiffLine {
                    kind: DesktopDiffLineKind::Remove,
                    content: old_lines[old_index].to_string(),
                });
                old_index += 1;
                lines.push(DesktopDiffLine {
                    kind: DesktopDiffLineKind::Add,
                    content: new_lines[new_index].to_string(),
                });
                new_index += 1;
            }
        } else if old_index < old_lines.len() {
            lines.push(DesktopDiffLine {
                kind: DesktopDiffLineKind::Remove,
                content: old_lines[old_index].to_string(),
            });
            old_index += 1;
        } else {
            lines.push(DesktopDiffLine {
                kind: DesktopDiffLineKind::Add,
                content: new_lines[new_index].to_string(),
            });
            new_index += 1;
        }
    }

    lines
}

pub(super) fn desktop_limited_terminal_output(output: &str) -> String {
    let lines = output.lines().collect::<Vec<_>>();
    if lines.len() <= TERMINAL_OUTPUT_MAX_LINES {
        return output.to_string();
    }

    format!(
        "{}\n... ({} {TERMINAL_OUTPUT_MORE_LINES_LABEL})",
        lines[..TERMINAL_OUTPUT_MAX_LINES].join("\n"),
        lines.len() - TERMINAL_OUTPUT_MAX_LINES
    )
}

pub(super) fn desktop_tool_body_text(tool_call: &DesktopToolCall) -> String {
    let output = tool_call.output.clone().unwrap_or_default();
    let body = match tool_call.tool.as_str() {
        "Bash" => {
            let command = desktop_tool_pick_str(&tool_call.input, "command").unwrap_or_default();
            if output.is_empty() {
                format!("{TERMINAL_COMMAND_PREFIX}{command}")
            } else {
                format!("{TERMINAL_COMMAND_PREFIX}{command}\n\n{output}")
            }
        }
        "Edit" => desktop_edit_tool_body(&tool_call.input, &output),
        "Write" => desktop_write_tool_body(&tool_call.input, &output),
        "AskUserQuestion" => desktop_question_tool_body(&tool_call.input, &output),
        "Read" => {
            let path = desktop_tool_pick_str(&tool_call.input, "file_path").unwrap_or_default();
            if output.is_empty() {
                path.to_string()
            } else {
                format!("{path}\n\n{output}")
            }
        }
        _ => {
            if output.is_empty() {
                serde_json::to_string_pretty(&tool_call.input)
                    .unwrap_or_else(|_| tool_call.input.to_string())
            } else {
                output
            }
        }
    };

    truncate_tool_output(&body, TOOL_OUTPUT_MAX_CHARS)
}

pub(super) fn desktop_edit_tool_body(input: &serde_json::Value, output: &str) -> String {
    let path = desktop_tool_pick_str(input, "file_path").unwrap_or_default();
    let old = desktop_tool_pick_str(input, "old_string");
    let new = desktop_tool_pick_str(input, "new_string");
    match (old, new) {
        (Some(old), Some(new)) => format!("{path}\n\n--- old\n{old}\n\n+++ new\n{new}"),
        _ if !output.is_empty() => format!("{path}\n\n{output}"),
        _ => path.to_string(),
    }
}

pub(super) fn desktop_write_tool_body(input: &serde_json::Value, output: &str) -> String {
    let path = desktop_tool_pick_str(input, "file_path").unwrap_or_default();
    let content = desktop_tool_pick_str(input, "content");
    if let Some(content) = content {
        format!("{path}\n\n+++ new\n{content}")
    } else if output.is_empty() {
        path.to_string()
    } else {
        format!("{path}\n\n{output}")
    }
}

pub(super) fn desktop_question_tool_body(input: &serde_json::Value, output: &str) -> String {
    let question = desktop_tool_pick_str(input, "question").unwrap_or_default();
    let mut lines = vec![question.to_string()];
    let options = desktop_tool_options(input);
    if !options.is_empty() {
        lines.push(String::new());
        lines.push("Options:".to_string());
        lines.extend(options.into_iter().map(|option| format!("- {option}")));
    }
    if !output.is_empty() {
        lines.push(String::new());
        lines.push("Answer:".to_string());
        lines.push(output.to_string());
    }
    lines.join("\n")
}

pub(super) fn desktop_tool_options(input: &serde_json::Value) -> Vec<String> {
    input
        .get("options")
        .and_then(serde_json::Value::as_array)
        .map(|options| {
            options
                .iter()
                .filter_map(|option| {
                    option.as_str().map(str::to_string).or_else(|| {
                        option
                            .get("label")
                            .and_then(serde_json::Value::as_str)
                            .or_else(|| option.get("value").and_then(serde_json::Value::as_str))
                            .map(str::to_string)
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

impl ChatView {
    pub(super) fn render_tool_call_body(&self, tool_call: &DesktopToolCall, cx: &App) -> Div {
        let debug_key = element_key(&tool_call.id);
        let body = match tool_call.tool.as_str() {
            "Bash" => {
                let command =
                    desktop_tool_pick_str(&tool_call.input, "command").unwrap_or_default();
                self.render_terminal_output(
                    command,
                    tool_call.output.as_deref().unwrap_or_default(),
                    cx,
                )
                .into_any_element()
            }
            "Edit" => {
                let old = desktop_tool_pick_str(&tool_call.input, "old_string");
                let new = desktop_tool_pick_str(&tool_call.input, "new_string");
                let file_path =
                    desktop_tool_pick_str(&tool_call.input, "file_path").unwrap_or_default();
                if let (Some(old), Some(new)) = (old, new) {
                    self.render_diff_view(file_path, old, new, cx)
                        .into_any_element()
                } else {
                    self.render_tool_code_block(desktop_tool_body_text(tool_call), cx)
                }
            }
            "Write" => {
                let content = desktop_tool_pick_str(&tool_call.input, "content");
                let file_path =
                    desktop_tool_pick_str(&tool_call.input, "file_path").unwrap_or_default();
                if let Some(content) = content {
                    self.render_diff_view(file_path, "", content, cx)
                        .into_any_element()
                } else {
                    self.render_tool_code_block(desktop_tool_body_text(tool_call), cx)
                }
            }
            "Read" => {
                let file_path =
                    desktop_tool_pick_str(&tool_call.input, "file_path").unwrap_or_default();
                self.render_read_output(
                    file_path,
                    tool_call.output.as_deref().unwrap_or_default(),
                    cx,
                )
                .into_any_element()
            }
            "AskUserQuestion" => self.render_question_output(
                &tool_call.input,
                tool_call.output.as_deref().unwrap_or_default(),
            ),
            _ => self.render_tool_code_block(desktop_tool_body_text(tool_call), cx),
        };

        div()
            .debug_selector(move || format!("desktop-tool-call-body-{debug_key}"))
            .border_t_1()
            .border_color(theme::longan_800().opacity(TOOL_CARD_BODY_BORDER_OPACITY))
            .p(px(TOOL_CARD_BODY_PADDING_PX))
            .child(body)
    }

    pub(super) fn render_terminal_output(&self, command: &str, output: &str, _cx: &App) -> Div {
        let limited = desktop_limited_terminal_output(output);
        div()
            .debug_selector(|| "desktop-terminal-output".to_string())
            .rounded(px(TOOL_OUTPUT_BLOCK_RADIUS_PX))
            .border_1()
            .border_color(theme::ring_panel())
            .bg(theme::longan_950())
            .overflow_hidden()
            .font_family(TOOL_CODE_FONT_FAMILY)
            .when(!command.is_empty(), |el| {
                el.child(
                    div()
                        .border_b_1()
                        .border_color(theme::longan_800().opacity(TOOL_OUTPUT_DIVIDER_OPACITY))
                        .px(px(TOOL_OUTPUT_X_PADDING_PX))
                        .py(px(TOOL_OUTPUT_HEADER_Y_PADDING_PX))
                        .text_size(px(TOOL_CODE_TEXT_SIZE_PX))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(theme::rambutan_400())
                        .child(format!("{TERMINAL_COMMAND_PREFIX}{command}")),
                )
            })
            .child(
                div()
                    .max_h(px(TOOL_OUTPUT_MAX_HEIGHT_PX))
                    .overflow_y_scrollbar()
                    .px(px(TOOL_OUTPUT_X_PADDING_PX))
                    .py(px(TOOL_OUTPUT_Y_PADDING_PX))
                    .text_size(px(TOOL_CODE_TEXT_SIZE_PX))
                    .text_color(theme::rambutan_300())
                    .child(limited),
            )
    }

    pub(super) fn render_diff_view(&self, file_path: &str, old: &str, new: &str, _cx: &App) -> Div {
        let lines = desktop_diff_lines(old, new);
        let additions = lines
            .iter()
            .filter(|line| line.kind == DesktopDiffLineKind::Add)
            .count();
        let removals = lines
            .iter()
            .filter(|line| line.kind == DesktopDiffLineKind::Remove)
            .count();
        let rows = lines
            .into_iter()
            .map(|line| self.render_diff_line(line).into_any_element())
            .collect::<Vec<_>>();

        div()
            .debug_selector(|| "desktop-diff-view".to_string())
            .rounded(px(TOOL_OUTPUT_BLOCK_RADIUS_PX))
            .border_1()
            .border_color(theme::ring_panel())
            .bg(theme::longan_950())
            .overflow_hidden()
            .font_family(TOOL_CODE_FONT_FAMILY)
            .text_size(px(TOOL_CODE_TEXT_SIZE_PX))
            .when(!file_path.is_empty(), |el| {
                el.child(
                    div()
                        .border_b_1()
                        .border_color(theme::longan_800().opacity(TOOL_OUTPUT_DIVIDER_OPACITY))
                        .bg(theme::longan_900().opacity(TOOL_DIFF_HEADER_BG_OPACITY))
                        .px(px(TOOL_OUTPUT_X_PADDING_PX))
                        .py(px(TOOL_OUTPUT_HEADER_Y_PADDING_PX))
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap(px(TOOL_DIFF_HEADER_GAP_PX))
                        .child(
                            div()
                                .min_w_0()
                                .truncate()
                                .text_color(theme::rambutan_400())
                                .child(file_path.to_string()),
                        )
                        .child(
                            div()
                                .text_color(theme::rambutan_500())
                                .flex_shrink_0()
                                .flex()
                                .gap(px(TOOL_DIFF_COUNT_GAP_PX))
                                .when(additions > 0, |el| {
                                    el.child(
                                        div()
                                            .text_color(theme::pandan_400())
                                            .child(format!("+{additions}")),
                                    )
                                })
                                .when(removals > 0, |el| {
                                    el.child(
                                        div()
                                            .text_color(theme::lychee_400())
                                            .child(format!("-{removals}")),
                                    )
                                }),
                        ),
                )
            })
            .child(
                div()
                    .max_h(px(TOOL_OUTPUT_MAX_HEIGHT_PX))
                    .overflow_y_scrollbar()
                    .children(rows),
            )
    }

    pub(super) fn render_diff_line(&self, line: DesktopDiffLine) -> Div {
        let (marker, color, background) = match line.kind {
            DesktopDiffLineKind::Add => (
                "+",
                theme::pandan_300(),
                Some(theme::pandan_400().opacity(TOOL_DIFF_LINE_BG_OPACITY)),
            ),
            DesktopDiffLineKind::Remove => (
                "-",
                theme::lychee_300(),
                Some(theme::lychee_400().opacity(TOOL_DIFF_LINE_BG_OPACITY)),
            ),
            DesktopDiffLineKind::Context => (" ", theme::rambutan_500(), None),
        };

        div()
            .flex()
            .items_start()
            .gap(px(TOOL_DIFF_LINE_GAP_PX))
            .when_some(background, |el, background| el.bg(background))
            .px(px(TOOL_OUTPUT_X_PADDING_PX))
            .py(px(TOOL_DIFF_LINE_Y_PADDING_PX))
            .text_size(px(TOOL_CODE_TEXT_SIZE_PX))
            .text_color(color)
            .child(
                div()
                    .w(px(TOOL_DIFF_MARKER_WIDTH_PX))
                    .flex_shrink_0()
                    .text_align(TextAlign::Right)
                    .text_color(color.opacity(TOOL_DIFF_MARKER_OPACITY))
                    .child(marker),
            )
            .child(div().min_w_0().child(line.content))
    }

    pub(super) fn render_read_output(&self, file_path: &str, output: &str, _cx: &App) -> Div {
        div()
            .rounded(px(TOOL_OUTPUT_BLOCK_RADIUS_PX))
            .border_1()
            .border_color(theme::ring_panel())
            .bg(theme::longan_950())
            .overflow_hidden()
            .font_family(TOOL_CODE_FONT_FAMILY)
            .when(!file_path.is_empty(), |el| {
                el.child(
                    div()
                        .border_b_1()
                        .border_color(theme::longan_800().opacity(TOOL_OUTPUT_DIVIDER_OPACITY))
                        .px(px(TOOL_OUTPUT_X_PADDING_PX))
                        .py(px(TOOL_OUTPUT_HEADER_Y_PADDING_PX))
                        .text_size(px(TOOL_CODE_TEXT_SIZE_PX))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(theme::rambutan_400())
                        .child(file_path.to_string()),
                )
            })
            .child(
                div()
                    .max_h(px(TOOL_OUTPUT_MAX_HEIGHT_PX))
                    .overflow_y_scrollbar()
                    .px(px(TOOL_OUTPUT_X_PADDING_PX))
                    .py(px(TOOL_OUTPUT_Y_PADDING_PX))
                    .text_size(px(TOOL_CODE_TEXT_SIZE_PX))
                    .text_color(theme::rambutan_300())
                    .child(truncate_tool_output(output, TOOL_OUTPUT_MAX_CHARS)),
            )
    }

    pub(super) fn render_question_output(
        &self,
        input: &serde_json::Value,
        output: &str,
    ) -> AnyElement {
        let question = desktop_tool_pick_str(input, "question").unwrap_or_default();
        let options = desktop_tool_options(input);

        div()
            .debug_selector(|| "desktop-question-output".to_string())
            .flex()
            .flex_col()
            .gap(px(TOOL_QUESTION_GAP_PX))
            .when(!question.is_empty(), |el| {
                el.child(
                    div()
                        .text_size(px(TOOL_QUESTION_TEXT_SIZE_PX))
                        .line_height(relative(TOOL_QUESTION_LINE_HEIGHT))
                        .text_color(theme::rambutan_100())
                        .child(question.to_string()),
                )
            })
            .when(!options.is_empty(), |el| {
                el.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(TOOL_QUESTION_OPTIONS_GAP_PX))
                        .children(options.into_iter().map(|option| {
                            div()
                                .flex()
                                .items_start()
                                .gap(px(TOOL_QUESTION_OPTION_GAP_PX))
                                .text_size(px(TOOL_QUESTION_OPTION_TEXT_SIZE_PX))
                                .line_height(relative(TOOL_QUESTION_OPTION_LINE_HEIGHT))
                                .text_color(theme::rambutan_300())
                                .child(
                                    div()
                                        .flex_shrink_0()
                                        .text_color(theme::rambutan_500())
                                        .child("•"),
                                )
                                .child(div().min_w_0().child(option))
                        })),
                )
            })
            .when(!output.is_empty(), |el| {
                el.child(
                    div()
                        .mt(px(TOOL_QUESTION_ANSWER_TOP_MARGIN_PX))
                        .rounded(px(TOOL_OUTPUT_BLOCK_RADIUS_PX))
                        .border_1()
                        .border_color(theme::ring_panel())
                        .bg(theme::longan_950())
                        .px(px(TOOL_QUESTION_ANSWER_X_PADDING_PX))
                        .py(px(TOOL_QUESTION_ANSWER_Y_PADDING_PX))
                        .text_size(px(TOOL_QUESTION_ANSWER_TEXT_SIZE_PX))
                        .line_height(relative(TOOL_QUESTION_ANSWER_LINE_HEIGHT))
                        .text_color(theme::rambutan_200())
                        .child(
                            div()
                                .mb(px(TOOL_QUESTION_ANSWER_LABEL_BOTTOM_MARGIN_PX))
                                .text_size(px(TOOL_QUESTION_ANSWER_LABEL_TEXT_SIZE_PX))
                                .font_family(theme::UI_SMALL_FONT_FAMILY)
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(theme::rambutan_500())
                                .child("Answer"),
                        )
                        .child(div().child(output.to_string())),
                )
            })
            .into_any_element()
    }

    pub(super) fn render_permission_prompt(&self, cx: &Context<'_, Self>) -> Div {
        let Some(request) = self.pending_permission.as_ref() else {
            return div();
        };
        let summary = desktop_permission_summary(&request.tool, &request.input);
        div()
            .mx(px(PERMISSION_PROMPT_X_MARGIN_PX))
            .my(px(PERMISSION_PROMPT_Y_MARGIN_PX))
            .child(
                div()
                    .debug_selector(|| "desktop-permission-prompt".to_string())
                    .overflow_hidden()
                    .rounded(px(PERMISSION_PROMPT_RADIUS_PX))
                    .border_1()
                    .border_color(theme::durian_400().opacity(PERMISSION_PROMPT_BORDER_OPACITY))
                    .bg(theme::durian_400().opacity(PERMISSION_PROMPT_BG_OPACITY))
                    .child(
                        div()
                            .debug_selector(|| "desktop-permission-summary".to_string())
                            .px(px(PERMISSION_PROMPT_X_PADDING_PX))
                            .py(px(PERMISSION_PROMPT_Y_PADDING_PX))
                            .border_b_1()
                            .border_color(
                                theme::durian_400().opacity(PERMISSION_PROMPT_DIVIDER_OPACITY),
                            )
                            .flex()
                            .items_center()
                            .gap(px(PERMISSION_PROMPT_Y_PADDING_PX))
                            .font_family(PERMISSION_PROMPT_FONT_FAMILY)
                            .font_weight(FontWeight::MEDIUM)
                            .text_size(px(PERMISSION_PROMPT_TEXT_SIZE_PX))
                            .font_family(theme::UI_SMALL_FONT_FAMILY)
                            .text_color(theme::durian_300())
                            .child(
                                svg()
                                    .path(assets::ALERT_TRIANGLE_ICON_PATH)
                                    .size(px(PERMISSION_PROMPT_ICON_SIZE_PX))
                                    .text_color(theme::durian_300())
                                    .flex_shrink_0(),
                            )
                            .child(
                                div()
                                    .font_weight(FontWeight::MEDIUM)
                                    .child("Permission needed"),
                            ),
                    )
                    .child(
                        div()
                            .px(px(PERMISSION_PROMPT_X_PADDING_PX))
                            .py(px(PERMISSION_PROMPT_Y_PADDING_PX))
                            .flex()
                            .items_center()
                            .font_family(PERMISSION_PROMPT_FONT_FAMILY)
                            .font_weight(FontWeight::MEDIUM)
                            .text_size(px(PERMISSION_PROMPT_TEXT_SIZE_PX))
                            .font_family(theme::UI_SMALL_FONT_FAMILY)
                            .text_color(theme::rambutan_400())
                            .child(
                                div()
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(theme::rambutan_200())
                                    .child(request.tool.clone()),
                            )
                            .when(!summary.is_empty(), |el| {
                                el.child(
                                    div()
                                        .min_w_0()
                                        .truncate()
                                        .text_color(theme::rambutan_500())
                                        .child(format!(" — {summary}")),
                                )
                            }),
                    )
                    .child(
                        div()
                            .border_t_1()
                            .border_color(
                                theme::durian_400().opacity(PERMISSION_PROMPT_DIVIDER_OPACITY),
                            )
                            .flex()
                            .items_center()
                            .child(
                                div()
                                    .id("desktop-permission-deny")
                                    .debug_selector(|| "desktop-permission-deny".to_string())
                                    .flex_1()
                                    .py(px(PERMISSION_PROMPT_Y_PADDING_PX))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .gap(px(PERMISSION_PROMPT_ACTION_GAP_PX))
                                    .font_family(PERMISSION_PROMPT_FONT_FAMILY)
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_size(px(PERMISSION_PROMPT_TEXT_SIZE_PX))
                                    .font_family(theme::UI_SMALL_FONT_FAMILY)
                                    .text_color(theme::rambutan_500())
                                    .tab_index(0)
                                    .role(Role::Button)
                                    .aria_label("Deny permission")
                                    .focus_visible(|el| {
                                        el.border_2().border_color(theme::persimmon_400())
                                    })
                                    .cursor_pointer()
                                    .hover(|el| {
                                        el.bg(theme::lychee_400()
                                            .opacity(PERMISSION_PROMPT_ACTION_HOVER_BG_OPACITY))
                                            .text_color(theme::lychee_300())
                                    })
                                    .on_click(cx.listener(|this, _ev, _window, cx| {
                                        this.respond_to_permission(false, cx);
                                    }))
                                    .on_key_down(cx.listener(
                                        |this, event: &KeyDownEvent, window, cx| {
                                            if is_web_button_activation_key(
                                                event.keystroke.key.as_str(),
                                            ) {
                                                window.prevent_default();
                                                cx.stop_propagation();
                                                this.respond_to_permission(false, cx);
                                            }
                                        },
                                    ))
                                    .child(
                                        svg()
                                            .path(assets::CLOSE_ICON_PATH)
                                            .size(px(PERMISSION_PROMPT_ICON_SIZE_PX))
                                            .text_color(theme::lychee_300())
                                            .flex_shrink_0(),
                                    )
                                    .child("Deny"),
                            )
                            .child(
                                div()
                                    .w(px(PERMISSION_PROMPT_DIVIDER_WIDTH_PX))
                                    .h_full()
                                    .min_h(px(PERMISSION_PROMPT_DIVIDER_MIN_HEIGHT_PX))
                                    .bg(theme::durian_400()
                                        .opacity(PERMISSION_PROMPT_DIVIDER_OPACITY)),
                            )
                            .child(
                                div()
                                    .id("desktop-permission-approve")
                                    .debug_selector(|| "desktop-permission-approve".to_string())
                                    .flex_1()
                                    .py(px(PERMISSION_PROMPT_Y_PADDING_PX))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .gap(px(PERMISSION_PROMPT_ACTION_GAP_PX))
                                    .font_family(PERMISSION_PROMPT_FONT_FAMILY)
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_size(px(PERMISSION_PROMPT_TEXT_SIZE_PX))
                                    .font_family(theme::UI_SMALL_FONT_FAMILY)
                                    .text_color(theme::pandan_400())
                                    .tab_index(0)
                                    .role(Role::Button)
                                    .aria_label("Approve permission")
                                    .focus_visible(|el| {
                                        el.border_2().border_color(theme::persimmon_400())
                                    })
                                    .cursor_pointer()
                                    .hover(|el| {
                                        el.bg(theme::pandan_400()
                                            .opacity(PERMISSION_PROMPT_ACTION_HOVER_BG_OPACITY))
                                    })
                                    .on_click(cx.listener(|this, _ev, _window, cx| {
                                        this.respond_to_permission(true, cx);
                                    }))
                                    .on_key_down(cx.listener(
                                        |this, event: &KeyDownEvent, window, cx| {
                                            if is_web_button_activation_key(
                                                event.keystroke.key.as_str(),
                                            ) {
                                                window.prevent_default();
                                                cx.stop_propagation();
                                                this.respond_to_permission(true, cx);
                                            }
                                        },
                                    ))
                                    .child(
                                        svg()
                                            .path(assets::CHECK_ICON_PATH)
                                            .size(px(PERMISSION_PROMPT_ICON_SIZE_PX))
                                            .text_color(theme::pandan_400())
                                            .flex_shrink_0(),
                                    )
                                    .child("Approve"),
                            ),
                    ),
            )
    }

    pub(super) fn render_tool_code_block(&self, text: String, _cx: &App) -> AnyElement {
        div()
            .max_h(px(TOOL_OUTPUT_MAX_HEIGHT_PX))
            .overflow_y_scrollbar()
            .rounded(px(TOOL_OUTPUT_BLOCK_RADIUS_PX))
            .border_1()
            .border_color(theme::ring_panel())
            .bg(theme::longan_950())
            .px(px(TOOL_OUTPUT_X_PADDING_PX))
            .py(px(TOOL_OUTPUT_Y_PADDING_PX))
            .text_size(px(TOOL_CODE_TEXT_SIZE_PX))
            .font_family(TOOL_CODE_FONT_FAMILY)
            .text_color(theme::rambutan_300())
            .child(text)
            .into_any_element()
    }

    pub(super) fn render_scroll_to_bottom_control(&self, cx: &Context<'_, Self>) -> AnyElement {
        div()
            .id("desktop-scroll-to-bottom")
            .absolute()
            .bottom(px(CHAT_SCROLL_TO_BOTTOM_OFFSET_PX))
            .right(px(CHAT_SCROLL_TO_BOTTOM_OFFSET_PX))
            .size(px(CHAT_SCROLL_TO_BOTTOM_SIZE_PX))
            .rounded(px(CHAT_SCROLL_TO_BOTTOM_RADIUS_PX))
            .bg(theme::longan_700())
            .border_1()
            .border_color(theme::ring_floating())
            .flex()
            .items_center()
            .justify_center()
            .text_size(px(CHAT_SCROLL_TO_BOTTOM_TEXT_SIZE_PX))
            .text_color(theme::rambutan_200())
            .tab_index(0)
            .role(Role::Button)
            .aria_label(CHAT_SCROLL_TO_BOTTOM_LABEL)
            .focus_visible(|el| el.border_2().border_color(theme::persimmon_400()))
            .cursor_pointer()
            .hover(|el| el.bg(theme::longan_600()))
            .tooltip(|window, cx| Tooltip::new(CHAT_SCROLL_TO_BOTTOM_LABEL).build(window, cx))
            .on_click(cx.listener(|this, _ev, _window, cx| {
                this.scroll_messages_to_bottom();
                cx.notify();
            }))
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                if is_web_button_activation_key(event.keystroke.key.as_str()) {
                    window.prevent_default();
                    cx.stop_propagation();
                    this.scroll_messages_to_bottom();
                    cx.notify();
                }
            }))
            .child("↓")
            .into_any_element()
    }

    pub(super) fn show_scroll_to_bottom_control(&self) -> bool {
        let offset_y = f32::from(self.messages_scroll_handle.offset().y);
        let max_offset_height = f32::from(self.messages_scroll_handle.max_offset().y);
        desktop_scroll_to_bottom_control_visible(offset_y, max_offset_height)
    }
}

//! Composer hints: `/` commands, `#` pull requests, `$` skills, and `@` mentions.

use super::*;

pub(super) use bruhs_models::logic::commands::SLASH_COMMANDS as DESKTOP_SLASH_COMMANDS;
pub(super) const COMPOSER_HINT_EMPTY_LABEL: &str = "No matches";
pub(super) const COMPOSER_SLASH_ICON: &str = "/";
pub(super) const COMPOSER_DIRECTORY_ICON: &str = "▸";
pub(super) const COMPOSER_FILE_ICON: &str = "·";
pub(super) const COMPOSER_HINT_POPOVER_OFFSET_PX: f32 = 4.0;
pub(super) const COMPOSER_HINT_POPOVER_RADIUS_PX: f32 = 8.0;
pub(super) const COMPOSER_HINT_POPOVER_MAX_HEIGHT_PX: f32 = 288.0;
pub(super) const COMPOSER_HINT_ICON_SLOT_WIDTH_PX: f32 = 12.0;
pub(super) const COMPOSER_HINT_ROW_TEXT_SIZE_PX: f32 = 14.0;
pub(super) const COMPOSER_HINT_SUB_TEXT_SIZE_PX: f32 = 12.0;
/// Quiet time after a keystroke before the `@` search runs.
pub(super) const MENTION_SEARCH_DEBOUNCE: Duration = Duration::from_millis(40);

pub(super) fn thread_title_after_clear_slash(current_title: &str) -> String {
    current_title.to_string()
}

pub(super) fn slash_command_query(
    text: &str,
    cursor: usize,
    suppressed_value: Option<&str>,
) -> Option<String> {
    if suppressed_value == Some(text) {
        return None;
    }

    let cursor = clamp_to_char_boundary(text, cursor);
    let before = &text[..cursor];
    for (index, ch) in before.char_indices().rev() {
        if ch.is_whitespace() {
            break;
        }
        if ch == '/' {
            let preceding = before[..index].chars().last();
            if preceding.is_none() || preceding == Some('\n') {
                return Some(before[index + 1..].to_ascii_lowercase());
            }
            return None;
        }
    }

    None
}

pub(super) fn clear_slash_trigger_text(text: &str, cursor: usize) -> String {
    let cursor = clamp_to_char_boundary(text, cursor);
    let before = &text[..cursor];
    let after = &text[cursor..];

    let mut trigger_pos = before.len();
    for (index, ch) in before.char_indices().rev() {
        if ch.is_whitespace() {
            break;
        }
        trigger_pos = index;
        if ch == '/' {
            break;
        }
    }

    let mut output = String::with_capacity(text.len());
    output.push_str(&before[..trigger_pos]);
    if !after.starts_with(char::is_whitespace) {
        output.push(' ');
    }
    output.push_str(after);
    output.trim_end().to_string()
}

pub(super) fn clamp_composer_hint_index(index: usize, count: usize) -> usize {
    if count == 0 {
        return 0;
    }
    index.min(count - 1)
}

pub(super) fn move_composer_hint_index(
    index: usize,
    count: usize,
    direction: ComposerHintDirection,
) -> usize {
    if count == 0 {
        return 0;
    }

    let index = clamp_composer_hint_index(index, count);
    match direction {
        ComposerHintDirection::Previous => {
            if index == 0 {
                count - 1
            } else {
                index - 1
            }
        }
        ComposerHintDirection::Next => (index + 1) % count,
    }
}

pub(super) fn select_composer_hint_index_on_hover(
    current: &mut usize,
    hovered: bool,
    index: usize,
) -> bool {
    if !hovered || *current == index {
        return false;
    }

    *current = index;
    true
}

pub(super) fn detect_mention_query(text: &str, cursor: usize) -> Option<String> {
    detect_marker_query(text, cursor, '@')
}

/// `#` after the start of the draft or whitespace. `# ` with a space stays
/// a heading, so the walk stops on that space and returns nothing.
pub(super) fn detect_pull_query(text: &str, cursor: usize) -> Option<String> {
    detect_marker_query(text, cursor, '#')
}

/// `$` after the start of the draft or whitespace; dollar amounts stay literal.
pub(super) fn detect_skill_query(text: &str, cursor: usize) -> Option<String> {
    detect_marker_query(text, cursor, '$')
        .filter(|query| !query.starts_with(|ch: char| ch.is_ascii_digit()))
}

fn detect_marker_query(text: &str, cursor: usize, marker: char) -> Option<String> {
    let cursor = clamp_to_char_boundary(text, cursor);
    let before = &text[..cursor];

    for (index, ch) in before.char_indices().rev() {
        if ch.is_whitespace() {
            break;
        }
        if ch == marker {
            let preceding = before[..index].chars().last();
            if preceding.is_none() || preceding.is_some_and(char::is_whitespace) {
                return Some(before[index + 1..].to_string());
            }
            return None;
        }
    }

    None
}

fn matching_thread_hits(cx: &App, query: &str, current: Option<&str>) -> Vec<ThreadHit> {
    let Some(catalog) = cx.try_global::<crate::app_state::ThreadCatalog>() else {
        return Vec::new();
    };
    let needle = query.to_lowercase();
    catalog
        .threads
        .iter()
        .filter(|(id, title)| {
            current != Some(id.as_str())
                && (needle.is_empty() || title.to_lowercase().contains(&needle))
        })
        .take(8)
        .map(|(id, title)| ThreadHit {
            id: id.clone(),
            title: title.clone(),
        })
        .collect()
}

fn trigger_span(text: &str, cursor: usize, marker: char) -> Option<(usize, usize)> {
    let cursor = clamp_to_char_boundary(text, cursor);
    let before = &text[..cursor];
    for (index, ch) in before.char_indices().rev() {
        if ch.is_whitespace() {
            return None;
        }
        if ch == marker {
            let preceding = before[..index].chars().last();
            if preceding.is_none() || preceding.is_some_and(char::is_whitespace) {
                return Some((index, cursor));
            }
            return None;
        }
    }
    None
}

/// Text-only `@` replacement. Production inserts a file chip; tests keep
/// this splice.
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "text-only mention splice; production inserts a chip, tests cover this path"
    )
)]
pub(super) fn replace_mention_trigger(text: &str, cursor: usize, replacement: &str) -> String {
    let cursor = clamp_to_char_boundary(text, cursor);
    let before = &text[..cursor];
    let after = &text[cursor..];

    let mut trigger_pos = before.len();
    for (index, ch) in before.char_indices().rev() {
        if ch.is_whitespace() {
            break;
        }
        trigger_pos = index;
        if ch == '@' {
            break;
        }
    }

    let mut output = String::with_capacity(text.len() + replacement.len() + 1);
    output.push_str(&before[..trigger_pos]);
    output.push_str(replacement);
    if !after.starts_with(char::is_whitespace) {
        output.push(' ');
    }
    output.push_str(after);
    output
}

pub(super) fn clamp_to_char_boundary(text: &str, mut index: usize) -> usize {
    index = index.min(text.len());
    while index > 0 && !text.is_char_boundary(index) {
        index -= 1;
    }
    index
}

pub(super) fn desktop_mention_icon(is_dir: bool) -> &'static str {
    if is_dir {
        COMPOSER_DIRECTORY_ICON
    } else {
        COMPOSER_FILE_ICON
    }
}

impl ChatView {
    /// ⌘P: put an `@` in the composer so the file picker opens.
    pub fn mention_file(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let value = self.input_state.read(cx).value().to_string();
        let next = if value.is_empty() || value.ends_with(char::is_whitespace) {
            format!("{value}@")
        } else {
            format!("{value} @")
        };
        self.composer_hints_suppressed_value = None;
        let end = next.len();
        self.input_state.update(cx, |input, cx| {
            input.set_value(next, window, cx);
            // Mention detection reads the cursor: put it after the `@`.
            input.set_selected_range(end..end, cx);
            input.focus(window, cx);
        });
        self.composer_hint_selected_index = 0;
        self.refresh_mention_hints(cx);
        cx.notify();
    }

    pub(super) fn run_slash_command(&mut self, content: &str, cx: &mut Context<'_, Self>) -> bool {
        let command = content.split_whitespace().next().unwrap_or_default();
        match command {
            "/clear" => {
                self.messages.clear();
                self.forget_message_expansions();
                self.content_blocks.clear();
                self.thread_title = thread_title_after_clear_slash(&self.thread_title);
                cx.emit(ChatViewEvent::MessagesChanged);
                cx.notify();
                true
            }
            "/help" => {
                self.messages.push(Message {
                    role: MessageRole::Assistant,
                    content: bruhs_models::logic::commands::COMMAND_HELP.to_string(),
                    model: None,
                    thinking: String::new(),
                    usage: None,
                    duration_ms: None,
                    started_at_ms: Some(EpochMs::now().as_millis()),
                    completed_at_ms: Some(EpochMs::now().as_millis()),
                    tool_calls: Vec::new(),
                    files: Vec::new(),
                    state: TurnState::Finished,
                    checkpoint: None,
                });
                self.scroll_messages_to_bottom();
                cx.emit(ChatViewEvent::MessagesChanged);
                cx.notify();
                true
            }
            "/model" | "/settings" => {
                cx.emit(ChatViewEvent::OpenSettings);
                true
            }
            "/goal" | "/compact" | "/handoff" | "/second-opinion" => {
                self.apply_local_command(command, cx);
                true
            }
            _ => false,
        }
    }

    pub(super) fn slash_command_matches(
        &self,
        cx: &App,
    ) -> Vec<&'static bruhs_models::logic::commands::SlashCommand> {
        let (value, cursor) = {
            let input = self.input_state.read(cx);
            (input.value().to_string(), input.cursor())
        };
        let Some(query) = slash_command_query(
            &value,
            cursor,
            self.composer_hints_suppressed_value.as_deref(),
        ) else {
            return Vec::new();
        };
        DESKTOP_SLASH_COMMANDS
            .iter()
            .filter(|command| {
                query.is_empty() || command.name.to_ascii_lowercase().contains(&query)
            })
            .collect()
    }

    pub(super) fn slash_command_trigger_active(&self, cx: &App) -> bool {
        let (value, cursor) = {
            let input = self.input_state.read(cx);
            (input.value().to_string(), input.cursor())
        };
        slash_command_query(
            &value,
            cursor,
            self.composer_hints_suppressed_value.as_deref(),
        )
        .is_some()
    }

    pub(super) fn render_slash_command_hints(&self, cx: &Context<'_, Self>) -> AnyElement {
        if !self.slash_command_trigger_active(cx) {
            return div().into_any_element();
        }

        let matches = self.slash_command_matches(cx);
        let selected_index =
            clamp_composer_hint_index(self.composer_hint_selected_index, matches.len());
        let rows = if matches.is_empty() {
            vec![div()
                .px_3()
                .py_2()
                .text_size(px(COMPOSER_HINT_SUB_TEXT_SIZE_PX))
                .font_family(theme::UI_SMALL_FONT_FAMILY)
                .font_weight(FontWeight::MEDIUM)
                .text_color(theme::rambutan_500())
                .child(COMPOSER_HINT_EMPTY_LABEL)
                .into_any_element()]
        } else {
            matches
                .into_iter()
                .enumerate()
                .map(|(index, command)| {
                    let name = command.name;
                    let debug_name = name.trim_start_matches('/');
                    let is_selected = index == selected_index;
                    div()
                        .id(("desktop-slash-command-row", element_key(name)))
                        .debug_selector(move || format!("desktop-slash-command-row-{debug_name}"))
                        .w_full()
                        .flex()
                        .items_center()
                        .gap_2()
                        .px_3()
                        .py_1p5()
                        .text_size(px(COMPOSER_HINT_ROW_TEXT_SIZE_PX))
                        .text_color(if is_selected {
                            theme::rambutan_100()
                        } else {
                            theme::rambutan_300()
                        })
                        .cursor_pointer()
                        .when(is_selected, |el| el.bg(theme::longan_800()))
                        .hover(|el| el.bg(theme::longan_800()).text_color(theme::rambutan_100()))
                        .on_hover(cx.listener(move |this, hovered, _window, cx| {
                            if select_composer_hint_index_on_hover(
                                &mut this.composer_hint_selected_index,
                                *hovered,
                                index,
                            ) {
                                cx.notify();
                            }
                        }))
                        .on_click(cx.listener(move |this, _ev, window, cx| {
                            if this.run_slash_command(name, cx) {
                                this.clear_active_slash_trigger(window, cx);
                            }
                        }))
                        .child(
                            div()
                                .w(px(COMPOSER_HINT_ICON_SLOT_WIDTH_PX))
                                .flex_shrink_0()
                                .text_align(TextAlign::Center)
                                .text_color(theme::rambutan_500())
                                .child(COMPOSER_SLASH_ICON),
                        )
                        .child(
                            div()
                                .min_w_0()
                                .flex_1()
                                .flex()
                                .items_center()
                                .gap_2()
                                .child(
                                    div()
                                        .min_w_0()
                                        .flex_1()
                                        .truncate()
                                        .font_weight(FontWeight::MEDIUM)
                                        .child(command.name),
                                )
                                .child(
                                    div()
                                        .min_w_0()
                                        .truncate()
                                        .text_size(px(COMPOSER_HINT_SUB_TEXT_SIZE_PX))
                                        .font_family(theme::UI_SMALL_FONT_FAMILY)
                                        .font_weight(FontWeight::MEDIUM)
                                        .text_color(theme::rambutan_500())
                                        .child(command.description),
                                ),
                        )
                        .into_any_element()
                })
                .collect::<Vec<_>>()
        };

        div()
            .debug_selector(|| "desktop-slash-command-hints".to_string())
            .mb(px(COMPOSER_HINT_POPOVER_OFFSET_PX))
            .rounded(px(COMPOSER_HINT_POPOVER_RADIUS_PX))
            .border_1()
            .border_color(theme::ring_floating())
            .bg(theme::longan_700())
            .py_1()
            .max_h(px(COMPOSER_HINT_POPOVER_MAX_HEIGHT_PX))
            .overflow_y_scrollbar()
            .children(rows)
            .into_any_element()
    }

    pub(super) fn composer_hint_popover_open(&self, cx: &App) -> bool {
        self.slash_command_trigger_active(cx)
            || self.mention_query.is_some()
            || self.pull_query.is_some()
            || self.skill_query.is_some()
    }

    pub(super) fn composer_hint_count(&self, cx: &App) -> usize {
        if self.slash_command_trigger_active(cx) {
            return self.slash_command_matches(cx).len();
        }
        if self.pull_query.is_some() {
            return self.pull_items.len();
        }
        if self.skill_query.is_some() {
            return self.skill_items.len();
        }
        if self.mention_query.is_some() {
            return self.thread_hits.len() + self.mention_items.len();
        }
        0
    }

    pub(super) fn sync_composer_hint_suppression(&mut self, cx: &App) {
        let value = self.input_state.read(cx).value().to_string();
        if self.composer_hints_suppressed_value.as_deref() != Some(value.as_str()) {
            self.composer_hints_suppressed_value = None;
        }
    }

    pub(super) fn dismiss_composer_hints(&mut self, cx: &mut Context<'_, Self>) {
        let value = self.input_state.read(cx).value().to_string();
        self.composer_hints_suppressed_value = Some(value);
        self.clear_mention_hints();
        self.clear_skill_hints();
        self.clear_pull_hints();
        self.composer_hint_selected_index = 0;
        cx.notify();
    }

    pub(super) fn handle_composer_hint_key(
        &mut self,
        key: &str,
        _window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> bool {
        if !self.composer_hint_popover_open(cx) {
            return false;
        }

        match key {
            "escape" => {
                self.dismiss_composer_hints(cx);
                true
            }
            "up" | "down" => {
                let count = self.composer_hint_count(cx);
                if count > 0 {
                    let direction = if key == "up" {
                        ComposerHintDirection::Previous
                    } else {
                        ComposerHintDirection::Next
                    };
                    self.composer_hint_selected_index = move_composer_hint_index(
                        self.composer_hint_selected_index,
                        count,
                        direction,
                    );
                    cx.notify();
                }
                true
            }
            _ => false,
        }
    }

    pub(super) fn apply_selected_composer_hint(
        &mut self,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> bool {
        self.restore_composer_hint_input_after_enter(window, cx);

        if self.slash_command_trigger_active(cx) {
            let matches = self.slash_command_matches(cx);
            let selected_index =
                clamp_composer_hint_index(self.composer_hint_selected_index, matches.len());
            let Some(command) = matches.get(selected_index).copied() else {
                return false;
            };
            if self.run_slash_command(command.name, cx) {
                self.clear_active_slash_trigger(window, cx);
                return true;
            }
        }

        if self.pull_query.is_some() {
            let selected_index =
                clamp_composer_hint_index(self.composer_hint_selected_index, self.pull_items.len());
            let Some(pull) = self.pull_items.get(selected_index).cloned() else {
                return false;
            };
            self.insert_context_chip(
                ChipKind::PullRequest {
                    number: pull.number,
                    title: pull.title,
                    url: pull.url,
                    state: pull.state,
                },
                window,
                cx,
            );
            return true;
        }

        if self.skill_query.is_some() {
            let selected_index = clamp_composer_hint_index(
                self.composer_hint_selected_index,
                self.skill_items.len(),
            );
            let Some(name) = self
                .skill_items
                .get(selected_index)
                .map(|skill| skill.name.clone())
            else {
                return false;
            };
            self.insert_skill_mention(&name, window, cx);
            return true;
        }

        if self.mention_query.is_some() {
            let total = self.thread_hits.len() + self.mention_items.len();
            let selected_index =
                clamp_composer_hint_index(self.composer_hint_selected_index, total);
            if let Some(hit) = self.thread_hits.get(selected_index) {
                let id = hit.id.clone();
                let title = hit.title.clone();
                self.insert_context_chip(
                    ChipKind::Thread {
                        thread_id: id,
                        title,
                    },
                    window,
                    cx,
                );
                return true;
            }
            let file_index = selected_index - self.thread_hits.len();
            let Some(path) = self
                .mention_items
                .get(file_index)
                .map(|item| item.path.clone())
            else {
                return false;
            };
            self.insert_file_mention(&path, window, cx);
            return true;
        }

        false
    }

    pub(super) fn restore_composer_hint_input_after_enter(
        &mut self,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let (value, cursor) = {
            let input = self.input_state.read(cx);
            (input.value().to_string(), input.cursor())
        };
        let cursor = clamp_to_char_boundary(&value, cursor);
        let Some(newline_start) = value[..cursor]
            .char_indices()
            .last()
            .and_then(|(index, ch)| if ch == '\n' { Some(index) } else { None })
        else {
            return;
        };

        let mut candidate = value;
        candidate.replace_range(newline_start..cursor, "");
        let candidate_cursor = newline_start;
        let suppressed = self.composer_hints_suppressed_value.as_deref();
        let hint_still_active = slash_command_query(&candidate, candidate_cursor, suppressed)
            .is_some()
            || detect_mention_query(&candidate, candidate_cursor).is_some()
            || detect_pull_query(&candidate, candidate_cursor).is_some()
            || detect_skill_query(&candidate, candidate_cursor).is_some();
        if !hint_still_active {
            return;
        }

        self.input_state.update(cx, |input, cx| {
            input.set_value(candidate, window, cx);
            let position = input.text().offset_to_position(candidate_cursor);
            input.set_cursor_position(position, window, cx);
        });
        self.refresh_mention_hints(cx);
        cx.notify();
    }

    pub(super) fn clear_mention_hints(&mut self) {
        self.mention_query = None;
        self.mention_items.clear();
        self.mention_task = None;
        self.thread_hits.clear();
    }

    fn clear_skill_hints(&mut self) {
        self.skill_query = None;
        self.skill_items.clear();
        self.skill_error = None;
        self.skill_task = None;
    }

    fn refresh_skill_hints(&mut self, query: String, cx: &mut Context<'_, Self>) {
        self.skill_query = Some(query.clone());
        let provider = bruhs_models::agent::AgentProvider::for_model(&self.model_id);
        let project = if self.project_path.is_empty() {
            None
        } else {
            Some(std::path::PathBuf::from(&self.project_path))
        };
        self.skill_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(MENTION_SEARCH_DEBOUNCE)
                .await;
            let Ok(search) = this.update(cx, |_, cx| {
                let query = query.clone();
                let project = project.clone();
                spawn_tokio(cx, async move {
                    let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
                    bruhs_core::list_skills(provider, home.as_deref(), project.as_deref(), &query)
                })
            }) else {
                return;
            };
            let skills = search.await;
            let _closed = this.update(cx, |this, cx| {
                if this.skill_query.as_deref() == Some(query.as_str()) {
                    this.skill_items = skills;
                    this.skill_error = None;
                    cx.notify();
                }
            });
        }));
    }

    pub(super) fn insert_skill_mention(
        &mut self,
        name: &str,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let (value, cursor) = {
            let input = self.input_state.read(cx);
            (input.value().to_string(), input.cursor())
        };
        let (next, caret) =
            bruhs_models::logic::commands::insert_skill_mention(&value, cursor, name);
        self.input_state.update(cx, |input, cx| {
            input.set_value(next, window, cx);
            input.set_selected_range(caret..caret, cx);
            input.focus(window, cx);
        });
        self.composer_hint_selected_index = 0;
        self.composer_hints_suppressed_value = None;
        self.clear_skill_hints();
        self.clear_mention_hints();
        self.clear_pull_hints();
        cx.notify();
    }

    fn clear_pull_hints(&mut self) {
        self.pull_query = None;
        self.pull_items.clear();
        self.pull_error = None;
        self.pull_task = None;
    }

    pub(super) fn clear_active_slash_trigger(
        &mut self,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.input_state.update(cx, |input, cx| {
            let value = input.value().to_string();
            let cursor = input.cursor();
            input.set_value(clear_slash_trigger_text(&value, cursor), window, cx);
            input.focus(window, cx);
        });
        self.composer_hint_selected_index = 0;
        self.composer_hints_suppressed_value = None;
        self.clear_mention_hints();
        cx.notify();
    }

    pub(super) fn refresh_mention_hints(&mut self, cx: &mut Context<'_, Self>) {
        let (value, cursor) = {
            let input = self.input_state.read(cx);
            (input.value().to_string(), input.cursor())
        };
        if self.composer_hints_suppressed_value.as_deref() == Some(value.as_str()) {
            self.clear_mention_hints();
            self.clear_skill_hints();
            self.clear_pull_hints();
            return;
        }

        if let Some(query) = detect_pull_query(&value, cursor) {
            self.clear_mention_hints();
            self.clear_skill_hints();
            self.refresh_pull_hints(query, cx);
            return;
        }
        self.clear_pull_hints();

        if let Some(query) = detect_skill_query(&value, cursor) {
            self.clear_mention_hints();
            self.refresh_skill_hints(query, cx);
            return;
        }
        self.clear_skill_hints();

        let Some(query) = detect_mention_query(&value, cursor) else {
            self.clear_mention_hints();
            return;
        };

        self.thread_hits = matching_thread_hits(cx, &query, self.thread_id.as_deref());
        // The previous matches stay up until this search answers (web does
        // the same), so the list does not flash empty while typing.
        self.mention_query = Some(query.clone());
        let root = PathBuf::from(&self.project_path);
        self.mention_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(MENTION_SEARCH_DEBOUNCE)
                .await;
            let Ok(search) = this.update(cx, |_, cx| {
                spawn_tokio(cx, git_info::mention_matches(root, query.clone()))
            }) else {
                return;
            };
            let items = search.await;
            // The chat may have closed meanwhile; then nobody needs the list.
            let _closed = this.update(cx, |this, cx| {
                if this.mention_query.as_deref() == Some(query.as_str()) {
                    this.mention_items = items;
                    cx.notify();
                }
            });
        }));
    }

    fn refresh_pull_hints(&mut self, query: String, cx: &mut Context<'_, Self>) {
        self.pull_query = Some(query.clone());
        let root = std::path::PathBuf::from(&self.project_path);
        if self.project_path.is_empty() || !root.is_dir() {
            self.pull_items.clear();
            self.pull_error = Some("Open a project to attach a pull request.".to_string());
            self.pull_task = None;
            return;
        }
        self.pull_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(MENTION_SEARCH_DEBOUNCE)
                .await;
            let Ok(search) = this.update(cx, |_, cx| {
                let root = root.clone();
                let query = query.clone();
                spawn_tokio(
                    cx,
                    async move { bruhs_core::list_pulls(&root, &query).await },
                )
            }) else {
                return;
            };
            let list = search.await;
            let _closed = this.update(cx, |this, cx| {
                if this.pull_query.as_deref() == Some(query.as_str()) {
                    this.pull_items = list.pulls;
                    this.pull_error = list.error;
                    cx.notify();
                }
            });
        }));
    }

    pub(super) fn insert_file_mention(
        &mut self,
        path: &str,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.insert_context_chip(
            ChipKind::File {
                path: path.to_string(),
            },
            window,
            cx,
        );
    }

    /// Put `kind` at the caret. An open `@` or `#` token is replaced by the
    /// chip label. The draft text stays the label so the caret stays aligned.
    pub(crate) fn insert_context_chip(
        &mut self,
        kind: ChipKind,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        use bruhs_models::logic::context_chip::{insert_chip, replace_range};

        let (value, cursor) = {
            let input = self.input_state.read(cx);
            (input.value().to_string(), input.cursor())
        };
        let marker = if self.pull_query.is_some() {
            Some('#')
        } else if self.mention_query.is_some() {
            Some('@')
        } else {
            None
        };
        let (text, chips, caret) = if let Some(marker) = marker {
            if let Some((start, end)) = trigger_span(&value, cursor, marker) {
                let (cleared, kept) = replace_range(&value, &self.context_chips, start, end, "");
                (cleared, kept, start)
            } else {
                (value, self.context_chips.clone(), cursor)
            }
        } else {
            (value, self.context_chips.clone(), cursor)
        };
        let id = uuid::Uuid::new_v4().to_string();
        let (next, new_caret, kept) = insert_chip(&text, &chips, caret, id, kind);
        self.install_chips(&next, kept, new_caret, window, cx);
    }

    pub(super) fn install_chips(
        &mut self,
        text: &str,
        chips: Vec<ContextChip>,
        caret: usize,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.chip_guard = true;
        self.context_chips = chips;
        self.chip_draft = text.to_string();
        let caret = caret.min(text.len());
        self.input_state.update(cx, |input, cx| {
            input.set_value(text.to_string(), window, cx);
            input.set_selected_range(caret..caret, cx);
            input.focus(window, cx);
        });
        self.composer_hint_selected_index = 0;
        self.composer_hints_suppressed_value = None;
        self.clear_mention_hints();
        self.clear_pull_hints();
        self.clear_skill_hints();
        cx.notify();
    }

    pub(super) fn retarget_chips(&mut self, cx: &App) {
        let next = self.input_state.read(cx).value().to_string();
        if self.chip_guard {
            self.chip_guard = false;
            self.chip_draft = next;
            return;
        }
        if next == self.chip_draft {
            return;
        }
        self.context_chips = bruhs_models::logic::context_chip::retarget(
            &self.context_chips,
            &self.chip_draft,
            &next,
        );
        self.chip_draft = next;
    }

    /// Backspace or forward delete removes a whole chip under the caret.
    pub(super) fn remove_chip_at_caret(
        &mut self,
        backspace: bool,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> bool {
        use bruhs_models::logic::context_chip::{backspace_chip, delete_forward_chip};

        if !self.input_state.focus_handle(cx).is_focused(window) {
            return false;
        }
        let (value, range) = {
            let input = self.input_state.read(cx);
            (input.value().to_string(), input.selected_range())
        };
        if range.start != range.end {
            return false;
        }
        let removed = if backspace {
            backspace_chip(&value, &self.context_chips, range.start)
        } else {
            delete_forward_chip(&value, &self.context_chips, range.start)
        };
        let Some((next, caret, chips)) = removed else {
            return false;
        };
        self.install_chips(&next, chips, caret, window, cx);
        true
    }

    pub(super) fn render_file_mention_hints(&self, cx: &Context<'_, Self>) -> AnyElement {
        if self.mention_query.is_none() {
            return div().into_any_element();
        }

        let thread_count = self.thread_hits.len();
        let rows = if self.thread_hits.is_empty() && self.mention_items.is_empty() {
            vec![div()
                .px_3()
                .py_2()
                .text_size(px(COMPOSER_HINT_SUB_TEXT_SIZE_PX))
                .font_family(theme::UI_SMALL_FONT_FAMILY)
                .font_weight(FontWeight::MEDIUM)
                .text_color(theme::rambutan_500())
                .child(COMPOSER_HINT_EMPTY_LABEL)
                .into_any_element()]
        } else {
            let selected_index = clamp_composer_hint_index(
                self.composer_hint_selected_index,
                thread_count + self.mention_items.len(),
            );
            let mut rows = self
                .thread_hits
                .iter()
                .enumerate()
                .map(|(index, hit)| {
                    let id = hit.id.clone();
                    let title = hit.title.clone();
                    let title_label = title.clone();
                    let is_selected = index == selected_index;
                    div()
                        .id(("desktop-thread-mention-row", element_key(&id)))
                        .w_full()
                        .flex()
                        .items_center()
                        .gap_2()
                        .px_3()
                        .py_1p5()
                        .text_size(px(COMPOSER_HINT_ROW_TEXT_SIZE_PX))
                        .text_color(if is_selected {
                            theme::rambutan_100()
                        } else {
                            theme::rambutan_300()
                        })
                        .cursor_pointer()
                        .when(is_selected, |el| el.bg(theme::longan_800()))
                        .hover(|el| el.bg(theme::longan_800()).text_color(theme::rambutan_100()))
                        .on_hover(cx.listener(move |this, hovered, _window, cx| {
                            if select_composer_hint_index_on_hover(
                                &mut this.composer_hint_selected_index,
                                *hovered,
                                index,
                            ) {
                                cx.notify();
                            }
                        }))
                        .on_click(cx.listener(move |this, _ev, window, cx| {
                            let id = id.clone();
                            let title = title.clone();
                            this.insert_context_chip(
                                ChipKind::Thread {
                                    thread_id: id,
                                    title,
                                },
                                window,
                                cx,
                            );
                        }))
                        .child(
                            div()
                                .w(px(COMPOSER_HINT_ICON_SLOT_WIDTH_PX))
                                .flex_shrink_0()
                                .text_align(TextAlign::Center)
                                .text_color(theme::rambutan_500())
                                .child("@"),
                        )
                        .child(
                            div()
                                .min_w_0()
                                .flex_1()
                                .truncate()
                                .font_weight(FontWeight::MEDIUM)
                                .child(title_label),
                        )
                        .into_any_element()
                })
                .collect::<Vec<_>>();
            rows.extend(self.mention_items.iter().enumerate().map(|(index, item)| {
                let index = thread_count + index;
                let path = item.path.clone();
                let debug_path = path.clone();
                let icon = desktop_mention_icon(item.is_dir);
                let is_selected = index == selected_index;
                div()
                    .id(("desktop-file-mention-row", element_key(&path)))
                    .debug_selector(move || {
                        format!("desktop-file-mention-row-{}", element_key(&debug_path))
                    })
                    .w_full()
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_3()
                    .py_1p5()
                    .text_size(px(COMPOSER_HINT_ROW_TEXT_SIZE_PX))
                    .text_color(if is_selected {
                        theme::rambutan_100()
                    } else {
                        theme::rambutan_300()
                    })
                    .cursor_pointer()
                    .when(is_selected, |el| el.bg(theme::longan_800()))
                    .hover(|el| el.bg(theme::longan_800()).text_color(theme::rambutan_100()))
                    .on_hover(cx.listener(move |this, hovered, _window, cx| {
                        if select_composer_hint_index_on_hover(
                            &mut this.composer_hint_selected_index,
                            *hovered,
                            index,
                        ) {
                            cx.notify();
                        }
                    }))
                    .on_click(cx.listener(move |this, _ev, window, cx| {
                        this.insert_file_mention(&path, window, cx);
                    }))
                    .child(
                        div()
                            .w(px(COMPOSER_HINT_ICON_SLOT_WIDTH_PX))
                            .flex_shrink_0()
                            .text_align(TextAlign::Center)
                            .text_color(theme::rambutan_500())
                            .child(icon),
                    )
                    .child(
                        div()
                            .min_w_0()
                            .flex_1()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .min_w_0()
                                    .flex_1()
                                    .truncate()
                                    .font_weight(FontWeight::MEDIUM)
                                    .child(item.name.clone()),
                            )
                            .child(
                                div()
                                    .min_w_0()
                                    .truncate()
                                    .text_size(px(COMPOSER_HINT_SUB_TEXT_SIZE_PX))
                                    .font_family(theme::UI_SMALL_FONT_FAMILY)
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(theme::rambutan_500())
                                    .child(item.path.clone()),
                            ),
                    )
                    .into_any_element()
            }));
            rows
        };

        div()
            .debug_selector(|| "desktop-file-mention-hints".to_string())
            .mb(px(COMPOSER_HINT_POPOVER_OFFSET_PX))
            .rounded(px(COMPOSER_HINT_POPOVER_RADIUS_PX))
            .border_1()
            .border_color(theme::ring_floating())
            .bg(theme::longan_700())
            .py_1()
            .max_h(px(COMPOSER_HINT_POPOVER_MAX_HEIGHT_PX))
            .overflow_y_scrollbar()
            .children(rows)
            .into_any_element()
    }

    pub(super) fn render_skill_hints(&self, cx: &Context<'_, Self>) -> AnyElement {
        if self.skill_query.is_none() {
            return div().into_any_element();
        }
        let rows = if self.skill_items.is_empty() {
            let message = self
                .skill_error
                .clone()
                .unwrap_or_else(|| COMPOSER_HINT_EMPTY_LABEL.to_string());
            let color = if self.skill_error.is_some() {
                theme::rambutan_400()
            } else {
                theme::rambutan_500()
            };
            vec![div()
                .px_3()
                .py_2()
                .text_size(px(COMPOSER_HINT_SUB_TEXT_SIZE_PX))
                .font_family(theme::UI_SMALL_FONT_FAMILY)
                .font_weight(FontWeight::MEDIUM)
                .text_color(color)
                .child(message)
                .into_any_element()]
        } else {
            let selected_index = clamp_composer_hint_index(
                self.composer_hint_selected_index,
                self.skill_items.len(),
            );
            self.skill_items
                .iter()
                .enumerate()
                .map(|(index, skill)| {
                    let chosen = skill.name.clone();
                    let is_selected = index == selected_index;
                    let label = format!("${}", skill.name);
                    let description = skill.description.clone();
                    div()
                        .id(("desktop-skill-row", element_key(&skill.name)))
                        .w_full()
                        .flex()
                        .items_center()
                        .px_3()
                        .py_1p5()
                        .text_size(px(COMPOSER_HINT_ROW_TEXT_SIZE_PX))
                        .text_color(if is_selected {
                            theme::rambutan_100()
                        } else {
                            theme::rambutan_300()
                        })
                        .cursor_pointer()
                        .when(is_selected, |el| el.bg(theme::longan_800()))
                        .hover(|el| el.bg(theme::longan_800()).text_color(theme::rambutan_100()))
                        .on_hover(cx.listener(move |this, hovered, _window, cx| {
                            if select_composer_hint_index_on_hover(
                                &mut this.composer_hint_selected_index,
                                *hovered,
                                index,
                            ) {
                                cx.notify();
                            }
                        }))
                        .on_click(cx.listener(move |this, _ev, window, cx| {
                            this.insert_skill_mention(&chosen, window, cx);
                        }))
                        .child(
                            div()
                                .min_w_0()
                                .flex_1()
                                .flex()
                                .items_center()
                                .gap_2()
                                .child(
                                    div()
                                        .flex_shrink_0()
                                        .font_weight(FontWeight::MEDIUM)
                                        .child(label),
                                )
                                .child(
                                    div()
                                        .min_w_0()
                                        .flex_1()
                                        .truncate()
                                        .text_size(px(COMPOSER_HINT_SUB_TEXT_SIZE_PX))
                                        .font_family(theme::UI_SMALL_FONT_FAMILY)
                                        .font_weight(FontWeight::MEDIUM)
                                        .text_color(theme::rambutan_500())
                                        .child(description),
                                ),
                        )
                        .into_any_element()
                })
                .collect::<Vec<_>>()
        };
        div()
            .debug_selector(|| "desktop-skill-hints".to_string())
            .mb(px(COMPOSER_HINT_POPOVER_OFFSET_PX))
            .rounded(px(COMPOSER_HINT_POPOVER_RADIUS_PX))
            .border_1()
            .border_color(theme::ring_floating())
            .bg(theme::longan_700())
            .py_1()
            .max_h(px(COMPOSER_HINT_POPOVER_MAX_HEIGHT_PX))
            .overflow_y_scrollbar()
            .children(rows)
            .into_any_element()
    }

    pub(super) fn render_pull_hints(&self, cx: &Context<'_, Self>) -> AnyElement {
        if self.pull_query.is_none() {
            return div().into_any_element();
        }
        let rows = if self.pull_items.is_empty() {
            let message = self
                .pull_error
                .clone()
                .unwrap_or_else(|| COMPOSER_HINT_EMPTY_LABEL.to_string());
            let color = if self.pull_error.is_some() {
                theme::rambutan_400()
            } else {
                theme::rambutan_500()
            };
            vec![div()
                .px_3()
                .py_2()
                .text_size(px(COMPOSER_HINT_SUB_TEXT_SIZE_PX))
                .font_family(theme::UI_SMALL_FONT_FAMILY)
                .font_weight(FontWeight::MEDIUM)
                .text_color(color)
                .child(message)
                .into_any_element()]
        } else {
            let selected_index =
                clamp_composer_hint_index(self.composer_hint_selected_index, self.pull_items.len());
            self.pull_items
                .iter()
                .enumerate()
                .map(|(index, pull)| {
                    let chosen = pull.clone();
                    let is_selected = index == selected_index;
                    let label = format!("#{} {}", pull.number, pull.title);
                    div()
                        .id(("desktop-pull-row", element_key(&pull.number.to_string())))
                        .w_full()
                        .flex()
                        .items_center()
                        .px_3()
                        .py_1p5()
                        .text_size(px(COMPOSER_HINT_ROW_TEXT_SIZE_PX))
                        .text_color(if is_selected {
                            theme::rambutan_100()
                        } else {
                            theme::rambutan_300()
                        })
                        .cursor_pointer()
                        .when(is_selected, |el| el.bg(theme::longan_800()))
                        .hover(|el| el.bg(theme::longan_800()).text_color(theme::rambutan_100()))
                        .on_hover(cx.listener(move |this, hovered, _window, cx| {
                            if select_composer_hint_index_on_hover(
                                &mut this.composer_hint_selected_index,
                                *hovered,
                                index,
                            ) {
                                cx.notify();
                            }
                        }))
                        .on_click(cx.listener(move |this, _ev, window, cx| {
                            let pull = chosen.clone();
                            this.insert_context_chip(
                                ChipKind::PullRequest {
                                    number: pull.number,
                                    title: pull.title,
                                    url: pull.url,
                                    state: pull.state,
                                },
                                window,
                                cx,
                            );
                        }))
                        .child(
                            div()
                                .min_w_0()
                                .flex_1()
                                .truncate()
                                .font_weight(FontWeight::MEDIUM)
                                .child(label),
                        )
                        .into_any_element()
                })
                .collect::<Vec<_>>()
        };
        div()
            .debug_selector(|| "desktop-pull-hints".to_string())
            .mb(px(COMPOSER_HINT_POPOVER_OFFSET_PX))
            .rounded(px(COMPOSER_HINT_POPOVER_RADIUS_PX))
            .border_1()
            .border_color(theme::ring_floating())
            .bg(theme::longan_700())
            .py_1()
            .max_h(px(COMPOSER_HINT_POPOVER_MAX_HEIGHT_PX))
            .overflow_y_scrollbar()
            .children(rows)
            .into_any_element()
    }
}

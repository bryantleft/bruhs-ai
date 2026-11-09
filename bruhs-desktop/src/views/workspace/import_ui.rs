//! Import thread: pick a thread from Claude Code, Codex, T3 Code or Cursor
//! and continue it here, plus the palette's ways back out (resume command,
//! Markdown, a new T3 thread). Matches the web's `import_thread_dialog`.
//! See `bruhs_models::logic::portable` for what moves and how.
//!
//! Listing, loading and exporting read other tools' files or run their
//! CLIs, so all of it runs on the background executor.

use super::*;
use bruhs_core::import::ImportRoots;
use bruhs_models::agent::Role as AgentRole;
use bruhs_models::logic::portable::{
    import_model, resume_command, transcript_markdown, ImportCandidate, ImportSource,
    ImportedThread, PortableMessage, PortablePart, PortableToolCall, IMPORT_CONTEXT_MAX_CHARS,
};
use bruhs_models::time::{format_relative_ms, RelativeTimeStyle};
use gpui_component::Sizable as _;
use std::time::Duration;

pub(super) const IMPORT_DIALOG_LABEL: &str = "Import thread";
pub(super) const IMPORT_SEARCH_PLACEHOLDER: &str = "Search titles and folders";
pub(super) const IMPORT_FOOTER_HINT: &str = "↑↓ to choose, enter to import";
pub(super) const IMPORT_FOOTER_IMPORTING: &str = "Importing…";
pub(super) const IMPORT_LOADING_LABEL: &str = "Loading…";
pub(super) const IMPORT_NO_SESSION_NOTICE: &str = "This thread has no session to resume yet.";
pub(super) const IMPORT_T3_NEEDS_PROJECT_NOTICE: &str =
    "Continue in T3 Code needs a thread with a project.";
/// How many candidates one listing asks for.
pub(super) const IMPORT_LIST_LIMIT: usize = 100;
/// Typing waits this long before searching again.
pub(super) const IMPORT_SEARCH_DEBOUNCE: Duration = Duration::from_millis(200);
/// Web `max-w-lg`.
pub(super) const IMPORT_DIALOG_WIDTH_PX: f32 = 512.0;
/// Web `max-h-80`.
pub(super) const IMPORT_LIST_MAX_HEIGHT_PX: f32 = 320.0;
/// Web `text-label-sm` / `text-label`.
pub(super) const IMPORT_LABEL_SM_TEXT_SIZE_PX: f32 = 12.0;
pub(super) const IMPORT_LABEL_TEXT_SIZE_PX: f32 = 14.0;
/// Web `rounded-grape`.
pub(super) const IMPORT_RADIUS_PX: f32 = 8.0;
pub(super) const IMPORT_SELECTED_ROW_BG_OPACITY: f32 = 0.60;
pub(super) const IMPORT_HOVER_ROW_BG_OPACITY: f32 = 0.40;
pub(super) const IMPORT_TAB_SELECTED_BG_OPACITY: f32 = 0.20;
pub(super) const IMPORT_TAB_SELECTED_RING_OPACITY: f32 = 0.30;
pub(super) const IMPORT_TAB_HOVER_BG_OPACITY: f32 = 0.50;
pub(super) const IMPORT_FOOTER_BORDER_OPACITY: f32 = 0.50;

/// The dialog's state. Closing it leaves the last listing; opening it
/// starts over.
pub(super) struct ImportPicker {
    pub(super) source: ImportSource,
    /// The newest listing, newest thread first.
    pub(super) found: Vec<ImportCandidate>,
    pub(super) selected: usize,
    pub(super) loading: bool,
    pub(super) importing: bool,
    pub(super) error: Option<String>,
    scroll: ScrollHandle,
    /// The pending listing. Replacing it drops (cancels) the older one, so
    /// only the newest search fills the list.
    list_task: Option<Task<()>>,
    import_task: Option<Task<()>>,
}

impl Default for ImportPicker {
    fn default() -> Self {
        Self {
            source: ImportSource::ClaudeCode,
            found: Vec::new(),
            selected: 0,
            loading: false,
            importing: false,
            error: None,
            scroll: ScrollHandle::new(),
            list_task: None,
            import_task: None,
        }
    }
}

/// Where an imported thread lives here: its project name (empty for a chat
/// without a project) and the folder its turns run in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ImportPlace {
    pub(super) project_name: String,
    pub(super) run_path: Option<String>,
}

/// A thread that ran in a folder is a project thread there. One that ran in
/// the home directory is a chat without a project, as bruhs's own home
/// chats are, but it still runs in the home directory: Claude Code finds a
/// session by the folder it ran in.
pub(super) fn import_place(cwd: Option<&str>, home: Option<&str>) -> ImportPlace {
    let cwd = cwd
        .map(|cwd| match cwd.trim_end_matches('/') {
            "" => "/",
            trimmed => trimmed,
        })
        .filter(|cwd| !cwd.is_empty());
    let home = home.map(|home| home.trim_end_matches('/'));
    let project_name = match cwd {
        Some(cwd) if Some(cwd) != home && cwd != "/" => {
            cwd.rsplit('/').next().unwrap_or_default().to_string()
        }
        _ => String::new(),
    };
    ImportPlace {
        project_name,
        run_path: cwd.map(str::to_string),
    }
}

/// `path` with the home directory shown as `~`.
pub(super) fn tilde_path(path: &str, home: Option<&str>) -> String {
    let Some(home) = home.map(|home| home.trim_end_matches('/')) else {
        return path.to_string();
    };
    if home.is_empty() {
        return path.to_string();
    }
    match path.strip_prefix(home) {
        Some(rest) if rest.is_empty() || rest.starts_with('/') => format!("~{rest}"),
        _ => path.to_string(),
    }
}

/// The second line of a row: where the thread ran, how long ago, and
/// whether it resumes or comes in as a transcript.
pub(super) fn import_row_detail(
    candidate: &ImportCandidate,
    home: Option<&str>,
    now: EpochMs,
) -> String {
    let place = candidate
        .cwd
        .as_deref()
        .map_or_else(|| "no folder".to_string(), |cwd| tilde_path(cwd, home));
    let when = format_relative_ms(candidate.updated_at, now, RelativeTimeStyle::Compact);
    let how = if candidate.session_id.is_some() {
        "resumes"
    } else {
        "transcript"
    };
    format!("{place} · {when} · {how}")
}

/// Candidates whose session is not already a thread here.
pub(super) fn visible_candidates<'a>(
    found: &'a [ImportCandidate],
    known_sessions: &HashSet<String>,
) -> Vec<&'a ImportCandidate> {
    found
        .iter()
        .filter(|candidate| {
            candidate
                .session_id
                .as_ref()
                .is_none_or(|session| !known_sessions.contains(session))
        })
        .collect()
}

/// An imported message as a desktop message: text parts become the content,
/// thinking parts the thinking, tool calls the tool rows. Empty messages are
/// dropped.
pub(super) fn imported_message(message: &PortableMessage) -> Option<Message> {
    let content = message.text();
    let mut thinking = Vec::new();
    let mut tool_calls = Vec::new();
    for part in &message.parts {
        match part {
            PortablePart::Text(_) => {}
            PortablePart::Thinking(text) if !text.trim().is_empty() => {
                thinking.push(text.trim().to_string());
            }
            PortablePart::Thinking(_) => {}
            PortablePart::ToolCall(call) => tool_calls.push(DesktopToolCall {
                id: call.id.clone(),
                tool: call.tool.clone(),
                input: call.input.clone(),
                output: call.output.clone(),
                is_error: call.is_error,
            }),
        }
    }
    let role = if message.role == AgentRole::User {
        MessageRole::User
    } else {
        MessageRole::Assistant
    };
    if role == MessageRole::User {
        thinking.clear();
        tool_calls.clear();
    }
    if content.is_empty() && thinking.is_empty() && tool_calls.is_empty() {
        return None;
    }
    Some(Message {
        role,
        content,
        model: message.model.clone(),
        thinking: thinking.join("\n\n"),
        usage: None,
        duration_ms: None,
        started_at_ms: message.started_at_ms,
        completed_at_ms: None,
        tool_calls,
        files: Vec::new(),
        state: TurnState::Finished,
        checkpoint: None,
    })
}

pub(super) fn imported_messages(messages: &[PortableMessage]) -> Vec<Message> {
    messages.iter().filter_map(imported_message).collect()
}

/// A thread's messages in the shape every exporter reads. Desktop messages
/// keep thinking, tool calls and text apart, so they go out in that order.
/// Turns still streaming or that failed are left out.
pub(crate) fn portable_messages(messages: &[Message]) -> Vec<PortableMessage> {
    messages
        .iter()
        .filter(|message| matches!(message.role, MessageRole::User | MessageRole::Assistant))
        .filter(|message| matches!(message.state, TurnState::Finished | TurnState::Stopped))
        .map(|message| {
            let mut parts = Vec::new();
            if !message.thinking.trim().is_empty() {
                parts.push(PortablePart::Thinking(message.thinking.clone()));
            }
            parts.extend(message.tool_calls.iter().map(|call| {
                PortablePart::ToolCall(PortableToolCall {
                    id: call.id.clone(),
                    tool: call.tool.clone(),
                    input: call.input.clone(),
                    output: call.output.clone(),
                    is_error: call.is_error,
                })
            }));
            if !message.content.trim().is_empty() {
                parts.push(PortablePart::Text(message.content.clone()));
            }
            PortableMessage {
                role: message.role,
                parts,
                started_at_ms: message.started_at_ms,
                model: message.model.clone(),
            }
        })
        .filter(|message| !message.parts.is_empty())
        .collect()
}

impl Workspace {
    /// The search field: typing searches again after a short pause.
    pub(super) fn new_import_input(
        window: &mut Window,
        cx: &mut Context<'_, Self>,
        subscriptions: &mut Vec<Subscription>,
    ) -> Entity<InputState> {
        let input = cx.new(|cx| InputState::new(window, cx).placeholder(IMPORT_SEARCH_PLACEHOLDER));
        subscriptions.push(cx.subscribe_in(
            &input,
            window,
            |this: &mut Self, _input, ev: &InputEvent, _window, cx| {
                if matches!(ev, InputEvent::Change)
                    && matches!(this.top_layer, Some(TopLayer::ImportThread))
                {
                    this.refresh_import_list(cx);
                }
            },
        ));
        input
    }

    pub(super) fn open_import_thread(
        &mut self,
        _: &ImportThread,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.open_top_layer(TopLayer::ImportThread);
        self.import_picker.selected = 0;
        self.import_picker.error = None;
        self.import_picker.importing = false;
        self.import_picker.import_task = None;
        let input = self.import_input.clone();
        input.update(cx, |input, cx| input.set_value("", window, cx));
        // Setting the value searches through the subscription; an already
        // empty field does not change, so search here as well.
        self.refresh_import_list(cx);
        cx.defer_in(window, move |_this, window, cx| {
            input.update(cx, |input, cx| input.focus(window, cx));
        });
        cx.notify();
    }

    /// List the picked source for the current query, 200ms after typing
    /// stops (at once for an empty query).
    pub(super) fn refresh_import_list(&mut self, cx: &mut Context<'_, Self>) {
        let source = self.import_picker.source;
        let query = self.import_input.read(cx).value().trim().to_string();
        let delay = (!query.is_empty()).then_some(IMPORT_SEARCH_DEBOUNCE);
        self.import_picker.loading = true;
        let executor = cx.background_executor().clone();
        let listing = cx.background_spawn(async move {
            if let Some(delay) = delay {
                executor.timer(delay).await;
            }
            bruhs_core::import::list(
                &ImportRoots::from_env(),
                source,
                Some(&query),
                Some(IMPORT_LIST_LIMIT),
            )
            .map_err(|error| error.to_string())
        });
        self.import_picker.list_task = Some(cx.spawn(async move |this, cx| {
            let listed = listing.await;
            // The workspace may have closed meanwhile; nothing to show then.
            let _closed = this.update(cx, |this, cx| {
                this.apply_import_list(listed);
                cx.notify();
            });
        }));
        cx.notify();
    }

    pub(super) fn apply_import_list(&mut self, listed: Result<Vec<ImportCandidate>, String>) {
        let picker = &mut self.import_picker;
        match listed {
            Ok(rows) => {
                picker.found = rows;
                picker.error = None;
            }
            Err(error) => {
                picker.found = Vec::new();
                picker.error = Some(error);
            }
        }
        picker.selected = 0;
        picker.loading = false;
        picker.scroll.set_offset(Point::default());
    }

    pub(super) fn set_import_source(&mut self, source: ImportSource, cx: &mut Context<'_, Self>) {
        if self.import_picker.source == source {
            return;
        }
        self.import_picker.source = source;
        // Rows of the previous source must not be imported by a quick Enter.
        self.import_picker.found.clear();
        self.import_picker.error = None;
        self.refresh_import_list(cx);
    }

    /// Provider sessions the threads here already continue.
    pub(super) fn known_session_ids(&self, cx: &App) -> HashSet<String> {
        self.threads
            .iter()
            .filter_map(|thread| thread.chat.read(cx).session_id.clone())
            .collect()
    }

    pub(super) fn visible_import_candidates(&self, cx: &App) -> Vec<ImportCandidate> {
        let known = self.known_session_ids(cx);
        visible_candidates(&self.import_picker.found, &known)
            .into_iter()
            .cloned()
            .collect()
    }

    fn move_import_selection(&mut self, key: &str, cx: &mut Context<'_, Self>) {
        let count = self.visible_import_candidates(cx).len();
        if let Some(next) =
            Self::move_command_selected_index(self.import_picker.selected, count, key)
        {
            self.import_picker.selected = next;
            self.import_picker.scroll.scroll_to_item(next);
            cx.notify();
        }
    }

    fn import_selected(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let rows = self.visible_import_candidates(cx);
        let index = Self::clamp_command_selected_index(self.import_picker.selected, rows.len());
        if let Some(candidate) = rows.into_iter().nth(index) {
            self.import_candidate(candidate, window, cx);
        }
    }

    /// Read `candidate`'s messages in the background, then open it here.
    pub(super) fn import_candidate(
        &mut self,
        candidate: ImportCandidate,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if self.import_picker.importing {
            return;
        }
        self.import_picker.importing = true;
        self.import_picker.error = None;
        let load = cx.background_spawn(async move {
            bruhs_core::import::load(&ImportRoots::from_env(), candidate.source, &candidate.id)
                .map_err(|error| error.to_string())
        });
        self.import_picker.import_task = Some(cx.spawn_in(window, async move |this, cx| {
            let loaded = load.await;
            let _closed = this.update_in(cx, |this, window, cx| {
                this.import_picker.importing = false;
                match loaded {
                    Ok(imported) => this.open_imported_thread(imported, window, cx),
                    Err(error) => {
                        this.import_picker.error = Some(format!("Could not import: {error}"));
                        cx.notify();
                    }
                }
            });
        }));
        cx.notify();
    }

    /// Make `imported` a thread here and open it.
    pub(super) fn open_imported_thread(
        &mut self,
        imported: ImportedThread,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let candidate = &imported.candidate;
        let home = home_dir_path().map(|home| path_to_string(&home));
        let place = import_place(candidate.cwd.as_deref(), home.as_deref());
        let title = if candidate.title.trim().is_empty() {
            threads_ui::THREAD_TITLE_GENERAL.to_string()
        } else {
            candidate.title.clone()
        };
        let thread = Self::create_thread_with_title(
            place.project_name,
            place.run_path.unwrap_or_else(current_dir_string),
            // The thread stays on its session's provider.
            import_model(candidate),
            self.permission_mode,
            &title,
            window,
            cx,
        );
        self.stamp_chat_run_prefs(&thread.chat, cx);
        let messages = imported_messages(&imported.messages);
        let session = candidate
            .session_id
            .clone()
            .zip(candidate.provider)
            .filter(|(id, _)| !id.is_empty());
        let context = imported.context.clone();
        thread.chat.update(cx, |chat, cx| {
            chat.messages = messages;
            chat.session_id = session.as_ref().map(|(id, _)| id.clone());
            chat.session_provider = session.as_ref().map(|(_, provider)| *provider);
            chat.import_context = context;
            // A resumed session lives in the folder it ran in: a new
            // worktree would hide it from the CLI.
            chat.run_locked = session.is_some();
            chat.scroll_messages_to_bottom();
            cx.notify();
        });
        let id = thread.id.clone();
        let label = candidate.source.label();
        self.threads.push(thread);
        self.select_thread_by_id(&id, cx);
        self.meetings_hub.update(cx, |hub, cx| {
            hub.show_notice(format!("Imported from {label}"), false, cx);
        });
    }

    fn show_thread_notice(&self, message: impl Into<String>, warning: bool, cx: &mut App) {
        let message = message.into();
        self.meetings_hub
            .update(cx, |hub, cx| hub.show_notice(message, warning, cx));
    }

    /// Copy the shell command that continues the open thread's session in
    /// its own CLI.
    pub(super) fn copy_resume_command(&mut self, cx: &mut Context<'_, Self>) {
        let Some(thread) = self.active_thread() else {
            return;
        };
        let chat = thread.chat.read(cx);
        let command =
            chat.session_id
                .as_deref()
                .zip(chat.session_provider)
                .map(|(session, provider)| {
                    resume_command(provider, session, Some(chat.project_path.as_str()))
                });
        match command {
            Some(command) => {
                cx.write_to_clipboard(ClipboardItem::new_string(command));
                self.show_thread_notice("Resume command copied", false, cx);
            }
            None => self.show_thread_notice(IMPORT_NO_SESSION_NOTICE, true, cx),
        }
    }

    /// The open thread's title, project, model and Markdown transcript.
    fn active_thread_markdown(&self, cx: &App) -> Option<(String, String, String, String)> {
        let thread = self.active_thread()?;
        let chat = thread.chat.read(cx);
        let markdown = transcript_markdown(
            &chat.thread_title,
            "bruhs",
            &portable_messages(&chat.messages),
            IMPORT_CONTEXT_MAX_CHARS,
        );
        let project = if thread.project_name.is_empty() {
            String::new()
        } else {
            thread.project_path.clone()
        };
        Some((
            chat.thread_title.clone(),
            project,
            chat.model_id.clone(),
            markdown,
        ))
    }

    pub(super) fn copy_thread_markdown(&mut self, cx: &mut Context<'_, Self>) {
        let Some((_, _, _, markdown)) = self.active_thread_markdown(cx) else {
            return;
        };
        cx.write_to_clipboard(ClipboardItem::new_string(markdown));
        self.show_thread_notice("Thread copied as Markdown", false, cx);
    }

    /// Start a T3 Code thread in this thread's project that carries its
    /// transcript. The T3 CLI runs in the background.
    pub(super) fn continue_in_t3(&mut self, cx: &mut Context<'_, Self>) {
        let Some((title, project, model, markdown)) = self.active_thread_markdown(cx) else {
            return;
        };
        if project.is_empty() {
            self.show_thread_notice(IMPORT_T3_NEEDS_PROJECT_NOTICE, true, cx);
            return;
        }
        let export = cx.background_spawn(async move {
            bruhs_core::import::export_to_t3(Path::new(&project), &title, &model, &markdown)
                .map_err(|error| error.to_string())
        });
        cx.spawn(async move |this, cx| {
            let exported = export.await;
            let _closed = this.update(cx, |this, cx| match exported {
                Ok(_) => this.show_thread_notice("Started a T3 thread", false, cx),
                Err(error) => this.show_thread_notice(format!("T3: {error}"), true, cx),
            });
        })
        .detach();
    }

    pub(super) fn copy_resume_command_action(
        &mut self,
        _: &CopyResumeCommand,
        _window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.copy_resume_command(cx);
    }

    pub(super) fn copy_thread_markdown_action(
        &mut self,
        _: &CopyThreadMarkdown,
        _window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.copy_thread_markdown(cx);
    }

    pub(super) fn continue_in_t3_action(
        &mut self,
        _: &ContinueInT3,
        _window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.continue_in_t3(cx);
    }

    fn render_import_source_tabs(&self, cx: &Context<'_, Self>) -> AnyElement {
        let current = self.import_picker.source;
        div()
            .mt_2()
            .flex()
            .gap_1()
            .id("desktop-import-sources")
            .role(Role::TabList)
            .aria_label("Import from")
            .children(
                ImportSource::ALL
                    .into_iter()
                    .enumerate()
                    .map(|(index, tab)| {
                        let selected = tab == current;
                        div()
                            .id(("desktop-import-source", index))
                            .debug_selector(move || {
                                format!("desktop-import-source-{}", tab.as_str())
                            })
                            .role(Role::Tab)
                            .aria_label(tab.label())
                            .aria_selected(selected)
                            .tab_index(0)
                            .rounded(px(IMPORT_RADIUS_PX))
                            .border_1()
                            .px(px(10.0))
                            .py_1()
                            .font_family(theme::UI_SMALL_FONT_FAMILY)
                            .font_weight(FontWeight::MEDIUM)
                            .text_size(px(IMPORT_LABEL_SM_TEXT_SIZE_PX))
                            .cursor_pointer()
                            .focus_visible(|el| el.border_color(theme::persimmon_400()))
                            .map(|el| {
                                if selected {
                                    el.bg(theme::blueberry_600()
                                        .opacity(IMPORT_TAB_SELECTED_BG_OPACITY))
                                        .border_color(
                                            theme::blueberry_500()
                                                .opacity(IMPORT_TAB_SELECTED_RING_OPACITY),
                                        )
                                        .text_color(theme::blueberry_200())
                                } else {
                                    el.border_color(gpui::transparent_black())
                                        .text_color(theme::rambutan_400())
                                        .hover(|el| {
                                            el.bg(theme::longan_800()
                                                .opacity(IMPORT_TAB_HOVER_BG_OPACITY))
                                                .text_color(theme::rambutan_200())
                                        })
                                }
                            })
                            .on_click(cx.listener(move |this, _ev, _window, cx| {
                                this.set_import_source(tab, cx);
                            }))
                            .on_key_down(cx.listener(
                                move |this, event: &KeyDownEvent, window, cx| {
                                    if is_web_button_activation_key(event.keystroke.key.as_str()) {
                                        window.prevent_default();
                                        cx.stop_propagation();
                                        this.set_import_source(tab, cx);
                                    }
                                },
                            ))
                            .child(tab.label())
                    }),
            )
            .into_any_element()
    }

    fn render_import_rows(&self, cx: &Context<'_, Self>) -> AnyElement {
        let picker = &self.import_picker;
        let message_row = |text: String, color: Hsla| {
            div()
                .px_4()
                .py_6()
                .text_center()
                .font_family(theme::UI_SMALL_FONT_FAMILY)
                .font_weight(FontWeight::MEDIUM)
                .text_size(px(IMPORT_LABEL_SM_TEXT_SIZE_PX))
                .text_color(color)
                .child(text)
                .into_any_element()
        };
        if let Some(error) = picker.error.clone() {
            return message_row(error, theme::lychee_400());
        }
        let rows = self.visible_import_candidates(cx);
        if rows.is_empty() {
            let text = if picker.loading {
                IMPORT_LOADING_LABEL.to_string()
            } else {
                format!("No {} threads found", picker.source.label())
            };
            return message_row(text, theme::rambutan_300());
        }
        let home = home_dir_path().map(|home| path_to_string(&home));
        let now = EpochMs::now();
        let selected = picker.selected.min(rows.len() - 1);
        div()
            .id("desktop-import-list")
            .debug_selector(|| "desktop-import-list".to_string())
            .role(Role::ListBox)
            .aria_label("Threads")
            .max_h(px(IMPORT_LIST_MAX_HEIGHT_PX))
            .py_1()
            .overflow_y_scroll()
            .track_scroll(&picker.scroll)
            .children(rows.into_iter().enumerate().map(|(index, row)| {
                let chosen = index == selected;
                let detail = import_row_detail(&row, home.as_deref(), now);
                let title = row.title.clone();
                let debug_key = element_key(&row.id);
                div()
                    .id(("desktop-import-row", index))
                    .debug_selector(move || format!("desktop-import-row-{debug_key}"))
                    .role(Role::ListBoxOption)
                    .aria_label(title.clone())
                    .aria_selected(chosen)
                    .w_full()
                    .flex()
                    .flex_col()
                    .items_start()
                    .gap_0p5()
                    .px_4()
                    .py_2()
                    .cursor_pointer()
                    .map(|el| {
                        if chosen {
                            el.bg(theme::longan_800().opacity(IMPORT_SELECTED_ROW_BG_OPACITY))
                        } else {
                            el.hover(|el| {
                                el.bg(theme::longan_800().opacity(IMPORT_HOVER_ROW_BG_OPACITY))
                            })
                        }
                    })
                    .on_hover(cx.listener(move |this, hovered, _window, cx| {
                        if *hovered && this.import_picker.selected != index {
                            this.import_picker.selected = index;
                            cx.notify();
                        }
                    }))
                    .on_click(cx.listener(move |this, _ev, window, cx| {
                        this.import_candidate(row.clone(), window, cx);
                    }))
                    .child(
                        div()
                            .w_full()
                            .min_w_0()
                            .truncate()
                            .font_family(theme::UI_SMALL_FONT_FAMILY)
                            .font_weight(FontWeight::MEDIUM)
                            .text_size(px(IMPORT_LABEL_SM_TEXT_SIZE_PX))
                            .text_color(theme::rambutan_200())
                            .child(title),
                    )
                    .child(
                        div()
                            .w_full()
                            .min_w_0()
                            .truncate()
                            .font_family(theme::UI_SMALL_FONT_FAMILY)
                            .font_weight(FontWeight::MEDIUM)
                            .text_size(px(IMPORT_LABEL_SM_TEXT_SIZE_PX))
                            .text_color(theme::rambutan_400())
                            .child(detail),
                    )
            }))
            .vertical_scrollbar(&picker.scroll)
            .into_any_element()
    }

    pub(super) fn render_import_thread(&self, cx: &Context<'_, Self>) -> AnyElement {
        let importing = self.import_picker.importing;
        div()
            .absolute()
            .top_0()
            .right_0()
            .bottom_0()
            .left_0()
            .occlude()
            .id("desktop-import-thread-overlay")
            .debug_selector(|| "desktop-import-thread-overlay".to_string())
            .bg(theme::longan_950().opacity(DESKTOP_MODAL_OVERLAY_OPACITY))
            .flex()
            .items_start()
            .justify_center()
            .pt(px(DESKTOP_DIALOG_TOP_PADDING_PX))
            .on_click(cx.listener(|this, _ev, _window, cx| {
                this.close_transient_surfaces();
                cx.notify();
            }))
            .child(
                div()
                    .id("desktop-import-thread-dialog")
                    .role(Role::Dialog)
                    .aria_label(IMPORT_DIALOG_LABEL)
                    .child(self.blur_scrim.backdrop())
                    .debug_selector(|| "desktop-import-thread-dialog".to_string())
                    .w(px(IMPORT_DIALOG_WIDTH_PX))
                    .max_w(relative(DESKTOP_DIALOG_MAX_WIDTH_RATIO))
                    .rounded(px(DESKTOP_DIALOG_RADIUS_PX))
                    .border_1()
                    .border_color(theme::ring_modal())
                    .bg(theme::longan_800())
                    .text_color(cx.theme().popover_foreground)
                    .flex()
                    .flex_col()
                    .overflow_hidden()
                    .track_focus(&self.modal_start_focus_handle)
                    .tab_group()
                    .capture_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                        let key = event.keystroke.key.as_str();
                        let navigates = Self::move_command_selected_index(0, 0, key).is_some();
                        if event.keystroke.modifiers != Modifiers::default()
                            || !(matches!(key, "escape" | "enter") || navigates)
                        {
                            return;
                        }
                        let composing = this.import_input.update(cx, |input, cx| {
                            input.marked_text_range(window, cx).is_some()
                        });
                        if composing {
                            if key == "enter" {
                                window.prevent_default();
                                cx.stop_propagation();
                            }
                            return;
                        }
                        window.prevent_default();
                        cx.stop_propagation();
                        match key {
                            "enter" => this.import_selected(window, cx),
                            "escape" => {
                                this.close_transient_surfaces();
                                cx.notify();
                            }
                            _ => this.move_import_selection(key, cx),
                        }
                    }))
                    .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                        this.trap_modal_tab_focus(event, window, cx);
                    }))
                    .on_click(cx.listener(|_this, _ev, _window, cx| {
                        cx.stop_propagation();
                    }))
                    .child(
                        div()
                            .px_4()
                            .py_3()
                            .border_b_1()
                            .border_color(theme::longan_800())
                            .child(
                                div()
                                    .id("desktop-import-title")
                                    .role(Role::Heading)
                                    .font_family(theme::UI_SMALL_FONT_FAMILY)
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_size(px(IMPORT_LABEL_SM_TEXT_SIZE_PX))
                                    .text_color(theme::rambutan_200())
                                    .child(IMPORT_DIALOG_LABEL),
                            )
                            .child(self.render_import_source_tabs(cx))
                            .child(
                                div()
                                    .mt_2()
                                    .w_full()
                                    .rounded(px(IMPORT_RADIUS_PX))
                                    .bg(theme::longan_900())
                                    .border_1()
                                    .border_color(theme::longan_700())
                                    .px(px(10.0))
                                    // A small (24px) input in 2px padding and a
                                    // 1px border: 30px, the web's py-1.5 field.
                                    .py(px(2.0))
                                    .child(
                                        Input::new(&self.import_input)
                                            .small()
                                            .aria_label("Search threads")
                                            .appearance(false)
                                            .bordered(false)
                                            .focus_bordered(false)
                                            .cleanable(false)
                                            .font_family(theme::UI_SMALL_FONT_FAMILY)
                                            .text_size(px(IMPORT_LABEL_SM_TEXT_SIZE_PX)),
                                    ),
                            ),
                    )
                    .child(self.render_import_rows(cx))
                    .child(
                        div()
                            .px_4()
                            .py(px(10.0))
                            .border_t_1()
                            .border_color(theme::longan_800().opacity(IMPORT_FOOTER_BORDER_OPACITY))
                            .flex()
                            .items_center()
                            .justify_between()
                            .font_family(theme::UI_FONT_FAMILY)
                            .font_weight(FontWeight::MEDIUM)
                            .text_size(px(IMPORT_LABEL_TEXT_SIZE_PX))
                            .text_color(theme::rambutan_300())
                            .child(if importing {
                                IMPORT_FOOTER_IMPORTING
                            } else {
                                IMPORT_FOOTER_HINT
                            })
                            .child(
                                div()
                                    .id("desktop-import-cancel")
                                    .debug_selector(|| "desktop-import-cancel".to_string())
                                    .flex_shrink_0()
                                    .text_color(theme::rambutan_500())
                                    .tab_index(0)
                                    .role(Role::Button)
                                    .aria_label("Cancel")
                                    .focus_visible(|el| {
                                        el.border_2().border_color(theme::persimmon_400())
                                    })
                                    .cursor_pointer()
                                    .hover(|el| el.text_color(theme::rambutan_300()))
                                    .on_click(cx.listener(|this, _ev, _window, cx| {
                                        this.close_transient_surfaces();
                                        cx.notify();
                                    }))
                                    .on_key_down(cx.listener(
                                        |this, event: &KeyDownEvent, window, cx| {
                                            if is_web_button_activation_key(
                                                event.keystroke.key.as_str(),
                                            ) {
                                                window.prevent_default();
                                                cx.stop_propagation();
                                                this.close_transient_surfaces();
                                                cx.notify();
                                            }
                                        },
                                    ))
                                    .child("Cancel"),
                            ),
                    )
                    .child(self.render_modal_focus_sentinel()),
            )
            .into_any_element()
    }
}

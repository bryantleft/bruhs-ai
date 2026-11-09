//! The right panel (tasks, diff, preview), the terminal drawer and page, and opening files in an editor.

use super::*;
use crate::app_state::spawn_tokio;

pub(super) fn tasks_panel_visibility_after_toggle(current_visible: bool) -> bool {
    !current_visible
}

/// `(tasks_visible, diff_visible, preview_visible)` after toggling
/// `surface`: opening a surface hides the others; toggling the showing
/// surface closes the panel.
pub(super) fn right_panel_after_toggle(
    tasks_visible: bool,
    diff_visible: bool,
    preview_visible: bool,
    surface: RightPanelSurface,
) -> (bool, bool, bool) {
    match surface {
        RightPanelSurface::Tasks => (!tasks_visible, false, false),
        RightPanelSurface::Diff => (false, !diff_visible, false),
        RightPanelSurface::Preview => (false, false, !preview_visible),
        RightPanelSurface::File => (tasks_visible, diff_visible, preview_visible),
        // Files and Pull have their own flags. Opening either closes these three.
        RightPanelSurface::Files | RightPanelSurface::Pull => (false, false, false),
    }
}

/// Which surface the panel body shows. The open file wins, then the branch
/// pull request, then Files, Diff, Preview, and Tasks.
pub(super) fn showing_surface(
    file: bool,
    pull: bool,
    files: bool,
    diff: bool,
    preview: bool,
) -> RightPanelSurface {
    if file {
        RightPanelSurface::File
    } else if pull {
        RightPanelSurface::Pull
    } else if files {
        RightPanelSurface::Files
    } else if diff {
        RightPanelSurface::Diff
    } else if preview {
        RightPanelSurface::Preview
    } else {
        RightPanelSurface::Tasks
    }
}
/// Web `terminal_page.rs`: `h-[52px]`, `text-label` title, `text-label-sm` hint.
pub(super) const TERMINAL_PAGE_HEADER_HEIGHT_PX: f32 = 52.0;
pub(super) const TERMINAL_PAGE_HEADER_TEXT_SIZE_PX: f32 = 14.0;
pub(super) const TERMINAL_PAGE_HEADER_HINT_SIZE_PX: f32 = 12.0;
pub(super) const TERMINAL_PAGE_TITLE: &str = "Terminal";
pub(super) const TERMINAL_PAGE_HINT: &str = "— shells on this machine, independent of any thread";

/// Web `clamp_panel_width`: the right panel stays between 360px and the
/// viewport minus the chat column's 360px.
pub(super) fn clamp_right_panel_width(requested: f32, viewport_width: f32) -> f32 {
    let max = viewport_width - CHAT_COLUMN_MIN_WIDTH_PX;
    if max <= RIGHT_PANEL_MIN_WIDTH_PX {
        return RIGHT_PANEL_MIN_WIDTH_PX;
    }
    requested.clamp(RIGHT_PANEL_MIN_WIDTH_PX, max)
}
/// Web right panel `h-[52px]` tab bar.
pub(super) const WORKSPACE_PANEL_TAB_BAR_HEIGHT_PX: f32 = 52.0;
pub(super) const WORKSPACE_PANEL_TAB_HEIGHT_PX: f32 = 28.0;
pub(super) const WORKSPACE_PANEL_TAB_TEXT_SIZE_PX: f32 = 12.0;
pub(super) const WORKSPACE_PANEL_TAB_ICON_SIZE_PX: f32 = 14.0;

impl Workspace {
    pub(super) fn right_panel_open(&self) -> bool {
        self.tasks_panel_visible
            || self.diff_panel_visible
            || self.preview_panel_visible
            || self.file_surface
            || self.files_panel_visible
            || self.pull_panel_visible
    }

    pub(super) fn set_preview_panel_visible(&mut self, visible: bool, cx: &mut Context<'_, Self>) {
        self.preview_panel_visible = visible;
        if visible {
            self.tasks_panel_visible = false;
            self.diff_panel_visible = false;
            self.file_surface = false;
            self.files_panel_visible = false;
            self.pull_panel_visible = false;
            self.last_panel_surface = RightPanelSurface::Preview;
        }
        self.sync_chat_panel_flags(cx);
        cx.notify();
    }

    /// ⌘⇧J: the Preview surface in the right panel.
    pub(super) fn toggle_preview(
        &mut self,
        _: &TogglePreview,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let (_, _, preview) = right_panel_after_toggle(
            self.tasks_panel_visible,
            self.diff_panel_visible,
            self.preview_panel_visible,
            RightPanelSurface::Preview,
        );
        if preview && self.active_view != AppView::Chat {
            self.open_chat_view(&OpenChatView, window, cx);
        }
        self.set_preview_panel_visible(preview, cx);
    }

    pub(super) fn sync_chat_panel_flags(&mut self, cx: &mut Context<'_, Self>) {
        let tasks = self.tasks_panel_visible;
        let diff = self.diff_panel_visible;
        let chats: Vec<_> = self
            .threads
            .iter()
            .map(|thread| thread.chat.clone())
            .collect();
        let preview = self.preview_panel_visible;
        let file_open = self.file_surface;
        for chat in chats {
            chat.update(cx, |chat, cx| {
                chat.tasks_panel_visible = tasks;
                chat.diff_panel_visible = diff;
                chat.right_panel_open = tasks
                    || diff
                    || preview
                    || file_open
                    || self.files_panel_visible
                    || self.pull_panel_visible;
                cx.notify();
            });
        }
    }

    /// Point the diff panel at the active thread's project and re-read git.
    pub(super) fn sync_diff_panel_project(&mut self, cx: &mut Context<'_, Self>) {
        let path = self.active_thread().and_then(|thread| {
            let chat = thread.chat.read(cx);
            // The chat's scoped project wins (the thread row may lag behind
            // a project picked mid-thread); otherwise the thread's.
            (chat.project_scoped && !chat.project_path.is_empty())
                .then(|| chat.project_path.clone())
                .or_else(|| (!thread.project_path.is_empty()).then(|| thread.project_path.clone()))
        });
        let turns = self.active_thread().map_or_else(Vec::new, |thread| {
            let chat = thread.chat.read(cx);
            turn_files_from_messages(&chat.messages, path.as_deref())
        });
        self.diff_panel.update(cx, |panel, cx| {
            panel.set_project(path, cx);
            panel.set_turns(turns, cx);
            panel.refresh(cx);
        });
    }

    /// Web `on_open_diff`: show the diff panel scoped to the turn that owns
    /// `message_index` (its position among the turns that changed files).
    pub(super) fn open_diff_for_turn(&mut self, message_index: usize, cx: &mut Context<'_, Self>) {
        let turn_index = self.active_thread().and_then(|thread| {
            let chat = thread.chat.read(cx);
            let root = if chat.project_scoped && !chat.project_path.is_empty() {
                Some(chat.project_path.as_str())
            } else {
                (!thread.project_path.is_empty()).then_some(thread.project_path.as_str())
            };
            let upto = chat.messages.get(..=message_index)?;
            turn_files_from_messages(upto, root).len().checked_sub(1)
        });
        if let Some(index) = turn_index {
            self.diff_panel
                .update(cx, |panel, cx| panel.select_turn(index, cx));
        }
        self.set_diff_panel_visible(true, cx);
    }

    pub(super) fn set_diff_panel_visible(&mut self, visible: bool, cx: &mut Context<'_, Self>) {
        self.diff_panel_visible = visible;
        if visible {
            self.tasks_panel_visible = false;
            self.preview_panel_visible = false;
            self.file_surface = false;
            self.files_panel_visible = false;
            self.pull_panel_visible = false;
            self.last_panel_surface = RightPanelSurface::Diff;
            self.sync_diff_panel_project(cx);
        }
        self.sync_chat_panel_flags(cx);
        cx.notify();
    }

    pub(super) fn toggle_diff(
        &mut self,
        _: &ToggleDiff,
        _window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let (_, diff, _) = right_panel_after_toggle(
            self.tasks_panel_visible,
            self.diff_panel_visible,
            self.preview_panel_visible,
            RightPanelSurface::Diff,
        );
        self.set_diff_panel_visible(diff, cx);
    }

    /// ⌘⌥B: close the panel, or reopen the surface it last showed.
    pub(super) fn toggle_panel(
        &mut self,
        _: &TogglePanel,
        _window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if self.right_panel_open() {
            self.tasks_panel_visible = false;
            self.diff_panel_visible = false;
            self.preview_panel_visible = false;
            self.file_surface = false;
            self.files_panel_visible = false;
            self.pull_panel_visible = false;
            self.sync_chat_panel_flags(cx);
            cx.notify();
        } else {
            match self.last_panel_surface {
                RightPanelSurface::Diff => self.set_diff_panel_visible(true, cx),
                RightPanelSurface::Tasks => self.set_tasks_panel_visible(true, cx),
                RightPanelSurface::Preview => self.set_preview_panel_visible(true, cx),
                RightPanelSurface::Files => self.set_files_panel_visible(true, cx),
                RightPanelSurface::Pull => {
                    if self.branch_pull.is_some() {
                        self.show_pull_surface(cx);
                    } else {
                        self.set_tasks_panel_visible(true, cx);
                    }
                }
                RightPanelSurface::File => {
                    if self.open_file.is_some() {
                        self.show_file_surface(cx);
                    } else {
                        self.set_tasks_panel_visible(true, cx);
                    }
                }
            }
        }
    }

    /// ⌘⇧M: the active thread's model popover.
    pub(super) fn toggle_model_picker(
        &mut self,
        _: &ToggleModelPicker,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let Some(chat) = self.active_thread().map(|thread| thread.chat.clone()) else {
            return;
        };
        chat.update(cx, |chat, cx| chat.toggle_model_picker(window, cx));
    }

    /// ⌘⇧X: where this thread, or the next one, runs.
    pub(super) fn toggle_workspace_menu(
        &mut self,
        _: &ToggleWorkspaceMenu,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let Some(chat) = self.active_thread().map(|thread| thread.chat.clone()) else {
            return;
        };
        chat.update(cx, |chat, cx| chat.toggle_workspace_menu(window, cx));
    }

    /// ⌘⇧G: the branch menu. On a new worktree it picks the base branch.
    pub(super) fn toggle_branch_menu(
        &mut self,
        _: &ToggleBranchMenu,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let Some(chat) = self.active_thread().map(|thread| thread.chat.clone()) else {
            return;
        };
        chat.update(cx, |chat, cx| chat.toggle_branch_menu(window, cx));
    }

    /// ⌘⇧E: Claude effort. Codex has no effort flag, so the chip stays hidden.
    pub(super) fn toggle_effort_menu(
        &mut self,
        _: &ToggleEffortMenu,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let Some(chat) = self.active_thread().map(|thread| thread.chat.clone()) else {
            return;
        };
        chat.update(cx, |chat, cx| chat.toggle_effort_menu(window, cx));
    }

    /// ⌘⇧L: run the next send in the previous worktree.
    pub(super) fn use_previous_worktree(
        &mut self,
        _: &UsePreviousWorktree,
        _window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let Some(chat) = self.active_thread().map(|thread| thread.chat.clone()) else {
            return;
        };
        chat.update(cx, |chat, cx| chat.use_previous_worktree(cx));
    }

    /// ⌘P: focus the composer with an `@`.
    pub(super) fn mention_file(
        &mut self,
        _: &MentionFile,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if self.active_view != AppView::Chat {
            self.active_view = AppView::Chat;
        }
        self.close_transient_surfaces();
        let Some(chat) = self.active_thread().map(|thread| thread.chat.clone()) else {
            return;
        };
        chat.update(cx, |chat, cx| chat.mention_file(window, cx));
        cx.notify();
    }

    /// ⌘W closes the right panel; with nothing open the key falls through.
    pub(super) fn close_panel(
        &mut self,
        _: &ClosePanel,
        _window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if !self.right_panel_open() {
            cx.propagate();
            return;
        }
        self.tasks_panel_visible = false;
        self.diff_panel_visible = false;
        self.preview_panel_visible = false;
        self.file_surface = false;
        self.files_panel_visible = false;
        self.pull_panel_visible = false;
        self.sync_chat_panel_flags(cx);
        cx.notify();
    }

    /// Open `path` (relative to the active project) in the chosen editor.
    pub(super) fn open_path_in_editor(
        &self,
        path: &str,
        line: Option<u32>,
        cx: &mut Context<'_, Self>,
    ) {
        let full = match self.active_thread().map(|t| t.project_path.as_str()) {
            Some(root) if !root.is_empty() && !path.starts_with('/') => {
                format!("{}/{}", root.trim_end_matches('/'), path)
            }
            _ => path.to_string(),
        };
        if let Err(err) =
            bruhs_core::editor::open_in_editor(&self.editor_id, Path::new(&full), line)
        {
            tracing::warn!("open in editor failed: {err}");
            self.meetings_hub.update(cx, |hub, cx| {
                hub.show_notice(format!("Open in editor failed: {err}"), true, cx)
            });
        }
    }

    pub(super) fn set_tasks_panel_visible(&mut self, visible: bool, cx: &mut Context<'_, Self>) {
        self.tasks_panel_visible = visible;
        if visible {
            self.diff_panel_visible = false;
            self.preview_panel_visible = false;
            self.file_surface = false;
            self.files_panel_visible = false;
            self.pull_panel_visible = false;
            self.last_panel_surface = RightPanelSurface::Tasks;
        }
        self.sync_chat_panel_flags(cx);
        cx.notify();
    }

    pub(super) fn toggle_tasks(
        &mut self,
        _: &ToggleTasks,
        _window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.set_tasks_panel_visible(
            tasks_panel_visibility_after_toggle(self.tasks_panel_visible),
            cx,
        );
    }

    /// ⌘J: the terminal drawer under the chat; on the Terminal view it
    /// focuses the active session instead.
    pub(super) fn toggle_terminal(
        &mut self,
        _: &ToggleTerminal,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if self.active_view == AppView::Terminal {
            self.sync_terminal_context(cx);
            self.terminal_panel.update(cx, |panel, cx| {
                if panel.has_sessions() {
                    panel.focus_active(window, cx);
                } else {
                    panel.open_default(window, cx);
                }
            });
            cx.notify();
            return;
        }
        if self.active_view != AppView::Chat {
            self.open_chat_view(&OpenChatView, window, cx);
        }
        self.terminal_drawer_open = !self.terminal_drawer_open;
        if self.terminal_drawer_open {
            self.sync_terminal_context(cx);
            self.terminal_panel.update(cx, |panel, cx| {
                if panel.has_sessions() {
                    panel.focus_active(window, cx);
                } else {
                    panel.open_default(window, cx);
                }
            });
        }
        cx.notify();
    }

    /// The Terminal view: the same sessions as the drawer, full height.
    pub(super) fn open_terminal_view(
        &mut self,
        _: &OpenTerminalView,
        _window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.active_view = AppView::Terminal;
        self.compact_sidebar_open = false;
        self.close_transient_surfaces();
        self.persist_and_notify(cx);
    }

    /// "Choose folder…" from the terminal: a shell there, no thread.
    pub(super) fn open_terminal_in_project(
        &mut self,
        project: DesktopProjectLocation,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.close_transient_surfaces();
        self.active_project = Some(project.clone());
        push_recent_project(&mut self.recent_projects, project.clone());
        if self.active_view == AppView::Chat {
            self.terminal_drawer_open = true;
        }
        self.sync_terminal_context(cx);
        self.terminal_panel.update(cx, |panel, cx| {
            panel.open(Some(project.path), Some(project.name), window, cx);
        });
        self.persist_and_notify(cx);
    }

    /// Tell the panel which folders to offer first: the active thread's
    /// project (else the active project) and the recents.
    pub(super) fn sync_terminal_context(&self, cx: &mut Context<'_, Self>) {
        let context = self
            .active_thread()
            .filter(|thread| !thread.project_path.is_empty())
            .map(|thread| (thread.project_name.clone(), thread.project_path.clone()))
            .or_else(|| {
                self.active_project
                    .as_ref()
                    .map(|p| (p.name.clone(), p.path.clone()))
            });
        let recents: Vec<(String, String)> = self
            .recent_projects
            .iter()
            .map(|p| (p.name.clone(), p.path.clone()))
            .collect();
        self.terminal_panel.update(cx, |panel, cx| {
            panel.set_context(context, recents, cx);
        });
    }

    pub(super) fn terminal_has_focus(&self, window: &Window, cx: &App) -> bool {
        self.terminal_panel.read(cx).is_focused(window, cx)
    }

    /// Palette "Open in system Terminal": the active thread's project (or home).
    pub(super) fn open_terminal(
        &mut self,
        _: &OpenTerminal,
        _window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let path = self
            .active_thread()
            .filter(|thread| !thread.project_path.is_empty())
            .map(|thread| thread.project_path.clone())
            .or_else(|| home_dir_path().map(|p| path_to_string(&p)))
            .unwrap_or_else(|| ".".to_string());
        spawn_detached(system_terminal_command(&path));
        cx.notify();
    }

    /// Web `terminal_page.rs` header: "Terminal — shells on this machine…".
    pub(super) fn render_terminal_page_header(&self) -> Div {
        div()
            .debug_selector(|| "desktop-terminal-page-header".to_string())
            .h(px(TERMINAL_PAGE_HEADER_HEIGHT_PX))
            .px_4()
            .flex()
            .items_center()
            .gap_2()
            .flex_shrink_0()
            .font_family(theme::UI_FONT_FAMILY)
            .child(
                div()
                    .text_size(px(TERMINAL_PAGE_HEADER_TEXT_SIZE_PX))
                    .text_color(theme::rambutan_100())
                    .child(TERMINAL_PAGE_TITLE),
            )
            .child(
                div()
                    .text_size(px(TERMINAL_PAGE_HEADER_HINT_SIZE_PX))
                    .text_color(theme::rambutan_500())
                    .child(TERMINAL_PAGE_HINT),
            )
    }

    /// Right panel shell: tab bar (Diff · Tasks · Preview · Files) over the
    /// chosen surface.
    pub(super) fn render_right_panel(&self, cx: &Context<'_, Self>) -> AnyElement {
        let surface = showing_surface(
            self.file_surface,
            self.pull_panel_visible,
            self.files_panel_visible,
            self.diff_panel_visible,
            self.preview_panel_visible,
        );
        let body: AnyElement = match surface {
            RightPanelSurface::Diff => self.diff_panel.clone().into_any_element(),
            RightPanelSurface::Tasks => self.tasks_panel.clone().into_any_element(),
            RightPanelSurface::Preview => self.preview_panel.clone().into_any_element(),
            RightPanelSurface::Files => self.render_files_panel(cx),
            RightPanelSurface::Pull => self.render_branch_pull(cx),
            RightPanelSurface::File => self.render_file_viewer(cx),
        };
        div()
            .debug_selector(|| "desktop-right-panel".to_string())
            .w_full()
            .h_full()
            .bg(theme::longan_950())
            .border_l_1()
            .border_color(theme::longan_800().opacity(WORKSPACE_SIDEBAR_BORDER_OPACITY))
            .flex()
            .flex_col()
            .child(
                div()
                    .h(px(WORKSPACE_PANEL_TAB_BAR_HEIGHT_PX))
                    .px_2()
                    .flex()
                    .items_center()
                    .gap_1()
                    .flex_shrink_0()
                    .border_b_1()
                    .border_color(theme::longan_800().opacity(WORKSPACE_SIDEBAR_BORDER_OPACITY))
                    .child(self.render_panel_tab(
                        RightPanelSurface::Diff,
                        assets::FILE_DIFF_ICON_PATH,
                        "Diff",
                        surface,
                        cx,
                    ))
                    .child(self.render_panel_tab(
                        RightPanelSurface::Tasks,
                        assets::ACTIVITY_ICON_PATH,
                        "Tasks",
                        surface,
                        cx,
                    ))
                    .child(self.render_panel_tab(
                        RightPanelSurface::Preview,
                        assets::TOOL_GLOBE_ICON_PATH,
                        "Preview",
                        surface,
                        cx,
                    ))
                    .child(self.render_panel_tab(
                        RightPanelSurface::Files,
                        assets::FOLDER_ICON_PATH,
                        "Files",
                        surface,
                        cx,
                    ))
                    .children(self.render_pull_panel_tab(surface, cx))
                    .children(self.render_file_panel_tab(surface, cx))
                    .child(div().flex_1())
                    .child(
                        div()
                            .id("desktop-panel-close")
                            .debug_selector(|| "desktop-panel-close".to_string())
                            .size(px(WORKSPACE_PANEL_TAB_HEIGHT_PX))
                            .rounded(px(WORKSPACE_SIDEBAR_TAB_RADIUS_PX))
                            .flex()
                            .items_center()
                            .justify_center()
                            .text_size(px(WORKSPACE_PANEL_TAB_TEXT_SIZE_PX))
                            .font_family(theme::UI_SMALL_FONT_FAMILY)
                            .text_color(theme::rambutan_500())
                            .tab_index(0)
                            .role(Role::Button)
                            .aria_label("Close panel")
                            .focus_visible(|el| el.border_2().border_color(theme::persimmon_400()))
                            .cursor_pointer()
                            .hover(|el| {
                                el.bg(theme::longan_800()
                                    .opacity(WORKSPACE_THREAD_ROW_HOVER_BG_OPACITY))
                                    .text_color(theme::rambutan_200())
                            })
                            .on_click(cx.listener(|this, _ev, window, cx| {
                                this.close_panel(&ClosePanel, window, cx);
                            }))
                            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                                if is_web_button_activation_key(event.keystroke.key.as_str()) {
                                    window.prevent_default();
                                    cx.stop_propagation();
                                    this.close_panel(&ClosePanel, window, cx);
                                }
                            }))
                            .child("×"),
                    ),
            )
            .child(div().flex_1().min_h_0().child(body))
            .into_any_element()
    }

    pub(super) fn render_panel_tab(
        &self,
        surface: RightPanelSurface,
        icon_path: &'static str,
        label: &'static str,
        showing: RightPanelSurface,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        let active = surface == showing;
        let selector = match surface {
            RightPanelSurface::Diff => "desktop-panel-tab-diff",
            RightPanelSurface::Tasks => "desktop-panel-tab-tasks",
            RightPanelSurface::Preview => "desktop-panel-tab-preview",
            RightPanelSurface::Files => "desktop-panel-tab-files",
            RightPanelSurface::Pull => "desktop-panel-tab-pull",
            RightPanelSurface::File => "desktop-panel-tab-file",
        };
        div()
            .id(selector)
            .debug_selector(move || selector.to_string())
            .h(px(WORKSPACE_PANEL_TAB_HEIGHT_PX))
            .px_2()
            .rounded(px(WORKSPACE_SIDEBAR_TAB_RADIUS_PX))
            .flex()
            .items_center()
            .gap_1()
            .text_size(px(WORKSPACE_PANEL_TAB_TEXT_SIZE_PX))
            .font_family(theme::UI_SMALL_FONT_FAMILY)
            .text_color(if active {
                theme::rambutan_100()
            } else {
                theme::rambutan_400()
            })
            .when(active, |el| el.bg(theme::longan_800()))
            .when(!active, |el| {
                el.hover(|el| {
                    el.bg(theme::longan_800().opacity(WORKSPACE_THREAD_ROW_HOVER_BG_OPACITY))
                        .text_color(theme::rambutan_100())
                })
            })
            .tab_index(0)
            .role(Role::Tab)
            .aria_label(format!("{label} panel"))
            .focus_visible(|el| el.border_2().border_color(theme::persimmon_400()))
            .cursor_pointer()
            .on_click(cx.listener(move |this, _ev, _window, cx| match surface {
                RightPanelSurface::Diff => this.set_diff_panel_visible(true, cx),
                RightPanelSurface::Tasks => this.set_tasks_panel_visible(true, cx),
                RightPanelSurface::Preview => this.set_preview_panel_visible(true, cx),
                RightPanelSurface::Files => this.set_files_panel_visible(true, cx),
                RightPanelSurface::Pull => this.show_pull_surface(cx),
                RightPanelSurface::File => this.show_file_surface(cx),
            }))
            .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
                if is_web_button_activation_key(event.keystroke.key.as_str()) {
                    window.prevent_default();
                    cx.stop_propagation();
                    match surface {
                        RightPanelSurface::Diff => this.set_diff_panel_visible(true, cx),
                        RightPanelSurface::Tasks => this.set_tasks_panel_visible(true, cx),
                        RightPanelSurface::Preview => this.set_preview_panel_visible(true, cx),
                        RightPanelSurface::Files => this.set_files_panel_visible(true, cx),
                        RightPanelSurface::Pull => this.show_pull_surface(cx),
                        RightPanelSurface::File => this.show_file_surface(cx),
                    }
                }
            }))
            .child(
                svg()
                    .path(icon_path)
                    .size(px(WORKSPACE_PANEL_TAB_ICON_SIZE_PX))
                    .text_color(if active {
                        theme::rambutan_100()
                    } else {
                        theme::rambutan_400()
                    })
                    .flex_shrink_0(),
            )
            .child(label)
            .into_any_element()
    }

    fn render_file_panel_tab(
        &self,
        showing: RightPanelSurface,
        cx: &Context<'_, Self>,
    ) -> Option<AnyElement> {
        let file = self.open_file.as_ref()?;
        let label = file.label.clone();
        let active = showing == RightPanelSurface::File;
        let tab_label = label.clone();
        Some(
            div()
                .id("desktop-panel-tab-file")
                .debug_selector(|| "desktop-panel-tab-file".to_string())
                .h(px(WORKSPACE_PANEL_TAB_HEIGHT_PX))
                .max_w(px(144.0))
                .px_2()
                .rounded(px(WORKSPACE_SIDEBAR_TAB_RADIUS_PX))
                .flex()
                .items_center()
                .gap_1()
                .text_size(px(WORKSPACE_PANEL_TAB_TEXT_SIZE_PX))
                .font_family(theme::UI_SMALL_FONT_FAMILY)
                .text_color(if active {
                    theme::rambutan_100()
                } else {
                    theme::rambutan_400()
                })
                .when(active, |el| el.bg(theme::longan_800()))
                .when(!active, |el| {
                    el.hover(|el| {
                        el.bg(theme::longan_800().opacity(WORKSPACE_THREAD_ROW_HOVER_BG_OPACITY))
                            .text_color(theme::rambutan_100())
                    })
                })
                .tab_index(0)
                .role(Role::Tab)
                .aria_label(format!("{label} panel"))
                .aria_selected(active)
                .focus_visible(|el| el.border_2().border_color(theme::persimmon_400()))
                .cursor_pointer()
                .on_click(cx.listener(|this, _ev, _window, cx| {
                    this.show_file_surface(cx);
                }))
                .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                    if is_web_button_activation_key(event.keystroke.key.as_str()) {
                        window.prevent_default();
                        cx.stop_propagation();
                        this.show_file_surface(cx);
                    }
                }))
                .child(
                    svg()
                        .path(assets::TOOL_FILE_ICON_PATH)
                        .size(px(WORKSPACE_PANEL_TAB_ICON_SIZE_PX))
                        .text_color(if active {
                            theme::rambutan_100()
                        } else {
                            theme::rambutan_400()
                        })
                        .flex_shrink_0(),
                )
                .child(div().min_w_0().truncate().child(tab_label))
                .into_any_element(),
        )
    }

    fn active_project_path(&self, cx: &App) -> Option<String> {
        let thread = self.active_thread()?;
        let chat = thread.chat.read(cx);
        if chat.project_scoped && !chat.project_path.is_empty() {
            Some(chat.project_path.clone())
        } else if !thread.project_path.is_empty() {
            Some(thread.project_path.clone())
        } else {
            None
        }
    }

    fn active_repo_branch(&self, cx: &App) -> Option<(String, String, String)> {
        let thread = self.active_thread()?;
        let path = self.active_project_path(cx)?;
        let branch = thread.branch.clone().filter(|branch| !branch.is_empty())?;
        Some((thread.id.clone(), path, branch))
    }

    pub(super) fn set_files_panel_visible(&mut self, visible: bool, cx: &mut Context<'_, Self>) {
        self.files_panel_visible = visible;
        if visible {
            self.tasks_panel_visible = false;
            self.diff_panel_visible = false;
            self.preview_panel_visible = false;
            self.file_surface = false;
            self.pull_panel_visible = false;
            self.last_panel_surface = RightPanelSurface::Files;
            self.load_files(cx);
        }
        self.sync_chat_panel_flags(cx);
        cx.notify();
    }

    pub(super) fn toggle_files(&mut self, cx: &mut Context<'_, Self>) {
        self.set_files_panel_visible(!self.files_panel_visible, cx);
    }

    pub(super) fn load_files(&mut self, cx: &mut Context<'_, Self>) {
        let Some(root) = self.active_project_path(cx) else {
            self.files_root = None;
            self.files_dir.clear();
            self.files_entries.clear();
            self.files_loading = false;
            self.files_error = Some("Open a project to browse files.".into());
            cx.notify();
            return;
        };
        if self.files_root.as_deref() != Some(root.as_str()) {
            self.files_dir.clear();
            self.files_root = Some(root.clone());
        }
        self.files_generation = self.files_generation.wrapping_add(1);
        let generation = self.files_generation;
        let relative = self.files_dir.clone();
        self.files_entries.clear();
        self.files_loading = true;
        self.files_error = None;
        let listed = spawn_tokio(cx, async move {
            bruhs_core::files::list_children(Path::new(&root), &relative).await
        });
        let workspace = cx.weak_entity();
        cx.spawn(async move |_, cx| {
            let listed = listed.await;
            let _ = workspace.update(cx, |this, cx| {
                if this.files_generation != generation {
                    return;
                }
                this.files_loading = false;
                match listed {
                    Ok(entries) => {
                        this.files_entries = entries;
                        this.files_error = None;
                    }
                    Err(error) => {
                        this.files_entries.clear();
                        this.files_error = Some(error.to_string());
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn render_files_panel(&self, cx: &Context<'_, Self>) -> AnyElement {
        let crumb = if self.files_dir.is_empty() {
            "Project".to_string()
        } else {
            self.files_dir.clone()
        };
        let mut header = div()
            .h(px(32.0))
            .px_3()
            .flex()
            .items_center()
            .gap_2()
            .flex_shrink_0()
            .border_b_1()
            .border_color(theme::longan_800().opacity(WORKSPACE_SIDEBAR_BORDER_OPACITY));
        if let Some(parent) = bruhs_models::logic::branch_pull::parent_dir(&self.files_dir) {
            let parent = parent.to_string();
            header = header.child(
                div()
                    .id("desktop-files-up")
                    .debug_selector(|| "desktop-files-up".to_string())
                    .h(px(24.0))
                    .px_2()
                    .rounded(px(WORKSPACE_SIDEBAR_TAB_RADIUS_PX))
                    .flex()
                    .items_center()
                    .text_size(px(12.0))
                    .font_family(theme::UI_SMALL_FONT_FAMILY)
                    .text_color(theme::rambutan_400())
                    .cursor_pointer()
                    .hover(|el| el.text_color(theme::rambutan_100()))
                    .child("Up")
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.files_dir = parent.clone();
                        this.load_files(cx);
                    })),
            );
        }
        header = header.child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .text_size(px(12.0))
                .font_family(theme::UI_SMALL_FONT_FAMILY)
                .text_color(theme::rambutan_500())
                .child(crumb),
        );
        let mut body = div()
            .flex_1()
            .min_h_0()
            .id("desktop-files-list")
            .overflow_y_scroll();
        if let Some(error) = &self.files_error {
            body = body.child(self.files_status_line(error));
        } else if self.files_loading && self.files_entries.is_empty() {
            body = body.child(self.files_status_line("Loading files…"));
        } else if self.files_entries.is_empty() {
            body = body.child(self.files_status_line("This folder is empty."));
        } else {
            for (index, entry) in self.files_entries.iter().enumerate() {
                let is_dir = entry.is_directory;
                let click_path = entry.path.trim_end_matches('/').to_string();
                let label = entry.name.trim_end_matches('/').to_string();
                let click_label = label.clone();
                let icon = if is_dir {
                    assets::FOLDER_ICON_PATH
                } else {
                    assets::TOOL_FILE_ICON_PATH
                };
                body = body.child(
                    div()
                        .id(SharedString::from(format!("desktop-files-row-{index}")))
                        .debug_selector({
                            let selector = format!("desktop-files-row-{index}");
                            move || selector
                        })
                        .h(px(32.0))
                        .px_3()
                        .flex()
                        .items_center()
                        .gap_2()
                        .cursor_pointer()
                        .text_size(px(12.0))
                        .font_family(theme::UI_SMALL_FONT_FAMILY)
                        .text_color(theme::rambutan_300())
                        .hover(|el| {
                            el.bg(theme::longan_800()
                                    .opacity(WORKSPACE_THREAD_ROW_HOVER_BG_OPACITY))
                                    .text_color(theme::rambutan_100())
                        })
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.open_files_entry(
                                is_dir,
                                click_path.clone(),
                                click_label.clone(),
                                cx,
                            );
                        }))
                        .child(
                            svg()
                                .path(icon)
                                .size(px(14.0))
                                .text_color(theme::rambutan_400())
                                .flex_shrink_0(),
                        )
                        .child(div().min_w_0().truncate().child(label)),
                );
            }
        }
        div()
            .debug_selector(|| "desktop-files-panel".to_string())
            .size_full()
            .flex()
            .flex_col()
            .child(header)
            .child(body)
            .into_any_element()
    }

    fn files_status_line(&self, text: &str) -> AnyElement {
        div()
            .px_3()
            .py_3()
            .text_size(px(12.0))
            .font_family(theme::UI_SMALL_FONT_FAMILY)
            .text_color(theme::rambutan_400())
            .child(text.to_string())
            .into_any_element()
    }

    fn open_files_entry(
        &mut self,
        is_dir: bool,
        path: String,
        label: String,
        cx: &mut Context<'_, Self>,
    ) {
        if is_dir {
            self.files_dir = path;
            self.load_files(cx);
        } else if let Some(root) = self.files_root.clone() {
            self.open_attached_file(path, label, Some(root), false, cx);
        }
    }

    fn render_pull_panel_tab(
        &self,
        showing: RightPanelSurface,
        cx: &Context<'_, Self>,
    ) -> Option<AnyElement> {
        let pull = self.branch_pull.as_ref()?;
        let label = format!("#{}", pull.number);
        let active = showing == RightPanelSurface::Pull;
        Some(
            div()
                .id("desktop-panel-tab-pull")
                .debug_selector(|| "desktop-panel-tab-pull".to_string())
                .h(px(WORKSPACE_PANEL_TAB_HEIGHT_PX))
                .max_w(px(144.0))
                .px_2()
                .rounded(px(WORKSPACE_SIDEBAR_TAB_RADIUS_PX))
                .flex()
                .items_center()
                .gap_1()
                .text_size(px(WORKSPACE_PANEL_TAB_TEXT_SIZE_PX))
                .font_family(theme::UI_SMALL_FONT_FAMILY)
                .text_color(if active {
                    theme::rambutan_100()
                } else {
                    theme::rambutan_400()
                })
                .when(active, |el| el.bg(theme::longan_800()))
                .tab_index(0)
                .role(Role::Tab)
                .aria_label(format!("{label} panel"))
                .aria_selected(active)
                .focus_visible(|el| el.border_2().border_color(theme::persimmon_400()))
                .cursor_pointer()
                .when(!active, |el| {
                    el.hover(|el| {
                        el.bg(theme::longan_800().opacity(WORKSPACE_THREAD_ROW_HOVER_BG_OPACITY))
                            .text_color(theme::rambutan_100())
                    })
                })
                .on_click(cx.listener(|this, _, _, cx| this.show_pull_surface(cx)))
                .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                    if is_web_button_activation_key(event.keystroke.key.as_str()) {
                        window.prevent_default();
                        cx.stop_propagation();
                        this.show_pull_surface(cx);
                    }
                }))
                .child(
                    svg()
                        .path(assets::GIT_BRANCH_ICON_PATH)
                        .size(px(WORKSPACE_PANEL_TAB_ICON_SIZE_PX))
                        .flex_shrink_0(),
                )
                .child(div().min_w_0().truncate().child(label))
                .into_any_element(),
        )
    }

    pub(super) fn show_pull_surface(&mut self, cx: &mut Context<'_, Self>) {
        self.pull_panel_visible = true;
        self.tasks_panel_visible = false;
        self.diff_panel_visible = false;
        self.preview_panel_visible = false;
        self.file_surface = false;
        self.files_panel_visible = false;
        self.last_panel_surface = RightPanelSurface::Pull;
        self.sync_chat_panel_flags(cx);
        cx.notify();
    }

    pub(super) fn refresh_branch_pull(&mut self, cx: &mut Context<'_, Self>) {
        let Some((thread_id, path, branch)) = self.active_repo_branch(cx) else {
            self.branch_pull = None;
            cx.notify();
            return;
        };
        self.pull_generation = self.pull_generation.wrapping_add(1);
        let generation = self.pull_generation;
        let lookup_branch = branch.clone();
        let loaded = spawn_tokio(cx, async move {
            bruhs_core::pulls::pull_for_branch(Path::new(&path), &lookup_branch).await
        });
        let workspace = cx.weak_entity();
        cx.spawn(async move |_, cx| {
            let lookup = loaded.await;
            let _ = workspace.update(cx, |this, cx| {
                if this.pull_generation != generation {
                    return;
                }
                let Some(pull) = lookup.pull else {
                    this.branch_pull = None;
                    cx.notify();
                    return;
                };
                let number = pull.number;
                let key =
                    bruhs_models::logic::branch_pull::branch_pull_key(&thread_id, &branch, number);
                let open = bruhs_models::logic::branch_pull::should_open_branch_pull(
                    lookup.signed_in,
                    Some(number),
                    this.opened_pull_key.as_deref(),
                    &key,
                );
                this.branch_pull = Some(DesktopBranchPull {
                    number,
                    title: pull.title,
                    url: pull.url,
                    state: pull.state,
                });
                if open {
                    this.opened_pull_key = Some(key);
                    this.show_pull_surface(cx);
                } else {
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn render_branch_pull(&self, cx: &Context<'_, Self>) -> AnyElement {
        let Some(pull) = self.branch_pull.clone() else {
            return div()
                .debug_selector(|| "desktop-branch-pull".to_string())
                .p_4()
                .text_size(px(12.0))
                .font_family(theme::UI_SMALL_FONT_FAMILY)
                .text_color(theme::rambutan_400())
                .child("No pull request for this branch.")
                .into_any_element();
        };
        let state = bruhs_models::logic::branch_pull::pull_state_label(&pull.state);
        let heading = format!("#{} {}", pull.number, pull.title);
        let url = bruhs_models::logic::branch_pull::github_pull_url(&pull.url).map(str::to_string);
        div()
            .debug_selector(|| "desktop-branch-pull".to_string())
            .size_full()
            .p_4()
            .flex()
            .flex_col()
            .items_start()
            .gap_3()
            .child(
                div()
                    .text_size(px(12.0))
                    .font_family(theme::UI_SMALL_FONT_FAMILY)
                    .text_color(theme::rambutan_500())
                    .child(state),
            )
            .child(
                div()
                    .text_size(px(14.0))
                    .font_family(theme::UI_FONT_FAMILY)
                    .text_color(theme::rambutan_100())
                    .child(heading),
            )
            .children(url.map(|url| {
                div()
                    .id("desktop-branch-pull-open")
                    .debug_selector(|| "desktop-branch-pull-open".to_string())
                    .h(px(32.0))
                    .px_2()
                    .rounded(px(WORKSPACE_SIDEBAR_TAB_RADIUS_PX))
                    .flex()
                    .items_center()
                    .cursor_pointer()
                    .text_size(px(12.0))
                    .font_family(theme::UI_SMALL_FONT_FAMILY)
                    .text_color(theme::rambutan_100())
                    .border_1()
                    .border_color(theme::longan_800())
                    .hover(|el| {
                        el.bg(theme::longan_800().opacity(WORKSPACE_THREAD_ROW_HOVER_BG_OPACITY))
                    })
                    .child("Open on GitHub")
                    .on_click(cx.listener(move |_, _, _, _| {
                        spawn_detached(browser_open_command(&url));
                    }))
            }))
            .into_any_element()
    }

    pub(super) fn show_file_surface(&mut self, cx: &mut Context<'_, Self>) {
        if self.open_file.is_none() {
            return;
        }
        self.file_surface = true;
        self.tasks_panel_visible = false;
        self.diff_panel_visible = false;
        self.preview_panel_visible = false;
        self.files_panel_visible = false;
        self.pull_panel_visible = false;
        self.last_panel_surface = RightPanelSurface::File;
        self.sync_chat_panel_flags(cx);
        cx.notify();
    }

    pub(super) fn open_attached_file(
        &mut self,
        path: String,
        label: String,
        project: Option<String>,
        source: bool,
        cx: &mut Context<'_, Self>,
    ) {
        self.file_generation = self.file_generation.wrapping_add(1);
        let generation = self.file_generation;
        self.open_file = Some(DesktopOpenFile {
            path,
            label,
            project,
            source,
        });
        self.file_preview = DesktopFilePreview::Loading;
        self.show_file_surface(cx);
        self.load_file_preview(generation, cx);
    }

    fn load_file_preview(&mut self, generation: u64, cx: &mut Context<'_, Self>) {
        let Some(open) = self.open_file.clone() else {
            return;
        };
        let requested = open.path.clone();
        let path = open.path;
        let project = open.project;
        let opened = cx.background_spawn(async move {
            let root = project.as_deref().map(Path::new);
            bruhs_core::open_for_viewer(&path, root)
        });
        let workspace = cx.weak_entity();
        cx.spawn(async move |_, cx| {
            let opened = opened.await;
            let _ = workspace.update(cx, |this, cx| {
                if this.file_generation != generation {
                    return;
                }
                let same = this
                    .open_file
                    .as_ref()
                    .is_some_and(|file| file.path == requested);
                if !same {
                    return;
                }
                let opener_label = this
                    .open_file
                    .as_ref()
                    .map(|file| file.label.clone())
                    .unwrap_or_default();
                this.file_preview = match opened {
                    Ok(file) => {
                        let label = if opener_label.is_empty() {
                            file.name
                        } else {
                            opener_label
                        };
                        DesktopFilePreview::Ready(DesktopReadyFile {
                            label,
                            path: file.path,
                            kind: file.kind,
                            text: file.text,
                            truncated: file.truncated,
                        })
                    }
                    Err(error) => DesktopFilePreview::Failed(error.message().to_string()),
                };
                cx.notify();
            });
        })
        .detach();
    }

    fn render_file_viewer(&self, _cx: &Context<'_, Self>) -> AnyElement {
        // Line links set `source`. Desktop HTML is always source, so the flag
        // is recorded on the open file and does not change this view.
        let _asked_for_source = self.open_file.as_ref().is_some_and(|file| file.source);
        let preview = self.file_preview.clone();
        let body = if self.open_file.is_none() {
            self.file_viewer_note("Select a file to read it.", false)
        } else {
            match preview {
                DesktopFilePreview::Empty => {
                    self.file_viewer_note("Select a file to read it.", false)
                }
                DesktopFilePreview::Loading => self.file_viewer_note("Opening…", false),
                DesktopFilePreview::Failed(message) => self.file_viewer_note(message, true),
                DesktopFilePreview::Ready(ready) => self.render_ready_file(&ready),
            }
        };
        div()
            .id("desktop-file-viewer")
            .debug_selector(|| "desktop-file-viewer".to_string())
            .size_full()
            .flex()
            .flex_col()
            .min_h_0()
            .child(body)
            .into_any_element()
    }

    fn render_ready_file(&self, ready: &DesktopReadyFile) -> AnyElement {
        div()
            .size_full()
            .flex()
            .flex_col()
            .min_h_0()
            .child(
                div()
                    .h(px(32.0))
                    .px_3()
                    .flex()
                    .items_center()
                    .flex_shrink_0()
                    .border_b_1()
                    .border_color(theme::longan_800().opacity(0.6))
                    .child(
                        div()
                            .min_w_0()
                            .flex_1()
                            .truncate()
                            .text_size(px(12.0))
                            .font_family(theme::UI_SMALL_FONT_FAMILY)
                            .text_color(theme::rambutan_100())
                            .id("desktop-file-viewer-name")
                            .tooltip({
                                let path = ready.path.clone();
                                move |window, cx| Tooltip::new(path.clone()).build(window, cx)
                            })
                            .child(ready.label.clone()),
                    ),
            )
            .child(
                div()
                    .id("desktop-file-viewer-body")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(self.render_ready_body(ready)),
            )
            .into_any_element()
    }

    fn render_ready_body(&self, ready: &DesktopReadyFile) -> AnyElement {
        match ready.kind {
            FileKind::Image => img(PathBuf::from(ready.path.clone()))
                .id("desktop-file-image")
                .object_fit(ObjectFit::Contain)
                .w_full()
                .with_fallback({
                    let path = ready.path.clone();
                    move || {
                        div()
                            .p_4()
                            .text_size(px(12.0))
                            .text_color(theme::rambutan_400())
                            .child(path.clone())
                            .into_any_element()
                    }
                })
                .into_any_element(),
            FileKind::Html | FileKind::Text => {
                let mut notes = Vec::new();
                if ready.kind == FileKind::Html {
                    notes.push("Scripts and neighboring files are not loaded.");
                }
                if ready.truncated {
                    notes.push("This preview shows the first 256 KiB.");
                }
                div()
                    .p_4()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .text_size(px(12.0))
                    .font_family(theme::UI_SMALL_FONT_FAMILY)
                    .text_color(theme::rambutan_100())
                    .child(ready.text.clone().unwrap_or_default())
                    .children(
                        notes
                            .into_iter()
                            .map(|text| div().text_color(theme::rambutan_500()).child(text)),
                    )
                    .into_any_element()
            }
            FileKind::Audio | FileKind::Video | FileKind::Pdf | FileKind::Other => div()
                .p_4()
                .text_size(px(12.0))
                .font_family(theme::UI_SMALL_FONT_FAMILY)
                .text_color(theme::rambutan_400())
                .child(ready.path.clone())
                .into_any_element(),
        }
    }

    fn file_viewer_note(&self, message: impl Into<String>, is_error: bool) -> AnyElement {
        div()
            .p(px(24.0))
            .text_size(px(12.0))
            .font_family(theme::UI_SMALL_FONT_FAMILY)
            .text_color(if is_error {
                theme::rambutan_400()
            } else {
                theme::rambutan_500()
            })
            .child(message.into())
            .into_any_element()
    }
}

/// The right panel's views, the terminal, and the restored split sizes.
pub(super) struct Panels {
    pub(super) tasks_panel: Entity<TasksPanelView>,
    pub(super) diff_panel: Entity<DiffPanelView>,
    pub(super) terminal_panel: Entity<TerminalPanelView>,
    pub(super) terminal_drawer_height: f32,
    pub(super) terminal_split_state: Entity<ResizableState>,
    pub(super) right_panel_width: f32,
    pub(super) right_panel_split_state: Entity<ResizableState>,
    pub(super) preview_panel: Entity<PreviewPanelView>,
}

impl Workspace {
    /// The tasks, diff, terminal and preview panels and their split handles, sized from the saved state.
    pub(super) fn new_panels(
        persisted: Option<&PersistedWorkspaceState>,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
        subscriptions: &mut Vec<Subscription>,
    ) -> Panels {
        let desktop_app = cx.global::<DesktopState>().app.clone();
        let tasks_panel = cx.new(|cx| TasksPanelView::new(desktop_app, cx));
        subscriptions.push(cx.subscribe_in(
            &tasks_panel,
            window,
            |this: &mut Self, _panel, event: &TasksPanelEvent, _window, cx| match event {
                TasksPanelEvent::CloseRequested => {
                    this.set_tasks_panel_visible(false, cx);
                }
                TasksPanelEvent::StartConversationRequested => {
                    this.new_general_thread(_window, cx);
                }
            },
        ));
        let diff_panel = cx.new(DiffPanelView::new);
        subscriptions.push(cx.subscribe_in(
            &diff_panel,
            window,
            |this: &mut Self, _panel, event: &DiffPanelEvent, _window, cx| match event {
                DiffPanelEvent::OpenInEditor { path, line } => {
                    this.open_path_in_editor(path, *line, cx);
                }
            },
        ));

        let (terminal_manager, terminal_runtime) = {
            let desktop_state = cx.global::<DesktopState>();
            (
                desktop_state.app.terminals().clone(),
                Arc::clone(&desktop_state.runtime),
            )
        };
        let terminal_panel =
            cx.new(|cx| TerminalPanelView::new(terminal_manager, terminal_runtime, cx));
        subscriptions.push(cx.subscribe_in(
            &terminal_panel,
            window,
            |this: &mut Self, _panel, event: &TerminalPanelEvent, window, cx| match event {
                TerminalPanelEvent::HideDrawer => {
                    this.terminal_drawer_open = false;
                    cx.notify();
                }
                TerminalPanelEvent::ChooseFolder => {
                    this.open_project_picker(&OpenProjectPicker, window, cx);
                    if let Some(TopLayer::ProjectPicker { for_terminal }) = &mut this.top_layer {
                        *for_terminal = true;
                    }
                }
                TerminalPanelEvent::CiteExcerpt { label, excerpt } => {
                    let Some(chat) = this.active_thread().map(|thread| thread.chat.clone()) else {
                        return;
                    };
                    let label = label.clone();
                    let excerpt = excerpt.clone();
                    chat.update(cx, |chat, cx| {
                        chat.insert_context_chip(
                            bruhs_models::logic::context_chip::ChipKind::Terminal {
                                label,
                                excerpt,
                            },
                            window,
                            cx,
                        );
                    });
                }
            },
        ));

        let terminal_drawer_height = persisted
            .and_then(|state| state.terminal_drawer_height)
            .map_or(TERMINAL_DRAWER_HEIGHT_PX, clamp_terminal_drawer_height);
        let terminal_split_state = cx.new(|_| ResizableState::default());
        subscriptions.push(cx.subscribe(
            &terminal_split_state,
            |this: &mut Self, state, event: &ResizablePanelEvent, cx| match event {
                ResizablePanelEvent::Resized => {
                    if let Some(height) = state.read(cx).sizes().get(1).copied() {
                        this.terminal_drawer_height =
                            clamp_terminal_drawer_height(f32::from(height));
                        this.persist_and_notify(cx);
                    }
                }
            },
        ));
        let right_panel_width = persisted
            .and_then(|state| state.right_panel_width)
            .filter(|width| width.is_finite() && *width >= RIGHT_PANEL_MIN_WIDTH_PX)
            .unwrap_or(WORKSPACE_RIGHT_PANEL_WIDTH_PX);
        let right_panel_split_state = cx.new(|_| ResizableState::default());
        subscriptions.push(cx.subscribe(
            &right_panel_split_state,
            |this: &mut Self, state, event: &ResizablePanelEvent, cx| match event {
                ResizablePanelEvent::Resized => {
                    if let Some(width) = state.read(cx).sizes().get(1).copied() {
                        this.right_panel_width = f32::from(width).max(RIGHT_PANEL_MIN_WIDTH_PX);
                        this.persist_and_notify(cx);
                    }
                }
            },
        ));
        let preview_panel = cx.new(|cx| PreviewPanelView::new(window, cx));
        subscriptions.push(cx.subscribe_in(
            &preview_panel,
            window,
            |_this: &mut Self, _panel, event: &PreviewPanelEvent, _window, _cx| match event {
                PreviewPanelEvent::OpenExternal(url) => spawn_detached(browser_open_command(url)),
            },
        ));
        Panels {
            tasks_panel,
            diff_panel,
            terminal_panel,
            terminal_drawer_height,
            terminal_split_state,
            right_panel_width,
            right_panel_split_state,
            preview_panel,
        }
    }
}

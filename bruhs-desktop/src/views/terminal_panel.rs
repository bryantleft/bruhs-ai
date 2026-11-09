//! Terminal tabs and their host. One entity per window: the workspace shows
//! it as the drawer under the chat (⌘J) or as the full Terminal view, so the
//! sessions are the same wherever you look at them.

use std::sync::Arc;

use bruhs_core::terminal::{TerminalManager, TerminalSpec};
use gpui::prelude::*;
use gpui::*;

use crate::views::terminal_view::{TerminalStatus, TerminalView};
use crate::{assets, theme};

/// Default drawer height; the user drags the split, the workspace persists it.
pub const TERMINAL_DRAWER_HEIGHT_PX: f32 = 300.0;
pub const TERMINAL_DRAWER_MIN_HEIGHT_PX: f32 = 120.0;
pub const TERMINAL_DRAWER_MAX_HEIGHT_PX: f32 = 800.0;

/// Keep a persisted or dragged height inside the drawer's range.
pub fn clamp_terminal_drawer_height(height: f32) -> f32 {
    if !height.is_finite() {
        return TERMINAL_DRAWER_HEIGHT_PX;
    }
    height.clamp(TERMINAL_DRAWER_MIN_HEIGHT_PX, TERMINAL_DRAWER_MAX_HEIGHT_PX)
}
const HEADER_HEIGHT_PX: f32 = 32.0;
const HEADER_TEXT_SIZE_PX: f32 = 12.0;
const TAB_HEIGHT_PX: f32 = 24.0;
const TAB_MAX_WIDTH_PX: f32 = 160.0;
const TAB_RADIUS_PX: f32 = 4.0;
const ICON_SIZE_PX: f32 = 12.0;
const CLOSE_ICON_SIZE_PX: f32 = 10.0;
const MENU_WIDTH_PX: f32 = 280.0;
const MENU_ROW_HEIGHT_PX: f32 = 28.0;
const EMPTY_TEXT_SIZE_PX: f32 = 13.0;
const EMPTY_HINT_TEXT_SIZE_PX: f32 = 12.0;
const EMPTY_ROW_WIDTH_PX: f32 = 320.0;
const RECENT_PROJECTS_IN_MENU: usize = 5;
const BORDER_OPACITY: f32 = 0.7;
const HOVER_BG_OPACITY: f32 = 0.5;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TerminalPanelMode {
    /// Under the chat transcript, fixed height.
    Drawer,
    /// The Terminal view, full height.
    Page,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TerminalPanelEvent {
    /// The drawer's × was clicked.
    HideDrawer,
    /// "Choose folder…": the workspace opens its project picker.
    ChooseFolder,
    /// The header's Cite button: the selected terminal text, for the composer.
    CiteExcerpt { label: String, excerpt: String },
}

/// Which session becomes active after closing `closed`: the active one if
/// it survives, else the next tab, else the previous, else none.
pub fn next_active_after_close(
    ids: &[String],
    closed: &str,
    active: Option<&str>,
) -> Option<String> {
    if active.is_some_and(|a| a != closed) {
        return active.map(str::to_string);
    }
    let pos = ids.iter().position(|id| id == closed)?;
    ids.get(pos + 1)
        .or_else(|| pos.checked_sub(1).and_then(|p| ids.get(p)))
        .cloned()
}

/// Folders offered by "+ New" and the empty state, deduplicated by path:
/// the surrounding context first, then recents.
pub fn menu_projects(
    context: Option<&(String, String)>,
    recents: &[(String, String)],
) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    if let Some(ctx) = context {
        out.push(ctx.clone());
    }
    for p in recents.iter().take(RECENT_PROJECTS_IN_MENU) {
        if !out.iter().any(|o| o.1 == p.1) {
            out.push(p.clone());
        }
    }
    out
}

pub struct TerminalPanelView {
    mode: TerminalPanelMode,
    manager: TerminalManager,
    runtime: Arc<tokio::runtime::Runtime>,
    sessions: Vec<Entity<TerminalView>>,
    active: Option<String>,
    new_menu_open: bool,
    /// `(name, path)` of the active thread's project, offered first.
    context_project: Option<(String, String)>,
    recent_projects: Vec<(String, String)>,
    error: Option<String>,
}

impl EventEmitter<TerminalPanelEvent> for TerminalPanelView {}

impl TerminalPanelView {
    pub fn new(
        manager: TerminalManager,
        runtime: Arc<tokio::runtime::Runtime>,
        _cx: &mut Context<'_, Self>,
    ) -> Self {
        Self {
            mode: TerminalPanelMode::Drawer,
            manager,
            runtime,
            sessions: Vec::new(),
            active: None,
            new_menu_open: false,
            context_project: None,
            recent_projects: Vec::new(),
            error: None,
        }
    }

    /// Where the panel is being shown; no notify, the caller re-renders.
    pub fn set_mode(&mut self, mode: TerminalPanelMode) {
        self.mode = mode;
    }

    pub fn set_context(
        &mut self,
        context_project: Option<(String, String)>,
        recent_projects: Vec<(String, String)>,
        cx: &mut Context<'_, Self>,
    ) {
        if self.context_project != context_project || self.recent_projects != recent_projects {
            self.context_project = context_project;
            self.recent_projects = recent_projects;
            cx.notify();
        }
    }

    pub fn has_sessions(&self) -> bool {
        !self.sessions.is_empty()
    }

    /// Why the last spawn failed, if it did.
    #[cfg(any(test, debug_assertions))]
    pub fn last_error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    pub fn session_ids(&self, cx: &App) -> Vec<String> {
        self.sessions
            .iter()
            .map(|s| s.read(cx).info.id.clone())
            .collect()
    }

    #[cfg(any(test, debug_assertions))]
    pub fn active_id(&self) -> Option<&str> {
        self.active.as_deref()
    }

    pub fn active_session(&self, cx: &App) -> Option<Entity<TerminalView>> {
        let active = self.active.as_deref()?;
        self.sessions
            .iter()
            .find(|s| s.read(cx).info.id == active)
            .cloned()
    }

    /// True when a session's grid has keyboard focus.
    pub fn is_focused(&self, window: &Window, cx: &App) -> bool {
        self.sessions.iter().any(|s| s.read(cx).is_focused(window))
    }

    pub fn focus_active(&self, window: &mut Window, cx: &mut App) {
        if let Some(view) = self.active_session(cx) {
            let handle = view.focus_handle(cx);
            window.focus(&handle, cx);
        }
    }

    /// Spawn a shell in `cwd` (home when `None`) and make it the active tab.
    pub fn open(
        &mut self,
        cwd: Option<String>,
        project_name: Option<String>,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.new_menu_open = false;
        let spec = TerminalSpec {
            cwd: cwd.map(std::path::PathBuf::from),
            project_name,
            ..TerminalSpec::default()
        };
        match self.manager.create(spec) {
            Ok(info) => {
                let manager = self.manager.clone();
                let runtime = Arc::clone(&self.runtime);
                let id = info.id.clone();
                let view = cx.new(|cx| TerminalView::new(info, manager, runtime, cx));
                cx.observe(&view, |_, _, cx| cx.notify()).detach();
                self.sessions.push(view);
                self.active = Some(id);
                self.error = None;
                let focus_view = self.active_session(cx);
                cx.defer_in(window, move |_this, window, cx| {
                    if let Some(view) = focus_view {
                        let handle = view.focus_handle(cx);
                        window.focus(&handle, cx);
                    }
                });
            }
            Err(err) => self.error = Some(err.to_string()),
        }
        cx.notify();
    }

    /// A session with no PTY, for render tests.
    #[cfg(test)]
    pub fn push_detached(
        &mut self,
        info: bruhs_core::terminal::TerminalInfo,
        cx: &mut Context<'_, Self>,
    ) -> Entity<TerminalView> {
        let size = crate::views::terminal_view::TermSize { cols: 80, rows: 24 };
        let id = info.id.clone();
        let view = cx.new(|cx| TerminalView::detached(info, size, cx));
        cx.observe(&view, |_, _, cx| cx.notify()).detach();
        self.sessions.push(view.clone());
        self.active = Some(id);
        cx.notify();
        view
    }

    /// Open the default session for the surrounding context (the thread's
    /// project, else home). No-op under test: a real shell would wake the
    /// gpui test scheduler from another thread.
    pub fn open_default(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        if cfg!(test) {
            return;
        }
        let (name, path) = match self.context_project.clone() {
            Some((name, path)) => (Some(name), Some(path)),
            None => (None, None),
        };
        self.open(path, name, window, cx);
    }

    pub fn close(&mut self, id: &str, cx: &mut Context<'_, Self>) {
        let ids = self.session_ids(cx);
        let next = next_active_after_close(&ids, id, self.active.as_deref());
        self.sessions.retain(|s| s.read(cx).info.id != id);
        self.manager.close(id);
        self.active = next;
        cx.notify();
    }

    pub fn select(&mut self, id: &str, window: &mut Window, cx: &mut Context<'_, Self>) {
        if self.sessions.iter().any(|s| s.read(cx).info.id == id) {
            self.active = Some(id.to_string());
            self.focus_active(window, cx);
            cx.notify();
        }
    }

    fn toggle_new_menu(&mut self, cx: &mut Context<'_, Self>) {
        self.new_menu_open = !self.new_menu_open;
        cx.notify();
    }

    fn render_tab(&self, view: &Entity<TerminalView>, cx: &Context<'_, Self>) -> AnyElement {
        let session = view.read(cx);
        let id = session.info.id.clone();
        let key = crate::views::workspace::element_key(&id);
        let title = session.title();
        let cwd_hint = session.info.cwd.clone().unwrap_or_else(|| "~".to_string());
        let active = self.active.as_deref() == Some(id.as_str());
        let select_id = id.clone();
        let close_id = id;
        div()
            .id(ElementId::Name(format!("terminal-tab-{key}").into()))
            .debug_selector(move || format!("desktop-terminal-tab-{key}"))
            .h(px(TAB_HEIGHT_PX))
            .max_w(px(TAB_MAX_WIDTH_PX))
            .pl_2()
            .pr_1()
            .rounded(px(TAB_RADIUS_PX))
            .flex()
            .items_center()
            .gap_1()
            .flex_shrink_0()
            .text_size(px(HEADER_TEXT_SIZE_PX))
            .font_family(theme::UI_SMALL_FONT_FAMILY)
            .text_color(if active {
                theme::rambutan_100()
            } else {
                theme::rambutan_400()
            })
            .when(active, |el| el.bg(theme::longan_800()))
            .when(!active, |el| {
                el.hover(|el| {
                    el.bg(theme::longan_800().opacity(HOVER_BG_OPACITY))
                        .text_color(theme::rambutan_100())
                })
            })
            .tab_index(0)
            .role(Role::Tab)
            .aria_label(format!("Terminal {title} ({cwd_hint})"))
            .aria_selected(active)
            .focus_visible(|el| el.border_2().border_color(theme::persimmon_400()))
            .cursor_pointer()
            .on_click(cx.listener(move |this, _ev, window, cx| {
                cx.stop_propagation();
                this.select(&select_id, window, cx);
            }))
            .child(div().truncate().child(title))
            .child(
                div()
                    .id(ElementId::Name(format!("terminal-tab-close-{key}").into()))
                    .debug_selector(move || format!("desktop-terminal-tab-close-{key}"))
                    .size(px(16.0))
                    .rounded(px(3.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_color(theme::rambutan_500())
                    .hover(|el| el.text_color(theme::lychee_300()))
                    .tab_index(0)
                    .role(Role::Button)
                    .aria_label("Close terminal")
                    .focus_visible(|el| el.border_1().border_color(theme::persimmon_400()))
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _ev, _window, cx| {
                        cx.stop_propagation();
                        this.close(&close_id, cx);
                    }))
                    .child(
                        svg()
                            .path(assets::CLOSE_ICON_PATH)
                            .size(px(CLOSE_ICON_SIZE_PX))
                            .text_color(theme::rambutan_500()),
                    ),
            )
            .into_any_element()
    }

    fn render_menu_row(
        &self,
        selector: String,
        label: String,
        hint: String,
        icon: &'static str,
        on_pick: impl Fn(&mut Self, &mut Window, &mut Context<'_, Self>) + 'static,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        div()
            .id(ElementId::Name(selector.clone().into()))
            .debug_selector(move || selector)
            .h(px(MENU_ROW_HEIGHT_PX))
            .w_full()
            .px_2()
            .rounded(px(TAB_RADIUS_PX))
            .flex()
            .items_center()
            .gap_2()
            .text_size(px(HEADER_TEXT_SIZE_PX))
            .font_family(theme::UI_SMALL_FONT_FAMILY)
            .text_color(theme::rambutan_200())
            .hover(|el| el.bg(theme::longan_800().opacity(HOVER_BG_OPACITY)))
            .tab_index(0)
            .role(Role::MenuItem)
            .aria_label(format!("{label} — {hint}"))
            .focus_visible(|el| el.border_1().border_color(theme::persimmon_400()))
            .cursor_pointer()
            .on_click(cx.listener(move |this, _ev, window, cx| {
                cx.stop_propagation();
                on_pick(this, window, cx);
            }))
            .child(
                svg()
                    .path(icon)
                    .size(px(ICON_SIZE_PX))
                    .text_color(theme::rambutan_500())
                    .flex_shrink_0(),
            )
            .child(div().truncate().child(label))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_right()
                    .text_color(theme::rambutan_500())
                    .child(hint),
            )
            .into_any_element()
    }

    fn render_new_menu(&self, cx: &Context<'_, Self>) -> AnyElement {
        let projects = menu_projects(self.context_project.as_ref(), &self.recent_projects);
        let mut rows: Vec<AnyElement> = projects
            .into_iter()
            .map(|(name, path)| {
                let key = crate::views::workspace::element_key(&path);
                let pick_path = path.clone();
                let pick_name = name.clone();
                self.render_menu_row(
                    format!("desktop-terminal-new-project-{key}"),
                    name,
                    path,
                    assets::FOLDER_ICON_PATH,
                    move |this, window, cx| {
                        this.open(Some(pick_path.clone()), Some(pick_name.clone()), window, cx)
                    },
                    cx,
                )
            })
            .collect();
        rows.push(self.render_menu_row(
            "desktop-terminal-new-home".to_string(),
            "Home".to_string(),
            "~".to_string(),
            assets::TOOL_TERMINAL_ICON_PATH,
            |this, window, cx| this.open(None, None, window, cx),
            cx,
        ));
        rows.push(self.render_menu_row(
            "desktop-terminal-new-folder".to_string(),
            "Choose folder…".to_string(),
            String::new(),
            assets::TOOL_FOLDER_SEARCH_ICON_PATH,
            |this, _window, cx| {
                this.new_menu_open = false;
                cx.emit(TerminalPanelEvent::ChooseFolder);
                cx.notify();
            },
            cx,
        ));
        let anchor_top = match self.mode {
            TerminalPanelMode::Drawer => None,
            TerminalPanelMode::Page => Some(px(HEADER_HEIGHT_PX)),
        };
        div()
            .id("terminal-new-menu")
            .debug_selector(|| "desktop-terminal-new-menu".to_string())
            .role(Role::Menu)
            .aria_label("New terminal")
            .absolute()
            .right_2()
            .when_some(anchor_top, |el, top| el.top(top))
            .when(anchor_top.is_none(), |el| el.bottom(px(HEADER_HEIGHT_PX)))
            .w(px(MENU_WIDTH_PX))
            .p_1()
            .rounded(px(6.0))
            .bg(theme::longan_900())
            .border_1()
            .border_color(theme::ring_floating())
            .occlude()
            .flex()
            .flex_col()
            .gap_px()
            .children(rows)
            .into_any_element()
    }

    fn render_header(&self, cx: &Context<'_, Self>) -> AnyElement {
        let tabs: Vec<AnyElement> = self
            .sessions
            .iter()
            .map(|view| self.render_tab(view, cx))
            .collect();
        let status = self
            .active_session(cx)
            .map_or(TerminalStatus::Live, |v| v.read(cx).status());
        let status_label = if self.sessions.is_empty() {
            ""
        } else {
            status.label()
        };
        div()
            .h(px(HEADER_HEIGHT_PX))
            .px_2()
            .flex()
            .items_center()
            .gap_2()
            .flex_shrink_0()
            .text_size(px(HEADER_TEXT_SIZE_PX))
            .font_family(theme::UI_SMALL_FONT_FAMILY)
            .child(
                svg()
                    .path(assets::TOOL_TERMINAL_ICON_PATH)
                    .size(px(ICON_SIZE_PX))
                    .text_color(theme::rambutan_400())
                    .flex_shrink_0(),
            )
            .child(
                div()
                    .id("terminal-tabs")
                    .role(Role::TabList)
                    .aria_label("Terminals")
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .items_center()
                    .gap_1()
                    .overflow_x_hidden()
                    .children(tabs),
            )
            .child(
                div()
                    .id("terminal-new")
                    .debug_selector(|| "desktop-terminal-new".to_string())
                    .h(px(TAB_HEIGHT_PX))
                    .px(px(6.0))
                    .rounded(px(TAB_RADIUS_PX))
                    .flex()
                    .items_center()
                    .gap_1()
                    .text_color(theme::rambutan_400())
                    .hover(|el| {
                        el.bg(theme::longan_800().opacity(HOVER_BG_OPACITY))
                            .text_color(theme::rambutan_100())
                    })
                    .tab_index(0)
                    .role(Role::Button)
                    .aria_label("New terminal")
                    .aria_expanded(self.new_menu_open)
                    .focus_visible(|el| el.border_1().border_color(theme::persimmon_400()))
                    .cursor_pointer()
                    .on_click(cx.listener(|this, _ev, _window, cx| {
                        cx.stop_propagation();
                        this.toggle_new_menu(cx);
                    }))
                    .child(
                        svg()
                            .path("icons/plus.svg")
                            .size(px(ICON_SIZE_PX))
                            .text_color(theme::rambutan_400())
                            .flex_shrink_0(),
                    )
                    .child("New"),
            )
            .when(!self.sessions.is_empty(), |el| {
                el.child(
                    div()
                        .id("terminal-cite")
                        .debug_selector(|| "desktop-terminal-cite".to_string())
                        .h(px(TAB_HEIGHT_PX))
                        .px(px(6.0))
                        .rounded(px(TAB_RADIUS_PX))
                        .flex()
                        .items_center()
                        .flex_shrink_0()
                        .text_color(theme::rambutan_400())
                        .hover(|el| {
                            el.bg(theme::longan_800().opacity(HOVER_BG_OPACITY))
                                .text_color(theme::rambutan_100())
                        })
                        .tab_index(0)
                        .role(Role::Button)
                        .aria_label("Cite in composer")
                        .focus_visible(|el| el.border_1().border_color(theme::persimmon_400()))
                        .cursor_pointer()
                        .on_click(cx.listener(|this, _ev, _window, cx| {
                            cx.stop_propagation();
                            let Some(session) = this.active_session(cx) else {
                                return;
                            };
                            let excerpt = session.read(cx).selected_text();
                            let Some(excerpt) = excerpt else {
                                return;
                            };
                            let title = session.read(cx).title();
                            cx.emit(TerminalPanelEvent::CiteExcerpt {
                                label: format!("Terminal excerpt, {title}"),
                                excerpt,
                            });
                        }))
                        .child("Cite"),
                )
            })
            .child(
                div()
                    .debug_selector(|| "desktop-terminal-status".to_string())
                    .flex_shrink_0()
                    .text_color(if status == TerminalStatus::Live {
                        theme::pandan_400()
                    } else {
                        theme::rambutan_500()
                    })
                    .child(status_label),
            )
            .when(self.mode == TerminalPanelMode::Drawer, |el| {
                el.child(
                    div()
                        .id("terminal-hide")
                        .debug_selector(|| "desktop-terminal-close-drawer".to_string())
                        .size(px(TAB_HEIGHT_PX))
                        .rounded(px(TAB_RADIUS_PX))
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_color(theme::rambutan_400())
                        .hover(|el| {
                            el.bg(theme::longan_800().opacity(HOVER_BG_OPACITY))
                                .text_color(theme::rambutan_100())
                        })
                        .tab_index(0)
                        .role(Role::Button)
                        .aria_label("Hide terminal (⌘J)")
                        .focus_visible(|el| el.border_1().border_color(theme::persimmon_400()))
                        .cursor_pointer()
                        .on_click(cx.listener(|_this, _ev, _window, cx| {
                            cx.stop_propagation();
                            cx.emit(TerminalPanelEvent::HideDrawer);
                        }))
                        .child(
                            svg()
                                .path(assets::CLOSE_ICON_PATH)
                                .size(px(ICON_SIZE_PX))
                                .text_color(theme::rambutan_400()),
                        ),
                )
            })
            .into_any_element()
    }

    fn render_empty_row(
        &self,
        selector: String,
        label: String,
        hint: String,
        on_pick: impl Fn(&mut Self, &mut Window, &mut Context<'_, Self>) + 'static,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        div()
            .id(ElementId::Name(selector.clone().into()))
            .debug_selector(move || selector)
            .w_full()
            .px_3()
            .py(px(6.0))
            .rounded(px(TAB_RADIUS_PX))
            .bg(theme::longan_900())
            .border_1()
            .border_color(theme::ring_card())
            .flex()
            .items_center()
            .justify_between()
            .gap_3()
            .text_size(px(EMPTY_HINT_TEXT_SIZE_PX))
            .font_family(theme::UI_SMALL_FONT_FAMILY)
            .text_color(theme::rambutan_200())
            .hover(|el| el.bg(theme::longan_800().opacity(0.7)))
            .tab_index(0)
            .role(Role::Button)
            .aria_label(format!("Open terminal in {label}"))
            .focus_visible(|el| el.border_color(theme::persimmon_400()))
            .cursor_pointer()
            .on_click(cx.listener(move |this, _ev, window, cx| {
                cx.stop_propagation();
                on_pick(this, window, cx);
            }))
            .child(div().truncate().child(label))
            .child(
                div()
                    .min_w_0()
                    .truncate()
                    .text_color(theme::rambutan_500())
                    .child(hint),
            )
            .into_any_element()
    }

    fn render_empty(&self, cx: &Context<'_, Self>) -> AnyElement {
        let projects = menu_projects(self.context_project.as_ref(), &self.recent_projects);
        let mut rows: Vec<AnyElement> = projects
            .into_iter()
            .map(|(name, path)| {
                let key = crate::views::workspace::element_key(&path);
                let pick_path = path.clone();
                let pick_name = name.clone();
                self.render_empty_row(
                    format!("desktop-terminal-empty-project-{key}"),
                    name,
                    path,
                    move |this, window, cx| {
                        this.open(Some(pick_path.clone()), Some(pick_name.clone()), window, cx)
                    },
                    cx,
                )
            })
            .collect();
        rows.push(self.render_empty_row(
            "desktop-terminal-empty-home".to_string(),
            "Home".to_string(),
            "~".to_string(),
            |this, window, cx| this.open(None, None, window, cx),
            cx,
        ));
        div()
            .debug_selector(|| "desktop-terminal-empty".to_string())
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap_4()
            .p_6()
            .child(
                svg()
                    .path(assets::TOOL_TERMINAL_ICON_PATH)
                    .size(px(18.0))
                    .text_color(theme::rambutan_400()),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap_1()
                    .child(
                        div()
                            .text_size(px(EMPTY_TEXT_SIZE_PX))
                            .text_color(theme::rambutan_100())
                            .child("Open a terminal"),
                    )
                    .child(
                        div()
                            .text_size(px(EMPTY_HINT_TEXT_SIZE_PX))
                            .font_family(theme::UI_SMALL_FONT_FAMILY)
                            .text_color(theme::rambutan_500())
                            .child("A login shell on this machine. Pick where it starts."),
                    ),
            )
            .child(
                div()
                    .w(px(EMPTY_ROW_WIDTH_PX))
                    .max_w_full()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .children(rows)
                    .child(
                        div()
                            .id("terminal-empty-folder")
                            .debug_selector(|| "desktop-terminal-empty-folder".to_string())
                            .mt_1()
                            .text_size(px(EMPTY_HINT_TEXT_SIZE_PX))
                            .font_family(theme::UI_SMALL_FONT_FAMILY)
                            .text_color(theme::rambutan_400())
                            .hover(|el| el.text_color(theme::rambutan_100()))
                            .tab_index(0)
                            .role(Role::Button)
                            .aria_label("Choose a folder for a new terminal")
                            .focus_visible(|el| el.border_1().border_color(theme::persimmon_400()))
                            .cursor_pointer()
                            .text_center()
                            .on_click(cx.listener(|_this, _ev, _window, cx| {
                                cx.stop_propagation();
                                cx.emit(TerminalPanelEvent::ChooseFolder);
                            }))
                            .child("Choose a folder…"),
                    ),
            )
            .when_some(self.error.clone(), |el, err| {
                el.child(
                    div()
                        .debug_selector(|| "desktop-terminal-error".to_string())
                        .text_size(px(EMPTY_HINT_TEXT_SIZE_PX))
                        .font_family(theme::UI_SMALL_FONT_FAMILY)
                        .text_color(theme::lychee_300())
                        .child(err),
                )
            })
            .into_any_element()
    }
}

impl Render for TerminalPanelView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let body: AnyElement = match self.active_session(cx) {
            Some(view) => div().flex_1().min_h_0().child(view).into_any_element(),
            None => self.render_empty(cx),
        };
        let drawer = self.mode == TerminalPanelMode::Drawer;
        div()
            .id("terminal-panel")
            .debug_selector(move || {
                if drawer {
                    "desktop-terminal-drawer".to_string()
                } else {
                    "desktop-terminal-page".to_string()
                }
            })
            .role(Role::Region)
            .aria_label("Terminal")
            .relative()
            .w_full()
            // The workspace sizes the drawer (a resizable split); the page
            // fills its column.
            .h_full()
            .min_h_0()
            .when(drawer, |el| {
                el.border_t_1()
                    .border_color(theme::longan_800().opacity(BORDER_OPACITY))
            })
            .when(!drawer, |el| el.flex_1())
            .bg(theme::longan_950())
            .flex()
            .flex_col()
            .on_mouse_down_out(cx.listener(|this, _ev, _window, cx| {
                if this.new_menu_open {
                    this.new_menu_open = false;
                    cx.notify();
                }
            }))
            .child(self.render_header(cx))
            .child(body)
            .when(self.new_menu_open, |el| el.child(self.render_new_menu(cx)))
    }
}

#[cfg(test)]
mod tests {
    use super::{menu_projects, next_active_after_close};

    fn ids(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn next_active_prefers_survivor_then_right_then_left() {
        let all = ids(&["a", "b", "c"]);
        assert_eq!(
            next_active_after_close(&all, "b", Some("a")),
            Some("a".into())
        );
        assert_eq!(
            next_active_after_close(&all, "b", Some("b")),
            Some("c".into())
        );
        assert_eq!(
            next_active_after_close(&all, "c", Some("c")),
            Some("b".into())
        );
        assert_eq!(next_active_after_close(&ids(&["a"]), "a", Some("a")), None);
        assert_eq!(next_active_after_close(&all, "zzz", Some("zzz")), None);
    }

    #[test]
    fn drawer_height_clamps_like_web() {
        use super::clamp_terminal_drawer_height as clamp;
        assert_eq!(clamp(300.0), 300.0);
        assert_eq!(clamp(10.0), 120.0);
        assert_eq!(clamp(5000.0), 800.0);
        assert_eq!(clamp(f32::NAN), 300.0);
    }

    #[test]
    fn menu_projects_dedupes_and_limits_recents() {
        let ctx = ("proj".to_string(), "/p".to_string());
        let recents: Vec<(String, String)> = (0..8)
            .map(|i| (format!("r{i}"), format!("/r{i}")))
            .chain(std::iter::once(("dup".to_string(), "/p".to_string())))
            .collect();
        let out = menu_projects(Some(&ctx), &recents);
        assert_eq!(out.len(), 6);
        assert_eq!(out[0].1, "/p");
        assert!(out.iter().filter(|p| p.1 == "/p").count() == 1);
        assert!(menu_projects(None, &[]).is_empty());
    }
}

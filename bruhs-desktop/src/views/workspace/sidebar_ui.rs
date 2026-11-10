//! The thread sidebar, 1:1 with `bruhs-web/src/components/sidebar.rs`:
//! brand header, search + new-thread row, project filter + new-project row,
//! three-line thread cards (project · title · branch + provider) with the
//! Settled fold, and the footer view switcher. Sizes are the web's Tailwind
//! values in px; colours are the same bruhs tokens.

use gpui::prelude::*;
use gpui::*;
use gpui_component::input::{Input, InputEvent, InputState};
use gpui_component::scroll::ScrollableElement;

use super::{
    element_key, is_web_button_activation_key, provider_mark, redraw_on_edit, threads_ui,
    DesktopThread, KeyDownEvent, OpenProjectPicker, OpenSettings, SidebarSection, Workspace,
    WORKSPACE_SIDEBAR_BORDER_OPACITY,
};
use crate::views::sidebar_order::relative_age;
use crate::{assets, theme};
use bruhs_models::logic::sidebar::{snooze_after, snooze_presets, SnoozeUnit};
use bruhs_models::{AppView, EpochMs, SidebarTab};

/// `w-[272px]`.
pub const SIDEBAR_WIDTH_PX: f32 = 272.0;
/// `h-[52px]` brand header.
pub const SIDEBAR_HEADER_HEIGHT_PX: f32 = 52.0;
/// Room for the macOS traffic lights (gpui-component puts them at x=12).
#[cfg(target_os = "macos")]
pub const SIDEBAR_HEADER_LEFT_INSET_PX: f32 = 88.0;
#[cfg(not(target_os = "macos"))]
pub const SIDEBAR_HEADER_LEFT_INSET_PX: f32 = 16.0;
const SIDEBAR_HEADER_GAP_PX: f32 = 8.0;
const SIDEBAR_BRAND_LOGO_PX: f32 = 20.0;
const SIDEBAR_BRAND_TEXT_PX: f32 = 14.0;
const SIDEBAR_CONTROLS_X_PADDING_PX: f32 = 8.0;
const SIDEBAR_CONTROLS_BOTTOM_PADDING_PX: f32 = 8.0;
const SIDEBAR_CONTROLS_GAP_PX: f32 = 4.0;
const SIDEBAR_SEARCH_HEIGHT_PX: f32 = 32.0;
const SIDEBAR_SEARCH_TEXT_PX: f32 = 14.0;
const SIDEBAR_ICON_BUTTON_PX: f32 = 32.0;
const SIDEBAR_SMALL_BUTTON_PX: f32 = 28.0;
const SIDEBAR_FILTER_HEIGHT_PX: f32 = 28.0;
const SIDEBAR_FILTER_MAX_WIDTH_PX: f32 = 208.0;
const SIDEBAR_FILTER_MENU_WIDTH_PX: f32 = 240.0;
const SIDEBAR_FILTER_MENU_ROW_PX: f32 = 28.0;
const SIDEBAR_LABEL_TEXT_PX: f32 = 12.0;
const SIDEBAR_ICON_PX: f32 = 14.0;
const SIDEBAR_SMALL_ICON_PX: f32 = 12.0;
const SIDEBAR_RADIUS_GRAPE_PX: f32 = 8.0;
const SIDEBAR_RADIUS_LYCHEE_PX: f32 = 14.0;
const SIDEBAR_RADIUS_SEED_PX: f32 = 4.0;
const SIDEBAR_LIST_X_PADDING_PX: f32 = 8.0;
const SIDEBAR_LIST_BOTTOM_PADDING_PX: f32 = 8.0;
const SIDEBAR_LIST_GAP_PX: f32 = 4.0;
const SIDEBAR_EMPTY_Y_PADDING_PX: f32 = 24.0;
const SIDEBAR_CARD_X_PADDING_PX: f32 = 10.0;
const SIDEBAR_CARD_Y_PADDING_PX: f32 = 10.0;
const SIDEBAR_CARD_LINE_HEIGHT_PX: f32 = 20.0;
const SIDEBAR_CARD_LINE_GAP_PX: f32 = 4.0;
const SIDEBAR_CARD_META_HEIGHT_PX: f32 = 16.0;
/// Web: `max-w-24` on the role name.
const SIDEBAR_CARD_ROLE_MAX_WIDTH_PX: f32 = 96.0;
const SIDEBAR_CARD_TITLE_TEXT_PX: f32 = 14.0;
const SIDEBAR_CARD_ROW_GAP_PX: f32 = 6.0;
const SIDEBAR_CARD_DOT_PX: f32 = 9.0;
const SIDEBAR_CARD_DELETE_PX: f32 = 24.0;
const SIDEBAR_CARD_DELETE_INSET_PX: f32 = 8.0;
const SIDEBAR_SETTLED_TOP_PADDING_PX: f32 = 8.0;
const SIDEBAR_SETTLED_X_PADDING_PX: f32 = 4.0;
const SIDEBAR_FOOTER_PADDING_PX: f32 = 8.0;
const SIDEBAR_FOOTER_GAP_PX: f32 = 2.0;
const SIDEBAR_FOOTER_ICON_PX: f32 = 16.0;
const HOVER_BG_OPACITY_40: f32 = 0.40;
const HOVER_BG_OPACITY_50: f32 = 0.50;
const ACTIVE_BG_OPACITY_60: f32 = 0.60;
const RING_WHITE_10: f32 = 0.10;

pub const SIDEBAR_BRAND_LABEL: &str = "bruhs.ai";
pub const SIDEBAR_SEARCH_PLACEHOLDER: &str = "Search";
pub const SIDEBAR_FILTER_ALL_LABEL: &str = "All projects";
pub const SIDEBAR_FILTER_CHAT_LABEL: &str = "Chat";
pub const SIDEBAR_EMPTY_LABEL: &str = "No threads yet. Press ⌘⇧O to start one.";
pub const SIDEBAR_NO_MATCH_LABEL: &str = "No threads match.";
pub const SIDEBAR_NEW_THREAD_LABEL: &str = "New thread (⌘⇧O)";
pub const SIDEBAR_NEW_PROJECT_LABEL: &str = "New session in a project";
pub const SIDEBAR_FILTER_LABEL: &str = "Filter threads by project";
pub const SIDEBAR_DELETE_LABEL: &str = "Delete thread";

/// What a card shows at its right edge instead of the age.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CardStatus {
    Working,
    Approval,
    Unread,
    Idle,
}

/// Web `SidebarProjectFilter` for the Code tab: every project's threads, or
/// one project's. (The web's "general chats" choice is the Chat tab here.)
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum SidebarFilter {
    #[default]
    All,
    Project(String),
}

impl SidebarFilter {
    pub fn matches(&self, project_name: &str) -> bool {
        match self {
            Self::All => true,
            Self::Project(name) => project_name == name,
        }
    }

    /// Label of the filter trigger for this selection.
    pub fn label(&self) -> String {
        match self {
            Self::All => SIDEBAR_FILTER_ALL_LABEL.to_string(),
            Self::Project(name) => name.clone(),
        }
    }
}

/// Web `visible_threads`: the project filter and a case-insensitive title
/// search.
pub fn thread_matches(
    project_name: &str,
    title: &str,
    filter: &SidebarFilter,
    query: &str,
) -> bool {
    let q = query.trim().to_lowercase();
    filter.matches(project_name) && (q.is_empty() || title.to_lowercase().contains(&q))
}

/// Distinct project names, most recent activity first (web `project_names`).
pub fn project_names(threads: &[DesktopThread]) -> Vec<String> {
    let mut seen: Vec<(String, EpochMs)> = Vec::new();
    for t in threads {
        if t.project_name.is_empty() {
            continue;
        }
        match seen.iter_mut().find(|(n, _)| *n == t.project_name) {
            Some(entry) => entry.1 = entry.1.max(t.updated_at_ms),
            None => seen.push((t.project_name.clone(), t.updated_at_ms)),
        }
    }
    seen.sort_by_key(|entry| std::cmp::Reverse(entry.1));
    seen.into_iter().map(|(n, _)| n).collect()
}

fn icon(path: &'static str, size: f32, color: Hsla) -> Svg {
    svg()
        .path(path)
        .size(px(size))
        .text_color(color)
        .flex_shrink_0()
}

impl Workspace {
    pub(super) fn sidebar_query(&self, cx: &App) -> String {
        self.sidebar_search.read(cx).value().to_string()
    }

    pub(super) fn render_sidebar(&self, cx: &Context<'_, Self>) -> Div {
        div()
            .w_full()
            .h_full()
            .relative()
            .debug_selector(|| "desktop-workspace-sidebar".to_string())
            .key_context("Sidebar")
            .bg(theme::longan_900())
            .border_r_1()
            .border_color(theme::longan_800().opacity(WORKSPACE_SIDEBAR_BORDER_OPACITY))
            .font_family(theme::UI_FONT_FAMILY)
            .text_color(theme::rambutan_300())
            .flex()
            .flex_col()
            .on_mouse_down_out(cx.listener(|this, _ev, _window, cx| {
                let mut changed = false;
                if this.sidebar_filter_open {
                    this.sidebar_filter_open = false;
                    changed = true;
                }
                if this.thread_menu_id.is_some() {
                    this.thread_menu_id = None;
                    changed = true;
                }
                if changed {
                    cx.notify();
                }
            }))
            .child(self.render_sidebar_header())
            .child(self.render_sidebar_controls(cx))
            .child(self.render_sidebar_list(cx))
            .child(self.render_sidebar_undo(cx))
            .children(self.render_sidebar_update_button(cx))
            .child(self.render_sidebar_footer(cx))
            .when(self.thread_menu_id.is_some(), |el| {
                el.child(self.render_thread_menu(cx))
            })
            .when(self.sidebar_filter_open, |el| {
                el.child(self.render_sidebar_filter_menu(cx))
            })
    }

    /// Brand row; doubles as the window drag strip behind the traffic lights.
    fn render_sidebar_header(&self) -> AnyElement {
        div()
            .id("sidebar-header")
            .debug_selector(|| "desktop-sidebar-header".to_string())
            .h(px(SIDEBAR_HEADER_HEIGHT_PX))
            .pl(px(SIDEBAR_HEADER_LEFT_INSET_PX))
            .pr_4()
            .flex()
            .items_center()
            .gap(px(SIDEBAR_HEADER_GAP_PX))
            .flex_shrink_0()
            .window_control_area(WindowControlArea::Drag)
            .child(img(assets::BRAND_LOGO_PATH).size(px(SIDEBAR_BRAND_LOGO_PX)))
            .child(
                div()
                    .text_size(px(SIDEBAR_BRAND_TEXT_PX))
                    .text_color(theme::rambutan_100())
                    .child(SIDEBAR_BRAND_LABEL),
            )
            .into_any_element()
    }

    fn sidebar_icon_button(
        &self,
        id: &'static str,
        label: &'static str,
        icon_path: &'static str,
        size: f32,
        on_activate: impl Fn(&mut Self, &mut Window, &mut Context<'_, Self>) + 'static + Clone,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        let on_key = on_activate.clone();
        div()
            .id(id)
            .debug_selector(move || id.to_string())
            .size(px(size))
            .rounded(px(SIDEBAR_RADIUS_GRAPE_PX))
            .flex()
            .items_center()
            .justify_center()
            .flex_shrink_0()
            .text_color(theme::rambutan_400())
            .hover(|el| {
                el.bg(theme::longan_800().opacity(HOVER_BG_OPACITY_50))
                    .text_color(theme::rambutan_100())
            })
            .tab_index(0)
            .role(Role::Button)
            .aria_label(label)
            .focus_visible(|el| el.border_2().border_color(theme::persimmon_400()))
            .cursor_pointer()
            .on_click(cx.listener(move |this, _ev, window, cx| {
                cx.stop_propagation();
                on_activate(this, window, cx);
            }))
            .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
                if is_web_button_activation_key(event.keystroke.key.as_str()) {
                    window.prevent_default();
                    cx.stop_propagation();
                    on_key(this, window, cx);
                }
            }))
            .child(icon(icon_path, SIDEBAR_ICON_PX, theme::rambutan_400()))
            .into_any_element()
    }

    /// Search + new thread, then the project filter + new project session.
    fn render_sidebar_controls(&self, cx: &Context<'_, Self>) -> Div {
        let filter_text = self.sidebar_filter.label();
        div()
            .px(px(SIDEBAR_CONTROLS_X_PADDING_PX))
            .pb(px(SIDEBAR_CONTROLS_BOTTOM_PADDING_PX))
            .flex()
            .flex_col()
            .gap(px(SIDEBAR_CONTROLS_GAP_PX))
            .flex_shrink_0()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(SIDEBAR_CONTROLS_GAP_PX))
                    .child(
                        div()
                            .id("sidebar-search")
                            .debug_selector(|| "desktop-sidebar-search".to_string())
                            .h(px(SIDEBAR_SEARCH_HEIGHT_PX))
                            .min_w_0()
                            .flex_1()
                            .px_2()
                            .rounded(px(SIDEBAR_RADIUS_GRAPE_PX))
                            .flex()
                            .items_center()
                            .gap_2()
                            .text_color(theme::rambutan_400())
                            .hover(|el| el.bg(theme::longan_800().opacity(HOVER_BG_OPACITY_40)))
                            .child(icon(
                                assets::TOOL_SEARCH_ICON_PATH,
                                SIDEBAR_ICON_PX,
                                theme::rambutan_500(),
                            ))
                            .child(
                                div().min_w_0().flex_1().child(
                                    Input::new(&self.sidebar_search)
                                        .aria_label("Search threads")
                                        .appearance(false)
                                        .bordered(false)
                                        .focus_bordered(false)
                                        .cleanable(false)
                                        .font_family(theme::CODE_FONT_FAMILY)
                                        .text_size(px(SIDEBAR_SEARCH_TEXT_PX)),
                                ),
                            ),
                    )
                    .child(self.sidebar_icon_button(
                        "desktop-sidebar-new-thread",
                        SIDEBAR_NEW_THREAD_LABEL,
                        assets::SQUARE_PEN_ICON_PATH,
                        SIDEBAR_ICON_BUTTON_PX,
                        |this, window, cx| this.new_thread(&super::NewThread, window, cx),
                        cx,
                    )),
            )
            .when(self.sidebar_tab == SidebarTab::Code, |el| {
                el.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(SIDEBAR_CONTROLS_GAP_PX))
                        .child(
                            div()
                                .id("sidebar-filter")
                                .debug_selector(|| "desktop-sidebar-filter".to_string())
                                .h(px(SIDEBAR_FILTER_HEIGHT_PX))
                                .max_w(px(SIDEBAR_FILTER_MAX_WIDTH_PX))
                                .px_2()
                                .rounded(px(SIDEBAR_RADIUS_GRAPE_PX))
                                .flex()
                                .items_center()
                                .gap(px(SIDEBAR_CARD_ROW_GAP_PX))
                                .text_size(px(SIDEBAR_LABEL_TEXT_PX))
                                .font_family(theme::UI_SMALL_FONT_FAMILY)
                                .text_color(theme::rambutan_300())
                                .hover(|el| {
                                    el.bg(theme::longan_800().opacity(HOVER_BG_OPACITY_50))
                                        .text_color(theme::rambutan_100())
                                })
                                .tab_index(0)
                                .role(Role::Button)
                                .aria_label(SIDEBAR_FILTER_LABEL)
                                .aria_expanded(self.sidebar_filter_open)
                                .focus_visible(|el| {
                                    el.border_2().border_color(theme::persimmon_400())
                                })
                                .cursor_pointer()
                                .on_click(cx.listener(|this, _ev, _window, cx| {
                                    cx.stop_propagation();
                                    this.sidebar_filter_open = !this.sidebar_filter_open;
                                    cx.notify();
                                }))
                                .on_key_down(cx.listener(
                                    |this, event: &KeyDownEvent, window, cx| {
                                        if is_web_button_activation_key(
                                            event.keystroke.key.as_str(),
                                        ) {
                                            window.prevent_default();
                                            cx.stop_propagation();
                                            this.sidebar_filter_open = !this.sidebar_filter_open;
                                            cx.notify();
                                        }
                                    },
                                ))
                                .child(icon(
                                    assets::FOLDER_ICON_PATH,
                                    SIDEBAR_ICON_PX,
                                    theme::rambutan_500(),
                                ))
                                .child(div().min_w_0().truncate().child(filter_text))
                                .child(icon(
                                    assets::CHEVRON_DOWN_ICON_PATH,
                                    SIDEBAR_SMALL_ICON_PX,
                                    theme::rambutan_500(),
                                )),
                        )
                        .child(div().flex_1())
                        .child(self.sidebar_icon_button(
                            "desktop-sidebar-new-project",
                            SIDEBAR_NEW_PROJECT_LABEL,
                            assets::FOLDER_PLUS_ICON_PATH,
                            SIDEBAR_SMALL_BUTTON_PX,
                            |this, window, cx| {
                                this.open_project_picker(&OpenProjectPicker, window, cx)
                            },
                            cx,
                        )),
                )
            })
    }

    fn render_sidebar_filter_row(
        &self,
        selector: String,
        label: String,
        icon_path: Option<&'static str>,
        selected: bool,
        filter: SidebarFilter,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        let filter_for_key = filter.clone();
        div()
            .id(ElementId::Name(selector.clone().into()))
            .debug_selector(move || selector)
            .h(px(SIDEBAR_FILTER_MENU_ROW_PX))
            .w_full()
            .px_2()
            .rounded(px(SIDEBAR_RADIUS_SEED_PX))
            .flex()
            .items_center()
            .gap_2()
            .text_size(px(SIDEBAR_LABEL_TEXT_PX))
            .font_family(theme::UI_SMALL_FONT_FAMILY)
            .text_color(if selected {
                theme::rambutan_100()
            } else {
                theme::rambutan_200()
            })
            .when(selected, |el| {
                el.bg(theme::longan_800().opacity(HOVER_BG_OPACITY_50))
            })
            .hover(|el| el.bg(theme::longan_800().opacity(ACTIVE_BG_OPACITY_60)))
            .tab_index(0)
            .role(Role::MenuItem)
            .aria_label(label.clone())
            .aria_selected(selected)
            .focus_visible(|el| el.border_1().border_color(theme::persimmon_400()))
            .cursor_pointer()
            .on_click(cx.listener(move |this, _ev, _window, cx| {
                cx.stop_propagation();
                this.sidebar_filter = filter.clone();
                this.sidebar_filter_open = false;
                cx.notify();
            }))
            .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
                if is_web_button_activation_key(event.keystroke.key.as_str()) {
                    window.prevent_default();
                    cx.stop_propagation();
                    this.sidebar_filter = filter_for_key.clone();
                    this.sidebar_filter_open = false;
                    cx.notify();
                }
            }))
            .when_some(icon_path, |el, path| {
                el.child(icon(path, SIDEBAR_SMALL_ICON_PX, theme::rambutan_500()))
            })
            .child(div().min_w_0().truncate().child(label))
            .into_any_element()
    }

    /// Floating project list; Chat/Code navigation lives above the canvas.
    fn render_sidebar_filter_menu(&self, cx: &Context<'_, Self>) -> AnyElement {
        let names = project_names(&self.threads);
        let current = self.sidebar_filter.clone();
        let mut rows: Vec<AnyElement> = vec![self.render_sidebar_filter_row(
            "desktop-sidebar-filter-all".to_string(),
            SIDEBAR_FILTER_ALL_LABEL.to_string(),
            None,
            current == SidebarFilter::All,
            SidebarFilter::All,
            cx,
        )];
        for name in names {
            let key = element_key(&name);
            rows.push(self.render_sidebar_filter_row(
                format!("desktop-sidebar-filter-{key}"),
                name.clone(),
                Some(assets::FOLDER_ICON_PATH),
                matches!(&current, SidebarFilter::Project(selected) if *selected == name),
                SidebarFilter::Project(name),
                cx,
            ));
        }
        div()
            .id("sidebar-filter-menu")
            .debug_selector(|| "desktop-sidebar-filter-menu".to_string())
            .role(Role::Menu)
            .aria_label(SIDEBAR_FILTER_LABEL)
            .absolute()
            .left(px(SIDEBAR_CONTROLS_X_PADDING_PX))
            .top(px(SIDEBAR_HEADER_HEIGHT_PX
                + SIDEBAR_SEARCH_HEIGHT_PX
                + SIDEBAR_CONTROLS_GAP_PX
                + SIDEBAR_FILTER_HEIGHT_PX
                + SIDEBAR_CONTROLS_GAP_PX))
            .w(px(SIDEBAR_FILTER_MENU_WIDTH_PX))
            .p_1()
            .rounded(px(SIDEBAR_RADIUS_GRAPE_PX))
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

    /// Pinned and active cards, then the Working, Snoozed, and Settled folds.
    fn render_sidebar_list(&self, cx: &Context<'_, Self>) -> AnyElement {
        let shelves = self.sidebar_shelves(cx);
        let searching = !self.sidebar_query(cx).trim().is_empty();
        let empty = shelves.pinned.is_empty()
            && shelves.active.is_empty()
            && shelves.working.is_empty()
            && shelves.snoozed.is_empty()
            && shelves.settled.is_empty();
        let cards: Vec<AnyElement> = shelves
            .pinned
            .iter()
            .chain(shelves.active.iter())
            .map(|thread| self.render_thread_card(thread, cx))
            .collect();
        div()
            .id("sidebar-threads")
            .debug_selector(|| "desktop-sidebar-threads".to_string())
            .role(Role::List)
            .aria_label("Threads")
            .flex_1()
            .min_h_0()
            .overflow_y_scrollbar()
            .px(px(SIDEBAR_LIST_X_PADDING_PX))
            .pb(px(SIDEBAR_LIST_BOTTOM_PADDING_PX))
            .flex()
            .flex_col()
            .gap(px(SIDEBAR_LIST_GAP_PX))
            .when(empty, |el| {
                el.child(
                    div()
                        .debug_selector(|| "desktop-sidebar-empty".to_string())
                        .px_2()
                        .py(px(SIDEBAR_EMPTY_Y_PADDING_PX))
                        .text_center()
                        .text_size(px(SIDEBAR_LABEL_TEXT_PX))
                        .font_family(theme::UI_SMALL_FONT_FAMILY)
                        .text_color(theme::rambutan_500())
                        .child(if searching {
                            SIDEBAR_NO_MATCH_LABEL
                        } else {
                            SIDEBAR_EMPTY_LABEL
                        }),
                )
            })
            .children(cards)
            .children(self.render_shelf(
                "desktop-sidebar-working-fold",
                format!("Working ({})", shelves.working.len()),
                self.sidebar_working_open,
                SidebarSection::Working,
                &shelves.working,
                cx,
            ))
            .children(self.render_shelf(
                "desktop-sidebar-snoozed-fold",
                format!("Snoozed ({})", shelves.snoozed.len()),
                self.sidebar_snoozed_open,
                SidebarSection::Snoozed,
                &shelves.snoozed,
                cx,
            ))
            .children(self.render_shelf(
                "desktop-sidebar-settled-fold",
                format!("Settled ({})", shelves.settled.len()),
                self.sidebar_settled_open,
                SidebarSection::Settled,
                &shelves.settled,
                cx,
            ))
            .into_any_element()
    }

    /// A folded shelf. An empty shelf takes no space.
    fn render_shelf(
        &self,
        id: &'static str,
        label: String,
        open: bool,
        section: SidebarSection,
        threads: &[&DesktopThread],
        cx: &Context<'_, Self>,
    ) -> Option<AnyElement> {
        if threads.is_empty() {
            return None;
        }
        let header = div()
            .id(id)
            .debug_selector(move || id.to_string())
            .mt(px(SIDEBAR_SETTLED_TOP_PADDING_PX))
            .px(px(SIDEBAR_SETTLED_X_PADDING_PX))
            .py_1()
            .w_full()
            .flex()
            .items_center()
            .gap_2()
            .text_size(px(SIDEBAR_LABEL_TEXT_PX))
            .font_family(theme::UI_SMALL_FONT_FAMILY)
            .text_color(theme::rambutan_500())
            .tab_index(0)
            .role(Role::Button)
            .aria_expanded(open)
            .aria_label(label.clone())
            .focus_visible(|el| el.border_2().border_color(theme::persimmon_400()))
            .cursor_pointer()
            .hover(|el| el.text_color(theme::rambutan_300()))
            .on_click(cx.listener(move |this, _ev, _window, cx| this.toggle_shelf(section, cx)))
            .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
                if is_web_button_activation_key(event.keystroke.key.as_str()) {
                    window.prevent_default();
                    cx.stop_propagation();
                    this.toggle_shelf(section, cx);
                }
            }))
            .child(div().flex_shrink_0().child(label))
            .child(
                div()
                    .flex_1()
                    .h(px(1.0))
                    .bg(theme::longan_800().opacity(0.8)),
            )
            .child(
                icon(
                    assets::CHEVRON_DOWN_ICON_PATH,
                    SIDEBAR_SMALL_ICON_PX,
                    theme::rambutan_500(),
                )
                .when(open, |el| {
                    el.with_transformation(Transformation::rotate(Radians(std::f32::consts::PI)))
                }),
            );
        let cards: Vec<AnyElement> = if open {
            threads
                .iter()
                .map(|thread| self.render_thread_card(thread, cx))
                .collect()
        } else {
            Vec::new()
        };
        Some(
            div()
                .debug_selector(move || format!("{id}-shelf"))
                .flex()
                .flex_col()
                .gap(px(SIDEBAR_LIST_GAP_PX))
                .child(header)
                .children(cards)
                .into_any_element(),
        )
    }

    /// Working / approval / unread / idle, like the web card's status dot.
    fn card_status(&self, thread: &DesktopThread, cx: &App) -> CardStatus {
        let chat = thread.chat.read(cx);
        if chat.is_streaming && chat.has_pending_permission() {
            CardStatus::Approval
        } else if chat.is_streaming {
            CardStatus::Working
        } else if self.unread_thread_ids.contains(&thread.id) {
            CardStatus::Unread
        } else {
            CardStatus::Idle
        }
    }

    /// One thread card: project (or "Chat") + pin + age/status, the title,
    /// then branch + provider glyph. Delete shows on hover at the top right.
    pub(super) fn render_thread_card(
        &self,
        thread: &DesktopThread,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        let chat = thread.chat.read(cx);
        let title = bruhs_models::logic::text::plain_title(&chat.thread_title).into_owned();
        let is_active = self.active_thread_id.as_deref() == Some(thread.id.as_str());
        let status = self.card_status(thread, cx);
        let key = thread.chat.entity_id();
        let debug_key = element_key(&thread.id);
        let row_group: SharedString = format!("desktop-thread-row-{key}").into();
        let age = relative_age(thread.updated_at_ms, EpochMs::now());
        let project_label = if thread.project_name.is_empty() {
            SIDEBAR_FILTER_CHAT_LABEL.to_string()
        } else {
            thread.project_name.clone()
        };
        let provider = bruhs_models::agent::AgentProvider::for_model(&thread.model_id);
        let id_for_select = thread.id.clone();
        let id_for_select_key = thread.id.clone();
        let id_for_delete = thread.id.clone();
        let id_for_delete_key = thread.id.clone();
        let id_for_menu = thread.id.clone();

        let status_slot: AnyElement = match status {
            CardStatus::Idle => div()
                .debug_selector(move || format!("desktop-thread-card-age-{debug_key}"))
                .child(age)
                .into_any_element(),
            other => {
                let (color, label) = match other {
                    CardStatus::Working => (theme::blueberry_400(), "Working"),
                    CardStatus::Approval => (theme::durian_400(), "Waiting for approval"),
                    _ => (theme::pandan_400(), "Ready"),
                };
                div()
                    .debug_selector(move || format!("desktop-thread-card-status-{debug_key}"))
                    .size(px(SIDEBAR_CARD_DOT_PX))
                    .rounded_full()
                    .bg(color)
                    .id(("desktop-thread-card-status", key))
                    .role(Role::Image)
                    .aria_label(label)
                    .into_any_element()
            }
        };

        let top_line = div()
            .h(px(SIDEBAR_CARD_LINE_HEIGHT_PX))
            .flex()
            .items_center()
            .gap(px(SIDEBAR_CARD_ROW_GAP_PX))
            .text_size(px(SIDEBAR_LABEL_TEXT_PX))
            .font_family(theme::UI_SMALL_FONT_FAMILY)
            .text_color(theme::rambutan_500())
            .child(icon(
                assets::FOLDER_ICON_PATH,
                SIDEBAR_ICON_PX,
                theme::rambutan_500(),
            ))
            .child(
                div()
                    .debug_selector(move || format!("desktop-thread-card-project-{debug_key}"))
                    .min_w_0()
                    .flex_1()
                    .truncate()
                    .child(project_label),
            )
            .when(thread.pinned, |el| {
                el.child(
                    div()
                        .debug_selector(move || format!("desktop-thread-pin-{debug_key}"))
                        .flex_shrink_0()
                        .child(icon(
                            assets::PIN_ICON_PATH,
                            SIDEBAR_SMALL_ICON_PX,
                            theme::rambutan_500(),
                        )),
                )
            })
            .child(
                div()
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .justify_end()
                    .h(px(SIDEBAR_CARD_LINE_HEIGHT_PX))
                    .group_hover(row_group.clone(), |el| el.invisible())
                    .child(status_slot),
            );

        let title_line = div()
            .mt(px(SIDEBAR_CARD_LINE_GAP_PX))
            .text_size(px(SIDEBAR_CARD_TITLE_TEXT_PX))
            .text_color(if is_active || status == CardStatus::Unread {
                theme::rambutan_100()
            } else {
                theme::rambutan_200()
            })
            .truncate()
            .debug_selector(move || format!("desktop-thread-card-title-{debug_key}"))
            .child(title);

        let branch = thread.branch.clone();
        // Web: the third line exists once the thread has a branch or a turn
        // (its model is recorded on the first send).
        let has_turns = !chat.messages.is_empty();
        // The role's id keeps the line; its current name fills it. A deleted
        // role leaves the name out (web).
        let role = chat.role_profile.as_ref().map(|role| role.name.clone());
        let has_meta = branch.is_some() || has_turns || chat.role.is_some();
        let meta_line = has_meta.then(|| {
            div()
                .mt(px(SIDEBAR_CARD_LINE_GAP_PX))
                .h(px(SIDEBAR_CARD_META_HEIGHT_PX))
                .flex()
                .items_center()
                .gap(px(SIDEBAR_CARD_ROW_GAP_PX))
                .text_size(px(SIDEBAR_LABEL_TEXT_PX))
                .font_family(theme::UI_SMALL_FONT_FAMILY)
                .text_color(theme::rambutan_500())
                .when_some(branch.clone(), |el, branch| {
                    el.child(icon(
                        assets::GIT_BRANCH_ICON_PATH,
                        SIDEBAR_SMALL_ICON_PX,
                        theme::rambutan_500(),
                    ))
                    .child(
                        div()
                            .debug_selector(move || {
                                format!("desktop-thread-card-branch-{debug_key}")
                            })
                            .min_w_0()
                            .flex_1()
                            .truncate()
                            .child(branch),
                    )
                })
                .when(branch.is_none(), |el| el.child(div().flex_1()))
                .when_some(role, |el, role| {
                    el.child(
                        div()
                            .debug_selector(move || format!("desktop-thread-card-role-{debug_key}"))
                            .flex_shrink_0()
                            .max_w(px(SIDEBAR_CARD_ROLE_MAX_WIDTH_PX))
                            .truncate()
                            .text_color(theme::rambutan_400())
                            .child(role),
                    )
                })
                .when(has_turns, |el| el.child(provider_mark(provider, cx)))
        });

        let delete_button = div()
            .id(("desktop-thread-delete", key))
            .debug_selector(move || format!("desktop-thread-delete-{debug_key}"))
            .absolute()
            .right(px(SIDEBAR_CARD_DELETE_INSET_PX))
            .top(px(SIDEBAR_CARD_DELETE_INSET_PX))
            .size(px(SIDEBAR_CARD_DELETE_PX))
            .rounded(px(SIDEBAR_RADIUS_SEED_PX))
            .flex()
            .items_center()
            .justify_center()
            .invisible()
            .group_hover(row_group.clone(), |el| el.visible())
            .text_color(theme::rambutan_500())
            .hover(|el| {
                el.bg(theme::longan_700().opacity(ACTIVE_BG_OPACITY_60))
                    .text_color(theme::lychee_400())
            })
            .tab_index(0)
            .role(Role::Button)
            .aria_label(SIDEBAR_DELETE_LABEL)
            .focus_visible(|el| el.visible().border_1().border_color(theme::persimmon_400()))
            .cursor_pointer()
            .on_click(cx.listener(move |this, _ev, _window, cx| {
                cx.stop_propagation();
                this.delete_thread_by_id(&id_for_delete, cx);
            }))
            .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
                if is_web_button_activation_key(event.keystroke.key.as_str()) {
                    window.prevent_default();
                    cx.stop_propagation();
                    this.delete_thread_by_id(&id_for_delete_key, cx);
                }
            }))
            .child(icon(
                assets::CLOSE_ICON_PATH,
                SIDEBAR_SMALL_ICON_PX,
                theme::rambutan_500(),
            ));

        let menu_button = div()
            .id(("desktop-thread-actions", key))
            .debug_selector(move || format!("desktop-thread-actions-{debug_key}"))
            .absolute()
            .right(px(SIDEBAR_CARD_DELETE_INSET_PX
                + SIDEBAR_CARD_DELETE_PX
                + 4.0))
            .top(px(SIDEBAR_CARD_DELETE_INSET_PX))
            .size(px(SIDEBAR_CARD_DELETE_PX))
            .rounded(px(SIDEBAR_RADIUS_SEED_PX))
            .flex()
            .items_center()
            .justify_center()
            .invisible()
            .group_hover(row_group.clone(), |el| el.visible())
            .text_color(theme::rambutan_500())
            .hover(|el| el.bg(theme::longan_700().opacity(ACTIVE_BG_OPACITY_60)))
            .tab_index(0)
            .role(Role::Button)
            .aria_label("Thread actions")
            .focus_visible(|el| el.visible().border_1().border_color(theme::persimmon_400()))
            .cursor_pointer()
            .on_click(cx.listener(move |this, _ev, _window, cx| {
                cx.stop_propagation();
                this.thread_menu_id = Some(id_for_menu.clone());
                cx.notify();
            }))
            .child("…");

        div()
            .id(("desktop-thread-row", key))
            .debug_selector(move || format!("desktop-thread-row-{debug_key}"))
            .group(row_group)
            .relative()
            .w_full()
            .rounded(px(SIDEBAR_RADIUS_LYCHEE_PX))
            .px(px(SIDEBAR_CARD_X_PADDING_PX))
            .py(px(SIDEBAR_CARD_Y_PADDING_PX))
            .flex()
            .flex_col()
            .tab_index(0)
            .role(Role::Button)
            .aria_label(format!("Open conversation {}", chat.thread_title))
            .aria_selected(is_active)
            .focus_visible(|el| el.border_2().border_color(theme::persimmon_400()))
            .cursor_pointer()
            .when(is_active, |el| {
                el.bg(theme::longan_800())
                    .border_1()
                    .border_color(gpui::white().opacity(RING_WHITE_10))
            })
            .when(!is_active, |el| {
                el.border_1()
                    .border_color(gpui::transparent_black())
                    .hover(|el| el.bg(theme::longan_800().opacity(HOVER_BG_OPACITY_40)))
            })
            .on_click(cx.listener(move |this, _ev, _window, cx| {
                this.select_thread_by_id(&id_for_select, cx);
            }))
            .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
                if is_web_button_activation_key(event.keystroke.key.as_str()) {
                    window.prevent_default();
                    cx.stop_propagation();
                    this.select_thread_by_id(&id_for_select_key, cx);
                }
            }))
            .child(top_line)
            .child(title_line)
            .children(meta_line)
            .child(menu_button)
            .child(delete_button)
            .into_any_element()
    }

    fn render_sidebar_undo(&self, cx: &Context<'_, Self>) -> AnyElement {
        let Some(entry) = self.sidebar_undo.as_ref() else {
            return div().into_any_element();
        };
        let message = entry.message.clone();
        div()
            .id("desktop-sidebar-undo")
            .debug_selector(|| "desktop-sidebar-undo".to_string())
            .role(Role::Status)
            .mx(px(8.0))
            .mb(px(8.0))
            .min_h(px(40.0))
            .px(px(12.0))
            .flex()
            .items_center()
            .gap_2()
            .rounded(px(SIDEBAR_RADIUS_GRAPE_PX))
            .bg(theme::longan_800())
            .border_1()
            .border_color(gpui::white().opacity(RING_WHITE_10))
            .text_size(px(SIDEBAR_LABEL_TEXT_PX))
            .font_family(theme::UI_SMALL_FONT_FAMILY)
            .text_color(theme::rambutan_100())
            .child(div().flex_1().min_w_0().truncate().child(message))
            .child(
                div()
                    .id("desktop-sidebar-undo-button")
                    .debug_selector(|| "desktop-sidebar-undo-button".to_string())
                    .min_h(px(32.0))
                    .px_2()
                    .flex()
                    .items_center()
                    .rounded(px(SIDEBAR_RADIUS_SEED_PX))
                    .text_color(theme::persimmon_400())
                    .cursor_pointer()
                    .tab_index(0)
                    .role(Role::Button)
                    .aria_label("Undo")
                    .focus_visible(|el| el.border_2().border_color(theme::persimmon_400()))
                    .on_click(cx.listener(|this, _ev, window, cx| {
                        cx.stop_propagation();
                        this.undo_sidebar(window, cx);
                    }))
                    .child("Undo"),
            )
            .into_any_element()
    }

    fn render_thread_menu(&self, cx: &Context<'_, Self>) -> AnyElement {
        let Some(id) = self.thread_menu_id.clone() else {
            return div().into_any_element();
        };
        let Some(thread) = self.threads.iter().find(|thread| thread.id == id) else {
            return div().into_any_element();
        };
        let chat = thread.chat.read(cx);
        let streaming = chat.is_streaming;
        let pinned = thread.pinned;
        let settled = thread.settled;
        let now = EpochMs::now().as_millis();
        let snoozed = thread.snoozed_until_ms.is_some_and(|until| until > now);
        let show_discard = self.active_thread_id.as_deref() == Some(id.as_str())
            && !chat.input_state.read(cx).value().is_empty();
        let presets = snooze_presets(now, threads_ui::utc_offset_minutes_east());
        let mut rows: Vec<AnyElement> = Vec::new();
        rows.push(self.thread_menu_item(
            "desktop-thread-menu-pin",
            if pinned { "Unpin" } else { "Pin" },
            false,
            cx,
            {
                let id = id.clone();
                move |this, _window, cx| {
                    this.toggle_pinned_by_id(&id, cx);
                    this.thread_menu_id = None;
                    cx.notify();
                }
            },
        ));
        rows.push(self.thread_menu_item(
            "desktop-thread-menu-settle",
            if settled { "Unsettle" } else { "Settle" },
            false,
            cx,
            {
                let id = id.clone();
                move |this, _window, cx| {
                    this.toggle_settled_by_id(&id, cx);
                    this.thread_menu_id = None;
                    cx.notify();
                }
            },
        ));
        if snoozed {
            rows.push(self.thread_menu_item(
                "desktop-thread-menu-wake",
                "Wake thread",
                false,
                cx,
                {
                    let id = id.clone();
                    move |this, _window, cx| this.wake_thread_by_id(&id, cx)
                },
            ));
        }
        for preset in presets {
            let until = preset.until_ms;
            let item_id: &'static str = match preset.label {
                "Later today" => "desktop-thread-menu-later",
                "Tomorrow" => "desktop-thread-menu-tomorrow",
                _ => "desktop-thread-menu-next-week",
            };
            rows.push(self.thread_menu_item(item_id, preset.label, false, cx, {
                let id = id.clone();
                move |this, _window, cx| this.snooze_thread_by_id(&id, until, cx)
            }));
        }
        rows.push(
            self.thread_menu_item("desktop-thread-menu-hour", "In 1 hour", false, cx, {
                let id = id.clone();
                move |this, _window, cx| {
                    if let Some(until) =
                        snooze_after(EpochMs::now().as_millis(), 1, SnoozeUnit::Hours)
                    {
                        this.snooze_thread_by_id(&id, until, cx);
                    }
                }
            }),
        );
        rows.push(self.thread_menu_item(
            "desktop-thread-menu-regenerate",
            "Regenerate title",
            streaming,
            cx,
            {
                let id = id.clone();
                move |this, _window, cx| this.regenerate_title_by_id(&id, cx)
            },
        ));
        if show_discard {
            rows.push(self.thread_menu_item(
                "desktop-thread-menu-discard",
                "Discard draft",
                false,
                cx,
                move |this, window, cx| this.discard_active_draft(window, cx),
            ));
        }
        div()
            .id("desktop-thread-menu")
            .debug_selector(|| "desktop-thread-menu".to_string())
            .absolute()
            .left(px(8.0))
            .bottom(px(52.0))
            .w(px(208.0))
            .p_1()
            .rounded(px(SIDEBAR_RADIUS_GRAPE_PX))
            .bg(theme::longan_900())
            .border_1()
            .border_color(theme::ring_floating())
            .occlude()
            .flex()
            .flex_col()
            .role(Role::Menu)
            .aria_label("Thread actions")
            .children(rows)
            .into_any_element()
    }

    fn thread_menu_item(
        &self,
        id: &'static str,
        label: &'static str,
        disabled: bool,
        cx: &Context<'_, Self>,
        action: impl Fn(&mut Workspace, &mut Window, &mut Context<'_, Workspace>) + 'static,
    ) -> AnyElement {
        div()
            .id(id)
            .debug_selector(move || id.to_string())
            .min_h(px(40.0))
            .w_full()
            .px(px(10.0))
            .flex()
            .items_center()
            .rounded(px(SIDEBAR_RADIUS_SEED_PX))
            .text_size(px(SIDEBAR_LABEL_TEXT_PX))
            .font_family(theme::UI_SMALL_FONT_FAMILY)
            .text_color(theme::rambutan_200())
            .role(Role::MenuItem)
            .aria_label(label)
            .when(disabled, |el| el.opacity(0.5).cursor_not_allowed())
            .when(!disabled, |el| {
                el.cursor_pointer()
                    .tab_index(0)
                    .focus_visible(|el| el.border_2().border_color(theme::persimmon_400()))
                    .hover(|el| el.bg(theme::longan_800().opacity(0.6)))
                    .on_click(cx.listener(move |this, _ev, window, cx| {
                        cx.stop_propagation();
                        action(this, window, cx);
                    }))
            })
            .child(label)
            .into_any_element()
    }

    fn render_sidebar_nav_icon(
        &self,
        view: AppView,
        icon_path: &'static str,
        label: &'static str,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        let active = self.active_view == view && !self.settings_visible();
        let selector = format!("desktop-sidebar-nav-{}", label.to_ascii_lowercase());
        let click_view = view.clone();
        let key_view = view;
        div()
            .id(ElementId::Name(selector.clone().into()))
            .debug_selector(move || selector)
            .size(px(SIDEBAR_ICON_BUTTON_PX))
            .rounded(px(SIDEBAR_RADIUS_GRAPE_PX))
            .flex()
            .items_center()
            .justify_center()
            .text_color(if active {
                theme::rambutan_100()
            } else {
                theme::rambutan_500()
            })
            .when(active, |el| {
                el.bg(theme::longan_800().opacity(ACTIVE_BG_OPACITY_60))
            })
            .when(!active, |el| {
                el.hover(|el| {
                    el.bg(theme::longan_800().opacity(HOVER_BG_OPACITY_40))
                        .text_color(theme::rambutan_100())
                })
            })
            .tab_index(0)
            .role(Role::Tab)
            .aria_label(label)
            .aria_selected(active)
            .focus_visible(|el| el.border_2().border_color(theme::persimmon_400()))
            .cursor_pointer()
            .on_click(cx.listener(move |this, _ev, window, cx| {
                cx.stop_propagation();
                this.activate_sidebar_view(click_view.clone(), window, cx);
            }))
            .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
                if is_web_button_activation_key(event.keystroke.key.as_str()) {
                    window.prevent_default();
                    cx.stop_propagation();
                    this.activate_sidebar_view(key_view.clone(), window, cx);
                }
            }))
            .child(icon(
                icon_path,
                SIDEBAR_FOOTER_ICON_PX,
                if active {
                    theme::rambutan_100()
                } else {
                    theme::rambutan_500()
                },
            ))
            .into_any_element()
    }

    /// Footer view switcher: tasks · terminal · calendar · meetings · people, settings last.
    fn render_sidebar_footer(&self, cx: &Context<'_, Self>) -> AnyElement {
        let settings_active = self.settings_visible();
        let settings = div()
            .id("desktop-sidebar-nav-settings")
            .debug_selector(|| "desktop-sidebar-nav-settings".to_string())
            .size(px(SIDEBAR_ICON_BUTTON_PX))
            .rounded(px(SIDEBAR_RADIUS_GRAPE_PX))
            .flex()
            .items_center()
            .justify_center()
            .text_color(if settings_active {
                theme::rambutan_100()
            } else {
                theme::rambutan_500()
            })
            .when(settings_active, |el| {
                el.bg(theme::longan_800().opacity(ACTIVE_BG_OPACITY_60))
            })
            .when(!settings_active, |el| {
                el.hover(|el| {
                    el.bg(theme::longan_800().opacity(HOVER_BG_OPACITY_40))
                        .text_color(theme::rambutan_100())
                })
            })
            .tab_index(0)
            .role(Role::Tab)
            .aria_label("Settings")
            .aria_selected(settings_active)
            .focus_visible(|el| el.border_2().border_color(theme::persimmon_400()))
            .cursor_pointer()
            .on_click(cx.listener(|this, _ev, window, cx| {
                cx.stop_propagation();
                this.open_settings(&OpenSettings, window, cx);
            }))
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                if is_web_button_activation_key(event.keystroke.key.as_str()) {
                    window.prevent_default();
                    cx.stop_propagation();
                    this.open_settings(&OpenSettings, window, cx);
                }
            }))
            .child(icon(
                "icons/settings.svg",
                SIDEBAR_FOOTER_ICON_PX,
                if settings_active {
                    theme::rambutan_100()
                } else {
                    theme::rambutan_500()
                },
            ));
        div()
            .id("sidebar-footer")
            .debug_selector(|| "desktop-sidebar-footer".to_string())
            .role(Role::Group)
            .aria_label("Views")
            .px(px(SIDEBAR_FOOTER_PADDING_PX))
            .py(px(SIDEBAR_FOOTER_PADDING_PX))
            .flex()
            .items_center()
            .gap(px(SIDEBAR_FOOTER_GAP_PX))
            .flex_shrink_0()
            .child(self.render_sidebar_nav_icon(AppView::Today, assets::SUN_ICON_PATH, "Today", cx))
            .child(self.render_sidebar_nav_icon(
                AppView::Inbox,
                assets::INBOX_ICON_PATH,
                "Inbox",
                cx,
            ))
            .child(self.render_sidebar_nav_icon(
                AppView::Knowledge,
                assets::BOOK_OPEN_ICON_PATH,
                "Knowledge",
                cx,
            ))
            .child(self.render_sidebar_nav_icon(
                AppView::Tasks,
                assets::LIST_CHECKS_ICON_PATH,
                "Tasks",
                cx,
            ))
            .child(self.render_sidebar_nav_icon(
                AppView::Terminal,
                assets::TOOL_TERMINAL_ICON_PATH,
                "Terminal",
                cx,
            ))
            .child(self.render_sidebar_nav_icon(
                AppView::Calendar,
                "icons/calendar.svg",
                "Calendar",
                cx,
            ))
            .child(self.render_sidebar_nav_icon(
                AppView::Meetings,
                assets::MIC_ICON_PATH,
                "Meetings",
                cx,
            ))
            .child(self.render_sidebar_nav_icon(
                AppView::People,
                assets::USERS_ICON_PATH,
                "People",
                cx,
            ))
            .child(div().flex_1())
            .child(settings)
            .into_any_element()
    }

    /// Footer nav: primary views switch like the top bar used to; web-only
    /// views show their placeholder page.
    pub(super) fn activate_sidebar_view(
        &mut self,
        view: AppView,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        match view {
            AppView::Today
            | AppView::Chat
            | AppView::Tasks
            | AppView::Calendar
            | AppView::Terminal => {
                self.activate_primary_view(view, window, cx);
            }
            other => {
                let focus_search = other == AppView::Search;
                if focus_search {
                    // Refresh once on entry. Query keystrokes then touch only the
                    // normalized index; MeetingsHub revisions cover background data.
                    self.universal_search_index.replace(None);
                }
                self.active_view = other;
                self.compact_sidebar_open = false;
                self.close_transient_surfaces();
                self.persist_and_notify(cx);
                if focus_search {
                    // Web `search_page` autofocuses its query input.
                    self.search_query_input
                        .update(cx, |input, cx| input.focus(window, cx));
                }
            }
        }
    }
}

impl Workspace {
    /// The sidebar title search; edits redraw the list.
    pub(super) fn new_sidebar_search(
        window: &mut Window,
        cx: &mut Context<'_, Self>,
        subscriptions: &mut Vec<Subscription>,
    ) -> Entity<InputState> {
        let sidebar_search =
            cx.new(|cx| InputState::new(window, cx).placeholder(SIDEBAR_SEARCH_PLACEHOLDER));
        subscriptions.push(redraw_on_edit(&sidebar_search, window, cx));
        sidebar_search
    }

    /// Global and per-project title instructions. Edits persist on this device.
    pub(super) fn new_title_instruction_inputs(
        global: String,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
        subscriptions: &mut Vec<Subscription>,
    ) -> (Entity<InputState>, Entity<InputState>, Entity<InputState>) {
        let global_input = cx.new(|cx| {
            InputState::new(window, cx)
                .multi_line(true)
                .placeholder("prefix: Bug\nmax 48")
                .default_value(global)
        });
        let project_input = cx.new(|cx| InputState::new(window, cx).placeholder("Project name"));
        let project_instructions = cx.new(|cx| {
            InputState::new(window, cx)
                .multi_line(true)
                .placeholder("prefix: Bug")
        });
        subscriptions.push(cx.subscribe_in(
            &global_input,
            window,
            |this, input, event: &InputEvent, _window, cx| {
                if matches!(event, InputEvent::Change) {
                    let value = input.read(cx).value().to_string();
                    if this.title_instructions != value {
                        this.title_instructions = value;
                        this.persist(cx);
                    }
                }
            },
        ));
        subscriptions.push(cx.subscribe_in(
            &project_input,
            window,
            |this, input, event: &InputEvent, window, cx| {
                if !matches!(event, InputEvent::Change) {
                    return;
                }
                let name = input.read(cx).value().trim().to_string();
                let text = this
                    .title_instructions_by_project
                    .get(&name)
                    .cloned()
                    .unwrap_or_default();
                let instructions = this.title_project_instructions_input.clone();
                instructions.update(cx, |field, cx| field.set_value(text, window, cx));
            },
        ));
        subscriptions.push(cx.subscribe_in(
            &project_instructions,
            window,
            |this, input, event: &InputEvent, _window, cx| {
                if !matches!(event, InputEvent::Change) {
                    return;
                }
                let name = this.title_project_input.read(cx).value().trim().to_string();
                if name.is_empty() {
                    return;
                }
                let value = input.read(cx).value().to_string();
                if value.trim().is_empty() {
                    this.title_instructions_by_project.remove(&name);
                } else {
                    this.title_instructions_by_project.insert(name, value);
                }
                this.persist(cx);
            },
        ));
        (global_input, project_input, project_instructions)
    }
}

#[cfg(test)]
mod tests {
    use super::{thread_matches, SidebarFilter};

    #[test]
    fn thread_filter_and_search_match_web_visible_threads() {
        let bruhs = SidebarFilter::Project("bruhs".to_string());
        assert!(thread_matches(
            "bruhs",
            "Fix login",
            &SidebarFilter::All,
            ""
        ));
        assert!(thread_matches("", "Groceries", &SidebarFilter::All, ""));
        assert!(thread_matches("bruhs", "Fix login", &bruhs, ""));
        assert!(!thread_matches("other", "Fix login", &bruhs, ""));
        assert!(thread_matches(
            "bruhs",
            "Add tests",
            &SidebarFilter::All,
            "TEST"
        ));
        assert!(!thread_matches(
            "bruhs",
            "Add tests",
            &SidebarFilter::All,
            "login"
        ));
    }

    #[test]
    fn filter_labels_match_web() {
        assert_eq!(SidebarFilter::All.label(), "All projects");
        assert_eq!(SidebarFilter::Project("bruhs".into()).label(), "bruhs");
    }
}

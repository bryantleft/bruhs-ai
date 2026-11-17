//! The thread sidebar, 1:1 with `bruhs-web/src/components/sidebar.rs`:
//! brand header, one control row (search, choose project folder, new
//! thread), three-line thread cards (project · title · branch + provider)
//! with the Settled fold, and the footer view switcher. Sizes are the web's
//! Tailwind values in px; colours are the same bruhs tokens.

use gpui::prelude::*;
use gpui::*;
use gpui_component::input::{Input, InputEvent, InputState};
use gpui_component::scroll::ScrollableElement;
use gpui_component::ActiveTheme;
use std::rc::Rc;

mod menu_popover;
mod viewport;
pub(super) use viewport::SidebarViewport;

use super::{
    element_key, is_web_button_activation_key, provider_mark, redraw_on_edit, threads_ui,
    DesktopThread, KeyDownEvent, OpenProjectPicker, OpenSettings, SidebarSection, Workspace,
    WORKSPACE_SIDEBAR_BORDER_OPACITY,
};
use crate::views::sidebar_order::relative_age;
use crate::{assets, theme};
use bruhs_models::logic::menu::menu_step;
use bruhs_models::logic::sidebar::{
    snooze_presets, thread_menu, SnoozeWhen, ThreadMenuEntry, ThreadMenuFacts,
};
use bruhs_models::{AppView, EpochMs};

/// Shared app drawer width.
pub const SIDEBAR_WIDTH_PX: f32 = 248.0;
/// Web `@keyframes bruhs-status-pulse`: two seconds, full opacity to 40%,
/// 0.45 from 50% to 90%, then back.
const STATUS_PULSE_PERIOD_MS: u64 = 2000;
const STATUS_PULSE_DIM_OPACITY: f32 = 0.45;

/// The Working mark's opacity `progress` (0..1) through one pulse.
fn status_pulse_opacity(progress: f32) -> f32 {
    let fade = |from: f32, to: f32, at: f32| from + (to - from) * at.clamp(0.0, 1.0);
    match progress {
        p if p < 0.4 => 1.0,
        p if p < 0.5 => fade(1.0, STATUS_PULSE_DIM_OPACITY, (p - 0.4) / 0.1),
        p if p < 0.9 => STATUS_PULSE_DIM_OPACITY,
        p => fade(STATUS_PULSE_DIM_OPACITY, 1.0, (p - 0.9) / 0.1),
    }
}
/// Matches the chat and panel headers.
pub const SIDEBAR_HEADER_HEIGHT_PX: f32 = theme::APP_HEADER_HEIGHT_PX;
/// Room for the macOS traffic lights (gpui-component puts them at x=12).
#[cfg(target_os = "macos")]
pub const SIDEBAR_HEADER_LEFT_INSET_PX: f32 = 88.0;
#[cfg(not(target_os = "macos"))]
pub const SIDEBAR_HEADER_LEFT_INSET_PX: f32 = 16.0;
const SIDEBAR_HEADER_GAP_PX: f32 = 8.0;
const SIDEBAR_BRAND_LOGO_PX: f32 = 20.0;
const SIDEBAR_BRAND_TEXT_PX: f32 = theme::APP_UI_TEXT_SIZE_PX;
const SIDEBAR_CONTROLS_X_PADDING_PX: f32 = 8.0;
const SIDEBAR_CONTROLS_BOTTOM_PADDING_PX: f32 = 8.0;
const SIDEBAR_CONTROLS_GAP_PX: f32 = 4.0;
const SIDEBAR_SEARCH_HEIGHT_PX: f32 = theme::CONTROL_PX;
const SIDEBAR_SEARCH_TEXT_PX: f32 = theme::APP_UI_TEXT_SIZE_PX;
const SIDEBAR_ICON_BUTTON_PX: f32 = theme::CONTROL_PX;
const SIDEBAR_LABEL_TEXT_PX: f32 = 12.0;
/// Icons inside controls: buttons, the search field, fold headers, card
/// actions and the footer.
const SIDEBAR_ICON_PX: f32 = theme::ICON_PX;
/// Icons beside a card's 12px project and branch text.
const SIDEBAR_SMALL_ICON_PX: f32 = theme::ICON_INLINE_SM_PX;
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
const SIDEBAR_CARD_TITLE_TEXT_PX: f32 = theme::APP_UI_TEXT_SIZE_PX;
const SIDEBAR_CARD_ROW_GAP_PX: f32 = 6.0;
const SIDEBAR_CARD_DOT_PX: f32 = 9.0;
const SIDEBAR_CARD_DELETE_PX: f32 = theme::CONTROL_XS_PX;
const SIDEBAR_CARD_DELETE_INSET_PX: f32 = 8.0;
/// Web `right-9`: the actions button sits 4px left of delete.
const SIDEBAR_CARD_MENU_RIGHT_PX: f32 = SIDEBAR_CARD_DELETE_INSET_PX + SIDEBAR_CARD_DELETE_PX + 4.0;
/// Web thread menu: `w-52`.
const THREAD_MENU_WIDTH_PX: f32 = 208.0;
/// Thread menu rows: shadcn `DropdownMenuItem` (32px, `px-2`).
const THREAD_MENU_ROW_PX: f32 = theme::CONTROL_PX;
const THREAD_MENU_ROW_X_PADDING_PX: f32 = 8.0;
/// Focus slots: one per `ThreadMenuItem`.
pub(super) const THREAD_MENU_MAX_ITEMS: usize = 10;
const SIDEBAR_SETTLED_TOP_PADDING_PX: f32 = 8.0;
const SIDEBAR_SETTLED_X_PADDING_PX: f32 = 4.0;
const SIDEBAR_FOOTER_PADDING_PX: f32 = 8.0;
/// Ten views and settings would need 372px in one row of 32px buttons; the
/// footer has 232px. They sit in rows of six spread across the row (web: a
/// six-column grid with `justify-between`), 4px apart (`gap-y-1`).
pub(super) const SIDEBAR_FOOTER_COLUMNS: usize = 6;
const SIDEBAR_FOOTER_ROW_GAP_PX: f32 = 4.0;
/// Agent workflows' unread count, pinned 2px outside the button's top right
/// corner (web: `h-[16px] min-w-[16px] px-1 text-[11px]`).
const SIDEBAR_BADGE_HEIGHT_PX: f32 = 16.0;
const SIDEBAR_BADGE_X_PADDING_PX: f32 = 4.0;
const SIDEBAR_BADGE_TEXT_PX: f32 = 11.0;
/// Insets start inside the button's 1px focus border, so 2px outside its
/// edge is 3px from there.
const SIDEBAR_BADGE_OFFSET_PX: f32 = -3.0;
const HOVER_BG_OPACITY_40: f32 = 0.40;
const HOVER_BG_OPACITY_50: f32 = 0.50;
const ACTIVE_BG_OPACITY_60: f32 = 0.60;
const RING_WHITE_10: f32 = 0.10;

pub const SIDEBAR_BRAND_LABEL: &str = "bruhs.ai";
pub const SIDEBAR_SEARCH_PLACEHOLDER: &str = "Search";
/// A card's project line for a thread without a project.
pub const SIDEBAR_NO_PROJECT_LABEL: &str = "No project";
pub const SIDEBAR_EMPTY_LABEL: &str = "No threads yet. Press ⌘⇧O to start one.";
pub const SIDEBAR_NO_MATCH_LABEL: &str = "No threads match.";
pub const SIDEBAR_NEW_THREAD_LABEL: &str = "New thread (⌘⇧O)";
pub const SIDEBAR_NEW_PROJECT_LABEL: &str = "Choose project folder";
pub const SIDEBAR_DELETE_LABEL: &str = "Delete thread";

/// What a card shows at its right edge instead of the age.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CardStatus {
    Working,
    Approval,
    Unread,
    Idle,
}

/// Sidebar search: a case-insensitive match on the title or the project
/// name, so typing a project's name lists its threads.
pub fn thread_matches(project_name: &str, title: &str, query: &str) -> bool {
    let q = query.trim().to_lowercase();
    q.is_empty() || title.to_lowercase().contains(&q) || project_name.to_lowercase().contains(&q)
}

fn icon(path: &'static str, size: f32, color: Hsla) -> Svg {
    svg()
        .path(path)
        .size(px(size))
        .text_color(color)
        .flex_shrink_0()
}

/// What a thread menu row runs, given the thread id.
type ThreadMenuAction = Rc<dyn Fn(&mut Workspace, &str, &mut Window, &mut Context<'_, Workspace>)>;

/// Box a thread menu row's action (gives the closure its signature).
fn menu_action(
    action: impl Fn(&mut Workspace, &str, &mut Window, &mut Context<'_, Workspace>) + 'static,
) -> ThreadMenuAction {
    Rc::new(action)
}

/// The thread menu's rows. Each has a fixed focus slot; the menu shows
/// them in the shared order (`bruhs_models::logic::sidebar::thread_menu`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ThreadMenuItem {
    Pin,
    Settle,
    Wake,
    InAnHour,
    LaterToday,
    Tomorrow,
    NextWeek,
    Regenerate,
    Discard,
    Custom,
}

impl ThreadMenuItem {
    const ALL: [Self; THREAD_MENU_MAX_ITEMS] = [
        Self::Pin,
        Self::Settle,
        Self::Wake,
        Self::InAnHour,
        Self::LaterToday,
        Self::Tomorrow,
        Self::NextWeek,
        Self::Regenerate,
        Self::Discard,
        Self::Custom,
    ];

    /// The slot for a shared menu row. Separators have none.
    fn for_entry(entry: &ThreadMenuEntry) -> Option<Self> {
        Some(match entry {
            ThreadMenuEntry::Pin { .. } => Self::Pin,
            ThreadMenuEntry::Settle { .. } => Self::Settle,
            ThreadMenuEntry::Wake => Self::Wake,
            ThreadMenuEntry::Snooze(preset) => match preset.when {
                SnoozeWhen::InAnHour => Self::InAnHour,
                SnoozeWhen::LaterToday => Self::LaterToday,
                SnoozeWhen::Tomorrow => Self::Tomorrow,
                SnoozeWhen::NextWeek => Self::NextWeek,
            },
            ThreadMenuEntry::SnoozeCustom => Self::Custom,
            ThreadMenuEntry::RegenerateTitle { .. } => Self::Regenerate,
            ThreadMenuEntry::DiscardDraft => Self::Discard,
            ThreadMenuEntry::Separator => return None,
        })
    }

    /// Index into `Workspace::thread_menu_items`.
    fn slot(self) -> usize {
        Self::ALL
            .iter()
            .position(|item| *item == self)
            .unwrap_or_default()
    }

    /// Element id and debug selector.
    fn id(self) -> &'static str {
        match self {
            Self::Pin => "desktop-thread-menu-pin",
            Self::Settle => "desktop-thread-menu-settle",
            Self::Wake => "desktop-thread-menu-wake",
            Self::InAnHour => "desktop-thread-menu-hour",
            Self::LaterToday => "desktop-thread-menu-later",
            Self::Tomorrow => "desktop-thread-menu-tomorrow",
            Self::NextWeek => "desktop-thread-menu-next-week",
            Self::Regenerate => "desktop-thread-menu-regenerate",
            Self::Discard => "desktop-thread-menu-discard",
            Self::Custom => "desktop-thread-menu-custom",
        }
    }
}

/// What choosing `entry` does to thread `id`. Snooze times count from the
/// click, not from when the menu opened.
fn thread_menu_action(entry: ThreadMenuEntry) -> ThreadMenuAction {
    match entry {
        ThreadMenuEntry::Pin { .. } => menu_action(|this, id, _window, cx| {
            this.toggle_pinned_by_id(id, cx);
        }),
        ThreadMenuEntry::Settle { .. } => menu_action(|this, id, _window, cx| {
            this.toggle_settled_by_id(id, cx);
        }),
        ThreadMenuEntry::Wake => {
            menu_action(|this, id, _window, cx| this.wake_thread_by_id(id, cx))
        }
        ThreadMenuEntry::Snooze(preset) => menu_action(move |this, id, _window, cx| {
            let now = EpochMs::now().as_millis();
            let until = snooze_presets(now, threads_ui::utc_offset_minutes_east())
                .into_iter()
                .find(|fresh| fresh.when == preset.when)
                .map(|fresh| fresh.until_ms);
            if let Some(until) = until {
                this.snooze_thread_by_id(id, until, cx);
            }
        }),
        ThreadMenuEntry::SnoozeCustom => {
            menu_action(|this, id, window, cx| this.open_snooze_custom(id, window, cx))
        }
        ThreadMenuEntry::RegenerateTitle { .. } => {
            menu_action(|this, id, _window, cx| this.regenerate_title_by_id(id, cx))
        }
        ThreadMenuEntry::DiscardDraft => {
            menu_action(|this, _id, window, cx| this.discard_active_draft(window, cx))
        }
        ThreadMenuEntry::Separator => menu_action(|_this, _id, _window, _cx| {}),
    }
}

/// Hairline between thread menu groups (web: `my-1 h-px bg-white/10`).
fn thread_menu_separator() -> AnyElement {
    div()
        .my_1()
        .h(px(1.0))
        .flex_shrink_0()
        .bg(gpui::white().opacity(RING_WHITE_10))
        .into_any_element()
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
            .child(self.render_sidebar_header())
            .child(self.render_sidebar_controls(cx))
            .child(self.render_sidebar_list(cx))
            .child(self.render_sidebar_undo(cx))
            .children(self.render_sidebar_update_button(cx))
            .child(self.render_sidebar_footer(cx))
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
            .border_1()
            .border_color(gpui::transparent_black())
            .focus_visible(|el| el.border_color(theme::persimmon_400()))
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

    /// One row: search, then choose project folder and new thread.
    fn render_sidebar_controls(&self, cx: &Context<'_, Self>) -> Div {
        div()
            .px(px(SIDEBAR_CONTROLS_X_PADDING_PX))
            .pb(px(SIDEBAR_CONTROLS_BOTTOM_PADDING_PX))
            .flex()
            .items_center()
            .gap(px(SIDEBAR_CONTROLS_GAP_PX))
            .flex_shrink_0()
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
                                .aria_label("Search threads and projects")
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
                "desktop-sidebar-new-project",
                SIDEBAR_NEW_PROJECT_LABEL,
                assets::FOLDER_PLUS_ICON_PATH,
                SIDEBAR_ICON_BUTTON_PX,
                |this, window, cx| this.open_project_picker(&OpenProjectPicker, window, cx),
                cx,
            ))
            .child(self.sidebar_icon_button(
                "desktop-sidebar-new-thread",
                SIDEBAR_NEW_THREAD_LABEL,
                assets::SQUARE_PEN_ICON_PATH,
                SIDEBAR_ICON_BUTTON_PX,
                |this, window, cx| this.new_thread(&super::NewThread, window, cx),
                cx,
            ))
    }

    /// Pinned and active cards, then the Working, Snoozed, and Settled folds.
    fn render_sidebar_list(&self, cx: &Context<'_, Self>) -> AnyElement {
        let shelves = self.sidebar_shelves(cx);
        let query = self.sidebar_query(cx);
        let searching = !query.trim().is_empty();
        let empty = shelves.pinned.is_empty()
            && shelves.active.is_empty()
            && shelves.working.is_empty()
            && shelves.snoozed.is_empty()
            && shelves.settled.is_empty();
        let rows = viewport::sidebar_rows(self, &shelves, cx);
        let (state, rows, focus) = self.sidebar_viewport.borrow_mut().prepare(
            rows,
            viewport::SidebarScope { query },
            cx.theme().font_size,
            cx,
        );
        let (start, end) = self.sidebar_viewport.borrow().boundaries();
        let len = rows.len();
        let content = list(
            state.clone(),
            cx.processor(move |this, index: usize, _, cx| {
                let row = rows[index];
                let element = match row {
                    viewport::SidebarRow::Thread {
                        index: thread_index,
                        ..
                    } => this.render_thread_card(&this.threads[thread_index], &focus[index], cx),
                    viewport::SidebarRow::Shelf {
                        section,
                        count,
                        open,
                    } => this
                        .render_shelf_header(section, count, open, &focus[index].card, cx)
                        .into_any_element(),
                };
                div()
                    .id(("sidebar-list-row", index))
                    .w_full()
                    .role(Role::ListItem)
                    .aria_position_in_set(index + 1)
                    .aria_size_of_set(len)
                    .when(index + 1 < len, |el| el.pb(px(SIDEBAR_LIST_GAP_PX)))
                    .child(element)
                    .into_any_element()
            }),
        )
        .size_full()
        .pb(px(SIDEBAR_LIST_BOTTOM_PADDING_PX));
        div()
            .id("sidebar-threads")
            .debug_selector(|| "desktop-sidebar-threads".to_string())
            .role(Role::List)
            .aria_label("Threads")
            .flex_1()
            .min_h_0()
            .relative()
            .px(px(SIDEBAR_LIST_X_PADDING_PX))
            .overflow_hidden()
            .flex()
            .flex_col()
            .when(empty, |el| el.pb(px(SIDEBAR_LIST_BOTTOM_PADDING_PX)))
            // a11y-exempt: zero-size programmatic focus boundaries, never tab stops.
            .child(
                div()
                    .size_0()
                    .track_focus(&start)
                    .tab_index(0)
                    .tab_stop(false),
            )
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
            .when(!empty, |el| {
                el.child(content).child(
                    canvas(
                        cx.processor(|this, _, _, _| {
                            this.sidebar_viewport.borrow_mut().seed_height_hints();
                        }),
                        |_, (), _, _| {},
                    )
                    .absolute()
                    .size_0(),
                )
            })
            .child(
                div()
                    .size_0()
                    .track_focus(&end)
                    .tab_index(0)
                    .tab_stop(false),
            )
            .vertical_scrollbar(&state)
            .into_any_element()
    }

    /// Shelf headers share the same list as their variable-height cards.
    fn render_shelf_header(
        &self,
        section: SidebarSection,
        count: usize,
        open: bool,
        focus: &FocusHandle,
        cx: &Context<'_, Self>,
    ) -> Stateful<Div> {
        let (id, name) = viewport::shelf_labels(section);
        let label = format!("{name} ({count})");
        div()
            .id(id)
            .track_focus(focus)
            .debug_selector(move || id.to_string())
            .mt(px(SIDEBAR_SETTLED_TOP_PADDING_PX))
            .px(px(SIDEBAR_SETTLED_X_PADDING_PX))
            .py_1()
            .min_h(px(theme::CONTROL_PX))
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
            .border_1()
            .border_color(gpui::transparent_black())
            .focus_visible(|el| el.border_color(theme::persimmon_400()))
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
                    SIDEBAR_ICON_PX,
                    theme::rambutan_500(),
                )
                .when(open, |el| {
                    el.with_transformation(Transformation::rotate(Radians(std::f32::consts::PI)))
                }),
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
    fn render_thread_card(
        &self,
        thread: &DesktopThread,
        focus: &viewport::SidebarRowFocus,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        let chat = thread.chat.read(cx);
        let title = bruhs_models::logic::text::plain_title(&chat.thread_title).into_owned();
        let is_active = self.active_thread_id.as_deref() == Some(thread.id.as_str());
        // While its menu is open the card keeps its hover look: the backdrop
        // that closes the menu covers the card.
        let menu_open = self.thread_menu_id.as_deref() == Some(thread.id.as_str());
        let status = self.card_status(thread, cx);
        let key = thread.chat.entity_id();
        let debug_key = element_key(&thread.id);
        let row_group: SharedString = format!("desktop-thread-row-{key}").into();
        let menu_group: SharedString = format!("desktop-thread-actions-{key}").into();
        let delete_group: SharedString = format!("desktop-thread-delete-{key}").into();
        let age = relative_age(thread.updated_at_ms, EpochMs::now());
        let project_label = if thread.project_name.is_empty() {
            SIDEBAR_NO_PROJECT_LABEL.to_string()
        } else {
            thread.project_name.clone()
        };
        let provider = bruhs_models::agent::AgentProvider::for_model(&thread.model_id);
        let id_for_select = thread.id.clone();
        let id_for_select_key = thread.id.clone();
        let id_for_delete = thread.id.clone();
        let id_for_delete_key = thread.id.clone();
        let id_for_menu = thread.id.clone();
        let id_for_menu_key = thread.id.clone();

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
                let dot = div()
                    .debug_selector(move || format!("desktop-thread-card-status-{debug_key}"))
                    .size(px(SIDEBAR_CARD_DOT_PX))
                    .rounded_full()
                    .bg(color)
                    .id(("desktop-thread-card-status", key))
                    .role(Role::Image)
                    .aria_label(label);
                // Web `animate-status-pulse`: the Working mark breathes
                // unless the system asks for reduced motion.
                if other == CardStatus::Working && !crate::motion::reduce_motion(cx) {
                    div()
                        .debug_selector(move || format!("desktop-thread-card-pulse-{debug_key}"))
                        .child(
                            dot.with_animation(
                                ("desktop-thread-card-pulse", key),
                                Animation::new(std::time::Duration::from_millis(
                                    STATUS_PULSE_PERIOD_MS,
                                ))
                                .repeat(),
                                |dot, delta| dot.opacity(status_pulse_opacity(delta)),
                            ),
                        )
                        .into_any_element()
                } else {
                    dot.into_any_element()
                }
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
                SIDEBAR_SMALL_ICON_PX,
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
                        // The row's buttons cover this spot, like the age.
                        .group_hover(row_group.clone(), |el| el.invisible())
                        .when(menu_open, |el| el.invisible())
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
                    .when(menu_open, |el| el.invisible())
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
            .track_focus(&focus.delete)
            .debug_selector(move || format!("desktop-thread-delete-{debug_key}"))
            .absolute()
            .right(px(SIDEBAR_CARD_DELETE_INSET_PX))
            .top(px(SIDEBAR_CARD_DELETE_INSET_PX))
            .size(px(SIDEBAR_CARD_DELETE_PX))
            .rounded(px(SIDEBAR_RADIUS_SEED_PX))
            .flex()
            .items_center()
            .justify_center()
            .group(delete_group.clone())
            .invisible()
            .group_hover(row_group.clone(), |el| el.visible())
            .when(menu_open, |el| el.visible())
            .hover(|el| el.bg(theme::longan_700().opacity(ACTIVE_BG_OPACITY_60)))
            .tab_index(0)
            .role(Role::Button)
            .aria_label(SIDEBAR_DELETE_LABEL)
            .border_1()
            .border_color(gpui::transparent_black())
            .focus_visible(|el| el.visible().border_color(theme::persimmon_400()))
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
            .child(
                icon(
                    assets::CLOSE_ICON_PATH,
                    SIDEBAR_ICON_PX,
                    theme::rambutan_500(),
                )
                .group_hover(delete_group, |el| el.text_color(theme::lychee_400())),
            );

        // Web: an `ellipsis` icon button, `hover:bg-longan-700/60
        // hover:text-rambutan-100`. Enter and Space click it too (GPUI
        // keyboard clicks), and the menu takes focus when it opens.
        let menu_button = div()
            .id(("desktop-thread-actions", key))
            .track_focus(&focus.menu)
            .debug_selector(move || format!("desktop-thread-actions-{debug_key}"))
            .group(menu_group.clone())
            .absolute()
            .right(px(SIDEBAR_CARD_MENU_RIGHT_PX))
            .top(px(SIDEBAR_CARD_DELETE_INSET_PX))
            .size(px(SIDEBAR_CARD_DELETE_PX))
            .rounded(px(SIDEBAR_RADIUS_SEED_PX))
            .flex()
            .items_center()
            .justify_center()
            .invisible()
            .group_hover(row_group.clone(), |el| el.visible())
            .hover(|el| el.bg(theme::longan_700().opacity(ACTIVE_BG_OPACITY_60)))
            .when(menu_open, |el| {
                el.visible()
                    .bg(theme::longan_700().opacity(ACTIVE_BG_OPACITY_60))
            })
            .tab_index(0)
            .role(Role::Button)
            .aria_label("Thread actions")
            .aria_expanded(menu_open)
            .border_1()
            .border_color(gpui::transparent_black())
            .focus_visible(|el| el.visible().border_color(theme::persimmon_400()))
            .cursor_pointer()
            .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                cx.stop_propagation();
                // Keys open it on key down, before the card can see them.
                if !event.is_keyboard() {
                    this.toggle_thread_menu(&id_for_menu, window, cx);
                }
            }))
            .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
                if is_web_button_activation_key(event.keystroke.key.as_str()) {
                    window.prevent_default();
                    cx.stop_propagation();
                    this.toggle_thread_menu(&id_for_menu_key, window, cx);
                }
            }))
            .child(
                icon(
                    assets::ELLIPSIS_ICON_PATH,
                    SIDEBAR_ICON_PX,
                    if menu_open {
                        theme::rambutan_100()
                    } else {
                        theme::rambutan_500()
                    },
                )
                .group_hover(menu_group, |el| el.text_color(theme::rambutan_100())),
            );
        // The menu hangs from a box over the button (same insets), so it
        // follows the card as the list scrolls.
        let menu_anchor = menu_open.then(|| {
            div()
                .absolute()
                .right(px(SIDEBAR_CARD_MENU_RIGHT_PX))
                .top(px(SIDEBAR_CARD_DELETE_INSET_PX))
                .size(px(SIDEBAR_CARD_DELETE_PX))
                .child(menu_popover::menu_popover(
                    size(px(SIDEBAR_CARD_DELETE_PX), px(SIDEBAR_CARD_DELETE_PX)),
                    self.render_thread_menu(thread, cx),
                ))
        });

        div()
            .id(("desktop-thread-row", key))
            .track_focus(&focus.card)
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
            .focus_visible(|el| el.border_color(theme::persimmon_400()))
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
            .children(menu_anchor)
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
                    .min_h(px(theme::CONTROL_PX))
                    .px_2()
                    .flex()
                    .items_center()
                    .rounded(px(SIDEBAR_RADIUS_SEED_PX))
                    .text_color(theme::persimmon_400())
                    .cursor_pointer()
                    .tab_index(0)
                    .role(Role::Button)
                    .aria_label("Undo")
                    .border_1()
                    .border_color(gpui::transparent_black())
                    .focus_visible(|el| el.border_color(theme::persimmon_400()))
                    .on_click(cx.listener(|this, _ev, window, cx| {
                        cx.stop_propagation();
                        this.undo_sidebar(window, cx);
                    }))
                    .child("Undo"),
            )
            .into_any_element()
    }

    /// The card's action menu (web `data-thread-menu`): the floating tone and
    /// the rows both apps share (`thread_menu`): shelf actions, snooze times,
    /// then title and draft actions, split by hairlines. Arrow keys, Home and
    /// End move between items; Tab and Escape close it and return focus to
    /// its button.
    fn render_thread_menu(&self, thread: &DesktopThread, cx: &Context<'_, Self>) -> AnyElement {
        let id = thread.id.clone();
        let chat = thread.chat.read(cx);
        let now = EpochMs::now().as_millis();
        let facts = ThreadMenuFacts {
            pinned: thread.pinned,
            settled: thread.settled,
            snoozed: thread.snoozed_until_ms.is_some_and(|until| until > now),
            streaming: chat.is_streaming,
            has_draft: self.active_thread_id.as_deref() == Some(id.as_str())
                && !chat.input_state.read(cx).value().is_empty(),
        };
        // Focusable items in visual order, for the arrow keys.
        let mut enabled: Vec<ThreadMenuItem> = Vec::new();
        let mut rows: Vec<AnyElement> = Vec::new();
        for entry in thread_menu(facts, now, threads_ui::utc_offset_minutes_east()) {
            let Some(item) = ThreadMenuItem::for_entry(&entry) else {
                rows.push(thread_menu_separator());
                continue;
            };
            let on = entry.enabled();
            if on {
                enabled.push(item);
            }
            let label = entry.label();
            rows.push(self.thread_menu_item(item, label, on, &id, cx, thread_menu_action(entry)));
        }
        div()
            .id("desktop-thread-menu")
            .debug_selector(|| "desktop-thread-menu".to_string())
            .key_context("ThreadMenu")
            .w(px(THREAD_MENU_WIDTH_PX))
            .p_1()
            .rounded(px(SIDEBAR_RADIUS_GRAPE_PX))
            .bg(theme::longan_700())
            .border_1()
            .border_color(theme::ring_floating())
            .occlude()
            .flex()
            .flex_col()
            .role(Role::Menu)
            .aria_label("Thread actions")
            .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
                let modifiers = event.keystroke.modifiers;
                if modifiers.platform || modifiers.control || modifiers.alt || modifiers.function {
                    return;
                }
                // Tab is routed by the workspace keystroke interceptor, which
                // runs before gpui-component's Tab binding (`key_decision`).
                let current = enabled
                    .iter()
                    .position(|item| this.thread_menu_items[item.slot()].is_focused(window));
                let next = menu_step(current, enabled.len(), event.keystroke.key.as_str());
                if let Some(item) = next.and_then(|next| enabled.get(next)) {
                    window.prevent_default();
                    cx.stop_propagation();
                    this.thread_menu_items[item.slot()].focus(window, cx);
                }
            }))
            .children(rows)
            .into_any_element()
    }

    /// One menu row (web: `min-h-control rounded-seed px-2 text-label-sm`).
    /// Choosing a row closes the menu and returns focus to the card's
    /// button. Each item keeps its own focus slot, so a row that appears or
    /// disables while the menu is open never shifts focus onto another action.
    fn thread_menu_item(
        &self,
        item: ThreadMenuItem,
        label: &'static str,
        enabled: bool,
        thread_id: &str,
        cx: &Context<'_, Self>,
        action: ThreadMenuAction,
    ) -> AnyElement {
        let id = item.id();
        let thread_id = thread_id.to_string();
        let run =
            move |this: &mut Workspace, window: &mut Window, cx: &mut Context<'_, Workspace>| {
                this.thread_menu_id = None;
                action(this, &thread_id, window, cx);
                this.focus_thread_menu_button(&thread_id, window, cx);
                cx.notify();
            };
        let run_for_key = run.clone();
        let focus = self.thread_menu_items[item.slot()].clone();
        div()
            .id(id)
            .debug_selector(move || id.to_string())
            .min_h(px(THREAD_MENU_ROW_PX))
            .w_full()
            .px(px(THREAD_MENU_ROW_X_PADDING_PX))
            .flex()
            .items_center()
            .rounded(px(SIDEBAR_RADIUS_SEED_PX))
            .border_1()
            .border_color(gpui::transparent_black())
            .text_size(px(SIDEBAR_LABEL_TEXT_PX))
            .font_family(theme::UI_SMALL_FONT_FAMILY)
            .text_color(theme::rambutan_200())
            .role(Role::MenuItem)
            .aria_label(label)
            .when(!enabled, |el| el.opacity(0.5).cursor_not_allowed())
            .when(enabled, |el| {
                el.track_focus(&focus)
                    .tab_index(0)
                    .cursor_pointer()
                    .focus_visible(|el| el.border_color(theme::persimmon_400()))
                    .hover(|el| el.bg(theme::longan_800().opacity(0.6)))
                    .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                        cx.stop_propagation();
                        if !event.is_keyboard() {
                            run(this, window, cx);
                        }
                    }))
                    // Keys stop here: the menu hangs from its card, whose
                    // Enter would open the thread.
                    .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
                        if is_web_button_activation_key(event.keystroke.key.as_str()) {
                            window.prevent_default();
                            cx.stop_propagation();
                            run_for_key(this, window, cx);
                        }
                    }))
            })
            .child(label)
            .into_any_element()
    }

    /// Open thread `id`'s menu with its first item focused, or close it when
    /// it is already open.
    pub(super) fn toggle_thread_menu(
        &mut self,
        id: &str,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if self.thread_menu_id.as_deref() == Some(id) {
            self.close_thread_menu(window, cx);
            return;
        }
        self.thread_menu_id = Some(id.to_string());
        self.thread_menu_items[ThreadMenuItem::Pin.slot()].focus(window, cx);
        cx.notify();
    }

    /// Whether thread `id`'s card is in the shown sidebar list this frame
    /// (sidebar shown; folds, filter and search applied). `drawer`: the
    /// window is narrow enough that the sidebar opens as a drawer.
    fn thread_card_shown(&self, id: &str, drawer: bool, cx: &App) -> bool {
        let sidebar_shown = if drawer {
            self.compact_sidebar_open
        } else {
            self.sidebar_visible
        };
        sidebar_shown
            && viewport::sidebar_rows(self, &self.sidebar_shelves(cx), cx)
                .into_iter()
                .any(|row| {
                    matches!(row, viewport::SidebarRow::Thread { index, .. }
                        if self.threads[index].id == id)
                })
    }

    /// Whether the open thread menu's card is shown. The menu draws with it.
    pub(super) fn thread_menu_card_shown(&self, drawer: bool, cx: &App) -> bool {
        self.thread_menu_id
            .as_deref()
            .is_some_and(|id| self.thread_card_shown(id, drawer, cx))
    }

    /// Whether keyboard focus is on one of the thread menu's items.
    pub(super) fn thread_menu_has_focus(&self, window: &Window) -> bool {
        self.thread_menu_items
            .iter()
            .any(|item| item.is_focused(window))
    }

    /// Close the open thread menu and return focus to its card's button.
    pub(super) fn close_thread_menu(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        if let Some(id) = self.thread_menu_id.take() {
            self.focus_thread_menu_button(&id, window, cx);
            cx.notify();
        }
    }

    /// Focus thread `id`'s "…" button. When an action moved the card out of
    /// the list (into a closed fold, say), focus the workspace instead: a
    /// handle that no longer draws would take every shortcut down with it.
    pub(super) fn focus_thread_menu_button(
        &self,
        id: &str,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let button = self
            .thread_card_shown(id, Self::sidebar_drawer_layout(window), cx)
            .then(|| self.threads.iter().find(|thread| thread.id == id))
            .flatten()
            .and_then(|thread| {
                self.sidebar_viewport
                    .borrow()
                    .menu_focus(thread.chat.entity_id())
            });
        match button {
            Some(button) => button.focus(window, cx),
            None => self.workspace_focus_handle.focus(window, cx),
        }
    }

    fn render_sidebar_nav_icon(
        &self,
        view: AppView,
        icon_path: &'static str,
        label: &'static str,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        self.sidebar_nav_button(view, icon_path, label, cx)
            .into_any_element()
    }

    /// Agent workflows, with the unread update count pinned to its corner
    /// and in its name, like the web's entry.
    fn render_agent_workflows_nav(&self, cx: &Context<'_, Self>) -> AnyElement {
        const LABEL: &str = "Agent workflows";
        let unread = self.reactive_automations.read(cx).unread_count();
        self.sidebar_nav_button(AppView::Automations, assets::BOT_ICON_PATH, LABEL, cx)
            .relative()
            .aria_label(crate::views::reactive_automations::unread_label(
                LABEL, unread,
            ))
            .when(unread > 0, |el| {
                el.child(
                    div()
                        .debug_selector(|| "desktop-sidebar-agent-workflows-badge".to_string())
                        .absolute()
                        .top(px(SIDEBAR_BADGE_OFFSET_PX))
                        .right(px(SIDEBAR_BADGE_OFFSET_PX))
                        .h(px(SIDEBAR_BADGE_HEIGHT_PX))
                        .min_w(px(SIDEBAR_BADGE_HEIGHT_PX))
                        .px(px(SIDEBAR_BADGE_X_PADDING_PX))
                        .rounded(px(SIDEBAR_BADGE_HEIGHT_PX / 2.0))
                        .bg(theme::blueberry_400())
                        .text_color(theme::longan_950())
                        .text_size(px(SIDEBAR_BADGE_TEXT_PX))
                        .line_height(px(SIDEBAR_BADGE_HEIGHT_PX))
                        .flex()
                        .justify_center()
                        .child(if unread > 99 {
                            "99+".to_string()
                        } else {
                            unread.to_string()
                        }),
                )
            })
            .into_any_element()
    }

    fn sidebar_nav_button(
        &self,
        view: AppView,
        icon_path: &'static str,
        label: &'static str,
        cx: &Context<'_, Self>,
    ) -> Stateful<Div> {
        let active = self.active_view == view && !self.settings_visible();
        let selector = format!(
            "desktop-sidebar-nav-{}",
            label.to_ascii_lowercase().replace(' ', "-")
        );
        let click_view = view.clone();
        let key_view = view;
        div()
            .id(ElementId::Name(selector.clone().into()))
            .debug_selector(move || selector)
            .size(px(SIDEBAR_ICON_BUTTON_PX))
            .flex_shrink_0()
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
            .border_1()
            .border_color(gpui::transparent_black())
            .focus_visible(|el| el.border_color(theme::persimmon_400()))
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
                SIDEBAR_ICON_PX,
                if active {
                    theme::rambutan_100()
                } else {
                    theme::rambutan_500()
                },
            ))
    }

    /// Footer view switcher, like the web's: the views in rows of six, then
    /// settings in the last row's last column.
    fn render_sidebar_footer(&self, cx: &Context<'_, Self>) -> AnyElement {
        let settings_active = self.settings_visible();
        let settings = div()
            .id("desktop-sidebar-nav-settings")
            .debug_selector(|| "desktop-sidebar-nav-settings".to_string())
            .size(px(SIDEBAR_ICON_BUTTON_PX))
            .flex_shrink_0()
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
            .border_1()
            .border_color(gpui::transparent_black())
            .focus_visible(|el| el.border_color(theme::persimmon_400()))
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
                SIDEBAR_ICON_PX,
                if settings_active {
                    theme::rambutan_100()
                } else {
                    theme::rambutan_500()
                },
            ));
        let mut cells = vec![
            self.render_sidebar_nav_icon(AppView::Today, assets::SUN_ICON_PATH, "Today", cx),
            self.render_sidebar_nav_icon(AppView::Inbox, assets::INBOX_ICON_PATH, "Inbox", cx),
            self.render_sidebar_nav_icon(
                AppView::Knowledge,
                assets::BOOK_OPEN_ICON_PATH,
                "Knowledge",
                cx,
            ),
            self.render_sidebar_nav_icon(
                AppView::Tasks,
                assets::LIST_CHECKS_ICON_PATH,
                "Tasks",
                cx,
            ),
            self.render_sidebar_nav_icon(
                AppView::Terminal,
                assets::TOOL_TERMINAL_ICON_PATH,
                "Terminal",
                cx,
            ),
            self.render_sidebar_nav_icon(AppView::Workbench, "icons/wrench.svg", "Workbench", cx),
            self.render_sidebar_nav_icon(AppView::Calendar, "icons/calendar.svg", "Calendar", cx),
            self.render_sidebar_nav_icon(AppView::Meetings, assets::MIC_ICON_PATH, "Meetings", cx),
            self.render_sidebar_nav_icon(AppView::People, assets::USERS_ICON_PATH, "People", cx),
            self.render_agent_workflows_nav(cx),
        ];
        // Settings takes the last column, under the first row's last view:
        // empty cells fill the views' last row up to it, and settings starts
        // a row of its own when the views fill theirs (web `col-start-6`).
        let free = SIDEBAR_FOOTER_COLUMNS - cells.len() % SIDEBAR_FOOTER_COLUMNS;
        let empty = (free + SIDEBAR_FOOTER_COLUMNS - 1) % SIDEBAR_FOOTER_COLUMNS;
        cells.extend((0..empty).map(|_| {
            div()
                .size(px(SIDEBAR_ICON_BUTTON_PX))
                .flex_shrink_0()
                .into_any_element()
        }));
        cells.push(settings.into_any_element());
        let mut rows = Vec::new();
        let mut cells = cells.into_iter().peekable();
        while cells.peek().is_some() {
            rows.push(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .children(cells.by_ref().take(SIDEBAR_FOOTER_COLUMNS)),
            );
        }
        div()
            .id("sidebar-footer")
            .debug_selector(|| "desktop-sidebar-footer".to_string())
            .role(Role::Group)
            .aria_label("Views")
            .px(px(SIDEBAR_FOOTER_PADDING_PX))
            .py(px(SIDEBAR_FOOTER_PADDING_PX))
            .flex()
            .flex_col()
            .gap(px(SIDEBAR_FOOTER_ROW_GAP_PX))
            .flex_shrink_0()
            .children(rows)
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
    ) -> (Entity<InputState>, Entity<InputState>) {
        let global_input = cx.new(|cx| {
            InputState::new(window, cx)
                .multi_line(true)
                .placeholder("prefix: Bug\nmax 48")
                .default_value(global)
        });
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
            &project_instructions,
            window,
            |this, input, event: &InputEvent, _window, cx| {
                if !matches!(event, InputEvent::Change) {
                    return;
                }
                let name = this.title_project.clone();
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
        (global_input, project_instructions)
    }
}

#[cfg(test)]
mod tests {
    use super::{status_pulse_opacity, thread_matches};

    /// Web `bruhs-status-pulse`'s keyframes.
    #[test]
    fn the_working_mark_pulses_like_web() {
        assert_eq!(status_pulse_opacity(0.0), 1.0);
        assert_eq!(status_pulse_opacity(0.39), 1.0);
        assert!((status_pulse_opacity(0.45) - 0.725).abs() < 1e-4);
        assert_eq!(status_pulse_opacity(0.5), 0.45);
        assert_eq!(status_pulse_opacity(0.89), 0.45);
        assert!((status_pulse_opacity(0.95) - 0.725).abs() < 1e-4);
        assert!((status_pulse_opacity(1.0) - 1.0).abs() < 1e-4);
    }

    /// Like web's `animate-status-pulse`, a Working card's mark pulses
    /// unless the system asks for reduced motion; the mark itself stays.
    #[gpui::test]
    fn the_working_mark_pulses_only_with_motion(cx: &mut gpui::TestAppContext) {
        use super::super::{element_key, performance_tests::fixture};
        let (workspace, cx) = fixture(cx);
        cx.simulate_resize(gpui::size(gpui::px(1280.0), gpui::px(800.0)));
        let id = workspace.update_in(cx, |workspace, window, cx| {
            workspace.new_thread_in_project("proj".into(), "/tmp/bruhs-pulse".into(), window, cx);
            let thread = workspace.threads.last().expect("a thread");
            thread.chat.update(cx, |chat, _| chat.is_streaming = true);
            let id = thread.id.clone();
            workspace.sidebar_working_open = true;
            cx.notify();
            id
        });
        let key = element_key(&id);
        let pulse: &'static str =
            Box::leak(format!("desktop-thread-card-pulse-{key}").into_boxed_str());
        let mark: &'static str =
            Box::leak(format!("desktop-thread-card-status-{key}").into_boxed_str());
        for reduced in [false, true] {
            cx.update(|_, cx| crate::motion::set_reduced_motion(reduced, cx));
            cx.run_until_parked();
            assert!(cx.debug_bounds(mark).is_some(), "the Working mark shows");
            assert_eq!(
                cx.debug_bounds(pulse).is_some(),
                !reduced,
                "reduced: {reduced}"
            );
        }
    }

    #[test]
    fn search_matches_titles_and_project_names() {
        assert!(thread_matches("bruhs", "Fix login", ""));
        assert!(thread_matches("", "Groceries", "  "));
        assert!(thread_matches("bruhs", "Add tests", "TEST"));
        assert!(thread_matches("bruhs-ai", "Fix login", "Bruhs"));
        assert!(!thread_matches("bruhs", "Add tests", "login"));
        assert!(!thread_matches("", "Groceries", "bruhs"));
    }
}

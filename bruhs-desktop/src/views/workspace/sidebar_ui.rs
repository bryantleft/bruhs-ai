//! The thread sidebar, 1:1 with `bruhs-web/src/components/sidebar.rs`:
//! brand header, one control row (search, choose project folder, new
//! thread), three-line thread cards (project · title · branch + provider)
//! with the Settled fold, and the footer view switcher. Sizes are the web's
//! Tailwind values in px; colours are the same bruhs tokens.

use crate::views::theme_regions::ThemeRegion as _;
use bruhs_ui::input::{Input, InputEvent, InputState, TextareaState};
use bruhs_ui::scroll::ScrollableElement;
use bruhs_ui::zoom::px;
use bruhs_ui::ActiveTheme;
use gpui::prelude::*;
use gpui::*;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

mod card_hover;
mod drag;
mod menu_popover;
mod viewport;
pub(super) use drag::SidebarDrag;
pub(super) use viewport::SidebarViewport;

pub(super) struct SidebarRename {
    id: String,
    input: Entity<InputState>,
    return_focus: FocusHandle,
    _subscription: Subscription,
}

use super::project_art::ProjectMark;
use super::{
    element_key, is_web_button_activation_key, provider_mark, redraw_on_edit, threads_ui,
    DesktopThread, KeyDownEvent, OpenProjectPicker, OpenSettings, SidebarSection, Workspace,
    WORKSPACE_SIDEBAR_BORDER_OPACITY,
};
use crate::views::sidebar_order::relative_age;
use crate::{assets, theme};
use bruhs_models::logic::menu::menu_step;
use bruhs_models::logic::sidebar::{
    is_thread_menu_key, snooze_presets, thread_menu, SnoozeWhen, ThreadMenuEntry, ThreadMenuFacts,
};
use bruhs_models::num::ToIntSaturating;
use bruhs_models::{AppView, EpochMs};
use bruhs_ui::press::on_press_view;

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
const SIDEBAR_HEADER_GAP_PX: f32 = 8.0;
const SIDEBAR_BRAND_ROW_HEIGHT_PX: f32 = theme::APP_HEADER_HEIGHT_PX;
const SIDEBAR_BRAND_LOGO_PX: f32 = 20.0;

type BrandLogoCache = Option<(ParsedSvg, Vec<(i32, Arc<RenderImage>)>)>;

/// Rasterize `bruhs.svg` at its exact display pixel size so Metal never
/// downsamples the fine halftone dot grid without mipmaps (which causes Moiré).
fn brand_logo_image(size_px: Pixels, scale_factor: f32, cx: &App) -> Option<Arc<RenderImage>> {
    thread_local! {
        static CACHE: RefCell<BrandLogoCache> =
            const { RefCell::new(None) };
    }
    let device_px = (f32::from(size_px) * scale_factor)
        .round()
        .to_i32_saturating()
        .max(1);
    CACHE.with(|slot| {
        let mut guard = slot.borrow_mut();
        let (parsed, entries) = match guard.as_mut() {
            Some(pair) => pair,
            None => {
                let parsed = cx.svg_renderer().parse_svg(assets::BRAND_LOGO_BYTES).ok()?;
                guard.insert((parsed, Vec::new()))
            }
        };
        if let Some((_, image)) = entries.iter().find(|(side, _)| *side == device_px) {
            return Some(Arc::clone(image));
        }
        let side = DevicePixels(device_px);
        let image = cx
            .svg_renderer()
            .render_parsed(parsed, SvgSize::ExactSize(size(side, side)))
            .ok()?;
        if entries.len() >= 8 {
            entries.remove(0);
        }
        entries.push((device_px, Arc::clone(&image)));
        Some(image)
    })
}
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
/// Small icons beside a card's 12px text.
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
const SIDEBAR_CARD_MODEL_MAX_WIDTH_PX: f32 = 138.0;
const SIDEBAR_CARD_MODEL_GAP_PX: f32 = 4.0;
const SIDEBAR_CARD_TITLE_TEXT_PX: f32 = theme::APP_UI_TEXT_SIZE_PX;
const SIDEBAR_CARD_ROW_GAP_PX: f32 = 6.0;
const SIDEBAR_CARD_DOT_PX: f32 = 9.0;
const SIDEBAR_CARD_DELETE_PX: f32 = theme::CONTROL_XS_PX;
const SIDEBAR_CARD_DELETE_INSET_PX: f32 = 8.0;
const SIDEBAR_CARD_SETTLE_WIDTH_PX: f32 = 62.0;
const SIDEBAR_CARD_QUICK_GAP_PX: f32 = 2.0;
const SIDEBAR_CARD_QUICK_WIDTH_PX: f32 =
    SIDEBAR_CARD_DELETE_PX + SIDEBAR_CARD_QUICK_GAP_PX + SIDEBAR_CARD_SETTLE_WIDTH_PX;
/// Web `right-9`: the actions button sits 4px left of delete.
const SIDEBAR_CARD_MENU_RIGHT_PX: f32 = SIDEBAR_CARD_DELETE_INSET_PX + SIDEBAR_CARD_DELETE_PX + 4.0;

#[derive(Clone)]
struct CardModel {
    id: String,
    handoff: bool,
    handoff_from: Option<String>,
    chat: Entity<super::ChatView>,
}

impl CardModel {
    fn description(&self, cx: &App) -> String {
        let mut description = bruhs_models::agent::model_label(&self.id);
        if let Some(from) = &self.handoff_from {
            description.push_str("\nHandoff parent model: ");
            description.push_str(&bruhs_models::agent::model_label(from));
        } else if self.handoff {
            description.push_str("\nHandoff thread");
        }
        let mut seen = std::collections::HashSet::from([self.id.as_str()]);
        for model in self
            .chat
            .read(cx)
            .messages
            .iter()
            .rev()
            .filter(|message| message.role == super::MessageRole::Assistant)
            .filter_map(|message| message.model.as_deref())
            .filter(|model| seen.insert(*model))
            .take(8)
        {
            description.push_str("\nEarlier model: ");
            description.push_str(&bruhs_models::agent::model_label(model));
        }
        description
    }

    fn render(&self, available: Pixels, window: &Window, cx: &App) -> AnyElement {
        let label = bruhs_models::agent::model_label(&self.id);
        let compact_label = label.strip_prefix("Claude ").unwrap_or(&label).to_owned();
        let label_width = sidebar_meta_text_width(&compact_label, window)
            + px(assets::PROVIDER_LOGO_SIZE_PX + SIDEBAR_CARD_MODEL_GAP_PX);
        let show_label = label_width <= available.min(px(SIDEBAR_CARD_MODEL_MAX_WIDTH_PX));
        let provider = bruhs_models::agent::AgentProvider::for_model(&self.id);
        div()
            .id("sidebar-card-model")
            .debug_selector(|| "desktop-thread-card-model".to_string())
            .flex_none()
            .flex()
            .items_center()
            .gap(px(SIDEBAR_CARD_MODEL_GAP_PX))
            .aria_label(format!("Model {label}"))
            .child(provider_mark(provider, cx))
            .when(show_label, |el| {
                el.child(
                    div()
                        .debug_selector(|| "desktop-thread-card-model-name".to_string())
                        .whitespace_nowrap()
                        .child(compact_label),
                )
            })
            .into_any_element()
    }
}

fn sidebar_meta_text_width(text: &str, window: &Window) -> Pixels {
    let text = text.lines().next().unwrap_or_default();
    window
        .text_system()
        .layout_line(
            text,
            px(SIDEBAR_LABEL_TEXT_PX),
            &[TextRun {
                len: text.len(),
                font: font(theme::UI_SMALL_FONT_FAMILY),
                ..Default::default()
            }],
            None,
        )
        .width
}

fn card_model_id<'a>(thread: &'a DesktopThread, chat: &'a super::ChatView) -> &'a str {
    if chat.messages.is_empty() {
        &chat.model_id
    } else {
        chat.messages
            .iter()
            .rev()
            .filter(|message| message.role == super::MessageRole::Assistant)
            .find_map(|message| message.model.as_deref())
            .unwrap_or(&thread.model_id)
    }
}

#[derive(Clone)]
struct CardIdentity {
    project: SharedString,
    mark: ProjectMark,
    title: SharedString,
    branch: Option<String>,
    role: Option<String>,
    model: CardModel,
    has_meta: bool,
}

impl CardIdentity {
    fn project_line(&self) -> Div {
        div()
            .h(px(SIDEBAR_CARD_LINE_HEIGHT_PX))
            .flex()
            .items_center()
            .gap(px(SIDEBAR_CARD_ROW_GAP_PX))
            .text_size(px(SIDEBAR_LABEL_TEXT_PX))
            .font_family(theme::UI_SMALL_FONT_FAMILY)
            .text_color(theme::rambutan_500())
            .child(self.mark.render(14.))
    }

    fn meta_line(
        &self,
        card_width: Pixels,
        pull: Option<String>,
        debug_key: u64,
        window: &Window,
        cx: &App,
    ) -> Div {
        let gap = px(SIDEBAR_CARD_ROW_GAP_PX);
        let branch_width = self
            .branch
            .as_ref()
            .map_or(px(0.), |branch| sidebar_meta_text_width(branch, window));
        let role_width = self.role.as_ref().map_or(px(0.), |role| {
            sidebar_meta_text_width(role, window).min(px(SIDEBAR_CARD_ROLE_MAX_WIDTH_PX)) + gap
        });
        let pull_width = pull
            .as_ref()
            .map_or(px(0.), |pull| sidebar_meta_text_width(pull, window) + gap);
        let model_space = card_width
            - px(2.0 * SIDEBAR_CARD_X_PADDING_PX)
            - gpui::px(2.0)
            - branch_width
            - role_width
            - pull_width
            - gap;
        div()
            .h(px(SIDEBAR_CARD_META_HEIGHT_PX))
            .flex()
            .items_center()
            .gap(gap)
            .text_size(px(SIDEBAR_LABEL_TEXT_PX))
            .font_family(theme::UI_SMALL_FONT_FAMILY)
            .text_color(theme::rambutan_500())
            .child(
                div()
                    .min_w_0()
                    .flex_1()
                    .when_some(self.branch.clone(), |el, branch| {
                        el.debug_selector(move || format!("desktop-thread-card-branch-{debug_key}"))
                            .truncate()
                            .child(branch)
                    }),
            )
            .when_some(self.role.clone(), |el, role| {
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
            .children(pull.map(|pull| div().flex_none().child(pull)))
            .child(self.model.render(model_space, window, cx))
    }
}
/// Web thread menu: `w-52`.
const THREAD_MENU_WIDTH_PX: f32 = 208.0;
/// Thread menu rows: shadcn `DropdownMenuItem` (32px, `px-2`).
const THREAD_MENU_ROW_PX: f32 = theme::CONTROL_PX;
const THREAD_MENU_ROW_X_PADDING_PX: f32 = 8.0;
/// Focus slots: one per `ThreadMenuItem`.
pub(super) const THREAD_MENU_MAX_ITEMS: usize =
    ThreadMenuItem::ALL.len() + super::snooze_options::MAX_OPTIONS;
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

pub const SIDEBAR_BRAND_LABEL: &str = "bruhs";
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
    UserSnooze(usize),
    InAnHour,
    LaterToday,
    Tomorrow,
    NextWeek,
    Regenerate,
    Discard,
    Archive,
    Delete,
    Custom,
    Rename,
}

impl ThreadMenuItem {
    const ALL: [Self; 13] = [
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
        Self::Archive,
        Self::Delete,
        Self::Rename,
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
            ThreadMenuEntry::Rename => Self::Rename,
            ThreadMenuEntry::RegenerateTitle { .. } => Self::Regenerate,
            ThreadMenuEntry::DiscardDraft => Self::Discard,
            ThreadMenuEntry::Separator => return None,
        })
    }

    /// Index into `Workspace::thread_menu_items`.
    fn slot(self) -> usize {
        if let Self::UserSnooze(index) = self {
            return Self::ALL.len() + index;
        }
        Self::ALL
            .iter()
            .position(|item| *item == self)
            .unwrap_or_default()
    }

    /// Element id and debug selector.
    fn id(self) -> &'static str {
        match self {
            Self::UserSnooze(i) => [
                "desktop-thread-snooze-0",
                "desktop-thread-snooze-1",
                "desktop-thread-snooze-2",
                "desktop-thread-snooze-3",
                "desktop-thread-snooze-4",
                "desktop-thread-snooze-5",
                "desktop-thread-snooze-6",
                "desktop-thread-snooze-7",
            ][i],
            Self::Archive => "desktop-thread-menu-archive",
            Self::Delete => "desktop-thread-menu-delete",
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
            Self::Rename => "desktop-thread-menu-rename",
        }
    }
}

/// What choosing `entry` does to thread `id`. Snooze times count from the
/// click, not from when the menu opened.
fn thread_menu_action(entry: ThreadMenuEntry) -> ThreadMenuAction {
    match entry {
        ThreadMenuEntry::Pin { .. } => menu_action(|this, id, window, cx| {
            this.request_thread_pin(id, window, cx);
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
        ThreadMenuEntry::Rename => {
            menu_action(|this, id, window, cx| this.start_sidebar_rename(id, window, cx))
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
    fn card_has_meta(&self, thread: &DesktopThread, cx: &App) -> bool {
        let chat = thread.chat.read(cx);
        thread.branch.is_some()
            || !chat.messages.is_empty()
            || chat.role.is_some()
            || matches!(
                self.thread_origins.get(&thread.id),
                Some(super::thread_details::Origin::Handoff(_))
            )
    }

    fn card_identity(&self, thread: &DesktopThread, cx: &Context<'_, Self>) -> CardIdentity {
        let chat = thread.chat.read(cx);
        let scope = chat.settings_project_path();
        let (project, mark) = if chat.scratch_folder().is_some() {
            ("Scratch".into(), ProjectMark::Folder)
        } else if thread.project_name.is_empty() {
            (SIDEBAR_NO_PROJECT_LABEL.into(), ProjectMark::Folder)
        } else {
            let name = self
                .project_list
                .title(scope)
                .unwrap_or(&thread.project_name);
            (name.into(), self.project_mark(scope, name, cx))
        };
        let (handoff, handoff_from) = match self.thread_origins.get(&thread.id) {
            Some(super::thread_details::Origin::Handoff(parent)) => (
                true,
                self.threads
                    .iter()
                    .find(|thread| &thread.id == parent)
                    .map(|thread| card_model_id(thread, thread.chat.read(cx)).to_owned()),
            ),
            _ => (false, None),
        };
        CardIdentity {
            project,
            mark,
            title: bruhs_models::logic::text::plain_title(&chat.thread_title)
                .into_owned()
                .into(),
            branch: thread.branch.clone(),
            role: chat.role_profile.as_ref().map(|role| role.name.clone()),
            model: CardModel {
                id: card_model_id(thread, chat).to_owned(),
                handoff,
                handoff_from,
                chat: thread.chat.clone(),
            },
            has_meta: self.card_has_meta(thread, cx),
        }
    }

    pub(super) fn sidebar_query(&self, cx: &App) -> String {
        self.sidebar_search.read(cx).value().to_string()
    }

    pub(super) fn render_sidebar(
        &self,
        width: Pixels,
        window: &Window,
        cx: &Context<'_, Self>,
    ) -> Div {
        let theme = cx.theme();
        let update_pill = self.render_sidebar_update_button(cx);
        let show_check = update_pill.is_none();
        let inner = div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .when_some(
                theme.color_role("cssSidebarInnerBackground"),
                |el, color| el.bg(color),
            )
            .children(crate::views::workspace::appearance::background_layer(
                "SidebarInner",
                cx,
            ))
            .when_some(
                theme.color_role("cssSidebarInnerForeground"),
                |el, color| el.text_color(color),
            )
            .child(self.render_sidebar_header(window, cx))
            .child(self.render_sidebar_controls(cx))
            .children(self.render_crew_rail(window, cx))
            .children(self.render_linked_conversations(cx))
            .child(self.render_sidebar_list(
                (width - px(2.0 * SIDEBAR_LIST_X_PADDING_PX) - gpui::px(1.0)).max(px(0.0)),
                window,
                cx,
            ))
            .child(self.render_sidebar_undo(cx))
            .children(update_pill)
            .child(self.render_sidebar_footer(show_check, cx));
        div()
            .w_full()
            .h_full()
            .relative()
            .debug_selector(|| "desktop-workspace-sidebar".to_string())
            .track_focus(
                self.area_focus
                    .handle(super::workspace_focus::Area::Sidebar),
            )
            .key_context("Sidebar")
            .bg(theme
                .color_role("cssSidebarBackground")
                .or_else(|| theme.color_role("sidebar"))
                .unwrap_or_else(theme::longan_900))
            .when(
                theme.color_role("cssSidebarBackground").is_none()
                    && theme.color_role("cssSidebarInnerBackground").is_none(),
                |el| el.theme_region("sidebar", cx),
            )
            .border_r_1()
            .children(crate::views::workspace::appearance::background_layer(
                "Sidebar", cx,
            ))
            .border_color(
                theme.color_role("cssSidebarBorder").unwrap_or_else(|| {
                    theme::longan_800().opacity(WORKSPACE_SIDEBAR_BORDER_OPACITY)
                }),
            )
            .font_family(theme::UI_FONT_FAMILY)
            .text_color(
                theme
                    .color_role("cssSidebarForeground")
                    .unwrap_or_else(theme::rambutan_300),
            )
            .flex()
            .flex_col()
            .child(inner)
    }

    /// The brand starts at the same height as the workspace navigation.
    fn render_sidebar_header(&self, window: &Window, cx: &Context<'_, Self>) -> AnyElement {
        let logo_size = px(SIDEBAR_BRAND_LOGO_PX);
        let logo = match brand_logo_image(logo_size, window.scale_factor(), cx) {
            Some(image) => img(image).size(logo_size),
            None => img(assets::BRAND_LOGO_PATH).size(logo_size),
        };
        div()
            .id("sidebar-header")
            .debug_selector(|| "desktop-sidebar-header".to_string())
            .h(px(SIDEBAR_BRAND_ROW_HEIGHT_PX))
            .px_4()
            .flex()
            .items_center()
            .gap(px(SIDEBAR_HEADER_GAP_PX))
            .flex_shrink_0()
            .window_control_area(WindowControlArea::Drag)
            .child(logo)
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
        on_activate: impl Fn(&mut Self, &mut Window, &mut Context<'_, Self>) + 'static,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
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
            .map(|el| on_press_view(el, cx, on_activate))
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
                                // Web: a bare <input> after the icon's gap.
                                .px_0()
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
    fn render_sidebar_list(
        &self,
        card_width: Pixels,
        window: &Window,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
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
        let drag_layout = self.sidebar_drag_layout(cx);
        let drag_overlays = drag_layout.map_or_else(Vec::new, |layout| {
            self.sidebar_drag_overlays(layout, window, cx)
        });
        let len = rows.len();
        let content = list(
            state.clone(),
            cx.processor(move |this, index: usize, window, cx| {
                let row = rows[index].clone();
                let (offset, hidden) =
                    drag_layout.map_or((px(0.), false), |layout| layout.row(index, row.section()));
                let element = match row {
                    viewport::SidebarRow::Thread {
                        index: thread_index,
                        folder,
                        ..
                    } => this.render_thread_card(
                        &this.threads[thread_index],
                        folder,
                        &focus[index],
                        card_width,
                        window,
                        cx,
                    ),
                    viewport::SidebarRow::Folder {
                        path,
                        checkout,
                        name,
                        count,
                        open,
                    } => {
                        let tooltip = checkout.clone();
                        let debug_path = path.clone();
                        let searching = !this.sidebar_query(cx).trim().is_empty();
                        let folder = div()
                            .id(SharedString::from(format!("sidebar-folder-{path}")))
                            .debug_selector(move || format!("desktop-sidebar-folder-{debug_path}"))
                            .track_focus(&focus[index].card)
                            .tab_index(0)
                            .role(Role::Button)
                            .aria_expanded(open)
                            .aria_label(format!("{name}, {count} threads"))
                            .cursor_pointer()
                            .border_1()
                            .border_color(gpui::transparent_black())
                            .rounded(px(6.))
                            .focus_visible(|el| el.border_color(theme::persimmon_400()))
                            .flex()
                            .items_center()
                            .gap_2()
                            .px_2()
                            .py_1()
                            .text_size(px(SIDEBAR_LABEL_TEXT_PX))
                            .text_color(theme::rambutan_400())
                            .when(!tooltip.is_empty(), |el| {
                                el.tooltip(move |w, cx| {
                                    bruhs_ui::tooltip::Tooltip::new(tooltip.clone()).build(w, cx)
                                })
                            })
                            .child(this.project_icon(checkout.as_ref(), SIDEBAR_SMALL_ICON_PX, cx))
                            .child(div().min_w_0().flex_1().truncate().child(name))
                            .child(count.to_string())
                            .child(
                                icon(
                                    assets::CHEVRON_DOWN_ICON_PATH,
                                    SIDEBAR_SMALL_ICON_PX,
                                    theme::rambutan_500(),
                                )
                                .when(!open, |el| {
                                    el.with_transformation(Transformation::rotate(Radians(
                                        -std::f32::consts::FRAC_PI_2,
                                    )))
                                }),
                            );
                        on_press_view(folder, cx, move |this, _, cx| {
                            if !searching {
                                if !this.labs_sidebar.collapsed.remove(path.as_ref()) {
                                    this.labs_sidebar.collapsed.insert(path.to_string());
                                }
                                this.persist_and_notify(cx);
                            }
                        })
                        .into_any_element()
                    }
                    viewport::SidebarRow::Shelf {
                        section,
                        count,
                        open,
                    } => this
                        .render_shelf_header(section, count, open, &focus[index].card, cx)
                        .into_any_element(),
                };
                let element = if hidden || offset != px(0.) {
                    div()
                        .relative()
                        .top(offset)
                        .when(hidden, |el| el.opacity(0.))
                        .child(element)
                        .into_any_element()
                } else {
                    element
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
        .pb(px(SIDEBAR_LIST_BOTTOM_PADDING_PX)
            + drag_layout.map_or(px(0.), drag::DragLayout::bottom_padding));
        div()
            .id("sidebar-threads")
            .debug_selector(|| "desktop-sidebar-threads".to_string())
            .role(Role::List)
            .aria_label("Threads")
            .on_drag_move(cx.listener(Self::update_sidebar_drag))
            .on_drop(cx.listener(Self::drop_sidebar_drag))
            .capture_any_mouse_up(cx.listener(|this, _, _, cx| {
                if this.sidebar_drag.is_some() {
                    let workspace = cx.weak_entity();
                    cx.defer(move |cx| {
                        let _ = workspace.update(cx, |this, cx| this.clear_sidebar_drag(cx));
                    });
                }
            }))
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
            .children(drag_overlays)
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
            .when(self.sidebar_drag_over_shelf(section, cx), |el| {
                el.border_color(theme::persimmon_400())
                    .text_color(theme::persimmon_400())
            })
            .focus_visible(|el| el.border_color(theme::persimmon_400()))
            .cursor_pointer()
            .hover(|el| el.text_color(theme::rambutan_300()))
            .map(|el| {
                on_press_view(el, cx, move |this, _window, cx| {
                    this.toggle_shelf(section, cx)
                })
            })
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
        } else if chat.is_working() {
            CardStatus::Working
        } else if self.unread_thread_ids.contains(&thread.id) {
            CardStatus::Unread
        } else {
            CardStatus::Idle
        }
    }

    /// One thread card: project (or "Chat") + pin + age/status, the title,
    /// then branch and model. Delete shows on hover at the top right.
    fn render_thread_card(
        &self,
        thread: &DesktopThread,
        folder: bool,
        focus: &viewport::SidebarRowFocus,
        card_width: Pixels,
        window: &Window,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        let chat = thread.chat.read(cx);
        let identity = self.card_identity(thread, cx);
        let is_active = self.active_view == AppView::Chat
            && self.active_thread_id.as_deref() == Some(thread.id.as_str());
        // While its menu is open the card keeps its hover look: the backdrop
        // that closes the menu covers the card.
        let menu_open = self.thread_menu_id.as_deref() == Some(thread.id.as_str());
        let snooze_open = menu_open && self.thread_menu_snooze_only;
        let status = self.card_status(thread, cx);
        let quick_actions = !folder
            && identity.has_meta
            && matches!(status, CardStatus::Idle | CardStatus::Unread)
            && self.section_for_thread(thread, cx) == SidebarSection::Active;
        let key = thread.chat.entity_id();
        let debug_key = element_key(&thread.id);
        let row_group: SharedString = format!("desktop-thread-row-{key}").into();
        let menu_group: SharedString = format!("desktop-thread-actions-{key}").into();
        let delete_group: SharedString = format!("desktop-thread-delete-{key}").into();
        let age = relative_age(thread.updated_at_ms, EpochMs::now());
        let id_for_select = thread.id.clone();
        let id_for_file_drop = thread.id.clone();
        let id_for_delete = thread.id.clone();
        let id_for_menu = thread.id.clone();
        let id_for_menu_key = thread.id.clone();
        let id_for_context_key = thread.id.clone();
        let id_for_context_click = thread.id.clone();
        let card_focus = focus.card.clone();
        let renaming = self
            .sidebar_rename
            .as_ref()
            .filter(|edit| edit.id == thread.id);
        let drag = (renaming.is_none() && self.sidebar_arrangement_enabled(cx))
            .then(|| self.sidebar_drag_item(thread, identity.clone()));
        let hover = (!menu_open && renaming.is_none() && !cx.has_active_drag()).then(|| {
            card_hover::ThreadCardHover::new(
                identity.clone(),
                chat.settings_project_path(),
                self,
                cx.weak_entity(),
                key,
            )
        });

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

        let top_line = identity
            .project_line()
            .child(
                div()
                    .debug_selector(move || format!("desktop-thread-card-project-{debug_key}"))
                    .min_w_0()
                    .flex_1()
                    .truncate()
                    .child(identity.project.clone()),
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
            });
        let status_line = div()
            .id(("desktop-thread-card-status-slot", key))
            .relative()
            .flex_shrink_0()
            .flex()
            .items_center()
            .justify_end()
            .h(px(SIDEBAR_CARD_LINE_HEIGHT_PX))
            .child(
                div()
                    .group_hover(row_group.clone(), |el| el.invisible())
                    .when(menu_open, |el| el.invisible())
                    .child(status_slot),
            )
            .when(
                quick_actions && renaming.is_none() && !menu_open || snooze_open,
                |el| {
                    el.group_hover(row_group.clone(), |el| {
                        el.w(px(SIDEBAR_CARD_QUICK_WIDTH_PX))
                    })
                    .when(snooze_open, |el| el.w(px(SIDEBAR_CARD_QUICK_WIDTH_PX)))
                    .child(self.sidebar_quick_actions(
                        thread,
                        row_group.clone(),
                        snooze_open,
                        cx,
                    ))
                },
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
            .when(renaming.is_none(), |el| el.child(identity.title.clone()))
            .when_some(renaming, |el, edit| {
                el.child(
                    Input::new(&edit.input)
                        .aria_label("Thread title")
                        .h(px(SIDEBAR_CARD_LINE_HEIGHT_PX)),
                )
            })
            .when(self.title_requests.contains_key(&thread.id), |row| {
                row.child(
                    div()
                        .id(("regenerating-title", key))
                        .absolute()
                        .role(Role::Status)
                        .aria_label("Regenerating title")
                        .a11y_synthetic_children(|tree| {
                            tree.parent_node().set_live(gpui::accesskit::Live::Polite)
                        }),
                )
            });

        // The role's id keeps the line; its current name fills it. A deleted
        // role leaves the name out (web).
        let meta_line = (if folder {
            self.labs_sidebar.two_lines
        } else {
            identity.has_meta
        })
        .then(|| {
            let pull = folder
                .then(|| self.known_thread_pulls.get(&thread.id))
                .flatten()
                .filter(|p| {
                    thread.branch.as_ref() == Some(&p.branch)
                        && super::labs::folder(thread, cx) == p.project
                })
                .map(|pull| format!("#{}", pull.number));
            identity
                .meta_line(card_width, pull, debug_key, window, cx)
                .mt(px(SIDEBAR_CARD_LINE_GAP_PX))
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
            .when(!quick_actions, |el| {
                el.group_hover(row_group.clone(), |el| el.visible())
            })
            .when(menu_open && !snooze_open, |el| el.visible())
            .hover(|el| el.bg(theme::longan_700().opacity(ACTIVE_BG_OPACITY_60)))
            .tab_index(0)
            .role(Role::Button)
            .aria_label(SIDEBAR_DELETE_LABEL)
            .border_1()
            .border_color(gpui::transparent_black())
            .focus_visible(|el| el.visible().border_color(theme::persimmon_400()))
            .cursor_pointer()
            .map(|el| {
                on_press_view(el, cx, move |this, window, cx| {
                    this.request_thread_delete(&id_for_delete, window, cx);
                })
            })
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
            .when(!quick_actions, |el| {
                el.group_hover(row_group.clone(), |el| el.visible())
            })
            .hover(|el| el.bg(theme::longan_700().opacity(ACTIVE_BG_OPACITY_60)))
            .when(menu_open && !snooze_open, |el| {
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
            let right = if snooze_open {
                SIDEBAR_CARD_X_PADDING_PX + SIDEBAR_CARD_SETTLE_WIDTH_PX + SIDEBAR_CARD_QUICK_GAP_PX
            } else {
                SIDEBAR_CARD_MENU_RIGHT_PX
            };
            div()
                .absolute()
                .right(px(right))
                .top(px(SIDEBAR_CARD_DELETE_INSET_PX))
                .size(px(SIDEBAR_CARD_DELETE_PX))
                .child(menu_popover::menu_popover(
                    size(px(SIDEBAR_CARD_DELETE_PX), px(SIDEBAR_CARD_DELETE_PX)),
                    self.render_thread_menu(thread, quick_actions, cx),
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
            .when_some(hover, |el, hover| {
                el.tooltip_show_delay(std::time::Duration::from_millis(450))
                    .tooltip(move |_, cx| cx.new(|_| hover.clone()).into())
            })
            .when(is_active, |el| {
                el.bg(theme::longan_800())
                    .border_1()
                    .border_color(gpui::white().opacity(RING_WHITE_10))
            })
            .when(!is_active, |el| {
                el.border_1()
                    .border_color(gpui::transparent_black())
                    .hover(move |el| {
                        el.bg(theme::longan_800().opacity(if quick_actions {
                            ACTIVE_BG_OPACITY_60
                        } else {
                            HOVER_BG_OPACITY_40
                        }))
                    })
            })
            .drag_over::<ExternalPaths>(|el, _, _, _| {
                el.bg(theme::longan_800())
                    .border_color(theme::persimmon_400())
            })
            .when_some(drag, |el, item| self.attach_sidebar_drag(el, item, cx))
            .on_drop(cx.listener(move |this, paths: &ExternalPaths, window, cx| {
                if paths.paths().is_empty() {
                    return;
                }
                let Some(chat) = this
                    .threads
                    .iter()
                    .find(|thread| thread.id == id_for_file_drop)
                    .map(|thread| thread.chat.clone())
                else {
                    return;
                };
                cx.stop_propagation();
                this.select_thread_by_id(&id_for_file_drop, cx);
                chat.update(cx, |chat, cx| {
                    chat.enqueue_paths(paths.paths(), cx);
                    chat.focus_composer(window, cx);
                });
            }))
            .when(renaming.is_none(), |el| {
                el.on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                    if event.is_keyboard() {
                        return;
                    }
                    cx.stop_propagation();
                    if event.click_count() == 2
                        && !event.modifiers().modified()
                        && this.active_view == AppView::Chat
                        && this.active_thread_id.as_deref() == Some(id_for_select.as_str())
                    {
                        this.start_sidebar_rename(&id_for_select, window, cx);
                    } else {
                        this.select_thread_by_id(&id_for_select, cx);
                    }
                }))
            })
            .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
                if card_focus.is_focused(window)
                    && this.sidebar_rename.is_none()
                    && is_web_button_activation_key(&event.keystroke.key)
                {
                    window.prevent_default();
                    cx.stop_propagation();
                    this.select_thread_by_id(&id_for_context_key, cx);
                    return;
                }
                let modifiers = event.keystroke.modifiers;
                if !card_focus.is_focused(window)
                    || modifiers.platform
                    || modifiers.control
                    || modifiers.alt
                {
                    return;
                }
                if !modifiers.shift
                    && this.sidebar_viewport.borrow().navigate_threads(
                        &event.keystroke.key,
                        window,
                        cx,
                    )
                {
                    window.prevent_default();
                    cx.stop_propagation();
                    cx.notify();
                    return;
                }
                let rename = event.keystroke.key == "f2" && !modifiers.shift;
                if !rename && !is_thread_menu_key(&event.keystroke.key, modifiers.shift) {
                    return;
                }
                window.prevent_default();
                cx.stop_propagation();
                if !event.is_held {
                    if rename {
                        this.start_sidebar_rename(&id_for_context_key, window, cx);
                    } else {
                        this.toggle_thread_menu(&id_for_context_key, window, cx);
                    }
                }
            }))
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, _, window, cx| {
                    window.prevent_default();
                    cx.stop_propagation();
                    this.toggle_thread_menu(&id_for_context_click, window, cx);
                }),
            )
            .child(if folder {
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(div().flex_1().min_w_0().child(title_line))
                    .child(status_line)
            } else {
                div().child(top_line.child(status_line)).child(title_line)
            })
            .children(meta_line)
            .child(menu_button)
            .child(delete_button)
            .children(menu_anchor)
            .into_any_element()
    }

    fn sidebar_quick_actions(
        &self,
        thread: &DesktopThread,
        row_group: SharedString,
        snooze_open: bool,
        cx: &Context<'_, Self>,
    ) -> Stateful<Div> {
        let key = thread.chat.entity_id();
        let debug_key = element_key(&thread.id);
        let snooze_id = thread.id.clone();
        let settle_id = thread.id.clone();
        let button = |id, name| {
            div()
                .id((id, key))
                .h(px(SIDEBAR_CARD_DELETE_PX))
                .flex()
                .items_center()
                .justify_center()
                .gap(px(4.))
                .rounded(px(SIDEBAR_RADIUS_SEED_PX))
                .role(Role::Button)
                .aria_label(name)
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .cursor_pointer()
                .text_color(theme::rambutan_400())
                .hover(|el| el.bg(theme::longan_700()).text_color(theme::rambutan_100()))
        };
        let snooze = button("desktop-thread-quick-snooze", "Snooze thread")
            .debug_selector(move || format!("desktop-thread-quick-snooze-{debug_key}"))
            .w(px(SIDEBAR_CARD_DELETE_PX))
            .aria_expanded(snooze_open)
            .when(snooze_open, |el| el.bg(theme::longan_700()))
            .child(icon(
                assets::CLOCK_ICON_PATH,
                SIDEBAR_SMALL_ICON_PX,
                theme::rambutan_400(),
            ));
        let settle = button("desktop-thread-quick-settle", "Settle thread")
            .debug_selector(move || format!("desktop-thread-quick-settle-{debug_key}"))
            .w(px(SIDEBAR_CARD_SETTLE_WIDTH_PX))
            .child(icon(
                assets::CHECK_ICON_PATH,
                SIDEBAR_SMALL_ICON_PX,
                theme::rambutan_400(),
            ))
            .child("Settle");
        div()
            .id(("desktop-thread-quick-actions", key))
            .absolute()
            .right_0()
            .top(px(
                (SIDEBAR_CARD_LINE_HEIGHT_PX - SIDEBAR_CARD_DELETE_PX) / 2.0
            ))
            .w(px(SIDEBAR_CARD_QUICK_WIDTH_PX))
            .invisible()
            .group_hover(row_group, |el| el.visible())
            .when(snooze_open, |el| el.visible())
            .flex()
            .gap(px(SIDEBAR_CARD_QUICK_GAP_PX))
            .child(on_press_view(snooze, cx, move |this, window, cx| {
                this.open_sidebar_snooze(&snooze_id, window, cx);
            }))
            .child(on_press_view(settle, cx, move |this, window, cx| {
                if this.thread_section(&settle_id, cx) == Some(SidebarSection::Active) {
                    this.thread_menu_id = None;
                    this.toggle_settled_by_id(&settle_id, cx);
                    this.focus_thread_menu_button(&settle_id, window, cx);
                }
            }))
    }

    fn open_sidebar_snooze(&mut self, id: &str, window: &mut Window, cx: &mut Context<'_, Self>) {
        let Some(thread) = self.threads.iter().find(|thread| thread.id == id) else {
            return;
        };
        if self.thread_menu_id.as_deref() == Some(id) && self.thread_menu_snooze_only {
            self.close_thread_menu(window, cx);
            return;
        }
        let first = self
            .snooze_choices(thread.chat.read(cx).settings_project_path())
            .map_or(ThreadMenuItem::InAnHour, |choices| {
                let now = EpochMs::now().as_millis();
                choices
                    .iter()
                    .take(super::snooze_options::MAX_OPTIONS)
                    .enumerate()
                    .filter_map(|(index, choice)| choice.until(now).map(|until| (until, index)))
                    .min()
                    .map_or(ThreadMenuItem::Custom, |(_, index)| {
                        ThreadMenuItem::UserSnooze(index)
                    })
            });
        self.thread_menu_id = Some(id.to_owned());
        self.thread_menu_snooze_only = true;
        self.thread_menu_items[first.slot()].focus(window, cx);
        cx.notify();
    }

    fn start_sidebar_rename(&mut self, id: &str, window: &mut Window, cx: &mut Context<'_, Self>) {
        if self.sidebar_rename.is_some() {
            return;
        }
        let Some(thread) = self.threads.iter().find(|thread| thread.id == id) else {
            return;
        };
        let Some([return_focus, _, _]) = self
            .sidebar_viewport
            .borrow()
            .row_targets(thread.chat.entity_id())
        else {
            return;
        };
        let title = thread.chat.read(cx).thread_title.clone();
        let input = cx.new(|cx| InputState::new(window, cx).default_value(title.clone()));
        let subscription = cx.subscribe_in(
            &input,
            window,
            |this, _, event: &InputEvent, window, cx| match event {
                InputEvent::PressEnter { .. } => this.finish_sidebar_rename(true, window, cx),
                InputEvent::Blur => this.finish_sidebar_rename(false, window, cx),
                _ => {}
            },
        );
        input.update(cx, |input, cx| {
            input.focus(window, cx);
            input.set_selected_range(0..title.len(), cx);
        });
        self.sidebar_rename = Some(SidebarRename {
            id: id.into(),
            input,
            return_focus,
            _subscription: subscription,
        });
        cx.notify();
    }

    fn finish_sidebar_rename(
        &mut self,
        restore_focus: bool,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let Some(edit) = self.sidebar_rename.take() else {
            return;
        };
        if let Some(thread) = self.threads.iter().find(|thread| thread.id == edit.id) {
            let draft = edit.input.read(cx).value();
            if let Some(title) = bruhs_models::logic::text::resolve_rename(
                &draft,
                &thread.chat.read(cx).thread_title,
            ) {
                thread.chat.update(cx, |chat, cx| {
                    chat.thread_title = title;
                    cx.notify();
                });
                self.title_requests.remove(&edit.id);
                self.persist_and_notify(cx);
            }
        }
        if restore_focus {
            edit.return_focus.focus(window, cx);
        }
        cx.notify();
    }

    pub(super) fn cancel_sidebar_rename(
        &mut self,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> bool {
        let Some(edit) = self.sidebar_rename.as_ref() else {
            return false;
        };
        if !edit.input.focus_handle(cx).is_focused(window) {
            return false;
        }
        let target = edit.return_focus.clone();
        self.sidebar_rename = None;
        target.focus(window, cx);
        cx.notify();
        true
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
    fn render_thread_menu(
        &self,
        thread: &DesktopThread,
        quick_actions: bool,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        let id = thread.id.clone();
        let chat = thread.chat.read(cx);
        let now = EpochMs::now().as_millis();
        let facts = ThreadMenuFacts {
            pinned: thread.pinned,
            settled: thread.settled,
            snoozed: thread.snoozed_until_ms.is_some_and(|until| until > now),
            title_action_disabled: self.title_requests.contains_key(&id),
            has_draft: self.active_thread_id.as_deref() == Some(id.as_str())
                && !chat.input_state.read(cx).value().is_empty(),
        };
        // Focusable items in visual order, for the arrow keys.
        let mut enabled: Vec<ThreadMenuItem> = Vec::new();
        let mut rows: Vec<AnyElement> = Vec::new();
        for entry in thread_menu(facts, now, threads_ui::utc_offset_minutes_east()) {
            if self.thread_menu_snooze_only
                && !matches!(
                    entry,
                    ThreadMenuEntry::Snooze(_) | ThreadMenuEntry::SnoozeCustom
                )
            {
                continue;
            }
            if let Some(choices) = self.snooze_choices(chat.settings_project_path()) {
                if let ThreadMenuEntry::Snooze(preset) = &entry {
                    if preset.when == SnoozeWhen::InAnHour {
                        let mut available: Vec<_> = choices
                            .iter()
                            .take(super::snooze_options::MAX_OPTIONS)
                            .enumerate()
                            .filter_map(|(i, c)| c.until(now).map(|t| (t, i, c)))
                            .collect();
                        available.sort_by_key(|(time, _, _)| *time);
                        for (_, index, choice) in available {
                            let item = ThreadMenuItem::UserSnooze(index);
                            let choice = choice.clone();
                            enabled.push(item);
                            rows.push(self.thread_menu_item(
                                item,
                                choice.label(),
                                true,
                                &id,
                                cx,
                                menu_action(move |this, id, _, cx| {
                                    if let Some(until) = choice.until(EpochMs::now().as_millis()) {
                                        this.snooze_thread_by_id(id, until, cx);
                                    }
                                }),
                            ));
                        }
                    }
                    continue;
                }
            }
            let Some(item) = ThreadMenuItem::for_entry(&entry) else {
                rows.push(thread_menu_separator());
                continue;
            };
            let on = entry.enabled();
            if on {
                enabled.push(item);
            }
            let label = if matches!(entry, ThreadMenuEntry::RegenerateTitle { enabled: false }) {
                "Regenerating…"
            } else {
                entry.label()
            };
            rows.push(self.thread_menu_item(item, label, on, &id, cx, thread_menu_action(entry)));
        }
        if !self.thread_menu_snooze_only {
            let can_archive = !chat.is_streaming;
            rows.push(thread_menu_separator());
            rows.push(self.thread_menu_item(
                ThreadMenuItem::Archive,
                "Archive thread",
                can_archive,
                &id,
                cx,
                menu_action(|this, id, window, cx| this.request_thread_archive(id, window, cx)),
            ));
            if can_archive {
                enabled.push(ThreadMenuItem::Archive);
            }
            if quick_actions {
                enabled.push(ThreadMenuItem::Delete);
                rows.push(self.thread_menu_item(
                    ThreadMenuItem::Delete,
                    SIDEBAR_DELETE_LABEL,
                    true,
                    &id,
                    cx,
                    menu_action(|this, id, window, cx| this.request_thread_delete(id, window, cx)),
                ));
            }
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
            .aria_label(if self.thread_menu_snooze_only {
                "Snooze thread"
            } else {
                "Thread actions"
            })
            .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
                let modifiers = event.keystroke.modifiers;
                if modifiers.platform || modifiers.control || modifiers.alt || modifiers.function {
                    return;
                }
                // Tab is routed by the workspace keystroke interceptor, which
                // runs before the window root's Tab binding (`key_decision`).
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
        label: impl Into<SharedString>,
        enabled: bool,
        thread_id: &str,
        cx: &Context<'_, Self>,
        action: ThreadMenuAction,
    ) -> AnyElement {
        let label = label.into();
        let id = item.id();
        let thread_id = thread_id.to_string();
        let run =
            move |this: &mut Workspace, window: &mut Window, cx: &mut Context<'_, Workspace>| {
                this.thread_menu_id = None;
                action(this, &thread_id, window, cx);
                if item != ThreadMenuItem::Rename || this.sidebar_rename.is_none() {
                    this.focus_thread_menu_button(&thread_id, window, cx);
                }
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
            .aria_label(label.clone())
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
        self.thread_menu_snooze_only = false;
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
            .map(|el| {
                on_press_view(el, cx, move |this, window, cx| {
                    this.activate_sidebar_view(view.clone(), window, cx);
                })
            })
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
    fn render_sidebar_footer(&self, show_check: bool, cx: &Context<'_, Self>) -> AnyElement {
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
            .aria_label(self.ui_language().text("Settings"))
            .aria_selected(settings_active)
            .border_1()
            .border_color(gpui::transparent_black())
            .focus_visible(|el| el.border_color(theme::persimmon_400()))
            .cursor_pointer()
            .map(|el| {
                on_press_view(el, cx, |this, window, cx| {
                    this.open_settings(&OpenSettings, window, cx);
                })
            })
            .child(icon(
                "icons/settings.svg",
                SIDEBAR_ICON_PX,
                if settings_active {
                    theme::rambutan_100()
                } else {
                    theme::rambutan_500()
                },
            ));
        let usage = on_press_view(
            bruhs_ui::press::focusable_control(
                div()
                    .id("desktop-sidebar-nav-usage")
                    .debug_selector(|| "desktop-sidebar-nav-usage".into())
                    .track_focus(&self.usage_hubs.dashboard_footer_focus)
                    .size(px(SIDEBAR_ICON_BUTTON_PX))
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(SIDEBAR_RADIUS_GRAPE_PX))
                    .hover(|el| el.bg(theme::longan_800().opacity(HOVER_BG_OPACITY_40)))
                    .child(icon(
                        assets::ACTIVITY_ICON_PATH,
                        SIDEBAR_ICON_PX,
                        theme::rambutan_500(),
                    )),
                Role::Button,
                self.ui_language().text("Usage"),
                bruhs_ui::press::Interactivity::Enabled,
            )
            .tooltip(|window, cx| bruhs_ui::tooltip::Tooltip::new("Usage").build(window, cx)),
            cx,
            |this, window, cx| {
                this.usage_hubs.dashboard_footer_focus.focus(window, cx);
                this.open_usage_dashboard(window, cx);
            },
        );
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
            usage.into_any_element(),
        ];
        if show_check {
            cells.push(self.sidebar_check_button(cx));
        }
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
        self.task_thread_view = None;
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
        language: crate::views::language::Language,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
        subscriptions: &mut Vec<Subscription>,
    ) -> Entity<InputState> {
        let sidebar_search = cx.new(|cx| {
            InputState::new(window, cx).placeholder(language.text(SIDEBAR_SEARCH_PLACEHOLDER))
        });
        subscriptions.push(redraw_on_edit(&sidebar_search, window, cx));
        sidebar_search
    }

    /// Global and per-project title instructions. Edits persist on this device.
    pub(super) fn new_title_instruction_inputs(
        global: String,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
        subscriptions: &mut Vec<Subscription>,
    ) -> (Entity<TextareaState>, Entity<TextareaState>) {
        let global_input = cx.new(|cx| {
            TextareaState::new(window, cx)
                .placeholder("prefix: Bug\nmax 48")
                .default_value(global)
        });
        let project_instructions =
            cx.new(|cx| TextareaState::new(window, cx).placeholder("prefix: Bug"));
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
    use super::{status_pulse_opacity, thread_matches, CardStatus};
    use crate::app_state::DesktopState;

    #[gpui::test]
    fn sidebar_unpinned_card_hover_shows_details_and_keeps_quick_actions_local(
        cx: &mut gpui::TestAppContext,
    ) {
        use gpui::Modifiers;
        let (workspace, cx, selected, target) = super::super::tests::thread_menu_workspace(cx);
        workspace.update(cx, |workspace, cx| {
            workspace.crew_rail_open = false;
            workspace.threads[1].branch = Some("main".into());
            cx.notify();
        });
        cx.run_until_parked();
        let key = super::element_key(&target);
        let selector = |name| Box::leak(format!("desktop-thread-{name}-{key}").into_boxed_str());
        let row = cx.debug_bounds(selector("row")).unwrap();
        cx.simulate_mouse_move(row.center(), None, Modifiers::default());
        cx.run_until_parked();
        cx.executor()
            .advance_clock(std::time::Duration::from_millis(500));
        cx.run_until_parked();
        let hover = cx.debug_bounds("desktop-sidebar-thread-hover").unwrap();
        let row = cx.debug_bounds(selector("row")).unwrap();
        assert!(hover.left() >= row.right());
        assert!((hover.top() - row.top()).abs() <= gpui::px(1.));

        let snooze = cx.debug_bounds(selector("quick-snooze")).unwrap();
        let project = cx
            .debug_bounds(Box::leak(
                format!("desktop-thread-card-project-{key}").into_boxed_str(),
            ))
            .unwrap();
        assert!(project.right() <= snooze.left());
        cx.simulate_click(snooze.center(), Modifiers::default());
        cx.run_until_parked();
        assert!(cx.debug_bounds("desktop-thread-menu-hour").is_some());
        assert!(cx.debug_bounds("desktop-thread-menu-pin").is_none());
        assert!(cx.debug_bounds("desktop-sidebar-thread-hover").is_none());
        cx.simulate_keystrokes("escape");
        cx.run_until_parked();
        cx.simulate_mouse_move(row.center(), None, Modifiers::default());
        cx.run_until_parked();
        let settle = cx.debug_bounds(selector("quick-settle")).unwrap();
        cx.simulate_click(settle.center(), Modifiers::default());
        workspace.read_with(cx, |workspace, _| {
            assert_eq!(
                workspace.active_thread_id.as_deref(),
                Some(selected.as_str())
            );
            assert!(
                workspace
                    .threads
                    .iter()
                    .find(|thread| thread.id == target)
                    .unwrap()
                    .settled
            );
        });
    }

    #[gpui::test]
    fn sidebar_card_keeps_only_the_provider_logo_when_the_model_name_cannot_fit(
        cx: &mut gpui::TestAppContext,
    ) {
        let (workspace, cx, _, _) = super::super::tests::thread_menu_workspace(cx);
        workspace.update(cx, |workspace, cx| {
            let thread = &mut workspace.threads[1];
            thread.branch = Some("main".into());
            thread
                .chat
                .update(cx, |chat, _| chat.model_id = "claude-sonnet-4-6".into());
            cx.notify();
        });
        cx.run_until_parked();
        assert!(cx.debug_bounds("desktop-thread-card-model-name").is_some());

        workspace.update(cx, |workspace, cx| {
            workspace.threads[1].branch = Some("feature/longer-project-checkout-names".into());
            cx.notify();
        });
        cx.run_until_parked();
        assert!(cx.debug_bounds("desktop-thread-card-model-name").is_none());
        let model = cx.debug_bounds("desktop-thread-card-model").unwrap();
        assert_eq!(
            model.size.width,
            super::px(crate::assets::PROVIDER_LOGO_SIZE_PX)
        );
    }

    #[gpui::test]
    fn sidebar_card_uses_the_recorded_models_and_handoff_origin(cx: &mut gpui::TestAppContext) {
        use crate::views::chat::Message;
        let (workspace, cx, parent, child) = super::super::tests::thread_menu_workspace(cx);
        workspace.update(cx, |workspace, cx| {
            workspace
                .thread_origins
                .insert(child, super::super::thread_details::Origin::Handoff(parent));
            let source = &workspace.threads[0];
            source.chat.update(cx, |chat, _| {
                chat.messages.push(Message::assistant_streaming(
                    1,
                    Some("claude-sonnet-4-6".into()),
                ));
            });
            let target = &workspace.threads[1];
            target
                .chat
                .update(cx, |chat, _| chat.model_id = "gpt-5.4".into());
            let draft = workspace.card_identity(target, cx);
            assert!(draft.has_meta);
            assert_eq!(draft.model.id, "gpt-5.4");
            assert!(draft.model.description(cx).contains("Handoff parent model"));
            target.chat.update(cx, |chat, _| {
                chat.messages
                    .push(Message::assistant_streaming(2, Some("gpt-5.4".into())));
                chat.messages.push(Message::assistant_streaming(
                    3,
                    Some("claude-opus-4-6".into()),
                ));
                chat.model_id = "claude-sonnet-4-6".into();
            });
            let recorded = workspace.card_identity(target, cx);
            assert_eq!(recorded.model.id, "claude-opus-4-6");
            assert!(recorded.model.description(cx).contains("Earlier model:"));
        });
    }

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

    #[gpui::test]
    fn thread_with_running_teamwork_task_shows_working_status_after_turn_ends(
        cx: &mut gpui::TestAppContext,
    ) {
        use super::super::{element_key, performance_tests::fixture};
        let (workspace, cx) = fixture(cx);
        cx.simulate_resize(gpui::size(gpui::px(1280.0), gpui::px(800.0)));
        let (id, task_id) = workspace.update_in(cx, |workspace, window, cx| {
            workspace.new_thread_in_project(
                "proj".into(),
                "/tmp/bruhs-teamwork".into(),
                window,
                cx,
            );
            let thread = workspace.threads.last().expect("a thread");
            let id = thread.id.clone();
            let task = cx.global::<DesktopState>().app.registry().register(
                id.clone(),
                bruhs_core::TaskKind::Swarm,
                "builder-1",
                serde_json::json!({ "worker_name": "builder-1", "persists_after_turn": true }),
            );
            workspace.sidebar_working_open = true;
            cx.notify();
            (id, task.id)
        });
        cx.run_until_parked();
        let key = element_key(&id);
        let mark: &'static str =
            Box::leak(format!("desktop-thread-card-status-{key}").into_boxed_str());
        assert!(
            cx.debug_bounds(mark).is_some(),
            "Working mark must show while teamwork worker runs even when not streaming"
        );
        assert!(
            cx.debug_bounds("desktop-chat-active-runs").is_some(),
            "Active runs strip must surface running background tasks directly in chat"
        );
        workspace.update_in(cx, |workspace, _window, cx| {
            let thread = workspace.threads.iter().find(|t| t.id == id).unwrap();
            assert!(!thread.chat.read(cx).is_streaming);
            assert_eq!(workspace.card_status(thread, cx), CardStatus::Working);
            cx.global::<DesktopState>()
                .app
                .registry()
                .stop_task(&task_id)
                .unwrap();
        });
        cx.run_until_parked();
        assert!(
            cx.debug_bounds("desktop-chat-active-runs").is_none(),
            "Active runs strip hides once all background runs stop"
        );
        workspace.update_in(cx, |workspace, _window, cx| {
            let thread = workspace.threads.iter().find(|t| t.id == id).unwrap();
            assert_ne!(workspace.card_status(thread, cx), CardStatus::Working);
        });
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

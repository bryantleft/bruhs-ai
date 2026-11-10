//! Chat header, hero heading and composer, 1:1 with
//! `bruhs-web/src/components/chat_header.rs` and `composer.rs`: a
//! `project / title` breadcrumb with the panel toggles, the "What should we
//! build in …?" heading, the rounded composer with model / access-mode /
//! Build / Stash chips and the persimmon send button, the project + branch
//! context strip under it, and the `/ commands · @ files · ⌘K palette`
//! hints. Sizes are the web's Tailwind values in px.

use gpui::prelude::*;
use gpui::*;
use gpui_component::input::Input;
use gpui_component::tooltip::Tooltip;

use bruhs_models::logic::commands::COMPACT_UNAVAILABLE;
use bruhs_models::logic::composer_text::{exceeds_message_limit_with, MESSAGE_LIMIT_HINT};
use bruhs_models::logic::context_chip::{extra_limit_chars, join_drafts, ContextChip};
use bruhs_models::logic::follow_up::{
    action_enabled_for, action_label, action_title, composer_action, ComposerAction,
};
use bruhs_models::logic::queue::QueuedPrompt;
use bruhs_models::logic::run_place::{WorkspaceChoice, CLAUDE_EFFORTS};

use super::{
    composer_input_disabled_like_web, composer_input_tab_index_like_web, element_key,
    is_web_button_activation_key, ChatView, ChatViewEvent, InputEnter, InputEscape, InputMoveDown,
    InputMoveUp, PermissionMode, StashDraft,
};
use crate::views::sidebar_order::relative_age;
use crate::{assets, theme};
use bruhs_models::agent::provider_for_model_id;
use bruhs_models::EpochMs;

// Header (web `h-[52px] px-5 gap-3`).
pub const CHAT_HEADER_HEIGHT_PX: f32 = 52.0;
pub const CHAT_HEADER_X_PADDING_PX: f32 = 20.0;
pub const CHAT_HEADER_GAP_PX: f32 = 12.0;
pub const CHAT_HEADER_TEXT_SIZE_PX: f32 = 14.0;
pub const CHAT_HEADER_CRUMB_MAX_WIDTH_PX: f32 = 160.0;
/// Web: the role tag is `max-w-32 text-label-sm` on `bg-longan-800/60`.
pub const CHAT_HEADER_ROLE_MAX_WIDTH_PX: f32 = 128.0;
pub const CHAT_HEADER_ROLE_TEXT_SIZE_PX: f32 = 12.0;
const CHAT_HEADER_ROLE_BG_OPACITY: f32 = 0.60;
pub const CHAT_HEADER_TOGGLE_HEIGHT_PX: f32 = 28.0;
pub const CHAT_HEADER_TOGGLE_X_PADDING_PX: f32 = 8.0;
pub const CHAT_HEADER_TOGGLE_GAP_PX: f32 = 4.0;
pub const CHAT_HEADER_TOGGLE_TEXT_SIZE_PX: f32 = 12.0;
pub const CHAT_HEADER_ICON_SIZE_PX: f32 = 14.0;
pub const CHAT_HEADER_TOGGLE_ACTIVE_BG_OPACITY: f32 = 0.60;
pub const CHAT_HEADER_TOGGLE_HOVER_BG_OPACITY: f32 = 0.40;

// Composer (web `max-w-3xl rounded-mango`).
pub const CHAT_COMPOSER_MAX_WIDTH_PX: f32 = 768.0;
pub const CHAT_COMPOSER_RADIUS_PX: f32 = 22.0;
pub const CHAT_COMPOSER_RING_OPACITY: f32 = 0.10;
/// Web `focus-within:inset-ring-persimmon-400/40` on the composer surface.
pub const CHAT_COMPOSER_FOCUS_RING_OPACITY: f32 = 0.40;
pub const CHAT_COMPOSER_INPUT_X_PADDING_PX: f32 = 16.0;
pub const CHAT_COMPOSER_INPUT_TOP_PADDING_PX: f32 = 14.0;
pub const CHAT_COMPOSER_FOOTER_X_PADDING_PX: f32 = 16.0;
pub const CHAT_COMPOSER_FOOTER_BOTTOM_PADDING_PX: f32 = 16.0;
pub const CHAT_COMPOSER_FOOTER_GAP_PX: f32 = 8.0;
pub const CHAT_COMPOSER_CHIP_GAP_PX: f32 = 4.0;
pub const CHAT_COMPOSER_PAGE_X_PADDING_PX: f32 = 24.0;
pub const CHAT_COMPOSER_BOTTOM_PADDING_PX: f32 = 16.0;
pub const CHAT_HERO_TEXT_SIZE_PX: f32 = 24.0;
pub const CHAT_HERO_BOTTOM_MARGIN_PX: f32 = 32.0;
pub const CHAT_HINTS_TOP_MARGIN_PX: f32 = 16.0;
pub const CHAT_HINTS_TEXT_SIZE_PX: f32 = 12.0;
pub const CHAT_KBD_HEIGHT_PX: f32 = 20.0;
pub const CHAT_KBD_MIN_WIDTH_PX: f32 = 20.0;

// Chips (web `CHIP`: h-7 px-2.5 gap-1.5 rounded-grape text-label-sm).
pub const CHIP_HEIGHT_PX: f32 = 28.0;
pub const CHIP_X_PADDING_PX: f32 = 10.0;
pub const CHIP_GAP_PX: f32 = 6.0;
pub const CHIP_RADIUS_PX: f32 = 8.0;
pub const CHIP_TEXT_SIZE_PX: f32 = 12.0;
pub const CHIP_ICON_SIZE_PX: f32 = 14.0;
pub const CHIP_CHEVRON_SIZE_PX: f32 = 12.0;
pub const CHIP_HOVER_BG_OPACITY: f32 = 0.50;
pub const CHIP_MODEL_MAX_WIDTH_PX: f32 = 192.0;

// Send / stop (web `size-8 rounded-orb`).
pub const SEND_BUTTON_SIZE_PX: f32 = 32.0;
pub const SEND_BUTTON_ICON_SIZE_PX: f32 = 14.0;
pub const SEND_BUTTON_DISABLED_OPACITY: f32 = 0.30;
pub const STOP_BUTTON_SQUARE_PX: f32 = 12.0;

// Context strip under the composer.
pub const STRIP_X_MARGIN_PX: f32 = 22.0;
pub const STRIP_X_PADDING_PX: f32 = 8.0;
pub const STRIP_TOP_PADDING_PX: f32 = 4.0;
pub const STRIP_BOTTOM_PADDING_PX: f32 = 4.0;
pub const STRIP_RADIUS_PX: f32 = 14.0;
pub const STRIP_BG_OPACITY: f32 = 0.70;
pub const STRIP_RING_OPACITY: f32 = 0.05;
pub const STRIP_TEXT_SIZE_PX: f32 = 12.0;
pub const STRIP_CHIP_HEIGHT_PX: f32 = 24.0;
pub const STRIP_ICON_SIZE_PX: f32 = 12.0;
pub const STRIP_DIRTY_BG_OPACITY: f32 = 0.15;

// Menus floated above the composer.
pub const MENU_WIDTH_PX: f32 = 288.0;
pub const STASH_MENU_WIDTH_PX: f32 = 320.0;
pub const MENU_ROW_TEXT_SIZE_PX: f32 = 12.0;
pub const MENU_MODE_LEFT_PX: f32 = 176.0;
pub const MENU_EFFORT_LEFT_PX: f32 = 280.0;
pub const MENU_STASH_LEFT_PX: f32 = 352.0;
pub const STASH_PREVIEW_CHARS: usize = 80;

pub const CHAT_INPUT_PLACEHOLDER: &str =
    "Ask anything, @ threads or files, # pull requests, $ skills, or / for commands";
pub const CHAT_HERO_PROJECT_PREFIX: &str = "What should we build in ";
pub const CHAT_HERO_PROJECT_SUFFIX: &str = "?";
pub const CHAT_HERO_GENERAL: &str = "What's on your mind?";
pub const CHAT_STASH_LABEL: &str = "Stash";
pub const CHAT_STASH_EMPTY: &str = "No stashed prompts. ⌘S stashes the current draft.";
pub const CHAT_BUILD_LABEL: &str = "Build";
pub const CHAT_PLAN_LABEL: &str = "Plan";

/// Where the composer sits: centered under the hero heading on an empty
/// thread, or docked under the transcript.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ComposerPlacement {
    Hero,
    Docked,
}

/// One chat-header toggle button (Diff, Tasks, right panel).
struct HeaderToggle {
    id: &'static str,
    selector: &'static str,
    icon_path: &'static str,
    label: Option<&'static str>,
    aria: &'static str,
    active: bool,
    /// Count shown after the label (running background tasks).
    badge: Option<usize>,
}

/// A parked draft (web `PromptStash` entry).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StashEntry {
    pub id: String,
    pub text: String,
    pub at_ms: EpochMs,
    /// Chips whose labels sit in `text`.
    pub chips: Vec<ContextChip>,
}

/// Web `model_label` (see `bruhs_models::agent::model_label`).
pub fn model_label(model_id: &str) -> String {
    bruhs_models::agent::model_label(model_id)
}

/// Web `stash_preview`: first line, cut to `max` chars with an ellipsis.
pub fn stash_preview(text: &str, max: usize) -> String {
    let first = text.lines().next().unwrap_or("").trim();
    if first.chars().count() <= max {
        return first.to_string();
    }
    let mut out: String = first.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

/// Web `restore_into_draft`: a stashed prompt joins a non-empty draft on a
/// new line instead of replacing it. Production joins chips with
/// `join_drafts`; tests keep this text-only path.
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "text-only restore; production joins chips, tests cover this path"
    )
)]
pub fn restore_into_draft(draft: &str, stashed: &str) -> String {
    if draft.trim().is_empty() {
        stashed.to_string()
    } else {
        format!("{}\n{}", draft.trim_end(), stashed)
    }
}

fn icon(path: &'static str, size: f32, color: Hsla) -> Svg {
    svg()
        .path(path)
        .size(px(size))
        .text_color(color)
        .flex_shrink_0()
}

impl ChatView {
    fn breadcrumb_project(&self) -> Option<String> {
        (self.project_scoped && !self.project_name.is_empty()).then(|| self.project_name.clone())
    }

    /// `project / title` with the Diff · Tasks · panel toggles at the right.
    pub(super) fn render_header(&self, cx: &Context<'_, Self>) -> AnyElement {
        let project = self.breadcrumb_project();
        let running = self.running_tasks_count;
        // The thread's role, by its current name. A deleted role shows nothing.
        let role = self.role_profile.as_ref().map(|role| role.name.clone());
        div()
            .id("chat-header")
            .debug_selector(|| "desktop-chat-header".to_string())
            .h(px(CHAT_HEADER_HEIGHT_PX))
            .px(px(CHAT_HEADER_X_PADDING_PX))
            .flex()
            .items_center()
            .gap(px(CHAT_HEADER_GAP_PX))
            .flex_shrink_0()
            .font_family(theme::UI_FONT_FAMILY)
            .child(
                div()
                    .min_w_0()
                    .flex_1()
                    .flex()
                    .items_center()
                    .gap_2()
                    .text_size(px(CHAT_HEADER_TEXT_SIZE_PX))
                    .when_some(project, |el, name| {
                        let label = format!("New thread in {name}");
                        el.child(
                            div()
                                .id("chat-header-project")
                                .debug_selector(|| "desktop-chat-header-project".to_string())
                                .max_w(px(CHAT_HEADER_CRUMB_MAX_WIDTH_PX))
                                .flex()
                                .items_center()
                                .gap(px(6.0))
                                .rounded(px(4.0))
                                .text_color(theme::rambutan_400())
                                .hover(|el| el.text_color(theme::rambutan_100()))
                                .tab_index(0)
                                .role(Role::Button)
                                .aria_label(label.clone())
                                .focus_visible(|el| {
                                    el.border_1().border_color(theme::persimmon_400())
                                })
                                .cursor_pointer()
                                .tooltip(move |window, cx| {
                                    Tooltip::new(label.clone()).build(window, cx)
                                })
                                .on_click(cx.listener(|_this, _ev, _window, cx| {
                                    cx.stop_propagation();
                                    cx.emit(ChatViewEvent::NewThreadInProject);
                                }))
                                .on_key_down(cx.listener(
                                    |_this, event: &KeyDownEvent, window, cx| {
                                        if is_web_button_activation_key(
                                            event.keystroke.key.as_str(),
                                        ) {
                                            window.prevent_default();
                                            cx.stop_propagation();
                                            cx.emit(ChatViewEvent::NewThreadInProject);
                                        }
                                    },
                                ))
                                .child(icon(
                                    assets::FOLDER_ICON_PATH,
                                    CHAT_HEADER_ICON_SIZE_PX,
                                    theme::rambutan_400(),
                                ))
                                .child(div().min_w_0().truncate().child(name)),
                        )
                        .child(
                            div()
                                .flex_shrink_0()
                                .text_color(theme::rambutan_600())
                                .child("/"),
                        )
                    })
                    .child(
                        div()
                            .debug_selector(|| "desktop-chat-header-title".to_string())
                            .min_w_0()
                            // With a role the tag sits right after the title
                            // (web), so the title takes only its own width.
                            .when(role.is_none(), |el| el.flex_1())
                            .truncate()
                            .text_color(theme::rambutan_100())
                            .child(
                                bruhs_models::logic::text::plain_title(&self.thread_title)
                                    .into_owned(),
                            ),
                    )
                    .when_some(role, |el, role| {
                        let hint = format!("Role: {role}. /role changes it.");
                        el.child(
                            div()
                                .id("chat-header-role")
                                .debug_selector(|| "desktop-chat-header-role".to_string())
                                .flex_shrink_0()
                                .max_w(px(CHAT_HEADER_ROLE_MAX_WIDTH_PX))
                                .truncate()
                                .rounded(px(4.0))
                                .bg(theme::longan_800().opacity(CHAT_HEADER_ROLE_BG_OPACITY))
                                .px(px(6.0))
                                .text_size(px(CHAT_HEADER_ROLE_TEXT_SIZE_PX))
                                .text_color(theme::rambutan_300())
                                .tooltip(move |window, cx| {
                                    Tooltip::new(hint.clone()).build(window, cx)
                                })
                                .child(role),
                        )
                        // Keeps the title and its tag at the left.
                        .child(div().flex_1())
                    }),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(CHAT_HEADER_TOGGLE_GAP_PX))
                    .flex_shrink_0()
                    .child(self.render_header_toggle(
                        HeaderToggle {
                            id: "diff-toggle",
                            selector: "desktop-chat-diff-toggle",
                            icon_path: assets::FILE_DIFF_ICON_PATH,
                            label: Some("Diff"),
                            aria: "Diff (⌘D)",
                            active: self.diff_panel_visible,
                            badge: None,
                        },
                        |_this, _window, cx| cx.emit(ChatViewEvent::ToggleDiff),
                        cx,
                    ))
                    .child(self.render_header_toggle(
                        HeaderToggle {
                            id: "tasks-toggle",
                            selector: "desktop-chat-tasks-toggle",
                            icon_path: assets::ACTIVITY_ICON_PATH,
                            label: Some("Tasks"),
                            aria: "Tasks (⌘⇧T)",
                            active: self.tasks_panel_visible,
                            badge: (running > 0).then_some(running),
                        },
                        |_this, _window, cx| cx.emit(ChatViewEvent::ToggleTasks),
                        cx,
                    ))
                    .child(self.render_header_toggle(
                        HeaderToggle {
                            id: "panel-toggle",
                            selector: "desktop-chat-panel-toggle",
                            icon_path: assets::PANEL_RIGHT_ICON_PATH,
                            label: None,
                            aria: "Toggle right panel (⌘⌥B)",
                            active: self.right_panel_open,
                            badge: None,
                        },
                        |_this, _window, cx| cx.emit(ChatViewEvent::TogglePanel),
                        cx,
                    )),
            )
            .into_any_element()
    }

    fn render_header_toggle(
        &self,
        toggle: HeaderToggle,
        on_activate: impl Fn(&mut Self, &mut Window, &mut Context<'_, Self>) + Clone + 'static,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        let HeaderToggle {
            id,
            selector,
            icon_path,
            label,
            aria,
            active,
            badge,
        } = toggle;
        let on_key = on_activate.clone();
        let color = if active {
            theme::rambutan_100()
        } else {
            theme::rambutan_400()
        };
        div()
            .id(id)
            .debug_selector(move || selector.to_string())
            .h(px(CHAT_HEADER_TOGGLE_HEIGHT_PX))
            .when(label.is_some(), |el| {
                el.px(px(CHAT_HEADER_TOGGLE_X_PADDING_PX))
            })
            .when(label.is_none(), |el| {
                el.w(px(CHAT_HEADER_TOGGLE_HEIGHT_PX)).justify_center()
            })
            .rounded(px(CHIP_RADIUS_PX))
            .flex()
            .items_center()
            .gap(px(CHAT_HEADER_TOGGLE_GAP_PX))
            .text_size(px(CHAT_HEADER_TOGGLE_TEXT_SIZE_PX))
            .font_family(theme::UI_SMALL_FONT_FAMILY)
            .text_color(color)
            .when(active, |el| {
                el.bg(theme::longan_800().opacity(CHAT_HEADER_TOGGLE_ACTIVE_BG_OPACITY))
            })
            .when(!active, |el| {
                el.hover(|el| {
                    el.bg(theme::longan_800().opacity(CHAT_HEADER_TOGGLE_HOVER_BG_OPACITY))
                        .text_color(theme::rambutan_100())
                })
            })
            .tab_index(0)
            .role(Role::Button)
            .aria_label(aria)
            .focus_visible(|el| el.border_2().border_color(theme::persimmon_400()))
            .cursor_pointer()
            .tooltip(move |window, cx| Tooltip::new(aria).build(window, cx))
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
            .child(icon(icon_path, CHAT_HEADER_ICON_SIZE_PX, color))
            .when_some(label, |el, label| el.child(label))
            .when_some(badge, |el, n| {
                el.child(
                    div()
                        .debug_selector(|| "desktop-chat-tasks-count".to_string())
                        .px_1()
                        .rounded(px(4.0))
                        .bg(theme::pandan_400().opacity(0.20))
                        .text_color(theme::pandan_300())
                        .child(n.to_string()),
                )
            })
            .into_any_element()
    }

    /// Heading + composer + context strip (+ hints on an empty thread).
    pub(super) fn render_composer_block(
        &self,
        placement: ComposerPlacement,
        window: &Window,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        let hero = placement == ComposerPlacement::Hero;
        div()
            .id("composer-block")
            .debug_selector(|| "desktop-composer".to_string())
            .w_full()
            .max_w(px(CHAT_COMPOSER_MAX_WIDTH_PX))
            .mx_auto()
            .flex_shrink_0()
            .flex()
            .flex_col()
            .font_family(theme::UI_FONT_FAMILY)
            .when(hero, |el| el.child(self.render_hero_heading(cx)))
            .child(self.render_file_mention_hints(cx))
            .child(self.render_pull_hints(cx))
            .child(self.render_skill_hints(cx))
            .child(self.render_slash_command_hints(cx))
            .when_some(self.render_goal_row(), |el, row| el.child(row))
            .child(self.render_composer_surface(window, cx))
            .children(self.render_context_strip(cx))
            .when(hero, |el| el.child(self.render_hints()))
            .into_any_element()
    }

    fn render_hero_heading(&self, cx: &Context<'_, Self>) -> AnyElement {
        let row = div()
            .id("chat-hero")
            .debug_selector(|| "desktop-chat-hero".to_string())
            .mb(px(CHAT_HERO_BOTTOM_MARGIN_PX))
            .w_full()
            .flex()
            .items_center()
            .justify_center()
            .font_family(theme::HEADING_FONT_FAMILY)
            .font_weight(FontWeight::MEDIUM)
            .text_size(px(CHAT_HERO_TEXT_SIZE_PX))
            .text_color(theme::rambutan_100());
        match self.breadcrumb_project() {
            Some(name) => row
                .child(div().flex_shrink_0().child(CHAT_HERO_PROJECT_PREFIX))
                .child(
                    div()
                        .id("chat-hero-project")
                        .debug_selector(|| "desktop-chat-hero-project".to_string())
                        .min_w_0()
                        .flex_1()
                        .truncate()
                        .border_b_1()
                        .border_color(theme::rambutan_500().opacity(0.6))
                        .hover(|el| el.border_color(theme::rambutan_300()))
                        .tab_index(0)
                        .role(Role::Button)
                        .aria_label("Change project")
                        .focus_visible(|el| el.border_color(theme::persimmon_400()))
                        .cursor_pointer()
                        .on_click(cx.listener(|_this, _ev, _window, cx| {
                            cx.stop_propagation();
                            cx.emit(ChatViewEvent::OpenProjectPicker);
                        }))
                        .child(name),
                )
                .child(div().flex_shrink_0().child(CHAT_HERO_PROJECT_SUFFIX))
                .into_any_element(),
            None => row.child(CHAT_HERO_GENERAL).into_any_element(),
        }
    }

    /// The rounded surface: input on top, chips + send below, menus floated
    /// above it.
    fn render_composer_surface(&self, window: &Window, cx: &Context<'_, Self>) -> AnyElement {
        // Web `focus-within`: the surface rings persimmon while the input has
        // focus.
        let focused = self.input_state.focus_handle(cx).is_focused(window);
        div()
            .id("composer-surface")
            .debug_selector(|| "desktop-composer-surface".to_string())
            .relative()
            .w_full()
            .rounded(px(CHAT_COMPOSER_RADIUS_PX))
            .bg(theme::longan_900())
            .border_1()
            .map(|el| {
                if focused {
                    el.border_color(
                        theme::persimmon_400().opacity(CHAT_COMPOSER_FOCUS_RING_OPACITY),
                    )
                } else {
                    el.border_color(gpui::white().opacity(CHAT_COMPOSER_RING_OPACITY))
                }
            })
            .flex()
            .flex_col()
            .drag_over::<ExternalPaths>(|el, _, _, _| el.border_color(theme::persimmon_400()))
            .on_drop(cx.listener(|this, paths: &ExternalPaths, _window, cx| {
                this.enqueue_paths(paths.paths(), cx);
            }))
            .when_some(self.render_queued_prompts(cx), |el, rows| el.child(rows))
            .when_some(self.render_text_attachments(cx), |el, rows| el.child(rows))
            .when_some(self.render_file_offers(cx), |el, rows| el.child(rows))
            .child(self.render_composer_input(cx))
            .when(self.message_over_limit(cx), |el| {
                el.child(self.render_message_limit())
            })
            .child(self.render_composer_footer(cx))
            .when(self.model_picker_open, |el| {
                el.child(
                    div()
                        .absolute()
                        .left(px(CHAT_COMPOSER_FOOTER_X_PADDING_PX))
                        .bottom(relative(1.0))
                        .mb_2()
                        .occlude()
                        .child(self.render_model_picker(cx)),
                )
            })
            .when(self.permission_menu_open, |el| {
                el.child(
                    div()
                        .absolute()
                        .left(px(MENU_MODE_LEFT_PX))
                        .bottom(relative(1.0))
                        .mb_2()
                        .occlude()
                        .child(self.render_permission_menu(cx)),
                )
            })
            .when(self.effort_menu_open && self.effort_chip_visible(), |el| {
                el.child(
                    div()
                        .absolute()
                        .left(px(MENU_EFFORT_LEFT_PX))
                        .bottom(relative(1.0))
                        .mb_2()
                        .occlude()
                        .child(self.render_effort_menu(cx)),
                )
            })
            .when(self.stash_open, |el| {
                el.child(
                    div()
                        .absolute()
                        .left(px(MENU_STASH_LEFT_PX))
                        .bottom(relative(1.0))
                        .mb_2()
                        .occlude()
                        .child(self.render_stash_menu(cx)),
                )
            })
            .into_any_element()
    }

    fn render_composer_input(&self, cx: &Context<'_, Self>) -> AnyElement {
        // a11y-exempt: like the web's textarea, the input draws no ring of its
        // own — `render_composer_surface` rings the whole composer instead.
        div()
            .debug_selector(|| "desktop-composer-input".to_string())
            .px(px(CHAT_COMPOSER_INPUT_X_PADDING_PX))
            .pt(px(CHAT_COMPOSER_INPUT_TOP_PADDING_PX))
            .capture_action(cx.listener(|this, _: &InputMoveUp, window, cx| {
                if this.handle_composer_hint_key("up", window, cx) {
                    cx.stop_propagation();
                }
            }))
            .capture_action(cx.listener(|this, _: &InputMoveDown, window, cx| {
                if this.handle_composer_hint_key("down", window, cx) {
                    cx.stop_propagation();
                }
            }))
            .capture_action(cx.listener(|this, _: &InputEscape, window, cx| {
                if this.permission_menu_open
                    || this.stash_open
                    || this.branch_menu_open
                    || this.workspace_menu_open
                    || this.effort_menu_open
                {
                    this.close_composer_menus(cx);
                    cx.stop_propagation();
                } else if this.handle_composer_hint_key("escape", window, cx) {
                    cx.stop_propagation();
                }
            }))
            .capture_action(cx.listener(|this, action: &InputEnter, window, cx| {
                if this.queue_edit.is_some() {
                    cx.stop_propagation();
                    return;
                }
                if !action.secondary && this.apply_selected_composer_hint(window, cx) {
                    cx.stop_propagation();
                }
            }))
            .child(
                Input::new(&self.input_state)
                    .aria_label("Message")
                    .appearance(false)
                    .bordered(false)
                    .focus_bordered(false)
                    .cleanable(false)
                    .font_family(theme::CODE_FONT_FAMILY)
                    .disabled(composer_input_disabled_like_web(self.is_streaming))
                    .tab_index(composer_input_tab_index_like_web(self.is_streaming)),
            )
            .into_any_element()
    }

    /// One footer chip. `active` is the pressed look (Plan mode).
    #[expect(
        clippy::too_many_arguments,
        reason = "mirrors the web chip's props one to one; every call site passes all of them"
    )]
    fn render_chip(
        &self,
        id: &'static str,
        selector: &'static str,
        aria: String,
        active: bool,
        expanded: Option<bool>,
        leading: AnyElement,
        label: AnyElement,
        trailing: Option<AnyElement>,
        on_activate: impl Fn(&mut Self, &mut Window, &mut Context<'_, Self>) + Clone + 'static,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        let on_key = on_activate.clone();
        let tooltip = aria.clone();
        div()
            .id(id)
            .debug_selector(move || selector.to_string())
            .h(px(CHIP_HEIGHT_PX))
            .px(px(CHIP_X_PADDING_PX))
            .rounded(px(CHIP_RADIUS_PX))
            .flex()
            .items_center()
            .gap(px(CHIP_GAP_PX))
            .flex_shrink_0()
            .text_size(px(CHIP_TEXT_SIZE_PX))
            .font_family(theme::UI_SMALL_FONT_FAMILY)
            .text_color(if active {
                theme::rambutan_100()
            } else {
                theme::rambutan_400()
            })
            .when(active, |el| el.bg(theme::longan_800()))
            .hover(|el| {
                el.bg(theme::longan_800().opacity(CHIP_HOVER_BG_OPACITY))
                    .text_color(theme::rambutan_100())
            })
            .tab_index(0)
            .role(Role::Button)
            .aria_label(aria)
            .when_some(expanded, |el, open| el.aria_expanded(open))
            .focus_visible(|el| el.border_2().border_color(theme::persimmon_400()))
            .cursor_pointer()
            .tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx))
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
            .child(leading)
            .child(label)
            .children(trailing)
            .into_any_element()
    }

    fn chevron(&self) -> AnyElement {
        icon(
            assets::CHEVRON_DOWN_ICON_PATH,
            CHIP_CHEVRON_SIZE_PX,
            theme::rambutan_500(),
        )
        .into_any_element()
    }

    /// Chips left, stop/send right (web composer footer).
    fn render_composer_footer(&self, cx: &Context<'_, Self>) -> AnyElement {
        let stash_count = self.stash.len();
        let draft_empty = self.input_state.read(cx).value().trim().is_empty();
        let separate_stop = self.is_streaming && !draft_empty;
        let is_plan = self.permission_mode == PermissionMode::Plan;
        let model_chip = self.render_chip(
            "desktop-model-status",
            "desktop-model-status",
            "Change model (⌘⇧M)".to_string(),
            false,
            Some(self.model_picker_open),
            self.provider_mark(cx),
            div()
                .max_w(px(CHIP_MODEL_MAX_WIDTH_PX))
                .truncate()
                .child(model_label(&self.model_id))
                .into_any_element(),
            Some(self.chevron()),
            |this, window, cx| this.toggle_model_picker(window, cx),
            cx,
        );
        let mode_chip = self.render_chip(
            "desktop-permission-mode",
            "desktop-permission-mode",
            self.permission_mode
                .chip_title(provider_for_model_id(&self.model_id))
                .to_string(),
            false,
            Some(self.permission_menu_open),
            icon(
                assets::permission_mode_icon_path(self.permission_mode),
                CHIP_ICON_SIZE_PX,
                theme::rambutan_400(),
            )
            .into_any_element(),
            div().child(self.permission_mode.label()).into_any_element(),
            Some(self.chevron()),
            |this, _window, cx| {
                this.permission_menu_open = !this.permission_menu_open;
                this.stash_open = false;
                this.model_picker_open = false;
                this.branch_menu_open = false;
                this.workspace_menu_open = false;
                this.effort_menu_open = false;
                cx.notify();
            },
            cx,
        );
        let plan_chip = self.render_chip(
            "desktop-plan-toggle",
            "desktop-plan-toggle",
            if is_plan {
                "Plan mode — click to return to build mode".to_string()
            } else {
                "Build mode — click to enter plan mode".to_string()
            },
            is_plan,
            None,
            icon(
                if is_plan {
                    assets::CLIPBOARD_LIST_ICON_PATH
                } else {
                    assets::BOT_ICON_PATH
                },
                CHIP_ICON_SIZE_PX,
                if is_plan {
                    theme::rambutan_100()
                } else {
                    theme::rambutan_400()
                },
            )
            .into_any_element(),
            div()
                .child(if is_plan {
                    CHAT_PLAN_LABEL
                } else {
                    CHAT_BUILD_LABEL
                })
                .into_any_element(),
            None,
            |this, _window, cx| this.toggle_plan(cx),
            cx,
        );
        let attach_chip = self.render_chip(
            "desktop-attach-file",
            "desktop-attach-file",
            "Attach files".to_string(),
            false,
            None,
            icon(
                assets::TOOL_FILE_ICON_PATH,
                CHIP_ICON_SIZE_PX,
                theme::rambutan_400(),
            )
            .into_any_element(),
            div().child("Attach").into_any_element(),
            None,
            |this, window, cx| this.attach_files(window, cx),
            cx,
        );
        let stash_chip = self.render_chip(
            "desktop-stash-chip",
            "desktop-stash-chip",
            "Prompt stash (⌘S stashes the draft)".to_string(),
            false,
            Some(self.stash_open),
            icon(
                assets::ARCHIVE_ICON_PATH,
                CHIP_ICON_SIZE_PX,
                theme::rambutan_400(),
            )
            .into_any_element(),
            div().child(CHAT_STASH_LABEL).into_any_element(),
            (stash_count > 0).then(|| {
                div()
                    .debug_selector(|| "desktop-stash-count".to_string())
                    .px(px(6.0))
                    .rounded_full()
                    .bg(theme::blueberry_500().opacity(0.20))
                    .text_color(theme::blueberry_400())
                    .child(stash_count.to_string())
                    .into_any_element()
            }),
            |this, _window, cx| {
                this.stash_open = !this.stash_open;
                this.permission_menu_open = false;
                this.model_picker_open = false;
                this.branch_menu_open = false;
                this.workspace_menu_open = false;
                this.effort_menu_open = false;
                cx.notify();
            },
            cx,
        );
        div()
            .debug_selector(|| "desktop-status-bar".to_string())
            .px(px(CHAT_COMPOSER_FOOTER_X_PADDING_PX))
            .pb(px(CHAT_COMPOSER_FOOTER_BOTTOM_PADDING_PX))
            .flex()
            .items_center()
            .justify_between()
            .gap(px(CHAT_COMPOSER_FOOTER_GAP_PX))
            .child(
                div()
                    .min_w_0()
                    .flex_1()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap(px(CHAT_COMPOSER_CHIP_GAP_PX))
                    .child(model_chip)
                    .child(mode_chip)
                    .child(plan_chip)
                    .when(self.effort_chip_visible(), |row| {
                        row.child(self.render_effort_chip(cx))
                    })
                    .child(attach_chip)
                    .child(stash_chip),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .flex_shrink_0()
                    .when(self.return_chip_visible(), |el| {
                        el.child(self.render_return_chip(cx))
                    })
                    .when(
                        self.messages.iter().any(|message| message.usage.is_some()),
                        |el| el.child(self.render_compact_control(cx)),
                    )
                    .when(separate_stop, |el| el.child(self.render_stop_button(cx)))
                    .child(self.render_send_button(cx)),
            )
            .into_any_element()
    }

    /// The thread goal stays until `/goal clear`. There is no progress percent.
    fn render_goal_row(&self) -> Option<AnyElement> {
        let goal = self.goal.as_ref()?;
        let text = goal.text.clone();
        let paused = goal.paused;
        Some(
            div()
                .debug_selector(|| "desktop-thread-goal".to_string())
                .mb_2()
                .px_1()
                .flex()
                .items_center()
                .gap_2()
                .text_size(px(12.0))
                .font_family(theme::UI_SMALL_FONT_FAMILY)
                .text_color(theme::rambutan_400())
                .child(div().flex_shrink_0().child("Goal"))
                .child(div().min_w_0().flex_1().truncate().child(text))
                .when(paused, |el| {
                    el.child(
                        div()
                            .flex_shrink_0()
                            .text_color(theme::rambutan_500())
                            .child("Paused"),
                    )
                })
                .into_any_element(),
        )
    }

    /// Shown once a turn has reported usage. Claude sends `/compact`. Codex
    /// keeps the unavailable notice.
    fn render_compact_control(&self, cx: &Context<'_, Self>) -> AnyElement {
        let title = if self.effort_chip_visible() {
            "Summarize this Claude thread"
        } else {
            COMPACT_UNAVAILABLE
        };
        div()
            .id("compact-context")
            .debug_selector(|| "desktop-compact-context".to_string())
            .h(px(24.0))
            .px(px(8.0))
            .rounded(px(4.0))
            .flex()
            .items_center()
            .text_size(px(12.0))
            .font_family(theme::UI_SMALL_FONT_FAMILY)
            .text_color(theme::rambutan_400())
            .hover(|el| {
                el.bg(theme::longan_800().opacity(0.60))
                    .text_color(theme::rambutan_200())
            })
            .cursor_pointer()
            .tab_index(0)
            .role(Role::Button)
            .aria_label(title)
            .tooltip(move |window, cx| Tooltip::new(title).build(window, cx))
            .focus_visible(|el| el.border_2().border_color(theme::persimmon_400()))
            .on_click(cx.listener(|this, _ev, _window, cx| {
                cx.stop_propagation();
                this.apply_local_command("/compact", cx);
            }))
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                if is_web_button_activation_key(event.keystroke.key.as_str()) {
                    window.prevent_default();
                    cx.stop_propagation();
                    this.apply_local_command("/compact", cx);
                }
            }))
            .child("Compact")
            .into_any_element()
    }

    /// Compact or Full beside send after a Claude thread has been idle for
    /// one prompt-cache window.
    fn render_return_chip(&self, cx: &Context<'_, Self>) -> AnyElement {
        let full = self.compact_full;
        let label = if full { "Full" } else { "Compact" };
        let title = if full {
            "Send the full draft"
        } else {
            "Summarize this thread, then send the draft"
        };
        div()
            .id("return-compact")
            .debug_selector(|| "desktop-return-compact".to_string())
            .h(px(24.0))
            .px(px(8.0))
            .rounded(px(4.0))
            .flex()
            .items_center()
            .text_size(px(12.0))
            .font_family(theme::UI_SMALL_FONT_FAMILY)
            .text_color(theme::rambutan_400())
            .hover(|el| {
                el.bg(theme::longan_800().opacity(0.60))
                    .text_color(theme::rambutan_200())
            })
            .cursor_pointer()
            .tab_index(0)
            .role(Role::Button)
            .aria_label(title)
            .tooltip(move |window, cx| Tooltip::new(title).build(window, cx))
            .focus_visible(|el| el.border_2().border_color(theme::persimmon_400()))
            .on_click(cx.listener(|this, _ev, _window, cx| {
                cx.stop_propagation();
                this.compact_full = !this.compact_full;
                cx.notify();
            }))
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                if is_web_button_activation_key(event.keystroke.key.as_str()) {
                    window.prevent_default();
                    cx.stop_propagation();
                    this.compact_full = !this.compact_full;
                    cx.notify();
                }
            }))
            .child(label)
            .into_any_element()
    }

    fn render_stop_button(&self, cx: &Context<'_, Self>) -> AnyElement {
        div()
            .id("stop-btn")
            .debug_selector(|| "desktop-stop-button".to_string())
            .size(px(SEND_BUTTON_SIZE_PX))
            .rounded_full()
            .bg(theme::lychee_500())
            .flex()
            .items_center()
            .justify_center()
            .hover(|el| el.bg(theme::lychee_400()))
            .tab_index(0)
            .role(Role::Button)
            .aria_label("Stop generation")
            .focus_visible(|el| el.border_2().border_color(theme::persimmon_400()))
            .cursor_pointer()
            .on_click(cx.listener(|this, _ev, _window, cx| {
                cx.stop_propagation();
                this.stop_streaming(cx);
            }))
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                if is_web_button_activation_key(event.keystroke.key.as_str()) {
                    window.prevent_default();
                    cx.stop_propagation();
                    this.stop_streaming(cx);
                }
            }))
            .child(
                div()
                    .size(px(STOP_BUTTON_SQUARE_PX))
                    .rounded(px(3.0))
                    .bg(theme::lychee_50()),
            )
            .into_any_element()
    }

    fn message_over_limit(&self, cx: &Context<'_, Self>) -> bool {
        exceeds_message_limit_with(
            &self.input_state.read(cx).value(),
            extra_limit_chars(&self.context_chips),
        )
    }

    fn composer_draft_empty(&self, cx: &Context<'_, Self>) -> bool {
        let typed_empty = self.input_state.read(cx).value().trim().is_empty();
        if self.is_streaming {
            typed_empty
        } else {
            typed_empty && self.text_attachments.is_empty() && !self.has_ready_upload()
        }
    }

    fn render_message_limit(&self) -> AnyElement {
        div()
            .id("message-limit")
            .px(px(16.0))
            .pb(px(4.0))
            .text_size(px(12.0))
            .text_color(theme::durian_300())
            .child(MESSAGE_LIMIT_HINT)
            .into_any_element()
    }

    fn render_text_attachments(&self, cx: &Context<'_, Self>) -> Option<AnyElement> {
        if self.text_attachments.is_empty() {
            return None;
        }
        let mut list = div()
            .id("text-attachments")
            .flex()
            .flex_col()
            .border_b_1()
            .border_color(theme::longan_800());
        for (index, item) in self.text_attachments.iter().enumerate() {
            let name = item.name.clone();
            list = list.child(
                div()
                    .id(SharedString::from(format!("paste-chip-{index}")))
                    .flex()
                    .h(px(24.0))
                    .items_center()
                    .gap_1()
                    .px(px(8.0))
                    .text_size(px(12.0))
                    .child(
                        div()
                            .min_w_0()
                            .flex_1()
                            .truncate()
                            .text_color(theme::rambutan_300())
                            .child(name),
                    )
                    .child(self.queue_icon_button(
                        SharedString::from(format!("paste-chip-remove-{index}")),
                        "Remove text attachment",
                        assets::CLOSE_ICON_PATH,
                        cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            if index < this.text_attachments.len() {
                                this.text_attachments.remove(index);
                                cx.notify();
                            }
                        }),
                    )),
            );
        }
        Some(list.into_any_element())
    }

    fn render_file_offers(&self, cx: &Context<'_, Self>) -> Option<AnyElement> {
        if self.file_offers.is_empty() {
            return None;
        }
        let mut list = div()
            .id("file-offers")
            .debug_selector(|| "desktop-file-offers".to_string())
            .flex()
            .flex_col()
            .border_b_1()
            .border_color(theme::longan_800());
        for offer in &self.file_offers {
            list = list.child(self.render_file_offer(offer, cx));
        }
        Some(list.into_any_element())
    }

    fn render_file_offer(
        &self,
        offer: &super::files::DesktopFileOffer,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        let id = offer.id.clone();
        let open_id = id.clone();
        let retry_id = id.clone();
        let remove_id = id.clone();
        let name = offer.name.clone();
        let status = offer.status_text().map(str::to_string);
        let status_error = offer.status_is_error();
        let can_open = offer.can_open();
        let can_retry = offer.can_retry();
        div()
            .id(SharedString::from(format!("file-offer-{id}")))
            .debug_selector(|| format!("desktop-file-offer-{id}"))
            .flex()
            .h(px(32.0))
            .items_center()
            .gap_1()
            .px(px(8.0))
            .text_size(px(12.0))
            .font_family(theme::UI_SMALL_FONT_FAMILY)
            .child({
                let mut name_button = div()
                    .id(SharedString::from(format!("file-offer-name-{open_id}")))
                    .min_w_0()
                    .flex_1()
                    .h(px(24.0))
                    .flex()
                    .items_center()
                    .truncate()
                    .text_color(theme::rambutan_300())
                    .child(name);
                if can_open {
                    name_button = name_button
                        .cursor_pointer()
                        .tab_index(0)
                        .role(Role::Button)
                        .aria_label("Open file")
                        .hover(|el| el.text_color(theme::rambutan_100()))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            this.open_ready_file(&open_id, cx);
                        }));
                }
                name_button
            })
            .children(status.map(|text| {
                div()
                    .flex_shrink_0()
                    .truncate()
                    .text_color(if status_error {
                        theme::rambutan_400()
                    } else {
                        theme::rambutan_500()
                    })
                    .child(text)
            }))
            .when(can_retry, |el| {
                el.child(
                    div()
                        .id(SharedString::from(format!("file-offer-retry-{retry_id}")))
                        .h(px(24.0))
                        .px(px(6.0))
                        .flex()
                        .items_center()
                        .rounded(px(4.0))
                        .cursor_pointer()
                        .tab_index(0)
                        .role(Role::Button)
                        .aria_label("Retry upload")
                        .text_color(theme::rambutan_400())
                        .hover(|el| el.text_color(theme::rambutan_100()))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            this.retry_file_offer(&retry_id, cx);
                        }))
                        .child("Retry"),
                )
            })
            .child(self.queue_icon_button(
                SharedString::from(format!("file-offer-remove-{remove_id}")),
                "Remove file",
                assets::CLOSE_ICON_PATH,
                cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    this.remove_file_offer(&remove_id, cx);
                }),
            ))
            .into_any_element()
    }

    fn composer_action_now(&self, cx: &Context<'_, Self>) -> ComposerAction {
        let draft_empty = self.composer_draft_empty(cx);
        let has_queued = self
            .thread_id
            .as_ref()
            .is_some_and(|id| self.prompt_queue.has_for(id));
        // The label stays on the stored action while Cmd is held.
        // Cmd/Ctrl+Enter still asks for the other one.
        composer_action(
            self.is_streaming,
            draft_empty,
            false,
            self.queue_held,
            has_queued,
            self.follow_up_now(),
        )
    }

    fn render_queued_prompts(&self, cx: &Context<'_, Self>) -> Option<AnyElement> {
        let thread_id = self.thread_id.clone()?;
        let rows = self.prompt_queue.for_thread(&thread_id);
        if rows.is_empty() {
            return None;
        }
        let mut list = div()
            .id("queued-prompts")
            .debug_selector(|| "desktop-queued-prompts".to_string())
            .flex()
            .flex_col()
            .border_b_1()
            .border_color(theme::longan_800());
        for prompt in rows {
            list = list.child(self.render_queued_row(prompt, cx));
        }
        Some(list.into_any_element())
    }

    fn render_queued_row(&self, prompt: QueuedPrompt, cx: &Context<'_, Self>) -> AnyElement {
        let editing = self
            .queue_edit
            .as_ref()
            .is_some_and(|edit| edit.id == prompt.id);
        let can_earlier = self.prompt_queue.can_nudge(&prompt.id, true);
        let can_later = self.prompt_queue.can_nudge(&prompt.id, false);
        let earlier_id = prompt.id.clone();
        let later_id = prompt.id.clone();
        let edit_item = prompt.clone();
        let remove_id = prompt.id.clone();
        div()
            .id(SharedString::from(format!("queued-{}", prompt.id)))
            .when(editing, |el| el.bg(theme::longan_800()))
            .flex()
            .items_center()
            .gap_1()
            .px(px(8.0))
            .py(px(4.0))
            .text_size(px(12.0))
            .child(self.queue_nudge_button(
                "Move queued message earlier",
                "↑",
                can_earlier,
                earlier_id,
                true,
                cx,
            ))
            .child(self.queue_nudge_button(
                "Move queued message later",
                "↓",
                can_later,
                later_id,
                false,
                cx,
            ))
            .child(
                div()
                    .flex_shrink_0()
                    .text_color(theme::blueberry_400())
                    .child("Queued"),
            )
            .child(
                div()
                    .min_w_0()
                    .flex_1()
                    .truncate()
                    .text_color(theme::rambutan_300())
                    .child(prompt.text.clone()),
            )
            .when(editing, |el| {
                el.child(self.queue_icon_button(
                    SharedString::from(format!("queued-save-{}", prompt.id)),
                    "Save queued message",
                    assets::CHECK_ICON_PATH,
                    cx.listener(|this, _, window, cx| {
                        cx.stop_propagation();
                        this.commit_queue_edit(window, cx);
                    }),
                ))
                .child(self.queue_icon_button(
                    SharedString::from(format!("queued-cancel-{}", prompt.id)),
                    "Cancel editing queued message",
                    assets::CLOSE_ICON_PATH,
                    cx.listener(|this, _, window, cx| {
                        cx.stop_propagation();
                        this.cancel_queue_edit(window, cx);
                    }),
                ))
            })
            .when(!editing, |el| {
                el.child(self.queue_icon_button(
                    SharedString::from(format!("queued-edit-{}", prompt.id)),
                    "Edit queued message",
                    assets::TOOL_PENCIL_ICON_PATH,
                    cx.listener(move |this, _, window, cx| {
                        cx.stop_propagation();
                        this.begin_queue_edit(edit_item.clone(), window, cx);
                    }),
                ))
                .child(self.queue_icon_button(
                    SharedString::from(format!("queued-remove-{}", prompt.id)),
                    "Remove queued message",
                    assets::CLOSE_ICON_PATH,
                    cx.listener(move |this, _, _window, cx| {
                        cx.stop_propagation();
                        if this.prompt_queue.remove(&remove_id) {
                            this.drop_queue_edit_if_removed(Some(&remove_id));
                            cx.emit(ChatViewEvent::MessagesChanged);
                            cx.notify();
                        }
                    }),
                ))
            })
            .into_any_element()
    }

    fn queue_nudge_button(
        &self,
        label: &'static str,
        glyph: &'static str,
        enabled: bool,
        id: String,
        earlier: bool,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        let direction = if earlier { "earlier" } else { "later" };
        div()
            .id(SharedString::from(format!("queued-{direction}-{id}")))
            .size(px(24.0))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(4.0))
            .text_size(px(12.0))
            .text_color(theme::rambutan_400())
            .role(Role::Button)
            .aria_label(label)
            .when(!enabled, |el| el.opacity(0.3))
            .when(enabled, |el| {
                el.cursor_pointer()
                    .tab_index(0)
                    .focus_visible(|el| el.border_2().border_color(theme::persimmon_400()))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        if this.prompt_queue.nudge(&id, earlier) {
                            cx.emit(ChatViewEvent::MessagesChanged);
                            cx.notify();
                        }
                    }))
            })
            .child(glyph)
            .into_any_element()
    }

    fn queue_icon_button(
        &self,
        id: impl Into<ElementId>,
        label: &'static str,
        path: &'static str,
        on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> AnyElement {
        div()
            .id(id)
            .size(px(24.0))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(4.0))
            .cursor_pointer()
            .tab_index(0)
            .role(Role::Button)
            .aria_label(label)
            .focus_visible(|el| el.border_2().border_color(theme::persimmon_400()))
            .on_click(on_click)
            .child(icon(path, 14.0, theme::rambutan_400()))
            .into_any_element()
    }

    /// Icon circle while idle. Queue, Resume, Interrupt, and Steer name themselves.
    fn render_send_button(&self, cx: &Context<'_, Self>) -> AnyElement {
        let action = self.composer_action_now(cx);
        let draft_empty = self.composer_draft_empty(cx);
        let enabled = action_enabled_for(
            action,
            draft_empty || self.message_over_limit(cx) || self.uploads_block_send(),
        );
        let label = action_label(action);
        let title = action_title(action).to_string();
        let interrupt = matches!(action, ComposerAction::Interrupt);
        let word = !matches!(action, ComposerAction::Send);
        let aria = if word {
            title
        } else {
            format!("{label} message")
        };
        div()
            .id("send-btn")
            .debug_selector(|| "desktop-send-button".to_string())
            .when(word, |el| el.h(px(SEND_BUTTON_SIZE_PX)).px(px(12.0)))
            .when(!word, |el| el.size(px(SEND_BUTTON_SIZE_PX)))
            .rounded_full()
            .flex()
            .items_center()
            .justify_center()
            .gap(px(6.0))
            .when(interrupt, |el| {
                el.bg(theme::lychee_500()).text_color(theme::lychee_50())
            })
            .when(!interrupt, |el| {
                el.bg(theme::persimmon_500())
                    .text_color(theme::longan_950())
            })
            .when(!enabled, |el| {
                el.opacity(SEND_BUTTON_DISABLED_OPACITY)
                    .cursor_not_allowed()
            })
            .when(enabled, |el| {
                el.hover(|el| {
                    if interrupt {
                        el.bg(theme::lychee_400())
                    } else {
                        el.bg(theme::persimmon_400())
                    }
                })
                .tab_index(0)
                .role(Role::Button)
                .aria_label(aria)
                .focus_visible(|el| el.border_2().border_color(theme::persimmon_400()))
                .cursor_pointer()
                .on_click(cx.listener(|this, _ev, window, cx| {
                    cx.stop_propagation();
                    this.send_follow_up(false, window, cx);
                }))
                .on_key_down(cx.listener(
                    |this, event: &KeyDownEvent, window, cx| {
                        if is_web_button_activation_key(event.keystroke.key.as_str()) {
                            window.prevent_default();
                            cx.stop_propagation();
                            this.send_follow_up(false, window, cx);
                        }
                    },
                ))
            })
            .when(interrupt, |el| {
                el.child(icon(assets::SQUARE_ICON_PATH, 12.0, theme::lychee_50()))
                    .child(label)
            })
            .when(word && !interrupt, |el| el.child(label))
            .when(!word, |el| {
                el.child(icon(
                    assets::ARROW_UP_ICON_PATH,
                    SEND_BUTTON_ICON_SIZE_PX,
                    theme::longan_950(),
                ))
            })
            .into_any_element()
    }

    fn branch_chip_label(&self) -> Option<String> {
        let started = self.thread_has_place();
        if !started
            && self.run_choice == WorkspaceChoice::NewWorktree
            && !self.base_branch.is_empty()
        {
            return Some(self.base_branch.clone());
        }
        if !started && self.run_choice == WorkspaceChoice::PreviousWorktree {
            if let Some(item) = &self.previous_worktree {
                return Some(item.branch.clone());
            }
        }
        self.git_current_branch.clone()
    }

    fn branch_chip_title(&self) -> &'static str {
        let started = self.thread_has_place();
        if !started && self.run_choice == WorkspaceChoice::NewWorktree {
            "Base branch (⌘⇧G)"
        } else if !started && self.run_choice == WorkspaceChoice::PreviousWorktree {
            "Previous worktree (⌘⇧G)"
        } else {
            "Switch branch (⌘⇧G)"
        }
    }

    fn workspace_chip_title(&self) -> &'static str {
        if self.thread_has_place() {
            "Workspace for the next thread (⌘⇧X)"
        } else {
            "Where this thread runs (⌘⇧X)"
        }
    }

    fn effort_chip_label(&self) -> &'static str {
        self.claude_effort
            .as_deref()
            .and_then(bruhs_models::logic::run_place::claude_effort)
            .unwrap_or("Effort")
    }

    pub fn toggle_workspace_menu(&mut self, _window: &mut Window, cx: &mut Context<'_, Self>) {
        let open = !self.workspace_menu_open;
        self.branch_menu_open = false;
        self.effort_menu_open = false;
        self.model_picker_open = false;
        self.permission_menu_open = false;
        self.stash_open = false;
        self.workspace_menu_open = open;
        cx.notify();
    }

    pub fn toggle_branch_menu(&mut self, _window: &mut Window, cx: &mut Context<'_, Self>) {
        let open = !self.branch_menu_open;
        self.workspace_menu_open = false;
        self.effort_menu_open = false;
        self.model_picker_open = false;
        self.permission_menu_open = false;
        self.stash_open = false;
        self.branch_menu_open = open;
        self.refresh_git_branches(cx);
        cx.notify();
    }

    pub fn toggle_effort_menu(&mut self, _window: &mut Window, cx: &mut Context<'_, Self>) {
        let open = !self.effort_menu_open;
        self.branch_menu_open = false;
        self.workspace_menu_open = false;
        self.model_picker_open = false;
        self.permission_menu_open = false;
        self.stash_open = false;
        self.effort_menu_open = open;
        cx.notify();
    }

    pub fn use_previous_worktree(&mut self, cx: &mut Context<'_, Self>) {
        if self.previous_worktree.is_some() {
            self.run_choice = WorkspaceChoice::PreviousWorktree;
            self.emit_run_prefs(cx);
            cx.emit(ChatViewEvent::Notice("Previous worktree".to_string()));
        } else {
            cx.emit(ChatViewEvent::Notice("No previous worktree".to_string()));
        }
        cx.notify();
    }

    fn choose_run_choice(&mut self, choice: WorkspaceChoice, cx: &mut Context<'_, Self>) {
        self.run_choice = choice;
        self.workspace_menu_open = false;
        self.emit_run_prefs(cx);
        cx.notify();
    }

    fn choose_effort(&mut self, effort: Option<String>, cx: &mut Context<'_, Self>) {
        self.claude_effort = effort;
        self.effort_menu_open = false;
        self.emit_run_prefs(cx);
        cx.notify();
    }

    fn render_effort_chip(&self, cx: &Context<'_, Self>) -> AnyElement {
        let label = self.effort_chip_label();
        self.render_chip(
            "desktop-effort-chip",
            "desktop-effort-chip",
            "Effort (⌘⇧E)".to_string(),
            false,
            Some(self.effort_menu_open),
            div()
                .text_size(px(CHIP_ICON_SIZE_PX))
                .text_color(theme::rambutan_400())
                .child("⚡")
                .into_any_element(),
            div().child(label).into_any_element(),
            Some(self.chevron()),
            |this, window, cx| this.toggle_effort_menu(window, cx),
            cx,
        )
    }

    fn render_effort_menu(&self, cx: &Context<'_, Self>) -> AnyElement {
        let mut rows = Vec::new();
        let default_selected = self.claude_effort.is_none();
        rows.push(self.effort_row("default", "Default", default_selected, None, cx));
        for level in CLAUDE_EFFORTS {
            let selected = self.claude_effort.as_deref() == Some(level);
            rows.push(self.effort_row(level, level, selected, Some(level.to_string()), cx));
        }
        self.menu_frame("desktop-effort-menu", 192.0, "Effort")
            .children(rows)
            .into_any_element()
    }

    fn effort_row(
        &self,
        key: &'static str,
        label: &'static str,
        selected: bool,
        pick: Option<String>,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        div()
            .id(ElementId::Name(format!("effort-{key}").into()))
            .debug_selector(move || format!("desktop-effort-option-{key}"))
            .w_full()
            .px_2()
            .py(px(6.0))
            .rounded(px(4.0))
            .text_size(px(MENU_ROW_TEXT_SIZE_PX))
            .font_family(theme::UI_SMALL_FONT_FAMILY)
            .text_color(theme::rambutan_200())
            .when(selected, |el| el.bg(theme::longan_800().opacity(0.5)))
            .hover(|el| el.bg(theme::longan_800().opacity(0.6)))
            .tab_index(0)
            .role(Role::MenuItem)
            .aria_label(label)
            .aria_selected(selected)
            .focus_visible(|el| el.border_1().border_color(theme::persimmon_400()))
            .cursor_pointer()
            .on_click({
                let pick = pick.clone();
                cx.listener(move |this, _ev, _window, cx| {
                    cx.stop_propagation();
                    this.choose_effort(pick.clone(), cx);
                })
            })
            .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
                if is_web_button_activation_key(event.keystroke.key.as_str()) {
                    window.prevent_default();
                    cx.stop_propagation();
                    this.choose_effort(pick.clone(), cx);
                }
            }))
            .child(label)
            .into_any_element()
    }

    fn render_workspace_chip(&self, cx: &Context<'_, Self>) -> AnyElement {
        let title = self.workspace_chip_title();
        div()
            .id("desktop-run-choice")
            .debug_selector(|| "desktop-run-choice".to_string())
            .h(px(STRIP_CHIP_HEIGHT_PX))
            .max_w(px(224.0))
            .px(px(6.0))
            .rounded(px(4.0))
            .flex()
            .items_center()
            .gap(px(6.0))
            .hover(|el| {
                el.bg(theme::longan_800().opacity(0.5))
                    .text_color(theme::rambutan_200())
            })
            .tab_index(0)
            .role(Role::Button)
            .aria_label(title)
            .aria_expanded(self.workspace_menu_open)
            .focus_visible(|el| el.border_1().border_color(theme::persimmon_400()))
            .cursor_pointer()
            .tooltip(move |window, cx| Tooltip::new(title).build(window, cx))
            .on_click(cx.listener(|this, _ev, window, cx| {
                cx.stop_propagation();
                this.toggle_workspace_menu(window, cx);
            }))
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                if is_web_button_activation_key(event.keystroke.key.as_str()) {
                    window.prevent_default();
                    cx.stop_propagation();
                    this.toggle_workspace_menu(window, cx);
                }
            }))
            .child(icon(
                assets::FOLDER_ICON_PATH,
                STRIP_ICON_SIZE_PX,
                theme::rambutan_500(),
            ))
            .child(div().min_w_0().truncate().child(self.run_choice.label()))
            .child(icon(
                assets::CHEVRON_DOWN_ICON_PATH,
                STRIP_ICON_SIZE_PX,
                theme::rambutan_500(),
            ))
            .into_any_element()
    }

    fn render_workspace_menu(&self, cx: &Context<'_, Self>) -> AnyElement {
        let mut choices = vec![WorkspaceChoice::Checkout, WorkspaceChoice::NewWorktree];
        if self.previous_worktree.is_some() {
            choices.push(WorkspaceChoice::PreviousWorktree);
        }
        let rows = choices.into_iter().map(|choice| {
            let selected = self.run_choice == choice;
            let key = choice.as_str();
            let detail = if choice == WorkspaceChoice::PreviousWorktree {
                self.previous_worktree
                    .as_ref()
                    .map(|item| item.label.clone())
                    .unwrap_or_default()
            } else {
                String::new()
            };
            div()
                .id(ElementId::Name(format!("workspace-{key}").into()))
                .debug_selector(move || format!("desktop-workspace-option-{key}"))
                .w_full()
                .px_2()
                .py(px(6.0))
                .rounded(px(4.0))
                .flex()
                .flex_col()
                .text_size(px(MENU_ROW_TEXT_SIZE_PX))
                .font_family(theme::UI_SMALL_FONT_FAMILY)
                .text_color(theme::rambutan_200())
                .when(selected, |el| el.bg(theme::longan_800().opacity(0.5)))
                .hover(|el| el.bg(theme::longan_800().opacity(0.6)))
                .tab_index(0)
                .role(Role::MenuItem)
                .aria_label(choice.label())
                .aria_selected(selected)
                .focus_visible(|el| el.border_1().border_color(theme::persimmon_400()))
                .cursor_pointer()
                .on_click(cx.listener(move |this, _ev, _window, cx| {
                    cx.stop_propagation();
                    this.choose_run_choice(choice, cx);
                }))
                .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
                    if is_web_button_activation_key(event.keystroke.key.as_str()) {
                        window.prevent_default();
                        cx.stop_propagation();
                        this.choose_run_choice(choice, cx);
                    }
                }))
                .child(choice.label())
                .when(!detail.is_empty(), |el| {
                    el.child(
                        div()
                            .text_size(px(12.0))
                            .text_color(theme::rambutan_500())
                            .child(detail),
                    )
                })
                .into_any_element()
        });
        self.menu_frame("desktop-workspace-menu", 256.0, "Workspace")
            .children(rows)
            .into_any_element()
    }

    /// Project + branch shoulder under the composer (project threads only).
    fn render_context_strip(&self, cx: &Context<'_, Self>) -> Option<AnyElement> {
        let project = self.breadcrumb_project()?;
        let branch = self.branch_chip_label();
        let show_dirty = (self.thread_has_place() || self.run_choice == WorkspaceChoice::Checkout)
            && self.git_dirty_count > 0;
        let dirty = self.git_dirty_count;
        let branch_title = self.branch_chip_title();
        Some(
            div()
                .id("composer-context-strip")
                .debug_selector(|| "desktop-composer-context-strip".to_string())
                .relative()
                .mx(px(STRIP_X_MARGIN_PX))
                .px(px(STRIP_X_PADDING_PX))
                .pt(px(STRIP_TOP_PADDING_PX))
                .pb(px(STRIP_BOTTOM_PADDING_PX))
                .rounded_b(px(STRIP_RADIUS_PX))
                .bg(theme::longan_900().opacity(STRIP_BG_OPACITY))
                .border_b_1()
                .border_l_1()
                .border_r_1()
                .border_color(gpui::white().opacity(STRIP_RING_OPACITY))
                .flex()
                .items_center()
                .gap_1()
                .text_size(px(STRIP_TEXT_SIZE_PX))
                .font_family(theme::UI_SMALL_FONT_FAMILY)
                .text_color(theme::rambutan_500())
                .child(
                    div()
                        .id("strip-project")
                        .debug_selector(|| "desktop-composer-project".to_string())
                        .h(px(STRIP_CHIP_HEIGHT_PX))
                        .min_w_0()
                        .px(px(6.0))
                        .rounded(px(4.0))
                        .flex()
                        .items_center()
                        .gap(px(6.0))
                        .hover(|el| {
                            el.bg(theme::longan_800().opacity(0.5))
                                .text_color(theme::rambutan_200())
                        })
                        .tab_index(0)
                        .role(Role::Button)
                        .aria_label("Change project")
                        .focus_visible(|el| el.border_1().border_color(theme::persimmon_400()))
                        .cursor_pointer()
                        .on_click(cx.listener(|_this, _ev, _window, cx| {
                            cx.stop_propagation();
                            cx.emit(ChatViewEvent::OpenProjectPicker);
                        }))
                        .child(icon(
                            assets::FOLDER_ICON_PATH,
                            STRIP_ICON_SIZE_PX,
                            theme::rambutan_500(),
                        ))
                        .child(div().min_w_0().truncate().child(project)),
                )
                .child(div().flex_1())
                .child(self.render_workspace_chip(cx))
                .when_some(branch, |el, branch| {
                    el.child(
                        div()
                            .id("desktop-git-branch")
                            .debug_selector(|| "desktop-git-branch".to_string())
                            .h(px(STRIP_CHIP_HEIGHT_PX))
                            .max_w(px(256.0))
                            .px(px(6.0))
                            .rounded(px(4.0))
                            .flex()
                            .items_center()
                            .gap(px(6.0))
                            .hover(|el| {
                                el.bg(theme::longan_800().opacity(0.5))
                                    .text_color(theme::rambutan_200())
                            })
                            .tab_index(0)
                            .role(Role::Button)
                            .aria_label(branch_title)
                            .aria_expanded(self.branch_menu_open)
                            .focus_visible(|el| el.border_1().border_color(theme::persimmon_400()))
                            .cursor_pointer()
                            .tooltip(move |window, cx| Tooltip::new(branch_title).build(window, cx))
                            .on_click(cx.listener(|this, _ev, window, cx| {
                                cx.stop_propagation();
                                this.toggle_branch_menu(window, cx);
                            }))
                            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                                if is_web_button_activation_key(event.keystroke.key.as_str()) {
                                    window.prevent_default();
                                    cx.stop_propagation();
                                    this.toggle_branch_menu(window, cx);
                                }
                            }))
                            .child(icon(
                                assets::GIT_BRANCH_ICON_PATH,
                                STRIP_ICON_SIZE_PX,
                                theme::rambutan_500(),
                            ))
                            .child(div().min_w_0().truncate().child(branch))
                            .when(show_dirty && dirty > 0, |el| {
                                el.child(
                                    div()
                                        .debug_selector(|| "desktop-git-dirty-count".to_string())
                                        .px_1()
                                        .rounded(px(4.0))
                                        .bg(theme::durian_400().opacity(STRIP_DIRTY_BG_OPACITY))
                                        .text_color(theme::durian_300())
                                        .child(format!("•{dirty}")),
                                )
                            })
                            .child(icon(
                                assets::CHEVRON_DOWN_ICON_PATH,
                                STRIP_ICON_SIZE_PX,
                                theme::rambutan_500(),
                            )),
                    )
                })
                .when(self.workspace_menu_open, |el| {
                    el.child(
                        div()
                            .absolute()
                            .right_0()
                            .bottom(relative(1.0))
                            .mb_1()
                            .occlude()
                            .child(self.render_workspace_menu(cx)),
                    )
                })
                .when(self.branch_menu_open, |el| {
                    el.child(
                        div()
                            .absolute()
                            .right_0()
                            .bottom(relative(1.0))
                            .mb_1()
                            .occlude()
                            .child(self.render_branch_menu(cx)),
                    )
                })
                .into_any_element(),
        )
    }

    fn kbd(&self, keys: &'static str) -> AnyElement {
        div()
            .h(px(CHAT_KBD_HEIGHT_PX))
            .min_w(px(CHAT_KBD_MIN_WIDTH_PX))
            .px(px(6.0))
            .rounded(px(4.0))
            .bg(theme::longan_800())
            .border_1()
            .border_color(gpui::white().opacity(0.10))
            .flex()
            .items_center()
            .justify_center()
            .font_family(theme::CODE_FONT_FAMILY)
            .text_size(px(CHAT_HINTS_TEXT_SIZE_PX))
            .text_color(theme::rambutan_300())
            .child(keys)
            .into_any_element()
    }

    /// `/ commands · @ files · ⌘K palette` under an empty thread's composer.
    fn render_hints(&self) -> AnyElement {
        let dot = || {
            div()
                .text_color(theme::rambutan_500())
                .child("·")
                .into_any_element()
        };
        div()
            .debug_selector(|| "desktop-chat-hints".to_string())
            .mt(px(CHAT_HINTS_TOP_MARGIN_PX))
            .w_full()
            .flex()
            .items_center()
            .justify_center()
            .gap(px(6.0))
            .text_size(px(CHAT_HINTS_TEXT_SIZE_PX))
            .font_family(theme::UI_SMALL_FONT_FAMILY)
            .text_color(theme::rambutan_500())
            .child(self.kbd("/"))
            .child("commands")
            .child(dot())
            .child(self.kbd("@"))
            .child("files")
            .child(dot())
            .child(self.kbd("⌘K"))
            .child("palette")
            .into_any_element()
    }

    fn menu_frame(&self, selector: &'static str, width: f32, label: &'static str) -> Stateful<Div> {
        div()
            .id(selector)
            .debug_selector(move || selector.to_string())
            .role(Role::Menu)
            .aria_label(label)
            .w(px(width))
            .p_1()
            .rounded(px(CHIP_RADIUS_PX))
            .bg(theme::longan_900())
            .border_1()
            .border_color(theme::ring_floating())
            .flex()
            .flex_col()
            .gap_px()
    }

    /// Default · Auto-accept · YOLO (web access-mode menu).
    fn render_permission_menu(&self, cx: &Context<'_, Self>) -> AnyElement {
        let provider = provider_for_model_id(&self.model_id);
        let rows: Vec<AnyElement> = [
            PermissionMode::Default,
            PermissionMode::AcceptEdits,
            PermissionMode::BypassPermissions,
        ]
        .into_iter()
        .map(|mode| {
            let selected = self.permission_mode == mode;
            let key = mode.as_str();
            div()
                .id(ElementId::Name(format!("permission-mode-{key}").into()))
                .debug_selector(move || format!("desktop-permission-mode-{key}"))
                .w_full()
                .px_2()
                .py(px(6.0))
                .rounded(px(4.0))
                .flex()
                .items_start()
                .gap_2()
                .text_size(px(MENU_ROW_TEXT_SIZE_PX))
                .font_family(theme::UI_SMALL_FONT_FAMILY)
                .text_color(theme::rambutan_200())
                .when(selected, |el| el.bg(theme::longan_800().opacity(0.5)))
                .hover(|el| el.bg(theme::longan_800().opacity(0.6)))
                .tab_index(0)
                .role(Role::MenuItem)
                .aria_label(mode.label())
                .aria_selected(selected)
                .focus_visible(|el| el.border_1().border_color(theme::persimmon_400()))
                .cursor_pointer()
                .on_click(cx.listener(move |this, _ev, _window, cx| {
                    cx.stop_propagation();
                    this.choose_permission_mode(mode, cx);
                }))
                .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
                    if is_web_button_activation_key(event.keystroke.key.as_str()) {
                        window.prevent_default();
                        cx.stop_propagation();
                        this.choose_permission_mode(mode, cx);
                    }
                }))
                .child(
                    icon(
                        assets::permission_mode_icon_path(mode),
                        CHIP_ICON_SIZE_PX,
                        theme::rambutan_400(),
                    )
                    .mt(px(1.0)),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .child(div().text_color(theme::rambutan_100()).child(mode.label()))
                        .child(
                            div()
                                .text_color(theme::rambutan_500())
                                .child(mode.description(Some(provider))),
                        ),
                )
                .into_any_element()
        })
        .collect();
        self.menu_frame("desktop-permission-menu", MENU_WIDTH_PX, "Access mode")
            .children(rows)
            .into_any_element()
    }

    /// Parked drafts: click restores, × deletes (web prompt stash).
    fn render_stash_menu(&self, cx: &Context<'_, Self>) -> AnyElement {
        let frame = self.menu_frame("desktop-stash-menu", STASH_MENU_WIDTH_PX, "Prompt stash");
        if self.stash.is_empty() {
            return frame
                .child(
                    div()
                        .px_3()
                        .py_2()
                        .text_size(px(MENU_ROW_TEXT_SIZE_PX))
                        .font_family(theme::UI_SMALL_FONT_FAMILY)
                        .text_color(theme::rambutan_500())
                        .child(CHAT_STASH_EMPTY),
                )
                .into_any_element();
        }
        let now = EpochMs::now();
        let rows: Vec<AnyElement> = self
            .stash
            .iter()
            .map(|entry| {
                let key = element_key(&entry.id);
                let restore_id = entry.id.clone();
                let delete_id = entry.id.clone();
                let preview = stash_preview(&entry.text, STASH_PREVIEW_CHARS);
                let when = relative_age(entry.at_ms, now);
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .child(
                        div()
                            .id(ElementId::Name(format!("stash-entry-{key}").into()))
                            .debug_selector(move || format!("desktop-stash-entry-{key}"))
                            .min_w_0()
                            .flex_1()
                            .px_2()
                            .py(px(6.0))
                            .rounded(px(CHIP_RADIUS_PX))
                            .flex()
                            .flex_col()
                            .text_size(px(MENU_ROW_TEXT_SIZE_PX))
                            .font_family(theme::UI_SMALL_FONT_FAMILY)
                            .hover(|el| el.bg(theme::longan_800().opacity(0.6)))
                            .tab_index(0)
                            .role(Role::MenuItem)
                            .aria_label(format!("Restore stashed prompt: {preview}"))
                            .focus_visible(|el| el.border_1().border_color(theme::persimmon_400()))
                            .cursor_pointer()
                            .on_click(cx.listener(move |this, _ev, window, cx| {
                                cx.stop_propagation();
                                this.restore_stash(&restore_id, window, cx);
                            }))
                            .child(
                                div()
                                    .truncate()
                                    .text_color(theme::rambutan_200())
                                    .child(preview),
                            )
                            .child(div().text_color(theme::rambutan_500()).child(when)),
                    )
                    .child(
                        div()
                            .id(ElementId::Name(format!("stash-delete-{key}").into()))
                            .debug_selector(move || format!("desktop-stash-delete-{key}"))
                            .size(px(24.0))
                            .rounded(px(4.0))
                            .flex()
                            .items_center()
                            .justify_center()
                            .text_color(theme::rambutan_500())
                            .hover(|el| el.text_color(theme::lychee_300()))
                            .tab_index(0)
                            .role(Role::Button)
                            .aria_label("Delete stashed prompt")
                            .focus_visible(|el| el.border_1().border_color(theme::persimmon_400()))
                            .cursor_pointer()
                            .on_click(cx.listener(move |this, _ev, _window, cx| {
                                cx.stop_propagation();
                                this.remove_stash(&delete_id, cx);
                            }))
                            .child(icon(
                                assets::CLOSE_ICON_PATH,
                                CHIP_CHEVRON_SIZE_PX,
                                theme::rambutan_500(),
                            )),
                    )
                    .into_any_element()
            })
            .collect();
        frame.children(rows).into_any_element()
    }

    fn choose_permission_mode(&mut self, mode: PermissionMode, cx: &mut Context<'_, Self>) {
        self.permission_menu_open = false;
        self.plan_return_mode = mode;
        cx.emit(ChatViewEvent::SetPermissionMode(mode));
        cx.notify();
    }

    /// Build ↔ Plan; leaving Plan restores the mode that was active before.
    pub(super) fn toggle_plan(&mut self, cx: &mut Context<'_, Self>) {
        let next = if self.permission_mode == PermissionMode::Plan {
            self.plan_return_mode
        } else {
            self.plan_return_mode = self.permission_mode;
            PermissionMode::Plan
        };
        cx.emit(ChatViewEvent::SetPermissionMode(next));
        cx.notify();
    }

    /// ⌘S: park the draft and clear the input.
    pub(super) fn stash_draft(
        &mut self,
        _: &StashDraft,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let text = self.input_state.read(cx).value().to_string();
        if text.trim().is_empty() {
            return;
        }
        let chips = self.context_chips.clone();
        self.stash.insert(
            0,
            StashEntry {
                id: uuid::Uuid::new_v4().to_string(),
                text,
                at_ms: EpochMs::now(),
                chips,
            },
        );
        self.context_chips.clear();
        self.chip_guard = true;
        self.chip_draft.clear();
        self.input_state.update(cx, |input, cx| {
            input.set_value("", window, cx);
        });
        cx.notify();
    }

    fn restore_stash(&mut self, id: &str, window: &mut Window, cx: &mut Context<'_, Self>) {
        let Some(pos) = self.stash.iter().position(|e| e.id == id) else {
            return;
        };
        let entry = self.stash.remove(pos);
        let draft = self.input_state.read(cx).value().to_string();
        let (next, chips) = join_drafts(
            &draft,
            &self.context_chips,
            &entry.text,
            &entry.chips,
            false,
        );
        self.install_chips(&next, chips, next.len(), window, cx);
        self.stash_open = false;
        self.focus_composer(window, cx);
        cx.notify();
    }

    fn remove_stash(&mut self, id: &str, cx: &mut Context<'_, Self>) {
        self.stash.retain(|e| e.id != id);
        cx.notify();
    }

    /// Close whichever composer menu is open (outside click, Escape).
    pub fn close_composer_menus(&mut self, cx: &mut Context<'_, Self>) {
        if self.permission_menu_open
            || self.stash_open
            || self.branch_menu_open
            || self.workspace_menu_open
            || self.effort_menu_open
        {
            self.permission_menu_open = false;
            self.stash_open = false;
            self.branch_menu_open = false;
            self.workspace_menu_open = false;
            self.effort_menu_open = false;
            cx.notify();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{model_label, restore_into_draft, stash_preview};

    #[test]
    fn model_label_falls_back_to_id() {
        assert_eq!(model_label("claude-sonnet-4-6"), "Claude Sonnet 4.6");
        assert_eq!(model_label("mystery-model"), "mystery-model");
    }

    #[test]
    fn stash_preview_and_restore_like_web() {
        assert_eq!(stash_preview("  first line\nsecond", 80), "first line");
        assert_eq!(stash_preview("abcdefghij", 5), "abcd…");
        assert_eq!(restore_into_draft("", "parked"), "parked");
        assert_eq!(restore_into_draft("draft  ", "parked"), "draft\nparked");
    }
}

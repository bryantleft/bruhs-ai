//! The transcript, 1:1 with `bruhs-web/src/components/timeline.rs` and
//! `timeline_view.rs`: user bubbles, the "Worked for …" fold, grouped tool
//! activity (`Ran 1 command and edited 1 file`), thinking, the working row,
//! changed-files cards, error rows with Retry, and the hover-only turn meta.
//! The pure helpers (tool labels, summaries, changed files) are ports of the
//! web module so both surfaces say the same thing about the same turn.

use crate::views::theme_regions::ThemeRegion as _;
use crate::views::copy_feedback::CopyFeedback;
use bruhs_models::logic::context_meter::format_tokens;
use bruhs_models::num::{ToFloatLossy, ToIntSaturating};
use bruhs_ui::press::on_press_view;
use bruhs_ui::zoom::px;
use chrono::{Local, TimeZone};
use std::time::Duration;

use bruhs_ui::tooltip::Tooltip;
use gpui::prelude::*;
use bruhs_ui::ActiveTheme;
use gpui::*;

#[cfg(any(test, debug_assertions))]
use super::TurnState;
use super::{
    ChatView, ChatViewEvent, DesktopToolCall, Message, MessageRole, element_key,
    fold_duration_label, turn_duration_ms, turn_fold_label,
};
use crate::views::diff_panel::{changed_files_from_tool_calls, split_path};
use crate::{assets, theme};
use bruhs_models::EpochMs;
pub use bruhs_models::logic::tools::{
    ToolKind, ToolState, tool_kind, tool_label, tool_state, work_summary,
};

// Web `Timeline`: `mx-auto max-w-3xl px-4 pt-4 pb-6`.
pub const TRANSCRIPT_MAX_WIDTH_PX: f32 = 736.0;
pub const TRANSCRIPT_X_PADDING_PX: f32 = 16.0;
pub const TRANSCRIPT_TOP_PADDING_PX: f32 = 16.0;
pub const TRANSCRIPT_BOTTOM_PADDING_PX: f32 = 24.0;
pub const TRANSCRIPT_BODY_TEXT_SIZE_PX: f32 = theme::APP_BODY_TEXT_SIZE_PX;
pub const TRANSCRIPT_LABEL_TEXT_SIZE_PX: f32 = 12.0;
pub const TRANSCRIPT_ROW_MIN_HEIGHT_PX: f32 = 24.0;
pub const TRANSCRIPT_ICON_BOX_PX: f32 = 24.0;
/// Icons beside transcript text (shared scale: inline icon).
pub const TRANSCRIPT_ICON_PX: f32 = theme::ICON_INLINE_PX;
pub const TRANSCRIPT_CHEVRON_PX: f32 = 12.0;
pub const TRANSCRIPT_ROW_RADIUS_PX: f32 = 8.0;
pub const TRANSCRIPT_CARD_RADIUS_PX: f32 = 14.0;
pub const USER_BUBBLE_MAX_WIDTH: f32 = 0.85;
pub const USER_BUBBLE_X_PADDING_PX: f32 = 12.0;
pub const USER_BUBBLE_Y_PADDING_PX: f32 = 8.0;
pub const WORKING_DOT_PX: f32 = 6.0;

const BODY_FONT_FAMILY: &str = "Monaspace Neon";
const LABEL_FONT_FAMILY: &str = "Monaspace Argon";

pub struct ToolView<'a> {
    pub call: &'a DesktopToolCall,
    pub kind: ToolKind,
    pub state: ToolState,
    pub label: String,
}

/// One file touched by Edit/Write calls in a turn, with aggregate stats.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ChangedFile {
    pub path: String,
    pub added: bool,
    pub additions: usize,
    pub deletions: usize,
}

/// Files touched by successful Edit/Write calls, first-seen order, with the
/// same line stats the diff panel shows.
pub fn changed_files(tool_calls: &[DesktopToolCall], root: Option<&str>) -> Vec<ChangedFile> {
    changed_files_from_tool_calls(tool_calls, root)
        .into_iter()
        .map(|file| ChangedFile {
            additions: file.additions(),
            deletions: file.deletions(),
            path: file.path,
            added: file.added,
        })
        .collect()
}

fn input_str(call: &DesktopToolCall, key: &str) -> String {
    call.input
        .get(key)
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string()
}

/// Elapsed label for the working row: whole seconds under a minute, then
/// the shared duration format.
pub fn working_label(elapsed_ms: f64) -> String {
    let secs = (elapsed_ms / 1000.0).max(0.0).floor();
    if secs < 60.0 {
        format!("Working for {}s", secs.to_u64_saturating())
    } else {
        format!(
            "Working for {}",
            fold_duration_label(elapsed_ms.max(0.0).to_u64_saturating())
        )
    }
}

fn clock_time(at_ms: i64) -> Option<String> {
    Some(
        Local
            .timestamp_millis_opt(at_ms)
            .single()?
            .format(crate::clock_format::pattern())
            .to_string(),
    )
}

/// Whether expanding a tool row would reveal anything.
pub fn tool_has_body(kind: ToolKind, call: &DesktopToolCall) -> bool {
    let has = |k: &str| !input_str(call, k).is_empty();
    let has_output = call.output.as_deref().is_some_and(|o| !o.trim().is_empty());
    match kind {
        ToolKind::Bash => has("command") || has_output,
        ToolKind::Edit => {
            has("old_string")
                || has("new_string")
                || has("new_source")
                || call
                    .input
                    .get("edits")
                    .and_then(|v| v.as_array())
                    .is_some_and(|a| !a.is_empty())
        }
        ToolKind::Write => has("content"),
        ToolKind::Read => has_output,
        ToolKind::Search | ToolKind::Web | ToolKind::Other => {
            has_output || call.input.as_object().is_some_and(|o| !o.is_empty())
        }
    }
}

pub fn kind_icon(kind: ToolKind) -> &'static str {
    match kind {
        ToolKind::Read => assets::EYE_ICON_PATH,
        ToolKind::Edit => assets::SQUARE_PEN_ICON_PATH,
        ToolKind::Write => assets::TOOL_FILE_PLUS_ICON_PATH,
        ToolKind::Bash => assets::TOOL_TERMINAL_ICON_PATH,
        ToolKind::Search => assets::TOOL_SEARCH_ICON_PATH,
        ToolKind::Web => assets::TOOL_GLOBE_ICON_PATH,
        ToolKind::Other => assets::WRENCH_ICON_PATH,
    }
}

/// Index of the latest assistant message.
pub fn latest_assistant_index(messages: &[Message]) -> Option<usize> {
    messages
        .iter()
        .rposition(|m| matches!(m.role, MessageRole::Assistant))
}

fn icon(path: &'static str, size: f32, color: Hsla) -> Svg {
    svg()
        .path(path)
        .size(px(size))
        .text_color(color)
        .flex_shrink_0()
}

/// The hover group that reveals a turn's action row.
fn turn_group(user: bool, index: usize) -> SharedString {
    if user {
        format!("desktop-user-turn-{index}").into()
    } else {
        format!("desktop-turn-{index}").into()
    }
}

fn chevron(path: &'static str, open: bool, quarter_turns: f32) -> Svg {
    let el = icon(
        path,
        TRANSCRIPT_CHEVRON_PX,
        theme::rambutan_500().opacity(0.7),
    );
    if open {
        el.with_transformation(Transformation::rotate(Radians(
            std::f32::consts::FRAC_PI_2 * quarter_turns,
        )))
    } else {
        el
    }
}

/// The web's 24px icon slot in front of activity rows.
fn icon_box(path: &'static str, color: Hsla) -> Div {
    div()
        .size(px(TRANSCRIPT_ICON_BOX_PX))
        .flex_shrink_0()
        .flex()
        .items_center()
        .justify_center()
        .child(icon(path, TRANSCRIPT_ICON_PX, color))
}

/// Web `.live-shimmer`: a bright band sweeps left→right through muted text
/// every 2.2 s (`@keyframes bruhs-shimmer`, gradient rambutan-500 → 100 →
/// 500 at 42/50/58% of a 250%-wide background). gpui has no text
/// gradients, so the band is applied per character.
const SHIMMER_PERIOD_MS: u64 = 2200;
const SHIMMER_HALF_WIDTH: f32 = 0.20;

fn shimmer_text(id: impl Into<ElementId>, text: SharedString, cx: &App) -> AnyElement {
    if crate::motion::reduce_motion(cx) {
        return div()
            .id(id)
            .debug_selector(|| "live-status-static".into())
            .text_color(theme::rambutan_500())
            .child(text)
            .into_any_element();
    }
    div()
        .id(id)
        .debug_selector(|| "live-status-animated".into())
        .with_animation(
            "live-shimmer",
            Animation::new(Duration::from_millis(SHIMMER_PERIOD_MS)).repeat(),
            move |el, delta| el.child(shimmer_runs(text.clone(), delta)),
        )
        .into_any_element()
}

/// Per-character highlights for the sweep at `delta` ∈ [0, 1).
fn shimmer_runs(text: SharedString, delta: f32) -> StyledText {
    let base: Rgba = theme::rambutan_500().into();
    let peak: Rgba = theme::rambutan_100().into();
    let chars = text.chars().count().max(1);
    let count = chars.to_f32_lossy();
    // background-position 100% → 0% of a 250% gradient: the band's centre
    // travels from −25% to +125% of the text width.
    let center = -0.25 + 1.5 * delta;
    let mut highlights = Vec::with_capacity(chars);
    let mut byte = 0;
    for (i, ch) in text.chars().enumerate() {
        let t = (i.to_f32_lossy() + 0.5) / count;
        let k = (1.0 - (t - center).abs() / SHIMMER_HALF_WIDTH).clamp(0.0, 1.0);
        let color = Rgba {
            r: base.r + (peak.r - base.r) * k,
            g: base.g + (peak.g - base.g) * k,
            b: base.b + (peak.b - base.b) * k,
            a: 1.0,
        };
        let len = ch.len_utf8();
        highlights.push((
            byte..byte + len,
            HighlightStyle {
                color: Some(color.into()),
                ..Default::default()
            },
        ));
        byte += len;
    }
    StyledText::new(text).with_highlights(highlights)
}

fn activity_row_button(id: impl Into<ElementId>) -> Stateful<Div> {
    div()
        .id(id)
        .min_h(px(TRANSCRIPT_ROW_MIN_HEIGHT_PX))
        .w_full()
        .flex()
        .items_center()
        .gap(px(6.0))
        .rounded(px(TRANSCRIPT_ROW_RADIUS_PX))
        .border_1()
        .border_color(gpui::hsla(0.0, 0.0, 0.0, 0.0))
        .px(px(2.0))
        .py(px(2.0))
        .cursor_pointer()
        .hover(|el| el.bg(theme::longan_800().opacity(0.3)))
}

impl ChatView {
    fn row_open(&self, key: &str, default: bool) -> bool {
        self.expanded_rows.get(key).copied().unwrap_or(default)
    }

    pub(super) fn toggle_row(&mut self, key: String, default: bool, cx: &mut Context<'_, Self>) {
        let next = !self.row_open(&key, default);
        self.expanded_rows.insert(key, next);
        cx.notify();
    }

    fn transcript_root(&self) -> Option<&str> {
        (self.project_scoped && !self.project_path.is_empty()).then_some(self.project_path.as_str())
    }

    /// One message → its timeline rows (web `build_rows` + `render_row`).
    pub(super) fn render_transcript_message(
        &self,
        index: usize,
        msg: &Message,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> AnyElement {
        match msg.role {
            MessageRole::User => self.render_user_row(index, msg, window, cx),
            MessageRole::Assistant => self.render_assistant_turn(index, msg, window, cx),
            MessageRole::Tool => div()
                .w_full()
                .mb_2()
                .px_3()
                .py_2()
                .rounded(px(TRANSCRIPT_ROW_RADIUS_PX))
                .bg(theme::longan_950())
                .border_1()
                .border_color(gpui::white().opacity(0.05))
                .text_size(px(TRANSCRIPT_LABEL_TEXT_SIZE_PX))
                .font_family(theme::UI_SMALL_FONT_FAMILY)
                .font_family(crate::theme::FontFace::Interface.with_default(BODY_FONT_FAMILY))
                .text_color(theme::rambutan_300())
                .child(msg.content.clone())
                .into_any_element(),
        }
    }

    // ── User ────────────────────────────────────────────────────────────

    /// Web `UserRow`: a right-aligned longan-800 bubble at 85% max width,
    /// with keyboard-reachable message actions beneath it.
    fn render_user_row(
        &self,
        index: usize,
        msg: &Message,
        window: &Window,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        // A message another thread's agent sent shows who sent it, not the
        // header written for the model.
        let (caption, body) = match bruhs_models::thread_link::parse_delivery(&msg.content) {
            Some(delivery) => (Some(delivery.caption()), delivery.body.to_string()),
            None => (None, msg.content.clone()),
        };
        div()
            .id(("desktop-user-turn", index))
            .group(turn_group(true, index))
            .debug_selector(move || format!("desktop-user-message-{index}"))
            .w_full()
            .flex()
            .flex_col()
            .items_end()
            .gap_1()
            .pb_4()
            .child(
                div()
                    .max_w(relative(USER_BUBBLE_MAX_WIDTH))
                    .rounded(px(TRANSCRIPT_CARD_RADIUS_PX))
                    .bg(cx.theme().color_role("cssUserBubbleBackground").or_else(|| cx.theme().color_role("messageSurface")).unwrap_or(theme::longan_800()))
                    .when(cx.theme().color_role("cssUserBubbleBackground").is_none(), |el| el.theme_region("messageSurface", cx))
                    .px(px(USER_BUBBLE_X_PADDING_PX))
                    .py(px(USER_BUBBLE_Y_PADDING_PX))
                    .font_family(crate::theme::FontFace::Interface.with_default(BODY_FONT_FAMILY))
                    .text_size(px(TRANSCRIPT_BODY_TEXT_SIZE_PX))
                    .line_height(relative(1.5))
                    .children(crate::views::workspace::appearance::rounded_background_layer(
                        "UserBubble",
                        px(TRANSCRIPT_CARD_RADIUS_PX),
                        cx,
                    ))
                    .text_color(cx.theme().color_role("cssUserBubbleForeground").unwrap_or_else(theme::rambutan_100))
                    .flex()
                    .flex_col()
                    .when_some(caption, |bubble, caption| {
                        bubble.child(
                            div()
                                .debug_selector(move || format!("desktop-user-from-thread-{index}"))
                                .mb(px(6.0))
                                .font_family(theme::UI_SMALL_FONT_FAMILY)
                                .text_size(px(12.0))
                                .line_height(relative(1.5))
                                .text_color(theme::blueberry_400())
                                .child(caption),
                        )
                    })
                    .child(div().child(body))
                    .when(!msg.files.is_empty(), |bubble| {
                        bubble.child(self.render_sent_files(index, msg, cx))
                    }),
            )
            .child(self.render_user_actions(index, msg, window, cx))
            .into_any_element()
    }

    fn render_sent_files(&self, index: usize, msg: &Message, cx: &Context<'_, Self>) -> AnyElement {
        let mut list = div().mt(px(8.0)).flex().flex_col().gap_1();
        for (file_index, file) in msg.files.iter().enumerate() {
            let path = file.path.clone();
            let label = file.name.clone();
            list = list.child(
                div()
                    .id(SharedString::from(format!(
                        "sent-file-{index}-{file_index}"
                    )))
                    .debug_selector(|| format!("desktop-sent-file-{index}-{file_index}"))
                    .h(px(32.0))
                    .px(px(8.0))
                    .flex()
                    .items_center()
                    .rounded(px(4.0))
                    .bg(theme::longan_900().opacity(0.5))
                    .text_size(px(12.0))
                    .font_family(theme::UI_SMALL_FONT_FAMILY)
                    .text_color(theme::rambutan_300())
                    .cursor_pointer()
                    .tab_index(0)
                    .role(Role::Button)
                    .aria_label(format!("Open {}", file.name))
                    .border_1()
                    .border_color(gpui::hsla(0.0, 0.0, 0.0, 0.0))
                    .focus_visible(|el| el.border_color(theme::persimmon_400()))
                    .hover(|el| el.text_color(theme::rambutan_100()))
                    .on_click(cx.listener(move |_this, _, _, cx| {
                        cx.stop_propagation();
                        cx.emit(ChatViewEvent::OpenFile {
                            path: path.clone(),
                            label: label.clone(),
                            project: None,
                            source: false,
                        });
                    }))
                    .child(div().min_w_0().flex_1().truncate().child(file.name.clone())),
            );
        }
        list.into_any_element()
    }

    /// The handle a turn's action row tracks, made on first render.
    fn turn_actions_focus(&self, user: bool, index: usize, cx: &App) -> FocusHandle {
        self.turn_actions_focus
            .borrow_mut()
            .entry((user, index))
            .or_insert_with(|| cx.focus_handle())
            .clone()
    }

    /// Whether a turn's action row shows without hover: while focus is in
    /// it, or while its rewind waits for confirmation (web `focus-within`).
    pub(super) fn turn_actions_shown(
        &self,
        user: bool,
        index: usize,
        window: &Window,
        cx: &App,
    ) -> bool {
        let confirming = user && self.rewind_confirm == Some(index) && !self.is_streaming;
        confirming
            || self
                .turn_actions_focus(user, index, cx)
                .contains_focused(window, cx)
    }

    /// Copy, clock, and Edit from here, shown on hover, while focus is in
    /// the row, or while a rewind waits for confirmation (web: hover or
    /// `focus-within`). Hidden rows stay in the tab order.
    fn render_user_actions(
        &self,
        index: usize,
        msg: &Message,
        window: &Window,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        let confirming = self.rewind_confirm == Some(index) && !self.is_streaming;
        let time = msg.started_at_ms.and_then(clock_time).unwrap_or_default();
        let copy_text = msg.content.clone();
        let focus = self.turn_actions_focus(true, index, cx);
        let shown = self.turn_actions_shown(true, index, window, cx);
        div()
            .track_focus(&focus)
            .debug_selector(move || format!("desktop-user-actions-{index}"))
            .when(!shown, |row| {
                row.opacity(0.0)
                    .group_hover(turn_group(true, index), |row| row.opacity(1.0))
            })
            .w_full()
            .max_w(relative(USER_BUBBLE_MAX_WIDTH))
            .flex()
            .flex_wrap()
            .items_center()
            .justify_end()
            .gap_2()
            .pr_1()
            .text_size(px(TRANSCRIPT_LABEL_TEXT_SIZE_PX))
            .font_family(theme::UI_SMALL_FONT_FAMILY)
            .text_color(theme::rambutan_500())
            .font_features(theme::tabular_nums())
            .when(confirming, |row| {
                row.child(self.render_rewind_confirm(index, cx))
            })
            .when(!confirming && !self.is_streaming, |row| {
                row.child(self.render_edit_from_here(index, cx))
            })
            .child(self.render_copy_button(
                format!("desktop-user-copy-{index}").into(),
                "Copy message",
                copy_text,
                cx,
            ))
            .child(time)
            .into_any_element()
    }

    fn render_edit_from_here(&self, index: usize, cx: &Context<'_, Self>) -> AnyElement {
        self.user_text_button(
            SharedString::from(format!("desktop-edit-from-here-{index}")),
            "Edit from here",
            "Edit from here",
            theme::rambutan_400(),
            cx.listener(move |this, _, _, cx| {
                this.request_rewind_confirm(index, cx);
            }),
        )
    }

    fn render_rewind_confirm(&self, index: usize, cx: &Context<'_, Self>) -> AnyElement {
        let has_checkpoint = self
            .messages
            .get(index)
            .and_then(|message| message.checkpoint.as_deref())
            .is_some_and(|tree| !tree.is_empty());
        let in_worktree = self
            .worktree_path
            .as_deref()
            .is_some_and(|path| !path.trim().is_empty());
        let copy = if in_worktree && has_checkpoint {
            "Revert files too also undoes your own edits."
        } else if in_worktree {
            "No checkpoint for this message."
        } else {
            "The next turn starts a new session. Files stay as they are."
        };
        let keep = if in_worktree {
            "Revert and keep changes"
        } else {
            "Rewind"
        };
        div()
            .id(SharedString::from(format!("desktop-rewind-group-{index}")))
            .flex()
            .flex_wrap()
            .items_center()
            .justify_end()
            .gap_2()
            .role(Role::Group)
            .aria_label("Rewind conversation")
            .child(div().text_color(theme::rambutan_400()).child(copy))
            .child(self.user_text_button(
                SharedString::from(format!("desktop-rewind-confirm-{index}")),
                keep,
                keep,
                theme::persimmon_400(),
                cx.listener(move |this, _, _, cx| {
                    this.confirm_rewind(index, cx);
                }),
            ))
            .when(in_worktree && has_checkpoint, |row| {
                row.child(self.user_text_button(
                    SharedString::from(format!("desktop-rewind-files-{index}")),
                    "Revert files too",
                    "Revert files too",
                    theme::rambutan_400(),
                    cx.listener(move |this, _, _, cx| {
                        if this.is_streaming {
                            return;
                        }
                        cx.emit(ChatViewEvent::RevertFiles { index });
                    }),
                ))
            })
            .child(self.user_text_button(
                SharedString::from(format!("desktop-rewind-cancel-{index}")),
                "Cancel",
                "Cancel",
                theme::rambutan_400(),
                cx.listener(move |this, _, _, cx| {
                    this.cancel_rewind_confirm(cx);
                }),
            ))
            .into_any_element()
    }

    fn user_text_button(
        &self,
        id: impl Into<ElementId>,
        label: &'static str,
        text: &'static str,
        color: impl Into<Hsla>,
        on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> AnyElement {
        let color = color.into();
        div()
            .id(id)
            .h(px(TRANSCRIPT_ROW_MIN_HEIGHT_PX))
            .px(px(6.0))
            .flex()
            .items_center()
            .rounded(px(4.0))
            .text_color(color)
            .role(Role::Button)
            .aria_label(label)
            .tab_index(0)
            .cursor_pointer()
            .border_1()
            .border_color(gpui::hsla(0.0, 0.0, 0.0, 0.0))
            .focus_visible(|el| el.border_color(theme::persimmon_400()))
            .hover(|el| el.bg(theme::longan_700()).text_color(theme::rambutan_100()))
            .on_click(on_click)
            .child(text)
            .into_any_element()
    }

    /// Web `CopyButton`: a 24px icon button that copies `text`, then shows
    /// a green check for a moment.
    fn render_copy_button(
        &self,
        key: SharedString,
        label: &'static str,
        text: String,
        cx: &Context<'_, Self>,
    ) -> Stateful<Div> {
        let copied = self.copy_feedback.is_shown(&key);
        let selector = key.clone();
        let click_key = key.clone();
        div()
            .id(ElementId::Name(key))
            .debug_selector(move || selector.to_string())
            .size(px(TRANSCRIPT_ICON_BOX_PX))
            .rounded(px(4.0))
            .flex()
            .items_center()
            .justify_center()
            .text_color(theme::rambutan_500())
            .role(Role::Button)
            .aria_label(label)
            .tab_index(0)
            .border_1()
            .border_color(gpui::hsla(0.0, 0.0, 0.0, 0.0))
            .focus_visible(|el| el.border_color(theme::persimmon_400()))
            .cursor_pointer()
            .hover(|el| el.bg(theme::longan_800()).text_color(theme::rambutan_200()))
            .tooltip(move |window, cx| Tooltip::new(label).build(window, cx))
            .on_click(cx.listener(move |this, _, _, cx| {
                cx.write_to_clipboard(ClipboardItem::new_string(text.clone()));
                CopyFeedback::show(this, |chat| &mut chat.copy_feedback, click_key.clone(), cx);
            }))
            .child(if copied {
                icon(assets::CHECK_ICON_PATH, theme::ICON_PX, theme::pandan_400())
            } else {
                icon(
                    assets::COPY_ICON_PATH,
                    theme::ICON_PX,
                    theme::rambutan_500(),
                )
            })
    }

    // ── Assistant turn ───────────────────────────────────────────────────

    /// Web `build_rows` for one assistant message: live turns show
    /// thinking · text · tools · working; failed turns stay flat with the
    /// error row; settled non-latest turns fold their activity behind
    /// "Worked for …"; changed files and the hover meta close the turn.
    fn render_assistant_turn(
        &self,
        index: usize,
        msg: &Message,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> AnyElement {
        let root = self.transcript_root().map(str::to_string);
        let streaming = msg.state.is_streaming();
        let tools: Vec<ToolView<'_>> = msg
            .tool_calls
            .iter()
            .filter_map(|call| {
                let state = tool_state(call, streaming);
                self.tool_visibility.shows(state).then(|| ToolView {
                    kind: tool_kind(&call.tool),
                    state,
                    label: tool_label(call, state, root.as_deref()),
                    call,
                })
            })
            .collect();
        let latest = latest_assistant_index(&self.messages) == Some(index);
        let visible =
            self.paragraph_progress
                .visible(&msg.content, self.response_stream, streaming);
        let visible = self.tool_visibility.answer(
            streaming,
            visible,
            msg.tool_calls
                .iter()
                .filter_map(|call| call.text_offset)
                .max(),
        );
        let zen = self.tool_visibility == super::tool_visibility::Visibility::Zen;
        let has_text = !visible.trim().is_empty();

        let mut turn = div()
            .id(("desktop-turn", index))
            .group(turn_group(false, index))
            .debug_selector(move || format!("desktop-assistant-message-{index}"))
            .w_full()
            .flex()
            .flex_col();

        let has_tool_error = self.tool_visibility != super::tool_visibility::Visibility::ShowAll && tools.iter().any(|tool| tool.state == ToolState::Error);
        let thinking = (!zen && !msg.thinking.is_empty())
            .then(|| self.render_thinking_row(index, msg, streaming && !has_text, cx));
        let work = (!tools.is_empty()).then(|| {
            self.render_work_group(
                index,
                &tools,
                !streaming && tools.len() >= 2 && !has_tool_error,
                cx,
            )
        });
        let text =
            has_text.then(|| self.render_assistant_text_row(index, visible, streaming, window, cx));

        if streaming {
            let thinking_only = !has_text && tools.is_empty() && msg.thinking.is_empty();
            turn = turn
                .children(thinking)
                .children(text)
                .children(work)
                .child(self.render_working_row(msg, thinking_only));
            return turn.into_any_element();
        }

        if msg.state == super::TurnState::InterruptedByRestart {
            return turn.children(thinking).children(work).children(text)
                .child(div().px_1().text_xs().text_color(theme::rambutan_400()).child("Interrupted by restart"))
                .into_any_element();
        }

        if let Some(error) = msg.state.error() {
            turn = turn
                .children(thinking)
                .children(work)
                .children(text)
                .child(self.render_error_row(index, error, cx));
            return turn.into_any_element();
        }

        let has_activity = thinking.is_some() || work.is_some();
        if has_activity && !latest && !has_tool_error {
            let hidden: Vec<AnyElement> = thinking.into_iter().chain(work).collect();
            turn = turn.child(self.render_fold_row(index, msg, hidden, cx));
        } else {
            turn = turn.children(thinking).children(work);
        }

        if has_text {
            turn = turn.children(text);
        } else if !has_activity && msg.tool_calls.is_empty() && !zen {
            turn = turn.child(
                div()
                    .px_1()
                    .pb_2()
                    .font_family(crate::theme::FontFace::Interface.with_default(BODY_FONT_FAMILY))
                    .text_size(px(TRANSCRIPT_BODY_TEXT_SIZE_PX))
                    .text_color(theme::rambutan_500())
                    .child("(empty response)"),
            );
        }
        turn = turn.children(self.render_published_artifacts(index, msg, cx));

        let files = changed_files(&msg.tool_calls, root.as_deref());
        if !files.is_empty() && !zen {
            turn = turn.child(self.render_changed_files_card(index, &files, latest, cx));
        }
        turn = turn.child(self.render_assistant_meta_row(index, msg, window, cx));
        turn.into_any_element()
    }

    /// Web `AssistantTextRow`: prose at body-sm / relaxed leading.
    fn render_assistant_text_row(
        &self,
        index: usize,
        visible: &str,
        streaming: bool,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> AnyElement {
        div()
            .min_w_0()
            .px_1()
            .pt(px(2.0))
            .pb_2()
            .flex()
            .flex_col()
            .gap_1()
            .font_family(crate::theme::FontFace::Interface.with_default(BODY_FONT_FAMILY))
            .text_size(px(TRANSCRIPT_BODY_TEXT_SIZE_PX))
            .line_height(relative(1.625))
            .text_color(theme::rambutan_200())
            .children(self.render_assistant_content_blocks(index, visible, streaming, window, cx))
            .into_any_element()
    }

    /// Web `fold_row`: "Worked for 3m 43s ›", the hidden rows below it
    /// behind a left rule once opened.
    fn render_fold_row(
        &self,
        index: usize,
        msg: &Message,
        hidden: Vec<AnyElement>,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        let expanded = self.expanded_folds.contains(&index);
        let label = turn_fold_label(turn_duration_ms(msg), msg.state.is_stopped());
        let aria = label.clone();
        div()
            .pb_2()
            .child(
                div()
                    .id(("desktop-turn-fold", index))
                    .debug_selector(move || format!("desktop-turn-fold-{index}"))
                    .min_h(px(TRANSCRIPT_ROW_MIN_HEIGHT_PX))
                    .flex()
                    .items_center()
                    .gap_1()
                    .rounded(px(TRANSCRIPT_ROW_RADIUS_PX))
                    .px_1()
                    .py(px(2.0))
                    .font_family(crate::theme::FontFace::Interface.with_default(BODY_FONT_FAMILY))
                    .text_size(px(TRANSCRIPT_BODY_TEXT_SIZE_PX))
                    .text_color(theme::rambutan_400())
                    .tab_index(0)
                    .role(Role::Button)
                    .aria_expanded(expanded)
                    .aria_label(aria)
                    .border_1()
                    .border_color(gpui::hsla(0.0, 0.0, 0.0, 0.0))
                    .focus_visible(|el| el.border_color(theme::persimmon_400()))
                    .cursor_pointer()
                    .hover(|el| el.text_color(theme::rambutan_200()))
                    .map(|el| {
                        on_press_view(el, cx, move |this, _window, cx| {
                            this.toggle_fold(index, cx);
                        })
                    })
                    .child(label)
                    .child(chevron(assets::CHEVRON_RIGHT_ICON_PATH, expanded, 1.0)),
            )
            .when(expanded, |el| {
                el.child(
                    div()
                        .mt_1()
                        .flex()
                        .flex_col()
                        .border_l_1()
                        .border_color(theme::longan_800().opacity(0.7))
                        .pl_2()
                        .children(hidden),
                )
            })
            .into_any_element()
    }

    // ── Thinking ─────────────────────────────────────────────────────────

    /// Web `ThinkingRow`: brain · "Thought process" / "Thinking" · chevron,
    /// the text behind a left rule when open.
    fn render_thinking_row(
        &self,
        index: usize,
        msg: &Message,
        live: bool,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        let open = self.expanded_thinking.contains(&index);
        let label = if live { "Thinking" } else { "Thought process" };
        div()
            .pb_1()
            .child(
                activity_row_button(("desktop-thinking-toggle", index))
                    .debug_selector(move || format!("desktop-thinking-toggle-{index}"))
                    .tab_index(0)
                    .role(Role::Button)
                    .aria_expanded(open)
                    .aria_label(label)
                    .focus_visible(|el| el.border_color(theme::persimmon_400()))
                    .map(|el| {
                        on_press_view(el, cx, move |this, _window, cx| {
                            this.toggle_thinking_block(index, cx);
                        })
                    })
                    .child(icon_box(assets::BRAIN_ICON_PATH, theme::rambutan_500()))
                    .child(
                        div()
                            .min_w_0()
                            .flex_1()
                            .truncate()
                            .font_family(
                                crate::theme::FontFace::Interface.with_default(BODY_FONT_FAMILY),
                            )
                            .text_size(px(TRANSCRIPT_BODY_TEXT_SIZE_PX))
                            .text_color(theme::rambutan_400())
                            .map(|el| {
                                if live {
                                    el.child(shimmer_text(
                                        ("desktop-thinking-shimmer", index),
                                        label.into(),
                                        cx,
                                    ))
                                } else {
                                    el.child(label)
                                }
                            }),
                    )
                    .child(chevron(assets::CHEVRON_DOWN_ICON_PATH, open, 2.0)),
            )
            .when(open, |el| {
                el.child(
                    div()
                        .debug_selector(move || format!("desktop-thinking-body-{index}"))
                        .ml(px(28.0))
                        .mt_1()
                        .border_l_1()
                        .border_color(theme::longan_800().opacity(0.7))
                        .pl_3()
                        .font_family(
                            crate::theme::FontFace::Interface.with_default(BODY_FONT_FAMILY),
                        )
                        .text_size(px(TRANSCRIPT_BODY_TEXT_SIZE_PX))
                        .line_height(relative(1.5))
                        .text_color(theme::rambutan_400())
                        .child(msg.thinking.clone()),
                )
            })
            .into_any_element()
    }

    // ── Tools ────────────────────────────────────────────────────────────

    /// Web `WorkGroup`: one tool renders as a row; several fold behind
    /// `Ran 1 command and edited 1 file` (open by default while live).
    fn render_work_group(
        &self,
        index: usize,
        tools: &[ToolView<'_>],
        collapsed_by_default: bool,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        if tools.len() == 1 {
            return div()
                .pb_2()
                .child(self.render_tool_row(index, &tools[0], cx))
                .into_any_element();
        }
        let key = format!("{index}:work");
        let default_open = !collapsed_by_default;
        let open = self.row_open(&key, default_open);
        let kinds: Vec<ToolKind> = tools.iter().map(|t| t.kind).collect();
        let summary = work_summary(&kinds);
        let first_kind = kinds[0];
        let header_icon = if kinds.iter().all(|k| *k == first_kind) {
            kind_icon(first_kind)
        } else {
            assets::HAMMER_ICON_PATH
        };
        let any_running = tools.iter().any(|t| t.state == ToolState::Running);
        let key_for_click = key;
        div()
            .pb_2()
            .child(
                activity_row_button(("desktop-work-group", index))
                    .debug_selector(move || format!("desktop-work-group-{index}"))
                    .tab_index(0)
                    .role(Role::Button)
                    .aria_expanded(open)
                    .aria_label(summary.clone())
                    .focus_visible(|el| el.border_color(theme::persimmon_400()))
                    .map(|el| {
                        on_press_view(el, cx, move |this, _window, cx| {
                            this.toggle_row(key_for_click.clone(), default_open, cx);
                        })
                    })
                    .child(icon_box(header_icon, theme::rambutan_500()))
                    .child(
                        div()
                            .min_w_0()
                            .flex_1()
                            .truncate()
                            .font_family(
                                crate::theme::FontFace::Interface.with_default(BODY_FONT_FAMILY),
                            )
                            .text_size(px(TRANSCRIPT_BODY_TEXT_SIZE_PX))
                            .text_color(theme::rambutan_400())
                            .map(|el| {
                                if any_running {
                                    el.child(shimmer_text(
                                        ("desktop-work-group-shimmer", index),
                                        summary.into(),
                                        cx,
                                    ))
                                } else {
                                    el.child(summary)
                                }
                            }),
                    )
                    .child(chevron(assets::CHEVRON_DOWN_ICON_PATH, open, 2.0)),
            )
            .when(open, |el| {
                el.child(
                    div()
                        .ml_3()
                        .mt(px(2.0))
                        .flex()
                        .flex_col()
                        .gap(px(1.0))
                        .border_l_1()
                        .border_color(theme::longan_800().opacity(0.7))
                        .pl(px(6.0))
                        .children(
                            tools
                                .iter()
                                .map(|tool| self.render_tool_row(index, tool, cx)),
                        ),
                )
            })
            .into_any_element()
    }

    /// Web `ToolRow`: kind icon · label · chevron; the body (terminal
    /// output, diff, file preview) opens under it.
    fn render_tool_row(
        &self,
        index: usize,
        tool: &ToolView<'_>,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        let can_expand = tool_has_body(tool.kind, tool.call);
        let key = format!("{index}:tool:{}", tool.call.id);
        let open = can_expand && self.row_open(&key, false);
        let live = tool.state == ToolState::Running;
        let (icon_color, label_color) = match tool.state {
            ToolState::Error => (theme::lychee_400().opacity(0.7), theme::lychee_300()),
            ToolState::Interrupted => (theme::rambutan_500(), theme::rambutan_500()),
            _ => (theme::rambutan_500(), theme::rambutan_400()),
        };
        let title = match tool.state {
            ToolState::Interrupted => format!("{} (interrupted)", tool.label),
            ToolState::Error => format!("{} (failed)", tool.label),
            _ => tool.label.clone(),
        };
        let debug_key = element_key(&tool.call.id);
        let key_for_click = key;
        let mut row = div()
            .id(("desktop-tool-row", debug_key))
            .debug_selector(move || format!("desktop-tool-call-card-{debug_key}"))
            .flex()
            .flex_col()
            .rounded(px(TRANSCRIPT_ROW_RADIUS_PX))
            .border_1()
            .border_color(gpui::hsla(0.0, 0.0, 0.0, 0.0))
            .px(px(2.0))
            .py(px(2.0));
        if can_expand {
            row = row
                .tab_index(0)
                .role(Role::Button)
                .aria_expanded(open)
                .aria_label(title.clone())
                .focus_visible(|el| el.border_color(theme::persimmon_400()))
                .cursor_pointer()
                .hover(|el| el.bg(theme::longan_800().opacity(0.3)))
                .tooltip(move |window, cx| Tooltip::new(title.clone()).build(window, cx))
                .map(|el| {
                    on_press_view(el, cx, move |this, _window, cx| {
                        this.toggle_row(key_for_click.clone(), false, cx);
                    })
                });
        }
        row.child(
            div()
                .min_h(px(TRANSCRIPT_ROW_MIN_HEIGHT_PX))
                .flex()
                .items_center()
                .gap(px(6.0))
                .child(icon_box(kind_icon(tool.kind), icon_color))
                .child(
                    div()
                        .min_w_0()
                        .flex_1()
                        .when(!open, |el| el.truncate())
                        .font_family(
                            crate::theme::FontFace::Interface.with_default(BODY_FONT_FAMILY),
                        )
                        .text_size(px(TRANSCRIPT_BODY_TEXT_SIZE_PX))
                        .text_color(label_color)
                        .map(|el| {
                            if live {
                                el.child(shimmer_text(
                                    ("desktop-tool-shimmer", debug_key),
                                    SharedString::from(tool.label.clone()),
                                    cx,
                                ))
                            } else {
                                el.child(tool.label.clone())
                            }
                        }),
                )
                .when(tool.state == ToolState::Error, |el| {
                    el.child(
                        div()
                            .debug_selector(move || format!("desktop-tool-error-badge-{debug_key}"))
                            .child(icon(
                                assets::CLOSE_ICON_PATH,
                                TRANSCRIPT_CHEVRON_PX,
                                theme::lychee_400().opacity(0.7),
                            )),
                    )
                })
                .child(
                    div()
                        .size(px(16.0))
                        .flex_shrink_0()
                        .flex()
                        .items_center()
                        .justify_center()
                        .when(!can_expand, |el| el.invisible())
                        .child(chevron(assets::CHEVRON_DOWN_ICON_PATH, open, 2.0)),
                ),
        )
        .when(open, |el| {
            el.child(
                div()
                    .ml(px(28.0))
                    .mt_1()
                    .cursor_default()
                    .child(self.render_tool_call_body(tool.call, cx)),
            )
        })
        .into_any_element()
    }

    // ── Working ──────────────────────────────────────────────────────────

    /// Web `WorkingRow`: a blueberry dot and "Working for 12s" (durian and
    /// "Waiting for your approval" while a permission prompt is up).
    fn render_working_row(&self, msg: &Message, thinking_only: bool) -> AnyElement {
        let awaiting = self.pending_permission.is_some() || self.pending_secret.is_some();
        let label = if awaiting {
            "Waiting for your approval".to_string()
        } else {
            let elapsed = msg
                .started_at_ms
                .map(|start| EpochMs::now().as_f64() - EpochMs::saturating_from_i64(start).as_f64())
                .or_else(|| {
                    self.stream_started_at
                        .map(|started| started.elapsed().as_millis().to_f64_lossy())
                });
            match elapsed {
                Some(ms) => working_label(ms),
                None => "Working".to_string(),
            }
        };
        div()
            .debug_selector(|| "desktop-working-row".to_string())
            .pb_2()
            .pt(px(2.0))
            .when(thinking_only, |el| {
                el.child(
                    div()
                        .min_h(px(TRANSCRIPT_ROW_MIN_HEIGHT_PX))
                        .flex()
                        .items_center()
                        .gap(px(6.0))
                        .px(px(2.0))
                        .child(icon_box(assets::BRAIN_ICON_PATH, theme::rambutan_500()))
                        .child(
                            div()
                                .font_family(
                                    crate::theme::FontFace::Interface
                                        .with_default(BODY_FONT_FAMILY),
                                )
                                .text_size(px(TRANSCRIPT_BODY_TEXT_SIZE_PX))
                                .text_color(theme::rambutan_300())
                                .child("Thinking"),
                        ),
                )
            })
            .child(
                div()
                    .h(px(TRANSCRIPT_ROW_MIN_HEIGHT_PX))
                    .flex()
                    .items_center()
                    .gap_2()
                    .px(px(6.0))
                    .font_family(crate::theme::FontFace::Interface.with_default(BODY_FONT_FAMILY))
                    .text_size(px(TRANSCRIPT_BODY_TEXT_SIZE_PX))
                    .text_color(theme::rambutan_400())
                    .font_features(theme::tabular_nums())
                    .child(
                        div()
                            .size(px(8.0))
                            .flex_shrink_0()
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(
                                div()
                                    .size(px(WORKING_DOT_PX))
                                    .rounded_full()
                                    .bg(if awaiting {
                                        theme::durian_400()
                                    } else {
                                        theme::blueberry_400()
                                    }),
                            ),
                    )
                    .child(label),
            )
            .into_any_element()
    }

    // ── Changed files ────────────────────────────────────────────────────

    /// Web `ChangedFilesCard`: `› 2 changed files +12 −1  Show files` with
    /// an "Open diff" button; the file list (A/M · dir/name · +a −d) opens
    /// under it, expanded by default on the latest turn.
    fn render_changed_files_card(
        &self,
        index: usize,
        files: &[ChangedFile],
        latest: bool,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        let key = format!("{index}:files");
        let open = self.row_open(&key, latest);
        let n = files.len();
        let adds: usize = files.iter().map(|f| f.additions).sum();
        let dels: usize = files.iter().map(|f| f.deletions).sum();
        let count_label = format!("{n} changed file{}", if n == 1 { "" } else { "s" });
        let key_for_click = key;
        div()
            .debug_selector(move || format!("desktop-changed-files-{index}"))
            .mb_4()
            .mt_1()
            .rounded(px(TRANSCRIPT_CARD_RADIUS_PX))
            .bg(theme::longan_900())
            .border_1()
            .border_color(gpui::white().opacity(0.05))
            .p_2()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .child(
                        div()
                            .id(("desktop-changed-files-toggle", index))
                            .debug_selector(move || format!("desktop-changed-files-toggle-{index}"))
                            .min_w_0()
                            .flex_1()
                            .flex()
                            .items_center()
                            .gap(px(6.0))
                            .rounded(px(TRANSCRIPT_ROW_RADIUS_PX))
                            .px_2()
                            .py(px(6.0))
                            .tab_index(0)
                            .role(Role::Button)
                            .aria_expanded(open)
                            .aria_label(count_label.clone())
                            .border_1()
                            .border_color(gpui::hsla(0.0, 0.0, 0.0, 0.0))
                            .focus_visible(|el| el.border_color(theme::persimmon_400()))
                            .cursor_pointer()
                            .hover(|el| el.bg(theme::longan_800().opacity(0.4)))
                            .map(|el| {
                                on_press_view(el, cx, move |this, _window, cx| {
                                    this.toggle_row(key_for_click.clone(), latest, cx);
                                })
                            })
                            .child({
                                let el = icon(
                                    assets::CHEVRON_RIGHT_ICON_PATH,
                                    TRANSCRIPT_ICON_PX,
                                    theme::rambutan_500(),
                                );
                                if open {
                                    el.with_transformation(Transformation::rotate(Radians(
                                        std::f32::consts::FRAC_PI_2,
                                    )))
                                } else {
                                    el
                                }
                            })
                            .child(
                                div()
                                    .flex_shrink_0()
                                    .flex()
                                    .items_center()
                                    .gap(px(6.0))
                                    .whitespace_nowrap()
                                    .font_family(
                                        crate::theme::FontFace::Interface
                                            .with_default(LABEL_FONT_FAMILY),
                                    )
                                    .text_size(px(TRANSCRIPT_LABEL_TEXT_SIZE_PX))
                                    .font_family(theme::UI_SMALL_FONT_FAMILY)
                                    .text_color(theme::rambutan_100())
                                    .child(count_label)
                                    .child(
                                        div()
                                            .text_color(theme::pandan_400())
                                            .font_features(theme::tabular_nums())
                                            .child(format!("+{adds}")),
                                    )
                                    .child(
                                        div()
                                            .text_color(theme::lychee_400())
                                            .font_features(theme::tabular_nums())
                                            .child(format!("−{dels}")),
                                    ),
                            )
                            .child(
                                div()
                                    .ml_1()
                                    .min_w_0()
                                    .flex_1()
                                    .truncate()
                                    .font_family(
                                        crate::theme::FontFace::Interface
                                            .with_default(LABEL_FONT_FAMILY),
                                    )
                                    .text_size(px(TRANSCRIPT_LABEL_TEXT_SIZE_PX))
                                    .font_family(theme::UI_SMALL_FONT_FAMILY)
                                    .text_color(theme::rambutan_500())
                                    .child(if open { "Hide files" } else { "Show files" }),
                            ),
                    )
                    .child(
                        div()
                            .id(("desktop-open-diff", index))
                            .debug_selector(move || format!("desktop-open-diff-{index}"))
                            .h(px(28.0))
                            .flex_shrink_0()
                            .flex()
                            .items_center()
                            .gap(px(6.0))
                            .rounded(px(TRANSCRIPT_ROW_RADIUS_PX))
                            .px(px(10.0))
                            .border_1()
                            .border_color(gpui::white().opacity(0.10))
                            .font_family(
                                crate::theme::FontFace::Interface.with_default(LABEL_FONT_FAMILY),
                            )
                            .text_size(px(TRANSCRIPT_LABEL_TEXT_SIZE_PX))
                            .font_family(theme::UI_SMALL_FONT_FAMILY)
                            .text_color(theme::rambutan_300())
                            .tab_index(0)
                            .role(Role::Button)
                            .aria_label("Open diff")
                            .focus_visible(|el| el.border_color(theme::persimmon_400()))
                            .cursor_pointer()
                            .hover(|el| {
                                el.bg(theme::longan_800().opacity(0.6))
                                    .text_color(theme::rambutan_100())
                            })
                            .map(|el| {
                                on_press_view(el, cx, move |_, _window, cx| {
                                    cx.emit(ChatViewEvent::OpenDiff {
                                        message_index: index,
                                    });
                                })
                            })
                            .child(icon(
                                assets::FILE_DIFF_ICON_PATH,
                                TRANSCRIPT_CHEVRON_PX,
                                theme::rambutan_300(),
                            ))
                            .child("Open diff"),
                    ),
            )
            .when(open, |el| {
                el.child(div().mt_1().flex().flex_col().gap(px(1.0)).px_1().children(
                    files.iter().enumerate().map(|(file_index, file)| {
                        let (dir, name) = split_path(&file.path);
                        div()
                            .id(("desktop-changed-file", index * 1000 + file_index))
                            .w_full()
                            .flex()
                            .items_center()
                            .gap_2()
                            .rounded(px(TRANSCRIPT_ROW_RADIUS_PX))
                            .px_2()
                            .py_1()
                            .font_family(
                                crate::theme::FontFace::Interface.with_default(BODY_FONT_FAMILY),
                            )
                            .text_size(px(TRANSCRIPT_LABEL_TEXT_SIZE_PX))
                            .tab_index(0)
                            .role(Role::Button)
                            .aria_label(format!("Open diff for {}", file.path))
                            .border_1()
                            .border_color(gpui::hsla(0.0, 0.0, 0.0, 0.0))
                            .focus_visible(|el| el.border_color(theme::persimmon_400()))
                            .cursor_pointer()
                            .hover(|el| el.bg(theme::longan_800().opacity(0.4)))
                            .on_click(cx.listener(move |_, _, _, cx| {
                                cx.emit(ChatViewEvent::OpenDiff {
                                    message_index: index,
                                })
                            }))
                            .child(
                                div()
                                    .w_3()
                                    .flex_shrink_0()
                                    .text_color(if file.added {
                                        theme::pandan_400()
                                    } else {
                                        theme::durian_400()
                                    })
                                    .child(if file.added { "A" } else { "M" }),
                            )
                            .child(
                                div()
                                    .min_w_0()
                                    .flex_1()
                                    .flex()
                                    .truncate()
                                    .child(div().text_color(theme::rambutan_500()).child(dir))
                                    .child(div().text_color(theme::rambutan_100()).child(name)),
                            )
                            .child(
                                div()
                                    .text_color(theme::pandan_400())
                                    .font_features(theme::tabular_nums())
                                    .child(format!("+{}", file.additions)),
                            )
                            .child(
                                div()
                                    .text_color(theme::lychee_400())
                                    .font_features(theme::tabular_nums())
                                    .child(format!("−{}", file.deletions)),
                            )
                    }),
                ))
            })
            .into_any_element()
    }

    // ── Error ────────────────────────────────────────────────────────────

    /// Web `ErrorRow`: a lychee alert with the provider's message and a
    /// Retry button that resends the prompt that failed.
    fn render_error_row(&self, index: usize, message: &str, cx: &Context<'_, Self>) -> AnyElement {
        let can_retry = self.retry_prompt_for(index).is_some();
        div()
            .debug_selector(move || format!("desktop-turn-error-{index}"))
            .mb_4()
            .flex()
            .items_start()
            .gap_2()
            .rounded(px(TRANSCRIPT_ROW_RADIUS_PX))
            .bg(theme::lychee_500().opacity(0.10))
            .border_1()
            .border_color(theme::lychee_400().opacity(0.30))
            .px_3()
            .py_2()
            .font_family(crate::theme::FontFace::Interface.with_default(LABEL_FONT_FAMILY))
            .text_size(px(TRANSCRIPT_LABEL_TEXT_SIZE_PX))
            .font_family(theme::UI_SMALL_FONT_FAMILY)
            .text_color(theme::lychee_200())
            .child(div().mt(px(2.0)).child(icon(
                assets::CIRCLE_ALERT_ICON_PATH,
                TRANSCRIPT_ICON_PX,
                theme::lychee_400(),
            )))
            .child(
                div()
                    .min_w_0()
                    .flex_1()
                    .line_height(relative(1.5))
                    .child(message.to_string()),
            )
            .when(can_retry, |el| {
                el.child(
                    div()
                        .id(("desktop-turn-retry", index))
                        .debug_selector(move || format!("desktop-turn-retry-{index}"))
                        .h(px(TRANSCRIPT_ROW_MIN_HEIGHT_PX))
                        .flex_shrink_0()
                        .flex()
                        .items_center()
                        .gap_1()
                        .rounded(px(4.0))
                        .px_2()
                        .text_color(theme::lychee_100())
                        .tab_index(0)
                        .role(Role::Button)
                        .aria_label("Retry")
                        .border_1()
                        .border_color(gpui::hsla(0.0, 0.0, 0.0, 0.0))
                        .focus_visible(|el| el.border_color(theme::persimmon_400()))
                        .cursor_pointer()
                        .hover(|el| el.bg(theme::lychee_500().opacity(0.20)))
                        .map(|el| {
                            on_press_view(el, cx, move |this, _window, cx| {
                                this.retry_failed_turn(index, cx);
                            })
                        })
                        .child(icon(
                            assets::ROTATE_CCW_ICON_PATH,
                            TRANSCRIPT_CHEVRON_PX,
                            theme::lychee_100(),
                        ))
                        .child("Retry"),
                )
            })
            .into_any_element()
    }

    /// The prompt a failed turn answered (the user message right before it).
    fn retry_prompt_for(&self, index: usize) -> Option<String> {
        let prev = self.messages.get(index.checked_sub(1)?)?;
        matches!(prev.role, MessageRole::User)
            .then(|| prev.content.clone())
            .filter(|t| !t.trim().is_empty())
    }

    /// Web `on_retry`: drop the failed turn and its prompt, then resend it.
    pub(super) fn retry_failed_turn(&mut self, index: usize, cx: &mut Context<'_, Self>) {
        if self.is_streaming {
            return;
        }
        if let Some(prompt) = self.drop_failed_turn(index) {
            self.submit_prompt(prompt, cx);
        }
    }

    /// Removes the failed turn at `index` and its prompt; returns the prompt
    /// to resend. The resent turn reuses these indices, so their fold and
    /// thinking state goes too.
    pub(super) fn drop_failed_turn(&mut self, index: usize) -> Option<String> {
        let prompt = self.retry_prompt_for(index)?;
        self.messages.drain(index - 1..=index);
        self.forget_message_expansions();
        Some(prompt)
    }

    // ── Meta ─────────────────────────────────────────────────────────────

    pub(crate) fn can_fork_response(&self, index: usize) -> bool {
        let Some((reply, earlier)) = self
            .messages
            .get(..=index)
            .and_then(|messages| messages.split_last())
        else {
            return false;
        };
        self.thread_id.is_some()
            && reply.role == MessageRole::Assistant
            && matches!(reply.state, super::TurnState::Finished)
            && earlier
                .iter()
                .rev()
                .find(|m| m.role != MessageRole::Tool)
                .is_some_and(|m| m.role == MessageRole::User)
    }

    pub(super) fn render_fork_source(&self, cx: &Context<'_, Self>) -> Option<AnyElement> {
        use crate::views::web_button::{web_button, ButtonState, WebButtonSize, WebButtonVariant};
        let fork = self.fork.as_ref()?;
        let parent_title = cx
            .try_global::<crate::app_state::ThreadCatalog>()
            .and_then(|c| c.threads.iter().find(|(id, _)| id == &fork.parent))
            .map(|(_, title)| title);
        let title = parent_title.unwrap_or(&fork.title);
        let row = div()
            .min_w_0()
            .px_4()
            .py_1()
            .flex()
            .items_center()
            .gap_2()
            .text_size(px(TRANSCRIPT_LABEL_TEXT_SIZE_PX))
            .text_color(theme::rambutan_500())
            .child(icon(
                assets::GIT_BRANCH_ICON_PATH,
                14.,
                theme::rambutan_500(),
            ))
            .child(
                div()
                    .min_w_0()
                    .truncate()
                    .child(format!("Forked from {title}")),
            );
        Some(
            row.when(parent_title.is_some(), |row| {
                let link = web_button(
                    "open-fork-source",
                    WebButtonVariant::Ghost,
                    WebButtonSize::Small,
                    ButtonState::Enabled,
                )
                .debug_selector(|| "open-fork-source".into())
                .aria_label("Open source conversation")
                .flex_shrink_0()
                .child("Open source");
                row.child(on_press_view(link, cx, |_, _, cx| {
                    cx.emit(ChatViewEvent::OpenForkSource)
                }))
            })
            .into_any_element(),
        )
    }

    /// Copy, citation and turn metadata, shown on hover or while focus is
    /// in the row (web: `group-hover/turn` or `focus-within`). Hidden rows
    /// stay in the tab order, so the keyboard still reaches them.
    fn render_assistant_meta_row(
        &self,
        index: usize,
        msg: &Message,
        window: &Window,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        let focus = self.turn_actions_focus(false, index, cx);
        let shown = self.turn_actions_shown(false, index, window, cx);
        let duration = turn_duration_ms(msg).map(fold_duration_label);
        let tokens = msg.usage.as_ref().map(|u| {
            format!(
                "{} in, {} out",
                format_tokens(u.input_tokens),
                format_tokens(u.output_tokens)
            )
        });
        let time = msg
            .completed_at_ms
            .or(msg.started_at_ms)
            .and_then(clock_time);
        let has_text = !msg.content.trim().is_empty();
        let exportable = has_text || self.has_published_artifacts(msg);
        let row = div()
            .track_focus(&focus)
            .debug_selector(move || format!("desktop-usage-meta-{index}"))
            .when(!shown, |row| {
                row.opacity(0.0)
                    .group_hover(turn_group(false, index), |row| row.opacity(1.0))
            })
            .min_h(px(TRANSCRIPT_ROW_MIN_HEIGHT_PX))
            .flex()
            .flex_wrap()
            .items_center()
            .gap_2()
            .px_1()
            .font_family(crate::theme::FontFace::Interface.with_default(LABEL_FONT_FAMILY))
            .text_size(px(TRANSCRIPT_LABEL_TEXT_SIZE_PX))
            .font_family(theme::UI_SMALL_FONT_FAMILY)
            .text_color(theme::rambutan_500())
            .font_features(theme::tabular_nums())
            .when(has_text, |el| {
                let quote = msg.content.clone();
                el.child(
                    div()
                        .id(("desktop-cite", index))
                        .h(px(TRANSCRIPT_ROW_MIN_HEIGHT_PX))
                        .px(px(6.0))
                        .flex()
                        .items_center()
                        .rounded(px(4.0))
                        .text_color(theme::persimmon_400())
                        .cursor_pointer()
                        .hover(|el| el.bg(theme::longan_800().opacity(0.6)))
                        .role(Role::Button)
                        .aria_label("Cite in composer")
                        .tab_index(0)
                        .border_1()
                        .border_color(gpui::hsla(0.0, 0.0, 0.0, 0.0))
                        .focus_visible(|el| el.border_color(theme::persimmon_400()))
                        .on_click(cx.listener(move |this, _ev, window, cx| {
                            cx.stop_propagation();
                            this.insert_context_chip(
                                bruhs_models::logic::context_chip::ChipKind::Citation {
                                    message_id: index.to_string(),
                                    quote: quote.clone(),
                                    comment: String::new(),
                                },
                                window,
                                cx,
                            );
                        }))
                        .child("Cite in composer"),
                )
                .child(self.render_copy_button(
                    format!("desktop-turn-copy-{index}").into(),
                    "Copy response",
                    msg.content.clone(),
                    cx,
                ))
                .child(self.render_response_export(index, cx))
            })
            .when(!has_text && self.has_published_artifacts(msg), |el| {
                el.child(self.render_response_export(index, cx))
            })
            .when(self.can_fork_response(index), |el| {
                use crate::views::web_button::{
                    web_button, ButtonState, WebButtonSize, WebButtonVariant,
                };
                let button = web_button(
                    ("fork-from-reply", index),
                    WebButtonVariant::Ghost,
                    WebButtonSize::IconSmall,
                    ButtonState::Enabled,
                )
                .debug_selector(move || format!("desktop-fork-from-reply-{index}"))
                .aria_label("Fork from this response")
                .tooltip(|w, cx| Tooltip::new("Fork from this response").build(w, cx))
                .child(icon(
                    assets::GIT_BRANCH_ICON_PATH,
                    14.,
                    theme::rambutan_400(),
                ));
                el.child(on_press_view(button, cx, move |_, _, cx| {
                    cx.emit(ChatViewEvent::ForkFromReply { index })
                }))
            })
            .children(msg.model.clone().map(|m| div().truncate().child(m)))
            .children(duration.map(|d| div().child(d)))
            .children(tokens.map(|t| div().child(t)))
            .children(time.map(|t| div().child(t)));
        div()
            .pb_4()
            .flex()
            .flex_col()
            .child(row)
            .children(
                exportable
                    .then(|| self.render_response_export_panel(index, cx))
                    .flatten(),
            )
            .into_any_element()
    }
}

/// `BRUHS_DESKTOP_SMOKE_SCENE=transcript`: the web's fixture thread
/// ("Add --json flag to the CLI") so both transcripts can be compared at
/// the same size.
#[cfg(any(test, debug_assertions))]
pub(super) fn transcript_fixture_messages(root: &str) -> Vec<Message> {
    use serde_json::json;
    let call =
        |id: &str, tool: &str, input: serde_json::Value, output: Option<&str>| DesktopToolCall {
            text_offset: None,
            id: id.to_string(),
            tool: tool.to_string(),
            input,
            output: output.map(str::to_string),
            is_error: false,
        };
    let message =
        |role: MessageRole, content: &str, started: i64, completed: Option<i64>| Message {
            role,
            content: content.to_string(),
            model: Some("claude-sonnet-4-6".to_string()),
            thinking: String::new(),
            usage: None,
            duration_ms: completed
                .map(|c| u64::try_from(c - started).expect("fixture completes after it starts")),
            started_at_ms: Some(started),
            completed_at_ms: completed,
            tool_calls: Vec::new(),
            files: Vec::new(),
            state: TurnState::Finished,
            checkpoint: None,
        };
    let base = 1_788_565_144_826_i64;
    let mut first = message(
        MessageRole::Assistant,
        "Added a `--json` flag and a small `json` module.\n\n## What changed\n\n- `tasks.rs`: `print_tasks` takes `json: bool` and pretty-prints with `serde_json`\n- `json.rs`: shared `emit` helper for future subcommands\n- README: documented the flag under **Tasks**\n\n```sh\nbruhs tasks list --json | jq '.[] | select(.status == \"open\")'\n```\n\nAll 14 tests pass.",
        base + 1_000,
        Some(base + 43_000),
    );
    first.thinking = "The CLI uses clap; adding a global flag and threading it into print_tasks is the smallest change.".to_string();
    first.tool_calls = vec![
        call(
            "t1",
            "Read",
            json!({"file_path": format!("{root}/bruhs-cli/src/main.rs")}),
            Some(
                "1\tuse clap::Parser;\n2\t\n3\t#[derive(Parser)]\n4\tstruct Cli { #[command(subcommand)] cmd: Cmd }\n",
            ),
        ),
        call(
            "t2",
            "Grep",
            json!({"pattern": "fn print_tasks", "path": "bruhs-cli/src"}),
            Some("bruhs-cli/src/tasks.rs:41:pub fn print_tasks(tasks: &[Task]) {"),
        ),
        call(
            "t3",
            "Edit",
            json!({
                "file_path": format!("{root}/bruhs-cli/src/tasks.rs"),
                "old_string": "pub fn print_tasks(tasks: &[Task]) {\n    for t in tasks {\n        println!(\"{} {}\", t.id, t.title);\n    }\n}",
                "new_string": "pub fn print_tasks(tasks: &[Task], json: bool) {\n    if json {\n        println!(\"{}\", serde_json::to_string_pretty(tasks).unwrap());\n        return;\n    }\n    for t in tasks {\n        println!(\"{} {}\", t.id, t.title);\n    }\n}"
            }),
            Some("The file has been updated."),
        ),
        call(
            "t4",
            "Write",
            json!({
                "file_path": format!("{root}/bruhs-cli/src/json.rs"),
                "content": "//! JSON output helpers.\n\nuse serde::Serialize;\n\npub fn emit<T: Serialize>(value: &T) {\n    println!(\"{}\", serde_json::to_string_pretty(value).expect(\"serializable\"));\n}\n"
            }),
            Some("File created."),
        ),
        call(
            "t5",
            "Bash",
            json!({"command": "cd bruhs-cli && cargo test --quiet"}),
            Some(
                "running 14 tests\n..............\ntest result: ok. 14 passed; 0 failed; 0 ignored\n",
            ),
        ),
    ];
    let mut failed = message(
        MessageRole::Assistant,
        "",
        base + 61_000,
        Some(base + 62_000),
    );
    failed.state = TurnState::Failed {
        error: "Request failed: 502 Bad Gateway from provider".to_string(),
    };
    let mut live = message(
        MessageRole::Assistant,
        "Running clippy now.",
        base + 156_000,
        None,
    );
    live.state = TurnState::Streaming;
    live.tool_calls = vec![
        call(
            "t6",
            "Bash",
            json!({"command": "cd bruhs-cli && cargo clippy --all-targets"}),
            Some("warning: unused variable `verbose`\n  --> src/main.rs:22:9\n"),
        ),
        call(
            "t7",
            "Edit",
            json!({
                "file_path": format!("{root}/bruhs-cli/src/main.rs"),
                "old_string": "let verbose = cli.verbose;",
                "new_string": "let _verbose = cli.verbose;"
            }),
            None,
        ),
    ];
    let user = |content: &str, at: i64| {
        let mut m = message(MessageRole::User, content, at, Some(at));
        m.model = None;
        m
    };
    vec![
        user(
            "Add a --json flag to the CLI so `bruhs tasks list` can print machine-readable output, and update the README.",
            base,
        ),
        first,
        user("Now run clippy and fix any warnings.", base + 60_000),
        failed,
        user("Retry clippy.", base + 150_000),
        live,
    ]
}

#[cfg(test)]
mod tests {
    use super::{
        DesktopToolCall, ToolKind, ToolState, changed_files, tool_label, tool_state,
        transcript_fixture_messages, work_summary, working_label,
    };
    use gpui::{Context, Render, TestAppContext, Window, div, prelude::*};
    use serde_json::json;

    #[gpui::test]
    fn motion_preference_replaces_live_animation_without_losing_label(cx: &mut TestAppContext) {
        struct Status;
        impl Render for Status {
            fn render(&mut self, _: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
                div().child(super::shimmer_text(
                    "status",
                    "Thinking — 準備中".into(),
                    cx,
                ))
            }
        }
        let (_, visual) = cx.add_window_view(|_, _| Status);
        visual.run_until_parked();
        assert!(visual.debug_bounds("live-status-static").is_some());
        assert!(visual.debug_bounds("live-status-animated").is_none());
        let static_bounds = visual.debug_bounds("live-status-static").unwrap();

        visual.update(|_, cx| crate::motion::set_reduced_motion(false, cx));
        visual.run_until_parked();
        assert!(visual.debug_bounds("live-status-static").is_none());
        assert_eq!(
            visual.debug_bounds("live-status-animated"),
            Some(static_bounds)
        );

        visual.update(|_, cx| crate::motion::set_reduced_motion(true, cx));
        visual.run_until_parked();
        assert!(visual.debug_bounds("live-status-animated").is_none());
        assert_eq!(
            visual.debug_bounds("live-status-static"),
            Some(static_bounds)
        );
    }

    fn call(tool: &str, input: serde_json::Value, output: Option<&str>) -> DesktopToolCall {
        DesktopToolCall {
            text_offset: None,
            id: "c".into(),
            tool: tool.into(),
            input,
            output: output.map(Into::into),
            is_error: false,
        }
    }

    #[test]
    fn labels_use_tense_and_relative_paths() {
        let c = call("Read", json!({"file_path":"/repo/src/main.rs"}), None);
        assert_eq!(
            tool_label(&c, ToolState::Running, Some("/repo")),
            "Reading src/main.rs"
        );
        assert_eq!(
            tool_label(&c, ToolState::Done, Some("/repo")),
            "Read src/main.rs"
        );
        assert_eq!(
            tool_label(&c, ToolState::Done, Some("/other")),
            "Read /repo/src/main.rs"
        );
        let b = call(
            "Bash",
            json!({"command":"/bin/zsh -lc 'cargo test --all'"}),
            Some(""),
        );
        assert_eq!(tool_label(&b, ToolState::Done, None), "Ran cargo");
        let g = call("Grep", json!({"pattern":"TODO"}), Some(""));
        assert_eq!(tool_label(&g, ToolState::Done, None), "Searched TODO");
        let m = call("mcp__linear__create_issue", json!({}), Some(""));
        assert_eq!(
            tool_label(&m, ToolState::Done, None),
            "Used create_issue (linear)"
        );
    }

    #[test]
    fn tool_state_reflects_output_and_streaming() {
        let done = call("Read", json!({}), Some("x"));
        assert_eq!(tool_state(&done, true), ToolState::Done);
        let pending = call("Read", json!({}), None);
        assert_eq!(tool_state(&pending, true), ToolState::Running);
        assert_eq!(tool_state(&pending, false), ToolState::Interrupted);
    }

    #[test]
    fn work_summary_counts_by_verb() {
        let kinds = [
            ToolKind::Read,
            ToolKind::Read,
            ToolKind::Edit,
            ToolKind::Bash,
        ];
        assert_eq!(
            work_summary(&kinds),
            "Read 2 files, edited 1 file and ran 1 command"
        );
        assert_eq!(work_summary(&kinds[..1]), "Read 1 file");
        assert_eq!(
            work_summary(&[ToolKind::Write, ToolKind::Edit, ToolKind::Web]),
            "Edited 2 files and searched 1 time"
        );
        assert_eq!(work_summary(&[]), "");
    }

    #[test]
    fn fixture_changed_files_match_the_web_card() {
        let root = "/repo";
        let messages = transcript_fixture_messages(root);
        let files = changed_files(&messages[1].tool_calls, Some(root));
        let summary: Vec<(String, bool, usize, usize)> = files
            .iter()
            .map(|f| (f.path.clone(), f.added, f.additions, f.deletions))
            .collect();
        assert_eq!(
            summary,
            vec![
                ("bruhs-cli/src/tasks.rs".to_string(), false, 5, 1),
                ("bruhs-cli/src/json.rs".to_string(), true, 7, 0),
            ]
        );
        // The live turn's Edit has no output yet, so it is not counted.
        assert!(changed_files(&messages[5].tool_calls, Some(root)).is_empty());
    }

    #[test]
    fn working_and_path_helpers_match_web() {
        assert_eq!(working_label(12_400.0), "Working for 12s");
        assert_eq!(working_label(90_000.0), "Working for 1m 30s");
    }
}

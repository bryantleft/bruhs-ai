//! The text view element: a document drawn as blocks of wrapped text, with
//! mouse selection and link clicks.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use gpui::prelude::*;
use gpui::{
    canvas, div, relative, rems, AnyElement, App, Bounds, CursorStyle, Element, ElementId, Entity,
    Font, FontStyle, FontWeight, GlobalElementId, Hsla, InspectorElementId, LayoutId, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, Point, SharedString, StrikethroughStyle,
    StyledText, TextLayout, TextRun, UnderlineStyle, Window,
};

use super::document::{Block, ColumnAlign, Inline, Link};
use super::state::{make_active, TextFormat, TextPosition, TextViewState};
use super::style::TextViewStyle;
use crate::scroll::ScrollableElement as _;
use crate::zoom::px;

type LinkFilter = Arc<dyn Fn(&Link, &App) -> Option<Link> + Send + Sync>;

/// Each segment drawn this frame, with its laid-out text.
/// Each segment drawn this frame: its number, its text's length (an empty
/// segment draws a space, so its layout is longer) and its laid-out text.
type Registry = Rc<RefCell<Vec<(usize, usize, TextLayout)>>>;

const LIST_INDENT_PX: f32 = 18.0;
const LIST_MARKER_GAP_PX: f32 = 5.0;
const TASK_BOX_PX: f32 = 13.0;
const TASK_CHECK_PX: f32 = 10.0;
const QUOTE_BORDER_PX: f32 = 2.0;
const QUOTE_PADDING_PX: f32 = 12.0;
const TABLE_CELL_X_PX: f32 = 10.0;
const TABLE_CELL_Y_PX: f32 = 6.0;
const TABLE_TEXT_PX: f32 = 14.0;
const HEADING_LINE_HEIGHT: f32 = 1.3;
const HEADING_GAP_REM: f32 = 0.75;
const HEADING_BOTTOM_REM: f32 = 0.25;
const BLOCK_GAP_REM: f32 = 0.5;
const ITEM_GAP_REM: f32 = 0.125;
const RULE_GAP_REM: f32 = 0.75;

/// Rich text: Markdown (or HTML) drawn as gpui elements.
///
/// ```ignore
/// TextView::markdown("reply", text).style(prose_style).link(|link, _| Some(link.clone()))
/// ```
///
/// The view keeps its parsed document in a [`TextViewState`] keyed by `id`;
/// a new `text` each frame is parsed again only when it changed.
#[derive(IntoElement)]
pub struct TextView {
    id: ElementId,
    format: TextFormat,
    source: SharedString,
    style: Option<TextViewStyle>,
    link_filter: Option<LinkFilter>,
    selectable: bool,
}

impl TextView {
    pub fn markdown(id: impl Into<ElementId>, text: impl Into<SharedString>) -> Self {
        Self::new(id, TextFormat::Markdown, text)
    }

    pub fn html(id: impl Into<ElementId>, text: impl Into<SharedString>) -> Self {
        Self::new(id, TextFormat::Html, text)
    }

    fn new(id: impl Into<ElementId>, format: TextFormat, text: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            format,
            source: text.into(),
            style: None,
            link_filter: None,
            selectable: true,
        }
    }

    pub fn style(mut self, style: TextViewStyle) -> Self {
        self.style = Some(style);
        self
    }

    /// Decide what each link becomes: the link to draw and open, or `None`
    /// to draw its text plainly.
    pub fn link(
        mut self,
        filter: impl Fn(&Link, &App) -> Option<Link> + Send + Sync + 'static,
    ) -> Self {
        self.link_filter = Some(Arc::new(filter));
        self
    }

    /// Whether the mouse selects its text (on by default).
    pub fn selectable(mut self, selectable: bool) -> Self {
        self.selectable = selectable;
        self
    }
}

/// What every block of one frame is drawn with.
struct Frame {
    style: TextViewStyle,
    /// The body text's size, which headings and task boxes scale from.
    font_size: Pixels,
    base_font: Font,
    base_color: Hsla,
    selection: Option<(TextPosition, TextPosition)>,
    link_filter: Option<LinkFilter>,
    registry: Registry,
}

impl Frame {
    fn link(&self, link: &Link, cx: &App) -> Option<Link> {
        match &self.link_filter {
            Some(filter) => filter(link, cx),
            None => is_safe_link(&link.url).then(|| link.clone()),
        }
    }

    /// The byte range of `segment` the selection covers.
    fn selected_in(&self, segment: usize, len: usize) -> Option<(usize, usize)> {
        let (start, end) = self.selection?;
        if segment < start.segment || segment > end.segment {
            return None;
        }
        let from = if segment == start.segment {
            start.offset
        } else {
            0
        };
        let to = if segment == end.segment {
            end.offset
        } else {
            len
        };
        (from < to).then_some((from, to))
    }
}

impl RenderOnce for TextView {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let format = self.format;
        let source = self.source.clone();
        let state = window.use_keyed_state(self.id.clone(), cx, |_, cx| match format {
            TextFormat::Markdown => TextViewState::markdown(&source, cx),
            TextFormat::Html => TextViewState::html(&source, cx),
        });
        state.update(cx, |state, _| state.replace(&self.source));

        let text_style = window.text_style();
        let frame = Frame {
            style: self.style.unwrap_or_default(),
            font_size: text_style.font_size.to_pixels(window.rem_size()),
            base_font: text_style.font(),
            base_color: text_style.color,
            selection: state.read(cx).selected_range(),
            link_filter: self.link_filter,
            registry: Rc::new(RefCell::new(Vec::new())),
        };
        let document = state.read(cx).document().clone();
        let blocks = render_blocks(&document.blocks, &frame, None, true, cx);
        let registry = frame.registry.clone();
        let link_filter = frame.link_filter.clone();

        div()
            .id(self.id)
            .w_full()
            .min_w_0()
            .flex()
            .flex_col()
            .line_height(relative(frame.style.line_height))
            .children(blocks)
            .when(self.selectable, |el| {
                let down_state = state.clone();
                let down_registry = registry.clone();
                el.cursor(CursorStyle::IBeam)
                    .on_mouse_down(MouseButton::Left, move |event: &MouseDownEvent, _, cx| {
                        let Some(at) = hit(&down_registry, event.position, false) else {
                            return;
                        };
                        let document = down_state.read(cx).document().clone();
                        let (anchor, head) = match event.click_count {
                            2 => word_at(&document.segments, at),
                            n if n >= 3 => segment_at(&document.segments, at),
                            _ => (at, at),
                        };
                        down_state.update(cx, |state, cx| {
                            state.selection = Some((anchor, head));
                            state.selecting = true;
                            cx.notify();
                        });
                        make_active(&down_state, cx);
                    })
                    .child(
                        canvas(
                            |_, _, _| {},
                            move |_, _, window, _| {
                                let (move_state, move_registry) = (state.clone(), registry.clone());
                                window.on_mouse_event(
                                    move |event: &MouseMoveEvent, phase, _, cx| {
                                        if !phase.bubble() || !move_state.read(cx).selecting {
                                            return;
                                        }
                                        // The button came up where this view could not
                                        // hear it (it left the screen mid-drag).
                                        if event.pressed_button != Some(MouseButton::Left) {
                                            move_state
                                                .update(cx, |state, _| state.selecting = false);
                                            return;
                                        }
                                        let Some(head) = hit(&move_registry, event.position, true)
                                        else {
                                            return;
                                        };
                                        move_state.update(cx, |state, cx| {
                                            if let Some((anchor, old)) = state.selection {
                                                if old != head {
                                                    state.selection = Some((anchor, head));
                                                    cx.notify();
                                                }
                                            }
                                        });
                                    },
                                );
                                let (up_state, up_registry) = (state.clone(), registry.clone());
                                window.on_mouse_event(
                                    move |event: &MouseUpEvent, phase, window, cx| {
                                        if !phase.bubble()
                                            || event.button != MouseButton::Left
                                            || !up_state.read(cx).selecting
                                        {
                                            return;
                                        }
                                        let click = up_state.update(cx, |state, cx| {
                                            state.selecting = false;
                                            let click = state
                                                .selection
                                                .is_some_and(|(anchor, head)| anchor == head);
                                            if click {
                                                state.selection = None;
                                            }
                                            cx.notify();
                                            click
                                        });
                                        if click {
                                            open_link(
                                                &up_state,
                                                &up_registry,
                                                link_filter.as_ref(),
                                                event.position,
                                                window,
                                                cx,
                                            );
                                        }
                                    },
                                );
                            },
                        )
                        .absolute()
                        .size_0(),
                    )
            })
    }
}

/// A plain click on a link opens it, if the view's filter allows it.
fn open_link(
    state: &Entity<TextViewState>,
    registry: &Registry,
    filter: Option<&LinkFilter>,
    at: Point<Pixels>,
    window: &mut Window,
    cx: &mut App,
) {
    let Some(pos) = hit(registry, at, false) else {
        return;
    };
    let link = find_inline(&state.read(cx).document().blocks, pos.segment)
        .and_then(|inline| inline.link_at(pos.offset).cloned());
    // The filter decides what a link opens, or that it opens nothing.
    // Without one, only web and mail links open: a reply or a document can
    // carry a `file:` or another app's link.
    let link = match (link, filter) {
        (Some(link), Some(filter)) => filter(&link, cx),
        (Some(link), None) => is_safe_link(&link.url).then_some(link),
        (None, _) => None,
    };
    if let Some(link) = link {
        if link.url.starts_with("https://") || link.url.starts_with("http://") {
            let action = super::OpenWebLink(link.url.to_string());
            if window.is_action_available(&action, cx) {
                window.dispatch_action(Box::new(action), cx);
                return;
            }
        }
        cx.open_url(&link.url);
    }
}

/// A link a view opens without its own filter: http, https or mailto.
fn is_safe_link(url: &str) -> bool {
    let lower = url.trim().to_ascii_lowercase();
    ["http://", "https://", "mailto:"]
        .iter()
        .any(|scheme| lower.starts_with(scheme))
}

fn find_inline(blocks: &[Block], segment: usize) -> Option<&Inline> {
    blocks.iter().find_map(|block| match block {
        Block::Paragraph(inline) | Block::Heading(_, inline) => {
            (inline.segment == segment).then_some(inline)
        }
        Block::CodeBlock { code, .. } => (code.segment == segment).then_some(code),
        Block::List { items, .. } => items
            .iter()
            .find_map(|item| find_inline(&item.blocks, segment)),
        Block::Quote(blocks) => find_inline(blocks, segment),
        Block::Table { head, rows, .. } => head
            .iter()
            .chain(rows.iter().flatten())
            .find(|cell| cell.segment == segment),
        Block::Rule => None,
    })
}

/// The text position under `point`: in the segment it is over, or with
/// `nearest`, the closest drawn segment.
fn hit(registry: &Registry, point: Point<Pixels>, nearest: bool) -> Option<TextPosition> {
    let drawn = registry.borrow();
    let within = drawn
        .iter()
        .find(|(_, _, layout)| layout.bounds().contains(&point));
    let (segment, len, layout) = match within {
        Some(found) => found,
        None if nearest => drawn.iter().min_by(|(_, _, a), (_, _, b)| {
            distance(a.bounds(), point).total_cmp(&distance(b.bounds(), point))
        })?,
        None => return None,
    };
    let bounds = layout.bounds();
    // Clamp into the segment so a drag past its edge selects to its end.
    let clamped = Point {
        x: point.x.max(bounds.left()).min(bounds.right()),
        y: point.y.min(bounds.bottom() - px(1.0)).max(bounds.top()),
    };
    let offset = match layout.index_for_position(clamped) {
        Ok(ix) | Err(ix) => ix,
    };
    Some(TextPosition {
        segment: *segment,
        offset: offset.min(*len),
    })
}

fn distance(bounds: Bounds<Pixels>, point: Point<Pixels>) -> f32 {
    let dx = (bounds.left() - point.x)
        .max(point.x - bounds.right())
        .max(Pixels::ZERO);
    let dy = (bounds.top() - point.y)
        .max(point.y - bounds.bottom())
        .max(Pixels::ZERO);
    let (dx, dy) = (f32::from(dx), f32::from(dy));
    // Rows matter more than columns: a pointer beside a line selects in it.
    dy * 1000.0 + dx
}

fn word_at(segments: &[String], at: TextPosition) -> (TextPosition, TextPosition) {
    let Some(text) = segments.get(at.segment) else {
        return (at, at);
    };
    let mut offset = at.offset.min(text.len());
    while !text.is_char_boundary(offset) {
        offset -= 1;
    }
    let is_word = |c: char| c.is_alphanumeric() || c == '_';
    let start = text[..offset]
        .char_indices()
        .rev()
        .take_while(|(_, c)| is_word(*c))
        .last()
        .map_or(offset, |(ix, _)| ix);
    let end = text[offset..]
        .char_indices()
        .find(|(_, c)| !is_word(*c))
        .map_or(text.len(), |(ix, _)| offset + ix);
    (
        TextPosition {
            segment: at.segment,
            offset: start,
        },
        TextPosition {
            segment: at.segment,
            offset: end,
        },
    )
}

fn segment_at(segments: &[String], at: TextPosition) -> (TextPosition, TextPosition) {
    let len = segments.get(at.segment).map_or(0, String::len);
    (
        TextPosition {
            segment: at.segment,
            offset: 0,
        },
        TextPosition {
            segment: at.segment,
            offset: len,
        },
    )
}

/// A block's own top margin (in rems), which the web gives headings, code,
/// quotes, tables and rules wherever they are, and the margin it leaves
/// below itself.
fn own_margins(block: &Block) -> (Option<f32>, f32) {
    match block {
        Block::Heading(..) => (Some(HEADING_GAP_REM), HEADING_BOTTOM_REM),
        Block::CodeBlock { .. } | Block::Quote(_) | Block::Table { .. } => {
            (Some(BLOCK_GAP_REM), 0.0)
        }
        Block::Rule => (Some(RULE_GAP_REM), RULE_GAP_REM),
        Block::Paragraph(_) | Block::List { .. } => (None, 0.0),
    }
}

/// `top_level` blocks are the view's own children: the web spaces those
/// by the paragraph gap (`.prose-msg > * + *`); blocks nested in a list
/// item or a quote touch, as `p` and `ul` do there.
fn render_blocks(
    blocks: &[Block],
    frame: &Frame,
    color: Option<Hsla>,
    top_level: bool,
    cx: &App,
) -> Vec<AnyElement> {
    let mut below_previous = 0.0_f32;
    blocks
        .iter()
        .enumerate()
        .map(|(ix, block)| {
            let paragraph_gap = frame.style.paragraph_gap.0;
            let (own_top, own_bottom) = own_margins(block);
            let generic = if top_level && ix > 0 {
                paragraph_gap
            } else {
                0.0
            };
            // Adjacent margins collapse to the larger, as in CSS.
            let top = own_top.unwrap_or(generic).max(below_previous);
            below_previous = own_bottom;
            let gap = |el: gpui::Div, _rem: f32| if top > 0.0 { el.mt(rems(top)) } else { el };
            match block {
                Block::Paragraph(inline) => gap(div(), paragraph_gap)
                    .child(segment(inline, frame, color, None, cx))
                    .into_any_element(),
                Block::Heading(level, inline) => {
                    let size = frame.style.heading_size(*level, frame.font_size);
                    gap(div(), HEADING_GAP_REM)
                        .text_size(size)
                        .line_height(relative(HEADING_LINE_HEIGHT))
                        .child(segment(
                            inline,
                            frame,
                            Some(frame.style.heading_color),
                            Some(FontWeight::SEMIBOLD),
                            cx,
                        ))
                        .into_any_element()
                }
                Block::CodeBlock { code, .. } => {
                    let refinement = frame.style.code_block.clone();
                    let code_color = refinement.text.color;
                    let code_family = refinement.text.font_family.clone();
                    let mut font = frame.base_font.clone();
                    font.family =
                        code_family.unwrap_or_else(|| frame.style.code_font_family.clone());
                    // Like the web's `pre`: the box (background, border,
                    // corners) holds still and its lines scroll sideways
                    // inside it without wrapping, padding and all.
                    let mut refinement = refinement;
                    let padding = std::mem::take(&mut refinement.padding);
                    let mut block = gap(div(), BLOCK_GAP_REM).w_full();
                    block.style().refine(&refinement);
                    let wrap = crate::theme::wrap_code();
                    let mut lines = div().when(!wrap, |el| el.whitespace_nowrap());
                    lines.style().padding = padding;
                    let lines = lines.child(code_segment(code, frame, font, code_color, cx));
                    if wrap {
                        block.child(lines).into_any_element()
                    } else {
                        block
                            .child(div().w_full().overflow_x_scrollbar().child(lines))
                            .into_any_element()
                    }
                }
                Block::List { start, items } => {
                    gap(div(), paragraph_gap)
                        .flex()
                        .flex_col()
                        .children(items.iter().enumerate().map(|(ix, item)| {
                            let marker = match (item.task, start) {
                                (Some(checked), _) => task_box(checked, frame),
                                (None, Some(start)) => {
                                    let ix = u64::try_from(ix).unwrap_or(u64::MAX);
                                    marker_text(format!("{}.", start.saturating_add(ix)), frame)
                                }
                                (None, None) => marker_text("•", frame),
                            };
                            // `li { margin-top: 0.125rem }`, the first one too.
                            div()
                                .mt(rems(ITEM_GAP_REM))
                                .flex()
                                .flex_row()
                                .child(
                                    // The marker sits in the 18px indent, ending
                                    // a little before the text, as `outside`
                                    // markers do; a long number overhangs left.
                                    div()
                                        .flex_none()
                                        .w(px(LIST_INDENT_PX))
                                        .pr(px(LIST_MARKER_GAP_PX))
                                        .flex()
                                        .justify_end()
                                        .child(marker),
                                )
                                .child(
                                    div().flex_1().min_w_0().flex().flex_col().children(
                                        render_blocks(&item.blocks, frame, color, false, cx),
                                    ),
                                )
                        }))
                        .into_any_element()
                }
                Block::Quote(blocks) => gap(div(), BLOCK_GAP_REM)
                    .flex()
                    .flex_col()
                    .border_l(px(QUOTE_BORDER_PX))
                    .border_color(frame.style.border_color)
                    .pl(px(QUOTE_PADDING_PX))
                    .children(render_blocks(
                        blocks,
                        frame,
                        Some(frame.style.quote_color),
                        false,
                        cx,
                    ))
                    .into_any_element(),
                Block::Table { aligns, head, rows } => {
                    let row = |cells: &[Inline], head: bool, last: bool| {
                        div()
                            .flex()
                            .flex_row()
                            .w_full()
                            .when(!last, |el| {
                                el.border_b_1().border_color(if head {
                                    frame.style.border_color
                                } else {
                                    frame.style.border_color.opacity(0.4)
                                })
                            })
                            .children(cells.iter().enumerate().map(|(ix, cell)| {
                                let align = aligns.get(ix).copied().unwrap_or(ColumnAlign::Start);
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .px(px(TABLE_CELL_X_PX))
                                    .py(px(TABLE_CELL_Y_PX))
                                    .map(|el| match align {
                                        ColumnAlign::Start => el,
                                        ColumnAlign::Center => el.text_center(),
                                        ColumnAlign::End => el.text_right(),
                                    })
                                    .child(segment(
                                        cell,
                                        frame,
                                        Some(if head {
                                            frame.style.table_head_color
                                        } else {
                                            frame.style.table_text_color
                                        }),
                                        head.then_some(FontWeight::MEDIUM),
                                        cx,
                                    ))
                            }))
                    };
                    let count = rows.len();
                    gap(div(), BLOCK_GAP_REM)
                        .w_full()
                        .flex()
                        .flex_col()
                        .text_size(px(TABLE_TEXT_PX))
                        .child(row(head, true, false))
                        .children(
                            rows.iter()
                                .enumerate()
                                .map(|(ix, cells)| row(cells, false, ix + 1 == count)),
                        )
                        .into_any_element()
                }
                Block::Rule => gap(div(), RULE_GAP_REM)
                    .h(px(1.0))
                    .w_full()
                    .bg(frame.style.border_color)
                    .into_any_element(),
            }
        })
        .collect()
}

fn marker_text(text: impl Into<SharedString>, frame: &Frame) -> AnyElement {
    div()
        .flex_none()
        .whitespace_nowrap()
        .text_color(frame.style.muted_color)
        .child(text.into())
        .into_any_element()
}

/// A task item's box: the web's disabled checkbox, in the bruhs tones.
fn task_box(checked: bool, frame: &Frame) -> AnyElement {
    // Centred on the first line of the item's text.
    div()
        .flex_none()
        .h(frame.font_size * frame.style.line_height)
        .flex()
        .items_center()
        .child(
            div()
                .size(px(TASK_BOX_PX))
                .rounded(px(3.0))
                .flex()
                .items_center()
                .justify_center()
                .map(|el| {
                    if checked {
                        el.bg(crate::theme::persimmon_500()).child(
                            gpui::svg()
                                .path(crate::icons::CHECK)
                                .size(px(TASK_CHECK_PX))
                                .text_color(crate::theme::longan_950()),
                        )
                    } else {
                        el.bg(gpui::white().opacity(0.05))
                            .border_1()
                            .border_color(gpui::white().opacity(0.15))
                    }
                }),
        )
        .into_any_element()
}

/// One segment of inline text, styled span by span.
fn segment(
    inline: &Inline,
    frame: &Frame,
    color: Option<Hsla>,
    weight: Option<FontWeight>,
    cx: &App,
) -> AnyElement {
    let base_color = color.unwrap_or(frame.base_color);
    let mut font = frame.base_font.clone();
    if let Some(weight) = weight {
        font.weight = weight;
    }
    let mut runs = Vec::new();
    for (range, style) in &inline.spans {
        let mut run_font = font.clone();
        let mut run_color = base_color;
        let mut background = None;
        let mut underline = None;
        let mut strikethrough = None;
        if style.strong {
            run_font.weight = FontWeight::SEMIBOLD;
            run_color = frame.style.strong_color;
        }
        if style.emphasis {
            run_font.style = FontStyle::Italic;
        }
        if style.strikethrough {
            run_color = frame.style.muted_color;
            strikethrough = Some(StrikethroughStyle {
                thickness: px(1.0),
                color: Some(run_color),
            });
        }
        if style.code {
            run_font.family = frame.style.code_font_family.clone();
            run_color = frame.style.inline_code_color;
            background = Some(frame.style.inline_code_background);
        }
        if let Some(link) = &style.link {
            if frame.link(link, cx).is_some() {
                run_color = frame.style.link_color;
                underline = Some(UnderlineStyle {
                    thickness: px(1.0),
                    color: Some(run_color),
                    wavy: false,
                });
            }
        }
        runs.push(TextRun {
            len: range.len(),
            font: run_font,
            color: run_color,
            background_color: background,
            underline,
            strikethrough,
        });
    }
    SegmentText::new(inline, runs, frame).into_any_element()
}

fn code_segment(
    code: &Inline,
    frame: &Frame,
    font: Font,
    color: Option<Hsla>,
    _cx: &App,
) -> AnyElement {
    let runs = vec![TextRun {
        len: code.text.len(),
        font,
        color: color.unwrap_or(frame.base_color),
        background_color: None,
        underline: None,
        strikethrough: None,
    }];
    SegmentText::new(code, runs, frame).into_any_element()
}

/// Laid-out text for one segment, drawn with the selection's highlight. It
/// records its layout as it prepaints, so the mouse handlers hit-test only
/// text drawn in the latest frame.
struct SegmentText {
    segment: usize,
    len: usize,
    text: StyledText,
    registry: Registry,
}

impl SegmentText {
    fn new(inline: &Inline, runs: Vec<TextRun>, frame: &Frame) -> Self {
        let runs = match frame.selected_in(inline.segment, inline.text.len()) {
            Some((from, to)) => highlight(runs, from, to, frame.style.selection_color),
            None => runs,
        };
        // An empty segment still takes a line, like an empty paragraph.
        let text = if inline.text.is_empty() {
            StyledText::new(" ")
        } else {
            StyledText::new(inline.text.clone()).with_runs(runs)
        };
        Self {
            segment: inline.segment,
            len: inline.text.len(),
            text,
            registry: frame.registry.clone(),
        }
    }
}

/// Split `runs` at `from`/`to` and give the part between a background.
fn highlight(runs: Vec<TextRun>, from: usize, to: usize, color: Hsla) -> Vec<TextRun> {
    let mut out = Vec::with_capacity(runs.len() + 2);
    let mut at = 0;
    for run in runs {
        let (start, end) = (at, at + run.len);
        at = end;
        let cuts = [start, from.clamp(start, end), to.clamp(start, end), end];
        for pair in cuts.windows(2) {
            let (a, b) = (pair[0], pair[1]);
            if a == b {
                continue;
            }
            let mut piece = run.clone();
            piece.len = b - a;
            if a >= from && b <= to {
                piece.background_color = Some(color);
            }
            out.push(piece);
        }
    }
    out
}

impl IntoElement for SegmentText {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}

impl Element for SegmentText {
    type RequestLayoutState = <StyledText as Element>::RequestLayoutState;
    type PrepaintState = <StyledText as Element>::PrepaintState;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        self.text.request_layout(id, inspector, window, cx)
    }

    fn prepaint(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        state: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        // The layout handle is shared: it holds this frame's lines once the
        // text below has prepainted.
        self.registry
            .borrow_mut()
            .push((self.segment, self.len, self.text.layout().clone()));
        self.text.prepaint(id, inspector, bounds, state, window, cx)
    }

    fn paint(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        state: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        self.text
            .paint(id, inspector, bounds, state, prepaint, window, cx);
    }
}

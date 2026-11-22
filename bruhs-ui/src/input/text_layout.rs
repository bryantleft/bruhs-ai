//! A field normally uses one GPUI text layout. For rich lists the marker and
//! body are separate elements so wrapping, hit testing and IME share a gutter.
use std::{ops::Range, rc::Rc};

use gpui::{
    div, AnyElement, App, Bounds, Element, IntoElement, LayoutId, ParentElement, Pixels, Point,
    Styled, StyledText, TextLayout, TextRun, Window,
};

use super::{list_layout::Gutter, state::DisplayMap};

struct Piece {
    start: usize,
    layout: TextLayout,
}
pub(super) struct Row {
    marker: Option<Piece>,
    body: Piece,
}
#[derive(Clone)]
pub(super) enum FieldLayout {
    Flow(TextLayout),
    Rows(Rc<[Row]>),
}

impl FieldLayout {
    pub fn position_for_index(&self, index: usize) -> Option<Point<Pixels>> {
        match self {
            Self::Flow(text) => text.position_for_index(index),
            Self::Rows(rows) => {
                let i = rows
                    .partition_point(|row| row.start() <= index)
                    .saturating_sub(1);
                let row = &rows[i];
                let piece = row
                    .marker
                    .as_ref()
                    .filter(|_| index < row.body.start)
                    .unwrap_or(&row.body);
                let local = index - piece.start;
                let point = piece.layout.position_for_index(local)?;
                // A soft break has two caret positions. Use the start of the
                // continuation when that is the index hit at its left edge.
                let height = piece.layout.line_height();
                let bounds = piece.layout.bounds();
                let next = gpui::point(bounds.left(), point.y + height);
                if next.y < bounds.bottom() {
                    let hit = piece
                        .layout
                        .index_for_position(gpui::point(next.x, next.y + height / 2.));
                    if matches!(hit, Ok(i) | Err(i) if i == local) {
                        return Some(next);
                    }
                }
                Some(point)
            }
        }
    }
    pub fn index_for_position(&self, at: Point<Pixels>) -> Result<usize, usize> {
        match self {
            Self::Flow(text) => text.index_for_position(at),
            Self::Rows(rows) => {
                let i = rows
                    .partition_point(|row| row.body.layout.bounds().bottom() <= at.y)
                    .min(rows.len() - 1);
                let row = &rows[i];
                let bounds = row.body.layout.bounds();
                let piece = row
                    .marker
                    .as_ref()
                    .filter(|_| {
                        at.x < bounds.left() && at.y < bounds.top() + row.body.layout.line_height()
                    })
                    .unwrap_or(&row.body);
                // A click in the hanging margin belongs to the start of that visual line.
                let at = Point {
                    x: at.x.max(piece.layout.bounds().left()),
                    ..at
                };
                piece
                    .layout
                    .index_for_position(at)
                    .map(|i| piece.start + i)
                    .map_err(|i| piece.start + i)
            }
        }
    }
    pub fn line_height(&self) -> Pixels {
        match self {
            Self::Flow(text) => text.line_height(),
            Self::Rows(rows) => rows[0].body.layout.line_height(),
        }
    }
    #[cfg(test)]
    pub fn logical_line_count(&self) -> usize {
        match self {
            Self::Flow(text) => text.line_layouts().len(),
            Self::Rows(rows) => rows
                .iter()
                .map(|r| r.body.layout.line_layouts().len())
                .sum(),
        }
    }
}
impl Row {
    fn start(&self) -> usize {
        self.marker.as_ref().unwrap_or(&self.body).start
    }
}

pub(super) enum EditorText {
    Flow(StyledText),
    Lists(AnyElement, FieldLayout),
}

impl EditorText {
    pub fn new(
        display: String,
        runs: Vec<TextRun>,
        map: &DisplayMap,
        wrap: bool,
        font: &gpui::Font,
        font_size: Pixels,
        window: &Window,
    ) -> Self {
        let Some(lists) = map
            .projection
            .as_ref()
            .map(|p| &p.list_lines)
            .filter(|v| wrap && !v.is_empty())
        else {
            return Self::Flow(StyledText::new(display).with_runs(runs));
        };
        let rem = window.rem_size();
        let system = window.text_system();
        let ch = system
            .advance(system.resolve_font(font), font_size, '0')
            .map_or(font_size * 0.6, |size| size.width);
        let mut tree = div().flex().flex_col().w_full().min_w_0();
        let mut rows = Vec::new();
        let mut styled = Pieces::new(&display, &runs);
        let mut pos = 0;
        for line in lists {
            let start = map.to_display(line.start);
            let body = map.to_display(line.body);
            if start < pos {
                continue;
            }
            if start > pos {
                let before = &display[pos..start];
                let end = start - usize::from(before.ends_with('\n'));
                let (text, piece) = styled.get(pos..end);
                tree = tree.child(text);
                rows.push(Row {
                    marker: None,
                    body: piece,
                });
            }
            let end = display[body..]
                .find('\n')
                .map_or(display.len(), |n| body + n);
            let mut inside_task = false;
            let width: Pixels = line
                .gutters
                .iter()
                .map(|gutter| match gutter {
                    Gutter::Bullet => rem * 1.25,
                    Gutter::Ordered(chars) => (ch * (*chars + 1) as f32).max(rem * 1.25),
                    Gutter::Task => {
                        // T3 adds list padding inside an ancestor task list, in
                        // addition to the checkbox and gap on each item.
                        let padding = if inside_task {
                            rem * 1.375
                        } else {
                            Pixels::ZERO
                        };
                        inside_task = true;
                        rem * 1.45 + padding
                    }
                })
                .sum();
            let mut gutter = div()
                .w(width)
                .max_w(gpui::relative(0.85))
                .flex_shrink_0()
                .flex()
                .justify_end()
                .whitespace_nowrap()
                .overflow_hidden();
            let marker = if start < body {
                let (text, piece) = styled.get(start..body);
                gutter = gutter.child(text);
                Some(piece)
            } else {
                None
            };
            let (text, piece) = styled.get(body..end);
            tree = tree.child(
                div()
                    .flex()
                    .w_full()
                    .min_w_0()
                    .items_start()
                    .child(gutter)
                    .child(div().flex_1().min_w_0().child(text)),
            );
            rows.push(Row {
                marker,
                body: piece,
            });
            pos = end + 1;
        }
        if pos <= display.len() {
            let (text, piece) = styled.get(pos..display.len());
            tree = tree.child(text);
            rows.push(Row {
                marker: None,
                body: piece,
            });
        }
        Self::Lists(tree.into_any_element(), FieldLayout::Rows(rows.into()))
    }
    pub fn layout(&self) -> FieldLayout {
        match self {
            Self::Flow(t) => FieldLayout::Flow(t.layout().clone()),
            Self::Lists(_, l) => l.clone(),
        }
    }
    pub fn request_layout(&mut self, window: &mut Window, cx: &mut App) -> LayoutId {
        match self {
            Self::Flow(t) => t.request_layout(None, None, window, cx).0,
            Self::Lists(tree, _) => tree.request_layout(window, cx),
        }
    }
    pub fn prepaint(&mut self, bounds: Bounds<Pixels>, window: &mut Window, cx: &mut App) {
        match self {
            Self::Flow(t) => t.prepaint(None, None, bounds, &mut (), window, cx),
            Self::Lists(tree, _) => {
                tree.prepaint(window, cx);
            }
        }
    }
    pub fn paint(&mut self, bounds: Bounds<Pixels>, window: &mut Window, cx: &mut App) {
        match self {
            Self::Flow(t) => t.paint(None, None, bounds, &mut (), &mut (), window, cx),
            Self::Lists(tree, _) => tree.paint(window, cx),
        }
    }
}

// Split the existing styled runs in display order, including selection and IME
// decoration. The newline between two layout rows still owns its source byte.
struct Pieces<'a> {
    display: &'a str,
    runs: &'a [TextRun],
    start: usize,
}
impl<'a> Pieces<'a> {
    fn new(display: &'a str, runs: &'a [TextRun]) -> Self {
        Self {
            display,
            runs,
            start: 0,
        }
    }
    fn get(&mut self, range: Range<usize>) -> (StyledText, Piece) {
        let mut runs = Vec::new();
        while let Some(run) = self.runs.first() {
            let end = self.start + run.len;
            let from = self.start.max(range.start);
            let to = end.min(range.end);
            if to > from {
                runs.push(TextRun {
                    len: to - from,
                    ..run.clone()
                });
            }
            if end > range.end {
                break;
            }
            self.start = end;
            self.runs = &self.runs[1..];
        }
        let text = StyledText::new(self.display[range.clone()].to_owned()).with_runs(runs);
        let piece = Piece {
            start: range.start,
            layout: text.layout().clone(),
        };
        (text, piece)
    }
}

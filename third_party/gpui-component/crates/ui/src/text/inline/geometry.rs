//! Visit visible characters without searching every preceding line per caret.
use gpui::{Bounds, Half as _, Pixels, Point, TextLayout, WrappedLineLayout, point, px};
use std::sync::Arc;

type CharacterVisitor<'a> = dyn FnMut(usize, usize, Point<Pixels>, Pixels) + 'a;

pub(super) fn visit_visible_characters(
    text: &str,
    layout: &TextLayout,
    line_height: Pixels,
    mask: Bounds<Pixels>,
    mut visit: impl FnMut(usize, usize, Point<Pixels>, Pixels),
) {
    let lines = layout.line_layouts();
    let origin = layout.bounds().origin;
    if !visit_lines(
        text,
        &lines,
        origin,
        layout.line_height(),
        line_height,
        mask,
        &mut visit,
    ) {
        // Truncated/alternate representations retain their previous source-index
        // geometry. The fast path requires matching UTF-8 line boundaries.
        visit_full(
            text,
            &lines,
            origin,
            layout.line_height(),
            line_height,
            &mut visit,
        );
    }
}

fn visit_lines(
    text: &str,
    lines: &[Arc<WrappedLineLayout>],
    mut origin: Point<Pixels>,
    layout_height: Pixels,
    line_height: Pixels,
    mask: Bounds<Pixels>,
    visit: &mut CharacterVisitor<'_>,
) -> bool {
    if layout_height <= px(0.) {
        return false;
    }
    let mut start = 0usize;
    for line in lines {
        let Some(end) = start.checked_add(line.len()) else {
            return false;
        };
        if text.get(start..end).is_none() {
            return false;
        }
        start = end.saturating_add(1);
    }
    if start.saturating_sub(1) != text.len() {
        return false;
    }

    let mut start = 0;
    for line in lines {
        let size = line.size(layout_height);
        let end = start + line.len();
        let through_newline = end.saturating_add(1).min(text.len());
        if origin.y <= mask.bottom() && origin.y + size.height + line_height >= mask.top() {
            // Include boundary carets: a wrap's first byte can map to the end
            // of its preceding visual line in GPUI's position_for_index.
            let last_center = (size.height - layout_height.half()).max(px(0.));
            let top = (mask.top() - origin.y - line_height - layout_height)
                .max(px(0.))
                .min(last_center);
            let bottom = (mask.bottom() - origin.y + line_height + layout_height)
                .max(px(0.))
                .min(last_center);
            let first = line
                .index_for_position(point(Pixels::MIN, top), layout_height)
                .unwrap_or_else(|index| index);
            let last = line
                .index_for_position(point(Pixels::MAX, bottom), layout_height)
                .unwrap_or_else(|index| index);
            // Line spans were validated before invoking any callbacks. Filtering
            // byte offsets also handles glyph clusters without slicing them.
            for (offset, c) in text[start..through_newline].char_indices() {
                if offset < first {
                    continue;
                }
                if offset > last {
                    break;
                }
                let next = offset + c.len_utf8();
                let Some(position) = line.position_for_index(offset, layout_height) else {
                    continue;
                };
                let position = origin + position;
                let next_position = line
                    .position_for_index(next, layout_height)
                    .map(|p| origin + p);
                let width = next_position
                    .filter(|next| next.y == position.y)
                    .map_or(line_height.half(), |next| next.x - position.x);
                visit(start + offset, start + next, position, width);
            }
        }
        start = end.saturating_add(1);
        origin.y += size.height;
    }
    true
}

fn position_for_index(
    lines: &[Arc<WrappedLineLayout>],
    mut origin: Point<Pixels>,
    height: Pixels,
    index: usize,
) -> Option<Point<Pixels>> {
    let mut start = 0;
    for line in lines {
        let end = start + line.len();
        if index < start {
            break;
        }
        if index <= end {
            return line
                .position_for_index(index - start, height)
                .map(|p| origin + p);
        }
        origin.y += line.size(height).height;
        start = end.saturating_add(1);
    }
    None
}

fn visit_full(
    text: &str,
    lines: &[Arc<WrappedLineLayout>],
    origin: Point<Pixels>,
    layout_height: Pixels,
    line_height: Pixels,
    visit: &mut CharacterVisitor<'_>,
) {
    for (offset, c) in text.char_indices() {
        let next = offset + c.len_utf8();
        let Some(position) = position_for_index(lines, origin, layout_height, offset) else {
            continue;
        };
        let width = position_for_index(lines, origin, layout_height, next)
            .filter(|next| next.y == position.y)
            .map_or(line_height.half(), |next| next.x - position.x);
        visit(offset, next, position, width);
    }
}

/// Compare clipped caret geometry and selection centers against a complete scan.
#[cfg(feature = "test-support")]
pub fn inline_geometry_matches_full_scan(
    text: &str,
    lines: &[Arc<WrappedLineLayout>],
    origin: Point<Pixels>,
    layout_height: Pixels,
    line_height: Pixels,
    mask: Bounds<Pixels>,
) -> bool {
    let mut expected = Vec::new();
    let mut actual = Vec::new();
    let visible = |position: Point<Pixels>, width: Pixels| {
        let rect = Bounds::from_corners(
            position,
            point(position.x + width, position.y + line_height),
        )
        .intersect(&mask);
        (rect.size.width > px(0.) && rect.size.height > px(0.))
            || mask.contains(&point(
                position.x + width.half(),
                position.y + line_height.half(),
            ))
    };
    visit_full(
        text,
        lines,
        origin,
        layout_height,
        line_height,
        &mut |at, next, p, w| {
            if visible(p, w) {
                expected.push((at, next, p, w));
            }
        },
    );
    let used = visit_lines(
        text,
        lines,
        origin,
        layout_height,
        line_height,
        mask,
        &mut |at, next, p, w| {
            if visible(p, w) {
                actual.push((at, next, p, w));
            }
        },
    );
    if !used || expected != actual {
        let differing = expected
            .iter()
            .zip(&actual)
            .position(|(a, b)| a != b)
            .unwrap_or(expected.len().min(actual.len()));
        eprintln!(
            "inline geometry mismatch: fast={used}, counts={}/{}, index={differing}, expected={:?}, actual={:?}",
            expected.len(),
            actual.len(),
            expected.get(differing),
            actual.get(differing)
        );
    }
    used && expected == actual
}

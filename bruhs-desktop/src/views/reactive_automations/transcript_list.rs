//! A crew transcript (a room's, a bruh's chat) as a virtual list, as the
//! thread chat draws its messages: only the rows on screen are laid out, so
//! a long history with Markdown costs little on each frame a working face
//! redraws. Row 0 holds what sits above the lines (Load earlier, the error,
//! the empty line); the lines follow it.
use super::bruh_page::{Layout, PAGE_PADDING_PX, TRANSCRIPT_GAP_PX};
use super::*;
use bruhs_ui::scroll::ScrollableElement;
use std::ops::Range;

/// Rows laid out past each edge of the view, as in the thread chat.
const OVERDRAW_PX: f32 = 128.0;

/// The rows a transcript shows, by key, and the list that lays them out.
pub(super) struct TranscriptList {
    pub(super) state: ListState,
    /// What row 0 holds.
    header: String,
    /// The lines' keys, from row 1 on.
    keys: Vec<String>,
}

impl TranscriptList {
    pub(super) fn new() -> Self {
        Self {
            state: ListState::new(1, ListAlignment::Top, gpui::px(OVERDRAW_PX)),
            header: String::new(),
            keys: Vec::new(),
        }
    }

    /// Row 0 now holds `key`'s content: measure it again when it changed.
    pub(super) fn sync_header(&mut self, key: String) {
        if key != self.header {
            self.state.splice(0..1, 1);
            self.header = key;
        }
    }

    /// Show lines with `keys` in place of those shown, replacing only the
    /// rows between the ones both have in common at either end. Lines that
    /// load above keep the reader's place; lines that load below do not
    /// move it.
    pub(super) fn sync(&mut self, keys: Vec<String>) {
        if let Some((range, count)) = changed(&self.keys, &keys) {
            self.state.splice(range.start + 1..range.end + 1, count);
        }
        self.keys = keys;
    }

    /// Show the newest line.
    pub(super) fn follow(&self) {
        self.state.scroll_to_end();
    }

    /// The rows: the header and each line.
    fn rows(&self) -> usize {
        self.keys.len() + 1
    }
}

/// The rows of `old` that `new` replaces and how many rows take their
/// place, or `None` when the keys are the same.
fn changed(old: &[String], new: &[String]) -> Option<(Range<usize>, usize)> {
    if old == new {
        return None;
    }
    let prefix = old.iter().zip(new).take_while(|(a, b)| a == b).count();
    let room = old.len().min(new.len()) - prefix;
    let suffix = old
        .iter()
        .rev()
        .zip(new.iter().rev())
        .take(room)
        .take_while(|(a, b)| a == b)
        .count();
    Some((prefix..old.len() - suffix, new.len() - prefix - suffix))
}

/// The transcript: `row` draws row `index` inside the page's column, and
/// the list keeps its own scrollbar.
pub(super) fn transcript_view(
    id: &'static str,
    label: &'static str,
    rows: &TranscriptList,
    layout: Layout,
    cx: &Context<'_, ReactiveAutomations>,
    row: impl Fn(&ReactiveAutomations, usize, &Context<'_, ReactiveAutomations>) -> Option<AnyElement>
        + 'static,
) -> Div {
    let state = rows.state.clone();
    let rows = rows.rows();
    div().relative().flex_1().min_h_0().flex().flex_col().child(
        div()
            .id(id)
            .debug_selector(move || id.to_string())
            .role(Role::Log)
            .aria_label(label)
            .relative()
            .flex_1()
            .min_h_0()
            .overflow_hidden()
            .vertical_scrollbar(&state)
            .child(
                list(
                    state.clone(),
                    cx.processor(move |this, index: usize, _window, cx| {
                        let Some(drawn) = row(this, index, cx) else {
                            return div().into_any_element();
                        };
                        // Web `p-4 sm:px-6` around the column and `gap-3`
                        // between its lines.
                        let last = index + 1 == rows;
                        div()
                            .w_full()
                            .flex()
                            .justify_center()
                            .child(
                                layout
                                    .row()
                                    .when(index == 0, |el| el.pt(px(PAGE_PADDING_PX)))
                                    .pb(px(if last {
                                        PAGE_PADDING_PX
                                    } else {
                                        TRANSCRIPT_GAP_PX
                                    }))
                                    .child(drawn),
                            )
                            .into_any_element()
                    }),
                )
                .size_full(),
            ),
    )
}

#[cfg(test)]
mod tests {
    use super::changed;

    fn keys(keys: &[&str]) -> Vec<String> {
        keys.iter().map(|key| (*key).to_string()).collect()
    }

    #[test]
    fn only_the_rows_that_changed_are_replaced() {
        let shown = keys(&["m3", "m4", "working:r"]);
        assert_eq!(changed(&shown, &shown), None);
        // Load earlier: two rows go in above m3.
        assert_eq!(
            changed(&shown, &keys(&["m1", "m2", "m3", "m4", "working:r"])),
            Some((0..0, 2))
        );
        // A reply takes the working line's place.
        assert_eq!(changed(&shown, &keys(&["m3", "m4", "m5"])), Some((2..3, 1)));
        // A first page loads above the working line.
        assert_eq!(changed(&keys(&["working:r"]), &shown), Some((0..0, 2)));
        // Repeated keys never count twice.
        assert_eq!(changed(&keys(&["a"]), &keys(&["a", "a"])), Some((1..1, 1)));
        assert_eq!(changed(&[], &keys(&["m1"])), Some((0..0, 1)));
    }
}

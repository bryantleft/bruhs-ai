//! The knowledge page's record picker, shared by web and desktop: a
//! case-insensitive search of each record's label or group, results in
//! pages of 24, and the keys that move the highlight. Moved from the web
//! crate's `record_picker/search.rs`.

/// Results shown at once; the highlight's page is the one on screen.
pub const PAGE_SIZE: usize = 24;

/// Lowercased labels and groups, so each keystroke only compares.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RecordIndex(Vec<(String, String)>);

impl RecordIndex {
    /// `entries` are each record's label and group, in list order.
    pub fn new<'a>(entries: impl IntoIterator<Item = (&'a str, Option<&'a str>)>) -> Self {
        Self(
            entries
                .into_iter()
                .map(|(label, group)| {
                    (
                        label.to_lowercase(),
                        group.unwrap_or_default().to_lowercase(),
                    )
                })
                .collect(),
        )
    }

    /// Positions of the records whose label or group contains `query`,
    /// ignoring case and surrounding space; every record for an empty query.
    pub fn matching(&self, query: &str) -> Vec<usize> {
        let query = query.trim().to_lowercase();
        self.0
            .iter()
            .enumerate()
            .filter(|(_, (label, group))| {
                query.is_empty() || label.contains(&query) || group.contains(&query)
            })
            .map(|(index, _)| index)
            .collect()
    }
}

/// The first result position on the page that shows `highlighted`.
pub fn page_start(count: usize, highlighted: usize) -> usize {
    highlighted.min(count.saturating_sub(1)) / PAGE_SIZE * PAGE_SIZE
}

/// The highlight after `key`, or `None` when the key does not move it.
/// Takes web key names (`ArrowDown`) and GPUI's (`down`).
pub fn navigate(current: usize, count: usize, key: &str) -> Option<usize> {
    let last = count.saturating_sub(1);
    let current = current.min(last);
    Some(match key {
        "ArrowDown" | "down" => current.saturating_add(1).min(last),
        "ArrowUp" | "up" => current.saturating_sub(1),
        "PageDown" | "pagedown" => current.saturating_add(PAGE_SIZE).min(last),
        "PageUp" | "pageup" => current.saturating_sub(PAGE_SIZE),
        "Home" | "home" => 0,
        "End" | "end" => last,
        _ => return None,
    })
}

/// The status line above the results.
pub fn match_count_label(count: usize) -> String {
    format!(
        "{count} matching {}",
        if count == 1 { "record" } else { "records" }
    )
}

/// The page indicator between Previous and Next, such as `2 / 5`.
pub fn page_label(count: usize, highlighted: usize) -> String {
    let current = highlighted.min(count.saturating_sub(1));
    format!(
        "{} / {}",
        current / PAGE_SIZE + 1,
        count.div_ceil(PAGE_SIZE).max(1)
    )
}

/// The footer's key hint and the empty list's message.
pub const PICKER_KEY_HINT: &str = "↑↓ to move, Enter to choose";
pub const PICKER_EMPTY: &str = "No matching records. Try another name.";
pub const PICKER_SEARCH_PLACEHOLDER: &str = "Type a name…";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_move_the_highlight_in_both_spellings() {
        assert_eq!(navigate(23, 10_000, "ArrowDown"), Some(24));
        assert_eq!(navigate(23, 10_000, "down"), Some(24));
        assert_eq!(navigate(24, 10_000, "up"), Some(23));
        assert_eq!(navigate(0, 10_000, "end"), Some(9_999));
        assert_eq!(navigate(9_999, 10_000, "home"), Some(0));
        assert_eq!(navigate(24, 10_000, "pagedown"), Some(48));
        assert_eq!(navigate(2, 10_000, "pageup"), Some(0));
        assert_eq!(navigate(0, 10_000, "tab"), None);
    }

    #[test]
    fn search_reads_label_or_group_and_pages_hold_24() {
        let index = RecordIndex::new([("Café plan", Some("Document")), ("Ship it", Some("Task"))]);
        assert_eq!(index.matching("  CAFÉ "), vec![0]);
        assert_eq!(index.matching("task"), vec![1]);
        assert_eq!(index.matching(""), vec![0, 1]);
        assert!(index.matching("plan task").is_empty());
        assert_eq!(page_start(100, 30), 24);
        assert_eq!(page_start(0, 5), 0);
        assert_eq!(page_label(100, 30), "2 / 5");
        assert_eq!(page_label(0, 0), "1 / 1");
        assert_eq!(match_count_label(1), "1 matching record");
        assert_eq!(match_count_label(3), "3 matching records");
    }
}

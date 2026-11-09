//! Visited pages. Desktop owns the stack. The web app uses the browser's
//! history, because every view is already a hash.

use crate::AppView;

/// One step in the page stack. The key is the full view, including detail
/// ids. `AppView::as_str` drops those ids, so it is not a key.
pub fn page_key(view: &AppView) -> String {
    serde_json::to_string(view).unwrap_or_else(|_| format!("\"{}\"", view.as_str()))
}

/// The view a stack key names. A key this build does not know is skipped.
pub fn page_from_key(key: &str) -> Option<AppView> {
    serde_json::from_str(key).ok()
}

/// Browser-style back and forward. Visiting the page already at the index
/// does nothing, so a render that records the current page is safe after
/// back or forward.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PageHistory {
    entries: Vec<String>,
    index: usize,
}

impl PageHistory {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn can_back(&self) -> bool {
        !self.entries.is_empty() && self.index > 0
    }

    pub fn can_forward(&self) -> bool {
        self.index + 1 < self.entries.len()
    }

    /// Record `page` when it differs from the current entry. A new page
    /// drops anything forward of the current index.
    pub fn visit(&mut self, page: &str) {
        if self
            .entries
            .get(self.index)
            .is_some_and(|current| current == page)
        {
            return;
        }
        if !self.entries.is_empty() {
            self.entries.truncate(self.index + 1);
        }
        self.entries.push(page.to_string());
        self.index = self.entries.len() - 1;
    }

    pub fn back(&mut self) -> Option<&str> {
        if !self.can_back() {
            return None;
        }
        self.index -= 1;
        self.entries.get(self.index).map(String::as_str)
    }

    pub fn forward(&mut self) -> Option<&str> {
        if !self.can_forward() {
            return None;
        }
        self.index += 1;
        self.entries.get(self.index).map(String::as_str)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn visit_back_and_forward_keep_one_copy_of_the_current_page() {
        let mut history = PageHistory::new();
        history.visit("today");
        history.visit("today");
        assert!(!history.can_back());
        history.visit("chat");
        history.visit("tasks");
        assert_eq!(history.back(), Some("chat"));
        assert_eq!(history.back(), Some("today"));
        assert_eq!(history.back(), None);
        assert_eq!(history.forward(), Some("chat"));
        history.visit("inbox");
        assert!(!history.can_forward());
        assert_eq!(history.back(), Some("chat"));
        assert_eq!(history.back(), Some("today"));
    }

    #[test]
    fn a_page_key_keeps_the_detail_id() {
        let view = AppView::MeetingDetail {
            meeting_id: "m-1".into(),
        };
        let key = page_key(&view);
        assert_ne!(key, "\"meeting_detail\"");
        assert_eq!(page_from_key(&key), Some(view));
        assert_eq!(page_from_key("nope"), None);
    }
}

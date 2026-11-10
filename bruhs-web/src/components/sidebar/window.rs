//! Variable-height thread cards retain native scrolling without mounting every row.
use std::collections::HashMap;

use bruhs_models::logic::sidebar::SidebarSection;

use crate::state::Thread;

const GAP: f64 = 4.0;
const OVERSCAN: f64 = 400.0;
pub const HEADER_HEIGHT: f64 = 52.0;

#[derive(Clone, PartialEq)]
pub struct Row {
    pub id: String,
    pub top: f64,
    pub height: f64,
    pub section: SidebarSection,
    pub index: usize,
}

#[derive(Clone, PartialEq)]
pub struct Header {
    pub section: SidebarSection,
    pub top: f64,
    pub count: usize,
}

/// One painted block, top to bottom. Headers stay mounted; thread rows window.
#[derive(Clone, PartialEq)]
pub enum Slot {
    Thread(Row),
    Header(Header),
}

impl Slot {
    pub fn key(&self) -> String {
        match self {
            Self::Thread(row) => row.id.clone(),
            Self::Header(header) => format!("header-{}", section_key(header.section)),
        }
    }

    pub fn top(&self) -> f64 {
        match self {
            Self::Thread(row) => row.top,
            Self::Header(header) => header.top,
        }
    }
}

pub fn section_key(section: SidebarSection) -> &'static str {
    match section {
        SidebarSection::Pinned => "pinned",
        SidebarSection::Active => "active",
        SidebarSection::Working => "working",
        SidebarSection::Snoozed => "snoozed",
        SidebarSection::Settled => "settled",
    }
}

#[derive(Default, PartialEq)]
pub struct Layout {
    rows: Vec<Row>,
    by_id: HashMap<String, usize>,
    pub headers: Vec<Header>,
    /// Settled header, when that shelf is showing. Older callers read this.
    pub header_top: Option<f64>,
    pub height: f64,
}

impl Layout {
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "unit tests build the active-then-settled layout through this constructor"
        )
    )]
    pub fn new(fresh: &[Thread], settled: &[Thread], open: bool) -> Self {
        Self::sections(&[], fresh, &[], false, &[], false, settled, open, false)
    }

    /// Pinned and active are always open. Working, snoozed, and settled fold.
    /// `boundaries` adds Pinned and Active labels while a drag is in progress.
    #[expect(
        clippy::too_many_arguments,
        reason = "each shelf is a list plus an open flag, and the drag boundary is a separate flag"
    )]
    pub fn sections(
        pinned: &[Thread],
        active: &[Thread],
        working: &[Thread],
        working_open: bool,
        snoozed: &[Thread],
        snoozed_open: bool,
        settled: &[Thread],
        settled_open: bool,
        boundaries: bool,
    ) -> Self {
        let mut layout = Self::default();
        if boundaries {
            layout.push_header(SidebarSection::Pinned, pinned.len());
        }
        layout.push_threads(pinned, SidebarSection::Pinned);
        if boundaries {
            layout.push_header(SidebarSection::Active, active.len());
        }
        layout.push_threads(active, SidebarSection::Active);
        layout.shelf(SidebarSection::Working, working, working_open);
        layout.shelf(SidebarSection::Snoozed, snoozed, snoozed_open);
        layout.shelf(SidebarSection::Settled, settled, settled_open);
        layout.header_top = layout
            .headers
            .iter()
            .find(|header| header.section == SidebarSection::Settled)
            .map(|header| header.top);
        layout
    }

    fn shelf(&mut self, section: SidebarSection, threads: &[Thread], open: bool) {
        if threads.is_empty() {
            return;
        }
        self.push_header(section, threads.len());
        if open {
            self.push_threads(threads, section);
        }
    }

    fn push_header(&mut self, section: SidebarSection, count: usize) {
        self.headers.push(Header {
            section,
            top: self.height,
            count,
        });
        self.height += HEADER_HEIGHT + GAP;
    }

    fn push_threads(&mut self, threads: &[Thread], section: SidebarSection) {
        for (index, thread) in threads.iter().enumerate() {
            self.push(thread, section, index);
        }
    }

    fn push(&mut self, thread: &Thread, section: SidebarSection, index: usize) {
        let height = if thread.branch.is_some() || thread.model.is_some() || thread.role.is_some() {
            85.0
        } else {
            65.0
        };
        self.by_id.insert(thread.id.clone(), self.rows.len());
        self.rows.push(Row {
            id: thread.id.clone(),
            top: self.height,
            height,
            section,
            index,
        });
        self.height += height + GAP;
    }

    /// Headers plus the thread rows inside the scroll window.
    pub fn slots(&self, top: f64, height: f64) -> Vec<Slot> {
        let mut slots: Vec<Slot> = self.headers.iter().cloned().map(Slot::Header).collect();
        slots.extend(self.window(top, height).cloned().map(Slot::Thread));
        slots.sort_by(|left, right| left.top().total_cmp(&right.top()));
        slots
    }

    pub fn row(&self, id: &str) -> Option<&Row> {
        self.by_id.get(id).map(|&index| &self.rows[index])
    }

    pub fn position(&self, id: &str) -> Option<usize> {
        self.by_id.get(id).map(|index| index + 1)
    }

    pub fn count(&self) -> usize {
        self.rows.len()
    }

    pub fn navigate(&self, id: &str, key: &str) -> Option<&str> {
        let index = *self.by_id.get(id)?;
        let next = match key {
            "ArrowDown" => (index + 1).min(self.rows.len() - 1),
            "ArrowUp" => index.saturating_sub(1),
            "Home" => 0,
            "End" => self.rows.len() - 1,
            _ => return None,
        };
        Some(&self.rows[next].id)
    }

    pub fn window(&self, top: f64, height: f64) -> impl Iterator<Item = &Row> {
        let start = self
            .rows
            .partition_point(|row| row.top + row.height < (top - OVERSCAN).max(0.0));
        self.rows[start..]
            .iter()
            .take_while(move |row| row.top <= top + height + OVERSCAN)
    }

    pub fn reveal(&self, id: &str, top: f64, height: f64) -> Option<f64> {
        let row = self.row(id)?;
        if row.top < top {
            Some(row.top)
        } else if row.top + row.height > top + height {
            Some((row.top + row.height - height).max(0.0))
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn thread(id: usize) -> Thread {
        serde_json::from_value(serde_json::json!({"id":id.to_string(),"title":"Synthetic", "project_name":"", "updated_at":"1"})).unwrap()
    }

    #[test]
    fn large_list_window_is_bounded_at_both_ends() {
        let rows = (0..10_000).map(thread).collect::<Vec<_>>();
        let layout = Layout::new(&rows, &[], false);
        assert!(layout.window(0.0, 900.0).count() < 30);
        assert!(layout.window(layout.height - 900.0, 900.0).count() < 30);
        assert_eq!(
            layout
                .window(layout.height - 900.0, 900.0)
                .last()
                .unwrap()
                .id,
            "9999"
        );
    }

    #[test]
    fn collapsed_settled_rows_do_not_enter_scroll_window() {
        let layout = Layout::new(&[thread(0)], &[thread(1)], false);
        assert_eq!(layout.count(), 1);
        assert_eq!(layout.header_top, Some(69.0));
        assert!(layout.row("1").is_none());
    }

    #[test]
    fn metadata_height_and_header_are_included_in_offsets() {
        let mut rich = thread(0);
        rich.model = Some("synthetic".into());
        let layout = Layout::new(&[rich], &[thread(1)], true);
        assert_eq!(layout.row("1").unwrap().top, 89.0 + HEADER_HEIGHT + GAP);
        assert_eq!(layout.position("1"), Some(2));
    }

    #[test]
    fn a_role_keeps_the_metadata_line() {
        let mut with_role = thread(0);
        with_role.role = Some("writer".into());
        let layout = Layout::new(&[with_role, thread(1)], &[], false);
        assert_eq!(layout.row("0").unwrap().height, 85.0);
        assert_eq!(layout.row("1").unwrap().height, 65.0);
    }

    #[test]
    fn revealing_selected_thread_does_not_scroll_when_already_visible() {
        let layout = Layout::new(&(0..100).map(thread).collect::<Vec<_>>(), &[], false);
        assert_eq!(layout.reveal("1", 0.0, 900.0), None);
        assert_eq!(layout.reveal("99", 0.0, 900.0), Some(5996.0));
        assert_eq!(layout.reveal("0", 500.0, 900.0), Some(0.0));
    }

    #[test]
    fn keyboard_navigation_reaches_unmounted_rows_and_stops_at_edges() {
        let layout = Layout::new(&(0..100).map(thread).collect::<Vec<_>>(), &[], false);
        assert_eq!(layout.navigate("0", "End"), Some("99"));
        assert_eq!(layout.navigate("99", "Home"), Some("0"));
        assert_eq!(layout.navigate("0", "ArrowUp"), Some("0"));
        assert_eq!(layout.navigate("99", "ArrowDown"), Some("99"));
        assert_eq!(layout.navigate("50", "ArrowDown"), Some("51"));
        assert_eq!(layout.navigate("50", "ArrowUp"), Some("49"));
        assert_eq!(layout.navigate("missing", "Home"), None);
        assert_eq!(layout.navigate("0", "Enter"), None);
    }
}

//! Only row geometry and focus targets live here; cards read current thread data.
use super::*;
use crate::views::workspace::labs;
use bruhs_ui::zoom::px;
use std::collections::HashMap;
use std::rc::Rc;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum RowKey {
    Thread(EntityId),
    Shelf(&'static str),
    Folder(SharedString),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum SidebarRow {
    Thread {
        index: usize,
        entity: EntityId,
        section: SidebarSection,
        has_meta: bool,
        folder: bool,
    },
    Folder {
        path: SharedString,
        checkout: SharedString,
        name: SharedString,
        count: usize,
        open: bool,
    },
    Shelf {
        section: SidebarSection,
        count: usize,
        open: bool,
    },
}

impl SidebarRow {
    pub(super) fn section(&self) -> SidebarSection {
        match self {
            Self::Thread { section, .. } | Self::Shelf { section, .. } => *section,
            Self::Folder { .. } => SidebarSection::Active,
        }
    }

    fn target_count(&self) -> usize {
        match self {
            Self::Thread { .. } => 3,
            Self::Shelf { .. } | Self::Folder { .. } => 1,
        }
    }

    fn key(&self) -> RowKey {
        match self {
            Self::Thread { entity, .. } => RowKey::Thread(*entity),
            Self::Shelf { section, .. } => RowKey::Shelf(shelf_labels(*section).0),
            Self::Folder { path, .. } => RowKey::Folder(path.clone()),
        }
    }
}

pub(super) fn shelf_labels(section: SidebarSection) -> (&'static str, &'static str) {
    match section {
        SidebarSection::Working => ("desktop-sidebar-working-fold", "Working"),
        SidebarSection::Snoozed => ("desktop-sidebar-snoozed-fold", "Snoozed"),
        SidebarSection::Settled => ("desktop-sidebar-settled-fold", "Settled"),
        SidebarSection::Pinned | SidebarSection::Active => {
            unreachable!("Pinned and Active have cards but no fold headers")
        }
    }
}

pub(super) fn sidebar_rows(
    workspace: &Workspace,
    shelves: &threads_ui::SidebarShelves<'_>,
    cx: &App,
) -> Vec<SidebarRow> {
    let indices: HashMap<_, _> = workspace
        .threads
        .iter()
        .enumerate()
        .map(|(index, thread)| (thread.chat.entity_id(), index))
        .collect();
    let card = |thread: &DesktopThread, section, folder: bool| SidebarRow::Thread {
        index: indices[&thread.chat.entity_id()],
        entity: thread.chat.entity_id(),
        section,
        folder,
        has_meta: if folder {
            workspace.labs_sidebar.two_lines
        } else {
            workspace.card_has_meta(thread, cx)
        },
    };
    let mut rows: Vec<_> = shelves
        .pinned
        .iter()
        .map(|t| card(t, SidebarSection::Pinned, false))
        .collect();
    if workspace.labs_sidebar.enabled {
        let mut folders = std::collections::BTreeMap::<String, Vec<&DesktopThread>>::new();
        for thread in &shelves.active {
            folders
                .entry(workspace.project_group_key(labs::folder(thread, cx)))
                .or_default()
                .push(thread);
        }
        let ranks: HashMap<_, _> = workspace
            .listed_projects()
            .into_iter()
            .enumerate()
            .map(|(i, p)| (p.path, i))
            .collect();
        let rank = |thread: &DesktopThread| {
            ranks
                .get(labs::folder(thread, cx))
                .copied()
                .unwrap_or(usize::MAX)
        };
        let mut ordered: Vec<_> = folders
            .into_iter()
            .map(|(key, threads)| {
                let first = threads
                    .iter()
                    .copied()
                    .min_by_key(|thread| rank(thread))
                    .expect("nonempty folder");
                (key, first, threads)
            })
            .collect();
        ordered.sort_by_key(|(_, first, _)| rank(first));
        let searching = !workspace.sidebar_query(cx).trim().is_empty();
        for (key, first, threads) in ordered {
            let open = searching || !workspace.labs_sidebar.collapsed.contains(&key);
            let path = labs::folder(first, cx);
            let merged = threads.iter().any(|t| labs::folder(t, cx) != path);
            let name = if first.chat.read(cx).scratch_folder().is_some() {
                "Scratch".to_owned()
            } else if merged {
                threads
                    .iter()
                    .filter_map(|thread| {
                        workspace
                            .project_list
                            .title(labs::folder(thread, cx))
                            .map(|title| (rank(thread), title))
                    })
                    .min_by_key(|(rank, _)| *rank)
                    .map(|(_, title)| title.to_owned())
                    .or_else(|| workspace.group_label(path))
                    .unwrap_or_else(|| first.project_name.clone())
            } else if let Some(title) = workspace.project_list.title(path) {
                title.to_owned()
            } else if first.project_name.is_empty() {
                SIDEBAR_NO_PROJECT_LABEL.to_owned()
            } else {
                first.project_name.clone()
            };
            rows.push(SidebarRow::Folder {
                path: key.into(),
                checkout: path.to_owned().into(),
                name: name.into(),
                count: threads.len(),
                open,
            });
            if open {
                rows.extend(
                    threads
                        .into_iter()
                        .map(|t| card(t, SidebarSection::Active, true)),
                );
            }
        }
    } else {
        rows.extend(
            shelves
                .active
                .iter()
                .map(|t| card(t, SidebarSection::Active, false)),
        );
    }
    for (section, threads, open) in [
        (
            SidebarSection::Working,
            &shelves.working,
            workspace.sidebar_working_open,
        ),
        (
            SidebarSection::Snoozed,
            &shelves.snoozed,
            workspace.sidebar_snoozed_open,
        ),
        (
            SidebarSection::Settled,
            &shelves.settled,
            workspace.sidebar_settled_open,
        ),
    ] {
        if !threads.is_empty() {
            rows.push(SidebarRow::Shelf {
                section,
                count: threads.len(),
                open,
            });
            if open {
                rows.extend(threads.iter().map(|t| card(t, section, false)));
            }
        }
    }
    rows
}

#[derive(Clone)]
pub(super) struct SidebarRowFocus {
    pub(super) card: FocusHandle,
    pub(super) menu: FocusHandle,
    pub(super) delete: FocusHandle,
}

impl SidebarRowFocus {
    fn new(cx: &App) -> Self {
        Self {
            card: cx.focus_handle().tab_index(0).tab_stop(true),
            menu: cx.focus_handle().tab_index(0).tab_stop(true),
            delete: cx.focus_handle().tab_index(0).tab_stop(true),
        }
    }

    fn targets(&self) -> [&FocusHandle; 3] {
        [&self.card, &self.menu, &self.delete]
    }
}

#[derive(PartialEq, Eq)]
pub(super) struct SidebarScope {
    pub(super) query: String,
}

pub(in crate::views::workspace) struct SidebarViewport {
    pub(in crate::views::workspace) list: ListState,
    pub(super) rows: Rc<[SidebarRow]>,
    focus: Rc<[SidebarRowFocus]>,
    start: FocusHandle,
    end: FocusHandle,
    rem_size: Option<Pixels>,
    scope: Option<SidebarScope>,
    needs_height_hints: bool,
    hint_width: Option<Pixels>,
    arrangement_scroll: Option<ListOffset>,
    pub(in crate::views::workspace) first_row_bounds: Option<Bounds<Pixels>>,
}

impl SidebarViewport {
    pub(in crate::views::workspace) fn new(cx: &App) -> Self {
        Self {
            // Native scroll events lay out visible rows in the same frame; no extra rows are needed.
            list: ListState::new(0, ListAlignment::Top, px(0.0)),
            rows: Rc::from([]),
            focus: Rc::from([]),
            start: cx.focus_handle().tab_index(0).tab_stop(false),
            end: cx.focus_handle().tab_index(0).tab_stop(false),
            rem_size: None,
            scope: None,
            needs_height_hints: true,
            hint_width: None,
            arrangement_scroll: None,
            first_row_bounds: None,
        }
    }

    pub(super) fn boundaries(&self) -> (FocusHandle, FocusHandle) {
        (self.start.clone(), self.end.clone())
    }

    pub(super) fn measured_width(&self) -> Option<Pixels> {
        self.hint_width
    }

    pub(super) fn thread_bounds(&self, entity: EntityId) -> Option<Bounds<Pixels>> {
        let index = self
            .rows
            .iter()
            .position(|row| row.key() == RowKey::Thread(entity))?;
        self.list.bounds_for_item(index)
    }

    pub(in crate::views::workspace) fn retain_arrangement_scroll(&mut self) {
        self.arrangement_scroll = Some(self.list.logical_scroll_top());
    }

    /// The card for chat `entity`'s focus targets: card, "…" button, delete.
    pub(in crate::views::workspace) fn row_targets(
        &self,
        entity: EntityId,
    ) -> Option<[FocusHandle; 3]> {
        self.rows
            .iter()
            .position(|row| row.key() == RowKey::Thread(entity))
            .map(|index| {
                let focus = &self.focus[index];
                [focus.card.clone(), focus.menu.clone(), focus.delete.clone()]
            })
    }

    /// The "…" button of the card for chat `entity`, while the list has one.
    pub(in crate::views::workspace) fn menu_focus(&self, entity: EntityId) -> Option<FocusHandle> {
        self.rows
            .iter()
            .position(|row| row.key() == RowKey::Thread(entity))
            .map(|index| self.focus[index].menu.clone())
    }

    pub(super) fn prepare(
        &mut self,
        rows: Vec<SidebarRow>,
        scope: SidebarScope,
        rem_size: Pixels,
        cx: &App,
    ) -> (ListState, Rc<[SidebarRow]>, Rc<[SidebarRowFocus]>) {
        let arrangement_scroll = self.arrangement_scroll.take();
        if self.rows.as_ref() != rows.as_slice() {
            let old: HashMap<_, _> = self
                .rows
                .iter()
                .zip(self.focus.iter())
                .map(|(row, focus)| (row.key(), focus.clone()))
                .collect();
            let focus: Rc<[_]> = rows
                .iter()
                .map(|row| {
                    old.get(&row.key())
                        .cloned()
                        .unwrap_or_else(|| SidebarRowFocus::new(cx))
                })
                .collect();
            let prefix = self
                .rows
                .iter()
                .zip(&rows)
                .take_while(|(a, b)| a == b)
                .count();
            let suffix = self.rows[prefix..]
                .iter()
                .rev()
                .zip(rows[prefix..].iter().rev())
                .take_while(|(a, b)| a == b)
                .count();
            self.list.splice_focusable(
                prefix..self.rows.len() - suffix,
                focus[prefix..rows.len() - suffix]
                    .iter()
                    .map(|f| Some(f.card.clone())),
            );
            self.rows = rows.into();
            self.focus = focus;
            self.needs_height_hints = true;
            if let Some(offset) = arrangement_scroll {
                self.list.scroll_to(offset);
            }
        }
        if self.scope.as_ref() != Some(&scope) {
            // A different result set starts at its first result. In particular,
            // clearing a single match must not keep that match as the scroll anchor.
            self.list.scroll_to(ListOffset::default());
            self.scope = Some(scope);
        }
        if self
            .rem_size
            .replace(rem_size)
            .is_some_and(|old| old != rem_size)
        {
            self.list.remeasure();
            self.needs_height_hints = true;
        }
        (self.list.clone(), self.rows.clone(), self.focus.clone())
    }

    /// GPUI clears height hints when the list width changes, including its first
    /// layout. Seed unknown rows after that layout so wheel and scrollbar input
    /// can reach the whole list immediately. Visible rows retain their real sizes.
    pub(super) fn seed_height_hints(&mut self) {
        self.first_row_bounds = self.list.bounds_for_item(0);
        let bounds = self.list.viewport_bounds();
        let width_changed = self.hint_width.replace(bounds.size.width) != Some(bounds.size.width);
        if !self.needs_height_hints && !width_changed {
            return;
        }
        let anchor = self
            .list
            .logical_scroll_top()
            .item_ix
            .min(self.rows.len().saturating_sub(1));
        if let Some(row) = self.list.bounds_for_item(anchor) {
            // This is an estimate for unmeasured rows, derived from this frame's
            // actual font/layout metrics. GPUI replaces it as each row is drawn.
            self.list.clone().with_uniform_item_height(row.size.height);
            self.needs_height_hints = false;
        }
    }

    fn focused(&self, window: &Window) -> Option<(usize, usize)> {
        self.rows
            .iter()
            .zip(self.focus.iter())
            .enumerate()
            .find_map(|(index, (row, focus))| {
                focus
                    .targets()
                    .into_iter()
                    .take(row.target_count())
                    .position(|handle| handle.is_focused(window))
                    .map(|target| (index, target))
            })
    }

    fn focus_target(&self, index: usize, target: usize, window: &mut Window, cx: &mut App) {
        self.list.scroll_to_reveal_item(index);
        let bounds = self
            .list
            .bounds_for_item(index)
            .or_else(|| (index == 0).then_some(self.first_row_bounds).flatten());
        if bounds.is_none() {
            // An unmeasured row has no pixel geometry yet. Anchor it directly.
            self.list.scroll_to(ListOffset {
                item_ix: index,
                offset_in_item: px(0.0),
            });
        }
        self.focus[index].targets()[target].focus(window, cx);
    }

    /// Move between conversation cards without activating them.
    pub(super) fn navigate_threads(&self, key: &str, window: &mut Window, cx: &mut App) -> bool {
        let Some((index, 0)) = self.focused(window) else {
            return false;
        };
        let is_thread = |row: &SidebarRow| matches!(row, SidebarRow::Thread { .. });
        if !is_thread(&self.rows[index]) {
            return false;
        }
        let next = match key {
            "down" => self.rows[index + 1..]
                .iter()
                .position(is_thread)
                .map(|offset| index + 1 + offset),
            "up" => self.rows[..index].iter().rposition(is_thread),
            "home" => self.rows.iter().position(is_thread),
            "end" => self.rows.iter().rposition(is_thread),
            _ => return false,
        };
        if let Some(next) = next {
            self.focus_target(next, 0, window, cx);
        }
        true
    }

    /// Traverse every logical row, including rows outside the viewport. Boundaries
    /// let GPUI continue to the surrounding controls without an extra tab stop.
    pub(in crate::views::workspace) fn tab(
        &self,
        backwards: bool,
        window: &mut Window,
        cx: &mut App,
    ) -> bool {
        if self.rows.is_empty() {
            return false;
        }
        if let Some((index, target)) = self.focused(window) {
            let next = if backwards {
                if target > 0 {
                    Some((index, target - 1))
                } else {
                    index
                        .checked_sub(1)
                        .map(|previous| (previous, self.rows[previous].target_count() - 1))
                }
            } else if target + 1 < self.rows[index].target_count() {
                Some((index, target + 1))
            } else if index + 1 < self.rows.len() {
                Some((index + 1, 0))
            } else {
                None
            };
            if let Some((index, target)) = next {
                self.focus_target(index, target, window, cx);
            } else {
                if backwards {
                    self.start.focus(window, cx);
                    window.focus_prev(cx);
                } else {
                    self.end.focus(window, cx);
                    window.focus_next(cx);
                }
            }
        } else {
            if backwards {
                window.focus_prev(cx);
            } else {
                window.focus_next(cx);
            }
            if self.focused(window).is_some() {
                let index = if backwards { self.rows.len() - 1 } else { 0 };
                let target = if backwards {
                    self.rows[index].target_count() - 1
                } else {
                    0
                };
                if backwards {
                    self.list.scroll_to_end();
                } else {
                    self.list.scroll_to(ListOffset::default());
                }
                self.focus_target(index, target, window, cx);
            }
        }
        true
    }
}

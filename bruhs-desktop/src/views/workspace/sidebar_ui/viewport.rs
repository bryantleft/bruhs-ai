//! Only row geometry and focus targets live here; cards read current thread data.
use super::*;
use gpui_component::zoom::px;
use std::collections::HashMap;
use std::rc::Rc;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum RowKey {
    Thread(EntityId),
    Shelf(&'static str),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SidebarRow {
    Thread {
        index: usize,
        entity: EntityId,
        has_meta: bool,
    },
    Shelf {
        section: SidebarSection,
        count: usize,
        open: bool,
    },
}

impl SidebarRow {
    fn target_count(self) -> usize {
        match self {
            Self::Thread { .. } => 3,
            Self::Shelf { .. } => 1,
        }
    }

    fn key(self) -> RowKey {
        match self {
            Self::Thread { entity, .. } => RowKey::Thread(entity),
            Self::Shelf { section, .. } => RowKey::Shelf(shelf_labels(section).0),
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
    let card = |thread: &&DesktopThread| {
        let chat = thread.chat.read(cx);
        SidebarRow::Thread {
            index: indices[&thread.chat.entity_id()],
            entity: thread.chat.entity_id(),
            has_meta: thread.branch.is_some() || !chat.messages.is_empty() || chat.role.is_some(),
        }
    };
    let mut rows: Vec<_> = shelves
        .pinned
        .iter()
        .chain(&shelves.active)
        .map(card)
        .collect();
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
                rows.extend(threads.iter().map(card));
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
    rows: Rc<[SidebarRow]>,
    focus: Rc<[SidebarRowFocus]>,
    start: FocusHandle,
    end: FocusHandle,
    rem_size: Option<Pixels>,
    scope: Option<SidebarScope>,
    needs_height_hints: bool,
    hint_width: Option<Pixels>,
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
            first_row_bounds: None,
        }
    }

    pub(super) fn boundaries(&self) -> (FocusHandle, FocusHandle) {
        (self.start.clone(), self.end.clone())
    }

    /// The card for chat `entity`'s focus targets: card, "…" button, delete.
    #[cfg(test)]
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

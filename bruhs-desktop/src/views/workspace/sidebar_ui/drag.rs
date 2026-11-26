use super::*;
use bruhs_models::logic::sidebar::DragEffect;

const SCROLL_EDGE_PX: f32 = 26.0;
const SCROLL_STEP_PX: f32 = 8.0;
const SHELF_RULE_PX: f32 = 1.0;
const SHELF_TEXT_PX: f32 = 18.0;
const SHELF_LABEL_PX: f32 = SHELF_TEXT_PX + 6.0;

fn shelf_padding(empty_settled: bool) -> Pixels {
    px((2.0 + f32::from(u8::from(empty_settled))) * SHELF_LABEL_PX)
}

#[derive(Clone)]
pub(super) struct ThreadDrag {
    id: String,
    entity: EntityId,
    identity: CardIdentity,
    width: Pixels,
    pinned: bool,
}

pub(in crate::views::workspace) struct SidebarDrag {
    item: ThreadDrag,
    preview: Entity<ThreadDragPreview>,
    placement: Option<Placement>,
    row_height: Pixels,
    scroll_pending: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct Placement {
    section: SidebarSection,
    destination: usize,
    slot: usize,
    effect: DragEffect,
}

#[derive(Clone, Copy)]
pub(super) struct DragLayout {
    source: usize,
    row_height: Pixels,
    destination: Option<usize>,
    pinned: Option<Pixels>,
    active: Option<Pixels>,
    empty_settled: bool,
    trailing_gap: Pixels,
}

impl DragLayout {
    pub(super) fn bottom_padding(self) -> Pixels {
        shelf_padding(self.empty_settled)
    }

    fn label_space(self, section: SidebarSection) -> Pixels {
        let count = u8::from(self.pinned.is_some())
            + u8::from(self.active.is_some() && section != SidebarSection::Pinned);
        px(f32::from(count) * SHELF_LABEL_PX)
    }

    pub(super) fn row(self, index: usize, section: SidebarSection) -> (Pixels, bool) {
        let shift = match self.destination {
            Some(slot) if (slot..self.source).contains(&index) => self.row_height,
            Some(slot) if (self.source + 1..slot).contains(&index) => -self.row_height,
            _ => px(0.),
        };
        (
            shift + self.label_space(section),
            self.destination.is_some() && index == self.source,
        )
    }

    fn card_height(self) -> Pixels {
        self.row_height - self.trailing_gap
    }
}

struct ThreadDragPreview {
    item: ThreadDrag,
    effect: DragEffect,
    projected: bool,
}

impl Render for ThreadDragPreview {
    fn render(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        if self.projected {
            div().size_0().into_any_element()
        } else {
            self.card(None, window, cx).into_any_element()
        }
    }
}

impl ThreadDragPreview {
    fn card(&self, height: Option<Pixels>, window: &Window, cx: &App) -> impl IntoElement {
        let badge = match self.effect {
            DragEffect::PinAt(_) => Some(("Pin", assets::PIN_ICON_PATH)),
            DragEffect::UnpinAt(_) => Some(("Unpin", assets::PIN_ICON_PATH)),
            DragEffect::Settle => Some(("Settle", assets::CHECK_ICON_PATH)),
            DragEffect::UnsettleAt(_) => Some(("Un-settle", assets::ARROW_UP_ICON_PATH)),
            DragEffect::WakeAt(_) => Some(("Wake", assets::CLOCK_ICON_PATH)),
            _ => None,
        };
        let identity = &self.item.identity;
        let project_line = identity
            .project_line()
            .child(
                div()
                    .min_w_0()
                    .flex_1()
                    .truncate()
                    .child(identity.project.clone()),
            )
            .when_some(badge, |el, (label, path)| {
                el.child(
                    div()
                        .debug_selector(move || format!("desktop-sidebar-drop-action-{label}"))
                        .flex()
                        .flex_shrink_0()
                        .items_center()
                        .gap_1()
                        .text_color(theme::persimmon_400())
                        .child(icon(path, SIDEBAR_SMALL_ICON_PX, theme::persimmon_400()))
                        .child(label),
                )
            })
            .when(self.item.pinned && badge.is_none(), |el| {
                el.child(icon(
                    assets::PIN_ICON_PATH,
                    SIDEBAR_SMALL_ICON_PX,
                    theme::rambutan_400(),
                ))
            });
        div()
            .debug_selector(|| "desktop-sidebar-thread-drag-preview".to_string())
            .w(self.item.width)
            .when_some(height, |el, height| el.h(height))
            .flex()
            .flex_col()
            .gap(px(SIDEBAR_CARD_LINE_GAP_PX))
            .px(px(SIDEBAR_CARD_X_PADDING_PX))
            .py(px(SIDEBAR_CARD_Y_PADDING_PX))
            .rounded(px(SIDEBAR_RADIUS_LYCHEE_PX))
            .border_1()
            .border_color(theme::persimmon_400())
            .bg(theme::longan_800())
            .text_size(px(SIDEBAR_CARD_TITLE_TEXT_PX))
            .text_color(theme::rambutan_100())
            .child(project_line)
            .child(div().min_w_0().truncate().child(identity.title.clone()))
            .when(identity.has_meta, |el| {
                el.child(identity.meta_line(
                    self.item.width,
                    None,
                    element_key(&self.item.id),
                    window,
                    cx,
                ))
            })
    }
}

#[derive(Clone, Copy)]
struct ShelfEdges {
    active: usize,
    parked: usize,
    settled: usize,
}

impl ShelfEdges {
    fn new(rows: &[viewport::SidebarRow]) -> Self {
        Self {
            active: rows.partition_point(|row| row.section() == SidebarSection::Pinned),
            parked: rows.partition_point(|row| {
                matches!(
                    row.section(),
                    SidebarSection::Pinned | SidebarSection::Active
                )
            }),
            settled: rows.partition_point(|row| row.section() != SidebarSection::Settled),
        }
    }

    fn start(self, section: SidebarSection) -> usize {
        match section {
            SidebarSection::Pinned => 0,
            SidebarSection::Active => self.active,
            SidebarSection::Settled => self.settled,
            _ => self.parked,
        }
    }
}

impl viewport::SidebarViewport {
    fn row_index(&self, entity: EntityId) -> Option<usize> {
        self.rows.iter().position(
            |row| matches!(row, viewport::SidebarRow::Thread { entity: id, .. } if *id == entity),
        )
    }

    fn edge(&self, row: usize) -> Option<Pixels> {
        if row < self.rows.len() {
            self.list.bounds_for_item(row).map(|bounds| bounds.top())
        } else {
            self.list
                .bounds_for_item(row.checked_sub(1)?)
                .map(|bounds| bounds.bottom())
        }
    }

    fn drag_layout(&self, drag: &SidebarDrag, edges: ShelfEdges) -> Option<DragLayout> {
        let source = self.row_index(drag.item.entity)?;
        let source_section = self.rows[source].section();
        let destination = drag
            .placement
            .filter(|target| target.section != SidebarSection::Settled);
        let viewport = self.list.viewport_bounds();
        let visible =
            |y: &Pixels| *y >= viewport.top() && *y < viewport.bottom() - px(SHELF_LABEL_PX);
        let pinned = self.edge(0).filter(visible);
        let active = self.edge(edges.active).and_then(|edge| {
            let shift = match destination.map(|target| target.section) {
                Some(SidebarSection::Pinned) if source_section != SidebarSection::Pinned => {
                    drag.row_height
                }
                Some(SidebarSection::Active) if source_section == SidebarSection::Pinned => {
                    -drag.row_height
                }
                _ => px(0.),
            };
            let y = edge
                + shift
                + if pinned.is_some() {
                    px(SHELF_LABEL_PX)
                } else {
                    px(0.)
                };
            visible(&y).then_some(y)
        });
        Some(DragLayout {
            source,
            row_height: drag.row_height,
            destination: destination.map(|target| target.slot),
            pinned,
            active,
            empty_settled: edges.settled == self.rows.len(),
            trailing_gap: if source + 1 < self.rows.len() {
                px(SIDEBAR_LIST_GAP_PX)
            } else {
                px(0.)
            },
        })
    }

    fn projected_top(&self, drag: &SidebarDrag, layout: DragLayout) -> Option<(Placement, Pixels)> {
        let target = drag
            .placement
            .filter(|target| target.section != SidebarSection::Settled)?;
        let edge = self.edge(target.slot).or_else(|| {
            self.list
                .bounds_for_item(target.slot.checked_sub(1)?)
                .map(|bounds| bounds.bottom())
        })?;
        let top = if target.slot > layout.source {
            edge - drag.row_height
        } else {
            edge
        };
        Some((target, top + layout.label_space(target.section)))
    }

    fn empty_settled_edge(&self, layout: DragLayout) -> Option<Pixels> {
        let viewport = self.list.viewport_bounds();
        let end = self.edge(self.rows.len())? + layout.label_space(SidebarSection::Settled);
        (end >= viewport.top() && end + px(SHELF_LABEL_PX) <= viewport.bottom()).then_some(end)
    }

    fn drag_scroll_step(&self, position: Point<Pixels>) -> Option<Pixels> {
        let viewport = self.list.viewport_bounds();
        if !viewport.contains(&position) {
            return None;
        }
        let top = self.list.logical_scroll_top();
        if position.y < viewport.top() + px(SCROLL_EDGE_PX)
            && (top.item_ix > 0 || top.offset_in_item > px(0.))
        {
            return Some(px(-SCROLL_STEP_PX));
        }
        let empty_settled = self
            .rows
            .last()
            .is_none_or(|row| row.section() != SidebarSection::Settled);
        if position.y >= viewport.bottom() - px(SCROLL_EDGE_PX)
            && self.edge(self.rows.len()).is_none_or(|end| {
                end + px(SIDEBAR_LIST_BOTTOM_PADDING_PX) + shelf_padding(empty_settled)
                    > viewport.bottom()
            })
        {
            return Some(px(SCROLL_STEP_PX));
        }
        None
    }

    fn hit_shelf(
        &self,
        position: Point<Pixels>,
        edges: ShelfEdges,
        drag: &SidebarDrag,
        layout: DragLayout,
    ) -> Option<(SidebarSection, usize)> {
        let viewport = self.list.viewport_bounds();
        if !viewport.contains(&position) {
            return None;
        }
        let y = position.y;
        let over_label = |label: Option<Pixels>| {
            label.is_some_and(|top| y >= top && y < top + px(SHELF_LABEL_PX))
        };
        if over_label(layout.pinned) {
            return Some((SidebarSection::Pinned, 0));
        }
        if over_label(layout.active) {
            return Some((SidebarSection::Active, edges.active));
        }
        // Keep the current target while the pointer is inside its preview. Re-testing the
        // original row at that position would send the card back across the divider.
        if let Some((target, top)) = self.projected_top(drag, layout) {
            if y >= top && y < top + layout.row_height {
                return Some((target.section, target.slot));
            }
        }
        if layout.empty_settled
            && self
                .empty_settled_edge(layout)
                .is_some_and(|top| y >= top && y < top + px(SHELF_LABEL_PX))
        {
            return Some((SidebarSection::Settled, edges.settled));
        }
        for index in self.list.logical_scroll_top().item_ix..self.rows.len() {
            let Some(bounds) = self.list.bounds_for_item(index) else {
                break;
            };
            if bounds.top() > viewport.bottom() {
                break;
            }
            let section = self.rows[index].section();
            let (shift, hidden) = layout.row(index, section);
            if !hidden && y >= bounds.top() + shift && y < bounds.bottom() + shift {
                let after = matches!(section, SidebarSection::Pinned | SidebarSection::Active)
                    && y >= bounds.center().y + shift;
                return Some((section, index + usize::from(after)));
            }
        }
        if edges.parked == self.rows.len() && self.edge(edges.parked).is_some() {
            return Some((SidebarSection::Active, edges.parked));
        }
        None
    }

    fn placement(
        &self,
        drag: &SidebarDrag,
        position: Point<Pixels>,
        workspace: &Workspace,
        cx: &App,
    ) -> Option<Placement> {
        let edges = ShelfEdges::new(&self.rows);
        let layout = self.drag_layout(drag, edges)?;
        let (section, slot) = self.hit_shelf(position, edges, drag, layout)?;
        let start = edges.start(section);
        let removed_before = slot > layout.source && self.rows[layout.source].section() == section;
        let destination = slot.saturating_sub(start + usize::from(removed_before));
        let effect = workspace.sidebar_drag_effect(&drag.item.id, section, destination, cx);
        (effect != DragEffect::None).then_some(Placement {
            section,
            destination,
            slot,
            effect,
        })
    }
}

impl Workspace {
    pub(super) fn sidebar_arrangement_enabled(&self, cx: &App) -> bool {
        !self.labs_sidebar.enabled && self.sidebar_query(cx).trim().is_empty()
    }

    pub(super) fn sidebar_drag_item(
        &self,
        thread: &DesktopThread,
        identity: CardIdentity,
    ) -> ThreadDrag {
        let width = self
            .sidebar_viewport
            .borrow()
            .measured_width()
            .unwrap_or_else(|| px(SIDEBAR_WIDTH_PX - 2.0 * SIDEBAR_LIST_X_PADDING_PX));
        ThreadDrag {
            id: thread.id.clone(),
            entity: thread.chat.entity_id(),
            identity,
            width: width.max(px(100.)),
            pinned: thread.pinned,
        }
    }

    pub(super) fn attach_sidebar_drag(
        &self,
        card: Stateful<Div>,
        item: ThreadDrag,
        cx: &Context<'_, Self>,
    ) -> Stateful<Div> {
        let workspace = cx.weak_entity();
        card.on_drag(item, move |item: &ThreadDrag, _, _, cx| {
            cx.stop_propagation();
            let preview = cx.new(|_| ThreadDragPreview {
                item: item.clone(),
                effect: DragEffect::None,
                projected: false,
            });
            let _ = workspace.update(cx, |this, cx| {
                let row_height = {
                    let viewport = this.sidebar_viewport.borrow();
                    viewport
                        .row_index(item.entity)
                        .and_then(|index| viewport.list.bounds_for_item(index))
                        .map_or(px(0.), |row| row.size.height)
                };
                this.sidebar_drag = Some(SidebarDrag {
                    item: item.clone(),
                    preview: preview.clone(),
                    placement: None,
                    row_height,
                    scroll_pending: false,
                });
                cx.notify();
            });
            preview
        })
    }

    pub(super) fn update_sidebar_drag(
        &mut self,
        event: &DragMoveEvent<ThreadDrag>,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let placement = self.sidebar_drag.as_ref().and_then(|drag| {
            self.sidebar_viewport
                .borrow()
                .placement(drag, event.event.position, self, cx)
        });
        self.update_sidebar_drag_placement(placement, cx);
        self.schedule_sidebar_drag_scroll(window, cx);
    }

    fn update_sidebar_drag_placement(
        &mut self,
        placement: Option<Placement>,
        cx: &mut Context<'_, Self>,
    ) {
        let Some(drag) = &mut self.sidebar_drag else {
            return;
        };
        if drag.placement == placement {
            return;
        }
        drag.placement = placement;
        drag.preview.update(cx, |preview, cx| {
            preview.effect = placement.map_or(DragEffect::None, |placement| placement.effect);
            preview.projected =
                placement.is_some_and(|target| target.section != SidebarSection::Settled);
            cx.notify();
        });
        cx.notify();
    }

    fn schedule_sidebar_drag_scroll(&mut self, window: &Window, cx: &mut Context<'_, Self>) {
        let Some(drag) = &mut self.sidebar_drag else {
            return;
        };
        if drag.scroll_pending
            || self
                .sidebar_viewport
                .borrow()
                .drag_scroll_step(window.mouse_position())
                .is_none()
        {
            return;
        }
        drag.scroll_pending = true;
        let preview = drag.preview.entity_id();
        let workspace = cx.weak_entity();
        window.on_next_frame(move |window, cx| {
            let _ = workspace.update(cx, |this, cx| {
                this.scroll_sidebar_drag(preview, window, cx);
            });
        });
    }

    fn scroll_sidebar_drag(
        &mut self,
        preview: EntityId,
        window: &Window,
        cx: &mut Context<'_, Self>,
    ) {
        let Some(drag) = self
            .sidebar_drag
            .as_mut()
            .filter(|drag| drag.preview.entity_id() == preview)
        else {
            return;
        };
        drag.scroll_pending = false;
        if !cx.has_active_drag() {
            return;
        }
        let position = window.mouse_position();
        let placement = {
            let viewport = self.sidebar_viewport.borrow();
            let Some(step) = viewport.drag_scroll_step(position) else {
                return;
            };
            let scroll_top = viewport.list.logical_scroll_top();
            viewport.list.scroll_by(step);
            let new_top = viewport.list.logical_scroll_top();
            if (new_top.item_ix, new_top.offset_in_item)
                == (scroll_top.item_ix, scroll_top.offset_in_item)
            {
                return;
            }
            self.sidebar_drag
                .as_ref()
                .and_then(|drag| viewport.placement(drag, position, self, cx))
        };
        self.update_sidebar_drag_placement(placement, cx);
        cx.notify();
        self.schedule_sidebar_drag_scroll(window, cx);
    }

    pub(super) fn drop_sidebar_drag(
        &mut self,
        item: &ThreadDrag,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let placement = self
            .sidebar_drag
            .as_ref()
            .filter(|drag| drag.item.id == item.id)
            .and_then(|drag| {
                self.sidebar_viewport
                    .borrow()
                    .placement(drag, window.mouse_position(), self, cx)
            });
        if let Some(placement) = placement {
            cx.stop_propagation();
            self.commit_sidebar_drag(&item.id, placement.section, placement.destination, cx);
        }
        self.clear_sidebar_drag(cx);
    }

    pub(super) fn clear_sidebar_drag(&mut self, cx: &mut Context<'_, Self>) {
        if self.sidebar_drag.take().is_some() {
            cx.notify();
        }
    }

    pub(super) fn sidebar_drag_over_shelf(&self, section: SidebarSection, cx: &App) -> bool {
        cx.has_active_drag()
            && self
                .sidebar_drag
                .as_ref()
                .and_then(|drag| drag.placement)
                .is_some_and(|placement| placement.section == section)
    }

    pub(in crate::views::workspace) fn cancel_sidebar_drag_on_escape(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if event.keystroke.key == "escape"
            && self.sidebar_drag.is_some()
            && cx.stop_active_drag(window)
        {
            self.clear_sidebar_drag(cx);
            window.prevent_default();
            cx.stop_propagation();
        }
    }

    pub(super) fn sidebar_drag_layout(&self, cx: &App) -> Option<DragLayout> {
        let drag = self
            .sidebar_drag
            .as_ref()
            .filter(|_| cx.has_active_drag())?;
        let viewport = self.sidebar_viewport.borrow();
        viewport.drag_layout(drag, ShelfEdges::new(&viewport.rows))
    }

    pub(super) fn sidebar_drag_overlays(
        &self,
        layout: DragLayout,
        window: &Window,
        cx: &App,
    ) -> Vec<AnyElement> {
        let Some(drag) = self.sidebar_drag.as_ref() else {
            return Vec::new();
        };
        let viewport = self.sidebar_viewport.borrow();
        let bounds = viewport.list.viewport_bounds();
        let projected = viewport.projected_top(drag, layout);
        let settled = if layout.empty_settled {
            viewport.empty_settled_edge(layout)
        } else {
            None
        };
        let labels = [
            (SidebarSection::Pinned, "Pinned", layout.pinned),
            (SidebarSection::Active, "Active", layout.active),
            (SidebarSection::Settled, "Settled", settled),
        ]
        .into_iter()
        .filter_map(|(section, label, top)| {
            let top = top?;
            let over = drag
                .placement
                .is_some_and(|placement| placement.section == section);
            let color = if over {
                theme::persimmon_400()
            } else {
                theme::rambutan_400()
            };
            Some(
                div()
                    .debug_selector(move || format!("desktop-sidebar-drop-shelf-{label}"))
                    .absolute()
                    .left(px(SIDEBAR_LIST_X_PADDING_PX + SIDEBAR_CARD_X_PADDING_PX))
                    .right(px(SIDEBAR_LIST_X_PADDING_PX))
                    .top(top - bounds.top())
                    .h(px(SHELF_LABEL_PX))
                    .flex()
                    .items_center()
                    .gap_2()
                    .text_size(px(SIDEBAR_LABEL_TEXT_PX))
                    .text_color(color)
                    .child(
                        div()
                            .debug_selector(move || format!("desktop-sidebar-drop-label-{label}"))
                            .h(px(SHELF_TEXT_PX))
                            .line_height(px(SHELF_TEXT_PX))
                            .flex_shrink_0()
                            .child(label),
                    )
                    .child(
                        div()
                            .debug_selector(move || format!("desktop-sidebar-drop-rule-{label}"))
                            .h(px(SHELF_RULE_PX))
                            .flex_1()
                            .bg(if over { color } else { theme::longan_700() }),
                    )
                    .into_any_element(),
            )
        });
        let card = projected.map(|(_, top)| {
            div()
                .absolute()
                .left(px(SIDEBAR_LIST_X_PADDING_PX))
                .top(top - bounds.top())
                .child(
                    drag.preview
                        .read(cx)
                        .card(Some(layout.card_height()), window, cx),
                )
                .into_any_element()
        });
        card.into_iter().chain(labels).collect()
    }
}

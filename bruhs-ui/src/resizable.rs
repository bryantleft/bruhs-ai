//! Panels side by side (or stacked) with draggable dividers between them.
//!
//! ```ignore
//! h_resizable("split")
//!     .with_state(&state)
//!     .child(resizable_panel().child(main))
//!     .child(resizable_panel().size(px(320.)).size_range(px(240.)..px(560.)).child(side))
//! ```
//!
//! A panel given a [`ResizablePanel::size`] is sized: it keeps the size its
//! [`ResizableState`] holds (the given size until a drag changes it), clamped
//! into its [`ResizablePanel::size_range`] on every frame, so a caller that
//! narrows the range moves it. Panels without a size share what is left.
//! Dragging a divider moves the boundary between its two neighbours; the
//! state emits [`ResizablePanelEvent::Resized`] when the drag ends.

use std::ops::Range;

use bruhs_models::num::ToFloatLossy as _;
use std::rc::Rc;

use gpui::prelude::*;
use gpui::{
    canvas, div, px, AnyElement, App, Axis, Bounds, Context, CursorStyle, ElementId, Entity,
    EventEmitter, MouseButton, MouseMoveEvent, MouseUpEvent, Pixels, Render, Window,
};

/// What a flexible panel keeps when a drag grows its neighbour.
const FLEX_PANEL_MIN_PX: f32 = 100.0;
/// The divider's hit band, centred on the boundary.
const HANDLE_BAND_PX: f32 = 9.0;
const HANDLE_PADDING_PX: f32 = 4.0;

/// How far the pointer has gone with a divider.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ResizeHandleState {
    /// The pointer is elsewhere.
    #[default]
    Idle,
    /// The pointer is over the divider's band.
    Hovered,
    /// The divider is being dragged.
    Dragging,
}

impl ResizeHandleState {
    pub fn is_active(self) -> bool {
        self != Self::Idle
    }
}

/// What a [`ResizeHandleRenderer`] is told about the divider it draws.
pub struct ResizeHandleContext {
    axis: Axis,
    state: ResizeHandleState,
}

impl ResizeHandleContext {
    /// The axis the divider resizes along: `Horizontal` for a vertical line
    /// between side-by-side panels.
    pub fn axis(&self) -> Axis {
        self.axis
    }

    pub fn state(&self) -> ResizeHandleState {
        self.state
    }

    pub fn is_active(&self) -> bool {
        self.state.is_active()
    }
}

/// Draws a divider's visible part, starting at the boundary. `None` keeps
/// the plain one-point line.
pub type ResizeHandleRenderer =
    Rc<dyn Fn(&ResizeHandleContext, &mut Window, &mut App) -> Option<AnyElement>>;

pub enum ResizablePanelEvent {
    /// A drag ended; [`ResizableState::sizes`] holds the panels' new sizes.
    Resized,
}

#[derive(Clone)]
struct Drag {
    handle: usize,
    start_pointer: Pixels,
    start_sizes: Vec<Pixels>,
    moved: bool,
}

/// The sizes of a panel group, kept across frames.
#[derive(Default)]
pub struct ResizableState {
    axis: Option<Axis>,
    sizes: Vec<Pixels>,
    sized: Vec<bool>,
    ranges: Vec<Range<Pixels>>,
    /// Each panel's size as the caller last gave it.
    requested: Vec<Option<Pixels>>,
    container: Bounds<Pixels>,
    drag: Option<Drag>,
    hovered_handle: Option<usize>,
}

impl EventEmitter<ResizablePanelEvent> for ResizableState {}

impl ResizableState {
    /// Each panel's size along the group's axis, in order.
    pub fn sizes(&self) -> &Vec<Pixels> {
        &self.sizes
    }

    /// Whether a divider is being dragged.
    pub fn is_resizing(&self) -> bool {
        self.drag.is_some()
    }

    fn along(&self, bounds: Bounds<Pixels>) -> (Pixels, Pixels) {
        match self.axis.unwrap_or(Axis::Horizontal) {
            Axis::Horizontal => (bounds.origin.x, bounds.size.width),
            Axis::Vertical => (bounds.origin.y, bounds.size.height),
        }
    }

    /// Sync with this frame's panels; `requested[i]` is panel i's given size.
    fn sync(&mut self, axis: Axis, requested: &[Option<Pixels>], ranges: &[Range<Pixels>]) {
        self.axis = Some(axis);
        if self.sizes.len() != requested.len() {
            self.sizes = requested
                .iter()
                .map(|size| size.unwrap_or_default())
                .collect();
            self.requested = requested.to_vec();
        }
        // A size the caller changes (a new zoom, a restored width) wins over
        // the last drag's, except during a drag.
        if self.drag.is_none() {
            for (ix, size) in requested.iter().enumerate() {
                if let Some(size) = size {
                    if self.requested.get(ix) != Some(&Some(*size)) {
                        self.sizes[ix] = *size;
                    }
                }
            }
        }
        self.requested = requested.to_vec();
        self.sized = requested.iter().map(Option::is_some).collect();
        self.ranges = ranges.to_vec();
        for (ix, range) in ranges.iter().enumerate() {
            if self.sized[ix] {
                self.sizes[ix] = clamp(self.sizes[ix], range);
            }
        }
        self.fit_flexible();
    }

    /// End a drag whose button came up somewhere this group no longer hears
    /// (it was hidden mid-drag).
    fn end_drag(&mut self, cx: &mut Context<'_, Self>) {
        if let Some(drag) = self.drag.take() {
            if drag.moved {
                cx.emit(ResizablePanelEvent::Resized);
            }
            cx.notify();
        }
    }

    /// Flexible panels share what the sized ones leave.
    fn fit_flexible(&mut self) {
        let (_, total) = self.along(self.container);
        let fixed: Pixels = (0..self.sizes.len())
            .filter(|ix| self.sized[*ix])
            .map(|ix| self.sizes[ix])
            .sum();
        let flexible = self.sized.iter().filter(|sized| !**sized).count();
        if flexible > 0 {
            let share = ((total - fixed).max(Pixels::ZERO)) / flexible.to_f32_lossy();
            for ix in 0..self.sizes.len() {
                if !self.sized[ix] {
                    self.sizes[ix] = share;
                }
            }
        }
    }

    /// The most a sized panel may take: the container less every other sized
    /// panel and a minimum for each flexible one.
    fn room_for(&self, ix: usize, sizes: &[Pixels]) -> Pixels {
        let (_, total) = self.along(self.container);
        let others: Pixels = (0..sizes.len())
            .filter(|other| *other != ix)
            .map(|other| {
                if self.sized[other] {
                    sizes[other]
                } else {
                    px(FLEX_PANEL_MIN_PX)
                }
            })
            .sum();
        (total - others).max(Pixels::ZERO)
    }

    fn drag_to(&mut self, pointer: Pixels) {
        let (before, after) = match &self.drag {
            Some(drag) => (drag.handle, drag.handle + 1),
            None => return,
        };
        // Between two flexible panels nothing can move.
        if !self.sized[before] && !self.sized[after] {
            return;
        }
        let Some(drag) = self.drag.as_mut() else {
            return;
        };
        drag.moved = true;
        let delta = pointer - drag.start_pointer;
        let (before, after) = (drag.handle, drag.handle + 1);
        let mut sizes = drag.start_sizes.clone();
        // The boundary moves by `delta`: the panel before it grows by that
        // much and the one after shrinks, within each sized panel's limits.
        let resize = |ix: usize, by: Pixels, sizes: &mut Vec<Pixels>, state: &Self| {
            let range = &state.ranges[ix];
            let room = state.room_for(ix, sizes);
            let limit = range.start..range.end.min(room).max(range.start);
            sizes[ix] = clamp(sizes[ix] + by, &limit);
        };
        match (self.sized[before], self.sized[after]) {
            (_, true) => resize(after, -delta, &mut sizes, self),
            (true, false) => resize(before, delta, &mut sizes, self),
            (false, false) => {}
        }
        self.sizes = sizes;
        self.fit_flexible();
    }
}

/// `size` within `range`; an inverted range keeps its start.
fn clamp(size: Pixels, range: &Range<Pixels>) -> Pixels {
    size.min(range.end).max(range.start)
}

/// A group of panels laid out along one axis.
#[derive(IntoElement)]
pub struct ResizablePanelGroup {
    id: ElementId,
    axis: Axis,
    state: Option<Entity<ResizableState>>,
    handle_renderer: Option<ResizeHandleRenderer>,
    panels: Vec<ResizablePanel>,
}

/// Side-by-side panels with vertical dividers.
pub fn h_resizable(id: impl Into<ElementId>) -> ResizablePanelGroup {
    ResizablePanelGroup::new(id, Axis::Horizontal)
}

/// Stacked panels with horizontal dividers.
pub fn v_resizable(id: impl Into<ElementId>) -> ResizablePanelGroup {
    ResizablePanelGroup::new(id, Axis::Vertical)
}

impl ResizablePanelGroup {
    fn new(id: impl Into<ElementId>, axis: Axis) -> Self {
        Self {
            id: id.into(),
            axis,
            state: None,
            handle_renderer: None,
            panels: Vec::new(),
        }
    }

    /// Keep the sizes in `state`, which the caller can read and subscribe to.
    pub fn with_state(mut self, state: &Entity<ResizableState>) -> Self {
        self.state = Some(state.clone());
        self
    }

    /// Draw the dividers with `renderer`.
    pub fn with_handle_appearance(mut self, renderer: ResizeHandleRenderer) -> Self {
        self.handle_renderer = Some(renderer);
        self
    }

    pub fn child(mut self, panel: ResizablePanel) -> Self {
        self.panels.push(panel);
        self
    }
}

/// The value a divider drag carries.
#[derive(Clone)]
struct DividerDrag;

struct DragGhost;

impl Render for DragGhost {
    fn render(&mut self, _: &mut Window, _: &mut Context<'_, Self>) -> impl IntoElement {
        gpui::Empty
    }
}

impl RenderOnce for ResizablePanelGroup {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let state = self.state.clone().unwrap_or_else(|| {
            window.use_keyed_state(self.id.clone(), cx, |_, _| ResizableState::default())
        });
        let axis = self.axis;
        let requested: Vec<_> = self.panels.iter().map(|panel| panel.size).collect();
        let ranges: Vec<_> = self
            .panels
            .iter()
            .map(|panel| panel.size_range.clone())
            .collect();
        // Syncing in render does not notify: nothing outside this frame
        // changed, and notifying here would redraw forever.
        state.update(cx, |state, _| state.sync(axis, &requested, &ranges));
        let read = state.read(cx);
        let sizes = read.sizes.clone();
        let dragging = read.drag.as_ref().map(|drag| drag.handle);
        let hovered = read.hovered_handle;

        let group_id = self.id.clone();
        let count = self.panels.len();
        let mut children: Vec<AnyElement> = Vec::with_capacity(count * 2);
        for (ix, panel) in self.panels.into_iter().enumerate() {
            let sized = panel.size.is_some();
            // The divider before this panel is its last child, on its leading
            // edge: drawn and hit-tested above both neighbours.
            let divider = (ix > 0).then(|| {
                let handle = ix - 1;
                let handle_state = if dragging == Some(handle) {
                    ResizeHandleState::Dragging
                } else if hovered == Some(handle) {
                    ResizeHandleState::Hovered
                } else {
                    ResizeHandleState::Idle
                };
                divider(
                    &group_id,
                    handle,
                    axis,
                    handle_state,
                    &state,
                    self.handle_renderer.clone(),
                    window,
                    cx,
                )
            });
            children.push(
                div()
                    .relative()
                    .min_w_0()
                    .min_h_0()
                    .map(|el| {
                        if sized {
                            let el = el.flex_none();
                            match axis {
                                Axis::Horizontal => el.h_full().w(sizes[ix]),
                                Axis::Vertical => el.w_full().h(sizes[ix]),
                            }
                        } else {
                            let el = el.flex_1();
                            match axis {
                                Axis::Horizontal => el.h_full(),
                                Axis::Vertical => el.w_full(),
                            }
                        }
                    })
                    .children(panel.children)
                    .children(divider)
                    .into_any_element(),
            );
        }

        let measure = state.clone();
        let moving = state.clone();
        let ending = state.clone();
        div()
            .id(group_id)
            .relative()
            .size_full()
            .flex()
            .map(|el| match axis {
                Axis::Horizontal => el.flex_row(),
                Axis::Vertical => el.flex_col(),
            })
            .children(children)
            // The container's size, for the flexible panels' share and the
            // sized panels' room.
            .child(
                canvas(
                    move |bounds, _, cx| {
                        measure.update(cx, |state, _| {
                            if state.container != bounds {
                                state.container = bounds;
                                state.fit_flexible();
                            }
                        });
                    },
                    move |_, _, window, _| {
                        // While a divider is held, every pointer move counts,
                        // wherever it goes (gpui's own drag reports a move
                        // only after the first).
                        let moving = moving.clone();
                        window.on_mouse_event(move |event: &MouseMoveEvent, _, _, cx| {
                            let pointer = match axis {
                                Axis::Horizontal => event.position.x,
                                Axis::Vertical => event.position.y,
                            };
                            moving.update(cx, |state, cx| {
                                if state.drag.is_none() {
                                    return;
                                }
                                // The button came up where this group could
                                // not hear it; the drag is over.
                                if event.pressed_button != Some(MouseButton::Left) {
                                    state.end_drag(cx);
                                    return;
                                }
                                state.drag_to(pointer);
                                cx.notify();
                            });
                        });
                        // A drag ends wherever the button comes up.
                        let ending = ending.clone();
                        window.on_mouse_event(move |event: &MouseUpEvent, _, _, cx| {
                            if event.button != MouseButton::Left {
                                return;
                            }
                            ending.update(cx, |state, cx| state.end_drag(cx));
                        });
                    },
                )
                .absolute()
                .size_full(),
            )
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "one call site; the arguments are the divider's whole context"
)]
fn divider(
    group: &ElementId,
    ix: usize,
    axis: Axis,
    handle_state: ResizeHandleState,
    state: &Entity<ResizableState>,
    renderer: Option<ResizeHandleRenderer>,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let context = ResizeHandleContext {
        axis,
        state: handle_state,
    };
    let look = renderer
        .and_then(|render| render(&context, window, cx))
        .unwrap_or_else(|| {
            let line = div().flex_none().bg(gpui::hsla(0.0, 0.0, 1.0, 0.1));
            match axis {
                Axis::Horizontal => line.w(px(1.)).h_full(),
                Axis::Vertical => line.h(px(1.)).w_full(),
            }
            .into_any_element()
        });
    let band = px(HANDLE_BAND_PX);
    let pad = px(HANDLE_PADDING_PX);
    let hover_state = state.clone();
    let press_state = state.clone();
    // The band straddles the boundary (its leading half over the panel
    // before); the look starts at the boundary.
    div()
        .id(ElementId::NamedChild(
            std::sync::Arc::new(group.clone()),
            format!("divider-{ix}").into(),
        ))
        .absolute()
        .flex()
        .occlude()
        .map(|el| match axis {
            Axis::Horizontal => el
                .top_0()
                .h_full()
                .left(-pad)
                .w(band)
                .pl(pad)
                .cursor(CursorStyle::ResizeLeftRight),
            Axis::Vertical => el
                .left_0()
                .w_full()
                .top(-pad)
                .h(band)
                .pt(pad)
                .flex_col()
                .cursor(CursorStyle::ResizeUpDown),
        })
        .on_hover(move |hovered, _, cx| {
            hover_state.update(cx, |state, cx| {
                // Leaving this divider clears only its own hover.
                let now = if *hovered {
                    Some(ix)
                } else if state.hovered_handle == Some(ix) {
                    None
                } else {
                    return;
                };
                if state.hovered_handle != now {
                    state.hovered_handle = now;
                    cx.notify();
                }
            });
        })
        .on_mouse_down(MouseButton::Left, move |event, _, cx| {
            cx.stop_propagation();
            press_state.update(cx, |state, cx| {
                let pointer = match axis {
                    Axis::Horizontal => event.position.x,
                    Axis::Vertical => event.position.y,
                };
                state.drag = Some(Drag {
                    handle: ix,
                    start_pointer: pointer,
                    start_sizes: state.sizes.clone(),
                    moved: false,
                });
                cx.notify();
            });
        })
        .on_drag(DividerDrag, |_, _, _, cx| cx.new(|_| DragGhost))
        .child(look)
        .into_any_element()
}

/// One panel of a group.
pub struct ResizablePanel {
    size: Option<Pixels>,
    size_range: Range<Pixels>,
    children: Vec<AnyElement>,
}

pub fn resizable_panel() -> ResizablePanel {
    ResizablePanel {
        size: None,
        size_range: Pixels::ZERO..Pixels::MAX,
        children: Vec::new(),
    }
}

impl ResizablePanel {
    /// Make the panel sized, starting at `size`.
    pub fn size(mut self, size: Pixels) -> Self {
        self.size = Some(size);
        self
    }

    /// The sizes a sized panel stays within.
    pub fn size_range(mut self, range: Range<Pixels>) -> Self {
        self.size_range = range;
        self
    }

    pub fn child(mut self, child: impl IntoElement) -> Self {
        self.children.push(child.into_any_element());
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{point, size, Modifiers, TestAppContext, VisualTestContext};

    struct Split {
        state: Entity<ResizableState>,
        range: Range<Pixels>,
        size: Pixels,
    }

    impl Render for Split {
        fn render(&mut self, _: &mut Window, _: &mut Context<'_, Self>) -> impl IntoElement {
            div().size_full().child(
                h_resizable("split")
                    .with_state(&self.state)
                    .child(resizable_panel().child(div().size_full()))
                    .child(
                        resizable_panel()
                            .size(self.size)
                            .size_range(self.range.clone())
                            .child(div().size_full()),
                    ),
            )
        }
    }

    fn split(
        cx: &mut TestAppContext,
        range: Range<Pixels>,
    ) -> (Entity<Split>, &mut VisualTestContext) {
        let (view, cx) = cx.add_window_view(move |_, cx| Split {
            state: cx.new(|_| ResizableState::default()),
            range,
            size: px(200.),
        });
        cx.simulate_resize(size(px(800.), px(400.)));
        cx.run_until_parked();
        (view, cx)
    }

    fn sizes(view: &Entity<Split>, cx: &mut VisualTestContext) -> Vec<Pixels> {
        cx.read(|cx| view.read(cx).state.read(cx).sizes().clone())
    }

    #[gpui::test]
    fn the_sized_panel_starts_at_its_size_and_the_other_fills_the_rest(cx: &mut TestAppContext) {
        let (view, cx) = split(cx, px(100.)..px(500.));
        assert_eq!(sizes(&view, cx), vec![px(600.), px(200.)]);
    }

    #[gpui::test]
    fn dragging_the_divider_resizes_and_reports_once_at_the_end(cx: &mut TestAppContext) {
        let (view, cx) = split(cx, px(100.)..px(500.));
        let state = cx.read(|cx| view.read(cx).state.clone());
        let resized = Rc::new(std::cell::Cell::new(0));
        let count = resized.clone();
        cx.update(|_, cx| {
            cx.subscribe(&state, move |_, _: &ResizablePanelEvent, _| {
                count.set(count.get() + 1)
            })
            .detach();
        });
        cx.simulate_mouse_down(
            point(px(600.), px(100.)),
            MouseButton::Left,
            Modifiers::none(),
        );
        cx.simulate_mouse_move(
            point(px(550.), px(100.)),
            Some(MouseButton::Left),
            Modifiers::none(),
        );
        cx.simulate_mouse_move(
            point(px(500.), px(100.)),
            Some(MouseButton::Left),
            Modifiers::none(),
        );
        assert!(cx.read(|cx| state.read(cx).is_resizing()));
        cx.simulate_mouse_up(
            point(px(500.), px(100.)),
            MouseButton::Left,
            Modifiers::none(),
        );
        cx.run_until_parked();
        assert_eq!(sizes(&view, cx), vec![px(500.), px(300.)]);
        assert!(!cx.read(|cx| state.read(cx).is_resizing()));
        assert_eq!(resized.get(), 1);
    }

    #[gpui::test]
    fn a_drag_stops_at_the_range(cx: &mut TestAppContext) {
        let (view, cx) = split(cx, px(100.)..px(250.));
        cx.simulate_mouse_down(
            point(px(600.), px(100.)),
            MouseButton::Left,
            Modifiers::none(),
        );
        cx.simulate_mouse_move(
            point(px(200.), px(100.)),
            Some(MouseButton::Left),
            Modifiers::none(),
        );
        cx.simulate_mouse_up(
            point(px(200.), px(100.)),
            MouseButton::Left,
            Modifiers::none(),
        );
        cx.run_until_parked();
        assert_eq!(sizes(&view, cx)[1], px(250.));
    }

    #[gpui::test]
    fn a_narrowed_range_moves_the_panel(cx: &mut TestAppContext) {
        let (view, cx) = split(cx, px(100.)..px(500.));
        view.update(cx, |split, cx| {
            split.range = px(320.)..px(320.);
            cx.notify();
        });
        cx.run_until_parked();
        assert_eq!(sizes(&view, cx), vec![px(480.), px(320.)]);
    }

    #[gpui::test]
    fn a_drag_whose_button_came_up_unheard_ends_at_the_next_move(cx: &mut TestAppContext) {
        let (view, cx) = split(cx, px(100.)..px(500.));
        cx.simulate_mouse_down(
            point(px(600.), px(100.)),
            MouseButton::Left,
            Modifiers::none(),
        );
        cx.simulate_mouse_move(
            point(px(580.), px(100.)),
            Some(MouseButton::Left),
            Modifiers::none(),
        );
        cx.simulate_mouse_move(
            point(px(560.), px(100.)),
            Some(MouseButton::Left),
            Modifiers::none(),
        );
        // No mouse up reaches the group; the next move has no button held.
        cx.simulate_mouse_move(point(px(300.), px(100.)), None, Modifiers::none());
        cx.run_until_parked();
        let state = cx.read(|cx| view.read(cx).state.clone());
        assert!(!cx.read(|cx| state.read(cx).is_resizing()));
        assert_eq!(sizes(&view, cx)[1], px(240.));
    }

    #[gpui::test]
    fn a_size_the_caller_changes_takes_effect(cx: &mut TestAppContext) {
        let (view, cx) = split(cx, px(100.)..px(500.));
        view.update(cx, |split, cx| {
            split.size = px(260.);
            cx.notify();
        });
        cx.run_until_parked();
        assert_eq!(sizes(&view, cx)[1], px(260.));
    }
}

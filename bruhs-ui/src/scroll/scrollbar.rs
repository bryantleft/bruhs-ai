//! The scrollbar element: a thumb over the edge of a scroll viewport that the
//! primary button drags, and a track a click jumps along.
//!
//! The scrollbar is window chrome, so its sizes are window points at any page
//! zoom, as a browser keeps its own scrollbars.

use std::cell::Cell;
use std::panic::Location;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui::{
    fill, point, px, size, App, Axis, Bounds, ContentMask, Element, ElementId, Global,
    GlobalElementId, Hitbox, HitboxBehavior, Hsla, InspectorElementId, IntoElement, LayoutId,
    ListState, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, Point, Position,
    ScrollHandle, Size, Style, UniformListScrollHandle, Window,
};

/// The strip along the viewport's edge that takes the pointer.
const TRACK_PX: f32 = 16.0;
/// The thumb at rest, and while hovered or dragged.
const THUMB_PX: f32 = 6.0;
const THUMB_ACTIVE_PX: f32 = 8.0;
/// The gap between the thumb and the viewport's edge.
const THUMB_INSET_PX: f32 = 4.0;
const MIN_THUMB_PX: f32 = 48.0;
/// How long a [`ScrollbarMode::Scrolling`] bar stays after the content moves.
const SCROLLING_LINGER: Duration = Duration::from_secs(2);

/// Something a scrollbar can read and move.
pub trait ScrollbarHandle: 'static {
    /// The viewport the scrollbar overlays, in window coordinates.
    fn viewport_bounds(&self) -> Bounds<Pixels>;
    /// The scroll offset: zero at the start, negative as the content moves up
    /// or left.
    fn offset(&self) -> Point<Pixels>;
    fn set_offset(&self, offset: Point<Pixels>);
    /// The content's whole size.
    fn content_size(&self) -> Size<Pixels>;
    /// The thumb was grabbed.
    fn start_drag(&self) {}
    /// The thumb was let go.
    fn end_drag(&self) {}
}

impl ScrollbarHandle for ScrollHandle {
    fn viewport_bounds(&self) -> Bounds<Pixels> {
        self.bounds()
    }
    fn offset(&self) -> Point<Pixels> {
        ScrollHandle::offset(self)
    }
    fn set_offset(&self, offset: Point<Pixels>) {
        ScrollHandle::set_offset(self, offset);
    }
    fn content_size(&self) -> Size<Pixels> {
        let max = self.max_offset();
        let viewport = self.bounds().size;
        size(viewport.width + max.x, viewport.height + max.y)
    }
}

impl ScrollbarHandle for UniformListScrollHandle {
    fn viewport_bounds(&self) -> Bounds<Pixels> {
        self.0.borrow().base_handle.bounds()
    }
    fn offset(&self) -> Point<Pixels> {
        self.0.borrow().base_handle.offset()
    }
    fn set_offset(&self, offset: Point<Pixels>) {
        self.0.borrow_mut().base_handle.set_offset(offset);
    }
    fn content_size(&self) -> Size<Pixels> {
        ScrollbarHandle::content_size(&self.0.borrow().base_handle)
    }
}

impl ScrollbarHandle for ListState {
    fn viewport_bounds(&self) -> Bounds<Pixels> {
        ListState::viewport_bounds(self)
    }
    fn offset(&self) -> Point<Pixels> {
        self.scroll_px_offset_for_scrollbar()
    }
    fn set_offset(&self, offset: Point<Pixels>) {
        self.set_offset_from_scrollbar(offset);
    }
    fn content_size(&self) -> Size<Pixels> {
        let max = self.max_offset_for_scrollbar();
        let viewport = ListState::viewport_bounds(self).size;
        size(viewport.width + max.x, viewport.height + max.y)
    }
    fn start_drag(&self) {
        self.scrollbar_drag_started();
    }
    fn end_drag(&self) {
        self.scrollbar_drag_ended();
    }
}

/// Which edges carry a scrollbar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScrollbarAxis {
    Vertical,
    Horizontal,
    Both,
}

impl ScrollbarAxis {
    pub fn has_vertical(self) -> bool {
        matches!(self, Self::Vertical | Self::Both)
    }
    pub fn has_horizontal(self) -> bool {
        matches!(self, Self::Horizontal | Self::Both)
    }
}

impl From<Axis> for ScrollbarAxis {
    fn from(axis: Axis) -> Self {
        match axis {
            Axis::Vertical => Self::Vertical,
            Axis::Horizontal => Self::Horizontal,
        }
    }
}

/// When the thumb shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ScrollbarMode {
    /// While the content scrolls, and for a moment after.
    Scrolling,
    /// While the pointer is over the viewport.
    Hover,
    /// Whenever the content overflows.
    #[default]
    Always,
}

/// The look every scrollbar shares. Set it with [`set_scrollbar_style`];
/// [`crate::theme`] does for the bruhs theme.
#[derive(Clone, Copy, Debug)]
pub struct ScrollbarStyle {
    pub mode: ScrollbarMode,
    pub thumb: Hsla,
    pub thumb_active: Hsla,
}

impl Default for ScrollbarStyle {
    fn default() -> Self {
        Self {
            mode: ScrollbarMode::Always,
            thumb: gpui::hsla(0.0, 0.0, 0.5, 0.6),
            thumb_active: gpui::hsla(0.0, 0.0, 0.6, 0.8),
        }
    }
}

impl Global for ScrollbarStyle {}

pub fn set_scrollbar_style(style: ScrollbarStyle, cx: &mut App) {
    cx.set_global(style);
}

pub fn scrollbar_style(cx: &App) -> ScrollbarStyle {
    cx.try_global::<ScrollbarStyle>()
        .copied()
        .unwrap_or_default()
}

/// A scrollbar over a viewport. Put it last in the viewport's parent (an
/// absolute overlay), or let [`super::ScrollableElement`] place it.
pub struct Scrollbar {
    id: ElementId,
    axis: ScrollbarAxis,
    handle: Rc<dyn ScrollbarHandle>,
    mode: Option<ScrollbarMode>,
    /// Overlay the element's own layout bounds instead of the handle's
    /// viewport (the scrollable wrapper's case).
    from_layout: bool,
}

impl Scrollbar {
    /// Bars on both edges, where the content overflows.
    #[track_caller]
    pub fn new<H: ScrollbarHandle + Clone>(handle: &H) -> Self {
        Self {
            id: ElementId::CodeLocation(*Location::caller()),
            axis: ScrollbarAxis::Both,
            handle: Rc::new(handle.clone()),
            mode: None,
            from_layout: false,
        }
    }

    #[track_caller]
    pub fn vertical<H: ScrollbarHandle + Clone>(handle: &H) -> Self {
        Self::new(handle).axis(ScrollbarAxis::Vertical)
    }

    #[track_caller]
    pub fn horizontal<H: ScrollbarHandle + Clone>(handle: &H) -> Self {
        Self::new(handle).axis(ScrollbarAxis::Horizontal)
    }

    /// Set the id that keeps its drag and hover state across frames; one is
    /// derived from the call site otherwise.
    pub fn id(mut self, id: impl Into<ElementId>) -> Self {
        self.id = id.into();
        self
    }

    pub fn axis(mut self, axis: impl Into<ScrollbarAxis>) -> Self {
        self.axis = axis.into();
        self
    }

    /// When the thumb shows, overriding [`ScrollbarStyle::mode`].
    pub fn mode(mut self, mode: ScrollbarMode) -> Self {
        self.mode = Some(mode);
        self
    }

    pub(super) fn viewport_from_layout(mut self) -> Self {
        self.from_layout = true;
        self
    }
}

#[derive(Clone, Copy, Default)]
struct Inner {
    /// The axis being dragged, and where on the thumb it was grabbed.
    drag: Option<(Axis, Pixels)>,
    hovered_thumb: Option<Axis>,
    pointer_inside: bool,
    last_offset: Point<Pixels>,
    last_scroll: Option<Instant>,
}

#[derive(Clone, Default)]
struct ScrollbarState(Rc<Cell<Inner>>);

impl ScrollbarState {
    fn get(&self) -> Inner {
        self.0.get()
    }
    fn update(&self, f: impl FnOnce(&mut Inner)) {
        let mut inner = self.0.get();
        f(&mut inner);
        self.0.set(inner);
    }
}

#[derive(Clone, Copy)]
struct Bar {
    axis: Axis,
    track: Bounds<Pixels>,
    thumb_len: Pixels,
    thumb_start: Pixels,
    /// How far the content scrolls on this axis.
    max_offset: Pixels,
}

impl Bar {
    /// The run the thumb slides along: the track less an inset at each end.
    fn inner_len(&self) -> Pixels {
        (along(self.track.size, self.axis) - px(THUMB_INSET_PX) * 2.0).max(Pixels::ZERO)
    }

    fn travel(&self) -> Pixels {
        self.inner_len() - self.thumb_len
    }

    fn thumb(&self, active: bool) -> Bounds<Pixels> {
        let thick = px(if active { THUMB_ACTIVE_PX } else { THUMB_PX });
        let inset = px(THUMB_INSET_PX);
        match self.axis {
            Axis::Vertical => Bounds::new(
                point(
                    self.track.right() - inset - thick,
                    self.track.top() + inset + self.thumb_start,
                ),
                size(thick, self.thumb_len),
            ),
            Axis::Horizontal => Bounds::new(
                point(
                    self.track.left() + inset + self.thumb_start,
                    self.track.bottom() - inset - thick,
                ),
                size(self.thumb_len, thick),
            ),
        }
    }

    /// The offset that puts the thumb's grab point under `pointer`.
    fn offset_at(&self, pointer: Point<Pixels>, grab: Pixels) -> Pixels {
        let travel = self.travel();
        if travel <= Pixels::ZERO {
            return Pixels::ZERO;
        }
        let start = along_point(self.track.origin, self.axis) + px(THUMB_INSET_PX);
        let ratio = ((along_point(pointer, self.axis) - start - grab) / travel).clamp(0.0, 1.0);
        -(self.max_offset * ratio)
    }
}

fn along(size: Size<Pixels>, axis: Axis) -> Pixels {
    match axis {
        Axis::Vertical => size.height,
        Axis::Horizontal => size.width,
    }
}

fn along_point(point: Point<Pixels>, axis: Axis) -> Pixels {
    match axis {
        Axis::Vertical => point.y,
        Axis::Horizontal => point.x,
    }
}

fn with_axis(offset: Point<Pixels>, axis: Axis, value: Pixels) -> Point<Pixels> {
    match axis {
        Axis::Vertical => point(offset.x, value),
        Axis::Horizontal => point(value, offset.y),
    }
}

/// The bars a viewport needs: one per requested axis whose content overflows.
fn bars(
    viewport: Bounds<Pixels>,
    content: Size<Pixels>,
    offset: Point<Pixels>,
    axis: ScrollbarAxis,
) -> Vec<Bar> {
    let overflows = |axis: Axis| along(content, axis) > along(viewport.size, axis) + px(0.5);
    let vertical = axis.has_vertical() && overflows(Axis::Vertical);
    let horizontal = axis.has_horizontal() && overflows(Axis::Horizontal);
    let track = px(TRACK_PX);
    let mut out = Vec::new();
    for (shown, bar_axis) in [(vertical, Axis::Vertical), (horizontal, Axis::Horizontal)] {
        if !shown {
            continue;
        }
        // When both show, each stops short of the other's corner.
        let corner = if vertical && horizontal {
            track
        } else {
            Pixels::ZERO
        };
        let bounds = match bar_axis {
            Axis::Vertical => Bounds::new(
                point(viewport.right() - track, viewport.top()),
                size(track, viewport.size.height - corner),
            ),
            Axis::Horizontal => Bounds::new(
                point(viewport.left(), viewport.bottom() - track),
                size(viewport.size.width - corner, track),
            ),
        };
        let track_len = (along(bounds.size, bar_axis) - px(THUMB_INSET_PX) * 2.0).max(Pixels::ZERO);
        let visible = along(viewport.size, bar_axis);
        let whole = along(content, bar_axis);
        let max_offset = (whole - visible).max(Pixels::ZERO);
        let thumb_len = (track_len * (visible / whole))
            .max(px(MIN_THUMB_PX))
            .min(track_len);
        let scrolled = (-along_point(offset, bar_axis)).clamp(Pixels::ZERO, max_offset);
        let thumb_start = if max_offset > Pixels::ZERO {
            (track_len - thumb_len) * (scrolled / max_offset)
        } else {
            Pixels::ZERO
        };
        out.push(Bar {
            axis: bar_axis,
            track: bounds,
            thumb_len,
            thumb_start,
            max_offset,
        });
    }
    out
}

pub struct PrepaintState {
    viewport: Bounds<Pixels>,
    bars: Vec<Bar>,
    hitboxes: Vec<Hitbox>,
    state: ScrollbarState,
}

impl IntoElement for Scrollbar {
    type Element = Self;
    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for Scrollbar {
    type RequestLayoutState = ();
    type PrepaintState = PrepaintState;

    fn id(&self) -> Option<ElementId> {
        Some(self.id.clone())
    }

    fn source_location(&self) -> Option<&'static Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        // Never part of the content's size. Overlaying its own layout, it
        // fills its (absolute) parent. Inside a scroll container it reads the
        // viewport from the handle and takes no room at all: an absolute box
        // with no inset sits after the content and would lengthen it.
        let mut style = Style {
            position: Position::Absolute,
            ..Style::default()
        };
        style.inset.top = gpui::Length::Definite(Pixels::ZERO.into());
        style.inset.left = gpui::Length::Definite(Pixels::ZERO.into());
        if self.from_layout {
            style.size.width = gpui::relative(1.).into();
            style.size.height = gpui::relative(1.).into();
        }
        (window.request_layout(style, None, cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) -> PrepaintState {
        let viewport = if self.from_layout {
            bounds
        } else {
            self.handle.viewport_bounds()
        };
        let state = window
            .use_keyed_state(self.id.clone(), cx, |_, _| ScrollbarState::default())
            .read(cx)
            .clone();
        let offset = self.handle.offset();
        if offset != state.get().last_offset {
            state.update(|inner| {
                inner.last_offset = offset;
                inner.last_scroll = Some(Instant::now());
            });
        }
        let bars = bars(viewport, self.handle.content_size(), offset, self.axis);
        let hitboxes = window.with_content_mask(Some(ContentMask { bounds: viewport }), |window| {
            bars.iter()
                .map(|bar| window.insert_hitbox(bar.track, HitboxBehavior::Normal))
                .collect()
        });
        PrepaintState {
            viewport,
            bars,
            hitboxes,
            state,
        }
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut (),
        prepaint: &mut PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        if prepaint.bars.is_empty() {
            return;
        }
        let style = scrollbar_style(cx);
        let mode = self.mode.unwrap_or(style.mode);
        let inner = prepaint.state.get();
        let viewport = prepaint.viewport;
        let mouse = window.mouse_position();
        let recently_scrolled = inner
            .last_scroll
            .is_some_and(|at| at.elapsed() < SCROLLING_LINGER);
        let shown = match mode {
            ScrollbarMode::Always => true,
            ScrollbarMode::Hover => inner.drag.is_some() || viewport.contains(&mouse),
            ScrollbarMode::Scrolling => {
                inner.drag.is_some()
                    || recently_scrolled
                    || prepaint
                        .hitboxes
                        .iter()
                        .any(|hitbox| hitbox.is_hovered(window))
            }
        };
        if mode == ScrollbarMode::Scrolling && recently_scrolled {
            // Draw again until the linger runs out, then the bar hides.
            window.request_animation_frame();
        }

        window.with_content_mask(Some(ContentMask { bounds: viewport }), |window| {
            for (bar, hitbox) in prepaint.bars.iter().zip(&prepaint.hitboxes) {
                let active = inner.drag.is_some_and(|(axis, _)| axis == bar.axis)
                    || hitbox.is_hovered(window);
                if shown {
                    let color = if active {
                        style.thumb_active
                    } else {
                        style.thumb
                    };
                    let thumb = bar.thumb(active);
                    // A pill, like the web's `border-radius: 8px` thumb.
                    let radius = thumb.size.width.min(thumb.size.height) / 2.0;
                    window.paint_quad(fill(thumb, color).corner_radii(radius));
                }
            }
        });

        let handle = self.handle.clone();
        let bars = prepaint.bars.clone();
        let hitboxes = prepaint.hitboxes.clone();
        let state = prepaint.state.clone();
        window.on_mouse_event({
            let (handle, bars, hitboxes, state) = (
                handle.clone(),
                bars.clone(),
                hitboxes.clone(),
                state.clone(),
            );
            move |event: &MouseDownEvent, phase, window, cx| {
                // Only the primary button drags or jumps; a right click is the
                // context menu's.
                if !phase.bubble() || event.button != MouseButton::Left {
                    return;
                }
                let Some((bar, _)) = bars
                    .iter()
                    .zip(&hitboxes)
                    .find(|(_, hitbox)| hitbox.is_hovered(window))
                else {
                    return;
                };
                cx.stop_propagation();
                let thumb = bar.thumb(true);
                let grab = if thumb.contains(&event.position) {
                    along_point(event.position, bar.axis) - along_point(thumb.origin, bar.axis)
                } else {
                    // A click on the track centres the thumb there.
                    bar.thumb_len / 2.0
                };
                let offset = handle.offset();
                handle.set_offset(with_axis(
                    offset,
                    bar.axis,
                    bar.offset_at(event.position, grab),
                ));
                handle.start_drag();
                state.update(|inner| inner.drag = Some((bar.axis, grab)));
                window.refresh();
            }
        });
        window.on_mouse_event({
            let (handle, bars, state) = (handle.clone(), bars.clone(), state.clone());
            move |event: &MouseMoveEvent, _, window, _| {
                let inner = state.get();
                if let Some((axis, grab)) = inner.drag {
                    if let Some(bar) = bars.iter().find(|bar| bar.axis == axis) {
                        let offset = handle.offset();
                        handle.set_offset(with_axis(
                            offset,
                            axis,
                            bar.offset_at(event.position, grab),
                        ));
                        window.refresh();
                    }
                    return;
                }
                let hovered = bars
                    .iter()
                    .zip(&hitboxes)
                    .find(|(_, hitbox)| hitbox.is_hovered(window))
                    .map(|(bar, _)| bar.axis);
                let inside = viewport.contains(&event.position);
                if hovered != inner.hovered_thumb || inside != inner.pointer_inside {
                    state.update(|inner| {
                        inner.hovered_thumb = hovered;
                        inner.pointer_inside = inside;
                    });
                    window.refresh();
                }
            }
        });
        window.on_mouse_event(move |event: &MouseUpEvent, _, window, _| {
            if event.button != MouseButton::Left {
                return;
            }
            let Some((axis, grab)) = state.get().drag else {
                return;
            };
            // Apply where the button came up before letting go, so a fast
            // flick ends exactly under the pointer.
            if let Some(bar) = bars.iter().find(|bar| bar.axis == axis) {
                let offset = handle.offset();
                handle.set_offset(with_axis(offset, axis, bar.offset_at(event.position, grab)));
            }
            handle.end_drag();
            state.update(|inner| inner.drag = None);
            window.refresh();
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn viewport() -> Bounds<Pixels> {
        Bounds::new(point(px(0.), px(0.)), size(px(200.), px(100.)))
    }

    #[test]
    fn content_that_fits_gets_no_bar() {
        let fits = bars(
            viewport(),
            size(px(200.), px(100.)),
            Point::default(),
            ScrollbarAxis::Both,
        );
        assert!(fits.is_empty());
    }

    #[test]
    fn the_thumb_is_sized_by_the_visible_share_and_placed_by_the_offset() {
        let bar = bars(
            viewport(),
            size(px(200.), px(400.)),
            point(px(0.), px(-150.)),
            ScrollbarAxis::Vertical,
        )[0];
        assert_eq!(bar.axis, Axis::Vertical);
        // A quarter of the content is visible.
        assert_eq!(bar.thumb_len, px(48.).max(px(25.)));
        assert_eq!(bar.max_offset, px(300.));
        // Halfway down the content, halfway along the travel (the track
        // less its two insets).
        let run = px(100. - THUMB_INSET_PX * 2.);
        assert_eq!(bar.thumb_start, (run - bar.thumb_len) * 0.5);
    }

    #[test]
    fn dragging_to_the_track_end_reaches_the_last_row() {
        let bar = bars(
            viewport(),
            size(px(200.), px(400.)),
            Point::default(),
            ScrollbarAxis::Vertical,
        )[0];
        assert_eq!(
            bar.offset_at(point(px(195.), px(100.)), bar.thumb_len),
            px(-300.)
        );
        assert_eq!(
            bar.offset_at(point(px(195.), px(0.)), Pixels::ZERO),
            Pixels::ZERO
        );
    }

    #[test]
    fn two_bars_leave_each_other_the_corner() {
        let both = bars(
            viewport(),
            size(px(400.), px(400.)),
            Point::default(),
            ScrollbarAxis::Both,
        );
        assert_eq!(both.len(), 2);
        assert_eq!(both[0].track.size.height, px(100. - TRACK_PX));
        assert_eq!(both[1].track.size.width, px(200. - TRACK_PX));
    }
}

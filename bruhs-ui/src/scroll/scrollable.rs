//! Scroll containers with scrollbars: `.overflow_y_scrollbar()` and friends
//! turn an element into a scroll area, and `.vertical_scrollbar(&handle)`
//! overlays a bar on one that already scrolls.

use std::panic::Location;
use std::rc::Rc;

use gpui::prelude::*;
use gpui::{
    div, AnyElement, App, Div, Element, ElementId, Overflow, PointRefinement, ScrollEvent,
    ScrollHandle, Stateful, StyleRefinement, Window,
};

use super::scrollbar::{Scrollbar, ScrollbarAxis, ScrollbarHandle};

/// Scrollbars for an element.
pub trait ScrollableElement: InteractiveElement + Styled + ParentElement + Element {
    /// Overlay a bar on each requested edge of an element that already
    /// scrolls with `handle` (`overflow_*_scroll` and `track_scroll`).
    #[track_caller]
    fn scrollbar<H: ScrollbarHandle + Clone>(
        self,
        handle: &H,
        axis: impl Into<ScrollbarAxis>,
    ) -> Self
    where
        Self: Sized,
    {
        let id = ElementId::CodeLocation(*Location::caller());
        self.child(Scrollbar::new(handle).id(id).axis(axis))
    }

    #[track_caller]
    fn vertical_scrollbar<H: ScrollbarHandle + Clone>(self, handle: &H) -> Self
    where
        Self: Sized,
    {
        self.scrollbar(handle, ScrollbarAxis::Vertical)
    }

    #[track_caller]
    fn horizontal_scrollbar<H: ScrollbarHandle + Clone>(self, handle: &H) -> Self
    where
        Self: Sized,
    {
        self.scrollbar(handle, ScrollbarAxis::Horizontal)
    }

    /// Scroll on both axes, with bars. The element stays the container its
    /// children are laid out in.
    #[track_caller]
    fn overflow_scrollbar(self) -> Scrollable<Self>
    where
        Self: Sized,
    {
        Scrollable::new(self, ScrollbarAxis::Both)
    }

    #[track_caller]
    fn overflow_x_scrollbar(self) -> Scrollable<Self>
    where
        Self: Sized,
    {
        Scrollable::new(self, ScrollbarAxis::Horizontal)
    }

    #[track_caller]
    fn overflow_y_scrollbar(self) -> Scrollable<Self>
    where
        Self: Sized,
    {
        Scrollable::new(self, ScrollbarAxis::Vertical)
    }
}

impl ScrollableElement for Div {}
impl ScrollableElement for Stateful<Div> {}

/// An element made into a scroll area, with its scrollbars.
#[derive(IntoElement)]
pub struct Scrollable<E: InteractiveElement + Styled + ParentElement + Element> {
    id: ElementId,
    element: E,
    axis: ScrollbarAxis,
}

impl<E: InteractiveElement + Styled + ParentElement + Element> Scrollable<E> {
    #[track_caller]
    fn new(element: E, axis: ScrollbarAxis) -> Self {
        Self {
            id: ElementId::CodeLocation(*Location::caller()),
            element,
            axis,
        }
    }

    /// Set the id that keeps the scroll position across frames. Needed only
    /// when one call site draws several scroll areas.
    pub fn id(mut self, id: impl Into<ElementId>) -> Self {
        self.id = id.into();
        self
    }
}

impl<E: InteractiveElement + Styled + ParentElement + Element> Styled for Scrollable<E> {
    fn style(&mut self) -> &mut StyleRefinement {
        self.element.style()
    }
}

impl<E: InteractiveElement + Styled + ParentElement + Element> ParentElement for Scrollable<E> {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.element.extend(elements);
    }
}

impl<E: InteractiveElement + Styled + ParentElement + Element> InteractiveElement
    for Scrollable<E>
{
    fn interactivity(&mut self) -> &mut gpui::Interactivity {
        self.element.interactivity()
    }
}

/// The wrapper takes the element's place in its parent's layout: its size
/// limits, flex sizing and alignment. It clips the scrolled axis so that, as
/// a flex item, it shrinks to its parent instead of growing to its content
/// (no `min_h_0()` needed at the call site).
fn outer_style(element: &mut impl Styled, axis: ScrollbarAxis) -> StyleRefinement {
    let style = element.style();
    StyleRefinement {
        size: style.size.clone(),
        min_size: style.min_size.clone(),
        max_size: style.max_size.clone(),
        flex_grow: style.flex_grow,
        flex_shrink: style.flex_shrink,
        flex_basis: style.flex_basis,
        align_self: style.align_self,
        overflow: PointRefinement {
            x: axis.has_horizontal().then_some(Overflow::Hidden),
            y: axis.has_vertical().then_some(Overflow::Hidden),
        },
        ..Default::default()
    }
}

impl<E> RenderOnce for Scrollable<E>
where
    E: InteractiveElement + Styled + ParentElement + Element + 'static,
{
    fn render(mut self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let handle = window
            .use_keyed_state(self.id.clone(), cx, |_, _| ScrollHandle::default())
            .read(cx)
            .clone();
        let outer = outer_style(&mut self.element, self.axis);
        let axis = self.axis;

        // The caller's element is the scrolled content: as long as its
        // content on the scrolled axis, at least as long as the area.
        let mut content = self
            .element
            .id((self.id.clone(), "content"))
            .flex_none()
            .map(|el| match axis {
                ScrollbarAxis::Vertical => el.h_auto().min_h_full(),
                ScrollbarAxis::Horizontal => el.w_auto().min_w_full(),
                ScrollbarAxis::Both => el.size_auto().min_size_full(),
            });
        // Its maximum size belongs to the wrapper; on the content it would
        // stop it overflowing, so nothing would scroll.
        let max = &mut content.style().max_size;
        match axis {
            ScrollbarAxis::Vertical => max.height = None,
            ScrollbarAxis::Horizontal => max.width = None,
            ScrollbarAxis::Both => *max = Default::default(),
        }

        let area = div()
            .id((self.id.clone(), "area"))
            .size_full()
            .flex()
            .track_scroll(&handle)
            .map(|el| match axis {
                ScrollbarAxis::Vertical => el.flex_col().overflow_y_scroll(),
                ScrollbarAxis::Horizontal => el.flex_row().overflow_x_scroll(),
                ScrollbarAxis::Both => el.overflow_scroll(),
            })
            // Otherwise gpui maps the other axis' delta onto a single-axis
            // area, and a sideways swipe scrolls it vertically.
            .when(axis != ScrollbarAxis::Both, |el| {
                el.restrict_scroll_to_axis()
            })
            .child(content);

        let wheel = Rc::new(handle.clone());
        div()
            .id(self.id.clone())
            .size_full()
            .refine_style(&outer)
            .relative()
            .flex()
            .map(|el| match axis {
                ScrollbarAxis::Horizontal => el.flex_row(),
                _ => el.flex_col(),
            })
            .child(area)
            // A plain mouse wheel only moves vertically; over a horizontal
            // area it scrolls sideways, as in a browser.
            .when(axis == ScrollbarAxis::Horizontal, |el| {
                el.on_scroll(move |event: &ScrollEvent, window, cx| {
                    let delta = event.delta.pixel_delta(window.line_height());
                    if delta.x != gpui::Pixels::ZERO || delta.y == gpui::Pixels::ZERO {
                        return;
                    }
                    let offset = ScrollbarHandle::offset(wheel.as_ref());
                    let max = wheel.max_offset().x;
                    let x = (offset.x + delta.y).clamp(-max, gpui::Pixels::ZERO);
                    if x != offset.x {
                        ScrollbarHandle::set_offset(wheel.as_ref(), gpui::point(x, offset.y));
                        cx.stop_propagation();
                        window.refresh();
                    }
                })
            })
            .child(
                div().absolute().inset_0().child(
                    Scrollbar::new(&handle)
                        .id((self.id.clone(), "scrollbar"))
                        .axis(axis)
                        .viewport_from_layout(),
                ),
            )
    }
}

trait RefineStyle: Styled + Sized {
    fn refine_style(mut self, style: &StyleRefinement) -> Self {
        self.style().refine(style);
        self
    }
}

impl<T: Styled> RefineStyle for T {}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{px, size, Context, ScrollDelta, TestAppContext};

    struct List;

    impl Render for List {
        fn render(&mut self, _: &mut Window, _: &mut Context<'_, Self>) -> impl IntoElement {
            div().size_full().child(
                div()
                    .id("list")
                    .debug_selector(|| "list".into())
                    .w(px(200.))
                    .max_h(px(100.))
                    .overflow_y_scrollbar()
                    .children((0..20).map(|ix| {
                        div()
                            .h(px(20.))
                            .debug_selector(move || format!("row-{ix}"))
                            .child(format!("Row {ix}"))
                    })),
            )
        }
    }

    #[gpui::test]
    fn the_wheel_scrolls_an_area_capped_by_its_max_height(cx: &mut TestAppContext) {
        let (_, cx) = cx.add_window_view(|_, _| List);
        cx.simulate_resize(size(px(400.), px(400.)));
        cx.run_until_parked();
        let before = cx.debug_bounds("row-19").unwrap();
        let list = cx.debug_bounds("list").unwrap();
        cx.simulate_event(ScrollEvent {
            position: gpui::point(px(50.), px(50.)),
            delta: ScrollDelta::Pixels(gpui::point(px(0.), px(-1000.))),
            ..Default::default()
        });
        cx.run_until_parked();
        let after = cx.debug_bounds("row-19").unwrap();
        assert!(
            after.top() < before.top(),
            "{before:?} -> {after:?}, list {list:?}"
        );
        assert_eq!(after.bottom(), px(100.));
    }

    struct Padded {
        handle: ScrollHandle,
    }

    impl Render for Padded {
        fn render(&mut self, _: &mut Window, _: &mut Context<'_, Self>) -> impl IntoElement {
            div().size_full().child(
                div()
                    .id("padded")
                    .h(px(100.))
                    .py(px(8.))
                    .overflow_y_scroll()
                    .track_scroll(&self.handle)
                    .children((0..3).map(|_| div().h(px(20.))))
                    .vertical_scrollbar(&self.handle),
            )
        }
    }

    #[gpui::test]
    fn a_bar_in_a_padded_list_that_fits_adds_no_scroll(cx: &mut TestAppContext) {
        let (view, cx) = cx.add_window_view(|_, _| Padded {
            handle: ScrollHandle::new(),
        });
        cx.simulate_resize(size(px(400.), px(400.)));
        cx.run_until_parked();
        let max = cx.read(|cx| view.read(cx).handle.max_offset());
        assert_eq!(max.y, px(0.), "{max:?}");
    }
}

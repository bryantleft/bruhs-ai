//! Scrollbars and scrollable containers.
//!
//! - [`ScrollableElement::overflow_y_scrollbar`] (and `_x_`, both) turns an
//!   element into a scroll area with a bar.
//! - [`ScrollableElement::vertical_scrollbar`] overlays a bar on an element
//!   that already scrolls with a [`gpui::ScrollHandle`].
//! - [`Scrollbar`] is the bar itself, for a gpui `list` or `uniform_list`.

mod scrollable;
mod scrollbar;

pub use scrollable::{Scrollable, ScrollableElement};
pub use scrollbar::{
    scrollbar_style, set_scrollbar_style, Scrollbar, ScrollbarAxis, ScrollbarHandle, ScrollbarMode,
    ScrollbarStyle,
};

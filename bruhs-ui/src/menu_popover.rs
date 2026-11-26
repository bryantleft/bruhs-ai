//! Places a card menu at the button that opened it, by the rule both apps
//! share (`bruhs_models::logic::menu::menu_origin`): under the button's left
//! edge, above it when the window has no room below, and inside the window.
//!
//! GPUI's `anchored()` flips around a single point, which would cover the
//! button with the flipped menu, so this element knows the button's height.

use bruhs_models::logic::menu::{self, MenuRect};
use bruhs_models::num::ToFloatLossy;
use gpui::*;

/// Top-left corner of a `menu`-sized popover for `trigger`, in window
/// coordinates.
pub fn menu_origin(
    trigger: Bounds<Pixels>,
    menu: Size<Pixels>,
    viewport: Size<Pixels>,
) -> Point<Pixels> {
    let length = |pixels: Pixels| f64::from(f32::from(pixels));
    let (left, top) = menu::menu_origin(
        MenuRect {
            left: length(trigger.left()),
            top: length(trigger.top()),
            width: length(trigger.size.width),
            height: length(trigger.size.height),
        },
        (length(menu.width), length(menu.height)),
        (length(viewport.width), length(viewport.height)),
    );
    // Placed from measured window geometry, so already in window points.
    point(
        crate::zoom::window_px(left.to_f32_lossy()),
        crate::zoom::window_px(top.to_f32_lossy()),
    )
}

/// `content` placed for a trigger of `trigger_size` whose top-left corner is
/// this element's own position. Deferred, so the menu paints above the
/// sidebar and is not clipped by the thread list.
pub fn menu_popover(trigger_size: Size<Pixels>, content: AnyElement) -> Deferred {
    deferred(MenuPopover {
        content,
        trigger_size,
        beside: false,
    })
    .with_priority(1)
}

/// Open a submenu beside its row, with the usual vertical placement in tight windows.
pub(crate) fn submenu_popover(trigger_size: Size<Pixels>, content: AnyElement) -> Deferred {
    deferred(MenuPopover {
        content,
        trigger_size,
        beside: true,
    })
    .with_priority(1)
}

fn submenu_origin(
    trigger: Bounds<Pixels>,
    size: Size<Pixels>,
    viewport: Size<Pixels>,
) -> Point<Pixels> {
    let gap = crate::zoom::window_px(menu::MENU_GAP_PX.to_f32_lossy());
    let margin = crate::zoom::window_px(menu::MENU_WINDOW_MARGIN_PX.to_f32_lossy());
    let right = trigger.right() + gap;
    let left = trigger.left() - gap - size.width;
    let top = trigger
        .top()
        .min(viewport.height - size.height - margin)
        .max(margin);
    if right + size.width <= viewport.width - margin {
        point(right, top)
    } else if left >= margin {
        point(left, top)
    } else {
        menu_origin(trigger, size, viewport)
    }
}

struct MenuPopover {
    content: AnyElement,
    trigger_size: Size<Pixels>,
    beside: bool,
}

impl IntoElement for MenuPopover {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for MenuPopover {
    type RequestLayoutState = LayoutId;
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let content = self.content.request_layout(window, cx);
        let style = Style {
            position: Position::Absolute,
            display: Display::Flex,
            ..Default::default()
        };
        (window.request_layout(style, [content], cx), content)
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        content: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let trigger = Bounds {
            origin: bounds.origin,
            size: self.trigger_size,
        };
        let menu = window.layout_bounds(*content).size;
        let viewport = window.viewport_size();
        let origin = if self.beside {
            submenu_origin(trigger, menu, viewport)
        } else {
            menu_origin(trigger, menu, viewport)
        };
        let offset = origin - bounds.origin;
        let offset = point(offset.x.round(), offset.y.round());
        window.with_element_offset(offset, |window| self.content.prepaint(window, cx));
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        _: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        self.content.paint(window, cx);
    }
}

#[cfg(test)]
mod tests {
    use super::{menu_origin, submenu_origin};
    use gpui::{point, px, size, Bounds};

    // `bruhs_models::logic::menu::{MENU_GAP_PX, MENU_WINDOW_MARGIN_PX}`.
    const MENU_GAP_PX: f32 = 4.0;
    const WINDOW_MARGIN_PX: f32 = 8.0;

    const MENU_W: f32 = 208.0;
    const MENU_H: f32 = 290.0;

    fn trigger_at(x: f32, y: f32) -> Bounds<gpui::Pixels> {
        Bounds {
            origin: point(px(x), px(y)),
            size: size(px(24.0), px(24.0)),
        }
    }

    #[test]
    fn the_menu_hangs_under_the_trigger_left_edge_like_web() {
        let origin = menu_origin(
            trigger_at(178.0, 194.0),
            size(px(MENU_W), px(MENU_H)),
            size(px(1280.0), px(800.0)),
        );
        assert_eq!(origin, point(px(178.0), px(194.0 + 24.0 + MENU_GAP_PX)));
    }

    #[test]
    fn without_room_below_the_menu_opens_above_the_trigger() {
        let origin = menu_origin(
            trigger_at(178.0, 700.0),
            size(px(MENU_W), px(MENU_H)),
            size(px(1280.0), px(800.0)),
        );
        assert_eq!(origin, point(px(178.0), px(700.0 - MENU_GAP_PX - MENU_H)));
    }

    #[test]
    fn a_short_window_keeps_the_menu_inside_its_margins() {
        let origin = menu_origin(
            trigger_at(178.0, 200.0),
            size(px(MENU_W), px(MENU_H)),
            size(px(640.0), px(320.0)),
        );
        // Neither side fits: pin the bottom edge to the bottom margin.
        assert_eq!(origin.y, px(320.0 - WINDOW_MARGIN_PX - MENU_H));
        let tiny = menu_origin(
            trigger_at(178.0, 40.0),
            size(px(MENU_W), px(MENU_H)),
            size(px(640.0), px(200.0)),
        );
        assert_eq!(
            tiny.y,
            px(WINDOW_MARGIN_PX),
            "too tall: start at the top margin"
        );
    }

    #[test]
    fn a_narrow_window_pulls_the_menu_left_to_stay_inside() {
        let origin = menu_origin(
            trigger_at(500.0, 100.0),
            size(px(MENU_W), px(MENU_H)),
            size(px(640.0), px(800.0)),
        );
        assert_eq!(origin.x, px(640.0 - WINDOW_MARGIN_PX - MENU_W));
    }

    #[test]
    fn a_submenu_uses_the_available_side_or_falls_back_to_vertical() {
        let menu = size(px(208.), px(80.));
        let viewport = size(px(640.), px(480.));
        assert_eq!(
            submenu_origin(trigger_at(178., 194.), menu, viewport),
            point(px(206.), px(194.))
        );
        assert_eq!(
            submenu_origin(trigger_at(560., 440.), menu, viewport),
            point(px(348.), px(392.))
        );
        let trigger = trigger_at(178., 240.);
        let compact = size(px(400.), px(480.));
        assert_eq!(
            submenu_origin(trigger, menu, compact),
            menu_origin(trigger, menu, compact)
        );
    }
}

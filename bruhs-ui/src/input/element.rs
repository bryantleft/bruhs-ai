//! The element that draws a field's text: laid out, scrolled to the caret,
//! hit-tested by the mouse and handed to the platform's text input.

use gpui::{
    fill, point, px, size, App, Bounds, ContentMask, CursorStyle, Element, ElementId,
    ElementInputHandler, Entity, GlobalElementId, Hitbox, HitboxBehavior, Hsla, InspectorElementId,
    IntoElement, LayoutId, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels,
    Style, StyledText, Window,
};

use super::state::{Drawn, EditorState, FieldKind};

const CARET_WIDTH_PX: f32 = 2.0;

pub(super) struct EditorElement<K: FieldKind> {
    pub state: Entity<EditorState<K>>,
    pub text: StyledText,
    pub caret_color: Hsla,
    pub disabled: bool,
}

pub(super) struct Prepainted {
    hitbox: Hitbox,
    text_bounds: Bounds<Pixels>,
}

impl<K: FieldKind> IntoElement for EditorElement<K> {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}

impl<K: FieldKind> Element for EditorElement<K> {
    type RequestLayoutState = (LayoutId, <StyledText as Element>::RequestLayoutState);
    type PrepaintState = Prepainted;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let (text_id, text_state) = self.text.request_layout(id, inspector, window, cx);
        let mut style = Style::default();
        style.size.width = gpui::relative(1.).into();
        // One line keeps its line; many lines wrap to the field's width.
        if !K::MULTI_LINE {
            style.overflow.x = gpui::Overflow::Hidden;
        }
        let layout = window.request_layout(style, [text_id], cx);
        (layout, (text_id, text_state))
    }

    fn prepaint(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        (text_id, text_state): &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Prepainted {
        let laid_out = window.layout_bounds(*text_id);
        let scroll_x = if K::MULTI_LINE {
            Pixels::ZERO
        } else {
            self.state.read(cx).scroll_x
        };
        let text_bounds = Bounds::new(
            point(laid_out.origin.x - scroll_x, laid_out.origin.y),
            size(
                laid_out.size.width.max(bounds.size.width),
                laid_out.size.height,
            ),
        );
        self.text
            .prepaint(id, inspector, text_bounds, text_state, window, cx);
        let layout = self.text.layout().clone();
        let hitbox = window.insert_hitbox(bounds, HitboxBehavior::Normal);

        self.state.update(cx, |state, cx| {
            let map = state.display_map();
            let caret = map.to_display(state.core.selection.head);
            let caret_point = layout.position_for_index(caret);
            state.drawn = Some(Drawn {
                layout: layout.clone(),
                text_bounds,
                map,
            });
            let Some(caret_point) = caret_point else {
                return;
            };
            if K::MULTI_LINE {
                // Keep the caret's line inside the scroll viewport.
                let viewport = state.scroll.bounds();
                if viewport.size.height > Pixels::ZERO && state.caret_visible {
                    let line_height = layout.line_height();
                    let offset = state.scroll.offset();
                    let top = caret_point.y;
                    let bottom = caret_point.y + line_height;
                    let shift = if top < viewport.top() {
                        viewport.top() - top
                    } else if bottom > viewport.bottom() {
                        viewport.bottom() - bottom
                    } else {
                        Pixels::ZERO
                    };
                    if shift != Pixels::ZERO {
                        let max = state.scroll.max_offset().y;
                        let y = (offset.y + shift).clamp(-max, Pixels::ZERO);
                        if y != offset.y {
                            state.scroll.set_offset(point(offset.x, y));
                            cx.notify();
                        }
                    }
                }
            } else {
                // Scroll sideways so the caret stays in view.
                let width = bounds.size.width;
                let x = caret_point.x - text_bounds.left();
                let mut scroll = state.scroll_x;
                if x - scroll > width - px(CARET_WIDTH_PX) {
                    scroll = x - width + px(CARET_WIDTH_PX);
                } else if x < scroll {
                    scroll = x;
                }
                let max = (laid_out.size.width - width + px(CARET_WIDTH_PX)).max(Pixels::ZERO);
                scroll = scroll.clamp(Pixels::ZERO, max);
                if scroll != state.scroll_x {
                    state.scroll_x = scroll;
                    cx.notify();
                }
            }
        });

        Prepainted {
            hitbox,
            text_bounds,
        }
    }

    fn paint(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        (_, text_state): &mut Self::RequestLayoutState,
        prepainted: &mut Prepainted,
        window: &mut Window,
        cx: &mut App,
    ) {
        let focus = gpui::Focusable::focus_handle(self.state.read(cx), cx);
        let focused = focus.is_focused(window);
        if !self.disabled {
            window.handle_input(
                &focus,
                ElementInputHandler::new(bounds, self.state.clone()),
                cx,
            );
            window.set_cursor_style(CursorStyle::IBeam, &prepainted.hitbox);
        }
        window.with_content_mask(Some(ContentMask { bounds }), |window| {
            self.text.paint(
                id,
                inspector,
                prepainted.text_bounds,
                text_state,
                &mut (),
                window,
                cx,
            );
            let state = self.state.read(cx);
            if focused && state.caret_visible && state.core.selection.is_empty() {
                if let Some(drawn) = &state.drawn {
                    let caret = drawn.map.to_display(state.core.selection.head);
                    if let Some(at) = drawn.layout.position_for_index(caret) {
                        let height = drawn.layout.line_height();
                        window.paint_quad(fill(
                            Bounds::new(at, size(px(CARET_WIDTH_PX), height)),
                            self.caret_color,
                        ));
                    }
                }
            }
        });

        if self.disabled {
            return;
        }
        let hitbox = prepainted.hitbox.clone();
        let down = self.state.clone();
        window.on_mouse_event(move |event: &MouseDownEvent, phase, window, cx| {
            if !phase.bubble() || event.button != MouseButton::Left || !hitbox.is_hovered(window) {
                return;
            }
            down.update(cx, |state, cx| {
                state.mouse_down(
                    event.position,
                    event.click_count,
                    event.modifiers.shift,
                    window,
                    cx,
                );
            });
        });
        let moving = self.state.clone();
        window.on_mouse_event(move |event: &MouseMoveEvent, _, _, cx| {
            if event.pressed_button != Some(MouseButton::Left) {
                moving.update(cx, |state, _| state.mouse_up());
                return;
            }
            moving.update(cx, |state, cx| state.mouse_drag(event.position, cx));
        });
        let up = self.state.clone();
        window.on_mouse_event(move |event: &MouseUpEvent, _, _, cx| {
            if event.button == MouseButton::Left {
                up.update(cx, |state, _| state.mouse_up());
            }
        });
    }
}

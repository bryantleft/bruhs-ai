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
        // One line keeps its line: the text never wraps, the field scrolls.
        let (text_id, text_state) = if K::MULTI_LINE {
            self.text.request_layout(id, inspector, window, cx)
        } else {
            let nowrap = gpui::TextStyleRefinement {
                white_space: Some(gpui::WhiteSpace::Nowrap),
                ..Default::default()
            };
            window.with_text_style(Some(nowrap), |window| {
                self.text.request_layout(id, inspector, window, cx)
            })
        };
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
                if viewport.size.height > Pixels::ZERO && state.reveal_pending {
                    state.reveal_pending = false;
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
            } else if state.reveal_pending {
                state.reveal_pending = false;
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
                state.scroll_x = scroll.clamp(Pixels::ZERO, max);
            }
        });

        // A single-line field whose scroll just moved lays its text out again
        // at the new offset, so this frame already shows the caret.
        let mut text_bounds = text_bounds;
        if !K::MULTI_LINE {
            let scroll = self.state.read(cx).scroll_x;
            if scroll != scroll_x {
                text_bounds.origin.x = laid_out.origin.x - scroll;
                self.text
                    .prepaint(id, inspector, text_bounds, text_state, window, cx);
                self.state.update(cx, |state, _| {
                    if let Some(drawn) = state.drawn.as_mut() {
                        drawn.text_bounds = text_bounds;
                    }
                });
            }
        }

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
        let menu = self.state.clone();
        let menu_hitbox = prepainted.hitbox.clone();
        window.on_mouse_event(move |event: &MouseDownEvent, phase, window, cx| {
            if !phase.bubble()
                || event.button != MouseButton::Right
                || !menu_hitbox.is_hovered(window)
            {
                return;
            }
            cx.stop_propagation();
            menu.update(cx, |state, cx| {
                state.context_click(event.position, window, cx)
            });
        });
        let moving = self.state.clone();
        window.on_mouse_event(move |event: &MouseMoveEvent, phase, _, cx| {
            if !phase.bubble() || !moving.read(cx).selecting {
                return;
            }
            if event.pressed_button != Some(MouseButton::Left) {
                moving.update(cx, |state, _| state.mouse_up());
                return;
            }
            moving.update(cx, |state, cx| state.mouse_drag(event.position, cx));
        });
        let up = self.state.clone();
        window.on_mouse_event(move |event: &MouseUpEvent, phase, _, cx| {
            if phase.bubble() && event.button == MouseButton::Left && up.read(cx).selecting {
                up.update(cx, |state, _| state.mouse_up());
            }
        });
    }
}

//! Window-local geometry for the modeless theme editor.
use super::*;
use gpui::{point, size, Bounds, MouseMoveEvent, MouseUpEvent, Pixels, Point, Size};
use std::{cell::Cell, rc::Rc};

#[derive(Default)]
pub(super) struct Panel {
    position: Option<Point<Pixels>>,
    size: Option<Size<Pixels>>,
    pub(super) minimized: bool,
    pub(super) inspector: inspector::Inspector,
    pub(super) capture: crate::views::theme_regions::Capture,
    pub(super) scroll: ScrollHandle,
    gesture: Option<Gesture>,
}
#[derive(Clone, Copy)]
struct Gesture {
    start: Point<Pixels>,
    bounds: Bounds<Pixels>,
    resize: bool,
}

impl Panel {
    pub(super) fn new(capture: crate::views::theme_regions::Capture) -> Self {
        Self { capture, ..Default::default() }
    }
    fn position(&self, width: Pixels, viewport: Size<Pixels>) -> Option<Point<Pixels>> {
        let edge = px(8.);
        self.position.map(|p| {
            point(
                p.x.clamp(edge, (viewport.width - width - edge).max(edge)),
                p.y.clamp(edge, (viewport.height - px(48.)).max(edge)),
            )
        })
    }
    fn motion(
        &mut self,
        event: &MouseMoveEvent,
        viewport: Size<Pixels>,
        cx: &mut Context<'_, Self>,
    ) {
        let Some(gesture) = self.gesture else { return };
        if event.pressed_button != Some(MouseButton::Left) {
            self.gesture = None;
        } else {
            let delta = event.position - gesture.start;
            if gesture.resize {
                let origin = gesture.bounds.origin;
                let room = size(
                    (viewport.width - origin.x - px(8.)).max(px(1.)),
                    (viewport.height - origin.y - px(8.)).max(px(1.)),
                );
                self.size = Some(size(
                    (gesture.bounds.size.width + delta.x)
                        .clamp(px(280.).min(room.width), room.width),
                    (gesture.bounds.size.height + delta.y)
                        .clamp(px(220.).min(room.height), room.height),
                ));
            } else {
                self.position = Some(gesture.bounds.origin + delta);
            }
        }
        cx.notify();
    }
}

pub(super) fn render(
    title: &'static str,
    content: Div,
    footer: Div,
    draft: &Draft,
    w: &Window,
    cx: &Context<'_, Workspace>,
) -> AnyElement {
    let state = &draft.panel;
    let panel = state.read(cx);
    let viewport = w.viewport_size();
    let width = panel
        .size
        .map_or(px(416.), |s| s.width)
        .min((viewport.width - px(32.)).max(px(1.)));
    let position = panel.position(width, viewport);
    let height = position
        .map_or(viewport.height - px(32.), |p| {
            viewport.height - p.y - px(8.)
        })
        .max(px(40.));
    let bounds = Rc::new(Cell::new(Bounds::<Pixels>::default()));
    let start = |resize| {
        let state = state.clone();
        let bounds = bounds.clone();
        move |event: &MouseDownEvent, _: &mut Window, cx: &mut App| {
            let bounds = bounds.get();
            state.update(cx, |panel, cx| {
                panel.position = Some(bounds.origin);
                panel.gesture = Some(Gesture {
                    start: event.position,
                    bounds,
                    resize,
                });
                cx.notify();
            });
            cx.stop_propagation();
        }
    };
    let icon_button = |id: &'static str, label: &'static str, icon: &'static str| {
        web_button(
            id,
            WebButtonVariant::Ghost,
            WebButtonSize::IconSmall,
            ButtonState::Enabled,
        )
        .debug_selector(move || id.into())
        .aria_label(label)
        .tooltip(move |w, cx| Tooltip::new(label).build(w, cx))
        .child(
            svg()
                .path(icon)
                .size(px(14.))
                .text_color(theme::rambutan_300()),
        )
    };
    let minimized = panel.minimized;
    let collapse = on_press_view(
        icon_button(
            "theme-minimize",
            if minimized {
                "Expand the theme editor"
            } else {
                "Minimize the theme editor"
            },
            if minimized {
                assets::CHEVRON_RIGHT_ICON_PATH
            } else {
                assets::CHEVRON_DOWN_ICON_PATH
            },
        ),
        cx,
        {
            let state = state.clone();
            move |ws, _, cx| {
                if let Some(Overlay::ThemeEditor(ThemeEditor {
                    draft: Some(draft), ..
                })) = &mut ws.overlay
                {
                    draft.picker = None;
                }
                state.update(cx, |panel, cx| {
                    panel.minimized = !panel.minimized;
                    cx.notify()
                })
            }
        },
    );
    let (status, inspect) = inspector::header(panel, draft.advanced, cx);
    let header = div()
        .flex()
        .items_center()
        .gap_1()
        .px_2()
        .py_1()
        .border_b_1()
        .border_color(theme::ring_modal())
        .child(
            div()
                .id("theme-drag-handle")
                .debug_selector(|| "theme-drag-handle".into())
                .flex_1()
                .cursor(gpui::CursorStyle::OpenHand)
                .py_1()
                .text_sm()
                .font_weight(FontWeight::MEDIUM)
                .flex()
                .items_center()
                .gap_2()
                .min_w_0()
                .child(div().flex_none().child(title))
                .child(
                    div()
                        .min_w_0()
                        .truncate()
                        .text_xs()
                        .text_color(theme::rambutan_400())
                        .child(status),
                )
                .on_mouse_down(MouseButton::Left, start(false)),
        )
        .child(inspect)
        .child(collapse)
        .child(on_press_view(
            icon_button(
                "close-theme-editor",
                "Close theme editor",
                assets::CLOSE_ICON_PATH,
            ),
            cx,
            |ws, w, cx| {
                ws.dismiss_theme_editor(w, cx);
            },
        ));
    let mut frame = div()
        .id("theme-editor-panel")
        .debug_selector(|| "theme-editor-panel".into())
        .role(Role::Dialog)
        .aria_label(title)
        .occlude()
        .absolute()
        .w(width)
        .max_h(height)
        .rounded(px(12.))
        .border_1()
        .border_color(theme::ring_modal())
        .bg(cx
            .theme()
            .color_role("surfaceOverlay")
            .unwrap_or_else(theme::longan_900))
        .text_color(cx.theme().popover_foreground)
        .shadow_lg()
        .flex()
        .flex_col()
        .overflow_hidden()
        .when_some(position, |el, p| el.left(p.x).top(p.y))
        .when(position.is_none(), |el| el.right(px(16.)).bottom(px(16.)))
        .child(header);
    if !minimized {
        frame = frame
            .when_some(panel.size, |el, s| el.h(s.height.min(height)))
            .when(panel.size.is_none(), |el| {
                el.max_h(
                    px(672.)
                        .min((viewport.height - px(96.)).max(px(180.)))
                        .min(height),
                )
            })
            .child(
                div()
                    .id("theme-edit-scroll")
                    .debug_selector(|| "theme-edit-scroll".into())
                    .track_scroll(&panel.scroll)
                    .min_h_0()
                    .flex_shrink(1.)
                    .overflow_y_scroll()
                    .child(content),
            )
            .child(
                footer
                    .flex_shrink_0()
                    .px_3()
                    .py_2()
                    .pr(px(24.))
                    .border_t_1()
                    .border_color(theme::ring_modal()),
            )
            .child(
                div()
                    .id("theme-resize-handle")
                    .debug_selector(|| "theme-resize-handle".into())
                    .absolute()
                    .bottom_0()
                    .right_0()
                    .size(px(17.))
                    .cursor(gpui::CursorStyle::ResizeUpLeftDownRight)
                    .text_xs()
                    .text_color(theme::rambutan_400())
                    .child("◢")
                    .on_mouse_down(MouseButton::Left, start(true)),
            );
    }
    if panel.gesture.is_some() {
        let moving = state.clone();
        let ending = state.clone();
        frame = frame.child(
            gpui::canvas(
                |_, _, _| {},
                move |_, _, w, _| {
                    let moving = moving.clone();
                    w.on_mouse_event(move |e: &MouseMoveEvent, phase, w, cx| {
                        if phase.capture() {
                            moving.update(cx, |p, cx| p.motion(e, w.viewport_size(), cx));
                        }
                    });
                    let ending = ending.clone();
                    w.on_mouse_event(move |e: &MouseUpEvent, phase, _, cx| {
                        if phase.capture() && e.button == MouseButton::Left {
                            ending.update(cx, |p, cx| {
                                p.gesture = None;
                                cx.notify();
                            });
                        }
                    });
                },
            )
            .absolute()
            .size_0(),
        );
    }
    div()
        .absolute()
        .inset_0()
        .on_children_prepainted(move |children, _, _| {
            if let Some(frame) = children.last() {
                bounds.set(*frame);
            }
        })
        .children(inspector::layer(draft, w, cx))
        .child(frame)
        .into_any_element()
}

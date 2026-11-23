use super::*;
use bruhs_models::num::{ToFloatLossy, ToIntSaturating};

/// The emulator counts lines up from the bottom. The thumb counts down from the top.
#[derive(Clone, Copy)]
struct Scrollback {
    history: i32,
    from_top: i32,
    track: Pixels,
    thumb: Pixels,
}
impl Scrollback {
    fn thumb_top(self) -> Pixels {
        (self.track - self.thumb) * (self.from_top.to_f32_lossy() / self.history.to_f32_lossy())
    }
    fn target(self, position: Pixels) -> i32 {
        let travel = self.track - self.thumb;
        if travel <= Pixels::ZERO {
            return self.from_top;
        }
        ((position / travel).clamp(0., 1.) * self.history.to_f32_lossy())
            .round()
            .to_i32_saturating()
    }
}

impl TerminalView {
    fn scrollback(&self) -> Option<Scrollback> {
        let history = i32::try_from(self.term.grid().history_size()).unwrap_or(i32::MAX);
        let rows = self.term.screen_lines().to_f32_lossy();
        let track = self.bounds.size.height;
        if history == 0 || track <= Pixels::ZERO || self.term.mode().contains(TermMode::ALT_SCREEN)
        {
            return None;
        }
        Some(Scrollback {
            history,
            from_top: history.saturating_sub(
                i32::try_from(self.term.grid().display_offset()).unwrap_or(i32::MAX),
            ),
            track,
            thumb: (track * (rows / (rows + history.to_f32_lossy())))
                .max(px(18.))
                .min(track),
        })
    }

    fn drag_scrollback(&mut self, y: Pixels, cx: &mut Context<'_, Self>) {
        let (Some(grab), Some(bar)) = (self.scrollbar_grab, self.scrollback()) else {
            return;
        };
        let target = bar.target(y - self.bounds.origin.y - grab);
        self.term
            .scroll_display(Scroll::Delta(bar.from_top - target));
        cx.notify();
    }

    pub(super) fn render_scrollback(
        &mut self,
        palette: &TerminalColors,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> Option<AnyElement> {
        let Some(bar) = self.scrollback() else {
            self.scrollbar_grab = None;
            if self.scrollbar_focus.is_focused(window) {
                self.focus_handle.focus(window, cx);
            }
            return None;
        };
        let group: SharedString = format!("terminal-scrollback-{}", self.info.id).into();
        let active = self.scrollbar_grab.is_some() || self.scrollbar_focus.is_focused(window);
        let thumb = div()
            .absolute()
            .top(bar.thumb_top())
            .left(px(1.))
            .right(px(1.))
            .h(bar.thumb)
            .debug_selector(|| "desktop-terminal-scroll-thumb".into())
            .rounded(px(3.))
            .bg(if active {
                palette.scrollbar_hover
            } else {
                palette.scrollbar
            })
            .group_hover(group.clone(), |el| el.bg(palette.scrollbar_hover));
        let mut track = div()
            .id("terminal-scrollback")
            .debug_selector(|| "desktop-terminal-scrollbar".into())
            .group(group)
            .absolute()
            .right(px(1.))
            .top(px(TERMINAL_PADDING_Y_PX))
            .w(px(8.))
            .h(bar.track)
            .occlude()
            .cursor_default()
            .role(Role::ScrollBar)
            .aria_label("Terminal scrollback")
            .aria_orientation(Orientation::Vertical)
            .aria_min_numeric_value(0.)
            .aria_max_numeric_value(f64::from(bar.history))
            .aria_numeric_value(f64::from(bar.from_top))
            .track_focus(&self.scrollbar_focus)
            .tab_index(0)
            .child(thumb)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &MouseDownEvent, w, cx| {
                    let Some(bar) = this.scrollback() else {
                        return;
                    };
                    let at = event.position.y - this.bounds.origin.y;
                    this.scrollbar_grab = Some(
                        if at >= bar.thumb_top() && at <= bar.thumb_top() + bar.thumb {
                            at - bar.thumb_top()
                        } else {
                            bar.thumb / 2.
                        },
                    );
                    this.scrollbar_focus.focus(w, cx);
                    this.drag_scrollback(event.position.y, cx);
                    cx.stop_propagation();
                }),
            )
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, w, cx| {
                let delta = match event.keystroke.key.as_str() {
                    "up" => Scroll::Delta(1),
                    "down" => Scroll::Delta(-1),
                    "pageup" => Scroll::PageUp,
                    "pagedown" => Scroll::PageDown,
                    "home" => Scroll::Top,
                    "end" => Scroll::Bottom,
                    _ => return,
                };
                this.term.scroll_display(delta);
                w.prevent_default();
                cx.stop_propagation();
                cx.notify();
            }));
        {
            let view = cx.weak_entity();
            track = track.child(
                canvas(
                    |_, _, _| (),
                    move |_, _, window, _| {
                        let moved = view.clone();
                        window.on_mouse_event(move |event: &MouseMoveEvent, phase, _, cx| {
                            if phase != DispatchPhase::Capture {
                                return;
                            }
                            let _ = moved.update(cx, |this, cx| {
                                if this.scrollbar_grab.is_none() {
                                    return;
                                }
                                if event.pressed_button == Some(MouseButton::Left) {
                                    this.drag_scrollback(event.position.y, cx);
                                    cx.stop_propagation();
                                } else {
                                    this.scrollbar_grab = None;
                                    cx.notify();
                                }
                            });
                        });
                        window.on_mouse_event(move |event: &MouseUpEvent, phase, _, cx| {
                            if phase != DispatchPhase::Capture || event.button != MouseButton::Left
                            {
                                return;
                            }
                            let _ = view.update(cx, |this, cx| {
                                if this.scrollbar_grab.is_some() {
                                    this.drag_scrollback(event.position.y, cx);
                                    this.scrollbar_grab = None;
                                    cx.stop_propagation();
                                    cx.notify();
                                }
                            });
                        });
                    },
                )
                .absolute()
                .size_full(),
            );
        }
        Some(track.into_any_element())
    }
}

#[cfg(test)]
mod tests {
    use super::{px, TermSize, TerminalInfo, TerminalView};
    use alacritty_terminal::grid::Dimensions;
    use gpui::{point, size, Modifiers, MouseButton, TestAppContext, VisualTestContext};

    #[gpui::test]
    fn terminal_history_drag_keyboard_and_alternate_screen(cx: &mut TestAppContext) {
        let (view, cx) = cx.add_window_view(|_, cx| {
            let info = TerminalInfo {
                id: "history".into(),
                cwd: None,
                title: "History".into(),
                project_name: None,
                cols: 80,
                rows: 24,
                created_at_ms: 0,
            };
            TerminalView::detached(info, TermSize { cols: 80, rows: 24 }, cx)
        });
        cx.simulate_resize(size(px(620.), px(240.)));
        let lines: String = (0..150).map(|i| format!("line {i}\r\n")).collect();
        view.update(cx, |v, cx| v.feed(lines.as_bytes(), cx));
        cx.run_until_parked();
        let thumb = cx.debug_bounds("desktop-terminal-scroll-thumb").unwrap();
        let track = cx.debug_bounds("desktop-terminal-scrollbar").unwrap();
        let start = thumb.center();
        let above = point(track.origin.x - px(40.), track.origin.y - px(6.));
        cx.simulate_mouse_down(start, MouseButton::Left, Modifiers::default());
        cx.simulate_mouse_move(above, MouseButton::Left, Modifiers::default());
        cx.simulate_mouse_up(above, MouseButton::Left, Modifiers::default());
        let offset =
            |cx: &VisualTestContext| view.read_with(cx, |v, _| v.term.grid().display_offset());
        let history = view.read_with(cx, |v, _| v.term.grid().history_size());
        assert!(history > 0);
        assert_eq!(offset(cx), history);
        cx.simulate_keystrokes("down");
        assert_eq!(offset(cx), history - 1);
        cx.simulate_keystrokes("pagedown");
        assert!(offset(cx) < history - 1);
        cx.simulate_keystrokes("end");
        assert_eq!(offset(cx), 0);
        cx.simulate_keystrokes("home");
        assert_eq!(offset(cx), history);
        view.update_in(cx, |v, w, cx| v.focus_handle.focus(w, cx));
        cx.simulate_keystrokes("shift-pagedown");
        assert!(offset(cx) < history);
        view.update(cx, |v, cx| v.feed(b"\x1b[?1049h", cx));
        cx.run_until_parked();
        assert!(cx.debug_bounds("desktop-terminal-scrollbar").is_none());
        cx.simulate_keystrokes("shift-pageup");
        assert_eq!(offset(cx), 0);
        view.update(cx, |v, cx| v.feed(b"\x1b[?1049l", cx));
        cx.run_until_parked();
        assert!(cx.debug_bounds("desktop-terminal-scrollbar").is_some());
    }
}

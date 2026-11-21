//! A quit gesture is owned by its window. Explicit Quit in the application menu remains immediate.
use super::*;
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};
#[cfg(target_os = "macos")]
#[path = "quit_shortcut_macos.rs"]
mod macos;
const DOUBLE: Duration = Duration::from_millis(500);
const HOLD: Duration = Duration::from_millis(1200);

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(super) enum Mode {
    Direct,
    #[default]
    Hold,
    DoubleClick,
}
impl Mode {
    fn key(self) -> &'static str {
        match self {
            Self::Direct => "direct",
            Self::Hold => "hold",
            Self::DoubleClick => "double-click",
        }
    }
}
#[derive(Default)]
pub(super) struct Gesture {
    pub mode: Mode,
    started: Option<Instant>,
    last_press: Option<Instant>,
    last_repeat: Option<Instant>,
    #[cfg(target_os = "macos")]
    native: Option<macos::Hold>,
    hint: Option<String>,
    timer: Option<Task<()>>,
}
impl Gesture {
    fn held(&self) -> bool {
        #[cfg(target_os = "macos")]
        {
            self.native.as_ref().is_some_and(|watch| watch.is_down())
        }
        #[cfg(not(target_os = "macos"))]
        {
            self.started.is_some()
        }
    }
    pub(super) fn new(mode: Mode) -> Self {
        Self {
            mode,
            ..Self::default()
        }
    }
}
fn primary(m: gpui::Modifiers) -> bool {
    if cfg!(target_os = "macos") {
        m.platform
    } else {
        m.control
    }
}
impl Workspace {
    pub(super) fn quit_shortcut_row(&self, cx: &Context<'_, Self>) -> AnyElement {
        self.settings_row(
            "settings-row-quit-shortcut",
            "Quit shortcut",
            "Choose how the keyboard shortcut closes bruhs.",
            self.choice_control(
                "settings-quit-shortcut",
                "Quit shortcut",
                self.quit_gesture.mode.key(),
                &[
                    ("direct", "Quit immediately"),
                    ("hold", "Hold to quit"),
                    ("double-click", "Press twice"),
                ],
                cx,
            ),
        )
    }
    pub(super) fn choose_quit_mode(&mut self, key: &str, cx: &mut Context<'_, Self>) {
        let mode = match key {
            "direct" => Mode::Direct,
            "hold" => Mode::Hold,
            "double-click" => Mode::DoubleClick,
            _ => return,
        };
        self.quit_gesture = Gesture {
            mode,
            ..Gesture::default()
        };
        self.persist_and_notify(cx);
    }
    pub(super) fn quit_down(
        &mut self,
        e: &KeyDownEvent,
        w: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let key = &e.keystroke;
        if key.key != "q" || !primary(key.modifiers) || key.modifiers.alt || key.modifiers.shift {
            if self.quit_gesture.started.is_some() || self.quit_gesture.hint.is_some() {
                self.cancel_quit(false, cx);
            }
            return;
        }
        w.prevent_default();
        cx.stop_propagation();
        let now = Instant::now();
        if e.is_held {
            self.quit_gesture.last_repeat = Some(now);
            if self.quit_gesture.mode == Mode::Hold
                && self
                    .quit_gesture
                    .started
                    .is_some_and(|t| now.duration_since(t) >= HOLD)
            {
                cx.quit();
            }
            return;
        }
        let state = &mut self.quit_gesture;
        if state.mode == Mode::Direct
            || state
                .last_press
                .is_some_and(|t| now.duration_since(t) <= DOUBLE)
        {
            cx.quit();
            return;
        }
        state.started = Some(now);
        #[cfg(target_os = "macos")]
        {
            state.native = if state.mode == Mode::Hold {
                macos::Hold::begin()
            } else {
                None
            };
        }
        state.last_repeat = None;
        state.last_press = Some(now);
        let chord = if cfg!(target_os = "macos") {
            "⌘Q"
        } else {
            "Ctrl+Q"
        };
        state.hint = Some(if state.mode == Mode::Hold {
            format!("Hold {chord} to quit")
        } else {
            format!("Press {chord} again to quit")
        });
        let mode = state.mode;
        state.timer = Some(cx.spawn_in(w, async move |this, cx| {
            cx.background_executor()
                .timer(if mode == Mode::Hold { HOLD } else { DOUBLE })
                .await;
            let _ = this.update_in(cx, |this, w, cx| {
                if this.quit_gesture.last_press != Some(now) {
                    return;
                }
                if mode == Mode::Hold
                    && this.quit_gesture.started == Some(now)
                    && w.is_window_active()
                    && (this.quit_gesture.held()
                        || this
                            .quit_gesture
                            .last_repeat
                            .is_some_and(|t| t.elapsed() < Duration::from_millis(150)))
                {
                    cx.quit();
                } else {
                    this.cancel_quit(false, cx);
                }
            });
        }));
        cx.notify();
    }
    pub(super) fn quit_up(
        &mut self,
        e: &gpui::KeyUpEvent,
        _: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if e.keystroke.key == "q" && self.quit_gesture.started.is_some() {
            if self.quit_gesture.mode == Mode::Hold
                && primary(e.keystroke.modifiers)
                && self
                    .quit_gesture
                    .started
                    .is_some_and(|started| started.elapsed() >= HOLD)
            {
                cx.quit();
                return;
            }
            self.cancel_quit(true, cx);
        }
    }
    pub(super) fn quit_modifiers(
        &mut self,
        e: &gpui::ModifiersChangedEvent,
        _: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if !primary(e.modifiers) && self.quit_gesture.started.is_some() {
            self.cancel_quit(true, cx);
        }
    }
    pub(super) fn cancel_quit_on_blur(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        if !window.is_window_active() && self.quit_gesture.last_press.is_some() {
            self.cancel_quit(false, cx);
        }
    }
    fn cancel_quit(&mut self, release: bool, cx: &mut Context<'_, Self>) {
        let state = &mut self.quit_gesture;
        state.started = None;
        #[cfg(target_os = "macos")]
        {
            state.native = None;
        }
        if !release {
            state.last_press = None;
        }
        if !(release && state.mode == Mode::DoubleClick) {
            state.hint = None;
            state.timer = None;
        }
        cx.notify();
    }
    pub(super) fn quit_hint(&self) -> Option<AnyElement> {
        self.quit_gesture.hint.clone().map(|text| {
            div()
                .absolute()
                .left_0()
                .right_0()
                .top(px(44.))
                .flex()
                .justify_center()
                .child(
                    div()
                        .id("quit-shortcut-hint")
                        .role(Role::Status)
                        .aria_label(text.clone())
                        .rounded(px(8.))
                        .bg(theme::longan_900())
                        .border_1()
                        .border_color(theme::longan_700())
                        .text_color(theme::rambutan_100())
                        .px_4()
                        .py_2()
                        .child(text),
                )
                .into_any_element()
        })
    }
}

//! Small, independent meeting window. Shares the existing hub; never starts a
//! second recorder, timer, transcript subscription, or capture session.

use gpui::prelude::*;
use gpui::*;

use super::meetings_hub::{format_elapsed, MeetingsHub};
use super::web_button::{
    is_web_button_activation_key, web_button, ButtonState, WebButtonSize, WebButtonVariant,
};
use crate::theme;
use bruhs_models::EpochMs;

mod movement;

const DOCK_WIDTH: f32 = 280.0;
const DOCK_HEIGHT: f32 = 52.0;

pub struct MeetingDock {
    hub: Entity<MeetingsHub>,
    workspace: AnyWindowHandle,
    _observer: Subscription,
    focus_handle: FocusHandle,
    move_error: Option<String>,
    #[cfg(debug_assertions)]
    _fixture_bounds: Option<Subscription>,
}

pub fn open(
    hub: Entity<MeetingsHub>,
    workspace: &mut Window,
    cx: &mut App,
) -> Result<WindowHandle<MeetingDock>, String> {
    let handle = create_window(hub, workspace, cx)?;
    workspace.minimize_window();
    Ok(handle)
}

// Kept separate because GPUI's simulated platform cannot minimize windows.
// Synthetic tests exercise construction and lifecycle, not macOS window management.
fn create_window(
    hub: Entity<MeetingsHub>,
    workspace: &mut Window,
    cx: &mut App,
) -> Result<WindowHandle<MeetingDock>, String> {
    let parent = workspace.window_handle();
    let display = workspace.display(cx);
    let bounds = Bounds::centered(
        display.as_ref().map(|d| d.id()),
        size(px(DOCK_WIDTH), px(DOCK_HEIGHT)),
        cx,
    );
    let handle = cx
        .open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: None,
                kind: WindowKind::PopUp,
                window_background: WindowBackgroundAppearance::Transparent,
                is_resizable: false,
                is_minimizable: false,
                // Borderless popup: AppKit must allow the handle's explicit
                // performWindowDrag. There is no system titlebar/background drag.
                is_movable: true,
                display_id: display.map(|d| d.id()),
                ..Default::default()
            },
            |window, cx| {
                window.set_window_title("bruhs · Meeting dock");
                window.on_window_should_close(cx, move |window, cx| {
                    restore_workspace(parent, window, cx);
                    true
                });
                cx.new(|cx| {
                    let focus_handle = cx.focus_handle();
                    window.focus(&focus_handle, cx);
                    let observer =
                        cx.observe_in(&hub, window, |this: &mut MeetingDock, _, window, cx| {
                            if this.hub.read(cx).live_meeting().is_none() {
                                restore_workspace(this.workspace, window, cx);
                                window.remove_window();
                            } else {
                                cx.notify();
                            }
                        });
                    MeetingDock {
                        hub,
                        workspace: parent,
                        _observer: observer,
                        focus_handle,
                        move_error: None,
                        #[cfg(debug_assertions)]
                        _fixture_bounds: None,
                    }
                })
            },
        )
        .map_err(|error| error.to_string())?;
    if let Err(error) = handle
        .update(cx, |_, window, _| configure_panel(window))
        .map_err(|error| error.to_string())
        .and_then(|result| result)
    {
        let _ = handle.update(cx, |_, window, _| window.remove_window());
        return Err(error);
    }
    Ok(handle)
}

#[cfg(all(target_os = "macos", not(test)))]
fn native_panel(window: &Window) -> Result<objc2::rc::Retained<objc2_app_kit::NSWindow>, String> {
    use objc2::{rc::Retained, MainThreadMarker};
    use objc2_app_kit::NSView;
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    let _main = MainThreadMarker::new().ok_or("Meeting dock requires the main thread")?;
    let handle = HasWindowHandle::window_handle(window).map_err(|e| e.to_string())?;
    let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
        return Err("Meeting dock requires an AppKit window".into());
    };
    // SAFETY: GPUI owns this live NSView for the borrowed window; retain our
    // reference on its main thread and never access another application's UI.
    let view: Retained<NSView> = unsafe { Retained::retain(handle.ns_view.as_ptr().cast()) }
        .ok_or("Meeting dock view is unavailable")?;
    view.window()
        .ok_or_else(|| "Meeting dock window is unavailable".into())
}

#[cfg(all(target_os = "macos", not(test)))]
fn configure_panel(window: &Window) -> Result<(), String> {
    use objc2_app_kit::{NSColor, NSWindowStyleMask};
    let panel = native_panel(window)?;
    // titlebar=None in GPUI still creates a titled full-size-content panel.
    // Remove its system frame so the Bruhs pill supplies the only outline.
    panel.setStyleMask(NSWindowStyleMask::NonactivatingPanel);
    panel.setOpaque(false);
    panel.setBackgroundColor(Some(&NSColor::clearColor()));
    panel.setHasShadow(false);
    panel.setHidesOnDeactivate(false);
    panel.setMovableByWindowBackground(false);
    Ok(())
}

#[cfg(any(not(target_os = "macos"), test))]
fn configure_panel(_window: &Window) -> Result<(), String> {
    Ok(())
}

fn restore_workspace(parent: AnyWindowHandle, _window: &mut Window, cx: &mut App) {
    // The nonactivating pill may have been used while another app was focused.
    cx.activate(true);
    let _ = parent.update(cx, |_, window, _| window.activate_window());
}

impl MeetingDock {
    /// Expose only this synthetic window's position through its AX title so
    /// native drag acceptance does not need a screenshot of the user's desktop.
    #[cfg(debug_assertions)]
    pub fn install_fixture_diagnostics(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        Self::update_fixture_position(window);
        self._fixture_bounds =
            Some(cx.observe_window_bounds(window, |_, window, _| {
                Self::update_fixture_position(window)
            }));
    }

    #[cfg(debug_assertions)]
    fn update_fixture_position(window: &mut Window) {
        let origin = window.bounds().origin;
        eprintln!(
            "SYNTHETIC_DOCK_POSITION: x={} y={}",
            f32::from(origin.x),
            f32::from(origin.y)
        );
        window.set_window_title(&format!(
            "bruhs · Synthetic dock · x={} y={}",
            f32::from(origin.x),
            f32::from(origin.y)
        ));
    }

    fn expand(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        restore_workspace(self.workspace, window, cx);
        window.remove_window();
    }

    fn stop(&mut self, _window: &mut Window, cx: &mut Context<'_, Self>) {
        self.hub.update(cx, |hub, cx| hub.stop_live(cx));
    }
}

impl Render for MeetingDock {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let hub = self.hub.read(cx);
        let elapsed = hub
            .live_meeting()
            .map(|m| format_elapsed((EpochMs::now().as_f64() - m.started_at_ms).max(0.0)))
            .unwrap_or_default();
        let stopping = hub.capture_action_in_flight();
        let has_notice = hub.notice.is_some();
        let status = if stopping {
            "Stopping"
        } else if self.move_error.is_some() {
            "Move failed"
        } else if has_notice {
            "Needs review"
        } else {
            "Live"
        };
        #[cfg(debug_assertions)]
        let fixture = self._fixture_bounds.is_some();
        div()
            .id("meeting-dock")
            .track_focus(&self.focus_handle)
            .tab_index(0)
            .tab_stop(false)
            .tab_group()
            .role(Role::Group)
            .aria_label("Meeting dock")
            .focus_visible(|el| el.border_color(theme::persimmon_400()))
            // This independent popup has no workspace Root to route Tab actions.
            .on_key_down(|event, window, cx| {
                if event.keystroke.key == "tab"
                    && !event.keystroke.modifiers.control
                    && !event.keystroke.modifiers.alt
                    && !event.keystroke.modifiers.platform
                {
                    window.prevent_default();
                    cx.stop_propagation();
                    if event.keystroke.modifiers.shift {
                        window.focus_prev(cx);
                    } else {
                        window.focus_next(cx);
                    }
                }
            })
            .size_full()
            .flex()
            .items_center()
            .p(px(3.0))
            .gap_1()
            .rounded_full()
            .bg(theme::longan_900())
            .border_1()
            .border_color(theme::longan_700())
            .font_family("Monaspace Argon")
            .text_size(px(12.0))
            .text_color(theme::rambutan_200())
            .child(
                web_button(
                    "dock-move",
                    WebButtonVariant::Ghost,
                    WebButtonSize::IconDefault,
                    ButtonState::Enabled,
                )
                .size(px(44.0))
                .flex_shrink_0()
                .debug_selector(|| "dock-move".into())
                .rounded_full()
                .p_0()
                .aria_label(
                    "Move meeting dock. Use arrow keys, Shift for larger steps, Home to center.",
                )
                .cursor(CursorStyle::OpenHand)
                .active(|el| el.bg(theme::longan_800()))
                .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                    if let Some(movement) = movement::Movement::from_key(&event.keystroke) {
                        window.prevent_default();
                        cx.stop_propagation();
                        this.move_error = movement::move_window(window, movement).err();
                        #[cfg(debug_assertions)]
                        if this._fixture_bounds.is_some() {
                            // AppKit may deliver the bounds callback reentrantly
                            // while this entity is borrowed. Read the actual owned
                            // window again after moving, not a stale observer title.
                            Self::update_fixture_position(window);
                        }
                        cx.notify();
                    }
                }))
                .on_mouse_down(MouseButton::Left, move |_, window, _| {
                    #[cfg(debug_assertions)]
                    if fixture {
                        eprintln!("SYNTHETIC_DOCK_HANDLE_DOWN");
                    }
                    window.start_window_move();
                    #[cfg(debug_assertions)]
                    if fixture {
                        Self::update_fixture_position(window);
                    }
                })
                .child(drag_grip()),
            )
            .child(
                div()
                    .id("dock-status")
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .whitespace_nowrap()
                    .child(
                        div()
                            .size(px(6.0))
                            .flex_shrink_0()
                            .rounded_full()
                            .bg(if stopping {
                                theme::rambutan_400()
                            } else {
                                theme::persimmon_500()
                            }),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(1.0))
                            .child(
                                div()
                                    .id("dock-elapsed")
                                    .debug_selector(|| "dock-elapsed".into())
                                    .text_size(px(13.0))
                                    .line_height(px(17.0))
                                    .text_color(theme::rambutan_100())
                                    .child(elapsed),
                            )
                            .child(
                                div()
                                    .id("dock-state")
                                    .debug_selector(|| "dock-state".into())
                                    .role(Role::Status)
                                    .aria_label(
                                        self.move_error.clone().unwrap_or_else(|| status.into()),
                                    )
                                    .text_size(px(10.0))
                                    .line_height(px(13.0))
                                    .text_color(if has_notice || self.move_error.is_some() {
                                        theme::durian_400()
                                    } else {
                                        theme::rambutan_400()
                                    })
                                    .child(status),
                            ),
                    ),
            )
            .child(
                web_button(
                    "dock-expand",
                    WebButtonVariant::Ghost,
                    WebButtonSize::Small,
                    ButtonState::Enabled,
                )
                .min_h(px(44.0))
                .debug_selector(|| "dock-expand".into())
                .h(px(44.0))
                .w(px(52.0))
                .flex_shrink_0()
                .px_1()
                .py_0()
                .rounded_full()
                .active(|el| el.bg(theme::longan_700()))
                .aria_label(if has_notice {
                    "Expand meeting dock; notice needs review"
                } else {
                    "Expand meeting dock"
                })
                .on_click(cx.listener(|this, _, window, cx| this.expand(window, cx)))
                .on_key_down(cx.listener(|this, ev: &KeyDownEvent, window, cx| {
                    if is_web_button_activation_key(ev.keystroke.key.as_str()) {
                        window.prevent_default();
                        cx.stop_propagation();
                        this.expand(window, cx);
                    }
                }))
                .child("Open"),
            )
            .child(
                web_button(
                    "dock-stop",
                    WebButtonVariant::Ghost,
                    WebButtonSize::Small,
                    ButtonState::disabled_if(stopping),
                )
                .min_h(px(44.0))
                .debug_selector(|| "dock-stop".into())
                .h(px(44.0))
                .w(px(64.0))
                .flex_shrink_0()
                .px_1()
                .py_0()
                .rounded_full()
                .bg(theme::longan_950())
                .when(!stopping, |el| el.active(|el| el.bg(theme::longan_700())))
                .role(if stopping { Role::Status } else { Role::Button })
                .aria_label(if stopping {
                    "Stopping recording"
                } else {
                    "Stop recording"
                })
                .when(!stopping, |el| {
                    el.on_click(cx.listener(|this, _, window, cx| this.stop(window, cx)))
                        .on_key_down(cx.listener(|this, ev: &KeyDownEvent, window, cx| {
                            if is_web_button_activation_key(ev.keystroke.key.as_str()) {
                                window.prevent_default();
                                cx.stop_propagation();
                                this.stop(window, cx);
                            }
                        }))
                })
                .child(
                    div()
                        .size(px(7.0))
                        .flex_shrink_0()
                        .rounded(px(2.0))
                        .bg(theme::rambutan_200()),
                )
                .child("Stop"),
            )
    }
}

fn drag_grip() -> Div {
    div().flex().gap(px(3.0)).children((0..2).map(|_| {
        div()
            .flex()
            .flex_col()
            .gap(px(3.0))
            .children((0..3).map(|_| div().size(px(2.0)).rounded_full().bg(theme::rambutan_400())))
    }))
}

#[cfg(test)]
mod tests {
    use super::{create_window, EpochMs, MeetingDock, MeetingsHub};
    use crate::server_client::ServerClient;
    use crate::views::meetings_hub::MeetingPrefs;
    use gpui::{px, size, AppContext, Empty, Entity, TestAppContext, WindowHandle};
    use std::sync::Arc;

    fn fixture(cx: &mut TestAppContext) -> (Entity<MeetingsHub>, WindowHandle<MeetingDock>) {
        let parent = cx.add_window(|_, _| Empty);
        let runtime = Arc::new(tokio::runtime::Runtime::new().unwrap());
        let hub = cx.new(|cx| {
            // No hub.start(): no SSE, polling, HTTP, permission probes or capture.
            let mut hub = MeetingsHub::new(
                ServerClient::new("http://127.0.0.1:9", "http://127.0.0.1:9").unwrap(),
                runtime,
                MeetingPrefs::default(),
                cx,
            );
            let meeting = bruhs_models::Meeting::new(
                "Synthetic dock only",
                EpochMs::now().as_f64() - 65_000.0,
            );
            hub.live_meeting_id = Some(meeting.id.clone());
            hub.meetings.push(meeting);
            hub
        });
        let dock = parent
            .update(cx, |_, window, cx| {
                create_window(hub.clone(), window, cx).unwrap()
            })
            .unwrap();
        cx.run_until_parked();
        (hub, dock)
    }

    #[gpui::test]
    fn dock_should_share_one_hub_and_restore_workspace_on_expand(cx: &mut TestAppContext) {
        let (hub, dock) = fixture(cx);
        assert_eq!(cx.windows().len(), 2);
        dock.update(cx, |dock, window, cx| {
            assert_eq!(dock.hub.entity_id(), hub.entity_id());
            assert_eq!(
                window.bounds().size,
                size(px(super::DOCK_WIDTH), px(super::DOCK_HEIGHT))
            );
            dock.expand(window, cx);
        })
        .unwrap();
        cx.run_until_parked();
        assert_eq!(cx.windows().len(), 1);
        assert!(hub.read_with(cx, |hub, _| hub.live_meeting().is_some()));
    }

    #[gpui::test]
    fn dock_should_close_when_its_shared_session_ends(cx: &mut TestAppContext) {
        let (hub, _dock) = fixture(cx);
        hub.update(cx, |hub, cx| {
            hub.live_meeting_id = None;
            cx.notify();
        });
        cx.run_until_parked();
        assert_eq!(cx.windows().len(), 1);
    }

    #[gpui::test]
    fn compact_dock_preserves_targets_with_long_elapsed_time_and_warning(cx: &mut TestAppContext) {
        let (hub, dock) = fixture(cx);
        let mut visual = gpui::VisualTestContext::from_window(dock.into(), cx);
        hub.update(&mut visual, |hub, cx| {
            hub.meetings[0].started_at_ms = EpochMs::now().as_f64() - 100.0 * 60.0 * 60.0 * 1000.0;
            hub.notice = Some(crate::views::meetings_hub::HubNotice {
                message: "Synthetic warning".into(),
                warning: true,
            });
            cx.notify();
        });
        visual.run_until_parked();
        for id in ["dock-move", "dock-expand", "dock-stop"] {
            let bounds = visual.debug_bounds(id).expect("dock control");
            assert!(
                bounds.size.width >= px(44.0) && bounds.size.height >= px(44.0),
                "small target {id}: {bounds:?}"
            );
            assert!(
                bounds.origin.x >= px(0.0) && bounds.right() <= px(super::DOCK_WIDTH),
                "horizontal overflow {id}: {bounds:?}"
            );
            assert!(
                bounds.origin.y >= px(0.0) && bounds.bottom() <= px(super::DOCK_HEIGHT),
                "vertical overflow {id}: {bounds:?}"
            );
        }
        let elapsed = visual.debug_bounds("dock-elapsed").unwrap();
        assert!(elapsed.size.height <= px(20.0), "elapsed time wrapped");
        let status = visual.debug_bounds("dock-state").unwrap();
        let expand = visual.debug_bounds("dock-expand").unwrap();
        assert!(elapsed.right() <= expand.left(), "elapsed overlaps Open");
        assert!(status.right() <= expand.left(), "status overlaps Open");
        assert!(status.bottom() <= px(super::DOCK_HEIGHT), "status clips");
    }

    #[gpui::test]
    fn keyboard_move_is_routed_and_platform_failure_is_visible(cx: &mut TestAppContext) {
        let (_, dock) = fixture(cx);
        let mut visual = gpui::VisualTestContext::from_window(dock.into(), cx);
        visual.simulate_keystrokes("tab right");
        visual.run_until_parked();
        dock.update(&mut visual, |dock, _, _| {
            assert_eq!(
                dock.move_error.as_deref(),
                Some("Keyboard dock movement requires macOS")
            );
        })
        .unwrap();
    }

    #[gpui::test]
    fn stopping_keeps_layout_stable_and_open_keyboard_accessible(cx: &mut TestAppContext) {
        let (hub, dock) = fixture(cx);
        let mut visual = gpui::VisualTestContext::from_window(dock.into(), cx);
        let controls = ["dock-move", "dock-expand", "dock-stop"];
        let before = controls.map(|id| visual.debug_bounds(id).unwrap());
        hub.update(&mut visual, |hub, cx| {
            *hub = MeetingsHub::synthetic_dock_fixture(
                Arc::new(tokio::runtime::Runtime::new().unwrap()),
                cx,
            );
            cx.notify();
        });
        visual.run_until_parked();
        assert_eq!(before, controls.map(|id| visual.debug_bounds(id).unwrap()));
        // Move → Open → Move (disabled Stop skipped) → Open in reverse.
        visual.simulate_keystrokes("tab tab tab shift-tab enter");
        visual.run_until_parked();
        assert_eq!(
            visual.windows().len(),
            1,
            "Open remains usable while stopping"
        );
    }
}

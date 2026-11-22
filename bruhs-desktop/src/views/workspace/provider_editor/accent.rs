//! Optional identity marker for an ACP instance. Hex and pointer input edit the same draft.
use super::*;
use crate::views::workspace::color_sliders::{color_surface, Hsv, SliderHost, Sliders};
use bruhs_models::agent_extensions::parse_accent_color;
use gpui::{rgb, Bounds, Pixels};
use std::{cell::Cell, rc::Rc};

const INITIAL: u32 = 0x2563eb;

pub(super) struct AccentPicker {
    pub(super) selected: Option<u32>,
    pub(super) open: bool,
    sliders: Sliders,
    hex: Entity<InputState>,
    trigger: FocusHandle,
    action_focus: FocusHandle,
    bounds: Rc<Cell<Bounds<Pixels>>>,
    _input: Option<Subscription>,
}

impl AccentPicker {
    pub(super) fn new(value: Option<&str>, w: &mut Window, cx: &mut App) -> Self {
        let selected = value.and_then(parse_accent_color);
        let color = selected.unwrap_or(INITIAL);
        Self {
            selected,
            open: false,
            sliders: Sliders::new(color, cx),
            hex: field(&format!("#{color:06x}"), w, cx),
            trigger: cx.focus_handle(),
            action_focus: cx.focus_handle(),
            bounds: Default::default(),
            _input: None,
        }
    }

    pub(super) fn observe_hex(&mut self, w: &mut Window, cx: &mut Context<'_, Workspace>) {
        self._input = Some(cx.subscribe_in(&self.hex, w, |ws, input, event, w, cx| {
            let Some(picker) = ws.provider_editor.as_mut().map(|e| &mut e.accent) else {
                return;
            };
            if !picker.open || input != &picker.hex {
                return;
            }
            match event {
                InputEvent::Change => {
                    if let Some(rgb) = parse_accent_color(&input.read(cx).value()) {
                        picker.sliders.hsv = Hsv::from_rgb(rgb, picker.sliders.hsv.hue);
                        picker.selected = Some(rgb);
                        cx.notify();
                    }
                }
                InputEvent::Blur => picker.sync_hex(w, cx),
                InputEvent::PressEnter { .. } => {
                    if parse_accent_color(&input.read(cx).value()).is_some() {
                        picker.selected = Some(picker.sliders.hsv.rgb());
                        picker.close(w, cx);
                        cx.notify();
                    }
                }
                _ => {}
            }
        }));
    }

    fn apply_hsv(&mut self, w: &mut Window, cx: &mut App) {
        self.selected = Some(self.sliders.hsv.rgb());
        self.sync_hex(w, cx);
    }

    fn sync_hex(&self, w: &mut Window, cx: &mut App) {
        let value = format!("#{:06x}", self.sliders.hsv.rgb());
        self.hex.update(cx, |f, cx| f.set_value(value, w, cx));
    }

    fn focus_order(&self, cx: &App) -> [FocusHandle; 5] {
        [
            self.sliders.saturation_focus.clone(),
            self.sliders.brightness_focus.clone(),
            self.sliders.hue_focus.clone(),
            self.hex.read(cx).focus_handle(cx),
            self.action_focus.clone(),
        ]
    }

    pub(super) fn close(&mut self, w: &mut Window, cx: &mut App) {
        self.open = false;
        self.sliders.dragging_hue = None;
        self.trigger.focus(w, cx);
    }
}

impl SliderHost for Workspace {
    fn sliders(&mut self) -> Option<&mut Sliders> {
        self.provider_editor
            .as_mut()
            .filter(|e| e.accent.open)
            .map(|e| &mut e.accent.sliders)
    }

    fn apply_slider_color(&mut self, w: &mut Window, cx: &mut Context<'_, Self>) {
        if let Some(editor) = &mut self.provider_editor {
            editor.accent.apply_hsv(w, cx);
        }
    }
}

impl Workspace {
    pub(super) fn provider_accent_row(&self, wide: bool, cx: &Context<'_, Self>) -> Div {
        let editor = self.provider_editor.as_ref().unwrap();
        let picker = &editor.accent;
        let label = format!(
            "{} accent color for {}",
            if picker.selected.is_some() {
                "Change"
            } else {
                "Add"
            },
            editor.name.read(cx).value()
        );
        let button = web_button(
            "acp-accent",
            WebButtonVariant::Outline,
            WebButtonSize::Small,
            ButtonState::disabled_if(editor.busy),
        )
        .debug_selector(|| "acp-accent-trigger".into())
        .track_focus(&picker.trigger)
        .aria_label(label)
        .aria_expanded(picker.open)
        .size(px(28.))
        .p_0()
        .when_some(picker.selected, |el, color| {
            el.child(div().size_full().rounded_md().bg(rgb(color)))
        })
        .when(picker.selected.is_none(), |el| el.child("+"));
        let trigger = button.when(!editor.busy, |el| {
            on_press_view(el, cx, |ws, w, cx| {
                let p = &mut ws.provider_editor.as_mut().unwrap().accent;
                p.open = !p.open;
                p.sliders.dragging_hue = None;
                if p.open {
                    p.sync_hex(w, cx);
                    p.sliders.saturation_focus.focus(w, cx);
                }
                cx.notify();
            })
        });
        let bounds = picker.bounds.clone();
        let mut control = div()
            .relative()
            .flex_none()
            .on_children_prepainted(move |children, _, _| {
                if let Some(trigger) = children.first() {
                    bounds.set(*trigger);
                }
            })
            .child(trigger);
        if picker.open {
            let panel = self.provider_accent_panel(cx);
            control = control.child(
                div()
                    .absolute()
                    .top_0()
                    .when(wide, |el| el.right_0().w(px(232.)))
                    .when(!wide, |el| el.left_0())
                    .child(bruhs_ui::menu_popover::menu_popover(
                        picker.bounds.get().size,
                        panel,
                    )),
            );
        }
        div()
            .flex()
            .gap_2()
            .when(wide, |el| el.items_center().justify_between())
            .when(!wide, |el| el.flex_col())
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(div().text_sm().child("Accent color"))
                    .child(hint("Optional marker shown in the model picker.")),
            )
            .child(
                div()
                    .flex()
                    .items_start()
                    .when(wide, |el| el.w(px(248.)).justify_end())
                    .child(control),
            )
    }

    pub(in crate::views::workspace) fn accent_panel_has_focus(&self, w: &Window, cx: &App) -> bool {
        self.provider_editor.as_ref().is_some_and(|e| {
            e.accent.open && e.accent.focus_order(cx).iter().any(|f| f.is_focused(w))
        })
    }

    // The floating panel paints on a deferred layer. Keep its tab sequence
    // together when the containing dialog handles keyboard navigation.
    pub(in crate::views::workspace) fn step_accent_focus(
        &mut self,
        reverse: bool,
        w: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> bool {
        let Some(e) = self.provider_editor.as_mut().filter(|e| e.accent.open) else {
            return false;
        };
        let p = &mut e.accent;
        let controls = p.focus_order(cx);
        let Some(index) = controls.iter().position(|handle| handle.is_focused(w)) else {
            return false;
        };
        let next = if reverse {
            index.checked_sub(1)
        } else {
            Some(index + 1)
        };
        if let Some(handle) = next.and_then(|i| controls.get(i)) {
            handle.focus(w, cx);
        } else {
            p.close(w, cx);
            if !reverse {
                w.focus_next(cx);
            }
            cx.notify();
        }
        true
    }

    fn provider_accent_panel(&self, cx: &Context<'_, Self>) -> AnyElement {
        let p = &self.provider_editor.as_ref().unwrap().accent;
        let bounds = p.bounds.clone();
        let selected = p.selected.is_some();
        let label = if selected { "Clear color" } else { "Add color" };
        let action = web_button(
            "acp-apply-accent",
            WebButtonVariant::Ghost,
            WebButtonSize::Small,
            ButtonState::Enabled,
        )
        .track_focus(&p.action_focus)
        .aria_label(label)
        .child(label);
        div()
            .id("acp-accent-panel")
            .role(Role::Group)
            .aria_label("Accent color picker")
            .capture_key_down(cx.listener(|ws, event: &KeyDownEvent, w, cx| {
                if event.keystroke.key.as_str() == "tab"
                    && ws.step_accent_focus(event.keystroke.modifiers.shift, w, cx)
                {
                    w.prevent_default();
                    cx.stop_propagation();
                }
            }))
            .occlude()
            .w(px(232.))
            .rounded_lg()
            .border_1()
            .border_color(theme::ring_modal())
            .bg(theme::longan_900())
            .p_2()
            .flex()
            .flex_col()
            .gap_2()
            .shadow_lg()
            .on_mouse_down_out(cx.listener(move |ws, event: &MouseDownEvent, _, cx| {
                if !bounds.get().contains(&event.position) {
                    if let Some(e) = &mut ws.provider_editor {
                        e.accent.open = false;
                        e.accent.sliders.dragging_hue = None;
                    }
                    cx.notify();
                }
            }))
            .child(color_surface(&p.sliders, false, cx))
            .child(color_surface(&p.sliders, true, cx))
            .child(Input::new(&p.hex).aria_label("Custom hex accent color"))
            .child(on_press_view(action, cx, move |ws, w, cx| {
                let p = &mut ws.provider_editor.as_mut().unwrap().accent;
                if selected {
                    p.selected = None;
                    p.sliders.hsv = Hsv::from_rgb(INITIAL, 0.);
                } else {
                    p.selected = Some(p.sliders.hsv.rgb());
                }
                p.close(w, cx);
                cx.notify();
            }))
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::Hsv;

    #[gpui::test]
    fn tab_moves_between_color_channels_and_home_sets_brightness(cx: &mut gpui::TestAppContext) {
        use crate::views::workspace::{tests::thread_menu_workspace, OpenSettings};
        let (workspace, cx, _, _) = thread_menu_workspace(cx);
        workspace.update_in(cx, |ws, w, cx| {
            ws.open_settings(&OpenSettings, w, cx);
            let mut editor = super::ProviderEditor::new(None, w, cx);
            editor.accent.observe_hex(w, cx);
            editor.page = super::Page::Details;
            ws.provider_editor = Some(editor);
            cx.notify();
        });
        cx.run_until_parked();
        let trigger = cx.debug_bounds("acp-accent-trigger").unwrap();
        cx.simulate_click(trigger.center(), gpui::Modifiers::default());
        cx.run_until_parked();
        workspace.update_in(cx, |ws, w, _| {
            let color = &ws.provider_editor.as_ref().unwrap().accent;
            assert!(color.open && color.sliders.saturation_focus.is_focused(w));
        });
        cx.simulate_keystrokes("tab");
        cx.run_until_parked();
        cx.simulate_keystrokes("home");
        workspace.update_in(cx, |ws, w, _| {
            let color = &ws.provider_editor.as_ref().unwrap().accent;
            assert!(color.sliders.brightness_focus.is_focused(w));
            assert_eq!(color.selected, Some(0x000000));
        });
        cx.simulate_keystrokes("end shift-tab");
        workspace.update_in(cx, |ws, w, _| {
            let color = &ws.provider_editor.as_ref().unwrap().accent;
            assert!(color.sliders.saturation_focus.is_focused(w));
            assert_eq!(color.sliders.hsv.value, 1.);
        });
        cx.simulate_keystrokes("escape");
        workspace.update_in(cx, |ws, w, _| {
            let color = &ws.provider_editor.as_ref().unwrap().accent;
            assert!(!color.open);
            assert!(color.trigger.is_focused(w));
        });
    }

    #[test]
    fn hex_survives_reopening_and_gray_preserves_the_last_hue() {
        for color in [
            0, 0xffffff, 0x010101, 0xff0000, 0x00ff00, 0x0000ff, 0x17b89a, 0xfefdfc,
        ] {
            assert_eq!(Hsv::from_rgb(color, 0.33).rgb(), color);
        }
        for gray in [0, 0x888888, 0xffffff] {
            assert_eq!(Hsv::from_rgb(gray, 0.75).hue, 0.75);
        }
    }
}

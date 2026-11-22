//! A swatch edits the live theme draft. Escape restores the whole color family.
use super::*;
use crate::views::workspace::color_sliders::{color_surface, Hsv, SliderHost, Sliders};
use gpui::{Bounds, Pixels};
use std::{cell::Cell, rc::Rc};

pub(super) struct ColorPicker {
    role: &'static str,
    label: &'static str,
    source: Entity<InputState>,
    trigger: FocusHandle,
    bounds: Rc<Cell<Bounds<Pixels>>>,
    original: super::super::library::Colors,
    regenerate: bool,
    alpha: Option<u8>,
    sliders: Sliders,
    hex: Entity<InputState>,
    rgb: Entity<InputState>,
}

enum Edit {
    Change(String),
    Dismiss,
    Cancel,
}
impl EventEmitter<Edit> for ColorPicker {}

fn rgb_text(value: u32) -> String {
    let [_, r, g, b] = value.to_be_bytes();
    format!("{r}, {g}, {b}")
}

fn parse_rgb(text: &str) -> Option<u32> {
    let text = text.trim();
    let text = if text
        .get(..4)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("rgb("))
    {
        text[4..].strip_suffix(')')?
    } else {
        text
    };
    let mut parts = text
        .split(|c: char| c == ',' || c.is_ascii_whitespace())
        .filter(|s| !s.is_empty());
    let mut bytes = [0; 4];
    for byte in &mut bytes[1..] {
        *byte = parts.next()?.parse().ok()?;
    }
    parts.next().is_none().then(|| u32::from_be_bytes(bytes))
}

impl ColorPicker {
    fn sync(&self, except: Option<&Entity<InputState>>, w: &mut Window, cx: &mut App) {
        let color = self.sliders.hsv.rgb();
        for (field, value) in [
            (&self.hex, format!("#{color:06x}")),
            (&self.rgb, rgb_text(color)),
        ] {
            if except != Some(field) && field.read(cx).value().as_ref() != value {
                field.update(cx, |field, cx| field.set_value(value, w, cx));
            }
        }
    }

    fn changed(
        &self,
        except: Option<&Entity<InputState>>,
        w: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.sync(except, w, cx);
        let mut value = format!("#{:06x}", self.sliders.hsv.rgb());
        if let Some(alpha) = self.alpha {
            use std::fmt::Write;
            write!(value, "{alpha:02x}").unwrap();
        }
        cx.emit(Edit::Change(value));
        cx.notify();
    }

    fn focus_order(&self, cx: &App) -> [FocusHandle; 5] {
        [
            self.sliders.saturation_focus.clone(),
            self.sliders.brightness_focus.clone(),
            self.sliders.hue_focus.clone(),
            self.hex.focus_handle(cx),
            self.rgb.focus_handle(cx),
        ]
    }

    fn step_focus(&self, reverse: bool, w: &mut Window, cx: &mut Context<'_, Self>) -> bool {
        let controls = self.focus_order(cx);
        let Some(index) = controls.iter().position(|f| f.is_focused(w)) else {
            return false;
        };
        self.sync(None, w, cx);
        let next = if reverse {
            index.checked_sub(1)
        } else {
            Some(index + 1)
        };
        if let Some(handle) = next.and_then(|i| controls.get(i)) {
            handle.focus(w, cx);
        } else {
            let target = if reverse {
                self.trigger.clone()
            } else {
                self.source.focus_handle(cx)
            };
            target.focus(w, cx);
            cx.emit(Edit::Dismiss);
        }
        true
    }

    fn observe(&self, window: &mut Window, cx: &mut Context<'_, Self>) {
        for field in [&self.hex, &self.rgb] {
            cx.subscribe_in(field, window, |picker, field, event, w, cx| match event {
                InputEvent::Change => {
                    let text = field.read(cx).value();
                    let color = if field == &picker.hex {
                        text.strip_prefix('#')
                            .filter(|s| s.len() == 6 && s.bytes().all(|c| c.is_ascii_hexdigit()))
                            .and_then(|s| u32::from_str_radix(s, 16).ok())
                    } else {
                        parse_rgb(&text)
                    };
                    if let Some(color) = color {
                        picker.sliders.hsv = Hsv::from_rgb(color, picker.sliders.hsv.hue);
                        picker.changed(Some(field), w, cx);
                    }
                }
                InputEvent::Blur => picker.sync(None, w, cx),
                InputEvent::Focus => picker.sync(Some(field), w, cx),
                _ => {}
            })
            .detach();
        }
    }
}

impl SliderHost for ColorPicker {
    fn sliders(&mut self) -> Option<&mut Sliders> {
        Some(&mut self.sliders)
    }
    fn apply_slider_color(&mut self, w: &mut Window, cx: &mut Context<'_, Self>) {
        self.changed(None, w, cx);
    }
}

impl Render for ColorPicker {
    fn render(&mut self, w: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let field = |caption: &'static str, input: &Entity<InputState>| {
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    div()
                        .text_xs()
                        .text_color(theme::rambutan_400())
                        .child(caption),
                )
                .child(
                    Input::new(input).aria_label(format!("{} picker {caption} value", self.label)),
                )
        };
        div()
            .id("theme-color-picker")
            .debug_selector(|| "theme-color-picker".into())
            .role(Role::Group)
            .aria_label(format!("Choose {} color", self.label))
            .capture_key_down(cx.listener(|p, e: &KeyDownEvent, w, cx| {
                if e.keystroke.key.as_str() == "escape" {
                    cx.emit(Edit::Cancel);
                } else if e.keystroke.key.as_str() == "tab" {
                    if !p.step_focus(e.keystroke.modifiers.shift, w, cx) {
                        return;
                    }
                } else {
                    return;
                }
                w.prevent_default();
                cx.stop_propagation();
            }))
            .on_mouse_down_out(cx.listener(|p, e: &MouseDownEvent, _, cx| {
                if !p.bounds.get().contains(&e.position) {
                    cx.emit(Edit::Dismiss);
                }
            }))
            .occlude()
            .w(px(288.))
            .max_w((w.viewport_size().width - px(8.)).max(px(1.)))
            .rounded_lg()
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
            .child(
                div()
                    .px_3()
                    .py_2()
                    .border_b_1()
                    .border_color(theme::ring_modal())
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(self.label),
                    )
                    .child(
                        div()
                            .size(px(25.))
                            .rounded_full()
                            .bg(gpui::rgb(self.sliders.hsv.rgb())),
                    ),
            )
            .child(
                div()
                    .p_3()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(color_surface(&self.sliders, false, cx))
                    .child(color_surface(&self.sliders, true, cx))
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .child(field("HEX", &self.hex))
                            .child(field("RGB", &self.rgb)),
                    ),
            )
    }
}

impl Workspace {
    fn active_theme_color_picker(&self) -> Option<&Entity<ColorPicker>> {
        let Some(Overlay::ThemeEditor(editor)) = &self.overlay else {
            return None;
        };
        editor.draft.as_ref()?.picker.as_ref()
    }

    pub(in crate::views::workspace) fn theme_picker_has_focus(&self, w: &Window, cx: &App) -> bool {
        self.active_theme_color_picker()
            .is_some_and(|p| p.read(cx).focus_order(cx).iter().any(|f| f.is_focused(w)))
    }

    pub(in crate::views::workspace) fn step_theme_picker_focus(
        &self,
        reverse: bool,
        w: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> bool {
        self.active_theme_color_picker()
            .is_some_and(|p| p.update(cx, |p, cx| p.step_focus(reverse, w, cx)))
    }

    pub(in crate::views::workspace) fn cancel_theme_color_picker(
        &mut self,
        w: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> bool {
        let Some(Overlay::ThemeEditor(ThemeEditor { draft: Some(d), .. })) = &mut self.overlay
        else {
            return false;
        };
        let Some(p) = d.picker.take() else {
            return false;
        };
        let p = p.read(cx);
        d.file.set_colors(d.mode, p.original.clone());
        d.regenerate = p.regenerate;
        p.trigger.clone().focus(w, cx);
        d.sync_fields(None, w, cx);
        self.preview_theme_draft(w, cx);
        cx.notify();
        true
    }

    pub(super) fn open_swatch(
        &mut self,
        role: &'static str,
        label: &'static str,
        w: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.select_theme_usage(role, false, cx);
        let Some(Overlay::ThemeEditor(ThemeEditor { draft: Some(d), .. })) = &mut self.overlay
        else {
            return;
        };
        if d.picker.take().is_some_and(|p| p.read(cx).role == role) {
            cx.notify();
            return;
        }
        let Some(field) = d.fields.iter().find(|f| f.role == role) else {
            return;
        };
        let raw = field.input.read(cx).value();
        let literal = raw.trim();
        let [r, g, b, a] = csscolorparser::parse(literal)
            .map(|color| color.to_rgba8())
            .unwrap_or([0, 0, 0, 255]);
        let color = u32::from_be_bytes([0, r, g, b]);
        let picker = cx.new(|cx| {
            let picker = ColorPicker {
                role,
                label,
                source: field.input.clone(),
                trigger: field.swatch.clone(),
                bounds: Default::default(),
                original: d.file.colors_for(d.mode).cloned().unwrap_or_default(),
                regenerate: d.regenerate,
                alpha: (a != 255 || literal.starts_with('#') && matches!(literal.len(), 5 | 9))
                    .then_some(a),
                sliders: Sliders::new(color, cx),
                hex: cx.new(|cx| InputState::new(w, cx).default_value(format!("#{color:06x}"))),
                rgb: cx.new(|cx| InputState::new(w, cx).default_value(rgb_text(color))),
            };
            picker.observe(w, cx);
            picker
        });
        cx.subscribe_in(&picker, w, |ws, sender, event, w, cx| {
            let Some(Overlay::ThemeEditor(ThemeEditor { draft: Some(d), .. })) = &mut ws.overlay
            else {
                return;
            };
            if d.picker.as_ref() != Some(sender) {
                return;
            }
            match event {
                Edit::Change(value) => {
                    let p = sender.read(cx);
                    let (field, role) = (p.source.clone(), p.role);
                    field.update(cx, |f, cx| f.set_value(value.clone(), w, cx));
                    ws.edit_theme_color(&field, role, w, cx);
                }
                Edit::Dismiss => {
                    d.picker = None;
                    cx.notify();
                }
                Edit::Cancel => {
                    ws.cancel_theme_color_picker(w, cx);
                }
            }
        })
        .detach();
        picker
            .read(cx)
            .sliders
            .saturation_focus
            .clone()
            .focus(w, cx);
        d.picker = Some(picker);
        cx.notify();
    }

    pub(super) fn theme_swatch(
        &self,
        d: &Draft,
        field: &ColorField,
        label: &'static str,
        cx: &Context<'_, Self>,
    ) -> Div {
        let role = field.role;
        let picker = d.picker.as_ref().filter(|p| p.read(cx).role == role);
        let color = csscolorparser::parse(&field.input.read(cx).value())
            .ok()
            .map(|c| gpui::rgba(u32::from_be_bytes(c.to_rgba8())));
        let button = div()
            .id(SharedString::from(format!("swatch-{role}")))
            .debug_selector(move || format!("theme-picker-{role}"))
            .role(Role::Button)
            .aria_label(format!("Choose {label} color"))
            .aria_expanded(picker.is_some())
            .track_focus(&field.swatch)
            .tab_index(0)
            .cursor_pointer()
            .size(px(24.))
            .rounded_full()
            .border_1()
            .border_color(theme::rambutan_400())
            .when_some(color, |el, c| el.bg(c))
            .focus_visible(|s| s.border_2().border_color(theme::persimmon_400()));
        let mut wrap =
            div()
                .relative()
                .flex_none()
                .child(on_press_view(button, cx, move |ws, w, cx| {
                    ws.open_swatch(role, label, w, cx)
                }));
        if let Some(p) = picker {
            let bounds = p.read(cx).bounds.clone();
            wrap = wrap
                .on_children_prepainted(move |children, _, _| {
                    if let Some(b) = children.first() {
                        bounds.set(*b);
                    }
                })
                .child(div().absolute().right_0().top_0().w(px(288.)).child(
                    bruhs_ui::menu_popover::menu_popover(
                        gpui::size(px(24.), px(24.)),
                        p.clone().into_any_element(),
                    ),
                ));
        }
        wrap
    }
}

#[cfg(test)]
mod tests {
    use super::{OpenSettings, Overlay, SettingsSection, ThemeEditor, Workspace};
    use crate::views::workspace::tests::thread_menu_workspace;
    use gpui::px;

    #[gpui::test]
    fn theme_swatch_edits_previews_and_cancels(cx: &mut gpui::TestAppContext) {
        let (workspace, cx, _, _) = thread_menu_workspace(cx);
        workspace.update_in(cx, |ws, w, cx| {
            ws.open_settings(&OpenSettings, w, cx);
            ws.set_settings_section(SettingsSection::Appearance, cx);
            ws.create_theme(None, w, cx);
        });
        cx.simulate_resize(gpui::size(px(820.), px(720.)));
        let click = |cx: &mut gpui::VisualTestContext, selector: &'static str| {
            cx.run_until_parked();
            let center = cx.debug_bounds(selector).expect(selector).center();
            cx.simulate_click(center, Default::default());
            cx.run_until_parked();
        };
        let colors = |ws: &Workspace| {
            let Some(Overlay::ThemeEditor(ThemeEditor { draft: Some(d), .. })) = &ws.overlay else {
                panic!("editor closed")
            };
            d.file.colors_for(d.mode).unwrap().clone()
        };
        let before = workspace.read_with(cx, |ws, _| colors(ws));
        click(cx, "theme-picker-accent");
        let pop = cx.debug_bounds("theme-color-picker").unwrap();
        assert!(pop.left() >= px(0.) && pop.right() <= px(820.) && pop.size.width > px(250.));
        click(cx, "color-picker-field");
        click(cx, "color-picker-hue");
        assert_ne!(workspace.read_with(cx, |ws, _| colors(ws)), before);
        cx.simulate_keystrokes("escape");
        cx.run_until_parked();
        assert!(cx.debug_bounds("theme-color-picker").is_none());
        assert_eq!(workspace.read_with(cx, |ws, _| colors(ws)), before);
        cx.simulate_keystrokes("space");
        cx.run_until_parked();
        assert!(cx.debug_bounds("theme-color-picker").is_some());
        for idx in 1..=4 {
            cx.simulate_keystrokes("tab");
            cx.run_until_parked();
            workspace.update_in(cx, |ws, w, cx| {
                let p = ws
                    .active_theme_color_picker()
                    .expect("picker open")
                    .read(cx);
                assert!(
                    p.focus_order(cx)[idx].is_focused(w),
                    "Tab should reach control {idx}"
                );
            });
        }
        cx.simulate_keystrokes("cmd-a");
        cx.simulate_input("rgb(12, 107, 241)");
        cx.run_until_parked();
        assert_eq!(
            workspace.read_with(cx, |ws, _| colors(ws)["accent"].clone()),
            "#0c6bf1"
        );
        cx.simulate_keystrokes("shift-tab cmd-a");
        cx.simulate_input("#fe1201");
        cx.run_until_parked();
        assert_eq!(
            workspace.read_with(cx, |ws, _| colors(ws)["accent"].clone()),
            "#fe1201"
        );
        cx.simulate_keystrokes("cmd-a");
        cx.simulate_input("#oops");
        cx.simulate_keystrokes("tab");
        cx.run_until_parked();
        workspace.read_with(cx, |ws, cx| {
            let Some(Overlay::ThemeEditor(ThemeEditor { draft: Some(d), .. })) = &ws.overlay else {
                panic!()
            };
            let p = d.picker.as_ref().unwrap().read(cx);
            assert_eq!(p.hex.read(cx).value().as_ref(), "#fe1201");
            assert_eq!(p.rgb.read(cx).value().as_ref(), "254, 18, 1");
        });
        let outside = cx.debug_bounds("theme-role-accent").unwrap().center();
        assert!(!cx
            .debug_bounds("theme-color-picker")
            .unwrap()
            .contains(&outside));
        click(cx, "theme-role-accent");
        assert!(cx.debug_bounds("theme-color-picker").is_none());
        workspace.update_in(cx, |ws, w, cx| {
            ws.toggle_advanced_colors(w, cx);
            let Some(Overlay::ThemeEditor(ThemeEditor { draft: Some(d), .. })) = &ws.overlay else {
                panic!()
            };
            let field = d
                .fields
                .iter()
                .find(|f| f.role == "surface")
                .unwrap()
                .input
                .clone();
            field.update(cx, |f, cx| f.set_value("#12345680", w, cx));
            ws.edit_theme_color(&field, "surface", w, cx);
            ws.open_swatch("surface", "Surface", w, cx);
        });
        cx.run_until_parked();
        cx.simulate_keystrokes("tab tab tab cmd-a");
        cx.simulate_input("#987654");
        cx.run_until_parked();
        assert_eq!(
            workspace.read_with(cx, |ws, _| colors(ws)["surface"].clone()),
            "#98765480"
        );
    }
}

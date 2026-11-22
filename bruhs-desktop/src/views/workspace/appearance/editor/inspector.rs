//! Pick a color from the window or spotlight the uses of an editable color family.
use super::*;
use crate::views::theme_regions::{self, Capture, Usage};
use gpui::{Bounds, ClickEvent, MouseMoveEvent, Pixels};

#[derive(Default)]
pub(super) struct Inspector {
    pub(super) picking: bool,
    pub(super) selected: Option<&'static str>,
    hover: Option<Usage>,
    count: usize,
    reveal: std::rc::Rc<std::cell::Cell<bool>>,
}

fn family(role: &str) -> Option<&'static color_families::Family> {
    GROUPS
        .iter()
        .flat_map(|(_, group)| group.iter())
        .find(|f| f.roles.split_whitespace().any(|r| r == role))
}
fn label(role: &'static str, advanced: bool) -> &'static str {
    if !advanced && role == "accent" {
        "Accent"
    } else {
        family(role).map_or(role, |f| f.label)
    }
}

impl Workspace {
    fn inspected_draft(&mut self) -> Option<&mut Draft> {
        let Some(Overlay::ThemeEditor(e)) = &mut self.overlay else {
            return None;
        };
        e.draft.as_mut()
    }
    pub(in crate::views::workspace) fn inspection_frame(
        &self,
        begin: bool,
        cx: &App,
    ) -> Option<AnyElement> {
        let Some(Overlay::ThemeEditor(ThemeEditor { draft: Some(d), .. })) = &self.overlay else {
            return None;
        };
        Some(theme_regions::boundary(d.panel.read(cx).capture.clone(), begin).into_any_element())
    }
    pub(in crate::views::workspace) fn stop_picking_theme_role(
        &mut self,
        cx: &mut Context<'_, Self>,
    ) -> bool {
        let Some(d) = self.inspected_draft() else {
            return false;
        };
        if !d.panel.read(cx).inspector.picking {
            return false;
        }
        d.panel.update(cx, |p, cx| {
            p.inspector = Inspector::default();
            cx.notify()
        });
        true
    }
    pub(super) fn select_theme_usage(
        &mut self,
        role: &'static str,
        toggle: bool,
        cx: &mut Context<'_, Self>,
    ) {
        let Some(d) = self.inspected_draft() else {
            return;
        };
        d.panel.update(cx, |p, cx| {
            let selected = if toggle && p.inspector.selected == Some(role) {
                None
            } else {
                Some(role)
            };
            if !p.inspector.picking && selected == p.inspector.selected {
                return;
            }
            p.inspector = Inspector {
                selected,
                ..Default::default()
            };
            cx.notify()
        });
    }

    fn inspect_point(
        &mut self,
        pos: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let Some(d) = self.inspected_draft() else {
            return;
        };
        let p = d.panel.read(cx);
        if !p.inspector.picking {
            return;
        }
        let picked = p.capture.borrow().pick(pos).and_then(|u| family(u.role));
        let Some(f) = picked else { return };
        if !matches!(f.role, "canvas" | "accent") {
            d.advanced = true;
        }
        if !d.filter.read(cx).is_empty() {
            d.filter
                .update(cx, |input, cx| input.set_value("", window, cx));
        }
        d.picker = None;
        d.panel.update(cx, |p, cx| {
            p.inspector = Inspector {
                selected: Some(f.role),
                reveal: std::rc::Rc::new(std::cell::Cell::new(true)),
                ..Default::default()
            };
            p.minimized = false;
            cx.notify();
        });
    }
}

pub(super) fn reveal_selected_row(
    row: Div,
    panel: &panel::Panel,
    role: &str,
    view: EntityId,
) -> Div {
    if panel.inspector.selected != Some(role) || !panel.inspector.reveal.get() {
        return row;
    }
    let pending = panel.inspector.reveal.clone();
    let scroll = panel.scroll.clone();
    row.on_children_prepainted(move |children, _, cx| {
        let Some(bounds) = children.first().copied() else {
            return;
        };
        if pending.replace(false) {
            let scroll = scroll.clone();
            cx.defer(move |cx| {
                let viewport = scroll.bounds();
                let above = viewport.top() + px(12.) - bounds.top();
                let below = viewport.bottom() - px(12.) - bounds.bottom();
                let delta = above.max(px(0.)) + below.min(px(0.));
                if delta != px(0.) {
                    let y = (scroll.offset().y + delta).clamp(-scroll.max_offset().y, px(0.));
                    scroll.set_offset(point(scroll.offset().x, y));
                    cx.notify(view);
                }
            });
        }
    })
}

pub(super) fn header(
    panel: &panel::Panel,
    advanced: bool,
    cx: &Context<'_, Workspace>,
) -> (String, Stateful<Div>) {
    let inspector = &panel.inspector;
    let picking = inspector.picking;
    let status = if picking {
        "Select an element · Esc to cancel".into()
    } else if let Some(role) = inspector.selected {
        format!(
            "{} · {} {}",
            label(role, advanced),
            inspector.count,
            if inspector.count == 1 { "use" } else { "uses" }
        )
    } else {
        "Select a color below".into()
    };
    let name = if picking {
        "Cancel inspecting app colors"
    } else {
        "Inspect app colors"
    };
    let button = web_button(
        "theme-inspect",
        if picking {
            WebButtonVariant::Outline
        } else {
            WebButtonVariant::Ghost
        },
        WebButtonSize::Small,
        ButtonState::Enabled,
    )
    .debug_selector(|| "theme-inspect".into())
    .aria_label(name)
    .aria_toggled(picking.into())
    .tooltip(move |w, cx| Tooltip::new(name).build(w, cx))
    .child(if picking { "Cancel" } else { "Inspect" });
    (
        status,
        on_press_view(button, cx, |ws, w, cx| {
            if ws.stop_picking_theme_role(cx) {
                return;
            }
            let Some(d) = ws.inspected_draft() else {
                return;
            };
            d.picker = None;
            d.panel.update(cx, |p, cx| {
                p.inspector = Inspector {
                    picking: true,
                    ..Default::default()
                };
                cx.notify()
            });
            ws.workspace_focus_handle.focus(w, cx);
        }),
    )
}

fn spotlight(viewport: Bounds<Pixels>, boxes: &[Bounds<Pixels>], w: &mut Window) {
    if boxes.is_empty() {
        return;
    }
    let mut shade = vec![viewport];
    for &hole in boxes {
        shade = shade
            .into_iter()
            .flat_map(|r| theme_regions::outside(r, hole))
            .collect();
    }
    for r in shade {
        w.paint_quad(gpui::fill(r, gpui::black().opacity(0.48)));
    }
    for &r in boxes {
        w.paint_quad(gpui::outline(r, theme::persimmon_400(), BorderStyle::Solid));
    }
}

pub(super) fn layer(d: &Draft, w: &Window, cx: &Context<'_, Workspace>) -> Option<AnyElement> {
    let p = d.panel.read(cx);
    let inspector = &p.inspector;
    if !inspector.picking && inspector.selected.is_none() {
        return None;
    }
    let picking = inspector.picking;
    let capture = p.capture.clone();
    let mut roles = Vec::new();
    if let Some(role) = inspector.selected {
        if d.advanced {
            if let Some(f) = family(role) {
                roles.extend(f.roles.split_ascii_whitespace());
            }
        } else {
            roles.push(role);
            if let Some(colors) = d.file.colors_for(d.mode) {
                if let Some(value) = colors.get(role) {
                    roles.extend(
                        colors
                            .iter()
                            .filter(|(k, v)| k.as_str() != role && v.eq_ignore_ascii_case(value))
                            .map(|(k, _)| k.as_str()),
                    );
                }
            }
        }
    }
    let roles = roles.into_iter().map(str::to_owned).collect::<Vec<_>>();
    let state = d.panel.clone();
    let region = inspector.hover;
    let drawing = canvas(
        move |_, _, cx| {
            if picking {
                return region.map_or_else(Vec::new, |u| vec![u.bounds]);
            }
            let roles = roles.iter().map(String::as_str).collect::<Vec<_>>();
            let (count, matched) = capture.borrow().matches(&roles);
            if state.read(cx).inspector.count != count {
                state.update(cx, |p, cx| {
                    p.inspector.count = count;
                    cx.notify()
                });
            }
            matched
        },
        move |viewport, boxes, w, _| {
            if picking {
                for r in boxes {
                    w.paint_quad(gpui::fill(r, theme::persimmon_400().opacity(0.18)));
                    w.paint_quad(gpui::outline(r, theme::persimmon_400(), BorderStyle::Solid));
                }
            } else {
                spotlight(viewport, &boxes, w);
            }
        },
    )
    .absolute()
    .inset_0();
    let mut layer = div().absolute().inset_0().child(drawing);
    if picking {
        let capture: Capture = p.capture.clone();
        let state = d.panel.clone();
        let target = div()
            .id("theme-inspect-surface")
            .absolute()
            .inset_0()
            .occlude()
            .cursor(CursorStyle::Crosshair)
            .on_mouse_move(move |e: &MouseMoveEvent, _, cx| {
                let usage = capture.borrow().pick(e.position);
                if state.read(cx).inspector.hover != usage {
                    state.update(cx, |p, cx| {
                        p.inspector.hover = usage;
                        cx.notify()
                    });
                }
            })
            .on_mouse_down(MouseButton::Left, |_, w, cx| {
                w.prevent_default();
                cx.stop_propagation();
            })
            .on_click(cx.listener(|ws, e: &ClickEvent, w, cx| {
                cx.stop_propagation();
                ws.inspect_point(e.position(), w, cx);
            }));
        layer = layer.child(target);
        if let Some(u) = region {
            let max = w.viewport_size();
            layer = layer.child(
                div()
                    .absolute()
                    .left(
                        u.bounds
                            .left()
                            .clamp(px(0.), (max.width - px(135.)).max(px(0.))),
                    )
                    .top(if u.bounds.top() > px(28.) {
                        u.bounds.top() - px(26.)
                    } else {
                        u.bounds.top() + px(4.)
                    })
                    .rounded_md()
                    .px_2()
                    .py_1()
                    .text_xs()
                    .bg(theme::rambutan_100())
                    .text_color(theme::longan_950())
                    .child(label(u.role, true)),
            );
        }
    }
    Some(layer.into_any_element())
}

#[cfg(test)]
mod tests {
    use super::{OpenSettings, Overlay, SettingsSection, ThemeEditor};
    use crate::views::workspace::tests::thread_menu_workspace;
    use gpui::{point, px, size};

    #[gpui::test]
    fn theme_inspector_picks_sidebar_and_cancels_with_escape(cx: &mut gpui::TestAppContext) {
        let (workspace, cx, _, _) = thread_menu_workspace(cx);
        workspace.update_in(cx, |ws, w, cx| {
            ws.open_settings(&OpenSettings, w, cx);
            ws.set_settings_section(SettingsSection::Appearance, cx);
            ws.create_theme(None, w, cx);
        });
        let click_inspect = |cx: &mut gpui::VisualTestContext| {
            let center = cx.debug_bounds("theme-inspect").unwrap().center();
            cx.simulate_click(center, Default::default());
            cx.run_until_parked();
        };
        for (width, height) in [(820., 640.), (1040., 820.)] {
            cx.simulate_resize(size(px(width), px(height)));
            cx.run_until_parked();
            click_inspect(cx);
            let nav = cx
                .debug_bounds("desktop-settings-nav")
                .expect("visible navigation");
            let pos = point(nav.left() + px(3.), nav.top() + px(20.));
            cx.simulate_mouse_move(pos, None, Default::default());
            cx.run_until_parked();
            workspace.read_with(cx, |ws, cx| {
                let Some(Overlay::ThemeEditor(ThemeEditor { draft: Some(d), .. })) = &ws.overlay
                else {
                    panic!("editor closed")
                };
                let p = d.panel.read(cx);
                assert!(p.inspector.picking);
                assert_eq!(
                    p.capture.borrow().pick(pos).map(|u| u.role),
                    Some("sidebar")
                );
                assert_eq!(p.inspector.hover.map(|u| u.role), Some("sidebar"));
            });
            cx.simulate_click(pos, Default::default());
            cx.run_until_parked();
            workspace.read_with(cx, |ws, cx| {
                let Some(Overlay::ThemeEditor(ThemeEditor { draft: Some(d), .. })) = &ws.overlay
                else {
                    panic!("editor closed")
                };
                let i = &d.panel.read(cx).inspector;
                assert!(d.advanced && !i.picking);
                assert_eq!(i.selected, Some("sidebar"));
                assert!(i.count > 0);
                assert_eq!(ws.settings_section, SettingsSection::Appearance);
            });
            let row = cx.debug_bounds("theme-usage-sidebar").unwrap().center();
            assert!(
                cx.debug_bounds("theme-edit-scroll").unwrap().contains(&row),
                "picked color is scrolled into view"
            );
            cx.simulate_click(row, Default::default());
            cx.run_until_parked();
            workspace.read_with(cx, |ws, cx| {
                let Some(Overlay::ThemeEditor(ThemeEditor { draft: Some(d), .. })) = &ws.overlay
                else {
                    panic!("editor closed")
                };
                assert!(d.panel.read(cx).inspector.selected.is_none());
            });
            click_inspect(cx);
            cx.simulate_keystrokes("escape");
            cx.run_until_parked();
            workspace.read_with(cx, |ws, cx| {
                let Some(Overlay::ThemeEditor(ThemeEditor { draft: Some(d), .. })) = &ws.overlay
                else {
                    panic!("Escape should keep the draft open")
                };
                assert!(!d.panel.read(cx).inspector.picking);
            });
        }
    }
}

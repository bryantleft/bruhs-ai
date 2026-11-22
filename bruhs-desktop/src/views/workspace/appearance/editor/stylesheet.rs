//! Theme CSS source, indentation and keyboard navigation.
use super::*;
use std::ops::Range;

pub(super) struct Stylesheet {
    name: Entity<InputState>,
    existing_id: Option<String>,
    folder_error: Option<String>,
    input: Entity<TextareaState>,
    expanded: bool,
    indent: bool,
    position: (usize, usize),
}

impl Stylesheet {
    pub(super) fn new(
        source: &str,
        existing_id: Option<String>,
        name: Entity<InputState>,
        w: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> Self {
        let input = cx.new(|cx| {
            TextareaState::new(w, cx)
                .rows(10)
                .soft_wrap(false)
                .default_value(source.to_owned())
        });
        cx.observe(&input, |view, input, cx| {
            let input = input.read(cx);
            let text = input.value();
            let prefix = &text[..input.selected_range().start];
            let line = prefix.bytes().filter(|b| *b == b'\n').count() + 1;
            let column = prefix
                .rsplit('\n')
                .next()
                .unwrap_or_default()
                .encode_utf16()
                .count()
                + 1;
            if view.position != (line, column) {
                view.position = (line, column);
                cx.notify();
            }
        })
        .detach();
        cx.subscribe(&input, |view, _, event: &InputEvent, cx| {
            match event {
                InputEvent::Focus => view.indent = true,
                InputEvent::Change => cx.emit(Changed),
                InputEvent::Blur => {}
                _ => return,
            }
            cx.notify();
        })
        .detach();
        cx.subscribe(&name, |_, _, e: &InputEvent, cx| {
            if matches!(e, InputEvent::Change) {
                cx.notify()
            }
        })
        .detach();
        Self {
            name,
            existing_id,
            folder_error: None,
            input,
            expanded: !source.is_empty(),
            indent: true,
            position: (1, 1),
        }
    }

    pub(super) fn value(&self, cx: &App) -> SharedString {
        self.input.read(cx).value()
    }

    fn handle_key(&mut self, key: &Keystroke, w: &mut Window, cx: &mut Context<'_, Self>) -> bool {
        let field = self.input.read(cx);
        if !self.expanded
            || !field.focus_handle(cx).is_focused(w)
            || field.is_composing()
            || key.modifiers.platform
            || key.modifiers.control
            || key.modifiers.alt
        {
            return false;
        }
        let selected = field.selected_range();
        let text = field.value();
        let edit = match key.key.as_str() {
            "escape" => {
                self.indent = false;
                cx.notify();
                return true;
            }
            "tab" if self.indent => indent_edit(&text, selected, key.modifiers.shift),
            "enter" if selected.is_empty() => {
                let line = text[..selected.start]
                    .rsplit('\n')
                    .next()
                    .unwrap_or_default();
                let indent = &line[..line.len() - line.trim_start_matches([' ', '\t']).len()];
                let more = if line.trim_end().ends_with('{') {
                    "  "
                } else {
                    ""
                };
                Some((selected, format!("\n{indent}{more}"), None))
            }
            _ => return false,
        };
        if let Some((range, insert, selection)) = edit {
            self.input.update(cx, |f, cx| {
                f.set_selected_range(range, cx);
                f.insert(insert, w, cx);
                if let Some(after) = selection {
                    f.set_selected_range(after, cx);
                }
            });
        }
        true
    }
}

pub(super) struct Changed;
impl EventEmitter<Changed> for Stylesheet {}

type IndentEdit = (Range<usize>, String, Option<Range<usize>>);

fn indent_edit(text: &str, selection: Range<usize>, reverse: bool) -> Option<IndentEdit> {
    let start = text[..selection.start].rfind('\n').map_or(0, |i| i + 1);
    let strip = |line: &str| {
        if line.starts_with("  ") {
            2
        } else if line.starts_with([' ', '\t']) {
            1
        } else {
            0
        }
    };
    if selection.is_empty() {
        if !reverse {
            return Some((selection, "  ".into(), None));
        }
        let length = strip(&text[start..]);
        let caret = selection.start.saturating_sub(length).max(start);
        return (length > 0).then_some((start..start + length, String::new(), Some(caret..caret)));
    }
    let end = selection.end - usize::from(text[..selection.end].ends_with('\n'));
    let lines = text[start..end]
        .split('\n')
        .map(|line| {
            if reverse {
                line[strip(line)..].to_owned()
            } else {
                format!("  {line}")
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    let after = start..start + lines.len();
    Some((start..end, lines, Some(after)))
}

impl Render for Stylesheet {
    fn render(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let label = "Custom CSS";
        let toggle = web_button(
            "theme-css-toggle",
            WebButtonVariant::Ghost,
            WebButtonSize::Small,
            ButtonState::Enabled,
        )
        .debug_selector(|| "theme-css-toggle".into())
        .aria_label(label)
        .aria_expanded(self.expanded)
        .child(
            svg()
                .path(if self.expanded {
                    assets::CHEVRON_DOWN_ICON_PATH
                } else {
                    assets::CHEVRON_RIGHT_ICON_PATH
                })
                .size(px(12.))
                .text_color(theme::rambutan_400()),
        )
        .child(label);
        let toggle = on_press_view(toggle, cx, |view, _, cx| {
            view.expanded = !view.expanded;
            cx.notify();
        });
        let panel = div()
            .w_full()
            .min_w_0()
            .flex()
            .flex_col()
            .gap_2()
            .child(div().flex().child(toggle));
        if !self.expanded {
            return panel;
        }
        let empty = self.input.read(cx).is_empty();
        let clear = web_button(
            "theme-css-clear",
            WebButtonVariant::Ghost,
            WebButtonSize::Small,
            ButtonState::disabled_if(empty),
        )
        .debug_selector(|| "theme-css-clear".into())
        .aria_label("Clear CSS")
        .child("Clear CSS")
        .when(!empty, |button| {
            on_press_view(button, cx, |view, w, cx| {
                view.input
                    .update(cx, |field, cx| field.replace_all("", w, cx))
            })
        });
        let hint = if self.indent {
            "Tab indents. Shift+Tab unindents. Press Escape, then Tab to leave."
        } else {
            "Tab moves to the next control."
        };
        let status = div()
            .flex()
            .justify_between()
            .px_3()
            .py_1()
            .border_t_1()
            .border_color(theme::ring_modal())
            .font_family(theme::CODE_FONT_FAMILY)
            .text_xs()
            .child("CSS")
            .child(format!("Ln {}, Col {}", self.position.0, self.position.1));
        let input = form_textarea(Textarea::new(&self.input).appearance(false), 208.)
            .text_size(px(12.))
            .aria_label(label);
        let source = div()
            .rounded_lg()
            .border_1()
            .border_color(theme::ring_modal())
            .bg(cx
                .theme()
                .color_role("codeBackground")
                .unwrap_or_else(theme::longan_950))
            .when(self.input.focus_handle(cx).is_focused(window), |el| {
                el.border_color(theme::persimmon_400())
            })
            .overflow_hidden()
            .child(
                div()
                    .debug_selector(|| "theme-css-input".into())
                    .child(input),
            )
            .child(status);
        let name = self.name.read(cx).value();
        let id = self.existing_id.clone().or_else(|| {
            (!name.trim().is_empty()).then(|| super::super::library::identifier(name.trim()))
        });
        let file = id.map_or_else(|| "<theme-id>.png".to_owned(), |id| format!("{id}.png"));
        let folder = web_button(
            "theme-css-folder",
            WebButtonVariant::Ghost,
            WebButtonSize::Small,
            ButtonState::Enabled,
        )
        .debug_selector(|| "theme-css-folder".into())
        .aria_label("Open themes folder")
        .child("Open themes folder");
        let folder = on_press_view(folder, cx, |view, _, cx| {
            let opened = (|| {
                let dir = super::super::background::directory()
                    .ok_or("Couldn’t locate the themes folder.".to_owned())?;
                std::fs::create_dir_all(&dir)
                    .map_err(|e| format!("Couldn’t open the themes folder. {e}"))?;
                cx.open_with_system(&dir);
                Ok::<_, String>(())
            })();
            view.folder_error = opened.err();
            cx.notify();
        });
        panel.child(div().text_xs().text_color(theme::rambutan_400())
            .child("Saved with this theme and included in exports. Color rules can style the sidebar, conversation, and user messages."))
            .child(source)
            .child(div().flex().items_start().justify_between().gap_1()
                .child(div().min_w_0().flex_1().text_xs().text_color(theme::rambutan_400()).child(hint))
                .child(clear))
            .child(div().text_xs().text_color(theme::rambutan_400()).child(format!("Layer linear, radial, or conic gradients in CSS. To include an image, save {file} in the themes folder and use var(--theme-background-image). WebP and JPEG also work, up to 16 MiB.")))
            .child(div().flex().child(folder))
            .children(self.folder_error.clone().map(|s|div().text_xs().text_color(theme::rambutan_400()).child(s)))
    }
}

impl Workspace {
    pub(in crate::views::workspace) fn handle_theme_css_key(
        &self,
        key: &Keystroke,
        w: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> bool {
        let Some(Overlay::ThemeEditor(ThemeEditor {
            draft: Some(draft), ..
        })) = &self.overlay
        else {
            return false;
        };
        draft.css.update(cx, |view, cx| view.handle_key(key, w, cx))
    }
}

#[cfg(test)]
mod tests {
    use super::{Overlay, ThemeEditor, indent_edit};
    use crate::views::workspace::{
        OpenSettings, SettingsSection, Workspace, tests::thread_menu_workspace,
    };
    use bruhs_ui::ActiveTheme;
    use gpui::{Focusable, px};

    #[test]
    fn css_editor_indents_a_selection_and_unindents_the_caret() {
        let s = "α {\n  x: y;\n}";
        assert_eq!(
            indent_edit(s, 0..13, false),
            Some((0..12, "  α {\n    x: y;".into(), Some(0..16)))
        );
        assert_eq!(
            indent_edit(s, 7..7, true),
            Some((5..7, String::new(), Some(5..5)))
        );
    }

    #[gpui::test]
    fn css_editor_previews_saves_reopens_and_clears(cx: &mut gpui::TestAppContext) {
        let (ws, cx, _, _) = thread_menu_workspace(cx);
        ws.update_in(cx, |ws, w, cx| {
            ws.open_settings(&OpenSettings, w, cx);
            ws.set_settings_section(SettingsSection::Appearance, cx);
            ws.create_theme(None, w, cx);
        });
        cx.simulate_resize(gpui::size(px(1040.), px(820.)));
        cx.run_until_parked();
        cx.simulate_input("Tide");
        let click = |cx: &mut gpui::VisualTestContext, id: &'static str| {
            cx.run_until_parked();
            let at = cx.debug_bounds(id).expect(id).center();
            cx.simulate_click(at, Default::default());
            cx.run_until_parked();
        };
        let css = |ws: &Workspace| match &ws.overlay {
            Some(Overlay::ThemeEditor(ThemeEditor { draft: Some(d), .. })) => d.css.clone(),
            _ => panic!("theme editor closed"),
        };
        click(cx, "theme-css-toggle");
        click(cx, "theme-css-input");
        cx.simulate_input("[data-slot=sidebar] {");
        cx.simulate_keystrokes("enter");
        cx.simulate_input("background: #257c70;");
        cx.run_until_parked();
        ws.read_with(cx, |ws, cx| {
            assert_eq!(
                css(ws).read(cx).value(cx).as_ref(),
                "[data-slot=sidebar] {\n  background: #257c70;"
            );
            assert_eq!(
                cx.theme().color_role("cssSidebarBackground"),
                Some(gpui::rgb(0x257c70).into())
            );
        });
        cx.simulate_keystrokes("enter shift-tab");
        cx.simulate_input("}");
        cx.simulate_keystrokes("tab");
        cx.run_until_parked();
        let source = ws.read_with(cx, |ws, cx| css(ws).read(cx).value(cx).to_string());
        assert_eq!(source, "[data-slot=sidebar] {\n  background: #257c70;\n}  ");
        let gradients = "[data-slot=sidebar] {background-image: radial-gradient(circle at left, transparent, #124), conic-gradient(from 30deg, #147, #892);}";
        cx.simulate_input(gradients);
        cx.run_until_parked();
        let source = format!("{source}{gradients}");
        ws.read_with(cx, |ws, cx| {
            assert_eq!(css(ws).read(cx).value(cx).as_ref(), source);
            assert!(crate::views::workspace::appearance::background_layer("Sidebar", cx).is_some());
        });
        cx.simulate_keystrokes("escape");
        cx.run_until_parked();
        ws.update_in(cx, |ws, w, cx| {
            let editor = css(ws).read(cx);
            assert!(!editor.indent && editor.input.focus_handle(cx).is_focused(w));
        });
        cx.simulate_keystrokes("tab");
        cx.run_until_parked();
        ws.update_in(cx, |ws, w, cx| {
            assert!(!css(ws).read(cx).input.focus_handle(cx).is_focused(w))
        });
        click(cx, "theme-editor-save");
        ws.update_in(cx, |ws, w, cx| {
            let installed = ws.custom_themes.find("tide").expect("saved theme").clone();
            let exported = installed.json();
            let restored = super::InstalledTheme::parse(&exported).unwrap();
            assert_eq!(restored.file.css(), source);
            ws.start_theme_editor(Some((restored.file, true)), false, w, cx);
            assert!(css(ws).read(cx).expanded);
        });
        cx.run_until_parked();
        // Clear is in the scrollable editor body, below the code field.
        ws.update_in(cx, |ws, w, cx| {
            let editor = css(ws);
            let input = editor.read(cx).input.clone();
            input.update(cx, |input, cx| input.focus(w, cx));
        });
        cx.simulate_keystrokes("escape tab");
        cx.run_until_parked();
        // Use the button's keyboard activation after leaving the code field.
        cx.simulate_keystrokes("space");
        cx.run_until_parked();
        ws.read_with(cx, |ws, cx| assert!(css(ws).read(cx).value(cx).is_empty()));
        click(cx, "theme-editor-cancel");
        ws.read_with(cx, |ws, cx| {
            assert_eq!(ws.custom_themes.find("tide").unwrap().file.css(), source);
            assert_eq!(
                cx.theme().color_role("cssSidebarBackground"),
                Some(gpui::rgb(0x257c70).into())
            );
        });

        ws.update_in(cx, |ws, w, cx| {
            let file = ws.custom_themes.find("tide").unwrap().file.clone();
            ws.start_theme_editor(Some((file, true)), false, w, cx);
        });
        click(cx, "theme-css-input");
        cx.simulate_keystrokes("cmd-a backspace");
        cx.run_until_parked();
        click(cx, "theme-editor-save");
        ws.read_with(cx, |ws, cx| {
            let installed = ws.custom_themes.find("tide").unwrap();
            let exported: serde_json::Value = serde_json::from_str(&installed.json()).unwrap();
            assert!(exported.get("css").is_none());
            assert!(cx.theme().color_role("cssSidebarBackground").is_none());
        });
    }
}

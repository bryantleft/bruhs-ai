//! Device typography. Empty names preserve bruhs's bundled fonts.
use super::*;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub(super) struct DeviceFonts {
    separate: bool,
    smoothing: bool,
    pub wrap: Option<bool>,
    hide_context_after_start: bool,
    interface: String,
    prompt: String,
    code: String,
    terminal: String,
}
impl DeviceFonts {
    pub(super) fn apply(&self, cx: &mut App) {
        let prompt = if self.separate {
            &self.prompt
        } else {
            &self.code
        };
        let terminal = if self.separate {
            &self.terminal
        } else {
            &self.code
        };
        cx.set_text_rendering_mode(if self.smoothing {
            gpui::TextRenderingMode::Grayscale
        } else {
            gpui::TextRenderingMode::PlatformDefault
        });
        theme::set_text_layout(self.wrap.unwrap_or(false), !self.hide_context_after_start);
        theme::set_font_families(&self.interface, prompt, &self.code, terminal);
        bruhs_ui::Theme::global_mut(cx).font_family = theme::FontFace::Interface.into();
        cx.refresh_windows();
    }
}

impl Workspace {
    pub(super) fn load_font_names(&mut self, cx: &App) {
        if self.available_fonts.is_empty() {
            self.available_fonts = cx.text_system().all_font_names();
            self.available_fonts.retain(|name| {
                !name.starts_with('.') && !name.starts_with('@') && !name.trim().is_empty()
            });
            self.available_fonts
                .sort_by_cached_key(|name| name.to_lowercase());
        }
    }

    pub(super) fn select_typeface(&mut self, id: &str, value: &str, cx: &mut Context<'_, Self>) {
        let font = match id {
            "settings-font-interface" => &mut self.fonts.interface,
            "settings-font-code" => &mut self.fonts.code,
            "settings-font-prompt" => &mut self.fonts.prompt,
            "settings-font-terminal" => &mut self.fonts.terminal,
            _ => return,
        };
        if value.is_empty() || self.available_fonts.iter().any(|name| name == value) {
            *font = value.to_owned();
            self.fonts.apply(cx);
            self.persist_and_notify(cx);
        }
    }

    fn font_row(
        &self,
        id: &'static str,
        selector: &'static str,
        label: &'static str,
        description: &'static str,
        current: &str,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        let mut options = vec![DropdownOption {
            value: "".into(),
            label: "bruhs default".into(),
            description: None,
        }];
        // A closed control only needs the selected label. Build the system list on demand.
        if self.settings_open_dropdown() == Some(id) {
            options.extend(self.available_fonts.iter().map(|name| DropdownOption {
                value: name.clone().into(),
                label: name.clone(),
                description: None,
            }));
        }
        if !current.is_empty() && !options.iter().any(|o| o.value.as_ref() == current) {
            options.push(DropdownOption {
                value: current.to_owned().into(),
                label: current.into(),
                description: None,
            });
        }
        let (toggle, select) = self.settings_dropdown_handlers(id, cx);
        self.settings_row(
            selector,
            label,
            description,
            settings_ui::control_column(dropdown(
                id,
                label,
                current,
                vec![DropdownGroup {
                    label: None,
                    mark: None,
                    options,
                }],
                self.settings_listbox(id),
                toggle,
                select,
            )),
        )
    }

    pub(super) fn text_layout_rows(&self, cx: &Context<'_, Self>) -> Vec<AnyElement> {
        vec![
            self.settings_row(
                "desktop-settings-row-word-wrap",
                "Word wrap",
                "Wrap long lines in code, diffs, and previews.",
                settings_ui::control_column(self.choice_control(
                    "settings-word-wrap",
                    "Word wrap",
                    match self.fonts.wrap {
                        None => "default",
                        Some(true) => "on",
                        Some(false) => "off",
                    },
                    &[("default", "bruhs default"), ("on", "On"), ("off", "Off")],
                    cx,
                )),
            ),
            self.settings_row(
                "desktop-settings-row-composer-context",
                "Composer context",
                "Keep branch and worktree controls below the composer.",
                settings_ui::switch(
                    "settings-composer-context",
                    !self.fonts.hide_context_after_start,
                    false,
                    "Composer context",
                    cx,
                    |this, _, cx| {
                        this.fonts.hide_context_after_start = !this.fonts.hide_context_after_start;
                        this.fonts.apply(cx);
                        this.persist_and_notify(cx);
                    },
                ),
            ),
        ]
    }
    pub(super) fn render_font_rows(&self, cx: &Context<'_, Self>) -> Vec<AnyElement> {
        let fonts = &self.fonts;
        let mut rows = vec![
            self.font_row(
                "settings-font-interface",
                "desktop-settings-font-interface",
                "Interface font",
                "Labels, headings and interface controls.",
                &fonts.interface,
                cx,
            ),
            self.settings_row(
                "desktop-settings-separate-fonts",
                "Separate fonts",
                "Pick separate fonts for prompts, code, and terminal.",
                settings_ui::switch(
                    "settings-separate-fonts",
                    fonts.separate,
                    false,
                    "Separate fonts",
                    cx,
                    |this, _, cx| {
                        let f = &mut this.fonts;
                        if !f.separate {
                            if f.prompt.is_empty() {
                                f.prompt = f.code.clone();
                            }
                            if f.terminal.is_empty() {
                                f.terminal = f.code.clone();
                            }
                        }
                        f.separate = !f.separate;
                        f.apply(cx);
                        this.persist_and_notify(cx);
                    },
                ),
            ),
        ];
        if fonts.separate {
            rows.push(self.font_row(
                "settings-font-prompt",
                "desktop-settings-font-prompt",
                "Prompt font",
                "Text in the chat composer.",
                &fonts.prompt,
                cx,
            ));
        }
        rows.push(self.font_row(
            "settings-font-code",
            "desktop-settings-font-code",
            if fonts.separate {
                "Code font"
            } else {
                "Monospace font"
            },
            if fonts.separate {
                "Code, tool output and diffs."
            } else {
                "Prompts, code, tool output and terminal sessions."
            },
            &fonts.code,
            cx,
        ));
        if fonts.separate {
            rows.push(self.font_row(
                "settings-font-terminal",
                "desktop-settings-font-terminal",
                "Terminal font",
                "Terminal output and its character grid.",
                &fonts.terminal,
                cx,
            ));
        }
        rows.push(self.settings_row(
            "desktop-settings-row-font-smoothing",
            "Font smoothing",
            "Use grayscale font smoothing.",
            settings_ui::switch(
                "settings-font-smoothing",
                fonts.smoothing,
                false,
                "Font smoothing",
                cx,
                |this, _, cx| {
                    this.fonts.smoothing = !this.fonts.smoothing;
                    this.fonts.apply(cx);
                    this.persist_and_notify(cx);
                },
            ),
        ));
        rows
    }
}

#[cfg(test)]
mod tests {
    use crate::theme::FontFace;
    use gpui::{SharedString, TextRenderingMode};

    #[gpui::test]
    fn shared_and_independent_fonts_apply_to_renderers_and_persist(cx: &mut gpui::TestAppContext) {
        let (workspace, cx, _, _) = super::super::tests::thread_menu_workspace(cx);
        workspace.update_in(cx, |this, window, cx| {
            this.available_fonts = vec!["Menlo".into(), "Monaco".into()];
            this.select_typeface("settings-font-code", "Menlo", cx);
            for face in [FontFace::Prompt, FontFace::Code, FontFace::Terminal] {
                assert_eq!(SharedString::from(face).as_ref(), "Menlo");
            }
            this.fonts.separate = true;
            this.select_typeface("settings-font-prompt", "Monaco", cx);
            this.select_typeface("settings-font-prompt", "Does not exist", cx);
            this.fonts.smoothing = true;
            this.select_settings_option("settings-word-wrap", "on", window, cx);
            assert!(matches!(
                cx.text_rendering_mode(),
                TextRenderingMode::Grayscale
            ));
            assert!(crate::theme::wrap_code());
            assert_eq!(SharedString::from(FontFace::Prompt).as_ref(), "Monaco");
            let prefs = this.snapshot(cx).fonts.unwrap();
            this.fonts = Default::default();
            this.fonts.apply(cx);
            assert!(!crate::theme::wrap_code());
            prefs.apply(cx);
            assert_eq!(SharedString::from(FontFace::Code).as_ref(), "Menlo");
            assert!(crate::theme::wrap_code());
            this.fonts.apply(cx);
        });
    }
}

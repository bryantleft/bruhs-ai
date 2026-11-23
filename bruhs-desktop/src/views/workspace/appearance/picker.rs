use super::editor::{theme_button, ThemeAction};
use super::library::{InstalledTheme, ThemeFile};
use super::*;

#[derive(Clone)]
pub(super) enum ThemeChoice {
    BuiltIn(ThemeFamily),
    Custom(Arc<InstalledTheme>),
}
impl ThemeChoice {
    pub(super) fn label(&self) -> &str {
        match self {
            Self::BuiltIn(f) => f.label(),
            Self::Custom(t) => &t.file.name,
        }
    }
    pub(super) fn key(&self) -> String {
        match self {
            Self::BuiltIn(f) => f.label().into(),
            Self::Custom(t) => format!("custom-{}", t.file.id),
        }
    }
    fn palette(&self, mode: ThemeMode) -> Option<&Palette> {
        match self {
            Self::BuiltIn(f) => Some(f.palette(mode)),
            Self::Custom(t) => t.palette(mode),
        }
    }
    pub(super) fn duplicate(&self) -> ThemeFile {
        let name = format!("{} copy", self.label().chars().take(43).collect::<String>());
        match self {
            Self::BuiltIn(f) => ThemeFile::seed(
                &name,
                f.palette(ThemeMode::Light),
                f.palette(ThemeMode::Dark),
            ),
            Self::Custom(t) => t.file.duplicate(),
        }
    }
}

use bruhs_ui::press::{focusable_control, on_press_view, Interactivity};
use bruhs_ui::theme::{Palette, ThemeMode};

pub(super) fn picker_button(id: String, label: String, selected: bool) -> Stateful<Div> {
    focusable_control(
        div().id(SharedString::from(id.clone())),
        Role::Button,
        label,
        Interactivity::Enabled,
    )
    .debug_selector(move || id)
    .aria_toggled(if selected {
        Toggled::True
    } else {
        Toggled::False
    })
    .border_color(if selected {
        theme::persimmon_400()
    } else {
        theme::rambutan_600().opacity(0.5)
    })
    .focus_visible(|el| {
        el.border_color(theme::rambutan_100())
            .bg(theme::longan_700())
    })
    .hover(|el| el.bg(theme::longan_800()))
}

fn preview_rect(x: f32, y: f32, width: f32, height: f32, color: Hsla) -> Div {
    div()
        .absolute()
        .left(relative(x))
        .top(relative(y))
        .w(relative(width))
        .h(relative(height))
        .rounded(px(3.))
        .bg(color)
}

/// The halves in System are clipped views of one workspace at the same scale.
fn workspace_preview(p: &Palette, half: Option<ThemeMode>) -> Div {
    let muted: Hsla = rgb(p.text[4]).into();
    let accent: Hsla = rgb(p.accent[4]).into();
    let content = div()
        .absolute()
        .top_0()
        .h_full()
        .bg(rgb(p.surface[0]))
        .w(relative(if half.is_some() { 2. } else { 1. }))
        .when(half == Some(ThemeMode::Dark), |el| el.left(relative(-1.)))
        .child(
            div()
                .absolute()
                .left_0()
                .top_0()
                .h_full()
                .w(relative(0.22))
                .bg(rgb(p.surface[1]))
                .border_r_1()
                .border_color(muted.opacity(0.3)),
        )
        .child(preview_rect(0.035, 0.09, 0.15, 0.075, muted.opacity(0.28)))
        .child(preview_rect(0.035, 0.22, 0.15, 0.07, accent.opacity(0.65)))
        .child(preview_rect(0.035, 0.33, 0.15, 0.07, accent.opacity(0.32)))
        .child(preview_rect(0.035, 0.44, 0.15, 0.07, accent.opacity(0.17)))
        .child(preview_rect(0.49, 0.12, 0.26, 0.1, accent.opacity(0.3)))
        .child(preview_rect(0.28, 0.29, 0.36, 0.05, muted.opacity(0.42)))
        .child(preview_rect(0.28, 0.39, 0.26, 0.05, muted.opacity(0.35)))
        .child(
            preview_rect(0.77, 0.08, 0.18, 0.45, rgb(p.surface[1]).into())
                .border_1()
                .border_color(muted.opacity(0.3)),
        )
        .children(
            [0.16, 0.26, 0.36].map(|y| preview_rect(0.81, y, 0.09, 0.035, muted.opacity(0.4))),
        )
        .child(
            preview_rect(0.27, 0.68, 0.65, 0.23, rgb(p.surface[1]).into())
                .border_1()
                .border_color(muted.opacity(0.3)),
        )
        .child(preview_rect(0.31, 0.77, 0.23, 0.05, muted.opacity(0.35)))
        .child(preview_rect(0.80, 0.73, 0.075, 0.12, accent).rounded_full());
    div()
        .relative()
        .h_full()
        .flex_1()
        .min_w_0()
        .overflow_hidden()
        .child(content)
}

impl Workspace {
    pub(super) fn apply_picker_change(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        self.apply_device_appearance(window, cx);
        self.persist_and_notify(cx);
    }

    pub(in crate::views::workspace) fn scheme_picker(&self, cx: &Context<'_, Self>) -> AnyElement {
        let light = self.selected_palette(ThemeMode::Light);
        let dark = self.selected_palette(ThemeMode::Dark);
        let choices = [
            ("System", ColorScheme::System),
            ("Light", ColorScheme::Light),
            ("Dark", ColorScheme::Dark),
        ];
        let cards = div()
            .w_full()
            .grid()
            .grid_cols(3)
            .gap_2()
            .children(choices.map(|(label, scheme)| {
                let preview = div()
                    .h(px(140.))
                    .w_full()
                    .rounded(px(8.))
                    .relative()
                    .overflow_hidden()
                    .flex()
                    .border_1()
                    .border_color(theme::rambutan_600().opacity(0.35))
                    .when(scheme == ColorScheme::System, |el| {
                        el.child(workspace_preview(light, Some(ThemeMode::Light)))
                            .child(workspace_preview(dark, Some(ThemeMode::Dark)))
                    })
                    .when(scheme == ColorScheme::Light, |el| {
                        el.child(workspace_preview(light, None))
                    })
                    .when(scheme == ColorScheme::Dark, |el| {
                        el.child(workspace_preview(dark, None))
                    });
                let button = picker_button(
                    format!("settings-scheme-{}", scheme.id()),
                    format!("Use {label} appearance"),
                    self.appearance.scheme == scheme,
                )
                .rounded(px(12.))
                .bg(theme::longan_900())
                .min_w_0()
                .p_2()
                .flex()
                .flex_col()
                .items_center()
                .gap_1()
                .child(preview)
                .child(label);
                on_press_view(button, cx, move |ws, window, cx| {
                    ws.appearance.scheme = scheme;
                    ws.apply_picker_change(window, cx);
                })
            }));
        self.appearance_group("desktop-settings-row-color-scheme", "Color scheme", cards)
    }

    pub(in crate::views::workspace) fn theme_library(
        &self,
        width: f32,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        let columns =
            bruhs_models::num::ToIntSaturating::to_usize_saturating(((width + 8.) / 264.).floor())
                .clamp(1, ThemeFamily::ALL.len());
        let mut cards = div()
            .w_full()
            .grid()
            .grid_cols(u16::try_from(columns).unwrap_or(u16::MAX))
            .gap_2()
            .children(
                ThemeFamily::ALL
                    .into_iter()
                    .map(|family| self.theme_card(ThemeChoice::BuiltIn(family), vec![], cx)),
            );
        for group in self.custom_themes.card_groups() {
            let first = &group[0];
            let choice = match first.file.collection() {
                Some(id) => self
                    .theme_navigation
                    .current(id, &group, &self.custom_themes),
                None => first.clone(),
            };
            cards = cards.child(self.theme_card(ThemeChoice::Custom(choice), group, cx));
        }
        let actions = div()
            .flex()
            .flex_wrap()
            .gap_1()
            .child(on_press_view(
                theme_button("settings-create-theme", "Create theme", false),
                cx,
                |ws, w, cx| ws.create_theme(None, w, cx),
            ))
            .child(on_press_view(
                theme_button("settings-add-theme", "Add theme", false),
                cx,
                |ws, w, cx| ws.open_theme_import(w, cx),
            ));
        div()
            .flex()
            .flex_col()
            .min_w_0()
            .child(self.settings_row(
                "desktop-settings-row-themes",
                "Themes",
                "",
                actions.into_any_element(),
            ))
            .child(cards)
            .into_any_element()
    }

    fn appearance_group(
        &self,
        selector: &'static str,
        title: &'static str,
        cards: Div,
    ) -> AnyElement {
        div()
            .min_w_0()
            .flex()
            .flex_col()
            .child(self.settings_row_with_reset(selector, title, "", None, false, None, None))
            .child(cards)
            .into_any_element()
    }

    fn choice_active(&self, choice: &ThemeChoice, mode: ThemeMode) -> bool {
        let selected = self.custom_themes.selection(mode);
        match choice {
            ThemeChoice::Custom(theme) => selected == Some(&theme.file.id),
            ThemeChoice::BuiltIn(family) => {
                selected.is_none()
                    && *family
                        == match mode {
                            ThemeMode::Light => self.appearance.light_theme,
                            ThemeMode::Dark => self.appearance.dark_theme,
                        }
            }
        }
    }

    pub(super) fn select_palette_choice(
        &mut self,
        choice: &ThemeChoice,
        half: Option<ThemeMode>,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        for mode in [ThemeMode::Light, ThemeMode::Dark] {
            if half.is_some_and(|half| half != mode) || choice.palette(mode).is_none() {
                continue;
            }
            let custom = match choice {
                ThemeChoice::Custom(theme) => Some(theme.file.id.clone()),
                ThemeChoice::BuiltIn(family) => {
                    match mode {
                        ThemeMode::Light => self.appearance.light_theme = *family,
                        ThemeMode::Dark => self.appearance.dark_theme = *family,
                    };
                    None
                }
            };
            Arc::make_mut(&mut self.custom_themes).select(mode, custom);
        }
        self.apply_picker_change(window, cx);
    }

    fn theme_card(
        &self,
        choice: ThemeChoice,
        group: Vec<Arc<InstalledTheme>>,
        cx: &Context<'_, Self>,
    ) -> Stateful<Div> {
        let collection = group.first().filter(|_| group.len() > 1).map(|t| &t.file);
        let label = collection
            .and_then(ThemeFile::collection_label)
            .unwrap_or(choice.label())
            .to_string();
        let key = collection
            .and_then(ThemeFile::collection)
            .map_or_else(|| choice.key(), |id| format!("collection-{id}"));
        let swatches = if collection.is_some() {
            self.collection_preview(&group, cx)
        } else {
            div().py_2().flex().justify_center().gap_2().children(
                [(ThemeMode::Light, "Light"), (ThemeMode::Dark, "Dark")]
                    .into_iter()
                    .filter_map(|(mode, name)| {
                        let p = choice.palette(mode)?;
                        let active = self.choice_active(&choice, mode);
                        let id = format!("settings-theme-{key}-{name}");
                        let tooltip = format!("{label} · {name}");
                        let button =
                            picker_button(id, format!("Use {label} for {name} mode"), active)
                                .relative()
                                .rounded_full()
                                .size(px(62.))
                                .p(px(4.))
                                .tooltip(move |window, cx| {
                                    Tooltip::new(tooltip.clone()).build(window, cx)
                                })
                                .child(super::collection::color_disc(p))
                                .when(active, |el| {
                                    el.child(
                                        div()
                                            .absolute()
                                            .bottom_0()
                                            .right_0()
                                            .size(px(18.))
                                            .rounded_full()
                                            .bg(theme::longan_900())
                                            .border_1()
                                            .border_color(theme::rambutan_500())
                                            .text_color(theme::rambutan_100())
                                            .text_size(px(12.))
                                            .flex()
                                            .justify_center()
                                            .items_center()
                                            .child(if mode == ThemeMode::Light {
                                                "☼"
                                            } else {
                                                "☾"
                                            }),
                                    )
                                });
                        let selected = choice.clone();
                        Some(on_press_view(button, cx, move |ws, window, cx| {
                            ws.select_palette_choice(&selected, Some(mode), window, cx)
                        }))
                    }),
            )
        };
        let modes = [ThemeMode::Light, ThemeMode::Dark]
            .into_iter()
            .filter(|m| choice.palette(*m).is_some())
            .collect::<Vec<_>>();
        let both = collection.is_none() && modes.iter().all(|m| self.choice_active(&choice, *m));
        let scope = if modes.len() == 2 {
            "both light and dark"
        } else if modes[0] == ThemeMode::Light {
            "light"
        } else {
            "dark"
        };
        let title = picker_button(
            format!("settings-theme-{key}"),
            if collection.is_some() {
                format!("Use the first variants of {label} for light and dark")
            } else {
                format!("Use {label} for {scope}")
            },
            both,
        )
        .border_color(gpui::transparent_black())
        .rounded(px(5.))
        .px_2()
        .py_1()
        .text_size(px(14.))
        .min_w_0()
        .flex_1()
        .child(div().truncate().child(label));
        let card_group = group.clone();
        let selected = choice.clone();
        let selected_title = choice.clone();
        let actions = [
            ThemeAction::Duplicate,
            ThemeAction::Edit,
            ThemeAction::Export,
            ThemeAction::Remove,
        ]
        .into_iter()
        .filter(|action| {
            *action == ThemeAction::Duplicate || matches!(choice, ThemeChoice::Custom(_))
        })
        .map(|action| {
            self.theme_action_button(
                action,
                choice.clone(),
                &key,
                collection.and_then(ThemeFile::collection_label),
                cx,
            )
        })
        .collect::<Vec<_>>();
        div()
            .id(SharedString::from(format!("theme-card-{key}")))
            .cursor_pointer()
            .on_click(cx.listener(move |ws, event: &ClickEvent, window, cx| {
                if !event.is_keyboard() {
                    ws.select_theme_card(&selected, &card_group, window, cx);
                }
            }))
            .rounded(px(12.))
            .border_1()
            .border_color(theme::rambutan_600().opacity(0.34))
            .bg(theme::longan_900())
            .p_2()
            .flex()
            .flex_col()
            .gap_1()
            .child(swatches)
            .child(
                div()
                    .flex()
                    .min_w_0()
                    .items_center()
                    .justify_between()
                    .child(on_press_view(title, cx, move |ws, w, cx| {
                        ws.select_theme_card(&selected_title, &group, w, cx)
                    }))
                    .child(div().flex().flex_shrink_0().children(actions)),
            )
    }
}

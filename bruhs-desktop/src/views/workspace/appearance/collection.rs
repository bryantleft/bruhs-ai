use super::library::{InstalledTheme, ThemeLibrary};
use super::picker::{picker_button, ThemeChoice};
use super::*;
use bruhs_ui::press::{focusable_control, on_press, Interactivity};
use bruhs_ui::theme::{Palette, ThemeMode};

#[derive(Default)]
pub(in crate::views::workspace) struct CollectionNavigation {
    pub(super) return_focus: Option<FocusHandle>,
    last_used: BTreeMap<String, String>,
    expanded: Option<(String, ThemeMode, SwatchFocus)>,
}
impl CollectionNavigation {
    pub(super) fn current(
        &self,
        id: &str,
        members: &[Arc<InstalledTheme>],
        library: &ThemeLibrary,
    ) -> Arc<InstalledTheme> {
        members
            .iter()
            .find(|t| self.last_used.get(id) == Some(&t.file.id))
            .or_else(|| {
                members.iter().find(|t| {
                    [library.light.as_ref(), library.dark.as_ref()].contains(&Some(&t.file.id))
                })
            })
            .unwrap_or(&members[0])
            .clone()
    }
    pub(in crate::views::workspace) fn collapse(&mut self) {
        self.expanded = None;
    }
    fn expanded(&self, id: &str, mode: ThemeMode) -> bool {
        self.expanded
            .as_ref()
            .is_some_and(|(key, m, _)| key == id && *m == mode)
    }
}

fn variant_labels(members: &[Arc<InstalledTheme>]) -> Vec<String> {
    let words: Vec<Vec<_>> = members
        .iter()
        .map(|t| t.file.name.split_whitespace().collect())
        .collect();
    let first = &words[0];
    let prefix = first
        .iter()
        .enumerate()
        .position(|(i, w)| {
            words.iter().any(|s| {
                s.get(i)
                    .is_none_or(|other| other.to_lowercase() != w.to_lowercase())
            })
        })
        .unwrap_or_else(|| first.len().saturating_sub(1));
    words
        .into_iter()
        .zip(members)
        .map(|(name, t)| {
            let rest = name.get(prefix..).unwrap_or_default().join(" ");
            if rest.is_empty() {
                t.file.name.clone()
            } else {
                rest
            }
        })
        .collect()
}

pub(super) fn color_disc(p: &Palette) -> Div {
    div()
        .size_full()
        .rounded_full()
        .border_1()
        .border_color(rgb(p.surface[4]))
        .bg(linear_gradient(
            36.,
            linear_color_stop(rgb(p.surface[0]), 0.10),
            linear_color_stop(rgb(p.accent[4]), 0.94),
        ))
}

type ThemeOption = (Arc<InstalledTheme>, String);

#[derive(Clone)]
struct SwatchFocus {
    group: FocusHandle,
    trigger: FocusHandle,
}
#[derive(IntoElement)]
struct ModeSwatches {
    owner: WeakEntity<Workspace>,
    id: String,
    mode: ThemeMode,
    options: Vec<ThemeOption>,
    selection: Option<String>,
    expanded: bool,
}

impl RenderOnce for ModeSwatches {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let Self {
            owner,
            id,
            mode,
            options,
            selection,
            expanded,
        } = self;
        let mode_name = if mode == ThemeMode::Light {
            "light"
        } else {
            "dark"
        };
        let key = format!("theme-variants-{id}-{mode_name}");
        let multiple = options.len() > 1;
        let focus =
            window.use_keyed_state(SharedString::from(format!("{key}-focus")), cx, |_, cx| {
                (
                    SwatchFocus {
                        group: cx.focus_handle(),
                        trigger: cx.focus_handle().tab_index(0).tab_stop(true),
                    },
                    std::cell::Cell::new(false),
                )
            });
        let (handles, previously_focused) = focus.read(cx);
        let handles = handles.clone();
        let focused = handles.group.contains_focused(window, cx);
        let changed = previously_focused.replace(focused) != focused;
        // The frame's focus path also covers keyboard navigation and a newly drawn group.
        let expanded = if multiple && changed {
            let owner = owner.clone();
            let id = id.clone();
            let focus = handles.clone();
            cx.defer(move |cx| {
                let _ = owner.update(cx, |ws, cx| {
                    ws.expand_theme_fan(&id, mode, &focus, focused, cx)
                });
            });
            focused
        } else {
            expanded
        };
        let chosen = options
            .iter()
            .find(|(t, _)| Some(&t.file.id) == selection.as_ref())
            .unwrap_or(&options[0]);
        let count = options.len();
        let active = Some(&chosen.0.file.id) == selection.as_ref();
        let label = if count > 1 {
            format!(
                "Choose {mode_name} variant, {count} options, currently {}",
                chosen.1
            )
        } else {
            format!("Use {mode_name} variant, currently {}", chosen.1)
        };
        let activate = |t: Arc<InstalledTheme>| {
            let owner = owner.clone();
            move |w: &mut Window, cx: &mut App| {
                let _ = owner.update(cx, |ws, cx| {
                    ws.use_collection_variant(t.clone(), mode, w, cx)
                });
            }
        };
        let tooltip = format!("{} · {mode_name}", chosen.1);
        let primary = picker_button(format!("{key}-current"), label, active)
            .when(multiple, |el| el.aria_expanded(expanded))
            .tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx))
            .track_focus(&handles.trigger)
            .size(px(56.))
            .p(px(2.))
            .rounded_full()
            .relative()
            .child(color_disc(chosen.0.palette(mode).unwrap()))
            .when(active, |el| {
                el.child(
                    div()
                        .absolute()
                        .right_0()
                        .bottom_0()
                        .rounded_full()
                        .bg(theme::longan_900())
                        .text_xs()
                        .child(if mode == ThemeMode::Light {
                            "☼"
                        } else {
                            "☾"
                        }),
                )
            });
        let mut group = div()
            .id(SharedString::from(format!("{key}-area")))
            .role(Role::Group)
            .aria_label(format!("{mode_name} theme variants"))
            .track_focus(&handles.group)
            .w(px(104.))
            .h(px(84.))
            .relative()
            .flex()
            .flex_col()
            .items_center()
            .justify_end()
            .child(on_press(primary, activate(chosen.0.clone())))
            .child(
                div()
                    .flex()
                    .justify_center()
                    .items_center()
                    .gap_1()
                    .w_full()
                    .h(px(19.))
                    .child(div().min_w_0().truncate().text_xs().child(chosen.1.clone()))
                    .when(count > 1, |el| {
                        el.child(
                            div()
                                .rounded_full()
                                .px_1()
                                .bg(theme::longan_700())
                                .text_xs()
                                .child(format!("+{}", count - 1)),
                        )
                    }),
            );
        if count > 1 {
            let hover_owner = owner.clone();
            let hover_id = id.clone();
            group = group.on_hover(move |hover, _, cx| {
                let _ = hover_owner.update(cx, |ws, cx| {
                    ws.expand_theme_fan(&hover_id, mode, &handles, *hover, cx)
                });
            });
            if expanded {
                for (index, (t, label)) in options.iter().enumerate() {
                    let position = index as f32 / (count - 1) as f32 - 0.5;
                    let active = selection.as_ref() == Some(&t.file.id);
                    let tip = format!("Use {label} for {mode_name} mode");
                    let option = picker_button(format!("{key}-{}", t.file.id), tip.clone(), active)
                        .absolute()
                        .left(px(38. + position * 68.))
                        .top(px(2. + position.abs() * 10.))
                        .size(px(28.))
                        .rounded_full()
                        .p(px(2.))
                        .bg(theme::longan_900())
                        .tooltip(move |w, cx| Tooltip::new(tip.clone()).build(w, cx))
                        .child(color_disc(t.palette(mode).unwrap()));
                    let option = on_press(option, activate(t.clone()));
                    if crate::motion::reduce_motion(cx) {
                        group = group.child(option);
                    } else {
                        let delay = index as u64 * 35;
                        let duration = 200 + delay;
                        let animation = Animation::new(std::time::Duration::from_millis(duration));
                        group = group.child(option.with_animation(
                            SharedString::from(format!("{key}-reveal-{}", t.file.id)),
                            animation,
                            move |el, elapsed| {
                                let progress = ((elapsed * duration as f32 - delay as f32) / 200.)
                                    .clamp(0., 1.);
                                let ease = 1. - (1. - progress).powi(3);
                                let size = 16. + 12. * ease;
                                el.left(px(52. + position * 68. * ease - size / 2.))
                                    .top(px(30. + (position.abs() * 10. - 28.) * ease))
                                    .size(px(size))
                                    .opacity(ease)
                            },
                        ));
                    }
                }
            }
        }
        group
    }
}

impl Workspace {
    fn expand_theme_fan(
        &mut self,
        id: &str,
        mode: ThemeMode,
        focus: &SwatchFocus,
        open: bool,
        cx: &mut Context<'_, Self>,
    ) {
        if !self.settings_visible() || self.settings_section != SettingsSection::Appearance {
            return;
        }
        let same = self.theme_navigation.expanded(id, mode);
        if open && !same {
            self.theme_navigation.expanded = Some((id.to_owned(), mode, focus.clone()));
            cx.notify();
        } else if !open && same {
            self.theme_navigation.collapse();
            cx.notify();
        }
    }

    pub(in crate::views::workspace) fn dismiss_theme_fan(
        &mut self,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> bool {
        if !self.settings_visible() || self.settings_section != SettingsSection::Appearance {
            return false;
        }
        let Some((_, _, focus)) = self.theme_navigation.expanded.take() else {
            return false;
        };
        if focus.group.contains_focused(window, cx) {
            focus.trigger.focus(window, cx);
        }
        cx.notify();
        true
    }

    pub(super) fn collection_preview(
        &self,
        members: &[Arc<InstalledTheme>],
        cx: &Context<'_, Self>,
    ) -> Div {
        let id = members[0].file.collection().unwrap();
        let labels = variant_labels(members);
        let mut row = div().flex().justify_center().items_center();
        for mode in [ThemeMode::Light, ThemeMode::Dark] {
            let options: Vec<_> = members
                .iter()
                .zip(&labels)
                .filter(|(t, _)| t.palette(mode).is_some())
                .map(|(t, label)| (t.clone(), label.clone()))
                .collect();
            if options.is_empty() {
                row = row.child(div().w(px(104.)));
                continue;
            }
            row = row.child(ModeSwatches {
                owner: cx.weak_entity(),
                id: id.to_owned(),
                mode,
                options,
                selection: self.custom_themes.selection(mode).map(str::to_owned),
                expanded: self.theme_navigation.expanded(id, mode),
            });
        }
        row
    }

    fn use_collection_variant(
        &mut self,
        t: Arc<InstalledTheme>,
        mode: ThemeMode,
        w: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if let Some(id) = t.file.collection() {
            self.theme_navigation
                .last_used
                .insert(id.to_owned(), t.file.id.clone());
        }
        self.select_palette_choice(&ThemeChoice::Custom(t), Some(mode), w, cx);
    }

    pub(super) fn select_theme_card(
        &mut self,
        choice: &ThemeChoice,
        group: &[Arc<InstalledTheme>],
        w: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if group.len() < 2 {
            self.select_palette_choice(choice, None, w, cx);
            return;
        }
        let first = &group[0].file;
        self.theme_navigation
            .last_used
            .insert(first.collection().unwrap().to_owned(), first.id.clone());
        let lib = Arc::make_mut(&mut self.custom_themes);
        for mode in [ThemeMode::Light, ThemeMode::Dark] {
            if let Some(t) = group.iter().find(|t| t.palette(mode).is_some()) {
                lib.select(mode, Some(t.file.id.clone()));
            }
        }
        self.apply_picker_change(w, cx);
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::views::workspace) struct VariantRemoval {
    label: String,
    members: Vec<Arc<InstalledTheme>>,
    checked: HashSet<String>,
}

impl Workspace {
    pub(super) fn open_variant_removal(
        &mut self,
        t: &InstalledTheme,
        members: Vec<Arc<InstalledTheme>>,
        w: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.theme_navigation.return_focus = w.focused(cx);
        self.open_overlay(Overlay::RemoveThemeVariants(VariantRemoval {
            label: t.file.collection_label().unwrap_or(&t.file.name).to_owned(),
            members,
            checked: HashSet::new(),
        }));
        self.focus_first_modal_tab_stop(w, cx);
        cx.notify();
    }
    pub(in crate::views::workspace) fn render_variant_removal(
        &self,
        removal: VariantRemoval,
        w: &Window,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        let title = format!("Remove themes from “{}”?", removal.label);
        let cards = div()
            .w_full()
            .grid()
            .grid_cols(if bruhs_ui::zoom::css_px(w.viewport_size().width) >= 640. {
                2
            } else {
                1
            })
            .gap_2()
            .children(removal.members.iter().map(|t| {
                let id = t.file.id.clone();
                let key = format!("remove-variant-{id}");
                let selected = removal.checked.contains(&id);
                let button = focusable_control(
                    div().id(SharedString::from(key.clone())),
                    Role::CheckBox,
                    t.file.name.clone(),
                    Interactivity::Enabled,
                )
                .debug_selector(move || key.clone())
                .aria_toggled(if selected {
                    Toggled::True
                } else {
                    Toggled::False
                })
                .relative()
                .min_w_0()
                .h(px(112.))
                .p_2()
                .rounded_lg()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap_2()
                .border_color(if selected {
                    theme::persimmon_400()
                } else {
                    theme::ring_modal()
                })
                .bg(if selected {
                    theme::longan_700()
                } else {
                    theme::longan_900()
                })
                .hover(|el| el.bg(theme::longan_700()))
                .child(
                    div()
                        .absolute()
                        .top_2()
                        .right_2()
                        .size(px(16.))
                        .rounded_sm()
                        .border_1()
                        .border_color(theme::rambutan_500())
                        .text_xs()
                        .flex()
                        .items_center()
                        .justify_center()
                        .when(selected, |el| {
                            el.bg(theme::persimmon_400())
                                .border_color(theme::persimmon_400())
                                .child(
                                    svg()
                                        .path(assets::CHECK_ICON_PATH)
                                        .size(px(12.))
                                        .text_color(theme::longan_950()),
                                )
                        }),
                )
                .child(
                    div().flex().gap_1().children(
                        [ThemeMode::Light, ThemeMode::Dark]
                            .into_iter()
                            .filter_map(|mode| {
                                t.palette(mode)
                                    .map(|p| div().size(px(44.)).child(color_disc(p)))
                            }),
                    ),
                )
                .child(
                    div()
                        .max_w_full()
                        .truncate()
                        .text_sm()
                        .child(t.file.name.clone()),
                );
                on_press_view(button, cx, move |ws, _, cx| {
                    if let Some(Overlay::RemoveThemeVariants(r)) = &mut ws.overlay {
                        if !r.checked.remove(&id) {
                            r.checked.insert(id.clone());
                        }
                        cx.notify();
                    }
                })
            }));
        let help = "Select the variants you want to remove. You can restore them by importing the collection again.";
        let content = div()
            .px_6()
            .pb_6()
            .pt_2()
            .flex()
            .flex_col()
            .gap_4()
            .child(
                div()
                    .text_sm()
                    .text_color(theme::rambutan_400())
                    .child(help),
            )
            .child(cards);
        let count = removal.checked.len();
        let label = if count == 0 {
            "Remove selected".into()
        } else {
            format!("Remove selected ({count})")
        };
        let confirm = web_button(
            "remove-variants-confirm",
            WebButtonVariant::Destructive,
            WebButtonSize::Small,
            ButtonState::disabled_if(count == 0),
        )
        .debug_selector(|| "remove-variants-confirm".into())
        .aria_label(label.clone())
        .child(label);
        let footer = div()
            .flex()
            .justify_end()
            .gap_2()
            .child(on_press_view(
                super::editor::theme_button("remove-variants-cancel", "Cancel", false),
                cx,
                |ws, w, cx| {
                    ws.dismiss_theme_editor(w, cx);
                },
            ))
            .child(confirm.when(count > 0, |el| {
                on_press_view(el, cx, |ws, w, cx| {
                    let Some(Overlay::RemoveThemeVariants(r)) = &ws.overlay else {
                        return;
                    };
                    if r.members
                        .iter()
                        .filter(|t| r.checked.contains(&t.file.id))
                        .any(|t| ws.custom_themes.find(&t.file.id) != Some(t))
                    {
                        ws.dismiss_theme_editor(w, cx);
                        ws.show_theme_import_notice(
                            "Your selected variants changed. Review the collection and try again."
                                .into(),
                            cx,
                        );
                        return;
                    }
                    let lib = Arc::make_mut(&mut ws.custom_themes);
                    lib.themes.retain(|t| !r.checked.contains(&t.file.id));
                    for id in &r.checked {
                        lib.forget_selection(id);
                    }
                    ws.dismiss_theme_editor(w, cx);
                    ws.apply_picker_change(w, cx);
                })
            }));
        self.render_modal_shell_sized(title, content, Some(footer), 512., w, cx)
    }
}

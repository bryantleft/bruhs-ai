use super::super::super::*;
use super::{Extension, Registry, Sort};
use bruhs_ui::press::on_press_view;
use futures_util::future::{AbortHandle, Abortable};
use std::time::Duration;

pub(in crate::views::workspace::appearance) struct Browser {
    owner: WeakEntity<Workspace>,
    pub(in crate::views::workspace::appearance) query: Entity<InputState>,
    registry: Registry,
    sort: Sort,
    sort_open: bool,
    sort_trigger: FocusHandle,
    sort_bounds: std::rc::Rc<std::cell::Cell<Bounds<Pixels>>>,
    sort_options: [FocusHandle; 4],
    results: Option<Vec<Extension>>,
    searching: bool,
    installing: Option<String>,
    error: Option<String>,
    revision: u64,
    request: Option<AbortHandle>,
    timer: Option<Task<()>>,
}

impl Drop for Browser {
    fn drop(&mut self) {
        if let Some(request) = self.request.take() {
            request.abort();
        }
    }
}

fn button(
    id: impl Into<SharedString>,
    label: impl Into<SharedString>,
    disabled: bool,
) -> Stateful<Div> {
    let id = id.into();
    let label = label.into();
    web_button(
        id.clone(),
        WebButtonVariant::Outline,
        WebButtonSize::Small,
        ButtonState::disabled_if(disabled),
    )
    .debug_selector(move || id.to_string())
    .aria_label(label.clone())
    .child(label)
}

impl Browser {
    pub(in crate::views::workspace::appearance) fn new(
        owner: WeakEntity<Workspace>,
        w: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> Self {
        let query = cx.new(|cx| InputState::new(w, cx).placeholder("Search themes…"));
        cx.subscribe(&query, |this, _, event: &InputEvent, cx| match event {
            InputEvent::Change => this.search(false, cx),
            InputEvent::PressEnter { .. } => this.search(true, cx),
            _ => {}
        })
        .detach();
        Self {
            owner,
            query,
            registry: Registry::default(),
            sort: Sort::Downloads,
            sort_open: false,
            sort_trigger: cx.focus_handle().tab_stop(true),
            sort_bounds: Default::default(),
            sort_options: std::array::from_fn(|_| cx.focus_handle().tab_stop(true)),
            results: None,
            searching: false,
            installing: None,
            error: None,
            revision: 0,
            request: None,
            timer: None,
        }
    }
    fn cancel_request(&mut self) {
        self.timer.take();
        if let Some(request) = self.request.take() {
            request.abort();
        }
        self.revision += 1;
    }
    fn search(&mut self, now: bool, cx: &mut Context<'_, Self>) {
        if self.installing.is_some() {
            return;
        }
        self.cancel_request();
        self.error = None;
        let query = self.query.read(cx).value().trim().to_owned();
        self.searching = !query.is_empty();
        self.sort_open = false;
        if query.is_empty() {
            self.results = None;
        }
        cx.notify();
        if query.is_empty() {
            return;
        }
        let revision = self.revision;
        let sort = self.sort;
        let registry = self.registry.clone();
        let (abort, registration) = AbortHandle::new_pair();
        self.request = Some(abort);
        self.timer = Some(cx.spawn(async move |this, cx| {
            if !now {
                cx.background_executor()
                    .timer(Duration::from_millis(350))
                    .await;
            }
            let request = cx.update(|cx| {
                crate::app_state::spawn_tokio(
                    cx,
                    Abortable::new(
                        async move { registry.search(&query, sort).await },
                        registration,
                    ),
                )
            });
            let Ok(result) = request.await else {
                return;
            };
            let _ = this.update(cx, |this, cx| {
                if this.revision != revision {
                    return;
                }
                this.searching = false;
                match result {
                    Ok(themes) => this.results = Some(themes),
                    Err(e) => {
                        this.results = None;
                        this.error = Some(e);
                    }
                }
                cx.notify();
            });
        }));
    }
    fn install(
        &mut self,
        ext: Extension,
        confirmed: bool,
        w: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if self.installing.is_some() {
            return;
        }
        let Some(owner) = self.owner.upgrade() else {
            return;
        };
        let expected = owner.read(cx).custom_themes.collection(&ext.collection);
        if !expected.is_empty() && !confirmed {
            let answer = w.prompt(
                PromptLevel::Warning,
                &format!("Update “{}”?", ext.name),
                Some("This replaces its installed variants, including any local edits. Variants no longer in the extension will be removed."),
                &[PromptButton::Cancel("Cancel".into()), PromptButton::Ok("Update theme".into())], cx,
            );
            cx.spawn_in(w, async move |this, cx| {
                if answer.await == Ok(1) {
                    let _ = this.update_in(cx, |this, w, cx| this.install(ext, true, w, cx));
                }
            })
            .detach();
            return;
        }
        self.cancel_request();
        self.searching = false;
        self.sort_open = false;
        self.error = None;
        self.installing = Some(ext.id.clone());
        let collection = ext.collection.clone();
        let (abort, registration) = AbortHandle::new_pair();
        self.request = Some(abort);
        let request = crate::app_state::spawn_tokio(
            cx,
            Abortable::new(async move { ext.install().await }, registration),
        );
        cx.spawn_in(w, async move |this, cx| {
            let Ok(result) = request.await else {
                return;
            };
            let _ = this.update_in(cx, |this, window, cx| {
                this.installing = None;
                let panel = cx.entity();
                let result = result.and_then(|themes| {
                    this.owner
                        .update(cx, |ws, cx| {
                            let Some(Overlay::ThemeEditor(editor)) = &ws.overlay else {
                                return Ok(());
                            };
                            if editor.community.as_ref() != Some(&panel) {
                                return Ok(());
                            }
                            let verb = if expected.is_empty() {
                                "added"
                            } else {
                                "updated"
                            };
                            let message = match themes.as_slice() {
                                [theme] => format!("{} {verb}", theme.file.name),
                                _ => format!("{} themes {verb}", themes.len()),
                            };
                            Arc::make_mut(&mut ws.custom_themes).replace_collection(
                                &collection,
                                &expected,
                                themes,
                            )?;
                            ws.dismiss_theme_editor(window, cx);
                            ws.show_theme_import_notice(message, cx);
                            ws.apply_picker_change(window, cx);
                            Ok::<_, String>(())
                        })
                        .unwrap_or(Ok(()))
                });
                if let Err(error) = result {
                    this.error = Some(error);
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    fn sort_control(&self, cx: &Context<'_, Self>) -> Div {
        let trigger = button(
            "community-sort",
            self.sort.label(),
            self.installing.is_some(),
        )
        .track_focus(&self.sort_trigger)
        .aria_label("Sort themes")
        .aria_expanded(self.sort_open)
        .child(svg().path(assets::CHEVRON_DOWN_ICON_PATH).size(px(12.)));
        let trigger = on_press_view(trigger, cx, |this, w, cx| {
            if this.installing.is_some() {
                return;
            }
            this.sort_open = !this.sort_open;
            if this.sort_open {
                let index = Sort::ALL.iter().position(|s| *s == this.sort).unwrap_or(0);
                let focus = this.sort_options[index].clone();
                cx.defer_in(w, move |_, w, cx| focus.focus(w, cx));
            }
            cx.notify();
        });
        div()
            .flex()
            .items_center()
            .gap_2()
            .child(
                div()
                    .text_xs()
                    .text_color(theme::rambutan_400())
                    .child("Sort"),
            )
            .child(
                div()
                    .relative()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .on_children_prepainted({
                                let bounds = self.sort_bounds.clone();
                                move |children, _, _| {
                                    if let Some(first) = children.first() {
                                        bounds.set(*first);
                                    }
                                }
                            })
                            .child(trigger),
                    )
                    .when(self.sort_open, |el| {
                        let menu = div()
                            .id("community-sort-menu")
                            .role(Role::Menu)
                            .aria_label("Sort themes")
                            .occlude()
                            .w(px(170.))
                            .p_1()
                            .rounded_md()
                            .border_1()
                            .border_color(theme::ring_modal())
                            .bg(theme::longan_900())
                            .flex()
                            .flex_col()
                            .on_mouse_down_out(cx.listener(
                                |this, event: &MouseDownEvent, _, cx| {
                                    if this.sort_bounds.get().contains(&event.position) {
                                        return;
                                    }
                                    this.sort_open = false;
                                    cx.notify();
                                },
                            ))
                            .capture_key_down(cx.listener(|this, event: &KeyDownEvent, w, cx| {
                                let current = this
                                    .sort_options
                                    .iter()
                                    .position(|f| f.is_focused(w))
                                    .unwrap_or(0);
                                match event.keystroke.key.as_str() {
                                    "down" => this.sort_options[(current + 1) % 4].focus(w, cx),
                                    "up" => this.sort_options[(current + 3) % 4].focus(w, cx),
                                    "escape" => {
                                        this.sort_open = false;
                                        this.sort_trigger.focus(w, cx);
                                        cx.notify();
                                    }
                                    _ => return,
                                }
                                w.prevent_default();
                                cx.stop_propagation();
                            }))
                            .children(Sort::ALL.into_iter().enumerate().map(|(i, sort)| {
                                let id = format!("community-sort-{}", sort.key());
                                let option = web_button(
                                    SharedString::from(id.clone()),
                                    WebButtonVariant::Ghost,
                                    WebButtonSize::Small,
                                    ButtonState::Enabled,
                                )
                                .debug_selector(move || id)
                                .track_focus(&self.sort_options[i])
                                .role(Role::MenuItemRadio)
                                .aria_label(sort.label())
                                .aria_toggled(if self.sort == sort {
                                    Toggled::True
                                } else {
                                    Toggled::False
                                })
                                .justify_start()
                                .child(sort.label());
                                on_press_view(option, cx, move |this, w, cx| {
                                    this.sort = sort;
                                    this.sort_trigger.focus(w, cx);
                                    this.search(true, cx);
                                })
                            }));
                        el.child(
                            div().relative().h_0().w_full().child(
                                div().absolute().top_0().right_0().size_0().child(
                                    deferred(
                                        anchored()
                                            .anchor(Anchor::TopRight)
                                            .offset(point(px(0.), px(4.)))
                                            .child(menu),
                                    )
                                    .with_priority(1),
                                ),
                            ),
                        )
                    }),
            )
    }

    fn card(&self, ext: &Extension, cx: &Context<'_, Self>) -> AnyElement {
        let installed = self.owner.upgrade().is_some_and(|ws| {
            !ws.read(cx)
                .custom_themes
                .collection(&ext.collection)
                .is_empty()
        });
        let running = self.installing.as_ref() == Some(&ext.id);
        let busy = self.installing.is_some();
        let action = match (installed, running) {
            (true, true) => "Updating…",
            (true, false) => "Update",
            (false, true) => "Installing…",
            _ => "Install",
        };
        let label = format!("{action} {}", ext.name);
        let item = ext.clone();
        let install = on_press_view(
            button(format!("community-install-{}", ext.id), action, busy).aria_label(label),
            cx,
            move |this, w, cx| this.install(item.clone(), false, w, cx),
        );
        let source = ext.source.clone().map(|url| {
            let id = format!("community-source-{}", ext.id);
            let title = format!("View source for {}", ext.name);
            let link = web_button(
                SharedString::from(id.clone()),
                WebButtonVariant::Ghost,
                WebButtonSize::IconSmall,
                ButtonState::Enabled,
            )
            .debug_selector(move || id)
            .aria_label(title.clone())
            .tooltip(move |w, cx| Tooltip::new(title.clone()).build(w, cx))
            .child(
                svg()
                    .path(assets::EXTERNAL_LINK_ICON_PATH)
                    .size(px(14.))
                    .text_color(theme::rambutan_400()),
            );
            on_press_view(link, cx, move |_, _, cx| cx.open_url(url.as_str()))
        });
        let mut icon = div()
            .size(px(36.))
            .flex_shrink_0()
            .flex()
            .items_center()
            .justify_center()
            .rounded_md()
            .bg(theme::longan_800());
        icon = if let Some(image) = &ext.image {
            icon.child(img(image.clone()).size_full().object_fit(ObjectFit::Cover))
        } else {
            icon.child(
                svg()
                    .path(assets::TOOL_SEARCH_ICON_PATH)
                    .size(px(16.))
                    .text_color(theme::rambutan_400()),
            )
        };
        let count = match ext.downloads {
            n if n >= 1_000_000 => {
                format!("{:.1}M", bruhs_models::num::ToFloatLossy::to_f64_lossy(n) / 1e6)
            }
            n if n >= 1000 => {
                format!("{:.1}K", bruhs_models::num::ToFloatLossy::to_f64_lossy(n) / 1000.)
            }
            n => n.to_string(),
        };
        div()
            .min_w_0()
            .p_3()
            .border_1()
            .border_color(theme::ring_modal())
            .rounded_lg()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div().flex().items_center().gap_2().child(icon).child(
                    div()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .child(div().truncate().text_sm().child(ext.name.clone()))
                        .child(
                            div()
                                .text_xs()
                                .truncate()
                                .text_color(theme::rambutan_400())
                                .child(format!("{} · {count} downloads", ext.publisher)),
                        ),
                ),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(theme::rambutan_400())
                    .line_clamp(2)
                    .child(if ext.description.is_empty() {
                        "A community color theme for your editor.".into()
                    } else {
                        ext.description.clone()
                    }),
            )
            .child(
                div()
                    .mt_auto()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(div().children(source))
                    .child(install),
            )
            .into_any_element()
    }
}

impl Render for Browser {
    fn render(&mut self, w: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let busy = self.installing.is_some();
        let popular = div()
            .flex()
            .items_center()
            .flex_wrap()
            .gap_1()
            .child(
                div()
                    .text_xs()
                    .text_color(theme::rambutan_400())
                    .child("Popular"),
            )
            .children(
                ["Dracula", "Catppuccin", "Nord", "Tokyo Night"].map(|name| {
                    on_press_view(
                        web_button(
                            SharedString::from(format!("community-popular-{name}")),
                            WebButtonVariant::Ghost,
                            WebButtonSize::Small,
                            ButtonState::disabled_if(busy),
                        )
                        .debug_selector(move || format!("community-popular-{name}"))
                        .aria_label(name)
                        .child(name),
                        cx,
                        move |this, w, cx| {
                            if !busy {
                                this.query.update(cx, |q, cx| q.set_value(name, w, cx));
                                this.search(true, cx);
                            }
                        },
                    )
                }),
            );
        let mut section = div()
            .flex()
            .flex_col()
            .gap_2()
            .child(div().text_sm().child("Search community themes"))
            .child(
                div()
                    .text_xs()
                    .text_color(theme::rambutan_400())
                    .child("Find open-source themes from Open VSX."),
            )
            .child(
                div().debug_selector(|| "community-search".into()).child(
                    Input::new(&self.query)
                        .aria_label("Search Open VSX themes")
                        .disabled(busy),
                ),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_2()
                    .items_center()
                    .justify_between()
                    .child(popular)
                    .when(
                        self.results.as_ref().is_some_and(|v| !v.is_empty()),
                        |row| row.child(self.sort_control(cx)),
                    ),
            );
        if let Some(e) = &self.error {
            section = section.child(
                div()
                    .id("community-theme-error")
                    .debug_selector(|| "community-theme-error".into())
                    .role(Role::Alert)
                    .text_sm()
                    .text_color(theme::lychee_200())
                    .child(e.clone()),
            );
        }
        if self.searching && self.results.is_none() {
            section = section.child(
                div()
                    .id("community-theme-progress")
                    .role(Role::Status)
                    .text_sm()
                    .child("Searching themes…"),
            );
        }
        if let Some(themes) = &self.results {
            if themes.is_empty() {
                section = section.child(
                    div()
                        .text_sm()
                        .child("No supported open-source themes found. Try a broader search."),
                );
            } else {
                section = section.child(
                    div()
                        .grid()
                        .grid_cols(if w.viewport_size().width >= px(640.) {
                            2
                        } else {
                            1
                        })
                        .gap_2()
                        .children(themes.iter().map(|ext| self.card(ext, cx))),
                );
            }
        }
        section
    }
}

impl Workspace {
    pub(in crate::views::workspace) fn dismiss_community_sort(
        &self,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> bool {
        let Some(Overlay::ThemeEditor(editor)) = &self.overlay else {
            return false;
        };
        let Some(browser) = &editor.community else {
            return false;
        };
        if !browser.read(cx).sort_open {
            return false;
        }
        browser.update(cx, |browser, cx| {
            browser.sort_open = false;
            browser.sort_trigger.focus(window, cx);
            cx.notify();
        });
        true
    }
}

#[cfg(test)]
impl Workspace {
    pub(in crate::views::workspace) fn community_registry_for_test(
        &self,
        origin: &str,
        cx: &mut App,
    ) {
        let Some(Overlay::ThemeEditor(editor)) = &self.overlay else {
            panic!("theme import dialog")
        };
        editor.community.as_ref().unwrap().update(cx, |browser, _| {
            browser.registry.origin = origin.parse().unwrap()
        });
    }
}

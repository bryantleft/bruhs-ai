//! Local CLI sign-in health, independent of release and package-registry checks.
use super::*;
use super::updates_ui::icon_svg;
use crate::app_state::spawn_tokio;
use bruhs_core::agent_extensions::availability::ProviderAvailability;
use std::time::Duration;

impl Workspace {
    pub(super) fn new_health_interval(
        seconds: u32,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
        subscriptions: &mut Vec<Subscription>,
    ) -> Entity<InputState> {
        let field = cx.new(|cx| InputState::new(window, cx).default_value(seconds.to_string()));
        subscriptions.push(cx.subscribe_in(
            &field,
            window,
            |this: &mut Self, field, event: &InputEvent, window, cx| {
                if !matches!(event, InputEvent::PressEnter { .. } | InputEvent::Blur) {
                    return;
                }
                let raw = field.read(cx).value().to_string();
                let next = raw
                    .trim()
                    .parse::<u32>()
                    .map(|n| if n == 0 { 0 } else { n.clamp(30, 86400) })
                    .unwrap_or(this.health_interval);
                if raw != next.to_string() {
                    field.update(cx, |input, cx| {
                        input.set_value(next.to_string(), window, cx)
                    });
                }
                if this.health_interval != next {
                    this.custom_background();
                    this.health_interval = next;
                    this.schedule_health_checks(cx);
                    this.persist_and_notify(cx);
                }
            },
        ));
        field
    }
    pub(super) fn check_provider_health(&mut self, cx: &mut Context<'_, Self>) {
        if self.settings_visible() && self.settings_section == SettingsSection::Updates { self.load_usage_hubs(cx); }
        if self.health_check.is_some() || self.provider_change_pending {
            return;
        }
        self.acp_models = crate::views::acp_models::load().unwrap_or_default();
        self.provider_availability = ProviderAvailability::load_for_project(self.settings_project_scope.as_ref()).map_err(|err| err.to_string());
        let Ok(prefs) = self.provider_availability.clone() else {
            cx.notify();
            return;
        };
        let probe = spawn_tokio(cx, crate::views::chat::cli_health(cx, prefs));
        self.health_check = Some(cx.spawn(async move |this, cx| {
            let mut health = probe.await;
            let _ = this.update(cx, |this, cx| {
                if let Some(previous) = &this.health_result {
                    for row in &mut health {
                        if row.status == bruhs_core::agent::health::Status::Disabled {
                            row.version = previous
                                .iter()
                                .find(|old| old.id == row.id)
                                .and_then(|old| old.version.clone());
                        }
                    }
                }
                this.acp_models = crate::views::acp_models::load().unwrap_or_default();
                this.health_result = Some(health);
                this.health_check = None;
                cx.notify();
            });
        }));
        cx.notify();
    }
    fn change_provider(
        &mut self,
        write: impl FnOnce() -> std::io::Result<()> + Send + 'static,
        cx: &mut Context<'_, Self>,
    ) {
        if self.provider_change_pending {
            return;
        }
        self.provider_change_pending = true;
        self.provider_change_error = None;
        let save = cx.background_spawn(async move { write() });
        cx.spawn(async move |this, cx| {
            let result = save.await;
            let _ = this.update(cx, |this, cx| {
                this.provider_change_pending = false;
                this.provider_change_error = result
                    .err()
                    .map(|err| format!("Could not change provider settings: {err}"));
                this.health_check = None;
                this.check_provider_health(cx);
            });
        })
        .detach();
        cx.notify();
    }

    pub(super) fn schedule_health_checks(&mut self, cx: &mut Context<'_, Self>) {
        self.health_timer = None;
        let seconds = self.health_interval;
        if seconds == 0 {
            return;
        }
        if !self.background_paused() {
            self.check_provider_health(cx);
        }
        self.health_timer = Some(cx.spawn(async move |this, cx| loop {
            cx.background_executor()
                .timer(Duration::from_secs(u64::from(seconds)))
                .await;
            if this
                .update(cx, |this, cx| {
                    if !this.background_paused() {
                        this.check_provider_health(cx)
                    }
                })
                .is_err()
            {
                return;
            }
        }));
    }
    pub(super) fn provider_health_card(&self, cx: &Context<'_, Self>) -> AnyElement {
        use bruhs_core::agent::health::{ProviderHealth, Status};
        let checking = self.health_check.is_some() || self.provider_change_pending;
        let refresh = web_button(
            "provider-health-check",
            WebButtonVariant::Outline,
            WebButtonSize::Small,
            ButtonState::disabled_if(checking),
        )
        .aria_label("Refresh provider status")
        .child(if checking { "Refreshing…" } else { "Refresh" });
        let mut rows = vec![self.defaults_scope_row(cx), self.settings_row(
            "settings-row-provider-health",
            "Provider status",
            "Versions, models, and sign-in status.",
            div()
                .flex()
                .flex_wrap()
                .gap_2()
                .child(on_press_view(refresh, cx, |this, _, cx| {
                    this.check_provider_health(cx)
                }))
                .child(on_press_view(
                    web_button(
                        "acp-add-provider",
                        WebButtonVariant::Outline,
                        WebButtonSize::Small,
                        ButtonState::disabled_if(self.provider_change_pending),
                    )
                    .track_focus(&self.provider_add_focus)
                    .aria_label("Add provider")
                    .debug_selector(|| "acp-add-provider".into())
                    .child("Add provider"),
                    cx,
                    |this, w, cx| this.open_provider_editor(None, w, cx),
                ))
                .into_any_element(),
        )];
        if let Some(error) = self
            .provider_change_error
            .as_ref()
            .or(self.provider_availability.as_ref().err())
        {
            rows.push(
                div()
                    .px_4()
                    .py_2()
                    .text_sm()
                    .text_color(theme::lychee_400())
                    .child(error.clone())
                    .into_any_element(),
            );
        }
        let providers = self.health_result.as_deref().unwrap_or_default();
        let status_label = |provider, name: &str, row: Option<&ProviderHealth>| {
            let version = row.and_then(|row| row.version.as_ref());
            let status = if self
                .provider_availability
                .as_ref()
                .is_ok_and(|prefs| !prefs.enabled(provider))
            {
                Some(Status::Disabled)
            } else {
                row.map(|row| row.status)
            };
            let status_text = status.map(Status::label).unwrap_or(if checking {
                "Checking…"
            } else {
                "Not checked"
            });
            let color = match status {
                Some(Status::Failed | Status::NotInstalled) => theme::lychee_400(),
                Some(Status::TimedOut | Status::SignedOut) => theme::durian_400(),
                _ => theme::rambutan_400(),
            };
            let mut details = status_text.to_owned();
            if let Some(count) = row
                .filter(|_| status != Some(Status::Disabled))
                .and_then(|row| row.model_count)
            {
                details.push_str(&format!(
                    " · {count} {}",
                    if count == 1 { "model" } else { "models" }
                ));
            }
            div()
                .w_full()
                .min_w_0()
                .flex()
                .items_center()
                .gap_3()
                .child(provider_identity_mark(
                    provider,
                    row.and_then(|r| r.accent_color),
                    cx,
                ))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(
                            div()
                                .flex()
                                .flex_wrap()
                                .items_center()
                                .gap_2()
                                .child(
                                    div()
                                        .font_weight(FontWeight::MEDIUM)
                                        .text_sm()
                                        .child(name.to_owned()),
                                )
                                .when_some(version, |el, version| {
                                    el.child(
                                        div()
                                            .text_xs()
                                            .text_color(theme::rambutan_500())
                                            .child(format!("v{version}")),
                                    )
                                }),
                        )
                        .child(
                            div()
                                .id(SharedString::from(format!(
                                    "provider-health-status-{}",
                                    row.map_or(provider.as_str(), |r| r.id.as_str())
                                )))
                                .role(gpui::accesskit::Role::Status)
                                .text_xs()
                                .text_color(color)
                                .child(details),
                        ),
                )
                .into_any_element()
        };
        for (provider, selector) in [
            (AgentProvider::Claude, "settings-health-claude"),
            (AgentProvider::Codex, "settings-health-codex"),
            (AgentProvider::Cursor, "settings-health-cursor"),
            (AgentProvider::Antigravity, "settings-health-antigravity"),
            (AgentProvider::Grok, "settings-health-grok"),
            (AgentProvider::OpenCode, "settings-health-opencode"),
        ] {
            let row = providers.iter().find(|row| row.provider == provider);
            let prefs = self.provider_availability.as_ref().ok();
            let enabled = prefs.is_some_and(|prefs| prefs.enabled(provider));
            let label = status_label(
                provider,
                provider.label(),
                row.filter(|r| !enabled || r.status != Status::Disabled),
            );
            let content = self.provider_switch(
                label,
                provider.as_str(),
                provider.label(),
                enabled,
                prefs.is_some_and(|p| p.environment_enabled(provider)),
                prefs.is_none() || self.provider_change_pending,
                cx,
                move |this, _, cx| {
                    this.change_provider(
                        move || ProviderAvailability::set_enabled(provider, !enabled),
                        cx,
                    )
                },
            );
            rows.push(self.settings_status_row(selector, provider.label(), content));
        }
        let custom = providers
            .iter()
            .filter(|row| row.provider == AgentProvider::Acp)
            .map(|row| {
                let label = status_label(row.provider, &row.name, Some(row));
                let Some(id) = row.id.strip_prefix("acp/") else {
                    return label;
                };
                let id = id.to_owned();
                let target = id.clone();
                let edit = web_button(
                    SharedString::from(format!("acp-edit-{id}")),
                    WebButtonVariant::Ghost,
                    WebButtonSize::Small,
                    ButtonState::disabled_if(self.provider_change_pending),
                )
                .aria_label(format!("Edit {}", row.name))
                .child("Edit");
                let label = div()
                    .flex()
                    .min_w_0()
                    .items_center()
                    .gap_2()
                    .child(div().min_w_0().flex_1().child(label))
                    .child(on_press_view(edit, cx, move |this, w, cx| {
                        this.open_provider_editor(Some(&target), w, cx)
                    }))
                    .into_any_element();
                let enabled = row.status != Status::Disabled;
                self.provider_switch(
                    label,
                    &row.id,
                    &row.name,
                    enabled,
                    row.environment_enabled,
                    checking,
                    cx,
                    move |this, _, cx| {
                        let id = id.clone();
                        this.change_provider(
                            move || bruhs_core::agent_extensions::set_acp_enabled(&id, !enabled),
                            cx,
                        );
                    },
                )
            })
            .collect::<Vec<_>>();
        if !custom.is_empty() {
            rows.push(
                self.settings_status_row(
                    "settings-health-acp",
                    "ACP agents",
                    div()
                        .w_full()
                        .flex()
                        .flex_col()
                        .gap_4()
                        .children(custom)
                        .into_any_element(),
                ),
            );
        }
        rows.push(self.settings_advanced_button(
            "provider-health-advanced",
            "Advanced provider settings",
            cx,
        ));
        if self.show_settings_advanced("provider-health-advanced") {
            rows.push(
                self.settings_row(
                    "settings-row-provider-health-interval",
                    "Provider health check interval",
                    "Seconds between background status checks. Set to 0 to refresh manually.",
                    settings_ui::control_column(
                        Input::new(&self.health_interval_input)
                            .aria_label("Provider health check interval in seconds")
                            .into_any_element(),
                    ),
                ),
            );
        }
        self.settings_card("Providers", "", rows)
    }
}

impl Workspace {
    fn provider_source_button(
        &self,
        id: &str,
        project: bool,
        device: bool,
        cx: &Context<'_, Workspace>,
    ) -> AnyElement {
        const MENU: &str = "project-provider-source";
        let open =
            self.settings_open_dropdown() == Some(MENU) && self.provider_override_source == id;
        let id = id.to_owned();
        let bounds = std::rc::Rc::new(std::cell::Cell::new(Bounds::default()));
        let observed = bounds.clone();
        let trigger = web_button(
            SharedString::from(format!("provider-source-{id}")),
            WebButtonVariant::Ghost,
            WebButtonSize::Small,
            ButtonState::Enabled,
        )
        .aria_label("Overridden for this project. Show where this value comes from")
        .tooltip(|window, cx| Tooltip::new("Overridden for this project").build(window, cx))
        .size(px(24.))
        .p_0()
        .child(icon_svg(
            assets::GIT_BRANCH_ICON_PATH,
            14.,
            theme::persimmon_400(),
        ));
        div()
            .relative()
            .flex_none()
            .on_children_prepainted(move |children, _, _| {
                if let Some(first) = children.first() {
                    observed.set(*first);
                }
            })
            .child(on_press_view(trigger, cx, move |ws, _, cx| {
                ws.provider_override_source.clone_from(&id);
                ws.set_settings_open_dropdown((!open).then_some(MENU));
                cx.notify();
            }))
            .when(open, |wrapper| {
                let menu = div()
                    .id("provider-setting-source")
                    .role(gpui::accesskit::Role::Group)
                    .aria_label("Provider setting sources")
                    .occlude()
                    .w(px(188.))
                    .rounded(px(8.))
                    .border_1()
                    .border_color(theme::ring_floating())
                    .bg(theme::longan_900())
                    .p_3()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .children([("Project", project), ("Device", device)].into_iter().map(
                        |(scope, enabled)| {
                            let effective = scope == "Project";
                            div()
                                .flex()
                                .items_center()
                                .justify_between()
                                .gap_3()
                                .rounded(px(5.))
                                .px_2()
                                .py_1()
                                .text_sm()
                                .when(effective, |row| {
                                    row.bg(theme::longan_800()).font_weight(FontWeight::MEDIUM)
                                })
                                .when(!effective, |row| row.text_color(theme::rambutan_400()))
                                .child(scope)
                                .child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .gap_1()
                                        .child(if enabled { "On" } else { "Off" })
                                        .child(div().w(px(14.)).when(effective, |el| {
                                            el.child(icon_svg(
                                                assets::CHECK_ICON_PATH,
                                                14.,
                                                theme::persimmon_400(),
                                            ))
                                        })),
                                )
                        },
                    ))
                    .on_mouse_down_out(cx.listener(move |ws, event: &MouseDownEvent, _, cx| {
                        if !bounds.get().contains(&event.position) {
                            ws.set_settings_open_dropdown(None);
                            cx.notify();
                        }
                    }));
                wrapper.child(div().absolute().top_0().left_0().child(
                    bruhs_ui::menu_popover::menu_popover(
                        size(px(24.), px(24.)),
                        menu.into_any_element(),
                    ),
                ))
            })
            .into_any_element()
    }

    fn provider_switch(
        &self,
        label: AnyElement,
        id: &str,
        name: &str,
        enabled: bool,
        environment_enabled: bool,
        disabled: bool,
        cx: &Context<'_, Workspace>,
        action: impl Fn(&mut Workspace, &mut Window, &mut Context<'_, Workspace>) + Clone + 'static,
    ) -> AnyElement {
        let switch = bruhs_ui::switch::switch(
            format!("provider-enabled-{id}"),
            format!("Enable {name}"),
            enabled,
            bruhs_ui::press::Interactivity::disabled_if(disabled),
        );
        let scope = self.settings_project_scope.clone();
        let target = id.to_owned();
        let overridden = !scope.is_empty()
            && self
                .provider_availability
                .as_ref()
                .is_ok_and(|p| p.project_override(id).is_some());
        let switch = switch.when(!disabled, |el| {
            on_press_view(el, cx, move |ws, w, cx| {
                if scope.is_empty() {
                    action(ws, w, cx);
                } else {
                    let scope = scope.clone();
                    let target = target.clone();
                    ws.change_provider(
                        move || {
                            ProviderAvailability::set_project_override(
                                scope.as_ref(),
                                &target,
                                Some(!enabled),
                            )
                        },
                        cx,
                    );
                }
            })
        });
        let scope = self.settings_project_scope.clone();
        let target = id.to_owned();
        div()
            .w_full()
            .flex()
            .items_center()
            .gap_3()
            .child(div().min_w_0().flex_1().child(label))
            .when(overridden, |row| {
                let reset = web_button(
                    SharedString::from(format!("reset-provider-{id}")),
                    WebButtonVariant::Ghost,
                    WebButtonSize::Small,
                    ButtonState::disabled_if(disabled),
                )
                .aria_label(format!("Reset {name} to inherited setting"))
                .child("Reset");
                row.child(self.provider_source_button(id, enabled, environment_enabled, cx)).child(reset.when(!disabled, |el| {
                    on_press_view(el, cx, move |ws, _, cx| {
                        let scope = scope.clone();
                        let target = target.clone();
                        ws.change_provider(
                            move || {
                                ProviderAvailability::set_project_override(
                                    scope.as_ref(),
                                    &target,
                                    None,
                                )
                            },
                            cx,
                        );
                    })
                }))
            })
            .child(switch)
            .into_any_element()
    }
}

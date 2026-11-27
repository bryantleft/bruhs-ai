//! Show the effective value and individual project overrides for a scoped setting.
use super::updates_ui::icon_svg;
use super::*;
use project_defaults::ProjectDefaults;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum ScopedSwitch {
    Restart,
    Resume,
    Snooze,
    Merged,
    KeepPull,
    AutoPull,
    Remote,
}

impl ScopedSwitch {
    fn project(self, prefs: &ProjectDefaults) -> Option<bool> {
        match self {
            Self::Restart => prefs.continue_threads,
            Self::Resume => prefs.resume_limited,
            Self::Snooze => prefs.snooze_limited,
            Self::Merged => prefs.settle_merged,
            Self::KeepPull => prefs.keep_pull,
            Self::AutoPull => prefs.auto_pull,
            Self::Remote => prefs.start_from_remote,
        }
    }

    fn set_project(self, prefs: &mut ProjectDefaults, value: Option<bool>) {
        let slot = match self {
            Self::Restart => &mut prefs.continue_threads,
            Self::Resume => &mut prefs.resume_limited,
            Self::Snooze => &mut prefs.snooze_limited,
            Self::Merged => &mut prefs.settle_merged,
            Self::KeepPull => &mut prefs.keep_pull,
            Self::AutoPull => &mut prefs.auto_pull,
            Self::Remote => &mut prefs.start_from_remote,
        };
        *slot = value;
    }

    fn environment(self, ws: &mut Workspace) -> &mut bool {
        match self {
            Self::Restart => &mut ws.maintenance.prefs.continue_threads,
            Self::Resume => &mut ws.quota_prefs.resume,
            Self::Snooze => &mut ws.quota_prefs.snooze,
            Self::Merged => &mut ws.pull_preferences.merged,
            Self::KeepPull => &mut ws.pull_preferences.keep,
            Self::AutoPull => &mut ws.maintenance.prefs.auto_pull,
            Self::Remote => &mut ws.worktree_prefs.start_from_remote,
        }
    }

    fn default_value(self) -> bool {
        matches!(self, Self::Restart)
    }

    fn changed(self, ws: &mut Workspace, cx: &mut Context<'_, Workspace>) {
        if self == Self::Resume {
            ws.cancel_disabled_quota_recoveries(cx);
        }
        if self == Self::AutoPull {
            ws.schedule_project_refresh(cx);
        }
        if matches!(
            self,
            Self::AutoPull | Self::Remote | Self::Resume | Self::Snooze
        ) {
            ws.stamp_run_prefs_on_threads(cx);
        }
        ws.persist_and_notify(cx);
    }

    pub(super) fn choose(
        self,
        ws: &mut Workspace,
        value: Option<bool>,
        cx: &mut Context<'_, Workspace>,
    ) {
        if ws.settings_project_scope.is_empty() {
            *self.environment(ws) = value.unwrap_or_else(|| self.default_value());
        } else {
            let key = &ws.settings_project_scope;
            let prefs = ws.project_defaults.entry(key.clone()).or_default();
            self.set_project(prefs, value);
            if *prefs == Default::default() {
                ws.project_defaults.remove(key);
            }
        }
        self.changed(ws, cx);
    }
}
pub(super) trait ProjectSetting: Copy + 'static {
    fn project_label(self, prefs: &ProjectDefaults) -> Option<SharedString>;
    fn clear_project(self, prefs: &mut ProjectDefaults);
    fn repository(self, _ws: &Workspace) -> Option<(&'static str, Option<SharedString>)> {
        None
    }
    fn before_reset(self, _ws: &mut Workspace, _w: &mut Window, _cx: &mut Context<'_, Workspace>) {}
    fn after_reset(self, ws: &mut Workspace, window: &mut Window, cx: &mut Context<'_, Workspace>);

    fn clear_projects(
        self,
        ws: &mut Workspace,
        window: &mut Window,
        cx: &mut Context<'_, Workspace>,
    ) {
        self.before_reset(ws, window, cx);
        ws.project_defaults.retain(|_, prefs| {
            if self.project_label(prefs).is_none() {
                return true;
            }
            self.clear_project(prefs);
            *prefs != Default::default()
        });
        ws.set_settings_open_dropdown(None);
        self.after_reset(ws, window, cx);
    }
}

impl ProjectSetting for ScopedSwitch {
    fn project_label(self, prefs: &ProjectDefaults) -> Option<SharedString> {
        self.project(prefs).map(|v| display_value(v).into())
    }
    fn clear_project(self, prefs: &mut ProjectDefaults) {
        self.set_project(prefs, None);
    }
    fn after_reset(self, ws: &mut Workspace, _: &mut Window, cx: &mut Context<'_, Workspace>) {
        self.changed(ws, cx);
    }
}

fn display_value(value: bool) -> &'static str {
    if value {
        "On"
    } else {
        "Off"
    }
}

fn source_value(value: SharedString) -> Stateful<Div> {
    let hint = value.clone();
    div()
        .id("source-value")
        .min_w_0()
        .max_w(px(145.))
        .truncate()
        .tooltip(move |w, cx| Tooltip::new(hint.clone()).build(w, cx))
        .child(value)
}

fn source_layer(
    label: &'static str,
    value: Option<SharedString>,
    effective: bool,
) -> Stateful<Div> {
    div()
        .id(label)
        .flex()
        .items_center()
        .justify_between()
        .gap_2()
        .px_2()
        .py_1()
        .rounded(px(5.))
        .text_sm()
        .when(effective, |row| {
            row.bg(theme::longan_800()).font_weight(FontWeight::MEDIUM)
        })
        .when(!effective, |row| row.text_color(theme::rambutan_400()))
        .child(label)
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(source_value(value.unwrap_or_else(|| "Inherits".into())))
                .child(div().w(px(14.)).when(effective, |el| {
                    el.child(icon_svg(
                        assets::CHECK_ICON_PATH,
                        14.,
                        theme::persimmon_400(),
                    ))
                })),
        )
}

impl Workspace {
    fn setting_override_projects(
        &self,
        setting: impl ProjectSetting,
        row: &'static str,
        count: usize,
        cx: &Context<'_, Self>,
    ) -> Div {
        let clear = web_button(
            "setting-source-clear",
            WebButtonVariant::Ghost,
            WebButtonSize::Small,
            ButtonState::Enabled,
        )
        .debug_selector(|| "setting-source-clear".into())
        .aria_label("Reset project overrides for this setting")
        .child(if count == 1 { "Reset it" } else { "Reset all" });
        let heading = div()
            .flex()
            .items_center()
            .justify_between()
            .pl_2()
            .text_xs()
            .text_color(theme::rambutan_400())
            .child("Overridden by")
            .child(on_press_view(clear, cx, move |ws, window, cx| {
                let trigger = ws.settings_dropdown_trigger.take();
                setting.clear_projects(ws, window, cx);
                if let Some(trigger) = trigger {
                    trigger.focus(window, cx);
                }
            }));
        let mut projects = div()
            .id("source-project-scroll")
            .max_h(px(160.))
            .overflow_y_scroll()
            .flex()
            .flex_col();
        let mut first = true;
        for option in self.settings_project_options(cx) {
            let path = option.value;
            let Some(value) = self
                .project_defaults
                .get(path.as_ref())
                .and_then(|p| setting.project_label(p))
            else {
                continue;
            };
            let item_id = SharedString::from(format!("setting-source-project-{path}"));
            let button = web_button(
                item_id.clone(),
                WebButtonVariant::Ghost,
                WebButtonSize::Small,
                ButtonState::Enabled,
            )
            .when(first, |button| {
                button.track_focus(&self.first_override_project)
            })
            .debug_selector(move || item_id.to_string())
            .aria_label(format!("Open {path} settings"))
            .w_full()
            .h_auto()
            .min_h(px(32.))
            .flex_none()
            .justify_between()
            .gap_2()
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .items_start()
                    .child(div().truncate().w_full().child(option.label))
                    .child(
                        div()
                            .truncate()
                            .w_full()
                            .text_xs()
                            .text_color(theme::rambutan_400())
                            .child(path.clone()),
                    ),
            )
            .child(source_value(value));
            first = false;
            projects = projects.child(on_press_view(button, cx, move |ws, window, cx| {
                let trigger = ws.settings_dropdown_trigger.take();
                ws.select_settings_option(
                    "settings-project-defaults-scope",
                    path.as_ref(),
                    window,
                    cx,
                );
                ws.expand_settings_row(row);
                ws.reveal_settings_row(row.into(), cx);
                if let Some(trigger) = trigger {
                    trigger.focus(window, cx);
                }
                cx.notify();
            }));
        }
        div()
            .border_t_1()
            .border_color(theme::ring_floating())
            .flex()
            .flex_col()
            .child(heading)
            .child(projects)
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "renders scoped setting source badge with explicit ids, labels, and fallback values"
    )]
    pub(super) fn setting_source(
        &self,
        setting: impl ProjectSetting,
        id: &'static str,
        row: &'static str,
        label: &'static str,
        environment: Option<SharedString>,
        builtin: SharedString,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        let project = self
            .project_defaults
            .get(&self.settings_project_scope)
            .and_then(|p| setting.project_label(p));
        let scoped = !self.settings_project_scope.is_empty();
        let has_environment = environment.is_some();
        let repository = if scoped {
            setting.repository(self)
        } else {
            None
        };
        let repo_effective = !has_environment
            && project.is_none()
            && repository.as_ref().is_some_and(|(_, v)| v.is_some());
        let count = if scoped {
            0
        } else {
            self.project_defaults
                .values()
                .filter(|p| setting.project_label(p).is_some())
                .count()
        };
        let origin = match (project.as_ref(), has_environment) {
            (Some(_), _) => "Overridden for this project",
            (None, true) => "Environment setting",
            _ if repo_effective => "Inherited from repository config",
            _ => "Built-in default",
        };
        let summary = match count {
            0 => origin.to_owned(),
            1 => format!("{origin} · 1 project override"),
            _ => format!("{origin} · {count} project overrides"),
        };
        let open = self.settings_open_dropdown() == Some(row);
        let bounds = Rc::new(std::cell::Cell::new(Bounds::default()));
        let observed = bounds.clone();
        let trigger_id = SharedString::from(format!("{id}-source"));
        let trigger = web_button(
            SharedString::from(format!("{row}-source")),
            WebButtonVariant::Ghost,
            WebButtonSize::IconSmall,
            ButtonState::Enabled,
        )
        .when(
            !open && project.is_none() && !repo_effective && count == 0,
            |button| {
                button
                    .opacity(0.)
                    .group_hover(row, |style| style.opacity(1.))
                    .focus_visible(|style| style.opacity(1.))
            },
        )
        .debug_selector(move || trigger_id.to_string())
        .aria_label(format!("{summary}. Show where this value comes from"))
        .aria_expanded(open)
        .tooltip(move |window, cx| Tooltip::new(summary.clone()).build(window, cx))
        .size(px(24.))
        .p_0()
        .child(icon_svg(
            assets::GIT_BRANCH_ICON_PATH,
            14.,
            if project.is_some() {
                theme::persimmon_400()
            } else {
                theme::rambutan_400()
            },
        ));
        let source = div()
            .relative()
            .flex_none()
            .on_children_prepainted(move |children, _, _| {
                if let Some(first) = children.first() {
                    observed.set(*first);
                }
            })
            .child(on_press_view(trigger, cx, move |ws, window, cx| {
                ws.set_settings_open_dropdown((!open).then_some(row.into()));
                if !open {
                    ws.settings_dropdown_trigger = window.focused(cx);
                    if count > 0 {
                        let first = ws.first_override_project.clone();
                        cx.defer_in(window, move |_, w, cx| first.focus(w, cx));
                    }
                }
                cx.notify();
            }))
            .when(open, |wrapper| {
                let mut panel = div()
                    .id("scoped-setting-source")
                    .role(gpui::accesskit::Role::Group)
                    .aria_label(format!("{label} sources"))
                    .occlude()
                    .w(px(270.))
                    .rounded(px(8.))
                    .border_1()
                    .border_color(theme::ring_floating())
                    .bg(theme::longan_900())
                    .p_2()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .px_2()
                            .py_1()
                            .text_xs()
                            .text_color(theme::rambutan_400())
                            .child("This machine"),
                    )
                    .when(scoped, |el| {
                        el.child(source_layer("Project", project.clone(), project.is_some()))
                    })
                    .child(source_layer(
                        "Environment",
                        environment,
                        project.is_none() && has_environment,
                    ))
                    .when_some(repository, |el, (name, value)| {
                        el.child(source_layer(name, value, repo_effective))
                    })
                    .child(source_layer(
                        "Default",
                        Some(builtin),
                        project.is_none() && !has_environment && !repo_effective,
                    ));
                if count > 0 {
                    panel = panel.child(self.setting_override_projects(setting, row, count, cx));
                }
                let panel = panel.on_mouse_down_out(cx.listener(
                    move |ws, event: &MouseDownEvent, _, cx| {
                        if !bounds.get().contains(&event.position) {
                            ws.set_settings_open_dropdown(None);
                            cx.notify();
                        }
                    },
                ));
                wrapper.child(div().absolute().top_0().left_0().child(
                    bruhs_ui::menu_popover::menu_popover(
                        size(px(24.), px(24.)),
                        panel.into_any_element(),
                    ),
                ))
            });
        source.into_any_element()
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "renders a scoped switch row with explicit setting descriptors and environment value"
    )]
    pub(super) fn scoped_switch_row(
        &self,
        setting: ScopedSwitch,
        id: &'static str,
        row: &'static str,
        label: &'static str,
        description: &'static str,
        environment: bool,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        let project = self
            .project_defaults
            .get(&self.settings_project_scope)
            .and_then(|p| setting.project(p));
        let scoped = !self.settings_project_scope.is_empty();
        let effective = project.unwrap_or(environment);
        let builtin = setting.default_value();
        let has_environment = environment != builtin;
        let source = self.setting_source(
            setting,
            id,
            row,
            label,
            has_environment.then(|| display_value(environment).into()),
            display_value(builtin).into(),
            cx,
        );
        let accessories = div().flex().items_center().child(source).when(
            if scoped {
                project.is_some()
            } else {
                has_environment
            },
            |el| {
                let reset_id = SharedString::from(format!(
                    "settings-reset-{}",
                    id.trim_start_matches("settings-")
                ));
                let reset = web_button(
                    reset_id.clone(),
                    WebButtonVariant::Ghost,
                    WebButtonSize::IconSmall,
                    ButtonState::Enabled,
                )
                .debug_selector(move || reset_id.to_string())
                .aria_label(format!("Reset {label}"))
                .tooltip(move |window, cx| {
                    Tooltip::new(if scoped {
                        "Use environment setting"
                    } else {
                        "Restore built-in default"
                    })
                    .build(window, cx)
                })
                .size(px(24.))
                .p_0()
                .child(icon_svg(
                    assets::ROTATE_CCW_ICON_PATH,
                    14.,
                    theme::rambutan_400(),
                ));
                el.child(on_press_view(reset, cx, move |ws, _, cx| {
                    setting.choose(ws, None, cx)
                }))
            },
        );
        let control = settings_ui::switch(id, effective, false, label, cx, move |ws, _, cx| {
            setting.choose(ws, Some(!effective), cx)
        });
        self.settings_row_with_reset(
            row,
            label,
            description,
            None,
            false,
            Some(control),
            Some(accessories.into_any_element()),
        )
    }
}

impl Workspace {
    #[expect(
        clippy::too_many_arguments,
        reason = "renders source badge and reset action with explicit setting descriptors"
    )]
    pub(super) fn setting_source_actions(
        &self,
        setting: impl ProjectSetting,
        id: &'static str,
        row: &'static str,
        label: &'static str,
        environment: Option<SharedString>,
        default: &'static str,
        changed: bool,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        let source = self.setting_source(setting, id, row, label, environment, default.into(), cx);
        if !changed {
            return source;
        }
        let key = SharedString::from(format!("{id}-reset"));
        let reset = web_button(
            key.clone(),
            WebButtonVariant::Ghost,
            WebButtonSize::IconSmall,
            ButtonState::Enabled,
        )
        .debug_selector(move || key.to_string())
        .aria_label(format!("Reset {label} to the inherited value"))
        .child(
            svg()
                .path(assets::ROTATE_CCW_ICON_PATH)
                .size(px(14.))
                .text_color(theme::rambutan_400()),
        );
        div()
            .flex()
            .items_center()
            .child(source)
            .child(on_press_view(reset, cx, move |ws, w, cx| {
                ws.select_settings_option(id, "inherit", w, cx);
            }))
            .into_any_element()
    }
}

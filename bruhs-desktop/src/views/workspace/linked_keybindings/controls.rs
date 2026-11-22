use super::*;

impl Workspace {
    pub(in crate::views::workspace) fn linked_shortcut_status(
        &self,
        cx: &Context<'_, Self>,
    ) -> Option<AnyElement> {
        let state = &self.linked_keybindings;
        if state.hosts.is_empty() && state.retry.is_none() && state.error.is_none() {
            return None;
        }
        let names = state
            .hosts
            .iter()
            .filter(|h| !h.data.editable && h.problem.is_none())
            .map(LinkedHost::name)
            .collect::<Vec<_>>();
        let detail = if let Some(error) = &state.error {
            error.clone()
        } else if let Some(problem) = state.hosts.iter().find_map(|h| h.problem.as_ref()) {
            let count = state.hosts.iter().filter(|h| h.problem.is_none()).count() + 1;
            if state.editable() {
                format!(
                    "{problem} Edits apply to {count} connected environment{}.",
                    if count == 1 { "" } else { "s" }
                )
            } else {
                problem.clone()
            }
        } else if state.busy {
            "Checking linked environments…".into()
        } else if !names.is_empty() {
            format!("Read-only access on {}.", names.join(", "))
        } else {
            format!(
                "Viewing {}. Edits apply to all {} environments.",
                state
                    .selected
                    .and_then(|i| state.hosts.get(i))
                    .map(LinkedHost::name)
                    .unwrap_or_else(|| "this machine".into()),
                1 + state.hosts.len()
            )
        };
        let mut action = div().flex().flex_wrap().gap_2();
        if state.retry.is_none() && !state.hosts.is_empty() {
            action = action.child(on_press_view(
                shortcut_button("refresh-shortcut-hosts", "Refresh", state.busy),
                cx,
                |this, _, cx| this.refresh_linked_shortcuts(cx),
            ));
        }
        if state.hosts.iter().any(|h| h.problem.is_some()) {
            action = action.child(on_press_view(
                shortcut_button("manage-shortcut-hosts", "Environments", false),
                cx,
                |this, _, cx| {
                    this.set_settings_section(SettingsSection::Environments, cx);
                    this.reveal_settings_row("settings-linked-shortcuts".into(), cx);
                },
            ));
        }
        if state.retry.is_some() {
            action = action
                .child(on_press_view(
                    shortcut_button("retry-shortcut-sync", "Retry", state.busy),
                    cx,
                    |this, _, cx| {
                        if this.linked_keybindings.busy {
                            return;
                        }
                        if let Some(edit) = this.linked_keybindings.retry.take() {
                            this.save_linked_edit(edit, cx);
                        }
                    },
                ))
                .child(on_press_view(
                    shortcut_button("stop-shortcut-sync", "Dismiss", state.busy),
                    cx,
                    |this, _, cx| {
                        if this.linked_keybindings.busy {
                            return;
                        }
                        this.linked_keybindings.retry = None;
                        this.linked_keybindings.error = None;
                        cx.notify();
                    },
                ));
        }
        Some(self.settings_row(
            "linked-shortcut-status",
            "Linked environments",
            detail,
            action.into_any_element(),
        ))
    }

    pub(in crate::views::workspace) fn shortcut_environment_rows(
        &self,
        cx: &Context<'_, Self>,
    ) -> Vec<AnyElement> {
        let state = &self.linked_keybindings;
        let mut rows = Vec::new();
        if !state.hosts.is_empty() {
            rows.push(
                self.settings_row(
                    "local-shortcuts-environment",
                    "This machine",
                    "Local keybindings",
                    on_press_view(
                        shortcut_button("view-local-shortcuts", "View keybindings", state.busy),
                        cx,
                        |this, _, cx| this.view_host_shortcuts(None, cx),
                    )
                    .into_any_element(),
                ),
            );
        }
        for (index, host) in state.hosts.iter().enumerate() {
            let name = host.name();
            let host = host.clone();
            let id = host.identity();
            rows.push(
                self.settings_row(
                    format!("shortcut-host-{id}"),
                    name,
                    host.problem.clone().unwrap_or_else(|| {
                        if host.data.editable {
                            "Can edit shortcuts".into()
                        } else {
                            "Read-only shortcuts".into()
                        }
                    }),
                    div()
                        .flex()
                        .flex_wrap()
                        .gap_2()
                        .child(on_press_view(
                            shortcut_button(
                                format!("view-shortcuts-{id}"),
                                "View keybindings",
                                state.busy,
                            ),
                            cx,
                            move |this, _, cx| this.view_host_shortcuts(Some(index), cx),
                        ))
                        .child(on_press_view(
                            shortcut_button(
                                format!("unlink-shortcuts-{id}"),
                                "Disconnect",
                                state.busy,
                            ),
                            cx,
                            move |this, _, cx| {
                                this.disconnect_shortcut_host(&host, cx);
                            },
                        ))
                        .into_any_element(),
                ),
            );
        }
        let controls = if let Some(form) = &state.form {
            div()
                .flex()
                .flex_col()
                .gap_2()
                .when(form.tunnel.is_none(), |el| {
                    el.child(Input::new(&form.origin).aria_label("Environment HTTPS address"))
                })
                .when_some(form.tunnel.as_ref(), |el, t| el.child(t.name.clone()))
                .child(Input::new(&form.token).aria_label("Shortcut access token"))
                .child(
                    div()
                        .flex()
                        .gap_2()
                        .child(on_press_view(
                            shortcut_button("connect-shortcuts", "Connect", state.busy),
                            cx,
                            |this, _, cx| this.connect_shortcut_host(cx),
                        ))
                        .child(on_press_view(
                            shortcut_button("cancel-shortcut-link", "Cancel", state.busy),
                            cx,
                            |this, _, cx| {
                                if this.linked_keybindings.busy {
                                    return;
                                }
                                this.linked_keybindings.form = None;
                                if this.linked_keybindings.retry.is_none() {
                                    this.linked_keybindings.error = None;
                                }
                                cx.notify();
                            },
                        )),
                )
        } else {
            div().child(on_press_view(
                shortcut_button(
                    "link-shortcut-host",
                    "Link environment",
                    state.busy || state.retry.is_some(),
                ),
                cx,
                |this, w, cx| {
                    this.open_shortcut_link(None, w, cx);
                },
            ))
        };
        let detail = state.error.as_deref().unwrap_or(
            "Link shortcuts for this session using a token from the host’s task access settings.",
        );
        let controls = controls
            .flex()
            .flex_col()
            .items_start()
            .gap_2()
            .child(on_press_view(
                shortcut_button("manage-shortcut-access", "Manage access", false),
                cx,
                |_, _, cx| {
                    if let Ok(url) =
                        crate::server_client::ServerClient::workbench_client().shortcut_access_url()
                    {
                        cx.open_url(url.as_str());
                    }
                },
            ));
        rows.push(self.settings_row(
            "settings-linked-shortcuts",
            "Shared keybindings",
            detail,
            controls.into_any_element(),
        ));
        rows
    }
}

pub(super) fn shortcut_button(
    id: impl Into<SharedString>,
    label: &'static str,
    disabled: bool,
) -> gpui::Stateful<gpui::Div> {
    let id = id.into();
    web_button(
        id.clone(),
        WebButtonVariant::Outline,
        WebButtonSize::Small,
        ButtonState::disabled_if(disabled),
    )
    .debug_selector(move || id.to_string())
    .aria_label(label)
    .child(label)
}

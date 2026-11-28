use super::*;
use crate::app_state::spawn_tokio;
use crate::views::browser_preferences::{BrowserAppearance, BrowserPreferences};

/// Deep links to individual files and comments retain their web destination.
fn github_review_number(url: &reqwest::Url) -> Option<u64> {
    if url.host_str() != Some("github.com")
        || url.port().is_some()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return None;
    }
    let path = url.path().trim_end_matches('/');
    let mut segments = path.strip_prefix('/')?.split('/');
    let owner = segments.next()?;
    let repo = segments.next()?;
    if owner.is_empty() || repo.is_empty() || segments.next()? != "pull" {
        return None;
    }
    let number = segments.next()?.parse::<u64>().ok()?;
    (number > 0 && segments.next().is_none()).then_some(number)
}

impl Workspace {
    pub(super) fn displayed_pull(&self) -> Option<&DesktopBranchPull> {
        self.linked_pull
            .as_ref()
            .filter(|(thread, _)| thread == &self.active_thread_id)
            .map(|(_, pull)| pull)
            .or(self.branch_pull.as_ref())
    }

    fn open_review_link(
        &mut self,
        number: u64,
        url: &reqwest::Url,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if self.active_view != AppView::Chat {
            self.open_chat_view(&OpenChatView, window, cx);
        }
        let address = url.as_str().trim_end_matches('/').to_owned();
        let thread = self.active_thread_id.clone();
        let cached = self
            .branch_pull
            .as_ref()
            .filter(|pull| pull.url.trim_end_matches('/') == address)
            .cloned();
        let needs_lookup = cached.is_none();
        self.linked_pull = Some((
            thread.clone(),
            cached.unwrap_or_else(|| DesktopBranchPull {
                number,
                title: "Loading pull request…".into(),
                url: address.clone(),
                state: String::new(),
            }),
        ));
        self.show_pull_surface(cx);
        if !needs_lookup {
            return;
        }
        let fetch_address = address.clone();
        let loaded = spawn_tokio(cx, async move {
            bruhs_core::pulls::view_pull(Path::new("."), &fetch_address).await
        });
        cx.spawn(async move |workspace, cx| {
            let result = loaded.await;
            let _ = workspace.update(cx, |this, cx| {
                let Some((target, pull)) = this.linked_pull.as_mut() else {
                    return;
                };
                if target != &thread || pull.url != address {
                    return;
                }
                match result {
                    Ok(found) => {
                        pull.title = found.title;
                        pull.state = found.state;
                    }
                    Err(reason) => {
                        pull.title = reason;
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn new_browser_settings(
        preferences: BrowserPreferences,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
        subscriptions: &mut Vec<Subscription>,
    ) -> Entity<InputState> {
        let value = preferences.custom_url.clone();
        cx.set_global(preferences);
        let field = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("https://example.com/search?q=%s")
                .default_value(value)
        });
        subscriptions.push(cx.subscribe(&field, |this, field, event: &InputEvent, cx| {
            if !matches!(event, InputEvent::Blur | InputEvent::PressEnter { .. }) {
                return;
            }
            let value = field.read(cx).value().trim().to_owned();
            if cx.global::<BrowserPreferences>().custom_url != value {
                cx.default_global::<BrowserPreferences>().custom_url = value;
                this.persist_and_notify(cx);
            }
        }));
        field
    }

    pub(super) fn open_web_link(
        &mut self,
        action: &bruhs_ui::text::OpenWebLink,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let Ok(url) = reqwest::Url::parse(&action.0) else {
            return;
        };
        if !matches!(url.scheme(), "http" | "https") || url.host().is_none() {
            return;
        }
        if self.diff_panel.read(cx).prefs.open_pull_links_in_app {
            if let Some(number) = github_review_number(&url) {
                self.open_review_link(number, &url, window, cx);
                return;
            }
        }
        if BrowserPreferences::get(cx).open_links_in_app
            && crate::views::preview_panel::WebViewHost::available()
        {
            if self.active_view != AppView::Chat {
                self.open_chat_view(&OpenChatView, window, cx);
            }
            self.set_preview_panel_visible(true, cx);
            self.preview_panel
                .update(cx, |panel, cx| panel.go_to(url.as_str(), window, cx));
        } else {
            cx.open_url(url.as_str());
        }
    }

    pub(super) fn change_browser_preference(
        &mut self,
        id: &str,
        value: &str,
        cx: &mut Context<'_, Self>,
    ) {
        let prefs = cx.default_global::<BrowserPreferences>();
        match id {
            "settings-browser-link-target" => prefs.open_links_in_app = value == "app",
            "settings-browser-search-engine"
                if BrowserPreferences::ENGINES
                    .iter()
                    .any(|(key, _)| *key == value) =>
            {
                prefs.engine = value.into()
            }
            "settings-browser-appearance" => {
                prefs.appearance = match value {
                    "light" => BrowserAppearance::Light,
                    "dark" => BrowserAppearance::Dark,
                    "system" => BrowserAppearance::System,
                    _ => return,
                }
            }
            "settings-browser-zoom"
                if BrowserPreferences::ZOOM
                    .iter()
                    .any(|(key, _)| *key == value) =>
            {
                if let Ok(percent) = value.parse() {
                    prefs.zoom_percent = percent;
                }
            }
            "settings-browser-recording-frame-rate"
                if BrowserPreferences::FRAME_RATES
                    .iter()
                    .any(|(key, _)| *key == value) =>
            {
                if let Ok(fps) = value.parse() {
                    prefs.recording_frame_rate = fps;
                }
            }
            _ => return,
        }
        self.persist_and_notify(cx);
        cx.refresh_windows();
    }

    pub(super) fn render_browser_rows(&self, cx: &Context<'_, Self>) -> Vec<AnyElement> {
        let prefs = BrowserPreferences::get(cx);
        let mut rows = self.tool_access_rows(bruhs_models::local_tools::ToolScope::Browser, cx);
        rows.push(self.settings_row(
            "desktop-settings-row-browser-engine",
            "Search engine",
            "Used for searches in the address bar.",
            settings_ui::control_column(self.choice_control(
                "settings-browser-search-engine",
                "Search engine",
                &prefs.engine,
                BrowserPreferences::ENGINES,
                cx,
            )),
        ));
        if prefs.engine == "custom" {
            rows.push(
                self.settings_row(
                    "desktop-settings-row-browser-search-url",
                    "Custom search URL",
                    if prefs.custom_url_valid() {
                        "Use %s where the search terms go."
                    } else {
                        "Enter an HTTP or HTTPS URL containing %s."
                    },
                    settings_ui::control_column(
                        Input::new(&self.browser_search_url_input)
                            .aria_label("Custom search URL")
                            .into_any_element(),
                    ),
                ),
            );
        }
        if crate::views::preview_panel::WebViewHost::available() {
            #[cfg(target_os = "macos")]
            rows.extend(self.browser_profile_rows(&prefs, cx));
            rows.extend(self.viewport_rows(&prefs, cx));
            rows.push(self.settings_row(
                "desktop-settings-row-browser-link-target",
                "Open links in",
                "Where links in chat and documents open.",
                settings_ui::control_column(self.choice_control(
                    "settings-browser-link-target",
                    "Open links in",
                    if prefs.open_links_in_app {
                        "app"
                    } else {
                        "system"
                    },
                    &[("system", "Your default browser"), ("app", "bruhs")],
                    cx,
                )),
            ));
            rows.push(self.settings_row(
                "desktop-settings-row-browser-appearance",
                "Default browser appearance",
                "Color scheme for embedded browser pages.",
                settings_ui::control_column(self.choice_control(
                    "settings-browser-appearance",
                    "Default browser appearance",
                    prefs.appearance.key(),
                    &[("system", "System"), ("light", "Light"), ("dark", "Dark")],
                    cx,
                )),
            ));
            rows.push(self.settings_row(
                "desktop-settings-row-browser-zoom",
                "Default browser zoom",
                "Default page zoom in the preview.",
                settings_ui::control_column(self.choice_control(
                    "settings-browser-zoom",
                    "Default browser zoom",
                    &prefs.zoom_percent.to_string(),
                    BrowserPreferences::ZOOM,
                    cx,
                )),
            ));
        }
        let recording = prefs.recording_preferences();
        rows.push(self.settings_row(
            "desktop-settings-row-browser-recording-frame-rate",
            "Browser recording frame rate",
            "Maximum recording rate. 30 fps saves CPU and storage; 60 fps is smoother.",
            settings_ui::control_column(self.choice_control(
                "settings-browser-recording-frame-rate",
                "Browser recording frame rate",
                &recording.frame_rate.to_string(),
                BrowserPreferences::FRAME_RATES,
                cx,
            )),
        ));
        rows.push(self.settings_row(
            "desktop-settings-row-browser-recording-key-presses",
            "Show key presses in recordings",
            "Show pressed keys and shortcuts in new recordings. Password fields are excluded.",
            settings_ui::switch(
                "settings-browser-recording-key-presses",
                recording.show_key_presses,
                false,
                "Show key presses in recordings",
                cx,
                |this, _, cx| {
                    let prefs = cx.default_global::<BrowserPreferences>();
                    prefs.recording_show_key_presses = !prefs.recording_show_key_presses;
                    this.persist_and_notify(cx);
                },
            ),
        ));
        rows.push(self.settings_row(
            "desktop-settings-row-browser-recording-mouse-presses",
            "Show mouse presses in recordings",
            "Highlight mouse presses and held buttons in new recordings.",
            settings_ui::switch(
                "settings-browser-recording-mouse-presses",
                recording.show_mouse_presses,
                false,
                "Show mouse presses in recordings",
                cx,
                |this, _, cx| {
                    let prefs = cx.default_global::<BrowserPreferences>();
                    prefs.recording_show_mouse_presses = !prefs.recording_show_mouse_presses;
                    this.persist_and_notify(cx);
                },
            ),
        ));
        rows.push(self.settings_row(
            "desktop-settings-row-browser-auto-show-floating-preview",
            "Auto-show floating preview",
            "Show the floating preview when an agent opens a browser or device unless the agent says otherwise.",
            settings_ui::switch(
                "settings-browser-auto-show-floating-preview",
                recording.auto_show_floating_preview,
                false,
                "Auto-show floating preview",
                cx,
                |this, _, cx| {
                    let prefs = cx.default_global::<BrowserPreferences>();
                    prefs.auto_show_floating_preview = !prefs.auto_show_floating_preview;
                    this.persist_and_notify(cx);
                },
            ),
        ));
        rows
    }
}

#[cfg(target_os = "macos")]
pub(super) struct ProfileEditor {
    name: Entity<InputState>,
    error: Option<String>,
    removing: Option<uuid::Uuid>,
}

#[cfg(target_os = "macos")]
impl ProfileEditor {
    pub(super) fn new(window: &mut Window, cx: &mut App) -> Self {
        Self {
            name: cx.new(|cx| InputState::new(window, cx).placeholder("Profile name")),
            error: None,
            removing: None,
        }
    }
}

#[cfg(target_os = "macos")]
impl Workspace {
    fn add_browser_profile(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let name = self
            .browser_profile_editor
            .name
            .read(cx)
            .value()
            .to_string();
        match cx.default_global::<BrowserPreferences>().add_profile(&name) {
            Ok(_) => {
                self.browser_profile_editor
                    .name
                    .update(cx, |input, cx| input.set_value("", window, cx));
                self.browser_profile_editor.error = None;
                self.persist_and_notify(cx);
            }
            Err(error) => {
                self.browser_profile_editor.error = Some(error.into());
                cx.notify();
            }
        }
    }

    fn remove_browser_profile(
        &mut self,
        id: uuid::Uuid,
        name: &str,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if self.browser_profile_editor.removing.is_some() || !objc2::available!(macos = 14.0) {
            return;
        }
        let answer = window.prompt(
            gpui::PromptLevel::Warning,
            &format!("Delete “{name}”?"),
            Some("Cookies, sign-ins and site storage for this browser profile will be deleted."),
            &[
                gpui::PromptButton::Cancel("Cancel".into()),
                gpui::PromptButton::Ok("Delete profile".into()),
            ],
            cx,
        );
        cx.spawn(async move |this, cx| {
            if answer.await != Ok(1) {
                return;
            }
            let Ok(Some(done)) = this.update(cx, |this, cx| {
                use wry::{WebView, WebViewExtDarwin};
                if this.browser_profile_editor.removing.is_some() {
                    return None;
                }
                this.browser_profile_editor.removing = Some(id);
                this.browser_profile_editor.error = None;
                this.preview_panel
                    .update(cx, |panel, _| panel.release_profile(id));
                let prefs = cx.default_global::<BrowserPreferences>();
                if prefs.default_profile == Some(id) {
                    prefs.default_profile = None;
                }
                this.persist_and_notify(cx);
                let (send, done) = tokio::sync::oneshot::channel();
                // A newly created profile may never have opened a WebKit store.
                let result = WebView::fetch_data_store_identifiers(move |stores| {
                    if stores.contains(id.as_bytes()) {
                        WebView::remove_data_store(id.as_bytes(), move |result| {
                            let _ = send.send(result.map_err(|e| e.to_string()));
                        });
                    } else {
                        let _ = send.send(Ok(()));
                    }
                });
                if let Err(error) = result {
                    this.browser_profile_editor.error = Some(error.to_string());
                    this.browser_profile_editor.removing = None;
                    return None;
                }
                Some(done)
            }) else {
                return;
            };
            let result = done
                .await
                .unwrap_or_else(|_| Err("WebKit stopped before removing the profile.".into()));
            let _ = this.update(cx, |this, cx| {
                this.browser_profile_editor.removing = None;
                match result {
                    Ok(()) => cx
                        .default_global::<BrowserPreferences>()
                        .profiles
                        .retain(|p| p.id != id),
                    Err(error) => {
                        this.browser_profile_editor.error =
                            Some(format!("Could not delete the profile. {error}"))
                    }
                }
                this.persist_and_notify(cx);
            });
        })
        .detach();
    }

    fn browser_profile_rows(
        &self,
        prefs: &BrowserPreferences,
        cx: &Context<'_, Self>,
    ) -> Vec<AnyElement> {
        use bruhs_ui::press::on_press_view;
        if !objc2::available!(macos = 14.0) {
            return Vec::new();
        }
        let mut manager = div().w_full().flex().flex_col().gap_2();
        let mut choices = div().flex().flex_wrap().gap_1();
        let active = prefs.page_settings().profile;
        for (i, (id, name)) in std::iter::once((None, "Default"))
            .chain(prefs.profiles.iter().map(|p| (Some(p.id), p.name.as_str())))
            .enumerate()
        {
            let disabled = id.is_some() && self.browser_profile_editor.removing == id;
            let selected = active == id;
            let choice = web_button(
                ("browser-use-profile", i),
                if selected {
                    WebButtonVariant::Outline
                } else {
                    WebButtonVariant::Ghost
                },
                WebButtonSize::Small,
                ButtonState::disabled_if(disabled),
            )
            .aria_toggled(if selected {
                gpui::Toggled::True
            } else {
                gpui::Toggled::False
            })
            .child(if selected {
                format!("✓ {name}")
            } else {
                name.into()
            });
            choices = choices.child(on_press_view(choice, cx, move |this, _, cx| {
                cx.default_global::<BrowserPreferences>().default_profile = id;
                this.persist_and_notify(cx);
                cx.refresh_windows();
            }));
            if let Some(id) = id {
                let name = name.to_owned();
                let label = name.clone();
                let remove = web_button(
                    ("browser-remove-profile", i),
                    WebButtonVariant::Ghost,
                    WebButtonSize::Small,
                    ButtonState::disabled_if(self.browser_profile_editor.removing.is_some()),
                )
                .aria_label(format!("Delete {name}"))
                .child("Delete");
                manager = manager.child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(div().flex_1().child(label))
                        .child(on_press_view(remove, cx, move |this, window, cx| {
                            this.remove_browser_profile(id, &name, window, cx)
                        })),
                );
            }
        }
        let add = web_button(
            "browser-add-profile",
            WebButtonVariant::Outline,
            WebButtonSize::Small,
            ButtonState::Enabled,
        )
        .child("Add profile");
        manager = manager
            .child(Input::new(&self.browser_profile_editor.name).aria_label("Profile name"))
            .child(on_press_view(add, cx, |this, window, cx| {
                this.add_browser_profile(window, cx)
            }));
        if let Some(error) = &self.browser_profile_editor.error {
            manager = manager.child(
                div()
                    .id("browser-profile-error")
                    .role(Role::Alert)
                    .child(error.clone()),
            );
        }
        vec![
            self.settings_row(
                "settings-browser-profiles",
                "Browser profiles",
                "Separate cookies and site storage per profile.",
                settings_ui::control_column(manager.into_any_element()),
            ),
            self.settings_row(
                "settings-default-browser-profile",
                "Default browser profile",
                "Profile used by the preview browser.",
                settings_ui::control_column(choices.into_any_element()),
            ),
        ]
    }
}

impl Workspace {
    pub(super) fn new_viewport_fields(
        window: &mut Window,
        cx: &mut Context<'_, Self>,
        subscriptions: &mut Vec<Subscription>,
    ) -> [Entity<InputState>; 2] {
        let viewport = BrowserPreferences::get(cx).viewport;
        std::array::from_fn(|i| {
            let value = if i == 0 {
                viewport.width
            } else {
                viewport.height
            };
            let field = cx.new(|cx| InputState::new(window, cx).default_value(value.to_string()));
            subscriptions.push(cx.subscribe_in(
                &field,
                window,
                move |this, field, event: &InputEvent, window, cx| {
                    if !matches!(event, InputEvent::Blur | InputEvent::PressEnter { .. }) {
                        return;
                    }
                    let value = field.read(cx).value().trim().parse::<u16>().ok();
                    let prefs = cx.default_global::<BrowserPreferences>();
                    let old = if i == 0 {
                        prefs.viewport.width
                    } else {
                        prefs.viewport.height
                    };
                    if let Some(value) = value.filter(|value| *value != old) {
                        prefs.viewport.mode = "responsive".into();
                        if i == 0 {
                            prefs.viewport.width = value.clamp(240, 7680);
                        } else {
                            prefs.viewport.height = value.clamp(240, 7680);
                        }
                        this.persist_and_notify(cx);
                        cx.refresh_windows();
                    }
                    // A mismatched value is corrected when focus leaves the field.
                    let viewport = &BrowserPreferences::get(cx).viewport;
                    let valid = if i == 0 {
                        viewport.width
                    } else {
                        viewport.height
                    };
                    if field.read(cx).value().as_ref() != valid.to_string() {
                        field.update(cx, |f, cx| f.set_value(valid.to_string(), window, cx));
                    }
                },
            ));
            field
        })
    }

    pub(super) fn refresh_viewport_fields(&self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let viewport = BrowserPreferences::get(cx).viewport;
        for (field, number) in self
            .viewport_fields
            .iter()
            .zip([viewport.width, viewport.height])
        {
            field.update(cx, |f, cx| f.set_value(number.to_string(), window, cx));
        }
    }

    fn viewport_rows(&self, prefs: &BrowserPreferences, cx: &Context<'_, Self>) -> Vec<AnyElement> {
        use crate::views::browser_preferences::BrowserViewport;
        let id = "settings-browser-viewport";
        let (toggle, select) = self.settings_dropdown_handlers(id, cx);
        let options = vec![
            DropdownGroup {
                label: None,
                mark: None,
                options: [("fill", "Fill panel"), ("responsive", "Responsive")]
                    .into_iter()
                    .map(|(value, label)| DropdownOption {
                        value: value.into(),
                        label: label.into(),
                        description: None,
                    })
                    .collect(),
            },
            DropdownGroup {
                label: Some("Standard".into()),
                mark: None,
                options: BrowserViewport::PRESETS
                    .iter()
                    .map(|&(value, label, w, h)| DropdownOption {
                        value: value.into(),
                        label: label.into(),
                        description: Some(format!("{w} × {h}")),
                    })
                    .collect(),
            },
        ];
        let mut controls = div().flex().flex_col().gap_2().child(dropdown(
            id,
            "Default browser viewport",
            &prefs.viewport.mode,
            options,
            self.settings_listbox(id),
            toggle,
            select,
        ));
        if prefs.viewport.dimensions().is_some() {
            let rotate = web_button(
                "settings-rotate-viewport",
                WebButtonVariant::Outline,
                WebButtonSize::Small,
                ButtonState::Enabled,
            )
            .child("Rotate");
            controls = controls.child(
                div()
                    .flex()
                    .gap_1()
                    .items_center()
                    .child(div().w(px(66.)).child(
                        Input::new(&self.viewport_fields[0]).aria_label("Default viewport width"),
                    ))
                    .child("×")
                    .child(div().w(px(66.)).child(
                        Input::new(&self.viewport_fields[1]).aria_label("Default viewport height"),
                    ))
                    .child(on_press_view(rotate, cx, |this, window, cx| {
                        let viewport = &mut cx.default_global::<BrowserPreferences>().viewport;
                        std::mem::swap(&mut viewport.width, &mut viewport.height);
                        this.refresh_viewport_fields(window, cx);
                        this.persist_and_notify(cx);
                        cx.refresh_windows();
                    })),
            );
        }
        vec![self.settings_row(
            "desktop-settings-row-browser-viewport",
            "Default browser viewport",
            "Page size for the preview browser.",
            settings_ui::control_column(controls.into_any_element()),
        )]
    }
}

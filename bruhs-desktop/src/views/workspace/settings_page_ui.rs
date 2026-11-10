//! The settings dialog shell: section nav, scroll spy, rows, cards, and the General, Keyboard and About sections.

use super::*;
use gpui_component::input::Input;

/// Web `<BrailleBruh size=96.0 />` in Settings → About.
const SETTINGS_ABOUT_BRUH_PX: f32 = 96.0;

fn follow_up_button(
    id: &'static str,
    label: &'static str,
    selected: bool,
    steer: bool,
    cx: &Context<'_, Workspace>,
) -> AnyElement {
    div()
        .id(id)
        .h(px(40.0))
        .px(px(12.0))
        .rounded(px(8.0))
        .flex()
        .items_center()
        .text_size(px(14.0))
        .cursor_pointer()
        .tab_index(0)
        .role(Role::Button)
        .aria_label(label)
        .aria_toggled(if selected {
            Toggled::True
        } else {
            Toggled::False
        })
        .when(selected, |el| {
            el.bg(theme::longan_800()).text_color(theme::rambutan_100())
        })
        .when(!selected, |el| {
            el.text_color(theme::rambutan_400()).hover(|el| {
                el.bg(theme::longan_800().opacity(0.4))
                    .text_color(theme::rambutan_100())
            })
        })
        .focus_visible(|el| el.border_2().border_color(theme::persimmon_400()))
        .on_click(cx.listener(move |this, _ev, _window, cx| {
            cx.stop_propagation();
            this.set_follow_up_steer(steer, cx);
        }))
        .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
            if is_web_button_activation_key(event.keystroke.key.as_str()) {
                window.prevent_default();
                cx.stop_propagation();
                this.set_follow_up_steer(steer, cx);
            }
        }))
        .child(label)
        .into_any_element()
}

impl SettingsSection {
    pub(super) const ALL: [SettingsSection; 11] = [
        Self::General,
        Self::Updates,
        Self::Beta,
        Self::Credentials,
        Self::Calendar,
        Self::Recording,
        Self::Transcription,
        Self::Speakers,
        Self::People,
        Self::Keyboard,
        Self::About,
    ];

    pub(super) fn index(self) -> usize {
        Self::ALL.iter().position(|s| *s == self).unwrap_or(0)
    }

    /// Web section ids (`SECTIONS[i].0`).
    pub(super) fn id(self) -> &'static str {
        match self {
            Self::Credentials => "credentials",
            Self::General => "general",
            Self::Updates => "updates",
            Self::Beta => "beta",
            Self::Calendar => "calendar",
            Self::Recording => "recording",
            Self::Transcription => "transcription",
            Self::Speakers => "speakers",
            Self::People => "people",
            Self::Keyboard => "keyboard",
            Self::About => "about",
        }
    }

    #[cfg(any(test, debug_assertions))]
    pub(super) fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|section| section.id() == id)
    }

    /// Web `SECTIONS` icons: settings · download · activity · calendar · mic · activity ·
    /// users · book-open · keyboard · help-circle.
    pub(super) fn icon_path(self) -> &'static str {
        match self {
            Self::Credentials => "icons/shield.svg",
            Self::General => "icons/settings.svg",
            Self::Updates => assets::DOWNLOAD_ICON_PATH,
            Self::Beta => assets::ACTIVITY_ICON_PATH,
            Self::Calendar => assets::CALENDAR_ICON_PATH,
            Self::Recording => assets::MIC_ICON_PATH,
            Self::Transcription => assets::ACTIVITY_ICON_PATH,
            Self::Speakers => assets::USERS_ICON_PATH,
            Self::People => assets::BOOK_OPEN_ICON_PATH,
            Self::Keyboard => assets::KEYBOARD_ICON_PATH,
            Self::About => assets::HELP_CIRCLE_ICON_PATH,
        }
    }

    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Credentials => "Credentials",
            Self::General => "General",
            Self::Updates => "Updates",
            Self::Beta => "Beta",
            Self::Calendar => "Calendar",
            Self::Recording => "Recording",
            Self::Transcription => "Transcription",
            Self::Speakers => "Speakers",
            Self::People => "People",
            Self::Keyboard => "Keybindings",
            Self::About => "About",
        }
    }

    /// Web `SettingsSection` descriptions under each card title.
    pub(super) fn description(self) -> &'static str {
        match self {
            Self::Credentials => "Saved credentials stay in your OS keychain. Agents still need approval for every use.",
            Self::General => "Defaults for new turns.",
            Self::Updates => "Provider CLIs and this build of bruhs.",
            Self::Beta => "Sidebar sections that are still settling in.",
            Self::Calendar => "Google Calendar sync for the calendar view and meeting auto-record.",
            Self::Recording => "When bruhs starts recording meetings on its own.",
            Self::Transcription => "Provider and model for the final transcript, plus an on-device live preview.",
            Self::Speakers => "Telling voices apart and remembering them across meetings.",
            Self::People => "What meeting summaries record about each person.",
            Self::Keyboard => "Each shortcut is a key, a command id, and an optional when clause. A new default is added when its command is not already listed.",
            Self::About => "",
        }
    }

    pub(super) fn selector(self) -> &'static str {
        match self {
            Self::Credentials => "desktop-settings-nav-credentials",
            Self::General => "desktop-settings-nav-general",
            Self::Updates => "desktop-settings-nav-updates",
            Self::Beta => "desktop-settings-nav-beta",
            Self::Calendar => "desktop-settings-nav-calendar",
            Self::Recording => "desktop-settings-nav-recording",
            Self::Transcription => "desktop-settings-nav-transcription",
            Self::Speakers => "desktop-settings-nav-speakers",
            Self::People => "desktop-settings-nav-people",
            Self::Keyboard => "desktop-settings-nav-keyboard",
            Self::About => "desktop-settings-nav-about",
        }
    }
}

/// Web `settings_page::active_section`: the last section whose top is above
/// the fold line, else the first.
pub(super) fn active_settings_section(tops: &[f32], fold_px: f32) -> Option<usize> {
    tops.iter()
        .rposition(|top| *top <= fold_px)
        .or((!tops.is_empty()).then_some(0))
}

/// Web settings nav column.
pub(super) const SETTINGS_NAV_WIDTH_PX: f32 = 208.0;
pub(super) const SETTINGS_ROW_CONTROL_WIDTH_PX: f32 = 240.0;
pub(super) const SETTINGS_ROW_TITLE_TEXT_SIZE_PX: f32 = 14.0;
pub(super) const SETTINGS_MODEL_DROPDOWN_ID: &str = "desktop-model";
pub(super) const SETTINGS_ACCESS_MODE_DROPDOWN_ID: &str = "desktop-access-mode";
pub(super) const SETTINGS_EDITOR_DROPDOWN_ID: &str = "desktop-editor";
pub(super) const SETTINGS_COMPACT_IDLE_DROPDOWN_ID: &str = "desktop-compact-idle";
pub(super) const SETTINGS_THREAD_ACCESS_DROPDOWN_ID: &str = "desktop-thread-access";
pub(super) const SETTINGS_THREAD_HOPS_DROPDOWN_ID: &str = "desktop-thread-hops";
/// [`bruhs_models::thread_link::HOP_CHOICES`] as dropdown values.
const THREAD_HOP_OPTIONS: [&str; 4] = ["2", "4", "6", "10"];

pub(super) fn settings_cards_width(viewport_width: f32) -> f32 {
    let available = viewport_width - SETTINGS_NAV_WIDTH_PX - 2.0 * SETTINGS_BODY_X_PADDING_PX;
    available.clamp(
        SETTINGS_PAGE_CONTENT_MIN_WIDTH_PX,
        SETTINGS_PAGE_CONTENT_MAX_WIDTH_PX,
    )
}

/// Web settings page: `h-[52px]` breadcrumb header, 44rem card column.
pub(super) const SETTINGS_PAGE_HEADER_HEIGHT_PX: f32 = 52.0;
pub(super) const SETTINGS_PAGE_X_PADDING_PX: f32 = 20.0;
pub(super) const SETTINGS_PAGE_TITLE_TEXT_SIZE_PX: f32 = 14.0;
pub(super) const SETTINGS_PAGE_CRUMB_TEXT_SIZE_PX: f32 = 12.0;
pub(super) const SETTINGS_PAGE_CLOSE_ICON_PX: f32 = 14.0;
pub(super) const SETTINGS_PAGE_CONTENT_MAX_WIDTH_PX: f32 = 704.0;
pub(super) const SETTINGS_PAGE_CONTENT_MIN_WIDTH_PX: f32 = 320.0;
pub(super) const SETTINGS_CODE_FONT_FAMILY: &str = "Monaspace Neon";
pub(super) const SETTINGS_CLOSE_BUTTON_SIZE_PX: f32 = 32.0;
pub(super) const SETTINGS_CONTROL_RADIUS_PX: f32 = 8.0;
/// Web settings body: `px-8 py-6`, sections `gap-8`, `scroll-mt-4`, spy fold 96.
pub(super) const SETTINGS_BODY_X_PADDING_PX: f32 = 32.0;
pub(super) const SETTINGS_BODY_Y_PADDING_PX: f32 = 24.0;
pub(super) const SETTINGS_BODY_GAP_PX: f32 = 32.0;
pub(super) const SETTINGS_SECTION_SCROLL_MARGIN_PX: f32 = 16.0;
pub(super) const SETTINGS_SCROLL_SPY_FOLD_PX: f32 = 96.0;
/// Web `SettingsRow`: `opacity-60` when disabled, text column `max-w-[60%]`.
pub(super) const SETTINGS_ROW_DISABLED_OPACITY: f32 = 0.6;
/// Web rows are `px-4` inside an inset ring; the desktop card ring is a real
/// 1px border, so 15px keeps the same 672px content box (and 60% text cap).
pub(super) const SETTINGS_ROW_X_PADDING_PX: f32 = 15.0;
pub(super) const SETTINGS_ROW_TEXT_MAX_FRACTION: f32 = 0.6;
pub(super) const SETTINGS_SECTION_GAP_PX: f32 = 8.0;
pub(super) const SETTINGS_CLOSE_HOVER_BG_OPACITY: f32 = 0.50;
pub(super) const SETTINGS_BODY_TEXT_SIZE_PX: f32 = 12.0;
pub(super) const SETTINGS_META_TEXT_SIZE_PX: f32 = 12.0;
pub(super) const SETTINGS_CODE_RADIUS_PX: f32 = 4.0;
pub(super) const SETTINGS_MODEL_ROW_MIN_HEIGHT_PX: f32 = 48.0;
pub(super) const SETTINGS_MODEL_ID_GAP_PX: f32 = 2.0;
pub(super) const SETTINGS_MODEL_LABEL_TEXT_SIZE_PX: f32 = 14.0;
pub(super) const SETTINGS_CLOSE_LABEL: &str = "Close settings";
pub(super) const SETTINGS_DESIGN_SYSTEM_URL: &str = "https://design.bruhs.dev";

impl Workspace {
    pub(super) fn open_settings(
        &mut self,
        _: &OpenSettings,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if !self.settings_visible() {
            self.settings_section = SettingsSection::General;
            self.settings_scroll_target = None;
            self.settings_scroll.set_offset(point(px(0.0), px(0.0)));
        }
        self.open_overlay(Overlay::Settings {
            open_dropdown: None,
        });
        self.credential_settings
            .update(cx, |view, cx| view.refresh(cx));
        self.focus_first_modal_tab_stop(window, cx);
        cx.notify();
    }

    /// Web `jump`: highlight the section now and scroll it under the header.
    pub(super) fn set_settings_section(
        &mut self,
        section: SettingsSection,
        cx: &mut Context<'_, Self>,
    ) {
        self.settings_section = section;
        self.settings_scroll_target = Some(section);
        self.set_settings_open_dropdown(None);
        cx.notify();
    }

    /// Top of each section relative to the scroll container, from the last
    /// frame's layout plus the current offset (web `on_scroll` tops).
    pub(super) fn settings_section_tops(&self) -> Option<Vec<f32>> {
        let container_top = self.settings_scroll.bounds().top();
        let offset_y = f32::from(self.settings_scroll.offset().y);
        SettingsSection::ALL
            .iter()
            .enumerate()
            .map(|(ix, _)| {
                let bounds = self.settings_scroll.bounds_for_item(ix)?;
                Some(f32::from(bounds.top() - container_top) + offset_y)
            })
            .collect()
    }

    /// Runs every frame while settings are open: apply a pending jump,
    /// otherwise let the scroll spy pick the section under the fold line.
    pub(super) fn sync_settings_scroll(&mut self, window: &mut Window) {
        let Some(tops) = self.settings_section_tops() else {
            if self.settings_scroll_target.is_some() {
                // No layout yet; try again next frame.
                window.request_animation_frame();
            }
            return;
        };
        if let Some(target) = self.settings_scroll_target.take() {
            let offset_y = f32::from(self.settings_scroll.offset().y);
            let layout_top = tops[target.index()] - offset_y;
            let max_scroll = f32::from(self.settings_scroll.max_offset().y).max(0.0);
            let y = (SETTINGS_SECTION_SCROLL_MARGIN_PX - layout_top).clamp(-max_scroll, 0.0);
            self.settings_scroll.set_offset(point(px(0.0), px(y)));
            // The spy re-reads the post-scroll layout on the next frame.
            window.request_animation_frame();
            return;
        }
        if let Some(ix) = active_settings_section(&tops, SETTINGS_SCROLL_SPY_FOLD_PX) {
            self.settings_section = SettingsSection::ALL[ix];
        }
    }

    pub(super) fn toggle_settings_dropdown(
        &mut self,
        id: &'static str,
        cx: &mut Context<'_, Self>,
    ) {
        let open = if self.settings_open_dropdown() == Some(id) {
            None
        } else {
            Some(id)
        };
        self.set_settings_open_dropdown(open);
        cx.notify();
    }

    pub(super) fn select_settings_option(
        &mut self,
        id: &'static str,
        value: &'static str,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        match id {
            SETTINGS_MODEL_DROPDOWN_ID => {
                if let Some(model) = model_info(value) {
                    self.set_default_model(model.id, cx);
                }
            }
            SETTINGS_ACCESS_MODE_DROPDOWN_ID => {
                if let Ok(mode) = value.parse() {
                    self.set_permission_mode(mode, cx);
                }
            }
            SETTINGS_EDITOR_DROPDOWN_ID
                if bruhs_models::logic::editor::editor_by_id(value).is_some() =>
            {
                self.editor_id = value.to_string();
                self.persist(cx);
            }
            settings_ui::SETTINGS_WHISPER_DROPDOWN_ID => {
                if let Some(model) = settings_ui::whisper_model_for_key(value) {
                    self.update_meeting_prefs(|prefs| prefs.whisper_model = model, cx);
                }
            }
            settings_ui::SETTINGS_TRANSCRIPTION_PROVIDER_DROPDOWN_ID => {
                if let Some(provider) = bruhs_models::TranscriptionProvider::for_key(value) {
                    self.update_meeting_prefs(|prefs| prefs.transcription.provider = provider, cx);
                }
            }
            settings_ui::SETTINGS_LIVE_WHISPER_DROPDOWN_ID => {
                if let Some(model) = settings_ui::whisper_model_for_key(value) {
                    self.update_meeting_prefs(|prefs| prefs.live_whisper_model = model, cx);
                }
            }
            SETTINGS_COMPACT_IDLE_DROPDOWN_ID => {
                self.compact_idle = IdleCompact::parse(value);
                self.stamp_run_prefs_on_threads(cx);
                self.persist_and_notify(cx);
            }
            SETTINGS_THREAD_ACCESS_DROPDOWN_ID => {
                self.thread_access = ThreadAccess::parse(value);
                self.stamp_run_prefs_on_threads(cx);
                self.persist_and_notify(cx);
            }
            SETTINGS_THREAD_HOPS_DROPDOWN_ID => {
                self.thread_hops = bruhs_models::thread_link::parse_hops(value);
                self.persist_and_notify(cx);
            }
            _ => {
                self.select_role_setting(id, value, window, cx);
            }
        }
        self.set_settings_open_dropdown(None);
        // The clicked option unmounts with the list; keep focus in the dialog
        // so Tab stays trapped.
        self.focus_first_modal_tab_stop(window, cx);
        cx.notify();
    }

    /// Toggle/select closures for a settings dropdown, routed back here.
    pub(super) fn settings_dropdown_handlers(
        &self,
        id: &'static str,
        cx: &Context<'_, Self>,
    ) -> (ToggleHandler, SelectHandler) {
        let entity = cx.entity().downgrade();
        let toggle_entity = entity.clone();
        let on_toggle: ToggleHandler = Rc::new(move |_window, cx| {
            let _ = toggle_entity.update(cx, |this, cx| this.toggle_settings_dropdown(id, cx));
        });
        let on_select: SelectHandler = Rc::new(move |value, window, cx| {
            let _ = entity.update(cx, |this, cx| {
                this.select_settings_option(id, value, window, cx)
            });
        });
        (on_toggle, on_select)
    }

    fn render_follow_up_control(&self, cx: &Context<'_, Self>) -> AnyElement {
        div()
            .id("desktop-settings-follow-up")
            .debug_selector(|| "desktop-settings-follow-up".to_string())
            .flex()
            .items_center()
            .gap_2()
            .child(follow_up_button(
                "desktop-settings-follow-up-queue",
                "Queue",
                !self.follow_up_steer,
                false,
                cx,
            ))
            .child(follow_up_button(
                "desktop-settings-follow-up-steer",
                "Steer",
                self.follow_up_steer,
                true,
                cx,
            ))
            .into_any_element()
    }

    fn render_thread_access_control(&self, cx: &Context<'_, Self>) -> AnyElement {
        let groups = vec![DropdownGroup {
            label: None,
            mark: None,
            options: ["message", "read", "off"]
                .into_iter()
                .zip(["Read and message", "Read only", "Off"])
                .map(|(value, label)| DropdownOption {
                    value,
                    label: label.to_string(),
                    description: None,
                })
                .collect(),
        }];
        let (toggle, select) =
            self.settings_dropdown_handlers(SETTINGS_THREAD_ACCESS_DROPDOWN_ID, cx);
        dropdown(
            SETTINGS_THREAD_ACCESS_DROPDOWN_ID,
            "Messages between threads",
            self.thread_access.as_str(),
            groups,
            self.settings_listbox(SETTINGS_THREAD_ACCESS_DROPDOWN_ID),
            toggle,
            select,
        )
    }

    fn render_thread_hops_control(&self, cx: &Context<'_, Self>) -> AnyElement {
        let groups = vec![DropdownGroup {
            label: None,
            mark: None,
            options: THREAD_HOP_OPTIONS
                .into_iter()
                .map(|value| DropdownOption {
                    value,
                    label: format!("{value} messages"),
                    description: None,
                })
                .collect(),
        }];
        let selected = THREAD_HOP_OPTIONS
            .into_iter()
            .find(|value| bruhs_models::thread_link::parse_hops(value) == self.thread_hops)
            .unwrap_or("6");
        let (toggle, select) =
            self.settings_dropdown_handlers(SETTINGS_THREAD_HOPS_DROPDOWN_ID, cx);
        dropdown(
            SETTINGS_THREAD_HOPS_DROPDOWN_ID,
            "Agent message limit",
            selected,
            groups,
            self.settings_listbox(SETTINGS_THREAD_HOPS_DROPDOWN_ID),
            toggle,
            select,
        )
    }

    fn render_compact_idle_control(&self, cx: &Context<'_, Self>) -> AnyElement {
        let groups = vec![DropdownGroup {
            label: None,
            mark: None,
            options: ["return", "before", "never"]
                .into_iter()
                .zip(["When you return", "Before the cache expires", "Never"])
                .map(|(value, label)| DropdownOption {
                    value,
                    label: label.to_string(),
                    description: None,
                })
                .collect(),
        }];
        let (toggle, select) =
            self.settings_dropdown_handlers(SETTINGS_COMPACT_IDLE_DROPDOWN_ID, cx);
        dropdown(
            SETTINGS_COMPACT_IDLE_DROPDOWN_ID,
            "Compact idle threads",
            self.compact_idle.as_str(),
            groups,
            self.settings_listbox(SETTINGS_COMPACT_IDLE_DROPDOWN_ID),
            toggle,
            select,
        )
    }

    fn set_follow_up_steer(&mut self, steer: bool, cx: &mut Context<'_, Self>) {
        if self.follow_up_steer == steer {
            return;
        }
        self.follow_up_steer = steer;
        self.stamp_run_prefs_on_threads(cx);
        self.persist_and_notify(cx);
    }

    pub(super) fn new_autocompact_input(
        saved: String,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
        subscriptions: &mut Vec<Subscription>,
    ) -> Entity<InputState> {
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Claude default")
                .default_value(saved)
        });
        subscriptions.push(cx.subscribe_in(
            &input,
            window,
            |this, input, event: &InputEvent, _window, cx| {
                if !matches!(event, InputEvent::Change) {
                    return;
                }
                let value = input.read(cx).value().to_string();
                let next = if value.trim().is_empty() {
                    None
                } else if let Some(tokens) = autocompact_tokens(&value) {
                    Some(tokens.to_string())
                } else {
                    return;
                };
                if this.claude_autocompact != next {
                    this.claude_autocompact = next;
                    this.stamp_run_prefs_on_threads(cx);
                    this.persist_and_notify(cx);
                }
            },
        ));
        input
    }

    pub(super) fn settings_row(
        &self,
        selector: &'static str,
        title: impl Into<SharedString>,
        description: impl Into<SharedString>,
        control: AnyElement,
    ) -> AnyElement {
        self.settings_row_opts(selector, title, description, None, false, control)
    }

    /// Web `SettingsRow`: title + description (+ a live status line) on the
    /// left, capped at 60% of the row; controls right-aligned. `disabled`
    /// dims the whole row (`opacity-60`).
    pub(super) fn settings_row_opts(
        &self,
        selector: &'static str,
        title: impl Into<SharedString>,
        description: impl Into<SharedString>,
        live_description: Option<SharedString>,
        disabled: bool,
        control: AnyElement,
    ) -> AnyElement {
        let description: SharedString = description.into();
        let meta = |text: SharedString| {
            div()
                .mt(px(SETTINGS_MODEL_ID_GAP_PX))
                .text_size(px(SETTINGS_META_TEXT_SIZE_PX))
                .font_family(theme::UI_SMALL_FONT_FAMILY)
                .font_weight(FontWeight::MEDIUM)
                .line_height(relative(1.625))
                .text_color(theme::rambutan_500())
                .child(text)
        };
        div()
            .debug_selector(move || selector.to_string())
            .min_h(px(SETTINGS_MODEL_ROW_MIN_HEIGHT_PX))
            .px(px(SETTINGS_ROW_X_PADDING_PX))
            .py_3()
            .flex()
            .items_center()
            .justify_between()
            .gap_3()
            .when(disabled, |el| el.opacity(SETTINGS_ROW_DISABLED_OPACITY))
            .child(
                div()
                    .min_w_0()
                    .flex_1()
                    .max_w(relative(SETTINGS_ROW_TEXT_MAX_FRACTION))
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .text_size(px(SETTINGS_ROW_TITLE_TEXT_SIZE_PX))
                            .font_weight(FontWeight::MEDIUM)
                            .line_height(relative(1.5))
                            .text_color(theme::rambutan_100())
                            .child(title.into()),
                    )
                    .when(!description.is_empty(), |el| el.child(meta(description)))
                    .when_some(live_description, |el, live| el.child(meta(live))),
            )
            .child(
                div()
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .justify_end()
                    .gap_2()
                    .child(control),
            )
            .into_any_element()
    }

    pub(super) fn settings_card(
        &self,
        title: &'static str,
        description: &'static str,
        rows: Vec<AnyElement>,
    ) -> AnyElement {
        let mut body = div()
            .rounded(px(14.0))
            .border_1()
            .border_color(gpui::white().opacity(0.05))
            .bg(theme::longan_900())
            .flex()
            .flex_col();
        let count = rows.len();
        for (index, row) in rows.into_iter().enumerate() {
            body = body.child(
                div()
                    .when(index + 1 < count, |el| {
                        el.border_b_1()
                            .border_color(theme::longan_800().opacity(0.7))
                    })
                    .child(row),
            );
        }
        div()
            .flex()
            .flex_col()
            .gap(px(SETTINGS_SECTION_GAP_PX))
            .child(self.settings_heading(title, description))
            .child(body)
            .into_any_element()
    }

    pub(super) fn render_settings_nav(&self, cx: &Context<'_, Self>) -> AnyElement {
        let mut nav = div()
            .id("desktop-settings-nav")
            .w(px(SETTINGS_NAV_WIDTH_PX))
            .flex_shrink_0()
            .p_3()
            .border_r_1()
            .border_color(theme::longan_800().opacity(0.6))
            .flex()
            .flex_col()
            .gap(px(2.0))
            .role(Role::TabList)
            .aria_label("Settings sections");
        for section in SettingsSection::ALL {
            let active = section == self.settings_section;
            let selector = section.selector();
            nav = nav.child(
                div()
                    .id(selector)
                    .debug_selector(move || selector.to_string())
                    .h(px(32.0))
                    .px_2()
                    .gap_2()
                    .rounded(px(8.0))
                    .flex()
                    .items_center()
                    .text_size(px(SETTINGS_ROW_TITLE_TEXT_SIZE_PX))
                    .text_color(if active {
                        theme::rambutan_100()
                    } else {
                        theme::rambutan_400()
                    })
                    .when(active, |el| el.bg(theme::longan_800().opacity(0.6)))
                    .when(!active, |el| {
                        el.hover(|el| {
                            el.bg(theme::longan_800().opacity(0.4))
                                .text_color(theme::rambutan_100())
                        })
                    })
                    .tab_index(0)
                    .role(Role::Tab)
                    .aria_label(section.label())
                    .aria_selected(active)
                    .focus_visible(|el| el.border_2().border_color(theme::persimmon_400()))
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _ev, _window, cx| {
                        this.set_settings_section(section, cx);
                    }))
                    .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
                        if is_web_button_activation_key(event.keystroke.key.as_str()) {
                            window.prevent_default();
                            cx.stop_propagation();
                            this.set_settings_section(section, cx);
                        }
                    }))
                    .child(
                        svg()
                            .path(section.icon_path())
                            .size(px(14.0))
                            .text_color(theme::rambutan_500())
                            .flex_shrink_0(),
                    )
                    .child(section.label()),
            );
        }
        nav.into_any_element()
    }

    pub(super) fn render_settings_general_rows(&self, cx: &Context<'_, Self>) -> Vec<AnyElement> {
        // Model: providers as groups with their logo, like the web dropdown.
        let model_groups: Vec<DropdownGroup> = desktop_model_provider_groups_like_web()
            .into_iter()
            .map(|(provider, models)| DropdownGroup {
                label: Some(provider_label(provider)),
                mark: Some(provider_mark(provider, cx)),
                options: models
                    .into_iter()
                    .map(|(_, model)| DropdownOption {
                        value: model.id,
                        label: model.label.to_string(),
                        description: Some(model.id.to_string()),
                    })
                    .collect(),
            })
            .collect();
        let (model_toggle, model_select) =
            self.settings_dropdown_handlers(SETTINGS_MODEL_DROPDOWN_ID, cx);
        let model_control = dropdown(
            SETTINGS_MODEL_DROPDOWN_ID,
            "Default model",
            &self.current_model,
            model_groups,
            self.settings_listbox(SETTINGS_MODEL_DROPDOWN_ID),
            model_toggle,
            model_select,
        );

        let mode_groups = vec![DropdownGroup {
            label: None,
            mark: None,
            options: PermissionMode::all()
                .iter()
                .map(|mode| DropdownOption {
                    value: mode.as_str(),
                    label: mode.label().to_string(),
                    description: Some(mode.description(None).to_string()),
                })
                .collect(),
        }];
        let (mode_toggle, mode_select) =
            self.settings_dropdown_handlers(SETTINGS_ACCESS_MODE_DROPDOWN_ID, cx);
        let mode_control = dropdown(
            SETTINGS_ACCESS_MODE_DROPDOWN_ID,
            "Access mode",
            self.permission_mode.as_str(),
            mode_groups,
            self.settings_listbox(SETTINGS_ACCESS_MODE_DROPDOWN_ID),
            mode_toggle,
            mode_select,
        );

        let editor_groups = vec![DropdownGroup {
            label: None,
            mark: None,
            options: bruhs_models::logic::editor::EDITORS
                .iter()
                .map(|editor| DropdownOption {
                    value: editor.id,
                    label: editor.label.to_string(),
                    description: None,
                })
                .collect(),
        }];
        let (editor_toggle, editor_select) =
            self.settings_dropdown_handlers(SETTINGS_EDITOR_DROPDOWN_ID, cx);
        let editor_control = dropdown(
            SETTINGS_EDITOR_DROPDOWN_ID,
            "Editor",
            &self.editor_id,
            editor_groups,
            self.settings_listbox(SETTINGS_EDITOR_DROPDOWN_ID),
            editor_toggle,
            editor_select,
        );

        vec![
            self.settings_row(
                "desktop-settings-row-model",
                "Model",
                "Used for new turns. The composer chip changes it per thread.",
                settings_ui::control_column(model_control),
            ),
            self.settings_row(
                "desktop-settings-row-access-mode",
                "Access mode",
                "How much the agent may do without asking. Default asks before shell commands and edits.",
                settings_ui::control_column(mode_control),
            ),
            self.settings_row(
                "desktop-settings-row-editor",
                "Editor",
                "Where ⌘O and the diff panel's Open button send the project and files.",
                settings_ui::control_column(editor_control),
            ),
            self.settings_row(
                "desktop-settings-row-follow-up",
                "Follow-up behavior",
                "Steer sends the next message into a running Claude turn. Codex always queues. The send button names the action. Hold Command or Control and press Enter for the other one.",
                settings_ui::control_column(self.render_follow_up_control(cx)),
            ),
            self.settings_row(
                "desktop-settings-row-autocompact",
                "Auto-compact after",
                "Claude compacts a long thread around this many tokens, from 100000 to 1000000. Empty uses Claude's own default. Codex has no auto-compact.",
                settings_ui::control_column(
                    div()
                        .w(px(240.0))
                        .h(px(40.0))
                        .child(Input::new(&self.autocompact_input))
                        .into_any_element(),
                ),
            ),
            self.settings_row(
                "desktop-settings-row-compact-idle",
                "Compact idle threads",
                "When you return, a Compact chip sits beside send after the prompt cache expires on a Claude thread over 200,000 tokens. Enter summarizes, then sends the draft. Before the cache expires compacts that thread while bruhs is open. Never skips both. Manual /compact still works.",
                settings_ui::control_column(self.render_compact_idle_control(cx)),
            ),
            self.settings_row(
                "desktop-settings-row-thread-access",
                "Messages between threads",
                "Lets a thread's agent list your other threads, read them, and send one a message or start a new thread. Read only keeps listing and reading. Off removes the tools. Plan turns only read.",
                settings_ui::control_column(self.render_thread_access_control(cx)),
            ),
            self.settings_row(
                "desktop-settings-row-thread-hops",
                "Agent message limit",
                "How many messages agents may send each other in a row before the chain stops and hands back to you. A message you type starts the count again.",
                settings_ui::control_column(self.render_thread_hops_control(cx)),
            ),
            self.settings_row(
                "desktop-settings-row-title-instructions",
                "Thread title instructions",
                "Applied to new titles and Regenerate title. A prefix: line is added in front. max N caps the length. This build does not send the other lines to a model.",
                settings_ui::control_column(
                    div()
                        .w(px(288.0))
                        .h(px(96.0))
                        .child(Input::new(&self.title_instructions_input))
                        .into_any_element(),
                ),
            ),
            self.settings_row(
                "desktop-settings-row-project-title",
                "Project title instructions",
                "Optional extra lines for one project. They are applied after the instructions above.",
                settings_ui::control_column(
                    div()
                        .w(px(288.0))
                        .flex()
                        .flex_col()
                        .gap_2()
                        .child(
                            div()
                                .h(px(40.0))
                                .child(Input::new(&self.title_project_input)),
                        )
                        .child(
                            div()
                                .h(px(80.0))
                                .child(Input::new(&self.title_project_instructions_input)),
                        )
                        .into_any_element(),
                ),
            ),
        ]
    }

    pub(super) fn render_settings_beta_rows(&self, cx: &Context<'_, Self>) -> Vec<AnyElement> {
        let toggle = settings_ui::switch(
            "desktop-settings-working-section",
            self.sidebar_working_section,
            false,
            "Working section",
            cx,
            |this, _window, cx| {
                this.sidebar_working_section = !this.sidebar_working_section;
                this.persist_and_notify(cx);
            },
        );
        vec![self.settings_row(
            "desktop-settings-row-working-section",
            "Working section",
            "Threads that are working, or waiting on work they will resume, move into a collapsed section under the active list. A dev server left running does not count. An approval comes back to the active list. This device only.",
            settings_ui::control_column(toggle),
        )]
    }

    pub(super) fn render_settings_about_rows(&self, cx: &Context<'_, Self>) -> Vec<AnyElement> {
        // Web `AboutSettings`: the packaged release version (from the bundle,
        // the one updates compare), else the crate version in a dev build.
        let version = self
            .updates
            .read(cx)
            .report
            .as_ref()
            .and_then(|report| report.installed_release().map(str::to_owned))
            .unwrap_or_else(|| env!("CARGO_PKG_VERSION").to_owned());
        let code = |text: String| {
            div()
                .text_size(px(SETTINGS_BODY_TEXT_SIZE_PX))
                .text_color(theme::rambutan_200())
                .font_family(SETTINGS_CODE_FONT_FAMILY)
                .child(text)
                .into_any_element()
        };
        let muted = |text: &'static str| {
            div()
                .text_size(px(SETTINGS_META_TEXT_SIZE_PX))
                .text_color(theme::rambutan_500())
                .child(text)
                .into_any_element()
        };
        // Web: `inline-flex h-7 items-center gap-1.5 rounded-grape px-2.5
        // text-label-sm text-rambutan-200 inset-ring-white/10 hover:bg-longan-800`.
        let design_link = div()
            .id("desktop-settings-about-design-link")
            .debug_selector(|| "desktop-settings-about-design-link".to_string())
            .role(Role::Link)
            .aria_label("design.bruhs.dev")
            .tab_index(0)
            .focus_visible(|el| el.border_color(theme::persimmon_500()))
            .h(px(28.0))
            .px(px(10.0))
            .rounded(px(8.0))
            .border_1()
            .border_color(gpui::white().opacity(0.10))
            .flex()
            .items_center()
            .gap(px(6.0))
            .text_size(px(SETTINGS_META_TEXT_SIZE_PX))
            .font_family(theme::UI_SMALL_FONT_FAMILY)
            .text_color(theme::rambutan_200())
            .cursor_pointer()
            .hover(|el| el.bg(theme::longan_800()).text_color(theme::rambutan_100()))
            .on_click(cx.listener(|_, _, _, cx| cx.open_url(SETTINGS_DESIGN_SYSTEM_URL)))
            .child("design.bruhs.dev")
            .child(
                svg()
                    .path(assets::TOOL_GLOBE_ICON_PATH)
                    .size(px(12.0))
                    .text_color(theme::rambutan_500()),
            )
            .into_any_element();
        vec![
                // Web `AboutSettings` header: the dithered Bruh beside name and version.
                div()
                    .debug_selector(|| "desktop-settings-about-version".to_string())
                    .px(px(SETTINGS_ROW_X_PADDING_PX))
                    .py_3()
                    .flex()
                    .items_center()
                    .gap_4()
                    .child(crate::views::braille::braille_bruh(SETTINGS_ABOUT_BRUH_PX))
                    .child(
                        div()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(
                                div()
                                    .text_size(px(SETTINGS_ROW_TITLE_TEXT_SIZE_PX))
                                    .font_weight(FontWeight::MEDIUM)
                                    .line_height(relative(1.5))
                                    .text_color(theme::rambutan_100())
                                    .child("bruhs"),
                            )
                            .child(code(version))
                            .child(
                                div()
                                    .text_size(px(SETTINGS_META_TEXT_SIZE_PX))
                                    .font_family(theme::UI_SMALL_FONT_FAMILY)
                                    .font_weight(FontWeight::MEDIUM)
                                    .line_height(relative(1.625))
                                    .text_color(theme::rambutan_500())
                                    .child("bruhs-desktop, built from the workspace crate."),
                            ),
                    )
                    .into_any_element(),
                self.settings_row(
                    "desktop-settings-about-mcp",
                    "MCP servers",
                    "Configured through Claude Code settings; connected servers appear in each session's init event.",
                    muted("Managed by Claude Code"),
                ),
                self.settings_row(
                    "desktop-settings-about-design",
                    "Design system",
                    "Bruhs: fruit-named tokens, all-mono type, no shadows.",
                    design_link,
                ),
        ]
    }

    /// One stacked section: heading + card, sized to the cards column.
    pub(super) fn render_settings_section(
        &self,
        section: SettingsSection,
        width: f32,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        let rows = match section {
            SettingsSection::Credentials => vec![div()
                .child(self.credential_settings.clone())
                .into_any_element()],
            SettingsSection::General => {
                // Web order: the general rows, then roles, then task models.
                let mut rows = self.render_settings_general_rows(cx);
                rows.extend(self.render_role_settings_rows(cx));
                rows.extend(self.render_task_model_rows(cx));
                rows
            }
            SettingsSection::Updates => self.render_settings_updates_rows(cx),
            SettingsSection::Beta => self.render_settings_beta_rows(cx),
            SettingsSection::Calendar => self.render_settings_calendar_rows(cx),
            SettingsSection::Recording => self.render_settings_recording_rows(cx),
            SettingsSection::Transcription => self.render_settings_transcription_rows(cx),
            SettingsSection::Speakers => self.render_settings_speakers_rows(cx),
            SettingsSection::People => self.render_settings_people_rows(cx),
            SettingsSection::Keyboard => self.render_keybinding_rows(cx),
            SettingsSection::About => self.render_settings_about_rows(cx),
        };
        let id = section.id();
        div()
            .debug_selector(move || format!("desktop-settings-section-{id}"))
            .w(px(width))
            .flex_shrink_0()
            .child(self.settings_card(section.label(), section.description(), rows))
            .into_any_element()
    }

    /// Settings as a page (web `settings_page.rs`): a `Settings / Section`
    /// breadcrumb with ×, the section nav on the left and the stacked cards
    /// on the right. It covers the whole window, so the header also carries
    /// the traffic-light inset and the drag region.
    pub(super) fn render_settings(&self, window: &Window, cx: &Context<'_, Self>) -> AnyElement {
        let section_label = self.settings_section.label();
        // A percentage width inside the vertical scroll container measures as
        // indefinite and collapses the row labels, so size the column here.
        let cards_width = settings_cards_width(f32::from(window.viewport_size().width));
        div()
            .absolute()
            .top_0()
            .right_0()
            .bottom_0()
            .left_0()
            .occlude()
            .id("desktop-settings-overlay")
            .debug_selector(|| "desktop-settings-overlay".to_string())
            .bg(theme::longan_950())
            .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, _window, cx| {
                this.continue_settings_slider_drag(event, cx);
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _event, _window, _cx| this.end_settings_slider_drag()),
            )
            .child(
                div()
                    .id("desktop-settings-dialog")
                    .role(Role::Dialog)
                    .aria_label("Settings")
                    .debug_selector(|| "desktop-settings-dialog".to_string())
                    .size_full()
                    .text_color(cx.theme().popover_foreground)
                    .font_family(theme::UI_FONT_FAMILY)
                    .flex()
                    .flex_col()
                    .overflow_hidden()
                    .track_focus(&self.modal_start_focus_handle)
                    .tab_group()
                    .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                        this.trap_modal_tab_focus(event, window, cx);
                    }))
                    .child(
                        div()
                            .id("desktop-settings-header")
                            .debug_selector(|| "desktop-settings-header".to_string())
                            .h(px(SETTINGS_PAGE_HEADER_HEIGHT_PX))
                            .pl(px(SIDEBAR_HEADER_LEFT_INSET_PX))
                            .pr(px(SETTINGS_PAGE_X_PADDING_PX))
                            .flex_shrink_0()
                            .border_b_1()
                            .border_color(
                                theme::longan_800().opacity(WORKSPACE_SIDEBAR_BORDER_OPACITY),
                            )
                            .flex()
                            .items_center()
                            .gap_2()
                            .window_control_area(WindowControlArea::Drag)
                            .child(
                                div()
                                    .text_size(px(SETTINGS_PAGE_TITLE_TEXT_SIZE_PX))
                                    .text_color(theme::rambutan_100())
                                    .child("Settings"),
                            )
                            .child(
                                div()
                                    .text_size(px(SETTINGS_PAGE_CRUMB_TEXT_SIZE_PX))
                                    .font_family(theme::UI_SMALL_FONT_FAMILY)
                                    .text_color(theme::rambutan_600())
                                    .child("/"),
                            )
                            .child(
                                div()
                                    .debug_selector(|| "desktop-settings-crumb".to_string())
                                    .text_size(px(SETTINGS_PAGE_CRUMB_TEXT_SIZE_PX))
                                    .font_family(theme::UI_SMALL_FONT_FAMILY)
                                    .text_color(theme::rambutan_400())
                                    .child(section_label),
                            )
                            .child(div().flex_1())
                            .child(
                                div()
                                    .id("desktop-settings-close")
                                    .debug_selector(|| "desktop-settings-close".to_string())
                                    .size(px(SETTINGS_CLOSE_BUTTON_SIZE_PX))
                                    .rounded(px(SETTINGS_CONTROL_RADIUS_PX))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .text_color(theme::rambutan_500())
                                    .tab_index(0)
                                    .role(Role::Button)
                                    .aria_label(SETTINGS_CLOSE_LABEL)
                                    .focus_visible(|el| {
                                        el.border_2().border_color(theme::persimmon_400())
                                    })
                                    .cursor_pointer()
                                    .hover(|el| {
                                        el.bg(theme::longan_800()
                                            .opacity(SETTINGS_CLOSE_HOVER_BG_OPACITY))
                                            .text_color(theme::rambutan_100())
                                    })
                                    .tooltip(|window, cx| {
                                        Tooltip::new(SETTINGS_CLOSE_LABEL).build(window, cx)
                                    })
                                    .on_click(cx.listener(|this, _ev, _window, cx| {
                                        this.close_transient_surfaces();
                                        cx.notify();
                                    }))
                                    .on_key_down(cx.listener(
                                        |this, event: &KeyDownEvent, window, cx| {
                                            if is_web_button_activation_key(
                                                event.keystroke.key.as_str(),
                                            ) {
                                                window.prevent_default();
                                                cx.stop_propagation();
                                                this.close_transient_surfaces();
                                                cx.notify();
                                            }
                                        },
                                    ))
                                    .child(
                                        svg()
                                            .path(assets::CLOSE_ICON_PATH)
                                            .size(px(SETTINGS_PAGE_CLOSE_ICON_PX))
                                            .text_color(theme::rambutan_500()),
                                    ),
                            ),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_h_0()
                            .flex()
                            .child(self.render_settings_nav(cx))
                            .child(
                                div()
                                    .id("desktop-settings-content")
                                    .debug_selector(|| "desktop-settings-content".to_string())
                                    .relative()
                                    .flex_1()
                                    .min_w_0()
                                    .min_h_0()
                                    .child(
                                        div()
                                            .id("desktop-settings-scroll")
                                            .size_full()
                                            .overflow_y_scroll()
                                            .track_scroll(&self.settings_scroll)
                                            .px(px(SETTINGS_BODY_X_PADDING_PX))
                                            .py(px(SETTINGS_BODY_Y_PADDING_PX))
                                            .flex()
                                            .flex_col()
                                            .items_center()
                                            .gap(px(SETTINGS_BODY_GAP_PX))
                                            .children(SettingsSection::ALL.iter().map(|section| {
                                                self.render_settings_section(
                                                    *section,
                                                    cards_width,
                                                    cx,
                                                )
                                            })),
                                    )
                                    .child(Scrollbar::vertical(&self.settings_scroll)),
                            ),
                    )
                    .child(self.render_modal_focus_sentinel()),
            )
            .into_any_element()
    }

    pub(super) fn settings_heading(&self, title: &'static str, description: &'static str) -> Div {
        div()
            .px_1()
            .flex()
            .flex_col()
            .child(
                div()
                    .text_size(px(SETTINGS_ROW_TITLE_TEXT_SIZE_PX))
                    .text_color(theme::rambutan_100())
                    .child(title),
            )
            .when(!description.is_empty(), |el| {
                el.child(
                    div()
                        .mt(px(2.0))
                        .text_size(px(SETTINGS_META_TEXT_SIZE_PX))
                        .font_family(theme::UI_SMALL_FONT_FAMILY)
                        .text_color(theme::rambutan_500())
                        .child(description),
                )
            })
    }
}

//! The settings dialog shell: section nav, scroll spy, rows, cards, and the General, Keyboard and About sections.

use crate::views::theme_regions::ThemeRegion as _;
use super::*;
use bruhs_ui::input::{Input, Textarea};
use bruhs_ui::press::on_press_view;
use bruhs_ui::zoom::px;

fn follow_up_button(
    id: &'static str,
    label: &'static str,
    selected: bool,
    steer: bool,
    cx: &Context<'_, Workspace>,
) -> AnyElement {
    div()
        .id(id)
        .h(px(theme::CONTROL_SM_PX))
        .px(px(12.0))
        .rounded(px(6.0))
        .flex()
        .items_center()
        .text_size(px(theme::APP_UI_TEXT_SIZE_PX))
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
        .border_1()
        .border_color(gpui::transparent_black())
        .focus_visible(|el| el.border_color(theme::persimmon_400()))
        .map(|el| {
            on_press_view(el, cx, move |this, _window, cx| {
                this.set_follow_up_steer(steer, cx);
            })
        })
        .child(label)
        .into_any_element()
}

impl SettingsSection {
    pub(super) const ALL: [SettingsSection; 27] = [
        Self::General,
        Self::Appearance,
        Self::Chat,
        Self::Diffs,
        Self::Keyboard,
        Self::Voice,
        Self::SnapShots,
        Self::Updates,
        Self::Threads,
        Self::Browser,
        Self::Devices,
        Self::ScheduledTasks,
        Self::Archived,
        Self::Projects,
        Self::SourceControl,
        Self::Worktrees,
        Self::Environments,
        Self::Access,
        Self::Vault,
        Self::Apps,
        Self::Calendar,
        Self::Meetings,
        Self::Maintenance,
        Self::Backup,
        Self::Diagnostics,
        Self::Labs,
        Self::About,
    ];

    fn group_heading(self) -> Option<&'static str> {
        match self {
                SettingsSection::Updates => Some("Agents"),
                SettingsSection::Projects => Some("Code"),
                SettingsSection::Environments => Some("Connections"),
                SettingsSection::Maintenance => Some("Application"),
                _ => None,
            }
    }

    fn nav_index(self) -> usize {
        let mut row = 0;
        for section in Self::ALL {
            row += usize::from(section.group_heading().is_some());
            if section == self { break; }
            row += 1;
        }
        row
    }

    /// Web section ids (`SECTIONS[i].0`).
    pub(super) fn id(self) -> &'static str {
        match self {
            Self::Vault => "vault",
            Self::Apps => "apps",
            Self::Backup => "backup",
            Self::General => "general",
            Self::Updates => "updates",
            Self::Appearance => "appearance",
            Self::Chat => "chat",
            Self::Diffs => "diffs",
            Self::Projects => "projects",
            Self::SourceControl => "source-control",
            Self::Diagnostics => "diagnostics",
            Self::Maintenance => "maintenance",
            Self::Worktrees => "worktrees",
            Self::Voice => "voice",
            Self::Threads => "threads",
            Self::Browser => "browser",
            Self::SnapShots => "snap-shot",
            Self::ScheduledTasks => "scheduled-tasks",
            Self::Environments => "connections",
            Self::Devices => "devices",
            Self::Labs => "labs",
            Self::Access => "access",
            Self::Archived => "archived",
            Self::Calendar => "calendar",
            Self::Meetings => "meetings",
            Self::Keyboard => "keyboard",
            Self::About => "about",
        }
    }

    #[cfg(any(test, debug_assertions))]
    pub(super) fn from_id(id: &str) -> Option<Self> {
        match id {
            "beta" => return Some(Self::General),
            "recording" | "transcription" | "speakers" | "people" => return Some(Self::Meetings),
            _ => {}
        }
        Self::ALL.into_iter().find(|section| section.id() == id)
    }

    /// Web `SECTIONS` icons: settings · download · activity · calendar · mic · activity ·
    /// users · book-open · keyboard · help-circle.
    pub(super) fn icon_path(self) -> &'static str {
        match self {
            Self::Vault => "icons/shield.svg",
            Self::Apps => assets::TOOL_GLOBE_ICON_PATH,
            Self::Backup => assets::ARCHIVE_ICON_PATH,
            Self::General => "icons/settings.svg",
            Self::Updates => assets::DOWNLOAD_ICON_PATH,
            Self::Appearance => assets::SUN_ICON_PATH,
            Self::Chat => "icons/message-square.svg",
            Self::Diffs => assets::FILE_DIFF_ICON_PATH,
            Self::Projects => assets::FOLDER_ICON_PATH,
            Self::SourceControl => assets::GIT_BRANCH_ICON_PATH,
            Self::Diagnostics => assets::ACTIVITY_ICON_PATH,
            Self::Maintenance => assets::WRENCH_ICON_PATH,
            Self::Worktrees => assets::GIT_BRANCH_ICON_PATH,
            Self::Voice => assets::MIC_ICON_PATH,
            Self::Threads => assets::BOT_ICON_PATH,
            Self::Browser => assets::TOOL_GLOBE_ICON_PATH,
            Self::SnapShots => assets::CAMERA_ICON_PATH,
            Self::ScheduledTasks => assets::CLOCK_ICON_PATH,
            Self::Environments => assets::TOOL_TERMINAL_ICON_PATH,
            Self::Devices => assets::SMARTPHONE_ICON_PATH,
            Self::Labs => assets::BRAIN_ICON_PATH,
            Self::Access => "icons/shield.svg",
            Self::Archived => assets::INBOX_ICON_PATH,
            Self::Calendar => assets::CALENDAR_ICON_PATH,
            Self::Meetings => assets::MIC_ICON_PATH,
            Self::Keyboard => assets::KEYBOARD_ICON_PATH,
            Self::About => assets::HELP_CIRCLE_ICON_PATH,
        }
    }

    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Vault => bruhs_models::vault::VAULT_TITLE,
            Self::Apps => bruhs_models::apps::APPS_TITLE,
            Self::Backup => bruhs_models::backup::BACKUP_TITLE,
            Self::General => "General",
            Self::Updates => "Providers & updates",
            Self::Appearance => "Appearance",
            Self::Chat => "Chat",
            Self::Diffs => "Diffs",
            Self::Projects => "Projects",
            Self::SourceControl => "Source control",
            Self::Diagnostics => "Diagnostics",
            Self::Maintenance => "Maintenance",
            Self::Worktrees => "Worktrees",
            Self::Voice => "Voice",
            Self::Threads => "Threads",
            Self::Browser => "Browser",
            Self::SnapShots => "SnapShots",
            Self::ScheduledTasks => "Scheduled tasks",
            Self::Environments => "Environments",
            Self::Devices => "Devices",
            Self::Labs => "Labs",
            Self::Access => "Sharing & access",
            Self::Archived => "Archived threads",
            Self::Calendar => "Calendar",
            Self::Meetings => "Meetings",
            Self::Keyboard => "Keybindings",
            Self::About => "About",
        }
    }

    /// Optional subtitle shown under standalone section cards.
    pub(super) fn description(self) -> &'static str {
        match self {
            Self::Vault => bruhs_models::vault::VAULT_DESCRIPTION,
            Self::Apps => bruhs_models::apps::APPS_DESCRIPTION,
            Self::Backup => bruhs_models::backup::BACKUP_DESCRIPTION,
            _ => "",
        }
    }

    pub(super) fn selector(self) -> &'static str {
        match self {
            Self::Vault => "desktop-settings-nav-vault",
            Self::Apps => "desktop-settings-nav-apps",
            Self::Backup => "desktop-settings-nav-backup",
            Self::General => "desktop-settings-nav-general",
            Self::Updates => "desktop-settings-nav-updates",
            Self::Appearance => "desktop-settings-nav-appearance",
            Self::Chat => "desktop-settings-nav-chat",
            Self::Diffs => "desktop-settings-nav-diffs",
            Self::Projects => "desktop-settings-nav-projects",
            Self::SourceControl => "desktop-settings-nav-source-control",
            Self::Diagnostics => "desktop-settings-nav-diagnostics",
            Self::Maintenance => "desktop-settings-nav-maintenance",
            Self::Worktrees => "desktop-settings-nav-worktrees",
            Self::Voice => "desktop-settings-nav-voice",
            Self::Threads => "desktop-settings-nav-threads",
            Self::Browser => "desktop-settings-nav-browser",
            Self::SnapShots => "desktop-settings-nav-snap-shot",
            Self::ScheduledTasks => "desktop-settings-nav-scheduled-tasks",
            Self::Environments => "desktop-settings-nav-environments",
            Self::Devices => "desktop-settings-nav-devices",
            Self::Labs => "desktop-settings-nav-labs",
            Self::Access => "desktop-settings-nav-access",
            Self::Archived => "desktop-settings-nav-archived",
            Self::Calendar => "desktop-settings-nav-calendar",
            Self::Meetings => "desktop-settings-nav-meetings",
            Self::Keyboard => "desktop-settings-nav-keyboard",
            Self::About => "desktop-settings-nav-about",
        }
    }
}

fn settings_inset(width: f32) -> f32 {
    if width < 768.0 {
        24.0
    } else {
        40.0
    }
}

fn settings_nav_width(width: f32) -> f32 {
    if width < 768.0 {
        56.0
    } else {
        240.0
    }
}

pub(super) const SETTINGS_ROW_CONTROL_WIDTH_PX: f32 = 240.0;
pub(super) const SETTINGS_ROW_TITLE_TEXT_SIZE_PX: f32 = 14.0;
pub(super) const SETTINGS_MODEL_DROPDOWN_ID: &str = "desktop-model";
pub(super) const SETTINGS_ACCESS_MODE_DROPDOWN_ID: &str = "desktop-access-mode";
pub(super) const SETTINGS_EDITOR_DROPDOWN_ID: &str = "desktop-editor";
pub(super) const SETTINGS_COMPACT_IDLE_DROPDOWN_ID: &str = "desktop-compact-idle";
pub(super) const SETTINGS_THREAD_ACCESS_DROPDOWN_ID: &str = "desktop-thread-access";
const SETTINGS_SEND_SHORTCUT: &str = "desktop-send-shortcut";
pub(super) const SETTINGS_THREAD_HOPS_DROPDOWN_ID: &str = "desktop-thread-hops";
pub(super) const SETTINGS_TITLE_PROJECT_DROPDOWN_ID: &str = "desktop-title-project";
/// [`bruhs_models::thread_link::HOP_CHOICES`] as dropdown values.
const THREAD_HOP_OPTIONS: [&str; 4] = ["2", "4", "6", "10"];

pub(super) fn settings_cards_width(viewport_width: f32) -> f32 {
    let available = (viewport_width - 2.0 * settings_inset(viewport_width)).min(1280.0)
        - settings_nav_width(viewport_width)
        - 2.0 * SETTINGS_BODY_X_PADDING_PX
        - 2.0;
    available.clamp(1.0, SETTINGS_PAGE_CONTENT_MAX_WIDTH_PX)
}

/// App breadcrumb header and 44rem card column.
pub(super) const SETTINGS_PAGE_HEADER_HEIGHT_PX: f32 = theme::APP_HEADER_HEIGHT_PX;
pub(super) const SETTINGS_PAGE_X_PADDING_PX: f32 = 20.0;
pub(super) const SETTINGS_PAGE_TITLE_TEXT_SIZE_PX: f32 = theme::APP_UI_TEXT_SIZE_PX;
pub(super) const SETTINGS_PAGE_CRUMB_TEXT_SIZE_PX: f32 = 12.0;
pub(super) const SETTINGS_PAGE_CLOSE_ICON_PX: f32 = theme::ICON_PX;
pub(super) const SETTINGS_PAGE_CONTENT_MAX_WIDTH_PX: f32 = 704.0;
pub(super) const SETTINGS_CODE_FONT_FAMILY: &str = "Monaspace Neon";
pub(super) const SETTINGS_CLOSE_BUTTON_SIZE_PX: f32 = theme::CONTROL_SM_PX;
pub(super) const SETTINGS_CONTROL_RADIUS_PX: f32 = 8.0;
/// Settings body spacing and scroll-spy fold.
pub(super) const SETTINGS_BODY_X_PADDING_PX: f32 = 24.0;
pub(super) const SETTINGS_BODY_Y_PADDING_PX: f32 = 20.0;
pub(super) const SETTINGS_BODY_GAP_PX: f32 = 24.0;
/// Disabled settings rows use the same opacity as the web component.
pub(super) const SETTINGS_ROW_DISABLED_OPACITY: f32 = 0.6;
/// Web rows are `px-4` inside an inset ring; the desktop card ring is a real
/// 1px border, so 15px keeps the same 672px content box.
pub(super) const SETTINGS_ROW_X_PADDING_PX: f32 = 15.0;
pub(super) const SETTINGS_SECTION_GAP_PX: f32 = 8.0;
pub(super) const SETTINGS_CLOSE_HOVER_BG_OPACITY: f32 = 0.50;
pub(super) const SETTINGS_BODY_TEXT_SIZE_PX: f32 = 12.0;
pub(super) const SETTINGS_META_TEXT_SIZE_PX: f32 = 12.0;
pub(super) const SETTINGS_CODE_RADIUS_PX: f32 = 4.0;
pub(super) const SETTINGS_MODEL_ROW_MIN_HEIGHT_PX: f32 = 44.0;
pub(super) const SETTINGS_ROW_TEXT_GAP_PX: f32 = 4.0;
pub(super) const SETTINGS_MODEL_LABEL_TEXT_SIZE_PX: f32 = theme::APP_UI_TEXT_SIZE_PX;
pub(super) const SETTINGS_CLOSE_LABEL: &str = "Close settings";
pub(super) const SETTINGS_DESIGN_SYSTEM_URL: &str = "https://design.bruhs.dev";

fn settings_row_padding(viewport_width: f32) -> f32 {
    if viewport_width < 640.0 { 11.0 } else { SETTINGS_ROW_X_PADDING_PX }
}

fn settings_rows_stack(viewport_width: f32) -> bool {
    let content_width = settings_cards_width(viewport_width)
        - 2.0 * (settings_row_padding(viewport_width) + 1.0);
    content_width < theme::SETTINGS_INLINE_CONTENT_WIDTH_PX
}

#[derive(Clone)]
pub(super) struct SettingsHit {
    command: Option<String>,
    source_title: Option<SharedString>,
    keywords: SharedString,
    section: SettingsSection,
    title: SharedString,
    selector: Option<SharedString>,
}

fn normalize_search(text: &str) -> String {
    use unicode_normalization::{char::is_combining_mark, UnicodeNormalization};
    text.nfkd()
        .filter(|c| !is_combining_mark(*c))
        .flat_map(char::to_lowercase)
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

impl SettingsHit {
    fn rank(&self, query: &str, section_label: &str) -> Option<u8> {
        let titles: Vec<_> = std::iter::once(self.title.as_ref())
            .chain(self.source_title.as_deref())
            .map(normalize_search)
            .collect();
        if titles.iter().any(|t| t == query) {
            return Some(0);
        }
        if titles.iter().any(|t| t.starts_with(query)) {
            return Some(1);
        }
        if titles.iter().any(|t| t.contains(query)) {
            return Some(2);
        }
        if titles.iter().any(|t| query.split_whitespace().all(|word| t.contains(word))) {
            return Some(3);
        }
        let terms = normalize_search(&format!("{} {} {} {}", titles.join(" "), section_label, self.section.label(), self.keywords));
        if terms.contains(query) {
            Some(4)
        } else {
            query.split_whitespace().all(|word| terms.contains(word)).then_some(5)
        }
    }
}

#[derive(Clone)]
pub(super) struct SettingsReveal {
    selector: SharedString,
    pending: Rc<std::cell::Cell<bool>>,
    scroll: ScrollHandle,
    workspace: gpui::EntityId,
}

#[derive(IntoElement)]
struct SettingsRow {
    force_stacked: bool,
    custom_label: Option<AnyElement>,
    reveal: Option<SettingsReveal>,
    selector: SharedString,
    title: SharedString,
    description: SharedString,
    live_description: Option<SharedString>,
    disabled: bool,
    control: Option<AnyElement>,
    reset: Option<AnyElement>,
}

impl RenderOnce for SettingsRow {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let viewport_width = bruhs_ui::zoom::css_px(window.viewport_size().width);
        let stacked = self.force_stacked || settings_rows_stack(viewport_width);
        let selector = self.selector;
        let label_selector = selector.clone();
        let control_selector = selector.clone();
        let meta = |text: SharedString| {
            div()
                .mt(px(SETTINGS_ROW_TEXT_GAP_PX))
                .text_size(px(SETTINGS_META_TEXT_SIZE_PX))
                .font_family(theme::UI_SMALL_FONT_FAMILY)
                .font_weight(FontWeight::MEDIUM)
                .line_height(relative(1.5))
                .theme_foreground("mutedForeground", theme::rambutan_500(), cx)
                .child(text)
        };
        div()
            .when_some(self.reveal, |row, reveal| {
                row.bg(theme::persimmon_500().opacity(0.09))
                    .on_children_prepainted(move |children, _, cx| {
                        let Some(label) = children.first() else {
                            return;
                        };
                        if reveal.pending.replace(false) {
                            let scroll = reveal.scroll.clone();
                            let id = reveal.workspace;
                            let top = label.top();
                            cx.defer(move |cx| {
                                let y = scroll.offset().y + scroll.bounds().top() - top + px(20.);
                                scroll.set_offset(point(
                                    px(0.),
                                    y.clamp(-scroll.max_offset().y, px(0.)),
                                ));
                                cx.notify(id);
                            });
                        }
                    })
            })
            .group(selector.clone())
            .debug_selector(move || selector.to_string())
            .min_h(px(SETTINGS_MODEL_ROW_MIN_HEIGHT_PX))
            .min_w_0()
            .px(px(settings_row_padding(viewport_width)))
            .py_3()
            .flex()
            .when(stacked, |el| el.flex_col().items_start().gap_3())
            .when(!stacked, |el| el.items_center().justify_between().gap_8())
            .when(self.disabled, |el| {
                el.opacity(SETTINGS_ROW_DISABLED_OPACITY)
            })
            .child(self.custom_label.unwrap_or_else(|| {
                div()
                    .debug_selector(move || format!("{label_selector}-label"))
                    .min_w_0()
                    .when(stacked, |el| el.w_full())
                    .when(!stacked, |el| el.flex_1())
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .text_size(px(SETTINGS_ROW_TITLE_TEXT_SIZE_PX))
                            .font_weight(FontWeight::MEDIUM)
                            .line_height(px(20.0))
                            .theme_foreground("text", theme::rambutan_100(), cx)
                            .when(self.reset.is_some(), |title| title.flex().items_center().gap_1())
                            .child(self.title)
                            .children(self.reset),
                    )
                    .when(!self.description.is_empty(), |el| {
                        el.child(meta(self.description))
                    })
                    .when_some(self.live_description, |el, live| el.child(meta(live)))
                    .into_any_element()
            }))
            .when_some(self.control, |row, control| row.child(
                div()
                    .debug_selector(move || format!("{control_selector}-control"))
                    .min_w_0()
                    .max_w_full()
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .when(stacked, |el| el.w_full().flex_wrap())
                    .when(!stacked, |el| el.justify_end())
                    .gap_2()
                    .child(control),
            ))
    }
}

impl Workspace {
    pub(super) fn open_settings(
        &mut self,
        _: &OpenSettings,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if self.usage_hubs.bounds().is_some() || self.provider_editor.is_some() || self.scheduled_task_dialog_visible(cx) || self.secret_provider_dialog_visible(cx) {
            return;
        }
        if !self.settings_visible() {
            self.linked_projects.leave_project_view();
            self.refresh_project_identities(cx);
            let current_system_language = language::Language::system();
            if self.system_language != current_system_language {
                self.system_language = current_system_language;
                self.refresh_language_ui(window, cx);
            }
            self.settings_search
                .update(cx, |input, cx| input.set_value("", window, cx));
            self.settings_search_open = false;
            self.palette_return_focus = window
                .focused(cx)
                .map(|focus| (self.active_view.clone(), focus));
            self.settings_search_anchor = None;
            self.license_browser = None;
            self.license_forward = false;
            self.settings_section = SettingsSection::General;
            self.settings_nav_reveal.set(true);
            self.settings_scroll.set_offset(point(px(0.0), px(0.0)));
        }
        self.open_overlay(Overlay::Settings {
            open_dropdown: None,
        });
        self.vault_settings
            .update(cx, |view, cx| view.refresh(window, cx));
        self.apps_settings
            .update(cx, |view, cx| view.refresh(window, cx));
        self.backup_settings
            .update(cx, |view, cx| view.refresh(window, cx));
        // A page on the web: nothing inside is focused until the first Tab.
        self.focus_modal_dialog(window, cx);
        cx.notify();
    }

    /// Select one settings pane and start at its first control.
    pub(super) fn set_settings_section(
        &mut self,
        section: SettingsSection,
        cx: &mut Context<'_, Self>,
    ) {
        self.license_browser = None;
        self.license_forward = false;
        self.retention_editor = None;
        self.details.close_menu();
        self.settings_search_open = false;
        self.settings_search_catalog.clear();
        self.settings_search_anchor = None;
        self.provider_availability = self.provider_availability.as_ref()
            .map(|p| p.for_project(self.settings_project_scope.as_ref())).map_err(Clone::clone);
        if self.settings_section == SettingsSection::Voice && section != SettingsSection::Voice {self.microphone_test.clear(); self.voice_models.close(); self.voice_cloud.dismiss();}
        let section_changed = self.settings_section != section;
        if section_changed && (section == SettingsSection::ScheduledTasks || self.settings_section == SettingsSection::ScheduledTasks) {
            self.reactive_automations.update(cx, |panel, _| panel.cancel_schedule_delete());
        }
        if section_changed { self.usage_hubs.close(); self.settings_advanced.clear(); self.theme_navigation=Default::default(); }
        self.settings_section = section;
        if section != SettingsSection::Projects {self.action_editor.open=false;}
        self.settings_nav_reveal.set(true);
        self.diagnostics.poll=None;
        let local_project_settings = section == SettingsSection::Projects && !self.linked_projects.viewing_inventory();
        if local_project_settings || section == SettingsSection::Threads {
            self.refresh_thread_models(cx);
        }
        if section == SettingsSection::Updates {self.refresh_linked_projects(false, cx);self.check_provider_health(cx);self.load_generators(cx);self.load_delivery_preferences(cx);}
        if section == SettingsSection::Maintenance {self.refresh_capture_retention(None,cx);}
        if section == SettingsSection::Worktrees {
            self.read_worktree_location(cx);
        }
        if local_project_settings || matches!(section, SettingsSection::Worktrees | SettingsSection::Threads) {
            self.watch_checkout_defaults(cx);
        } else {
            self.checkout_defaults_watch = None;
            self.checkout_defaults = Default::default();
        }
        #[cfg(target_os = "macos")]
        if section == SettingsSection::Labs {
            self.fast_checkout.refresh();
        }
        if section == SettingsSection::Keyboard {self.refresh_keymap_file(cx);}
        self.watch_linked_shortcuts(section, cx);
        self.watch_linked_projects(section, cx);
        if section != SettingsSection::Environments { self.linked_keybindings.dismiss_form(); }
        if section == SettingsSection::About {self.refresh_launcher(false,cx);}
        if section==SettingsSection::Diagnostics {self.open_diagnostics(cx);}
        self.environments.poll = None;
        if matches!(section, SettingsSection::Devices | SettingsSection::Browser) { self.refresh_tool_access(cx); }
        if section == SettingsSection::Environments { self.open_environments(cx); }
        if section_changed && section == SettingsSection::SourceControl { self.load_delivery_preferences(cx); }
        if section == SettingsSection::Voice {
            self.reload_voice_dictionary(cx);
            self.refresh_voice_runtime(None, cx);
            self.refresh_voice_devices(cx);
        }
        self.keybinding_recording = None;
        self.keybinding_draft = None;
        self.keybinding_when_open = None;
        self.settings_scroll.set_offset(point(px(0.), px(0.)));
        self.set_settings_open_dropdown(None);
        cx.notify();
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
        if open.is_some() && id.starts_with("settings-font-"){self.load_font_names(cx);}
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
        if id == SETTINGS_MODEL_DROPDOWN_ID {
            self.choose_settings_model(value, window, cx);
            return;
        }
        if id == "keybinding-new-command" {
            if let Some(draft) = &mut self.keybinding_draft {
                draft.command = Some(value.to_string());
            }
            self.set_settings_open_dropdown(None);
            cx.notify();
            return;
        }
        if self.select_generator_option(id,value,window,cx) || self.select_worktree_cleanup(id, value, cx) || self.select_model_filter(id,value,cx) || self.select_cleanup(id,value,cx) || self.select_voice_runtime(id,value,cx) || self.select_project_default(id, value, cx) {
            self.set_settings_open_dropdown(None);
            return;
        }
        match id {
            "usage-hub-key-source" => self.select_hub_key_source(value),
            "settings-project-defaults-scope" => {
                if matches!(self.settings_section, SettingsSection::Projects | SettingsSection::ScheduledTasks) && self.select_linked_project_scope(value, cx) {
                    return;
                }
                self.settings_search_catalog.clear();
                self.commit_writer_instructions(window,cx);
                self.commit_branch_fields(window, cx);
                self.retention_editor = None;
                self.settings_project_scope = value.to_owned();
                if matches!(self.settings_section,SettingsSection::Projects | SettingsSection::Threads) {self.refresh_thread_models(cx);}
                self.settings_models_load = None;
                self.settings_model_listings.clear();
                if matches!(self.settings_section, SettingsSection::Worktrees | SettingsSection::Projects | SettingsSection::Threads) { self.watch_checkout_defaults(cx); }
                self.provider_availability = self.provider_availability.as_ref()
                    .map(|p| p.for_project(value.as_ref())).map_err(Clone::clone);
                if self.settings_section == SettingsSection::Updates {
                    self.health_check = None;
                    self.health_result = None;
                    self.provider_change_error = None;
                    self.check_provider_health(cx);
                }
                self.load_branch_fields(window, cx);
                self.worktree_cleanup_status = None;
                self.settings_advanced.clear();
                if self.settings_section==SettingsSection::SourceControl {self.load_delivery_preferences(cx);}
                self.action_editor.open = false;
                self.action_editor.art_open=false; self.action_editor.art_error=None;
                self.snooze_editor.open = false;
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
            thread_notifications::MODE_ID => {
                if let Some((mode, _, _)) = thread_notifications::NotificationMode::OPTIONS
                    .into_iter()
                    .find(|(_, id, _)| *id == value)
                {
                    self.thread_notifications.mode = mode;
                    self.persist(cx);
                }
            }
            "settings-diff-colors" | "settings-diff-layout" | "settings-diff-file-state" => {
                let mut prefs = self.diff_panel.read(cx).prefs;
                match id {
                    "settings-diff-colors" => prefs.blue_orange = value == "blue-orange",
                    "settings-diff-layout" => prefs.split = value == "split",
                    _ => prefs.collapsed = value == "collapsed",
                }
                self.diff_panel
                    .update(cx, |panel, cx| panel.set_preferences(prefs, cx));
                self.persist(cx);
            }
            "settings-response-streaming" => {
                if let Some(mode)=crate::views::chat::response_stream::Mode::parse(value) {self.response_stream=mode;self.stamp_run_prefs_on_threads(cx);self.persist_and_notify(cx);}
            }
            "settings-writing-style" | "settings-writer-options" | "settings-guide-options" | "settings-writer-model" | "settings-guide-model" | "settings-delivery-merge" | "settings-delivery-templates" | "settings-delivery-credits" => self.save_delivery_preference(id,value,cx),
            "settings-update-track" => {
                let track = match value {
                    "stable" => updates_ui::UpdateTrack::Stable,
                    "nightly" => updates_ui::UpdateTrack::Nightly,
                    _ => return,
                };
                self.updates.update(cx, |updates, cx| updates.set_track(track, cx));
                self.persist(cx);
            }
            "settings-text-generation-model" => self.choose_title_model(value,cx),
            "diagnostic-metric" | "diagnostic-range" => self.choose_diagnostic(id,value,cx),
            "settings-effort-display" => {
                self.appearance.effort_label = value == "label";
                self.stamp_run_prefs_on_threads(cx);
                self.persist_and_notify(cx);
            }
            "settings-tool-visibility" => {
                if let Some(mode)=crate::views::chat::tool_visibility::Visibility::parse(value) {
                    self.tool_visibility=mode;self.stamp_run_prefs_on_threads(cx);self.persist_and_notify(cx);
                }
            }
            "background-profile" | "background-active" | "background-idle" => self.set_background_option(id,value,window,cx),
            "settings-quit-shortcut" => self.choose_quit_mode(value,cx),
            "settings-thread-model-options" | "settings-fallback-model-options" => self.set_native_model_option(id,value,cx),
            "settings-kiro-options" | "settings-fallback-kiro-options"=> self.choose_kiro_default(id,value,cx),
            "settings-fallback-model" => self.choose_fallback_model(value,cx),
            "settings-branch-naming" => self.choose_branch_naming(value,cx),
            "settings-settle-merged" | "settings-keep-pull" => self.choose_pull_policy(id,value,cx),
            "settings-project-grouping-override" => self.set_grouping_override(value,cx),
            "settings-project-order" => self.choose_project_order(value,cx),
            "settings-quota-snooze" | "settings-quota-resume" => self.choose_quota_preference(id,value,cx),
            "settings-snooze-day" => self.choose_snooze_day(value,window,cx),
            "settings-capture-sound" => {
                let Some(sound) = snapshots::CaptureSound::from_key(value) else { return };
                self.snapshots.sound = sound;
                self.persist(cx);
            }
            "settings-word-wrap"=>{
                let wrap=match value {"default"=>None,"on"=>Some(true),"off"=>Some(false),_=>return};
                self.fonts.wrap=wrap;
                self.fonts.apply(cx);
                self.diff_panel.update(cx,|panel,cx|panel.set_default_wrap(wrap,cx));
                self.persist_and_notify(cx);
            }
            "settings-log-retention" => {
                let Ok(days) = value.parse::<u16>() else {
                    return;
                };
                self.maintenance.prefs.retention_days = days;
                self.schedule_log_cleanup(cx);
                self.persist_and_notify(cx);
            }
            id if id.starts_with("settings-font-")=>self.select_typeface(id,value,cx),
            "settings-language" => self.set_ui_language(value, window, cx),
            "settings-clock-format" => {
                use crate::clock_format::ClockFormat;
                self.appearance.clock = match value {
                    "12h" => ClockFormat::Twelve,
                    "24h" => ClockFormat::TwentyFour,
                    "system" => ClockFormat::System,
                    _ => return,
                };
                self.appearance.clock.apply();
                self.persist_and_notify(cx);
                cx.refresh_windows();
            }
            "settings-reader-announcements" => {
                use crate::views::chat::screen_reader::Announcements;
                self.screen_reader.announcements = match value {
                    "status" => Announcements::Status,
                    "messages" => Announcements::Messages,
                    "everything" => Announcements::Everything,
                    _ => return,
                };
                self.stamp_run_prefs_on_threads(cx);
                self.persist_and_notify(cx);
            }
            "settings-browser-viewport" => {
                cx.default_global::<crate::views::browser_preferences::BrowserPreferences>()
                    .viewport
                    .select(value);
                self.refresh_viewport_fields(window, cx);
                self.persist_and_notify(cx);
                cx.refresh_windows();
            }
            "settings-browser-search-engine"
            | "settings-browser-zoom"
            | "settings-browser-appearance"
            | "settings-browser-link-target" => {
                self.change_browser_preference(id, value, cx);
            }
            "settings-voice-destination" => {
                use crate::voice_capture::Destination;
                if let Some(provider)=voice_cloud::provider(value) {
                    self.open_voice_cloud(provider,window,cx);
                    self.set_settings_open_dropdown(None);
                    return;
                }
                self.voice_cloud.close(window,cx);
                self.dictation_prefs.destination=match value {"off"=>Destination::Off,"local"=>Destination::Local,"compatible"=>Destination::Compatible,_=>return};
                self.stamp_run_prefs_on_threads(cx);
                self.persist(cx);
            }
            "settings-voice-shortcut-mode" => {
                use crate::voice_capture::ShortcutMode;
                self.dictation_prefs.shortcut_mode = match value {
                    "Auto" => ShortcutMode::Auto, "Hold" => ShortcutMode::Hold,
                    "Toggle" => ShortcutMode::Toggle, _ => return,
                };
                self.stamp_run_prefs_on_threads(cx);
                self.persist(cx);
            }
            "settings-voice-model" => {
                if let Some(model) = settings_ui::whisper_model_for_key(value) {
                    self.dictation_prefs.model = model;
                    self.stamp_run_prefs_on_threads(cx);
                    self.persist(cx);
                }
            }
            "settings-voice-language" => {
                self.dictation_prefs.language = (value != "auto").then(|| value.to_owned());
                self.stamp_run_prefs_on_threads(cx);
                self.persist(cx);
            }
            "settings-worktree-submodules" => {
                use bruhs_models::logic::run_place::WorktreeSubmodules;
                self.worktree_prefs.submodules = if value == "inherit" {WorktreeSubmodules::FromProject}
                    else if let Some(mode) = WorktreeSubmodules::parse_choice(value) { mode } else { return };
                self.stamp_run_prefs_on_threads(cx);
                self.persist(cx);
            }
            "settings-new-thread-placement" => {
                self.choose_default_placement(value,cx);
            }
            "settings-chat-width" => {
                self.chat_width = match value {
                    "wide" => ChatWidth::Wide,
                    "full" => ChatWidth::Full,
                    _ => ChatWidth::Comfortable,
                };
                self.stamp_run_prefs_on_threads(cx);
                self.persist(cx);
            }
            SETTINGS_SEND_SHORTCUT => {
                if let Some((mode, _)) = SendShortcut::OPTIONS
                    .into_iter()
                    .find(|(_, id)| *id == value)
                {
                    self.send_shortcut = mode;
                    self.stamp_run_prefs_on_threads(cx);
                    self.persist(cx);
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
            SETTINGS_TITLE_PROJECT_DROPDOWN_ID => {
                self.title_project = value.to_string();
                let text = self
                    .title_instructions_by_project
                    .get(value)
                    .cloned()
                    .unwrap_or_default();
                self.title_project_instructions_input
                    .update(cx, |field, cx| field.set_value(text, window, cx));
            }
            _ => {
                self.select_role_setting(id, value, window, cx);
            }
        }
        self.set_settings_open_dropdown(None);
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

    pub(super) fn choice_control(
        &self,
        id: &'static str,
        label: &'static str,
        current: &str,
        choices: &[(&'static str, &'static str)],
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        let options = choices
            .iter()
            .map(|&(value, label)| DropdownOption {
                value,
                label: self.ui_language().text(label).into(),
                description: None,
            })
            .collect();
        let (toggle, select) = self.settings_dropdown_handlers(id, cx);
        dropdown(
            id,
            self.ui_language().text(label),
            current,
            vec![DropdownGroup {
                label: None,
                mark: None,
                options,
            }],
            self.settings_listbox(id),
            toggle,
            select,
        )
    }

    fn render_follow_up_control(&self, cx: &Context<'_, Self>) -> AnyElement {
        div()
            .id("desktop-settings-follow-up")
            .debug_selector(|| "desktop-settings-follow-up".to_string())
            .p(px(2.0))
            .rounded(px(8.0))
            .bg(theme::longan_950())
            .border_1()
            .border_color(gpui::white().opacity(0.08))
            .flex()
            .items_center()
            .gap(px(2.0))
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
            |this, input, event: &InputEvent, window, cx| {
                let value = input.read(cx).value().to_string();
                match event {
                    InputEvent::Change => {
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
                    }
                    InputEvent::Blur => {
                        if value.trim().is_empty() || autocompact_tokens(&value).is_some() {
                            return;
                        }
                        let stored = this.claude_autocompact.clone().unwrap_or_default();
                        input.update(cx, |state, cx| state.set_value(stored, window, cx));
                    }
                    InputEvent::Focus | InputEvent::PressEnter { .. } => {}
                }
            },
        ));
        input
    }

    pub(super) fn settings_row(
        &self,
        selector: impl Into<SharedString>,
        title: impl Into<SharedString>,
        description: impl Into<SharedString>,
        control: AnyElement,
    ) -> AnyElement {
        self.settings_row_opts(selector, title, description, None, false, control)
    }

    /// Row labels and controls stack when the settings column cannot fit both.
    pub(super) fn settings_row_opts(
        &self,
        selector: impl Into<SharedString>,
        title: impl Into<SharedString>,
        description: impl Into<SharedString>,
        live_description: Option<SharedString>,
        disabled: bool,
        control: AnyElement,
    ) -> AnyElement {
        self.settings_row_with_reset(selector, title, description, live_description, disabled, Some(control), None)
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "settings row helper accepts optional live description and reset control"
    )]
    pub(super) fn settings_row_with_reset(
        &self,
        selector: impl Into<SharedString>,
        title: impl Into<SharedString>,
        description: impl Into<SharedString>,
        live_description: Option<SharedString>,
        disabled: bool,
        control: Option<AnyElement>,
        reset: Option<AnyElement>,
    ) -> AnyElement {
        let mut row = self.searchable_settings_row(selector, title.into(), description.into());
        row.live_description = live_description;
        row.disabled = disabled;
        row.control = control;
        row.reset = reset;
        row.into_any_element()
    }

    pub(super) fn stacked_settings_row(
        &self,
        selector: &'static str,
        title: &str,
        description: SharedString,
        control: AnyElement,
    ) -> AnyElement {
        let mut row = self.searchable_settings_row(selector, title.to_owned().into(), description);
        row.force_stacked = true;
        row.control = Some(control);
        row.into_any_element()
    }

    pub(super) fn settings_status_row(
        &self,
        selector: &'static str,
        title: &str,
        content: AnyElement,
    ) -> AnyElement {
        let mut row = self.searchable_settings_row(selector, title.to_owned().into(), "".into());
        row.custom_label = Some(content);
        row.into_any_element()
    }

    fn searchable_settings_row(
        &self,
        selector: impl Into<SharedString>,
        title: SharedString,
        description: SharedString,
    ) -> SettingsRow {
        let selector = selector.into();
        // Project names and paths on manual order rows are user data.
        let language = self.ui_language();
        let localize = matches!(
            selector.as_ref(),
            "desktop-settings-row-thread-notifications"
                | "desktop-settings-row-in-app-notifications"
                | "desktop-settings-row-project-grouping"
                | "desktop-settings-row-project-order"
                | "desktop-settings-row-working-section"
                | "desktop-settings-row-confirm-unpin"
                | "desktop-settings-row-confirm-archive"
                | "desktop-settings-row-confirm-delete"
                | "settings-row-quit-shortcut"
                | "desktop-settings-row-language"
                | "desktop-settings-row-clock"
                | "desktop-settings-row-editor"
                | "settings-about-licenses"
        );
        let translated_title = localize.then(|| language.translation(&title)).flatten();
        let translated_description = localize
            .then(|| language.translation(&description))
            .flatten();
        if let Some(rows) = self.settings_search_inventory.borrow_mut().as_mut() {
            let keywords = if selector == "desktop-settings-row-language" {
                format!(
                    "{title} {description} {} {}",
                    translated_description.unwrap_or(""),
                    language::SEARCH_TERMS
                )
                .into()
            } else if translated_title.is_some() || translated_description.is_some() {
                format!(
                    "{title} {description} {}",
                    translated_description.unwrap_or("")
                )
                .into()
            } else {
                description.clone()
            };
            rows.push(SettingsHit {
                command: None,
                source_title: translated_title.map(|_| title.clone()),
                title: translated_title.map_or_else(|| title.clone(), SharedString::from),
                keywords,
                section: self.settings_section,
                selector: Some(selector.clone()),
            });
        }
        let title = translated_title.map(SharedString::from).unwrap_or(title);
        let description = translated_description
            .map(SharedString::from)
            .unwrap_or(description);
        SettingsRow {
            force_stacked: false,
            reveal: self
                .settings_search_anchor
                .as_ref()
                .filter(|reveal| reveal.selector == selector)
                .cloned(),
            selector,
            title,
            description,
            custom_label: None,
            live_description: None,
            disabled: false,
            control: None,
            reset: None,
        }
    }
    pub(super) fn settings_card(
        &self,
        title: &'static str,
        description: &'static str,
        rows: Vec<AnyElement>,
    ) -> AnyElement {
        if title.is_empty() && description.is_empty() {
            return self.settings_card_body(rows).into_any_element();
        }
        self.settings_card_with_heading(self.settings_heading(title, description).into_any_element(), rows)
    }

    fn settings_card_body(&self, rows: Vec<AnyElement>) -> Div {
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
        body
    }

    pub(super) fn settings_card_with_heading(&self, heading: AnyElement, rows: Vec<AnyElement>) -> AnyElement {
        div()
            .flex()
            .flex_col()
            .gap(px(SETTINGS_SECTION_GAP_PX))
            .child(heading)
            .child(self.settings_card_body(rows))
            .into_any_element()
    }

    fn settings_find_available(&self, window: &Window, cx: &App) -> bool {
        self.top_layer.is_none()
            && self.settings_open_dropdown().is_none()
            && self.keybinding_recording.is_none()
            && self.keybinding_when_open.is_none()
            && self.active_modal_focus_bounds(cx).0 == self.modal_start_focus_handle
            && !self.theme_picker_has_focus(window, cx)
            && !self.accent_panel_has_focus(window, cx)
    }

    fn focus_settings_search(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        self.settings_search_open = true;
        let field = self.settings_search.clone();
        cx.defer_in(window, move |_, window, cx| {
            field.update(cx, |input, cx| {
                input.focus(window, cx);
                input.set_selected_range(0..input.value().len(), cx);
            })
        });
        cx.notify();
    }

    pub(super) fn clear_focused_settings_query(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> bool {
        if !self.settings_visible() || !self.settings_search.focus_handle(cx).is_focused(window)
            || self.settings_search.read(cx).value().is_empty()
        {
            return false;
        }
        self.settings_search.update(cx, |input, cx| input.set_value("", window, cx));
        self.refresh_settings_search(cx);
        true
    }

    pub(super) fn refresh_settings_search(&mut self, cx: &mut Context<'_, Self>) {
        let query = normalize_search(&self.settings_search.read(cx).value());
        self.settings_search_index = 0;
        self.settings_nav_scroll.set_offset(point(px(0.), px(0.)));
        self.settings_search_results.clear();
        if query.is_empty() {
            self.settings_nav_reveal.set(true);
            self.settings_search_catalog.clear();
            cx.notify();
            return;
        }
        if self.settings_search_catalog.is_empty() {
            // Snapshot available controls once per search, including conditional rows.
            for section in SettingsSection::ALL {
                self.settings_search_inventory.replace(Some(Vec::new()));
                drop(self.render_settings_section(section, SETTINGS_PAGE_CONTENT_MAX_WIDTH_PX, cx));
                let mut entries = self.settings_search_inventory.take().unwrap_or_default();
                if section == SettingsSection::Chat {
                    if let Some(hit) = entries.iter_mut().find(|hit| {
                        hit.selector.as_deref() == Some("desktop-settings-row-rich-composer")
                    }) {
                        hit.keywords = format!("{} bold italic strikethrough WYSIWYG", hit.keywords).into();
                    }
                }
                if section == SettingsSection::Appearance {
                    for hit in &mut entries {
                        match hit.selector.as_deref() {
                            Some("desktop-settings-row-color-scheme") => {
                                hit.keywords = "system light dark appearance mode".into();
                            }
                            Some("desktop-settings-row-themes") => {
                                let names = theme::ThemeFamily::ALL.map(|family| family.label()).join(" ");
                                hit.keywords = format!("palette light dark {names}").into();
                            }
                            _ => {}
                        }
                    }
                }
                if section == SettingsSection::Updates {
                    if let Some(hit) = entries.iter_mut().find(|hit| hit.selector.as_deref() == Some("settings-row-usage-providers")) {
                        hit.keywords = "CLIProxyAPI CLI Proxy hub quota subscription limits management key usage sources add remove".into();
                    }
                }
                if section == SettingsSection::Maintenance {
                    if let Some(hit) = entries.iter_mut().find(|hit|hit.selector.as_deref() == Some("desktop-settings-row-continue-threads")) {
                        hit.keywords = "resume running active interrupted work restart reboot machine crash desktop update automatically".into();
                    }
                }
                if section == SettingsSection::SourceControl {
                    // T3 keeps both search entries available while guides are off.
                    // Both lead to the switch that reveals their controls.
                    entries.retain(|hit| !matches!(hit.selector.as_deref(), Some("settings-row-guide-model" | "settings-row-guide-instructions")));
                    for (title, keywords) in [
                        ("Review guide model", "pull request PR guided review coding agent maintainer"),
                        ("Review guide instructions", "pull request PR guided review preferences prompt custom"),
                    ] {
                        entries.push(SettingsHit {
                            command: None,
                            source_title: None, title: title.into(), keywords: keywords.into(), section,
                            selector: Some("settings-row-review-guides".into()),
                        });
                    }
                }
                self.settings_search_catalog
                    .extend(entries.into_iter().map(|mut hit| {
                        hit.section = section;
                        hit
                    }));
                self.settings_search_catalog.push(SettingsHit {
                    command: None,
                    section,
                    source_title: Some(section.label().into()),
                    title: self.ui_language().text(section.label()).into(),
                    keywords: match section {
                        SettingsSection::General => {
                            "Notifications alerts quiet hours sidebar external editor".into()
                        }
                        SettingsSection::Diffs => "pull requests changes review".into(),
                        SettingsSection::Meetings => {
                            "recording transcription speakers people whisper diarize".into()
                        }
                        _ => section.description().into(),
                    },
                    selector: None,
                });
            }
        }
        let language = self.ui_language();
        let mut commands = std::collections::BTreeMap::<String, Vec<String>>::new();
        for rule in bruhs_models::logic::keybindings::default_keybindings() {
            commands.entry(rule.command).or_default();
        }
        for rule in self.settings_keybindings() {
            let shortcuts = commands.entry(rule.command.clone()).or_default();
            if !rule.key.is_empty() {
                shortcuts.push(keybindings_ui::shortcut_label(&rule.key));
                shortcuts.push(rule.key.clone());
            }
        }
        let command_hits = commands.into_iter().map(|(command, shortcuts)| SettingsHit {
            title: keybindings_ui::command_label(&command).into(),
            source_title: None,
            keywords: format!("{command} {}", shortcuts.join(" ")).into(),
            section: SettingsSection::Keyboard,
            command: Some(command),
            selector: None,
        });
        let mut matches: Vec<_> = self.settings_search_catalog.iter().cloned().chain(command_hits)
            .filter_map(|hit| hit.rank(&query, language.text(hit.section.label())).map(|rank| (rank, hit)))
            .collect();
        matches.sort_by_key(|(rank, hit)| (hit.command.is_some(), *rank));
        self.settings_search_results = matches.into_iter().map(|(_, hit)| hit).collect();
        cx.notify();
    }

    pub(super) fn expand_settings_row(&mut self, id: &str) {
        match id {
            "settings-row-provider-health-interval" => {
                self.settings_advanced.insert("provider-health-advanced");
            }
            "desktop-settings-row-pr-templates" | "settings-row-writer-model" => {
                self.settings_advanced.insert("source-writer-advanced");
            }
            "desktop-settings-row-worktree-submodules" => {
                self.settings_advanced.insert("worktree-creation-advanced");
            }
            "settings-row-cleanup-settled"
            | "settings-row-cleanup-unchanged"
            | "settings-row-worktree-keep-when" => {
                self.settings_advanced.insert("worktree-cleanup-advanced");
            }
            _ => {}
        }
    }

    pub(super) fn reveal_settings_row(&mut self, selector: SharedString, cx: &Context<'_, Self>) {
        self.settings_search_anchor = Some(SettingsReveal {
            selector,
            pending: Rc::new(std::cell::Cell::new(true)),
            scroll: self.settings_scroll.clone(),
            workspace: cx.entity_id(),
        });
    }

    pub(super) fn open_settings_hit(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if self.settings_search.read(cx).value().trim().is_empty() {
            return;
        }
        let Some(hit) = self.settings_search_results.get(index).cloned() else {
            return;
        };
        self.set_settings_section(hit.section, cx);
        if let Some(command) = hit.command {
            let assigned = self.settings_keybindings().iter().any(|rule| rule.command == command);
            self.keybinding_filter.update(cx, |input, cx| input.set_value(command.clone(), window, cx));
            if !assigned {
                self.open_new_binding_for(Some(command), window, cx);
            }
        }
        if let Some(id) = hit.selector {
            self.expand_settings_row(&id);
            self.reveal_settings_row(id, cx);
        }
        self.settings_search
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.focus_modal_dialog(window, cx);
        cx.notify();
    }

    pub(super) fn render_settings_nav(
        &self,
        width: f32,
        window: &Window,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        let compact = width < 768.0;
        let expanded = compact && self.settings_search_open;
        let rail = compact && !expanded;
        let has_query = !self.settings_search.read(cx).value().trim().is_empty();
        let search_focused = self.settings_search.focus_handle(cx).is_focused(window);
        let search_header_border =
            theme::longan_800().opacity(WORKSPACE_SIDEBAR_BORDER_OPACITY);
        let search = if rail {
            div()
                .debug_selector(|| "desktop-settings-search-header".to_owned())
                .h(px(SETTINGS_PAGE_HEADER_HEIGHT_PX))
                .flex_shrink_0()
                .border_b_1()
                .border_color(search_header_border)
                .flex()
                .items_center()
                .justify_center()
                .child(
                    div()
                        .id("desktop-settings-show-search")
                        .debug_selector(|| "desktop-settings-show-search".to_owned())
                        .tab_index(0)
                        .size(px(30.))
                        .rounded(px(8.))
                        .border_1()
                        .border_color(gpui::transparent_black())
                        .hover(|el| el.bg(theme::longan_800().opacity(0.6)))
                        .focus_visible(|el| el.border_color(theme::persimmon_400()))
                        .role(Role::Button)
                        .aria_label(self.ui_language().text("Search settings"))
                        .cursor_pointer()
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(
                            svg()
                                .path(assets::TOOL_SEARCH_ICON_PATH)
                                .size(px(theme::ICON_PX))
                                .text_color(theme::rambutan_400()),
                        )
                        .map(|el| {
                            on_press_view(el, cx, |this, window, cx| {
                                this.focus_settings_search(window, cx)
                            })
                        }),
                )
                .into_any_element()
        } else {
            div()
                .debug_selector(|| "desktop-settings-search-header".to_owned())
                .h(px(SETTINGS_PAGE_HEADER_HEIGHT_PX))
                .px(px(8.))
                .flex_shrink_0()
                .border_b_1()
                .border_color(search_header_border)
                .flex()
                .items_center()
                .child(
                    div()
                        .id("desktop-settings-search-box")
                        .debug_selector(|| "desktop-settings-search-box".to_owned())
                        .h(px(30.))
                        .w_full()
                        .min_w_0()
                        .px(px(8.))
                        .rounded(px(8.))
                        .bg(theme::longan_950().opacity(0.65))
                        .border_1()
                        .border_color(if search_focused {
                            theme::persimmon_400()
                        } else {
                            theme::longan_800().opacity(0.9)
                        })
                        .hover(|el| {
                            if search_focused {
                                el
                            } else {
                                el.bg(theme::longan_950().opacity(0.85))
                                    .border_color(theme::longan_700())
                            }
                        })
                        .cursor_text()
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.focus_settings_search(window, cx);
                        }))
                        .child(
                            svg()
                                .path(assets::TOOL_SEARCH_ICON_PATH)
                                .size(px(14.))
                                .flex_shrink_0()
                                .text_color(if search_focused {
                                    theme::rambutan_300()
                                } else {
                                    theme::rambutan_500()
                                }),
                        )
                        .child(
                            div().min_w_0().flex_1().child(
                                Input::new(&self.settings_search)
                                    .appearance(false)
                                    .px_0()
                                    .bordered(false)
                                    .focus_bordered(false)
                                    .cleanable(false)
                                    .text_size(px(13.))
                                    .role(Role::EditableComboBox)
                                    .aria_expanded(
                                        has_query && !self.settings_search_results.is_empty(),
                                    )
                                    .aria_label(self.ui_language().text("Search settings")),
                            ),
                        )
                        .when(!has_query && !search_focused, |el| {
                            el.child(bruhs_ui::kbd::kbd("/"))
                        })
                        .when(has_query, |el| {
                            let button = web_button(
                                "settings-clear-search",
                                WebButtonVariant::Ghost,
                                WebButtonSize::IconSmall,
                                ButtonState::Enabled,
                            )
                            .debug_selector(|| "settings-clear-search".to_owned())
                            .aria_label(self.ui_language().text("Clear settings search"))
                            .size(px(20.))
                            .min_w(px(20.))
                            .min_h(px(20.))
                            .flex_shrink_0()
                            .child(
                                svg()
                                    .path(assets::CLOSE_ICON_PATH)
                                    .size(px(12.))
                                    .text_color(theme::rambutan_400()),
                            );
                            el.child(on_press_view(button, cx, |this, window, cx| {
                                this.settings_search.update(cx, |input, cx| {
                                    input.set_value("", window, cx);
                                    input.focus(window, cx);
                                });
                                this.refresh_settings_search(cx);
                            }))
                        }),
                )
                .into_any_element()
        };
        let shell = div()
            .id("desktop-settings-nav")
            .debug_selector(|| "desktop-settings-nav".to_owned())
            .when(expanded, |nav| nav.flex_1().min_w_0())
            .when(!expanded, |nav| nav.w(px(settings_nav_width(width))))
            .theme_background("sidebar", theme::longan_900(), cx)
            .flex_shrink_0()
            .border_r_1()
            .border_color(search_header_border)
            .flex()
            .flex_col()
            .child(search);
        let index = self.settings_section.nav_index();
        let show_sections = !has_query;
        let pending = self.settings_nav_reveal.clone();
        let scroll = self.settings_nav_scroll.clone();
        let view = cx.entity_id();
        let mut nav = div().on_children_prepainted(move |_, _, cx| {
            if show_sections && pending.replace(false) {
                let scroll = scroll.clone();
                cx.defer(move |cx| {
                    scroll.scroll_to_item(index);
                    cx.notify(view);
                });
            }
        })
            .id("desktop-settings-links")
            .debug_selector(|| "desktop-settings-links".to_owned())
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .track_scroll(&self.settings_nav_scroll)
            .px(px(8.))
            .py_2()
            .flex()
            .flex_col()
            .gap(px(2.0))
            .when(show_sections, |nav| nav.role(Role::TabList).aria_label(self.ui_language().text("Settings sections")))
            .when(has_query, |nav| nav.role(Role::ListBox).aria_label(self.ui_language().text("Settings search results")));
        if has_query {
            for (index, hit) in self.settings_search_results.iter().enumerate() {
                nav = nav.child(
                    div()
                        .id(("settings-hit", index))
                        .debug_selector(move || format!("desktop-settings-hit-{index}"))
                        .on_mouse_move(cx.listener(move |this, _, _, cx| {
                            if this.settings_search_index != index {
                                this.settings_search_index = index;
                                cx.notify();
                            }
                        }))
                        .border_1()
                        .border_color(gpui::transparent_black())
                        .focus_visible(|el| el.border_color(theme::persimmon_400()))
                        .role(Role::ListBoxOption)
                        .aria_selected(index == self.settings_search_index)
                        .aria_label(format!("{} in {}", hit.title, self.ui_language().text(hit.section.label())))
                        .cursor_pointer()
                        .flex()
                        .items_start()
                        .gap_2()
                        .flex_shrink_0()
                        .p_2()
                        .rounded(px(8.0))
                        .when(index == self.settings_search_index, |el| {
                            el.bg(theme::longan_800())
                        })
                        .hover(|el| el.bg(theme::longan_800()))
                        .child(svg().path(hit.section.icon_path()).mt_1().size(px(theme::ICON_PX))
                            .flex_shrink_0().text_color(theme::rambutan_500()))
                        .child(div().min_w_0().flex_1()
                            .child(div().truncate().child(hit.title.clone()))
                            .child(div().text_xs().text_color(theme::rambutan_500())
                                .child(self.ui_language().text(hit.section.label()))))
                        .map(|el| {
                            on_press_view(el, cx, move |this, window, cx| {
                                this.open_settings_hit(index, window, cx)
                            })
                        }),
                );
            }
            if self.settings_search_results.is_empty() {
                nav = nav.child(div()
                    .id("settings-no-results")
                    .debug_selector(|| "desktop-settings-no-results".to_owned())
                    .px_2()
                    .text_xs()
                    .role(Role::Status)
                    .aria_label(self.ui_language().text("No settings found"))
                    .child(self.ui_language().text("No settings found")));
            }
            return shell.child(nav).into_any_element();
        }
        for (position, section) in SettingsSection::ALL.into_iter().enumerate() {
            let group = section.group_heading();
            if let Some(group) = group {
                nav = if rail {
                    nav.child(div().my_2().border_t_1().border_color(theme::longan_800()))
                } else {
                    nav.child(
                        div()
                            .mt_3()
                            .px_2()
                            .text_xs()
                            .theme_foreground("sidebarMutedForeground", theme::rambutan_500(), cx)
                            .child(self.ui_language().text(group)),
                    )
                };
            }
            let active = section == self.settings_section;
            let target = cx.weak_entity();
            let section_label = self.ui_language().text(section.label());
            let selector = section.selector();
            nav = nav.child(
                div()
                    .id(selector)
                    .debug_selector(move || selector.to_string())
                    .h(px(36.0))
                    .flex_shrink_0()
                    .when(rail, |el| el.justify_center())
                    .tooltip(move |w, cx| Tooltip::new(section_label).build(w, cx))
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
                    .when(active, |el| el.theme_background("sidebarRowSelected", theme::longan_800().opacity(0.6), cx))
                    .when(!active, |el| {
                        el.hover(|el| {
                            el.bg(theme::longan_800().opacity(0.4))
                                .text_color(theme::rambutan_100())
                        })
                    })
                    .track_focus(&self.settings_nav_focus[position])
                    .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
                        let mods = event.keystroke.modifiers;
                        if mods.platform || mods.control || mods.alt {
                            return;
                        }
                        let last = SettingsSection::ALL.len() - 1;
                        let next = match event.keystroke.key.as_str() {
                            "up" => position.saturating_sub(1),
                            "down" => (position + 1).min(last),
                            "home" => 0,
                            "end" => last,
                            _ => return,
                        };
                        this.settings_nav_focus[next].focus(window, cx);
                        this.settings_nav_scroll.scroll_to_item(SettingsSection::ALL[next].nav_index());
                        window.prevent_default();
                        cx.stop_propagation();
                    }))
                    .role(Role::Tab)
                    .on_a11y_action(gpui::AccessibleAction::Click, move |_, _, cx| {
                        let _ = target.update(cx, |this, cx| this.set_settings_section(section, cx));
                    })
                    .aria_label(section_label)
                    .aria_selected(active)
                    .border_1()
                    .border_color(gpui::transparent_black())
                    .focus_visible(|el| el.border_color(theme::persimmon_400()))
                    .cursor_pointer()
                    .map(|el| {
                        on_press_view(el, cx, move |this, _window, cx| {
                            this.set_settings_section(section, cx);
                        })
                    })
                    .child(
                        svg()
                            .path(section.icon_path())
                            .size(px(theme::ICON_PX))
                            .text_color(theme::rambutan_500())
                            .flex_shrink_0(),
                    )
                    .when(!rail, |el| el.child(div().min_w_0().flex_1().truncate().child(section_label))),
            );
        }
        shell.child(nav).into_any_element()
    }

    fn render_settings_general(&self, cx: &Context<'_, Self>) -> AnyElement {
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
            self.ui_language().text("Editor"),
            &self.editor_id,
            editor_groups,
            self.settings_listbox(SETTINGS_EDITOR_DROPDOWN_ID),
            editor_toggle,
            editor_select,
        );

        let editor = vec![self.settings_row(
            "desktop-settings-row-editor",
            "Editor",
            "Used by ⌘O and Open buttons.",
            settings_ui::control_column(editor_control),
        )];
        div()
            .flex()
            .flex_col()
            .gap(px(24.))
            .child(self.settings_card("Notifications", "", self.thread_notification_rows(cx)))
            .child(self.settings_card("Sidebar", "", self.render_settings_sidebar_rows(cx)))
            .child(self.settings_card(
                "Confirmations",
                "",
                vec![
                    self.settings_row(
                        "desktop-settings-row-confirm-unpin",
                        "Unpin confirmation",
                        "",
                        settings_ui::switch(
                            "settings-confirm-unpin",
                            self.confirm_thread_unpin,
                            false,
                            self.ui_language().text("Unpin confirmation"),
                            cx,
                            |this, _, cx| {
                                this.confirm_thread_unpin = !this.confirm_thread_unpin;
                                this.persist_and_notify(cx);
                            },
                        ),
                    ),
                    self.settings_row(
                        "desktop-settings-row-confirm-archive",
                        "Archive confirmation",
                        "",
                        settings_ui::switch(
                            "settings-confirm-archive",
                            self.confirm_thread_archive,
                            false,
                            self.ui_language().text("Archive confirmation"),
                            cx,
                            |this, _, cx| {
                                this.confirm_thread_archive = !this.confirm_thread_archive;
                                this.persist_and_notify(cx);
                            },
                        ),
                    ),
                    self.settings_row(
                        "desktop-settings-row-confirm-delete",
                        "Delete confirmation",
                        "",
                        settings_ui::switch(
                            "settings-confirm-delete",
                            self.confirm_thread_delete,
                            false,
                            self.ui_language().text("Delete confirmation"),
                            cx,
                            |this, _, cx| {
                                this.confirm_thread_delete = !this.confirm_thread_delete;
                                this.persist_and_notify(cx);
                            },
                        ),
                    ),
                    self.quit_shortcut_row(cx),
                ],
            ))
            .child(self.settings_card(
                "Display",
                "",
                vec![self.language_row(cx), self.settings_row(
                    "desktop-settings-row-clock",
                    "Time format",
                    "",
                    settings_ui::control_column(self.choice_control(
                        "settings-clock-format",
                        "Time format",
                        self.appearance.clock.key(),
                        &[
                            ("system", "System default"),
                            ("12h", "12-hour"),
                            ("24h", "24-hour"),
                        ],
                        cx,
                    )),
                )],
            ))
            .child(self.settings_card("External editor", "", editor))
            .child(
                crate::views::reactive_automations::AgentNotificationSettings(
                    self.reactive_automations.clone(),
                    self.ui_language(),
                ),
            )
            .into_any_element()
    }

    fn set_composer_rich_text(&mut self, enabled: bool, cx: &mut Context<'_, Self>) {
        self.composer_rich_text = enabled;
        self.stamp_run_prefs_on_threads(cx);
        self.persist_and_notify(cx);
    }

    fn rich_composer_row(&self, cx: &Context<'_, Self>) -> AnyElement {
        let label = "Rich text composer";
        let toggle = settings_ui::switch(
            "settings-rich-composer",
            self.composer_rich_text,
            false,
            label,
            cx,
            |ws, _, cx| {
                ws.set_composer_rich_text(!ws.composer_rich_text, cx);
            },
        );
        let reset = (!self.composer_rich_text).then(|| {
            let button = web_button(
                "reset-rich-composer",
                WebButtonVariant::Ghost,
                WebButtonSize::IconSmall,
                ButtonState::Enabled,
            )
            .debug_selector(|| "reset-rich-composer".into())
            .aria_label("Reset rich text composer")
            .child(
                svg()
                    .path(assets::ROTATE_CCW_ICON_PATH)
                    .size(px(14.))
                    .text_color(theme::rambutan_400()),
            );
            on_press_view(button, cx, |ws, _, cx| ws.set_composer_rich_text(true, cx))
                .into_any_element()
        });
        self.settings_row_with_reset(
            "desktop-settings-row-rich-composer",
            label,
            "Preview Markdown as you type.",
            None,
            false,
            Some(toggle),
            reset,
        )
    }

    fn render_chat_rows(&self, cx: &Context<'_, Self>) -> Vec<AnyElement> {
        let modifier = if cfg!(target_os = "macos") {
            "⌘"
        } else {
            "Ctrl"
        };
        let groups = vec![DropdownGroup {
            label: None,
            mark: None,
            options: SendShortcut::OPTIONS
                .into_iter()
                .map(|(mode, value)| DropdownOption {
                    value,
                    label: match mode {
                        SendShortcut::Enter => "Enter".to_string(),
                        SendShortcut::ModEnterMultiline => {
                            format!("{modifier} + Enter for multiline prompts")
                        }
                        SendShortcut::ModEnter => format!("{modifier} + Enter always"),
                    },
                    description: None,
                })
                .collect(),
        }];
        let (toggle, select) = self.settings_dropdown_handlers(SETTINGS_SEND_SHORTCUT, cx);
        vec![
            self.settings_row("settings-row-tool-visibility","Tool calls in chat","How much tool activity appears in chat.",settings_ui::control_column(self.choice_control("settings-tool-visibility","Tool calls in chat",self.tool_visibility.key(),&[("all","Show all"),("failures","Hide successful"),("hide","Hide all"),("zen","Zen")],cx))),
            self.settings_row("desktop-settings-row-slash-skills","Show skills in slash menu","Include skills in /.",settings_ui::switch("settings-slash-skills",self.slash_skills,false,"Show skills in slash menu",cx,|this,_,cx|{
                this.slash_skills = !this.slash_skills;
                this.stamp_run_prefs_on_threads(cx);
                this.persist_and_notify(cx);
            })),
            self.settings_row("desktop-settings-row-reader-announcements","Screen reader announcements",
                "What to announce during a turn.",
                settings_ui::control_column(self.choice_control("settings-reader-announcements","Screen reader announcements",self.screen_reader.announcements.key(),
                    &[("status","Status only"),("messages","Status and messages"),("everything","Everything")],cx))),
            self.settings_row("desktop-settings-row-reader-title","Screen reader window title",
                "Prefix window title with thread status and name.",
                settings_ui::switch("settings-reader-title",self.screen_reader.window_title,false,"Screen reader window title",cx,|this,_,cx|{
                    this.screen_reader.window_title = !this.screen_reader.window_title;
                    this.persist_and_notify(cx);
                })),
            self.rich_composer_row(cx),
            self.settings_row(
                "desktop-settings-row-send-shortcut",
                "Send shortcut",
                "Shift + Enter adds a new line.",
                settings_ui::control_column(dropdown(SETTINGS_SEND_SHORTCUT, "Send shortcut", self.send_shortcut.id(), groups, self.settings_listbox(SETTINGS_SEND_SHORTCUT), toggle, select)),
            ),
            self.settings_row(
                "desktop-settings-row-follow-up",
                "Follow up behavior",
                "Queue or steer when sending mid-turn.",
                self.render_follow_up_control(cx),
            ),
            self.settings_row(
                "desktop-settings-row-autocompact",
                "Automatically compact after",
                "Claude token threshold (100k to 1M).",
                settings_ui::control_column(
                    div()
                        .w_full()
                        .h(px(theme::CONTROL_PX))
                        .child(Input::new(&self.autocompact_input).aria_label("Automatic compaction token count"))
                        .into_any_element(),
                ),
            ),
            self.settings_row(
                "desktop-settings-row-compact-idle",
                "Compact idle threads",
                "Compact long Claude threads when idle.",
                settings_ui::control_column(self.render_compact_idle_control(cx)),
            ),
        ]
    }

    fn render_diff_rows(&self, cx: &Context<'_, Self>) -> Vec<AnyElement> {
        let prefs = self.diff_panel.read(cx).prefs;
        vec![
            self.settings_row(
                "desktop-settings-row-diff-colors",
                "Diff colors",
                "",
                settings_ui::control_column(self.choice_control(
                    "settings-diff-colors",
                    "Diff colors",
                    if prefs.blue_orange {
                        "blue-orange"
                    } else {
                        "red-green"
                    },
                    &[
                        ("red-green", "Red & green"),
                        ("blue-orange", "Blue & orange"),
                    ],
                    cx,
                )),
            ),
            self.settings_row(
                "desktop-settings-row-diff-whitespace",
                "Hide whitespace changes",
                "",
                settings_ui::switch(
                    "settings-diff-whitespace",
                    prefs.hide_whitespace,
                    false,
                    "Hide whitespace changes",
                    cx,
                    |this, _, cx| {
                        let mut prefs = this.diff_panel.read(cx).prefs;
                        prefs.hide_whitespace = !prefs.hide_whitespace;
                        this.diff_panel
                            .update(cx, |panel, cx| panel.set_preferences(prefs, cx));
                        if this.diff_panel_visible {
                            this.sync_diff_panel_project(cx);
                        }
                        this.persist_and_notify(cx);
                    },
                ),
            ),
            self.settings_row(
                "desktop-settings-row-diff-file-state",
                "Default diff file state",
                "",
                settings_ui::control_column(self.choice_control(
                    "settings-diff-file-state",
                    "Default diff file state",
                    if prefs.collapsed {
                        "collapsed"
                    } else {
                        "expanded"
                    },
                    &[("expanded", "Expanded"), ("collapsed", "Collapsed")],
                    cx,
                )),
            ),
            self.settings_row(
                "desktop-settings-row-diff-layout",
                "Diff layout",
                "",
                settings_ui::control_column(self.choice_control(
                    "settings-diff-layout",
                    "Diff layout",
                    if prefs.split { "split" } else { "stacked" },
                    &[("stacked", "Stacked"), ("split", "Side by side")],
                    cx,
                )),
            ),
            self.settings_row("desktop-settings-row-proactive-panels","Open pull requests and changes panels proactively",
                "Open linked PRs or changes after large edits.",
                settings_ui::switch("settings-proactive-panels",prefs.proactive_panels,false,"Open panels proactively",cx,|this,_,cx|{
                    this.diff_panel.update(cx,|panel,cx|{let mut prefs = panel.prefs; prefs.proactive_panels = !prefs.proactive_panels; panel.set_preferences(prefs,cx)});
                    this.persist_and_notify(cx);
                })),
            self.settings_row("desktop-settings-row-open-pull-links","Open pull requests in bruhs",
                "Open GitHub PR links inside bruhs.",
                settings_ui::switch("settings-open-pull-links",prefs.open_pull_links_in_app,false,"Open pull requests in bruhs",cx,|this,_,cx|{
                    this.diff_panel.update(cx,|panel,cx|{let mut prefs = panel.prefs; prefs.open_pull_links_in_app = !prefs.open_pull_links_in_app; panel.set_preferences(prefs,cx)});
                    this.persist_and_notify(cx);
                })),
        ]
    }

    fn render_appearance(&self, width: f32, cx: &Context<'_, Self>) -> AnyElement {
        let percent_slider = |kind, value: u8| {
            settings_ui::control_column(
                div()
                    .w_full()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(self.render_settings_slider(kind, cx)),
                    )
                    .child(div().flex_shrink_0().child(format!("{value}%")))
                    .into_any_element(),
            )
        };
        let step = |increase: bool| {
            let (id, text) = if increase {
                ("settings-zoom-in", "Zoom in")
            } else {
                ("settings-zoom-out", "Zoom out")
            };
            let level = if increase {
                crate::zoom::zoom_in(self.zoom)
            } else {
                crate::zoom::zoom_out(self.zoom)
            };
            web_button(
                id,
                WebButtonVariant::Outline,
                WebButtonSize::Small,
                ButtonState::disabled_if(level == self.zoom),
            )
            .aria_label(text)
            .child(if increase { "+" } else { "−" })
            .on_click(cx.listener(move |this, _, window, cx| this.set_zoom(level, window, cx)))
        };
        let colors = vec![
            self.settings_row(
                "desktop-settings-row-contrast",
                "Contrast",
                "",
                percent_slider(
                    settings_ui::SettingsSlider::Contrast,
                    self.appearance.contrast,
                ),
            ),
            self.settings_row(
                "desktop-settings-row-glass-opacity",
                "Glass opacity",
                "Opacity of menus, dialogs, and composer.",
                percent_slider(
                    settings_ui::SettingsSlider::GlassOpacity,
                    self.appearance.glass_opacity,
                ),
            ),
        ];
        let interface = vec![
            self.settings_row("desktop-settings-row-panel-animations","Panel animations","Duration in ms. 0 disables animation.",
                settings_ui::control_column(div().w_full().flex().items_center().gap_2().child(div().flex_1().min_w_0().child(self.render_settings_slider(settings_ui::SettingsSlider::PanelDuration,cx))).child(format!("{} ms",self.appearance.panel_duration_ms)).into_any_element())),

            self.settings_row(
                "desktop-settings-row-chat-width",
                "Chat width",
                "",
                settings_ui::control_column(self.choice_control(
                    "settings-chat-width",
                    "Chat width",
                    self.chat_width.id(),
                    &[
                        ("comfortable", "Comfortable"),
                        ("wide", "Wide"),
                        ("full", "Full width"),
                    ],
                    cx,
                )),
            ),
            self.thread_detail_settings_row(cx),
            self.settings_row(
                "desktop-settings-row-effort-display", "Effort display",
                "",
                settings_ui::control_column(self.choice_control("settings-effort-display", "Effort display",
                    if self.appearance.effort_label { "label" } else { "bars" },
                    &[("label", "Word (default)"), ("bars", "Bars")], cx)),
            ),
            self.settings_row(
                "desktop-settings-row-render-math",
                "Render math",
                "Typeset LaTeX in chat.",
                settings_ui::switch(
                    "settings-render-math",
                    self.appearance.render_math,
                    false,
                    "Render math",
                    cx,
                    |this, _, cx| {
                        this.appearance.render_math = !this.appearance.render_math;
                        this.stamp_run_prefs_on_threads(cx);
                        this.persist_and_notify(cx);
                    },
                ),
            ),
            self.settings_row(
                "desktop-settings-row-zoom",
                "Interface size",
                "",
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(step(false))
                    .child(format!("{:.0}%", self.zoom * 100.))
                    .child(step(true))
                    .into_any_element(),
            ),
        ];
        div()
            .flex()
            .flex_col()
            .gap(px(24.))
            .child(self.scheme_picker(cx))
            .child(self.theme_library(width, cx))
            .child(self.settings_card("Contrast & opacity", "", colors))
            .child(self.settings_card("Interface", "", interface.into_iter().chain(self.text_layout_rows(cx)).collect()))
            .child(self.settings_card("Fonts","",self.render_font_rows(cx)))
            .into_any_element()
    }

    pub(super) fn catalog_model_groups(&self, cx:&Context<'_,Self>, include:impl Fn(AgentProvider)->bool)->Vec<DropdownGroup> {
        desktop_model_provider_groups_like_web()
            .into_iter()
            .filter(|(provider,_)| include(*provider) && self.provider_availability.as_ref().is_ok_and(|prefs| prefs.enabled(*provider)))
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
            .collect()
    }

    fn inherited_group(label: String) -> DropdownGroup {
        DropdownGroup {
            label: None,
            mark: None,
            options: vec![DropdownOption {
                value: "inherit",
                label: format!("Use environment default ({label})"),
                description: None,
            }],
        }
    }


    fn render_settings_threads(&self, cx: &Context<'_, Self>) -> AnyElement {
        let scoped = !self.settings_project_scope.is_empty();
        let project = self.project_defaults.get(&self.settings_project_scope);
        let mut mode_groups = vec![DropdownGroup {
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
        if scoped {
            mode_groups.insert(0, Self::inherited_group(self.permission_mode.label().to_owned()));
        }
        let (mode_toggle, mode_select) =
            self.settings_dropdown_handlers(SETTINGS_ACCESS_MODE_DROPDOWN_ID, cx);
        let mode_control = dropdown(
            SETTINGS_ACCESS_MODE_DROPDOWN_ID,
            "Access mode",
            if scoped {
                project
                    .and_then(|p| p.permissions)
                    .map_or("inherit", |m| m.as_str())
            } else {
                self.permission_mode.as_str()
            },
            mode_groups,
            self.settings_listbox(SETTINGS_ACCESS_MODE_DROPDOWN_ID),
            mode_toggle,
            mode_select,
        );

        let placement = self.new_thread_placement_row(cx);
        let mut defaults = vec![
            self.settings_row(
                "desktop-settings-row-access-mode",
                "Access mode",
                "Default file and command permissions.",
                settings_ui::control_column(mode_control),
            ),
        ];
        if !scoped {defaults.insert(0,self.fallback_row(cx));}
        let messages = vec![
            self.settings_row(
                "desktop-settings-row-thread-access",
                "Messages between threads",
                "Let agents list, read, or message other threads.",
                settings_ui::control_column(self.render_thread_access_control(cx)),
            ),
            self.settings_row(
                "desktop-settings-row-thread-hops",
                "Agent message limit",
                "Max consecutive agent-to-agent messages.",
                settings_ui::control_column(self.render_thread_hops_control(cx)),
            ),
        ];
        let titles = vec![
            self.text_generation_model_row(cx),
            self.settings_row(
                "desktop-settings-row-title-instructions",
                "Thread title instructions",
                "Style rules for new and regenerated titles.",
                settings_ui::control_column(
                    div()
                        .w_full()
                        .h(px(96.0))
                        .child(
                            Textarea::new(&self.title_instructions_input)
                                .aria_label("Thread title instructions")
                                .h_full(),
                        )
                        .into_any_element(),
                ),
            ),
            self.settings_row(
                "desktop-settings-row-project-title",
                "Project title instructions",
                "Extra rules for one project.",
                settings_ui::control_column(
                    div()
                        .w_full()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .child(self.render_title_project_control(cx))
                        // Web shows the field once a project is chosen.
                        .when(!self.title_project.is_empty(), |el| {
                            el.child(
                                div().h(px(80.0)).child(
                                    Textarea::new(&self.title_project_instructions_input)
                                        .aria_label("Project thread title instructions")
                                        .h_full(),
                                ),
                            )
                        })
                        .into_any_element(),
                ),
            ),
        ];
        div()
            .flex()
            .flex_col()
            .gap(px(24.0))
            .child(self.settings_card(
                "New projects",
                "",
                vec![self.settings_row(
                    "desktop-settings-row-project-start-folder",
                    "Add project starts in",
                    "Default folder for Add project.",
                    settings_ui::control_column(
                        Input::new(&self.project_start_folder_input)
                            .aria_label("Add project starts in")
                            .into_any_element(),
                    ),
                ),self.settings_row("desktop-settings-row-clone-host-folders","Clone into host folders","Group clones by host and owner.",
                    settings_ui::switch("settings-clone-host-folders",self.clone_host_folders,false,"Clone into host folders",cx,|this,_,cx|{
                        this.clone_host_folders = !this.clone_host_folders;
                        this.persist_and_notify(cx);
                    }))],
            ))
            .child(self.settings_card("New threads", "", vec![self.defaults_scope_row(cx), self.new_thread_model_row(cx), placement,self.response_stream_row(cx)]))
            .child(self.settings_card("New thread defaults", "", defaults))
            .child(self.settings_card("Thread lifecycle", "", self.render_settlement_rows(cx)))
            .child(self.render_snooze_settings(cx))
            .child(self.render_quota_settings(cx))
            .child(self.settings_card("Messages between threads", "", messages))
            .child(self.settings_card("Roles", "", self.render_role_settings_rows(cx)))
            .child(self.settings_card("Task models", "", self.render_task_model_rows(cx)))
            .child(self.settings_card("Thread titles", "", titles))
            .into_any_element()
    }

    /// Web: a list of the projects threads are in, "Choose a project" first.
    fn render_title_project_control(&self, cx: &Context<'_, Self>) -> AnyElement {
        let options = std::iter::once(DropdownOption {
            value: "",
            label: "Choose a project".to_string(),
            description: None,
        })
        .chain(
            self.thread_project_names()
                .into_iter()
                .map(|name| DropdownOption {
                    value: role_settings_ui::project_value(&name),
                    label: name,
                    description: None,
                }),
        )
        .collect();
        let groups = vec![DropdownGroup {
            label: None,
            mark: None,
            options,
        }];
        let (toggle, select) =
            self.settings_dropdown_handlers(SETTINGS_TITLE_PROJECT_DROPDOWN_ID, cx);
        dropdown(
            SETTINGS_TITLE_PROJECT_DROPDOWN_ID,
            "Project for title instructions",
            role_settings_ui::project_value(&self.title_project),
            groups,
            self.settings_listbox(SETTINGS_TITLE_PROJECT_DROPDOWN_ID),
            toggle,
            select,
        )
    }

    fn changed_device_settings(&self, cx: &App) -> Vec<&'static str> {
        let mut changed = Vec::new();
        if self.appearance.scheme != appearance::ColorScheme::default() {
            changed.push("Color scheme");
        }
        if self.custom_themes.has_selection() || self.appearance.light_theme != Default::default() || self.appearance.dark_theme != Default::default() {
            changed.push("Themes");
        }
        if self.appearance.language != Default::default() {
            changed.push("Language");
        }
        if self.appearance.clock != Default::default() {
            changed.push("Time format");
        }
        if self.snapshots != Default::default() {changed.push("SnapShots");}
        if self.fonts!=Default::default(){changed.push("Fonts");}
        if self.appearance.contrast != 100 {
            changed.push("Contrast");
        }
        if self.appearance.glass_opacity != 100 {
            changed.push("Glass opacity");
        }
        if self.appearance.thread_details != Default::default() {changed.push("Thread details");}
        if !self.appearance.effort_label { changed.push("Effort display"); }
        if !self.appearance.render_math {
            changed.push("Render math");
        }
        if crate::views::browser_preferences::BrowserPreferences::get(cx).has_changed_settings() {
            changed.push("Browser preferences");
        }
        if self.dictation_prefs.has_changed_settings() {
            changed.push("Voice preferences");
        }
        if self.diff_panel.read(cx).prefs != Default::default() {
            changed.push("Diff preferences");
        }
        if self.screen_reader != Default::default() {
            changed.push("Screen reader preferences");
        }
        if !self.composer_rich_text { changed.push("Rich text composer"); }
        if self.send_shortcut != SendShortcut::default() {
            changed.push("Send shortcut");
        }
        if self.thread_notifications.mode != thread_notifications::NotificationMode::Off {
            changed.push("Thread notifications");
        }
        if self.thread_notifications.in_app {
            changed.push("In-app notifications");
        }
        if self.confirm_thread_archive {
            changed.push("Archive confirmation");
        }
        if self.confirm_thread_unpin {
            changed.push("Unpin confirmation");
        }
        if !self.confirm_thread_delete {
            changed.push("Delete confirmation");
        }
        if self.project_list.grouping.mode != Default::default() { changed.push("Project grouping"); }
        if !self.sidebar_working_section {
            changed.push("Working section");
        }
        if self.editor_id != bruhs_models::logic::editor::DEFAULT_EDITOR_ID {
            changed.push("External editor");
        }
        if self.chat_width != ChatWidth::default() {
            changed.push("Chat width");
        }
        if !crate::zoom::is_default(self.zoom) {
            changed.push("Interface size");
        }
        if !self.updates.read(cx).enabled {
            changed.push("Check for updates");
        }
        changed
    }

    fn confirm_restore_device_settings(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let changed = self.changed_device_settings(cx);
        if changed.is_empty() {
            return;
        }
        let language = self.ui_language();
        let detail = format!(
            "{} {}",language.text("Restore these device preferences to their defaults?"),
            changed.into_iter().map(|key|language.text(key)).collect::<Vec<_>>().join(", ")
        );
        let answer = window.prompt(
            gpui::PromptLevel::Info,
            language.text("Restore device defaults?"),
            Some(&detail),
            &[
                gpui::PromptButton::Cancel(language.text("Cancel").into()),
                gpui::PromptButton::Ok(language.text("Restore defaults").into()),
            ],
            cx,
        );
        cx.spawn_in(window, async move |this, cx| {
            if answer.await == Ok(1) {
                let _ = this.update_in(cx, |this, window, cx| {
                    Arc::make_mut(&mut this.custom_themes).reset_selection();
                    this.appearance = Default::default();
                    this.system_language = language::Language::system();
                    this.refresh_language_ui(window, cx);
                    this.snapshots = Default::default();
                    this.sync_capture_shortcut(cx);
                    this.fonts=Default::default();
                    this.diff_panel.update(cx,|p,cx|p.set_default_wrap(None,cx));
                    this.apply_device_appearance(window, cx);
                    cx.default_global::<crate::views::browser_preferences::BrowserPreferences>()
                        .restore_defaults();
                    this.refresh_viewport_fields(window, cx);
                    this.browser_search_url_input
                        .update(cx, |field, cx| field.set_value("", window, cx));
                    this.dictation_prefs.restore_defaults();
                    this.voice_cleanup_fields.reset(window,cx);
                    this.voice_model_input.update(cx,|field,cx|field.set_value("",window,cx));
                    this.composer_rich_text = true;
                    this.send_shortcut = Default::default();
                    this.screen_reader = Default::default();
                    this.diff_panel.update(cx, |panel, cx| {
                        panel.set_preferences(Default::default(), cx)
                    });
                    if this.diff_panel_visible {
                        this.sync_diff_panel_project(cx);
                    }
                    this.chat_width = ChatWidth::default();
                    this.stamp_run_prefs_on_threads(cx);
                    this.thread_notifications = Default::default();
                    this.confirm_thread_archive = false;
                    this.confirm_thread_unpin = false;
                    this.confirm_thread_delete = true;
                    this.sidebar_working_section = true;
                    this.project_list.grouping.mode = Default::default();
                    this.editor_id = bruhs_models::logic::editor::DEFAULT_EDITOR_ID.to_string();
                    if !this.updates.read(cx).enabled {
                        this.updates
                            .update(cx, |updates, cx| updates.set_enabled(true, cx));
                    }
                    this.set_zoom(crate::zoom::DEFAULT, window, cx);
                });
            }
        })
        .detach();
    }

    pub(super) fn render_settings_sidebar_rows(&self, cx: &Context<'_, Self>) -> Vec<AnyElement> {
        let toggle = settings_ui::switch(
            "desktop-settings-working-section",
            self.sidebar_working_section && !self.labs_sidebar.enabled,
            self.labs_sidebar.enabled,
            self.ui_language().text("Working section"),
            cx,
            |this, _window, cx| {
                this.sidebar_working_section = !this.sidebar_working_section;
                this.persist_and_notify(cx);
            },
        );
        let mut rows = vec![self.project_grouping_row(cx)];
        rows.extend(self.render_project_order_rows(cx));
        rows.push(self.settings_row(
            "desktop-settings-row-working-section",
            "Working section",
            if self.labs_sidebar.enabled {"Project folders are enabled in Labs. Working threads appear in their project folder."} else {"Group working threads in the sidebar."},
            toggle,
        ));
        rows
    }

    fn render_about(&self, cx: &Context<'_, Self>) -> AnyElement {
        // Web `AboutSettings`: the packaged release version (from the bundle,
        // the one updates compare), else the crate version in a dev build.
        let version = self
            .updates
            .read(cx)
            .report
            .as_ref()
            .and_then(|report| report.installed_release().map(str::to_owned))
            .unwrap_or_else(|| {
                let release = env!("BRUHS_RELEASE_VERSION");
                if release.is_empty() { env!("CARGO_PKG_VERSION") } else { release }.to_owned()
            });
        let muted = |text: &'static str| {
            div()
                .text_size(px(SETTINGS_BODY_TEXT_SIZE_PX))
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
        let action = |id: &'static str, label: &'static str| {
            web_button(id, WebButtonVariant::Outline, WebButtonSize::Small, ButtonState::Enabled)
                .aria_label(label)
                .child(label)
        };
        let packaged = !env!("BRUHS_RELEASE_VERSION").is_empty() ||
            self.updates.read(cx).report.as_ref().is_some_and(|r| r.app.release.is_some());
        let mut app = self.render_app_update_row(updates_ui::UpdateRow::About(&version), cx);
        app.extend([
                self.settings_row(
                    "settings-update-track", "Update track",
                    if packaged { "" } else { "Follows this Git checkout." },
                    if packaged { settings_ui::control_column(self.choice_control("settings-update-track", "Update track", self.updates.read(cx).track.key(), &[("stable", "Stable"), ("nightly", "Nightly")], cx)) } else { muted("Source checkout") },
                ),
                self.launcher_row(cx),
                self.settings_row(
                    "settings-about-mobile", "Mobile app", "Pair a phone with this host.",
                    on_press_view(action("about-open-mobile", "Pair a device"), cx,
                        |this, _, cx| this.set_settings_section(SettingsSection::Access, cx)).into_any_element(),
                ),
                self.settings_row(
                    "settings-about-welcome", "Welcome page", "Reopen the setup overview.",
                    on_press_view(action("about-open-welcome", "Open welcome page"), cx, |this, window, cx| {
                        this.open_overlay(Overlay::Welcome);
                        this.focus_first_modal_tab_stop(window, cx);
                        cx.notify();
                    }).into_any_element(),
                ),
        ]);
        let legal = vec![
                self.settings_row(
                    "settings-about-privacy", "Privacy policy", "",
                    on_press_view(action("about-open-privacy", "Read policy"), cx,
                        |_, _, cx| cx.open_url("https://bruhs.ai/privacy")).into_any_element(),
                ),
                self.settings_row(
                    "settings-about-licenses", "Open source licenses",
                    "",
                    on_press_view(action("about-open-licenses", self.ui_language().text("View licenses"))
                        .track_focus(&self.license_trigger).debug_selector(|| "about-open-licenses".into()), cx, |this, window, cx| {
                        this.open_license_browser(window,cx);
                    }).into_any_element(),
                ),
        ];
        let more = vec![
                self.settings_row(
                    "desktop-settings-about-mcp",
                    "MCP servers",
                    "Configured via Claude Code.",
                    muted("Managed by Claude Code"),
                ),
                self.settings_row(
                    "desktop-settings-about-design",
                    "Design system",
                    "",
                    design_link,
                ),
        ];
        div().flex().flex_col().gap(px(24.))
            .child(self.settings_card("App", "", app))
            .child(self.settings_card("Legal", "", legal))
            .child(self.settings_card("bruhs", "", more))
            .into_any_element()
    }

    /// One stacked section: heading + card, sized to the cards column.
    pub(super) fn render_settings_section(
        &self,
        section: SettingsSection,
        width: f32,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        let id = section.id();
        let container = div()
            .debug_selector(move || format!("desktop-settings-section-{id}"))
            .w(px(width))
            .flex_shrink_0();
        let rows = match section {
            SettingsSection::Vault => {
                vec![div().child(self.vault_settings.clone()).into_any_element()]
            }
            SettingsSection::Apps => {
                vec![div().child(self.apps_settings.clone()).into_any_element()]
            }
            SettingsSection::Backup => {
                vec![div().child(self.backup_settings.clone()).into_any_element()]
            }
            SettingsSection::General => {
                return container
                    .child(self.render_settings_general(cx))
                    .into_any_element()
            }
            SettingsSection::Updates => return container.child(div().flex().flex_col().gap(px(24.)).child(self.settings_card("Updates", "", self.render_settings_updates_rows(cx))).child(self.provider_health_card(cx)).child(self.usage_hubs_card(cx)).child(self.generators_card(cx))).into_any_element(),
            SettingsSection::Archived => self.render_archived_threads(cx),
            SettingsSection::Projects => {
                return container
                    .child(self.render_project_actions(cx))
                    .into_any_element()
            }
            SettingsSection::SourceControl => {
                return container
                    .child(self.render_source_control(cx))
                    .into_any_element()
            }
            SettingsSection::Maintenance => {
                return container
                    .child(self.render_maintenance(cx))
                    .into_any_element()
            }
            SettingsSection::Worktrees => {
                return container.child(self.worktree_settings(cx)).into_any_element();
            }
            SettingsSection::ScheduledTasks => {
                self.scheduled_task_settings_rows(width, cx)
            }
            SettingsSection::Diagnostics => self.diagnostic_rows(cx),
            SettingsSection::Environments => self.environment_rows(cx),
            SettingsSection::Devices => self.device_rows(cx),
            SettingsSection::Labs => {
                return container.child(self.settings_card("Beta", "", self.labs_rows(cx))).into_any_element();
            }
            SettingsSection::Access => {
                return container
                    .child(self.render_mobile(cx))
                    .into_any_element()
            }
            SettingsSection::Browser => self.render_browser_rows(cx),
            SettingsSection::SnapShots => self.render_snapshot_rows(cx),
            SettingsSection::Voice => return container.child(self.render_voice_settings(cx)).into_any_element(),
            SettingsSection::Diffs => self.render_diff_rows(cx),
            SettingsSection::Chat => self.render_chat_rows(cx),
            SettingsSection::Appearance => {
                return container
                    .child(self.render_appearance(width, cx))
                    .into_any_element()
            }
            SettingsSection::Threads => {
                return container
                    .child(self.render_settings_threads(cx))
                    .into_any_element()
            }
            SettingsSection::Calendar => self.render_settings_calendar_rows(cx),
            SettingsSection::Meetings => {
                return container
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(24.0))
                            .child(self.settings_card(
                                "Transcription",
                                "",
                                self.render_settings_transcription_rows(cx),
                            ))
                            .child(self.settings_card(
                                "Recording",
                                "",
                                self.render_settings_recording_rows(cx),
                            ))
                            .child(self.settings_card(
                                "Speakers",
                                "",
                                self.render_settings_speakers_rows(cx),
                            ))
                            .child(self.settings_card(
                                "People",
                                "",
                                self.render_settings_people_rows(cx),
                            )),
                    )
                    .into_any_element()
            }
            SettingsSection::Keyboard => {
                return container
                    .child(self.render_keybinding_settings(cx))
                    .into_any_element()
            }
            SettingsSection::About => {
                return container.child(self.render_about(cx)).into_any_element();
            }
        };
        let description = section.description();
        let title = if description.is_empty() { "" } else { section.label() };
        container
            .child(self.settings_card(title, description, rows))
            .into_any_element()
    }

    /// A centered settings dialog with its own navigation, header, and scrolling pane.
    pub(super) fn render_settings(&self, window: &Window, cx: &Context<'_, Self>) -> AnyElement {
        let section_label = self.ui_language().text(self.settings_section.label());
        // A percentage width inside the vertical scroll container measures as
        // indefinite and collapses the row labels, so size the column here.
        let width = bruhs_ui::zoom::css_px(window.viewport_size().width);
        let inset = settings_inset(width);
        // The native traffic lights sit above the GPUI overlay in a windowed Mac app.
        let titlebar_clearance = if cfg!(target_os = "macos") && !window.is_fullscreen() {
            (theme::APP_HEADER_HEIGHT_PX - inset).max(0.)
        } else {
            0.
        };
        let cards_width = settings_cards_width(width);
        div()
            .absolute()
            .top_0()
            .right_0()
            .bottom_0()
            .left_0()
            .occlude()
            .id("desktop-settings-overlay")
            .debug_selector(|| "desktop-settings-overlay".to_string())
            .w(window.viewport_size().width)
            .h(window.viewport_size().height)
            .pt(px(titlebar_clearance))
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .bg(theme::longan_950().opacity(0.65))
            .on_click(cx.listener(|this, _, _, cx| {
                this.close_transient_surfaces();
                cx.notify();
            }))
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
                    .aria_label(self.ui_language().text("Settings"))
                    .debug_selector(|| "desktop-settings-dialog".to_string())
                    .theme_dialog("canvas", cx)
                    .child(self.blur_scrim.backdrop())
                    .on_click(|_, _, cx| cx.stop_propagation())
                    .rounded(px(14.0))
                    .border_1()
                    .border_color(theme::ring_modal())
                    .bg(theme::glass_surface(cx.theme().color_role("canvas").unwrap_or_else(theme::longan_950)))
                    .h(px((bruhs_ui::zoom::css_px(window.viewport_size().height)
                        - 2.0 * inset
                        - titlebar_clearance)
                        .clamp(1.0, 960.0)))
                    .w(px((width - 2.0 * inset).clamp(1.0, 1280.0)))
                    .min_h_0()
                    .min_w_0()
                    .text_color(cx.theme().popover_foreground)
                    .font_family(theme::UI_FONT_FAMILY)
                    .flex()
                    .overflow_hidden()
                    .when(self.settings_visible(), |dialog| dialog.track_focus(&self.modal_start_focus_handle).tab_group())
                    .capture_action(cx.listener(|this, _: &FocusChatInput, window, cx| {
                        if window.has_focused_input(cx) || !this.settings_find_available(window, cx) {
                            cx.propagate();
                        } else {
                            this.focus_settings_search(window, cx);
                            cx.stop_propagation();
                        }
                    }))
                    .capture_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                        let query_focused =
                            this.settings_search.focus_handle(cx).is_focused(window);
                        if !this.settings_find_available(window, cx)
                            || (query_focused && this.settings_search.read(cx).is_composing()) {
                            return;
                        }
                        let mods = event.keystroke.modifiers;
                        let handled = match event.keystroke.key.as_str() {
                            "f" if (mods.platform || mods.control) && !mods.alt && !mods.shift => {
                                this.focus_settings_search(window, cx);
                                true
                            }
                            "/" if !event.keystroke.modifiers.modified()
                                && !bruhs_ui::input::WindowExt::has_focused_input(window, cx) =>
                            {
                                this.focus_settings_search(window, cx);
                                true
                            }
                            "escape" => this.clear_focused_settings_query(window, cx),
                            "up" | "down"
                                if query_focused
                                    && !this.settings_search_results.is_empty()
                                    && !this.settings_search.read(cx).value().is_empty() =>
                            {
                                if let Some(next) = bruhs_models::logic::menu::menu_step(
                                    Some(this.settings_search_index),
                                    this.settings_search_results.len(),
                                    event.keystroke.key.as_str(),
                                ) {
                                    this.settings_search_index = next;
                                    this.settings_nav_scroll.scroll_to_item(next);
                                }
                                cx.notify();
                                true
                            }
                            _ => false,
                        };
                        if handled {
                            window.prevent_default();
                            cx.stop_propagation();
                        }
                    }))
                    .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                        this.trap_modal_tab_focus(event, window, cx)
                    }))
                    .child(self.render_settings_nav(width, window, cx))
                    .when(width >= 768.0 || !self.settings_search_open, |dialog| dialog.child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .min_h_0()
                            .flex()
                            .flex_col()
                            .child(
                                div()
                                    .id("desktop-settings-header")
                                    .debug_selector(|| "desktop-settings-header".to_string())
                                    .h(px(SETTINGS_PAGE_HEADER_HEIGHT_PX))
                                    .pl(px(SETTINGS_PAGE_X_PADDING_PX))
                                    .pr(px(SETTINGS_PAGE_X_PADDING_PX))
                                    .flex_shrink_0()
                                    .border_b_1()
                                    .border_color(
                                        theme::longan_800()
                                            .opacity(WORKSPACE_SIDEBAR_BORDER_OPACITY),
                                    )
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .child(
                                        div()
                                            .text_size(px(SETTINGS_PAGE_TITLE_TEXT_SIZE_PX))
                                            .text_color(theme::rambutan_100())
                                            .child(self.ui_language().text("Settings")),
                                    )
                                    .child(
                                        div()
                                            .text_size(px(SETTINGS_PAGE_CRUMB_TEXT_SIZE_PX))
                                            .font_family(theme::UI_SMALL_FONT_FAMILY)
                                            .text_color(theme::rambutan_600())
                                            .child("/"),
                                    )
                                    .when(self.license_browser.is_some(), |header| header
                                        .child(on_press_view(web_button("license-back-about", WebButtonVariant::Ghost,
                                            WebButtonSize::Small, ButtonState::Enabled).debug_selector(||"license-back-about".into())
                                            .aria_label(self.ui_language().text("Back to About")).child(section_label), cx, |this,window,cx| {
                                                this.return_to_about(window,cx);
                                            }))
                                        .child(div().text_color(theme::rambutan_600()).child("/")))
                                    .child(
                                        div()
                                            .debug_selector(|| "desktop-settings-crumb".to_string())
                                            .flex_1()
                                            .min_w_0()
                                            .truncate()
                                            .text_size(px(SETTINGS_PAGE_CRUMB_TEXT_SIZE_PX))
                                            .font_family(theme::UI_SMALL_FONT_FAMILY)
                                            .text_color(theme::rambutan_400())
                                            .child(if self.license_browser.is_some() {self.ui_language().text("Open source licenses")} else {section_label}),
                                    )
                                    .when(self.settings_section == SettingsSection::General, |el| {
                                        el.child(
                                            web_button(
                                                "desktop-settings-restore",
                                                WebButtonVariant::Ghost,
                                                WebButtonSize::Small,
                                                ButtonState::disabled_if(
                                                    self.changed_device_settings(cx).is_empty(),
                                                ),
                                            )
                                            .debug_selector(|| {
                                                "desktop-settings-restore".to_owned()
                                            })
                                            .aria_label(self.ui_language().text("Restore device defaults"))
                                            .child(
                                                svg()
                                                    .path(assets::ROTATE_CCW_ICON_PATH)
                                                    .size(px(14.))
                                                    .text_color(theme::rambutan_400()),
                                            )
                                            .when(width >= 768., |el| {
                                                el.child(self.ui_language().text("Restore device defaults"))
                                            })
                                            .on_click(
                                                cx.listener(|this, _, window, cx| {
                                                    this.confirm_restore_device_settings(window, cx)
                                                }),
                                            ),
                                        )
                                    })
                                    .child(
                                        div()
                                            .id("desktop-settings-close")
                                            .debug_selector(|| "desktop-settings-close".to_string())
                                            .flex_shrink_0()
                                            .size(px(SETTINGS_CLOSE_BUTTON_SIZE_PX))
                                            .rounded(px(SETTINGS_CONTROL_RADIUS_PX))
                                            .flex()
                                            .items_center()
                                            .justify_center()
                                            .text_color(theme::rambutan_500())
                                            .tab_index(0)
                                            .role(Role::Button)
                                            .aria_label(self.ui_language().text(SETTINGS_CLOSE_LABEL))
                                            .border_1()
                                            .border_color(gpui::transparent_black())
                                            .focus_visible(|el| {
                                                el.border_color(theme::persimmon_400())
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
                                            .map(|el| {
                                                on_press_view(el, cx, |this, _window, cx| {
                                                    this.close_transient_surfaces();
                                                    cx.notify();
                                                })
                                            })
                                            .child(
                                                svg()
                                                    .path(assets::CLOSE_ICON_PATH)
                                                    .size(px(SETTINGS_PAGE_CLOSE_ICON_PX))
                                                    .text_color(theme::rambutan_500()),
                                            ),
                                    ),
                            )
                            .child(
                                div().flex_1().min_h_0().flex().child(
                                    div()
                                        .id("desktop-settings-content")
                                        .debug_selector(|| "desktop-settings-content".to_string())
                                        .relative()
                                        .flex_1()
                                        .min_w_0()
                                        .min_h_0()
                                        .when_some(self.license_browser.as_ref(), |body, browser| body.child(browser.clone()))
                                        .when(self.license_browser.is_none(), |body| body.child(
                                            div()
                                                .id("desktop-settings-scroll")
                                                .relative()
                                                .size_full()
                                                .overflow_y_scroll()
                                                .track_scroll(&self.settings_scroll)
                                                .px(px(SETTINGS_BODY_X_PADDING_PX))
                                                .py(px(SETTINGS_BODY_Y_PADDING_PX))
                                                .flex()
                                                .flex_col()
                                                .items_center()
                                                .gap(px(SETTINGS_BODY_GAP_PX))
                                                .child(self.render_settings_section(
                                                    self.settings_section,
                                                    cards_width,
                                                    cx,
                                                ))
                                                .vertical_scrollbar(&self.settings_scroll),
                                        )),
                                ),
                            ),
                    ))
                    .when(self.settings_visible(), |dialog| dialog
                        .when(self.settings_has_notice(cx), |el| el.children(self.render_hub_notice(window, cx)))
                        .children(self.floating_update_progress(cx))
                        .child(self.render_modal_focus_sentinel())),
            )
            .when(self.settings_section == SettingsSection::ScheduledTasks
                && self.scheduled_panel().read(cx).scheduled_editor_open(), |layer| {
                    layer.child(crate::views::reactive_automations::ScheduledTaskDialog {
                        panel: self.scheduled_panel().clone(),
                        host_field: self.scheduled_host_field(cx),
                        projects: self.scheduled_editor_projects(cx),
                        project_models: self.scheduled_editor_models(),
                        default_model: self.scheduled_default_model(),
                        favorites: self.favorite_models.clone(),
                        toggle_favorite: {
                            let ws = cx.weak_entity();
                            Rc::new(move |id, cx| { let _ = ws.update(cx, |ws, cx| ws.toggle_favorite_model(id, cx)); })
                        },
                    })
                })
            .when(self.secret_provider_dialog_visible(cx), |layer| {
                layer.child(div().absolute().inset_0()
                    .on_key_down(cx.listener(Self::trap_modal_tab_focus))
                    .child(secret_source_settings::SecretProviderDialog(self.vault_settings.read(cx).secret_sources())))
            })
            .children(self.render_generator_dialog(window,cx))
            .children(self.render_provider_editor(window,cx))
            .children(self.render_hub_dialog(window,cx))
            .into_any_element()
    }

    pub(super) fn settings_heading(&self, title: &'static str, description: &'static str) -> Div {
        div()
            .px(px(SETTINGS_ROW_X_PADDING_PX + 1.0))
            .flex()
            .flex_col()
            .child(
                div()
                    .text_size(px(SETTINGS_ROW_TITLE_TEXT_SIZE_PX))
                    .text_color(theme::rambutan_100())
                    .child(self.ui_language().text(title)),
            )
            .when(!description.is_empty(), |el| {
                el.child(
                    div()
                        .mt(px(2.0))
                        .text_size(px(SETTINGS_META_TEXT_SIZE_PX))
                        .font_family(theme::UI_SMALL_FONT_FAMILY)
                        .text_color(theme::rambutan_500())
                        .child(self.ui_language().text(description)),
                )
            })
    }
}

#[cfg(test)]
mod ui_audit_tests {
    use super::{
        settings_rows_stack, SettingsSection, Workspace, SETTINGS_TITLE_PROJECT_DROPDOWN_ID,
    };
    use crate::{app_state::DesktopState, theme};
    use ::core::prelude::v1::test;
    use gpui::{px, size, AppContext, TestAppContext};
    use std::{cell::RefCell, rc::Rc};

    #[test]
    fn settings_rows_stack_when_labels_and_controls_cannot_fit() {
        // Switching from the icon rail to the full navigation reduces the row width.
        for (viewport, stacked) in [
            (640., true), (697., true), (698., false), (767., false),
            (768., true), (913., true), (914., false), (1920., false),
        ] {
            assert_eq!(settings_rows_stack(viewport), stacked, "{viewport}px viewport");
        }
    }

    #[gpui::test]
    fn project_title_instructions_pick_a_project_from_a_list_like_web(cx: &mut TestAppContext) {
        cx.update(|cx| {
            bruhs_ui::init(cx);
            theme::apply_bruhs_theme(cx);
            cx.set_global(DesktopState::new().expect("desktop state"));
        });
        let capture = Rc::new(RefCell::new(None));
        let captured = capture.clone();
        let (_, cx) = cx.add_window_view(|window, cx| {
            let workspace = cx.new(|cx| Workspace::new(window, cx));
            *captured.borrow_mut() = Some(workspace.clone());
            bruhs_ui::Root::new(workspace, window, cx)
        });
        let workspace = capture.borrow().clone().expect("workspace");
        workspace.update_in(cx, |workspace, window, cx| {
            let thread = workspace.threads.first_mut().expect("a thread");
            thread.project_name = "Alpha".into();
            assert_eq!(workspace.thread_project_names(), ["Alpha"]);
            workspace
                .title_instructions_by_project
                .insert("Alpha".into(), "prefix: A".into());
            workspace.open_settings(&super::OpenSettings, window, cx);
            workspace.set_settings_section(SettingsSection::Threads, cx);
            assert!(workspace.title_project.is_empty(), "nothing chosen yet");
            // Choosing a project loads its lines into the field.
            workspace.select_settings_option(
                SETTINGS_TITLE_PROJECT_DROPDOWN_ID,
                "Alpha",
                window,
                cx,
            );
            assert_eq!(workspace.title_project, "Alpha");
            let field = workspace.title_project_instructions_input.clone();
            assert_eq!(field.read(cx).value().as_ref(), "prefix: A");
            field.update(cx, |input, cx| input.set_value("", window, cx));
        });
        // The field appears once a project is chosen; type into it.
        cx.update(|window, _| window.refresh());
        cx.run_until_parked();
        workspace.update_in(cx, |workspace, window, cx| {
            let field = workspace.title_project_instructions_input.clone();
            field.update(cx, |input, cx| input.focus(window, cx));
        });
        cx.update(|window, _| window.refresh());
        cx.run_until_parked();
        cx.simulate_input("max 20");
        cx.run_until_parked();
        workspace.update_in(cx, |workspace, window, cx| {
            assert_eq!(
                workspace
                    .title_instructions_by_project
                    .get("Alpha")
                    .map(String::as_str),
                Some("max 20")
            );
            workspace.select_settings_option(SETTINGS_TITLE_PROJECT_DROPDOWN_ID, "", window, cx);
            assert!(workspace.title_project.is_empty());
        });
    }

    #[gpui::test]
    fn settings_search_opens_the_selected_row_from_the_keyboard(cx: &mut TestAppContext) {
        cx.update(|cx| {
            bruhs_ui::init(cx);
            theme::apply_bruhs_theme(cx);
            cx.set_global(DesktopState::new().expect("desktop state"));
        });
        let (workspace, cx) = cx.add_window_view(Workspace::new);
        for width in [1100., 650.] {
            cx.simulate_resize(size(px(width), px(700.)));
            workspace.update_in(cx, |workspace, window, cx| {
                workspace.open_settings(&super::OpenSettings, window, cx);
                workspace.set_settings_section(SettingsSection::General, cx);
            });
            cx.run_until_parked();
            let general = cx.debug_bounds(SettingsSection::General.selector()).expect("General tab");
            cx.simulate_click(general.center(), gpui::Modifiers::default());
            cx.simulate_keystrokes("down end down");
            workspace.update_in(cx, |ws, window, _| {
                assert_eq!(ws.settings_section, SettingsSection::General);
                assert!(ws.settings_nav_focus.last().unwrap().is_focused(window));
            });
            cx.simulate_keystrokes("home up down enter");
            cx.run_until_parked();
            workspace.read_with(cx, |ws, _| assert_eq!(ws.settings_section, SettingsSection::Appearance));
            if width < 768. {
                let search = cx.debug_bounds("desktop-settings-show-search").expect("compact search button");
                cx.simulate_click(search.center(), gpui::Modifiers::default());
            } else {
                cx.simulate_keystrokes("/");
            }
            cx.run_until_parked();
            let nav = cx.debug_bounds("desktop-settings-nav").expect("search navigation");
            let dialog = cx.debug_bounds("desktop-settings-dialog").expect("settings dialog");
            if width < 768. {
                assert!(nav.size.width >= dialog.size.width - px(2.));
                assert!(cx.debug_bounds("desktop-settings-content").is_none());
            } else {
                assert!(nav.size.width < dialog.size.width / 2.);
                assert!(cx.debug_bounds("desktop-settings-content").is_some());
                let search_header = cx
                    .debug_bounds("desktop-settings-search-header")
                    .expect("search header");
                let pane_header = cx
                    .debug_bounds("desktop-settings-header")
                    .expect("settings header");
                let links = cx
                    .debug_bounds("desktop-settings-links")
                    .expect("settings links");
                assert!((f32::from(search_header.bottom()) - f32::from(pane_header.bottom())).abs() <= 1.0);
                assert!(links.top() >= search_header.bottom() - px(1.0));
            }
            cx.simulate_input("project");
            cx.run_until_parked();
            let count = workspace.read_with(cx, |workspace, _| workspace.settings_search_results.len());
            assert!(count > 1);
            cx.simulate_keystrokes("up");
            cx.run_until_parked();
            workspace.read_with(cx, |workspace, _| assert_eq!(workspace.settings_search_index, count - 1));
            cx.simulate_keystrokes("down");
            cx.run_until_parked();
            workspace.read_with(cx, |workspace, _| assert_eq!(workspace.settings_search_index, 0));
            cx.simulate_keystrokes("escape");
            cx.run_until_parked();
            workspace.read_with(cx, |workspace, cx| {
                assert!(workspace.settings_visible());
                assert!(workspace.settings_search.read(cx).value().is_empty());
            });
            cx.simulate_input("no such setting exists");
            cx.run_until_parked();
            assert!(cx.debug_bounds("desktop-settings-no-results").is_some());
            let clear = cx.debug_bounds("settings-clear-search").expect("clear search button");
            cx.simulate_click(clear.center(), gpui::Modifiers::default());
            cx.run_until_parked();
            cx.simulate_input("project title instructions");
            cx.run_until_parked();
            assert!(
                cx.debug_bounds("desktop-settings-hit-0").is_some(),
                "missing search result at {width}"
            );
            cx.simulate_keystrokes("enter");
            cx.run_until_parked();
            let row = cx
                .debug_bounds("desktop-settings-row-project-title")
                .expect("selected setting");
            let viewport = cx
                .debug_bounds("desktop-settings-content")
                .expect("viewport");
            assert!(
                row.top() < viewport.bottom() && row.bottom() > viewport.top(),
                "selected row {row:?} should be visible in {viewport:?}"
            );
            workspace.read_with(cx, |workspace, _| {
                assert_eq!(workspace.settings_section, SettingsSection::Threads)
            });
        }
    }

    #[gpui::test]
    fn settings_search_finds_current_shortcuts_and_opens_unassigned_commands(cx: &mut TestAppContext) {
        cx.update(|cx| {
            bruhs_ui::init(cx);
            theme::apply_bruhs_theme(cx);
            cx.set_global(DesktopState::new().expect("desktop state"));
        });
        let (workspace, cx) = cx.add_window_view(Workspace::new);
        workspace.update_in(cx, |ws, window, cx| {
            ws.open_settings(&super::OpenSettings, window, cx);
            ws.keybindings.retain(|rule| rule.command != "chat.new");
            let palette = ws.keybindings.iter_mut().find(|rule| rule.command == "command.palette").unwrap();
            palette.key = "mod+alt+f12".into();
        });
        cx.run_until_parked();
        cx.simulate_keystrokes("cmd-f");
        cx.run_until_parked();
        cx.simulate_input("mod+alt+f12");
        cx.run_until_parked();
        workspace.read_with(cx, |ws, _| {
            assert_eq!(ws.settings_search_results[0].command.as_deref(), Some("command.palette"));
        });
        cx.simulate_keystrokes("ctrl-f");
        cx.run_until_parked();
        cx.simulate_input("chát.new");
        cx.run_until_parked();
        workspace.read_with(cx, |ws, cx| {
            assert_eq!(ws.settings_search.read(cx).value().as_ref(), "chát.new");
            assert_eq!(ws.settings_search_results[0].command.as_deref(), Some("chat.new"));
        });
        cx.simulate_keystrokes("enter");
        cx.run_until_parked();
        workspace.read_with(cx, |ws, cx| {
            assert_eq!(ws.settings_section, SettingsSection::Keyboard);
            assert_eq!(ws.keybinding_filter.read(cx).value().as_ref(), "chat.new");
            assert_eq!(ws.keybinding_draft.as_ref().and_then(|draft| draft.command.as_deref()), Some("chat.new"));
            assert!(ws.settings_keybindings().iter().all(|rule| rule.command != "chat.new"));
        });
    }

    #[gpui::test]
    fn desktop_settings_minimum_window_keeps_visible_stack_and_jump(cx: &mut TestAppContext) {
        cx.update(|cx| {
            bruhs_ui::init(cx);
            theme::apply_bruhs_theme(cx);
            cx.set_global(DesktopState::new().expect("desktop state"));
        });
        let capture = Rc::new(RefCell::new(None));
        let captured = capture.clone();
        let (_, cx) = cx.add_window_view(|window, cx| {
            let workspace = cx.new(|cx| Workspace::new(window, cx));
            *captured.borrow_mut() = Some(workspace.clone());
            bruhs_ui::Root::new(workspace, window, cx)
        });
        let workspace = capture.borrow().clone().expect("workspace");
        cx.simulate_resize(size(px(641.0), px(481.0)));
        workspace.update_in(cx, |workspace, window, cx| {
            workspace.open_settings(&super::OpenSettings, window, cx);
            workspace.set_settings_section(SettingsSection::Meetings, cx);
        });
        cx.run_until_parked();
        let overlay = cx
            .debug_bounds("desktop-settings-overlay")
            .expect("settings overlay");
        let dialog = cx
            .debug_bounds("desktop-settings-dialog")
            .expect("settings dialog");
        assert_eq!(overlay.size, size(px(641.0), px(481.0)));
        assert!(dialog.left() > overlay.left() && dialog.right() < overlay.right());
        let label = cx
            .debug_bounds("desktop-settings-row-transcription-provider-label")
            .expect("provider label");
        let control = cx
            .debug_bounds("desktop-settings-row-transcription-provider-control")
            .expect("provider control");
        assert!(
            label.size.width >= px(240.0),
            "label must not become a glyph column"
        );
        assert!(control.top() >= label.bottom(), "narrow rows must stack");
        assert!(
            label.top() >= px(52.0) && label.top() < px(481.0),
            "section jump must bring its label into view: {label:?}"
        );

        for (width, stacked) in [(913., true), (914., false), (767., false), (768., true)] {
            cx.simulate_resize(size(px(width), px(850.)));
            cx.run_until_parked();
            let label = cx.debug_bounds("desktop-settings-row-transcription-provider-label").unwrap();
            let control = cx.debug_bounds("desktop-transcription-provider").unwrap();
            let row = cx.debug_bounds("desktop-settings-row-transcription-provider").unwrap();
            assert_eq!(control.size.width, px(320.));
            assert!(control.right() <= row.right());
            if stacked {
                assert_eq!(control.left(), label.left());
                assert_eq!(control.top() - label.bottom(), px(12.));
            } else {
                assert_eq!(control.left() - label.right(), px(32.));
                assert!((control.center().y - label.center().y).abs() <= px(1.));
            }
            if width == 913. {
                cx.simulate_click(control.center(), gpui::Modifiers::default());
                cx.run_until_parked();
                assert!(cx.debug_bounds("desktop-transcription-provider-list").is_some());
            }
            if width == 914. {
                let menu = cx.debug_bounds("desktop-transcription-provider-list").unwrap();
                assert!((menu.left() - control.left()).abs() <= px(1.));
                cx.simulate_keystrokes("escape");
                cx.run_until_parked();
                assert!(cx.debug_bounds("desktop-transcription-provider-list").is_none());
                assert!(cx.debug_bounds("desktop-settings-dialog").is_some());
            }
        }

        cx.simulate_resize(size(px(641.), px(950.)));
        workspace.update_in(cx, |ws, window, cx| ws.set_zoom(2.0, window, cx));
        cx.run_until_parked();
        let control = cx.debug_bounds("desktop-transcription-provider").unwrap();
        let row = cx.debug_bounds("desktop-settings-row-transcription-provider").unwrap();
        let pane = cx.debug_bounds("desktop-settings-content").unwrap();
        let close = cx.debug_bounds("desktop-settings-close").unwrap();
        assert!(close.left() >= pane.left() && close.right() <= pane.right());
        assert!(row.left() >= pane.left() && row.right() <= pane.right());
        assert!(control.left() >= row.left() && control.right() <= row.right());
        assert!(control.size.width < px(320. * 2.), "the wide selector shrinks with its column");
        workspace.update_in(cx, |ws, window, cx| ws.set_zoom(1.0, window, cx));
    }
}

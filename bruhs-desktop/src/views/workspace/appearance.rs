use super::*;
use bruhs_ui::theme::ThemeFamily;
mod collection;
mod color;
mod palette;
mod stylesheet;
mod background;
pub(crate) use background::{layer as background_layer, layer_with_radius as rounded_background_layer};
mod color_families;
mod community;
mod editor;
mod import;
mod library;
mod picker;
mod vscode;
pub(super) use collection::{CollectionNavigation, VariantRemoval};
pub(super) use editor::ThemeEditor;
pub(super) use library::ThemeLibrary;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(super) enum ColorScheme {
    System,
    Light,
    #[default]
    Dark,
}
impl ColorScheme {
    pub(super) fn id(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::Light => "light",
            Self::Dark => "dark",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub(super) struct DeviceAppearance {
    pub(super) language: language::Language,
    pub(super) clock: crate::clock_format::ClockFormat,
    pub(super) scheme: ColorScheme,
    pub(super) light_theme: ThemeFamily,
    pub(super) dark_theme: ThemeFamily,
    pub(super) contrast: u8,
    pub(super) render_math: bool,
    pub(super) effort_label: bool,
    pub(super) thread_details: thread_details::Preferences,
    pub(super) glass_opacity: u8,
    pub(super) panel_duration_ms: u16,
    pub(super) environment_mark: environment_mark::Mark,
}
impl Default for DeviceAppearance {
    fn default() -> Self {
        Self {
            language: Default::default(),
            clock: Default::default(),
            scheme: ColorScheme::Dark,
            light_theme: ThemeFamily::Bruhs,
            dark_theme: ThemeFamily::Bruhs,
            contrast: 100,
            render_math: true,
            effort_label: true,
            thread_details: Default::default(),
            glass_opacity: 100,
            panel_duration_ms: 0,
            environment_mark: Default::default(),
        }
    }
}
impl DeviceAppearance {
    pub(super) fn normalized(mut self) -> Self {
        self.panel_duration_ms = self.panel_duration_ms.min(400);
        self.glass_opacity = self.glass_opacity.clamp(40, 100);
        self.contrast = self.contrast.clamp(50, 200);
        self
    }
}

impl Workspace {
    fn selected_custom_theme(&self, mode: theme::ThemeMode) -> Option<&library::InstalledTheme> {
        let id = self.custom_themes.selection(mode)?;
        self.custom_themes.find(id).map(Arc::as_ref)
    }

    fn selected_custom_palette(&self, mode: theme::ThemeMode) -> Option<&library::InstalledPalette> {
        self.selected_custom_theme(mode)?.installed_palette(mode)
    }

    fn selected_palette(&self, mode: theme::ThemeMode) -> &theme::Palette {
        if let Some(custom) = self.selected_custom_palette(mode) {
            return &custom.palette;
        }
        match mode {
            theme::ThemeMode::Light => self.appearance.light_theme,
            theme::ThemeMode::Dark => self.appearance.dark_theme,
        }
        .palette(mode)
    }
    pub(super) fn apply_device_appearance(&self, window: &mut Window, cx: &mut Context<'_, Self>) {
        use bruhs_ui::ThemeMode;
        self.appearance.clock.apply();
        let mode = match self.appearance.scheme {
            ColorScheme::Light => ThemeMode::Light,
            ColorScheme::Dark => ThemeMode::Dark,
            ColorScheme::System => match window.appearance() {
                WindowAppearance::Light | WindowAppearance::VibrantLight => ThemeMode::Light,
                WindowAppearance::Dark | WindowAppearance::VibrantDark => ThemeMode::Dark,
            },
        };
        if let Some(custom) = self.selected_custom_palette(mode) {
            background::select(
                self.custom_themes.selection(mode),
                custom.images.clone(),
                window,
                cx,
            );
            bruhs_ui::Theme::use_color_roles(
                custom.palette,
                Some(custom.roles.clone()),
                mode,
                Some(window),
                cx,
            );
        } else {
            background::select(None, Default::default(), window, cx);
            bruhs_ui::Theme::use_palette(*self.selected_palette(mode), mode, Some(window), cx);
        }
        self.fonts.apply(cx);
        bruhs_ui::theme::set_glass_opacity(self.appearance.glass_opacity, cx);
        bruhs_ui::theme::set_contrast(self.appearance.contrast, cx);
    }
}

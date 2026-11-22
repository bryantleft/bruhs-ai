//! The device language preference in General settings.
use super::*;
use crate::views::language::LANGUAGES;
pub(super) use crate::views::language::{Language, SEARCH_TERMS};
use bruhs_ui::press::on_press_view;
use bruhs_ui::zoom::px;

impl Workspace {
    pub(super) fn ui_language(&self) -> Language {
        self.appearance.language.effective(self.system_language)
    }
    pub(super) fn set_ui_language(
        &mut self,
        value: &str,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let Some(&(language, _, _)) = LANGUAGES.iter().find(|(_, id, _)| *id == value) else {
            return;
        };
        self.appearance.language = language;
        self.system_language = Language::system();
        self.refresh_language_ui(window, cx);
        self.persist_and_notify(cx);
    }
    pub(super) fn refresh_language_ui(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let language = self.ui_language();
        self.sidebar_search.update(cx, |field, cx| {
            field.set_placeholder(
                language.text(sidebar_ui::SIDEBAR_SEARCH_PLACEHOLDER),
                window,
                cx,
            )
        });
        let prompt = language.text("Search settings");
        self.settings_search
            .update(cx, |input, cx| input.set_placeholder(prompt, window, cx));
        self.settings_search_catalog.clear();
        self.refresh_settings_search(cx);
    }
    pub(super) fn language_row(&self, cx: &Context<'_, Self>) -> AnyElement {
        let language = self.appearance.language;
        let current = LANGUAGES.iter().find(|(v, _, _)| *v == language).unwrap().1;
        let choices: Vec<_> = LANGUAGES
            .iter()
            .map(|(_, value, label)| (*value, *label))
            .collect();
        let reset = (language != Language::default()).then(|| {
            let label = self.ui_language().text("Reset language");
            let button = web_button(
                "reset-language",
                WebButtonVariant::Ghost,
                WebButtonSize::IconSmall,
                ButtonState::Enabled,
            )
            .debug_selector(|| "reset-language".into())
            .aria_label(label)
            .tooltip(move |w, cx| Tooltip::new(label).build(w, cx))
            .child(
                svg()
                    .path(assets::ROTATE_CCW_ICON_PATH)
                    .size(px(14.))
                    .text_color(theme::rambutan_400()),
            );
            on_press_view(button, cx, |ws, w, cx| ws.set_ui_language("en", w, cx))
                .into_any_element()
        });
        self.settings_row_with_reset(
            "desktop-settings-row-language",
            "Language",
            "System default follows your device’s language.",
            None,
            false,
            Some(settings_ui::control_column(self.choice_control(
                "settings-language",
                "Language",
                current,
                &choices,
                cx,
            ))),
            reset,
        )
    }
}

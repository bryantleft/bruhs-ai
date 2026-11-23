//! Language negotiation and the bundled interface message catalog.
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, sync::LazyLock};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum Language {
    #[default]
    En,
    De,
    Es,
    Fr,
    Ja,
    Ko,
    #[serde(rename = "pt-BR")]
    PtBr,
    Ru,
    Zh,
    #[serde(other)]
    System,
}

pub(crate) const SEARCH_TERMS: &str = "language locale English Deutsch Sprache español português idioma français langue русский язык 日本語 言語 한국어 언어 中文 语言";

pub(crate) const LANGUAGES: &[(Language, &str, &str)] = &[
    (Language::System, "system", "System default"),
    (Language::En, "en", "English"),
    (Language::De, "de", "Deutsch"),
    (Language::Es, "es", "Español"),
    (Language::Fr, "fr", "Français"),
    (Language::Ja, "ja", "日本語"),
    (Language::Ko, "ko", "한국어"),
    (Language::PtBr, "pt-BR", "Português (Brasil)"),
    (Language::Ru, "ru", "Русский"),
    (Language::Zh, "zh", "中文（简体）"),
];

// Each row contains the English source followed by de, es, fr, ja, ko, pt-BR, ru and zh.
static MESSAGES: LazyLock<HashMap<&'static str, [&'static str; 8]>> = LazyLock::new(|| {
    include_str!("language/settings.tsv")
        .lines()
        .map(|line| {
            let mut cells = line.split('\t');
            let source = cells.next().expect("non-empty translation row");
            let values = std::array::from_fn(|_| cells.next().expect("missing translation"));
            assert!(cells.next().is_none(), "extra translation for {source}");
            (source, values)
        })
        .collect()
});

impl Language {
    pub(crate) fn system() -> Self {
        Self::negotiate(sys_locale::get_locales())
    }
    fn negotiate(tags: impl IntoIterator<Item = impl AsRef<str>>) -> Self {
        for tag in tags {
            let tag = tag.as_ref().trim().replace('_', "-").to_ascii_lowercase();
            let mut parts = tag.split('-');
            let base = parts.next().unwrap_or_default();
            if base == "zh"
                && parts
                    .next()
                    .is_some_and(|part| matches!(part, "hant" | "tw" | "hk" | "mo"))
            {
                continue;
            }
            if let Some(&(lang, _, _)) = LANGUAGES
                .iter()
                .skip(1)
                .find(|(_, id, _)| id.eq_ignore_ascii_case(&tag) || *id == base)
            {
                return lang;
            }
        }
        Self::En
    }
    pub(crate) fn effective(self, system: Self) -> Self {
        if self == Self::System { system } else { self }
    }
    pub(crate) fn translation(self, key: &str) -> Option<&'static str> {
        let i = match self {
            Self::De => 0,
            Self::Es => 1,
            Self::Fr => 2,
            Self::Ja => 3,
            Self::Ko => 4,
            Self::PtBr => 5,
            Self::Ru => 6,
            Self::Zh => 7,
            _ => return None,
        };
        Some(MESSAGES.get(key)?[i])
    }
    pub(crate) fn text(self, key: &'static str) -> &'static str {
        self.translation(key).unwrap_or(key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn system_language_uses_the_first_supported_device_preference() {
        assert_eq!(Language::negotiate(["zh-Hant-TW", "de-CH"]), Language::De);
        assert_eq!(Language::negotiate(["pt_BR", "en-US"]), Language::PtBr);
        assert_eq!(Language::negotiate(["zh-Hans-CN"]), Language::Zh);
        assert_eq!(Language::negotiate(["it", "nl"]), Language::En);
    }
    #[test]
    fn saved_language_tags_match_the_picker_and_legacy_devices_start_in_english() {
        assert_eq!(Language::default(), Language::En);
        for &(language, tag, _) in LANGUAGES {
            assert_eq!(serde_json::to_value(language).unwrap(), tag);
            assert_eq!(
                serde_json::from_value::<Language>(tag.into()).unwrap(),
                language
            );
        }
        assert_eq!(Language::System.effective(Language::Fr), Language::Fr);
        assert_eq!(Language::En.effective(Language::Fr), Language::En);
        assert_eq!(Language::Zh.text("Language"), "语言");
        assert_eq!(Language::Ja.text("Language"), "言語");
    }

    #[test]
    fn language_catalog_covers_each_locale_without_duplicate_sources() {
        let rows = include_str!("language/settings.tsv").lines().count();
        assert_eq!(rows, MESSAGES.len());
        for (source, cells) in MESSAGES.iter() {
            assert!(!source.is_empty() && cells.iter().all(|c| !c.trim().is_empty()));
        }
    }
}

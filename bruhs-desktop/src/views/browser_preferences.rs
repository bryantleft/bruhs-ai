//! Device preferences for addresses typed in the preview and Computer browser.
use gpui::{App, Global};
use reqwest::Url;
use serde::{Deserialize, Serialize};

const DEFAULT_SEARCH: &str = "https://duckduckgo.com/?q=%s";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum BrowserAppearance {
    #[default]
    System,
    Light,
    Dark,
}

impl BrowserAppearance {
    pub fn key(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::Light => "light",
            Self::Dark => "dark",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct BrowserViewport {
    pub mode: String,
    pub width: u16,
    pub height: u16,
}
impl Default for BrowserViewport {
    fn default() -> Self {
        Self {
            mode: "fill".into(),
            width: 1280,
            height: 800,
        }
    }
}
impl BrowserViewport {
    pub const PRESETS: &[(&str, &str, u16, u16)] = &[
        ("iphone-se", "iPhone SE", 375, 667),
        ("iphone-xr", "iPhone XR", 414, 896),
        ("iphone-12-pro", "iPhone 12 Pro", 390, 844),
        ("iphone-14-pro-max", "iPhone 14 Pro Max", 430, 932),
        ("pixel-7", "Pixel 7", 412, 915),
        ("samsung-galaxy-s8-plus", "Samsung Galaxy S8+", 360, 740),
        (
            "samsung-galaxy-s20-ultra",
            "Samsung Galaxy S20 Ultra",
            412,
            915,
        ),
        ("ipad-mini", "iPad Mini", 768, 1024),
        ("ipad-air", "iPad Air", 820, 1180),
        ("ipad-pro", "iPad Pro", 1024, 1366),
        ("surface-pro-7", "Surface Pro 7", 912, 1368),
        ("surface-duo", "Surface Duo", 540, 720),
        ("galaxy-z-fold-5", "Galaxy Z Fold 5", 344, 882),
        ("asus-zenbook-fold", "Asus Zenbook Fold", 853, 1280),
        ("samsung-galaxy-a51-71", "Samsung Galaxy A51/71", 412, 914),
        ("nest-hub", "Nest Hub", 1024, 600),
        ("nest-hub-max", "Nest Hub Max", 1280, 800),
    ];
    pub fn dimensions(&self) -> Option<(u16, u16)> {
        (self.mode == "responsive" || Self::PRESETS.iter().any(|p| p.0 == self.mode))
            .then(|| (self.width.clamp(240, 7680), self.height.clamp(240, 7680)))
    }
    pub fn select(&mut self, mode: &str) {
        if let Some((_, _, width, height)) = Self::PRESETS.iter().find(|p| p.0 == mode) {
            self.width = *width;
            self.height = *height;
        } else if !matches!(mode, "fill" | "responsive") {
            return;
        }
        self.mode = mode.to_owned();
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct BrowserProfile {
    pub id: uuid::Uuid,
    pub name: String,
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct PagePreferences {
    pub appearance: BrowserAppearance,
    pub zoom_percent: Option<u16>,
    pub viewport: Option<(u16, u16)>,
    pub profile: Option<uuid::Uuid>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct BrowserPreferences {
    pub open_links_in_app: bool,
    pub engine: String,
    pub custom_url: String,
    pub zoom_percent: u16,
    pub viewport: BrowserViewport,
    pub appearance: BrowserAppearance,
    pub profiles: Vec<BrowserProfile>,
    pub default_profile: Option<uuid::Uuid>,
}

impl Default for BrowserPreferences {
    fn default() -> Self {
        Self {
            open_links_in_app: false,
            engine: "duckduckgo".into(),
            custom_url: String::new(),
            zoom_percent: 100,
            viewport: Default::default(),
            appearance: Default::default(),
            profiles: Vec::new(),
            default_profile: None,
        }
    }
}

impl Global for BrowserPreferences {}

impl BrowserPreferences {
    pub fn restore_defaults(&mut self) {
        let profiles = std::mem::take(&mut self.profiles);
        *self = Self {
            profiles,
            ..Default::default()
        };
    }

    pub fn has_changed_settings(&self) -> bool {
        let mut defaults = self.clone();
        defaults.restore_defaults();
        self != &defaults
    }

    pub fn page_settings(&self) -> PagePreferences {
        PagePreferences {
            appearance: self.appearance,
            viewport: self.viewport.dimensions(),
            zoom_percent: Some(self.zoom_percent.clamp(50, 200)),
            profile: self
                .default_profile
                .filter(|id| self.profiles.iter().any(|profile| profile.id == *id)),
        }
    }

    pub fn add_profile(&mut self, name: &str) -> Result<uuid::Uuid, &'static str> {
        let name = name.trim();
        if name.is_empty() || name.chars().count() > 80 || name.chars().any(char::is_control) {
            return Err("Enter a profile name of 1 to 80 characters.");
        }
        if self
            .profiles
            .iter()
            .any(|profile| profile.name.eq_ignore_ascii_case(name))
            || name.eq_ignore_ascii_case("Default")
        {
            return Err("A browser profile already has this name.");
        }
        let id = uuid::Uuid::new_v4();
        self.profiles.push(BrowserProfile {
            id,
            name: name.into(),
        });
        Ok(id)
    }

    pub const ENGINES: &[(&str, &str)] = &[
        ("duckduckgo", "DuckDuckGo"),
        ("google", "Google"),
        ("bing", "Bing"),
        ("brave", "Brave Search"),
        ("kagi", "Kagi"),
        ("ecosia", "Ecosia"),
        ("custom", "Custom"),
    ];
    pub const ZOOM: &[(&str, &str)] = &[
        ("50", "50%"),
        ("75", "75%"),
        ("90", "90%"),
        ("100", "100%"),
        ("110", "110%"),
        ("125", "125%"),
        ("150", "150%"),
        ("175", "175%"),
        ("200", "200%"),
    ];

    pub fn get(cx: &App) -> Self {
        cx.try_global::<Self>().cloned().unwrap_or_default()
    }

    pub fn custom_url_valid(&self) -> bool {
        let Ok(url) = Url::parse(&self.custom_url.replace("%s", "QUERY")) else {
            return false;
        };
        matches!(url.scheme(), "http" | "https")
            && url.host().is_some()
            && url.username().is_empty()
            && url.password().is_none()
            && self.custom_url.split_once("://").is_some_and(|(_, rest)| {
                let authority = rest.split(['/', '?', '#']).next().unwrap_or(rest);
                !authority.contains("%s") && rest[authority.len()..].contains("%s")
            })
    }

    /// Resolve a human-entered address. Programmatic navigation bypasses search.
    pub fn address(&self, raw: &str) -> Option<String> {
        let text = raw.trim();
        if text.is_empty() {
            return None;
        }
        if text.bytes().all(|c| c.is_ascii_digit()) && text.parse::<u16>().is_ok() {
            return Some(format!("http://localhost:{text}"));
        }
        // A pasted non-web URI can contain private data. Keep it out of search queries.
        if let Some((scheme, rest)) = text.split_once(':') {
            let valid_scheme = scheme.starts_with(|c: char| c.is_ascii_alphabetic())
                && scheme
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || b"+.-".contains(&c));
            let port = rest
                .split(['/', '?', '#'])
                .next()
                .is_some_and(|p| p.parse::<u16>().is_ok());
            if valid_scheme && !port && !rest.starts_with("//") {
                return None;
            }
            if valid_scheme && rest.starts_with("//") {
                let url = Url::parse(text).ok()?;
                return (matches!(url.scheme(), "http" | "https") && url.host().is_some())
                    .then(|| url.to_string());
            }
        }
        if !text.chars().any(char::is_whitespace) {
            let host = text.split(['/', '?', '#']).next().unwrap_or(text);
            if host == "localhost"
                || host.starts_with("localhost:")
                || host.starts_with('[')
                || host.contains('.')
                || host
                    .rsplit_once(':')
                    .is_some_and(|(_, p)| p.parse::<u16>().is_ok())
            {
                let url = format!(
                    "{}://{text}",
                    if host == "localhost"
                        || host.contains(':')
                        || host.parse::<std::net::IpAddr>().is_ok()
                    {
                        "http"
                    } else {
                        "https"
                    }
                );
                return Url::parse(&url).ok().map(|_| url);
            }
        }
        let template = match self.engine.as_str() {
            "google" => "https://www.google.com/search?q=%s",
            "bing" => "https://www.bing.com/search?q=%s",
            "brave" => "https://search.brave.com/search?q=%s",
            "kagi" => "https://kagi.com/search?q=%s",
            "ecosia" => "https://www.ecosia.org/search?q=%s",
            "custom" if self.custom_url_valid() => &self.custom_url,
            _ => DEFAULT_SEARCH,
        };
        let mut encoded = String::new();
        for byte in text.bytes() {
            if byte.is_ascii_alphanumeric() || b"-_.~".contains(&byte) {
                encoded.push(char::from(byte));
            } else {
                use std::fmt::Write;
                let _ = write!(encoded, "%{byte:02X}");
            }
        }
        Some(template.replace("%s", &encoded))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_respects_the_engine_without_redirecting_local_dev_addresses() {
        let mut prefs = BrowserPreferences {
            engine: "custom".into(),
            custom_url: "https://example.org/find/%s".into(),
            ..Default::default()
        };
        assert_eq!(
            prefs.address("c++ & é").as_deref(),
            Some("https://example.org/find/c%2B%2B%20%26%20%C3%A9")
        );
        assert_eq!(
            prefs.address("5173").as_deref(),
            Some("http://localhost:5173")
        );
        assert_eq!(
            prefs.address("localhost:3000/a").as_deref(),
            Some("http://localhost:3000/a")
        );
        assert_eq!(
            prefs.address("example.org").as_deref(),
            Some("https://example.org")
        );
        assert!(prefs.address("file:///etc/hosts").is_none());
        assert!(prefs.address("mailto:reader@example.org").is_none());
        assert!(prefs.address("data:text/plain,private note").is_none());
        prefs.custom_url = "https://%s.example.org".into();
        assert_eq!(
            prefs.address("query").as_deref(),
            Some("https://duckduckgo.com/?q=query")
        );
    }
}

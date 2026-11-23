use bruhs_models::agent::AgentProvider;
use bruhs_ui::icons;
use gpui::{AssetSource, SharedString};
use std::borrow::Cow;

pub const BRAND_LOGO_PATH: &str = "bruhs.svg";
pub const BRAND_LOGO_BYTES: &[u8] = include_bytes!("../../bruhs-web/bruhs.svg");
pub const PROVIDER_LOGO_SIZE_PX: f32 = 12.0;
pub const PROVIDER_LOGO_OPACITY: f32 = 0.6;

// Shared Lucide icon paths, served by `bruhs_ui::icons::load`.
pub const ACTIVITY_ICON_PATH: &str = icons::ACTIVITY;
pub const ARROW_UP_ICON_PATH: &str = icons::ARROW_UP;
pub const CLOSE_ICON_PATH: &str = icons::CLOSE;
pub const CHECK_ICON_PATH: &str = icons::CHECK;
pub const ALERT_TRIANGLE_ICON_PATH: &str = icons::ALERT_TRIANGLE;
pub const GIT_BRANCH_ICON_PATH: &str = icons::GIT_BRANCH;
pub const PIN_ICON_PATH: &str = icons::PIN;
/// Web `ellipsis` (lucide), the thread card's actions button.
pub const ELLIPSIS_ICON_PATH: &str = icons::ELLIPSIS;
pub const FILE_DIFF_ICON_PATH: &str = icons::FILE_DIFF;
pub const FOLDER_ICON_PATH: &str = icons::FOLDER;
pub const SQUARE_ICON_PATH: &str = icons::SQUARE;
pub const BRAIN_ICON_PATH: &str = icons::BRAIN;
pub const TOOL_TERMINAL_ICON_PATH: &str = icons::TERMINAL;
pub const TOOL_FILE_ICON_PATH: &str = icons::FILE;
pub const TOOL_PENCIL_ICON_PATH: &str = icons::PENCIL;
pub const TOOL_FILE_PLUS_ICON_PATH: &str = icons::FILE_PLUS;
pub const TOOL_FOLDER_SEARCH_ICON_PATH: &str = icons::FOLDER_SEARCH;
pub const TOOL_SEARCH_ICON_PATH: &str = icons::SEARCH;
pub const TOOL_GLOBE_ICON_PATH: &str = icons::GLOBE;
pub const SQUARE_PEN_ICON_PATH: &str = icons::SQUARE_PEN;
/// The composer's Attach control, as on the web.
pub const CAMERA_ICON_PATH: &str = icons::CAMERA;
pub const PAPERCLIP_ICON_PATH: &str = icons::PAPERCLIP;
pub const FOLDER_PLUS_ICON_PATH: &str = icons::FOLDER_PLUS;
pub const LIST_CHECKS_ICON_PATH: &str = icons::LIST_CHECKS;
pub const MIC_ICON_PATH: &str = icons::MIC;
pub const USERS_ICON_PATH: &str = icons::USERS;
pub const CLOCK_ICON_PATH: &str = icons::CLOCK;
pub const SMARTPHONE_ICON_PATH: &str = icons::SMARTPHONE;
pub const CALENDAR_ICON_PATH: &str = icons::CALENDAR;
pub const PLUS_ICON_PATH: &str = icons::PLUS;
pub const FILE_TEXT_ICON_PATH: &str = icons::FILE_TEXT;
/// The web's `bookmark` icon: a saved source in the Knowledge library.
pub const BOOKMARK_ICON_PATH: &str = icons::BOOKMARK;
pub const USER_PLUS_ICON_PATH: &str = icons::USER_PLUS;
/// The update pill's spinner (web `loader`).
pub const LOADER_ICON_PATH: &str = icons::LOADER;
pub const MINUS_ICON_PATH: &str = icons::MINUS;
pub const BOOK_OPEN_ICON_PATH: &str = icons::BOOK_OPEN;
pub const SUN_ICON_PATH: &str = icons::SUN;
pub const INBOX_ICON_PATH: &str = icons::INBOX;
pub const WRAP_TEXT_ICON_PATH: &str = icons::WRAP_TEXT;
pub const MESSAGE_SQUARE_ICON_PATH: &str = icons::MESSAGE_SQUARE;
pub const PEN_LINE_ICON_PATH: &str = icons::PEN_LINE;
pub const ARCHIVE_ICON_PATH: &str = icons::ARCHIVE;
pub const CLIPBOARD_LIST_ICON_PATH: &str = icons::CLIPBOARD_LIST;
pub const CHEVRON_DOWN_ICON_PATH: &str = icons::CHEVRON_DOWN;
pub const PANEL_RIGHT_ICON_PATH: &str = icons::PANEL_RIGHT;
pub const BOT_ICON_PATH: &str = icons::BOT;
pub const KEYBOARD_ICON_PATH: &str = icons::KEYBOARD;
pub const EYE_ICON_PATH: &str = icons::EYE;
pub const WRENCH_ICON_PATH: &str = icons::WRENCH;
pub const HAMMER_ICON_PATH: &str = icons::HAMMER;
pub const CHEVRON_LEFT_ICON_PATH: &str = icons::CHEVRON_LEFT;
pub const CHEVRON_RIGHT_ICON_PATH: &str = icons::CHEVRON_RIGHT;
pub const CIRCLE_ALERT_ICON_PATH: &str = icons::CIRCLE_ALERT;
pub const ROTATE_CCW_ICON_PATH: &str = icons::ROTATE_CCW;
pub const COPY_ICON_PATH: &str = icons::COPY;
pub const DOWNLOAD_ICON_PATH: &str = icons::DOWNLOAD;
pub const HELP_CIRCLE_ICON_PATH: &str = icons::HELP_CIRCLE;
pub const EXTERNAL_LINK_ICON_PATH: &str = icons::EXTERNAL_LINK;
pub const REFRESH_ICON_PATH: &str = icons::REFRESH;

pub struct DesktopAssets;

/// Monaspace static faces at Regular/Medium/SemiBold/Bold, plus Regular and
/// Medium italics (SIL OFL, see `assets/fonts/LICENSE`), matching
/// the families the web theme serves: Argon (labels/UI), Neon (body/code),
/// Xenon (headings), Krypton (display), Radon (callouts).
pub fn embedded_fonts() -> Vec<Cow<'static, [u8]>> {
    vec![
        Cow::Borrowed(&include_bytes!("../assets/fonts/MonaspaceArgon-Regular.otf")[..]),
        Cow::Borrowed(&include_bytes!("../assets/fonts/MonaspaceArgon-Medium.otf")[..]),
        Cow::Borrowed(&include_bytes!("../assets/fonts/MonaspaceArgon-SemiBold.otf")[..]),
        Cow::Borrowed(&include_bytes!("../assets/fonts/MonaspaceArgon-Bold.otf")[..]),
        // `wdth` 104 instances of the variable Argon: stand-ins for the web's
        // tracked small labels (see `theme::UI_SMALL_FONT_FAMILY`).
        Cow::Borrowed(&include_bytes!("../assets/fonts/MonaspaceArgonTracked-Regular.ttf")[..]),
        Cow::Borrowed(&include_bytes!("../assets/fonts/MonaspaceArgonTracked-Medium.ttf")[..]),
        Cow::Borrowed(&include_bytes!("../assets/fonts/MonaspaceArgonTracked-SemiBold.ttf")[..]),
        Cow::Borrowed(&include_bytes!("../assets/fonts/MonaspaceArgonTracked-Bold.ttf")[..]),
        // Slanted instances (`slnt` -11) of the variable fonts the web
        // serves, for italic text: the web synthesizes it, and gpui draws
        // upright text when a family has no italic face.
        Cow::Borrowed(&include_bytes!("../assets/fonts/MonaspaceArgon-Italic.ttf")[..]),
        Cow::Borrowed(&include_bytes!("../assets/fonts/MonaspaceArgon-MediumItalic.ttf")[..]),
        Cow::Borrowed(&include_bytes!("../assets/fonts/MonaspaceArgonTracked-Italic.ttf")[..]),
        Cow::Borrowed(
            &include_bytes!("../assets/fonts/MonaspaceArgonTracked-MediumItalic.ttf")[..],
        ),
        Cow::Borrowed(&include_bytes!("../assets/fonts/MonaspaceNeon-Italic.ttf")[..]),
        Cow::Borrowed(&include_bytes!("../assets/fonts/MonaspaceNeon-MediumItalic.ttf")[..]),
        Cow::Borrowed(&include_bytes!("../assets/fonts/MonaspaceNeon-Regular.otf")[..]),
        Cow::Borrowed(&include_bytes!("../assets/fonts/MonaspaceNeon-Medium.otf")[..]),
        Cow::Borrowed(&include_bytes!("../assets/fonts/MonaspaceNeon-SemiBold.otf")[..]),
        Cow::Borrowed(&include_bytes!("../assets/fonts/MonaspaceNeon-Bold.otf")[..]),
        Cow::Borrowed(&include_bytes!("../assets/fonts/MonaspaceXenon-Regular.otf")[..]),
        Cow::Borrowed(&include_bytes!("../assets/fonts/MonaspaceXenon-Medium.otf")[..]),
        Cow::Borrowed(&include_bytes!("../assets/fonts/MonaspaceXenon-SemiBold.otf")[..]),
        Cow::Borrowed(&include_bytes!("../assets/fonts/MonaspaceXenon-Bold.otf")[..]),
    ]
}

impl AssetSource for DesktopAssets {
    fn load(&self, path: &str) -> gpui::Result<Option<Cow<'static, [u8]>>> {
        if let Some(svg) = icons::load(path) {
            return Ok(Some(Cow::Borrowed(svg)));
        }
        match path {
            BRAND_LOGO_PATH => Ok(Some(Cow::Borrowed(BRAND_LOGO_BYTES))),
            "logos/claude.svg" => Ok(Some(Cow::Borrowed(
                &include_bytes!("../../bruhs-web/logos/claude.svg")[..],
            ))),
            "logos/openai.svg" => Ok(Some(Cow::Borrowed(
                &include_bytes!("../../bruhs-web/logos/openai.svg")[..],
            ))),
            "logos/vercel.svg" => Ok(Some(Cow::Borrowed(
                &include_bytes!("../../bruhs-web/logos/vercel.svg")[..],
            ))),
            "icons/kiro.svg" => Ok(Some(Cow::Borrowed(include_bytes!(
                "../assets/icons/kiro.svg"
            )))),
            _ => Ok(None),
        }
    }

    fn list(&self, path: &str) -> gpui::Result<Vec<SharedString>> {
        match path {
            "" => Ok(vec![BRAND_LOGO_PATH.into(), "logos".into(), "icons".into()]),
            "logos" => Ok(vec![
                "claude.svg".into(),
                "openai.svg".into(),
                "vercel.svg".into(),
            ]),
            "icons" => {
                let mut names = vec![SharedString::from("kiro.svg")];
                names.extend(
                    icons::ALL_PATHS
                        .iter()
                        .filter_map(|p| p.strip_prefix("icons/"))
                        .map(SharedString::from),
                );
                Ok(names)
            }
            _ => Ok(Vec::new()),
        }
    }
}

pub fn provider_logo_path(provider: AgentProvider) -> &'static str {
    match provider {
        AgentProvider::Claude => "logos/claude.svg",
        AgentProvider::Codex => "logos/openai.svg",
        _ => BOT_ICON_PATH,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        provider_logo_path, AgentProvider, DesktopAssets, ALERT_TRIANGLE_ICON_PATH,
        CHECK_ICON_PATH, CLOSE_ICON_PATH, ELLIPSIS_ICON_PATH, FOLDER_ICON_PATH,
        GIT_BRANCH_ICON_PATH, PROVIDER_LOGO_OPACITY, PROVIDER_LOGO_SIZE_PX, SUN_ICON_PATH,
    };
    use gpui::AssetSource as _;

    #[test]
    fn top_bar_icons_should_be_embedded() {
        for path in [
            "icons/panel-left.svg",
            "icons/bot.svg",
            "icons/layout-dashboard.svg",
            "icons/calendar.svg",
            "icons/plus.svg",
            "icons/search.svg",
            "icons/settings.svg",
            "icons/square-terminal.svg",
        ] {
            assert!(
                DesktopAssets.load(path).unwrap().is_some(),
                "missing {path}"
            );
        }
    }

    /// Every `"icons/….svg"` the desktop names must be served by `DesktopAssets`
    /// (via `bruhs_ui::icons::load` or app-specific overrides): an unknown path
    /// loads nothing and the icon draws blank.
    #[test]
    fn every_icon_path_in_the_source_is_served() {
        fn visit(dir: &std::path::Path, found: &mut Vec<String>) {
            for entry in std::fs::read_dir(dir).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    visit(&path, found);
                } else if path.extension().is_some_and(|ext| ext == "rs") {
                    let text = std::fs::read_to_string(&path).unwrap();
                    let mut rest = text.as_str();
                    while let Some(start) = rest.find("\"icons/") {
                        rest = &rest[start + 1..];
                        let end = rest.find('"').unwrap();
                        let literal = &rest[..end];
                        if literal.ends_with(".svg")
                            && literal.bytes().all(|b| {
                                b.is_ascii_lowercase() || b.is_ascii_digit() || b"-./".contains(&b)
                            })
                        {
                            found.push(literal.to_string());
                        }
                        rest = &rest[end..];
                    }
                }
            }
        }
        let mut paths = Vec::new();
        visit(
            &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
            &mut paths,
        );
        assert!(DesktopAssets.load(SUN_ICON_PATH).unwrap().is_some());
        for path in paths {
            assert!(
                DesktopAssets.load(&path).unwrap().is_some(),
                "{path} is named in the source but not served"
            );
        }
    }

    #[test]
    fn provider_logo_path_should_match_web_assets() {
        assert_eq!(
            provider_logo_path(AgentProvider::Claude),
            "logos/claude.svg"
        );
        assert_eq!(provider_logo_path(AgentProvider::Codex), "logos/openai.svg");
    }

    #[test]
    fn provider_logo_size_should_match_web_provider_logo_class() {
        assert_eq!(PROVIDER_LOGO_SIZE_PX, 12.0);
    }

    #[test]
    fn provider_logo_opacity_should_match_web_provider_logo_class() {
        assert_eq!(PROVIDER_LOGO_OPACITY, 0.6);
    }

    #[test]
    fn git_branch_icon_path_should_match_web_icon_name() {
        assert_eq!(GIT_BRANCH_ICON_PATH, "icons/git-branch.svg");
    }

    #[test]
    fn folder_icon_path_should_match_web_icon_name() {
        assert_eq!(FOLDER_ICON_PATH, "icons/folder.svg");
    }

    #[test]
    fn ellipsis_icon_should_load_like_web_icon_name() {
        assert_eq!(ELLIPSIS_ICON_PATH, "icons/ellipsis.svg");
        let svg = DesktopAssets.load(ELLIPSIS_ICON_PATH).unwrap().unwrap();
        assert!(std::str::from_utf8(&svg)
            .unwrap()
            .contains(r#"<circle cx="19" cy="12" r="1"/>"#));
    }

    #[test]
    fn permission_prompt_icon_paths_should_match_web_icon_names() {
        assert_eq!(ALERT_TRIANGLE_ICON_PATH, "icons/alert-triangle.svg");
        assert_eq!(CLOSE_ICON_PATH, "icons/x.svg");
        assert_eq!(CHECK_ICON_PATH, "icons/check.svg");
    }
}

use bruhs_models::agent::{AgentProvider, PermissionMode};
use gpui::{AssetSource, SharedString};
use std::borrow::Cow;

pub const BRAND_LOGO_PATH: &str = "bruhs.svg";
pub const PROVIDER_LOGO_SIZE_PX: f32 = 12.0;
pub const PROVIDER_LOGO_OPACITY: f32 = 0.6;
pub const ACTIVITY_ICON_PATH: &str = "icons/activity.svg";
pub const ARROW_UP_ICON_PATH: &str = "icons/arrow-up.svg";
pub const CLOSE_ICON_PATH: &str = "icons/x.svg";
pub const CHECK_ICON_PATH: &str = "icons/check.svg";
pub const ALERT_TRIANGLE_ICON_PATH: &str = "icons/alert-triangle.svg";
pub const GIT_BRANCH_ICON_PATH: &str = "icons/git-branch.svg";
pub const PIN_ICON_PATH: &str = "icons/pin.svg";
pub const FILE_DIFF_ICON_PATH: &str = "icons/file-diff.svg";
pub const FOLDER_ICON_PATH: &str = "icons/folder.svg";
pub const SQUARE_ICON_PATH: &str = "icons/square.svg";
pub const BRAIN_ICON_PATH: &str = "icons/brain.svg";
pub const TOOL_TERMINAL_ICON_PATH: &str = "icons/terminal.svg";
pub const TOOL_FILE_ICON_PATH: &str = "icons/file.svg";
pub const TOOL_PENCIL_ICON_PATH: &str = "icons/pencil.svg";
pub const TOOL_FILE_PLUS_ICON_PATH: &str = "icons/file-plus.svg";
pub const TOOL_FOLDER_SEARCH_ICON_PATH: &str = "icons/folder-search.svg";
pub const TOOL_SEARCH_ICON_PATH: &str = "icons/search.svg";
pub const TOOL_GLOBE_ICON_PATH: &str = "icons/globe.svg";
pub const TOOL_MESSAGE_QUESTION_ICON_PATH: &str = "icons/message-circle-question.svg";
// Sidebar / composer icons (same lucide paths as bruhs-web/src/ui/icon.rs).
pub const SQUARE_PEN_ICON_PATH: &str = "icons/square-pen.svg";
pub const FOLDER_PLUS_ICON_PATH: &str = "icons/folder-plus.svg";
pub const LIST_CHECKS_ICON_PATH: &str = "icons/list-checks.svg";
pub const MIC_ICON_PATH: &str = "icons/mic.svg";
pub const USERS_ICON_PATH: &str = "icons/users.svg";
pub const CALENDAR_ICON_PATH: &str = "icons/calendar.svg";
pub const PLUS_ICON_PATH: &str = "icons/plus.svg";
pub const MINUS_ICON_PATH: &str = "icons/minus.svg";
pub const BOOK_OPEN_ICON_PATH: &str = "icons/book-open.svg";
pub const WRAP_TEXT_ICON_PATH: &str = "icons/wrap-text.svg";
pub const MESSAGE_SQUARE_ICON_PATH: &str = "icons/message-square.svg";
pub const PEN_LINE_ICON_PATH: &str = "icons/pen-line.svg";
pub const ARCHIVE_ICON_PATH: &str = "icons/archive.svg";
pub const CLIPBOARD_LIST_ICON_PATH: &str = "icons/clipboard-list.svg";
pub const CHEVRON_DOWN_ICON_PATH: &str = "icons/chevron-down.svg";
pub const PANEL_RIGHT_ICON_PATH: &str = "icons/panel-right.svg";
pub const BOT_ICON_PATH: &str = "icons/bot.svg";
pub const KEYBOARD_ICON_PATH: &str = "icons/keyboard.svg";
pub const EYE_ICON_PATH: &str = "icons/eye.svg";
pub const WRENCH_ICON_PATH: &str = "icons/wrench.svg";
pub const HAMMER_ICON_PATH: &str = "icons/hammer.svg";
pub const CHEVRON_RIGHT_ICON_PATH: &str = "icons/chevron-right.svg";
pub const CIRCLE_ALERT_ICON_PATH: &str = "icons/circle-alert.svg";
pub const ROTATE_CCW_ICON_PATH: &str = "icons/rotate-ccw.svg";
pub const COPY_ICON_PATH: &str = "icons/copy.svg";
pub const DOWNLOAD_ICON_PATH: &str = "icons/download.svg";
pub const HELP_CIRCLE_ICON_PATH: &str = "icons/help-circle.svg";

const TOP_BAR_PANEL_LEFT_ICON: &[u8] =
    include_bytes!("../../third_party/gpui-component/crates/assets/assets/icons/panel-left.svg");
const TOP_BAR_BOT_ICON: &[u8] =
    include_bytes!("../../third_party/gpui-component/crates/assets/assets/icons/bot.svg");
const TOP_BAR_TASKS_ICON: &[u8] = include_bytes!(
    "../../third_party/gpui-component/crates/assets/assets/icons/layout-dashboard.svg"
);
const TOP_BAR_CALENDAR_ICON: &[u8] =
    include_bytes!("../../third_party/gpui-component/crates/assets/assets/icons/calendar.svg");
const TOP_BAR_PLUS_ICON: &[u8] =
    include_bytes!("../../third_party/gpui-component/crates/assets/assets/icons/plus.svg");
const TOP_BAR_SETTINGS_ICON: &[u8] =
    include_bytes!("../../third_party/gpui-component/crates/assets/assets/icons/settings.svg");
const TOP_BAR_TERMINAL_ICON: &[u8] = include_bytes!(
    "../../third_party/gpui-component/crates/assets/assets/icons/square-terminal.svg"
);
const EXTERNAL_LINK_ICON: &[u8] =
    include_bytes!("../../third_party/gpui-component/crates/assets/assets/icons/external-link.svg");
pub const EXTERNAL_LINK_ICON_PATH: &str = "icons/external-link.svg";
pub const REFRESH_ICON_PATH: &str = "icons/refresh-cw.svg";
const SQUARE_PEN_ICON: &[u8] = br#"<svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" xmlns="http://www.w3.org/2000/svg"><path d="M12 3H5a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h14a2 2 0 0 0 2-2v-7"/><path d="M18.375 2.625a1 1 0 0 1 3 3l-9.013 9.014a2 2 0 0 1-.853.505l-2.873.84a.5.5 0 0 1-.62-.62l.84-2.873a2 2 0 0 1 .506-.852z"/></svg>"#;
const FOLDER_PLUS_ICON: &[u8] = br#"<svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" xmlns="http://www.w3.org/2000/svg"><path d="M12 10v6"/><path d="M9 13h6"/><path d="M20 20a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.9a2 2 0 0 1-1.69-.9L9.6 3.9A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2Z"/></svg>"#;
const LIST_CHECKS_ICON: &[u8] = br#"<svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" xmlns="http://www.w3.org/2000/svg"><path d="m3 17 2 2 4-4"/><path d="m3 7 2 2 4-4"/><path d="M13 6h8"/><path d="M13 12h8"/><path d="M13 18h8"/></svg>"#;
const MIC_ICON: &[u8] = br#"<svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" xmlns="http://www.w3.org/2000/svg"><path d="M12 2a3 3 0 0 0-3 3v7a3 3 0 0 0 6 0V5a3 3 0 0 0-3-3Z"/><path d="M19 10v2a7 7 0 0 1-14 0v-2"/><line x1="12" x2="12" y1="19" y2="22"/></svg>"#;
const MINUS_ICON: &[u8] = br#"<svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" xmlns="http://www.w3.org/2000/svg"><path d="M5 12h14"/></svg>"#;
const WRAP_TEXT_ICON: &[u8] = br#"<svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" xmlns="http://www.w3.org/2000/svg"><path d="M3 6h18"/><path d="M3 12h15a3 3 0 1 1 0 6h-4"/><path d="m16 16-2 2 2 2"/><path d="M3 18h7"/></svg>"#;
const BOOK_OPEN_ICON: &[u8] = br#"<svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" xmlns="http://www.w3.org/2000/svg"><path d="M12 7v14"/><path d="M3 18a1 1 0 0 1-1-1V4a1 1 0 0 1 1-1h5a4 4 0 0 1 4 4 4 4 0 0 1 4-4h5a1 1 0 0 1 1 1v13a1 1 0 0 1-1 1h-6a3 3 0 0 0-3 3 3 3 0 0 0-3-3z"/></svg>"#;
const USERS_ICON: &[u8] = br#"<svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" xmlns="http://www.w3.org/2000/svg"><path d="M16 21v-2a4 4 0 0 0-4-4H6a4 4 0 0 0-4 4v2"/><circle cx="9" cy="7" r="4"/><path d="M22 21v-2a4 4 0 0 0-3-3.87"/><path d="M16 3.13a4 4 0 0 1 0 7.75"/></svg>"#;
const MESSAGE_SQUARE_ICON: &[u8] = br#"<svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" xmlns="http://www.w3.org/2000/svg"><path d="M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z"/></svg>"#;
const PEN_LINE_ICON: &[u8] = br#"<svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" xmlns="http://www.w3.org/2000/svg"><path d="M12 20h9"/><path d="M16.376 3.622a1 1 0 0 1 3.002 3.002L7.368 18.635a2 2 0 0 1-.855.506l-2.872.838a.5.5 0 0 1-.62-.62l.838-2.872a2 2 0 0 1 .506-.854z"/></svg>"#;
const ARCHIVE_ICON: &[u8] = br#"<svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" xmlns="http://www.w3.org/2000/svg"><rect width="20" height="5" x="2" y="3" rx="1"/><path d="M4 8v11a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8"/><path d="M10 12h4"/></svg>"#;
const EYE_ICON: &[u8] = br#"<svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" xmlns="http://www.w3.org/2000/svg"><path d="M2.062 12.348a1 1 0 0 1 0-.696 10.75 10.75 0 0 1 19.876 0 1 1 0 0 1 0 .696 10.75 10.75 0 0 1-19.876 0"/><circle cx="12" cy="12" r="3"/></svg>"#;
const WRENCH_ICON: &[u8] = br#"<svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" xmlns="http://www.w3.org/2000/svg"><path d="M14.7 6.3a1 1 0 0 0 0 1.4l1.6 1.6a1 1 0 0 0 1.4 0l3.77-3.77a6 6 0 0 1-7.94 7.94l-6.91 6.91a2.12 2.12 0 0 1-3-3l6.91-6.91a6 6 0 0 1 7.94-7.94l-3.76 3.76z"/></svg>"#;
const HAMMER_ICON: &[u8] = br#"<svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" xmlns="http://www.w3.org/2000/svg"><path d="m15 12-8.373 8.373a1 1 0 1 1-3-3L12 9"/><path d="m18 15 4-4"/><path d="m21.5 11.5-1.914-1.914A2 2 0 0 1 19 8.172V7l-2.26-2.26a6 6 0 0 0-4.202-1.756L9 2.96l.92.82A6.18 6.18 0 0 1 12 8.4V10l2 2h1.172a2 2 0 0 1 1.414.586L18.5 14.5"/></svg>"#;
const CHEVRON_RIGHT_ICON: &[u8] = br#"<svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" xmlns="http://www.w3.org/2000/svg"><path d="m9 18 6-6-6-6"/></svg>"#;
const CIRCLE_ALERT_ICON: &[u8] = br#"<svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" xmlns="http://www.w3.org/2000/svg"><circle cx="12" cy="12" r="10"/><line x1="12" x2="12" y1="8" y2="12"/><line x1="12" x2="12.01" y1="16" y2="16"/></svg>"#;
const ROTATE_CCW_ICON: &[u8] = br#"<svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" xmlns="http://www.w3.org/2000/svg"><path d="M3 12a9 9 0 1 0 9-9 9.75 9.75 0 0 0-6.74 2.74L3 8"/><path d="M3 3v5h5"/></svg>"#;
const DOWNLOAD_ICON: &[u8] = br#"<svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" xmlns="http://www.w3.org/2000/svg"><path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4"/><polyline points="7 10 12 15 17 10"/><line x1="12" x2="12" y1="15" y2="3"/></svg>"#;
const COPY_ICON: &[u8] = br#"<svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" xmlns="http://www.w3.org/2000/svg"><rect width="14" height="14" x="8" y="8" rx="2" ry="2"/><path d="M4 16c-1.1 0-2-.9-2-2V4c0-1.1.9-2 2-2h10c1.1 0 2 .9 2 2"/></svg>"#;
const KEYBOARD_ICON: &[u8] = br#"<svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" xmlns="http://www.w3.org/2000/svg"><path d="M10 8h.01"/><path d="M12 12h.01"/><path d="M14 8h.01"/><path d="M16 12h.01"/><path d="M18 8h.01"/><path d="M6 8h.01"/><path d="M7 16h10"/><path d="M8 12h.01"/><rect width="20" height="16" x="2" y="4" rx="2"/></svg>"#;
const HELP_CIRCLE_ICON: &[u8] = br#"<svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" xmlns="http://www.w3.org/2000/svg"><circle cx="12" cy="12" r="10"/><path d="M9.09 9a3 3 0 0 1 5.83 1c0 2-3 3-3 3"/><path d="M12 17h.01"/></svg>"#;
const CHEVRON_DOWN_ICON: &[u8] =
    include_bytes!("../../third_party/gpui-component/crates/assets/assets/icons/chevron-down.svg");
const PANEL_RIGHT_ICON: &[u8] =
    include_bytes!("../../third_party/gpui-component/crates/assets/assets/icons/panel-right.svg");
const REFRESH_ICON: &[u8] = br#"<svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" xmlns="http://www.w3.org/2000/svg"><path d="M3 12a9 9 0 0 1 9-9 9.75 9.75 0 0 1 6.74 2.74L21 8"/><path d="M21 3v5h-5"/><path d="M21 12a9 9 0 0 1-9 9 9.75 9.75 0 0 1-6.74-2.74L3 16"/><path d="M8 16H3v5"/></svg>"#;

pub struct DesktopAssets;

/// Monaspace static faces at Regular/Medium/SemiBold/Bold (SIL OFL, see `assets/fonts/LICENSE`), matching
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

const ACTIVITY_ICON: &[u8] = br#"<svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" xmlns="http://www.w3.org/2000/svg"><path d="M22 12h-2.48a2 2 0 0 0-1.93 1.46l-2.35 8.36a.5.5 0 0 1-.96 0L9.24 2.18a.5.5 0 0 0-.96 0l-2.35 8.36A2 2 0 0 1 4 12H2"/></svg>"#;
const ARROW_UP_ICON: &[u8] = br#"<svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" xmlns="http://www.w3.org/2000/svg"><path d="m5 12 7-7 7 7"/><path d="M12 19V5"/></svg>"#;
const CLOSE_ICON: &[u8] = br#"<svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" xmlns="http://www.w3.org/2000/svg"><path d="M18 6 6 18"/><path d="m6 6 12 12"/></svg>"#;
const CHECK_ICON: &[u8] = br#"<svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" xmlns="http://www.w3.org/2000/svg"><path d="M20 6 9 17l-5-5"/></svg>"#;
const ALERT_TRIANGLE_ICON: &[u8] = br#"<svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" xmlns="http://www.w3.org/2000/svg"><path d="m21.73 18-8-14a2 2 0 0 0-3.48 0l-8 14A2 2 0 0 0 4 21h16a2 2 0 0 0 1.73-3Z"/><path d="M12 9v4"/><path d="M12 17h.01"/></svg>"#;
const GIT_BRANCH_ICON: &[u8] = br#"<svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" xmlns="http://www.w3.org/2000/svg"><line x1="6" x2="6" y1="3" y2="15"/><circle cx="18" cy="6" r="3"/><circle cx="6" cy="18" r="3"/><path d="M18 9a9 9 0 0 1-9 9"/></svg>"#;
const PIN_ICON: &[u8] = br#"<svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" xmlns="http://www.w3.org/2000/svg"><path d="M12 17v5"/><path d="M9 10.76a2 2 0 0 1-1.11 1.79l-1.78.9A2 2 0 0 0 5 15.24V16a1 1 0 0 0 1 1h12a1 1 0 0 0 1-1v-.76a2 2 0 0 0-1.11-1.79l-1.78-.9A2 2 0 0 1 15 10.76V7a1 1 0 0 1 1-1 2 2 0 0 0 0-4H8a2 2 0 0 0 0 4 1 1 0 0 1 1 1z"/></svg>"#;
const FILE_DIFF_ICON: &[u8] = br#"<svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" xmlns="http://www.w3.org/2000/svg"><path d="M15 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V7z"/><path d="M9 10h6"/><path d="M12 13V7"/><path d="M9 17h6"/></svg>"#;
const FOLDER_ICON: &[u8] = br#"<svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" xmlns="http://www.w3.org/2000/svg"><path d="M20 20a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.9a2 2 0 0 1-1.69-.9L9.6 3.9A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2Z"/></svg>"#;
const SQUARE_ICON: &[u8] = br#"<svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" xmlns="http://www.w3.org/2000/svg"><rect width="14" height="14" x="5" y="5" rx="2"/></svg>"#;
const BRAIN_ICON: &[u8] = br#"<svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" xmlns="http://www.w3.org/2000/svg"><path d="M12 5a3 3 0 1 0-5.997.125 4 4 0 0 0-2.526 5.77 4 4 0 0 0 .556 6.588A4 4 0 1 0 12 18Z"/><path d="M12 5a3 3 0 1 1 5.997.125 4 4 0 0 1 2.526 5.77 4 4 0 0 1-.556 6.588A4 4 0 1 1 12 18Z"/></svg>"#;
const TOOL_TERMINAL_ICON: &[u8] = br#"<svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" xmlns="http://www.w3.org/2000/svg"><polyline points="4 17 10 11 4 5"/><line x1="12" x2="20" y1="19" y2="19"/></svg>"#;
const TOOL_FILE_ICON: &[u8] = br#"<svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" xmlns="http://www.w3.org/2000/svg"><path d="M15 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V7Z"/><polyline points="14 2 14 8 20 8"/></svg>"#;
const TOOL_PENCIL_ICON: &[u8] = br#"<svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" xmlns="http://www.w3.org/2000/svg"><path d="M17 3a2.85 2.83 0 1 1 4 4L7.5 20.5 2 22l1.5-5.5Z"/><path d="m15 5 4 4"/></svg>"#;
const TOOL_FILE_PLUS_ICON: &[u8] = br#"<svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" xmlns="http://www.w3.org/2000/svg"><path d="M15 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V7Z"/><polyline points="14 2 14 8 20 8"/><line x1="12" x2="12" y1="18" y2="12"/><line x1="9" x2="15" y1="15" y2="15"/></svg>"#;
const TOOL_FOLDER_SEARCH_ICON: &[u8] = br#"<svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" xmlns="http://www.w3.org/2000/svg"><path d="M11 20H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h3.93a2 2 0 0 1 1.66.9l.82 1.2a2 2 0 0 0 1.66.9H20a2 2 0 0 1 2 2v3.5"/><circle cx="17" cy="17" r="3"/><path d="m21 21-1.5-1.5"/></svg>"#;
const TOOL_SEARCH_ICON: &[u8] = br#"<svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" xmlns="http://www.w3.org/2000/svg"><circle cx="11" cy="11" r="8"/><path d="m21 21-4.3-4.3"/></svg>"#;
const TOOL_GLOBE_ICON: &[u8] = br#"<svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" xmlns="http://www.w3.org/2000/svg"><circle cx="12" cy="12" r="10"/><path d="M12 2a14.5 14.5 0 0 0 0 20 14.5 14.5 0 0 0 0-20"/><path d="M2 12h20"/></svg>"#;
const TOOL_MESSAGE_QUESTION_ICON: &[u8] = br#"<svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" xmlns="http://www.w3.org/2000/svg"><path d="M7.9 20A9 9 0 1 0 4 16.1L2 22Z"/><path d="M9.09 9a3 3 0 0 1 5.83 1c0 2-3 3-3 3"/><path d="M12 17h.01"/></svg>"#;
const SHIELD_ICON: &[u8] = br#"<svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" xmlns="http://www.w3.org/2000/svg"><path d="M20 13c0 5-3.5 7.5-7.66 8.95a1 1 0 0 1-.67-.01C7.5 20.5 4 18 4 13V6a1 1 0 0 1 1-1c2 0 4.5-1.2 6.24-2.72a1.17 1.17 0 0 1 1.52 0C14.51 3.81 17 5 19 5a1 1 0 0 1 1 1z"/></svg>"#;
const SHIELD_CHECK_ICON: &[u8] = br#"<svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" xmlns="http://www.w3.org/2000/svg"><path d="M20 13c0 5-3.5 7.5-7.66 8.95a1 1 0 0 1-.67-.01C7.5 20.5 4 18 4 13V6a1 1 0 0 1 1-1c2 0 4.5-1.2 6.24-2.72a1.17 1.17 0 0 1 1.52 0C14.51 3.81 17 5 19 5a1 1 0 0 1 1 1z"/><path d="m9 12 2 2 4-4"/></svg>"#;
const CLIPBOARD_LIST_ICON: &[u8] = br#"<svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" xmlns="http://www.w3.org/2000/svg"><rect width="8" height="4" x="8" y="2" rx="1" ry="1"/><path d="M16 4h2a2 2 0 0 1 2 2v14a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2V6a2 2 0 0 1 2-2h2"/><path d="M12 11h4"/><path d="M12 16h4"/><path d="M8 11h.01"/><path d="M8 16h.01"/></svg>"#;
const HAND_METAL_ICON: &[u8] = br#"<svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" xmlns="http://www.w3.org/2000/svg"><path d="M18 12.5V10a2 2 0 0 0-2-2v0a2 2 0 0 0-2 2v1.4"/><path d="M14 11V9a2 2 0 0 0-2-2v0a2 2 0 0 0-2 2v2"/><path d="M10 10.5V5a2 2 0 0 0-2-2v0a2 2 0 0 0-2 2v9"/><path d="M18 10.5a2 2 0 1 1 4 0V14a8 8 0 0 1-8 8h-4a8 8 0 0 1-8-8 2 2 0 1 1 4 0"/></svg>"#;

impl AssetSource for DesktopAssets {
    fn load(&self, path: &str) -> gpui::Result<Option<Cow<'static, [u8]>>> {
        match path {
            BRAND_LOGO_PATH => Ok(Some(Cow::Borrowed(
                &include_bytes!("../../bruhs-web/bruhs.svg")[..],
            ))),
            "logos/claude.svg" => Ok(Some(Cow::Borrowed(
                &include_bytes!("../../bruhs-web/logos/claude.svg")[..],
            ))),
            "logos/openai.svg" => Ok(Some(Cow::Borrowed(
                &include_bytes!("../../bruhs-web/logos/openai.svg")[..],
            ))),
            "logos/vercel.svg" => Ok(Some(Cow::Borrowed(
                &include_bytes!("../../bruhs-web/logos/vercel.svg")[..],
            ))),
            ACTIVITY_ICON_PATH => Ok(Some(Cow::Borrowed(ACTIVITY_ICON))),
            ARROW_UP_ICON_PATH => Ok(Some(Cow::Borrowed(ARROW_UP_ICON))),
            CLOSE_ICON_PATH => Ok(Some(Cow::Borrowed(CLOSE_ICON))),
            CHECK_ICON_PATH => Ok(Some(Cow::Borrowed(CHECK_ICON))),
            ALERT_TRIANGLE_ICON_PATH => Ok(Some(Cow::Borrowed(ALERT_TRIANGLE_ICON))),
            GIT_BRANCH_ICON_PATH => Ok(Some(Cow::Borrowed(GIT_BRANCH_ICON))),
            PIN_ICON_PATH => Ok(Some(Cow::Borrowed(PIN_ICON))),
            FILE_DIFF_ICON_PATH => Ok(Some(Cow::Borrowed(FILE_DIFF_ICON))),
            FOLDER_ICON_PATH => Ok(Some(Cow::Borrowed(FOLDER_ICON))),
            SQUARE_ICON_PATH => Ok(Some(Cow::Borrowed(SQUARE_ICON))),
            BRAIN_ICON_PATH => Ok(Some(Cow::Borrowed(BRAIN_ICON))),
            TOOL_TERMINAL_ICON_PATH => Ok(Some(Cow::Borrowed(TOOL_TERMINAL_ICON))),
            TOOL_FILE_ICON_PATH => Ok(Some(Cow::Borrowed(TOOL_FILE_ICON))),
            TOOL_PENCIL_ICON_PATH => Ok(Some(Cow::Borrowed(TOOL_PENCIL_ICON))),
            TOOL_FILE_PLUS_ICON_PATH => Ok(Some(Cow::Borrowed(TOOL_FILE_PLUS_ICON))),
            TOOL_FOLDER_SEARCH_ICON_PATH => Ok(Some(Cow::Borrowed(TOOL_FOLDER_SEARCH_ICON))),
            TOOL_SEARCH_ICON_PATH => Ok(Some(Cow::Borrowed(TOOL_SEARCH_ICON))),
            TOOL_GLOBE_ICON_PATH => Ok(Some(Cow::Borrowed(TOOL_GLOBE_ICON))),
            TOOL_MESSAGE_QUESTION_ICON_PATH => Ok(Some(Cow::Borrowed(TOOL_MESSAGE_QUESTION_ICON))),
            "icons/panel-left.svg" => Ok(Some(Cow::Borrowed(TOP_BAR_PANEL_LEFT_ICON))),
            "icons/bot.svg" => Ok(Some(Cow::Borrowed(TOP_BAR_BOT_ICON))),
            "icons/layout-dashboard.svg" => Ok(Some(Cow::Borrowed(TOP_BAR_TASKS_ICON))),
            "icons/calendar.svg" => Ok(Some(Cow::Borrowed(TOP_BAR_CALENDAR_ICON))),
            "icons/plus.svg" => Ok(Some(Cow::Borrowed(TOP_BAR_PLUS_ICON))),
            "icons/settings.svg" => Ok(Some(Cow::Borrowed(TOP_BAR_SETTINGS_ICON))),
            "icons/square-terminal.svg" => Ok(Some(Cow::Borrowed(TOP_BAR_TERMINAL_ICON))),
            EXTERNAL_LINK_ICON_PATH => Ok(Some(Cow::Borrowed(EXTERNAL_LINK_ICON))),
            SQUARE_PEN_ICON_PATH => Ok(Some(Cow::Borrowed(SQUARE_PEN_ICON))),
            FOLDER_PLUS_ICON_PATH => Ok(Some(Cow::Borrowed(FOLDER_PLUS_ICON))),
            LIST_CHECKS_ICON_PATH => Ok(Some(Cow::Borrowed(LIST_CHECKS_ICON))),
            MIC_ICON_PATH => Ok(Some(Cow::Borrowed(MIC_ICON))),
            USERS_ICON_PATH => Ok(Some(Cow::Borrowed(USERS_ICON))),
            MINUS_ICON_PATH => Ok(Some(Cow::Borrowed(MINUS_ICON))),
            BOOK_OPEN_ICON_PATH => Ok(Some(Cow::Borrowed(BOOK_OPEN_ICON))),
            WRAP_TEXT_ICON_PATH => Ok(Some(Cow::Borrowed(WRAP_TEXT_ICON))),
            MESSAGE_SQUARE_ICON_PATH => Ok(Some(Cow::Borrowed(MESSAGE_SQUARE_ICON))),
            PEN_LINE_ICON_PATH => Ok(Some(Cow::Borrowed(PEN_LINE_ICON))),
            ARCHIVE_ICON_PATH => Ok(Some(Cow::Borrowed(ARCHIVE_ICON))),
            CHEVRON_DOWN_ICON_PATH => Ok(Some(Cow::Borrowed(CHEVRON_DOWN_ICON))),
            KEYBOARD_ICON_PATH => Ok(Some(Cow::Borrowed(KEYBOARD_ICON))),
            EYE_ICON_PATH => Ok(Some(Cow::Borrowed(EYE_ICON))),
            WRENCH_ICON_PATH => Ok(Some(Cow::Borrowed(WRENCH_ICON))),
            HAMMER_ICON_PATH => Ok(Some(Cow::Borrowed(HAMMER_ICON))),
            CHEVRON_RIGHT_ICON_PATH => Ok(Some(Cow::Borrowed(CHEVRON_RIGHT_ICON))),
            CIRCLE_ALERT_ICON_PATH => Ok(Some(Cow::Borrowed(CIRCLE_ALERT_ICON))),
            ROTATE_CCW_ICON_PATH => Ok(Some(Cow::Borrowed(ROTATE_CCW_ICON))),
            COPY_ICON_PATH => Ok(Some(Cow::Borrowed(COPY_ICON))),
            DOWNLOAD_ICON_PATH => Ok(Some(Cow::Borrowed(DOWNLOAD_ICON))),
            HELP_CIRCLE_ICON_PATH => Ok(Some(Cow::Borrowed(HELP_CIRCLE_ICON))),
            PANEL_RIGHT_ICON_PATH => Ok(Some(Cow::Borrowed(PANEL_RIGHT_ICON))),
            REFRESH_ICON_PATH => Ok(Some(Cow::Borrowed(REFRESH_ICON))),
            "icons/shield.svg" => Ok(Some(Cow::Borrowed(SHIELD_ICON))),
            "icons/shield-check.svg" => Ok(Some(Cow::Borrowed(SHIELD_CHECK_ICON))),
            "icons/clipboard-list.svg" => Ok(Some(Cow::Borrowed(CLIPBOARD_LIST_ICON))),
            "icons/hand-metal.svg" => Ok(Some(Cow::Borrowed(HAND_METAL_ICON))),
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
            "icons" => Ok(vec![
                "activity.svg".into(),
                "arrow-up.svg".into(),
                "x.svg".into(),
                "check.svg".into(),
                "alert-triangle.svg".into(),
                "git-branch.svg".into(),
                "folder.svg".into(),
                "square.svg".into(),
                "brain.svg".into(),
                "terminal.svg".into(),
                "file.svg".into(),
                "pencil.svg".into(),
                "file-plus.svg".into(),
                "folder-search.svg".into(),
                "search.svg".into(),
                "globe.svg".into(),
                "message-circle-question.svg".into(),
                "panel-left.svg".into(),
                "bot.svg".into(),
                "layout-dashboard.svg".into(),
                "calendar.svg".into(),
                "plus.svg".into(),
                "settings.svg".into(),
                "shield.svg".into(),
                "shield-check.svg".into(),
                "download.svg".into(),
                "clipboard-list.svg".into(),
                "hand-metal.svg".into(),
            ]),
            _ => Ok(Vec::new()),
        }
    }
}

pub fn provider_logo_path(provider: AgentProvider) -> &'static str {
    match provider {
        AgentProvider::Claude => "logos/claude.svg",
        AgentProvider::Codex => "logos/openai.svg",
    }
}

pub fn permission_mode_icon_path(mode: PermissionMode) -> &'static str {
    match mode {
        PermissionMode::Default => "icons/shield.svg",
        PermissionMode::AcceptEdits => "icons/shield-check.svg",
        PermissionMode::Plan => "icons/clipboard-list.svg",
        PermissionMode::BypassPermissions => "icons/hand-metal.svg",
    }
}

#[cfg(test)]
mod tests {
    use super::{
        permission_mode_icon_path, provider_logo_path, AgentProvider, DesktopAssets,
        PermissionMode, ALERT_TRIANGLE_ICON_PATH, CHECK_ICON_PATH, CLOSE_ICON_PATH,
        FOLDER_ICON_PATH, GIT_BRANCH_ICON_PATH, PROVIDER_LOGO_OPACITY, PROVIDER_LOGO_SIZE_PX,
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
    fn permission_mode_icon_path_should_match_web_icons() {
        assert_eq!(
            permission_mode_icon_path(PermissionMode::Default),
            "icons/shield.svg"
        );
        assert_eq!(
            permission_mode_icon_path(PermissionMode::AcceptEdits),
            "icons/shield-check.svg"
        );
        assert_eq!(
            permission_mode_icon_path(PermissionMode::Plan),
            "icons/clipboard-list.svg"
        );
        assert_eq!(
            permission_mode_icon_path(PermissionMode::BypassPermissions),
            "icons/hand-metal.svg"
        );
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
    fn permission_prompt_icon_paths_should_match_web_icon_names() {
        assert_eq!(ALERT_TRIANGLE_ICON_PATH, "icons/alert-triangle.svg");
        assert_eq!(CLOSE_ICON_PATH, "icons/x.svg");
        assert_eq!(CHECK_ICON_PATH, "icons/check.svg");
    }
}

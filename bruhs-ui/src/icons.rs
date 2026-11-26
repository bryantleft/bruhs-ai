//! Icons the toolkit and desktop app draw, as Lucide SVGs.
//!
//! Components refer to an icon by path. Serve these from the app's
//! [`gpui::AssetSource`] by calling [`load`] first.

use gpui::{svg, App, Hsla, IntoElement, Pixels, RenderOnce, Styled, Window};

pub const GAUGE: &str = "icons/gauge.svg";
pub const TRENDING_UP: &str = "icons/trending-up.svg";
pub const TRENDING_DOWN: &str = "icons/trending-down.svg";
pub const TICKET: &str = "icons/ticket.svg";
pub const ACTIVITY: &str = "icons/activity.svg";
pub const ALERT_TRIANGLE: &str = "icons/alert-triangle.svg";
pub const ARCHIVE: &str = "icons/archive.svg";
pub const ARROW_UP: &str = "icons/arrow-up.svg";
pub const BOOK_OPEN: &str = "icons/book-open.svg";
pub const BOOKMARK: &str = "icons/bookmark.svg";
pub const BOT: &str = "icons/bot.svg";
pub const BRAIN: &str = "icons/brain.svg";
pub const CALENDAR: &str = "icons/calendar.svg";
pub const CAMERA: &str = "icons/camera.svg";
pub const CHECK: &str = "icons/check.svg";
pub const CHEVRON_DOWN: &str = "icons/chevron-down.svg";
pub const CHEVRON_LEFT: &str = "icons/chevron-left.svg";
pub const CHEVRON_RIGHT: &str = "icons/chevron-right.svg";
pub const CIRCLE_ALERT: &str = "icons/circle-alert.svg";
pub const CLIPBOARD_LIST: &str = "icons/clipboard-list.svg";
pub const CLOCK: &str = "icons/clock.svg";
pub const CLOSE: &str = "icons/x.svg";
pub const COPY: &str = "icons/copy.svg";
pub const DOWNLOAD: &str = "icons/download.svg";
pub const ELLIPSIS: &str = "icons/ellipsis.svg";
pub const EXTERNAL_LINK: &str = "icons/external-link.svg";
pub const EYE: &str = "icons/eye.svg";
pub const FILE: &str = "icons/file.svg";
pub const FILE_DIFF: &str = "icons/file-diff.svg";
pub const FILE_PLUS: &str = "icons/file-plus.svg";
pub const FILE_TEXT: &str = "icons/file-text.svg";
pub const FOLDER: &str = "icons/folder.svg";
pub const FOLDER_PLUS: &str = "icons/folder-plus.svg";
pub const FOLDER_SEARCH: &str = "icons/folder-search.svg";
pub const GIT_BRANCH: &str = "icons/git-branch.svg";
pub const GLOBE: &str = "icons/globe.svg";
pub const HAMMER: &str = "icons/hammer.svg";
pub const HAND_METAL: &str = "icons/hand-metal.svg";
pub const HELP_CIRCLE: &str = "icons/help-circle.svg";
pub const INBOX: &str = "icons/inbox.svg";
pub const KEYBOARD: &str = "icons/keyboard.svg";
pub const LAYOUT_DASHBOARD: &str = "icons/layout-dashboard.svg";
pub const LIST_CHECKS: &str = "icons/list-checks.svg";
pub const LOADER: &str = "icons/loader.svg";
pub const MESSAGE_QUESTION: &str = "icons/message-circle-question.svg";
pub const MESSAGE_SQUARE: &str = "icons/message-square.svg";
pub const MIC: &str = "icons/mic.svg";
pub const MINUS: &str = "icons/minus.svg";
pub const PANEL_LEFT: &str = "icons/panel-left.svg";
pub const PANEL_RIGHT: &str = "icons/panel-right.svg";
pub const PAPERCLIP: &str = "icons/paperclip.svg";
pub const PEN_LINE: &str = "icons/pen-line.svg";
pub const PENCIL: &str = "icons/pencil.svg";
pub const PIN: &str = "icons/pin.svg";
pub const PLUS: &str = "icons/plus.svg";
pub const REFRESH: &str = "icons/refresh-cw.svg";
pub const ROTATE_CCW: &str = "icons/rotate-ccw.svg";
pub const SEARCH: &str = "icons/search.svg";
pub const SEND: &str = "icons/send.svg";
pub const SETTINGS: &str = "icons/settings.svg";
pub const SHIELD: &str = "icons/shield.svg";
pub const SHIELD_CHECK: &str = "icons/shield-check.svg";
pub const SMARTPHONE: &str = "icons/smartphone.svg";
pub const SQUARE: &str = "icons/square.svg";
pub const SQUARE_PEN: &str = "icons/square-pen.svg";
pub const SQUARE_TERMINAL: &str = "icons/square-terminal.svg";
pub const STAR: &str = "icons/star.svg";
pub const STAR_FILLED: &str = "icons/star-filled.svg";
pub const SUN: &str = "icons/sun.svg";
pub const TERMINAL: &str = "icons/terminal.svg";
pub const USER_PLUS: &str = "icons/user-plus.svg";
pub const USERS: &str = "icons/users.svg";
pub const WRAP_TEXT: &str = "icons/wrap-text.svg";
pub const WRENCH: &str = "icons/wrench.svg";
pub const ZAP: &str = "icons/zap.svg";

const ACTIVITY_SVG: &[u8] = br#"<svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" xmlns="http://www.w3.org/2000/svg"><path d="M22 12h-2.48a2 2 0 0 0-1.93 1.46l-2.35 8.36a.5.5 0 0 1-.96 0L9.24 2.18a.5.5 0 0 0-.96 0l-2.35 8.36A2 2 0 0 1 4 12H2"/></svg>"#;
const ALERT_TRIANGLE_SVG: &[u8] = br#"<svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" xmlns="http://www.w3.org/2000/svg"><path d="m21.73 18-8-14a2 2 0 0 0-3.48 0l-8 14A2 2 0 0 0 4 21h16a2 2 0 0 0 1.73-3Z"/><path d="M12 9v4"/><path d="M12 17h.01"/></svg>"#;
const ARCHIVE_SVG: &[u8] = br#"<svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" xmlns="http://www.w3.org/2000/svg"><rect width="20" height="5" x="2" y="3" rx="1"/><path d="M4 8v11a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8"/><path d="M10 12h4"/></svg>"#;
const ARROW_UP_SVG: &[u8] = br#"<svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" xmlns="http://www.w3.org/2000/svg"><path d="m5 12 7-7 7 7"/><path d="M12 19V5"/></svg>"#;
const BOOK_OPEN_SVG: &[u8] = br#"<svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" xmlns="http://www.w3.org/2000/svg"><path d="M12 7v14"/><path d="M3 18a1 1 0 0 1-1-1V4a1 1 0 0 1 1-1h5a4 4 0 0 1 4 4 4 4 0 0 1 4-4h5a1 1 0 0 1 1 1v13a1 1 0 0 1-1 1h-6a3 3 0 0 0-3 3 3 3 0 0 0-3-3z"/></svg>"#;
const BOOKMARK_SVG: &[u8] = br#"<svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" xmlns="http://www.w3.org/2000/svg"><path d="M19 21l-7-4-7 4V5a2 2 0 0 1 2-2h10a2 2 0 0 1 2 2Z"/></svg>"#;
const BOT_SVG: &[u8] = include_bytes!("../assets/icons/bot.svg");
const BRAIN_SVG: &[u8] = br#"<svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" xmlns="http://www.w3.org/2000/svg"><path d="M12 5a3 3 0 1 0-5.997.125 4 4 0 0 0-2.526 5.77 4 4 0 0 0 .556 6.588A4 4 0 1 0 12 18Z"/><path d="M12 5a3 3 0 1 1 5.997.125 4 4 0 0 1 2.526 5.77 4 4 0 0 1-.556 6.588A4 4 0 1 1 12 18Z"/></svg>"#;
const CALENDAR_SVG: &[u8] = include_bytes!("../assets/icons/calendar.svg");
const CAMERA_SVG: &[u8] = include_bytes!("../assets/icons/camera.svg");
const CHECK_SVG: &[u8] = br#"<svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" xmlns="http://www.w3.org/2000/svg"><path d="M20 6 9 17l-5-5"/></svg>"#;
const CHEVRON_DOWN_SVG: &[u8] = include_bytes!("../assets/icons/chevron-down.svg");
const CHEVRON_LEFT_SVG: &[u8] = include_bytes!("../assets/icons/chevron-left.svg");
const CHEVRON_RIGHT_SVG: &[u8] = br#"<svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" xmlns="http://www.w3.org/2000/svg"><path d="m9 18 6-6-6-6"/></svg>"#;
const CIRCLE_ALERT_SVG: &[u8] = br#"<svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" xmlns="http://www.w3.org/2000/svg"><circle cx="12" cy="12" r="10"/><line x1="12" x2="12" y1="8" y2="12"/><line x1="12" x2="12.01" y1="16" y2="16"/></svg>"#;
const CLIPBOARD_LIST_SVG: &[u8] = br#"<svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" xmlns="http://www.w3.org/2000/svg"><rect width="8" height="4" x="8" y="2" rx="1" ry="1"/><path d="M16 4h2a2 2 0 0 1 2 2v14a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2V6a2 2 0 0 1 2-2h2"/><path d="M12 11h4"/><path d="M12 16h4"/><path d="M8 11h.01"/><path d="M8 16h.01"/></svg>"#;
const CLOCK_SVG: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="9"/><path d="M12 6v6l4 2"/></svg>"#;
const CLOSE_SVG: &[u8] = br#"<svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" xmlns="http://www.w3.org/2000/svg"><path d="M18 6 6 18"/><path d="m6 6 12 12"/></svg>"#;
const COPY_SVG: &[u8] = br#"<svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" xmlns="http://www.w3.org/2000/svg"><rect width="14" height="14" x="8" y="8" rx="2" ry="2"/><path d="M4 16c-1.1 0-2-.9-2-2V4c0-1.1.9-2 2-2h10c1.1 0 2 .9 2 2"/></svg>"#;
const DOWNLOAD_SVG: &[u8] = br#"<svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" xmlns="http://www.w3.org/2000/svg"><path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4"/><polyline points="7 10 12 15 17 10"/><line x1="12" x2="12" y1="15" y2="3"/></svg>"#;
const ELLIPSIS_SVG: &[u8] = br#"<svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" xmlns="http://www.w3.org/2000/svg"><circle cx="12" cy="12" r="1"/><circle cx="19" cy="12" r="1"/><circle cx="5" cy="12" r="1"/></svg>"#;
const EXTERNAL_LINK_SVG: &[u8] = include_bytes!("../assets/icons/external-link.svg");
const EYE_SVG: &[u8] = br#"<svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" xmlns="http://www.w3.org/2000/svg"><path d="M2.062 12.348a1 1 0 0 1 0-.696 10.75 10.75 0 0 1 19.876 0 1 1 0 0 1 0 .696 10.75 10.75 0 0 1-19.876 0"/><circle cx="12" cy="12" r="3"/></svg>"#;
const FILE_SVG: &[u8] = br#"<svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" xmlns="http://www.w3.org/2000/svg"><path d="M15 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V7Z"/><polyline points="14 2 14 8 20 8"/></svg>"#;
const FILE_DIFF_SVG: &[u8] = br#"<svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" xmlns="http://www.w3.org/2000/svg"><path d="M15 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V7z"/><path d="M9 10h6"/><path d="M12 13V7"/><path d="M9 17h6"/></svg>"#;
const FILE_PLUS_SVG: &[u8] = br#"<svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" xmlns="http://www.w3.org/2000/svg"><path d="M15 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V7Z"/><polyline points="14 2 14 8 20 8"/><line x1="12" x2="12" y1="18" y2="12"/><line x1="9" x2="15" y1="15" y2="15"/></svg>"#;
const FILE_TEXT_SVG: &[u8] = br#"<svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" xmlns="http://www.w3.org/2000/svg"><path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8Z"/><path d="M14 2v6h6M8 13h8M8 17h6"/></svg>"#;
const FOLDER_SVG: &[u8] = br#"<svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" xmlns="http://www.w3.org/2000/svg"><path d="M20 20a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.9a2 2 0 0 1-1.69-.9L9.6 3.9A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2Z"/></svg>"#;
const FOLDER_PLUS_SVG: &[u8] = br#"<svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" xmlns="http://www.w3.org/2000/svg"><path d="M12 10v6"/><path d="M9 13h6"/><path d="M20 20a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.9a2 2 0 0 1-1.69-.9L9.6 3.9A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2Z"/></svg>"#;
const FOLDER_SEARCH_SVG: &[u8] = br#"<svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" xmlns="http://www.w3.org/2000/svg"><path d="M11 20H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h3.93a2 2 0 0 1 1.66.9l.82 1.2a2 2 0 0 0 1.66.9H20a2 2 0 0 1 2 2v3.5"/><circle cx="17" cy="17" r="3"/><path d="m21 21-1.5-1.5"/></svg>"#;
const GIT_BRANCH_SVG: &[u8] = br#"<svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" xmlns="http://www.w3.org/2000/svg"><line x1="6" x2="6" y1="3" y2="15"/><circle cx="18" cy="6" r="3"/><circle cx="6" cy="18" r="3"/><path d="M18 9a9 9 0 0 1-9 9"/></svg>"#;
const GLOBE_SVG: &[u8] = br#"<svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" xmlns="http://www.w3.org/2000/svg"><circle cx="12" cy="12" r="10"/><path d="M12 2a14.5 14.5 0 0 0 0 20 14.5 14.5 0 0 0 0-20"/><path d="M2 12h20"/></svg>"#;
const HAMMER_SVG: &[u8] = br#"<svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" xmlns="http://www.w3.org/2000/svg"><path d="m15 12-8.373 8.373a1 1 0 1 1-3-3L12 9"/><path d="m18 15 4-4"/><path d="m21.5 11.5-1.914-1.914A2 2 0 0 1 19 8.172V7l-2.26-2.26a6 6 0 0 0-4.202-1.756L9 2.96l.92.82A6.18 6.18 0 0 1 12 8.4V10l2 2h1.172a2 2 0 0 1 1.414.586L18.5 14.5"/></svg>"#;
const HAND_METAL_SVG: &[u8] = br#"<svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" xmlns="http://www.w3.org/2000/svg"><path d="M18 12.5V10a2 2 0 0 0-2-2v0a2 2 0 0 0-2 2v1.4"/><path d="M14 11V9a2 2 0 0 0-2-2v0a2 2 0 0 0-2 2v2"/><path d="M10 10.5V5a2 2 0 0 0-2-2v0a2 2 0 0 0-2 2v9"/><path d="M18 10.5a2 2 0 1 1 4 0V14a8 8 0 0 1-8 8h-4a8 8 0 0 1-8-8 2 2 0 1 1 4 0"/></svg>"#;
const HELP_CIRCLE_SVG: &[u8] = br#"<svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" xmlns="http://www.w3.org/2000/svg"><circle cx="12" cy="12" r="10"/><path d="M9.09 9a3 3 0 0 1 5.83 1c0 2-3 3-3 3"/><path d="M12 17h.01"/></svg>"#;
const INBOX_SVG: &[u8] = br#"<svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" xmlns="http://www.w3.org/2000/svg"><polyline points="22 12 16 12 14 15 10 15 8 12 2 12"/><path d="M5.45 5.11 2 12v6a2 2 0 0 0 2 2h16a2 2 0 0 0 2-2v-6l-3.45-6.89A2 2 0 0 0 16.76 4H7.24a2 2 0 0 0-1.79 1.11z"/></svg>"#;
const KEYBOARD_SVG: &[u8] = br#"<svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" xmlns="http://www.w3.org/2000/svg"><path d="M10 8h.01"/><path d="M12 12h.01"/><path d="M14 8h.01"/><path d="M16 12h.01"/><path d="M18 8h.01"/><path d="M6 8h.01"/><path d="M7 16h10"/><path d="M8 12h.01"/><rect width="20" height="16" x="2" y="4" rx="2"/></svg>"#;
const LAYOUT_DASHBOARD_SVG: &[u8] = include_bytes!("../assets/icons/layout-dashboard.svg");
const LIST_CHECKS_SVG: &[u8] = br#"<svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" xmlns="http://www.w3.org/2000/svg"><path d="m3 17 2 2 4-4"/><path d="m3 7 2 2 4-4"/><path d="M13 6h8"/><path d="M13 12h8"/><path d="M13 18h8"/></svg>"#;
const LOADER_SVG: &[u8] = br#"<svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" xmlns="http://www.w3.org/2000/svg"><path d="M12 2v4"/><path d="m16.2 7.8 2.9-2.9"/><path d="M18 12h4"/><path d="m16.2 16.2 2.9 2.9"/><path d="M12 18v4"/><path d="m4.9 19.1 2.9-2.9"/><path d="M2 12h4"/><path d="m4.9 4.9 2.9 2.9"/></svg>"#;
const MESSAGE_QUESTION_SVG: &[u8] = br#"<svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" xmlns="http://www.w3.org/2000/svg"><path d="M7.9 20A9 9 0 1 0 4 16.1L2 22Z"/><path d="M9.09 9a3 3 0 0 1 5.83 1c0 2-3 3-3 3"/><path d="M12 17h.01"/></svg>"#;
const MESSAGE_SQUARE_SVG: &[u8] = br#"<svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" xmlns="http://www.w3.org/2000/svg"><path d="M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z"/></svg>"#;
const MIC_SVG: &[u8] = br#"<svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" xmlns="http://www.w3.org/2000/svg"><path d="M12 2a3 3 0 0 0-3 3v7a3 3 0 0 0 6 0V5a3 3 0 0 0-3-3Z"/><path d="M19 10v2a7 7 0 0 1-14 0v-2"/><line x1="12" x2="12" y1="19" y2="22"/></svg>"#;
const MINUS_SVG: &[u8] = br#"<svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" xmlns="http://www.w3.org/2000/svg"><path d="M5 12h14"/></svg>"#;
const PANEL_LEFT_SVG: &[u8] = include_bytes!("../assets/icons/panel-left.svg");
const PANEL_RIGHT_SVG: &[u8] = include_bytes!("../assets/icons/panel-right.svg");
const PAPERCLIP_SVG: &[u8] = br#"<svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" xmlns="http://www.w3.org/2000/svg"><path d="m21.44 11.05-9.19 9.19a6 6 0 0 1-8.49-8.49l8.57-8.57A4 4 0 1 1 18 8.84l-8.59 8.57a2 2 0 0 1-2.83-2.83l8.49-8.48"/></svg>"#;
const PEN_LINE_SVG: &[u8] = br#"<svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" xmlns="http://www.w3.org/2000/svg"><path d="M12 20h9"/><path d="M16.376 3.622a1 1 0 0 1 3.002 3.002L7.368 18.635a2 2 0 0 1-.855.506l-2.872.838a.5.5 0 0 1-.62-.62l.838-2.872a2 2 0 0 1 .506-.854z"/></svg>"#;
const PENCIL_SVG: &[u8] = br#"<svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" xmlns="http://www.w3.org/2000/svg"><path d="M17 3a2.85 2.83 0 1 1 4 4L7.5 20.5 2 22l1.5-5.5Z"/><path d="m15 5 4 4"/></svg>"#;
const PIN_SVG: &[u8] = br#"<svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" xmlns="http://www.w3.org/2000/svg"><path d="M12 17v5"/><path d="M9 10.76a2 2 0 0 1-1.11 1.79l-1.78.9A2 2 0 0 0 5 15.24V16a1 1 0 0 0 1 1h12a1 1 0 0 0 1-1v-.76a2 2 0 0 0-1.11-1.79l-1.78-.9A2 2 0 0 1 15 10.76V7a1 1 0 0 1 1-1 2 2 0 0 0 0-4H8a2 2 0 0 0 0 4 1 1 0 0 1 1 1z"/></svg>"#;
const PLUS_SVG: &[u8] = include_bytes!("../assets/icons/plus.svg");
const REFRESH_SVG: &[u8] = br#"<svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" xmlns="http://www.w3.org/2000/svg"><path d="M3 12a9 9 0 0 1 9-9 9.75 9.75 0 0 1 6.74 2.74L21 8"/><path d="M21 3v5h-5"/><path d="M21 12a9 9 0 0 1-9 9 9.75 9.75 0 0 1-6.74-2.74L3 16"/><path d="M8 16H3v5"/></svg>"#;
const ROTATE_CCW_SVG: &[u8] = br#"<svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" xmlns="http://www.w3.org/2000/svg"><path d="M3 12a9 9 0 1 0 9-9 9.75 9.75 0 0 0-6.74 2.74L3 8"/><path d="M3 3v5h5"/></svg>"#;
const SEARCH_SVG: &[u8] = br#"<svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" xmlns="http://www.w3.org/2000/svg"><circle cx="11" cy="11" r="8"/><path d="m21 21-4.3-4.3"/></svg>"#;
const SEND_SVG: &[u8] = br#"<svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" xmlns="http://www.w3.org/2000/svg"><path d="M14.536 21.686a.5.5 0 0 0 .937-.024l6.5-19a.496.496 0 0 0-.635-.635l-19 6.5a.5.5 0 0 0-.024.937l7.93 3.18a2 2 0 0 1 1.112 1.11z"/><path d="m21.854 2.147-10.94 10.939"/></svg>"#;
const SETTINGS_SVG: &[u8] = include_bytes!("../assets/icons/settings.svg");
const SHIELD_SVG: &[u8] = br#"<svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" xmlns="http://www.w3.org/2000/svg"><path d="M20 13c0 5-3.5 7.5-7.66 8.95a1 1 0 0 1-.67-.01C7.5 20.5 4 18 4 13V6a1 1 0 0 1 1-1c2 0 4.5-1.2 6.24-2.72a1.17 1.17 0 0 1 1.52 0C14.51 3.81 17 5 19 5a1 1 0 0 1 1 1z"/></svg>"#;
const SHIELD_CHECK_SVG: &[u8] = br#"<svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" xmlns="http://www.w3.org/2000/svg"><path d="M20 13c0 5-3.5 7.5-7.66 8.95a1 1 0 0 1-.67-.01C7.5 20.5 4 18 4 13V6a1 1 0 0 1 1-1c2 0 4.5-1.2 6.24-2.72a1.17 1.17 0 0 1 1.52 0C14.51 3.81 17 5 19 5a1 1 0 0 1 1 1z"/><path d="m9 12 2 2 4-4"/></svg>"#;
const SMARTPHONE_SVG: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect width="14" height="20" x="5" y="2" rx="2" ry="2"/><path d="M12 18h.01"/></svg>"#;
const SQUARE_SVG: &[u8] = br#"<svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" xmlns="http://www.w3.org/2000/svg"><rect width="14" height="14" x="5" y="5" rx="2"/></svg>"#;
const SQUARE_PEN_SVG: &[u8] = br#"<svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" xmlns="http://www.w3.org/2000/svg"><path d="M12 3H5a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h14a2 2 0 0 0 2-2v-7"/><path d="M18.375 2.625a1 1 0 0 1 3 3l-9.013 9.014a2 2 0 0 1-.853.505l-2.873.84a.5.5 0 0 1-.62-.62l.84-2.873a2 2 0 0 1 .506-.852z"/></svg>"#;
const SQUARE_TERMINAL_SVG: &[u8] = include_bytes!("../assets/icons/square-terminal.svg");
const SUN_SVG: &[u8] = br#"<svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" xmlns="http://www.w3.org/2000/svg"><circle cx="12" cy="12" r="4"/><path d="M12 2v2"/><path d="M12 20v2"/><path d="m4.93 4.93 1.41 1.41"/><path d="m17.66 17.66 1.41 1.41"/><path d="M2 12h2"/><path d="M20 12h2"/><path d="m6.34 17.66-1.41 1.41"/><path d="m19.07 4.93-1.41 1.41"/></svg>"#;
const TERMINAL_SVG: &[u8] = br#"<svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" xmlns="http://www.w3.org/2000/svg"><polyline points="4 17 10 11 4 5"/><line x1="12" x2="20" y1="19" y2="19"/></svg>"#;
const USER_PLUS_SVG: &[u8] = br#"<svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" xmlns="http://www.w3.org/2000/svg"><path d="M16 21v-2a4 4 0 0 0-4-4H6a4 4 0 0 0-4 4v2"/><circle cx="9" cy="7" r="4"/><path d="M19 8v6M22 11h-6"/></svg>"#;
const USERS_SVG: &[u8] = br#"<svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" xmlns="http://www.w3.org/2000/svg"><path d="M16 21v-2a4 4 0 0 0-4-4H6a4 4 0 0 0-4 4v2"/><circle cx="9" cy="7" r="4"/><path d="M22 21v-2a4 4 0 0 0-3-3.87"/><path d="M16 3.13a4 4 0 0 1 0 7.75"/></svg>"#;
const WRAP_TEXT_SVG: &[u8] = br#"<svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" xmlns="http://www.w3.org/2000/svg"><path d="M3 6h18"/><path d="M3 12h15a3 3 0 1 1 0 6h-4"/><path d="m16 16-2 2 2 2"/><path d="M3 18h7"/></svg>"#;
const WRENCH_SVG: &[u8] = br#"<svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" xmlns="http://www.w3.org/2000/svg"><path d="M14.7 6.3a1 1 0 0 0 0 1.4l1.6 1.6a1 1 0 0 0 1.4 0l3.77-3.77a6 6 0 0 1-7.94 7.94l-6.91 6.91a2.12 2.12 0 0 1-3-3l6.91-6.91a6 6 0 0 1 7.94-7.94l-3.76 3.76z"/></svg>"#;
const ZAP_SVG: &[u8] = br#"<svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" xmlns="http://www.w3.org/2000/svg"><path d="M4 14a1 1 0 0 1-.78-1.63l9.9-10.2a.5.5 0 0 1 .86.46l-1.92 6.02A1 1 0 0 0 13 10h7a1 1 0 0 1 .78 1.63l-9.9 10.2a.5.5 0 0 1-.86-.46l1.92-6.02A1 1 0 0 0 11 14z"/></svg>"#;

// GPUI caches SVG masks by path. Generate the outline and fill masks from the same shape.
macro_rules! star_svg {
    ($fill:literal) => {
        concat!(
            r#"<svg width="24" height="24" viewBox="0 0 24 24" xmlns="http://www.w3.org/2000/svg"><path stroke="currentColor" stroke-width="2" stroke-linejoin="round" fill=""#,
            $fill,
            r#"" d="M12 2.8l2.75 6.05 6.65.85-4.88 4.58 1.3 6.62L12 17.58l-5.82 3.32 1.3-6.62L2.6 9.7l6.65-.85L12 2.8Z"/></svg>"#
        )
        .as_bytes()
    };
}

const STAR_SVG: &[u8] = star_svg!("none");
const STAR_FILLED_SVG: &[u8] = star_svg!("currentColor");

/// Every `icons/*.svg` path served by [`load`].
pub const ALL_PATHS: &[&str] = &[
    TICKET,
    TRENDING_DOWN,
    TRENDING_UP,
    GAUGE,
    ACTIVITY,
    ALERT_TRIANGLE,
    ARCHIVE,
    ARROW_UP,
    BOOK_OPEN,
    BOOKMARK,
    BOT,
    BRAIN,
    CALENDAR,
    CAMERA,
    CHECK,
    CHEVRON_DOWN,
    CHEVRON_LEFT,
    CHEVRON_RIGHT,
    CIRCLE_ALERT,
    CLIPBOARD_LIST,
    CLOCK,
    CLOSE,
    COPY,
    DOWNLOAD,
    ELLIPSIS,
    EXTERNAL_LINK,
    EYE,
    FILE,
    FILE_DIFF,
    FILE_PLUS,
    FILE_TEXT,
    FOLDER,
    FOLDER_PLUS,
    FOLDER_SEARCH,
    GIT_BRANCH,
    GLOBE,
    HAMMER,
    HAND_METAL,
    HELP_CIRCLE,
    INBOX,
    KEYBOARD,
    LAYOUT_DASHBOARD,
    LIST_CHECKS,
    LOADER,
    MESSAGE_QUESTION,
    MESSAGE_SQUARE,
    MIC,
    MINUS,
    PANEL_LEFT,
    PANEL_RIGHT,
    PAPERCLIP,
    PEN_LINE,
    PENCIL,
    PIN,
    PLUS,
    REFRESH,
    ROTATE_CCW,
    SEARCH,
    SEND,
    SETTINGS,
    SHIELD,
    SHIELD_CHECK,
    SMARTPHONE,
    SQUARE,
    SQUARE_PEN,
    SQUARE_TERMINAL,
    STAR,
    STAR_FILLED,
    SUN,
    TERMINAL,
    USER_PLUS,
    USERS,
    WRAP_TEXT,
    WRENCH,
    ZAP,
];

/// The SVG for one of this crate's icon paths, or `None` for any other path.
pub fn load(path: &str) -> Option<&'static [u8]> {
    match path {
        GAUGE => Some(include_bytes!("../assets/icons/gauge.svg")),
        TRENDING_UP => Some(include_bytes!("../assets/icons/trending-up.svg")),
        TRENDING_DOWN => Some(include_bytes!("../assets/icons/trending-down.svg")),
        TICKET => Some(include_bytes!("../assets/icons/ticket.svg")),
        ACTIVITY => Some(ACTIVITY_SVG),
        ALERT_TRIANGLE => Some(ALERT_TRIANGLE_SVG),
        ARCHIVE => Some(ARCHIVE_SVG),
        ARROW_UP => Some(ARROW_UP_SVG),
        BOOK_OPEN => Some(BOOK_OPEN_SVG),
        BOOKMARK => Some(BOOKMARK_SVG),
        BOT => Some(BOT_SVG),
        BRAIN => Some(BRAIN_SVG),
        CALENDAR => Some(CALENDAR_SVG),
        CAMERA => Some(CAMERA_SVG),
        CHECK => Some(CHECK_SVG),
        CHEVRON_DOWN => Some(CHEVRON_DOWN_SVG),
        CHEVRON_LEFT => Some(CHEVRON_LEFT_SVG),
        CHEVRON_RIGHT => Some(CHEVRON_RIGHT_SVG),
        CIRCLE_ALERT => Some(CIRCLE_ALERT_SVG),
        CLIPBOARD_LIST => Some(CLIPBOARD_LIST_SVG),
        CLOCK => Some(CLOCK_SVG),
        CLOSE => Some(CLOSE_SVG),
        COPY => Some(COPY_SVG),
        DOWNLOAD => Some(DOWNLOAD_SVG),
        ELLIPSIS => Some(ELLIPSIS_SVG),
        EXTERNAL_LINK => Some(EXTERNAL_LINK_SVG),
        EYE => Some(EYE_SVG),
        FILE => Some(FILE_SVG),
        FILE_DIFF => Some(FILE_DIFF_SVG),
        FILE_PLUS => Some(FILE_PLUS_SVG),
        FILE_TEXT => Some(FILE_TEXT_SVG),
        FOLDER => Some(FOLDER_SVG),
        FOLDER_PLUS => Some(FOLDER_PLUS_SVG),
        FOLDER_SEARCH => Some(FOLDER_SEARCH_SVG),
        GIT_BRANCH => Some(GIT_BRANCH_SVG),
        GLOBE => Some(GLOBE_SVG),
        HAMMER => Some(HAMMER_SVG),
        HAND_METAL => Some(HAND_METAL_SVG),
        HELP_CIRCLE => Some(HELP_CIRCLE_SVG),
        INBOX => Some(INBOX_SVG),
        KEYBOARD => Some(KEYBOARD_SVG),
        LAYOUT_DASHBOARD => Some(LAYOUT_DASHBOARD_SVG),
        LIST_CHECKS => Some(LIST_CHECKS_SVG),
        LOADER => Some(LOADER_SVG),
        MESSAGE_QUESTION => Some(MESSAGE_QUESTION_SVG),
        MESSAGE_SQUARE => Some(MESSAGE_SQUARE_SVG),
        MIC => Some(MIC_SVG),
        MINUS => Some(MINUS_SVG),
        PANEL_LEFT => Some(PANEL_LEFT_SVG),
        PANEL_RIGHT => Some(PANEL_RIGHT_SVG),
        PAPERCLIP => Some(PAPERCLIP_SVG),
        PEN_LINE => Some(PEN_LINE_SVG),
        PENCIL => Some(PENCIL_SVG),
        PIN => Some(PIN_SVG),
        PLUS => Some(PLUS_SVG),
        REFRESH => Some(REFRESH_SVG),
        ROTATE_CCW => Some(ROTATE_CCW_SVG),
        SEARCH => Some(SEARCH_SVG),
        SEND => Some(SEND_SVG),
        SETTINGS => Some(SETTINGS_SVG),
        SHIELD => Some(SHIELD_SVG),
        SHIELD_CHECK => Some(SHIELD_CHECK_SVG),
        SMARTPHONE => Some(SMARTPHONE_SVG),
        SQUARE => Some(SQUARE_SVG),
        SQUARE_PEN => Some(SQUARE_PEN_SVG),
        SQUARE_TERMINAL => Some(SQUARE_TERMINAL_SVG),
        STAR => Some(STAR_SVG),
        STAR_FILLED => Some(STAR_FILLED_SVG),
        SUN => Some(SUN_SVG),
        TERMINAL => Some(TERMINAL_SVG),
        USER_PLUS => Some(USER_PLUS_SVG),
        USERS => Some(USERS_SVG),
        WRAP_TEXT => Some(WRAP_TEXT_SVG),
        WRENCH => Some(WRENCH_SVG),
        ZAP => Some(ZAP_SVG),
        _ if path.starts_with("bruhs-ui/language-icons/") => crate::language_icons::load(path),
        _ => IconName::ALL
            .into_iter()
            .find(|name| name.path() == path)
            .map(IconName::svg),
    }
}

/// A named icon that [`Icon`] draws.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IconName {
    Close,
    Delete,
    Eye,
    EyeOff,
    Play,
    Trash,
    TriangleAlert,
}

impl IconName {
    const ALL: [IconName; 7] = [
        IconName::Close,
        IconName::Delete,
        IconName::Eye,
        IconName::EyeOff,
        IconName::Play,
        IconName::Trash,
        IconName::TriangleAlert,
    ];

    /// The asset path [`load`] serves it at.
    pub fn path(self) -> &'static str {
        match self {
            IconName::Close => "bruhs-ui/icons/close.svg",
            IconName::Delete => "bruhs-ui/icons/delete.svg",
            IconName::Eye => "bruhs-ui/icons/eye.svg",
            IconName::EyeOff => "bruhs-ui/icons/eye-off.svg",
            IconName::Play => "bruhs-ui/icons/play.svg",
            IconName::Trash => "bruhs-ui/icons/trash-2.svg",
            IconName::TriangleAlert => "bruhs-ui/icons/triangle-alert.svg",
        }
    }

    fn svg(self) -> &'static [u8] {
        match self {
            IconName::Close => include_bytes!("../assets/icons/close.svg"),
            IconName::Delete => include_bytes!("../assets/icons/delete.svg"),
            IconName::Eye => include_bytes!("../assets/icons/eye.svg"),
            IconName::EyeOff => include_bytes!("../assets/icons/eye-off.svg"),
            IconName::Play => include_bytes!("../assets/icons/play.svg"),
            IconName::Trash => include_bytes!("../assets/icons/trash-2.svg"),
            IconName::TriangleAlert => include_bytes!("../assets/icons/triangle-alert.svg"),
        }
    }
}

/// An icon drawn in the surrounding text's color, at the text size unless
/// [`Icon::size`] says otherwise.
#[derive(IntoElement)]
pub struct Icon {
    name: IconName,
    size: Option<Pixels>,
    color: Option<Hsla>,
}

impl Icon {
    pub fn new(name: IconName) -> Self {
        Self {
            name,
            size: None,
            color: None,
        }
    }

    pub fn size(mut self, size: Pixels) -> Self {
        self.size = Some(size);
        self
    }

    /// Draw in `color` instead of the text color.
    pub fn color(mut self, color: Hsla) -> Self {
        self.color = Some(color);
        self
    }
}

impl RenderOnce for Icon {
    fn render(self, window: &mut Window, _: &mut App) -> impl IntoElement {
        let text = window.text_style();
        // An svg never takes its color or size from the text around it.
        let size = self
            .size
            .unwrap_or_else(|| text.font_size.to_pixels(window.rem_size()));
        svg()
            .path(self.name.path())
            .flex_none()
            .size(size)
            .text_color(self.color.unwrap_or(text.color))
    }
}

#[cfg(test)]
mod tests {
    use super::{load, ALL_PATHS};

    #[test]
    fn every_icon_path_loads_an_svg() {
        for &path in ALL_PATHS {
            let svg = load(path).unwrap_or_default();
            assert!(svg.starts_with(b"<svg"), "{path}");
        }
        for name in super::IconName::ALL {
            let svg = load(name.path()).unwrap_or_default();
            assert!(svg.starts_with(b"<svg"), "{name:?}");
        }
        assert_eq!(load("icons/unknown.svg"), None);
    }
}

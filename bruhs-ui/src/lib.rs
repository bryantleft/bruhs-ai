//! Bruhs-styled GPUI components.
//!
//! The Bruhs design system (fruit-token colors, Monaspace type, tone-and-ring
//! elevation, no drop shadows) drawn with [gpui]. Everything here is
//! app-agnostic: a component takes its value, state and handlers from the
//! caller and never reaches into the app that hosts it.
//!
//! The app's whole UI toolkit comes from this crate: text fields
//! ([`input`]), rich text ([`text`]), scrolling ([`scroll`]), panels
//! ([`resizable`]), [`tooltip`], [`checkbox`], zoom and the window [`Root`].
//! All of it is written on gpui's public API; nothing comes from another UI
//! library.
//!
//! * [`theme`] — design tokens and the app theme (`cx.theme()`).
//! * [`press`] — focus, naming and one action per click, Enter or Space.
//! * [`button`] — the web's `Button` variants and sizes.
//! * [`switch`] — the web's `Switch`.
//! * [`dropdown`] — a trigger and inline listbox with keyboard navigation.
//! * [`menu_popover`] — a menu placed under (or above) the button that
//!   opened it.
//! * [`page_header`], [`copy_feedback`] — small shared pieces.
//! * [`native_blur`] — the web's `backdrop-blur-sm` behind modal scrims
//!   (macOS; a no-op elsewhere).
//! * [`motion`], [`zoom`] — the reduced-motion preference and page zoom steps.
//!
//! Components that draw an icon load it by path from the app's
//! [`gpui::AssetSource`]; serve [`icons::load`] from yours.

pub mod actions;
pub mod animated_number;
pub mod banner;
pub mod button;
pub mod card;
pub mod checkbox;
pub mod copy_feedback;
pub mod dialog;
pub mod dropdown;
pub mod icons;
pub mod input;
pub mod kbd;
mod language_icons;
pub mod line_chart;
pub mod menu_popover;
pub mod motion;
pub mod native_blur;
pub mod page_header;
pub mod press;
pub mod resizable;
pub mod resize_handle;
pub mod root;
pub mod scroll;
pub mod switch;
pub mod tabs;
pub mod text;
pub mod text_field;
pub mod theme;
pub mod title_bar;
pub mod tooltip;
pub mod zoom;

pub use icons::{Icon, IconName};
pub use input::{Sizable, Size, WindowExt};
pub use root::{FocusTrapElement, Root};
pub use theme::{ActiveTheme, Theme, ThemeMode};

/// Set up bruhs-ui for an app: the theme, and the key bindings of the
/// window root, text fields and text selection.
pub fn init(cx: &mut gpui::App) {
    cx.default_global::<Theme>();
    root::init(cx);
    input::init(cx);
    text::init(cx);
}
pub use title_bar::TitleBar;

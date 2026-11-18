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
//! Some of these still wrap the vendored GPUI Kit; each is being replaced by
//! our own, behind the same paths (`docs/ui-kit-replacement.md`).
//!
//! * [`theme`] — design tokens and the gpui-component theme they configure.
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
pub mod button;
pub mod checkbox;
pub mod copy_feedback;
pub mod dropdown;
pub mod icons;
pub mod input;
pub mod menu_popover;
pub mod motion;
pub mod native_blur;
pub mod page_header;
pub mod press;
pub mod resizable;
pub mod resize_handle;
pub mod scroll;
pub mod switch;
pub mod text;
pub mod text_field;
pub mod theme;
pub mod tooltip;
pub mod zoom;

// The kit pieces an app needs at its root. These move into bruhs-ui itself,
// phase by phase (docs/ui-kit-replacement.md); the paths stay.
pub use gpui_component::{
    init, ActiveTheme, Disableable, FocusTrapElement, Icon, IconName, Root, RopeExt, Sizable,
    Theme, ThemeMode, ThemeTokens, TitleBar, WindowExt,
};

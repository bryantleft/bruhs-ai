//! Bruhs-styled GPUI components.
//!
//! The Bruhs design system (fruit-token colors, Monaspace type, tone-and-ring
//! elevation, no drop shadows) drawn with [gpui] on top of
//! [gpui-component](gpui_component). Everything here is app-agnostic: a
//! component takes its value, state and handlers from the caller and never
//! reaches into the app that hosts it.
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
pub mod copy_feedback;
pub mod dropdown;
pub mod icons;
pub mod menu_popover;
pub mod motion;
pub mod native_blur;
pub mod page_header;
pub mod press;
pub mod switch;
pub mod theme;
pub mod zoom;

//! Bruhs design tokens, now in `bruhs-ui`.

pub use bruhs_ui::theme::*;

/// Minimum space inside a Settings row for its label and control to sit side by side.
pub const SETTINGS_INLINE_CONTENT_WIDTH_PX: f32 = 512.0;

// Resolve a device's selected typeface at each styled element.
pub const UI_FONT_FAMILY: FontFace = FontFace::Interface;
pub const UI_SMALL_FONT_FAMILY: FontFace = FontFace::Small;
pub const HEADING_FONT_FAMILY: FontFace = FontFace::Heading;
pub const CODE_FONT_FAMILY: FontFace = FontFace::Code;

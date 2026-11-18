//! How a [`super::TextView`] draws its blocks. The defaults are the web's
//! chat prose (`.prose-msg`).

use std::sync::Arc;

use gpui::{rems, Hsla, Pixels, Rems, StyleRefinement, Styled as _};

use crate::theme;
use crate::zoom::px;

/// The text view's look. Body text takes the surrounding text style (font,
/// size, color); everything else comes from here.
#[derive(Clone)]
pub struct TextViewStyle {
    /// The gap between blocks (web: `.prose-msg > * + *`).
    pub paragraph_gap: Rems,
    /// The body line height, as a multiple of the font size.
    pub line_height: f32,
    heading_font_size: Arc<dyn Fn(u8, Pixels) -> Pixels + Send + Sync>,
    /// Refines the code block's box and its text (family, size, color).
    pub code_block: StyleRefinement,
    pub code_font_family: gpui::SharedString,
    pub heading_color: Hsla,
    pub strong_color: Hsla,
    pub muted_color: Hsla,
    pub link_color: Hsla,
    pub inline_code_color: Hsla,
    pub inline_code_background: Hsla,
    pub quote_color: Hsla,
    pub border_color: Hsla,
    pub table_text_color: Hsla,
    pub table_head_color: Hsla,
    pub selection_color: Hsla,
}

impl Default for TextViewStyle {
    fn default() -> Self {
        Self {
            paragraph_gap: rems(0.65),
            line_height: 1.55,
            // h1 1.125rem, h2 1rem, the rest 0.875rem.
            heading_font_size: Arc::new(|level, _| match level {
                1 => px(18.0),
                2 => px(16.0),
                _ => px(14.0),
            }),
            // `.prose-msg pre`: longan-950, a longan-800 border, grape
            // corners, 10px 12px padding, 0.75rem text at 1.55.
            code_block: StyleRefinement::default()
                .bg(theme::longan_950())
                .border_1()
                .border_color(theme::longan_800())
                .rounded(px(8.0))
                .px(px(12.0))
                .py(px(10.0))
                .text_size(px(12.0))
                .line_height(gpui::relative(1.55))
                .font_family(theme::CODE_FONT_FAMILY)
                .text_color(theme::rambutan_300()),
            code_font_family: theme::CODE_FONT_FAMILY.into(),
            heading_color: theme::rambutan_100(),
            strong_color: theme::rambutan_200(),
            muted_color: theme::rambutan_500(),
            link_color: theme::persimmon_300(),
            inline_code_color: theme::rambutan_200(),
            // The web's chip is longan-900/50 inside a longan-800/50 border;
            // a run cannot draw a border, so the fill takes the border's tone.
            inline_code_background: theme::longan_800().opacity(0.6),
            quote_color: theme::rambutan_400(),
            border_color: theme::longan_800(),
            table_text_color: theme::rambutan_300(),
            table_head_color: theme::rambutan_400(),
            selection_color: theme::persimmon_500().opacity(0.3),
        }
    }
}

impl TextViewStyle {
    pub fn paragraph_gap(mut self, gap: Rems) -> Self {
        self.paragraph_gap = gap;
        self
    }

    /// The font size of each heading level (1-6), given the body size.
    pub fn heading_font_size(
        mut self,
        size: impl Fn(u8, Pixels) -> Pixels + Send + Sync + 'static,
    ) -> Self {
        self.heading_font_size = Arc::new(size);
        self
    }

    pub fn code_block(mut self, style: StyleRefinement) -> Self {
        self.code_block = style;
        self
    }

    pub(super) fn heading_size(&self, level: u8, base: Pixels) -> Pixels {
        (self.heading_font_size)(level, base)
    }
}

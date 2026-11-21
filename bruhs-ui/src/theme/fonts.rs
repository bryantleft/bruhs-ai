//! Resolve semantic font roles when rendering. An unset selection keeps the bundled face.
use super::{CODE_FONT_FAMILY, HEADING_FONT_FAMILY, UI_FONT_FAMILY, UI_SMALL_FONT_FAMILY};
use gpui::SharedString;
use std::cell::RefCell;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FontFace {
    Interface,
    Small,
    Heading,
    Prompt,
    Code,
    Terminal,
}

thread_local! {
    static FAMILIES: RefCell<[Option<SharedString>;4]> = const {RefCell::new([None,None,None,None])};
}

pub fn set_font_families(interface: &str, prompt: &str, code: &str, terminal: &str) {
    FAMILIES.with_borrow_mut(|fonts| {
        *fonts = [interface, prompt, code, terminal]
            .map(|name| (!name.is_empty()).then(|| SharedString::from(name.to_owned())))
    });
}

impl FontFace {
    pub fn with_default(self, default: &'static str) -> SharedString {
        let index = match self {
            Self::Interface | Self::Small | Self::Heading => 0,
            Self::Prompt => 1,
            Self::Code => 2,
            Self::Terminal => 3,
        };
        FAMILIES.with_borrow(|fonts| fonts[index].clone().unwrap_or_else(|| default.into()))
    }
}
impl From<FontFace> for SharedString {
    fn from(face: FontFace) -> Self {
        face.with_default(match face {
            FontFace::Interface => UI_FONT_FAMILY,
            FontFace::Small => UI_SMALL_FONT_FAMILY,
            FontFace::Heading => HEADING_FONT_FAMILY,
            FontFace::Code | FontFace::Prompt | FontFace::Terminal => CODE_FONT_FAMILY,
        })
    }
}

thread_local! {
    static WRAP_CODE:std::cell::Cell<bool>=const {std::cell::Cell::new(false)};
    static SHOW_CONTEXT:std::cell::Cell<bool>=const {std::cell::Cell::new(true)};
}
pub fn set_text_layout(wrap: bool, show_context: bool) {
    WRAP_CODE.set(wrap);
    SHOW_CONTEXT.set(show_context);
}
pub fn wrap_code() -> bool {
    WRAP_CODE.get()
}
pub fn show_composer_context() -> bool {
    SHOW_CONTEXT.get()
}

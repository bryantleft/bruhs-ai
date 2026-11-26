//! Compact, theme-aware language marks for fenced Markdown blocks.
use crate::{
    theme::{self, ActiveTheme},
    zoom::px,
};
use gpui::*;

#[derive(Clone, Copy)]
enum Color {
    Neutral,
    Green,
    Blue,
    Purple,
    Cyan,
    Magenta,
    Pink,
    Orange,
    Yellow,
    Red,
}

impl Color {
    fn rgb(self, dark: bool) -> Rgba {
        let (light, night) = match self {
            Self::Neutral => (0x84848a, 0xadadb1),
            Self::Green => (0x199f43, 0x5ecc71),
            Self::Blue => (0x1a85d4, 0x69b1ff),
            Self::Purple => (0x693acf, 0x9d6afb),
            Self::Cyan => (0x1ca1c7, 0x68cdf2),
            Self::Magenta => (0xa631be, 0xd568ea),
            Self::Pink => (0xd32a61, 0xff678d),
            Self::Orange => (0xd47628, 0xffa359),
            Self::Yellow => (0xd5a910, 0xffd452),
            Self::Red => (0xd52c36, 0xff6762),
        };
        rgb(if dark { night } else { light })
    }
}

const LANGUAGES: &[(&str, &str, Color)] = &[
    ("", "TXT", Color::Neutral),
    ("asm", "ASM", Color::Neutral),
    ("bash", "SH", Color::Green),
    ("c", "C", Color::Blue),
    ("cpp", "C++", Color::Blue),
    ("csharp", "C#", Color::Purple),
    ("css", "CSS", Color::Purple),
    ("dart", "DA", Color::Cyan),
    ("diff", "±", Color::Green),
    ("dockerfile", "DK", Color::Blue),
    ("elixir", "EX", Color::Magenta),
    ("go", "GO", Color::Cyan),
    ("graphql", "GQL", Color::Pink),
    ("haskell", "HS", Color::Purple),
    ("html", "HT", Color::Orange),
    ("java", "JV", Color::Orange),
    ("javascript", "JS", Color::Yellow),
    ("json", "{}", Color::Orange),
    ("jsx", "JSX", Color::Cyan),
    ("kotlin", "KT", Color::Purple),
    ("lua", "LU", Color::Blue),
    ("markdown", "MD", Color::Green),
    ("php", "PHP", Color::Purple),
    ("powershell", "PS", Color::Blue),
    ("python", "PY", Color::Blue),
    ("r", "R", Color::Blue),
    ("ruby", "RB", Color::Red),
    ("rust", "RS", Color::Orange),
    ("scala", "SC", Color::Red),
    ("scss", "SCSS", Color::Pink),
    ("sql", "SQL", Color::Magenta),
    ("svelte", "SV", Color::Red),
    ("swift", "SW", Color::Orange),
    ("toml", "TL", Color::Neutral),
    ("tsx", "TSX", Color::Cyan),
    ("typescript", "TS", Color::Blue),
    ("vue", "VU", Color::Green),
    ("xml", "XML", Color::Orange),
    ("yaml", "YML", Color::Red),
    ("zig", "ZG", Color::Orange),
];

pub(crate) fn element(language: &str) -> AnyElement {
    let (label, color) = LANGUAGES
        .iter()
        .find(|(id, _, _)| *id == language)
        .map_or(("<>", Color::Neutral), |(_, label, color)| (*label, *color));
    LanguageMark(label, color).into_any_element()
}

#[derive(IntoElement)]
struct LanguageMark(&'static str, Color);

impl RenderOnce for LanguageMark {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        div()
            .flex_none()
            .h(px(14.))
            .min_w(px(16.))
            .px(px(2.))
            .rounded(px(3.))
            .bg(theme::longan_700())
            .flex()
            .items_center()
            .justify_center()
            .text_size(px(8.))
            .line_height(px(12.))
            .font_family(theme::UI_SMALL_FONT_FAMILY)
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(self.1.rgb(cx.theme().is_dark()))
            .child(self.0)
    }
}

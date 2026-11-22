//! File-type artwork used on fenced Markdown blocks.
use crate::{theme::ActiveTheme, zoom::px};
use gpui::*;

struct Mark {
    path: &'static str,
    bytes: &'static [u8],
    color: Color,
}

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

macro_rules! mark {
    ($file:literal, $color:ident) => {
        Mark {
            path: concat!("bruhs-ui/language-icons/", $file, ".svg"),
            bytes: include_bytes!(concat!("../assets/language-icons/", $file, ".svg")),
            color: Color::$color,
        }
    };
}

const GENERIC: Mark = mark!("default", Neutral);
const LANGUAGES: &[(&str, Mark)] = &[
    ("", mark!("text", Neutral)),
    ("asm", GENERIC),
    ("bash", mark!("bash", Green)),
    ("c", mark!("c", Blue)),
    ("cpp", mark!("cpp", Blue)),
    ("csharp", mark!("csharp", Purple)),
    ("css", mark!("css", Purple)),
    ("dart", mark!("dart", Cyan)),
    ("diff", mark!("diff", Green)),
    ("dockerfile", mark!("docker", Blue)),
    ("elixir", mark!("elixir", Magenta)),
    ("go", mark!("go", Cyan)),
    ("graphql", mark!("graphql", Pink)),
    ("haskell", mark!("haskell", Purple)),
    ("html", mark!("html", Orange)),
    ("java", mark!("java", Orange)),
    ("javascript", mark!("javascript", Yellow)),
    ("json", mark!("json", Orange)),
    ("jsx", mark!("react", Cyan)),
    ("kotlin", mark!("kotlin", Purple)),
    ("lua", mark!("lua", Blue)),
    ("markdown", mark!("markdown", Green)),
    ("php", mark!("php", Purple)),
    ("powershell", mark!("powershell", Blue)),
    ("python", mark!("python", Blue)),
    ("r", mark!("r", Blue)),
    ("ruby", mark!("ruby", Red)),
    ("rust", mark!("rust", Orange)),
    ("scala", mark!("scala", Red)),
    ("scss", mark!("sass", Pink)),
    ("sql", mark!("database", Magenta)),
    ("svelte", mark!("svelte", Red)),
    ("swift", mark!("swift", Orange)),
    ("toml", mark!("toml", Neutral)),
    ("tsx", mark!("react", Cyan)),
    ("typescript", mark!("typescript", Blue)),
    ("vue", mark!("vue", Green)),
    ("xml", mark!("xml", Orange)),
    ("yaml", mark!("yml", Red)),
    ("zig", mark!("zig", Orange)),
];

pub(crate) fn load(path: &str) -> Option<&'static [u8]> {
    LANGUAGES
        .iter()
        .find(|(_, mark)| mark.path == path)
        .map(|(_, mark)| mark.bytes)
}

#[derive(IntoElement)]
struct LanguageIcon(&'static Mark);

impl RenderOnce for LanguageIcon {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        svg()
            .path(self.0.path)
            .size(px(14.))
            .flex_none()
            .text_color(self.0.color.rgb(cx.theme().is_dark()))
    }
}

pub(crate) fn element(language: &str) -> AnyElement {
    let mark = LANGUAGES
        .iter()
        .find(|(id, _)| *id == language)
        .map_or(&GENERIC, |(_, mark)| mark);
    LanguageIcon(mark).into_any_element()
}

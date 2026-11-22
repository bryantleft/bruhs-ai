//! Resolve background declarations into independent CSS image and geometry lists.
use super::gradient::Gradient;
use cssparser::{Parser, ParserInput, Token};
use std::collections::BTreeMap;
use std::sync::Arc;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Length {
    Auto,
    Px(i32),
    FromEndPx(i32),
    Percent(i32),
}
impl Length {
    pub(super) fn parse(s: &str) -> Option<Self> {
        if s == "0" {
            return Some(Self::Px(0));
        }
        if s == "auto" {
            return Some(Self::Auto);
        }
        let (value, percent) = s
            .strip_suffix('%')
            .map(|v| (v, true))
            .or_else(|| s.strip_suffix("px").map(|v| (v, false)))?;
        let value = value.parse::<f32>().ok()?;
        if !value.is_finite() {
            return None;
        }
        let units = (value * 100.).round() as i32;
        Some(if percent {
            Self::Percent(units)
        } else {
            Self::Px(units)
        })
    }
    pub(super) fn value(self, extent: f32, zoom: f32) -> Option<f32> {
        match self {
            Self::Auto => None,
            Self::Px(n) => Some(n as f32 * zoom / 100.),
            Self::FromEndPx(n) => Some(extent - n as f32 * zoom / 100.),
            Self::Percent(n) => Some(extent * n as f32 / 10000.),
        }
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Sizing {
    Cover,
    Contain,
    Explicit(Length, Length),
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum Source {
    None,
    Companion,
    Gradient(Arc<Gradient>),
}

impl Source {
    fn parse(value: &str) -> Option<Self> {
        match value {
            IMAGE => Some(Self::Companion),
            "none" => Some(Self::None),
            _ => Gradient::parse(value).map(|g| Self::Gradient(Arc::new(g))),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct Geometry {
    pub sizing: Sizing,
    pub position: (Length, Length),
    pub repeat: (bool, bool),
}

impl Default for Geometry {
    fn default() -> Self {
        Self {
            sizing: Sizing::Explicit(Length::Auto, Length::Auto),
            position: (Length::Percent(0), Length::Percent(0)),
            repeat: (true, true),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Style {
    pub(super) images: Vec<Source>,
    sizes: Vec<Sizing>,
    positions: Vec<(Length, Length)>,
    repeats: Vec<(bool, bool)>,
    pub(in crate::views::workspace::appearance) color: Option<[u8; 4]>,
}

impl Default for Style {
    fn default() -> Self {
        let initial = Geometry::default();
        Self {
            images: vec![Source::None],
            sizes: vec![initial.sizing],
            positions: vec![initial.position],
            repeats: vec![initial.repeat],
            color: None,
        }
    }
}

// Use CSS tokens to keep nested functions (including slash-alpha colors) together.
pub(super) fn words(input: &str) -> Option<Vec<&str>> {
    let mut source = ParserInput::new(input);
    let mut p = Parser::new(&mut source);
    let mut out = Vec::new();
    while !p.is_exhausted() {
        p.skip_whitespace();
        let start = p.position();
        match p.next().ok()? {
            Token::Function(_) | Token::ParenthesisBlock => {
                p.parse_nested_block(|p| {
                    p.expect_no_error_token()
                        .map_err(cssparser::ParseError::<()>::from)
                })
                .ok()?;
            }
            _ => {}
        }
        out.push(p.slice_from(start));
    }
    Some(out)
}

pub(super) fn comma_list(input: &str) -> Option<Vec<&str>> {
    let mut source = ParserInput::new(input);
    Parser::new(&mut source)
        .parse_comma_separated(|p| {
            let start = p.position();
            p.expect_no_error_token()?;
            Ok::<_, cssparser::ParseError<'_, ()>>(p.slice_from(start).trim())
        })
        .ok()
}

fn size(tokens: &[&str]) -> Option<Sizing> {
    match tokens {
        ["cover"] => Some(Sizing::Cover),
        ["contain"] => Some(Sizing::Contain),
        [x] => Some(Sizing::Explicit(Length::parse(x)?, Length::Auto)),
        [x, y] => Some(Sizing::Explicit(Length::parse(x)?, Length::parse(y)?)),
        _ => None,
    }
}
pub(super) fn position(tokens: &[&str]) -> Option<(Length, Length)> {
    // Three- and four-value positions attach a length to the preceding edge.
    if tokens.len() > 2 {
        let mut axes = [None, None];
        let mut centered = false;
        let mut tokens = tokens.iter().copied().peekable();
        while let Some(edge) = tokens.next() {
            let axis = match edge {
                "left" | "right" => 0,
                "top" | "bottom" => 1,
                "center" if !centered => {
                    centered = true;
                    continue;
                }
                _ => return None,
            };
            let offset = tokens.peek().and_then(|s| Length::parse(s));
            if offset.is_some() {
                tokens.next();
            }
            let from_end = matches!(edge, "right" | "bottom");
            let value = match offset.unwrap_or(Length::Percent(0)) {
                Length::Percent(n) if from_end => Length::Percent(10000_i32.saturating_sub(n)),
                Length::Px(n) if from_end => Length::FromEndPx(n),
                l @ (Length::Percent(_) | Length::Px(_)) => l,
                _ => return None,
            };
            if axes[axis].replace(value).is_some() {
                return None;
            }
        }
        if centered {
            let missing = axes.iter_mut().find(|axis| axis.is_none())?;
            *missing = Some(Length::Percent(5000));
        }
        return Some((axes[0]?, axes[1]?));
    }
    let (x, y) = match tokens {
        ["top" | "bottom"] => ("center", tokens[0]),
        [x] => (*x, "center"),
        [y @ ("top" | "bottom"), x] | [y @ "center", x @ ("left" | "right")] => (*x, *y),
        [x, y] => (*x, *y),
        _ => return None,
    };
    let axis = |s, first, last| {
        if s == first {
            Some(Length::Percent(0))
        } else if s == "center" {
            Some(Length::Percent(5000))
        } else if s == last {
            Some(Length::Percent(10000))
        } else {
            Length::parse(s).filter(|v| *v != Length::Auto)
        }
    };
    Some((axis(x, "left", "right")?, axis(y, "top", "bottom")?))
}
fn repeat(tokens: &[&str]) -> Option<(bool, bool)> {
    match tokens {
        ["repeat"] => Some((true, true)),
        ["no-repeat"] => Some((false, false)),
        ["repeat-x"] => Some((true, false)),
        ["repeat-y"] => Some((false, true)),
        [x @ ("repeat" | "no-repeat"), y @ ("repeat" | "no-repeat")] => {
            Some((*x == "repeat", *y == "repeat"))
        }
        _ => None,
    }
}

// Keep the companion resource symbolic. Imported CSS never supplies a filesystem path.
const IMAGE: &str = "\0theme-background";
fn substitute(
    raw: &str,
    vars: &BTreeMap<String, String>,
    chain: &mut Vec<String>,
    budget: &mut usize,
) -> Option<String> {
    if chain.len() >= 32 {
        return None;
    }
    *budget = budget.checked_sub(1)?;
    let mut rest = raw;
    let mut out = String::new();
    while let Some(i) = rest.find("var(") {
        out.push_str(&rest[..i]);
        let tail = &rest[i + 4..];
        let mut depth = 0;
        let end = tail.char_indices().find_map(|(i, c)| match c {
            '(' => {
                depth += 1;
                None
            }
            ')' if depth == 0 => Some(i),
            ')' => {
                depth -= 1;
                None
            }
            _ => None,
        })?;
        let arg = &tail[..end];
        let (name, fallback) = arg
            .split_once(',')
            .map_or((arg, None), |(n, v)| (n, Some(v)));
        let name = name.trim();
        if !name.starts_with("--") {
            return None;
        }
        let value = if name == "--theme-background-image" {
            Some(IMAGE.to_owned())
        } else if chain.iter().any(|n| n == name) {
            None
        } else {
            vars.get(name).and_then(|v| {
                chain.push(name.into());
                let v = substitute(v, vars, chain, budget);
                chain.pop();
                v
            })
        };
        out.push_str(&value.or_else(|| fallback.and_then(|v| substitute(v, vars, chain, budget)))?);
        if out.len() > 50000 {
            return None;
        }
        rest = &tail[end + 1..];
    }
    out.push_str(rest);
    Some(out)
}

impl Style {
    pub(in crate::views::workspace::appearance) fn active(&self) -> bool {
        self.images.iter().any(|s| *s != Source::None)
    }

    pub(super) fn geometry(&self, i: usize) -> Geometry {
        Geometry {
            sizing: self.sizes[i % self.sizes.len()],
            position: self.positions[i % self.positions.len()],
            repeat: self.repeats[i % self.repeats.len()],
        }
    }

    pub(in crate::views::workspace::appearance) fn declaration(
        &mut self,
        name: &str,
        raw: &str,
        vars: &BTreeMap<String, String>,
    ) {
        if !name
            .get(..10)
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case("background"))
        {
            return;
        }
        let Some(value) = substitute(raw, vars, &mut Vec::new(), &mut 512) else {
            return;
        };
        let value = value.trim().to_ascii_lowercase();
        if name.eq_ignore_ascii_case("background-color") {
            if let Ok(color) = csscolorparser::parse(&value) {
                self.color = Some(color.to_rgba8());
            }
            return;
        }
        let Some(parts) = comma_list(&value) else {
            return;
        };
        match name.to_ascii_lowercase().as_str() {
            "background-image" => {
                if let Some(list) = parts.iter().map(|s| Source::parse(s)).collect() {
                    self.images = list;
                }
            }
            "background-size" => {
                if let Some(list) = parts.iter().map(|s| size(&words(s)?)).collect() {
                    self.sizes = list;
                }
            }
            "background-position" => {
                if let Some(list) = parts.iter().map(|s| position(&words(s)?)).collect() {
                    self.positions = list;
                }
            }
            "background-repeat" => {
                if let Some(list) = parts.iter().map(|s| repeat(&words(s)?)).collect() {
                    self.repeats = list;
                }
            }
            "background" => {
                let mut next = Style {
                    images: Vec::new(),
                    sizes: Vec::new(),
                    positions: Vec::new(),
                    repeats: Vec::new(),
                    color: Some([0; 4]),
                };
                for (i, part) in parts.iter().enumerate() {
                    let Some((image, geometry, color)) = shorthand(part) else {
                        return;
                    };
                    if let Some(color) = color {
                        if i + 1 != parts.len() {
                            return;
                        }
                        next.color = Some(color);
                    }
                    next.images.push(image);
                    next.sizes.push(geometry.sizing);
                    next.positions.push(geometry.position);
                    next.repeats.push(geometry.repeat);
                }
                *self = next;
            }
            _ => {}
        }
    }
}

fn shorthand(input: &str) -> Option<(Source, Geometry, Option<[u8; 4]>)> {
    let mut geometry = Geometry::default();
    let mut position_words = Vec::new();
    let mut size_words = Vec::new();
    let mut repeat_words = Vec::new();
    let mut after_slash = false;
    let mut image = None;
    let mut color = None;
    let tokens = words(input)?;
    if tokens.is_empty() {
        return None;
    }
    for word in tokens {
        if let Some(source) = Source::parse(word) {
            if image.replace(source).is_some() {
                return None;
            }
        } else if word == "/" && !after_slash && !position_words.is_empty() {
            after_slash = true;
        } else if matches!(word, "repeat" | "no-repeat" | "repeat-x" | "repeat-y") {
            repeat_words.push(word);
        } else if let Ok(c) = csscolorparser::parse(word) {
            if color.replace(c.to_rgba8()).is_some() {
                return None;
            }
        } else if after_slash {
            size_words.push(word);
        } else {
            position_words.push(word);
        }
    }
    if !position_words.is_empty() {
        geometry.position = position(&position_words)?;
    }
    if after_slash {
        geometry.sizing = size(&size_words)?;
    }
    if !repeat_words.is_empty() {
        geometry.repeat = repeat(&repeat_words)?;
    }
    Some((image.unwrap_or(Source::None), geometry, color))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layer_lists_cycle_without_losing_unmatched_longhands() {
        let mut style = Style::default();
        let vars = BTreeMap::new();
        style.declaration("background-position", "left, center, right", &vars);
        style.declaration(
            "background-image",
            "var(--theme-background-image), none, linear-gradient(blue, white)",
            &vars,
        );
        style.declaration("background-repeat", "no-repeat, repeat-y", &vars);
        assert_eq!(style.images.len(), 3);
        assert_eq!(
            style.geometry(2).position,
            (Length::Percent(10000), Length::Percent(5000))
        );
        assert_eq!(style.geometry(2).repeat, (false, false));
        assert_eq!(style.geometry(1).repeat, (false, true));
        let saved = style.clone();
        style.declaration("background", "red, var(--theme-background-image)", &vars);
        style.declaration("background-image", "none, unsupported(x)", &vars);
        assert_eq!(style, saved);
        style.declaration("background","var(--theme-background-image) right / 25% auto no-repeat, linear-gradient(to bottom, rgb(1 2 3 / 50%), white) #102030",&vars);
        assert_eq!(style.images.len(), 2);
        assert_eq!(style.geometry(1), Geometry::default());
        assert_eq!(style.color, Some([16, 32, 48, 255]));
    }
}

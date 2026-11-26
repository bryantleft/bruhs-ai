//! Resolve native theme colors and stable workspace surface selectors from author CSS.
//! Keep the source intact in ThemeFile so other clients can render their own selectors.
use super::library::ROLES;
use bruhs_ui::theme::ThemeMode;
use cssparser::{
    parse_important, AtRuleParser, CowRcStr, DeclarationParser, Delimiter, ParseError, Parser,
    ParserInput, ParserState, QualifiedRuleParser, RuleBodyItemParser, RuleBodyParser,
    StyleSheetParser,
};
use std::collections::BTreeMap;

type Rgba = [u8; 4];
type Properties<'a> = BTreeMap<&'a str, ((bool, u16), &'a str)>;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Surface {
    Root,
    Sidebar,
    SidebarInner,
    Conversation,
    Header,
    UserBubble,
}
#[derive(Clone, Copy)]
struct Selector {
    surface: Surface,
    mode: Option<ThemeMode>,
    specificity: u16,
}

// The same appearance qualifiers that T3 documents for a theme stylesheet.
fn selector(source: &str) -> Option<Selector> {
    let s = source.trim();
    let (root, mode, mut specificity, s) = [
        ("html:root:not(.dark)", Some(ThemeMode::Light), 21),
        (":root:not(.dark)", Some(ThemeMode::Light), 20),
        ("html:not(.dark)", Some(ThemeMode::Light), 11),
        ("html:root.dark", Some(ThemeMode::Dark), 21),
        (":root.dark", Some(ThemeMode::Dark), 20),
        ("html.dark", Some(ThemeMode::Dark), 11),
        (".dark", Some(ThemeMode::Dark), 10),
        ("html:root", None, 11),
        (":root", None, 10),
        ("html", None, 1),
    ]
    .into_iter()
    .find_map(|(prefix, mode, weight)| {
        s.strip_prefix(prefix)
            .map(|rest| (true, mode, weight, rest))
    })
    .unwrap_or((false, None, 0, s));
    let surface = if s.is_empty() && root {
        Surface::Root
    } else {
        if root && !s.starts_with(char::is_whitespace) {
            return None;
        }
        let mut input = ParserInput::new(s.trim());
        let mut p = Parser::new(&mut input);
        p.expect_square_bracket_block().ok()?;
        let target = p
            .parse_nested_block(|p| {
                let name = p.expect_ident_cloned()?;
                if p.is_exhausted() {
                    return match name.as_ref() {
                        "data-chat-header" => Ok(Surface::Header),
                        "data-user-message-bubble" => Ok(Surface::UserBubble),
                        _ => Err(p.new_custom_error::<(), ()>(())),
                    };
                }
                p.expect_delim('=')?;
                let value = p.expect_ident_or_string()?;
                match (name.as_ref(), value.as_ref()) {
                    ("data-slot", "sidebar") => Ok(Surface::Sidebar),
                    ("data-slot", "sidebar-inner") => Ok(Surface::SidebarInner),
                    ("data-workspace-area", "conversation") => Ok(Surface::Conversation),
                    _ => Err(p.new_custom_error::<(), ()>(())),
                }
            })
            .ok()?;
        p.expect_exhausted().ok()?;
        specificity += 10;
        target
    };
    Some(Selector {
        surface,
        mode,
        specificity,
    })
}

struct StyleRule<'i> {
    selectors: Vec<Selector>,
    declarations: Vec<Declaration<'i>>,
}
struct Declaration<'i> {
    name: CowRcStr<'i>,
    value: &'i str,
    important: bool,
}
struct Rules;
struct Declarations;
impl<'i> AtRuleParser<'i> for Rules {
    type Prelude = ();
    type AtRule = StyleRule<'i>;
    type Error = ();
}
impl<'i> QualifiedRuleParser<'i> for Rules {
    type Prelude = Vec<Selector>;
    type QualifiedRule = StyleRule<'i>;
    type Error = ();
    fn parse_prelude<'t>(
        &mut self,
        p: &mut Parser<'i, 't>,
    ) -> Result<Self::Prelude, ParseError<'i, ()>> {
        let list = p.parse_comma_separated(|p| {
            let start = p.position();
            p.expect_no_error_token()?;
            Ok(selector(p.slice_from(start)))
        })?;
        Ok(list.into_iter().flatten().collect())
    }
    fn parse_block<'t>(
        &mut self,
        selectors: Self::Prelude,
        _: &ParserState,
        p: &mut Parser<'i, 't>,
    ) -> Result<Self::QualifiedRule, ParseError<'i, ()>> {
        let declarations = RuleBodyParser::new(p, &mut Declarations)
            .filter_map(Result::ok)
            .collect();
        Ok(StyleRule {
            selectors,
            declarations,
        })
    }
}
impl<'i> AtRuleParser<'i> for Declarations {
    type Prelude = ();
    type AtRule = Declaration<'i>;
    type Error = ();
}
impl<'i> QualifiedRuleParser<'i> for Declarations {
    type Prelude = ();
    type QualifiedRule = Declaration<'i>;
    type Error = ();
}
impl<'i> RuleBodyItemParser<'i, Declaration<'i>, ()> for Declarations {
    fn parse_declarations(&self) -> bool {
        true
    }
    fn parse_qualified(&self) -> bool {
        false
    }
}
impl<'i> DeclarationParser<'i> for Declarations {
    type Declaration = Declaration<'i>;
    type Error = ();
    fn parse_value<'t>(
        &mut self,
        name: CowRcStr<'i>,
        p: &mut Parser<'i, 't>,
        _: &ParserState,
    ) -> Result<Self::Declaration, ParseError<'i, ()>> {
        let value = p.parse_until_before(Delimiter::Bang, |p| {
            let start = p.position();
            p.expect_no_error_token()?;
            Ok(p.slice_from(start).trim())
        })?;
        let important = p.try_parse(parse_important).is_ok();
        p.expect_exhausted()?;
        Ok(Declaration {
            name,
            value,
            important,
        })
    }
}

fn properties<'a>(rules: &'a [StyleRule<'a>], mode: ThemeMode, surface: Surface) -> Properties<'a> {
    let mut result = Properties::new();
    for rule in rules {
        let Some(weight) = rule
            .selectors
            .iter()
            .filter(|s| s.surface == surface && s.mode.is_none_or(|m| m == mode))
            .map(|s| s.specificity)
            .max()
        else {
            continue;
        };
        for d in &rule.declarations {
            let name = if d.name.starts_with("--") {
                &*d.name
            } else if d.name.eq_ignore_ascii_case("color") {
                "color"
            } else if d.name.eq_ignore_ascii_case("border-color") {
                "border-color"
            } else {
                continue;
            };
            let priority = (d.important, weight);
            if !name.starts_with("--")
                && !d.value.starts_with("var(")
                && csscolorparser::parse(d.value).is_err()
            {
                continue;
            }
            if result.get(name).is_none_or(|(old, _)| priority >= *old) {
                result.insert(name, (priority, d.value));
            }
        }
    }
    result
}

fn resolve(
    value: &str,
    variables: &BTreeMap<String, String>,
    path: &mut Vec<String>,
) -> Option<Rgba> {
    if path.len() >= 32 {
        return None;
    }
    let mut input = ParserInput::new(value);
    let mut p = Parser::new(&mut input);
    if p.try_parse(|p| p.expect_function_matching("var")).is_err() {
        return csscolorparser::parse(value).ok().map(|c| c.to_rgba8());
    }
    let (name, fallback) = p
        .parse_nested_block(|p| {
            let name = p.expect_ident_cloned()?;
            if !name.starts_with("--") {
                return Err(p.new_custom_error::<(), ()>(()));
            }
            let fallback = if p.try_parse(Parser::expect_comma).is_ok() {
                let start = p.position();
                p.expect_no_error_token()?;
                Some(p.slice_from(start).trim())
            } else {
                None
            };
            Ok((name.to_string(), fallback))
        })
        .ok()?;
    p.expect_exhausted().ok()?;
    if path.contains(&name) {
        return None;
    }
    path.push(name.clone());
    let color = variables
        .get(&name)
        .and_then(|v| resolve(v, variables, path))
        .or_else(|| fallback.and_then(|v| resolve(v, variables, path)));
    path.pop();
    color
}

fn variable(role: &str) -> String {
    if role == "terminalSelection" {
        return "--app-theme-terminal-selection-background".into();
    }
    let mut key = String::from("--app-theme-");
    for ch in role.chars() {
        if ch.is_ascii_uppercase() {
            key.push('-')
        }
        key.push(ch.to_ascii_lowercase());
    }
    key
}

// Public CSS variables read by theme surfaces. Most use the underlying role's
// kebab-case spelling. These names differ from their --app-theme-* role.
fn public_variable(internal: &str) -> String {
    let name = internal.trim_start_matches("--app-theme-");
    let alias = match name {
        "canvas" => "background",
        "chrome" => "app-chrome-background",
        "toolbar" => "toolbar-background",
        "surface" => "card",
        "surface-overlay" => "popover",
        "text" => "foreground",
        "focus" => "ring",
        "error" => "destructive",
        "error-foreground" => "destructive-foreground",
        "accent-surface" => "accent",
        "accent-surface-foreground" => "accent-foreground",
        // T3 uses --accent for an interaction surface, distinct from its seed color.
        "accent" | "accent-foreground" => return internal.to_owned(),
        other => other,
    };
    format!("--{alias}")
}

pub(super) fn apply(
    css: &str,
    mode: ThemeMode,
    colors: &mut BTreeMap<String, Rgba>,
) -> super::background::Styles {
    let mut images = super::background::Styles::new();
    if css.trim().is_empty() || css.encode_utf16().count() > 50_000 {
        return images;
    }
    let mut input = ParserInput::new(css);
    let mut parser = Parser::new(&mut input);
    let rules = StyleSheetParser::new(&mut parser, &mut Rules)
        .filter_map(Result::ok)
        .collect::<Vec<_>>();
    let mut variables = BTreeMap::new();
    for (role, [r, g, b, a]) in &*colors {
        variables.insert(variable(role), format!("#{r:02x}{g:02x}{b:02x}{a:02x}"));
    }
    let roles = ROLES
        .split_ascii_whitespace()
        .map(|role| {
            let internal = variable(role);
            let alias = public_variable(&internal);
            (role, internal, alias)
        })
        .collect::<Vec<_>>();
    for (_, internal, alias) in &roles {
        if alias != internal {
            variables.insert(alias.clone(), format!("var({internal})"));
        }
    }
    for (alias, source) in [
        ("--primary", "--message-action"),
        ("--primary-foreground", "--message-action-foreground"),
        ("--card-foreground", "--foreground"),
        ("--popover-foreground", "--foreground"),
    ] {
        variables.insert(alias.into(), format!("var({source})"));
    }
    for (key, ((important, _), value)) in properties(&rules, mode, Surface::Root) {
        // The app installs its raw palette as inline custom properties.
        if key.starts_with("--")
            && (important || !key.starts_with("--app-theme-") || !variables.contains_key(key))
        {
            variables.insert(key.into(), value.into());
        }
    }
    for (role, _, alias) in roles {
        if let Some(color) = variables
            .get(&alias)
            .and_then(|v| resolve(v, &variables, &mut Vec::new()))
        {
            colors.insert(role.into(), color);
        }
    }
    let sidebar = properties(&rules, mode, Surface::Sidebar);
    for (surface, name) in [
        (Surface::Root, "Root"),
        (Surface::Sidebar, "Sidebar"),
        (Surface::SidebarInner, "SidebarInner"),
        (Surface::Conversation, "Conversation"),
        (Surface::Header, "Header"),
        (Surface::UserBubble, "UserBubble"),
    ] {
        let style = properties(&rules, mode, surface);
        let mut local = variables.clone();
        if surface == Surface::SidebarInner {
            for (key, (_, value)) in &sidebar {
                if key.starts_with("--") {
                    local.insert((*key).into(), (*value).into());
                }
            }
        }
        for (key, (_, value)) in &style {
            if key.starts_with("--") {
                local.insert((*key).into(), (*value).into());
            }
        }
        let mut declarations = Vec::new();
        for rule in &rules {
            if let Some(weight) = rule
                .selectors
                .iter()
                .filter(|s| s.surface == surface && s.mode.is_none_or(|m| m == mode))
                .map(|s| s.specificity)
                .max()
            {
                declarations.extend(rule.declarations.iter().map(|d| ((d.important, weight), d)));
            }
        }
        declarations.sort_by_key(|(priority, _)| *priority);
        let mut image = super::background::Style::default();
        for (_, d) in declarations {
            image.declaration(&d.name, d.value, &local)
        }
        let background_color = image.color;
        if image.active() {
            images.insert(name, image);
        }
        if surface == Surface::Root {
            continue;
        }
        if let Some(color) = background_color {
            colors.insert(format!("css{name}Background"), color);
        }
        for (property, suffix) in [("color", "Foreground"), ("border-color", "Border")] {
            if property == "border-color" && surface != Surface::Sidebar {
                continue;
            }
            if let Some(color) = style
                .get(property)
                .and_then(|(_, v)| resolve(v, &local, &mut Vec::new()))
            {
                colors.insert(format!("css{name}{suffix}"), color);
            }
        }
    }
    images
}

#[cfg(test)]
mod tests {
    use super::{apply, BTreeMap, ThemeMode};

    #[test]
    fn native_css_cascades_for_each_variant_and_surface() {
        let css = r#"
            @import "https://invalid.test/external.css";
            :root { --mark: #138399; --background: #121416; --app-theme-accent: red; }
            [data-user-message-bubble] {color: var(--background);}
            :root.dark { --app-theme-text: #e2e5e9 !important; }
            [data-slot="sidebar"] { --local: #334455; color: var(--local); }
            .dark [data-slot=sidebar] { background: var(--mark); }
            [data-slot=sidebar] { background: yellow; }
            :root:not(.dark) [data-slot=sidebar] { background-color: #ddccee; }
            [data-slot='sidebar-inner'] { background: var(--local); }
            [data-chat-header], [data-workspace-area=conversation] { background: var(--missing, #ffff0080); }
            [data-chat-header] { background-image: url("https://invalid.test/image.png"); }
            .unrelated [data-slot=sidebar] {background:red !important;}
        "#;
        for (mode, bg, text) in [
            (
                ThemeMode::Dark,
                [0x13, 0x83, 0x99, 255],
                [0xe2, 0xe5, 0xe9, 255],
            ),
            (ThemeMode::Light, [0xdd, 0xcc, 0xee, 255], [0, 0, 0, 255]),
        ] {
            let mut colors = BTreeMap::from([
                ("text".into(), [0, 0, 0, 255]),
                ("accent".into(), [1, 2, 3, 255]),
            ]);
            apply(css, mode, &mut colors);
            assert_eq!(colors["cssSidebarBackground"], bg);
            assert_eq!(colors["text"], text);
            assert_eq!(colors["canvas"], [0x12, 0x14, 0x16, 255]);
            assert_eq!(colors["cssUserBubbleForeground"], colors["canvas"]);
            assert_eq!(colors["accent"], [1, 2, 3, 255]);
            assert_eq!(colors["cssSidebarInnerBackground"], [0x33, 0x44, 0x55, 255]);
            assert_eq!(colors["cssConversationBackground"], [255, 255, 0, 128]);
            assert_eq!(colors["cssHeaderBackground"], [255, 255, 0, 128]);
        }
    }
}

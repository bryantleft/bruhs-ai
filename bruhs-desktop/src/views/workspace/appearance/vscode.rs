//! Convert local VS Code workbench theme files into portable T3 version 1 themes.
use super::{color, palette};

use super::library::{identifier, reserved, ThemeFile};
use bruhs_ui::theme::ThemeMode;
use color::{source_color, Rgb};
use serde_json::Value;

pub(super) fn recognizes(value: &Value) -> bool {
    value["version"] != 1
        && (value["tokenColors"].is_array()
            || value["colors"]
                .as_object()
                .is_some_and(|colors| colors.keys().any(|key| key.contains('.'))))
}

pub(super) fn display_name(raw: &str) -> String {
    let raw = raw.trim();
    let name = if raw.contains(char::is_whitespace) {
        raw.to_owned()
    } else {
        raw.split(['-', '_', '.'])
            .filter(|part| !part.is_empty())
            .map(|part| {
                if part == raw {
                    return part.to_owned();
                }
                let mut chars = part.chars();
                let Some(first) = chars.next() else {
                    return String::new();
                };
                format!("{}{}", first.to_uppercase(), chars.as_str())
            })
            .collect::<Vec<_>>()
            .join(" ")
    };
    let mut len = 0;
    name.chars()
        .take_while(|ch| {
            len += ch.len_utf16();
            len <= 48
        })
        .collect()
}

pub(super) fn convert(value: &Value) -> Result<ThemeFile, String> {
    let colors = &value["colors"];
    let source = |keys: &[&str]| {
        keys.iter()
            .find_map(|key| source_color(colors[*key].as_str()?))
    };
    let over = |bg: Rgb, keys: &[&str]| source(keys).map(|(fg, alpha)| bg.mix(fg, alpha).rounded());
    let (canvas, _) = source(&["editor.background", "editorPane.background"])
        .ok_or("The VS Code theme needs an editor.background color.")?;
    let canvas = canvas.rounded();
    let mode = match value["type"]
        .as_str()
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "light" | "hc-light" => ThemeMode::Light,
        "dark" | "hc-black" => ThemeMode::Dark,
        _ if canvas.dark() => ThemeMode::Dark,
        _ => ThemeMode::Light,
    };
    let raised = over(canvas, &["editorWidget.background", "dropdown.background"]);
    let visible =
        |c: Rgb| c.contrast(canvas) >= 1.1 && raised.is_none_or(|bg| c.contrast(bg) >= 1.1);
    let fallback = if mode == ThemeMode::Dark {
        Rgb::hex(0x346bf1)
    } else {
        Rgb::hex(0x1b4ed8)
    };
    let accent = [
        "focusBorder",
        "button.background",
        "textLink.foreground",
        "activityBarBadge.background",
        "progressBar.background",
        "badge.background",
    ]
    .iter()
    .filter_map(|key| source(&[key]))
    .find(|(c, alpha)| visible(canvas.mix(*c, *alpha).rounded()));
    let (tint, accent) = match accent {
        Some((c, alpha)) => (canvas.mix(c, 0.2).rounded(), canvas.mix(c, alpha).rounded()),
        None => {
            let c = [fallback, Rgb::hex(0xffffff), Rgb::hex(0)]
                .into_iter()
                .find(|c| visible(*c))
                .unwrap_or(fallback);
            (canvas.mix(c, 0.2).rounded(), c)
        }
    };
    let mut roles = palette::derived(canvas, tint);
    let raised = raised.unwrap_or(roles["surfaceRaised"]);
    let sidebar =
        over(canvas, &["sideBar.background", "activityBar.background"]).unwrap_or(roles["sidebar"]);
    let terminal = over(canvas, &["terminal.background", "panel.background"])
        .unwrap_or(roles["terminalBackground"]);
    let action = over(canvas, &["button.background"])
        .filter(|c| c.contrast(canvas) >= 1.1 && c.contrast(raised) >= 1.1)
        .unwrap_or(accent);
    let input_visible = |c: &Rgb| c.contrast(canvas) >= 1.1 && c.contrast(action) >= 1.1;
    let input = ["input.background", "input.border"]
        .iter()
        .filter_map(|key| over(canvas, &[key]))
        .find(input_visible)
        .or_else(|| {
            [
                roles["input"],
                roles["surfaceRaised"],
                Rgb::hex(if mode == ThemeMode::Dark {
                    0x1e1e1e
                } else {
                    0xd4d4d8
                }),
                Rgb::hex(0),
                Rgb::hex(0xffffff),
            ]
            .into_iter()
            .find(input_visible)
        })
        .unwrap_or(Rgb::hex(0x808080));
    roles.extend([
        ("accent", accent),
        ("focus", accent),
        ("messageAction", action),
        ("input", input),
        ("surfaceRaised", raised),
        ("sidebar", sidebar),
        ("terminalBackground", terminal),
    ]);
    for (role, bg, keys) in [
        ("surface", canvas, &["editorWidget.background"][..]),
        (
            "surfaceOverlay",
            canvas,
            &[
                "menu.background",
                "quickInput.background",
                "dropdown.background",
            ],
        ),
        (
            "border",
            canvas,
            &["panel.border", "editorGroup.border", "contrastBorder"],
        ),
        (
            "accentSurface",
            canvas,
            &["list.activeSelectionBackground", "list.hoverBackground"],
        ),
        ("codeBackground", canvas, &["textCodeBlock.background"]),
        (
            "searchMatchBackground",
            canvas,
            &["editor.findMatchHighlightBackground"],
        ),
        (
            "searchMatchActiveBackground",
            canvas,
            &["editor.findMatchBackground"],
        ),
        ("sidebarBorder", sidebar, &["sideBar.border"]),
        ("sidebarRowHover", sidebar, &["list.hoverBackground"]),
        (
            "sidebarRowActive",
            sidebar,
            &["list.inactiveSelectionBackground", "list.hoverBackground"],
        ),
        (
            "sidebarRowSelected",
            sidebar,
            &["list.activeSelectionBackground"],
        ),
        (
            "terminalCursor",
            terminal,
            &["terminalCursor.foreground", "editorCursor.foreground"],
        ),
        (
            "terminalSelection",
            terminal,
            &["terminal.selectionBackground", "editor.selectionBackground"],
        ),
        (
            "terminalScrollbar",
            terminal,
            &["scrollbarSlider.background"],
        ),
    ] {
        if let Some(c) = over(bg, keys) {
            roles.insert(role, c);
        }
    }
    for (role, bg, keys) in [
        ("text", canvas, &["editor.foreground", "foreground"][..]),
        (
            "textMuted",
            canvas,
            &["descriptionForeground", "disabledForeground"],
        ),
        ("placeholder", raised, &["input.placeholderForeground"]),
        ("sidebarForeground", sidebar, &["sideBar.foreground"]),
        ("terminalForeground", terminal, &["terminal.foreground"]),
        ("messageActionForeground", action, &["button.foreground"]),
        ("accentForeground", accent, &["button.foreground"]),
        (
            "searchMatchForeground",
            roles["searchMatchBackground"],
            &[
                "editor.findMatchHighlightForeground",
                "editor.foreground",
                "foreground",
            ],
        ),
        (
            "searchMatchActiveForeground",
            roles["searchMatchActiveBackground"],
            &[
                "editor.findMatchForeground",
                "editor.foreground",
                "foreground",
            ],
        ),
    ] {
        let ink = over(bg, keys)
            .into_iter()
            .chain([roles[role].rounded()])
            .find(|c| c.contrast(bg) >= 4.5)
            .unwrap_or_else(|| Rgb::hex(if bg.dark() { 0xffffff } else { 0 }));
        roles.insert(role, ink);
    }
    for (role, keys) in [
        ("error", &["editorError.foreground", "errorForeground"][..]),
        ("warning", &["editorWarning.foreground"][..]),
    ] {
        let color = over(canvas, keys)
            .filter(|c| c.contrast(canvas) >= 1.1)
            .unwrap_or(roles[role]);
        roles.insert(role, color.lch().ink_on(canvas, 4.5, canvas.dark()));
    }
    let name = ["displayName", "name"]
        .iter()
        .filter_map(|key| value[*key].as_str())
        .map(display_name)
        .find(|n| !n.is_empty())
        .unwrap_or_else(|| "VS Code theme".into());
    let mut id = identifier(&name);
    if reserved(&id) {
        id.push_str("-vscode");
    }
    Ok(ThemeFile::single(
        id,
        name,
        mode,
        roles
            .into_iter()
            .map(|(k, v)| (k.to_owned(), v.css()))
            .collect(),
    ))
}

#[cfg(test)]
mod tests {
    use super::super::library::InstalledTheme;
    use serde_json::Value;

    #[test]
    fn vscode_palette_preserves_derived_roles_and_overrides() {
        let reference: Value = serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/vscode-theme-import.json"
        )))
        .unwrap();
        let roles = reference["colors"].as_object().unwrap();
        let mut differences = Vec::new();
        for (i, case) in reference["cases"].as_array().unwrap().iter().enumerate() {
            let theme = InstalledTheme::parse(&case["source"].to_string()).unwrap();
            let file = serde_json::to_value(&theme.file).unwrap();
            for field in ["name", "appearance", "id"] {
                assert_eq!(file[field], case[field], "{field}");
            }
            assert_eq!(file["colors"].as_object().unwrap().len(), roles.len());
            for (role, expected) in roles {
                let actual = &file["colors"][role];
                if actual != &expected[i] {
                    differences.push(format!(
                        "{} {role}: {actual} vs {}",
                        case["name"], expected[i]
                    ));
                }
            }
        }
        assert!(differences.is_empty(), "{}", differences.join("\n"));
    }

    #[test]
    fn vscode_accepts_comments_and_trailing_commas() {
        let jsonc = r##"{ // VS Code accepts comments in a color theme.
            "displayName":"Lake", "colors":{"editor.background":"#cde",}, /* toolbar inherits */
        }"##;
        let theme = InstalledTheme::parse(jsonc).unwrap();
        assert_eq!(theme.file.colors["canvas"], "#ccddee");
        let exported = theme.json();
        assert_eq!(InstalledTheme::parse(&exported).unwrap(), theme);
        assert!(InstalledTheme::parse(&format!("/* invalid in a T3 file */ {exported}")).is_err());
        assert!(InstalledTheme::parse(r#"{"tokenColors":[]}"#)
            .unwrap_err()
            .contains("editor.background"));
        assert!(InstalledTheme::parse("{\"colors\":{}, /* unterminated }").is_err());
    }
}

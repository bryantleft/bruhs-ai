//! Related controls in the Advanced editor share fills, ink and interaction states.
use super::{color::Rgb, library::Colors, palette, ThemeFamily};
use bruhs_models::num::ToIntSaturating;
use bruhs_ui::ThemeMode;

pub(super) struct Family {
    pub label: &'static str,
    pub role: &'static str,
    pub(super) roles: &'static str,
}

impl Family {
    pub(super) fn matches(&self, query: &str) -> bool {
        if query.is_empty() || self.label.to_ascii_lowercase().contains(query) {
            return true;
        }
        self.roles.split_ascii_whitespace().any(|role| {
            let label = match role {
                "canvas" => "background",
                "toolbar" => "toolbar background",
                "toolbarForeground" => "toolbar text",
                "toolbarControlForeground" => "toolbar control text",
                "accent" => "accent color",
                "errorForeground" => "error text",
                "errorSurface" => "error background",
                "warningForeground" => "warning text",
                "warningSurface" => "warning background",
                "updateForeground" => "update text",
                "updateSurface" => "update background",
                _ => "",
            };
            if label.contains(query) {
                return true;
            }
            let mut words = String::new();
            for c in role.chars() {
                if c.is_ascii_uppercase() {
                    words.push(' ');
                }
                words.push(c.to_ascii_lowercase());
            }
            words.contains(query) || role.to_ascii_lowercase().contains(query)
        })
    }
}

const fn family(label: &'static str, role: &'static str, roles: &'static str) -> Family {
    Family { label, role, roles }
}
pub(super) const GROUPS: &[(&str, &[Family])] = &[
    (
        "Foundation",
        &[
            family("Background", "canvas", "canvas chrome toolbar"),
            family("Surface", "surface", "surface"),
            family("Raised surface", "surfaceRaised", "surfaceRaised"),
            family("Overlay", "surfaceOverlay", "surfaceOverlay"),
            family(
                "Text",
                "text",
                "text toolbarForeground toolbarControlForeground",
            ),
            family(
                "Muted text",
                "mutedForeground",
                "textMuted mutedForeground placeholder secondaryLabel iconMuted sidebarMutedForeground",
            ),
            family("Border", "border", "border toolbarBorder sidebarBorder"),
            family("Input", "input", "input"),
        ],
    ),
    (
        "Brand & content",
        &[
            family(
                "Subtle surface",
                "secondary",
                "secondary secondaryForeground muted toolbarControl",
            ),
            family(
                "Highlight surface",
                "accentSurface",
                "accentSurface accentSurfaceForeground toolbarControlHover",
            ),
            family(
                "Accent",
                "accent",
                "accent accentForeground focus update updateForeground updateSurface terminalCursor",
            ),
            family(
                "Action",
                "messageAction",
                "messageAction messageActionForeground messageActionHover",
            ),
            family(
                "Message surface",
                "messageSurface",
                "messageSurface messageForeground",
            ),
            family(
                "Code surface",
                "codeBackground",
                "codeBackground codeForeground",
            ),
        ],
    ),
    (
        "Search",
        &[
            family(
                "Search matches",
                "searchMatchBackground",
                "searchMatchBackground searchMatchForeground",
            ),
            family(
                "Current match",
                "searchMatchActiveBackground",
                "searchMatchActiveBackground searchMatchActiveForeground",
            ),
        ],
    ),
    (
        "Context",
        &[
            family("Sidebar background", "sidebar", "sidebar sidebarForeground"),
            family(
                "Sidebar controls",
                "sidebarControlSurface",
                "sidebarControlSurface",
            ),
            family(
                "Sidebar selection",
                "sidebarRowSelected",
                "sidebarRowHover sidebarRowActive sidebarRowSelected",
            ),
            family(
                "Terminal background",
                "terminalBackground",
                "terminalBackground terminalForeground terminalSelection terminalScrollbar terminalScrollbarHover",
            ),
        ],
    ),
    (
        "Status",
        &[
            family("Error", "error", "error errorForeground errorSurface"),
            family(
                "Warning",
                "warning",
                "warning warningForeground warningSurface",
            ),
        ],
    ),
];

fn parse(value: &str) -> Option<(Rgb, f64)> {
    let [r, g, b, a] = csscolorparser::parse(value).ok()?.to_rgba8();
    Some((Rgb([r, g, b].map(f64::from)), f64::from(a) / 255.))
}

fn color(colors: &Colors, role: &str, fallback: Rgb) -> Rgb {
    colors
        .get(role)
        .and_then(|s| parse(s))
        .map_or(fallback, |(rgb, _)| rgb)
}

pub(super) fn guided(colors: &Colors, mode: ThemeMode) -> Colors {
    let base = ThemeFamily::Bruhs.palette(mode);
    let canvas = color(colors, "canvas", Rgb::hex(base.surface[0]));
    let accent = color(colors, "accent", Rgb::hex(base.accent[4]));
    palette::derived(canvas, accent)
        .into_iter()
        .map(|(key, rgb)| (key.to_owned(), rgb.css()))
        .collect()
}

pub(super) fn edit(
    colors: &mut Colors,
    mode: ThemeMode,
    role: &str,
    literal: &str,
    advanced: bool,
) {
    if literal.is_empty() {
        colors.remove(role);
        return;
    }
    colors.insert(role.into(), literal.into());
    let Some((rgb, alpha)) = parse(literal) else {
        return;
    };
    if !advanced {
        *colors = guided(colors, mode);
        return;
    }
    let base = ThemeFamily::Bruhs.palette(mode);
    let canvas = color(colors, "canvas", Rgb::hex(base.surface[0]));
    let accent = color(colors, "accent", Rgb::hex(base.accent[4]));
    let sidebar = color(colors, "sidebar", canvas);
    let fill = canvas.mix(rgb, alpha);
    let ink = fill.readable_ink();
    let mut set = |key: &str, value: String| {
        colors.insert(key.into(), value);
    };
    match role {
        "canvas" | "text" | "mutedForeground" | "border" => {
            let keys: &[&str] = match role {
                "canvas" => &["chrome", "toolbar"],
                "text" => &["toolbarForeground", "toolbarControlForeground"],
                "border" => &["toolbarBorder", "sidebarBorder"],
                _ => &[
                    "textMuted",
                    "placeholder",
                    "secondaryLabel",
                    "iconMuted",
                    "sidebarMutedForeground",
                ],
            };
            for key in keys {
                set(key, literal.to_owned());
            }
        }
        "accent" | "error" | "warning" => {
            let amount = match (role, canvas.dark()) {
                ("accent", true) => 0.32,
                ("accent", false) | (_, true) => 0.16,
                (_, false) => 0.08,
            };
            let bg = canvas.mix(fill, amount);
            let (fg, surface) = match role {
                "error" => ("errorForeground", "errorSurface"),
                "warning" => ("warningForeground", "warningSurface"),
                _ => {
                    for key in ["focus", "update", "terminalCursor"] {
                        set(key, literal.to_owned());
                    }
                    set("accentForeground", ink.css());
                    ("updateForeground", "updateSurface")
                }
            };
            set(surface, bg.css());
            set(fg, rgb.lch().ink_on(bg, 4.6, bg.dark()).css());
        }
        "secondary" => {
            for key in ["muted", "toolbarControl"] {
                set(key, literal.to_owned());
            }
            set("secondaryForeground", ink.css());
        }
        "accentSurface" => {
            set("toolbarControlHover", literal.to_owned());
            set("accentSurfaceForeground", ink.css());
        }
        "messageAction" => {
            set("messageActionForeground", ink.css());
            let target = if ink.0[0] > 240. {
                Rgb::hex(0)
            } else {
                Rgb::hex(0xffffff)
            };
            let mut hover = rgb.mix(target, 0.12).css();
            if alpha < 1. {
                hover.push_str(&format!(
                    "{:02x}",
                    (alpha * 255.).round().to_u8_saturating()
                ));
            }
            set("messageActionHover", hover);
        }
        "messageSurface" => set("messageForeground", ink.css()),
        "codeBackground" => set("codeForeground", ink.css()),
        "sidebar" => set("sidebarForeground", ink.css()),
        "searchMatchBackground" | "searchMatchActiveBackground" => {
            set(role, fill.css());
            set(
                if role == "searchMatchBackground" {
                    "searchMatchForeground"
                } else {
                    "searchMatchActiveForeground"
                },
                ink.css(),
            );
        }
        "sidebarRowSelected" => {
            let fill = sidebar.mix(rgb, alpha);
            set("sidebarRowHover", sidebar.mix(fill, 0.5).css());
            set("sidebarRowActive", sidebar.mix(fill, 0.8).css());
        }
        "terminalBackground" => {
            set("terminalForeground", ink.css());
            set(
                "terminalSelection",
                fill.mix(accent, if fill.dark() { 0.35 } else { 0.18 })
                    .css(),
            );
            set(
                "terminalScrollbar",
                fill.mix(ink, if fill.dark() { 0.42 } else { 0.22 }).css(),
            );
            set(
                "terminalScrollbarHover",
                fill.mix(ink, if fill.dark() { 0.55 } else { 0.32 }).css(),
            );
        }
        _ => {}
    }
}

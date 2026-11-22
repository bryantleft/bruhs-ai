use super::color::{Lch, Rgb};
use std::collections::BTreeMap;

pub(super) type Roles = BTreeMap<&'static str, Rgb>;

/// Generate the colors the source extension leaves unspecified. Surface steps
/// follow the canvas lightness, tinted by the accent. Text is contrast-checked
/// against the surface it will actually be drawn on.
pub(super) fn derived(canvas: Rgb, tint: Rgb) -> Roles {
    let dark = canvas.dark();
    let choose = |d, l| if dark { d } else { l };
    let base = canvas.lch();
    let accent = tint.lch();
    let hue = if accent.c < 0.02 { base.h } else { accent.h };
    let chroma = (accent.c * 0.22).clamp(0.008, 0.045);
    let surface = |step: f64, c| {
        Lch {
            l: (base.l + choose(step, -step)).clamp(0.05, 0.98),
            c,
            h: hue,
        }
        .rgb()
    };
    let shade = |d, l, cap: f64, scale| surface(choose(d, l), cap.min(accent.c * scale));
    let ink = Lch {
        l: choose(0.95, 0.2),
        c: 0.035_f64.min(accent.c * 0.25),
        h: hue,
    };
    let on = |bg| ink.ink_on(bg, 4.6, dark);
    let text = ink.ink_on(canvas, 7., dark);
    let muted = |bg: Rgb| bg.muted_ink(text, if bg.dark() { 5.082 } else { 4.705 });
    let action = Lch {
        l: (accent.l + choose(0.06, -0.02)).clamp(0.35, 0.85),
        c: (accent.c * 0.9).max(0.06),
        h: hue + 50f64.to_radians(),
    };
    let raised = surface(0.05, chroma);
    let sidebar = surface(0.045, chroma * 1.4);
    let secondary = shade(0.1, 0.06, 0.09, 0.5);
    let quiet = shade(0.06, 0.04, 0.06, 0.35);
    let highlight = shade(0.13, 0.08, 0.11, 0.55);
    let message = shade(0.16, 0.1, 0.13, 0.6);
    let update = shade(0.14, 0.09, 0.12, 0.55);
    let search = surface(0.22, 0.12_f64.min(accent.c));
    let active_search = surface(0.4, 0.18_f64.min(accent.c));
    let mut roles = Roles::from([
        ("canvas", canvas),
        ("chrome", canvas),
        ("toolbar", canvas),
        ("toolbarForeground", text),
        ("toolbarBorder", shade(0.14, 0.1, 0.08, 0.4)),
        ("toolbarControl", surface(choose(0.09, 0.05), chroma * 1.3)),
        ("toolbarControlForeground", text),
        (
            "toolbarControlHover",
            surface(choose(0.14, 0.09), chroma * 1.6),
        ),
        ("surface", surface(0.015, chroma)),
        ("surfaceRaised", raised),
        ("surfaceOverlay", surface(0.075, chroma)),
        ("text", text),
        ("textMuted", muted(canvas)),
        ("border", shade(0.16, 0.12, 0.07, 0.35)),
        ("input", shade(0.21, 0.16, 0.08, 0.4)),
        ("focus", tint),
        ("accent", tint),
        ("accentForeground", tint.readable_ink()),
        ("secondary", secondary),
        ("secondaryForeground", on(secondary)),
        ("muted", quiet),
        (
            "mutedForeground",
            quiet.muted_ink(text, 4.6).lch().serialized_rgb(),
        ),
        ("placeholder", raised.muted_ink(text, 4.6)),
        ("secondaryLabel", muted(canvas)),
        ("iconMuted", muted(canvas)),
        ("update", tint),
        ("updateForeground", on(update)),
        ("updateSurface", update),
        ("accentSurface", highlight),
        ("accentSurfaceForeground", on(highlight)),
        ("messageSurface", message),
        ("messageForeground", on(message)),
        ("messageAction", action.rgb()),
        ("messageActionForeground", action.rgb().readable_ink()),
        (
            "messageActionHover",
            Lch {
                l: action.l + choose(0.06, -0.06),
                ..action
            }
            .rgb(),
        ),
        ("codeBackground", surface(0.035, chroma * 0.8)),
        ("codeForeground", text),
        ("searchMatchBackground", search),
        ("searchMatchForeground", search.readable_ink()),
        ("searchMatchActiveBackground", active_search),
        ("searchMatchActiveForeground", active_search.readable_ink()),
        ("sidebar", sidebar),
        ("sidebarForeground", on(sidebar)),
        ("sidebarMutedForeground", muted(sidebar)),
        (
            "sidebarControlSurface",
            surface(choose(0.1, 0.07), chroma * 1.5),
        ),
        ("sidebarRowHover", shade(0.08, 0.06, 0.08, 0.45)),
        ("sidebarRowActive", shade(0.12, 0.09, 0.1, 0.55)),
        ("sidebarRowSelected", shade(0.14, 0.1, 0.11, 0.6)),
        ("sidebarBorder", shade(0.17, 0.12, 0.08, 0.4)),
        ("terminalBackground", canvas),
        ("terminalForeground", text),
        ("terminalCursor", tint),
        ("terminalSelection", shade(0.18, 0.12, 0.12, 0.55)),
        ("terminalScrollbar", surface(choose(0.22, 0.16), chroma)),
        ("terminalScrollbarHover", surface(choose(0.3, 0.22), chroma)),
    ]);
    for (color, foreground, surface, base, ink) in [
        (
            "error",
            "errorForeground",
            "errorSurface",
            if dark { 0xfb414a } else { 0xfb2c36 },
            if dark { 0xff6467 } else { 0xc10007 },
        ),
        (
            "warning",
            "warningForeground",
            "warningSurface",
            0xfe9a00,
            if dark { 0xffb900 } else { 0xbb4d00 },
        ),
    ] {
        let base = Rgb::hex(base);
        let bg = canvas.mix(base, choose(0.16, 0.08));
        roles.extend([
            (color, base),
            (surface, bg),
            (
                foreground,
                Rgb::hex(ink).lch().ink(bg, 4.6, dark).serialized_rgb(),
            ),
        ]);
    }
    roles
}

//! Icons the components draw, as Lucide SVGs.
//!
//! Components refer to an icon by path. Serve these from the app's
//! [`gpui::AssetSource`] by calling [`load`] first.

use gpui::{svg, App, Hsla, IntoElement, Pixels, RenderOnce, Styled, Window};

pub const CHECK: &str = "icons/check.svg";
pub const CHEVRON_DOWN: &str = "icons/chevron-down.svg";
pub const ELLIPSIS: &str = "icons/ellipsis.svg";

const CHECK_SVG: &[u8] = br#"<svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" xmlns="http://www.w3.org/2000/svg"><path d="M20 6 9 17l-5-5"/></svg>"#;
const CHEVRON_DOWN_SVG: &[u8] = include_bytes!("../assets/icons/chevron-down.svg");
const ELLIPSIS_SVG: &[u8] = br#"<svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" xmlns="http://www.w3.org/2000/svg"><circle cx="12" cy="12" r="1"/><circle cx="19" cy="12" r="1"/><circle cx="5" cy="12" r="1"/></svg>"#;

/// The SVG for one of this crate's icon paths, or `None` for any other path.
pub fn load(path: &str) -> Option<&'static [u8]> {
    match path {
        CHECK => Some(CHECK_SVG),
        CHEVRON_DOWN => Some(CHEVRON_DOWN_SVG),
        ELLIPSIS => Some(ELLIPSIS_SVG),
        _ => IconName::ALL
            .into_iter()
            .find(|name| name.path() == path)
            .map(IconName::svg),
    }
}

/// A named icon that [`Icon`] draws.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IconName {
    Close,
    Delete,
    Eye,
    EyeOff,
    TriangleAlert,
}

impl IconName {
    const ALL: [IconName; 5] = [
        IconName::Close,
        IconName::Delete,
        IconName::Eye,
        IconName::EyeOff,
        IconName::TriangleAlert,
    ];

    /// The asset path [`load`] serves it at.
    pub fn path(self) -> &'static str {
        match self {
            IconName::Close => "bruhs-ui/icons/close.svg",
            IconName::Delete => "bruhs-ui/icons/delete.svg",
            IconName::Eye => "bruhs-ui/icons/eye.svg",
            IconName::EyeOff => "bruhs-ui/icons/eye-off.svg",
            IconName::TriangleAlert => "bruhs-ui/icons/triangle-alert.svg",
        }
    }

    fn svg(self) -> &'static [u8] {
        match self {
            IconName::Close => include_bytes!("../assets/icons/close.svg"),
            IconName::Delete => include_bytes!("../assets/icons/delete.svg"),
            IconName::Eye => include_bytes!("../assets/icons/eye.svg"),
            IconName::EyeOff => include_bytes!("../assets/icons/eye-off.svg"),
            IconName::TriangleAlert => include_bytes!("../assets/icons/triangle-alert.svg"),
        }
    }
}

/// An icon drawn in the surrounding text's color, at the text size unless
/// [`Icon::size`] says otherwise.
#[derive(IntoElement)]
pub struct Icon {
    name: IconName,
    size: Option<Pixels>,
    color: Option<Hsla>,
}

impl Icon {
    pub fn new(name: IconName) -> Self {
        Self {
            name,
            size: None,
            color: None,
        }
    }

    pub fn size(mut self, size: Pixels) -> Self {
        self.size = Some(size);
        self
    }

    /// Draw in `color` instead of the text color.
    pub fn color(mut self, color: Hsla) -> Self {
        self.color = Some(color);
        self
    }
}

impl RenderOnce for Icon {
    fn render(self, window: &mut Window, _: &mut App) -> impl IntoElement {
        let text = window.text_style();
        // An svg never takes its color or size from the text around it.
        let size = self
            .size
            .unwrap_or_else(|| text.font_size.to_pixels(window.rem_size()));
        svg()
            .path(self.name.path())
            .flex_none()
            .size(size)
            .text_color(self.color.unwrap_or(text.color))
    }
}

#[cfg(test)]
mod tests {
    use super::{load, CHECK, CHEVRON_DOWN, ELLIPSIS};

    #[test]
    fn every_icon_path_loads_an_svg() {
        for path in [CHECK, CHEVRON_DOWN, ELLIPSIS] {
            let svg = load(path).unwrap_or_default();
            assert!(svg.starts_with(b"<svg"), "{path}");
        }
        for name in super::IconName::ALL {
            let svg = load(name.path()).unwrap_or_default();
            assert!(svg.starts_with(b"<svg"), "{name:?}");
        }
        assert_eq!(load("icons/unknown.svg"), None);
    }
}

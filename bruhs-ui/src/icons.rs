//! Icons the components draw, as Lucide SVGs.
//!
//! Components refer to an icon by path. Serve these from the app's
//! [`gpui::AssetSource`] by calling [`load`] first.

pub const CHECK: &str = "icons/check.svg";
pub const CHEVRON_DOWN: &str = "icons/chevron-down.svg";
pub const ELLIPSIS: &str = "icons/ellipsis.svg";

const CHECK_SVG: &[u8] = br#"<svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" xmlns="http://www.w3.org/2000/svg"><path d="M20 6 9 17l-5-5"/></svg>"#;
const CHEVRON_DOWN_SVG: &[u8] =
    include_bytes!("../../third_party/gpui-component/crates/assets/assets/icons/chevron-down.svg");
const ELLIPSIS_SVG: &[u8] = br#"<svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" xmlns="http://www.w3.org/2000/svg"><circle cx="12" cy="12" r="1"/><circle cx="19" cy="12" r="1"/><circle cx="5" cy="12" r="1"/></svg>"#;

/// The SVG for one of this crate's icon paths, or `None` for any other path.
pub fn load(path: &str) -> Option<&'static [u8]> {
    match path {
        CHECK => Some(CHECK_SVG),
        CHEVRON_DOWN => Some(CHEVRON_DOWN_SVG),
        ELLIPSIS => Some(ELLIPSIS_SVG),
        _ => None,
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
        assert_eq!(load("icons/unknown.svg"), None);
    }
}

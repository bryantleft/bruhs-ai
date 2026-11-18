//! The window's title bar: transparent, with the app drawing its own header
//! under the macOS traffic lights.

use gpui::{point, px, TitlebarOptions};

/// The traffic lights' position, centred in the app's 44pt header. Window
/// chrome stays in window points at any page zoom.
pub const TRAFFIC_LIGHT_POSITION: (f32, f32) = (12.0, 15.0);

/// The title bar of every app window.
pub struct TitleBar;

impl TitleBar {
    /// Window options' title bar: no system title, a transparent bar, and the
    /// traffic lights placed for the app's header.
    pub fn title_bar_options() -> TitlebarOptions {
        TitlebarOptions {
            title: None,
            appears_transparent: true,
            traffic_light_position: Some(point(
                px(TRAFFIC_LIGHT_POSITION.0),
                px(TRAFFIC_LIGHT_POSITION.1),
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bar_is_transparent_and_untitled_with_placed_traffic_lights() {
        let options = TitleBar::title_bar_options();
        assert!(options.appears_transparent);
        assert!(options.title.is_none());
        assert_eq!(
            options.traffic_light_position,
            Some(point(px(12.0), px(15.0)))
        );
    }
}

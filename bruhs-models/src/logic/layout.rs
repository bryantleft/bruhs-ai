//! Window layout rules both apps share: the web's breakpoints and the right
//! panel's width. Widths are CSS pixels, which desktop draws as points.

/// Tailwind's `sm`, `md` and `lg`. The web's layouts change at these window
/// widths, and desktop's switch at the same ones.
pub const SM_BREAKPOINT: f64 = 640.0;
pub const MD_BREAKPOINT: f64 = 768.0;
pub const LG_BREAKPOINT: f64 = 1024.0;

pub const RIGHT_PANEL_DEFAULT_WIDTH: f64 = 540.0;
pub const RIGHT_PANEL_MIN_WIDTH: f64 = 360.0;
/// The chat column never drops below this while the panel is open.
pub const CHAT_COLUMN_MIN_WIDTH: f64 = 360.0;

/// The widest the right panel may be: 70% of the viewport, and never so
/// wide the chat column collapses.
fn panel_max_width(viewport_width: f64) -> f64 {
    (viewport_width * 0.7)
        .min(viewport_width - CHAT_COLUMN_MIN_WIDTH)
        .round()
        .max(RIGHT_PANEL_MIN_WIDTH)
}

/// Clamp a requested panel width: at least the minimum, at most 70% of the
/// viewport, and never so wide the chat column collapses.
pub fn clamp_panel_width(requested: f64, viewport_width: f64) -> f64 {
    requested.clamp(RIGHT_PANEL_MIN_WIDTH, panel_max_width(viewport_width))
}

/// The docked panel's (min, max) width beside a docked sidebar
/// `sidebar_width` wide (0 when it is hidden). The web's flex layout keeps
/// the chat column at its minimum (`md:min-w-[360px]`) and shrinks the
/// panel, so a narrow window can take the panel below its own minimum.
pub fn docked_panel_range(viewport_width: f64, sidebar_width: f64) -> (f64, f64) {
    let room = (viewport_width - sidebar_width - CHAT_COLUMN_MIN_WIDTH).max(0.0);
    let max = panel_max_width(viewport_width).min(room);
    (RIGHT_PANEL_MIN_WIDTH.min(max), max)
}

/// The docked panel's width: the remembered width, clamped, giving way to
/// the chat column. A window resize never changes the remembered width.
pub fn docked_panel_width(requested: f64, viewport_width: f64, sidebar_width: f64) -> f64 {
    let (min, max) = docked_panel_range(viewport_width, sidebar_width);
    clamp_panel_width(requested, viewport_width).clamp(min, max)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn panel_width_clamps_to_viewport() {
        assert_eq!(clamp_panel_width(540.0, 1440.0), 540.0);
        assert_eq!(clamp_panel_width(100.0, 1440.0), RIGHT_PANEL_MIN_WIDTH);
        assert_eq!(clamp_panel_width(2000.0, 1440.0), 1008.0);
        // Narrow viewport: cap by the chat column floor, never below the minimum.
        assert_eq!(clamp_panel_width(600.0, 800.0), 440.0);
        assert_eq!(clamp_panel_width(600.0, 600.0), RIGHT_PANEL_MIN_WIDTH);
    }

    #[test]
    fn a_docked_panel_gives_way_to_the_chat_column() {
        // 800px beside the 248px sidebar: the chat keeps 360, the panel 192.
        assert_eq!(docked_panel_range(800.0, 248.0), (192.0, 192.0));
        assert_eq!(docked_panel_width(540.0, 800.0, 248.0), 192.0);
        // The sidebar hidden: the panel's own clamp.
        assert_eq!(docked_panel_range(800.0, 0.0), (360.0, 440.0));
        assert_eq!(docked_panel_width(540.0, 800.0, 0.0), 440.0);
        // Wide windows keep the remembered width.
        assert_eq!(docked_panel_width(540.0, 1440.0, 248.0), 540.0);
        assert_eq!(docked_panel_range(1440.0, 248.0), (360.0, 832.0));
    }
}

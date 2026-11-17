//! How a bruh's face is drawn: the style the dot renderer takes, and the
//! expression each status gives it. Both apps draw from this, so they agree.

use crate::logic::dither::{BRUH_FACE_RGB, BRUH_MOUTH_RGB};
use crate::num::{ToFloatLossy, ToIntSaturating};

use super::{BruhLook, BruhStatus};

/// One full sweep of a working bruh's gaze, in milliseconds (never faster
/// than once per two seconds). The single source for sweep timing.
pub const SWEEP_PERIOD_MS: i64 = 2400;
/// How far a working bruh's gaze swings to either side.
pub const SWEEP_LOOK_X: f32 = 0.6;

/// Face size in the rail, in px.
pub const RAIL_FACE_PX: f32 = 20.0;
/// Face size on a bruh's page, in px.
pub const PAGE_FACE_PX: f32 = 48.0;
/// Face size in the look editor, in px.
pub const EDITOR_FACE_PX: f32 = 96.0;
/// Face size of each state preview, in px.
pub const STATE_PREVIEW_PX: f32 = 32.0;
/// Stroke of the needs-you ring, in px.
pub const RING_STROKE_PX: f32 = 1.5;
/// Radius of the needs-you ring, as a share of the face box.
pub const RING_RADIUS: f32 = 0.48;

/// Whether the eyes are open holes or thin closed lines.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Eyes {
    #[default]
    Open,
    Closed,
}

/// How to draw one face: where it looks, its colours and its eyes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FaceStyle {
    /// -1 turns the head fully left, 1 fully right.
    pub look_x: f32,
    /// -1 tilts it up, 1 down.
    pub look_y: f32,
    pub face_rgb: u32,
    pub mouth_rgb: u32,
    pub eyes: Eyes,
}

impl Default for FaceStyle {
    fn default() -> Self {
        Self {
            look_x: 0.0,
            look_y: 0.0,
            face_rgb: BRUH_FACE_RGB,
            mouth_rgb: BRUH_MOUTH_RGB,
            eyes: Eyes::Open,
        }
    }
}

/// What to draw for a bruh right now.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct FaceView {
    pub style: FaceStyle,
    /// A ring around the face, in this colour.
    pub ring_rgb: Option<u32>,
    /// Whether the face changes over time (so the surface should redraw).
    pub animates: bool,
}

/// Gaze of a working bruh at `now_ms`: a slow sine sweep, one per
/// [`SWEEP_PERIOD_MS`].
#[must_use]
pub fn sweep_look_x(now_ms: i64) -> f32 {
    let phase = now_ms.rem_euclid(SWEEP_PERIOD_MS).to_f32_lossy() / SWEEP_PERIOD_MS.to_f32_lossy();
    SWEEP_LOOK_X * (std::f32::consts::TAU * phase).sin()
}

/// The face for a bruh with this look and status.
#[must_use]
pub fn face_view(
    look: &BruhLook,
    status: BruhStatus,
    now_ms: i64,
    reduced_motion: bool,
) -> FaceView {
    let own = FaceStyle {
        look_x: look.look.look_x(),
        face_rgb: look.face.rgb(),
        mouth_rgb: look.mouth.rgb(),
        ..FaceStyle::default()
    };
    match status {
        BruhStatus::Asleep => FaceView {
            style: FaceStyle {
                eyes: Eyes::Closed,
                ..own
            },
            ..FaceView::default()
        },
        BruhStatus::Working if reduced_motion => FaceView {
            style: own,
            ..FaceView::default()
        },
        BruhStatus::Working => FaceView {
            style: FaceStyle {
                look_x: sweep_look_x(now_ms),
                ..own
            },
            ring_rgb: None,
            animates: true,
        },
        BruhStatus::NeedsYou => FaceView {
            style: FaceStyle { look_x: 0.0, ..own },
            ring_rgb: Some(own.face_rgb),
            animates: false,
        },
        BruhStatus::Paused => FaceView {
            style: FaceStyle {
                look_x: 0.0,
                face_rgb: dim(own.face_rgb, 0.55),
                mouth_rgb: dim(own.mouth_rgb, 0.55),
                ..own
            },
            ..FaceView::default()
        },
        BruhStatus::Off => FaceView {
            style: FaceStyle {
                face_rgb: desaturate(own.face_rgb),
                mouth_rgb: desaturate(own.mouth_rgb),
                ..own
            },
            ..FaceView::default()
        },
    }
}

fn channels(rgb: u32) -> [f32; 3] {
    [16, 8, 0].map(|shift| f32::from(u8::try_from((rgb >> shift) & 0xff).unwrap_or(0)))
}

fn pack([r, g, b]: [f32; 3]) -> u32 {
    let byte = |c: f32| u32::from(c.round().to_u8_saturating());
    (byte(r) << 16) | (byte(g) << 8) | byte(b)
}

/// `rgb` with each channel scaled by `factor`.
#[must_use]
pub fn dim(rgb: u32, factor: f32) -> u32 {
    pack(channels(rgb).map(|c| c * factor))
}

/// The grey of the same brightness (Rec. 601 luma).
#[must_use]
pub fn desaturate(rgb: u32) -> u32 {
    let [r, g, b] = channels(rgb);
    let luma = 0.299 * r + 0.587 * g + 0.114 * b;
    pack([luma; 3])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crew::{BruhColor, Look};

    #[test]
    fn working_sweeps_at_most_once_per_two_seconds() {
        const { assert!(SWEEP_PERIOD_MS >= 2000) };
        assert!(sweep_look_x(0).abs() < 1e-6);
        assert!((sweep_look_x(600) - 0.6).abs() < 1e-4);
        assert!((sweep_look_x(1800) + 0.6).abs() < 1e-4);
    }

    #[test]
    fn each_status_has_its_expression() {
        let look = BruhLook {
            look: Look::Right,
            ..BruhLook::default()
        };
        let now = 600;
        let view = |status, reduced| face_view(&look, status, now, reduced);

        let asleep = view(BruhStatus::Asleep, false);
        assert_eq!(asleep.style.eyes, Eyes::Closed);
        assert!(!asleep.animates);
        assert_eq!(asleep.style.look_x, look.look.look_x());

        let working = view(BruhStatus::Working, false);
        assert!(working.animates);
        assert_eq!(working.style.look_x, sweep_look_x(now));
        assert_eq!(working.style.eyes, Eyes::Open);

        let still = view(BruhStatus::Working, true);
        assert!(!still.animates);
        assert_eq!(still.style.look_x, look.look.look_x());

        let needs = view(BruhStatus::NeedsYou, false);
        assert_eq!(needs.style.look_x, 0.0);
        assert_eq!(needs.style.eyes, Eyes::Open);
        assert_eq!(needs.ring_rgb, Some(BruhColor::Bruh.rgb()));
        assert!(!needs.animates);

        let paused = view(BruhStatus::Paused, false);
        assert_eq!(paused.style.look_x, 0.0);
        let (full, dark) = (
            channels(BruhColor::Bruh.rgb()),
            channels(paused.style.face_rgb),
        );
        assert!(full.iter().zip(dark).all(|(f, d)| d < *f));

        let off = view(BruhStatus::Off, false).style;
        for rgb in [off.face_rgb, off.mouth_rgb] {
            let [r, g, b] = channels(rgb);
            assert!(r == g && g == b);
        }
    }

    #[test]
    fn dim_and_desaturate_keep_the_extremes() {
        assert_eq!(dim(0x00ff_ffff, 0.0), 0);
        assert_eq!(dim(0x00ff_ffff, 1.0), 0x00ff_ffff);
        assert_eq!(desaturate(0x00ff_ffff), 0x00ff_ffff);
        assert_eq!(desaturate(0), 0);
    }
}

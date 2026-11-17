//! Dither, the Bruhs brand texture, for the app: the Bruh mascot as a
//! halftone of dots, and a level meter drawn as one braille cell. A port of
//! `@bruhs/theme/dither` (AGENTS.md section 19) and of bruhs.ai's
//! `src/scripts/braille.js`, so every surface places the same dots. Pure math;
//! each surface draws.

use crate::crew::{Eyes, FaceStyle};
use crate::num::{ToFloatLossy, ToIntSaturating};

/// The Bruh's colors, from `bruhs.svg` (theme tokens `bruh-base`, `bruh-mouth`).
pub const BRUH_FACE_RGB: u32 = 0x00fa_da5e;
pub const BRUH_MOUTH_RGB: u32 = 0x00fc_4141;

// Light from the upper left, mostly frontal (normalized in `shade`), with some
// ambient so the shadow side keeps its dots and the outline stays whole.
const LIGHT: (f32, f32, f32) = (-0.35, -0.45, 0.82);
const AMBIENT: f32 = 0.25;
/// How much the dots shrink toward the rim, which rounds the silhouette.
const RIM: f32 = 0.35;

// Geometry in face radii, from the logo's 600×600 viewBox (r = 300), facing
// forward (`bruhGeometry.faces.original`). The eyes are mirrored about the
// center (the logo has them 102 and 101 units out): the hex grid is
// symmetric, so mirrored eyes cut identical slits.
const EYE_X: f32 = 101.5 / 300.0;
const EYE_Y: f32 = (223.0 - 300.0) / 300.0;
const MOUTH: (f32, f32) = ((307.0 - 300.0) / 300.0, (378.0 - 300.0) / 300.0);

// A full look turns the head this far (radians) and shifts the ball this
// share of its radius.
const MAX_YAW: f32 = 0.75;
const MAX_PITCH: f32 = 0.45;
const LEAN: f32 = 0.04;
/// Closed eyes keep this share of an open eye's dot size: a thin line.
pub const CLOSED_EYE_SCALE: f32 = 0.35;
const EYE_RX: f32 = 40.0 / 300.0;
const MOUTH_RX: f32 = 120.0 / 300.0;

/// Which color a dot is drawn in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DotKind {
    Face,
    Mouth,
}

/// One dot to draw: its center and radius, in the box's units.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Dot {
    pub x: f32,
    pub y: f32,
    pub radius: f32,
    /// `Mouth` once the dot is at least half mouth.
    pub kind: DotKind,
    /// How much of the mouth's red it takes, 0 to 1.
    pub mix: f32,
}

impl Dot {
    /// The dot's `0xRRGGBB` color: face yellow blended toward mouth red in
    /// five steps (as bruhs.ai and `@bruhs/theme/dither` draw it).
    pub fn rgb(&self) -> u32 {
        self.rgb_in(&FaceStyle::default())
    }

    /// The dot's color in a styled face: its face color blended toward its
    /// mouth color.
    pub fn rgb_in(&self, style: &FaceStyle) -> u32 {
        let step = (self.mix.clamp(0.0, 1.0) * 4.0).round() / 4.0;
        let channel = |shift: u32| {
            let a = f32::from(u8::try_from((style.face_rgb >> shift) & 0xff).unwrap_or(0));
            let b = f32::from(u8::try_from((style.mouth_rgb >> shift) & 0xff).unwrap_or(0));
            u32::from((a + (b - a) * step).round().to_u8_saturating()) << shift
        };
        channel(16) | channel(8) | channel(0)
    }
}

/// Target grid spacing for a face radius `r`: about 15 dots per radius on a
/// large face, down to 9 on a small one (fewer turns the outline into a
/// hexagon). [`grid_for`] tightens it a little so the features land exactly.
pub fn pitch_for(r: f32) -> f32 {
    r / (r / 10.0).clamp(9.0, 15.0)
}

/// The hex grid for one face size: dot spacing, row spacing, and where row 0
/// sits below the center (row `n` is at `cy + phase + n * row_h`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Grid {
    pub pitch: f32,
    pub row_h: f32,
    pub phase: f32,
}

/// The hex grid for a face radius, fitted to the mascot: the eye line and
/// the mouth line both fall exactly on a row, where `bruhs.svg` puts them, at
/// every size. The rows between them are a whole number (rounded up from
/// [`pitch_for`]'s density, so the grid only gets finer), and the rows are
/// anchored on the eye line instead of the ball's center. Columns still start
/// at the center, so the face stays mirror-symmetric.
pub fn grid_for(r: f32) -> Grid {
    let gap = (MOUTH.1 - EYE_Y) * r;
    let target = pitch_for(r) * 3.0_f32.sqrt() / 2.0;
    let row_h = gap / (gap / target - 1e-6).ceil().max(1.0);
    let eye_y = EYE_Y * r;
    Grid {
        pitch: 2.0 * row_h / 3.0_f32.sqrt(),
        row_h,
        phase: eye_y - (eye_y / row_h).round() * row_h,
    }
}

/// Brightness (0 to 1) of the sphere at a point in face radii, or `None`
/// outside it.
pub fn shade(dx: f32, dy: f32) -> Option<f32> {
    let d2 = dx * dx + dy * dy;
    if d2 > 1.0 {
        return None;
    }
    let nz = (1.0 - d2).sqrt();
    let length = (LIGHT.0 * LIGHT.0 + LIGHT.1 * LIGHT.1 + LIGHT.2 * LIGHT.2).sqrt();
    let lambert = ((LIGHT.0 * dx + LIGHT.1 * dy + LIGHT.2 * nz) / length).max(0.0);
    Some((AMBIENT + (1.0 - AMBIENT) * lambert) * (1.0 - RIM + RIM * nz.sqrt()))
}

/// Every dot of a static Bruh filling a `size` × `size` box (face radius
/// 0.42 × size) looking straight ahead. See [`bruh_dots_styled`].
pub fn bruh_dots(size: f32) -> Vec<Dot> {
    bruh_dots_styled(size, &FaceStyle::default())
}

/// A point of the front-facing face (in face radii) lifted onto the sphere.
fn lift(x: f32, y: f32) -> (f32, f32, f32) {
    (x, y, (1.0 - x * x - y * y).max(0.0).sqrt())
}

/// The head's turn: a tilt (pitch, positive looks down) then a turn (yaw,
/// positive looks right), on screen axes with y down and z toward the viewer.
struct Rotation {
    cos_y: f32,
    sin_y: f32,
    cos_p: f32,
    sin_p: f32,
}

impl Rotation {
    fn new(yaw: f32, pitch: f32) -> Self {
        Self {
            cos_y: yaw.cos(),
            sin_y: yaw.sin(),
            cos_p: pitch.cos(),
            sin_p: pitch.sin(),
        }
    }

    fn to_screen(&self, (x, y, z): (f32, f32, f32)) -> (f32, f32, f32) {
        let ny = y * self.cos_p + z * self.sin_p;
        let nz = -y * self.sin_p + z * self.cos_p;
        (
            x * self.cos_y + nz * self.sin_y,
            ny,
            -x * self.sin_y + nz * self.cos_y,
        )
    }

    fn to_local(&self, x: f32, y: f32, z: f32) -> (f32, f32) {
        let lx = x * self.cos_y - z * self.sin_y;
        let lz = x * self.sin_y + z * self.cos_y;
        (lx, y * self.cos_p - lz * self.sin_p)
    }
}

/// One feature of the face, placed on the turned head.
struct Feature {
    at: (f32, f32),
    rx: f32,
    /// Screen height (face radii below the center) of its line.
    line: f32,
    /// Fades out as the feature turns round the rim.
    fade: f32,
}

/// Every dot of a Bruh filling a `size` × `size` box (face radius 0.42 ×
/// size), on the grid [`grid_for`] fits to the face, each dot's area tracking
/// the light. Looking turns the whole head in 3D (the features ride the
/// sphere, under a fixed light) and leans the ball a little; a face looking
/// straight ahead is exactly symmetric. Features are anti-aliased: a dot
/// shrinks (eyes) or reddens (mouth) by how much of the feature covers it, so
/// a turning face glides through the grid. Vertically a feature is a line
/// shared between the two rows around it; at rest it sits exactly on a row,
/// which keeps one-row slits. Closed eyes shrink an eye-covered dot to
/// [`CLOSED_EYE_SCALE`] instead of removing it, leaving a thin line.
/// A port of `bruhDots` in bruhs.ai's `src/scripts/braille.js`.
pub fn bruh_dots_styled(size: f32, style: &FaceStyle) -> Vec<Dot> {
    if size.is_nan() || size <= 0.0 {
        return Vec::new();
    }
    let look_x = if style.look_x.is_nan() {
        0.0
    } else {
        style.look_x.clamp(-1.0, 1.0)
    };
    let look_y = if style.look_y.is_nan() {
        0.0
    } else {
        style.look_y.clamp(-1.0, 1.0)
    };
    let r = size * 0.42;
    let cx = size / 2.0 + look_x * LEAN * r;
    let cy = size / 2.0 + look_y * LEAN * r;
    let Grid {
        pitch,
        row_h,
        phase,
    } = grid_for(r);
    let head = Rotation::new(look_x * MAX_YAW, look_y * MAX_PITCH);
    let soft = pitch * 0.6 / r;
    let land = |at: (f32, f32), rx: f32| {
        let seen = head.to_screen(lift(at.0, at.1));
        // On the grid `grid_for` fits, this is exactly `at.1` for the eyes and
        // the mouth.
        let rest_line = (phase + ((at.1 * r - phase) / row_h).round() * row_h) / r;
        Feature {
            at,
            rx,
            line: seen.1 + rest_line - at.1,
            fade: smoothstep(0.05, 0.3, seen.2),
        }
    };
    let left_eye = land((-EYE_X, EYE_Y), EYE_RX);
    let right_eye = land((EYE_X, EYE_Y), EYE_RX);
    let mouth = land(MOUTH, MOUTH_RX);
    // How much of a feature covers the dot at screen height `dy` whose point
    // on the head's own surface is `local`, 0 to 1.
    let cover = |f: &Feature, dy: f32, local: (f32, f32)| {
        let across = (1.0 - (dy - f.line).abs() * r / row_h).max(0.0);
        if across < 1e-4 || (local.1 - f.at.1).abs() > 0.2 {
            return 0.0;
        }
        let along = 1.0 - smoothstep(f.rx, f.rx + soft, (local.0 - f.at.0).abs());
        across * along * f.fade
    };
    let eye_keep = match style.eyes {
        Eyes::Open => 0.0,
        Eyes::Closed => CLOSED_EYE_SCALE,
    };

    let rows = (r / row_h).ceil().to_i32_saturating() + 2;
    let cols = (r / pitch).ceil().to_i32_saturating() + 1;
    let mut dots = Vec::new();
    for row in -rows..=rows {
        let y = cy + phase + row.to_f32_lossy() * row_h;
        for col in -cols..=cols {
            // Offsets in whole half-steps, so a dot and its mirror twin get
            // exactly negated positions and a front-facing face stays symmetric.
            let half_steps = (2 * col + row.rem_euclid(2)).to_f32_lossy();
            let x = cx + half_steps * pitch / 2.0;
            let (dx, dy) = (
                half_steps * pitch / (2.0 * r),
                (phase + row.to_f32_lossy() * row_h) / r,
            );
            let Some(mut value) = shade(dx, dy) else {
                continue;
            };
            let local = head.to_local(dx, dy, (1.0 - dx * dx - dy * dy).max(0.0).sqrt());
            let eye = cover(&left_eye, dy, local).max(cover(&right_eye, dy, local));
            let mix = cover(&mouth, dy, local);
            value += (value.max(0.75) - value) * mix;
            let radius = pitch / 2.0 * 0.92 * value.sqrt() * (1.0 - eye * (1.0 - eye_keep));
            if radius < pitch * 0.07 {
                continue;
            }
            let kind = if mix >= 0.5 {
                DotKind::Mouth
            } else {
                DotKind::Face
            };
            dots.push(Dot {
                x,
                y,
                radius,
                kind,
                mix,
            });
        }
    }
    dots
}

/// 0 below `a`, 1 above `b`, eased between.
fn smoothstep(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Lit dots (0 to 4) in one column of a braille level meter, for a level in
/// [0, 1] such as audio RMS. Square-rooted so quiet speech still shows: 0.05
/// lights one dot, 0.25 two, 0.6 three, 1.0 four.
pub fn meter_dots(level: f32) -> u8 {
    if level.is_nan() || level <= 0.0 {
        return 0;
    }
    (level.min(1.0).sqrt() * 4.0)
        .round()
        .to_u8_saturating()
        .min(4)
}

/// One meter column, top to bottom: whether each dot is lit. Fills from the
/// bottom, like a level.
pub fn meter_column(level: f32) -> [bool; 4] {
    let lit = meter_dots(level);
    [lit >= 4, lit >= 3, lit >= 2, lit >= 1]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nearest(dots: &[Dot], x: f32, y: f32) -> Dot {
        *dots
            .iter()
            .min_by(|a, b| {
                let da = (a.x - x).hypot(a.y - y);
                let db = (b.x - x).hypot(b.y - y);
                da.total_cmp(&db)
            })
            .expect("a Bruh has dots")
    }

    #[test]
    fn the_sphere_is_lit_from_the_upper_left_and_keeps_its_shadow_side() {
        assert!(shade(-0.3, -0.4).unwrap() > shade(0.5, 0.5).unwrap());
        assert!(shade(0.9, 0.3).unwrap() > 0.1);
        assert_eq!(shade(0.8, 0.8), None);
    }

    #[test]
    fn the_grid_scales_with_the_face() {
        assert!((pitch_for(150.0) - 10.0).abs() < 1e-4);
        assert!((pitch_for(40.0) - 40.0 / 9.0).abs() < 1e-4);
    }

    #[test]
    fn a_bruh_is_round_symmetric_and_lit_with_eye_holes_and_a_one_row_mouth() {
        let size = 360.0;
        let dots = bruh_dots(size);
        let (c, r) = (size / 2.0, size * 0.42);
        assert!(dots.iter().all(|d| (d.x - c).hypot(d.y - c) <= r + 1e-3));
        let left = dots.iter().filter(|d| d.x < c - 1.0).count();
        let right = dots.iter().filter(|d| d.x > c + 1.0).count();
        assert!(left.abs_diff(right) <= 2, "left {left} vs right {right}");
        assert!(
            nearest(&dots, c - 0.4 * r, c - 0.5 * r).radius
                > nearest(&dots, c + 0.5 * r, c + 0.6 * r).radius
        );
        // Eye slits: one row, mirror images (every face dot has a twin).
        let eye_y = c + EYE_Y * r;
        let in_slit = |d: &&Dot, side: f32| {
            (d.y - eye_y).abs() < 1e-3 && (d.x - (c + side * EYE_X * r)).abs() <= EYE_RX * r
        };
        assert!(!dots.iter().any(|d| in_slit(&d, -1.0) || in_slit(&d, 1.0)));
        let mouth_y = c + MOUTH.1 * r;
        let face: Vec<&Dot> = dots
            .iter()
            .filter(|d| d.kind == DotKind::Face && (d.y - mouth_y).abs() > 1e-3)
            .collect();
        assert!(face.iter().all(|d| face
            .iter()
            .any(|t| (t.x - (2.0 * c - d.x)).abs() < 0.01 && (t.y - d.y).abs() < 0.01)));
        let mouth: Vec<&Dot> = dots.iter().filter(|d| d.kind == DotKind::Mouth).collect();
        assert!(mouth.len() >= 6);
        assert!(mouth.iter().all(|d| (d.y - mouth[0].y).abs() < 1e-3));
        assert!(bruh_dots(0.0).is_empty());
    }

    #[test]
    fn the_eye_and_mouth_lines_land_exactly_where_the_logo_has_them() {
        for size in [48.0, 96.0, 180.0, 360.0, 720.0] {
            let (c, r) = (size / 2.0, size * 0.42);
            let grid = grid_for(r);
            assert!(grid.pitch <= pitch_for(r) + 1e-4, "{size}: never coarser");
            let dots = bruh_dots(size);
            let on_row = |y: f32| dots.iter().any(|d| (d.y - y).abs() < 1e-3);
            assert!(on_row(c + EYE_Y * r), "{size}: a row on the eye line");
            assert!(on_row(c + MOUTH.1 * r), "{size}: a row on the mouth line");
            let mouth: Vec<&Dot> = dots.iter().filter(|d| d.kind == DotKind::Mouth).collect();
            assert!(!mouth.is_empty(), "{size}: a mouth");
            assert!(
                mouth.iter().all(|d| (d.y - (c + MOUTH.1 * r)).abs() < 1e-3),
                "{size}: red off the mouth line"
            );
        }
    }

    #[test]
    fn mouth_dots_blend_from_face_yellow_to_mouth_red() {
        let dot = |mix: f32| Dot {
            x: 0.0,
            y: 0.0,
            radius: 1.0,
            kind: DotKind::Face,
            mix,
        };
        assert_eq!(dot(0.0).rgb(), BRUH_FACE_RGB);
        assert_eq!(dot(1.0).rgb(), BRUH_MOUTH_RGB);
        let half = dot(0.5).rgb();
        assert!(half != BRUH_FACE_RGB && half != BRUH_MOUTH_RGB);
        assert_eq!(dot(0.49).rgb(), half);
    }

    #[test]
    fn turning_moves_the_features_toward_the_turn() {
        let size = 180.0;
        let at = |x: f32| {
            bruh_dots_styled(
                size,
                &FaceStyle {
                    look_x: x,
                    ..Default::default()
                },
            )
        };
        let mouth_x = |dots: &[Dot]| {
            let m: Vec<_> = dots.iter().filter(|d| d.kind == DotKind::Mouth).collect();
            m.iter().map(|d| d.x).sum::<f32>() / m.len().to_f32_lossy()
        };
        let (left, front, right) = (at(-0.6), at(0.0), at(0.6));
        assert!(mouth_x(&right) > mouth_x(&front) + 1.0);
        assert!(mouth_x(&left) < mouth_x(&front) - 1.0);
        assert_eq!(bruh_dots(size), front);
    }

    #[test]
    fn closed_eyes_draw_a_thin_line_instead_of_a_hole() {
        let size = 180.0;
        let (c, r) = (size / 2.0, size * 0.42);
        let closed = FaceStyle {
            eyes: Eyes::Closed,
            ..Default::default()
        };
        let open = bruh_dots(size);
        let shut = bruh_dots_styled(size, &closed);
        assert!(shut.len() > open.len());
        let pitch = grid_for(r).pitch;
        let eye_y = c + EYE_Y * r;
        let slit: Vec<&Dot> = shut
            .iter()
            .filter(|d| {
                (d.y - eye_y).abs() < 1e-3
                    && [-1.0, 1.0]
                        .iter()
                        .any(|s| (d.x - (c + s * EYE_X * r)).abs() <= EYE_RX * r)
            })
            .collect();
        assert!(!slit.is_empty());
        assert!(slit
            .iter()
            .all(|d| d.radius <= CLOSED_EYE_SCALE * pitch / 2.0 * 0.92 + 1e-3));
    }

    #[test]
    fn styled_colours_blend_face_to_mouth() {
        let style = FaceStyle {
            face_rgb: 0x0087_be52,
            mouth_rgb: 0x00fc_4141,
            ..Default::default()
        };
        let dot = |mix: f32| Dot {
            x: 0.0,
            y: 0.0,
            radius: 1.0,
            kind: DotKind::Face,
            mix,
        };
        assert_eq!(dot(0.0).rgb_in(&style), 0x0087_be52);
        assert_eq!(dot(1.0).rgb_in(&style), 0x00fc_4141);
        assert_eq!(dot(0.5).rgb(), dot(0.5).rgb_in(&FaceStyle::default()));
    }

    #[test]
    fn look_is_clamped() {
        let at = |x: f32| {
            bruh_dots_styled(
                120.0,
                &FaceStyle {
                    look_x: x,
                    ..Default::default()
                },
            )
        };
        assert_eq!(at(5.0), at(1.0));
        assert_eq!(at(-5.0), at(-1.0));
    }

    #[test]
    fn the_meter_fills_from_the_bottom_on_a_square_root_scale() {
        let lit: Vec<u8> = [0.0, 0.05, 0.25, 0.6, 1.0, 2.0, f32::NAN, -1.0]
            .map(meter_dots)
            .to_vec();
        assert_eq!(lit, [0, 1, 2, 3, 4, 4, 0, 0]);
        assert_eq!(meter_column(0.25), [false, false, true, true]);
        assert_eq!(meter_column(0.0), [false; 4]);
    }
}

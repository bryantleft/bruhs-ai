//! Dither, the Bruhs brand texture, for the app: the Bruh mascot as a
//! halftone of dots, and a level meter drawn as one braille cell. A port of
//! `@bruhs/theme/dither` (AGENTS.md section 19) and of bruhs.ai's
//! `src/scripts/braille.js`, so every surface places the same dots. Pure math;
//! each surface draws.

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
const EYE_Y: f32 = -0.257;
const MOUTH: (f32, f32) = (0.023, 0.26);
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
    pub kind: DotKind,
}

/// Grid spacing for a face radius `r`: about 15 dots per radius on a large
/// face, down to 9 on a small one (fewer turns the outline into a hexagon).
pub fn pitch_for(r: f32) -> f32 {
    r / (r / 10.0).clamp(9.0, 15.0)
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
/// 0.42 × size): a hex grid anchored on the face's center, each dot's area
/// tracking the light. The eyes and mouth are one row each: an oval hole cut
/// across staggered rows comes out ▽ on one eye and △ on the other.
pub fn bruh_dots(size: f32) -> Vec<Dot> {
    if size.is_nan() || size <= 0.0 {
        return Vec::new();
    }
    let (cx, cy, r) = (size / 2.0, size / 2.0, size * 0.42);
    let pitch = pitch_for(r);
    let row_h = pitch * 3.0_f32.sqrt() / 2.0;
    let eye_row = (EYE_Y * r / row_h).round().to_i32_saturating();
    let mouth_row = (MOUTH.1 * r / row_h).round().to_i32_saturating();
    let rows = (r / row_h).ceil().to_i32_saturating() + 1;
    let cols = (r / pitch).ceil().to_i32_saturating() + 1;

    let mut dots = Vec::new();
    for row in -rows..=rows {
        let y = cy + row.to_f32_lossy() * row_h;
        for col in -cols..=cols {
            // Offsets in whole half-steps, so a dot and its mirror twin get
            // exactly negated positions and the face stays symmetric.
            let half_steps = (2 * col + row.rem_euclid(2)).to_f32_lossy();
            let x = cx + half_steps * pitch / 2.0;
            let (dx, dy) = (half_steps * pitch / (2.0 * r), (y - cy) / r);
            let Some(mut value) = shade(dx, dy) else {
                continue;
            };
            if row == eye_row && ((dx.abs() - EYE_X) / EYE_RX).abs() <= 1.0 {
                continue;
            }
            let is_mouth = row == mouth_row && ((dx - MOUTH.0) / MOUTH_RX).abs() <= 1.0;
            if is_mouth {
                value = value.max(0.75);
            }
            let radius = pitch / 2.0 * 0.92 * value.sqrt();
            if radius < pitch * 0.07 {
                continue;
            }
            let kind = if is_mouth {
                DotKind::Mouth
            } else {
                DotKind::Face
            };
            dots.push(Dot { x, y, radius, kind });
        }
    }
    dots
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
        let row_h = pitch_for(r) * 3.0_f32.sqrt() / 2.0;
        let eye_y = c + (EYE_Y * r / row_h).round() * row_h;
        let in_slit = |d: &&Dot, side: f32| {
            (d.y - eye_y).abs() < 1e-3 && (d.x - (c + side * EYE_X * r)).abs() <= EYE_RX * r
        };
        assert!(!dots.iter().any(|d| in_slit(&d, -1.0) || in_slit(&d, 1.0)));
        let mouth_y = c + (MOUTH.1 * r / row_h).round() * row_h;
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
    fn the_meter_fills_from_the_bottom_on_a_square_root_scale() {
        let lit: Vec<u8> = [0.0, 0.05, 0.25, 0.6, 1.0, 2.0, f32::NAN, -1.0]
            .map(meter_dots)
            .to_vec();
        assert_eq!(lit, [0, 1, 2, 3, 4, 4, 0, 0]);
        assert_eq!(meter_column(0.25), [false, false, true, true]);
        assert_eq!(meter_column(0.0), [false; 4]);
    }
}

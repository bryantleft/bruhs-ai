//! Dither, the Bruhs brand texture: the Bruh mascot and a level meter drawn as
//! braille-style dots (cells of 2 columns × 4 rows). A port of
//! `@bruhs/theme/dither` (AGENTS.md section 19) so the web canvas and the
//! desktop painter place the same dots. Pure math; each surface draws.

use crate::num::{ToFloatLossy, ToIntSaturating};

/// 4×4 Bayer matrix, row-major. Thresholds are `(n + 0.5) / 16`.
const BAYER: [u8; 16] = [0, 8, 2, 10, 12, 4, 14, 6, 3, 11, 1, 9, 15, 7, 13, 5];

/// Ordered-dither threshold in (0, 1) for the dot at grid position (col, row).
pub fn threshold(col: usize, row: usize) -> f32 {
    let n = BAYER[(row % 4) * 4 + col % 4];
    (f32::from(n) + 0.5) / 16.0
}

/// The Bruh's colors, from `bruhs.svg` (theme tokens `bruh-base`, `bruh-mouth`).
pub const BRUH_FACE_RGB: u32 = 0x00fa_da5e;
pub const BRUH_MOUTH_RGB: u32 = 0x00fc_4141;

// Geometry in face radii, from the logo's 600×600 viewBox (r = 300), facing
// forward (`bruhGeometry.faces.original`).
const LEFT_EYE: (f32, f32) = (-0.34, -0.257);
const RIGHT_EYE: (f32, f32) = (0.337, -0.257);
const MOUTH: (f32, f32) = (0.023, 0.26);
const EYE_RX: f32 = 40.0 / 300.0;
const MOUTH_RX: f32 = 120.0 / 300.0;
const FEATURE_RY: f32 = 10.0 / 300.0;
// Light from the upper left, mostly frontal.
const LIGHT: (f32, f32, f32) = (-0.36, -0.42, 0.83);

/// Which layer a dot belongs to; each is drawn in one color.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DotKind {
    Face,
    Mouth,
    /// Sparse dots just outside the face, drawn in a quiet neutral.
    Halo,
}

/// One dot to draw: its center, in the same units as the box it was laid out in.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Dot {
    pub x: f32,
    pub y: f32,
    pub kind: DotKind,
}

/// Spacing of the dot grid. [`DotGrid::with_pitch`] gives the Bruhs proportions.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DotGrid {
    /// Distance between dots inside a cell.
    pub pitch: f32,
    /// Extra space between cells, which is what makes the dots read as braille.
    pub gap: f32,
    /// Dot radius.
    pub dot: f32,
}

impl DotGrid {
    pub fn with_pitch(pitch: f32) -> Self {
        Self {
            pitch,
            gap: pitch * 0.6,
            dot: pitch * 0.36,
        }
    }

    /// The grid for a Bruh `size` across: about 18 dots per face radius so
    /// the eyes stay legible on a small mark, 1.5 to 6 px apart.
    pub fn for_size(size: f32) -> Self {
        Self::with_pitch((size * 0.4 / 18.0).clamp(1.5, 6.0))
    }
}

/// What lies under a point of a Bruh centered at (`cx`, `cy`) with radius `r`:
/// its layer and brightness (0 to 1), or `None` inside an eye, which is a hole.
/// `min_feature` is the thinnest an eye or the mouth may be, in face radii, so
/// they survive a coarse grid.
pub fn sample(
    x: f32,
    y: f32,
    cx: f32,
    cy: f32,
    r: f32,
    min_feature: f32,
) -> Option<(DotKind, f32)> {
    let dx = (x - cx) / r;
    let dy = (y - cy) / r;
    let d2 = dx * dx + dy * dy;
    if d2 > 1.0 {
        // Fades to nothing a fifth of a radius out.
        let fade = (1.0 - (d2.sqrt() - 1.0) / 0.2).max(0.0);
        return Some((DotKind::Halo, 0.24 * fade * fade));
    }
    let ry = FEATURE_RY.max(min_feature);
    let inside = |(ex, ey): (f32, f32), rx: f32| {
        let u = (dx - ex) / rx;
        let v = (dy - ey) / ry;
        u * u + v * v <= 1.0
    };
    if inside(MOUTH, MOUTH_RX) {
        return Some((DotKind::Mouth, 1.0));
    }
    if inside(LEFT_EYE, EYE_RX) || inside(RIGHT_EYE, EYE_RX) {
        return None;
    }
    let nz = (1.0 - d2).sqrt();
    let lambert = (LIGHT.0 * dx + LIGHT.1 * dy + LIGHT.2 * nz).max(0.0);
    Some((DotKind::Face, (0.1 + lambert).min(1.0)))
}

/// Every dot for a Bruh filling a `size` × `size` box (face radius 0.4 × size),
/// grid centered. Static by design: the app draws the finished face.
pub fn bruh_dots(size: f32, grid: DotGrid) -> Vec<Dot> {
    let DotGrid { pitch, gap, .. } = grid;
    if size.is_nan() || pitch.is_nan() || size <= 0.0 || pitch <= 0.0 {
        return Vec::new();
    }
    let cell_w = pitch * 2.0 + gap;
    let cell_h = pitch * 4.0 + gap;
    let cols = (size / cell_w).ceil().to_usize_saturating();
    let rows = (size / cell_h).ceil().to_usize_saturating();
    let ox = (size - cols.to_f32_lossy() * cell_w + gap + pitch) / 2.0;
    let oy = (size - rows.to_f32_lossy() * cell_h + gap + pitch) / 2.0;
    let (cx, cy, r) = (size / 2.0, size / 2.0, size * 0.4);
    // Eyes and mouth at least 1.8 dot rows tall, so they read past the gaps
    // between braille cells on a small mark.
    let min_feature = pitch * 1.8 / r;

    let mut dots = Vec::new();
    for cell_row in 0..rows {
        for cell_col in 0..cols {
            for dr in 0..4_usize {
                for dc in 0..2_usize {
                    let x = ox + cell_col.to_f32_lossy() * cell_w + dc.to_f32_lossy() * pitch;
                    let y = oy + cell_row.to_f32_lossy() * cell_h + dr.to_f32_lossy() * pitch;
                    let Some((kind, value)) = sample(x, y, cx, cy, r, min_feature) else {
                        continue;
                    };
                    if value > threshold(cell_col * 2 + dc, cell_row * 4 + dr) {
                        dots.push(Dot { x, y, kind });
                    }
                }
            }
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

    #[test]
    fn every_bayer_level_appears_once_per_tile() {
        let mut levels: Vec<f32> = (0..4)
            .flat_map(|row| (0..4).map(move |col| threshold(col, row)))
            .collect();
        levels.sort_by(f32::total_cmp);
        levels.dedup();
        assert_eq!(levels.len(), 16);
        assert!(levels.iter().all(|t| *t > 0.0 && *t < 1.0));
        assert_eq!(threshold(1, 2), threshold(5, 6));
    }

    #[test]
    fn the_face_is_lit_from_the_upper_left_with_holes_for_eyes() {
        let lit = sample(-30.0, -55.0, 0.0, 0.0, 100.0, 0.05);
        let shaded = sample(70.0, 60.0, 0.0, 0.0, 100.0, 0.05);
        assert!(matches!(lit, Some((DotKind::Face, _))));
        assert!(lit.unwrap().1 > shaded.unwrap().1);
        assert_eq!(sample(-34.0, -25.7, 0.0, 0.0, 100.0, 0.05), None);
        assert_eq!(sample(33.7, -25.7, 0.0, 0.0, 100.0, 0.05), None);
        assert!(matches!(
            sample(2.3, 26.0, 0.0, 0.0, 100.0, 0.05),
            Some((DotKind::Mouth, _))
        ));
    }

    #[test]
    fn the_halo_fades_out_before_the_box_edge() {
        let near = sample(110.0, 0.0, 0.0, 0.0, 100.0, 0.0).unwrap();
        assert_eq!(near.0, DotKind::Halo);
        assert!(near.1 > 0.0);
        assert_eq!(
            sample(125.0, 0.0, 0.0, 0.0, 100.0, 0.0),
            Some((DotKind::Halo, 0.0))
        );
    }

    #[test]
    fn a_bruh_has_face_mouth_and_halo_dots_inside_its_box() {
        let size = 96.0;
        let dots = bruh_dots(size, DotGrid::with_pitch(2.5));
        let count = |kind| dots.iter().filter(|d| d.kind == kind).count();
        assert!(count(DotKind::Face) > 200, "face {}", count(DotKind::Face));
        assert!(count(DotKind::Mouth) > 0);
        assert!(dots
            .iter()
            .all(|d| (0.0..=size).contains(&d.x) && (0.0..=size).contains(&d.y)));
        assert!(bruh_dots(0.0, DotGrid::with_pitch(2.5)).is_empty());
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

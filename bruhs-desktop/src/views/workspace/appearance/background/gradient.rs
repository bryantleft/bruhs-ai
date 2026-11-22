//! CSS gradient color stops, interpolation and native texture generation.
mod geometry;
use super::style::{Length, comma_list, words};
use geometry::{Projection, Shape, angle};
use image::{Rgba, RgbaImage};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum StopPosition {
    Length(Length),
    Angle(i64),
}

impl StopPosition {
    fn parse(token: &str, conic: bool) -> Option<Self> {
        if conic && !token.ends_with('%') {
            return angle(token).map(Self::Angle);
        }
        let length = Length::parse(token)?;
        (length != Length::Auto).then_some(Self::Length(length))
    }

    fn value(self, span: f32, zoom: f32) -> Option<f32> {
        match self {
            Self::Length(length) => length.value(span, zoom),
            Self::Angle(millidegrees) => Some(millidegrees as f32 / 1000.),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct Gradient {
    shape: Shape,
    stops: Vec<([u8; 4], Option<StopPosition>)>,
    repeating: bool,
}

impl Gradient {
    pub fn parse(input: &str) -> Option<Self> {
        let (name, body) = input.strip_suffix(')')?.split_once('(')?;
        let family = name.strip_prefix("repeating-").unwrap_or(name);
        let repeating = family != name;
        let mut parts = comma_list(body)?.into_iter().peekable();
        let mut shape = Shape::initial(family)?;
        if let Some(explicit) = shape.with_prelude(parts.peek()?) {
            shape = explicit;
            parts.next();
        }
        let mut stops = Vec::new();
        let mut colors = 0;
        for part in parts {
            let tokens = words(part)?;
            let (color, positions) = tokens.split_first()?;
            let color = csscolorparser::parse(color).ok()?.to_rgba8();
            colors += 1;
            if positions.len() > 2 {
                return None;
            }
            if positions.is_empty() {
                stops.push((color, None));
            }
            for position in positions {
                let value = StopPosition::parse(position, shape.is_conic())?;
                stops.push((color, Some(value)));
            }
        }
        (colors >= 2).then(|| Self {
            shape,
            stops,
            repeating,
        })
    }

    fn resolved(&self, span: f32, zoom: f32) -> Vec<(f32, [u8; 4])> {
        let mut result = Vec::with_capacity(self.stops.len());
        let mut last = 0;
        for (i, (color, position)) in self.stops.iter().enumerate() {
            let pos = position
                .and_then(|p| p.value(span, zoom))
                .or_else(|| (i == 0).then_some(0.))
                .or_else(|| (i + 1 == self.stops.len()).then_some(span));
            result.push((pos.unwrap_or(0.), *color));
            if let Some(mut end) = pos {
                if i > last {
                    let start = result[last].0;
                    end = end.max(start);
                    for j in last + 1..i {
                        result[j].0 = start + (end - start) * (j - last) as f32 / (i - last) as f32;
                    }
                }
                result[i].0 = end;
                last = i;
            }
        }
        result
    }

    // Chromium paints a zero-radius radial gradient as a solid. For a repeat,
    // percentage stops can still describe the relative coverage of its colors,
    // even though absolute distances have collapsed.
    fn collapsed_radial(&self) -> RgbaImage {
        let stops = self.resolved(1., 0.);
        let (last_pos, mut color) = *stops.last().unwrap();
        let period = last_pos - stops[0].0;
        if self.repeating && period > 0. {
            let mut mean = [0.; 4];
            for pair in stops.windows(2) {
                let (a, b) = (pair[0], pair[1]);
                let weight = (b.0 - a.0) / (2. * period);
                for (i, channel) in mean.iter_mut().enumerate() {
                    *channel += weight * (a.1[i] as f32 + b.1[i] as f32);
                }
            }
            color = mean.map(|v| v.round() as u8);
        }
        let [r, g, b, a] = color;
        RgbaImage::from_pixel(1, 1, Rgba([b, g, r, a]))
    }

    pub fn raster(&self, w: f32, h: f32, zoom: f32, scale: f32) -> RgbaImage {
        let projection = self.shape.resolve(w, h, zoom);
        if matches!(projection, Projection::Collapsed) {
            return self.collapsed_radial();
        }
        let span = projection.span();
        let stops = self.resolved(span, zoom);
        let (first, last) = (stops[0], stops[stops.len() - 1]);
        let width = if matches!(projection, Projection::Linear { dx: 0., .. }) {
            1
        } else {
            (w * scale).ceil().clamp(1., 2048.) as u32
        };
        let height = if matches!(projection, Projection::Linear { dy: 0., .. }) {
            1
        } else {
            (h * scale).ceil().clamp(1., 2048.) as u32
        };
        // Cache the ramp, including premultiplied alpha. Resolve the containing
        // segment first so adjacent stops at the same position keep a sharp edge.
        let ramps: Vec<_> = stops
            .windows(2)
            .map(|pair| {
                (0..=255)
                    .map(|i| {
                        let t = i as f32 / 255.;
                        let (a, b) = (pair[0].1, pair[1].1);
                        let left = a[3] as f32 * (1. - t);
                        let right = b[3] as f32 * t;
                        let alpha = left + right;
                        let channel = |j: usize| {
                            ((left * a[j] as f32 + right * b[j] as f32) / alpha).round() as u8
                        };
                        Rgba([channel(2), channel(1), channel(0), alpha.round() as u8])
                    })
                    .collect::<Vec<_>>()
            })
            .collect();
        let bgra = |[r, g, b, a]: [u8; 4]| Rgba([b, g, r, a]);
        RgbaImage::from_fn(width, height, |x, y| {
            let px = (x as f32 + 0.5) * (w / width as f32);
            let py = (y as f32 + 0.5) * (h / height as f32);
            let mut at = projection.at(px, py);
            if self.repeating {
                if last.0 == first.0 {
                    return bgra(last.1);
                }
                at = (at - first.0).rem_euclid(last.0 - first.0) + first.0;
            }
            if at < first.0 {
                return bgra(first.1);
            }
            if at >= last.0 {
                return bgra(last.1);
            }
            let next = stops.partition_point(|(pos, _)| *pos <= at);
            let prev = next - 1;
            let fraction = (at - stops[prev].0) / (stops[next].0 - stops[prev].0);
            ramps[prev][(fraction * 255.).round() as usize]
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hard_stops_repeat_and_scale_in_css_pixels() {
        let g = Gradient::parse("repeating-linear-gradient(to right, red 0 2px, blue 2px 4px)")
            .unwrap();
        let tile = g.raster(16., 10., 2., 1.);
        let pixels = tile.pixels().map(|p| p.0).collect::<Vec<_>>();
        assert_eq!(tile.dimensions(), (16, 1));
        for (i, p) in pixels.iter().enumerate() {
            assert_eq!(
                *p,
                if i % 8 < 4 {
                    [0, 0, 255, 255]
                } else {
                    [255, 0, 0, 255]
                }
            );
        }
    }

    #[test]
    fn transparency_unspecified_stops_and_corner_direction() {
        let g = Gradient::parse(
            "linear-gradient(to top right, transparent, rgb(255 0 0 / 100%), white)",
        )
        .unwrap();
        assert_eq!(g.resolved(100., 1.)[1].0, 50.);
        let Projection::Linear { dx: x, dy: y, .. } = g.shape.resolve(400., 100., 1.) else {
            panic!("linear gradient")
        };
        assert!((x / -y - 0.25).abs() < 0.00001);
        let fade = Gradient::parse("linear-gradient(.25turn, transparent, red)").unwrap();
        let pixel = fade.raster(1., 1., 1., 1.).get_pixel(0, 0).0;
        assert_eq!(pixel, [0, 0, 255, 128]);
    }
    #[test]
    fn angular_stop_positions_keep_negative_and_multi_turn_angles() {
        let g = Gradient::parse(
            "repeating-conic-gradient(from 15deg, #fff -50grad, red, blue 1.25turn)",
        )
        .unwrap();
        let stops = g.resolved(360., 2.);
        assert_eq!([stops[0].0, stops[1].0, stops[2].0], [-45., 202.5, 450.]);
        for invalid in [
            "conic-gradient(red 4px, blue)",
            "radial-gradient(red 75deg, blue)",
            "radial-gradient(circle 50%, red, blue)",
            "radial-gradient(ellipse -3px 1%, red, blue)",
            "radial-gradient(ellipse circle, red, blue)",
            "conic-gradient(at right 3px right 2px, red, blue)",
        ] {
            assert!(Gradient::parse(invalid).is_none(), "{invalid}");
        }
    }
    #[test]
    fn collapsed_radial_period_preserves_color_coverage() {
        for (source, expected) in [
            (
                "radial-gradient(0px 10px, red 10px, blue 35px)",
                [255, 0, 0, 255],
            ),
            (
                "repeating-radial-gradient(circle 0px, red 0% 10%, blue 10% 100%)",
                [230, 0, 26, 255],
            ),
            (
                "repeating-radial-gradient(10px 0px, transparent, red, blue)",
                [64, 0, 128, 191],
            ),
        ] {
            let tile = Gradient::parse(source).unwrap().raster(200., 100., 1., 2.);
            assert_eq!(tile.dimensions(), (1, 1));
            for (&actual, &want) in tile.get_pixel(0, 0).0.iter().zip(expected.iter()) {
                assert!(actual.abs_diff(want) <= 1);
            }
        }
    }
}

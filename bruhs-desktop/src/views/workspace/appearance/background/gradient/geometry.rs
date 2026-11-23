//! Resolve author coordinates against the size of one CSS background tile.
use super::super::style::{Length, position, words};
use bruhs_models::num::{ToFloatLossy, ToIntSaturating};

pub(super) fn angle(input: &str) -> Option<i64> {
    let degrees = if input.parse::<f64>() == Ok(0.) {
        0.
    } else {
        let (n, scale) = [
            ("grad", 0.9),
            ("rad", 180. / std::f64::consts::PI),
            ("deg", 1.),
            ("turn", 360.),
        ]
        .into_iter()
        .find_map(|(unit, scale)| Some((input.strip_suffix(unit)?, scale)))?;
        n.parse::<f64>().ok()? * scale
    };
    degrees
        .is_finite()
        .then(|| (degrees * 1000.).round().to_i64_saturating())
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum Direction {
    Angle(i32), // thousandths of a degree, clockwise from up
    Corner(i8, i8),
}

impl Direction {
    pub(super) fn parse(input: &str) -> Option<Self> {
        if let Some(rest) = input.strip_prefix("to ") {
            let mut x = 0;
            let mut y = 0;
            for side in rest.split_whitespace() {
                match side {
                    "left" if x == 0 => x = -1,
                    "right" if x == 0 => x = 1,
                    "top" if y == 0 => y = -1,
                    "bottom" if y == 0 => y = 1,
                    _ => return None,
                }
            }
            return ((x, y) != (0, 0)).then_some(Self::Corner(x, y));
        }
        Some(Self::Angle(
            i32::try_from(angle(input)?.rem_euclid(360000)).ok()?,
        ))
    }

    pub(super) fn vector(&self, w: f32, h: f32) -> (f32, f32) {
        let (x, y) = match self {
            Self::Angle(angle) => {
                let (s, c) = (angle.to_f32_lossy() / 1000.).to_radians().sin_cos();
                (s, -c)
            }
            Self::Corner(x, y) => (f32::from(*x) * h, f32::from(*y) * w),
        };
        let len = x.hypot(y);
        let snap = |v: f32| if v.abs() < 1e-6 { 0. } else { v };
        (snap(x / len), snap(y / len))
    }
}

type Center = (Length, Length);
const CENTER: Center = (Length::Percent(5000), Length::Percent(5000));

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum RadialSize {
    Extent {
        circle: bool,
        farthest: bool,
        corner: bool,
    },
    Radius(Length),
    Radii(Length, Length),
}

impl RadialSize {
    fn parse(tokens: &[&str]) -> Option<Self> {
        let mut circle = None;
        let mut size = Vec::new();
        for &token in tokens {
            match token {
                "circle" | "ellipse" => {
                    if circle.replace(token == "circle").is_some() {
                        return None;
                    }
                }
                _ => size.push(token),
            }
        }
        let length = |s| match Length::parse(s)? {
            l @ (Length::Px(n) | Length::Percent(n)) if n >= 0 => Some(l),
            _ => None,
        };
        match size.as_slice() {
            [] | ["farthest-corner" | "closest-corner" | "closest-side" | "farthest-side"] => {
                let extent = size.first().copied().unwrap_or("farthest-corner");
                Some(Self::Extent {
                    circle: circle.unwrap_or(false),
                    farthest: extent.starts_with("farthest"),
                    corner: extent.ends_with("corner"),
                })
            }
            [r] if circle != Some(false) => {
                let l @ Length::Px(_) = length(r)? else {
                    return None;
                };
                Some(Self::Radius(l))
            }
            [x, y] if circle != Some(true) => Some(Self::Radii(length(x)?, length(y)?)),
            _ => None,
        }
    }

    fn radii(self, x: f32, y: f32, w: f32, h: f32, zoom: f32) -> (f32, f32) {
        match self {
            Self::Radius(r) => {
                let r = r
                    .value(w, zoom)
                    .expect("parsed radial radius is a numeric length");
                (r, r)
            }
            Self::Radii(rx, ry) => (
                rx.value(w, zoom)
                    .expect("parsed radial x-radius is a numeric length"),
                ry.value(h, zoom)
                    .expect("parsed radial y-radius is a numeric length"),
            ),
            Self::Extent {
                circle,
                farthest,
                corner,
            } => {
                let distance = if farthest { f32::max } else { f32::min };
                let dx = distance(x.abs(), (w - x).abs());
                let dy = distance(y.abs(), (h - y).abs());
                if circle {
                    let r = if corner {
                        dx.hypot(dy)
                    } else {
                        distance(dx, dy)
                    };
                    (r, r)
                } else {
                    let scale = if corner { std::f32::consts::SQRT_2 } else { 1. };
                    (scale * dx, scale * dy)
                }
            }
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum Shape {
    Linear(Direction),
    Radial { size: RadialSize, center: Center },
    Conic { from: i64, center: Center },
}

impl Shape {
    pub fn initial(family: &str) -> Option<Self> {
        match family {
            "linear-gradient" => Some(Self::Linear(Direction::Angle(180000))),
            "radial-gradient" => Some(Self::Radial {
                size: RadialSize::Extent {
                    circle: false,
                    farthest: true,
                    corner: true,
                },
                center: CENTER,
            }),
            "conic-gradient" => Some(Self::Conic {
                from: 0,
                center: CENTER,
            }),
            _ => None,
        }
    }

    pub fn is_conic(&self) -> bool {
        matches!(self, Self::Conic { .. })
    }

    pub fn with_prelude(&self, input: &str) -> Option<Self> {
        if matches!(self, Self::Linear(_)) {
            return Direction::parse(input).map(Self::Linear);
        }
        let tokens = words(input)?;
        if tokens.is_empty() {
            return None;
        }
        let (args, center) = match tokens.iter().position(|&t| t == "at") {
            Some(i) => (&tokens[..i], position(&tokens[i + 1..])?),
            None => (tokens.as_slice(), CENTER),
        };
        match self {
            Self::Radial { .. } => Some(Self::Radial {
                size: RadialSize::parse(args)?,
                center,
            }),
            Self::Conic { .. } => {
                let from = match args {
                    [] => 0,
                    ["from", a] => angle(a)?.rem_euclid(360000),
                    _ => return None,
                };
                Some(Self::Conic { from, center })
            }
            Self::Linear(_) => unreachable!(),
        }
    }

    pub fn resolve(&self, w: f32, h: f32, zoom: f32) -> Projection {
        let center = |(x, y): Center| {
            (
                x.value(w, zoom)
                    .expect("parsed gradient center x is a numeric length"),
                y.value(h, zoom)
                    .expect("parsed gradient center y is a numeric length"),
            )
        };
        match self {
            Self::Linear(direction) => {
                let (dx, dy) = direction.vector(w, h);
                let span = w * dx.abs() + h * dy.abs();
                Projection::Linear {
                    dx,
                    dy,
                    start: (span - w * dx - h * dy) * 0.5,
                    span,
                }
            }
            Self::Radial { size, center: at } => {
                let (x, y) = center(*at);
                let (rx, ry) = size.radii(x, y, w, h, zoom);
                if rx == 0. || ry == 0. {
                    return Projection::Collapsed;
                }
                Projection::Radial {
                    x,
                    y,
                    radius: rx,
                    aspect: rx / ry,
                }
            }
            Self::Conic { from, center: at } => {
                let (x, y) = center(*at);
                Projection::Conic {
                    x,
                    y,
                    from: from.to_f32_lossy() / 1000.,
                }
            }
        }
    }
}

pub(super) enum Projection {
    Collapsed,
    Linear {
        dx: f32,
        dy: f32,
        start: f32,
        span: f32,
    },
    Radial {
        x: f32,
        y: f32,
        radius: f32,
        aspect: f32,
    },
    Conic {
        x: f32,
        y: f32,
        from: f32,
    },
}
impl Projection {
    pub fn span(&self) -> f32 {
        match *self {
            Self::Collapsed => 0.,
            Self::Linear { span, .. } => span,
            Self::Radial { radius, .. } => radius,
            Self::Conic { .. } => 360.,
        }
    }
    pub fn at(&self, px: f32, py: f32) -> f32 {
        match *self {
            Self::Collapsed => 0.,
            Self::Linear { dx, dy, start, .. } => px * dx + py * dy + start,
            Self::Radial { x, y, aspect, .. } => (px - x).hypot((py - y) * aspect),
            Self::Conic { x, y, from } => {
                ((px - x).atan2(y - py).to_degrees() - from).rem_euclid(360.)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn resolve(family: &str, prelude: &str) -> Projection {
        Shape::initial(family)
            .unwrap()
            .with_prelude(prelude)
            .unwrap()
            .resolve(200., 100., 1.)
    }
    #[test]
    fn radial_extents_use_distances_from_the_authored_center() {
        for (keyword, rx, ry) in [
            ("closest-side", 50., 25.),
            ("farthest-side", 150., 75.),
            (
                "closest-corner",
                50. * std::f32::consts::SQRT_2,
                25. * std::f32::consts::SQRT_2,
            ),
            (
                "farthest-corner",
                150. * std::f32::consts::SQRT_2,
                75. * std::f32::consts::SQRT_2,
            ),
        ] {
            let p = resolve("radial-gradient", &format!("{keyword} at 25% 25%"));
            assert!((p.span() - rx).abs() < 0.001);
            assert!((p.at(50., 25. + ry) - rx).abs() < 0.001);
        }
        let circle = resolve("radial-gradient", "circle farthest-corner at left top");
        assert!((circle.span() - 200_f32.hypot(100.)).abs() < 0.001);
        let ellipse = resolve("radial-gradient", "40% 10px at right 20px bottom 10%");
        assert_eq!(ellipse.at(180., 90.), 0.);
        assert_eq!(ellipse.at(180., 100.), 80.);
        let slim = resolve("radial-gradient", "0px 10px at center");
        assert!(matches!(slim, Projection::Collapsed));
    }

    #[test]
    fn conic_starts_at_twelve_and_rotates_clockwise() {
        let p = resolve("conic-gradient", "from -0.25turn at right 10px top 20px");
        for ((x, y), degrees) in [
            ((180., 20.), 0.),
            ((190., 10.), 90.),
            ((200., 20.), 180.),
            ((190., 30.), 270.),
        ] {
            assert!((p.at(x, y) - degrees).abs() < 0.001);
        }
    }
}

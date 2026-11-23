//! The converter uses sRGB for alpha composition and WCAG contrast, and OKLCH
//! for lightness adjustments that retain the source palette's hue.
use bruhs_models::num::ToIntSaturating;

fn linear(v: f64) -> f64 {
    if v <= 0.04045 {
        v / 12.92
    } else {
        ((v + 0.055) / 1.055).powf(2.4)
    }
}

fn encoded(v: f64) -> f64 {
    let v = v.clamp(0., 1.);
    if v <= 0.0031308 {
        12.92 * v
    } else {
        1.055 * v.powf(1. / 2.4) - 0.055
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) struct Rgb(pub [f64; 3]);

impl Rgb {
    pub(super) const fn hex(value: u32) -> Self {
        Self([
            ((value >> 16) & 255) as f64,
            ((value >> 8) & 255) as f64,
            (value & 255) as f64,
        ])
    }

    pub(super) fn css(self) -> String {
        let [r, g, b] = self.0.map(|v| v.round().to_u8_saturating());
        format!("#{r:02x}{g:02x}{b:02x}")
    }

    pub(super) fn mix(self, other: Self, amount: f64) -> Self {
        Self(std::array::from_fn(|i| {
            self.0[i] + (other.0[i] - self.0[i]) * amount
        }))
    }

    pub(super) fn rounded(self) -> Self {
        Self(self.0.map(|v| v.round().clamp(0., 255.)))
    }

    fn luminance(self) -> f64 {
        let [r, g, b] = self.0.map(|v| {
            let v = v / 255.;
            if v <= 0.03928 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        });
        0.2126 * r + 0.7152 * g + 0.0722 * b
    }

    pub(super) fn dark(self) -> bool {
        self.luminance() < 0.179
    }

    pub(super) fn contrast(self, other: Self) -> f64 {
        let a = self.luminance();
        let b = other.luminance();
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    }

    pub(super) fn readable_ink(self) -> Self {
        let light = Self::hex(0xfffaff);
        let dark = Self::hex(0x241523);
        let a = self.contrast(light);
        let b = self.contrast(dark);
        if a.max(b) >= 4.5 {
            if a >= b { light } else { dark }
        } else if self.dark() {
            Self::hex(0xffffff)
        } else {
            Self::hex(0)
        }
    }

    pub(super) fn muted_ink(self, ink: Self, ratio: f64) -> Self {
        if self.contrast(ink) < ratio {
            return ink;
        }
        let mut lo = 0.;
        let mut hi = 1.;
        for _ in 0..12 {
            let mid = (lo + hi) * 0.5;
            if self.contrast(ink.mix(self, mid)) >= ratio {
                lo = mid
            } else {
                hi = mid
            }
        }
        ink.mix(self, lo)
    }

    pub(super) fn lch(self) -> Lch {
        // OKLab's forward transform. Keep double precision here: even very
        // small chroma changes the derived surface hue on a neutral canvas.
        let [r, g, b] = self.0.map(|v| linear(v / 255.));
        let l = (0.4122214708 * r + 0.5363325363 * g + 0.0514459929 * b).cbrt();
        let m = (0.2119034982 * r + 0.6806995451 * g + 0.1073969566 * b).cbrt();
        let s = (0.0883024619 * r + 0.2817188376 * g + 0.6299787005 * b).cbrt();
        let lightness = 0.2104542553 * l + 0.793617785 * m - 0.0040720468 * s;
        let x = 1.9779984951 * l - 2.428592205 * m + 0.4505937099 * s;
        let y = 0.0259040371 * l + 0.7827717662 * m - 0.808675766 * s;
        Lch {
            l: lightness,
            c: x.hypot(y),
            h: y.atan2(x),
        }
    }
}

#[derive(Copy, Clone)]
pub(super) struct Lch {
    pub l: f64,
    pub c: f64,
    pub h: f64,
}

impl Lch {
    fn linear_rgb(self) -> [f64; 3] {
        let chroma = self.c;
        let a = chroma * self.h.cos();
        let b = chroma * self.h.sin();
        let l = (self.l + 0.3963377774 * a + 0.2158037573 * b).powi(3);
        let m = (self.l - 0.1055613458 * a - 0.0638541728 * b).powi(3);
        let s = (self.l - 0.0894841775 * a - 1.291485548 * b).powi(3);
        [
            4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s,
            -1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s,
            -0.0041960863 * l - 0.7034186147 * m + 1.707614701 * s,
        ]
    }

    fn fit(self) -> Self {
        let in_gamut = |channels: [f64; 3]| {
            channels
                .into_iter()
                .all(|v| (-0.0001..=1.0001).contains(&v))
        };
        if in_gamut(self.linear_rgb()) {
            return self;
        }
        let (mut lo, mut hi) = (0., self.c);
        while hi - lo > 0.000001 {
            let mid = (lo + hi) * 0.5;
            if in_gamut((Self { c: mid, ..self }).linear_rgb()) {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        Self { c: lo, ..self }
    }

    pub(super) fn rgb(self) -> Rgb {
        Rgb(self.fit().linear_rgb().map(|v| encoded(v) * 255.)).rounded()
    }

    // Theme files serialize OKLCH with six decimal places for lightness and
    // chroma and three for hue before resolving them to display colors.
    pub(super) fn serialized_rgb(self) -> Rgb {
        let c = self.fit();
        let precision = |value: f64, scale: f64| (value * scale).round() / scale;
        Self {
            l: precision(c.l, 1e6),
            c: precision(c.c, 1e6),
            h: precision(c.h.to_degrees().rem_euclid(360.), 1e3).to_radians(),
        }
        .rgb()
    }

    pub(super) fn ink_on(self, surface: Rgb, ratio: f64, lighter: bool) -> Rgb {
        self.ink(surface, ratio, lighter).rgb()
    }

    pub(super) fn ink(self, surface: Rgb, ratio: f64, lighter: bool) -> Self {
        if self.rgb().contrast(surface) >= ratio {
            return self;
        }
        let (mut lo, mut hi) = if lighter { (self.l, 1.) } else { (0., self.l) };
        for _ in 0..18 {
            let l = (lo + hi) * 0.5;
            let passes = (Self { l, ..self }).rgb().contrast(surface) >= ratio;
            if passes == lighter { hi = l } else { lo = l }
        }
        Self {
            l: if lighter { hi } else { lo },
            ..self
        }
    }
}

// VS Code workbench colors accept hex and color(srgb ...) / color(display-p3 ...).
pub(super) fn source_color(value: &str) -> Option<(Rgb, f64)> {
    let value = value.trim();
    let hex = value.strip_prefix('#').unwrap_or(value);
    if matches!(hex.len(), 3 | 4 | 6 | 8) && hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        let size = if hex.len() <= 4 { 1 } else { 2 };
        let mut bytes = [255u8; 4];
        for (i, chunk) in hex.as_bytes().chunks(size).enumerate() {
            let v = u8::from_str_radix(std::str::from_utf8(chunk).ok()?, 16).ok()?;
            bytes[i] = if size == 1 { v * 17 } else { v };
        }
        return Some((
            Rgb([bytes[0], bytes[1], bytes[2]].map(f64::from)),
            f64::from(bytes[3]) / 255.,
        ));
    }
    let body = value.strip_prefix("color(")?.strip_suffix(')')?.trim();
    let (space, components) = body.split_once(char::is_whitespace)?;
    let (channels, alpha) = components.split_once('/').unwrap_or((components, "1"));
    let number = |s: &str| {
        let s = s.trim();
        let v = if let Some(p) = s.strip_suffix('%') {
            p.parse::<f64>().ok()? / 100.
        } else {
            s.parse().ok()?
        };
        v.is_finite().then_some(v)
    };
    let mut channels = channels.split_whitespace();
    let rgb = [
        number(channels.next()?)?,
        number(channels.next()?)?,
        number(channels.next()?)?,
    ];
    if channels.next().is_some() {
        return None;
    }
    let rgb = if space.eq_ignore_ascii_case("srgb") {
        rgb
    } else if space.eq_ignore_ascii_case("display-p3") {
        let [r, g, b] = rgb.map(linear);
        [
            1.2249401762805 * r - 0.2249401762805 * g,
            -0.042056961239 * r + 1.042056961239 * g,
            -0.0196375547643 * r - 0.0786360655012 * g + 1.0982736202656 * b,
        ]
        .map(encoded)
    } else {
        return None;
    };
    Some((Rgb(rgb.map(|v| v * 255.)), number(alpha)?.clamp(0., 1.)))
}

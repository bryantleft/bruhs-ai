//! Paint theme background layers back to front and reload the active companion image on disk.
mod gradient;
mod style;
use gpui::{prelude::*, *};
use gradient::Gradient;
use std::{
    collections::{BTreeMap, VecDeque},
    fs,
    io::Read,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, SystemTime},
};
pub(super) use style::Style;
use style::{Geometry, Sizing, Source};

pub(super) type Styles = BTreeMap<&'static str, Style>;
const MAX_BYTES: u64 = 16 * 1024 * 1024;
pub(super) fn directory() -> Option<PathBuf> {
    crate::paths::desktop_state_file()?
        .parent()
        .map(|dir| dir.join("themes"))
}

#[derive(PartialEq, Eq)]
struct Revision {
    path: PathBuf,
    len: u64,
    modified: Option<SystemTime>,
    #[cfg(unix)]
    inode: u64,
}
fn discover(dir: &Path, id: &str) -> Option<Revision> {
    if id.is_empty()
        || id.len() > 48
        || !id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
    {
        return None;
    }
    ["webp", "png", "jpg", "jpeg"].into_iter().find_map(|ext| {
        let path = dir.join(format!("{id}.{ext}"));
        let meta = fs::symlink_metadata(&path).ok()?;
        (meta.is_file() && meta.len() <= MAX_BYTES).then(|| Revision {
            path,
            len: meta.len(),
            modified: meta.modified().ok(),
            #[cfg(unix)]
            inode: std::os::unix::fs::MetadataExt::ino(&meta),
        })
    })
}
struct Picture {
    image: Arc<RenderImage>,
    tiles: (u32, u32),
    native: (u32, u32),
    guard_texels: bool,
}
impl Picture {
    fn load(rev: &Revision) -> Option<Self> {
        let mut options = fs::OpenOptions::new();
        options.read(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
        }
        let file = options.open(&rev.path).ok()?;
        let meta = file.metadata().ok()?;
        if !meta.is_file() || meta.len() != rev.len {
            return None;
        }
        let mut bytes = Vec::with_capacity(rev.len as usize);
        file.take(MAX_BYTES + 1).read_to_end(&mut bytes).ok()?;
        if bytes.len() as u64 != rev.len {
            return None;
        }
        let format = image::guess_format(&bytes).ok()?;
        if !matches!(
            format,
            image::ImageFormat::WebP | image::ImageFormat::Jpeg | image::ImageFormat::Png
        ) {
            return None;
        }
        let mut reader = image::ImageReader::with_format(std::io::Cursor::new(bytes), format);
        let mut limits = image::Limits::default();
        limits.max_image_width = Some(16384);
        limits.max_image_height = Some(16384);
        limits.max_alloc = Some(256 * 1024 * 1024);
        reader.limits(limits);
        use image::ImageDecoder;
        let mut decoder = reader.into_decoder().ok()?;
        if decoder.total_bytes() > 256 * 1024 * 1024 {
            return None;
        }
        let orientation = decoder
            .orientation()
            .unwrap_or(image::metadata::Orientation::NoTransforms);
        let mut decoded = image::DynamicImage::from_decoder(decoder).ok()?;
        decoded.apply_orientation(orientation);
        let mut rgba = decoded.into_rgba8();
        let (w, h) = rgba.dimensions();
        if w == 0 || h == 0 {
            return None;
        }
        for pixel in rgba.pixels_mut() {
            pixel.0.swap(0, 2)
        }
        // Pack tiny repeating patterns into one texture to avoid one draw per source pixel.
        let tiles = (
            if h < 256 { 64u32.div_ceil(w) } else { 1 },
            if w < 256 { 64u32.div_ceil(h) } else { 1 },
        );
        if tiles != (1, 1) {
            rgba = image::RgbaImage::from_fn(w * tiles.0, h * tiles.1, |x, y| {
                *rgba.get_pixel(x % w, y % h)
            })
        }
        Some(Self {
            image: Arc::new(RenderImage::new(vec![image::Frame::new(rgba)])),
            tiles,
            native: (w, h),
            guard_texels: false,
        })
    }
}

impl Geometry {
    fn tile_size(&self, b: Bounds<Pixels>, native: Option<(u32, u32)>) -> (f32, f32) {
        let zoom = bruhs_ui::zoom::zoom();
        let (bw, bh) = (f32::from(b.size.width), f32::from(b.size.height));
        let Some((nw, nh)) = native else {
            return match self.sizing {
                Sizing::Explicit(x, y) => (
                    x.value(bw, zoom).unwrap_or(bw),
                    y.value(bh, zoom).unwrap_or(bh),
                ),
                _ => (bw, bh),
            };
        };
        let (nw, nh) = (nw as f32 * zoom, nh as f32 * zoom);
        match self.sizing {
            Sizing::Cover | Sizing::Contain => {
                let k = if self.sizing == Sizing::Cover {
                    (bw / nw).max(bh / nh)
                } else {
                    (bw / nw).min(bh / nh)
                };
                (nw * k, nh * k)
            }
            Sizing::Explicit(x, y) => match (x.value(bw, zoom), y.value(bh, zoom)) {
                (None, None) => (nw, nh),
                (Some(w), None) => (w, w * nh / nw),
                (None, Some(h)) => (h * nw / nh, h),
                (Some(w), Some(h)) => (w, h),
            },
        }
    }
}

impl Picture {
    fn paint(
        &self,
        b: Bounds<Pixels>,
        tile: (f32, f32),
        s: Geometry,
        radius: Pixels,
        window: &mut Window,
    ) {
        let zoom = bruhs_ui::zoom::zoom();
        let (bw, bh) = (f32::from(b.size.width), f32::from(b.size.height));
        let (w, h) = tile;
        if w <= 0. || h <= 0. {
            return;
        }
        let origin = (
            s.position.0.value(bw - w, zoom).unwrap_or_default(),
            s.position.1.value(bh - h, zoom).unwrap_or_default(),
        );
        let axis = |extent: f32, tile: f32, origin: f32, repeated: bool, count: u32| {
            let step = (tile * count as f32).max(1.);
            if repeated {
                let offset = origin.rem_euclid(step);
                (
                    if offset == 0. { 0. } else { offset - step },
                    extent,
                    0.,
                    extent,
                    step,
                )
            } else {
                (
                    origin,
                    origin + tile,
                    origin.max(0.),
                    (origin + tile).min(extent),
                    step,
                )
            }
        };
        let (left, right, clip_left, clip_right, dx) =
            axis(bw, w, origin.0, s.repeat.0, self.tiles.0);
        let (top, bottom, clip_top, clip_bottom, dy) =
            axis(bh, h, origin.1, s.repeat.1, self.tiles.1);
        if clip_left >= clip_right || clip_top >= clip_bottom {
            return;
        }
        let at = |x, y| b.origin + point(px(x), px(y));
        let mask = ContentMask {
            bounds: Bounds::from_corners(at(clip_left, clip_top), at(clip_right, clip_bottom)),
        };
        window.with_content_mask(Some(mask), |window| {
            let mut y = top;
            while y < bottom {
                let mut x = left;
                while x < right {
                    let bounds = Bounds::new(at(x, y), size(px(dx), px(dy)));
                    let texture = if self.guard_texels {
                        let gx = dx / (self.native.0 * self.tiles.0) as f32;
                        let gy = dy / (self.native.1 * self.tiles.1) as f32;
                        Bounds::from_corners(at(x - gx, y - gy), at(x + dx + gx, y + dy + gy))
                    } else {
                        bounds
                    };
                    let _ = window.paint_image(
                        b.intersect(&bounds),
                        texture,
                        Corners::all(radius),
                        self.image.clone(),
                        0,
                        false,
                    );
                    if !s.repeat.0 {
                        break;
                    }
                    x += dx;
                }
                if !s.repeat.1 {
                    break;
                }
                y += dy;
            }
        })
    }
}

#[derive(PartialEq)]
struct GradientKey {
    gradient: Arc<Gradient>,
    tile: (f32, f32),
    scale: f32,
    zoom: f32,
}

// Shared by identical surfaces, including message bubbles. Bound both atlas
// memory and entry count when windows resize or previews change rapidly.
#[derive(Default)]
struct GradientCache(VecDeque<(GradientKey, Arc<Picture>, usize)>);
impl Global for GradientCache {}
fn gradient_picture(
    gradient: &Arc<Gradient>,
    tile: (f32, f32),
    window: &mut Window,
    cx: &mut App,
) -> Arc<Picture> {
    let key = GradientKey {
        gradient: gradient.clone(),
        tile,
        scale: window.scale_factor(),
        zoom: bruhs_ui::zoom::zoom(),
    };
    let cache = &mut cx.default_global::<GradientCache>().0;
    if let Some(i) = cache.iter().position(|(k, _, _)| k == &key) {
        let entry = cache.remove(i).unwrap();
        let result = entry.1.clone();
        cache.push_front(entry);
        return result;
    }
    let rgba = gradient.raster(tile.0, tile.1, key.zoom, key.scale);
    // Pack small CSS tiles to keep a fine repeating pattern to a few draws.
    let tiles = if rgba.width() <= 256 && rgba.height() <= 256 {
        (
            (64. / tile.0.max(1.)).ceil() as u32,
            (64. / tile.1.max(1.)).ceil() as u32,
        )
    } else {
        (1, 1)
    };
    let rgba = if tiles == (1, 1) {
        rgba
    } else {
        image::RgbaImage::from_fn(rgba.width() * tiles.0, rgba.height() * tiles.1, |x, y| {
            *rgba.get_pixel(x % rgba.width(), y % rgba.height())
        })
    };
    // GPUI samples the sprite atlas with a linear filter. Extrude each edge so
    // stretching a one-texel-wide ramp never blends in the neighboring sprite.
    let (width, height) = rgba.dimensions();
    let guarded = image::RgbaImage::from_fn(width + 2, height + 2, |x, y| {
        *rgba.get_pixel(
            x.saturating_sub(1).min(width - 1),
            y.saturating_sub(1).min(height - 1),
        )
    });
    let bytes = guarded.len();
    let picture = Arc::new(Picture {
        guard_texels: true,
        native: (width / tiles.0, height / tiles.1),
        tiles,
        image: Arc::new(RenderImage::new(vec![image::Frame::new(guarded)])),
    });
    cache.push_front((key, picture.clone(), bytes));
    let mut used = cache.iter().map(|(_, _, n)| n).sum::<usize>();
    let mut dropped = Vec::new();
    while cache.len() > 1 && (cache.len() > 64 || used > 24 * 1024 * 1024) {
        let (_, old, n) = cache.pop_back().unwrap();
        used -= n;
        dropped.push(old.image.clone());
    }
    if !dropped.is_empty() {
        cx.defer(move |cx| {
            for image in dropped {
                cx.drop_image(image, None)
            }
        });
    }
    picture
}

#[derive(Default)]
struct Active {
    id: Option<String>,
    styles: Styles,
    picture: Option<Arc<Picture>>,
    _watch: Option<Task<()>>,
}
impl Global for Active {}
pub(super) fn select(id: Option<&str>, styles: Styles, window: &mut Window, cx: &mut App) {
    let id = id.filter(|_| !styles.is_empty());
    let companion = styles
        .values()
        .any(|style| style.images.contains(&Source::Companion));
    let directory = companion.then(directory).flatten();
    let active = cx.default_global::<Active>();
    if active.id.as_deref() == id && active._watch.is_some() == directory.is_some() {
        active.styles = styles;
        return;
    }
    let old = std::mem::take(active);
    if let Some(p) = old.picture {
        cx.drop_image(p.image.clone(), Some(window));
    }
    drop(old._watch);
    let Some(id) = id else { return };
    let Some(dir) = directory else {
        cx.set_global(Active {
            id: Some(id.into()),
            styles,
            ..Default::default()
        });
        return;
    };
    let name = id.to_owned();
    let watch = cx.spawn(async move |cx| {
        let mut previous = None;
        loop {
            let dir = dir.clone();
            let name = name.clone();
            let (current, result) = cx
                .background_executor()
                .spawn(async move {
                    let rev = discover(&dir, &name);
                    let result = (rev != previous)
                        .then(|| rev.as_ref().and_then(Picture::load).map(Arc::new));
                    (rev, result)
                })
                .await;
            previous = current;
            if let Some(image) = result {
                cx.update(|cx| {
                    let old = std::mem::replace(&mut cx.global_mut::<Active>().picture, image);
                    if let Some(old) = old {
                        cx.drop_image(old.image.clone(), None)
                    }
                    cx.refresh_windows();
                });
            }
            cx.background_executor().timer(Duration::from_secs(1)).await;
        }
    });
    cx.set_global(Active {
        id: Some(id.into()),
        styles,
        picture: None,
        _watch: Some(watch),
    });
}
pub(crate) fn layer(surface: &str, cx: &App) -> Option<AnyElement> {
    layer_with_radius(surface, px(0.), cx)
}
pub(crate) fn layer_with_radius(surface: &str, radius: Pixels, cx: &App) -> Option<AnyElement> {
    let active = cx.try_global::<Active>()?;
    let style = active.styles.get(surface)?.clone();
    let image = active.picture.clone();
    Some(
        canvas(
            |_, _, _| (),
            move |b, _, w, cx| {
                if b.size.width <= px(0.) || b.size.height <= px(0.) {
                    return;
                }
                for (i, source) in style.images.iter().enumerate().rev() {
                    let geometry = style.geometry(i);
                    let native = if *source == Source::Companion {
                        image.as_ref().map(|p| p.native)
                    } else {
                        None
                    };
                    let tile = geometry.tile_size(b, native);
                    if tile.0 <= 0. || tile.1 <= 0. {
                        continue;
                    }
                    let picture = match source {
                        Source::None => continue,
                        Source::Companion => {
                            let Some(p) = image.as_ref() else { continue };
                            p.clone()
                        }
                        Source::Gradient(g) => gradient_picture(g, tile, w, cx),
                    };
                    picture.paint(b, tile, geometry, radius, w);
                }
            },
        )
        .absolute()
        .inset_0()
        .into_any_element(),
    )
}

#[cfg(test)]
mod tests {
    use super::style::{Geometry, Length, Sizing, Source};
    use super::{MAX_BYTES, Picture, discover};
    use crate::views::workspace::appearance::stylesheet;
    use bruhs_ui::ThemeMode;
    use std::{collections::BTreeMap, fs};

    #[test]
    fn companion_file_discovery_matches_reference_precedence_and_limit() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();
        assert!(discover(base, "missing").is_none());
        for ext in ["jpeg", "jpg", "png", "webp"] {
            let path = base.join(format!("tide.{ext}"));
            image::RgbImage::from_pixel(5, 7, image::Rgb([19, 78, 133]))
                .save(&path)
                .unwrap();
            let revision = discover(base, "tide").unwrap();
            assert_eq!(revision.path, path);
            assert_eq!(Picture::load(&revision).unwrap().native, (5, 7));
        }
        fs::File::create(base.join("tide.webp"))
            .unwrap()
            .set_len(MAX_BYTES + 1)
            .unwrap();
        assert_eq!(discover(base, "tide").unwrap().path, base.join("tide.png"));
        assert!(discover(base, "../tide").is_none());
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(base.join("tide.png"), base.join("link.png")).unwrap();
            assert!(discover(base, "link").is_none());
        }
        fs::write(base.join("tide.png"), b"partial write").unwrap();
        assert!(Picture::load(&discover(base, "tide").unwrap()).is_none());
        fs::remove_file(base.join("tide.png")).unwrap();
        assert_eq!(discover(base, "tide").unwrap().path, base.join("tide.jpg"));
    }

    #[test]
    fn background_shorthand_and_longhands_follow_the_active_theme_variant() {
        let css = r#"
          :root { --wall: var(--theme-background-image); }
          .dark [data-slot=sidebar] { background: var(--wall) center / cover no-repeat #012345; }
          [data-slot=sidebar] { background-image: none; --inner: var(--wall); }
          [data-slot=sidebar-inner] { background-image:var(--inner); background-repeat:repeat-x; }
          [data-chat-header] { background-image:var(--theme-background-image) !important; }
          .dark [data-chat-header] { background:#141210; background-position:bottom right; }
          [data-user-message-bubble] { background: var(--wall) left / 25% auto; background:none; }
          [data-workspace-area=conversation] { background-image:url("https://invalid.test/outside.png"); }
        "#;
        let dark = stylesheet::apply(css, ThemeMode::Dark, &mut BTreeMap::new());
        let expected = Geometry {
            sizing: Sizing::Cover,
            position: (Length::Percent(5000), Length::Percent(5000)),
            repeat: (false, false),
        };
        assert_eq!(dark["Sidebar"].geometry(0), expected);
        assert_eq!(dark["Sidebar"].color, Some([1, 35, 69, 255]));
        assert_eq!(dark["Sidebar"].images, [Source::Companion]);
        assert_eq!(dark["SidebarInner"].geometry(0).repeat, (true, false));
        assert_eq!(
            dark["Header"].geometry(0).position,
            (Length::Percent(10000), Length::Percent(10000))
        );
        assert!(!dark.contains_key("UserBubble") && !dark.contains_key("Conversation"));
        let light = stylesheet::apply(css, ThemeMode::Light, &mut BTreeMap::new());
        assert!(!light.contains_key("Sidebar"));
        assert!(light["Header"].active() && light["SidebarInner"].active());
    }
}

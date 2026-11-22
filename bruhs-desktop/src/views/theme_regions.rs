//! Geometry of rendered theme usages. Each native window owns its own inspection session.
use bruhs_ui::ActiveTheme;
use gpui::{prelude::*, *};
use std::{
    cell::RefCell,
    collections::HashMap,
    rc::{Rc, Weak},
};

type Rect = Bounds<Pixels>;
#[derive(Clone, Copy, PartialEq)]
pub(crate) struct Usage {
    pub bounds: Rect,
    pub role: &'static str,
}

#[derive(Default)]
pub(crate) struct Regions {
    items: Vec<Usage>,
    dialog: Option<(usize, Rect)>,
    collecting: bool,
}

#[derive(Default)]
struct Sessions(HashMap<WindowId, Weak<RefCell<Regions>>>);
impl Global for Sessions {}

pub(crate) type Capture = Rc<RefCell<Regions>>;

pub(crate) fn start(w: &Window, cx: &mut App) -> Capture {
    let capture = Capture::default();
    cx.default_global::<Sessions>()
        .0
        .retain(|_, session| session.strong_count() > 0);
    cx.global_mut::<Sessions>()
        .0
        .insert(w.window_handle().window_id(), Rc::downgrade(&capture));
    // An already rendered view can now contribute its usages.
    cx.refresh_windows();
    capture
}

pub(crate) fn boundary(capture: Capture, begin: bool) -> impl IntoElement {
    canvas(
        move |_, _, _| {
            let mut regions = capture.borrow_mut();
            if begin {
                regions.items.clear();
                regions.dialog = None;
            }
            regions.collecting = begin;
        },
        |_, _, _, _| {},
    )
    .absolute()
    .size_0()
}

pub(crate) trait ThemeRegion: ParentElement + Styled + Sized {
    fn theme_background(self, role: &'static str, fallback: Hsla, cx: &App) -> Self {
        self.bg(cx.theme().color_role(role).unwrap_or(fallback))
            .theme_region(role, cx)
    }
    fn theme_foreground(self, role: &'static str, fallback: Hsla, cx: &App) -> Self {
        self.text_color(cx.theme().color_role(role).unwrap_or(fallback))
            .theme_region(role, cx)
    }
    fn theme_region(self, role: &'static str, cx: &App) -> Self {
        self.region(role, false, cx)
    }
    fn theme_dialog(self, role: &'static str, cx: &App) -> Self {
        self.region(role, true, cx)
    }
    fn region(self, role: &'static str, dialog: bool, cx: &App) -> Self {
        if !cx
            .try_global::<Sessions>()
            .is_some_and(|s| s.0.values().any(|slot| slot.strong_count() > 0))
        {
            return self;
        }
        self.child(
            canvas(
                move |bounds, w, cx| {
                    let Some(session) = cx
                        .try_global::<Sessions>()
                        .and_then(|s| s.0.get(&w.window_handle().window_id()))
                        .and_then(Weak::upgrade)
                    else {
                        return;
                    };
                    let mut regions = session.borrow_mut();
                    if !regions.collecting {
                        return;
                    }
                    let bounds = bounds.intersect(&w.content_mask().bounds);
                    if bounds.is_empty() {
                        return;
                    }
                    if dialog {
                        regions.dialog = Some((regions.items.len(), bounds));
                    }
                    regions.items.push(Usage { bounds, role });
                },
                |_, _, _, _| {},
            )
            .absolute()
            .inset_0(),
        )
    }
}
impl<T: ParentElement + Styled> ThemeRegion for T {}

pub(crate) fn outside(r: Rect, cover: Rect) -> Vec<Rect> {
    if !r.intersects(&cover) {
        return vec![r];
    }
    let c = r.intersect(&cover);
    [
        (r.left(), r.top(), r.right(), c.top()),
        (r.left(), c.bottom(), r.right(), r.bottom()),
        (r.left(), c.top(), c.left(), c.bottom()),
        (c.right(), c.top(), r.right(), c.bottom()),
    ]
    .into_iter()
    .filter(|(l, t, r, b)| l < r && t < b)
    .map(|(l, t, r, b)| Bounds::from_corners(point(l, t), point(r, b)))
    .collect()
}
impl Regions {
    pub fn pick(&self, pos: Point<Pixels>) -> Option<Usage> {
        let start = self
            .dialog
            .filter(|(_, b)| b.contains(&pos))
            .map_or(0, |(i, _)| i);
        self.items[start..]
            .iter()
            .rev()
            .find(|u| u.bounds.contains(&pos))
            .copied()
    }
    pub fn matches(&self, roles: &[&str]) -> (usize, Vec<Rect>) {
        let mut boxes = Vec::new();
        let mut count = 0;
        for (i, u) in self
            .items
            .iter()
            .enumerate()
            .filter(|(_, u)| roles.contains(&u.role))
        {
            let visible = match self.dialog {
                Some((n, b)) if i < n => outside(u.bounds, b),
                _ => vec![u.bounds],
            };
            if !visible.is_empty() {
                count += 1;
                boxes.extend(visible);
            }
        }
        (count, boxes)
    }
}

//! Backdrop blur for modal scrims — the web's `backdrop-blur-sm`.
//!
//! gpui has no backdrop filter (its only blur is `Shadow::blur_radius`), so
//! the blur comes from AppKit: a native child view of the gpui window — the
//! same arrangement the preview panel uses for its `WKWebView` — whose layer
//! carries a `CIGaussianBlur` in `backgroundFilters`. Core Animation's
//! background filters are exactly what CSS calls `backdrop-filter`.
//!
//! `NSVisualEffectView` is the obvious candidate and the wrong one: every
//! material washes the scrim towards grey. Measured against the web's
//! `bg-longan-950/70`, rgb(23, 20, 18), the least tinted material still
//! lands at rgb(33, 34, 31). A bare Gaussian adds no colour of its own.
//!
//! Two consequences shape the design:
//!
//! * Native subviews sit **above** gpui's Metal layer, so a pane covering the
//!   whole scrim would blur the dialog too. The scrim is therefore four panes
//!   laid out in the ring *around* the dialog, leaving the dialog crisp.
//!   Squaring off the dialog's rounded corners that way leaves four slivers
//!   of backdrop unblurred, which nothing distinguishes from the blurred
//!   backdrop beside them: at 70% dimming there is no detail left to sharpen.
//! * The scrim is click-to-dismiss, and a plain `NSView` would swallow those
//!   clicks. The panes are a subclass whose `hitTest:` returns null, so the
//!   mouse passes through to gpui.
//!
//! Callers see [`BlurScrimHandle`]: hide it when no blurred dialog is open, then
//! hang [`BlurScrimHandle::backdrop`] inside each dialog. The backdrop is a
//! `canvas` that reports the dialog's laid-out bounds during prepaint, the
//! only point at which its height — driven by its content — is known.

use gpui::{canvas, Bounds, IntoElement, Pixels, Styled, Window};
use std::cell::RefCell;
use std::rc::Rc;

/// Matches the web scrim's `backdrop-blur-sm`, a 4px Gaussian.
const BACKDROP_BLUR_RADIUS: f64 = 4.0;

/// Every dialog has a 1px border, and gpui lays an absolutely positioned
/// child out inside it, so the hole the backdrop leaves has to be that much
/// wider on each side or the blur eats the border.
const DIALOG_BORDER_PX: f32 = 1.0;

/// A rectangle in gpui's coordinate space (origin top-left).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl Rect {
    pub fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    fn from_bounds(bounds: Bounds<Pixels>) -> Self {
        Self::new(
            f32::from(bounds.origin.x),
            f32::from(bounds.origin.y),
            f32::from(bounds.size.width),
            f32::from(bounds.size.height),
        )
    }

    /// The same rectangle, grown by `by` on every side.
    fn inflate(self, by: f32) -> Self {
        Self::new(
            self.x - by,
            self.y - by,
            self.width + by * 2.0,
            self.height + by * 2.0,
        )
    }

    fn right(self) -> f32 {
        self.x + self.width
    }

    fn bottom(self) -> f32 {
        self.y + self.height
    }
}

/// The panes that blur `overlay` without touching `dialog` or its border.
fn panes(overlay: Rect, dialog: Rect) -> [Rect; 4] {
    ring(overlay, dialog.inflate(DIALOG_BORDER_PX))
}

/// `overlay` minus `dialog`, as four rectangles: the strips above and below
/// the dialog run the overlay's full width; the side strips fill what is left
/// beside it. A dialog reaching past an edge simply shrinks the strips there.
fn ring(overlay: Rect, dialog: Rect) -> [Rect; 4] {
    let clamp_y = |v: f32| v.clamp(overlay.y, overlay.bottom());
    let clamp_x = |v: f32| v.clamp(overlay.x, overlay.right());
    let top = clamp_y(dialog.y);
    let bottom = clamp_y(dialog.bottom());
    let left = clamp_x(dialog.x);
    let right = clamp_x(dialog.right());
    [
        // Above the dialog.
        Rect::new(overlay.x, overlay.y, overlay.width, top - overlay.y),
        // Below it.
        Rect::new(overlay.x, bottom, overlay.width, overlay.bottom() - bottom),
        // Left of it.
        Rect::new(overlay.x, top, left - overlay.x, bottom - top),
        // Right of it.
        Rect::new(right, top, overlay.right() - right, bottom - top),
    ]
}

/// The blur panes for one window, created the first time a dialog opens.
///
/// Cloning shares the panes, so the handle can be stored once on the view and
/// captured by each frame's probe closure.
#[derive(Clone, Default)]
pub struct BlurScrimHandle {
    state: Rc<RefCell<State>>,
}

/// A window that cannot blur says so once, not once per frame.
#[derive(Default)]
enum State {
    #[default]
    Unbuilt,
    Ready(platform::BlurScrim),
    Unavailable,
}

impl BlurScrimHandle {
    /// Blur the window everywhere except `dialog`.
    fn show(&self, window: &mut Window, dialog: Bounds<Pixels>) {
        let mut state = self.state.borrow_mut();
        if matches!(*state, State::Unbuilt) {
            *state = match platform::BlurScrim::new(window) {
                Ok(scrim) => State::Ready(scrim),
                Err(err) => {
                    tracing::warn!("no backdrop blur: {err}");
                    State::Unavailable
                }
            };
        }
        let State::Ready(scrim) = &mut *state else {
            return;
        };
        let viewport = window.viewport_size();
        let overlay = Rect::new(
            0.0,
            0.0,
            f32::from(viewport.width),
            f32::from(viewport.height),
        );
        if let Err(err) = scrim.show(panes(overlay, Rect::from_bounds(dialog)), overlay.height) {
            tracing::warn!("no backdrop blur: {err}");
            *state = State::Unavailable;
        }
    }

    /// Stop blurring. Cheap to call on every frame that has no dialog.
    pub fn hide(&self) {
        if let State::Ready(scrim) = &mut *self.state.borrow_mut() {
            scrim.hide();
        }
    }

    /// An element that draws nothing and blurs the window behind everything
    /// except itself. Add it as a child of a dialog and it fills that dialog.
    pub fn backdrop(&self) -> impl IntoElement {
        let handle = self.clone();
        canvas(
            move |bounds, window, _cx| handle.show(window, bounds),
            |_bounds, _, _, _| {},
        )
        .absolute()
        .top_0()
        .left_0()
        .size_full()
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use super::{Rect, BACKDROP_BLUR_RADIUS};
    use objc2::rc::Retained;
    use objc2::runtime::{AnyClass, AnyObject};
    use objc2::{define_class, msg_send, MainThreadMarker, MainThreadOnly};
    use objc2_app_kit::NSView;
    use objc2_foundation::{NSArray, NSNumber, NSPoint, NSRect, NSSize, NSString};
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};

    // AppKit pulls in Core Image, but nothing here references a symbol from
    // it, so ask for the framework explicitly and keep `CIFilter` loadable.
    // SAFETY: the block declares no items; it only links the framework.
    #[link(name = "CoreImage", kind = "framework")]
    unsafe extern "C" {}

    define_class!(
        /// A pane the mouse falls straight through, so the scrim underneath
        /// keeps its click-to-dismiss behaviour.
        #[unsafe(super(NSView))]
        #[thread_kind = MainThreadOnly]
        #[name = "BruhsBlurPane"]
        struct BlurPaneView;

        impl BlurPaneView {
            #[unsafe(method(hitTest:))]
            fn hit_test(&self, _point: NSPoint) -> *mut NSView {
                std::ptr::null_mut()
            }
        }
    );

    /// The Core Image filters the backdrop chain uses, each with the one
    /// input it sets. Keys come only from here, so key-value coding can never
    /// be handed a key the filter does not have (which would raise an
    /// Objective-C exception across the FFI boundary).
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum FilterKind {
        Crop,
        AffineClamp,
        GaussianBlur,
    }

    impl FilterKind {
        fn name(self) -> &'static str {
            match self {
                Self::Crop => "CICrop",
                Self::AffineClamp => "CIAffineClamp",
                Self::GaussianBlur => "CIGaussianBlur",
            }
        }

        /// The documented input this chain configures.
        fn input_key(self) -> &'static str {
            match self {
                Self::Crop => "inputRectangle",
                Self::AffineClamp => "inputTransform",
                Self::GaussianBlur => "inputRadius",
            }
        }
    }

    /// A `CIFilter` of a known kind. Its input can only be set through the
    /// typed constructors below, with the value type that input takes.
    struct CiFilter {
        kind: FilterKind,
        object: Retained<AnyObject>,
    }

    impl CiFilter {
        /// The filter with its default inputs.
        fn new(kind: FilterKind) -> Result<Self, String> {
            let class = AnyClass::get(c"CIFilter").ok_or("Core Image is not loaded")?;
            let name = NSString::from_str(kind.name());
            // SAFETY: `+[CIFilter filterWithName:]` takes an NSString and
            // returns an autoreleased filter, or nil for a name Core Image
            // does not know.
            let object: Option<Retained<AnyObject>> =
                unsafe { msg_send![class, filterWithName: &*name] };
            let object = object.ok_or_else(|| format!("no {} filter", kind.name()))?;
            Ok(Self { kind, object })
        }

        /// `CICrop` cutting its input to `frame`.
        fn crop(frame: NSRect) -> Result<Self, String> {
            let filter = Self::new(FilterKind::Crop)?;
            let vector_class = AnyClass::get(c"CIVector").ok_or("no CIVector")?;
            // SAFETY: `+[CIVector vectorWithX:Y:Z:W:]` takes four CGFloats;
            // for `inputRectangle` they are x, y, width and height.
            let rect: Option<Retained<AnyObject>> = unsafe {
                msg_send![
                    vector_class,
                    vectorWithX: frame.origin.x,
                    Y: frame.origin.y,
                    Z: frame.size.width,
                    W: frame.size.height,
                ]
            };
            filter.set_input(&*rect.ok_or("no crop rectangle")?);
            Ok(filter)
        }

        /// `CIAffineClamp` with an identity transform.
        fn affine_clamp() -> Result<Self, String> {
            let filter = Self::new(FilterKind::AffineClamp)?;
            // The documented default for `inputTransform` is the identity,
            // but what Core Image actually hands back collapses the backdrop
            // to a speck, so set it.
            let transform_class =
                AnyClass::get(c"NSAffineTransform").ok_or("no NSAffineTransform")?;
            // SAFETY: `+[NSAffineTransform transform]` returns a new identity
            // transform, autoreleased.
            let identity: Option<Retained<AnyObject>> =
                unsafe { msg_send![transform_class, transform] };
            filter.set_input(&*identity.ok_or("no identity transform")?);
            Ok(filter)
        }

        /// `CIGaussianBlur` of `radius` points.
        fn gaussian_blur(radius: f64) -> Result<Self, String> {
            let filter = Self::new(FilterKind::GaussianBlur)?;
            filter.set_input(&NSNumber::new_f64(radius));
            Ok(filter)
        }

        /// Private: only the constructors above call it, each with the value
        /// type its kind's input takes.
        fn set_input(&self, value: &AnyObject) {
            let key = NSString::from_str(self.kind.input_key());
            // SAFETY: `-[CIFilter setValue:forKey:]` with `key`, a documented
            // input of this filter kind (see `FilterKind::input_key`), and a
            // value of the type that input takes (see the constructors).
            unsafe { msg_send![&*self.object, setValue: value, forKey: &*key] }
        }
    }

    /// The backdrop filter chain for a pane occupying `frame` of the window.
    ///
    /// Core Image hands the pane the backdrop of the whole window, in the
    /// window's own coordinates, rather than the slice behind the pane — so a
    /// bare blur drags the dialog's lit surface out across the scrim as a
    /// glow. `CICrop` cuts the input back to the pane. `CIAffineClamp` then
    /// stretches the pane's own edge pixels out forever, which is what stops
    /// the blur from sampling the transparency beyond the crop and darkening
    /// every edge. Only then does it blur.
    fn backdrop_filters(frame: NSRect) -> Result<Retained<NSArray<AnyObject>>, String> {
        let crop = CiFilter::crop(frame)?;
        let clamp = CiFilter::affine_clamp()?;
        let blur = CiFilter::gaussian_blur(BACKDROP_BLUR_RADIUS)?;
        Ok(NSArray::from_retained_slice(&[
            crop.object,
            clamp.object,
            blur.object,
        ]))
    }

    /// Run `changes` in a Core Animation transaction and push it out.
    ///
    /// gpui draws with Metal and never commits an AppKit layer tree of its
    /// own, so a pane's new frame and filters would otherwise sit unapplied
    /// until something else — the user moving the mouse, say — happened to
    /// drive a display cycle, and a dialog would open unblurred. Actions are
    /// off because these panes should appear with the dialog, not fade in.
    fn transaction<T>(changes: impl FnOnce() -> T) -> T {
        let class = AnyClass::get(c"CATransaction");
        if let Some(class) = class {
            // SAFETY: `+[CATransaction begin]` and `setDisableActions:` (a
            // BOOL) open Core Animation's implicit transaction; the blur is
            // only ever built and driven on the main thread.
            unsafe {
                let _: () = msg_send![class, begin];
                let _: () = msg_send![class, setDisableActions: true];
            }
        }
        let result = changes();
        if let Some(class) = class {
            // SAFETY: `+[CATransaction commit]` closes the transaction opened
            // above and `+flush` pushes it to the render server; main thread.
            unsafe {
                let _: () = msg_send![class, commit];
                let _: () = msg_send![class, flush];
            }
        }
        result
    }

    /// One blur pane parented to the gpui window's content view.
    struct BlurPane {
        view: Retained<BlurPaneView>,
        layer: Retained<AnyObject>,
        /// What the pane was last placed at, to skip rebuilding its filters
        /// on the frames that did not move it. The window's height is part of
        /// it because that is what the flip to AppKit's coordinates uses: a
        /// window resized only vertically leaves the gpui rectangle alone and
        /// still moves the pane.
        placed: Option<(Rect, f32)>,
    }

    impl BlurPane {
        fn new(parent: &NSView, mtm: MainThreadMarker) -> Result<Self, String> {
            // SAFETY: `-[NSView init]` on a freshly allocated `BlurPaneView`
            // (an NSView subclass with no extra ivars), on the main thread
            // as `mtm` proves.
            let view: Retained<BlurPaneView> = unsafe { msg_send![BlurPaneView::alloc(mtm), init] };
            view.setWantsLayer(true);
            // SAFETY: the view is layer-backed as of the line above, so it
            // has a layer.
            let layer: Option<Retained<AnyObject>> = unsafe { msg_send![&*view, layer] };
            let layer = layer.ok_or("layer-backed view without a layer")?;
            // Without this the blur bleeds past the pane's edges.
            // SAFETY: `masksToBounds` is a `BOOL` property of every CALayer.
            unsafe { msg_send![&*layer, setMasksToBounds: true] }
            view.setHidden(true);
            parent.addSubview(&view);
            Ok(Self {
                view,
                layer,
                placed: None,
            })
        }

        /// `frame` is in gpui's coordinate space (origin top-left); AppKit's
        /// content view is bottom-left, so y is flipped against its height.
        fn show(&mut self, frame: Rect, parent_height: f32) -> Result<(), String> {
            if self.placed != Some((frame, parent_height)) {
                let in_window = NSRect::new(
                    NSPoint::new(
                        f64::from(frame.x),
                        f64::from(parent_height - frame.y - frame.height),
                    ),
                    NSSize::new(
                        f64::from(frame.width.max(0.0)),
                        f64::from(frame.height.max(0.0)),
                    ),
                );
                self.view.setFrame(in_window);
                // Core Animation copies the filters as they are assigned, so
                // a moved or resized pane needs a chain cropped to where it
                // has landed.
                let filters = backdrop_filters(in_window)?;
                // SAFETY: `backgroundFilters` is Core Animation's backdrop
                // filter — the same thing CSS calls `backdrop-filter` — and
                // takes an array of CIFilters.
                unsafe { msg_send![&*self.layer, setBackgroundFilters: &*filters] }
                self.placed = Some((frame, parent_height));
            }
            // Hiding an NSView alone can leave its backdrop filter composited
            // over Metal content after the dialog closes.
            // SAFETY: `hidden` is a BOOL property of every CALayer; main thread.
            unsafe { msg_send![&*self.layer, setHidden: false] }
            self.view.setHidden(false);
            Ok(())
        }

        fn hide(&mut self) {
            self.view.setHidden(true);
            // SAFETY: CALayer hidden/backgroundFilters are BOOL/NSArray
            // properties; clearing the filter removes the compositor effect.
            unsafe {
                let _: () = msg_send![&*self.layer, setHidden: true];
                let _: () = msg_send![&*self.layer, setBackgroundFilters: Option::<&NSArray<AnyObject>>::None];
            }
            self.placed = None;
        }
    }

    impl Drop for BlurPane {
        fn drop(&mut self) {
            self.view.removeFromSuperview();
        }
    }

    /// The four panes forming the ring around a modal dialog.
    pub(super) struct BlurScrim {
        panes: [BlurPane; 4],
        shown: bool,
    }

    impl BlurScrim {
        pub(super) fn new(window: &gpui::Window) -> Result<Self, String> {
            let mtm =
                MainThreadMarker::new().ok_or("blur scrim must be built on the main thread")?;
            // `gpui::Window` has its own inherent `window_handle()`, so the
            // raw-window-handle trait method needs naming explicitly.
            let handle = HasWindowHandle::window_handle(window)
                .map_err(|e| format!("window handle: {e}"))?;
            let RawWindowHandle::AppKit(appkit) = handle.as_raw() else {
                return Err("not an AppKit window".to_string());
            };
            // SAFETY: gpui hands out a live NSView pointer for the window it
            // owns; `retain` takes our own reference to it.
            let parent: Retained<NSView> =
                unsafe { Retained::retain(appkit.ns_view.as_ptr().cast()) }
                    .ok_or("null NSView from gpui")?;
            let panes = [
                BlurPane::new(&parent, mtm)?,
                BlurPane::new(&parent, mtm)?,
                BlurPane::new(&parent, mtm)?,
                BlurPane::new(&parent, mtm)?,
            ];
            Ok(Self {
                panes,
                shown: false,
            })
        }

        pub(super) fn show(&mut self, ring: [Rect; 4], parent_height: f32) -> Result<(), String> {
            let panes = &mut self.panes;
            transaction(|| -> Result<(), String> {
                for (pane, frame) in panes.iter_mut().zip(ring) {
                    pane.show(frame, parent_height)?;
                }
                Ok(())
            })?;
            self.shown = true;
            Ok(())
        }

        pub(super) fn hide(&mut self) {
            if self.shown {
                transaction(|| {
                    for pane in &mut self.panes {
                        pane.hide();
                    }
                });
                self.shown = false;
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::{backdrop_filters, FilterKind};
        use objc2_foundation::{NSPoint, NSRect, NSSize};

        #[test]
        fn each_filter_sets_only_its_documented_input() {
            assert_eq!(FilterKind::Crop.input_key(), "inputRectangle");
            assert_eq!(FilterKind::AffineClamp.input_key(), "inputTransform");
            assert_eq!(FilterKind::GaussianBlur.input_key(), "inputRadius");
        }

        #[test]
        fn the_backdrop_chain_builds_with_core_image() {
            let frame = NSRect::new(NSPoint::new(10.0, 20.0), NSSize::new(300.0, 200.0));
            let filters = backdrop_filters(frame).expect("Core Image filters");
            assert_eq!(filters.count(), 3);
        }
    }
}

/// Elsewhere the dialogs simply have no backdrop blur.
#[cfg(not(target_os = "macos"))]
mod platform {
    use super::Rect;

    pub(super) struct BlurScrim;

    impl BlurScrim {
        pub(super) fn new(_window: &gpui::Window) -> Result<Self, String> {
            Err("backdrop blur is implemented for macOS only".to_string())
        }

        pub(super) fn show(&mut self, _ring: [Rect; 4], _parent_height: f32) -> Result<(), String> {
            Ok(())
        }

        pub(super) fn hide(&mut self) {}
    }
}

#[cfg(test)]
mod tests {
    use super::{panes, ring, Rect, DIALOG_BORDER_PX};

    #[test]
    fn the_ring_tiles_the_overlay_around_the_dialog() {
        let overlay = Rect::new(0.0, 0.0, 1000.0, 800.0);
        let dialog = Rect::new(300.0, 100.0, 400.0, 200.0);
        let [top, bottom, left, right] = ring(overlay, dialog);
        assert_eq!(top, Rect::new(0.0, 0.0, 1000.0, 100.0));
        assert_eq!(bottom, Rect::new(0.0, 300.0, 1000.0, 500.0));
        assert_eq!(left, Rect::new(0.0, 100.0, 300.0, 200.0));
        assert_eq!(right, Rect::new(700.0, 100.0, 300.0, 200.0));
        // No pane covers the dialog, and together they cover everything else.
        let covered: f32 = [top, bottom, left, right]
            .iter()
            .map(|r| r.width * r.height)
            .sum();
        assert_eq!(covered + dialog.width * dialog.height, 1000.0 * 800.0);
    }

    #[test]
    fn a_dialog_overflowing_the_overlay_shrinks_the_strips_beside_it() {
        let overlay = Rect::new(0.0, 0.0, 500.0, 400.0);
        // Taller than the overlay and hanging off the right edge.
        let dialog = Rect::new(400.0, -50.0, 300.0, 600.0);
        let panes = ring(overlay, dialog);
        for r in panes {
            assert!(r.width >= 0.0 && r.height >= 0.0, "negative pane {r:?}");
            assert!(
                r.x >= overlay.x && r.y >= overlay.y,
                "pane escapes the overlay: {r:?}"
            );
            assert!(
                r.right() <= overlay.right() && r.bottom() <= overlay.bottom(),
                "pane escapes the overlay: {r:?}"
            );
        }
        // Only the strip left of the dialog survives.
        assert_eq!(panes[2], Rect::new(0.0, 0.0, 400.0, 400.0));
    }

    /// gpui measures the dialog inside its border, so blurring right up to
    /// what it reports would smear the border away.
    #[test]
    fn the_panes_clear_the_dialog_border_too() {
        let overlay = Rect::new(0.0, 0.0, 1000.0, 800.0);
        let dialog = Rect::new(300.0, 100.0, 400.0, 200.0);
        let [top, bottom, left, right] = panes(overlay, dialog);
        assert_eq!(top.bottom(), dialog.y - DIALOG_BORDER_PX);
        assert_eq!(bottom.y, dialog.bottom() + DIALOG_BORDER_PX);
        assert_eq!(left.right(), dialog.x - DIALOG_BORDER_PX);
        assert_eq!(right.x, dialog.right() + DIALOG_BORDER_PX);
    }
}

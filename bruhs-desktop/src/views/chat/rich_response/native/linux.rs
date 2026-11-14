//! Native WebKitGTK child windows in GPUI's X11/XWayland window.

use super::event_waiter::{EventWaiter, PollSpec};
use gpui::{App, Task, Window};
#[cfg(debug_assertions)]
use gpui::{Bounds, Pixels};
use gtk::glib::{self, translate::ToGlibPtr as _};
use gtk::prelude::*;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use std::cell::RefCell;
use std::rc::{Rc, Weak};
use wry::{WebViewBuilderExtUnix as _, WebViewExtUnix as _};

thread_local! {
    // Only native hosts own the dispatcher. The last host closes its poll
    // socket and cancels the GPUI task; a later host can start a new dispatcher.
    static RUNTIME: RefCell<Weak<GtkRuntime>> = const { RefCell::new(Weak::new()) };
}

struct GtkRuntime {
    waiter: EventWaiter,
    task: RefCell<Option<Task<()>>>,
    error: RefCell<Option<String>>,
}

pub(super) struct GtkRuntimeLease(Rc<GtkRuntime>);

impl GtkRuntimeLease {
    pub(super) fn acquire(app: &App) -> Result<Self, String> {
        if let Some(reason) = crate::webview_platform::inline_unavailable_reason() {
            return Err(reason.into());
        }
        if gtk::is_initialized() && !gtk::is_initialized_main_thread() {
            return Err(
                "GTK was initialized on another thread; restart bruhs to open native previews."
                    .into(),
            );
        }
        gtk::init().map_err(|error| format!(
            "WebKitGTK could not open the X11/XWayland display: {error}. Install WebKitGTK 4.1 and enable XWayland in Wayland sessions, then restart bruhs."
        ))?;
        let display = gtk::gdk::Display::default().ok_or("GTK has no display")?;
        if display.type_().name() != "GdkX11Display" {
            return Err("GTK selected a Wayland display. Relaunch bruhs with GDK_BACKEND=x11 and DISPLAY available for inline previews.".into());
        }
        RUNTIME.with(|runtime| {
            if let Some(runtime) = runtime.borrow().upgrade() {
                let lease = Self(runtime);
                lease.check()?;
                return Ok(lease);
            }
            let state = Rc::new(GtkRuntime {
                waiter: EventWaiter::new()?,
                task: RefCell::new(None),
                error: RefCell::new(None),
            });
            let weak = Rc::downgrade(&state);
            *state.task.borrow_mut() = Some(app.spawn(async move |app| loop {
                let Some(state) = weak.upgrade() else { break };
                let ready = dispatch_and_query().and_then(|poll| state.waiter.wait(poll));
                drop(state);
                let result = match ready {
                    Ok(ready) => ready
                        .recv()
                        .await
                        .map_err(|error| error.to_string())
                        .and_then(|result| result),
                    Err(error) => Err(error),
                };
                if let Err(error) = result {
                    if let Some(state) = weak.upgrade() {
                        *state.error.borrow_mut() =
                            Some(format!("The native preview dispatcher stopped: {error}"));
                        app.update(App::refresh_windows);
                    }
                    break;
                }
            }));
            *runtime.borrow_mut() = Rc::downgrade(&state);
            Ok(Self(state))
        })
    }

    pub(super) fn check(&self) -> Result<(), String> {
        self.0
            .error
            .borrow()
            .as_ref()
            .map_or(Ok(()), |error| Err(error.clone()))
    }
}

/// GTK dispatch remains on GPUI's UI thread. The worker waits only for GLib's
/// descriptors/deadline, including its wakeup FD for newly added sources.
fn dispatch_and_query() -> Result<PollSpec, String> {
    let context = glib::MainContext::default();
    let _owner = context.acquire().map_err(|error| error.to_string())?;
    // One GLib iteration dispatches the current ready batch. Remaining work
    // gives a zero deadline and yields through the worker before the next batch.
    context.iteration(false);
    let (ready, priority) = context.prepare();
    let mut timeout_ms = -1;
    let mut descriptors = Vec::<glib::ffi::GPollFD>::new();
    loop {
        let capacity = i32::try_from(descriptors.len()).map_err(|error| error.to_string())?;
        // SAFETY: context is acquired on this GTK main thread. GLib writes at
        // most capacity initialized records and reports the required count.
        let required = unsafe {
            glib::ffi::g_main_context_query(
                context.to_glib_none().0,
                priority,
                &mut timeout_ms,
                descriptors.as_mut_ptr(),
                capacity,
            )
        };
        let required = usize::try_from(required).map_err(|error| error.to_string())?;
        if required <= descriptors.len() {
            descriptors.truncate(required);
            break;
        }
        descriptors
            .try_reserve_exact(required - descriptors.len())
            .map_err(|error| error.to_string())?;
        descriptors.resize(
            required,
            glib::ffi::GPollFD {
                fd: -1,
                events: 0,
                revents: 0,
            },
        );
    }
    let descriptors = descriptors
        .into_iter()
        .map(|descriptor| {
            Ok(libc::pollfd {
                fd: descriptor.fd,
                events: i16::try_from(descriptor.events).map_err(|error| error.to_string())?,
                revents: 0,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    Ok(PollSpec {
        descriptors,
        timeout_ms: if ready { 0 } else { timeout_ms },
    })
}

/// GTK owns a genuine toplevel GDK window and its frame clock. Only its X11
/// server-side parent is changed; substituting a foreign GDK window (Wry's raw
/// X11 builder) loses GTK's clock and normal child allocation on resize.
pub(super) struct GtkChild {
    window: gtk::Window,
    fixed: gtk::Fixed,
    display: gdkx11::X11Display,
    xlib: x11_dl::xlib::Xlib,
    xid: libc::c_ulong,
}

fn parent_xid(window: &Window) -> Result<libc::c_ulong, String> {
    match HasWindowHandle::window_handle(window).map_err(|error| error.to_string())?.as_raw() {
        RawWindowHandle::Xlib(handle) => Ok(handle.window),
        RawWindowHandle::Xcb(handle) => Ok(handle.window.get().into()),
        RawWindowHandle::Wayland(_) => Err("Inline WebKitGTK previews require an X11/XWayland parent. Enable XWayland and relaunch bruhs with DISPLAY available.".into()),
        _ => Err("This Linux window has no X11/XWayland parent for an inline preview.".into()),
    }
}

impl GtkChild {
    fn new(parent: &Window) -> Result<Self, String> {
        let parent = parent_xid(parent)?;
        let display = gtk::gdk::Display::default()
            .ok_or("GTK has no display")?
            .downcast::<gdkx11::X11Display>()
            .map_err(|_| "GTK has no X11 display")?;
        let window = gtk::Window::new(gtk::WindowType::Toplevel);
        window.set_decorated(false);
        window.set_skip_taskbar_hint(true);
        window.set_skip_pager_hint(true);
        window.set_focus_on_map(false);
        let fixed = gtk::Fixed::new();
        fixed.show();
        window.add(&fixed);
        window.realize();
        let native = window
            .window()
            .ok_or("GTK did not create its native preview window")?;
        native.set_override_redirect(true);
        let xid = native
            .downcast::<gdkx11::X11Window>()
            .map_err(|_| "GTK preview window is not X11")?
            .xid();
        let child = Self {
            window,
            fixed,
            display,
            xlib: x11_dl::xlib::Xlib::open().map_err(|error| error.to_string())?,
            xid,
        };
        child.display.error_trap_push();
        // SAFETY: GTK owns this live X11 display/window on the UI thread. The
        // GPUI parent is borrowed for construction and its close observer drops
        // this owned child before parent destruction. No foreign GDK window is
        // installed and GTK retains its own native frame clock.
        unsafe {
            let display =
                gdkx11::ffi::gdk_x11_display_get_xdisplay(child.display.to_glib_none().0).cast();
            (child.xlib.XReparentWindow)(display, child.xid, parent, 0, 0);
        }
        child.display.upcast_ref::<gtk::gdk::Display>().sync();
        let error = child.display.error_trap_pop();
        if error != 0 {
            return Err(format!(
                "Could not embed the GTK preview window (X11 error {error})"
            ));
        }
        Ok(child)
    }

    pub(super) fn resize(
        &self,
        view: &wry::WebView,
        bounds: wry::Rect,
        parent_scale: f32,
    ) -> Result<(), String> {
        let widget = view.webview();
        let gtk_scale = f64::from(widget.scale_factor().max(1));
        let size = bounds.size.to_logical::<i32>(gtk_scale);
        let width = size.width.max(1);
        let height = size.height.max(1);
        self.fixed.set_size_request(width, height);
        widget.set_size_request(width, height);
        self.window.resize(width, height);
        let position = bounds.position.to_logical::<i32>(gtk_scale);
        self.window.move_(position.x, position.y);
        view.set_bounds(wry::Rect {
            position: wry::dpi::LogicalPosition::new(0, 0).into(),
            size: wry::dpi::LogicalSize::new(width, height).into(),
        })
        .map_err(|error| error.to_string())?;
        self.fixed
            .size_allocate(&gtk::Allocation::new(0, 0, width, height));
        self.fixed.queue_resize();
        widget.queue_resize();
        let position = bounds.position.to_physical::<i32>(gtk_scale);
        let size = bounds.size.to_physical::<u32>(gtk_scale);
        // SAFETY: this owned child XID/display remain live until GtkChild::drop.
        // Physical parent-relative coordinates are authoritative; GTK's cached
        // toplevel position would otherwise include a window-manager frame.
        unsafe {
            let display =
                gdkx11::ffi::gdk_x11_display_get_xdisplay(self.display.to_glib_none().0).cast();
            (self.xlib.XMoveResizeWindow)(
                display,
                self.xid,
                position.x,
                position.y,
                size.width.max(1),
                size.height.max(1),
            );
        }
        self.display.upcast_ref::<gtk::gdk::Display>().flush();
        view.zoom(f64::from(parent_scale) / gtk_scale)
            .map_err(|error| error.to_string())
    }

    pub(super) fn show(&self) {
        self.window.show();
    }
    pub(super) fn hide(&self) {
        self.window.hide();
    }

    #[cfg(debug_assertions)]
    pub(super) fn smoke_report(
        &self,
        view: &wry::WebView,
        bounds: Bounds<Pixels>,
        parent_scale: f32,
    ) {
        let widget = view.webview();
        let scale = f64::from(widget.scale_factor().max(1));
        let intended = wry::dpi::LogicalSize::new(
            f64::from(f32::from(bounds.size.width)),
            f64::from(f32::from(bounds.size.height)),
        )
        .to_physical::<u32>(f64::from(parent_scale))
        .to_logical::<i32>(scale);
        let (requested_width, requested_height) = widget.size_request();
        eprintln!(
            "BRUHS_RICH_RESPONSE_SMOKE gtk_allocation={}x{} intended={}x{} requested={}x{} frame_clock={}",
            widget.allocated_width(),
            widget.allocated_height(),
            intended.width,
            intended.height,
            requested_width,
            requested_height,
            widget.frame_clock().is_some()
        );
    }
}

impl Drop for GtkChild {
    fn drop(&mut self) {
        self.window.hide();
        // SAFETY: only this owner destroys the GTK host, after the Wry reference
        // has been dropped. No widget state is queried after destruction.
        unsafe {
            self.window.destroy();
        }
        // Fence the GDK/Xlib connection before GPUI destroys its XCB parent.
        self.display.upcast_ref::<gtk::gdk::Display>().sync();
    }
}

pub(super) fn build_child(
    builder: wry::WebViewBuilder<'_>,
    window: &Window,
) -> Result<(wry::WebView, GtkChild), String> {
    let child = GtkChild::new(window)?;
    let view = builder
        .with_bounds(wry::Rect {
            position: wry::dpi::LogicalPosition::new(0, 0).into(),
            size: wry::dpi::LogicalSize::new(1, 1).into(),
        })
        .build_gtk(&child.fixed)
        .map_err(|error| format!("WebKitGTK could not create its inline child view: {error}"))?;
    Ok((view, child))
}

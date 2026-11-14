//! Select the Linux parent-window backend before any worker threads start.

#[cfg(any(target_os = "linux", test))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LinuxBackend {
    X11,
    WaylandWithoutX11,
    Headless,
}

#[cfg(any(target_os = "linux", test))]
fn linux_backend(display: bool, wayland: bool, headless: bool) -> LinuxBackend {
    if headless {
        LinuxBackend::Headless
    } else if display {
        LinuxBackend::X11
    } else if wayland {
        LinuxBackend::WaylandWithoutX11
    } else {
        LinuxBackend::Headless
    }
}

/// Called once by `main`, before logging, executors, or platform initialization.
pub(crate) fn configure_startup() {
    #[cfg(target_os = "linux")]
    {
        let backend = linux_backend(
            nonempty_env("DISPLAY"),
            nonempty_env("WAYLAND_DISPLAY"),
            std::env::var_os("ZED_HEADLESS").is_some(),
        );
        if backend == LinuxBackend::X11 {
            // SAFETY: main calls this before starting any worker threads. GPUI
            // chooses Wayland solely from WAYLAND_DISPLAY; WebKitGTK cannot
            // attach a GTK child to GPUI's separately owned Wayland surface.
            // XWayland provides an actual child window inside the GPUI window.
            unsafe {
                std::env::remove_var("WAYLAND_DISPLAY");
                std::env::set_var("GDK_BACKEND", "x11");
            }
        }
    }
}

#[cfg(target_os = "linux")]
fn nonempty_env(name: &str) -> bool {
    std::env::var_os(name).is_some_and(|value| !value.is_empty())
}

pub(crate) fn inline_unavailable_reason() -> Option<&'static str> {
    #[cfg(target_os = "linux")]
    {
        match linux_backend(
            nonempty_env("DISPLAY"),
            nonempty_env("WAYLAND_DISPLAY"),
            std::env::var_os("ZED_HEADLESS").is_some(),
        ) {
            LinuxBackend::X11 => None,
            LinuxBackend::WaylandWithoutX11 => Some(
                "Inline previews require XWayland in a Wayland session. Install or enable XWayland, then relaunch with DISPLAY available. Source remains available.",
            ),
            LinuxBackend::Headless => Some(
                "Inline previews require an X11 or XWayland display. Start the desktop app in a graphical session with DISPLAY available.",
            ),
        }
    }
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    {
        None
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    {
        Some("This platform has no embedded browser. Open the web app for an interactive preview.")
    }
}

#[cfg(test)]
mod tests {
    use super::{linux_backend, LinuxBackend};

    #[test]
    fn linux_selects_an_inline_capable_parent_in_xwayland_sessions() {
        assert_eq!(linux_backend(true, true, false), LinuxBackend::X11);
        assert_eq!(linux_backend(true, false, false), LinuxBackend::X11);
        assert_eq!(
            linux_backend(false, true, false),
            LinuxBackend::WaylandWithoutX11
        );
        assert_eq!(linux_backend(true, true, true), LinuxBackend::Headless);
        assert_eq!(linux_backend(false, false, false), LinuxBackend::Headless);
    }
}

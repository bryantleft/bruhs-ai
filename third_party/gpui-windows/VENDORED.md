# GPUI Windows platform

Source: `crates/gpui_windows` from
https://github.com/zed-industries/zed/tree/d2a4fb6ded7c470be25d1541281da3882495b65b/crates/gpui_windows.
License: Apache-2.0; see `LICENSE-APACHE`.

The DirectComposition target uses `topmost = false`, so native WebView2 child
windows appear above the GPUI scene. That was a local change; zed made the same
change upstream before this revision, so it is no longer a difference. Chat
hides native embeds when overlays occlude them.

The one local change: dropping a window clears its activation callback before native destruction is
queued. An already queued activation notification then observes no callback
instead of updating a logical window GPUI has removed.

The manifest expands the upstream workspace dependency declarations, pins its
internal crates to the same revision, and keeps this vendor outside the app
workspace. No other renderer behavior is changed. Windows is not built on
the macOS gate; check it on Windows after moving the revision.

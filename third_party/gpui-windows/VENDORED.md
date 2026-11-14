# GPUI Windows platform

Source: `crates/gpui_windows` from
https://github.com/bnleft/zed/tree/47abf3f6a213244d9de96128d27693101defe986/crates/gpui_windows.
License: Apache-2.0; see `LICENSE-APACHE`.

The DirectComposition target uses `topmost = false` so native WebView2 child
windows appear above the GPUI scene. With `true`, widgets receive input and
save state while the opaque parent scene covers their pixels. This retains
DirectComposition and transparent-window rendering.

[Microsoft's target layering contract](https://learn.microsoft.com/en-us/windows/win32/api/dcomp/nf-dcomp-idcompositiondevice-createtargetforhwnd)
defines this ordering. Chat hides native embeds when overlays occlude them.

Dropping a window clears its activation callback before native destruction is
queued. An already queued activation notification then observes no callback
instead of updating a logical window GPUI has removed.

The manifest expands the upstream workspace dependency declarations, pins its
internal crates to the same revision, and keeps this vendor outside the app
workspace. No other renderer behavior is changed.

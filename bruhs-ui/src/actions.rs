//! Actions the components respond to. `CloseOverlays` (Escape) closes
//! whichever popover, listbox or dialog is open.
//!
//! The namespace stays `workspace` so existing key bindings
//! (`workspace::CloseOverlays`) keep working.

gpui::actions!(workspace, [CloseOverlays]);

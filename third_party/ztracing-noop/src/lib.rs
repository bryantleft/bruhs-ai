//! A no-op `#[instrument]` attribute, written for bruhs.
//!
//! zed's `sum_tree` (pulled in by gpui) marks a few methods with
//! `#[ztracing::instrument(skip_all)]`. Without zed's `ztracing` cfg that
//! attribute expands to nothing, but the real crate (and `zlog`,
//! `ztracing_macro`) are GPL-3.0-or-later, and linking them would make the
//! distributed desktop app GPL. The root `Cargo.toml` patches `ztracing` to
//! this crate instead: the item comes back unchanged.

use proc_macro::TokenStream;

/// Returns the item as written; any arguments (`skip_all`, …) are ignored.
#[proc_macro_attribute]
pub fn instrument(_args: TokenStream, item: TokenStream) -> TokenStream {
    item
}

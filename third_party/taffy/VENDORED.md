# Vendored Taffy

Source: Taffy 0.10.1 from crates.io, upstream commit
`639f0acdd20f914f847abffbeaa903d66cae4882` in
[DioxusLabs/taffy](https://github.com/DioxusLabs/taffy/tree/639f0acdd20f914f847abffbeaa903d66cae4882).
The published crate SHA-256 is
`aea22054047c16c3f34d3ac473a2170be1424b1115b2a3adcf28cfb067c88859`.
`LICENSE.md` is retained from that same upstream commit.

The only library source change is in `src/compute/flexbox.rs`: calculate the
minimum-content fallback with `unwrap_or_else`, so an already-known minimum does
not trigger a discarded child measurement. The fallback and all returned sizing
values are unchanged. No public API, feature, style, cache policy or schema changes.

Keep the published package files intact when updating this copy, then reapply and
verify the lazy fallback. The normalized Cargo manifest is the published one.
The application workspace excludes this dependency from workspace membership.

Verification is recorded in `docs/performance-goal-2026-11-09.md`: original/candidate
node geometry comparisons, exact-upstream conformance tests, native input/geometry
contracts, complete input/frame and initial-loading comparisons, the workspace
quick gate and MCP response compatibility. Physical display timing remains separate.

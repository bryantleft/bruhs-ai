# Vendored: gpui-component

| | |
|---|---|
| Upstream | https://github.com/longbridge/gpui-component |
| License | Apache-2.0 (see `LICENSE-APACHE`; Longbridge) |
| Version | `gpui-component` 0.5.2, `gpui-component-macros` 0.5.1, `gpui-component-assets` 0.5.1 |
| Upstream commit | `0775df394083c1ed74f36f846b78868d1267398f` (2026-07-10, "settings: Improves sidebar/resizable behavior in the settings UI. (#2540)") |

Upstream did not tag this version: 367 of the 375 vendored files are
byte-identical to upstream at `0775df3`, and the other 8 are the local
changes listed below. Upstream's `v0.5.1` tag (`0f0ab35`) is older; 0.5.2 was
an unreleased `main` version when this was vendored.

The crate is **excluded** from the Cargo workspace (`exclude` in the root
`Cargo.toml`) and consumed by path from `bruhs-desktop`:
`../third_party/gpui-component/crates/ui`. Only `crates/ui`, `crates/macros`,
and `crates/assets` are kept.

## Local changes

Changes relative to upstream `0775df3`:

- `Cargo.toml` (workspace): `gpui`, `gpui_platform`, `gpui_web`, `gpui_macros`,
  `reqwest_client` point at the `bnleft/zed` fork pinned by `rev` instead of
  `zed-industries/zed` (see "gpui pins" below).
- `crates/ui/Cargo.toml`: adds a `test-support = []` feature.
- `crates/ui/src/lib.rs`, `root.rs`, `input/native.rs`, `input/content_type.rs`:
  the macOS-native input path is gated on `not(feature = "test-support")`
  (upstream: `not(test)` / unconditional) so `bruhs-desktop`'s tests can use it.
- `crates/ui/src/input/input.rs`: adds `Input::aria_label`.
- `crates/ui/src/title_bar.rs`: `TITLE_BAR_HEIGHT` 34 -> 44,
  `TITLE_BAR_LEFT_PADDING` 80 -> 88, traffic-light position (9, 9) -> (12, 15).
- `crates/ui/src/resizable/resize_handle.rs`: a 4px hover strip, matching
  the web app.
- The whole tree is formatted with the workspace's rustfmt settings.

## gpui pins

This directory is not a workspace member, so it keeps its own copy of the gpui
git pins in `Cargo.toml`, all at the same `bnleft/zed` rev as the root
workspace: `gpui`, `gpui_platform`, `gpui_web`, `gpui_macros`,
`reqwest_client` (rev `47abf3f6a213244d9de96128d27693101defe986`). The root
`Cargo.toml` `[workspace.dependencies]` states that rev once for the rest of the
repo. When moving gpui forward, bump the root rev **and** the five pins here so
Cargo resolves a single gpui.

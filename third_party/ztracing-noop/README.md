# ztracing (no-op)

Stand-in for zed's `ztracing` crate, which is GPL-3.0-or-later. gpui's
`sum_tree` only uses its `#[instrument]` attribute, and only as a no-op in
normal builds, so this crate provides that attribute and nothing else. It is
original code under the workspace's MIT license.

The root `Cargo.toml` `[patch."https://github.com/bnleft/zed"]` entry points
`ztracing` here, which also drops `zlog` and `ztracing_macro` from the build.
When moving the gpui rev, check `cargo tree -p bruhs-desktop -i ztracing`
still resolves to this path and that `sum_tree` uses nothing beyond
`instrument`.

# Vendored: mathjax-svg-rs

| | |
|---|---|
| Upstream | https://github.com/sevenc-nanashi/mathjax-svg-rs |
| License | MIT (see `LICENSE`, from upstream commit `dea672fc0540562364c1c1dcb53052a6a32937b6`) |
| Version | 0.4.0 from crates.io (SHA-256 `2b2c39f596739e90fe29a19551185250b30ff074cda25982d670205bad012ee2`) |

The published package is kept intact, except for the changes listed below. The
root `Cargo.toml` uses it through `[patch.crates-io]`, and the application
workspace excludes it.

## Why

0.4.0, the latest release, depends on `boa_engine ^0.21.1`, which pins the
`icu_*` crates to `~2.0`. gpui at the zed revision the workspace pins (see
`third_party/gpui-component/VENDORED.md`) requires `icu_segmenter ^2.3`. Cargo
cannot have both, so the lockfile cannot resolve. `boa_engine` 0.22 uses icu
`~2.3`.

## Local changes

- `Cargo.toml`: `boa_engine` 0.21.1 -> 0.22.0.
- `src/lib.rs`: boa 0.22 renamed `JsError::to_opaque` to `into_opaque`, which
  now returns a `JsResult`. Both error paths format the thrown value as
  before. If converting the value fails, they fall back to the conversion
  error's message.

The crate's own tests pass against boa 0.22 (4 tests: rendering, alignment,
errors). Drop this copy once an upstream release depends on boa 0.22 or later.

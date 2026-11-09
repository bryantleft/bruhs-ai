# Contributing

Issues and pull requests are welcome. Keep changes focused, run the gate,
and describe what you checked by hand.

## Layout

A Cargo workspace; run commands from the root with `-p <crate>` or from a
crate's directory. Types and pure logic every surface shares live in
`bruhs-models` (the only shared crate the WASM app can use); native app logic
lives in `bruhs-core`. See the crate table in the [README](README.md#architecture).

```bash
scripts/dev.sh                      # daemon + server + web, Ctrl-C stops all
cd bruhs-web && trunk serve         # web app with hot reload on :8080
cd bruhs-server && cargo run        # API + built web app on :3001
cargo run -p bruhs-desktop          # native app
cargo run -p bruhs-capture          # capture daemon on :3002 (macOS)
cargo run -p bruhs-mcp              # MCP server over stdio
cd bruhs-site && npm test           # landing page (Node 22.18+)
```

The desktop and web apps are two renderings of one product: a change to one
should land on the other, with the web app as the reference.

## The gate

`scripts/gate.sh` must pass before anything merges to `main`
(`--quick` runs fmt, clippy and tests only). Each step's output goes to
`target/gate/<step>.log`.

| Step | Command |
|---|---|
| `fmt` | `cargo fmt --all --check` |
| `clippy` | `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings` |
| `clippy-wasm` | `cargo clippy -p bruhs-web --target wasm32-unknown-unknown --locked -- -D warnings` |
| `test` | `cargo test --workspace --locked` |
| `bench-check` | `scripts/task-bench.sh --self-check` (no agent, no tokens) |
| `wasm-build` | `cargo build -p bruhs-web --target wasm32-unknown-unknown --locked` |
| `desktop-release` | `cargo check -p bruhs-desktop --release --locked` |
| `rustdoc` | `cargo doc --workspace --exclude bruhs-web --no-deps` with `-D warnings` |
| `deny` | `cargo deny check` (skipped if `cargo-deny` isn't installed) |
| `trunk-build` | `trunk build --release` (skipped if `trunk` isn't installed) |

GitHub Actions (`.github/workflows/ci.yml`) runs the same checks on hosted
Linux and macOS runners when started from the Actions tab.

## Code rules

- Lints live in the root `Cargo.toml`. Suppress one with
  `#[expect(lint, reason = "…")]`, never a bare `#[allow]`.
- Every `unsafe` block has a `// SAFETY:` comment.
- Numeric `as` casts that can truncate, wrap, drop a sign or lose precision
  fail the gate. Use `From`/`TryFrom`, `EpochMs` for timestamps, or
  `bruhs_models::num` (`to_f64_lossy`, `to_u32_saturating`, …) when rounding
  is the intent.
- Dependencies must pass `cargo deny check`: permissive licenses only, no
  open advisories.
- The web app's CSP allows scripts only from its own origin, so libraries
  are vendored under `bruhs-web/vendor/lib` rather than loaded from a CDN.

## Design rules

bruhs uses the [bruhs design system](https://design.bruhs.dev/llms.txt).

- All-mono type (Monaspace), warm dark neutrals, fruit-named tokens and
  radii.
- No drop shadows, glass or gradients: elevation is a tone step plus an inset
  ring.
- Persimmon is the brand and the call to action; blueberry marks selection
  and information; neutrals carry the chrome.
- Keyboard-native, pointer-complete: repeated workflows get shortcuts, and
  every control still has a clear, accessible pointer target.
- Make state, feedback and recovery explicit; never report a save that did
  not happen.

Motion:

- Typing, keyboard actions, search updates, streaming text and long lists
  are immediate, with no staggered delays.
- Stagger only small, occasional group reveals: 30–40 ms between groups,
  about 120 ms of accumulated delay at most, the whole reveal within 300 ms.
- Control transitions take 120–200 ms, share one easing and stay
  interruptible. No action or focus target waits for decoration.
- Respect reduced motion on both surfaces: drop spatial motion, staggering
  and decorative loops, keep state and feedback readable.

## Benchmarks

`scripts/task-bench.sh` runs coding and chat tasks through bruhs and other
agent CLIs and records pass/fail and tokens; `scripts/token-bench.sh`
measures per-turn overhead. Both spend real tokens. See
[bench/README.md](bench/README.md) and [bench/RESULTS.md](bench/RESULTS.md).

## Changelog

User-facing changes go under `## [Unreleased]` in [CHANGELOG.md](CHANGELOG.md)
([Keep a Changelog](https://keepachangelog.com/en/1.1.0/): Added, Changed,
Fixed…), written for people who use bruhs rather than for its code. The site
builds [bruhs.ai/changelog](https://bruhs.ai/changelog) from it on every deploy
of `main`.

## Releases

To cut a release:

1. In `CHANGELOG.md`, rename `## [Unreleased]` to `## [<version>] - <date>`,
   add a fresh empty `## [Unreleased]`, and update the links at the bottom.
2. Commit and push `main`.
3. Run `scripts/release.sh <version>`.

`scripts/release.sh <version>` publishes a release from a clean checkout of
`origin/main` and uses the version's `CHANGELOG.md` section as its notes. It builds with
`scripts/bundle-macos.sh <version> --sign --notarize`, uploads
`Bruhs-macOS-<arch>.dmg`, `Bruhs-macOS-<arch>.zip` (what installed apps
update from) and `SHA256SUMS` to a draft release, then publishes it, which
tags `v<version>`. Asset names carry no version, so
`releases/latest/download/Bruhs-macOS-arm64.dmg` always points at the newest.

- `scripts/bundle-macos.sh <version>` alone builds an ad-hoc-signed
  `dist/Bruhs.app` to try locally; `--sign` adds the Developer ID signature
  and `--notarize` Apple's notarization.
- Signing uses the first "Developer ID Application" identity in the keychain
  (`BRUHS_SIGN_IDENTITY` overrides). Notarization uses the notarytool
  profile `bruhs-notary` (`BRUHS_NOTARY_PROFILE`); create it once with
  `xcrun notarytool store-credentials bruhs-notary --apple-id … --team-id …
  --password <app-specific>`.
- The bundle carries `Contents/Resources/THIRD_PARTY_LICENSES`, written by
  `scripts/third-party-licenses.sh` (needs
  `cargo install --locked cargo-about --features cli`).
- `BRUHS_RELEASE_VERSION`, set by the bundle script, makes a build a release:
  it then updates from GitHub releases instead of git.
- The icon is `packaging/macos/AppIcon.icns`, generated from
  `bruhs-web/bruhs.svg` by `packaging/macos/make-icon.sh`.
- gpui's `sum_tree` depends on zed's GPL-3.0 `ztracing`; the workspace patches
  it to the no-op `third_party/ztracing-noop` so the shipped binaries stay
  MIT.

## Third-party code

- `third_party/gpui-component` is a vendored copy of
  [longbridge/gpui-component](https://github.com/longbridge/gpui-component)
  (Apache-2.0); `VENDORED.md` lists the local changes.
- gpui comes from the [bnleft/zed](https://github.com/bnleft/zed) fork, which
  adds browser-style prose line breaking.

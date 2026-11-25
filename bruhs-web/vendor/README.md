# Local Bruhs theme

`bruhs.css` is generated from **@bruhs/theme 0.3.0** (MIT; see
`BRUHS-LICENSE`). The upstream source SHA-256 and byte counts are recorded in
`bruhs-source.json`. Colors, typography tokens, font faces, and animation
definitions are preserved. Unused selector rules are pruned and font sources
point to the WOFF2 files already bundled in `bruhs-web/fonts`.

Trunk includes this file locally; production startup no longer downloads the
4.4 MB uncompressed theme from a CDN. Normal Rust/Trunk builds need no Node
installation. Regenerate after adding theme-only utility classes:

```sh
theme_build_dir=$(mktemp -d /tmp/bruhs-theme.XXXXXX)
npm pack @bruhs/theme@0.3.0 --pack-destination "$theme_build_dir"
tar -xzf "$theme_build_dir/bruhs-theme-0.3.0.tgz" -C "$theme_build_dir"
npm install --prefix "$theme_build_dir" --ignore-scripts --no-audit --no-fund purgecss@7.0.2
node scripts/build-bruhs-theme.mjs "$theme_build_dir/package" "$theme_build_dir"
```

Inspect the generated diff and browser screenshots after regeneration. The
Rust source scan includes shared-model class strings as well as frontend
components. Dynamically assembled class names need explicit coverage; prefer
complete static class strings in model/UI helpers.

# Vendored browser libraries (`lib/`)

xterm.js (+ its fit addon) is served from this origin, because `index.html`'s
Content-Security-Policy only lets scripts load from `'self'`. Syntax
highlighting, TeX math, and Mermaid diagrams now run natively in WASM via
`bruhs-models` with zero vendored JS. Trunk copies `lib/` to `dist/vendor/`
(URLs `/vendor/<package>-<version>/…`; the version in the directory name busts
browser caches on upgrade). `lib-sources.json` records, per package, the npm
tarball, its `integrity`, the CDN URLs it replaced and the sha256 and size of
every vendored file; files are copied unmodified.

| Directory | Package | Notes |
|---|---|---|
| `xterm-5.5.0/` | `@xterm/xterm@5.5.0` | `lib/xterm.js`, `css/xterm.css` |
| `addon-fit-0.10.0/` | `@xterm/addon-fit@0.10.0` | `lib/addon-fit.js` |

To upgrade one, fetch the exact version and copy the same files:

```sh
dir=$(mktemp -d /tmp/bruhs-vendor.XXXXXX) && cd "$dir"
npm pack @xterm/xterm@<version>     # or @xterm/addon-fit@…
tar -xzf xterm-<version>.tgz
# copy the files listed for the package in lib-sources.json into a new
# lib/<package>-<version>/ directory, update index.html / js/bruhs-glue.js,
# then recompute sha256 and bytes in lib-sources.json:
shasum -a 256 <file>
```

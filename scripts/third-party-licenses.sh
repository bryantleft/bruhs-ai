#!/usr/bin/env bash
# Write the license notice Bruhs.app ships (Contents/Resources/
# THIRD_PARTY_LICENSES): every Rust crate in the workspace (cargo-about), then the bundled native libraries, the web app's
# vendored scripts and the Monaspace fonts.
#
#   scripts/third-party-licenses.sh <out-file>
#
# Needs cargo-about: cargo install --locked cargo-about --features cli
set -euo pipefail
cd "$(dirname "$0")/.."
out=${1:?usage: scripts/third-party-licenses.sh <out-file>}
command -v cargo-about >/dev/null || {
  echo "cargo-about is missing: cargo install --locked cargo-about --features cli" >&2
  exit 1
}

# Where a crate's sources live (for licenses of the C/C++ code they bundle).
crate_dir() {
  cargo metadata --format-version 1 --locked 2>/dev/null |
    python3 -c 'import json,sys,os; m=json.load(sys.stdin); print(next(os.path.dirname(p["manifest_path"]) for p in m["packages"] if p["name"]==sys.argv[1]))' "$1"
}

section() { # title, license file
  printf '\n--------------------------------------------------------------------------\n%s\n\n' "$1"
  cat "$2"
}

{
  # The whole workspace: a superset of what the app links, which is fine
  # for a notice.
  cargo about generate --fail --locked --workspace \
    -c packaging/licenses/about.toml packaging/licenses/about.hbs

  printf '\n\nBundled libraries, scripts and fonts\n------------------------------------\n'
  # The gpui crates from the zed fork carry no license field, so cargo-about
  # skips them; gpui is Apache-2.0.
  section "GPUI and its zed crates (Apache-2.0), from github.com/bnleft/zed" "$(crate_dir gpui)/LICENSE-APACHE"
  section "whisper.cpp (MIT), in bruhs-capture" "$(crate_dir whisper-rs-sys)/whisper.cpp/LICENSE"
  section "sherpa-onnx (Apache-2.0), Contents/Frameworks/libsherpa-onnx-*.dylib" "$(crate_dir sherpa-rs-sys)/sherpa-onnx/LICENSE"
  section "ONNX Runtime (MIT), Contents/Frameworks/libonnxruntime*.dylib" packaging/licenses/onnxruntime-LICENSE
  for lib in bruhs-web/vendor/lib/*/; do
    name=$(basename "$lib")
    [[ -f $lib/LICENSE ]] && section "$name, in the web app" "$lib/LICENSE"
  done
  section "Bruhs design tokens (MIT), in the web app" bruhs-web/vendor/BRUHS-LICENSE
  section "Monaspace fonts (SIL Open Font License 1.1)" bruhs-desktop/assets/fonts/LICENSE
} >"$out"
echo "wrote $out ($(wc -l <"$out" | tr -d ' ') lines)"

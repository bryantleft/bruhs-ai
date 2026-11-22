#!/usr/bin/env bash
# Generate the human-readable bundle notice and its searchable desktop catalog.
# To refresh the embedded source-build catalog, copy the generated .zip to
# bruhs-desktop/assets/THIRD_PARTY_LICENSES.zip.
set -euo pipefail
cd "$(dirname "$0")/.."
command -v cargo-about >/dev/null || {
  echo "cargo-about is missing: cargo install --locked cargo-about --features cli" >&2
  exit 1
}
exec python3 scripts/third-party-licenses.py "${1:?usage: scripts/third-party-licenses.sh <out-file>}"

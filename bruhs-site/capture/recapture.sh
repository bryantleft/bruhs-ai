#!/bin/sh
# Re-captures the landing-page screenshots from the committed bruhs-web and
# writes them into public/shots/. Needs trunk, cargo, node 22+, python3,
# Google Chrome, ImageMagick (magick) and cwebp. Run it outside any tool
# sandbox: it serves the app on 127.0.0.1:8111 and drives Chrome on :9351.
#
#   capture/recapture.sh --build   # export HEAD, build bruhs-web, then capture
#   capture/recapture.sh           # reuse the last build
#   capture/recapture.sh people    # just these shots (see shots.mjs for names)
set -e
HERE=$(cd "$(dirname "$0")" && pwd)
REPO=$(git -C "$HERE" rev-parse --show-toplevel)
SRC=/tmp/bruhs-shot-src DIST=/tmp/bruhs-shot-dist RAW=/tmp/bruhs-shots
if [ "$1" = "--build" ]; then
  shift
  # HEAD, not the working tree: uncommitted work never reaches the site.
  rm -rf "$SRC" && mkdir -p "$SRC"
  git -C "$REPO" archive HEAD | tar -x -C "$SRC"
  (cd "$SRC/bruhs-web" && CARGO_TARGET_DIR="$REPO/target" BRUHS_API_URL='' trunk build --release --dist "$DIST")
  (cd "$HERE/validator" && CARGO_TARGET_DIR=/tmp/bruhs-seed-target cargo build)
fi
mkdir -p "$RAW"
# The validator round-trips the seed through the real bruhs-models types, so a
# renamed field fails here instead of rendering an empty screen.
node "$HERE/seed.mjs" "$RAW/seed.raw.json"
/tmp/bruhs-seed-target/debug/bruhs-seed-validator "$RAW/seed.raw.json" > "$RAW/seed.json"
python3 "$HERE/fake-api.py" "$DIST" 8111 "$RAW/seed.json" & API=$!
rm -rf /tmp/bruhs-shot-chrome
"/Applications/Google Chrome.app/Contents/MacOS/Google Chrome" --headless=new --remote-debugging-port=9351 \
  --user-data-dir=/tmp/bruhs-shot-chrome --hide-scrollbars --no-first-run --no-default-browser-check about:blank >/dev/null 2>&1 & CHROME=$!
trap 'kill $API $CHROME 2>/dev/null' EXIT
sleep 3
REPORT="$RAW/report.json" node "$HERE/shots.mjs" http://127.0.0.1:8111 "$RAW/seed.json" "$RAW" 9351 "$@"
"$HERE/export.sh" "$RAW"

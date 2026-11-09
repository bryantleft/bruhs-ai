#!/bin/sh
# Regenerates the raster brand assets in public/ from brand/*.html with
# headless Google Chrome: og.png (social card) and apple-touch-icon.png.
# Run brand/logo.mjs first if the mark changed. Binds 127.0.0.1:8492.
set -e
cd "$(dirname "$0")/.."
CHROME="/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"
python3 -m http.server 8492 --bind 127.0.0.1 >/dev/null 2>&1 & SERVER=$!
trap 'kill $SERVER' EXIT
sleep 1
"$CHROME" --headless=new --hide-scrollbars --force-device-scale-factor=1 --window-size=1200,630 \
  --virtual-time-budget=4000 --screenshot=public/og.png http://127.0.0.1:8492/brand/og.html 2>/dev/null
"$CHROME" --headless=new --hide-scrollbars --force-device-scale-factor=1 --window-size=180,180 \
  --virtual-time-budget=2000 --screenshot=public/apple-touch-icon.png http://127.0.0.1:8492/brand/touch-icon.html 2>/dev/null
echo "wrote public/og.png and public/apple-touch-icon.png"

#!/usr/bin/env bash
# Regenerate packaging/macos/AppIcon.icns from bruhs-web/favicon.svg (the
# bruhs.ai dot mark on its longan tile; bruhs-site/brand/logo.mjs writes it).
# macOS icons draw their art inside ~80% of the canvas (824 of 1024), so each
# size is rendered smaller and centered. Needs rsvg-convert and
# ImageMagick (`brew install librsvg imagemagick`).
set -euo pipefail
cd "$(dirname "$0")/../.."
svg=bruhs-web/favicon.svg
out=packaging/macos/AppIcon.icns
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
set_dir="$work/AppIcon.iconset"
mkdir -p "$set_dir"
render() { # size file
  local size=$1 file=$2 art
  art=$(( size * 824 / 1024 ))
  rsvg-convert -w "$art" -h "$art" "$svg" -o "$work/art.png"
  magick -size "${size}x${size}" xc:none "$work/art.png" -gravity center -composite "$set_dir/$file"
}
for base in 16 32 128 256 512; do
  render "$base" "icon_${base}x${base}.png"
  render $(( base * 2 )) "icon_${base}x${base}@2x.png"
done
iconutil -c icns "$set_dir" -o "$out"
echo "wrote $out"

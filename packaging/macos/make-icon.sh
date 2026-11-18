#!/usr/bin/env bash
# Regenerate packaging/macos/AppIcon.icns from the site's dot Bruh on its
# longan tile (bruhs-site/brand/logo.mjs writes both SVGs): AppIcon-small.svg
# up to 256px and AppIcon.svg above, the grids the site's bruhDots picks for
# the art's radius at those sizes.
# Its 512px render is also the server's tasks icon and the mobile app icon.
# macOS icons draw their art inside ~80% of the canvas (824 of 1024), so each
# size is rendered smaller and centered. Needs rsvg-convert and
# ImageMagick (`brew install librsvg imagemagick`).
set -euo pipefail
cd "$(dirname "$0")/../.."
out=packaging/macos/AppIcon.icns
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
set_dir="$work/AppIcon.iconset"
mkdir -p "$set_dir"
render() { # size file
  local size=$1 file=$2 art svg=packaging/macos/AppIcon.svg
  (( size <= 256 )) && svg=packaging/macos/AppIcon-small.svg
  art=$(( size * 824 / 1024 ))
  rsvg-convert -w "$art" -h "$art" "$svg" -o "$work/art.png"
  magick -size "${size}x${size}" xc:none "$work/art.png" -gravity center -composite "$set_dir/$file"
}
for base in 16 32 128 256 512; do
  render "$base" "icon_${base}x${base}.png"
  render $(( base * 2 )) "icon_${base}x${base}@2x.png"
done
iconutil -c icns "$set_dir" -o "$out"
for copy in bruhs-server/src/routes/coordination/icon-512.png bruhs-mobile/assets/icon.png; do
  cp "$set_dir/icon_256x256@2x.png" "$copy"
done
echo "wrote $out, bruhs-server/src/routes/coordination/icon-512.png and bruhs-mobile/assets/icon.png"

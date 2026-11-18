#!/usr/bin/env bash
# Package an app into the disk image people download: the Bruhs background,
# the app and an Applications shortcut side by side, and the Bruhs volume icon.
#
#   packaging/macos/make-dmg.sh dist/Bruhs.app dist/Bruhs-macOS-arm64.dmg
#
# Finder writes the layout (window size, icon spots, background) into the
# image's .DS_Store, so this drives Finder over AppleScript: the first run asks
# to let the terminal control Finder. The spots match the art in
# packaging/macos/dmg-background.tiff (bruhs-site/brand/dmg.html); change both
# together.
set -euo pipefail
cd "$(dirname "$0")/../.."

app=${1:?usage: make-dmg.sh <Name.app> <out.dmg>}
out=${2:?usage: make-dmg.sh <Name.app> <out.dmg>}
volname=Bruhs
background=packaging/macos/dmg-background.tiff
icon=packaging/macos/AppIcon.icns
# Content size of the window in points. The background is 20pt taller, so
# title bars that differ across macOS versions never uncover the window color.
width=640
height=400
title_bar=32
icon_size=112
app_spot="180, 190"
apps_spot="460, 190"

[[ -d $app ]] || { echo "no app at $app" >&2; exit 1; }
mount=/Volumes/$volname
[[ ! -e $mount ]] || { echo "$mount is already mounted; eject it first" >&2; exit 1; }

work=$(mktemp -d)
cleanup() {
  [[ -d $mount ]] && hdiutil detach -quiet -force "$mount" 2>/dev/null || true
  rm -rf "$work"
}
trap cleanup EXIT

stage="$work/stage"
mkdir -p "$stage/.background"
cp -R "$app" "$stage/"
ln -s /Applications "$stage/Applications"
cp "$background" "$stage/.background/background.tiff"
cp "$icon" "$stage/.VolumeIcon.icns"

# A writable image first, so Finder can save the layout into it.
size_mb=$(( $(du -sm "$stage" | cut -f1) + 20 ))
hdiutil create -quiet -volname "$volname" -srcfolder "$stage" -fs HFS+ \
  -format UDRW -size "${size_mb}m" "$work/rw.dmg"
hdiutil attach -quiet -readwrite -noverify -noautoopen -mountpoint "$mount" "$work/rw.dmg"
# The volume shows its icon only with the custom-icon flag set.
if command -v SetFile >/dev/null; then SetFile -a C "$mount"; fi

app_name=$(basename "$app")
# The title bar sits inside the window bounds; the background fills the rest.
# The path bar is hidden here since it would cover the art's bottom edge.
osascript <<EOF
tell application "Finder"
  tell disk "$volname"
    open
    set current view of container window to icon view
    set toolbar visible of container window to false
    set statusbar visible of container window to false
    set pathbar visible of container window to false
    set bounds of container window to {200, 120, 200 + $width, 120 + $height + $title_bar}
    set opts to icon view options of container window
    set arrangement of opts to not arranged
    set icon size of opts to $icon_size
    set text size of opts to 13
    set label position of opts to bottom
    set shows item info of opts to false
    set shows icon preview of opts to false
    set background picture of opts to file ".background:background.tiff"
    set position of item "$app_name" of container window to {$app_spot}
    set position of item "Applications" of container window to {$apps_spot}
    close
    open
    update without registering applications
    delay 2
    close
  end tell
end tell
EOF

# Wait for Finder to flush .DS_Store, then seal the image.
for _ in {1..20}; do [[ -f $mount/.DS_Store ]] && break; sleep 0.5; done
[[ -f $mount/.DS_Store ]] || { echo "Finder saved no layout" >&2; exit 1; }
rm -rf "$mount/.fseventsd" "$mount/.Trashes"
chmod -Rf go-w "$mount" 2>/dev/null || true
sync
hdiutil detach -quiet "$mount"
rm -f "$out"
hdiutil convert -quiet "$work/rw.dmg" -format UDZO -imagekey zlib-level=9 -o "$out"
echo "wrote $out"

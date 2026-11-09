#!/usr/bin/env bash
# Build Bruhs.app: the desktop app with the server (API + web app) and the
# capture daemon inside, then optionally sign, notarize and package it.
#
#   scripts/bundle-macos.sh 0.1.0                 # unsigned, for a local try
#   scripts/bundle-macos.sh 0.1.0 --sign          # Developer ID signed
#   scripts/bundle-macos.sh 0.1.0 --sign --notarize
#
# Output in dist/:
#   Bruhs.app
#   Bruhs-macOS-<arch>.dmg   what people download (Applications shortcut inside)
#   Bruhs-macOS-<arch>.zip   what installed apps update themselves from
#   SHA256SUMS               checksums the in-app updater verifies
#
# Signing uses the first "Developer ID Application" identity in the login
# keychain (override with BRUHS_SIGN_IDENTITY). Notarization uses a notarytool
# keychain profile, BRUHS_NOTARY_PROFILE (default "bruhs-notary"); create it
# once with:
#   xcrun notarytool store-credentials bruhs-notary \
#     --apple-id <you@example.com> --team-id <TEAMID> --password <app-specific>
set -euo pipefail
shopt -s nullglob
cd "$(dirname "$0")/.."

version=${1:-}
shift || true
sign=0
notarize=0
for arg in "$@"; do
  case $arg in
    --sign) sign=1 ;;
    --notarize) notarize=1 ;;
    *) echo "unknown option: $arg" >&2; exit 2 ;;
  esac
done
if [[ ! $version =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.]+)?$ ]]; then
  echo "usage: scripts/bundle-macos.sh <version like 0.1.0> [--sign] [--notarize]" >&2
  exit 2
fi
if (( notarize && !sign )); then
  echo "--notarize needs --sign" >&2
  exit 2
fi
[[ $(uname) == Darwin ]] || { echo "macOS only" >&2; exit 1; }
command -v cargo-about >/dev/null || {
  echo "cargo-about is missing (license notice): cargo install --locked cargo-about --features cli" >&2
  exit 1
}

case $(uname -m) in
  arm64) arch=arm64 ;;
  x86_64) arch=x86_64 ;;
  *) echo "unsupported architecture $(uname -m)" >&2; exit 1 ;;
esac
dist=dist
app="$dist/Bruhs.app"
dmg="$dist/Bruhs-macOS-$arch.dmg"
zip="$dist/Bruhs-macOS-$arch.zip"
profile=${BRUHS_NOTARY_PROFILE:-bruhs-notary}
entitlements=packaging/macos/entitlements.plist

step() { printf '\n==> %s\n' "$*"; }

identity=${BRUHS_SIGN_IDENTITY:-}
if (( sign )) && [[ -z $identity ]]; then
  identity=$(security find-identity -v -p codesigning | sed -n 's/.*"\(Developer ID Application: [^"]*\)".*/\1/p' | head -1)
  [[ -n $identity ]] || { echo "no Developer ID Application identity in the keychain" >&2; exit 1; }
fi
if (( notarize )); then
  xcrun notarytool history --keychain-profile "$profile" >/dev/null 2>&1 || {
    echo "notarytool profile '$profile' is missing; see the header of this script" >&2
    exit 1
  }
fi

# A release build checks GitHub releases instead of the checkout (see
# bruhs-core/build_info.rs).
export BRUHS_RELEASE_VERSION=$version

step "web app (trunk build --release)"
(cd bruhs-web && trunk build --release --dist ../target/bundle-web)

step "binaries (cargo build --release)"
cargo build --release --locked -p bruhs-desktop -p bruhs-server -p bruhs-capture

step "assemble $app"
rm -rf "$dist"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources" "$app/Contents/Frameworks"
build=$(git rev-list --count HEAD 2>/dev/null || echo 1)
sed -e "s/@VERSION@/$version/g" -e "s/@BUILD@/$build/g" packaging/macos/Info.plist.in >"$app/Contents/Info.plist"
plutil -lint "$app/Contents/Info.plist" >/dev/null
cp packaging/macos/AppIcon.icns "$app/Contents/Resources/AppIcon.icns"
scripts/third-party-licenses.sh "$app/Contents/Resources/THIRD_PARTY_LICENSES"
cp -R target/bundle-web "$app/Contents/Resources/web"
for bin in bruhs-desktop bruhs-server bruhs-capture; do
  cp "target/release/$bin" "$app/Contents/MacOS/$bin"
done
# The capture daemon loads ONNX Runtime and sherpa-onnx by @rpath; ship the
# real files (not their symlinks) in Frameworks and point the rpath there.
for lib in $(otool -L target/release/bruhs-capture | awk '/@rpath\//{print $1}' | sed 's|@rpath/||'); do
  cp -L "target/release/$lib" "$app/Contents/Frameworks/$lib"
done
install_name_tool -add_rpath "@executable_path/../Frameworks" "$app/Contents/MacOS/bruhs-capture"
for lib in "$app"/Contents/Frameworks/*.dylib; do
  # Libraries that load each other by @rpath find their siblings too.
  install_name_tool -add_rpath "@loader_path" "$lib" 2>/dev/null || true
done

if (( sign )); then
  step "sign with: $identity"
  sign_one() { codesign --force --timestamp --options runtime --sign "$identity" "$@"; }
  for lib in "$app"/Contents/Frameworks/*.dylib; do sign_one "$lib"; done
  for bin in bruhs-server bruhs-capture; do
    sign_one --entitlements "$entitlements" "$app/Contents/MacOS/$bin"
  done
  sign_one --entitlements "$entitlements" "$app"
  codesign --verify --deep --strict --verbose=2 "$app"
else
  step "ad-hoc sign (runs on this Mac only; cannot update itself)"
  codesign --force --deep --sign - "$app"
fi

if (( notarize )); then
  step "notarize the app"
  ditto -c -k --keepParent "$app" "$dist/notarize.zip"
  xcrun notarytool submit "$dist/notarize.zip" --keychain-profile "$profile" --wait
  rm "$dist/notarize.zip"
  xcrun stapler staple "$app"
  spctl --assess --type execute --verbose "$app"
fi

step "update zip $zip"
ditto -c -k --keepParent "$app" "$zip"

step "disk image $dmg"
stage="$dist/dmg"
mkdir -p "$stage"
cp -R "$app" "$stage/"
ln -s /Applications "$stage/Applications"
hdiutil create -quiet -volname "Bruhs" -srcfolder "$stage" -fs HFS+ -format UDZO "$dmg"
rm -rf "$stage"
if (( sign )); then
  codesign --force --timestamp --sign "$identity" "$dmg"
fi
if (( notarize )); then
  step "notarize the disk image"
  xcrun notarytool submit "$dmg" --keychain-profile "$profile" --wait
  xcrun stapler staple "$dmg"
fi

step "checksums"
(cd "$dist" && shasum -a 256 "$(basename "$dmg")" "$(basename "$zip")" >SHA256SUMS && cat SHA256SUMS)
printf '\nBruhs %s (%s) in %s/\n' "$version" "$arch" "$dist"

#!/usr/bin/env bash
# Combine binaries from the native release workflow with the web bundle and
# license notices already prepared by bundle-macos.sh at this exact revision.
set -euo pipefail
cd "$(dirname "$0")/.."
version=${1:?release version}
run=${2:?release-native.yml workflow run ID}
repo=bryantleft/bruhs-ai
commit=$(git rev-parse HEAD)
run_info=$(gh run view "$run" --repo "$repo" --json headSha,workflowName,conclusion --jq '[.headSha,.workflowName,.conclusion] | join("|")')
[[ $run_info == "$commit|Release desktop binaries|success" ]] || { echo "Expected a successful native build of $commit, got $run_info" >&2; exit 1; }
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
resources=dist/Bruhs.app/Contents/Resources
test -s "$resources/web/index.html"
test -s "$resources/THIRD_PARTY_LICENSES"
for platform in Linux Windows; do
  bin="$work/native-$platform"
  gh run download "$run" --repo "$repo" -n "native-$platform" -D "$bin"
  [[ $(cat "$bin/commit") == "$commit" && $(cat "$bin/version") == "$version" ]]
  ext=""
  dest="$work/deb/opt/bruhs"
  if [[ $platform == Windows ]]; then ext=".exe"; dest="$work/windows/Bruhs"; fi
  mkdir -p "$dest"
  for name in bruhs-desktop bruhs-server bruhs-capture; do install -m755 "$bin/$name$ext" "$dest/"; done
  cp -R "$resources/web" "$resources/THIRD_PARTY_LICENSES" "$resources/THIRD_PARTY_LICENSES.zip" LICENSE "$dest/"
done
windows="$work/windows/Bruhs"
scripts/sign-windows.sh "$windows"/bruhs-{desktop,server,capture}.exe
installer="$PWD/dist/Bruhs-Windows-x64-Setup.exe"
makensis -WX -V2 "-DPAYLOAD=$windows" "-DOUTPUT=$installer" "-DVERSION=$version" packaging/windows/bruhs.nsi
scripts/sign-windows.sh "$installer"
deb="$work/deb"
mkdir -p "$deb/DEBIAN" "$deb/usr/share/applications" "$deb/usr/share/icons/hicolor/scalable/apps" "$deb/usr/bin"
ln -s /opt/bruhs/bruhs-desktop "$deb/usr/bin/bruhs"
cp bruhs-site/public/logo.svg "$deb/usr/share/icons/hicolor/scalable/apps/bruhs.svg"
cat >"$deb/usr/share/applications/bruhs.desktop" <<DESKTOP
[Desktop Entry]
Type=Application
Name=Bruhs
Exec=/opt/bruhs/bruhs-desktop
Icon=bruhs
Categories=Development;
Terminal=false
DESKTOP
depends=$(cat "$work/native-Linux/depends")
test -n "$depends"
cat >"$deb/DEBIAN/control" <<CONTROL
Package: bruhs
Version: ${version/-nightly./~nightly.}
Architecture: amd64
Maintainer: Bruhs <hello@bruhs.ai>
Description: Bruhs desktop workspace
Depends: $depends, libvulkan1
CONTROL
dpkg-deb --root-owner-group -Zxz --build "$deb" dist/Bruhs-Linux-x64.deb
(cd dist && shasum -a 256 Bruhs-Windows-x64-Setup.exe Bruhs-Linux-x64.deb >> SHA256SUMS && shasum -c SHA256SUMS)

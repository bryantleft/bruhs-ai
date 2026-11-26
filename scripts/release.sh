#!/usr/bin/env bash
# Publish a Bruhs release: signed + notarized Bruhs.app as a GitHub Release.
#
#   scripts/release.sh 0.2.0 <native-workflow-run-id>
#
# Windows signing requires Java, Azure CLI, and a logged-in user with the
# Artifact Signing Certificate Profile Signer role on the release profile.
# Export BRUHS_SIGNING_ENDPOINT (e.g. eus.codesigning.azure.net),
# BRUHS_SIGNING_ACCOUNT, and BRUHS_SIGNING_PROFILE before running.
#
# Runs from a clean checkout of `origin/main` (on `main` or detached) that
# the gate passed on. CHANGELOG.md must have a dated `## [<version>]` section:
# it becomes the release notes, and bruhs.ai/changelog shows the same entry
# (the site builds that page from CHANGELOG.md when main deploys).
# Builds with
# scripts/bundle-macos.sh --sign --notarize, uploads the disk image, the
# update zip and SHA256SUMS to a draft release, then publishes it, which
# creates the v<version> tag at this commit. Nothing is public until every
# asset is up, so installed apps never see a release without its zip.
# Installed apps find it through GitHub's releases/latest API; the site's
# Download button links to releases/latest/download/Bruhs-macOS-<arch>.dmg.
set -euo pipefail
cd "$(dirname "$0")/.."

version=${1:-}
[[ $version =~ ^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(-nightly\.(0|[1-9][0-9]*))?$ ]] || { echo "usage: scripts/release.sh <version like 0.1.0 or 0.1.1-nightly.1>" >&2; exit 2; }
native_run=${2:?pass the successful release-native.yml run ID for this commit and version}
command -v dpkg-deb >/dev/null || { echo "Install dpkg to assemble the Linux package" >&2; exit 1; }
command -v makensis >/dev/null || { echo "Install NSIS to assemble the Windows installer" >&2; exit 1; }
tag="v$version"
release_flags=(--latest)
if [[ $version == *-nightly.* ]]; then
  release_flags=(--prerelease --latest=false)
fi
repo=bryantleft/bruhs-ai

[[ -z $(git status --porcelain --untracked-files=no) ]] || { echo "commit or stash changes first" >&2; exit 1; }
git fetch -q origin main
[[ $(git rev-parse HEAD) == $(git rev-parse origin/main) ]] || { echo "release origin/main: push it first, or check it out (HEAD differs)" >&2; exit 1; }

# The release notes: CHANGELOG.md's section for this version, without its heading.
notes=$(awk -v v="$version" '
  $0 ~ "^## \\[" v "\\] - [0-9]{4}-[0-9]{2}-[0-9]{2}$" { on = 1; next }
  on && (/^## \[/ || /^\[[^]]+\]: /) { exit }
  on { print }
' CHANGELOG.md | sed -e '/./,$!d' | awk '
  # GitHub shows every newline in release notes, so join wrapped lines:
  # a continuation (indented, or a plain line after text) joins the line above.
  /^$/ { if (buf != "") print buf; print ""; buf = ""; next }
  /^(#|- )/ { if (buf != "") print buf; buf = $0; next }
  { sub(/^ +/, ""); buf = (buf == "") ? $0 : buf " " $0 }
  END { if (buf != "") print buf }
')
[[ -n $notes ]] || { echo "CHANGELOG.md has no dated '## [$version] - YYYY-MM-DD' section" >&2; exit 1; }
node bruhs-site/src/lib/changelog.js --check || exit 1
if git rev-parse -q --verify "refs/tags/$tag" >/dev/null || git ls-remote --exit-code --tags origin "$tag" >/dev/null 2>&1; then
  echo "$tag already exists" >&2; exit 1
fi
gh auth status >/dev/null || { echo "gh is not signed in" >&2; exit 1; }

scripts/bundle-macos.sh "$version" --sign --notarize

gh run watch "$native_run" --repo "$repo" --exit-status
scripts/package-native.sh "$version" "$native_run"

arch=$(uname -m)
gh release create "$tag" \
  --repo "$repo" \
  --draft \
  --target "$(git rev-parse HEAD)" \
  --title "Bruhs $tag" \
  --notes "$notes

**Install:** On Windows x64, run \`Bruhs-Windows-x64-Setup.exe\` and launch Bruhs from Start. Install [WebView2](https://developer.microsoft.com/microsoft-edge/webview2/) and the [Visual C++ x64 runtime](https://aka.ms/vc14/vc_redist.x64.exe) if needed. On Ubuntu 24.04 x64, run \`sudo apt install ./Bruhs-Linux-x64.deb\` and launch Bruhs from Applications. On Mac, open \`Bruhs-macOS-$arch.dmg\` and drag Bruhs to Applications (macOS 13 or later, Apple silicon). Installed Mac copies update themselves from Settings → Updates. Every release: https://bruhs.ai/changelog" \
  "dist/Bruhs-macOS-$arch.dmg" \
  "dist/Bruhs-macOS-$arch.zip" \
  dist/Bruhs-Windows-x64-Setup.exe dist/Bruhs-Linux-x64.deb \
  dist/SHA256SUMS
gh release edit "$tag" --repo "$repo" --draft=false "${release_flags[@]}"
git fetch -q origin "refs/tags/$tag:refs/tags/$tag"
echo "released https://github.com/$repo/releases/tag/$tag"

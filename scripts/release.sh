#!/usr/bin/env bash
# Publish a Bruhs release: signed + notarized Bruhs.app as a GitHub Release.
#
#   scripts/release.sh 0.1.0
#
# Runs from a clean `main` that the gate passed on. Builds with
# scripts/bundle-macos.sh --sign --notarize, uploads the disk image, the
# update zip and SHA256SUMS to a draft release, then publishes it, which
# creates the v<version> tag at this commit. Nothing is public until every
# asset is up, so installed apps never see a release without its zip.
# Installed apps find it through GitHub's releases/latest API; the site's
# Download button links to releases/latest/download/Bruhs-macOS-<arch>.dmg.
set -euo pipefail
cd "$(dirname "$0")/.."

version=${1:-}
[[ $version =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || { echo "usage: scripts/release.sh <version like 0.1.0>" >&2; exit 2; }
tag="v$version"
repo=bryantleft/bruhs-ai

[[ $(git rev-parse --abbrev-ref HEAD) == main ]] || { echo "release from main" >&2; exit 1; }
[[ -z $(git status --porcelain --untracked-files=no) ]] || { echo "commit or stash changes first" >&2; exit 1; }
git fetch -q origin main
[[ $(git rev-parse HEAD) == $(git rev-parse origin/main) ]] || { echo "push main first (HEAD differs from origin/main)" >&2; exit 1; }
if git rev-parse -q --verify "refs/tags/$tag" >/dev/null || git ls-remote --exit-code --tags origin "$tag" >/dev/null 2>&1; then
  echo "$tag already exists" >&2; exit 1
fi
gh auth status >/dev/null || { echo "gh is not signed in" >&2; exit 1; }

scripts/bundle-macos.sh "$version" --sign --notarize

arch=$(uname -m)
gh release create "$tag" \
  --repo "$repo" \
  --draft \
  --target "$(git rev-parse HEAD)" \
  --title "Bruhs $tag" \
  --generate-notes \
  "dist/Bruhs-macOS-$arch.dmg" \
  "dist/Bruhs-macOS-$arch.zip" \
  dist/SHA256SUMS
gh release edit "$tag" --repo "$repo" --draft=false --latest
git fetch -q origin "refs/tags/$tag:refs/tags/$tag"
echo "released https://github.com/$repo/releases/tag/$tag"

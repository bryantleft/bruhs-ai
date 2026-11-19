#!/usr/bin/env bash
# Sign the three staged Windows executables using the release operator's Azure login.
set -euo pipefail
bin=${1:?pass the directory containing the three Windows release binaries}
: "${BRUHS_SIGNING_ENDPOINT:?set to the regional Azure Artifact Signing hostname}"
: "${BRUHS_SIGNING_ACCOUNT:?set to the Azure Artifact Signing account name}"
: "${BRUHS_SIGNING_PROFILE:?set to the Public Trust certificate profile name}"
[[ "$BRUHS_SIGNING_ENDPOINT" =~ ^[a-z0-9]+\.codesigning\.azure\.net$ ]] || { echo "Invalid Azure signing hostname" >&2; exit 1; }

files=("$bin/bruhs-desktop.exe" "$bin/bruhs-server.exe" "$bin/bruhs-capture.exe")
for file in "${files[@]}"; do test -s "$file" || { echo "Missing Windows release binary: $file" >&2; exit 1; }; done
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
jar="$work/jsign-7.6.jar"
curl --fail --silent --show-error --location --retry 2 --output "$jar" https://github.com/ebourg/jsign/releases/download/7.6/jsign-7.6.jar
printf '%s  %s\n' '6cf58aa080e7ff82656f320cff8d4a3bff30408df8766806dcba3740dd839fa5' "$jar" | shasum -a 256 -c -
token=$(az account get-access-token --resource https://codesigning.azure.net --query accessToken --output tsv --only-show-errors)
BRUHS_SIGNING_TOKEN="$token" "${JAVA_HOME:+$JAVA_HOME/bin/}java" -jar "$jar" \
  --storetype TRUSTEDSIGNING --keystore "$BRUHS_SIGNING_ENDPOINT" --storepass env:BRUHS_SIGNING_TOKEN \
  --alias "$BRUHS_SIGNING_ACCOUNT/$BRUHS_SIGNING_PROFILE" --alg SHA-256 --tsmode RFC3161 \
  --tsaurl http://timestamp.acs.microsoft.com --name Bruhs --url https://bruhs.ai "${files[@]}"

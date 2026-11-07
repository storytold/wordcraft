#!/usr/bin/env bash
# Build an ad-hoc signed app for this Mac, without a release, DMG, or signing account.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"
PROFILE="${WORDCRAFT_LOCAL_PROFILE:-dev}"
case "$PROFILE" in
  dev) BIN_DIR=debug ;;
  release) BIN_DIR=release ;;
  *) echo "WORDCRAFT_LOCAL_PROFILE must be dev or release" >&2; exit 2 ;;
esac
cargo build --locked -p wordcraft --profile "$PROFILE"
APP="$ROOT/dist/WordCraft Mac.app"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp "${CARGO_TARGET_DIR:-target}/$BIN_DIR/wordcraft" "$APP/Contents/MacOS/WordCraft"
cp assets/app-icon/wordcraft.icns "$APP/Contents/Resources/WordCraft.icns"
VERSION="$(awk '/^\[workspace.package\]/{p=1;next} /^\[/{p=0} p && /^version =/{gsub(/"/, "", $3);print $3;exit}' Cargo.toml)"
SHA="$(git rev-parse HEAD)"
sed -e "s/@VERSION@/$VERSION/g" -e "s/@SHORT_VERSION@/$VERSION/g" -e "s/@BUILD_SHA@/$SHA/g" \
  -e 's/ai.storyteller.wordcraft/com.opportunisticlabs.wordcraft.mac-native/g' \
  packaging/macos/Info.plist.in > "$APP/Contents/Info.plist"
for doc in LICENSE-MIT LICENSE-APACHE NOTICE ATTRIBUTION.md; do
  cp "$doc" "$APP/Contents/Resources/"
done
codesign --force --sign - "$APP"
codesign --verify --strict "$APP"
echo "$APP"

#!/usr/bin/env bash
# Build WordCraft for iPad and package it as an IPA (macOS with Xcode only):
#
#   $DIST/wordcraft-<version>-ipados.ipa    Payload/WordCraft.app, ad-hoc signed
#
# The IPA is for sideloading: AltStore, Sideloadly or Xcode re-sign it with the installer's own
# Apple ID (packaging/ios/README.md). It is not an App Store build.
#
# Usage: packaging/ios/package.sh [--skip-build]
#
# Env:
#   IOS_SIGN_IDENTITY   codesign identity; default "-" (ad-hoc, for a sideloading tool to re-sign)
#   CRAFT_FONTS_DIR, CRAFT_I18N_DIR   as for the desktop builds: embedded fonts and translations
set -euo pipefail
# shellcheck source=../env.sh
. "$(dirname "${BASH_SOURCE[0]}")/../env.sh"
HERE="$ROOT/packaging/ios"

SKIP_BUILD=0
while [[ $# -gt 0 ]]; do
  case "$1" in
    --skip-build) SKIP_BUILD=1; shift ;;
    -h | --help) sed -n '2,14p' "$0"; exit 0 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done

TARGET=aarch64-apple-ios
# Keep in sync with MinimumOSVersion in Info.plist.in. iPadOS 16 runs on every iPad with a
# trackpad-capable keyboard and has the pointer and key-press APIs the input code uses.
export IPHONEOS_DEPLOYMENT_TARGET=16.0
IDENTITY="${IOS_SIGN_IDENTITY:--}"
SHORT_VERSION="${VERSION%%-*}"
WORK="$CARGO_TARGET_DIR/ios-package"
APP="$WORK/Payload/WordCraft.app"
IPA="$DIST/wordcraft-$VERSION-ipados.ipa"
PATCHES="$CARGO_TARGET_DIR/ios-src/patch.toml"

echo "==> WordCraft $VERSION for iPadOS ($TARGET), identity: $IDENTITY"

# ---- build -------------------------------------------------------------------------------------
if [[ "$SKIP_BUILD" = 0 ]]; then
  "$HERE/prepare-sources.sh"
  rustup target add "$TARGET"
  # Not --locked: the patched winit and rfd are path sources, which the lock file doesn't name.
  (cd "$ROOT" && cargo build --release -p wordcraft --target "$TARGET" --config "$PATCHES")
fi
BIN="$CARGO_TARGET_DIR/$TARGET/release/wordcraft"
[[ -x "$BIN" ]] || { echo "error: $BIN was not built" >&2; exit 1; }

# ---- WordCraft.app -----------------------------------------------------------------------------
echo "==> assembling $APP"
rm -rf "$WORK"
mkdir -p "$APP"
cp "$BIN" "$APP/WordCraft"
copy_docs "$APP"
printf 'APPL????' >"$APP/PkgInfo"

# The app icon: an asset catalog compiled by actool, which also names the icon in Info.plist.
CATALOG="$WORK/Assets.xcassets"
mkdir -p "$CATALOG/AppIcon.appiconset"
cp "$ROOT/assets/app-icon/wordcraft-ios-1024.png" "$CATALOG/AppIcon.appiconset/icon-1024.png"
cat >"$CATALOG/Contents.json" <<'JSON'
{ "info": { "author": "xcode", "version": 1 } }
JSON
cat >"$CATALOG/AppIcon.appiconset/Contents.json" <<'JSON'
{
  "images": [{ "filename": "icon-1024.png", "idiom": "universal", "platform": "ios", "size": "1024x1024" }],
  "info": { "author": "xcode", "version": 1 }
}
JSON
xcrun actool "$CATALOG" --compile "$APP" --platform iphoneos --target-device ipad \
  --minimum-deployment-target "$IPHONEOS_DEPLOYMENT_TARGET" --app-icon AppIcon \
  --output-partial-info-plist "$WORK/icons.plist" --output-format human-readable-text

SDK_VERSION="$(xcrun --sdk iphoneos --show-sdk-version)"
SDK_BUILD="$(xcrun --sdk iphoneos --show-sdk-build-version)"
XCODE_VERSION_FULL="$(xcodebuild -version | awk '/^Xcode/ { print $2 }')"
XCODE_BUILD="$(xcodebuild -version | awk '/^Build version/ { print $3 }')"
# DTXcode is the version as four digits: Xcode 16.2 is 1620.
IFS=. read -r XC_MAJOR XC_MINOR XC_PATCH <<<"$XCODE_VERSION_FULL"
XCODE_VERSION="$(printf '%02d%d%d' "$XC_MAJOR" "${XC_MINOR:-0}" "${XC_PATCH:-0}")"
sed -e "s/@VERSION@/$VERSION/g" -e "s/@SHORT_VERSION@/$SHORT_VERSION/g" \
  -e "s/@BUILD_SHA@/${WORDCRAFT_BUILD_SHA:-unknown}/g" -e "s/@MIN_IOS@/$IPHONEOS_DEPLOYMENT_TARGET/g" \
  -e "s/@SDK_VERSION@/$SDK_VERSION/g" -e "s/@SDK_BUILD@/$SDK_BUILD/g" \
  -e "s/@XCODE_VERSION@/$XCODE_VERSION/g" -e "s/@XCODE_BUILD@/$XCODE_BUILD/g" \
  "$HERE/Info.plist.in" >"$APP/Info.plist"
/usr/libexec/PlistBuddy -c "Merge $WORK/icons.plist" "$APP/Info.plist"
plutil -lint "$APP/Info.plist"
plutil -convert binary1 "$APP/Info.plist"

xattr -cr "$APP"
codesign --force --sign "$IDENTITY" --timestamp=none "$APP"
codesign --verify --verbose=2 "$APP"

# ---- IPA ---------------------------------------------------------------------------------------
echo "==> building $IPA"
rm -f "$IPA"
(cd "$WORK" && zip -qry -X "$IPA" Payload)
ls -l "$IPA"

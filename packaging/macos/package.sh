#!/usr/bin/env bash
# Build, sign and (optionally) notarize the macOS release artifacts:
#
#   $DIST/wordcraft-<version>-macos-<arch>.dmg          WordCraft.app on a drag-to-Applications DMG
#   $DIST/wordcraft-cli-<version>-macos-<arch>.zip      the headless CLI
#
# Usage: packaging/macos/package.sh [--arch universal|aarch64|x86_64] [--skip-build]
#
# Signing (env):
#   MACOS_SIGN_IDENTITY   codesign identity (name or SHA-1). Default "-" = ad-hoc (local testing;
#                         Gatekeeper will reject the result on other Macs). In CI, import-cert.sh sets it.
#   MACOS_KEYCHAIN        keychain holding the identity (optional)
# Notarization (env; all three needed, and a real identity):
#   APPLE_ID, APPLE_PASSWORD (app-specific password), APPLE_TEAM_ID
set -euo pipefail
# shellcheck source=../env.sh
. "$(dirname "${BASH_SOURCE[0]}")/../env.sh"
HERE="$ROOT/packaging/macos"

ARCH=universal
SKIP_BUILD=0
while [[ $# -gt 0 ]]; do
  case "$1" in
    --arch) ARCH="$2"; shift 2 ;;
    --skip-build) SKIP_BUILD=1; shift ;;
    -h | --help) sed -n '2,16p' "$0"; exit 0 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done
case "$ARCH" in
  universal) TARGETS=(aarch64-apple-darwin x86_64-apple-darwin) ;;
  aarch64) TARGETS=(aarch64-apple-darwin) ;;
  x86_64) TARGETS=(x86_64-apple-darwin) ;;
  *) echo "unknown --arch $ARCH" >&2; exit 2 ;;
esac

# Keep in sync with LSMinimumSystemVersion in Info.plist.in.
export MACOSX_DEPLOYMENT_TARGET=11.0
IDENTITY="${MACOS_SIGN_IDENTITY:--}"
SHORT_VERSION="${VERSION%%-*}"
WORK="$CARGO_TARGET_DIR/macos-package"
APP="$WORK/WordCraft.app"
DMG="$DIST/wordcraft-$VERSION-macos-$ARCH.dmg"
CLI_ZIP="$DIST/wordcraft-cli-$VERSION-macos-$ARCH.zip"

NOTARIZE=0
if [[ "$IDENTITY" = "-" ]]; then
  warn "macOS: ad-hoc signing (no MACOS_SIGN_IDENTITY); artifacts are not notarized"
elif [[ -n "${APPLE_ID:-}" ]] && [[ -n "${APPLE_PASSWORD:-}" ]] && [[ -n "${APPLE_TEAM_ID:-}" ]]; then
  NOTARIZE=1
else
  warn "macOS: APPLE_ID / APPLE_PASSWORD / APPLE_TEAM_ID incomplete; signed but not notarized"
fi

echo "==> WordCraft $VERSION for macOS ($ARCH), identity: $IDENTITY, notarize: $NOTARIZE"

# ---- build -------------------------------------------------------------------------------------
if [[ "$SKIP_BUILD" = 0 ]]; then
  args=()
  for t in "${TARGETS[@]}"; do args+=(--target "$t"); done
  (cd "$ROOT" && cargo build --release --locked -p wordcraft -p wordcraft-cli "${args[@]}")
fi

rm -rf "$WORK"
mkdir -p "$WORK/bin"
for bin in wordcraft wordcraft-cli; do
  inputs=()
  for t in "${TARGETS[@]}"; do inputs+=("$CARGO_TARGET_DIR/$t/release/$bin"); done
  lipo -create -output "$WORK/bin/$bin" "${inputs[@]}"
  lipo -info "$WORK/bin/$bin"
done

# ---- signing helpers ---------------------------------------------------------------------------
sign() {
  # Hardened runtime + secure timestamp for a real identity; ad-hoc can't be timestamped.
  local ts=(--timestamp)
  [[ "$IDENTITY" = "-" ]] && ts=(--timestamp=none)
  local kc=()
  [[ -n "${MACOS_KEYCHAIN:-}" ]] && kc=(--keychain "$MACOS_KEYCHAIN")
  codesign --force --sign "$IDENTITY" ${kc[@]+"${kc[@]}"} "${ts[@]}" "$@"
}

notarize() {
  local file="$1" out id status
  echo "==> notarizing $(basename "$file") (this can take a few minutes)"
  out="$(xcrun notarytool submit "$file" --apple-id "$APPLE_ID" --password "$APPLE_PASSWORD" \
    --team-id "$APPLE_TEAM_ID" --wait --timeout 1h --output-format json)" || true
  echo "$out"
  id="$(printf '%s' "$out" | plutil -extract id raw -o - - 2>/dev/null || true)"
  status="$(printf '%s' "$out" | plutil -extract status raw -o - - 2>/dev/null || true)"
  if [[ "$status" != "Accepted" ]]; then
    if [[ -n "$id" ]]; then
      xcrun notarytool log "$id" --apple-id "$APPLE_ID" --password "$APPLE_PASSWORD" --team-id "$APPLE_TEAM_ID" || true
    fi
    echo "error: notarization of $(basename "$file") failed (status: ${status:-unknown})" >&2
    exit 1
  fi
}

# ---- WordCraft.app ----------------------------------------------------------------------------
echo "==> assembling $APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
# Executable and icon carry the display name (CFBundleExecutable / CFBundleIconFile).
cp "$WORK/bin/wordcraft" "$APP/Contents/MacOS/WordCraft"
cp "$ROOT/assets/app-icon/wordcraft.icns" "$APP/Contents/Resources/WordCraft.icns"
# Licences of the embedded craft-fonts fonts (only when built with CRAFT_FONTS_DIR).
copy_docs "$APP/Contents/Resources"
sed -e "s/@VERSION@/$VERSION/g" -e "s/@SHORT_VERSION@/$SHORT_VERSION/g" \
  -e "s/@BUILD_SHA@/${WORDCRAFT_BUILD_SHA:-unknown}/g" \
  "$HERE/Info.plist.in" >"$APP/Contents/Info.plist"
plutil -lint "$APP/Contents/Info.plist"
printf 'APPL????' >"$APP/Contents/PkgInfo"

# codesign --strict rejects extended attributes such as com.apple.FinderInfo or a resource fork
# ("Finder information, or similar detritus not allowed"); a source file copied in can carry them.
# Clear them before signing; nothing after this point may write xattrs onto the bundle (#525).
xattr -cr "$APP"
# Sign inside-out: nested code first, then the bundle itself (no --deep on the final signature).
# Today the only nested code is the main executable; frameworks/helpers would be signed here too.
sign --options runtime --entitlements "$HERE/entitlements.plist" "$APP/Contents/MacOS/WordCraft"
sign --options runtime --entitlements "$HERE/entitlements.plist" "$APP"
codesign --verify --strict --deep --verbose=2 "$APP"

if [[ "$NOTARIZE" = 1 ]]; then
  ditto -c -k --keepParent "$APP" "$WORK/WordCraft-notarize.zip"
  notarize "$WORK/WordCraft-notarize.zip"
  xcrun stapler staple "$APP"
  xcrun stapler validate "$APP"
  spctl --assess --type execute -vvv "$APP"
fi

# ---- DMG ---------------------------------------------------------------------------------------
echo "==> building $DMG"
STAGE="$WORK/dmg"
mkdir -p "$STAGE"
ditto "$APP" "$STAGE/WordCraft.app"
ln -s /Applications "$STAGE/Applications"
# Finder window layout: background, icon size and positions (packaging/macos/dmg/README.md).
mkdir -p "$STAGE/.background"
cp "$HERE/dmg/background.tiff" "$STAGE/.background/background.tiff"
cp "$HERE/dmg/dmg-layout.DS_Store" "$STAGE/.DS_Store"
rm -f "$DMG"
# Retries an hdiutil verb that attaches a device: on CI runners it fails now and then with
# "Resource busy". Usage: hdiutil_retry <verb> <args...>; five tries with growing pauses.
hdiutil_retry() {
  local verb="$1"
  local attempt
  for attempt in 1 2 3 4 5; do
    if hdiutil "$@"; then
      return 0
    fi
    if [[ "$attempt" = 5 ]]; then
      echo "error: hdiutil $verb failed 5 times" >&2
      return 1
    fi
    warn "hdiutil $verb failed (attempt $attempt of 5); retrying in $((attempt * 10))s"
    sleep $((attempt * 10))
  done
  return 1
}
# `hdiutil create -srcfolder` copies the staged files as they are. The previous
# `makehybrid -hfs` + `convert` gave every file and folder in the image a com.apple.FinderInfo
# (Finder location -1,-1), so the signed app failed `codesign --verify --deep --strict` inside the
# mounted DMG and after copying it out (#525). The volume name has no version: .DS_Store finds the
# background through an alias that includes it.
hdiutil_retry create -srcfolder "$STAGE" -volname "WordCraft" -fs HFS+ -format UDZO \
  -imagekey zlib-level=9 -ov "$DMG"
sign "$DMG"
codesign --verify --strict --verbose=2 "$DMG"

# Gate: the app inside the finished image, and a copy installed from it with ditto, must pass the
# same strict check as the app before packaging, so a container signature can't hide a damaged app
# signature (#525). Returns non-zero (set -e then stops the build) after detaching the image.
verify_dmg_app() {
  local dmg="$1" mount="$WORK/dmg-check" copy="$WORK/dmg-check-copy" rc=0
  mkdir -p "$mount" "$copy"
  hdiutil_retry attach -readonly -nobrowse -noautoopen -mountpoint "$mount" "$dmg" >/dev/null
  echo "==> strict signature check of WordCraft.app inside $(basename "$dmg")"
  codesign --verify --deep --strict --verbose=2 "$mount/WordCraft.app" || rc=1
  if [[ "$rc" = 0 ]]; then
    ditto "$mount/WordCraft.app" "$copy/WordCraft.app" || rc=1
  fi
  if [[ "$rc" = 0 ]]; then
    echo "==> strict signature check of a ditto copy of it"
    codesign --verify --deep --strict --verbose=2 "$copy/WordCraft.app" || rc=1
  fi
  if [[ "$rc" = 0 ]] && [[ "$NOTARIZE" = 1 ]]; then
    spctl --assess --type execute -vvv "$copy/WordCraft.app" || rc=1
  fi
  if [[ "$rc" != 0 ]]; then
    echo "error: WordCraft.app in $(basename "$dmg") fails strict verification; extended attributes:" >&2
    xattr -lr "$mount/WordCraft.app" >&2 || true
  fi
  hdiutil_retry detach "$mount" >/dev/null || hdiutil detach "$mount" -force >/dev/null || rc=1
  rm -rf "$copy"
  return "$rc"
}
verify_dmg_app "$DMG"
if [[ "$NOTARIZE" = 1 ]]; then
  notarize "$DMG"
  xcrun stapler staple "$DMG"
  xcrun stapler validate "$DMG"
  spctl --assess --type open --context context:primary-signature -vvv "$DMG"
fi

# ---- CLI ---------------------------------------------------------------------------------------
echo "==> building $CLI_ZIP"
CLI_DIR="$WORK/wordcraft-cli-$VERSION-macos-$ARCH"
mkdir -p "$CLI_DIR"
cp "$WORK/bin/wordcraft-cli" "$CLI_DIR/"
copy_docs "$CLI_DIR"
sign --options runtime "$CLI_DIR/wordcraft-cli"
codesign --verify --strict --verbose=2 "$CLI_DIR/wordcraft-cli"
rm -f "$CLI_ZIP"
ditto -c -k --norsrc --noextattr --keepParent "$CLI_DIR" "$CLI_ZIP"
# A bare Mach-O can't carry a stapled ticket; Gatekeeper looks the notarization up online.
if [[ "$NOTARIZE" = 1 ]]; then notarize "$CLI_ZIP"; fi

"$WORK/bin/wordcraft-cli" --version
echo "==> done"
ls -lh "$DMG" "$CLI_ZIP"

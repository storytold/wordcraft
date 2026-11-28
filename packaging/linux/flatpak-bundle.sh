#!/usr/bin/env bash
# Repackage a WordCraft Linux tarball as a single-file Flatpak bundle:
#
#   $DIST/wordcraft-<version>-linux-<arch>.flatpak
#
# Usage: packaging/linux/flatpak-bundle.sh [--no-test] [TARBALL]
#
# TARBALL defaults to $DIST/wordcraft-<version>-linux-<arch>.tar.gz from package.sh. The bundle
# is built for the host architecture (x86_64 or aarch64), which must match the tarball's.
# Needs flatpak, flatpak-builder and the SVG pixbuf loader (librsvg2-common) for the host's
# `appstreamcli compose`; the freedesktop runtime and SDK named in the manifest are
# installed per-user from Flathub. Unless --no-test, the bundle is then installed per-user and
# `wordcraft-cli --version` is run inside the sandbox as a smoke test.
# Manifest: packaging/linux/flatpak/ai.storyteller.wordcraft.bundle.yml.
set -euo pipefail
# shellcheck source=../env.sh
. "$(dirname "${BASH_SOURCE[0]}")/../env.sh"
HERE="$ROOT/packaging/linux"
APP_ID=ai.storyteller.wordcraft
FLATPAK_ID="$APP_ID"
# The remote indexes both app IDs on the stable ref branch; IDs isolate installations.
FLATPAK_BRANCH=stable
if [[ "${WORDCRAFT_BUILD_CHANNEL:-}" == nightly-v1 || "$VERSION" == *-nightly* ]]; then
  FLATPAK_ID="$APP_ID.nightly"
fi
FLATHUB=https://dl.flathub.org/repo/flathub.flatpakrepo

TEST=1
OCI=0
TARBALL=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --no-test) TEST=0; shift ;;
    --oci) OCI=1; shift ;;
    -h | --help) sed -n '2,14p' "$0"; exit 0 ;;
    -*) echo "unknown argument: $1" >&2; exit 2 ;;
    *) TARBALL="$1"; shift ;;
  esac
done

ARCH="$(uname -m)"
case "$ARCH" in
  x86_64) ;;
  aarch64 | arm64) ARCH=aarch64 ;;
  *) echo "unsupported architecture $ARCH" >&2; exit 2 ;;
esac
BASENAME_VERSION="$VERSION"
if [[ "$FLATPAK_ID" != "$APP_ID" && "$BASENAME_VERSION" != *-nightly* ]]; then
  BASENAME_VERSION="$BASENAME_VERSION-nightly"
fi
BASENAME="wordcraft-$BASENAME_VERSION-linux-$ARCH"
TARBALL="${TARBALL:-$DIST/$BASENAME.tar.gz}"
[[ -f "$TARBALL" ]] || { echo "error: $TARBALL not found (run packaging/linux/package.sh --formats tar first)" >&2; exit 1; }
for tool in flatpak flatpak-builder; do
  command -v "$tool" >/dev/null || { echo "error: $tool not found (apt install flatpak flatpak-builder librsvg2-common)" >&2; exit 1; }
done

echo "==> WordCraft $VERSION Flatpak bundle for $ARCH from $(basename "$TARBALL")"

WORK="$CARGO_TARGET_DIR/flatpak-bundle"
rm -rf "$WORK"
mkdir -p "$WORK/stage"
tar -xzf "$TARBALL" -C "$WORK/stage" --strip-components=1
[[ -x "$WORK/stage/bin/wordcraft" ]] || { echo "error: $TARBALL has no bin/wordcraft" >&2; exit 1; }
if [[ "$FLATPAK_ID" != "$APP_ID" ]]; then
  sed -e "s|^Icon=$APP_ID$|Icon=$FLATPAK_ID|" \
      -e "s|^Name=WordCraft$|Name=WordCraft Nightly|" \
      -e 's|^StartupWMClass=.*|StartupWMClass=ai.storyteller.wordcraft.nightly-v1|' \
      -e '/^MimeType=/d' \
      "$WORK/stage/share/applications/$APP_ID.desktop" >"$WORK/stage/share/applications/$FLATPAK_ID.desktop"
  rm -f "$WORK/stage/share/applications/$APP_ID.desktop"
  sed -e "s|<id>$APP_ID</id>|<id>$FLATPAK_ID</id>|g" \
      -e "s|<name>WordCraft</name>|<name>WordCraft Nightly</name>|g" \
      -e "s|<launchable type=\"desktop-id\">$APP_ID.desktop</launchable>|<launchable type=\"desktop-id\">$FLATPAK_ID.desktop</launchable>|g" \
      -e "s|<icon type=\"stock\">$APP_ID</icon>|<icon type=\"stock\">$FLATPAK_ID</icon>|g" \
      -e '/<mediatype>/d' \
      "$WORK/stage/share/metainfo/$APP_ID.metainfo.xml" >"$WORK/stage/share/metainfo/$FLATPAK_ID.metainfo.xml"
  rm -f "$WORK/stage/share/metainfo/$APP_ID.metainfo.xml"
  mv "$WORK/stage/share/mime/packages/$APP_ID.xml" "$WORK/stage/share/mime/packages/$FLATPAK_ID.xml"
  rm -rf "$WORK/stage/share/icons/hicolor"
  NIGHTLY_ICON="$ROOT/assets/app-icon/wordcraft-nightly.png"
  [[ -f "$NIGHTLY_ICON" ]] || NIGHTLY_ICON="$ROOT/assets/app-icon/hicolor/256x256/apps/$APP_ID.png"
  install -Dm644 "$NIGHTLY_ICON" \
    "$WORK/stage/share/icons/hicolor/256x256/apps/$FLATPAK_ID.png"
  # The nightly tar carries the launcher in bin/wordcraft and the GUI in lib/wordcraft-nightly/;
  # the bundle must install both. The launcher keeps the profile under XDG_DATA_HOME, which inside
  # the sandbox is this app's own persistent data directory
  # (~/.var/app/ai.storyteller.wordcraft.nightly/data), not the throwaway sandbox home.
  sed -e "s|ai\.storyteller\.wordcraft|$FLATPAK_ID|g" \
      -e 's|^\(      - install -Dm755 bin/wordcraft /app/bin/wordcraft\)$|\1\n      - install -Dm755 lib/wordcraft-nightly/wordcraft /app/lib/wordcraft-nightly/wordcraft|' \
      "$HERE/flatpak/$APP_ID.bundle.yml" >"$WORK/$FLATPAK_ID.yml"
  grep -qx '      - install -Dm755 lib/wordcraft-nightly/wordcraft /app/lib/wordcraft-nightly/wordcraft' \
    "$WORK/$FLATPAK_ID.yml" || { echo "error: the nightly manifest did not take the launcher's binary" >&2; exit 1; }
else
  cp "$HERE/flatpak/$APP_ID.bundle.yml" "$WORK/$FLATPAK_ID.yml"
fi

flatpak remote-add --user --if-not-exists flathub "$FLATHUB"
# --disable-rofiles-fuse: no FUSE in containers and CI runners.
flatpak-builder --user --install-deps-from=flathub --disable-rofiles-fuse --force-clean \
  --default-branch="$FLATPAK_BRANCH" --repo="$WORK/repo" --state-dir="$WORK/state" "$WORK/build" "$WORK/$FLATPAK_ID.yml"

OUT="$DIST/$BASENAME.flatpak"
# --runtime-repo lets `flatpak install` fetch the freedesktop runtime from Flathub if missing.
flatpak build-bundle --runtime-repo="$FLATHUB" "$WORK/repo" "$OUT" "$FLATPAK_ID" "$FLATPAK_BRANCH"
echo "wrote $OUT"
if [[ "$OCI" = 1 ]]; then
  OUT_OCI="$DIST/$BASENAME-oci"
  rm -rf "$OUT_OCI"
  flatpak build-bundle --oci "$WORK/repo" "$OUT_OCI" "$FLATPAK_ID" "$FLATPAK_BRANCH"
  echo "wrote OCI layout to $OUT_OCI"
fi

if [[ "$TEST" = 1 ]]; then
  flatpak install --user -y --noninteractive --reinstall "$OUT"
  flatpak info --user "$FLATPAK_ID"
  flatpak run --command=wordcraft-cli "$FLATPAK_ID" --version
  if [[ "$FLATPAK_ID" != "$APP_ID" ]]; then
    # The launcher and the GUI it starts are both installed, and the version query answers.
    flatpak run --command=wordcraft "$FLATPAK_ID" --version
  fi
  flatpak run --command=sh "$FLATPAK_ID" -c 'ls /app/share/applications /app/share/metainfo /app/share/mime/packages /app/share/icons/hicolor/*/apps'
fi
echo "==> done"
ls -lh "$OUT"

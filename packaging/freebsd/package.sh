#!/usr/bin/env bash
# Build and package WordCraft for FreeBSD:
#
#   $DIST/wordcraft-<version>-freebsd-<arch>.tar.gz   a /usr/local-style tree (<arch> is x86_64, or
#       aarch64 when CARGO_BUILD_TARGET=aarch64-unknown-freebsd):
#       wordcraft-<version>-freebsd-<arch>/{bin, share/applications, share/icons, share/mime,
#       share/metainfo, share/doc/wordcraft}
#
# Install by copying the tree's contents into /usr/local, e.g.
#   tar -xzf wordcraft-*-freebsd-x86_64.tar.gz --strip-components 1 -C /usr/local
#
# Usage: packaging/freebsd/package.sh [--skip-build] [--dry-run]
#   --skip-build  package the binaries already in CARGO_TARGET_DIR
#   --dry-run     print the resolved arch, target, build and archive name, then exit 0 without
#                 building, staging or writing anything
#
# Needs: bash, cargo and the build dependencies listed in .github/workflows/freebsd.yml. FreeBSD's
# install(1) has no -D, so directories are created first.
set -euo pipefail
# shellcheck source=../env.sh
. "$(dirname "${BASH_SOURCE[0]}")/../env.sh"
LINUX="$ROOT/packaging/linux"
APP_ID=ai.storyteller.wordcraft

SKIP_BUILD=0
DRY_RUN=0
while [[ $# -gt 0 ]]; do
  case "$1" in
    --skip-build) SKIP_BUILD=1; shift ;;
    --dry-run) DRY_RUN=1; shift ;;
    -h | --help) sed -n '2,15p' "$0"; exit 0 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done

# FreeBSD calls x86_64 "amd64"; release names use the Rust/Linux spelling.
release_arch() {
  case "$1" in
    amd64 | x86_64) echo x86_64 ;;
    arm64 | aarch64) echo aarch64 ;;
    *) echo "unsupported architecture $1" >&2; return 2 ;;
  esac
}
# The release workflow cross-compiles aarch64 in an x86_64 VM with CARGO_BUILD_TARGET set to
# aarch64-unknown-freebsd (craft-common-actions#39): name the archive after the target.
# host-tuple means the build host's triple, as rustc names it.
if [[ "${CARGO_BUILD_TARGET:-}" == host-tuple ]]; then
  CARGO_BUILD_TARGET="$(rustc -vV | sed -n 's/^host: //p')"
fi
case "${CARGO_BUILD_TARGET:-}" in
  "") ARCH="$(release_arch "$(uname -m)")" ;;
  aarch64-unknown-freebsd | x86_64-unknown-freebsd) ARCH="$(release_arch "${CARGO_BUILD_TARGET%%-*}")" ;;
  *) echo "CARGO_BUILD_TARGET=$CARGO_BUILD_TARGET is not a supported FreeBSD target" >&2; exit 2 ;;
esac
# Only binaries for this machine run here (the --version check at the end); a FreeBSD target built
# on another OS never does.
HOST_ARCH="$(release_arch "$(uname -m)" 2>/dev/null)" || HOST_ARCH="$(uname -m)"
if [ -n "${CARGO_BUILD_TARGET:-}" ] && [ "$(uname -s)" != FreeBSD ]; then
  HOST_ARCH="$(uname -s) $HOST_ARCH"
fi
BASENAME="wordcraft-$VERSION-freebsd-$ARCH"

echo "==> WordCraft $VERSION for FreeBSD $ARCH"

if [[ "$DRY_RUN" = 1 ]]; then
  echo "dry run: target=${CARGO_BUILD_TARGET:-native} arch=$ARCH"
  echo "dry run: cargo build --release --locked ${CARGO_BUILD_TARGET:+--target $CARGO_BUILD_TARGET} -p wordcraft -p wordcraft-cli"
  echo "dry run: would write $DIST/$BASENAME.tar.gz"
  exit 0
fi

if [[ "$SKIP_BUILD" = 0 ]]; then
  (cd "$ROOT" && cargo build --release --locked ${CARGO_BUILD_TARGET:+--target "$CARGO_BUILD_TARGET"} -p wordcraft -p wordcraft-cli)
fi
# Cargo puts a cross build under target/<triple>/release.
BIN="$CARGO_TARGET_DIR/${CARGO_BUILD_TARGET:+$CARGO_BUILD_TARGET/}release"
WORK="$CARGO_TARGET_DIR/freebsd-package"
STAGE="$WORK/$BASENAME"
rm -rf "$WORK"

# FreeBSD's install(1) has no -D: create the directories first.
mkdir -p "$STAGE/bin" "$STAGE/share/applications" "$STAGE/share/mime/packages" "$STAGE/share/metainfo" \
  "$STAGE/share/icons" "$STAGE/share/doc/wordcraft"
install -m 755 "$BIN/wordcraft" "$BIN/wordcraft-cli" "$STAGE/bin/"
strip "$STAGE/bin/wordcraft" "$STAGE/bin/wordcraft-cli" 2>/dev/null || true
# The desktop entry, MIME type and metainfo are the freedesktop files the Linux packages use.
install -m 644 "$LINUX/$APP_ID.desktop" "$STAGE/share/applications/$APP_ID.desktop"
install -m 644 "$LINUX/$APP_ID.mime.xml" "$STAGE/share/mime/packages/$APP_ID.xml"
sed -e "s/@VERSION@/$VERSION/g" -e "s/@DATE@/$WORDCRAFT_BUILD_DATE/g" \
  "$LINUX/$APP_ID.metainfo.xml.in" >"$STAGE/share/metainfo/$APP_ID.metainfo.xml"
cp -R "$ROOT/assets/app-icon/hicolor" "$STAGE/share/icons/"
copy_docs "$STAGE/share/doc/wordcraft"

# ---- .tar.gz -------------------------------------------------------------------------------------
mkdir -p "$DIST"
tar -C "$WORK" -czf "$DIST/$BASENAME.tar.gz" "$BASENAME"
echo "wrote $DIST/$BASENAME.tar.gz"

if [ -z "${CARGO_BUILD_TARGET:-}" ] && [ "$ARCH" = "$HOST_ARCH" ]; then
  "$STAGE/bin/wordcraft-cli" --version
else
  # They cannot run here, so check what they are instead: executables, not libraries or objects.
  # An older file(1) calls a PIE a shared object; its interpreter still marks it as a program.
  want="$ARCH"; [ "$ARCH" != x86_64 ] || want=x86-64
  for b in "$STAGE"/bin/*; do
    case "$(file -b "$b")" in
      ELF*" executable, "*"$want"*FreeBSD* | ELF*"$want"*FreeBSD*interpreter*) ;;
      *) echo "error: $b is not a FreeBSD $ARCH executable: $(file -b "$b")" >&2; exit 1 ;;
    esac
  done
  echo "skipping --version: a CARGO_BUILD_TARGET=$CARGO_BUILD_TARGET build does not run on this $HOST_ARCH machine"
fi
echo "==> done"
ls -lh "$DIST"

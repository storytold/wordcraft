#!/usr/bin/env bash
# App-owned GUI-only nightly tar: no FUSE/AppImage, CLI, association or install hooks.
set -euo pipefail
ARCH=
while [[ $# -gt 0 ]]; do
  case "$1" in
    --arch) [[ $# -ge 2 ]] || { echo 'missing --arch value' >&2; exit 2; }; ARCH="$2"; shift 2 ;;
    -h|--help) echo 'Usage: nightly.sh --arch x86_64'; exit 0 ;;
    *) echo "unsupported nightly argument: $1" >&2; exit 2 ;;
  esac
done
[[ "$ARCH" == x86_64 && "$(uname -s)" == Linux ]] || { echo 'only native Linux x86_64 nightly packaging is supported' >&2; exit 2; }
# shellcheck source=../nightly-common.sh
. "$(dirname "${BASH_SOURCE[0]}")/../nightly-common.sh"
nightly_init linux "$ARCH" x86_64-unknown-linux-gnu
OUTPUT="$DIST/wordcraft-$VERSION-linux-x86_64-nightly.tar.gz"
[[ ! -e "$OUTPUT" && ! -L "$OUTPUT" ]] || nightly_die 'refusing to replace an existing nightly archive'
nightly_build_gui
STAGE="$NIGHTLY_WORK/stage"
mkdir -p "$STAGE"
# An unpacked GUI tar is isolated too: bin/wordcraft is a launcher that passes the Nightly
# channel's launch profile (the one place its user state lives) and the real binary sits beside
# it. Redirecting XDG_* instead is refused by the build's root validator.
cp "$NIGHTLY_GUI" "$STAGE/wordcraft.real"
chmod 755 "$STAGE/wordcraft.real"
cat >"$STAGE/wordcraft" <<'LAUNCHER'
#!/bin/sh
set -eu
real_bin="$(dirname "$0")/wordcraft.real"
if [ "${1:-}" = "--version" ] || [ "${1:-}" = "-V" ]; then
  exec "$real_bin" "$@"
fi
exec "$real_bin" --launch-profile nightly-v1 --profile-root "${HOME:?HOME is required}/.local/share/wordcraft-nightly" -- "$@"
LAUNCHER
chmod 755 "$STAGE/wordcraft"
NIGHTLY_ICON="$ROOT/assets/app-icon/wordcraft-nightly.png"
[[ -f "$NIGHTLY_ICON" ]] || NIGHTLY_ICON="$ROOT/assets/app-icon/wordcraft-1024.png"
cp "$NIGHTLY_ICON" "$STAGE/wordcraft-nightly.png"
nightly_copy_notices "$STAGE"
# The private mktemp cwd is the helper authority root, not a CLI path parent.
(cd "$NIGHTLY_WORK" && python3 "$ROOT/packaging/nightly-archive.py" linux stage archive.tar.gz)
nightly_publish_archive "$NIGHTLY_WORK/archive.tar.gz" "$OUTPUT"

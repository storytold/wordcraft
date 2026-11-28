#!/usr/bin/env bash
# Render the Nightly Linux tar exactly as package.sh does, without building or opening WordCraft,
# and run the launcher it installs. Guards the tar step's paths and the launcher's argument rules.
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/../.." && pwd)"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

export CARGO_TARGET_DIR="$work/target"
export DIST="$work/dist"
export WORDCRAFT_BUILD_CHANNEL=nightly-v1
export WORDCRAFT_VERSION="0.0.0-nightly.20260101.1.1.g0123456789ab"
export WORDCRAFT_BUILD_DATE="2026-01-01"
mkdir -p "$CARGO_TARGET_DIR/release" "$DIST"

# A local argv recorder stands in for the compiled binaries. It never opens a UI.
cat >"$CARGO_TARGET_DIR/release/wordcraft" <<'RECORDER'
#!/bin/sh
printf '<%s>\n' "$@"
RECORDER
cp "$CARGO_TARGET_DIR/release/wordcraft" "$CARGO_TARGET_DIR/release/wordcraft-cli"
chmod 755 "$CARGO_TARGET_DIR/release/wordcraft" "$CARGO_TARGET_DIR/release/wordcraft-cli"

# The tar format is first in package.sh's default set; a broken nightly tar aborted every run here.
(cd "$ROOT" && bash packaging/linux/package.sh --skip-build --formats tar)

base="wordcraft-$WORDCRAFT_VERSION-linux-$(uname -m)"
archive="$DIST/$base.tar.gz"
[ -f "$archive" ] || { echo "error: package.sh wrote no $archive" >&2; exit 1; }
tar -xzf "$archive" -C "$work"
tree="$work/$base"

# The FHS tree the desktop file and the Flatpak bundle expect: the launcher in bin/, the GUI in
# lib/wordcraft-nightly/.
[ -x "$tree/bin/wordcraft" ] && [ ! -L "$tree/bin/wordcraft" ] || { echo 'error: bin/wordcraft is not the launcher' >&2; exit 1; }
[ -x "$tree/lib/wordcraft-nightly/wordcraft" ] || { echo 'error: no GUI in lib/wordcraft-nightly' >&2; exit 1; }
cmp "$tree/lib/wordcraft-nightly/wordcraft" "$CARGO_TARGET_DIR/release/wordcraft"
cmp "$tree/bin/wordcraft" "$HERE/nightly-launcher.sh"

home="$work/home with space"
mkdir -p "$home"
expect() {
  expected="$1"
  shift
  actual=$(env -i HOME="$home" PATH="/usr/bin:/bin" "$tree/bin/wordcraft" "$@")
  if [ "$actual" != "$expected" ]; then
    printf 'expected:\n%s\nactual:\n%s\n' "$expected" "$actual" >&2
    exit 1
  fi
}
profile="$home/.local/share/wordcraft-nightly"
expect "$(printf '<%s>\n<%s>\n<%s>\n<%s>\n<%s>' \
  --launch-profile nightly-v1 --profile-root "$profile" --)"
expect "$(printf '<%s>\n<%s>\n<%s>\n<%s>\n<%s>\n<%s>\n<%s>' \
  --launch-profile nightly-v1 --profile-root "$profile" -- 'doc with spaces.docx' '--other-name.docx')" \
  'doc with spaces.docx' '--other-name.docx'
expect "$(printf '<%s>' --version)" --version
expect "$(printf '<%s>' -V)" -V
# The ArtCraft Launcher's own form passes through untouched, including the root it chose.
root="$work/launcher profile with space"
expect "$(printf '<%s>\n<%s>\n<%s>\n<%s>\n<%s>\n<%s>' \
  --launch-profile nightly-v1 --profile-root "$root" -- 'a.docx')" \
  --launch-profile nightly-v1 --profile-root "$root" -- 'a.docx'
expect "$(printf '<%s>\n<%s>\n<%s>\n<%s>\n<%s>' \
  --launch-profile nightly-v1 --profile-root "$root" --version)" \
  --launch-profile nightly-v1 --profile-root "$root" --version

# A data directory the person set themselves (and the Flatpak sandbox's own) keeps the profile
# beside the stable folder rather than on the throwaway home.
data="$work/xdg data with space"
mkdir -p "$data"
actual=$(env -i HOME="$home" XDG_DATA_HOME="$data" PATH="/usr/bin:/bin" "$tree/bin/wordcraft" --version)
[ "$actual" = '<--version>' ]
actual=$(env -i HOME="$home" XDG_DATA_HOME="$data" PATH="/usr/bin:/bin" "$tree/bin/wordcraft" 'x.docx')
[ "$actual" = "$(printf '<%s>\n<%s>\n<%s>\n<%s>\n<%s>\n<%s>' --launch-profile nightly-v1 --profile-root "$data/wordcraft-nightly" -- x.docx)" ]

# A launcher found through PATH still resolves its own folder, never the working directory.
mkdir -p "$work/elsewhere"
actual=$(cd "$work/elsewhere" && env -i HOME="$home" PATH="$tree/bin:/usr/bin:/bin" wordcraft --version)
[ "$actual" = '<--version>' ]

# The other two layouts the same file is installed in: the AppImage (AppRun beside usr/bin) and
# the app-owned nightly tar (wordcraft-bin beside the launcher).
mkdir -p "$work/AppDir/usr/bin" "$work/flat"
install -m 755 "$HERE/nightly-launcher.sh" "$work/AppDir/AppRun"
cp "$CARGO_TARGET_DIR/release/wordcraft" "$work/AppDir/usr/bin/wordcraft"
install -m 755 "$HERE/nightly-launcher.sh" "$work/flat/wordcraft"
cp "$CARGO_TARGET_DIR/release/wordcraft" "$work/flat/wordcraft-bin"
for layout in "$work/AppDir/AppRun" "$work/flat/wordcraft"; do
  actual=$(env -i HOME="$home" PATH="/usr/bin:/bin" "$layout" 'x.docx')
  [ "$actual" = "$(printf '<%s>\n<%s>\n<%s>\n<%s>\n<%s>\n<%s>' --launch-profile nightly-v1 --profile-root "$home/.local/share/wordcraft-nightly" -- x.docx)" ] \
    || { echo "error: $layout did not wrap: $actual" >&2; exit 1; }
  actual=$(env -i HOME="$home" PATH="/usr/bin:/bin" "$layout" --version)
  [ "$actual" = '<--version>' ] || { echo "error: $layout did not pass --version: $actual" >&2; exit 1; }
done

echo "PASS: the Nightly tar starts the GUI through its launcher and keeps the profile it is given"

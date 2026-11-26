#!/usr/bin/env bash
# Fetch and patch the dependencies the iPad build changes, and write the Cargo config that uses them.
#
#   winit  storytold/winit at the workspace's pinned rev, plus patches/winit.patch: hardware
#          keyboard keys and modifiers, key repeat, trackpad hover, two-finger scroll and clicks.
#   rfd    rfd 0.17.2 from crates.io, plus patches/rfd.patch: an iOS backend (no native dialogs;
#          the app uses its own file chooser).
#
# Output: $CARGO_TARGET_DIR/ios-src/{winit,rfd} and $CARGO_TARGET_DIR/ios-src/patch.toml, for
# `cargo build --config "$CARGO_TARGET_DIR/ios-src/patch.toml"`. The desktop builds don't use it.
#
# Usage: packaging/ios/prepare-sources.sh
set -euo pipefail
# shellcheck source=../env.sh
. "$(dirname "${BASH_SOURCE[0]}")/../env.sh"
HERE="$ROOT/packaging/ios"
OUT="$CARGO_TARGET_DIR/ios-src"

# The winit rev is the one `[patch.crates-io]` in the root Cargo.toml pins.
WINIT_REV="$(sed -n 's/^winit = { git = "https:\/\/github.com\/storytold\/winit.git", rev = "\([0-9a-f]*\)" }$/\1/p' "$ROOT/Cargo.toml")"
if [[ -z "$WINIT_REV" ]]; then
  echo "error: no winit rev in the root Cargo.toml's [patch.crates-io]" >&2
  exit 1
fi
RFD_VERSION=0.17.2

rm -rf "$OUT/winit" "$OUT/rfd"
mkdir -p "$OUT"

echo "==> winit $WINIT_REV"
git init -q "$OUT/winit"
git -C "$OUT/winit" fetch -q --depth 1 https://github.com/storytold/winit.git "$WINIT_REV"
git -C "$OUT/winit" checkout -q FETCH_HEAD
git -C "$OUT/winit" apply --whitespace=nowarn "$HERE/patches/winit.patch"

echo "==> rfd $RFD_VERSION"
curl -fsSL --retry 4 "https://static.crates.io/crates/rfd/rfd-$RFD_VERSION.crate" -o "$OUT/rfd.crate"
mkdir -p "$OUT/rfd"
tar -xzf "$OUT/rfd.crate" -C "$OUT/rfd" --strip-components 1
rm "$OUT/rfd.crate"
(cd "$OUT/rfd" && git init -q && git apply --whitespace=nowarn "$HERE/patches/rfd.patch")

cat >"$OUT/patch.toml" <<TOML
# Written by packaging/ios/prepare-sources.sh; the iPad build's patched dependencies.
[patch.crates-io]
winit = { path = "$OUT/winit" }
rfd = { path = "$OUT/rfd" }
TOML
echo "==> wrote $OUT/patch.toml"

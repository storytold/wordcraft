#!/usr/bin/env bash
# Validate the staged Arch launcher without building or opening WordCraft.
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
stage="$work/source"
dst="$work/dist"
base=wordcraft-0.0.1-nightly.1-linux-x86_64
mkdir -p "$stage/bin" "$dst" "$work/extract"

# A local argv/env recorder stands in for the compiled binaries. It never opens a UI.
cat >"$stage/bin/wordcraft" <<'RECORDER'
#!/bin/sh
printf 'XDG_CONFIG_HOME=%s\n' "${XDG_CONFIG_HOME:-}"
printf 'XDG_DATA_HOME=%s\n' "${XDG_DATA_HOME:-}"
printf '<%s>\n' "$@"
RECORDER
chmod 755 "$stage/bin/wordcraft"
cp "$stage/bin/wordcraft" "$stage/bin/wordcraft-cli"

bash "$HERE/nightly-arch-tar.sh" "$stage" "$work/stage" "$dst" "$base"
tar -xzf "$dst/$base.tar.gz" -C "$work/extract"
bin="$work/extract/$base/bin"
# Simulate the existing Arch bin-tree copy and public launcher rename.
mkdir -p "$work/installed"
cp -R "$bin" "$work/installed/bin"
bin="$work/installed/bin"
mv "$bin/wordcraft" "$bin/wordcraft-nightly"
mv "$bin/wordcraft-cli" "$bin/wordcraft-nightly-cli"

test ! -e "$stage/bin/.wordcraft-nightly"
cmp "$stage/bin/wordcraft" "$bin/.wordcraft-nightly/wordcraft"
cmp "$stage/bin/wordcraft-cli" "$bin/wordcraft-nightly-cli"
export HOME="$work/home with space"
unset XDG_CONFIG_HOME XDG_DATA_HOME
expect() {
  expected="$1"
  shift
  actual=$(PATH="$bin:$PATH" wordcraft-nightly "$@")
  if [[ "$actual" != "$expected" ]]; then
    printf 'expected:\n%s\nactual:\n%s\n' "$expected" "$actual" >&2
    exit 1
  fi
}
expect "$(printf 'XDG_CONFIG_HOME=%s\nXDG_DATA_HOME=%s\n<>' "$HOME/.config/wordcraft-nightly" "$HOME/.local/share/wordcraft-nightly")"
export XDG_CONFIG_HOME="$work/xdg config with space"
export XDG_DATA_HOME="$work/xdg data with space"
expect "$(printf 'XDG_CONFIG_HOME=%s\nXDG_DATA_HOME=%s\n<%s>\n<%s>' "$XDG_CONFIG_HOME/wordcraft-nightly" "$XDG_DATA_HOME/wordcraft-nightly" 'doc with spaces.docx' '--other-name.docx')" \
  'doc with spaces.docx' '--other-name.docx'
expect "$(printf 'XDG_CONFIG_HOME=%s\nXDG_DATA_HOME=%s\n<--version>' "$XDG_CONFIG_HOME" "$XDG_DATA_HOME")" --version
expect "$(printf 'XDG_CONFIG_HOME=%s\nXDG_DATA_HOME=%s\n<-V>' "$XDG_CONFIG_HOME" "$XDG_DATA_HOME")" -V

echo "PASS: Arch Nightly handoff isolates the profile and preserves launch arguments"

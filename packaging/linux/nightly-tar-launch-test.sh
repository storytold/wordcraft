#!/usr/bin/env bash
# The launcher inside the plain Nightly tar (packaging/linux/nightly-tar-launch.sh): craft-common-actions'
# template rendered for WordCraft. Every argument follows the profile tokens, the profile root is the
# package identity under the XDG data home (or HOME), and a version query is asked in the Launcher's
# own form so the GUI answers before any profile is read.
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
mkdir -p "$work/tar"

cat >"$work/tar/wordcraft-bin" <<'RECORDER'
#!/bin/sh
printf '<%s>\n' "$@"
RECORDER
chmod 755 "$work/tar/wordcraft-bin"
install -m 755 "$HERE/nightly-tar-launch.sh" "$work/tar/wordcraft"

# The bytes are the shared template rendered for this app, not a local script.
sha256sum "$HERE/nightly-tar-launch.sh" | cut -d' ' -f1 | grep -qx \
  b67f728b25e00d7b1b759891283653d20d7900d6c5bc5aba813054cb1412f4cc || {
  echo "FAIL: nightly-tar-launch.sh is not the shared template rendered for wordcraft" >&2
  exit 1
}

home="$work/home with space"
mkdir -p "$home"
identity="ai.storyteller.wordcraft.nightly"

expect() {
  local root="$1" actual
  shift
  actual=$(env -i HOME="$home" PATH="/usr/bin:/bin" "$work/tar/wordcraft" "$@")
  [ "$actual" = "$(printf '<%s>\n' --launch-profile nightly-v1 --profile-root "$root" -- "$@")" ] || {
    printf 'expected:\n%s\nactual:\n%s\n' \
      "$(printf '<%s>\n' --launch-profile nightly-v1 --profile-root "$root" -- "$@")" "$actual" >&2
    exit 1
  }
}

expect_query() {
  local root="$1" actual
  for flag in --version -V; do
    actual=$(env -i HOME="$home" PATH="/usr/bin:/bin" "$work/tar/wordcraft" "$flag")
    [ "$actual" = "$(printf '<%s>\n' --launch-profile nightly-v1 --profile-root "$root" --version)" ] || {
      printf 'expected:\n%s\nactual:\n%s\n' \
        "$(printf '<%s>\n' --launch-profile nightly-v1 --profile-root "$root" --version)" "$actual" >&2
      exit 1
    }
  done
}

expect "$home/.local/share/$identity"
expect "$home/.local/share/$identity" 'doc with spaces.docx' '--other-name.docx'
expect_query "$home/.local/share/$identity"

data="$work/xdg data with space"
actual=$(env -i HOME="$home" XDG_DATA_HOME="$data" PATH="/usr/bin:/bin" "$work/tar/wordcraft" 'x.docx')
[ "$actual" = "$(printf '<%s>\n' --launch-profile nightly-v1 --profile-root "$data/$identity" -- x.docx)" ]
[ ! -e "$data" ] || { echo "FAIL: the launcher created the profile folder" >&2; exit 1; }

# The launcher found through PATH still resolves the GUI beside itself.
mkdir -p "$work/elsewhere"
actual=$(cd "$work/elsewhere" && env -i HOME="$home" PATH="$work/tar:/usr/bin:/bin" wordcraft 'x.docx')
[ "$actual" = "$(printf '<%s>\n' --launch-profile nightly-v1 --profile-root "$home/.local/share/$identity" -- x.docx)" ]

# The flat tar nightly.sh stages: `wordcraft` beside `wordcraft-bin`, nothing else.
source="$HERE/nightly.sh"
grep -q 'nightly-tar-launch.sh" "$STAGE/wordcraft"' "$source" || {
  echo "FAIL: nightly.sh does not stage the template as wordcraft" >&2
  exit 1
}

echo "PASS: the flat tar launcher is the shared template and starts the GUI through the channel's profile"

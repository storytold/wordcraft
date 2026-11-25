#!/bin/sh
# The Nightly channel's launcher, installed by every Linux format in place of the GUI binary.
#
# A Nightly keeps all of its user state in its own launch profile, so the launcher adds
# `--launch-profile nightly-v1 --profile-root ...` and passes the user's arguments after `--`.
# Redirecting XDG_* instead is refused by the build's profile validator, which derives the stable
# boundary from the environment, so the chosen profile would sit inside that boundary.
#
# The GUI sits beside the launcher in one of these layouts:
#   wordcraft-bin                         an unpacked app-owned nightly tar (nightly.sh)
#   .wordcraft-nightly/wordcraft          the Arch handoff (nightly-arch-tar.sh)
#   ../lib/wordcraft-nightly/wordcraft    deb/rpm and the plain FHS tar (package.sh)
#   usr/bin/wordcraft                     an AppImage, whose AppRun is this launcher
# The path is resolved from this script, never from the working directory, and only a regular
# executable is used.
#
# The ArtCraft Launcher starts the tar's `wordcraft` with the profile form already supplied
# (`--launch-profile nightly-v1 --profile-root <abs> -- files`, or `--version` for its check).
# That form passes straight through: wrapping it again would turn its tokens into file names and
# ignore the root the Launcher chose. `--version`/`-V` do too, so a version check opens no window
# and creates no profile.
set -eu

case $0 in
  */*) self=$0 ;;
  *) self=$(command -v -- "$0") || { echo "wordcraft: cannot locate this launcher" >&2; exit 2; } ;;
esac
here=$(CDPATH='' cd -- "$(dirname -- "$self")" && pwd -P) \
  || { echo "wordcraft: cannot resolve this launcher's folder" >&2; exit 2; }

real_bin=
for candidate in \
  "$here/wordcraft-bin" \
  "$here/.wordcraft-nightly/wordcraft" \
  "$here/../lib/wordcraft-nightly/wordcraft" \
  "$here/usr/bin/wordcraft"
do
  if [ -f "$candidate" ] && [ -x "$candidate" ]; then
    real_bin=$candidate
    break
  fi
done
[ -n "$real_bin" ] || { echo "wordcraft: cannot find the Nightly GUI next to $here" >&2; exit 2; }

case ${1:-} in
  --launch-profile | --version | -V) exec "$real_bin" "$@" ;;
esac

data_home=${XDG_DATA_HOME:-}
if [ -z "$data_home" ]; then
  data_home=${HOME:?HOME is required}/.local/share
fi
exec "$real_bin" --launch-profile nightly-v1 --profile-root "$data_home/wordcraft-nightly" -- "$@"

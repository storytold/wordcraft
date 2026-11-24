#!/bin/sh
set -eu

# The Nightly channel's own launch profile holds every piece of user state (preferences, window
# place, logs, recovery, fonts, templates, the dictionary and the suite font store). Redirecting
# XDG_* instead is refused by the build's root validator, which derives the stable boundary from
# the environment, so the profile would sit inside it.
real_bin="$(dirname "$0")/.wordcraft-nightly/wordcraft"
if [ "${1:-}" = "--version" ] || [ "${1:-}" = "-V" ]; then
  exec "$real_bin" "$@"
fi
exec "$real_bin" --launch-profile nightly-v1 \
  --profile-root "${HOME:?HOME is required}/.local/share/wordcraft-nightly" -- "$@"

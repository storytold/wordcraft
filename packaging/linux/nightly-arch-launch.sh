#!/bin/sh
set -eu

real_bin="$(dirname "$0")/.wordcraft-nightly/wordcraft"
if [ "${1:-}" = "--version" ] || [ "${1:-}" = "-V" ]; then
  exec "$real_bin" "$@"
fi
export XDG_CONFIG_HOME="${XDG_CONFIG_HOME:-${HOME:?HOME is required}/.config}/wordcraft-nightly"
export XDG_DATA_HOME="${XDG_DATA_HOME:-${HOME:?HOME is required}/.local/share}/wordcraft-nightly"
exec "$real_bin" "$@"

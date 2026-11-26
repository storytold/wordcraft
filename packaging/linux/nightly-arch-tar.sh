#!/usr/bin/env bash
# Stage the WordCraft Nightly-only handoff for the Arch packager.
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
source_tree="$1"
work="$2"
destination="$3"
basename="$4"
stage="$work/$basename"

mkdir -p "$work"
cp -R "$source_tree" "$stage"
mkdir "$stage/bin/.wordcraft-nightly"
mv "$stage/bin/wordcraft" "$stage/bin/.wordcraft-nightly/wordcraft"
cp "$here/nightly-launcher.sh" "$stage/bin/wordcraft"
chmod 755 "$stage/bin/wordcraft"
tar -C "$work" -czf "$destination/$basename.tar.gz" "$basename"
echo "wrote $destination/$basename.tar.gz"

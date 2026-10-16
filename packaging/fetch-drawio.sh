#!/usr/bin/env bash
# Fetch the draw.io web app (Apache-2.0, https://github.com/jgraph/drawio) into vendor/drawio,
# which is what the diagram editor is served from: no network, no third-party embed site.
#
# Usage: packaging/fetch-drawio.sh [version]        (default: the version in VERSION, else v31.4.2)
#
# Sources: the Gitee mirror first (fast from China), GitHub second.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DEST="$ROOT/vendor/drawio"
VERSION="${1:-v31.4.2}"
WORK="${TMPDIR:-/tmp}/wordcraft-drawio-$VERSION"

MIRRORS=(
  "https://gitee.com/mirrors/drawio.git"
  "https://github.com/jgraph/drawio.git"
)

if [ -f "$DEST/VERSION" ] && [ "$(cat "$DEST/VERSION")" = "$VERSION" ]; then
  echo "vendor/drawio is already at $VERSION (delete it to fetch again)"
  exit 0
fi

echo "==> fetching draw.io $VERSION"
rm -rf "$WORK"
for mirror in "${MIRRORS[@]}"; do
  echo "    trying $mirror"
  if GIT_TERMINAL_PROMPT=0 git clone --depth 1 --branch "$VERSION" "$mirror" "$WORK" 2>/dev/null; then
    break
  fi
  rm -rf "$WORK"
done
[ -d "$WORK/src/main/webapp" ] || { echo "error: could not fetch draw.io $VERSION" >&2; exit 1; }

WEBAPP="$WORK/src/main/webapp"
echo "==> installing the web app into vendor/drawio"
rm -rf "$DEST"
mkdir -p "$DEST"
# Everything the editor loads at runtime; the pruned files are the hosted-service integrations,
# the dev sources' source maps and the Java servlet packaging, none of which a local editor uses.
rsync -a \
  --exclude 'WEB-INF/' --exclude 'META-INF/' \
  --exclude '*.map' \
  --exclude 'js/integrate.min.js' \
  --exclude 'js/viewer-static.min.js' \
  --exclude 'service-worker.js' --exclude 'workbox-*.js' \
  --exclude 'connect/' \
  --exclude 'dropbox.html' --exclude 'onedrive3.html' --exclude 'github.html' \
  --exclude 'gitlab.html' --exclude 'teams.html' --exclude 'monday-app-association.json' \
  --exclude 'clear.html' --exclude 'export3.html' --exclude 'open.html' \
  --exclude 'vsdxImporter.html' \
  "$WEBAPP/" "$DEST/"

printf '%s\n' "$VERSION" > "$DEST/VERSION"
printf 'draw.io %s — Apache-2.0 — https://github.com/jgraph/drawio\n' "$VERSION" > "$DEST/README.md"

rm -rf "$WORK"
echo "==> vendor/drawio: $(du -sh "$DEST" | cut -f1)"

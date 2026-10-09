#!/usr/bin/env bash
# Installs a COPY of the repo's client as ~/.local/bin/wordcraft-chat.
set -euo pipefail
install -Dm755 "$(dirname "$0")/wordcraft_chat.py" "$HOME/.local/bin/wordcraft-chat"
echo "installed: $HOME/.local/bin/wordcraft-chat"

#!/usr/bin/env bash
# Installs the project's git hooks from .githooks/ into .git/hooks/.
# Run once after cloning: bash scripts/install-hooks.sh
set -euo pipefail

REPO_ROOT="$(git rev-parse --show-toplevel)"
SRC="$REPO_ROOT/.githooks"
DST="$REPO_ROOT/.git/hooks"

if [ ! -d "$SRC" ]; then
  echo "Error: .githooks/ directory not found at $REPO_ROOT" >&2
  exit 1
fi

for hook in "$SRC"/*; do
  name="$(basename "$hook")"
  target="$DST/$name"
  cp "$hook" "$target"
  chmod +x "$target"
  echo "Installed: .git/hooks/$name"
done

echo "Done. Git hooks are active."

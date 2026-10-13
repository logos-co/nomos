#!/usr/bin/env bash
set -euo pipefail

VERSION="0.10.0"
INSTALL="cargo install taplo-cli --locked --version $VERSION"

current="$(taplo --version 2>/dev/null || echo "none")"

if [[ "$current" != "taplo $VERSION" ]]; then
  if ! { : </dev/tty; } 2>/dev/null; then
    echo "taplo $VERSION required (found: $current). No terminal to confirm. Run: $INSTALL" >&2
    exit 1
  fi
  {
    echo "This repo requires taplo $VERSION (found: $current)."
    echo "It uses the taplo on your PATH and will replace it with $VERSION in ~/.cargo/bin."
    echo "To back off, answer N and skip these hooks with SKIP=taplo-format,taplo-lint."
  } >/dev/tty
  read -r -p "Install taplo $VERSION? [y/N] " answer </dev/tty >/dev/tty
  [[ "$answer" == [yY] ]] || { echo "Aborted." >&2; exit 1; }
  $INSTALL
fi

exec taplo "$@"

#!/usr/bin/env bash
set -euo pipefail

: "${CONTROL_FIFO:?CONTROL_FIFO must be set}"

if [[ $# -ne 1 ]]; then
  echo "usage: $0 <command>" >&2
  exit 1
fi

printf '%s\n' "$1" > "$CONTROL_FIFO"

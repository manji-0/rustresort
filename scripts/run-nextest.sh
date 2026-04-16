#!/usr/bin/env bash
set -euo pipefail

jobs="${NEXTEST_JOBS:-1}"

if [ "$#" -eq 0 ]; then
  if [ -n "${NEXTEST_FILTER:-}" ]; then
    cargo nextest run -j "${jobs}" -- "${NEXTEST_FILTER}"
  else
    cargo nextest run -j "${jobs}"
  fi
else
  cargo nextest run -j "${jobs}" -- "$@"
fi

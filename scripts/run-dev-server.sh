#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

source ./scripts/export-dev-env.sh

echo "Starting RustResort dev server with integrated UI at ${RUSTRESORT__SERVER__PROTOCOL}://${RUSTRESORT__SERVER__DOMAIN}/ui"
./scripts/build-dev-server.sh
exec ./scripts/run-dev-binary.sh

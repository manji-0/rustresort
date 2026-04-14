#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PORT="${RUSTRESORT_UI_PORT:-3011}"
BASE_URL="${RUSTRESORT_UI_BASE_URL:-http://127.0.0.1:${PORT}}"
DB_PATH="${RUSTRESORT_UI_DB_PATH:-/tmp/rustresort-web-ui-e2e.db}"
LOG_PATH="${RUSTRESORT_UI_LOG_PATH:-/tmp/rustresort-web-ui-e2e.log}"
HEALTH_TIMEOUT_SECONDS="${RUSTRESORT_UI_HEALTH_TIMEOUT_SECONDS:-180}"

cleanup() {
  if [[ -n "${SERVER_PID:-}" ]]; then
    kill "${SERVER_PID}" >/dev/null 2>&1 || true
    wait "${SERVER_PID}" >/dev/null 2>&1 || true
  fi
}
trap cleanup EXIT

rm -f "${DB_PATH}" "${DB_PATH}-shm" "${DB_PATH}-wal"

cd "${ROOT_DIR}"

./scripts/build-wasm-ui.sh

if [[ ! -d node_modules/playwright ]]; then
  npm install
fi

env \
  RUSTRESORT_UI_HOST=127.0.0.1 \
  RUSTRESORT_UI_PORT="${PORT}" \
  RUSTRESORT_UI_DB_PATH="${DB_PATH}" \
  RUSTRESORT_UI_USERNAME="${RUSTRESORT_UI_USERNAME:-admin}" \
  RUSTRESORT_UI_PASSWORD="${RUSTRESORT_UI_PASSWORD:-admin-password}" \
  cargo run --bin ui_playwright_server >"${LOG_PATH}" 2>&1 &
SERVER_PID=$!

for _ in $(seq 1 "${HEALTH_TIMEOUT_SECONDS}"); do
  if curl -fsS "${BASE_URL}/health" >/dev/null 2>&1; then
    break
  fi
  sleep 1
done

if ! curl -fsS "${BASE_URL}/health" >/dev/null 2>&1; then
  echo "server failed to become healthy within ${HEALTH_TIMEOUT_SECONDS}s; log: ${LOG_PATH}" >&2
  exit 1
fi

npm run install:browsers >/dev/null
RUSTRESORT_UI_BASE_URL="${BASE_URL}" npm run test:web-ui

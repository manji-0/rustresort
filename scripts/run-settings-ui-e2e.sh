#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PORT="${RUSTRESORT_UI_PORT:-3010}"
BASE_URL="${RUSTRESORT_UI_BASE_URL:-http://localhost:${PORT}}"
DB_PATH="${RUSTRESORT_UI_DB_PATH:-/tmp/rustresort-settings-ui-e2e.db}"
LOG_PATH="${RUSTRESORT_UI_LOG_PATH:-/tmp/rustresort-settings-ui-e2e.log}"
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

if [[ ! -d node_modules/playwright ]]; then
  npm install
fi

env \
  RUSTRESORT__SERVER__HOST=127.0.0.1 \
  RUSTRESORT__SERVER__PORT="${PORT}" \
  RUSTRESORT__SERVER__DOMAIN="localhost:${PORT}" \
  RUSTRESORT__SERVER__PROTOCOL=http \
  RUSTRESORT__DATABASE__PATH="${DB_PATH}" \
  RUSTRESORT__AUTH__USERNAME=admin \
  RUSTRESORT__AUTH__PASSWORD=admin-password \
  RUSTRESORT__AUTH__SESSION_SECRET=0123456789abcdef0123456789abcdef \
  RUSTRESORT__INSTANCE__TITLE="RustResort UI Test" \
  RUSTRESORT__INSTANCE__DESCRIPTION="UI test instance" \
  RUSTRESORT__INSTANCE__CONTACT_EMAIL=admin@example.com \
  cargo run --bin rustresort >"${LOG_PATH}" 2>&1 &
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
RUSTRESORT_UI_BASE_URL="${BASE_URL}" npm run test:settings-ui

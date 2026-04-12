#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

source ./scripts/export-dev-env.sh

CONTROL_DIR="$(mktemp -d)"
CONTROL_FIFO="$CONTROL_DIR/commands.fifo"
mkfifo "$CONTROL_FIFO"
exec 3<>"$CONTROL_FIFO"

SERVER_PID=""
UI_WATCH_PID=""
BUILD_WATCH_PID=""
RESTART_WATCH_PID=""

start_server() {
  echo "Starting RustResort dev server with integrated UI at ${RUSTRESORT__SERVER__PROTOCOL}://${RUSTRESORT__SERVER__DOMAIN}/ui"
  ./scripts/run-dev-binary.sh &
  SERVER_PID=$!
}

stop_server() {
  if [[ -n "$SERVER_PID" ]] && kill -0 "$SERVER_PID" 2>/dev/null; then
    kill "$SERVER_PID" 2>/dev/null || true
    wait "$SERVER_PID" 2>/dev/null || true
  fi
  SERVER_PID=""
}

restart_server() {
  stop_server
  start_server
}

rebuild_and_restart_server() {
  ./scripts/build-dev-server.sh
  restart_server
}

cleanup() {
  if [[ -n "$UI_WATCH_PID" ]]; then
    kill "$UI_WATCH_PID" 2>/dev/null || true
  fi
  if [[ -n "$BUILD_WATCH_PID" ]]; then
    kill "$BUILD_WATCH_PID" 2>/dev/null || true
  fi
  if [[ -n "$RESTART_WATCH_PID" ]]; then
    kill "$RESTART_WATCH_PID" 2>/dev/null || true
  fi
  stop_server
  exec 3<&-
  exec 3>&-
  rm -f "$CONTROL_FIFO"
  rmdir "$CONTROL_DIR" 2>/dev/null || true
}

trap cleanup EXIT INT TERM

./scripts/build-wasm-ui.sh
./scripts/build-dev-server.sh
start_server

CONTROL_FIFO="$CONTROL_FIFO" watchexec \
  --postpone \
  --watch crates/rustresort-ui/src \
  --watch crates/rustresort-ui/Cargo.toml \
  --watch scripts/build-wasm-ui.sh \
  --watch scripts/write-watch-command.sh \
  -- ./scripts/write-watch-command.sh ui-build &
UI_WATCH_PID=$!

CONTROL_FIFO="$CONTROL_FIFO" watchexec \
  --postpone \
  --watch src \
  --watch crates/rustresort-models/src \
  --watch crates/rustresort-storage/src \
  --watch Cargo.toml \
  --watch Cargo.lock \
  --watch scripts/build-dev-server.sh \
  --watch scripts/run-dev-binary.sh \
  --watch scripts/export-dev-env.sh \
  --watch scripts/write-watch-command.sh \
  -- ./scripts/write-watch-command.sh server-build &
BUILD_WATCH_PID=$!

CONTROL_FIFO="$CONTROL_FIFO" watchexec \
  --postpone \
  --watch config \
  --watch migrations \
  --watch scripts/run-dev-server.sh \
  --watch scripts/write-watch-command.sh \
  -- ./scripts/write-watch-command.sh server-restart &
RESTART_WATCH_PID=$!

while IFS= read -r command <&3; do
  case "$command" in
    ui-build)
      ./scripts/build-wasm-ui.sh
      ;;
    server-build)
      rebuild_and_restart_server
      ;;
    server-restart)
      restart_server
      ;;
  esac
done

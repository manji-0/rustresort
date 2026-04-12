#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CRATE_DIR="$ROOT_DIR/crates/rustresort-ui"
DIST_DIR="$CRATE_DIR/dist"
TARGET_DIR="$ROOT_DIR/target/wasm-ui"

RUSTC="$(rustup which rustc)"

RUSTC="$RUSTC" rustup run stable cargo build \
  --manifest-path "$CRATE_DIR/Cargo.toml" \
  --target wasm32-unknown-unknown \
  --release \
  --target-dir "$TARGET_DIR"

wasm-bindgen \
  --target web \
  --out-dir "$DIST_DIR" \
  "$TARGET_DIR/wasm32-unknown-unknown/release/rustresort_ui.wasm"

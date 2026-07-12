#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

npm --prefix "$ROOT_DIR/frontend" ci
npm --prefix "$ROOT_DIR/frontend" run build
cargo build --manifest-path "$ROOT_DIR/Cargo.toml" --release --features embedded-frontend

printf 'Built %s\n' "$ROOT_DIR/target/release/codex-gateway"

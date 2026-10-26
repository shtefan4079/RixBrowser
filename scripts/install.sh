#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."
./scripts/cargo.sh build --package rix-app --release --offline

PREFIX="${PREFIX:-$HOME/.local}"
BIN_DIR="${PREFIX}/bin"

mkdir -p "${BIN_DIR}"
install -m 755 target/release/rix-browser "${BIN_DIR}/rix-browser"

echo "Installed rix-browser to ${BIN_DIR}/rix-browser"

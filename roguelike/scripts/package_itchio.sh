#!/usr/bin/env bash

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TARGET_DIR="${TARGET_DIR:-$ROOT_DIR/target}"
DIST_DIR="${DIST_DIR:-$ROOT_DIR/dist/itchio}"
OUT_NAME="roguelike"
ZIP_BASENAME="escape-from-yerba-buena"
WASM_TARGET="wasm32-unknown-unknown"
WASM_FILE="$TARGET_DIR/$WASM_TARGET/release/$OUT_NAME.wasm"
PACKAGE_DIR="$DIST_DIR/web"
ZIP_PATH="$DIST_DIR/${ZIP_BASENAME}-itchio.zip"

require_cmd() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "Missing required command: $1" >&2
    exit 1
  fi
}

lockfile_version() {
  local crate="$1"
  awk -v crate="$crate" '
    $0 == "[[package]]" { in_pkg = 0 }
    $0 == "name = \"" crate "\"" { in_pkg = 1; next }
    in_pkg && $1 == "version" {
      gsub(/"/, "", $3)
      print $3
      exit
    }
  ' "$ROOT_DIR/Cargo.lock"
}

require_cmd cargo
require_cmd wasm-bindgen

REQUIRED_WASM_BINDGEN_VERSION="$(lockfile_version "wasm-bindgen")"
INSTALLED_WASM_BINDGEN_VERSION="$(wasm-bindgen --version | awk '{print $2}')"

if [[ -n "$REQUIRED_WASM_BINDGEN_VERSION" ]] && [[ "$INSTALLED_WASM_BINDGEN_VERSION" != "$REQUIRED_WASM_BINDGEN_VERSION" ]]; then
  echo "Updating wasm-bindgen-cli from $INSTALLED_WASM_BINDGEN_VERSION to $REQUIRED_WASM_BINDGEN_VERSION..."
  cargo install -f wasm-bindgen-cli --version "$REQUIRED_WASM_BINDGEN_VERSION"
fi

mkdir -p "$PACKAGE_DIR"

echo "Building release WASM bundle..."
cargo build \
  --manifest-path "$ROOT_DIR/Cargo.toml" \
  --release \
  --target "$WASM_TARGET" \
  --no-default-features \
  --features windowed

if [[ ! -f "$WASM_FILE" ]]; then
  echo "Expected WASM output not found at $WASM_FILE" >&2
  exit 1
fi

echo "Generating browser glue with wasm-bindgen..."
wasm-bindgen \
  --target web \
  --no-typescript \
  --out-dir "$PACKAGE_DIR" \
  --out-name "$OUT_NAME" \
  "$WASM_FILE"

cp "$ROOT_DIR/web/index.html" "$PACKAGE_DIR/index.html"

if command -v zip >/dev/null 2>&1; then
  rm -f "$ZIP_PATH"
  (
    cd "$PACKAGE_DIR"
    zip -r "$ZIP_PATH" .
  )
  echo "Created itch.io zip at $ZIP_PATH"
else
  echo "zip not found; publish the contents of $PACKAGE_DIR manually."
fi

echo "Web build is ready in $PACKAGE_DIR"

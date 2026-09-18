#!/usr/bin/env bash
# Builds the browser bundle into web/pkg.
#
# wasm-bindgen rather than wasm-pack: the output is loaded by a plain <script
# type="module">, so there is no npm package to generate and nothing to bundle.
set -euo pipefail
cd "$(dirname "$0")"

# cargo install puts it in ~/.cargo/bin, which is not always on PATH when
# rustup's shims are.
export PATH="$PATH:${CARGO_HOME:-$HOME/.cargo}/bin"
command -v wasm-bindgen >/dev/null || {
  echo "wasm-bindgen not found. Install it with:" >&2
  echo "  cargo install wasm-bindgen-cli --version 0.2.128" >&2
  exit 1
}

cargo build --release -p pixelgen-wasm --target wasm32-unknown-unknown
wasm-bindgen --target web --no-typescript --out-dir web/pkg \
  target/wasm32-unknown-unknown/release/pixelgen_wasm.wasm

# Optional, and worth it: roughly halves the payload.
if command -v wasm-opt >/dev/null; then
  wasm-opt -Os web/pkg/pixelgen_wasm_bg.wasm -o web/pkg/pixelgen_wasm_bg.wasm
fi

echo
echo "serve it with:  python3 -m http.server -d web 8000"

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

# The CLI and the crate must be the same version - a mismatch produces a bundle
# that fails at import time with a message about neither of them. Cargo.lock is
# the single source of truth for which one that is.
want=$(awk '/^name = "wasm-bindgen"$/ { getline; gsub(/[",]/, "", $3); print $3; exit }' Cargo.lock)
have=$(wasm-bindgen --version 2>/dev/null | awk '{print $2}' || true)

if [ "$have" != "$want" ]; then
  echo "need wasm-bindgen-cli $want${have:+, found $have}. Install it with:" >&2
  echo "  cargo install wasm-bindgen-cli --version $want" >&2
  exit 1
fi

cargo build --release -p pixelgen-wasm --target wasm32-unknown-unknown
wasm-bindgen --target web --no-typescript --out-dir web/pkg \
  target/wasm32-unknown-unknown/release/pixelgen_wasm.wasm

# Optional, and worth it: roughly halves the payload.
if command -v wasm-opt >/dev/null; then
  wasm-opt -Os web/pkg/pixelgen_wasm_bg.wasm -o web/pkg/pixelgen_wasm_bg.wasm
fi

echo
echo "serve it with:  python3 -m http.server -d web 8000"

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

# A hook runner runs this inside a toolchain of its own choosing, which will
# have only the host target installed. Ask rustc which toolchain it belongs to
# rather than rustup, which would answer for the default one and add the target
# to a toolchain nothing here is using. The runner may also keep that toolchain
# under a rustup home of its own, which the sysroot names too -
# <home>/toolchains/<name> - and prek keeps its rustup binary in that home, so
# the machine needs no rustup of its own.
if [ ! -d "$(rustc --print target-libdir --target wasm32-unknown-unknown)" ]; then
  sysroot=$(rustc --print sysroot)
  home=$(dirname "$(dirname "$sysroot")")
  rustup=$(command -v rustup || echo "$home/rustup")
  if [ ! -x "$rustup" ]; then
    echo "rustc at $sysroot has no wasm32-unknown-unknown target, and there is no" >&2
    echo "rustup to add it with. Install the target for that toolchain (on Arch," >&2
    echo "the rust-wasm package) or install rustup." >&2
    exit 1
  fi
  RUSTUP_HOME=$home "$rustup" target add --toolchain "$(basename "$sysroot")" wasm32-unknown-unknown
fi

cargo build --release -p pixelgen-wasm --target wasm32-unknown-unknown
wasm-bindgen --target web --no-typescript --out-dir web/pkg \
  target/wasm32-unknown-unknown/release/pixelgen_wasm.wasm

echo
echo "serve it with:  python3 -m http.server -d web 8000"

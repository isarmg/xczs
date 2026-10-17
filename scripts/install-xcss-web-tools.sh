#!/usr/bin/env bash
# Fixed browser compiler tools; product and server Cargo.lock stay independent.
set -euo pipefail
browser_rust_toolchain=1.99.0
browser_wasm_bindgen_version=0.2.129
rustup toolchain install "$browser_rust_toolchain" --profile minimal \
  --component rustfmt --component clippy --target wasm32-unknown-unknown
if ! command -v wasm-bindgen >/dev/null || \
  [[ "$(wasm-bindgen --version)" != "wasm-bindgen $browser_wasm_bindgen_version" ]]
then
  cargo +"$browser_rust_toolchain" install wasm-bindgen-cli \
    --version "$browser_wasm_bindgen_version" --locked
fi
wasm-bindgen --version

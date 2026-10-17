#!/usr/bin/env bash
set -euo pipefail

project_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$project_dir"
# shellcheck source=scripts/lib/toolchain.sh
source "$project_dir/scripts/lib/toolchain.sh"

run() {
  printf '\n==> %s\n' "$*"
  "$@"
}

require() {
  command -v "$1" >/dev/null 2>&1 || {
    printf 'required command is unavailable: %s\n' "$1" >&2
    exit 1
  }
}

require cargo
require git
require node
require npm
require stat

IFS= read -r required_node_version < .node-version
xczs_require_exact_node_version "$project_dir" "$required_node_version" node

run rustc --version
run cargo --version
run node --version
run npm --version
run ./scripts/check-shell.sh
run cargo fmt --all --check
run node scripts/check-release-workflow.mjs
run npm run check:js
run npm run check:xcss
run npm run check:docs
run npm run check:independence
run git diff --check
run git diff --cached --check

run cargo +1.99.0 run --quiet --locked --manifest-path tools/web-build/Cargo.toml -- build --product-root .
run npm run check:types
run cargo +1.99.0 fmt --manifest-path web/xcss-web/Cargo.toml -- --check
run cargo +1.99.0 clippy --manifest-path web/xcss-web/Cargo.toml --locked --target wasm32-unknown-unknown --all-targets --no-deps -- -D warnings
run cargo +1.99.0 test --locked --manifest-path tools/web-build/Cargo.toml
run npm run test:frontend:unit
run cargo clippy --locked --target x86_64-unknown-linux-gnu --all-targets --all-features -- -D warnings
run cargo test --locked --target x86_64-unknown-linux-gnu --all-targets --all-features

printf '\n==> development checks passed; uncommitted work is allowed\n'

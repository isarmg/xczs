#!/usr/bin/env bash
set -euo pipefail

project_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$project_dir"
required_cargo_audit_version="0.22.2"
IFS= read -r required_node_version < .node-version
# shellcheck source=scripts/lib/toolchain.sh
source "$project_dir/scripts/lib/toolchain.sh"

run() {
  printf '\n==> %s\n' "$*"
  "$@"
}

require() {
  if ! command -v "$1" >/dev/null 2>&1; then
    printf 'required command is unavailable: %s\n' "$1" >&2
    exit 1
  fi
}

require cargo
require git
require node
require npm
require python3
require stat
require systemd-analyze

# npm only warns for an engine mismatch. Reject it here before audits, builds,
# dependency code, or browser tooling can create a false-green quality result.
xczs_require_exact_node_version "$project_dir" "$required_node_version" node

run rustc --version
run cargo --version
run node --version
run npm --version
run ./scripts/check-shell.sh
run cargo fmt --all --check

# Install the locked packages without lifecycle scripts before executing the
# repository's JavaScript checks, then reject vulnerable dependencies first.
run npm ci --ignore-scripts --no-audit --no-fund
run npm audit --audit-level=high
run node scripts/check-release-workflow.mjs
run npm run check:js
run npm run check:docs
run npm run check:independence
run npm run build:platform
run npm run check:types
run cargo +1.99.0 fmt --manifest-path web/leptos/Cargo.toml --all --check
run cargo +1.99.0 clippy --manifest-path web/leptos/Cargo.toml --locked --target wasm32-unknown-unknown --all-targets --no-deps -- -D warnings
run python3 -B scripts/test-leptos-build.py
run npm run test:frontend:unit

cargo_audit_version="$(cargo audit --version 2>/dev/null)" || {
  printf 'required Cargo subcommand is unavailable: cargo audit\n' >&2
  exit 1
}
expected_cargo_audit_version="cargo-audit-audit $required_cargo_audit_version"
[[ "$cargo_audit_version" == "$expected_cargo_audit_version" ]] || {
  printf 'cargo-audit %s is required; found: %s\n' \
    "$required_cargo_audit_version" \
    "$cargo_audit_version" >&2
  exit 1
}
printf '\n==> %s\n' "$cargo_audit_version"
if [[ "${XCZS_ISOLATED_QUALITY_GATE:-}" == "1" ]]; then
  [[ -n "${XCZS_QUALITY_AUDIT_DB:-}" ]] || {
    printf 'isolated release gate requires its sealed RustSec database\n' >&2
    exit 1
  }
  run cargo audit \
    --db "$XCZS_QUALITY_AUDIT_DB" \
    --no-fetch \
    --no-yanked
  run cargo audit \
    --file web/leptos/Cargo.lock \
    --db "$XCZS_QUALITY_AUDIT_DB" \
    --no-fetch \
    --no-yanked
elif [[ -n "${XCZS_QUALITY_AUDIT_DB:-}" ]]; then
  printf 'XCZS_QUALITY_AUDIT_DB is reserved for the isolated release gate\n' >&2
  exit 1
else
  run cargo fetch --locked
  run cargo audit --deny yanked
  run cargo fetch --manifest-path web/leptos/Cargo.toml --locked
  run cargo audit --file web/leptos/Cargo.lock --deny yanked
fi

# The independent xssi repository tests signed packaging and installation.
# This product validates its current runtime layout and its real runtime.
run ./scripts/check-deployment.sh

run cargo clippy --locked --target x86_64-unknown-linux-gnu --all-targets --all-features -- -D warnings
run cargo test --locked --target x86_64-unknown-linux-gnu --all-targets --all-features
run ./scripts/check-coverage.sh
run cargo build --locked --release --target x86_64-unknown-linux-gnu
release_binary="${CARGO_TARGET_DIR:-$project_dir/target}/x86_64-unknown-linux-gnu/release/xczs"
run bash scripts/check-release-runtime.sh "$release_binary"

run env XCZS_FRONTEND_BINARY="$release_binary" npm run test:frontend:run
if [[ "${XCZS_ISOLATED_QUALITY_GATE:-}" == "1" ]]; then
  printf '\n==> SKIP: The isolated release gate does not run an unpinned host Microsoft Edge; Chromium and Firefox are required matrix entries.\n'
elif command -v microsoft-edge >/dev/null 2>&1 || command -v microsoft-edge-stable >/dev/null 2>&1; then
  run env XCZS_FRONTEND_BINARY="$release_binary" npm run test:frontend:run -- --edge --project=edge
else
  printf '\n==> SKIP: Microsoft Edge is not installed; Chromium and Firefox are required matrix entries.\n'
fi

if [[ "${XCZS_ISOLATED_QUALITY_GATE:-}" == "1" ]]; then
  [[ "${XCZS_SOURCE_REVISION:-}" =~ ^([0-9a-f]{40}|[0-9a-f]{64})$ ]] || {
    printf 'isolated quality gate requires the verified full source commit\n' >&2
    exit 1
  }
  [[ ! -e .git && ! -L .git ]] || {
    printf 'isolated quality source unexpectedly contains Git metadata\n' >&2
    exit 1
  }
  printf \
    '\n==> isolated checks passed; the release packager will re-verify source tree %s\n' \
    "$XCZS_SOURCE_REVISION"
else
  run git diff --check
  run git diff --cached --check
  if [[ -n "$(git status --porcelain)" ]]; then
    printf 'working tree is not clean:\n' >&2
    git status --short >&2
    exit 1
  fi
  printf '\n==> all release checks passed; working tree is clean\n'
fi

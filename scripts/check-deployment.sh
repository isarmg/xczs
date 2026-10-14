#!/usr/bin/env bash
set -euo pipefail
[[ $# == 0 ]] || { echo 'Usage: check-deployment.sh' >&2; exit 2; }
project_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)"
exec cargo test --locked --manifest-path "$project_root/Cargo.toml" --test runtime_state

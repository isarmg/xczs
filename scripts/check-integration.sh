#!/usr/bin/env bash
set -euo pipefail

project_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$project_dir"

[[ $# == 0 ]] || { echo 'Usage: check-integration.sh' >&2; exit 2; }

binary="$(node scripts/build-frontend-candidate.mjs --print-path)"
./scripts/check-deployment.sh
XCZS_FRONTEND_BINARY="$binary" npm run test:frontend:run

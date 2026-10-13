#!/usr/bin/env bash
set -euo pipefail

project_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$project_dir"

if [[ -e .git || -L .git ]]; then
  command -v git >/dev/null 2>&1 || {
    printf 'required command is unavailable: git\n' >&2
    exit 1
  }
  mapfile -d '' shell_scripts < <(
    git ls-files --cached --others --exclude-standard -z -- \
      install.sh \
      ':(glob)scripts/**/*.sh' \
      ':(glob)tests/**/*.sh' |
      sort -z
  )
else
  # A verified release archive intentionally has no Git metadata. Every file
  # in that tree was tracked at the selected commit, so discover its sources
  # directly instead of silently skipping the isolated release gate.
  mapfile -d '' shell_scripts < <(
    find scripts tests install.sh -type f -name '*.sh' -print0 |
      sort -z
  )
fi
((${#shell_scripts[@]} > 0)) || {
  printf 'no shell sources were found\n' >&2
  exit 1
}

for script in "${shell_scripts[@]}"; do
  printf 'bash -n %q\n' "$script"
  bash -n "$script"
done

if command -v shellcheck >/dev/null 2>&1; then
  shellcheck --version
  if [[ "${XCZS_REQUIRE_SHELLCHECK:-}" == "1" ]]; then
    shellcheck_version="$(
      shellcheck --version | while IFS= read -r line; do
        if [[ "$line" == "version: "* ]]; then
          printf '%s\n' "${line#version: }"
          break
        fi
      done
    )"
    if [[ "$shellcheck_version" != "0.11.0" ]]; then
      printf 'ShellCheck 0.11.0 is required; found %s\n' \
        "${shellcheck_version:-unknown}" >&2
      exit 1
    fi
  fi
  shellcheck --severity=warning "${shell_scripts[@]}"
elif [[ "${XCZS_REQUIRE_SHELLCHECK:-}" == "1" ]]; then
  printf 'required command is unavailable: shellcheck\n' >&2
  exit 1
else
  printf 'SKIP: ShellCheck is not installed; CI requires and pins version 0.11.0.\n'
fi

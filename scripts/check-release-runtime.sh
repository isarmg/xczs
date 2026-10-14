#!/usr/bin/env bash
set -euo pipefail

project_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$project_dir"
[[ $# == 1 && "$1" == /* && -f "$1" && -x "$1" && ! -L "$1" ]] || {
  printf 'usage: %s ABSOLUTE_RELEASE_BINARY\n' "$0" >&2
  exit 2
}
release_binary="$1"
# Intentional process crashes must not leave core files containing state or
# credentials. This limit only applies to this isolated acceptance subprocess.
ulimit -c 0
# The fixture is compiled from this checkout, which may not be the isolated
# source tree that produced the supplied formal package binary.
npm run build:platform
cargo build --locked --release --target x86_64-unknown-linux-gnu --example runtime_shutdown_probe
node scripts/check-shutdown-runtime.mjs "${CARGO_TARGET_DIR:-$project_dir/target}/x86_64-unknown-linux-gnu/release/examples/runtime_shutdown_probe"
# A release build accepts administrator-controlled configuration. Compile the
# fixture as the builder, then provision and execute only this isolated harness
# with the same administrator authority required by an actual installation.
messages="$(mktemp "${TMPDIR:-/tmp}/xczs-release-runtime-build.XXXXXXXX")"
trap 'rm -f -- "$messages"' EXIT
cargo test --locked --no-run --message-format=json \
  --target x86_64-unknown-linux-gnu --test crash_recovery > "$messages"
python3 - "$messages" "$release_binary" "$project_dir" "${CARGO_TARGET_DIR:-$project_dir/target}" <<'PY'
import json,os,pathlib,stat,subprocess,sys
messages,binary,project,target=sys.argv[1:]
project=pathlib.Path(project).resolve(strict=True)
target=pathlib.Path(target).resolve(strict=True)
executables=set()
with open(messages) as source:
    for line in source:
        if len(line)>1024*1024:raise ValueError('compiler message exceeds its bound')
        item=json.loads(line)
        if (item.get('reason')=='compiler-artifact' and item.get('profile',{}).get('test') is True
                and item.get('target',{}).get('name')=='crash_recovery' and item.get('executable')):
            executable=pathlib.Path(item['executable'])
            if not executable.is_absolute() or executable.resolve(strict=True)!=executable or not executable.is_relative_to(target):
                raise ValueError('release fixture executable is outside the physical target')
            info=executable.stat()
            if not stat.S_ISREG(info.st_mode) or info.st_mode&0o022 or not info.st_mode&0o111:
                raise ValueError('release fixture executable is not controlled')
            executables.add(executable)
if len(executables)!=1:raise ValueError('Cargo did not produce exactly one crash-recovery harness')
command=['/usr/bin/env','-i','PATH=/usr/bin:/bin','LANG=C.UTF-8','TMPDIR=/tmp',
         'XCZS_TEST_BINARY='+binary,str(executables.pop()),'--nocapture']
if os.geteuid()!=0:command=['/usr/bin/sudo','--']+command
subprocess.run(command,cwd=project,check=True,timeout=600)
PY

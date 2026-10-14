#!/usr/bin/env bash
# Package the binary already exercised by release_smoke, using the real pinned
# installer/helper/SBOM pipeline. This is not a publishable tagged release.
set -euo pipefail
[[ $# == 4 ]] || { echo 'Usage: package-installed-candidate.sh BINARY METADATA XSSI OUTPUT' >&2; exit 2; }
source_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)"
binary="$1"; metadata="$2"; installer="$3"; output="$4"
for value in "$binary" "$metadata" "$installer" "$output"; do [[ "$value" == /* ]]; done
revision="$(git -C "$source_root" rev-parse HEAD)"
[[ -z "$(git -C "$source_root" status --porcelain=v1 --untracked-files=all)" ]]
xssi_revision="$(sed -n 's/^git_rev=//p' "$source_root/config/xssi.lock")"
[[ "$xssi_revision" =~ ^[0-9a-f]{40}$ && "$(git -C "$installer" rev-parse HEAD)" == "$xssi_revision" ]]
[[ ! -e "$output" && ! -L "$output" ]]
mkdir -m 0700 "$output"
version="$(python3 - "$source_root/Cargo.toml" <<'PY'
import sys,tomllib
with open(sys.argv[1],'rb') as stream: print(tomllib.load(stream)['package']['version'])
PY
)"
payload="xczs-$version-linux-x86_64"
stage="$output/$payload"
mkdir -p "$stage/bin" "$stage/share/contracts" "$stage/config"
python3 - "$binary" "$metadata/xczs-identity.json" "$revision" <<'PY'
import hashlib,json,pathlib,sys
binary,identity_file,revision=sys.argv[1:]
receipt=json.loads(pathlib.Path(binary+'.xssi-web-build.json').read_bytes())
identity=json.loads(pathlib.Path(identity_file).read_bytes())
assert receipt['format']=='xssi-web-build-receipt-v1'
for name,value in [('product','xczs'),('source_revision',revision),('target','x86_64-unknown-linux-gnu')]:
    assert receipt['identity'][name]==value and identity[name]==value
with open(binary,'rb') as stream: assert hashlib.file_digest(stream,'sha256').hexdigest()==receipt['binary_sha256']
PY
install -m 0755 "$binary" "$stage/bin/xczs"
install -m 0644 "$metadata/xczs-state-contract.json" "$stage/share/state-contract.json"
install -m 0644 "$metadata/xczs-web-assets.json" "$stage/share/web-assets.json"
cp -a -- "$metadata/contracts/." "$stage/share/contracts/"
install -m 0644 "$source_root/config/config.json.example" "$stage/config/config.json.example"
install -m 0644 "$source_root/LICENSE-APACHE" "$stage/share/LICENSE"
export XSSI_SOURCE_REVISION="$revision"
export XSSI_IDENTITY_FILE="$metadata/xczs-identity.json"
export XSSI_SCHEMA_FILE="$metadata/xczs-schema.json"
export XSSI_FRONTEND_METADATA_FILE="$metadata/frontend.json"
bash "$installer/scripts/build-package.sh" xczs "$source_root" "$stage"
python3 -I -B "$installer/scripts/package.py" verify "$stage"
[[ -z "$(git -C "$source_root" status --porcelain=v1 --untracked-files=all)" ]]
# Match the actual release envelope; the lifecycle gate verifies this SHA and
# the embedded product/source identity before executing installer code.
tar --owner=0 --group=0 --numeric-owner -C "$output" -czf "$output/$payload.tar.gz" "$payload"
(cd "$output" && sha256sum "$payload.tar.gz" > SHA256SUMS)
printf 'Prepared install candidate from the existing binary at %s\n' "$revision"

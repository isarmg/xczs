#!/usr/bin/env bash
set -euo pipefail
export GIT_NO_REPLACE_OBJECTS=1
source_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)"
product="xczs"
required_cargo_cyclonedx_version="0.5.9"
required_cargo_audit_version="0.22.2"
output=""
while (($#)); do
  case "$1" in
    --output-dir) output="${2:?missing output directory}"; shift 2 ;;
    /*) [[ -z "$output" ]] || { echo 'Duplicate output directory' >&2; exit 2; }; output="$1"; shift ;;
    *) echo 'Usage: package-release.sh --output-dir ABSOLUTE' >&2; exit 2 ;;
  esac
done
[[ "$output" = /* && -d "$output" ]] || { echo 'Existing absolute output directory is required.' >&2; exit 2; }
[[ -z "$(git -C "$source_root" status --porcelain=v1 --untracked-files=all)" ]] || { echo 'Release source must be clean.' >&2; exit 1; }
source_revision="$(git -C "$source_root" rev-parse --verify HEAD)"
version="$(python3 - "$source_root/Cargo.toml" <<'VERSION'
import sys, tomllib
with open(sys.argv[1], 'rb') as stream: print(tomllib.load(stream)['package']['version'])
VERSION
)"
[[ "$(git -C "$source_root" cat-file -t "refs/tags/v$version")" = tag && "$(git -C "$source_root" rev-parse "refs/tags/v$version^{commit}")" = "$source_revision" ]] || { echo 'A source-bound annotated release tag is required.' >&2; exit 1; }
xssi_revision="$(sed -n 's/^git_rev=//p' "$source_root/config/xssi.lock")"
[[ "$xssi_revision" =~ ^[0-9a-f]{40}$ && "$xssi_revision" != 0000000000000000000000000000000000000000 ]] || { echo 'Pin the reviewed xssi commit before building a release.' >&2; exit 1; }
# Keep the supply-chain gates fixed while installation logic lives in xssi.
[[ "$(cargo audit --version)" = "cargo-audit-audit $required_cargo_audit_version" ]] || { echo 'Pinned cargo-audit is required.' >&2; exit 1; }
[[ "$(cargo cyclonedx --version)" = "cargo-cyclonedx-cyclonedx $required_cargo_cyclonedx_version" ]] || { echo 'Pinned cargo-cyclonedx is required.' >&2; exit 1; }
archive="$product-$version-linux-x86_64.tar.gz"
[[ ! -e "$output/$archive" && ! -L "$output/$archive" && ! -e "$output/SHA256SUMS" && ! -L "$output/SHA256SUMS" ]] || { echo 'Refusing to overwrite release assets.' >&2; exit 1; }
work="$(mktemp -d "$output/.$product-package.XXXXXXXX")"
trap 'rm -rf -- "$work"' EXIT
# Build only immutable, verified source objects; dependencies cannot mutate the
# working tree that supplies the source-bound identity or packaging inputs.
original_source="$source_root"
mkdir -m 0700 "$work/source"
git -C "$original_source" archive --format=tar --output="$work/source.tar" "$source_revision"
python3 - "$original_source" "$source_revision" "$work/source.tar" "$work/source" <<'SOURCE'
import pathlib,subprocess,sys,tarfile
repository,revision,archive,destination=sys.argv[1:]
raw=subprocess.check_output(['git','-C',repository,'ls-tree','-rz','--full-tree',revision])
expected={}
for record in raw.split(b'\0'):
    if not record: continue
    identity,name=record.split(b'\t',1);mode,kind,object_id=identity.split()
    if mode not in (b'100644',b'100755') or kind!=b'blob': raise SystemExit('Source tree contains an unsupported entry')
    expected[name.decode('utf-8')]=(mode,object_id)
seen=set()
with tarfile.open(archive,'r:') as source:
    for item in source.getmembers():
        relative=pathlib.PurePosixPath(item.name)
        if relative.is_absolute() or '..' in relative.parts: raise SystemExit('Unsafe source archive path')
        target=pathlib.Path(destination)/relative
        if item.isdir(): target.mkdir(parents=True,exist_ok=True);continue
        if not item.isfile() or item.name not in expected or item.name in seen: raise SystemExit('Unexpected source entry')
        mode,object_id=expected[item.name]
        content=source.extractfile(item).read()
        trusted=subprocess.check_output(['git','-C',repository,'cat-file','blob',object_id.decode()])
        if content!=trusted: raise SystemExit('Source archive bytes differ from the commit')
        target.parent.mkdir(parents=True,exist_ok=True)
        target.write_bytes(content);target.chmod(0o755 if mode==b'100755' else 0o644)
        seen.add(item.name)
if seen!=set(expected): raise SystemExit('Source archive omitted committed files')
SOURCE
source_root="$work/source"
# The shared release builder requires an own source-bound checkout. Populate
# only Git metadata/index here; the already verified archive bytes stay intact.
git init --quiet "$source_root"
git -C "$source_root" fetch --quiet --no-tags "$original_source" "$source_revision"
git -C "$source_root" reset --soft FETCH_HEAD
git -C "$source_root" read-tree HEAD
[[ "$(git -C "$source_root" rev-parse HEAD)" = "$source_revision" && -z "$(git -C "$source_root" status --porcelain=v1 --untracked-files=all)" ]] || { echo 'Private build checkout differs from the verified snapshot.' >&2; exit 1; }
cargo +1.99.0 fetch --manifest-path "$source_root/web/xcss-web/Cargo.toml" --locked
cargo audit --deny yanked --file "$source_root/Cargo.lock"
cargo audit --deny yanked --file "$source_root/web/xcss-web/Cargo.lock"
mkdir -p "$work/stage/bin" "$work/stage/share/systemd" "$work/stage/config" "$work/metadata"
git init --quiet "$work/xssi"
git -C "$work/xssi" remote add origin https://github.com/isarmg/xssi.git
python3 - "$work/xssi" "$xssi_revision" <<'FETCH'
import os,subprocess,sys
root,revision=sys.argv[1:]
environment={key:value for key,value in os.environ.items()
             if not any(part in key.upper() for part in ('TOKEN','PASSWORD','SECRET','SIGNING','PRIVATE_KEY'))}
environment['GIT_TRACE_REDACT']='1'
environment['GIT_TERMINAL_PROMPT']='0'
subprocess.run(['git','-C',root,'fetch','--quiet','--depth=1','origin',revision],env=environment,check=True)
FETCH
export XSSI_BUILD_TOOLS_DIR="$work/xssi"
export XSSI_FRONTEND_METADATA_FILE="$work/metadata/frontend.json"
git -C "$work/xssi" checkout --quiet --detach FETCH_HEAD
[[ "$(git -C "$work/xssi" rev-parse HEAD)" = "$xssi_revision" ]]
builder=(cargo +1.99.0 run --quiet --locked --manifest-path "$source_root/tools/web-build/Cargo.toml" -- server --product-root "$source_root")
export XCZS_SOURCE_REVISION="$source_revision"
export XCSS_WEB_DIST="$source_root/web/runtime-dist"
export XSSI_BUILD_METADATA_DIR="$work/metadata"
"${builder[@]}" --config xcss-web-build.json --mode release --source-revision "$source_revision" > "$work/build.log"
server_binary="$(tail -n 1 "$work/build.log")"
[[ "$server_binary" = /* && -f "$server_binary" && ! -L "$server_binary" && -x "$server_binary" ]] || { echo 'Builder did not return a physical executable.' >&2; exit 1; }
install -m 0755 "$server_binary" "$work/stage/bin/$product"
python3 - "$server_binary.xssi-web-build.json" "$work/stage/bin/$product" "$product" "$source_revision" <<'RECEIPT'
import hashlib,json,pathlib,sys
receipt_file,binary_file,product,revision=sys.argv[1:]
receipt=json.loads(pathlib.Path(receipt_file).read_bytes())
identity=receipt['identity']
if receipt['format']!='xssi-web-build-receipt-v1' or identity['product']!=product or identity['source_revision']!=revision or identity['target']!='x86_64-unknown-linux-gnu':
    raise SystemExit('Staged binary differs from its source-bound build receipt')
with open(binary_file,'rb') as stream: digest=hashlib.file_digest(stream,'sha256').hexdigest()
if digest!=receipt['binary_sha256']: raise SystemExit('Staged binary was replaced after verification')
RECEIPT
install -m 0644 "$work/metadata/$product-state-contract.json" "$work/stage/share/state-contract.json"
install -d -m 0755 "$work/stage/share/contracts"
cp -a -- "$work/metadata/contracts/." "$work/stage/share/contracts/"
install -m 0644 "$work/metadata/$product-web-assets.json" "$work/stage/share/web-assets.json"
install -m 0644 "$source_root/config/config.json.example" "$work/stage/config/config.json.example"
install -m 0644 "$source_root/LICENSE-APACHE" "$work/stage/share/LICENSE"
export XSSI_SOURCE_REVISION="$source_revision"
export XSSI_IDENTITY_FILE="$work/metadata/$product-identity.json"
export XSSI_SCHEMA_FILE="$work/metadata/$product-schema.json"
bash "$work/xssi/scripts/build-package.sh" "$product" "$source_root" "$work/stage"
[[ "$(git -C "$source_root" rev-parse HEAD)" = "$source_revision" && -z "$(git -C "$source_root" status --porcelain=v1 --untracked-files=all)" ]] || { echo 'Private source changed during build.' >&2; exit 1; }
[[ "$(git -C "$original_source" rev-parse HEAD)" = "$source_revision" && "$(git -C "$original_source" rev-parse "refs/tags/v$version^{commit}")" = "$source_revision" ]] || { echo 'Release identity changed during build.' >&2; exit 1; }
[[ -z "$(git -C "$original_source" status --porcelain=v1 --untracked-files=all)" ]] || { echo 'Source changed during build.' >&2; exit 1; }
epoch="$(git -C "$original_source" show -s --format=%ct "$source_revision")"
# Keep the current package envelope identical to the upgrade reader's contract.
payload="$product-$version-linux-x86_64"
mv -- "$work/stage" "$work/$payload"
tar --sort=name --mtime="@$epoch" --owner=0 --group=0 --numeric-owner -C "$work" -czf "$work/$archive" "$payload"
python3 -B - "$work/xssi/scripts" "$work/$archive" "$work/verified-archive" "$product" "$version" "$source_revision" <<'ARCHIVE'
import pathlib,sys
sys.path.insert(0,sys.argv[1])
import package,upgrade
archive,destination,product,version,revision=sys.argv[2:]
destination=pathlib.Path(destination)
destination.mkdir(mode=0o700)
payload=upgrade.extract(pathlib.Path(archive),destination)
manifest=package.verify(payload)
identity=manifest['identity']
if identity['product']!=product or identity['version']!=version or identity['source_revision']!=revision:
    raise SystemExit('Extracted archive differs from the reviewed release identity')
ARCHIVE
(cd "$work" && sha256sum "$archive" > SHA256SUMS)
# Publish without replacing any existing release asset, even if another build races.
ln "$work/SHA256SUMS" "$output/SHA256SUMS"
ln "$work/$archive" "$output/$archive"

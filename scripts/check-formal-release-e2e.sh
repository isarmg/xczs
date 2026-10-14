#!/usr/bin/env bash
set -euo pipefail
[[ $# == 0 ]] || { echo 'Usage: check-formal-release-e2e.sh' >&2; exit 2; }
project_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)"
[[ -z "$(git -C "$project_dir" status --porcelain=v1 --untracked-files=all)" ]] || { echo 'Formal package E2E requires clean source.' >&2; exit 1; }
source_sha="$(git -C "$project_dir" rev-parse --verify HEAD)"
test_root="$(mktemp -d "${RUNNER_TEMP:-${TMPDIR:-/tmp}}/xczs-formal-release-e2e.XXXXXXXX")"
trap 'rm -rf -- "$test_root"' EXIT
chmod 0700 "$test_root"
repository="$test_root/source with spaces & #"
output="$test_root/output with spaces & #"
package="$test_root/package"
git clone --quiet --no-local -- "$project_dir" "$repository"
git -C "$repository" checkout --quiet --detach "$source_sha"
version="$(python3 - "$repository/Cargo.toml" <<'PY'
import sys,tomllib
with open(sys.argv[1],'rb') as stream: print(tomllib.load(stream)['package']['version'])
PY
)"
git -C "$repository" -c user.name=package-e2e -c user.email=package-e2e@example.invalid -c tag.gpgSign=false tag --annotate --force --message='isolated package verification' "v$version" "$source_sha"
mkdir -m 0700 "$output" "$package"
"$repository/scripts/package-release.sh" --output-dir "$output"
archive="xczs-$version-linux-x86_64.tar.gz"
(cd "$output" && sha256sum --check SHA256SUMS)
tar --extract --gzip --no-same-owner --file "$output/$archive" --directory "$package"
python3 - "$package" "$source_sha" <<'PY'
import hashlib,json,pathlib,stat,sys
root=pathlib.Path(sys.argv[1]);raw=(root/'share/package.json').read_bytes()
assert (root/'share/package.sha256').read_bytes()==(hashlib.sha256(raw).hexdigest()+'\n').encode()
manifest=json.loads(raw)
assert manifest['manifest_version']==1
assert manifest['identity']['product']=='xczs'
assert manifest['identity']['source_revision']==sys.argv[2]
assert manifest['setup']=='libexec/setup'
for item in manifest['files']:
    relative=pathlib.PurePosixPath(item['path'])
    assert not relative.is_absolute() and '..' not in relative.parts
    target=root/relative;info=target.lstat()
    assert stat.S_ISREG(info.st_mode) and info.st_nlink==1
    assert stat.S_IMODE(info.st_mode)==int(item['mode'],8)
    assert hashlib.sha256(target.read_bytes()).hexdigest()==item['sha256']
assert (root/'install.sh').is_file() and (root/'maintenance.sh').is_file()
PY
python3 "$package/libexec/package.py" verify "$package" > "$test_root/verified.json"
"$package/bin/xczs" --version
bash "$project_dir/scripts/check-release-runtime.sh" "$package/bin/xczs"
if [[ -n "${XCZS_FORMAL_E2E_EXPORT_DIR:-}" ]]; then
  [[ ! -e "$XCZS_FORMAL_E2E_EXPORT_DIR" && ! -L "$XCZS_FORMAL_E2E_EXPORT_DIR" ]] || { echo 'Export path exists.' >&2; exit 1; }
  install -d -m 0700 "$XCZS_FORMAL_E2E_EXPORT_DIR"
  install -m 0644 "$output/$archive" "$output/SHA256SUMS" "$XCZS_FORMAL_E2E_EXPORT_DIR/"
fi
printf 'Formal SHA-256 package and runtime verification passed for %s at %s\n' "$version" "$source_sha"

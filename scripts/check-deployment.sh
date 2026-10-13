#!/usr/bin/env bash
set -euo pipefail
project_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$project_dir"
[[ "$#" == 0 ]] || { printf 'Usage: %s\n' "${0##*/}" >&2; exit 2; }
command -v python3 >/dev/null
command -v systemd-analyze >/dev/null
bash -n install.sh
python3 scripts/check-install.py
validation_dir="$(mktemp -d /tmp/xczs-unit-validation.XXXXXXXX)"
trap 'rm -rf -- "$validation_dir"' EXIT
mkdir -p "$validation_dir/etc/systemd/system" "$validation_dir/bin"
install -m 0755 /bin/true "$validation_dir/bin/true"
for unit in sysinit.target basic.target shutdown.target network-online.target multi-user.target; do
  printf '[Unit]\nDescription=Offline verification fixture\n' > "$validation_dir/etc/systemd/system/$unit"
done
sed 's#/opt/isarmg/xczs/bin/xczs#/bin/true#' systemd/xczs.service > "$validation_dir/etc/systemd/system/xczs.service"
systemd-analyze --root="$validation_dir" --generators=no verify /etc/systemd/system/xczs.service
printf 'Installer contract, current JSON and systemd unit passed offline checks. No service was installed or started.\n'

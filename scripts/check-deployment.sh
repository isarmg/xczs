#!/usr/bin/env bash
set -euo pipefail
[[ $# == 0 ]] || { echo 'Usage: check-deployment.sh' >&2; exit 2; }
project_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)"
python3 - "$project_root" <<'CHECK'
import json, pathlib, sys
root=pathlib.Path(sys.argv[1]); product='xczs'
assert not (root/'install.sh').exists(), 'installation belongs to pinned xssi'
units=list((root/('deploy' if product=='xcos' else 'systemd')).glob('*.service'))
assert len(units)==(2 if product=='xcos' else 1)
for unit in units:
    text=unit.read_text()
    assert 'ExecStartPre=' not in text and 'RuntimeDirectory=' not in text
    assert '/etc/isarmg/' not in text and '/var/lib/isarmg/' not in text and '/run/isarmg/' not in text
    assert f'/opt/isarmg/{product}/logs' in text and f'/opt/isarmg/{product}/run' in text
    if unit.name==f'{product}.service':
        assert f'ExecStart=/opt/isarmg/{product}/bin/{product} --root /opt/isarmg/{product}' in text
    else:
        assert '/config/mediamtx.yml' in text
        assert '/config/mediamtx.env' in text
config=json.loads((root/'config'/('xcos.json.example' if product=='xcos' else 'config.json.example')).read_text())
assert 'trusted_proxies' not in config
assert config['data_dir']==f'/opt/isarmg/{product}/state'
CHECK
printf 'Current root layout and runtime-only units passed offline checks.\n'

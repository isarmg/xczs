"""Internal implementation of the package's install.sh setup|upgrade entry point."""
import argparse
import json
import os
from pathlib import Path
import pwd
import shutil
import stat
import subprocess
import sys
import tempfile

USER = "isarmg-xczs"
CONFIG = Path("/etc/isarmg/xczs/config.json")
BINARY = Path("/opt/isarmg/xczs/bin/xczs")
STATE = Path("/var/lib/isarmg/xczs")
SHARED = Path("/srv/xczs")
UNIT = Path("/etc/systemd/system/xczs.service")


def require(condition, message):
    if not condition:
        raise RuntimeError(message)


def run(*args, **kwargs):
    return subprocess.run(args, check=True, **kwargs)


def regular(path):
    metadata = path.lstat()
    require(stat.S_ISREG(metadata.st_mode) and metadata.st_nlink == 1,
            f"Expected a regular single-link file: {path}")


def directory(path, uid=0, gid=0, mode=0o755):
    if not path.exists() and not path.is_symlink():
        # Parents in the fixed installation layout remain root-owned.
        directory(path.parent)
        path.mkdir(mode=mode)
        os.chown(path, uid, gid)
        os.chmod(path, mode)
    metadata = path.lstat()
    require(stat.S_ISDIR(metadata.st_mode) and metadata.st_uid == uid
            and stat.S_IMODE(metadata.st_mode) == mode,
            f"Unsafe directory ownership or mode: {path}")


def read_settings(path):
    regular(path)
    def unique(pairs):
        result = {}
        for key, value in pairs:
            require(key not in result, f"Duplicate configuration key: {key}")
            result[key] = value
        return result
    require(path.stat().st_size <= 1024 * 1024, "Configuration exceeds the size limit")
    settings = json.loads(path.read_text(), object_pairs_hook=unique)
    require(isinstance(settings, dict), "Configuration must be an object")
    roots = []
    for name, default in [("serve_path", None), ("data_dir", str(STATE))]:
        value = settings.get(name, default)
        require(isinstance(value, str) and value.startswith("/")
                and not any(ord(c) < 32 or ord(c) == 127 for c in value),
                f"{name} must be an explicit absolute path without control characters")
        path = Path(value)
        require(".." not in path.parts, f"{name} must not contain parent traversal")
        roots.append(path)
    return settings, roots[0], roots[1]


def unit_text(template, shared, state):
    def quoted(path):
        return '"' + str(path).replace("\\", "\\\\").replace('"', '\\"').replace("%", "%%") + '"'
    text = template.read_text()
    lines = [f"ReadWritePaths={quoted(shared)} {quoted(state)}" if line.startswith("ReadWritePaths=") else line for line in text.splitlines()]
    text = "\n".join(lines) + "\n"
    return text


def atomic_file(path, contents, mode):
    fd, temporary = tempfile.mkstemp(prefix=f".{path.name}.", dir=path.parent)
    try:
        with os.fdopen(fd, "wb") as output:
            output.write(contents)
            output.flush()
            os.fchmod(output.fileno(), mode)
            os.fsync(output.fileno())
        os.replace(temporary, path)
        parent = os.open(path.parent, os.O_RDONLY | os.O_DIRECTORY)
        try:
            os.fsync(parent)
        finally:
            os.close(parent)
    finally:
        if os.path.exists(temporary):
            os.unlink(temporary)


def main():
    parser = argparse.ArgumentParser(description="Use package-root install.sh setup|upgrade")
    parser.add_argument("--package", required=True, type=Path)
    parser.add_argument("mode", choices=["setup", "upgrade"])
    parser.add_argument("config", nargs="?", type=Path)
    parser.add_argument("username", nargs="?", default="admin")
    parser.add_argument("--dry-run", action="store_true")
    args = parser.parse_args()
    package = args.package.resolve()
    for path in [package / "bin/xczs", package / "config/config.json.example", package / "systemd/xczs.service"]:
        regular(path)
    require(args.mode != "setup" or args.config is not None, "Usage: install.sh setup CONFIG_JSON [USERNAME]")
    require(args.mode != "upgrade" or args.config is None, "Usage: install.sh upgrade")
    selected_config = args.config if args.mode == "setup" else CONFIG
    _, shared, state = read_settings(selected_config)
    if args.dry_run:
        print(json.dumps({"mode": args.mode, "service_user": USER, "binary": str(BINARY),
                          "config": str(CONFIG), "data_dir": str(state), "serve_path": str(shared),
                          "runtime_dir": "/run/isarmg/xczs", "config_preserved": selected_config == CONFIG}))
        return
    require(os.geteuid() == 0, "Installation requires root; the application runs as isarmg-xczs")
    for command in ["systemctl", "systemd-analyze", "runuser"]:
        require(shutil.which(command), f"Required command is unavailable: {command}")
    try:
        account = pwd.getpwnam(USER)
    except KeyError:
        run("useradd", "--system", "--user-group", "--home-dir", str(STATE), "--shell", "/usr/sbin/nologin", USER)
        account = pwd.getpwnam(USER)
    require(account.pw_uid != 0, "The service account must not be root")
    directory(BINARY.parent)
    directory(CONFIG.parent, account.pw_uid, account.pw_gid, 0o700)
    if args.mode == "setup" and (CONFIG.exists() or CONFIG.is_symlink()):
        regular(CONFIG)
        require(CONFIG.read_bytes() == selected_config.read_bytes(), "setup will not overwrite a different existing configuration")
    if not CONFIG.exists() and not CONFIG.is_symlink():
        require(args.mode == "setup", "upgrade requires an existing configuration")
        fd = os.open(CONFIG, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
        with os.fdopen(fd, "wb") as output:
            output.write(selected_config.read_bytes())
            output.flush()
            os.fchmod(output.fileno(), 0o600)
            os.fchown(output.fileno(), account.pw_uid, account.pw_gid)
            os.fsync(output.fileno())
    regular(CONFIG)
    metadata = CONFIG.stat()
    require(metadata.st_uid == account.pw_uid and stat.S_IMODE(metadata.st_mode) == 0o600,
            "Configuration must already be owned by the service account with mode 0600")
    _, shared, state = read_settings(CONFIG)
    # Never recursively change existing shared data or grant service ownership
    # over arbitrary host paths. Custom roots are prepared by the operator.
    if shared == SHARED and not shared.exists():
        directory(shared, account.pw_uid, account.pw_gid, 0o750)
    require(shared.is_dir() and not shared.is_symlink(), "Prepare the configured shared directory first")
    if state == STATE:
        directory(state, account.pw_uid, account.pw_gid, 0o700)
    else:
        require(state.exists(), "Prepare a custom data_dir as the service account before installation")
        directory(state, account.pw_uid, account.pw_gid, 0o700)
    run("runuser", "-u", USER, "--", "test", "-r", str(shared))
    run("runuser", "-u", USER, "--", "test", "-w", str(shared))
    run("runuser", "-u", USER, "--", "test", "-x", str(shared))
    candidate = BINARY.parent / ".xczs.candidate"
    atomic_file(candidate, (package / "bin/xczs").read_bytes(), 0o755)
    unit_candidate = UNIT.parent / ".xczs.candidate.service"
    desired_unit = unit_text(package / "systemd/xczs.service", shared, state).encode()
    atomic_file(unit_candidate, desired_unit.replace(str(BINARY).encode(), str(candidate).encode()), 0o644)
    was_active = subprocess.run(["systemctl", "is-active", "--quiet", "xczs.service"], check=False).returncode == 0
    previous_binary = BINARY.read_bytes() if BINARY.exists() else None
    previous_unit = UNIT.read_bytes() if UNIT.exists() else None
    installed = False
    stopped = False
    def app(*arguments):
        run("runuser", "-u", USER, "--", "env", "-i", "PATH=/usr/bin:/bin", str(candidate), "--config", str(CONFIG), *arguments)
    try:
        app("config", "validate")
        run("systemd-analyze", "verify", str(unit_candidate))
        if args.mode == "upgrade":
            require(previous_binary is not None and previous_unit is not None, "upgrade requires the current installation")
            run("systemctl", "stop", "xczs.service")
            stopped = True
            app("check")
        else:
            require(not was_active, "setup refuses an active installation")
            app("init", "--username", args.username)  # password inherited only through stdin
            app("check")
        atomic_file(unit_candidate, desired_unit, 0o644)
        os.replace(candidate, BINARY)
        installed = True
        os.replace(unit_candidate, UNIT)
        run("systemctl", "daemon-reload")
        if args.mode == "upgrade" and was_active:
            run("systemctl", "start", "xczs.service")
    except Exception:
        if installed and previous_binary is not None:
            subprocess.run(["systemctl", "stop", "xczs.service"], check=False)
            atomic_file(BINARY, previous_binary, 0o755)
            if previous_unit is not None:
                atomic_file(UNIT, previous_unit, 0o644)
            run("systemctl", "daemon-reload")
        if stopped and was_active:
            run("systemctl", "start", "xczs.service")
        raise
    finally:
        candidate.unlink(missing_ok=True)
        unit_candidate.unlink(missing_ok=True)
    print(json.dumps({"status": "installed", "mode": args.mode, "service_user": USER}))
    if args.mode == "setup":
        print("Start the service when ready: systemctl enable --now xczs.service", file=sys.stderr)


if __name__ == "__main__":
    try:
        main()
    except Exception as error:
        raise SystemExit(f"Installation failed: {error}") from None

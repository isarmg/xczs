"""Offline installer contract checks; never touches host accounts or services."""
import contextlib
import importlib.util
import io
import json
import os
import sys
from types import SimpleNamespace
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

sys.dont_write_bytecode = True

ROOT = Path(__file__).resolve().parent.parent
spec = importlib.util.spec_from_file_location("xczs_installer", ROOT / "libexec/install.py")
installer = importlib.util.module_from_spec(spec)
spec.loader.exec_module(installer)


class InstallerContract(unittest.TestCase):
    def test_current_template_has_one_socket_and_no_credential_bootstrap(self):
        settings, shared, state = installer.read_settings(ROOT / "config/config.json.example")
        self.assertEqual(settings["bind"], "127.0.0.1:5000")
        self.assertEqual(settings["trusted_proxies"], [])
        self.assertEqual(settings["log_level"], "info")
        self.assertEqual(shared, Path("/srv/xczs"))
        self.assertEqual(state, Path("/var/lib/isarmg/xczs"))
        for removed in ["port", "auth", "log_file"]:
            self.assertNotIn(removed, settings)

    def test_unit_allowlist_matches_custom_roots_and_escapes_specifiers(self):
        text = installer.unit_text(ROOT / "systemd/xczs.service", Path('/srv/a b%"'), Path("/data/private"))
        self.assertIn('ReadWritePaths="/srv/a b%%\\\"" "/data/private"', text)
        self.assertNotIn("StateDirectory=", text)
        self.assertIn("User=isarmg-xczs", text)
        self.assertIn("RuntimeDirectory=isarmg/xczs", text)
        self.assertIn("ProtectHome=read-only", text)
        self.assertNotIn("PrivateTmp=yes", text)

    def test_rejects_ambiguous_input_without_changing_it(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "config.json"
            for value in ['{"serve_path":"relative"}', '{"serve_path":"/srv/x","serve_path":"/srv/y"}', '{"serve_path":"/srv/x/../y"}']:
                path.write_text(value)
                with self.assertRaises(RuntimeError):
                    installer.read_settings(path)
                self.assertEqual(path.read_text(), value)

    def test_setup_dry_run_requires_selected_config_and_does_not_create_state(self):
        with tempfile.TemporaryDirectory() as directory:
            package = Path(directory)
            for name in ["bin", "config", "systemd"]:
                (package / name).mkdir()
            (package / "bin/xczs").write_bytes(b"not executed by dry-run")
            for name in ["config/config.json.example", "systemd/xczs.service"]:
                (package / name).write_bytes((ROOT / name).read_bytes())
            selected = package / "chosen.json"
            selected.write_text(json.dumps({"serve_path": str(package / "share"), "data_dir": str(package / "state")}))
            output = io.StringIO()
            with patch("sys.argv", ["install.sh", "--package", str(package), "setup", str(selected), "custom-admin", "--dry-run"]), contextlib.redirect_stdout(output):
                installer.main()
            report = json.loads(output.getvalue())
            self.assertEqual(report["data_dir"], str(package / "state"))
            self.assertEqual(report["service_user"], "isarmg-xczs")
            self.assertFalse((package / "state").exists())
            self.assertFalse((package / "share").exists())

    def exercise_install(self, mode, active=False, failure=None):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            package = root / "package"
            for name in ["bin", "config", "systemd"]:
                (package / name).mkdir(parents=True)
            (package / "bin/xczs").write_bytes(b"candidate")
            for name in ["config/config.json.example", "systemd/xczs.service"]:
                (package / name).write_bytes((ROOT / name).read_bytes())
            shared, state = root / "shared", root / "state"
            shared.mkdir()
            state.mkdir(mode=0o700)
            config = root / "private/config.json"
            config.parent.mkdir(mode=0o700)
            config.write_text(json.dumps({"serve_path": str(shared), "data_dir": str(state)}))
            config.chmod(0o600)
            binary, unit = root / "bin/xczs", root / "units/xczs.service"
            binary.parent.mkdir()
            unit.parent.mkdir()
            binary.write_bytes(b"previous")
            unit.write_bytes(b"previous unit")
            original_config = config.read_bytes()
            calls = []
            def command(*args, **kwargs):
                calls.append(args)
                if failure and failure in args:
                    raise RuntimeError("injected candidate failure")
                return SimpleNamespace(returncode=0)
            def safe_directory(path, *args, **kwargs):
                path.mkdir(parents=True, exist_ok=True)
            argv = ["install.sh", "--package", str(package), mode]
            if mode == "setup":
                argv += [str(config), "chosen-admin"]
            with contextlib.ExitStack() as stack:
                for name, value in [("CONFIG", config), ("BINARY", binary), ("UNIT", unit), ("STATE", state), ("run", command), ("directory", safe_directory)]:
                    stack.enter_context(patch.object(installer, name, value))
                stack.enter_context(patch.object(installer.os, "geteuid", return_value=0))
                stack.enter_context(patch.object(installer.pwd, "getpwnam", return_value=SimpleNamespace(pw_uid=os.getuid(), pw_gid=os.getgid())))
                stack.enter_context(patch.object(installer.shutil, "which", return_value="/mock/tool"))
                stack.enter_context(patch.object(installer.subprocess, "run", return_value=SimpleNamespace(returncode=0 if active else 3)))
                stack.enter_context(patch("sys.argv", argv))
                stack.enter_context(contextlib.redirect_stdout(io.StringIO()))
                stack.enter_context(contextlib.redirect_stderr(io.StringIO()))
                if failure:
                    with self.assertRaisesRegex(RuntimeError, "injected"):
                        installer.main()
                else:
                    installer.main()
            self.assertEqual(config.read_bytes(), original_config)
            self.assertEqual(binary.read_bytes(), b"previous" if failure else b"candidate")
            app_calls = [call for call in calls if "PATH=/usr/bin:/bin" in call]
            self.assertTrue(app_calls)
            self.assertTrue(all("-i" in call for call in app_calls))
            self.assertEqual(any("init" in call for call in app_calls), mode == "setup")
            if mode == "setup":
                self.assertTrue(any(call[-3:] == ("init", "--username", "chosen-admin") for call in app_calls))
            self.assertEqual(any(call[:2] == ("systemctl", "start") for call in calls), mode == "upgrade" and active)
            self.assertFalse(any("enable" in call for call in calls))
            if failure:
                self.assertEqual(unit.read_bytes(), b"previous unit")
            else:
                self.assertIn(str(shared), unit.read_text())
                self.assertIn(str(state), unit.read_text())
            self.assertFalse((binary.parent / ".xczs.candidate").exists())

    def test_setup_initializes_selected_username_without_starting(self):
        self.exercise_install("setup")

    def test_upgrade_preserves_config_and_restarts_only_previously_active(self):
        for active in [False, True]:
            with self.subTest(active=active):
                self.exercise_install("upgrade", active=active)

    def test_failed_upgrade_preserves_binary_unit_config_and_resumes_prior_service(self):
        self.exercise_install("upgrade", active=True, failure="check")

    def test_failed_restart_rolls_back_the_previous_binary_and_unit(self):
        self.exercise_install("upgrade", active=True, failure="start")


if __name__ == "__main__":
    unittest.main()

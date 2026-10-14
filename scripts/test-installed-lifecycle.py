#!/usr/bin/env python3
"""Archive/guard regression tests only; these do not count as installation tests."""
import importlib.util
import io
import os
import json
from pathlib import Path
import stat
import subprocess
import sys
import tarfile
import tempfile
import unittest
from types import SimpleNamespace
from unittest.mock import patch

SCRIPT = Path(__file__).with_name('check-installed-lifecycle.py')
spec = importlib.util.spec_from_file_location('installed_lifecycle', SCRIPT)
lifecycle = importlib.util.module_from_spec(spec)
spec.loader.exec_module(lifecycle)
SHA = 'a' * 40
PREFIX = 'xczs-0.1.0-linux-x86_64'


class ArchiveBoundary(unittest.TestCase):
    def setUp(self):
        self.work = tempfile.TemporaryDirectory()
        self.addCleanup(self.work.cleanup)
        self.archive = Path(self.work.name) / (PREFIX + '.tar.gz')

    def build(self, *, prefix=PREFIX, revision=SHA, extra=None):
        manifest = {'identity': {'product': 'xczs', 'source_revision': revision},
                    'services': [{'name': 'xczs.service'}]}
        with tarfile.open(self.archive, 'w:gz') as bundle:
            data = json.dumps(manifest).encode()
            member = tarfile.TarInfo(prefix + '/share/package.json')
            member.size = len(data)
            bundle.addfile(member, io.BytesIO(data))
            if extra:
                bundle.addfile(extra)
        self.checksum()

    def checksum(self):
        self.archive.with_name('SHA256SUMS').write_text(
            lifecycle.archive_digest(self.archive) + '  ' + self.archive.name + '\n')

    def test_current_envelope(self):
        self.build()
        self.assertEqual(lifecycle.verify_archive(self.archive, SHA), lifecycle.archive_digest(self.archive))

    def test_reject_wrong_revision(self):
        self.build(revision='b' * 40)
        with self.assertRaisesRegex(RuntimeError, 'candidate revision'):
            lifecycle.verify_archive(self.archive, SHA)

    def test_reject_wrong_envelope(self):
        self.build(prefix='flat')
        with self.assertRaisesRegex(RuntimeError, 'package envelope'):
            lifecycle.verify_archive(self.archive, SHA)

    def test_reject_checksum_tampering(self):
        self.build()
        self.archive.write_bytes(self.archive.read_bytes() + b'tamper')
        with self.assertRaisesRegex(RuntimeError, 'checksum mismatch'):
            lifecycle.verify_archive(self.archive, SHA)

    def test_reject_link(self):
        link = tarfile.TarInfo(PREFIX + '/link')
        link.type = tarfile.SYMTYPE
        link.linkname = '/etc/passwd'
        self.build(extra=link)
        with self.assertRaisesRegex(RuntimeError, 'regular package entries'):
            lifecycle.verify_archive(self.archive, SHA)

    def test_reject_parent_path(self):
        self.build(extra=tarfile.TarInfo(PREFIX + '/../escape'))
        with self.assertRaisesRegex(RuntimeError, 'Unsafe package path'):
            lifecycle.verify_archive(self.archive, SHA)

    def test_reject_duplicate(self):
        self.build(extra=tarfile.TarInfo(PREFIX + '/share/package.json'))
        with self.assertRaisesRegex(RuntimeError, 'Duplicate package path'):
            lifecycle.verify_archive(self.archive, SHA)

    def test_formal_extractor_selects_real_envelope(self):
        self.build()
        destination = Path(self.work.name) / 'unpacked'
        destination.mkdir()
        subprocess.run(['tar', '--extract', '--gzip', '--no-same-owner', '--file', str(self.archive),
                        '--directory', str(destination)], check=True)
        self.assertFalse((destination / 'share/package.json').exists(), 'Old flat path must fail')
        self.assertTrue((destination / PREFIX / 'share/package.json').is_file())
        formal = SCRIPT.with_name('check-formal-release-e2e.sh').read_text()
        self.assertIn('package="$package/xczs-$version-linux-x86_64"', formal)

    def test_real_extraction_preserves_package_directories_under_private_umask(self):
        with tarfile.open(self.archive, 'w:gz') as bundle:
            for name in (PREFIX, PREFIX + '/share'):
                directory = tarfile.TarInfo(name)
                directory.type, directory.mode = tarfile.DIRTYPE, 0o755
                bundle.addfile(directory)
        legacy, fixed = Path(self.work.name) / 'legacy', Path(self.work.name) / 'fixed'
        legacy.mkdir()
        fixed.mkdir()
        previous = os.umask(0o077)
        try:
            with tarfile.open(self.archive) as bundle:
                bundle.extractall(legacy, filter='data')
            package = lifecycle.extract_package(self.archive, fixed)
        finally:
            os.umask(previous)
        # This reproduces the observed CI preflight failure without executing
        # a fixture installer or changing the real package verifier.
        self.assertNotEqual(stat.S_IMODE((legacy / PREFIX / 'share').stat().st_mode), 0o755)
        self.assertEqual(stat.S_IMODE(package.stat().st_mode), 0o755)
        self.assertEqual(stat.S_IMODE((package / 'share').stat().st_mode), 0o755)

    def test_extraction_rejects_noncontract_directory_mode(self):
        with tarfile.open(self.archive, 'w:gz') as bundle:
            directory = tarfile.TarInfo(PREFIX)
            directory.type, directory.mode = tarfile.DIRTYPE, 0o777
            bundle.addfile(directory)
        with self.assertRaisesRegex(RuntimeError, 'directory mode must be 0755'):
            lifecycle.extract_package(self.archive, Path(self.work.name) / 'rejected')

    def test_install_parent_requires_every_ancestor_to_be_root_controlled(self):
        parent = Path('/var/lib')
        safe = SimpleNamespace(st_uid=0, st_mode=stat.S_IFDIR | 0o755)
        unsafe = [SimpleNamespace(st_uid=0, st_mode=stat.S_IFDIR | 0o777),
                  SimpleNamespace(st_uid=1001, st_mode=stat.S_IFDIR | 0o755),
                  SimpleNamespace(st_uid=0, st_mode=stat.S_IFLNK | 0o755)]
        with patch.object(Path, 'lstat', return_value=safe) as inspect:
            lifecycle.verify_install_parent(parent)
            self.assertEqual(inspect.call_count, 3)
        for bad in unsafe:
            for position in range(3):
                records = [safe, safe, safe]
                records[position] = bad
                with self.subTest(mode=bad.st_mode, uid=bad.st_uid, ancestor=position), \
                        patch.object(Path, 'lstat', side_effect=records):
                    with self.assertRaisesRegex(RuntimeError, 'root-controlled'):
                        lifecycle.verify_install_parent(parent)

    def test_config_permissions_follow_readonly_product_contract(self):
        directory = SimpleNamespace(st_uid=0, st_gid=987, st_mode=stat.S_IFDIR | 0o750)
        config = SimpleNamespace(st_uid=0, st_gid=987, st_mode=stat.S_IFREG | 0o640)
        with patch.object(Path, 'lstat', side_effect=[directory, config]):
            lifecycle.verify_config_permissions(Path('/installed'), 987)
        for changes in ({'st_uid': 987}, {'st_gid': 0},
                        {'st_mode': stat.S_IFREG | 0o600},
                        {'st_mode': stat.S_IFREG | 0o660},
                        {'st_mode': stat.S_IFLNK | 0o640}):
            bad = SimpleNamespace(**(vars(config) | changes))
            with self.subTest(changes=changes), \
                    patch.object(Path, 'lstat', side_effect=[directory, bad]):
                with self.assertRaisesRegex(RuntimeError, 'read-only configuration'):
                    lifecycle.verify_config_permissions(Path('/installed'), 987)

    def test_journal_diagnostics_only_return_public_whitelisted_fields(self):
        root = Path(self.work.name)
        (root / 'maintenance').mkdir()
        journal = root / 'maintenance/install.json'
        journal.write_text(json.dumps({'product': 'xczs', 'phase': 'staging',
                                       'password': 'never-include', 'unexpected': 'private'}))
        self.assertEqual(lifecycle.installation_observation(root),
                         {'product': 'xczs', 'installation_phase': 'staging'})
        journal.write_text(json.dumps({'product': 'xczs', 'phase': 'private-value'}))
        with self.assertRaisesRegex(RuntimeError, 'unknown public'):
            lifecycle.installation_observation(root)

    def test_failure_diagnostics_redact_exact_temporary_credentials(self):
        directory = Path(self.work.name) / 'diagnostics'
        password, token = 'temporary-password-123', 'temporary-token-456'
        with patch.dict(os.environ, {'LIFECYCLE_DIAGNOSTICS': str(directory)}), \
                patch.object(lifecycle, 'SECRETS', [password, token]), \
                patch.object(lifecycle, 'STAGE', 'regression'):
            with self.assertRaisesRegex(RuntimeError, 'regression'):
                lifecycle.run(sys.executable, '-c',
                              f"import sys; print('{password}'); print('{token}', file=sys.stderr); sys.exit(3)")
        evidence = (directory / 'lifecycle.jsonl').read_text()
        self.assertNotIn(password, evidence)
        self.assertNotIn(token, evidence)
        self.assertIn('[REDACTED]', evidence)
        self.assertEqual(json.loads(evidence)['exit_code'], 3)
        self.assertEqual(json.loads(evidence)['stage'], 'regression')

    def test_refuses_non_runner_before_reading_archive(self):
        result = subprocess.run([sys.executable, '-I', '-B', str(SCRIPT), '/not-an-archive', SHA],
                                env={}, capture_output=True, text=True)
        self.assertEqual(result.returncode, 1)
        self.assertIn('Requires root on a disposable GitHub-hosted runner', result.stderr)


if __name__ == '__main__':
    unittest.main()

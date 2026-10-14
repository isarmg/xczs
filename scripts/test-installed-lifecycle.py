#!/usr/bin/env python3
"""Archive/guard regression tests only; these do not count as installation tests."""
import importlib.util
import io
import os
import json
from pathlib import Path
import subprocess
import sys
import tarfile
import tempfile
import unittest
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

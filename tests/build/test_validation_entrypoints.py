"""Regression checks for real validation entry points, without booting a VM."""
import importlib.util
import os
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

import pytest

REPO = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location('vyoma_e2e_config', REPO / 'tests/e2e/conftest.py')
config = importlib.util.module_from_spec(spec)
spec.loader.exec_module(config)


class ValidationEntrypointTests(unittest.TestCase):
    def test_profile_failure_survives_tail(self):
        with tempfile.TemporaryDirectory() as temporary:
            stub = Path(temporary) / 'cargo-failure.sh'
            stub.write_text('#!/bin/sh\necho cargo-test-failed\nexit 37\n')
            result = subprocess.run(['make', '-o', 'image', 'check-profiles',
                                     f'DOCKER_RUN=bash {stub}'], cwd=REPO,
                                    capture_output=True, text=True)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn('cargo-test-failed', result.stdout)
            self.assertNotIn('PROFILES: OK', result.stdout)

    def test_smoke_builds_kernel_and_rootfs_before_boot(self):
        result = subprocess.run(['make', '-n', 'smoke'], cwd=REPO,
                                capture_output=True, text=True, check=True)
        output = result.stdout
        self.assertLess(output.index('bash base/modules/kernel.sh'), output.index('bash base/scripts/smoke-test.sh'))
        self.assertLess(output.index('bash base/modules/rootfs.sh'), output.index('bash base/scripts/smoke-test.sh'))

    def test_paths_are_repository_relative(self):
        with tempfile.TemporaryDirectory() as temporary, patch.dict(os.environ, {}, clear=True):
            previous = Path.cwd()
            try:
                os.chdir(temporary)
                self.assertEqual(config.artifact_path('BZIMAGE', 'out/bzImage'), str(REPO / 'out/bzImage'))
                os.environ['BZIMAGE'] = 'custom/kernel'
                self.assertEqual(config.artifact_path('BZIMAGE', 'out/bzImage'), str(REPO / 'custom/kernel'))
                os.environ['BZIMAGE'] = temporary + '/kernel'
                self.assertEqual(config.artifact_path('BZIMAGE', 'out/bzImage'), temporary + '/kernel')
            finally:
                os.chdir(previous)

    def test_missing_and_empty_images_fail_instead_of_skip(self):
        with tempfile.TemporaryDirectory() as temporary:
            base = Path(temporary)
            for missing in ('kernel', 'initrd', 'disk'):
                for empty in (False, True):
                    with self.subTest(missing=missing, empty=empty):
                        for name in ('kernel', 'initrd', 'disk'):
                            (base / name).write_bytes(b'artifact')
                        if empty:
                            (base / missing).write_bytes(b'')
                        else:
                            (base / missing).unlink()
                        with patch.dict(os.environ, {'BZIMAGE': str(base / 'kernel'),
                                                     'INITRAMFS': str(base / 'initrd'),
                                                     'DISK': str(base / 'disk')}, clear=True):
                            with self.assertRaises(pytest.fail.Exception):
                                next(config.vm.__wrapped__())

    def test_shell_smoke_and_gui_reject_missing_images_from_other_cwd(self):
        with tempfile.TemporaryDirectory() as temporary:
            for script in ('smoke-test.sh', 'test-e2e-gui.sh'):
                result = subprocess.run(['bash', str(REPO / 'base/scripts' / script)],
                                        cwd=temporary, capture_output=True, text=True,
                                        env={**os.environ, 'BZIMAGE': temporary + '/missing',
                                             'INITRAMFS': temporary + '/missing-initrd'})
                self.assertNotEqual(result.returncode, 0)
                self.assertIn('FAIL:', result.stdout + result.stderr)
                self.assertNotIn('SKIP:', result.stdout + result.stderr)


if __name__ == '__main__':
    unittest.main(verbosity=2)

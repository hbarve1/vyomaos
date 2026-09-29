"""Exercise actual rootfs assembly with disposable local artifact fixtures."""
import hashlib
import os
from pathlib import Path
import shutil
import subprocess
import tarfile
import tempfile
import unittest

REPO = Path(__file__).resolve().parents[2]


class RootfsTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='vyoma-rootfs-')
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        (self.root / 'base/modules').mkdir(parents=True)
        for name in ('base/config.sh', 'base/image-apps.sh', 'base/modules/rootfs.sh'):
            shutil.copy2(REPO / name, self.root / name)
        self.allowlist = self.root / 'base/r1-apps.txt'
        self.allowlist.write_text('# selected payload\nshell\n')
        self.supervisor = self.root / 'target/x86_64-unknown-linux-musl/release/supervisor'
        self.supervisor.parent.mkdir(parents=True)
        self.supervisor.write_text('#!/bin/sh\nexit 0\n')
        self.supervisor.chmod(0o755)
        self.app = self.root / 'apps/shell'
        self.wasm = self.app / 'target/wasm32-wasip2/release/shell.wasm'
        self.wasm.parent.mkdir(parents=True)
        self.wasm.write_bytes(b'\0asm\x01\0\0\0')
        self.manifest = self.app / 'vyoma.toml'
        self.manifest.write_text('[app]\nname="shell"\nversion="0.1.0"\nwasm="shell.wasm"\n')
        cache = self.root / 'out/cache'
        cache.mkdir(parents=True)
        busybox = cache / 'busybox-fixture-x86_64-musl'
        busybox.write_text('busybox fixture\n')
        runtime = self.root / 'wasmtime'
        runtime.write_text('runtime fixture\n')
        archive = cache / 'wasmtime-vfixture-x86_64-linux.tar.xz'
        with tarfile.open(archive, 'w:xz') as tar:
            tar.add(runtime, arcname='wasmtime-vfixture-x86_64-linux/wasmtime')
        digest = lambda path: hashlib.sha256(path.read_bytes()).hexdigest()
        (self.root / 'base/versions.sh').write_text(
            'KERNEL_VERSION=unused\nKERNEL_SHA256=unused\n'
            'BUSYBOX_VERSION=fixture\nWASMTIME_VERSION=fixture\n'
            f'BUSYBOX_SHA256={digest(busybox)}\nWASMTIME_SHA256={digest(archive)}\n'
        )
        self.image = self.root / 'out/initramfs.cpio.gz'
        self.image.write_bytes(b'previous image')
        self.env = dict(os.environ)

    def build(self, ok=True, cwd=None):
        result = subprocess.run(['bash', str(self.root / 'base/modules/rootfs.sh')],
                                cwd=cwd or self.root, env=self.env, text=True,
                                stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
        self.assertEqual(result.returncode == 0, ok, result.stdout)
        return result.stdout

    def rejected(self, diagnostic):
        self.assertIn(diagnostic, self.build(ok=False))
        self.assertEqual(self.image.read_bytes(), b'previous image')

    def test_only_selected_apps_packed_and_boot_matches(self):
        optional = self.root / 'apps/desktop/target/wasm32-wasip2/release'
        optional.mkdir(parents=True)
        (optional / 'desktop.wasm').write_bytes(b'optional')
        (self.root / 'apps/desktop/vyoma.toml').write_text('optional')
        self.build(cwd='/tmp')
        payload = self.root / 'out/rootfs'
        self.assertEqual([p.name for p in (payload / 'apps').iterdir()], ['shell'])
        boot = (payload / 'etc/vyoma/boot.toml').read_text()
        self.assertIn('/apps/shell/vyoma.toml', boot)
        self.assertNotIn('desktop', boot)
        self.assertNotIn('fall', (payload / 'init').read_text())
        archive = subprocess.run(['bash', '-o', 'pipefail', '-c',
                                  'gzip -dc "$1" | cpio -it --quiet', '_', str(self.image)],
                                 capture_output=True, text=True, check=True).stdout
        self.assertIn('apps/shell/shell.wasm', archive)
        self.assertNotIn('apps/desktop', archive)

    def test_missing_supervisor(self):
        self.supervisor.unlink()
        self.rejected('Required R1 supervisor')

    def test_non_executable_supervisor(self):
        self.supervisor.chmod(0o644)
        self.rejected('Required R1 supervisor')

    def test_missing_selected_wasm(self):
        self.wasm.unlink()
        self.rejected('shell.wasm')

    def test_empty_selected_wasm(self):
        self.wasm.write_bytes(b'')
        self.rejected('shell.wasm')

    def test_missing_selected_manifest(self):
        self.manifest.unlink()
        self.rejected('vyoma.toml')

    def test_added_selected_app_must_exist(self):
        self.allowlist.write_text('shell\nmissing\n')
        self.rejected('apps/missing/vyoma.toml')

    def test_invalid_duplicate_or_empty_allowlists_fail(self):
        for content in ('shell\nshell\n', '../escape\n', '# no apps\n', 'shell other\n'):
            with self.subTest(content=content):
                self.allowlist.write_text(content)
                self.rejected('ERROR:')

    def test_missing_allowlist_fails(self):
        self.allowlist.unlink()
        self.rejected('allowlist missing')

    def test_pack_failure_preserves_previous_image(self):
        binaries = self.root / 'bin'
        binaries.mkdir()
        cpio = binaries / 'cpio'
        cpio.write_text('#!/bin/sh\nexit 42\n')
        cpio.chmod(0o755)
        self.env['PATH'] = str(binaries) + ':' + self.env['PATH']
        self.build(ok=False)
        self.assertEqual(self.image.read_bytes(), b'previous image')
        self.assertEqual(list((self.root / 'out').glob('initramfs.cpio.gz.tmp.*')), [])


if __name__ == '__main__':
    unittest.main(verbosity=2)

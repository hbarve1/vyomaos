"""Exercise the production kernel script/Makefile with a tiny local source tarball.

Real tar, patch, make and digest checks run; the fixture kernel Makefile replaces
Linux compilation. Full Linux compilation/boot remains a separate acceptance run.
"""
import hashlib
import os
from pathlib import Path
import shutil
import subprocess
import tarfile
import tempfile
import unittest

REPO = Path(__file__).resolve().parents[2]


class KernelBuildTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="vyoma-kernel-")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        (self.root / "base/modules").mkdir(parents=True)
        (self.root / "base/patches/kernel").mkdir(parents=True)
        for name in ("base/config.sh", "base/modules/kernel.sh", "base/image-apps.sh", "base/r1-apps.txt", "Makefile"):
            shutil.copy2(REPO / name, self.root / name)
        self.config = self.root / "base/kernel.config"
        self.config.write_text("CONFIG_TEST_ONE=y\n")
        self.source = self.root / "fixture/linux-fixture"
        self.source.mkdir(parents=True)
        (self.source / "payload").write_text("original\n")
        (self.source / "Makefile").write_text(
            "allnoconfig:\n\tcp $(KCONFIG_ALLCONFIG) .config\n"
            "bzImage:\n\t! grep -q CONFIG_TEST_FAIL=y .config\n"
            "\tmkdir -p arch/x86/boot\n"
            "\tcat .config payload > arch/x86/boot/bzImage\n"
        )
        self.archive = self.root / "fixture.tar.xz"
        self.pack()
        config = self.root / "base/config.sh"
        config.write_text(config.read_text().replace(
            'https://www.kernel.org/pub/linux/kernel/v5.x/linux-$KERNEL_VERSION.tar.xz',
            self.archive.as_uri(),
        ))
        # This shim exercises Make's argument propagation, without a Docker daemon.
        self.bin = self.root / "bin"
        self.bin.mkdir()
        docker = self.bin / "docker"
        docker.write_text(
            '#!/usr/bin/env bash\nset -eu\n'
            '[[ "$1" != build ]] || exit 0\n'
            'while [[ "$1" != vyomaos-builder:* ]]; do shift; done\n'
            'shift\nexec "$@"\n'
        )
        docker.chmod(0o755)
        # Prerequisites of Make's image target, which the shim does not build.
        (self.root / "docker").mkdir()
        (self.root / "docker/Dockerfile").touch()
        (self.root / "rust-toolchain.toml").touch()

    def pack(self, version="fixture"):
        with tarfile.open(self.archive, "w:xz") as archive:
            archive.add(self.source, arcname="linux-fixture")
        digest = hashlib.sha256(self.archive.read_bytes()).hexdigest()
        (self.root / "base/versions.sh").write_text(
            f'readonly KERNEL_VERSION="{version}"\n'
            f'readonly KERNEL_SHA256="{digest}"\n'
        )

    def run_build(self, ok=True, via_make=False, **env):
        command = ["bash", "base/modules/kernel.sh"]
        if via_make:
            command = ["make", "kernel"] + [f"{key}={value}" for key, value in env.items()]
        result = subprocess.run(
            command, cwd=self.root, text=True, stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            env={**os.environ, "PATH": f"{self.bin}:{os.environ['PATH']}", **env},
        )
        self.assertEqual(result.returncode == 0, ok, result.stdout)
        return result.stdout

    def image(self):
        return (self.root / "out/bzImage").read_text()

    def test_initial_build_and_content_cache(self):
        self.run_build()
        self.assertIn("CONFIG_TEST_ONE=y", self.image())
        before = (self.root / "out/bzImage").stat().st_mtime_ns
        self.assertIn("reusing", self.run_build())
        self.assertEqual(before, (self.root / "out/bzImage").stat().st_mtime_ns)

    def test_config_content_change_even_with_old_mtime(self):
        self.run_build()
        self.config.write_text("CONFIG_TEST_TWO=y\n")
        os.utime(self.config, (1, 1))
        self.run_build()
        self.assertIn("CONFIG_TEST_TWO=y", self.image())
        self.assertNotIn("CONFIG_TEST_ONE", self.image())

    def test_make_propagates_config_and_switches_back(self):
        other = self.root / "other.config"
        other.write_text("CONFIG_TEST_OTHER=y\n")
        self.run_build(via_make=True, KERNEL_CONFIG="other.config")
        self.assertIn("CONFIG_TEST_OTHER=y", self.image())
        self.run_build(via_make=True)
        self.assertNotIn("CONFIG_TEST_OTHER", self.image())

    def test_make_rebuilds_deleted_image_despite_stamp(self):
        self.run_build(via_make=True)
        (self.root / "out/bzImage").unlink()
        self.run_build(via_make=True)
        self.assertIn("CONFIG_TEST_ONE=y", self.image())

    def test_corrupt_output_is_rebuilt(self):
        self.run_build()
        (self.root / "out/bzImage").write_text("corruption")
        self.run_build()
        self.assertIn("CONFIG_TEST_ONE=y", self.image())

    def test_corrupt_published_config_is_rebuilt(self):
        self.run_build()
        (self.root / "out/kernel.config").write_text("corruption")
        self.run_build()
        self.assertEqual(self.config.read_text(), (self.root / "out/kernel.config").read_text())

    def test_source_version_and_digest_change(self):
        self.run_build()
        (self.source / "payload").write_text("new version\n")
        self.pack(version="fixture2")
        self.run_build()
        self.assertIn("new version", self.image())
        self.assertEqual(len(list((self.root / "out/kernel").glob("source-*"))), 2)

    def test_patches_are_ordered_and_removal_reverts(self):
        self.run_build()
        patch_dir = self.root / "base/patches/kernel"
        for name, before, after in [("01", "original", "first"), ("02", "first", "second")]:
            (patch_dir / f"{name}.patch").write_text(
                f"--- a/payload\n+++ b/payload\n@@ -1 +1 @@\n-{before}\n+{after}\n"
            )
        self.run_build()
        self.assertIn("second", self.image())
        (patch_dir / "02.patch").unlink()
        self.run_build()
        self.assertIn("first", self.image())
        (patch_dir / "01.patch").unlink()
        self.run_build()
        self.assertIn("original", self.image())

    def test_failed_build_invalidates_stamp_and_can_recover(self):
        self.run_build()
        previous = self.image()
        self.config.write_text("CONFIG_TEST_FAIL=y\n")
        self.run_build(ok=False)
        self.assertFalse((self.root / "out/.kernel.stamp").exists())
        self.assertEqual(self.image(), previous)
        self.config.write_text("CONFIG_RECOVERED=y\n")
        self.run_build()
        self.assertIn("CONFIG_RECOVERED=y", self.image())

    def test_bad_patch_does_not_publish_stamp(self):
        self.run_build()
        (self.root / "base/patches/kernel/bad.patch").write_text("not a patch\n")
        self.run_build(ok=False)
        self.assertFalse((self.root / "out/.kernel.stamp").exists())

    def test_sourcing_module_does_not_build_or_change_directory(self):
        result = subprocess.run(
            ["bash", "-c", 'source base/modules/kernel.sh; test "$PWD" = "$PROJECT_ROOT"'],
            cwd=self.root, capture_output=True, text=True,
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertFalse((self.root / "out").exists())

    def test_missing_config_fails(self):
        output = self.run_build(ok=False, KERNEL_CONFIG="missing.config")
        self.assertIn("Kernel config not found", output)

    def test_unsupported_platforms_fail_in_script_and_make(self):
        for platform in ("typo", "iot-rpi", "mcu-arm-cortex-m", "server-arm64"):
            for via_make in (False, True):
                with self.subTest(platform=platform, via_make=via_make):
                    self.assertIn("Unsupported PLATFORM", self.run_build(
                        ok=False, via_make=via_make, PLATFORM=platform,
                    ))
        self.assertFalse((self.root / "out").exists())

    def test_corrupt_source_cache_fails_after_input_change(self):
        self.run_build()
        (self.root / "out/linux-fixture.tar.xz").write_text("corruption")
        self.config.write_text("CONFIG_CHANGED=y\n")
        self.assertIn("SHA-256 mismatch", self.run_build(ok=False))
        self.assertFalse((self.root / "out/.kernel.stamp").exists())


if __name__ == "__main__":
    unittest.main(verbosity=2)

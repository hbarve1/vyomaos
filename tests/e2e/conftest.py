"""Pytest fixtures for VyomaOS E2E tests.

Provides session-scoped VM fixture that boots QEMU and function-scoped
screenshot fixture that captures the current display state.
"""

import os
import sys
import time
from pathlib import Path
import pytest

# Ensure the tests/e2e directory is on the path for local imports
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

from harness import QmpClient


REPO_ROOT = Path(__file__).resolve().parents[2]


def artifact_path(variable, default):
    """Resolve defaults and relative overrides from the repository, never cwd."""
    value = os.environ.get(variable, default)
    if value is None:
        return None
    path = Path(value)
    return str(path if path.is_absolute() else REPO_ROOT / path)


@pytest.fixture(scope="session")
def vm():
    """Boot a VyomaOS QEMU VM, wait for all apps to spawn, yield the QmpClient.

    Environment variables:
        BZIMAGE: Path to kernel image (default: out/bzImage)
        INITRAMFS: Path to initramfs (default: out/initramfs.cpio.gz)
        DISK: Path to data disk image (optional)
    """
    kernel = artifact_path("BZIMAGE", "out/bzImage")
    initrd = artifact_path("INITRAMFS", "out/initramfs.cpio.gz")
    disk = artifact_path("DISK", None)

    if not os.path.isfile(kernel) or os.path.getsize(kernel) == 0:
        pytest.fail(f"Kernel not found: {kernel} (run 'make build' first)")
    if not os.path.isfile(initrd) or os.path.getsize(initrd) == 0:
        pytest.fail(f"Initramfs not found: {initrd} (run 'make build' first)")

    if disk is not None and (not os.path.isfile(disk) or os.path.getsize(disk) == 0):
        pytest.fail(f"Configured disk not found or empty: {disk}")

    client = QmpClient(kernel=kernel, initrd=initrd, disk=disk)
    client.start()

    try:
        client.wait_for_serial(r"\[lifecycle\].*all apps spawned", timeout=30)
        client.mark_boot_done()
    except TimeoutError:
        client.shutdown()
        raise RuntimeError(
            "Boot timeout: '[lifecycle] all apps spawned' not seen within 30s. "
            f"Check serial log: {client.serial_log_path}"
        )

    yield client

    client.shutdown()


@pytest.fixture(scope="function")
def screenshot(vm, tmp_path):
    """Take a screendump from the running VM and return the file path.

    The screenshot is saved as a PPM file in the test's tmp directory.
    """
    path = str(tmp_path / "screenshot.ppm")
    vm.screendump(path)
    # Give QEMU a moment to flush the file
    time.sleep(0.2)
    return path

import os
import subprocess
import time
import pytest
from harness import QmpClient


def _tail_log(path: str, n: int = 20) -> str:
    try:
        with open(path) as f:
            return "".join(f.readlines()[-n:])
    except FileNotFoundError:
        return "(serial log not found)"


@pytest.fixture(scope="session")
def vm(tmp_path_factory):
    tmp        = tmp_path_factory.mktemp("vyoma")
    qmp_sock   = str(tmp / "qmp.sock")
    serial_log = str(tmp / "serial.log")
    screenshot = str(tmp / "screen.ppm")

    bzimage   = os.environ.get("BZIMAGE",   "out/bzImage")
    initramfs = os.environ.get("INITRAMFS", "out/initramfs.cpio.gz")

    proc = subprocess.Popen(
        [
            "qemu-system-x86_64",
            "-kernel",  bzimage,
            "-initrd",  initramfs,
            "-m",       "512M",
            "-no-reboot",
            "-append",  "console=ttyS0 quiet",
            "-device",  "virtio-vga",
            "-display", "none",
            "-qmp",     f"unix:{qmp_sock},server,nowait",
            "-serial",  f"file:{serial_log}",
        ],
        stderr=subprocess.DEVNULL,
    )

    time.sleep(5)
    if proc.returncode is not None:
        raise RuntimeError(
            f"QEMU failed to start (exit {proc.returncode})\n" + _tail_log(serial_log)
        )

    client = QmpClient(qmp_sock, serial_log, screenshot)
    try:
        client.wait_log(r"\[lifecycle\].*all apps spawned", timeout=45)
    except TimeoutError:
        proc.terminate()
        proc.wait(timeout=5)
        raise RuntimeError(
            "Boot timeout: '[lifecycle] all apps spawned' not seen within 45s\n"
            + _tail_log(serial_log)
        )

    time.sleep(3)
    client.connect()

    yield client

    client.close()
    proc.terminate()
    proc.wait(timeout=5)

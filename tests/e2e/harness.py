"""QmpClient — QEMU Machine Protocol client for E2E testing.

Launches QEMU with QMP over a Unix domain socket and serial output
piped to a file. Provides methods to send keystrokes, take screenshots,
wait for serial output patterns, and cleanly shut down the VM.
"""

import json
import os
import re
import shutil
import socket
import subprocess
import time


class QmpClient:
    """Manages a QEMU instance via QMP for E2E testing."""

    def __init__(self, kernel: str, initrd: str, disk: str = None):
        self._kernel = kernel
        self._initrd = initrd
        self._disk = disk
        self._proc = None
        self._sock = None
        self._rbuf = b""
        self._qmp_path = "/tmp/vyoma-qmp.sock"
        self._serial_log = "/tmp/vyoma-serial.log"
        self._boot_time = None
        self._boot_done_time = None

        # Clean up stale socket/pipe files
        for path in (self._qmp_path, self._serial_log):
            if os.path.exists(path):
                os.unlink(path)

    def start(self):
        """Launch QEMU and connect to QMP socket."""
        cmd = [
            "qemu-system-x86_64",
            "-kernel", self._kernel,
            "-initrd", self._initrd,
            "-m", "512M",
            "-no-reboot",
            "-append", "console=ttyS0 quiet",
            "-device", "virtio-vga",
            "-display", "none",
            "-qmp", f"unix:{self._qmp_path},server,wait=off",
            "-serial", f"file:{self._serial_log}",
        ]

        # Use KVM if available
        if os.path.exists("/dev/kvm"):
            cmd.extend(["-enable-kvm"])

        if self._disk:
            cmd.extend([
                "-drive", f"file={self._disk},format=raw,if=virtio",
            ])

        self._boot_time = time.time()
        self._proc = subprocess.Popen(
            cmd,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )

        # Connect to QMP
        self._connect_qmp()

    def _connect_qmp(self):
        """Wait for QMP socket and perform handshake."""
        deadline = time.time() + 30
        while not os.path.exists(self._qmp_path):
            if time.time() > deadline:
                raise TimeoutError(
                    f"QMP socket {self._qmp_path} not found within 30s"
                )
            if self._proc.poll() is not None:
                raise RuntimeError(
                    f"QEMU exited with code {self._proc.returncode} before QMP connected"
                )
            time.sleep(0.1)

        self._sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        self._sock.settimeout(10)
        self._sock.connect(self._qmp_path)

        # QMP handshake: read greeting, send qmp_capabilities
        self._recv_msg()  # greeting
        self._send_cmd({"execute": "qmp_capabilities"})
        self._recv_msg()  # {"return": {}}

    def _send_cmd(self, obj: dict):
        """Send a JSON command over QMP."""
        data = json.dumps(obj).encode() + b"\n"
        self._sock.sendall(data)

    def _recv_msg(self) -> dict:
        """Read one JSON message from QMP, skipping async events."""
        while True:
            while b"\n" not in self._rbuf:
                chunk = self._sock.recv(4096)
                if not chunk:
                    raise RuntimeError("QMP connection closed unexpectedly")
                self._rbuf += chunk
            line, self._rbuf = self._rbuf.split(b"\n", 1)
            msg = json.loads(line)
            # Skip asynchronous event messages
            if "event" not in msg:
                return msg

    def send_key(self, key: str):
        """Send a key press+release via QMP input-send-event.

        Args:
            key: QMP key name (e.g. 'ret', 'a', 'spc', 'backspace')
        """
        for down in (True, False):
            self._send_cmd({
                "execute": "input-send-event",
                "arguments": {
                    "events": [{
                        "type": "key",
                        "data": {
                            "down": down,
                            "key": {"type": "qcode", "data": key},
                        },
                    }],
                },
            })
            self._recv_msg()

    def screendump(self, path: str):
        """Take a screenshot and save as PPM at the given path.

        Args:
            path: Output file path for the screenshot (PPM format).
        """
        self._send_cmd({
            "execute": "screendump",
            "arguments": {"filename": path},
        })
        resp = self._recv_msg()
        if "error" in resp:
            raise RuntimeError(f"screendump failed: {resp['error']}")

        # Wait for QEMU to write the file
        deadline = time.time() + 5
        while time.time() < deadline:
            if os.path.exists(path) and os.path.getsize(path) > 0:
                return
            time.sleep(0.1)
        raise RuntimeError(f"Screenshot file not produced within 5s: {path}")

    def wait_for_serial(self, pattern: str, timeout: float = 30) -> str:
        """Wait for a regex pattern to appear in serial output.

        Args:
            pattern: Regex pattern to match against serial log lines.
            timeout: Maximum seconds to wait.

        Returns:
            The first matching line.

        Raises:
            TimeoutError: If pattern not found within timeout.
        """
        regex = re.compile(pattern)
        deadline = time.time() + timeout
        while time.time() < deadline:
            try:
                with open(self._serial_log) as f:
                    for line in f:
                        if regex.search(line):
                            return line.rstrip()
            except FileNotFoundError:
                pass
            time.sleep(0.2)
        raise TimeoutError(
            f"Pattern {pattern!r} not found in serial output within {timeout}s"
        )

    def mark_boot_done(self):
        """Record the moment boot completed (all apps spawned).

        Called by the conftest fixture after wait_for_serial succeeds.
        """
        self._boot_done_time = time.time()

    @property
    def boot_duration(self) -> float:
        """Seconds between QEMU start and boot completion.

        If mark_boot_done() was called, returns the recorded duration.
        Otherwise falls back to elapsed time since QEMU was started
        (which keeps ticking and is only useful as an upper bound).
        """
        if self._boot_time is None:
            return 0.0
        if self._boot_done_time is not None:
            return self._boot_done_time - self._boot_time
        return time.time() - self._boot_time

    @property
    def serial_log_path(self) -> str:
        """Path to the serial output log file."""
        return self._serial_log

    def shutdown(self):
        """Cleanly shut down QEMU and release resources."""
        if self._sock:
            try:
                self._send_cmd({"execute": "quit"})
            except Exception:
                pass
            try:
                self._sock.close()
            except Exception:
                pass
            self._sock = None

        if self._proc:
            try:
                self._proc.terminate()
                self._proc.wait(timeout=10)
            except Exception:
                self._proc.kill()
                self._proc.wait(timeout=5)
            self._proc = None

        # Clean up socket file
        if os.path.exists(self._qmp_path):
            try:
                os.unlink(self._qmp_path)
            except OSError:
                pass

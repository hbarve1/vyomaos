import json
import os
import re
import socket
import time
from PIL import Image

SCREEN_W = 1440
SCREEN_H = 900


class QmpClient:
    def __init__(self, qmp_sock: str, serial_log: str, screenshot_tmp: str):
        self._sock_path      = qmp_sock
        self._serial_log     = serial_log
        self._screenshot_tmp = screenshot_tmp
        self._sock           = None
        self._rbuf           = b""

    def connect(self):
        deadline = time.time() + 30
        while not os.path.exists(self._sock_path):
            if time.time() > deadline:
                raise TimeoutError(f"QMP socket not found within 30s: {self._sock_path}")
            time.sleep(0.1)
        self._sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        self._sock.connect(self._sock_path)
        try:
            self._recv()                                 # consume QMP greeting
            self._send({"execute": "qmp_capabilities"})
            self._recv()                                 # consume {"return": {}}
        except Exception:
            self.close()
            raise

    def close(self):
        if self._sock:
            self._sock.close()
            self._sock = None

    def _send(self, obj: dict):
        self._sock.sendall(json.dumps(obj).encode() + b"\n")

    def _recv(self) -> dict:
        # Note: QEMU may send async events between responses; skip them.
        while True:
            while b"\n" not in self._rbuf:
                chunk = self._sock.recv(4096)
                if not chunk:
                    raise RuntimeError("QMP connection closed unexpectedly")
                self._rbuf += chunk
            line, self._rbuf = self._rbuf.split(b"\n", 1)
            msg = json.loads(line)
            if "event" not in msg:
                return msg

    def _input_event(self, event: dict):
        self._send({"execute": "input-send-event", "arguments": {"events": [event]}})
        self._recv()

    def move(self, x: int, y: int):
        self._input_event({"type": "abs", "data": {"axis": "x", "value": x * 32767 // SCREEN_W}})
        self._input_event({"type": "abs", "data": {"axis": "y", "value": y * 32767 // SCREEN_H}})

    def click(self, x: int, y: int):
        self.move(x, y)
        self._input_event({"type": "btn", "data": {"down": True,  "button": "left"}})
        self._input_event({"type": "btn", "data": {"down": False, "button": "left"}})

    def key(self, key_name: str):
        self._input_event({"type": "key", "data": {"down": True,  "key": {"type": "qcode", "data": key_name}}})
        self._input_event({"type": "key", "data": {"down": False, "key": {"type": "qcode", "data": key_name}}})

    def screenshot(self) -> Image.Image:
        self._send({"execute": "screendump", "arguments": {"filename": self._screenshot_tmp}})
        resp = self._recv()
        if "error" in resp:
            raise RuntimeError(f"QMP screendump error: {resp['error']}")
        deadline = time.time() + 2
        while not os.path.exists(self._screenshot_tmp):
            if time.time() > deadline:
                raise RuntimeError(
                    f"Screenshot not produced within 2s: {self._screenshot_tmp}"
                )
            time.sleep(0.05)
        img = Image.open(self._screenshot_tmp)
        img.load()
        return img

    def wait_log(self, pattern: str, timeout: int = 10) -> str:
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
            time.sleep(0.1)
        raise TimeoutError(
            f"Pattern {pattern!r} not seen in {self._serial_log} within {timeout}s"
        )

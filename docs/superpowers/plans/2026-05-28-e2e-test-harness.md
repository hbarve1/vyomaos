# VyomaOS E2E Test Harness Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a Python + pytest E2E test harness that boots VyomaOS in QEMU, injects keyboard/mouse input via QMP, takes screenshots, and asserts on visual and log state.

**Architecture:** Seven files under `tests/e2e/` — `requirements.txt`, `assertions.py`, `harness.py` (QmpClient), `conftest.py` (session fixture), and three scenario test files. Unit tests for `assertions.py` and `harness.py` run without QEMU using mocks. Scenario tests require a live VM. A new `test-e2e` Makefile target runs the full suite inside the existing Docker image.

**Tech Stack:** Python 3, pytest≥7.0, Pillow≥9.0, QEMU QMP Unix-socket protocol, existing Docker builder image (Ubuntu 22.04).

---

## File Map

| File | Purpose | New/Modify |
|------|---------|------------|
| `tests/e2e/requirements.txt` | pytest + Pillow deps | New |
| `tests/e2e/assertions.py` | Pixel assertion helpers | New |
| `tests/e2e/test_assertions.py` | Unit tests for assertions.py | New |
| `tests/e2e/harness.py` | QmpClient — QMP socket wrapper | New |
| `tests/e2e/test_harness.py` | Unit tests for QmpClient | New |
| `tests/e2e/conftest.py` | Session-scoped VM fixture | New |
| `tests/e2e/test_boot.py` | Boot scenario tests | New |
| `tests/e2e/test_window.py` | Drag scenario test | New |
| `tests/e2e/test_input.py` | Keyboard scenario test | New |
| `Makefile` | Add `test-e2e` target + phony | Modify |

---

### Task 1: Scaffold `tests/e2e/` + `requirements.txt`

**Files:**
- Create: `tests/e2e/requirements.txt`

- [ ] **Step 1: Create directory and requirements file**

```bash
mkdir -p tests/e2e
```

Create `tests/e2e/requirements.txt`:
```
pytest>=7.0
Pillow>=9.0
```

- [ ] **Step 2: Verify pip can resolve the file**

```bash
pip install --dry-run -r tests/e2e/requirements.txt 2>&1 | head -5
```
Expected: resolves `pytest` and `Pillow`, no errors.

- [ ] **Step 3: Commit**

```bash
git add tests/e2e/requirements.txt
git commit -m "feat(e2e): scaffold tests/e2e with requirements.txt"
```

---

### Task 2: `assertions.py` with unit tests (TDD)

**Files:**
- Create: `tests/e2e/test_assertions.py`
- Create: `tests/e2e/assertions.py`

- [ ] **Step 1: Write the failing tests**

Create `tests/e2e/test_assertions.py`:
```python
import os
import sys
import pytest
from PIL import Image

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from assertions import (
    DESKTOP_BG, MENUBAR_BG, TOLERANCE,
    assert_pixel, assert_region_color, assert_no_ghost,
    assert_log_contains, assert_min_unique_colors,
)


def _solid(w, h, rgb):
    return Image.new("RGB", (w, h), rgb)


def test_assert_pixel_exact_match():
    img = _solid(10, 10, (100, 150, 200))
    assert_pixel(img, 5, 5, (100, 150, 200))


def test_assert_pixel_within_tolerance():
    img = _solid(10, 10, (100, 100, 100))
    assert_pixel(img, 0, 0, (90, 90, 90), tolerance=15)


def test_assert_pixel_fails_outside_tolerance():
    img = _solid(10, 10, (100, 100, 100))
    with pytest.raises(AssertionError, match=r"Pixel \(5,5\)"):
        assert_pixel(img, 5, 5, (0, 0, 0), tolerance=15)


def test_assert_region_color_solid_match():
    img = _solid(100, 100, DESKTOP_BG)
    assert_region_color(img, 0, 0, 100, 100, DESKTOP_BG, min_ratio=0.9)


def test_assert_region_color_fails_wrong_color():
    img = _solid(100, 100, (255, 0, 0))
    with pytest.raises(AssertionError, match="Region"):
        assert_region_color(img, 0, 0, 100, 100, DESKTOP_BG, min_ratio=0.5)


def test_assert_no_ghost_passes_on_desktop_bg():
    img = _solid(100, 100, DESKTOP_BG)
    assert_no_ghost(img, 10, 10, 50, 50)


def test_assert_no_ghost_fails_on_non_bg():
    img = _solid(100, 100, (255, 128, 0))
    with pytest.raises(AssertionError):
        assert_no_ghost(img, 10, 10, 50, 50)


def test_assert_log_contains_found(tmp_path):
    log = tmp_path / "serial.log"
    log.write_text("[lifecycle] all apps spawned\n")
    assert_log_contains(str(log), r"\[lifecycle\].*all apps spawned")


def test_assert_log_contains_not_found(tmp_path):
    log = tmp_path / "serial.log"
    log.write_text("nothing here\n")
    with pytest.raises(AssertionError, match="not found"):
        assert_log_contains(str(log), r"missing_pattern", timeout=0.2)


def test_assert_log_contains_missing_file(tmp_path):
    with pytest.raises(AssertionError, match="not found"):
        assert_log_contains(str(tmp_path / "no.log"), r"pattern", timeout=0.2)


def test_assert_min_unique_colors_pass():
    img = Image.new("RGB", (200, 200))
    pixels = [((x * 13 + y * 7) % 256, (y * 11) % 256, (x + y * 3) % 256)
              for y in range(200) for x in range(200)]
    img.putdata(pixels)
    assert_min_unique_colors(img, 30)


def test_assert_min_unique_colors_fails_solid():
    img = _solid(100, 100, (50, 50, 50))
    with pytest.raises(AssertionError, match="unique color buckets"):
        assert_min_unique_colors(img, 30)
```

- [ ] **Step 2: Run tests — expect ImportError**

```bash
cd tests/e2e && pip install -q pytest Pillow && pytest test_assertions.py -v
```
Expected: `ModuleNotFoundError: No module named 'assertions'`

- [ ] **Step 3: Create `tests/e2e/assertions.py`**

```python
import re
import time
from PIL import Image

DESKTOP_BG = (28, 28, 30)
MENUBAR_BG = (20, 20, 22)
TOLERANCE  = 15


def _matches(actual: tuple, expected: tuple, tolerance: int) -> bool:
    return all(abs(a - e) <= tolerance for a, e in zip(actual, expected))


def assert_pixel(img: Image.Image, x: int, y: int, rgb: tuple, tolerance: int = TOLERANCE):
    actual = img.getpixel((x, y))[:3]
    if not _matches(actual, rgb, tolerance):
        raise AssertionError(
            f"Pixel ({x},{y}): expected RGB{rgb} ±{tolerance}, got RGB{actual}"
        )


def assert_region_color(
    img: Image.Image,
    x: int, y: int, w: int, h: int,
    rgb: tuple,
    min_ratio: float = 0.7,
    tolerance: int = TOLERANCE,
):
    match_count = 0
    total = 0
    for py in range(y, min(y + h, img.height), 5):
        for px in range(x, min(x + w, img.width), 5):
            total += 1
            if _matches(img.getpixel((px, py))[:3], rgb, tolerance):
                match_count += 1
    if total == 0:
        raise AssertionError(f"Region ({x},{y},{w},{h}) has no pixels")
    ratio = match_count / total
    if ratio < min_ratio:
        raise AssertionError(
            f"Region ({x},{y},{w},{h}): expected ≥{min_ratio:.0%} pixels matching "
            f"RGB{rgb} ±{tolerance}, got {ratio:.0%} ({match_count}/{total})"
        )


def assert_no_ghost(img: Image.Image, x: int, y: int, w: int, h: int):
    assert_region_color(img, x, y, w, h, DESKTOP_BG, min_ratio=0.7)


def assert_log_contains(serial_log: str, pattern: str, timeout: int = 5):
    regex = re.compile(pattern)
    deadline = time.time() + timeout
    while time.time() < deadline:
        try:
            with open(serial_log) as f:
                for line in f:
                    if regex.search(line):
                        return
        except FileNotFoundError:
            pass
        time.sleep(0.1)
    raise AssertionError(
        f"Pattern {pattern!r} not found in {serial_log} within {timeout}s"
    )


def assert_min_unique_colors(img: Image.Image, min_count: int = 30):
    buckets: set = set()
    for py in range(0, img.height, 5):
        for px in range(0, img.width, 5):
            r, g, b = img.getpixel((px, py))[:3]
            buckets.add((r >> 3, g >> 3, b >> 3))
    if len(buckets) < min_count:
        raise AssertionError(
            f"Expected ≥{min_count} unique color buckets (5-bit), got {len(buckets)}"
        )
```

- [ ] **Step 4: Run tests — expect 12 passed**

```bash
cd tests/e2e && pytest test_assertions.py -v
```
Expected:
```
test_assertions.py::test_assert_pixel_exact_match PASSED
test_assertions.py::test_assert_pixel_within_tolerance PASSED
test_assertions.py::test_assert_pixel_fails_outside_tolerance PASSED
test_assertions.py::test_assert_region_color_solid_match PASSED
test_assertions.py::test_assert_region_color_fails_wrong_color PASSED
test_assertions.py::test_assert_no_ghost_passes_on_desktop_bg PASSED
test_assertions.py::test_assert_no_ghost_fails_on_non_bg PASSED
test_assertions.py::test_assert_log_contains_found PASSED
test_assertions.py::test_assert_log_contains_not_found PASSED
test_assertions.py::test_assert_log_contains_missing_file PASSED
test_assertions.py::test_assert_min_unique_colors_pass PASSED
test_assertions.py::test_assert_min_unique_colors_fails_solid PASSED
12 passed
```

- [ ] **Step 5: Commit**

```bash
git add tests/e2e/assertions.py tests/e2e/test_assertions.py
git commit -m "feat(e2e): assertions.py — pixel, region, log, color helpers with 12 unit tests"
```

---

### Task 3: `harness.py` (QmpClient) with unit tests (TDD)

**Files:**
- Create: `tests/e2e/test_harness.py`
- Create: `tests/e2e/harness.py`

- [ ] **Step 1: Write the failing tests**

Create `tests/e2e/test_harness.py`:
```python
import json
import os
import sys
import pytest
from unittest.mock import MagicMock, patch

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from harness import QmpClient

GREETING = json.dumps({"QMP": {"version": {}, "capabilities": []}}).encode() + b"\n"
OK       = json.dumps({"return": {}}).encode() + b"\n"


def _make_client(tmp_path):
    return QmpClient(
        str(tmp_path / "qmp.sock"),
        str(tmp_path / "serial.log"),
        str(tmp_path / "screen.ppm"),
    )


def _connected(tmp_path, extra=()):
    """Return (client, fake_socket) with connect() already called."""
    fake = MagicMock()
    fake.recv.side_effect = [GREETING, OK] + list(extra)
    with patch("harness.socket.socket") as cls, \
         patch("harness.os.path.exists", return_value=True):
        cls.return_value = fake
        c = _make_client(tmp_path)
        c.connect()
    return c, fake


def test_connect_sends_qmp_capabilities(tmp_path):
    c, fake = _connected(tmp_path)
    sent = b"".join(call.args[0] for call in fake.sendall.call_args_list)
    assert b'"qmp_capabilities"' in sent


def test_move_sends_two_abs_events(tmp_path):
    c, fake = _connected(tmp_path, extra=[OK, OK])
    c.move(720, 450)
    sent = b"".join(call.args[0] for call in fake.sendall.call_args_list)
    assert sent.count(b'"input-send-event"') == 2
    assert b'"axis": "x"' in sent
    assert b'"axis": "y"' in sent


def test_click_sends_four_events(tmp_path):
    c, fake = _connected(tmp_path, extra=[OK] * 4)
    c.click(100, 200)
    sent = b"".join(call.args[0] for call in fake.sendall.call_args_list)
    assert sent.count(b'"input-send-event"') == 4


def test_key_sends_two_events_with_qcode(tmp_path):
    c, fake = _connected(tmp_path, extra=[OK, OK])
    c.key("ret")
    sent = b"".join(call.args[0] for call in fake.sendall.call_args_list)
    assert sent.count(b'"input-send-event"') == 2
    assert b'"ret"' in sent
    assert b'"qcode"' in sent


def test_wait_log_returns_matching_line(tmp_path):
    (tmp_path / "serial.log").write_text("[lifecycle] all apps spawned\n")
    c = _make_client(tmp_path)
    result = c.wait_log(r"\[lifecycle\].*all apps spawned", timeout=1)
    assert "all apps spawned" in result


def test_wait_log_raises_timeout(tmp_path):
    (tmp_path / "serial.log").write_text("nothing\n")
    c = _make_client(tmp_path)
    with pytest.raises(TimeoutError, match="not seen"):
        c.wait_log(r"never_matches", timeout=0.2)


def test_close_closes_socket(tmp_path):
    c, fake = _connected(tmp_path)
    c.close()
    fake.close.assert_called_once()
```

- [ ] **Step 2: Run tests — expect ImportError**

```bash
cd tests/e2e && pytest test_harness.py -v
```
Expected: `ModuleNotFoundError: No module named 'harness'`

- [ ] **Step 3: Create `tests/e2e/harness.py`**

```python
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
        self._sock_path     = qmp_sock
        self._serial_log    = serial_log
        self._screenshot_tmp = screenshot_tmp
        self._sock          = None
        self._rbuf          = b""

    def connect(self):
        deadline = time.time() + 30
        while not os.path.exists(self._sock_path):
            if time.time() > deadline:
                raise TimeoutError(f"QMP socket not found within 30s: {self._sock_path}")
            time.sleep(0.1)
        self._sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        self._sock.connect(self._sock_path)
        self._recv()                                 # consume QMP greeting
        self._send({"execute": "qmp_capabilities"})
        self._recv()                                 # consume {"return": {}}

    def close(self):
        if self._sock:
            self._sock.close()
            self._sock = None

    def _send(self, obj: dict):
        self._sock.sendall(json.dumps(obj).encode() + b"\n")

    def _recv(self) -> dict:
        while b"\n" not in self._rbuf:
            chunk = self._sock.recv(4096)
            if not chunk:
                raise RuntimeError("QMP connection closed unexpectedly")
            self._rbuf += chunk
        line, self._rbuf = self._rbuf.split(b"\n", 1)
        return json.loads(line)

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
        return Image.open(self._screenshot_tmp)

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
```

- [ ] **Step 4: Run tests — expect 7 passed**

```bash
cd tests/e2e && pytest test_harness.py -v
```
Expected:
```
test_harness.py::test_connect_sends_qmp_capabilities PASSED
test_harness.py::test_move_sends_two_abs_events PASSED
test_harness.py::test_click_sends_four_events PASSED
test_harness.py::test_key_sends_two_events_with_qcode PASSED
test_harness.py::test_wait_log_returns_matching_line PASSED
test_harness.py::test_wait_log_raises_timeout PASSED
test_harness.py::test_close_closes_socket PASSED
7 passed
```

- [ ] **Step 5: Run both unit test files together**

```bash
cd tests/e2e && pytest test_assertions.py test_harness.py -v
```
Expected: `19 passed`

- [ ] **Step 6: Commit**

```bash
git add tests/e2e/harness.py tests/e2e/test_harness.py
git commit -m "feat(e2e): harness.py QmpClient with 7 unit tests — connect, move, click, key, wait_log"
```

---

### Task 4: `conftest.py` — session-scoped VM fixture

**Files:**
- Create: `tests/e2e/conftest.py`

The fixture is validated by the scenario tests (Tasks 5–7); no separate unit test.

- [ ] **Step 1: Create `tests/e2e/conftest.py`**

```python
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
```

- [ ] **Step 2: Verify conftest imports cleanly**

```bash
cd tests/e2e && python -c "import conftest; print('conftest OK')"
```
Expected: `conftest OK`

- [ ] **Step 3: Commit**

```bash
git add tests/e2e/conftest.py
git commit -m "feat(e2e): conftest.py session VM fixture — boots QEMU, waits for lifecycle log, connects QMP"
```

---

### Task 5: `test_boot.py` — boot scenarios

**Files:**
- Create: `tests/e2e/test_boot.py`

These tests require a live QEMU VM (run `make build` first, then `make test-e2e`).

- [ ] **Step 1: Create `tests/e2e/test_boot.py`**

```python
from assertions import MENUBAR_BG, DESKTOP_BG, assert_region_color, assert_min_unique_colors


def test_desktop_renders(vm):
    img = vm.screenshot()
    assert_min_unique_colors(img, 30)
    assert_region_color(img, 0, 0, img.width, 24, MENUBAR_BG, min_ratio=0.7)


def test_desktop_bg_present(vm):
    img = vm.screenshot()
    assert_region_color(
        img,
        100, 100, img.width - 200, img.height - 200,
        DESKTOP_BG, min_ratio=0.3,
    )
```

- [ ] **Step 2: Confirm collection (no VM needed)**

```bash
cd tests/e2e && pytest test_boot.py --collect-only
```
Expected:
```
<Function test_desktop_renders>
<Function test_desktop_bg_present>
2 tests collected
```

- [ ] **Step 3: Commit**

```bash
git add tests/e2e/test_boot.py
git commit -m "feat(e2e): test_boot.py — desktop renders with menubar dark + bg present"
```

---

### Task 6: `test_window.py` — drag scenario

**Files:**
- Create: `tests/e2e/test_window.py`

- [ ] **Step 1: Create `tests/e2e/test_window.py`**

```python
import time
from assertions import assert_no_ghost


def test_drag_leaves_no_ghost(vm):
    vm.move(200, 60)
    vm.click(200, 60)
    for dx in range(0, 100, 10):
        vm.move(200 + dx, 60)
        time.sleep(0.05)
    vm.move(300, 60)
    time.sleep(0.3)
    img = vm.screenshot()
    assert_no_ghost(img, 180, 45, 40, 30)
```

- [ ] **Step 2: Confirm collection**

```bash
cd tests/e2e && pytest test_window.py --collect-only
```
Expected: `1 test collected`

- [ ] **Step 3: Commit**

```bash
git add tests/e2e/test_window.py
git commit -m "feat(e2e): test_window.py — drag leaves no ghost at old titlebar position"
```

---

### Task 7: `test_input.py` — keyboard scenario

**Files:**
- Create: `tests/e2e/test_input.py`

- [ ] **Step 1: Create `tests/e2e/test_input.py`**

```python
import time
from assertions import assert_min_unique_colors


def test_keyboard_reaches_shell(vm):
    vm.key("ret")
    time.sleep(0.5)
    img = vm.screenshot()
    assert_min_unique_colors(img, 30)
```

- [ ] **Step 2: Confirm collection**

```bash
cd tests/e2e && pytest test_input.py --collect-only
```
Expected: `1 test collected`

- [ ] **Step 3: Commit**

```bash
git add tests/e2e/test_input.py
git commit -m "feat(e2e): test_input.py — keyboard Enter keeps screen alive (5 unique color buckets)"
```

---

### Task 8: Makefile `test-e2e` target

**Files:**
- Modify: `Makefile`

- [ ] **Step 1: Locate relevant lines**

```bash
grep -n "PHONY\|test-e2e" Makefile
```
Expected output shows the `.PHONY` line(s) and the existing `test-e2e-gui` blocks.

- [ ] **Step 2: Add `test-e2e` to the `.PHONY` list**

In `Makefile`, find this line (around line 93):
```makefile
        test-unit-apps test-gui-protocol test-e2e-gui test-gui
```
Change to:
```makefile
        test-unit-apps test-gui-protocol test-e2e-gui test-gui test-e2e
```

- [ ] **Step 3: Add the `test-e2e` target after the last `test-e2e-gui` block**

After the `test-e2e-gui: build` block (around line 342–343), insert:
```makefile

# ── test-e2e: pytest E2E harness (requires make build first) ─────────────────
test-e2e: $(BZIMAGE) $(INITRAMFS)
	$(DOCKER_RUN) bash -c " \
	  pip install -q -r tests/e2e/requirements.txt && \
	  BZIMAGE=$(BZIMAGE) INITRAMFS=$(INITRAMFS) \
	  pytest tests/e2e/ -v --tb=short 2>&1"
```

**Important:** The recipe line indentation must use a real tab character (`\t`), not spaces. If your editor converts tabs to spaces, use `cat -A Makefile | grep test-e2e` to verify a leading `^I` (tab) on recipe lines.

- [ ] **Step 4: Verify Makefile parses (dry run)**

```bash
make -n test-e2e 2>&1 | head -10
```
Expected: prints the `docker run ... pip install ... pytest` command chain without executing it.

- [ ] **Step 5: Run unit tests inside Docker (no VM required)**

```bash
make shell
# Inside the container:
pip install -q -r tests/e2e/requirements.txt
pytest tests/e2e/test_assertions.py tests/e2e/test_harness.py -v
exit
```
Expected: `19 passed`

- [ ] **Step 6: Commit**

```bash
git add Makefile
git commit -m "feat(e2e): add test-e2e Makefile target — pip + pytest inside Docker"
```

---

## Running the Suite

**Unit tests only (no build needed):**
```bash
make shell
pip install -q -r tests/e2e/requirements.txt
pytest tests/e2e/test_assertions.py tests/e2e/test_harness.py -v
```

**Full E2E suite (requires `make build` first):**
```bash
make build
make test-e2e
```
Expected boot-to-result time: ~60s (45s boot timeout + 3s render wait + test execution).

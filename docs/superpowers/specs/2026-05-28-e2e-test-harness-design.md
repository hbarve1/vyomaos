# VyomaOS E2E Test Harness Design

> **For agentic workers:** Use `superpowers:subagent-driven-development` or `superpowers:executing-plans` to implement this spec task-by-task.

**Goal:** Build a Python + pytest E2E test harness that boots VyomaOS in QEMU, injects keyboard/mouse input via QMP, takes screenshots, and asserts on visual and log state — giving the project a "Cypress for a VM" testing layer.

**Architecture:** Single QEMU VM instance per pytest session, controlled via QMP Unix socket. Tests run sequentially against shared framebuffer state. Three initial scenario tests validate the harness itself; more are added per-feature.

**Tech Stack:** Python 3, pytest, Pillow (PIL), QEMU QMP protocol, existing Docker builder image (Ubuntu 22.04).

---

## Context: Existing Infrastructure

The project already has:
- `base/scripts/smoke-test.sh` — headless QEMU boot, serial log assertion only
- `base/scripts/test-e2e-gui.sh` — QEMU monitor screendump + Python color-count check
- `base/scripts/check-screenshot.py` — PPM pixel reader, menu bar dark ratio, unique color count
- `make test-e2e-gui` Makefile target — builds then runs the GUI test script

The new harness **extends** this infrastructure: it reuses the QEMU boot pattern, the PPM parsing logic from `check-screenshot.py`, and the same Docker image. The existing `test-e2e-gui.sh` remains as a lightweight pre-check; the pytest suite is the full harness.

---

## File Structure

```
tests/
  e2e/
    requirements.txt     # pytest, Pillow
    harness.py           # QmpClient class
    conftest.py          # session-scoped VM fixture
    assertions.py        # assert_pixel, assert_region_color, assert_log_contains
    test_boot.py         # scenario: OS boots, desktop renders
    test_window.py       # scenario: window drag leaves no ghost pixels
    test_input.py        # scenario: keyboard input reaches focused app
```

---

## Component: QmpClient (`harness.py`)

Wraps QEMU's QMP JSON-RPC protocol over a Unix socket.

```python
class QmpClient:
    def __init__(self, qmp_sock: str, serial_log: str)
    def connect(self)                          # handshake + send qmp_capabilities
    def close(self)
    def click(self, x: int, y: int)            # mouse move + btn-left down + up
    def move(self, x: int, y: int)             # mouse move only (no click)
    def key(self, key_name: str)               # keypress down + up
    def screenshot(self) -> "PIL.Image.Image"  # screendump PPM → Pillow Image
    def wait_log(self, pattern: str, timeout: int = 10) -> str  # regex, raises TimeoutError
```

**QMP event flow for click(x, y):**
1. `input-send-event` with `{"type":"abs","data":{"axis":"x","value": x * 32767 // screen_w}}`
2. `input-send-event` with `{"type":"abs","data":{"axis":"y","value": y * 32767 // screen_h}}`
3. `input-send-event` with `{"type":"btn","data":{"down":true,"button":"left"}}`
4. `input-send-event` with `{"type":"btn","data":{"down":false,"button":"left"}}`

**Screenshot:** sends `{"execute":"screendump","arguments":{"filename":"/tmp/screen.ppm"}}` via QMP, reads the PPM file, converts to `PIL.Image` via `Image.open()`.

**wait_log:** polls the serial log file with 0.1s sleep intervals, returns the matching line, raises `TimeoutError` if pattern not seen within `timeout` seconds.

---

## Component: Pytest Fixture (`conftest.py`)

Single `scope="session"` fixture boots QEMU once for the entire test run.

```python
@pytest.fixture(scope="session")
def vm(tmp_path_factory):
    tmp = tmp_path_factory.mktemp("vyoma")
    qmp_sock = str(tmp / "qmp.sock")
    serial_log = str(tmp / "serial.log")
    screenshot_tmp = str(tmp / "screen.ppm")

    proc = subprocess.Popen([
        "qemu-system-x86_64",
        "-kernel", os.environ["BZIMAGE"],
        "-initrd", os.environ["INITRAMFS"],
        "-m", "512M",
        "-no-reboot",
        "-append", "console=ttyS0 quiet",
        "-device", "virtio-vga",
        "-display", "none",
        "-qmp", f"unix:{qmp_sock},server,nowait",
        "-serial", f"file:{serial_log}",
    ])

    client = QmpClient(qmp_sock, serial_log, screenshot_tmp)
    client.wait_log(r"\[lifecycle\].*all apps spawned", timeout=45)
    time.sleep(3)   # allow first frame render
    client.connect()

    yield client

    client.close()
    proc.terminate()
    proc.wait(timeout=5)
```

`BZIMAGE` and `INITRAMFS` are read from environment variables (defaulting to `out/bzImage` and `out/initramfs.cpio.gz`), set by the Makefile when invoking pytest.

---

## Component: Assertions (`assertions.py`)

Extends pixel logic from `check-screenshot.py`.

```python
DESKTOP_BG = (28, 28, 30)      # 0x1C1C1EFF
MENUBAR_BG = (20, 20, 22)      # approximate dark
TOLERANCE  = 15                 # per-channel tolerance for color comparisons

def assert_pixel(img, x, y, rgb, tolerance=TOLERANCE):
    """Assert a single pixel matches rgb within tolerance."""

def assert_region_color(img, x, y, w, h, rgb, min_ratio=0.7, tolerance=TOLERANCE):
    """Assert ≥ min_ratio of sampled pixels in region match rgb."""

def assert_no_ghost(img, x, y, w, h):
    """Assert region matches desktop bg — used to verify drag cleared old position."""

def assert_log_contains(serial_log: str, pattern: str, timeout: int = 5):
    """Assert serial log contains pattern within timeout seconds."""

def assert_min_unique_colors(img, min_count=30):
    """Assert screenshot has at least min_count distinct color buckets (5-bit quantized)."""
```

---

## Scenarios

### `test_boot.py`

```python
def test_desktop_renders(vm):
    img = vm.screenshot()
    assert_min_unique_colors(img, 30)
    assert_region_color(img, 0, 0, img.width, 24, MENUBAR_BG, min_ratio=0.7)

def test_desktop_bg_present(vm):
    img = vm.screenshot()
    # Center region should be desktop background
    assert_region_color(img, 100, 100, img.width - 200, img.height - 200,
                        DESKTOP_BG, min_ratio=0.3)
```

### `test_window.py`

```python
def test_drag_leaves_no_ghost(vm):
    img_before = vm.screenshot()
    # Find a window titlebar — click and drag 100px right
    vm.move(200, 60)
    vm.click(200, 60)
    for dx in range(0, 100, 10):
        vm.move(200 + dx, 60)
        time.sleep(0.05)
    vm.move(300, 60)
    # Release (move without click held = release)
    time.sleep(0.3)
    img_after = vm.screenshot()
    # Old titlebar position (200, 60) should now be desktop bg
    assert_no_ghost(img_after, 180, 45, 40, 30)
```

### `test_input.py`

```python
def test_keyboard_reaches_shell(vm):
    # Focus shell window and send Enter; serial log won't show shell prompt
    # but we verify the app doesn't crash by checking it's still rendered
    vm.key("ret")
    time.sleep(0.5)
    img = vm.screenshot()
    assert_min_unique_colors(img, 30)  # screen still rendering = app alive
```

---

## Makefile Integration

```makefile
# ── test-e2e: pytest E2E harness (requires make build first) ─────────────────
test-e2e: $(BZIMAGE) $(INITRAMFS)
	$(DOCKER_RUN) bash -c " \
	  pip install -q -r tests/e2e/requirements.txt && \
	  BZIMAGE=$(BZIMAGE) INITRAMFS=$(INITRAMFS) \
	  pytest tests/e2e/ -v --tb=short 2>&1"
```

`requirements.txt`:
```
pytest>=7.0
Pillow>=9.0
```

No new Docker image layer needed — `pip install` at test time inside the existing container is sufficient for these two lightweight packages.

---

## Error Handling

- **QEMU fails to start:** `proc.returncode` checked after 5s; fixture raises `RuntimeError("QEMU failed to start")` with last 20 lines of serial log.
- **Boot timeout:** `wait_log` raises `TimeoutError`; pytest reports as fixture setup failure with the timeout value and last serial log lines.
- **Screenshot missing:** `QmpClient.screenshot()` raises `RuntimeError` if QMP returns error or PPM file is not produced within 2s.
- **Assertion failure:** Standard pytest `AssertionError` with pixel coordinates and expected/actual RGB values in the message.

---

## Out of Scope

- Visual regression baseline image comparison (pixel-perfect diffs) — too brittle for a framebuffer renderer
- Parallel test execution — tests share VM state, must run sequentially
- Unit test infrastructure fix (arm64 SIGSEGV) — separate PR
- GUI scenario library beyond the 3 initial tests — added per-feature

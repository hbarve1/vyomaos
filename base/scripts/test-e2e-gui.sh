#!/usr/bin/env bash
# test-e2e-gui.sh — boots VyomaOS in QEMU with virtio-gpu display,
# takes a screendump via QEMU monitor, and verifies the framebuffer is
# not blank (>= 20 distinct RGB colors expected from the desktop UI).
#
# Prerequisites: qemu-system-x86_64 with virtio-vga, socat, python3
# Designed for Linux CI; on macOS requires homebrew qemu.
set -euo pipefail

BZIMAGE="${BZIMAGE:-out/bzImage}"
INITRAMFS="${INITRAMFS:-out/initramfs.cpio.gz}"
MONITOR_SOCK="/tmp/vyoma-mon-$$.sock"
SERIAL_LOG="/tmp/vyoma-serial-$$.log"
SCREENSHOT="/tmp/vyoma-screen-$$.ppm"
BOOT_TIMEOUT=45
RENDER_WAIT=5
MIN_COLORS=20

# Require build artifacts.
for f in "$BZIMAGE" "$INITRAMFS"; do
    [ -f "$f" ] || { echo "E2E-GUI: SKIP: $f not found (run make build first)"; exit 0; }
done

# Require socat for monitor communication.
command -v socat >/dev/null 2>&1 || { echo "E2E-GUI: SKIP: socat not installed"; exit 0; }

cleanup() {
    kill "$QEMU_PID" 2>/dev/null || true
    rm -f "$MONITOR_SOCK" "$SERIAL_LOG" "$SCREENSHOT"
}
trap cleanup EXIT

echo "E2E-GUI: booting QEMU with virtio-gpu..."

qemu-system-x86_64 \
    -kernel  "$BZIMAGE" \
    -initrd  "$INITRAMFS" \
    -m       512M \
    -no-reboot \
    -append  "console=ttyS0 quiet" \
    -device  virtio-vga \
    -display none \
    -monitor unix:"$MONITOR_SOCK",server,nowait \
    -serial  file:"$SERIAL_LOG" \
    2>/dev/null &
QEMU_PID=$!

echo "E2E-GUI: waiting for 'all apps spawned' (timeout ${BOOT_TIMEOUT}s)..."
for i in $(seq 1 "$BOOT_TIMEOUT"); do
    if grep -q "\[lifecycle\].*all apps spawned" "$SERIAL_LOG" 2>/dev/null; then
        echo "E2E-GUI: boot OK at ${i}s"
        break
    fi
    [ "$i" -eq "$BOOT_TIMEOUT" ] && { echo "E2E-GUI: FAIL: boot timeout"; exit 1; }
    sleep 1
done

# Give desktop app time to render first frame.
sleep "$RENDER_WAIT"

echo "E2E-GUI: capturing screendump..."
echo "screendump $SCREENSHOT" | socat - "UNIX-CONNECT:$MONITOR_SOCK" 2>/dev/null || true
sleep 1

if [ ! -f "$SCREENSHOT" ] || [ ! -s "$SCREENSHOT" ]; then
    echo "E2E-GUI: FAIL: screendump not produced"
    exit 1
fi

# Count distinct RGB colors in the PPM (skip header lines).
COLORS=$(python3 - "$SCREENSHOT" <<'PYEOF'
import sys, struct
data = open(sys.argv[1], 'rb').read()
# PPM P6 header: "P6\n<W> <H>\n<MAX>\n" then raw RGB bytes
lines = data.split(b'\n', 3)
if not lines[0].startswith(b'P6'):
    print(0)
    sys.exit()
raw = lines[3]
pixels = set(raw[i:i+3] for i in range(0, len(raw)-2, 3))
print(len(pixels))
PYEOF
)

echo "E2E-GUI: distinct colors in framebuffer: $COLORS"

if [ "$COLORS" -lt "$MIN_COLORS" ]; then
    echo "E2E-GUI: FAIL: framebuffer appears blank (${COLORS} colors < ${MIN_COLORS} expected)"
    exit 1
fi

echo "E2E-GUI: PASS"
exit 0

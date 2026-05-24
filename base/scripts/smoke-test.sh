#!/usr/bin/env bash
# smoke-test.sh — headless QEMU boot test for VyomaOS (FR-003, US2)
# Exit 0 = supervisor reached "all apps spawned" within 30 seconds.
# Exit 1 = timeout, kernel panic, or QEMU error.
set -euo pipefail

BZIMAGE="${BZIMAGE:-out/bzImage}"
INITRAMFS="${INITRAMFS:-out/initramfs.cpio.gz}"
LOG_FILE="/tmp/smoke-$(date +%s).log"
TIMEOUT_SECS=30

if [[ ! -f "$BZIMAGE" ]]; then
    echo "SMOKE: FAIL: kernel not found: $BZIMAGE" >&2
    exit 1
fi
if [[ ! -f "$INITRAMFS" ]]; then
    echo "SMOKE: FAIL: initramfs not found: $INITRAMFS" >&2
    exit 1
fi

cleanup() { rm -f "$LOG_FILE"; }
trap cleanup EXIT

# Boot headlessly; -nographic already maps serial->stdio, so no -serial flag needed.
timeout "$TIMEOUT_SECS" qemu-system-x86_64 \
    -kernel  "$BZIMAGE" \
    -initrd  "$INITRAMFS" \
    -nographic \
    -append  "console=ttyS0 quiet" \
    -m       512M \
    -no-reboot \
    2>/dev/null \
    | tee "$LOG_FILE" \
    | grep -m1 "\[lifecycle\].*all apps spawned" \
    > /dev/null 2>&1

GREP_STATUS=${PIPESTATUS[2]}

# Also check for kernel panic regardless of grep result.
if grep -q "Kernel panic" "$LOG_FILE" 2>/dev/null; then
    echo "SMOKE: FAIL: kernel panic detected" >&2
    exit 1
fi

if [[ $GREP_STATUS -eq 0 ]]; then
    echo "SMOKE: PASS"
    exit 0
else
    echo "SMOKE: FAIL: ready signal not found within ${TIMEOUT_SECS}s" >&2
    exit 1
fi

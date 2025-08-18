#!/usr/bin/env bash
set -euo pipefail

OUTDIR=out
KERNEL="$OUTDIR/bzImage"
INITRAMFS="$OUTDIR/initramfs.cpio.gz"

if [ ! -f "$KERNEL" ] || [ ! -f "$INITRAMFS" ]; then
  echo "Missing kernel or initramfs. Run ./build.sh first." >&2
  exit 1
fi

# try hvf (macOS), fallback to tcg
if qemu-system-x86_64 -accel help 2>/dev/null | grep -q hvf; then
  ACCEL="hvf"
else
  ACCEL="tcg"
fi

qemu-system-x86_64 \
  -machine accel=$ACCEL \
  -m 512M \
  -kernel "$KERNEL" \
  -initrd "$INITRAMFS" \
  -nographic \
  -append "console=ttyS0 loglevel=3"

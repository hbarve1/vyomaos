#!/usr/bin/env bash
set -euo pipefail

OUTDIR=out
mkdir -p "$OUTDIR"

# 1) Download a pre-built kernel (QEMU-friendly bzImage)
KERNEL_BIN="$OUTDIR/bzImage"

if [ ! -f "$KERNEL_BIN" ]; then
  echo "Setting up QEMU kernel (bzImage)..."
  
  echo "⚠️  Kernel build not available on this system"
  echo "💡 The Linux kernel build requires Linux build tools"
  echo "🔧 Run ./get_proper_kernel.sh for kernel options"
  echo ""
  echo "Creating placeholder kernel..."
  dd if=/dev/zero of="$KERNEL_BIN" bs=1M count=2 2>/dev/null || true
fi

# Check kernel status
if [ -f "$KERNEL_BIN" ]; then
  echo "📋 Kernel file info:"
  file "$KERNEL_BIN"
  
  if file "$KERNEL_BIN" | grep -q "ELF"; then
    echo "⚠️  Warning: Kernel is an ELF executable (not a real kernel)"
    echo "💡 This will cause QEMU boot issues."
    echo "🔧 To fix this, run: ./get_proper_kernel.sh"
  elif file "$KERNEL_BIN" | grep -q "data"; then
    echo "⚠️  Warning: Kernel is a placeholder (not a real kernel)"
    echo "💡 This will cause QEMU boot issues."
    echo "🔧 To fix this, run: ./get_proper_kernel.sh"
  else
    echo "✅ Kernel appears to be valid!"
  fi
fi


# 2) Download BusyBox static
BUSYBOX_URL="https://busybox.net/downloads/binaries/1.35.0-x86_64-linux-musl/busybox"
BUSYBOX_BIN="$OUTDIR/busybox"
if [ ! -f "$BUSYBOX_BIN" ]; then
  echo "Downloading BusyBox..."
  curl -L -o "$BUSYBOX_BIN" "$BUSYBOX_URL"
  chmod +x "$BUSYBOX_BIN"
fi

# 3) Download Wasmtime runtime
WASMTIME_URL="https://github.com/bytecodealliance/wasmtime/releases/download/v16.0.0/wasmtime-v16.0.0-x86_64-macos.tar.xz"
WASMTIME_TAR="$OUTDIR/wasmtime.tar.xz"
WASMTIME_DIR="$OUTDIR/wasmtime"
if [ ! -d "$WASMTIME_DIR" ]; then
  echo "Downloading Wasmtime..."
  curl -L -o "$WASMTIME_TAR" "$WASMTIME_URL"
  mkdir -p "$WASMTIME_DIR"
  gtar -xJf "$WASMTIME_TAR" -C "$WASMTIME_DIR" --strip-components=1
fi

# 4) Prepare rootfs
ROOTFS="$OUTDIR/rootfs"
rm -rf "$ROOTFS"
mkdir -p "$ROOTFS"/{bin,proc,sys,dev,usr/bin,etc}

# copy busybox
cp "$BUSYBOX_BIN" "$ROOTFS/bin/busybox"
ln -s /bin/busybox "$ROOTFS/bin/sh"

# copy wasmtime
cp "$WASMTIME_DIR/wasmtime" "$ROOTFS/usr/bin/wasmtime"
chmod +x "$ROOTFS/usr/bin/wasmtime"

# add minimal /etc
cat > "$ROOTFS/etc/passwd" <<'EOF'
root:x:0:0:root:/root:/bin/sh
EOF

cat > "$ROOTFS/etc/group" <<'EOF'
root:x:0:
EOF

# 5) add wasm app (tiny base64 compiled WASM)
cat > "$ROOTFS/wasm_app.wasm.base64" <<'EOF'
AGFzbQEAAAABBgFgAX8BfwMCAQAFAgEABwEDZm4ABgABAAkK
EOF
base64 -D -i "$ROOTFS/wasm_app.wasm.base64" -o "$ROOTFS/wasm_app.wasm"
chmod 644 "$ROOTFS/wasm_app.wasm"

# 6) init script
cat > "$ROOTFS/init" <<'EOF'
#!/bin/sh
mount -t proc none /proc
mount -t sysfs none /sys
mount -t devtmpfs none /dev || true

echo "WASM OS: init starting..."
chmod +x /usr/bin/wasmtime

# Run wasm app as PID 1
exec /usr/bin/wasmtime /wasm_app.wasm || ({ echo 'wasm runtime failed'; sleep 5; exec /bin/sh; })
EOF
chmod +x "$ROOTFS/init"

# 7) pack initramfs (no sudo required on macOS)
pushd "$OUTDIR/rootfs" >/dev/null
find . | cpio -o -H newc | gzip > ../initramfs.cpio.gz
popd >/dev/null

echo "✅ Build finished. Files in $OUTDIR: vmlinuz, initramfs.cpio.gz"
echo "➡️  Run ./run.sh to boot in QEMU."

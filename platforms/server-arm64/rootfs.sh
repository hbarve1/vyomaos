#!/bin/sh
# rootfs.sh — ARM64 server platform
#
# Assembles a minimal initramfs for headless server targets using the
# server-headless.toml platform profile. No display or GUI modules are included.
#
# Usage (called by platforms/server-arm64/Makefile):
#   rootfs.sh --out <out_dir> --profile <profile_toml_path>
#
# Produces:
#   <out_dir>/initramfs.cpio.gz  — compressed initramfs
#
# Requires (available in the vyomaos-builder Docker image):
#   wasmtime, busybox, supervisor binary, compiled WASM apps

set -e

# ── Argument parsing ──────────────────────────────────────────────────────────
OUT=""
PROFILE=""
while [ "$#" -gt 0 ]; do
    case "$1" in
        --out)     OUT="$2";     shift 2 ;;
        --profile) PROFILE="$2"; shift 2 ;;
        *) echo "unknown argument: $1" >&2; exit 1 ;;
    esac
done

if [ -z "$OUT" ]; then
    echo "rootfs.sh: --out required" >&2
    exit 1
fi
if [ -z "$PROFILE" ]; then
    PROFILE="supervisor/src/profile/profiles/server-headless.toml"
fi

WORK_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
ROOTFS="$OUT/rootfs"

echo "[rootfs] server-arm64: assembling initramfs in $ROOTFS"
echo "[rootfs] server-arm64: using profile $PROFILE"

# ── Create directory skeleton ─────────────────────────────────────────────────
rm -rf "$ROOTFS"
mkdir -p \
    "$ROOTFS/bin" \
    "$ROOTFS/sbin" \
    "$ROOTFS/etc/vyoma" \
    "$ROOTFS/etc/vyoma/apps" \
    "$ROOTFS/dev" \
    "$ROOTFS/proc" \
    "$ROOTFS/sys" \
    "$ROOTFS/tmp" \
    "$ROOTFS/data" \
    "$ROOTFS/run"

# ── BusyBox ───────────────────────────────────────────────────────────────────
if [ -f "$WORK_ROOT/out/busybox" ]; then
    cp "$WORK_ROOT/out/busybox" "$ROOTFS/bin/busybox"
    chmod +x "$ROOTFS/bin/busybox"
    # Install BusyBox applets
    "$ROOTFS/bin/busybox" --install -s "$ROOTFS/bin"
else
    echo "[rootfs] WARNING: busybox not found at out/busybox — network tools may be missing"
fi

# ── Wasmtime runtime ──────────────────────────────────────────────────────────
if [ -f "$WORK_ROOT/out/wasmtime" ]; then
    cp "$WORK_ROOT/out/wasmtime" "$ROOTFS/bin/wasmtime"
    chmod +x "$ROOTFS/bin/wasmtime"
else
    echo "[rootfs] WARNING: wasmtime not found at out/wasmtime"
fi

# ── Supervisor (PID 1) ────────────────────────────────────────────────────────
SUP_BIN="$WORK_ROOT/supervisor/target/aarch64-unknown-linux-musl/release/supervisor"
if [ ! -f "$SUP_BIN" ]; then
    # Fall back to x86_64 build for CI/dev environments
    SUP_BIN="$WORK_ROOT/supervisor/target/x86_64-unknown-linux-musl/release/supervisor"
fi
if [ -f "$SUP_BIN" ]; then
    cp "$SUP_BIN" "$ROOTFS/sbin/init"
    chmod +x "$ROOTFS/sbin/init"
else
    echo "[rootfs] ERROR: supervisor binary not found" >&2
    exit 1
fi

# ── Platform profile ──────────────────────────────────────────────────────────
# Copy the server-headless profile so the supervisor can read it at boot.
cp "$WORK_ROOT/$PROFILE" "$ROOTFS/etc/vyoma/platform.toml"

# ── WASM apps ─────────────────────────────────────────────────────────────────
# Include apps that make sense for a headless server deployment:
#   - http-server: serves content over network
#   - doc-viewer:  dual-mode app (will use HTTP mode on server profile)
#   - hello-world: baseline sanity-check app
WASM_APPS="http-server doc-viewer hello-world"

for app in $WASM_APPS; do
    WASM_SRC="$WORK_ROOT/apps/$app/target/wasm32-wasip2/release/$app.wasm"
    MANIFEST_SRC="$WORK_ROOT/apps/$app/vyoma.toml"
    if [ -f "$WASM_SRC" ]; then
        cp "$WASM_SRC" "$ROOTFS/etc/vyoma/apps/$app.wasm"
        echo "[rootfs] included $app.wasm"
    else
        echo "[rootfs] WARNING: $app.wasm not found (run 'make apps' first)"
    fi
    if [ -f "$MANIFEST_SRC" ]; then
        cp "$MANIFEST_SRC" "$ROOTFS/etc/vyoma/apps/$app.toml"
    fi
done

# ── boot.toml ─────────────────────────────────────────────────────────────────
# The supervisor reads /etc/vyoma/boot.toml to know which apps to launch.
# Server profile: launch doc-viewer (HTTP mode) and http-server.
cat > "$ROOTFS/etc/vyoma/boot.toml" << 'BOOT'
[[apps]]
manifest = "/etc/vyoma/apps/doc-viewer.toml"
restart  = "always"

[[apps]]
manifest = "/etc/vyoma/apps/http-server.toml"
restart  = "always"

[[apps]]
manifest = "/etc/vyoma/apps/hello-world.toml"
restart  = "never"
BOOT

echo "[rootfs] generated boot.toml (server-headless profile)"

# ── /init symlink ──────────────────────────────────────────────────────────────
ln -sf /sbin/init "$ROOTFS/init"

# ── Pack initramfs ─────────────────────────────────────────────────────────────
echo "[rootfs] packing initramfs.cpio.gz"
(cd "$ROOTFS" && find . | cpio --quiet -H newc -o) | gzip -9 > "$OUT/initramfs.cpio.gz"

echo "[rootfs] server-arm64: done — $OUT/initramfs.cpio.gz"

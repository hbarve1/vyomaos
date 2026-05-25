#!/bin/sh
# rootfs.sh — ARM64 mobile platform (TM07)
#
# Builds the initramfs for a VyomaOS mobile/tablet target.
# Cross-compiles the supervisor to aarch64-unknown-linux-musl.
# Includes the touch-demo app and the mobile platform profile.
#
# Invoked by the root Makefile when PLATFORM=mobile-arm64:
#   docker run ... bash platforms/mobile-arm64/rootfs.sh
#
# Environment variables (set by root Makefile):
#   OUT          — output directory (default: out/)
#   WORK         — build scratch directory (default: /work)

set -e

OUT="${OUT:-out}"
WORK="${WORK:-/work}"
INITRAMFS="${OUT}/initramfs-arm64.cpio.gz"
SYSROOT="${WORK}/rootfs-arm64"
PLATFORM_DIR="platforms/mobile-arm64"
PROFILE_DIR="supervisor/src/profile/profiles"

echo "[mobile-arm64] building ARM64 mobile initramfs → ${INITRAMFS}"

# ── Cross-compilation target ──────────────────────────────────────────────────
CROSS_TARGET="aarch64-unknown-linux-musl"

# ── 1. Compile supervisor (ARM64 musl static binary) ─────────────────────────
echo "[mobile-arm64] compiling supervisor for ${CROSS_TARGET}"
RUSTFLAGS="-D warnings" cargo build \
    --manifest-path supervisor/Cargo.toml \
    --target "${CROSS_TARGET}" \
    --release

SUPERVISOR_BIN="supervisor/target/${CROSS_TARGET}/release/supervisor"

# ── 2. Compile WASM apps (arch-independent) ───────────────────────────────────
echo "[mobile-arm64] compiling WASM apps (wasm32-wasip2)"
cargo build \
    --manifest-path apps/touch-demo/Cargo.toml \
    --target wasm32-wasip2 \
    --release

TOUCH_DEMO_WASM="apps/touch-demo/target/wasm32-wasip2/release/touch-demo.wasm"

# ── 3. Create sysroot ─────────────────────────────────────────────────────────
echo "[mobile-arm64] creating sysroot at ${SYSROOT}"
rm -rf "${SYSROOT}"
mkdir -p \
    "${SYSROOT}/bin" \
    "${SYSROOT}/dev" \
    "${SYSROOT}/etc/vyoma/profiles" \
    "${SYSROOT}/proc" \
    "${SYSROOT}/sys" \
    "${SYSROOT}/tmp" \
    "${SYSROOT}/apps/touch-demo"

# Supervisor binary (PID 1)
install -m 0755 "${SUPERVISOR_BIN}" "${SYSROOT}/bin/supervisor"
ln -sf /bin/supervisor "${SYSROOT}/init"

# Wasmtime runtime (ARM64 build — must be pre-downloaded)
if [ -f "${WORK}/wasmtime-arm64" ]; then
    install -m 0755 "${WORK}/wasmtime-arm64" "${SYSROOT}/usr/bin/wasmtime"
else
    echo "[mobile-arm64] warn: wasmtime-arm64 not found at ${WORK}/wasmtime-arm64"
    echo "[mobile-arm64] warn: download aarch64 wasmtime from https://github.com/bytecodealliance/wasmtime/releases"
fi

# Platform profile
install -m 0644 "${PROFILE_DIR}/mobile.toml" \
    "${SYSROOT}/etc/vyoma/profiles/mobile.toml"

# touch-demo app
install -m 0644 "${TOUCH_DEMO_WASM}" \
    "${SYSROOT}/apps/touch-demo/touch-demo.wasm"
cat > "${SYSROOT}/apps/touch-demo/vyoma.toml" <<'TOML'
[app]
name    = "touch-demo"
version = "0.1.0"
wasm    = "touch-demo.wasm"

[capabilities]
stdio   = true
display = true
touch   = true

[window]
width  = 1080
height = 500
TOML

# Boot config — launches touch-demo
cat > "${SYSROOT}/etc/vyoma/boot.toml" <<'TOML'
[[apps]]
manifest = "/apps/touch-demo/vyoma.toml"
restart  = "always"
TOML

# ── 4. Pack initramfs ─────────────────────────────────────────────────────────
echo "[mobile-arm64] packing initramfs → ${INITRAMFS}"
mkdir -p "${OUT}"
cd "${SYSROOT}" && find . | cpio -H newc -o | gzip -9 > "${WORK}/${INITRAMFS}"

echo "[mobile-arm64] done: ${INITRAMFS}"

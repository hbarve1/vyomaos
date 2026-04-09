#!/usr/bin/env bash
# VyomaOS Root Filesystem Module
# Builds out/initramfs.cpio.gz using a musl-static BusyBox binary.

set -euo pipefail

source "$(dirname "${BASH_SOURCE[0]}")/../config.sh"

ROOTFS="$OUTDIR/rootfs"

# ── BusyBox musl-static binary ────────────────────────────────────────────────
BUSYBOX_VERSION="1.36.1"
BUSYBOX_URL="https://www.busybox.net/downloads/binaries/${BUSYBOX_VERSION}-x86_64-linux-musl/busybox"
BUSYBOX_SHA256="b8cc24c9574d809e7279c3be349795c5d5ceb6fdf19ca709f80cde50e47de314"
BUSYBOX_CACHE="$OUTDIR/cache/busybox-${BUSYBOX_VERSION}-x86_64-musl"

# ── Download with SHA-256 verification ───────────────────────────────────────
download_verified() {
    local url="$1" dest="$2" expected="$3"

    if [[ -f "$dest" ]]; then
        log_info "Using cached: $(basename "$dest")"
    else
        mkdir -p "$(dirname "$dest")"
        log_info "Downloading: $url"
        wget --quiet --show-progress -O "$dest.tmp" "$url"
        mv "$dest.tmp" "$dest"
    fi

    log_info "Verifying SHA-256..."
    local actual
    actual="$(sha256sum "$dest" | awk '{print $1}')"
    if [[ "$actual" != "$expected" ]]; then
        log_error "SHA-256 mismatch for $(basename "$dest")"
        log_error "  Expected: $expected"
        log_error "  Actual:   $actual"
        rm -f "$dest"
        return 1
    fi
    log_info "SHA-256 OK"
}

build_rootfs() {
    log_info "Building rootfs..."

    # ── Directory skeleton ────────────────────────────────────────────────────
    rm -rf "$ROOTFS"
    mkdir -p \
        "$ROOTFS"/{bin,sbin,usr/bin,usr/sbin,lib,lib64,dev,proc,sys,tmp,run,apps} \
        "$ROOTFS/etc"

    # ── BusyBox ───────────────────────────────────────────────────────────────
    download_verified "$BUSYBOX_URL" "$BUSYBOX_CACHE" "$BUSYBOX_SHA256"
    install -m 0755 "$BUSYBOX_CACHE" "$ROOTFS/bin/busybox"

    # Minimal applet symlinks — keep this list short (attack surface reduction)
    for applet in sh mount poweroff echo ls cat; do
        ln -sf /bin/busybox "$ROOTFS/bin/$applet"
    done

    # ── /init (PID 1 bootstrap) ───────────────────────────────────────────────
    # Mounts virtual filesystems then execs the Rust supervisor.
    # Falls back to a shell if the supervisor isn't built yet.
    cat > "$ROOTFS/init" << 'INIT_EOF'
#!/bin/sh
set -e
/bin/mount -t proc  none /proc
/bin/mount -t sysfs none /sys
echo "VyomaOS booting..."
if [ -x /usr/bin/supervisor ]; then
    exec /usr/bin/supervisor
else
    echo "WARNING: supervisor not found, dropping to shell"
    exec /bin/sh
fi
INIT_EOF
    chmod 0755 "$ROOTFS/init"

    # ── Minimal /etc stubs ────────────────────────────────────────────────────
    printf 'root:x:0:0:root:/root:/bin/sh\n' > "$ROOTFS/etc/passwd"
    printf 'root:x:0:\n'                     > "$ROOTFS/etc/group"

    # ── Rust supervisor binary ────────────────────────────────────────────────
    local supervisor_bin="$PROJECT_ROOT/supervisor/target/x86_64-unknown-linux-musl/release/supervisor"
    if [[ -f "$supervisor_bin" ]]; then
        install -m 0755 "$supervisor_bin" "$ROOTFS/usr/bin/supervisor"
        log_info "Installed supervisor ($(du -h "$supervisor_bin" | cut -f1))"
    else
        log_info "WARNING: supervisor binary not found, /init will fall back to shell"
    fi

    # ── WASM apps (if built) ──────────────────────────────────────────────────
    local apps_dir="$PROJECT_ROOT/apps/build"
    if [[ -d "$apps_dir" ]]; then
        for wasm in "$apps_dir"/*.wasm; do
            [[ -f "$wasm" ]] || continue
            cp "$wasm" "$ROOTFS/apps/"
            log_info "  Included: $(basename "$wasm") ($(du -h "$wasm" | cut -f1))"
        done
    fi

    # ── Pack initramfs ────────────────────────────────────────────────────────
    log_info "Packing initramfs -> $INITRAMFS_FILE"
    (
        cd "$ROOTFS"
        find . | cpio --quiet -H newc -o
    ) | gzip -9 > "$INITRAMFS_FILE"

    log_success "Rootfs built: $INITRAMFS_FILE ($(du -sh "$INITRAMFS_FILE" | cut -f1))"
}

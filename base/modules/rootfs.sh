#!/usr/bin/env bash
# VyomaOS Root Filesystem Module
# Builds out/initramfs.cpio.gz using a musl-static BusyBox binary.

set -euo pipefail

source "$(dirname "${BASH_SOURCE[0]}")/../config.sh"

ROOTFS="$OUTDIR/rootfs"

# ── Wasmtime musl-static binary ──────────────────────────────────────────────
WASMTIME_VERSION="43.0.0"
WASMTIME_TARBALL="wasmtime-v${WASMTIME_VERSION}-x86_64-linux.tar.xz"
WASMTIME_URL="https://github.com/bytecodealliance/wasmtime/releases/download/v${WASMTIME_VERSION}/${WASMTIME_TARBALL}"
WASMTIME_SHA256="e75a4933253fbc7b027c670b699490f163e3c86784f1db66581ae80fc0eb652c"
WASMTIME_CACHE="$OUTDIR/cache/${WASMTIME_TARBALL}"

# ── BusyBox musl-static binary ────────────────────────────────────────────────
BUSYBOX_VERSION="1.35.0"
BUSYBOX_URL="https://www.busybox.net/downloads/binaries/${BUSYBOX_VERSION}-x86_64-linux-musl/busybox"
BUSYBOX_SHA256="6e123e7f3202a8c1e9b1f94d8941580a25135382b99e8d3e34fb858bba311348"
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
        "$ROOTFS"/{bin,sbin,usr/bin,usr/sbin,lib,lib64,dev,proc,sys,tmp,run,apps,data} \
        "$ROOTFS/etc/vyoma"

    # ── BusyBox ───────────────────────────────────────────────────────────────
    download_verified "$BUSYBOX_URL" "$BUSYBOX_CACHE" "$BUSYBOX_SHA256"
    cp -f "$BUSYBOX_CACHE" "$ROOTFS/bin/busybox"
    chmod 0755 "$ROOTFS/bin/busybox"

    # Minimal applet symlinks — keep this list short (attack surface reduction)
    for applet in sh mount poweroff echo ls cat; do
        ln -sf /bin/busybox "$ROOTFS/bin/$applet"
    done

    # ── Wasmtime runtime ─────────────────────────────────────────────────────
    download_verified "$WASMTIME_URL" "$WASMTIME_CACHE" "$WASMTIME_SHA256"
    # Extract the single 'wasmtime' binary from the tarball, strip debug info.
    tar -xJf "$WASMTIME_CACHE" --strip-components=1 \
        -C "$OUTDIR/cache" \
        "wasmtime-v${WASMTIME_VERSION}-x86_64-linux/wasmtime"
    cp -f "$OUTDIR/cache/wasmtime" "$ROOTFS/usr/bin/wasmtime"
    chmod 0755 "$ROOTFS/usr/bin/wasmtime"
    log_info "Installed wasmtime ($(du -h "$ROOTFS/usr/bin/wasmtime" | cut -f1))"

    # ── glibc runtime for wasmtime (x86_64-linux glibc variant) ──────────────
    # wasmtime is dynamically linked; copy the glibc loader and its dependencies
    # from the builder container (Ubuntu 22.04, glibc 2.35).
    mkdir -p "$ROOTFS/lib/x86_64-linux-gnu" "$ROOTFS/lib64"
    for lib in \
        /lib/x86_64-linux-gnu/ld-linux-x86-64.so.2 \
        /lib/x86_64-linux-gnu/libc.so.6 \
        /lib/x86_64-linux-gnu/libgcc_s.so.1 \
        /lib/x86_64-linux-gnu/libm.so.6 \
        /lib/x86_64-linux-gnu/libpthread.so.0 \
        /lib/x86_64-linux-gnu/libdl.so.2; do
        cp -f "$lib" "$ROOTFS/lib/x86_64-linux-gnu/"
        chmod 0755 "$ROOTFS/lib/x86_64-linux-gnu/$(basename "$lib")"
    done
    # ld-linux expects /lib64/ld-linux-x86-64.so.2 as well (ELF interpreter path)
    ln -sf /lib/x86_64-linux-gnu/ld-linux-x86-64.so.2 \
        "$ROOTFS/lib64/ld-linux-x86-64.so.2"
    log_info "Installed glibc runtime for wasmtime"

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

    # ── Boot configuration ────────────────────────────────────────────────────
    cp "$PROJECT_ROOT/base/modules/scripts/boot.toml" "$ROOTFS/etc/vyoma/boot.toml"
    log_info "Installed boot.toml"

    # ── Rust supervisor binary ────────────────────────────────────────────────
    local supervisor_bin="$PROJECT_ROOT/target/x86_64-unknown-linux-musl/release/supervisor"
    if [[ -f "$supervisor_bin" ]]; then
        cp -f "$supervisor_bin" "$ROOTFS/usr/bin/supervisor"
        chmod 0755 "$ROOTFS/usr/bin/supervisor"
        log_info "Installed supervisor ($(du -h "$supervisor_bin" | cut -f1))"
    else
        log_info "WARNING: supervisor binary not found, /init will fall back to shell"
    fi

    # ── WASM apps ─────────────────────────────────────────────────────────────
    # Each app gets its own subdirectory: /apps/<name>/<name>.wasm + vyoma.toml
    # This layout matches the manifest paths declared in boot.toml.
    for app_src_dir in "$PROJECT_ROOT/apps"/*/; do
        local app_name
        app_name="$(basename "$app_src_dir")"
        local wasm_file="$app_src_dir/target/wasm32-wasip2/release/${app_name}.wasm"
        local manifest_file="$app_src_dir/vyoma.toml"

        [[ -f "$wasm_file" && -f "$manifest_file" ]] || continue

        mkdir -p "$ROOTFS/apps/${app_name}"
        cp "$wasm_file"     "$ROOTFS/apps/${app_name}/${app_name}.wasm"
        cp "$manifest_file" "$ROOTFS/apps/${app_name}/vyoma.toml"
        local icon_file="$app_src_dir/icon.png"
        if [[ -f "$icon_file" ]]; then
            cp "$icon_file" "$ROOTFS/apps/${app_name}/icon.png"
        fi
        log_info "  Included: ${app_name} ($(du -h "$wasm_file" | cut -f1))"
    done

    # ── Font files for scalable rendering ────────────────────────────────────
    mkdir -p "$ROOTFS/fonts"
    cp -r base/fonts/*.ttf "$ROOTFS/fonts/" 2>/dev/null || true

    # ── Pack initramfs ────────────────────────────────────────────────────────
    log_info "Packing initramfs -> $INITRAMFS_FILE"
    (
        cd "$ROOTFS"
        find . | cpio --quiet -H newc -o
    ) | gzip -9 > "$INITRAMFS_FILE"

    log_success "Rootfs built: $INITRAMFS_FILE ($(du -sh "$INITRAMFS_FILE" | cut -f1))"
}

build_rootfs

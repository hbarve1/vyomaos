#!/usr/bin/env bash
# VyomaOS Root Filesystem Module
# Builds the explicit R1 payload; every selected artifact is mandatory.

set -euo pipefail

source "$(dirname "${BASH_SOURCE[0]}")/../config.sh"

ROOTFS="$OUTDIR/rootfs"

# ── Wasmtime musl-static binary ──────────────────────────────────────────────
WASMTIME_TARBALL="wasmtime-v${WASMTIME_VERSION}-x86_64-linux.tar.xz"
WASMTIME_URL="https://github.com/bytecodealliance/wasmtime/releases/download/v${WASMTIME_VERSION}/${WASMTIME_TARBALL}"
WASMTIME_CACHE="$OUTDIR/cache/${WASMTIME_TARBALL}"

# ── BusyBox musl-static binary ────────────────────────────────────────────────
BUSYBOX_URL="https://www.busybox.net/downloads/binaries/${BUSYBOX_VERSION}-x86_64-linux-musl/busybox"
BUSYBOX_CACHE="$OUTDIR/cache/busybox-${BUSYBOX_VERSION}-x86_64-musl"

# ── Download with SHA-256 verification ───────────────────────────────────────
download_verified() {
    download_file "$1" "$2" "$(basename "$2")" "$3"
}

build_rootfs() {
    log_info "Building R1 rootfs..."
    local supervisor_bin="$PROJECT_ROOT/target/x86_64-unknown-linux-musl/release/supervisor"
    local selected_apps
    selected_apps="$(bash "$PROJECT_ROOT/base/image-apps.sh")"
    local apps=()
    mapfile -t apps <<< "$selected_apps"
    local app_name required
    # Fail before downloads or replacing an existing rootfs/image.
    for required in "$supervisor_bin"; do
        [[ -s "$required" && -x "$required" ]] || {
            log_error "Required R1 supervisor missing, empty or not executable: $required (run make supervisor)"
            return 1
        }
    done
    for app_name in "${apps[@]}"; do
        for required in \
            "$PROJECT_ROOT/apps/$app_name/vyoma.toml" \
            "$PROJECT_ROOT/apps/$app_name/target/wasm32-wasip2/release/$app_name.wasm"; do
            [[ -s "$required" ]] || {
                log_error "Required R1 app artifact missing or empty: $required (run make apps)"
                return 1
            }
        done
    done

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
    cat > "$ROOTFS/init" << 'INIT_EOF'
#!/bin/sh
set -e
/bin/mount -t proc  none /proc
/bin/mount -t sysfs none /sys
echo "VyomaOS booting..."
exec /usr/bin/supervisor
INIT_EOF
    chmod 0755 "$ROOTFS/init"

    # ── Minimal /etc stubs ────────────────────────────────────────────────────
    printf 'root:x:0:0:root:/root:/bin/sh\n' > "$ROOTFS/etc/passwd"
    printf 'root:x:0:\n'                     > "$ROOTFS/etc/group"

    # ── Boot configuration ────────────────────────────────────────────────────
    # Boot entries and installed payload derive from the same allowlist.
    printf '# Generated from base/r1-apps.txt; T1 console integration pending.\n' > "$ROOTFS/etc/vyoma/boot.toml"
    for app_name in "${apps[@]}"; do
        printf '\n[[apps]]\nmanifest = "/apps/%s/vyoma.toml"\nrestart = "never"\n' "$app_name" >> "$ROOTFS/etc/vyoma/boot.toml"
    done
    cp "$PROJECT_ROOT/base/r1-apps.txt" "$ROOTFS/etc/vyoma/r1-apps.txt"

    # ── Rust supervisor binary ────────────────────────────────────────────────
    cp "$supervisor_bin" "$ROOTFS/usr/bin/supervisor"
    chmod 0755 "$ROOTFS/usr/bin/supervisor"

    # ── Selected WASM apps ────────────────────────────────────────────────────
    for app_name in "${apps[@]}"; do
        local app_src_dir="$PROJECT_ROOT/apps/$app_name"
        local wasm_file="$app_src_dir/target/wasm32-wasip2/release/$app_name.wasm"
        local manifest_file="$app_src_dir/vyoma.toml"

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
    cp -r "$PROJECT_ROOT"/base/fonts/*.ttf "$ROOTFS/fonts/" 2>/dev/null || true

    # ── Pack initramfs ────────────────────────────────────────────────────────
    log_info "Packing initramfs -> $INITRAMFS_FILE"
    local packed
    packed="$(mktemp "$OUTDIR/initramfs.cpio.gz.tmp.XXXXXX")"
    (
        cd "$ROOTFS"
        find . | cpio --quiet -H newc -o
    ) | gzip -9 > "$packed" || { rm -f "$packed"; return 1; }
    mv "$packed" "$INITRAMFS_FILE"

    log_success "Rootfs built: $INITRAMFS_FILE ($(du -sh "$INITRAMFS_FILE" | cut -f1))"
}

build_rootfs

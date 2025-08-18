#!/usr/bin/env bash
# VyomaOS Root Filesystem Module

source "$(dirname "${BASH_SOURCE[0]}")/../config.sh"

# Get the absolute directory of this script at load time
ROOTFS_MODULE_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

create_wasm_runtime() {
    local script_path="$ROOTFS_MODULE_DIR/scripts/wasmtime"
    
    if [[ -f "$script_path" ]]; then
        cp "$script_path" "$ROOTFS_DIR/usr/bin/wasmtime"
        chmod +x "$ROOTFS_DIR/usr/bin/wasmtime"
    else
        log_error "WASM runtime script not found: $script_path"
        return 1
    fi
}

create_init_script() {
    local script_path="$ROOTFS_MODULE_DIR/scripts/init"
    
    if [[ -f "$script_path" ]]; then
        cp "$script_path" "$ROOTFS_DIR/init"
        chmod +x "$ROOTFS_DIR/init"
    else
        log_error "Init script not found: $script_path"
        return 1
    fi
}

include_wasm_apps() {
    # Detect project root and copy WASM applications
    if [[ -d "/home/hbarve1/codes/vyomaos/apps/build" ]]; then
        APPS_BUILD_DIR="/home/hbarve1/codes/vyomaos/apps/build"
    else
        APPS_BUILD_DIR="$(dirname "$(dirname "${BASH_SOURCE[0]}")")/apps/build"
    fi
    
    if [[ -d "$APPS_BUILD_DIR" ]]; then
        echo "Including WASM applications:"
        for wasm_file in "$APPS_BUILD_DIR"/*.wasm; do
            if [[ -f "$wasm_file" ]]; then
                cp "$wasm_file" "$ROOTFS_DIR/apps/"
                echo "  $(basename "$wasm_file") ($(du -h "$wasm_file" | cut -f1))"
            fi
        done
    fi
}

build_rootfs() {
    log_info "Building rootfs..."
    
    # Create directory structure
    mkdir -p "$ROOTFS_DIR"/{bin,proc,sys,dev,usr/bin,apps}
    
    # Create VyomaOS marker file
    echo "VyomaOS $(date)" > "$ROOTFS_DIR/.vyomaos"
    
    # Setup BusyBox
    download_file "$BUSYBOX_URL" "$OUTDIR/busybox" "BusyBox" || return 1
    chmod +x "$OUTDIR/busybox"
    cp "$OUTDIR/busybox" "$ROOTFS_DIR/bin/busybox"
    
    # Create BusyBox symlinks
    for cmd in sh echo mount poweroff wc od head strings tr grep tail sort basename cut; do
        ln -sf /bin/busybox "$ROOTFS_DIR/bin/$cmd"
    done
    
    # Create WebAssembly runtime
    create_wasm_runtime || return 1
    
    # Include WASM applications
    include_wasm_apps
    
    # Create init script
    create_init_script || return 1
    
    # Build initramfs
    cd "$ROOTFS_DIR"
    find . | cpio -o -H newc 2>/dev/null | gzip > "$INITRAMFS_FILE"
    
    return 0
}

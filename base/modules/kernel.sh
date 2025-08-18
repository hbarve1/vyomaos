#!/usr/bin/env bash
# VyomaOS Kernel Module

source "$(dirname "${BASH_SOURCE[0]}")/../config.sh"

build_kernel() {
    [[ -f "$KERNEL_FILE" ]] && return 0
    log_info "Building kernel..."
    
    download_file "$KERNEL_SOURCE_URL" "$KERNEL_TAR_FILE" "kernel source" || return 1
    
    mkdir -p "$OUTDIR"
    tar -xJf "$KERNEL_TAR_FILE" -C "$OUTDIR"
    
    cd "$KERNEL_SOURCE_DIR"
    make defconfig >/dev/null 2>&1
    
    # Essential kernel config for VyomaOS
    {
        echo "CONFIG_SERIAL_8250=y"
        echo "CONFIG_SERIAL_8250_CONSOLE=y"
        echo "CONFIG_DEVTMPFS=y"
        echo "CONFIG_DEVTMPFS_MOUNT=y"
    } >> .config
    
    make -j$(nproc) bzImage >/dev/null 2>&1 || return 1
    cp arch/x86/boot/bzImage "$KERNEL_FILE"
    return 0
}

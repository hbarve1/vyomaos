#!/usr/bin/env bash
# VyomaOS Minimal Kernel Module

source "$(dirname "${BASH_SOURCE[0]}")/../config.sh"
source "$(dirname "${BASH_SOURCE[0]}")/utils.sh"

build_kernel() {
    [[ -f "$KERNEL_FILE" ]] && return 0
    log_info "Building kernel..."
    
    download_file "$KERNEL_SOURCE_URL" "$KERNEL_TAR_FILE" "kernel source" || return 1
    
    mkdir -p "$OUTDIR"
    tar -xJf "$KERNEL_TAR_FILE" -C "$OUTDIR"
    
    cd "$KERNEL_SOURCE_DIR"
    make defconfig >/dev/null 2>&1
    echo "CONFIG_SERIAL_8250=y" >> .config
    echo "CONFIG_SERIAL_8250_CONSOLE=y" >> .config
    echo "CONFIG_DEVTMPFS=y" >> .config
    echo "CONFIG_DEVTMPFS_MOUNT=y" >> .config
    
    make -j$(nproc) bzImage >/dev/null 2>&1 || return 1
    cp arch/x86/boot/bzImage "$KERNEL_FILE"
    log_success "Kernel built."
    return 0
}

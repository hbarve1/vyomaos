#!/usr/bin/env bash
# VyomaOS Minimal QEMU Module

source "$(dirname "${BASH_SOURCE[0]}")/../config.sh"

boot_system() {
    command -v qemu-system-x86_64 &>/dev/null || { log_error "QEMU not found"; return 1; }
    [[ -f "$KERNEL_FILE" && -f "$INITRAMFS_FILE" ]] || { log_error "Missing files"; return 1; }
    
    log_info "Booting VyomaOS..."
    qemu-system-x86_64 \
        -kernel "$KERNEL_FILE" \
        -initrd "$INITRAMFS_FILE" \
        -cpu qemu64 \
        -m 512M \
        -accel tcg \
        -append "console=ttyS0 loglevel=3" \
        -nographic
    
    log_success "Session completed."
    return 0
}

#!/usr/bin/env bash
# VyomaOS QEMU Module

source "$(dirname "${BASH_SOURCE[0]}")/../config.sh"

boot_system() {
    [[ -f "$KERNEL_FILE" && -f "$INITRAMFS_FILE" ]] || {
        log_error "Missing build files. Run: ./vyomaos.sh build"
        return 1
    }
    
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
}

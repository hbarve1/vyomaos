#!/usr/bin/env bash
# VyomaOS QEMU Module

source "$(dirname "${BASH_SOURCE[0]}")/../config.sh"

boot_system() {
    [[ -f "$KERNEL_FILE" && -f "$INITRAMFS_FILE" ]] || {
        log_error "Missing build files. Run: ./vyomaos.sh build"
        return 1
    }
    
    # Check initramfs size and adjust memory accordingly
    initramfs_size=$(du -m "$INITRAMFS_FILE" | cut -f1)
    if [ "$initramfs_size" -gt 10 ]; then
        memory="1G"
        log_info "Booting VyomaOS with large initramfs (${initramfs_size}MB)..."
    else
        memory="512M"
        log_info "Booting VyomaOS..."
    fi
    
    qemu-system-x86_64 \
        -kernel "$KERNEL_FILE" \
        -initrd "$INITRAMFS_FILE" \
        -cpu qemu64 \
        -m "$memory" \
        -accel tcg \
        -append "console=ttyS0 loglevel=3" \
        -nographic
    
    log_success "Session completed."
}

#!/usr/bin/env bash
set -euo pipefail

# VyomaOS Run Script
# Boots VyomaOS in QEMU.

# Load configuration
source "$(dirname "${BASH_SOURCE[0]}")/../config.sh"

check_qemu() {
    if ! command -v qemu-system-x86_64 &> /dev/null; then
        log_error "QEMU not found. Please install it with: sudo apt-get install qemu-system-x86"
        exit 1
    fi
}

check_files() {
    if [[ ! -f "$KERNEL_FILE" || ! -f "$INITRAMFS_FILE" ]]; then
        log_error "Kernel or initramfs not found."
        log_info "Please run './vyomaos.sh build' first."
        exit 1
    fi
}

run_qemu() {
    log_info "Starting VyomaOS in QEMU..."
    
    qemu-system-x86_64 \
        -kernel "$KERNEL_FILE" \
        -initrd "$INITRAMFS_FILE" \
        -cpu "$QEMU_CPU" \
        -m "$QEMU_MEMORY" \
        -smp "$QEMU_SMP" \
        $QEMU_ACCEL \
        -append "$QEMU_KERNEL_ARGS" \
        -nographic
        
    log_success "QEMU session finished."
}

main() {
    log_info "VyomaOS Boot Script"
    
    check_qemu
    check_files
    run_qemu
}

main

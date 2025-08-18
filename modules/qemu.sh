#!/usr/bin/env bash
# VyomaOS QEMU Module
# Handles QEMU virtualization and system boot

# Load dependencies
source "$(dirname "${BASH_SOURCE[0]}")/../config.sh"
source "$(dirname "${BASH_SOURCE[0]}")/utils.sh"

# Check QEMU availability
check_qemu() {
    if ! command_exists qemu-system-x86_64; then
        log_error "QEMU not found. Install with: sudo apt-get install qemu-system-x86"
        return 1
    fi
    return 0
}

# Validate required files exist
check_files() {
    local missing=()
    
    if [[ ! -f "$KERNEL_FILE" ]]; then
        missing+=("kernel")
    fi
    
    if [[ ! -f "$INITRAMFS_FILE" ]]; then
        missing+=("initramfs")
    fi
    
    if [[ ${#missing[@]} -gt 0 ]]; then
        log_error "Missing files: ${missing[*]}"
        log_info "Run './vyomaos.sh build' first."
        return 1
    fi
    
    return 0
}

# Boot VyomaOS in QEMU
boot_system() {
    log_info "Booting VyomaOS..."
    
    check_qemu || return 1
    check_files || return 1
    
    echo ""
    qemu-system-x86_64 \
        -kernel "$KERNEL_FILE" \
        -initrd "$INITRAMFS_FILE" \
        -cpu "$QEMU_CPU" \
        -m "$QEMU_MEMORY" \
        -accel tcg \
        -append "$QEMU_ARGS" \
        -nographic
    
    echo ""
    log_success "System session completed."
    return 0
}

#!/usr/bin/env bash
# VyomaOS - Minimal WebAssembly Operating System

set -e

# Load configuration
source "config.sh"

# Load modules
source "modules/utils.sh"
source "modules/kernel.sh"
source "modules/rootfs.sh"
source "modules/qemu.sh"

show_banner() {
    echo ""
    echo "  █     █ █     █ █████ █   █ █████"
    echo "  █     █ █     █ █   █ ██ ██ █   █"
    echo "  █     █ █  █  █ █   █ █ █ █ █████"
    echo "   █   █   █ █ █  █   █ █   █ █   █"
    echo "    █ █     █ █   █████ █   █ █   █"
    echo ""
    echo "  A Minimal WebAssembly Operating System"
    echo ""
}

cmd_build() {
    log_info "Building VyomaOS..."
    check_system_deps
    build_kernel
    build_rootfs
    log_success "Build complete!"
}

cmd_run() {
    boot_system
}

cmd_clean() {
    log_info "Cleaning up..."
    rm -rf "$KERNEL_DIR" "$ROOTFS_DIR" "$KERNEL_FILE" "$INITRAMFS_FILE" &>/dev/null || true
    log_success "Cleanup complete!"
}

main() {
    show_banner
    
    case "${1:-}" in
        build) cmd_build ;;
        run) cmd_run ;;
        clean) cmd_clean ;;
        *) echo "Usage: $0 {build|run|clean}"; exit 1 ;;
    esac
}

main "$@"

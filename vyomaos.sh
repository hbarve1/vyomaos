#!/usr/bin/env bash
set -euo pipefail

# VyomaOS Main Entry Point
# Modular minimal WebAssembly operating system

# Load configuration and modules
readonly SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/config.sh"
source "$SCRIPT_DIR/modules/utils.sh"
source "$SCRIPT_DIR/modules/kernel.sh"
source "$SCRIPT_DIR/modules/rootfs.sh"
source "$SCRIPT_DIR/modules/qemu.sh"

# Show project banner
show_banner() {
    cat << 'EOF'
 __     __                            ___  ____  
 \ \   / /   _ ___  _ __ ___   __ _   / _ \/ ___| 
  \ \ / / | | / _ \| '_ ` _ \ / _` | | | | \___ \ 
   \ V /| |_| (_) | | | | | | (_| | | |_| |___) |
    \_/  \__, \___/|_| |_| |_|\__,_|  \___/|____/ 
         |___/                                    
Minimal WebAssembly Operating System
EOF
}

# Show usage information
show_help() {
    echo ""
    echo "Usage: $0 [COMMAND]"
    echo ""
    echo "Commands:"
    echo "  build    Build the complete operating system"
    echo "  run      Boot VyomaOS in QEMU"
    echo "  clean    Remove all build artifacts"
    echo "  help     Show this help message"
    echo ""
    echo "Quick start:"
    echo "  $0 build && $0 run"
    echo ""
}

# Build complete system
cmd_build() {
    log_info "Building VyomaOS..."
    
    check_system_deps || exit 1
    create_dirs "$OUTDIR"
    
    build_kernel || exit 1
    build_rootfs || exit 1
    
    log_success "VyomaOS build complete!"
}

# Run system in QEMU
cmd_run() {
    boot_system
}

# Clean build artifacts
cmd_clean() {
    log_info "Cleaning build artifacts..."
    rm -rf "$OUTDIR"
    log_success "Clean complete."
}

# Main entry point
main() {
    show_banner
    
    case "${1:-help}" in
        build)   cmd_build ;;
        run)     cmd_run ;;
        clean)   cmd_clean ;;
        help|-h|--help) show_help ;;
        *)
            log_error "Unknown command: $1"
            show_help
            exit 1
            ;;
    esac
}

main "$@"

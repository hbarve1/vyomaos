#!/usr/bin/env bash
set -euo pipefail

# VyomaOS Main Entry Point
# Unified interface for all VyomaOS operations

# Configuration
readonly SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
readonly SCRIPTS_DIR="$SCRIPT_DIR/scripts"

# Colors for output
readonly RED='\033[0;31m'
readonly GREEN='\033[0;32m'
readonly YELLOW='\033[1;33m'
readonly BLUE='\033[0;34m'
readonly NC='\033[0m' # No Color

# Logging functions
log_info() { echo -e "${BLUE}ℹ️  $1${NC}"; }
log_success() { echo -e "${GREEN}✅ $1${NC}"; }
log_warning() { echo -e "${YELLOW}⚠️  $1${NC}"; }
log_error() { echo -e "${RED}❌ $1${NC}"; }

show_banner() {
    cat << 'EOF'
__     __  __  ___                        ___  ____  
\ \   / / |  |/  / |__   ___  _ __ ___   / _ \/ ___| 
 \ \ / /  | '  /| '_ \ / _ \| '_ ` _ \ | | | \___ \ 
  \ V /   | . \| | | | (_) | | | | | || |_| |___) |
   \_/    |_|\_\_| |_|\___/|_| |_| |_| \___/|____/ 
                                                     
WebAssembly Operating System
EOF
}

show_help() {
    cat << EOF
VyomaOS - WebAssembly Operating System

Usage: $0 [COMMAND] [OPTIONS]

Commands:
  build       Build VyomaOS (download components, create initramfs)
  kernel      Manage Linux kernel (download, build, validate)
  run         Boot VyomaOS in QEMU
  clean       Clean build artifacts
  status      Show current status
  help        Show this help message

Kernel Commands:
  kernel status     Check kernel status
  kernel download   Download pre-built kernel (may not work)
  kernel docker     Build kernel using Docker
  kernel local      Build kernel locally (requires Make 4.0+)
  kernel manual     Manual kernel placement

Examples:
  $0 build                    # Build VyomaOS
  $0 kernel docker            # Build kernel using Docker
  $0 run                      # Boot VyomaOS
  $0 build && $0 kernel docker && $0 run  # Complete workflow

Quick Start:
  1. $0 build                 # Build the OS
  2. $0 kernel docker         # Get a working kernel
  3. $0 run                   # Boot VyomaOS

For more information, see README.md
EOF
}

check_scripts_dir() {
    if [[ ! -d "$SCRIPTS_DIR" ]]; then
        log_error "Scripts directory not found: $SCRIPTS_DIR"
        log_info "Please ensure you're running this from the VyomaOS project root"
        exit 1
    fi
}

run_command() {
    local command="$1"
    local script="$2"
    shift 2
    
    if [[ ! -f "$script" ]]; then
        log_error "Script not found: $script"
        exit 1
    fi
    
    if [[ ! -x "$script" ]]; then
        log_warning "Making script executable: $script"
        chmod +x "$script"
    fi
    
    log_info "Running: $command"
    echo ""
    "$script" "$@"
}

main() {
    # Show banner
    show_banner
    echo ""
    
    # Check scripts directory
    check_scripts_dir
    
    # Parse command line arguments
    case "${1:-help}" in
        build)
            run_command "build" "$SCRIPTS_DIR/build.sh"
            ;;
        kernel)
            run_command "kernel" "$SCRIPTS_DIR/kernel.sh" "${@:2}"
            ;;
        run)
            run_command "run" "$SCRIPTS_DIR/run.sh"
            ;;
        clean)
            log_info "Cleaning build artifacts..."
            rm -rf "$SCRIPT_DIR/out"
            log_success "Clean completed"
            ;;
        status)
            log_info "VyomaOS Status"
            echo "=================="
            
            # Check if out directory exists
            if [[ -d "$SCRIPT_DIR/out" ]]; then
                log_success "Build directory exists"
                
                # Check kernel
                if [[ -f "$SCRIPT_DIR/out/bzImage" ]]; then
                    log_info "Kernel file: $SCRIPT_DIR/out/bzImage"
                    file "$SCRIPT_DIR/out/bzImage"
                else
                    log_warning "No kernel file found"
                fi
                
                # Check initramfs
                if [[ -f "$SCRIPT_DIR/out/initramfs.cpio.gz" ]]; then
                    log_success "Initramfs exists"
                else
                    log_warning "No initramfs found"
                fi
            else
                log_warning "Build directory not found"
                log_info "Run '$0 build' to create build artifacts"
            fi
            
            # Check dependencies
            echo ""
            log_info "Dependencies:"
            local deps=("curl" "tar" "gzip" "base64" "qemu-system-x86_64")
            for dep in "${deps[@]}"; do
                if command -v "$dep" &> /dev/null; then
                    log_success "$dep: found"
                else
                    log_error "$dep: missing"
                fi
            done
            ;;
        help|--help|-h)
            show_help
            ;;
        *)
            log_error "Unknown command: $1"
            echo ""
            show_help
            exit 1
            ;;
    esac
}

# Run main function
main "$@"

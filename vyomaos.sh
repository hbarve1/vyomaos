#!/usr/bin/env bash
set -euo pipefail

# VyomaOS Main Entry Point
# Unified interface for all VyomaOS operations

# Configuration
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SCRIPTS_DIR="$SCRIPT_DIR/scripts"
source "$SCRIPT_DIR/config.sh"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

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

VyomaOS - A minimal Linux-based OS for running WebAssembly applications.

Usage: $0 [COMMAND]

Commands:
  build       Build the entire OS, including the kernel.
  run         Boot VyomaOS in QEMU.
  clean       Clean all build artifacts.
  help        Show this help message.

Workflow:
  1. $0 build
  2. $0 run
EOF
}

run_script() {
    local script_name="$1"
    shift
    local script_path="$SCRIPTS_DIR/$script_name"

    if [[ ! -f "$script_path" ]]; then
        log_error "Script not found: $script_path"
        exit 1
    fi
    
    log_info "Executing $script_name..."
    echo ""
    bash "$script_path" "$@"
}

main() {
    show_banner
    
    case "${1:-help}" in
        build)
            run_script "build.sh"
            ;;
        run)
            run_script "run.sh"
            ;;
        clean)
            log_info "Cleaning build artifacts..."
            rm -rf "$OUTDIR"
            log_success "Clean completed."
            ;;
        help|--help|-h)
            show_help
            ;;
        *)
            log_error "Unknown command: $1"
            show_help
            exit 1
            ;;
    esac
}

main "$@"

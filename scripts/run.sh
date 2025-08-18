#!/usr/bin/env bash
set -euo pipefail

# VyomaOS Run Script
# Boots VyomaOS in QEMU

# Configuration
readonly SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
readonly PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"
readonly OUTDIR="$PROJECT_ROOT/out"
readonly KERNEL_FILE="$OUTDIR/bzImage"
readonly INITRAMFS_FILE="$OUTDIR/initramfs.cpio.gz"

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

# QEMU configuration
readonly QEMU_MEMORY="512M"
readonly QEMU_CPU="host"
readonly QEMU_SMP="1"

# Utility functions
check_dependencies() {
    if ! command -v qemu-system-x86_64 &> /dev/null; then
        log_error "QEMU not found. Please install QEMU first."
        log_info "macOS: brew install qemu"
        log_info "Ubuntu: sudo apt-get install qemu-system-x86"
        exit 1
    fi
    
    log_success "QEMU found"
}

check_files() {
    local missing_files=()
    
    if [[ ! -f "$KERNEL_FILE" ]]; then
        missing_files+=("kernel")
    fi
    
    if [[ ! -f "$INITRAMFS_FILE" ]]; then
        missing_files+=("initramfs")
    fi
    
    if [[ ${#missing_files[@]} -gt 0 ]]; then
        log_error "Missing required files: ${missing_files[*]}"
        log_info "Run './scripts/build.sh' to build VyomaOS first"
        exit 1
    fi
    
    log_success "All required files found"
}

check_kernel_validity() {
    if [[ -f "$KERNEL_FILE" ]]; then
        if file "$KERNEL_FILE" | grep -q "ELF"; then
            log_warning "Kernel is an ELF executable (not a real kernel)"
            log_info "This will cause QEMU boot issues"
            log_info "Run './scripts/kernel.sh' to fix this"
            return 1
        elif file "$KERNEL_FILE" | grep -q "data\|HTML"; then
            log_warning "Kernel is a placeholder or invalid file"
            log_info "This will cause QEMU boot issues"
            log_info "Run './scripts/kernel.sh' to fix this"
            return 1
        fi
    fi
    
    return 0
}

detect_acceleration() {
    # Try hvf (macOS), fallback to tcg
    if qemu-system-x86_64 -accel help 2>/dev/null | grep -q hvf; then
        echo "hvf"
    else
        echo "tcg"
    fi
}

show_qemu_info() {
    log_info "QEMU Configuration:"
    echo "  Kernel: $KERNEL_FILE"
    echo "  Initramfs: $INITRAMFS_FILE"
    echo "  Memory: $QEMU_MEMORY"
    echo "  Acceleration: $ACCEL"
    echo "  CPU: $QEMU_CPU"
    echo ""
}

run_qemu() {
    log_info "Starting VyomaOS in QEMU..."
    echo ""
    
    # QEMU command
    qemu-system-x86_64 \
        -machine accel="$ACCEL" \
        -cpu "$QEMU_CPU" \
        -smp "$QEMU_SMP" \
        -m "$QEMU_MEMORY" \
        -kernel "$KERNEL_FILE" \
        -initrd "$INITRAMFS_FILE" \
        -nographic \
        -append "console=ttyS0 loglevel=3" \
        -no-reboot \
        -no-shutdown
}

main() {
    log_info "VyomaOS Boot Script"
    echo "======================"
    
    # Check dependencies
    check_dependencies
    
    # Check required files
    check_files
    
    # Check kernel validity
    if ! check_kernel_validity; then
        log_warning "Kernel may not work properly"
        read -p "Continue anyway? (y/N): " -r
        if [[ ! $REPLY =~ ^[Yy]$ ]]; then
            log_info "Boot cancelled"
            exit 0
        fi
    fi
    
    # Detect acceleration
    ACCEL=$(detect_acceleration)
    
    # Show configuration
    show_qemu_info
    
    # Run QEMU
    run_qemu
}

# Handle Ctrl+C gracefully
trap 'echo -e "\n${YELLOW}⚠️  QEMU interrupted${NC}"; exit 0' INT

# Run main function
main "$@"

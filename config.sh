#!/usr/bin/env bash
# VyomaOS Minimal Configuration

# Include guard
if [[ -n "${VYOMAOS_CONFIG_SOURCED:-}" ]]; then
    return 0
fi
readonly VYOMAOS_CONFIG_SOURCED=1

# Paths
readonly PROJECT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
readonly OUTDIR="$PROJECT_ROOT/out"
readonly ROOTFS_DIR="$OUTDIR/rootfs"

# URLs
readonly KERNEL_SOURCE_URL="https://www.kernel.org/pub/linux/kernel/v5.x/linux-5.10.113.tar.xz"
readonly BUSYBOX_URL="https://busybox.net/downloads/binaries/1.35.0-x86_64-linux-musl/busybox"

# Files
readonly KERNEL_FILE="$OUTDIR/bzImage"
readonly INITRAMFS_FILE="$OUTDIR/initramfs.cpio.gz"
readonly KERNEL_SOURCE_DIR="$OUTDIR/linux-5.10.113"
readonly KERNEL_TAR_FILE="$OUTDIR/linux-5.10.113.tar.xz"

# Logging
log_info() { echo "ℹ️  $1"; }
log_success() { echo "✅ $1"; }
log_error() { echo "❌ $1"; }

# Utility functions
ensure_outdir() {
    mkdir -p "$OUTDIR"
}

get_system_info() {
    echo "System: $(uname -s)"
    echo "Architecture: $(uname -m)"
    echo "Kernel: $(uname -r)"
}

check_dependency() {
    local dep="$1"
    if ! command -v "$dep" &> /dev/null; then
        log_error "Missing dependency: $dep"
        return 1
    fi
    return 0
}

check_dependencies() {
    local deps=("curl" "tar" "gzip" "base64")
    local missing=()
    
    for dep in "${deps[@]}"; do
        if ! check_dependency "$dep"; then
            missing+=("$dep")
        fi
    done
    
    if [[ ${#missing[@]} -gt 0 ]]; then
        log_error "Missing dependencies: ${missing[*]}"
        return 1
    fi
    
    return 0
}

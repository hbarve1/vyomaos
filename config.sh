#!/usr/bin/env bash
# VyomaOS Configuration File
# Minimal configuration for modular architecture

# Include guard
if [[ -n "${VYOMAOS_CONFIG_SOURCED:-}" ]]; then
    return 0
fi
readonly VYOMAOS_CONFIG_SOURCED=1

# Project metadata
readonly PROJECT_NAME="VyomaOS"
readonly PROJECT_VERSION="1.2.0"

# Paths
readonly PROJECT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
readonly OUTDIR="$PROJECT_ROOT/out"
readonly ROOTFS_DIR="$OUTDIR/rootfs"

# Component URLs
readonly KERNEL_SOURCE_URL="https://www.kernel.org/pub/linux/kernel/v5.x/linux-5.10.113.tar.xz"
readonly BUSYBOX_URL="https://busybox.net/downloads/binaries/1.35.0-x86_64-linux-musl/busybox"

# Build settings
readonly KERNEL_VERSION="5.10.113"
readonly BUILD_JOBS=$(nproc)

# QEMU settings
readonly QEMU_MEMORY="512M"
readonly QEMU_CPU="qemu64"
readonly QEMU_ARGS="console=ttyS0 loglevel=3"

# File paths
readonly KERNEL_FILE="$OUTDIR/bzImage"
readonly INITRAMFS_FILE="$OUTDIR/initramfs.cpio.gz"
readonly KERNEL_SOURCE_DIR="$OUTDIR/linux-$KERNEL_VERSION"
readonly KERNEL_TAR_FILE="$OUTDIR/linux-$KERNEL_VERSION.tar.xz"

# Essential kernel config options
readonly KERNEL_CONFIG_OPTS=(
    "CONFIG_SERIAL_8250=y"
    "CONFIG_SERIAL_8250_CONSOLE=y"
    "CONFIG_DEVTMPFS=y"
    "CONFIG_DEVTMPFS_MOUNT=y"
)

# Colors and logging
readonly RED='\033[0;31m'
readonly GREEN='\033[0;32m'
readonly YELLOW='\033[1;33m'
readonly BLUE='\033[0;34m'
readonly NC='\033[0m'

log_info() { echo -e "${BLUE}ℹ️  $1${NC}"; }
log_success() { echo -e "${GREEN}✅ $1${NC}"; }
log_warning() { echo -e "${YELLOW}⚠️  $1${NC}"; }
log_error() { echo -e "${RED}❌ $1${NC}"; }

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

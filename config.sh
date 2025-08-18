#!/usr/bin/env bash
# VyomaOS Configuration File
# Centralized configuration for all scripts

# Include guard
if [[ -n "${VYOMAOS_CONFIG_SOURCED:-}" ]]; then
    return 0
fi
VYOMAOS_CONFIG_SOURCED=1

# Project information
PROJECT_NAME="VyomaOS"
PROJECT_VERSION="1.1.0"
PROJECT_DESCRIPTION="A minimal Linux-based OS for running WebAssembly applications on Ubuntu."

# Directory structure
PROJECT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SCRIPTS_DIR="$PROJECT_ROOT/scripts"
OUTDIR="$PROJECT_ROOT/out"
KERNEL_SOURCE_DIR="$OUTDIR/linux-5.10.113"

# Component URLs
BUSYBOX_URL="https://busybox.net/downloads/binaries/1.35.0-x86_64-linux-musl/busybox"
WASMTIME_URL="https://github.com/bytecodealliance/wasmtime/releases/download/v16.0.0/wasmtime-v16.0.0-x86_64-linux.tar.xz"
KERNEL_SOURCE_URL="https://www.kernel.org/pub/linux/kernel/v5.x/linux-5.10.113.tar.xz"
KERNEL_TAR_FILE="$OUTDIR/$(basename "$KERNEL_SOURCE_URL")"

# Kernel configuration
KERNEL_VERSION="5.10.113"
KERNEL_CONFIG_OPTS=(
    "CONFIG_SERIAL_8250=y"
    "CONFIG_SERIAL_8250_CONSOLE=y"
    "CONFIG_DEVTMPFS=y"
    "CONFIG_DEVTMPFS_MOUNT=y"
    "CONFIG_VIRTIO_PCI=y"
    "CONFIG_VIRTIO_BLK=y"
    "CONFIG_VIRTIO_NET=y"
    "CONFIG_9P_FS=y"
    "CONFIG_NET_9P=y"
    "CONFIG_NET_9P_VIRTIO=y"
)

# QEMU configuration
QEMU_MEMORY="512M"
QEMU_CPU="qemu64"
QEMU_SMP=$(nproc)
QEMU_ACCEL="-accel tcg" # Use software acceleration for compatibility
QEMU_KERNEL_ARGS="console=ttyS0 loglevel=3"

# Build configuration
BUILD_PARALLEL_JOBS=$(nproc)

# File paths
KERNEL_FILE="$OUTDIR/bzImage"
INITRAMFS_FILE="$OUTDIR/initramfs.cpio.gz"
BUSYBOX_FILE="$OUTDIR/busybox"
WASMTIME_DIR="$OUTDIR/wasmtime"
ROOTFS_DIR="$OUTDIR/rootfs"

# Colors for output
if [[ -z "${RED:-}" ]]; then
    RED='\033[0;31m'
    GREEN='\033[0;32m'
    YELLOW='\033[1;33m'
    BLUE='\033[0;34m'
    NC='\033[0m'
fi

# Logging functions
if ! declare -F log_info >/dev/null; then
    log_info() { echo -e "${BLUE}ℹ️  $1${NC}"; }
    log_success() { echo -e "${GREEN}✅ $1${NC}"; }
    log_warning() { echo -e "${YELLOW}⚠️  $1${NC}"; }
    log_error() { echo -e "${RED}❌ $1${NC}"; }
fi

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

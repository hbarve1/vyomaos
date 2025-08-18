#!/usr/bin/env bash
# VyomaOS Configuration File
# Centralized configuration for all scripts

# Include guard
if [[ -n "${VYOMAOS_CONFIG_SOURCED:-}" ]]; then
    return 0
fi
readonly VYOMAOS_CONFIG_SOURCED=1

# Project information
readonly PROJECT_NAME="VyomaOS"
readonly PROJECT_VERSION="1.0.0"
readonly PROJECT_DESCRIPTION="A minimal Linux-based OS for running WebAssembly applications"

# Directory structure
PROJECT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
readonly SCRIPTS_DIR="$PROJECT_ROOT/scripts"
readonly OUTDIR="$PROJECT_ROOT/out"
readonly DOCS_DIR="$PROJECT_ROOT/docs"

# Component URLs
readonly BUSYBOX_URL="https://busybox.net/downloads/binaries/1.35.0-x86_64-linux-musl/busybox"
readonly WASMTIME_URL="https://github.com/bytecodealliance/wasmtime/releases/download/v16.0.0/wasmtime-v16.0.0-x86_64-linux.tar.xz"
readonly KERNEL_SOURCE_URL="https://www.kernel.org/pub/linux/kernel/v5.x/linux-5.10.113.tar.xz"

# Kernel configuration
readonly KERNEL_VERSION="5.10.113"
readonly KERNEL_CONFIG_OPTS=(
    "CONFIG_SERIAL_8250=y"
    "CONFIG_SERIAL_8250_CONSOLE=y"
    "CONFIG_DEVTMPFS=y"
    "CONFIG_DEVTMPFS_MOUNT=y"
)

# QEMU configuration
readonly QEMU_MEMORY="512M"
readonly QEMU_CPU="qemu64"
readonly QEMU_SMP="1"
readonly QEMU_KERNEL_ARGS="console=ttyS0 loglevel=3"

# Build configuration
readonly BUILD_PARALLEL_JOBS=$(nproc 2>/dev/null || echo 4)
readonly DOWNLOAD_TIMEOUT=300

# File paths
readonly KERNEL_FILE="$OUTDIR/bzImage"
readonly INITRAMFS_FILE="$OUTDIR/initramfs.cpio.gz"
readonly BUSYBOX_FILE="$OUTDIR/busybox"
readonly WASMTIME_DIR="$OUTDIR/wasmtime"
readonly ROOTFS_DIR="$OUTDIR/rootfs"

# Colors for output (if not already defined)
if [[ -z "${RED:-}" ]]; then
    readonly RED='\033[0;31m'
    readonly GREEN='\033[0;32m'
    readonly YELLOW='\033[1;33m'
    readonly BLUE='\033[0;34m'
    readonly NC='\033[0m'
fi

# Logging functions (if not already defined)
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

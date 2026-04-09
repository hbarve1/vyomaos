#!/usr/bin/env bash
# VyomaOS Configuration

# Prevent multiple sourcing
[[ -n "${VYOMAOS_CONFIG_LOADED:-}" ]] && return 0
readonly VYOMAOS_CONFIG_LOADED=1

# Paths
readonly PROJECT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
readonly OUTDIR="$PROJECT_ROOT/out"
readonly ROOTFS_DIR="$OUTDIR/rootfs"
readonly KERNEL_FILE="$OUTDIR/bzImage"
readonly INITRAMFS_FILE="$OUTDIR/initramfs.cpio.gz"
readonly KERNEL_SOURCE_DIR="$OUTDIR/linux-5.10.113"
readonly KERNEL_TAR_FILE="$OUTDIR/linux-5.10.113.tar.xz"
readonly KERNEL_SOURCE_URL="https://www.kernel.org/pub/linux/kernel/v5.x/linux-5.10.113.tar.xz"

# Logging
log_info() { echo "ℹ️  $1"; }
log_success() { echo "✅ $1"; }
log_error() { echo "❌ $1"; }

# Download utility
download_file() {
    local url="$1" output="$2" description="$3"
    [[ -f "$output" ]] && return 0
    log_info "Downloading $description..."
    curl -L -o "$output" "$url"
}

# System dependencies check
check_system_deps() {
    local missing=()
    for cmd in curl tar gzip make gcc flex bison bc qemu-system-x86_64; do
        command -v "$cmd" &>/dev/null || missing+=("$cmd")
    done
    for pkg in libelf-dev libssl-dev libncurses-dev; do
        dpkg -s "$pkg" &>/dev/null || missing+=("$pkg")
    done
    if [[ ${#missing[@]} -gt 0 ]]; then
        log_error "Missing dependencies: ${missing[*]}"
        log_error "Install with: sudo apt-get install ${missing[*]}"
        return 1
    fi
    return 0
}

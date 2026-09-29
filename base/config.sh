#!/usr/bin/env bash
# VyomaOS Configuration

# Prevent multiple sourcing
[[ -n "${VYOMAOS_CONFIG_LOADED:-}" ]] && return 0
readonly VYOMAOS_CONFIG_LOADED=1

# Paths
readonly PROJECT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
source "$PROJECT_ROOT/base/versions.sh"
readonly OUTDIR="$PROJECT_ROOT/out"
readonly ROOTFS_DIR="$OUTDIR/rootfs"
readonly KERNEL_FILE="$OUTDIR/bzImage"
readonly INITRAMFS_FILE="$OUTDIR/initramfs.cpio.gz"
readonly KERNEL_SOURCE_DIR="$OUTDIR/linux-$KERNEL_VERSION"
readonly KERNEL_TAR_FILE="$OUTDIR/linux-$KERNEL_VERSION.tar.xz"
readonly KERNEL_SOURCE_URL="https://www.kernel.org/pub/linux/kernel/v5.x/linux-$KERNEL_VERSION.tar.xz"

# Logging
log_info() { echo "ℹ️  $1"; }
log_success() { echo "✅ $1"; }
log_error() { echo "❌ $1"; }

# Download utility
download_file() {
    local url="$1" output="$2" description="$3" expected="$4"
    local candidate="$output" temporary=""
    if [[ ! -f "$output" ]]; then
        mkdir -p "$(dirname "$output")" || return 1
        temporary="$(mktemp "${output}.tmp.XXXXXX")" || return 1
        candidate="$temporary"
        log_info "Downloading $description..."
        if ! curl --fail --location --silent --show-error "$url" -o "$candidate"; then
            rm -f "$temporary"
            return 1
        fi
    fi
    if ! echo "$expected  $candidate" | sha256sum --check --status; then
        log_error "SHA-256 mismatch for $description: $output"
        [[ -z "$temporary" ]] || rm -f "$temporary"
        return 1
    fi
    [[ -z "$temporary" ]] || mv "$temporary" "$output" || return 1
    log_info "SHA-256 OK: $description"
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

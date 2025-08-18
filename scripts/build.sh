#!/usr/bin/env bash
set -euo pipefail

# VyomaOS Build Script
# Builds a minimal Linux-based OS for running WebAssembly applications

# Configuration
readonly SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
readonly CONFIG_FILE="$(dirname "$SCRIPT_DIR")/config.sh"

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

# Load configuration
if [[ -f "$CONFIG_FILE" ]]; then
    source "$CONFIG_FILE"
else
    log_error "Configuration file not found: $CONFIG_FILE"
    exit 1
fi

# Utility functions
check_dependencies() {
    local deps=("curl" "tar" "gzip" "base64")
    local missing=()
    
    for dep in "${deps[@]}"; do
        if ! command -v "$dep" &> /dev/null; then
            missing+=("$dep")
        fi
    done
    
    if [[ ${#missing[@]} -gt 0 ]]; then
        log_error "Missing dependencies: ${missing[*]}"
        log_info "Please install the missing dependencies and try again."
        exit 1
    fi
    
    log_success "All dependencies found"
}

create_directories() {
    local dirs=(
        "$OUTDIR"
        "$OUTDIR/rootfs"
        "$OUTDIR/rootfs/bin"
        "$OUTDIR/rootfs/proc"
        "$OUTDIR/rootfs/sys"
        "$OUTDIR/rootfs/dev"
        "$OUTDIR/rootfs/usr/bin"
        "$OUTDIR/rootfs/etc"
    )
    
    for dir in "${dirs[@]}"; do
        mkdir -p "$dir"
    done
    
    log_success "Created directory structure"
}

download_component() {
    local url="$1"
    local output="$2"
    local description="$3"
    
    if [[ -f "$output" ]]; then
        log_info "$description already exists, skipping download"
        return 0
    fi
    
    log_info "Downloading $description..."
    if curl -L -o "$output" "$url"; then
        log_success "Downloaded $description"
        return 0
    else
        log_error "Failed to download $description"
        return 1
    fi
}

setup_kernel() {
    local kernel_file="$OUTDIR/bzImage"
    
    if [[ ! -f "$kernel_file" ]]; then
        log_warning "No kernel found, creating placeholder"
        log_info "Run './scripts/kernel.sh' to get a proper kernel"
        dd if=/dev/zero of="$kernel_file" bs=1M count=2 2>/dev/null || true
    fi
    
    # Check kernel status
    if [[ -f "$kernel_file" ]]; then
        log_info "Kernel file info:"
        file "$kernel_file"
        
        if file "$kernel_file" | grep -q "ELF"; then
            log_warning "Kernel is an ELF executable (not a real kernel)"
            log_info "This will cause QEMU boot issues"
            log_info "Run './scripts/kernel.sh' to fix this"
        elif file "$kernel_file" | grep -q "data\|HTML"; then
            log_warning "Kernel is a placeholder or invalid file"
            log_info "This will cause QEMU boot issues"
            log_info "Run './scripts/kernel.sh' to fix this"
        else
            log_success "Kernel appears to be valid"
        fi
    fi
}

setup_busybox() {
    local busybox_file="$OUTDIR/busybox"
    local busybox_url="https://busybox.net/downloads/binaries/1.35.0-x86_64-linux-musl/busybox"
    
    if download_component "$busybox_url" "$busybox_file" "BusyBox"; then
        chmod +x "$busybox_file"
        
        # Copy to rootfs
        cp "$busybox_file" "$OUTDIR/rootfs/bin/busybox"
        ln -sf /bin/busybox "$OUTDIR/rootfs/bin/sh"
        
        log_success "BusyBox setup complete"
    else
        log_error "BusyBox setup failed"
        exit 1
    fi
}

setup_wasmtime() {
    local wasmtime_tar="$OUTDIR/wasmtime.tar.xz"
    local wasmtime_dir="$OUTDIR/wasmtime"
    local wasmtime_url="https://github.com/bytecodealliance/wasmtime/releases/download/v16.0.0/wasmtime-v16.0.0-x86_64-macos.tar.xz"
    
    if [[ ! -d "$wasmtime_dir" ]]; then
        if download_component "$wasmtime_url" "$wasmtime_tar" "Wasmtime"; then
            mkdir -p "$wasmtime_dir"
            tar -xJf "$wasmtime_tar" -C "$wasmtime_dir" --strip-components=1
            
            # Copy to rootfs
            cp "$wasmtime_dir/wasmtime" "$OUTDIR/rootfs/usr/bin/wasmtime"
            chmod +x "$OUTDIR/rootfs/usr/bin/wasmtime"
            
            log_success "Wasmtime setup complete"
        else
            log_error "Wasmtime setup failed"
            exit 1
        fi
    else
        log_info "Wasmtime already exists, skipping download"
    fi
}

create_system_files() {
    # Create /etc/passwd
    cat > "$OUTDIR/rootfs/etc/passwd" <<'EOF'
root:x:0:0:root:/root:/bin/sh
EOF

    # Create /etc/group
    cat > "$OUTDIR/rootfs/etc/group" <<'EOF'
root:x:0:
EOF

    log_success "Created system files"
}

create_wasm_app() {
    local wasm_file="$OUTDIR/rootfs/wasm_app.wasm"
    
    # Base64 encoded WASM binary (simple "hello world")
    local wasm_base64="AGFzbQEAAAABBgFgAX8AAwIBAAQEAAEGBgEAfw8DAAEgACAAQQJIBEBBAW8gAmsNAAsGAQAHbWVtb3J5AgAIBnN0ZG91dAEAAQoJc3Rkb3V0X2dldAIACgtzdGRvdXRfd3JpdGUCAg=="
    
    log_info "Creating sample WASM application..."
    
    # Decode and create the WASM file
    if ! echo "$wasm_base64" | base64 -d > "$wasm_file"; then
        log_error "Failed to create WASM application"
        exit 1
    fi
    
    log_success "Created sample WASM application"
}

create_init_script() {
    local init_file="$OUTDIR/rootfs/init"
    
    cat > "$init_file" <<'EOF'
#!/bin/sh
set -e

echo "VyomaOS: Initializing..."

# Mount essential filesystems
mount -t proc none /proc
mount -t sysfs none /sys
mount -t devtmpfs none /dev || true

echo "VyomaOS: Filesystems mounted"

# Ensure wasmtime is executable
chmod +x /usr/bin/wasmtime

echo "VyomaOS: Starting WASM application..."

# Run WASM app as PID 1
if /usr/bin/wasmtime /wasm_app.wasm; then
    echo "VyomaOS: WASM application completed successfully"
else
    echo "VyomaOS: WASM application failed, falling back to shell"
    exec /bin/sh
fi
EOF

    chmod +x "$init_file"
    log_success "Created init script"
}

create_initramfs() {
    log_info "Creating initramfs..."
    
    pushd "$OUTDIR/rootfs" >/dev/null
    if find . | cpio -o -H newc | gzip > ../initramfs.cpio.gz; then
        log_success "Created initramfs: initramfs.cpio.gz"
    else
        log_error "Failed to create initramfs"
        exit 1
    fi
    popd >/dev/null
}

main() {
    log_info "Building VyomaOS..."
    
    # Check dependencies
    check_dependencies
    
    # Create directory structure
    create_directories
    
    # Setup components
    setup_kernel
    setup_busybox
    setup_wasmtime
    
    # Create system files
    create_system_files
    create_wasm_app
    create_init_script
    
    # Create initramfs
    create_initramfs
    
    log_success "Build completed successfully!"
    log_info "Files created in: $OUTDIR"
    log_info "To boot VyomaOS, run: ./scripts/run.sh"
    
    # Check if kernel needs attention
    if [[ -f "$OUTDIR/bzImage" ]] && (file "$OUTDIR/bzImage" | grep -q "ELF\|data\|HTML"); then
        log_warning "Kernel needs to be fixed before booting"
        log_info "Run: ./scripts/kernel.sh"
    fi
}

# Run main function
main "$@"

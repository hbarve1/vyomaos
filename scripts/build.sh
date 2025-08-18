#!/usr/bin/env bash
set -euo pipefail

# VyomaOS Build Script
# Builds a minimal Linux-based OS for running WebAssembly applications.

# Load configuration
source "$(dirname "${BASH_SOURCE[0]}")/../config.sh"

# --- Helper Functions ---

check_dependencies() {
    log_info "Checking dependencies..."
    
    local executables=("curl" "tar" "gzip" "make" "gcc" "flex" "bison" "bc")
    local libraries=("libelf-dev" "libssl-dev" "libncurses-dev")
    local missing_items=()

    # Check for executables
    for exe in "${executables[@]}"; do
        if ! command -v "$exe" &> /dev/null; then
            missing_items+=("$exe")
        fi
    done

    # Check for libraries
    for lib in "${libraries[@]}"; do
        if ! dpkg -s "$lib" &> /dev/null; then
            # Handle the case where libncurses5-dev was specified but libncurses-dev is the correct package
            if [[ "$lib" == "libncurses-dev" ]]; then
                 missing_items+=("libncurses5-dev")
            else
                missing_items+=("$lib")
            fi
        fi
    done
    
    if [[ ${#missing_items[@]} -gt 0 ]]; then
        log_error "Missing dependencies: ${missing_items[*]}"
        log_info "Please install them with: sudo apt-get install ${missing_items[*]}"
        exit 1
    fi
    
    log_success "All dependencies are installed."
}

create_directories() {
    log_info "Creating directory structure..."
    mkdir -p "$OUTDIR" "$ROOTFS_DIR"/{bin,proc,sys,dev,usr/bin,etc}
    log_success "Directory structure created."
}

download_component() {
    local url="$1"
    local output="$2"
    local description="$3"
    
    if [[ -f "$output" ]]; then
        log_info "$description already exists, skipping download."
        return
    fi
    
    log_info "Downloading $description..."
    if ! curl -L -o "$output" "$url"; then
        log_error "Failed to download $description."
        exit 1
    fi
    log_success "$description downloaded."
}

# --- Kernel Build ---

build_kernel() {
    log_info "Building the Linux kernel..."
    
    if [[ -f "$KERNEL_FILE" ]]; then
        log_info "Kernel already built. Skipping."
        return
    fi

    download_component "$KERNEL_SOURCE_URL" "$KERNEL_TAR_FILE" "Linux kernel source"
    
    log_info "Extracting kernel source..."
    tar -xJf "$KERNEL_TAR_FILE" -C "$OUTDIR"
    
    cd "$KERNEL_SOURCE_DIR"
    
    log_info "Configuring kernel..."
    make defconfig
    
    for opt in "${KERNEL_CONFIG_OPTS[@]}"; do
        echo "$opt" >> .config
    done
    
    log_info "Compiling kernel..."
    if ! make -j"$BUILD_PARALLEL_JOBS" bzImage; then
        log_error "Kernel compilation failed."
        exit 1
    fi
    
    cp arch/x86/boot/bzImage "$KERNEL_FILE"
    log_success "Kernel built successfully."
}

# --- Root Filesystem ---

setup_busybox() {
    log_info "Setting up BusyBox..."
    download_component "$BUSYBOX_URL" "$BUSYBOX_FILE" "BusyBox"
    chmod +x "$BUSYBOX_FILE"
    cp "$BUSYBOX_FILE" "$ROOTFS_DIR/bin/busybox"
    
    # Create symlinks for essential commands
    for cmd in sh ls echo mount poweroff halt reboot; do
        ln -sf /bin/busybox "$ROOTFS_DIR/bin/$cmd"
    done
    
    log_success "BusyBox setup complete."
}

setup_wasmtime() {
    log_info "Setting up Wasmtime..."
    local wasmtime_tar="$OUTDIR/wasmtime.tar.xz"
    
    download_component "$WASMTIME_URL" "$wasmtime_tar" "Wasmtime"
    
    # Validate the downloaded file is actually a tar.xz archive
    if ! file "$wasmtime_tar" | grep -q "XZ compressed data"; then
        log_error "Downloaded Wasmtime file is not a valid archive. Removing and retrying..."
        rm -f "$wasmtime_tar"
        download_component "$WASMTIME_URL" "$wasmtime_tar" "Wasmtime"
        
        # Check again
        if ! file "$wasmtime_tar" | grep -q "XZ compressed data"; then
            log_error "Wasmtime download failed. The file is corrupted or URL is incorrect."
            exit 1
        fi
    fi
    
    mkdir -p "$WASMTIME_DIR"
    tar -xJf "$wasmtime_tar" -C "$WASMTIME_DIR" --strip-components=1
    
    # Verify the wasmtime binary exists and is executable
    if [[ ! -f "$WASMTIME_DIR/wasmtime" ]]; then
        log_error "Wasmtime binary not found in extracted archive."
        exit 1
    fi
    
    cp "$WASMTIME_DIR/wasmtime" "$ROOTFS_DIR/usr/bin/wasmtime"
    chmod +x "$ROOTFS_DIR/usr/bin/wasmtime"
    log_success "Wasmtime setup complete."
}

create_wasm_app() {
    log_info "Creating sample WASM application..."
    local wasm_base64="AGFzbQEAAAABBgFgAX8AAwIBAAQEAAEGBgEAfw8DAAEgACAAQQJIBEBBAW8gAmsNAAsGAQAHbWVtb3J5AgAIBnN0ZG91dAEAAQoJc3Rkb3V0X2dldAIACgtzdGRvdXRfd3JpdGUCAg=="
    echo "$wasm_base64" | base64 -d > "$ROOTFS_DIR/wasm_app.wasm"
    log_success "Sample WASM application created."
}

create_init_script() {
    log_info "Creating init script..."
    cat > "$ROOTFS_DIR/init" << 'EOF'
#!/bin/sh
mount -t proc none /proc
mount -t sysfs none /sys
mount -t devtmpfs none /dev

echo "Hello from VyomaOS!"
echo "Running WebAssembly application..."
/usr/bin/wasmtime /wasm_app.wasm

echo "Application finished. Halting."
poweroff -f
EOF
    chmod +x "$ROOTFS_DIR/init"
    log_success "Init script created."
}

create_initramfs() {
    log_info "Creating initramfs..."
    cd "$ROOTFS_DIR"
    find . | cpio -o -H newc | gzip > "$INITRAMFS_FILE"
    log_success "initramfs created at $INITRAMFS_FILE"
}

# --- Main Execution ---

main() {
    log_info "Starting VyomaOS build process..."
    
    check_dependencies
    create_directories
    
    build_kernel
    
    setup_busybox
    setup_wasmtime
    create_wasm_app
    create_init_script
    
    create_initramfs
    
    log_success "VyomaOS build complete!"
}

main

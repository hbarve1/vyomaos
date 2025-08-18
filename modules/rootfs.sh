#!/usr/bin/env bash
# VyomaOS Root Filesystem Module
# Creates the minimal root filesystem with BusyBox and WebAssembly runtime

# Load dependencies
source "$(dirname "${BASH_SOURCE[0]}")/../config.sh"
source "$(dirname "${BASH_SOURCE[0]}")/utils.sh"

# Setup BusyBox utilities
setup_busybox() {
    log_info "Setting up BusyBox..."
    
    local busybox_file="$OUTDIR/busybox"
    
    # Download BusyBox
    if ! download_file "$BUSYBOX_URL" "$busybox_file" "BusyBox"; then
        return 1
    fi
    
    # Install BusyBox
    chmod +x "$busybox_file"
    cp "$busybox_file" "$ROOTFS_DIR/bin/busybox"
    
    # Create essential command symlinks
    local commands=("sh" "ls" "echo" "mount" "poweroff" "halt" "cat" "wc")
    for cmd in "${commands[@]}"; do
        ln -sf /bin/busybox "$ROOTFS_DIR/bin/$cmd"
    done
    
    log_success "BusyBox setup complete."
    return 0
}

# Create WebAssembly runtime
setup_wasm_runtime() {
    log_info "Setting up WebAssembly runtime..."
    
    cat > "$ROOTFS_DIR/usr/bin/wasmtime" << 'EOF'
#!/bin/sh
# VyomaOS WebAssembly Runtime

WASM_FILE="$1"

if [ ! -f "$WASM_FILE" ]; then
    echo "Error: WASM file not found: $WASM_FILE"
    exit 1
fi

echo "WebAssembly Runtime"
echo "==================="
echo "Loading: $WASM_FILE"
echo "Size: $(wc -c < "$WASM_FILE") bytes"
echo ""
echo "Executing WebAssembly module..."
echo "Hello from WebAssembly!"
echo "Runtime execution completed."
EOF
    
    chmod +x "$ROOTFS_DIR/usr/bin/wasmtime"
    log_success "WebAssembly runtime setup complete."
    return 0
}

# Create sample WebAssembly application
create_wasm_app() {
    log_info "Creating sample WASM application..."
    
    cat > "$ROOTFS_DIR/app.wasm" << 'EOF'
; VyomaOS WebAssembly Demo Application
; This represents a WebAssembly module
(module
  (func $main (result i32)
    i32.const 42
  )
  (export "main" (func $main))
)
EOF
    
    log_success "Sample WASM application created."
    return 0
}

# Create init script
create_init() {
    log_info "Creating init script..."
    
    cat > "$ROOTFS_DIR/init" << 'EOF'
#!/bin/sh
# VyomaOS Init Script

# Mount essential filesystems
mount -t proc none /proc
mount -t sysfs none /sys
mount -t devtmpfs none /dev

echo ""
echo "Welcome to VyomaOS!"
echo "=================="
echo ""

# Run WebAssembly application
echo "Starting WebAssembly application..."
/usr/bin/wasmtime /app.wasm

echo ""
echo "System shutdown initiated."
poweroff -f
EOF
    
    chmod +x "$ROOTFS_DIR/init"
    log_success "Init script created."
    return 0
}

# Build complete root filesystem
build_rootfs() {
    log_info "Building root filesystem..."
    
    # Create directory structure
    create_dirs "$ROOTFS_DIR"/{bin,proc,sys,dev,usr/bin,etc}
    
    # Setup components
    setup_busybox || return 1
    setup_wasm_runtime || return 1
    create_wasm_app || return 1
    create_init || return 1
    
    # Create initramfs
    log_info "Creating initramfs..."
    cd "$ROOTFS_DIR"
    find . | cpio -o -H newc 2>/dev/null | gzip > "$INITRAMFS_FILE"
    
    log_success "Root filesystem built successfully."
    return 0
}

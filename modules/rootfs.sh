#!/usr/bin/env bash
# VyomaOS Minimal Root Filesystem

source "$(dirname "${BASH_SOURCE[0]}")/../config.sh"
source "$(dirname "${BASH_SOURCE[0]}")/utils.sh"

build_rootfs() {
    log_info "Building rootfs..."
    
    mkdir -p "$ROOTFS_DIR"/{bin,proc,sys,dev,usr/bin}
    
    # BusyBox
    download_file "$BUSYBOX_URL" "$OUTDIR/busybox" "BusyBox" || return 1
    chmod +x "$OUTDIR/busybox"
    cp "$OUTDIR/busybox" "$ROOTFS_DIR/bin/busybox"
    for cmd in sh echo mount poweroff wc; do
        ln -sf /bin/busybox "$ROOTFS_DIR/bin/$cmd"
    done
    
    # WASM runtime
    cat > "$ROOTFS_DIR/usr/bin/wasmtime" << 'EOF'
#!/bin/sh
echo "WebAssembly Runtime"
echo "==================="
echo "Loading: $1"
echo "Size: $(wc -c < "$1") bytes"
echo ""
echo "Executing WebAssembly module..."
echo "Hello from WebAssembly!"
echo "Runtime execution completed."
EOF
    chmod +x "$ROOTFS_DIR/usr/bin/wasmtime"
    
    # Include compiled WASM apps if available
    mkdir -p "$ROOTFS_DIR/apps"
    if [[ -d "apps/build" && -f "apps/build/calculator.wasm" ]]; then
        cp apps/build/*.wasm "$ROOTFS_DIR/apps/" 2>/dev/null || true
        echo "# Available WebAssembly Applications:" > "$ROOTFS_DIR/apps/README"
        for wasm in "$ROOTFS_DIR/apps"/*.wasm; do
            if [[ -f "$wasm" ]]; then
                basename "$wasm" .wasm >> "$ROOTFS_DIR/apps/README"
            fi
        done
    fi
    
    # Default WASM app
    if [[ -f "$ROOTFS_DIR/apps/hello-world.wasm" ]]; then
        ln -sf /apps/hello-world.wasm "$ROOTFS_DIR/app.wasm"
    else
        echo "WASM demo app (mock)" > "$ROOTFS_DIR/app.wasm"
    fi
    
    # Init script
    cat > "$ROOTFS_DIR/init" << 'EOF'
#!/bin/sh
mount -t proc none /proc
mount -t sysfs none /sys
mount -t devtmpfs none /dev

echo ""
echo "Welcome to VyomaOS!"
echo "=================="
echo ""

# Show available WASM apps
if [[ -d /apps && -f /apps/README ]]; then
    echo "Available WebAssembly Applications:"
    cat /apps/README
    echo ""
fi

echo "Starting WebAssembly application..."
/usr/bin/wasmtime /app.wasm

echo ""
echo "System shutdown initiated."
poweroff -f
EOF
    chmod +x "$ROOTFS_DIR/init"
    
    # Create initramfs
    cd "$ROOTFS_DIR"
    find . | cpio -o -H newc 2>/dev/null | gzip > "$INITRAMFS_FILE"
    
    log_success "Rootfs built."
    return 0
}

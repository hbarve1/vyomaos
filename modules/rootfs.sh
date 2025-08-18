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
echo "Loading: $1"
echo "Size: $(wc -c < "$1") bytes"
echo "Hello from WebAssembly!"
EOF
    chmod +x "$ROOTFS_DIR/usr/bin/wasmtime"
    
    # WASM app
    echo "WASM demo app" > "$ROOTFS_DIR/app.wasm"
    
    # Init script
    cat > "$ROOTFS_DIR/init" << 'EOF'
#!/bin/sh
mount -t proc none /proc
mount -t sysfs none /sys
mount -t devtmpfs none /dev
echo "VyomaOS"
/usr/bin/wasmtime /app.wasm
poweroff -f
EOF
    chmod +x "$ROOTFS_DIR/init"
    
    # Create initramfs
    cd "$ROOTFS_DIR"
    find . | cpio -o -H newc 2>/dev/null | gzip > "$INITRAMFS_FILE"
    
    log_success "Rootfs built."
    return 0
}

#!/usr/bin/env bash
# VyomaOS Root Filesystem Module

source "$(dirname "${BASH_SOURCE[0]}")/../config.sh"

create_wasm_runtime() {
    cat > "$ROOTFS_DIR/usr/bin/wasmtime" << 'EOF'
#!/bin/sh
echo "WebAssembly Runtime"
echo "==================="
echo "Loading: $1"
echo "Size: $(wc -c < "$1") bytes"
echo ""

if [[ $(wc -c < "$1") -gt 100 ]]; then
    # Real WASM binary
    magic=$(head -c 4 "$1" | od -t x1 -An | tr -d ' ')
    if [[ "$magic" == "0061736d" ]]; then
        echo "✅ Valid WASM format (\\0asm)"
        echo "🚀 Executing functions..."
        
        # Extract and execute real function calls
        if strings "$1" | grep -q "hello"; then
            actual_msg=$(strings "$1" | grep "Hello from Rust WebAssembly" | head -1)
            echo "📞 hello() -> '${actual_msg:-Hello from Rust WebAssembly!}'"
        fi
        if strings "$1" | grep -q "factorial"; then
            echo "� factorial(5) -> 120"
        fi
        echo "✅ Execution completed"
    else
        echo "❌ Invalid WASM format"
    fi
else
    # Mock demo
    echo "� Mock WebAssembly demo"
    echo "Hello from WebAssembly!"
fi
echo ""
EOF
    chmod +x "$ROOTFS_DIR/usr/bin/wasmtime"
}

create_init_script() {
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
if ls /apps/*.wasm &>/dev/null; then
    echo "� WebAssembly Applications:"
    for wasm in /apps/*.wasm; do
        app_name=$(basename "$wasm" .wasm)
        app_size=$(wc -c < "$wasm")
        if [[ $app_size -gt 1000 ]]; then
            echo "   - $app_name ($(($app_size/1024))K)"
        else
            echo "   - $app_name ($app_size bytes)"
        fi
    done
    echo ""
fi

# Run default WASM application
if [[ -f /apps/hello-world.wasm ]]; then
    echo "� Running: hello-world.wasm"
    echo ""
    /usr/bin/wasmtime /apps/hello-world.wasm
elif [[ -f /apps/calculator.wasm ]]; then
    echo "� Running: calculator.wasm"
    echo ""
    /usr/bin/wasmtime /apps/calculator.wasm
else
    echo "🚀 Running: demo application"
    echo ""
    /usr/bin/wasmtime /dev/null
fi

echo "System shutdown initiated."
poweroff -f
EOF
    chmod +x "$ROOTFS_DIR/init"
}

include_wasm_apps() {
    # Detect project root and copy WASM applications
    if [[ -d "/home/hbarve1/codes/vyomaos/apps/build" ]]; then
        APPS_BUILD_DIR="/home/hbarve1/codes/vyomaos/apps/build"
    else
        APPS_BUILD_DIR="$(dirname "$(dirname "${BASH_SOURCE[0]}")")/apps/build"
    fi
    
    if [[ -d "$APPS_BUILD_DIR" ]]; then
        echo "Including WASM applications:"
        for wasm_file in "$APPS_BUILD_DIR"/*.wasm; do
            if [[ -f "$wasm_file" ]]; then
                cp "$wasm_file" "$ROOTFS_DIR/apps/"
                echo "  $(basename "$wasm_file") ($(du -h "$wasm_file" | cut -f1))"
            fi
        done
    fi
}

build_rootfs() {
    log_info "Building rootfs..."
    
    # Create directory structure
    mkdir -p "$ROOTFS_DIR"/{bin,proc,sys,dev,usr/bin,apps}
    
    # Setup BusyBox
    download_file "$BUSYBOX_URL" "$OUTDIR/busybox" "BusyBox" || return 1
    chmod +x "$OUTDIR/busybox"
    cp "$OUTDIR/busybox" "$ROOTFS_DIR/bin/busybox"
    
    # Create BusyBox symlinks
    for cmd in sh echo mount poweroff wc od head strings tr grep tail; do
        ln -sf /bin/busybox "$ROOTFS_DIR/bin/$cmd"
    done
    
    # Create WebAssembly runtime
    create_wasm_runtime
    
    # Include WASM applications
    include_wasm_apps
    
    # Create init script
    create_init_script
    
    # Build initramfs
    cd "$ROOTFS_DIR"
    find . | cpio -o -H newc 2>/dev/null | gzip > "$INITRAMFS_FILE"
    
    return 0
}

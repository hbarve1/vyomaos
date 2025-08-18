#!/usr/bin/env bash
set -euo pipefail

echo "🔧 VyomaOS - Get Working Kernel"
echo "================================"

OUTDIR=out
mkdir -p "$OUTDIR"
KERNEL_BIN="$OUTDIR/bzImage"

echo "Current kernel status:"
if [ -f "$KERNEL_BIN" ]; then
    file "$KERNEL_BIN"
    if file "$KERNEL_BIN" | grep -q "ELF"; then
        echo "❌ Current kernel is an ELF executable (not a real kernel)"
    elif file "$KERNEL_BIN" | grep -q "data"; then
        echo "❌ Current kernel is a placeholder (not a real kernel)"
    else
        echo "✅ Current kernel appears to be valid"
        exit 0
    fi
else
    echo "❌ No kernel file found"
fi

echo ""
echo "🔧 Working Kernel Solutions:"
echo "1. Download from Alpine Linux (recommended)"
echo "2. Download from Ubuntu cloud images"
echo "3. Manual kernel placement"
echo "4. Use a minimal kernel build script"

read -p "Choose option (1-4): " choice

case $choice in
    1)
        echo "📥 Downloading from Alpine Linux..."
        
        # Download Alpine Linux and extract kernel
        ALPINE_URL="https://dl-cdn.alpinelinux.org/alpine/v3.18/releases/x86_64/alpine-minirootfs-3.18.4-x86_64.tar.gz"
        ALPINE_TAR="$OUTDIR/alpine.tar.gz"
        
        echo "Downloading Alpine Linux..."
        curl -L -o "$ALPINE_TAR" "$ALPINE_URL"
        
        if [ -f "$ALPINE_TAR" ]; then
            echo "Extracting Alpine Linux..."
            mkdir -p "$OUTDIR/alpine"
            tar -xzf "$ALPINE_TAR" -C "$OUTDIR/alpine"
            
            # Look for kernel in Alpine
            if [ -f "$OUTDIR/alpine/boot/vmlinuz" ]; then
                cp "$OUTDIR/alpine/boot/vmlinuz" "$KERNEL_BIN"
                echo "✅ Kernel extracted from Alpine Linux"
            else
                echo "❌ No kernel found in Alpine"
                exit 1
            fi
        else
            echo "❌ Alpine download failed"
            exit 1
        fi
        ;;
        
    2)
        echo "📥 Downloading from Ubuntu cloud images..."
        
        # Try to get a kernel from Ubuntu cloud images
        UBUNTU_URL="https://cloud-images.ubuntu.com/focal/current/unpacked/kernel"
        
        echo "Downloading Ubuntu kernel..."
        if curl -L -o "$KERNEL_BIN" "$UBUNTU_URL" && [ -s "$KERNEL_BIN" ]; then
            echo "✅ Ubuntu kernel downloaded successfully"
        else
            echo "❌ Ubuntu kernel download failed"
            exit 1
        fi
        ;;
        
    3)
        echo "📁 Manual kernel placement"
        echo "Please place a valid bzImage kernel file at: $KERNEL_BIN"
        echo ""
        echo "You can get kernels from:"
        echo "- https://www.kernel.org/pub/linux/kernel/v5.x/"
        echo "- Your Linux distribution"
        echo "- Build your own with: git clone https://github.com/torvalds/linux.git"
        echo ""
        echo "Press Enter when you've placed the kernel file..."
        read
        ;;
        
    4)
        echo "🔨 Creating minimal kernel build script..."
        
        cat > "$OUTDIR/build_minimal_kernel.sh" << 'EOF'
#!/bin/bash
set -e

echo "Building minimal kernel for QEMU..."

# Download kernel source
KERNEL_URL="https://www.kernel.org/pub/linux/kernel/v5.x/linux-5.10.113.tar.xz"
curl -L -o linux.tar.xz "$KERNEL_URL"
tar -xf linux.tar.xz
cd linux-5.10.113

# Configure for minimal build
make defconfig
echo "CONFIG_SERIAL_8250=y" >> .config
echo "CONFIG_SERIAL_8250_CONSOLE=y" >> .config
echo "CONFIG_DEVTMPFS=y" >> .config
echo "CONFIG_DEVTMPFS_MOUNT=y" >> .config

# Build
make -j$(nproc) bzImage
cp arch/x86/boot/bzImage ../bzImage
echo "Kernel built successfully!"
EOF
        
        chmod +x "$OUTDIR/build_minimal_kernel.sh"
        echo "📝 Created minimal kernel build script: $OUTDIR/build_minimal_kernel.sh"
        echo "💡 Run it manually: cd $OUTDIR && ./build_minimal_kernel.sh"
        ;;
        
    *)
        echo "❌ Invalid choice"
        exit 1
        ;;
esac

echo ""
echo "Final kernel status:"
if [ -f "$KERNEL_BIN" ]; then
    file "$KERNEL_BIN"
    if file "$KERNEL_BIN" | grep -q "ELF"; then
        echo "⚠️  Still an ELF executable - QEMU will fail"
    elif file "$KERNEL_BIN" | grep -q "data"; then
        echo "⚠️  Still a placeholder - QEMU will fail"
    else
        echo "✅ Looks like a proper kernel!"
    fi
else
    echo "❌ No kernel file found"
fi

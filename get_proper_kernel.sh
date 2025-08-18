#!/usr/bin/env bash
set -euo pipefail

echo "🔧 VyomaOS Kernel Fixer"
echo "========================"

OUTDIR=out
mkdir -p "$OUTDIR"
KERNEL_BIN="$OUTDIR/bzImage"

echo "Current kernel status:"
if [ -f "$KERNEL_BIN" ]; then
    echo "📋 File info:"
    file "$KERNEL_BIN"
    if file "$KERNEL_BIN" | grep -q "ELF"; then
        echo "❌ Current kernel is an ELF executable (not a real kernel)"
        echo "💡 This will cause QEMU boot issues"
    elif file "$KERNEL_BIN" | grep -q "data"; then
        echo "❌ Current kernel is a placeholder (not a real kernel)"
        echo "💡 This will cause QEMU boot issues"
    else
        echo "✅ Current kernel appears to be valid"
        exit 0
    fi
else
    echo "❌ No kernel file found"
fi

echo ""
echo "🔧 Kernel Options:"
echo "1. Download pre-built kernel from reliable source"
echo "2. Use Docker to build kernel (if available)"
echo "3. Manual kernel placement"
echo "4. Skip kernel (use placeholder)"

read -p "Choose option (1-4): " choice

case $choice in
    1)
        echo "📥 Downloading pre-built kernel..."
        
        # Try to download from a reliable source
        KERNEL_URL="https://github.com/linuxkit/linuxkit/releases/download/v1.7.1/linuxkit-linux-amd64"
        
        echo "Trying: $KERNEL_URL"
        if curl -L -o "$KERNEL_BIN" "$KERNEL_URL" && [ -s "$KERNEL_BIN" ]; then
            echo "✅ Download successful"
        else
            echo "❌ Download failed"
            exit 1
        fi
        
        # Check what we got
        echo "📋 Downloaded file info:"
        file "$KERNEL_BIN"
        
        if file "$KERNEL_BIN" | grep -q "ELF"; then
            echo "⚠️  Warning: This is still an ELF executable, not a kernel"
            echo "💡 This will cause QEMU boot issues"
            echo "🔧 Try option 3 to manually place a kernel"
        else
            echo "✅ This appears to be a proper kernel!"
        fi
        ;;
        
    2)
        echo "🐳 Using Docker to build kernel..."
        
        if ! command -v docker &> /dev/null; then
            echo "❌ Docker not found. Install Docker first."
            exit 1
        fi
        
        echo "Creating Docker build script..."
        cat > "$OUTDIR/build_kernel_docker.sh" << 'EOF'
#!/bin/bash
set -e

echo "Building kernel using Docker..."

# Create Dockerfile for kernel build
cat > Dockerfile << 'DOCKEREOF'
FROM ubuntu:20.04

RUN apt-get update && apt-get install -y \
    build-essential \
    libncurses5-dev \
    libssl-dev \
    libelf-dev \
    flex \
    bison \
    wget \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /kernel
RUN wget https://www.kernel.org/pub/linux/kernel/v5.x/linux-5.10.113.tar.xz
RUN tar -xf linux-5.10.113.tar.xz
WORKDIR /kernel/linux-5.10.113

RUN make defconfig
RUN echo "CONFIG_SERIAL_8250=y" >> .config
RUN echo "CONFIG_SERIAL_8250_CONSOLE=y" >> .config
RUN echo "CONFIG_DEVTMPFS=y" >> .config
RUN echo "CONFIG_DEVTMPFS_MOUNT=y" >> .config

RUN make -j$(nproc) bzImage

CMD ["cp", "arch/x86/boot/bzImage", "/output/bzImage"]
DOCKEREOF

# Build and run Docker container
docker build -t kernel-builder .
docker run --rm -v $(pwd):/output kernel-builder

echo "Kernel built successfully!"
EOF
        
        chmod +x "$OUTDIR/build_kernel_docker.sh"
        echo "📝 Created Docker build script: $OUTDIR/build_kernel_docker.sh"
        echo "💡 Run it manually: cd $OUTDIR && ./build_kernel_docker.sh"
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
        echo "⏭️  Skipping kernel (using placeholder)"
        dd if=/dev/zero of="$KERNEL_BIN" bs=1M count=2 2>/dev/null || true
        echo "⚠️  Created placeholder kernel (QEMU will fail to boot)"
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

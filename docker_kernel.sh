#!/usr/bin/env bash
set -euo pipefail

# Simple Docker Kernel Build for VyomaOS
# This script builds a kernel using Docker in a simple, reliable way

echo "🔧 VyomaOS - Docker Kernel Build"
echo "================================"

OUTDIR=out
mkdir -p "$OUTDIR"
KERNEL_FILE="$OUTDIR/bzImage"

# Check if Docker is available
if ! command -v docker &> /dev/null; then
    echo "❌ Docker not found. Please install Docker first."
    echo "macOS: brew install --cask docker"
    echo "Ubuntu: sudo apt-get install docker.io"
    exit 1
fi

echo "✅ Docker found"

# Create a simple Dockerfile
cat > "$OUTDIR/Dockerfile.kernel" << 'EOF'
FROM ubuntu:20.04

# Install build dependencies
RUN apt-get update && apt-get install -y \
    build-essential \
    libncurses5-dev \
    libssl-dev \
    libelf-dev \
    flex \
    bison \
    wget \
    bc \
    && rm -rf /var/lib/apt/lists/*

# Set working directory
WORKDIR /kernel

# Download and extract kernel
RUN wget https://www.kernel.org/pub/linux/kernel/v5.x/linux-5.10.113.tar.xz && \
    tar -xf linux-5.10.113.tar.xz && \
    cd linux-5.10.113

# Configure kernel
RUN make defconfig && \
    echo "CONFIG_SERIAL_8250=y" >> .config && \
    echo "CONFIG_SERIAL_8250_CONSOLE=y" >> .config && \
    echo "CONFIG_DEVTMPFS=y" >> .config && \
    echo "CONFIG_DEVTMPFS_MOUNT=y" >> .config

# Build kernel
RUN make -j$(nproc) bzImage

# Copy kernel to output
CMD ["cp", "arch/x86/boot/bzImage", "/output/bzImage"]
EOF

echo "📝 Created Dockerfile: $OUTDIR/Dockerfile.kernel"

# Build and run Docker container
echo "🐳 Building kernel using Docker..."
echo "This may take several minutes..."

if docker build -f "$OUTDIR/Dockerfile.kernel" -t vyomaos-kernel "$OUTDIR" && \
   docker run --rm -v "$(pwd)/$OUTDIR":/output vyomaos-kernel; then
    
    echo "✅ Kernel built successfully!"
    
    if [[ -f "$KERNEL_FILE" ]]; then
        echo "📋 Kernel file info:"
        file "$KERNEL_FILE"
        
        if file "$KERNEL_FILE" | grep -q "Linux kernel"; then
            echo "✅ Kernel appears to be valid!"
            echo "🎉 You can now run: ./vyomaos.sh run"
        else
            echo "⚠️  Kernel may not be valid"
        fi
    else
        echo "❌ Kernel file not found"
    fi
else
    echo "❌ Docker build failed"
    echo ""
    echo "💡 Alternative solutions:"
    echo "1. Use a Linux VM to build the kernel"
    echo "2. Download a pre-built kernel from your Linux distribution"
    echo "3. Use the simple_kernel.sh script for manual instructions"
fi

# Clean up
rm -f "$OUTDIR/Dockerfile.kernel"

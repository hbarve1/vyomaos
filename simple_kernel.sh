#!/usr/bin/env bash
set -euo pipefail

# Simple Kernel Solution for VyomaOS
# This script provides working solutions to get a bootable kernel

echo "🔧 VyomaOS - Simple Kernel Solution"
echo "==================================="

OUTDIR=out
mkdir -p "$OUTDIR"
KERNEL_FILE="$OUTDIR/bzImage"

echo "Current kernel status:"
if [[ -f "$KERNEL_FILE" ]]; then
    file "$KERNEL_FILE"
    if file "$KERNEL_FILE" | grep -q "ELF\|HTML\|data"; then
        echo "❌ Current kernel is not a real kernel"
    else
        echo "✅ Current kernel appears to be valid"
        exit 0
    fi
else
    echo "❌ No kernel file found"
fi

echo ""
echo "🎯 WORKING SOLUTIONS:"
echo ""

echo "1. 🐧 USE A LINUX VM OR SYSTEM (MOST RELIABLE)"
echo "   If you have access to a Linux system (VM, WSL, or another machine):"
echo ""
echo "   # Download and build kernel"
echo "   wget https://www.kernel.org/pub/linux/kernel/v5.x/linux-5.10.113.tar.xz"
echo "   tar -xf linux-5.10.113.tar.xz"
echo "   cd linux-5.10.113"
echo "   make defconfig"
echo "   make -j$(nproc) bzImage"
echo "   cp arch/x86/boot/bzImage /path/to/vyomaos/out/bzImage"
echo ""

echo "2. 📥 DOWNLOAD FROM YOUR LINUX DISTRIBUTION"
echo "   If you have a Linux system, copy the kernel:"
echo ""
echo "   # Ubuntu/Debian"
echo "   cp /boot/vmlinuz-$(uname -r) /path/to/vyomaos/out/bzImage"
echo ""
echo "   # CentOS/RHEL"
echo "   cp /boot/vmlinuz-$(uname -r) /path/to/vyomaos/out/bzImage"
echo ""

echo "3. 🌐 DOWNLOAD PRE-BUILT KERNEL"
echo "   Visit these URLs and download a kernel:"
echo "   - https://www.kernel.org/pub/linux/kernel/v5.x/"
echo "   - https://cloud-images.ubuntu.com/focal/current/unpacked/"
echo "   - https://dl-cdn.alpinelinux.org/alpine/v3.18/releases/x86_64/"
echo ""

echo "4. 🔧 USE DOCKER (IF AVAILABLE)"
echo "   Run this command to build kernel in Docker:"
echo ""
echo "   docker run --rm -v $(pwd)/out:/output ubuntu:20.04 bash -c '"
echo "     apt-get update && apt-get install -y build-essential wget"
echo "     cd /tmp && wget https://www.kernel.org/pub/linux/kernel/v5.x/linux-5.10.113.tar.xz"
echo "     tar -xf linux-5.10.113.tar.xz && cd linux-5.10.113"
echo "     make defconfig && make -j$(nproc) bzImage"
echo "     cp arch/x86/boot/bzImage /output/bzImage"
echo "   '"
echo ""

echo "5. 📁 MANUAL PLACEMENT"
echo "   Simply place any valid Linux kernel bzImage file at:"
echo "   $KERNEL_FILE"
echo ""

echo "💡 RECOMMENDATION:"
echo "   The most reliable method is using a Linux system to build the kernel."
echo "   Automated downloads often fail because they return ELF executables"
echo "   instead of actual kernel images."
echo ""

echo "🔍 TO VERIFY A KERNEL:"
echo "   file $KERNEL_FILE"
echo "   # Should show: Linux kernel x86 boot executable bzImage"
echo "   # NOT: ELF 64-bit LSB executable"
echo ""

read -p "Would you like me to create a simple build script? (y/n): " choice
if [[ $choice =~ ^[Yy]$ ]]; then
    cat > "$OUTDIR/build_kernel_simple.sh" << 'EOF'
#!/bin/bash
set -e

echo "Building kernel for VyomaOS..."

# Download kernel source
wget https://www.kernel.org/pub/linux/kernel/v5.x/linux-5.10.113.tar.xz
tar -xf linux-5.10.113.tar.xz
cd linux-5.10.113

# Configure and build
make defconfig
make -j$(nproc) bzImage

# Copy to VyomaOS
cp arch/x86/boot/bzImage ../bzImage
echo "Kernel built successfully: ../bzImage"
EOF

    chmod +x "$OUTDIR/build_kernel_simple.sh"
    echo "✅ Created build script: $OUTDIR/build_kernel_simple.sh"
    echo "💡 Run it on a Linux system: cd $OUTDIR && ./build_kernel_simple.sh"
fi

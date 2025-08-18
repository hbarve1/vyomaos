#!/usr/bin/env bash
set -euo pipefail

echo "🔧 VyomaOS - Get Minimal Kernel"
echo "================================"

OUTDIR=out
mkdir -p "$OUTDIR"
KERNEL_BIN="$OUTDIR/bzImage"

echo "📥 Downloading minimal kernel for QEMU..."

# Try multiple minimal kernel sources
KERNEL_SOURCES=(
    "https://github.com/linuxkit/linuxkit/releases/download/v1.7.1/linuxkit-linux-amd64"
    "https://github.com/linuxkit/linuxkit/releases/download/v1.7.1/linuxkit-linux-amd64"
)

# First, let's try to get a working kernel from a different approach
echo "🔍 Trying alternative kernel sources..."

# Try to download from a minimal Linux distribution
MINIMAL_URL="https://dl-cdn.alpinelinux.org/alpine/v3.18/releases/x86_64/alpine-minirootfs-3.18.4-x86_64.tar.gz"
echo "Downloading Alpine Linux to extract kernel..."
curl -L -o "$OUTDIR/alpine.tar.gz" "$MINIMAL_URL"

if [ -f "$OUTDIR/alpine.tar.gz" ]; then
    echo "Extracting Alpine Linux..."
    mkdir -p "$OUTDIR/alpine"
    tar -xzf "$OUTDIR/alpine.tar.gz" -C "$OUTDIR/alpine"
    
    # Look for any kernel-like files
    echo "Searching for kernel files..."
    find "$OUTDIR/alpine" -type f -name "*vmlinuz*" -o -name "*bzImage*" -o -name "*Image*" | head -5
fi

echo ""
echo "💡 Since automated downloads aren't working, here's what you need to do:"
echo ""
echo "1. MANUAL KERNEL DOWNLOAD:"
echo "   Visit: https://www.kernel.org/pub/linux/kernel/v5.x/"
echo "   Download: linux-5.10.113.tar.xz"
echo "   Extract and build:"
echo "   tar -xf linux-5.10.113.tar.xz"
echo "   cd linux-5.10.113"
echo "   make defconfig"
echo "   make -j$(nproc) bzImage"
echo "   cp arch/x86/boot/bzImage ../vyomaos/out/bzImage"
echo ""
echo "2. USE A DISTRIBUTION KERNEL:"
echo "   - Ubuntu: /boot/vmlinuz-$(uname -r)"
echo "   - macOS: Download from a Linux VM"
echo "   - Or use a Linux system to build"
echo ""
echo "3. PLACE KERNEL FILE:"
echo "   Put the kernel file at: $KERNEL_BIN"
echo "   It should be a bzImage, not an ELF executable"

#!/usr/bin/env bash
set -euo pipefail

echo "🔧 VyomaOS - Fix Kernel Issue"
echo "=============================="

OUTDIR=out
mkdir -p "$OUTDIR"
KERNEL_BIN="$OUTDIR/bzImage"

echo "Current kernel status:"
if [ -f "$KERNEL_BIN" ]; then
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
echo "🔧 SOLUTIONS TO GET A WORKING KERNEL:"
echo ""
echo "1. MANUAL DOWNLOAD (Easiest):"
echo "   curl -L -o $KERNEL_BIN https://www.kernel.org/pub/linux/kernel/v5.x/linux-5.10.113.tar.xz"
echo ""
echo "2. BUILD YOUR OWN (Most reliable):"
echo "   git clone https://github.com/torvalds/linux.git"
echo "   cd linux"
echo "   make defconfig"
echo "   make -j$(nproc) bzImage"
echo "   cp arch/x86/boot/bzImage ../$KERNEL_BIN"
echo ""
echo "3. USE A DISTRIBUTION KERNEL:"
echo "   Download from your Linux distribution"
echo "   Place the bzImage file at: $KERNEL_BIN"
echo ""
echo "4. MANUAL PLACEMENT:"
echo "   Place any valid Linux kernel bzImage at: $KERNEL_BIN"
echo ""

read -p "Would you like me to try downloading from kernel.org? (y/n): " choice

if [[ $choice =~ ^[Yy]$ ]]; then
    echo "📥 Downloading from kernel.org..."
    KERNEL_URL="https://www.kernel.org/pub/linux/kernel/v5.x/linux-5.10.113.tar.xz"
    
    if curl -L -o "$KERNEL_BIN" "$KERNEL_URL" && [ -s "$KERNEL_BIN" ]; then
        echo "✅ Download successful"
        echo "📋 File info:"
        file "$KERNEL_BIN"
        
        if file "$KERNEL_BIN" | grep -q "ELF"; then
            echo "⚠️  Still an ELF executable - this won't work"
            echo "💡 You need to manually build or download a proper kernel"
        else
            echo "✅ This looks like a proper kernel!"
        fi
    else
        echo "❌ Download failed"
    fi
else
    echo "💡 Please manually place a kernel file at: $KERNEL_BIN"
fi

echo ""
echo "🔧 After getting a proper kernel, run:"
echo "   ./run.sh"
echo ""
echo "📋 The kernel file should be a bzImage, not an ELF executable"

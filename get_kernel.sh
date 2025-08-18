#!/usr/bin/env bash
set -euo pipefail

echo "🔍 Finding QEMU-friendly Linux kernel..."

# Try multiple sources for a proper kernel
KERNEL_SOURCES=(
    "https://github.com/linuxkit/linuxkit/releases/download/v1.7.1/linuxkit-linux-amd64"
    "https://github.com/linuxkit/linuxkit/releases/download/v1.7.1/linuxkit-linux-amd64"
)

OUTDIR=out
mkdir -p "$OUTDIR"
KERNEL_BIN="$OUTDIR/bzImage"

echo "📥 Attempting to download kernel..."

for url in "${KERNEL_SOURCES[@]}"; do
    echo "Trying: $url"
    if curl -L -o "$KERNEL_BIN" "$url" && [ -s "$KERNEL_BIN" ]; then
        echo "✅ Download successful"
        break
    fi
done

# Check what we got
if [ -f "$KERNEL_BIN" ]; then
    echo "📋 File info:"
    file "$KERNEL_BIN"
    echo "📏 File size: $(ls -lh "$KERNEL_BIN" | awk '{print $5}')"
    
    # Check if it's actually a kernel
    if file "$KERNEL_BIN" | grep -q "ELF"; then
        echo "⚠️  Warning: This appears to be an ELF executable, not a kernel"
        echo "💡 This means it's not a proper bzImage kernel for QEMU"
    else
        echo "✅ This appears to be a proper kernel file"
    fi
else
    echo "❌ Failed to download kernel"
fi

echo ""
echo "🔧 Manual kernel options:"
echo "1. Download from kernel.org: https://www.kernel.org/pub/linux/kernel/v5.x/"
echo "2. Build your own: git clone https://github.com/torvalds/linux.git"
echo "3. Use distribution kernels (Ubuntu, Alpine, etc.)"
echo "4. Check if your QEMU installation has sample kernels"

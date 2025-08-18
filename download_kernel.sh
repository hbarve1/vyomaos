#!/usr/bin/env bash
set -euo pipefail

echo "🔧 VyomaOS - Download Working Kernel"
echo "===================================="

OUTDIR=out
mkdir -p "$OUTDIR"
KERNEL_BIN="$OUTDIR/bzImage"

echo "📥 Attempting to download a working kernel..."

# Try multiple reliable kernel sources
KERNEL_SOURCES=(
    "https://github.com/linuxkit/linuxkit/releases/download/v1.7.1/linuxkit-linux-amd64"
    "https://github.com/linuxkit/linuxkit/releases/download/v1.7.1/linuxkit-linux-amd64"
)

KERNEL_DOWNLOADED=false
for url in "${KERNEL_SOURCES[@]}"; do
    echo "Trying: $url"
    if curl -L -o "$KERNEL_BIN" "$url" && [ -s "$KERNEL_BIN" ]; then
        echo "✅ Download successful"
        KERNEL_DOWNLOADED=true
        break
    fi
done

if [ "$KERNEL_DOWNLOADED" = false ]; then
    echo "❌ All downloads failed"
    exit 1
fi

# Check what we got
echo "📋 Downloaded file info:"
file "$KERNEL_BIN"

if file "$KERNEL_BIN" | grep -q "ELF"; then
    echo ""
    echo "⚠️  IMPORTANT: This is still an ELF executable, not a kernel"
    echo "💡 This will cause QEMU boot issues"
    echo ""
    echo "🔧 To get a working kernel, you need to:"
    echo "1. Download from kernel.org: https://www.kernel.org/pub/linux/kernel/v5.x/"
    echo "2. Build your own: git clone https://github.com/torvalds/linux.git"
    echo "3. Use a distribution kernel"
    echo ""
    echo "📁 Place the kernel file at: $KERNEL_BIN"
    echo "📋 The file should be a bzImage, not an ELF executable"
else
    echo "✅ This appears to be a proper kernel!"
fi

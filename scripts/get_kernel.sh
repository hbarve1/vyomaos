#!/usr/bin/env bash
set -euo pipefail

# Simple Kernel Download Script
# Downloads a working kernel from reliable sources

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"
OUTDIR="$PROJECT_ROOT/out"
KERNEL_FILE="$OUTDIR/bzImage"

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

log_info() { echo -e "${BLUE}ℹ️  $1${NC}"; }
log_success() { echo -e "${GREEN}✅ $1${NC}"; }
log_warning() { echo -e "${YELLOW}⚠️  $1${NC}"; }
log_error() { echo -e "${RED}❌ $1${NC}"; }

echo "🔧 VyomaOS - Get Working Kernel"
echo "================================"

mkdir -p "$OUTDIR"

# Option 1: Download from Alpine Linux (most reliable)
echo "📥 Option 1: Download from Alpine Linux"
echo "This will download a minimal Alpine Linux ISO and extract the kernel"

read -p "Try this option? (y/n): " choice
if [[ $choice =~ ^[Yy]$ ]]; then
    log_info "Downloading Alpine Linux ISO..."
    
    # Download Alpine Linux ISO
    ALPINE_ISO_URL="https://dl-cdn.alpinelinux.org/alpine/v3.18/releases/x86_64/alpine-standard-3.18.4-x86_64.iso"
    ALPINE_ISO="$OUTDIR/alpine.iso"
    
    if curl -L -o "$ALPINE_ISO" "$ALPINE_ISO_URL"; then
        log_success "Downloaded Alpine ISO"
        
        # Extract kernel from ISO
        log_info "Extracting kernel from ISO..."
        if command -v 7z &> /dev/null; then
            7z x "$ALPINE_ISO" boot/vmlinuz-virt -o"$OUTDIR" -y
            if [[ -f "$OUTDIR/boot/vmlinuz-virt" ]]; then
                cp "$OUTDIR/boot/vmlinuz-virt" "$KERNEL_FILE"
                log_success "Kernel extracted successfully!"
                exit 0
            fi
        else
            log_warning "7z not found, trying alternative extraction..."
            # Try to extract using dd and mount
            sudo mkdir -p /mnt/alpine
            sudo mount -o loop "$ALPINE_ISO" /mnt/alpine 2>/dev/null || true
            if [[ -f "/mnt/alpine/boot/vmlinuz-virt" ]]; then
                cp "/mnt/alpine/boot/vmlinuz-virt" "$KERNEL_FILE"
                sudo umount /mnt/alpine
                log_success "Kernel extracted successfully!"
                exit 0
            fi
        fi
    fi
fi

# Option 2: Download from Ubuntu cloud images
echo ""
echo "📥 Option 2: Download from Ubuntu cloud images"
echo "This will download a kernel from Ubuntu's cloud image repository"

read -p "Try this option? (y/n): " choice
if [[ $choice =~ ^[Yy]$ ]]; then
    log_info "Downloading Ubuntu kernel..."
    
    # Try multiple Ubuntu kernel URLs
    UBUNTU_URLS=(
        "https://cloud-images.ubuntu.com/focal/current/unpacked/kernel"
        "https://cloud-images.ubuntu.com/jammy/current/unpacked/kernel"
        "https://cloud-images.ubuntu.com/lunar/current/unpacked/kernel"
    )
    
    for url in "${UBUNTU_URLS[@]}"; do
        log_info "Trying: $url"
        if curl -L -o "$KERNEL_FILE" "$url" && [[ -s "$KERNEL_FILE" ]]; then
            if file "$KERNEL_FILE" | grep -q "ELF"; then
                log_success "Downloaded kernel successfully!"
                exit 0
            else
                log_warning "Downloaded file is not a kernel"
                rm -f "$KERNEL_FILE"
            fi
        fi
    done
fi

# Option 3: Manual download instructions
echo ""
echo "📥 Option 3: Manual Download"
echo "Since automated downloads are unreliable, here's what you need to do:"
echo ""
echo "1. Visit: https://www.kernel.org/pub/linux/kernel/v5.x/"
echo "2. Download: linux-5.10.113.tar.xz"
echo "3. Extract and build:"
echo "   tar -xf linux-5.10.113.tar.xz"
echo "   cd linux-5.10.113"
echo "   make defconfig"
echo "   make -j$(nproc) bzImage"
echo "   cp arch/x86/boot/bzImage $KERNEL_FILE"
echo ""
echo "4. Or get a kernel from your Linux distribution:"
echo "   - Ubuntu: /boot/vmlinuz-$(uname -r)"
echo "   - Debian: /boot/vmlinuz-$(uname -r)"
echo "   - CentOS: /boot/vmlinuz-$(uname -r)"
echo ""
echo "5. Place the kernel file at: $KERNEL_FILE"

# Option 4: Use a minimal kernel build script
echo ""
echo "📥 Option 4: Minimal Kernel Build Script"
echo "This will create a simple script to build a minimal kernel"

read -p "Create build script? (y/n): " choice
if [[ $choice =~ ^[Yy]$ ]]; then
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
    log_success "Created build script: $OUTDIR/build_minimal_kernel.sh"
    log_info "Run it manually: cd $OUTDIR && ./build_minimal_kernel.sh"
fi

echo ""
log_info "Summary:"
echo "- Option 1: Alpine Linux ISO (most reliable)"
echo "- Option 2: Ubuntu cloud images (may not work)"
echo "- Option 3: Manual download and build"
echo "- Option 4: Minimal build script created"
echo ""
log_info "The kernel file should be placed at: $KERNEL_FILE"

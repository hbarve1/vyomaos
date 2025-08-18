#!/usr/bin/env bash
set -euo pipefail

# VyomaOS Kernel Management Script
# Handles kernel download, build, and validation

# Configuration
readonly SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"
readonly OUTDIR="$PROJECT_ROOT/out"
readonly KERNEL_FILE="$OUTDIR/bzImage"

# Colors for output
readonly RED='\033[0;31m'
readonly GREEN='\033[0;32m'
readonly YELLOW='\033[1;33m'
readonly BLUE='\033[0;34m'
readonly NC='\033[0m' # No Color

# Logging functions
log_info() { echo -e "${BLUE}ℹ️  $1${NC}"; }
log_success() { echo -e "${GREEN}✅ $1${NC}"; }
log_warning() { echo -e "${YELLOW}⚠️  $1${NC}"; }
log_error() { echo -e "${RED}❌ $1${NC}"; }

# Kernel sources
readonly KERNEL_SOURCES=(
    "https://github.com/linuxkit/linuxkit/releases/download/v1.7.1/linuxkit-linux-amd64"
    "https://github.com/linuxkit/linuxkit/releases/download/v1.7.1/linuxkit-linux-amd64"
)

# Utility functions
check_kernel_status() {
    if [[ ! -f "$KERNEL_FILE" ]]; then
        log_warning "No kernel file found"
        return 1
    fi
    
    log_info "Kernel file info:"
    file "$KERNEL_FILE"
    
    if file "$KERNEL_FILE" | grep -q "ELF"; then
        log_warning "Kernel is an ELF executable (not a real kernel)"
        return 1
    elif file "$KERNEL_FILE" | grep -q "data\|HTML"; then
        log_warning "Kernel is a placeholder or invalid file"
        return 1
    else
        log_success "Kernel appears to be valid"
        return 0
    fi
}

download_kernel() {
    log_info "Attempting to download kernel from multiple sources..."
    
    for url in "${KERNEL_SOURCES[@]}"; do
        log_info "Trying: $url"
        if curl -L -o "$KERNEL_FILE" "$url" && [[ -s "$KERNEL_FILE" ]]; then
            log_success "Download successful"
            return 0
        fi
    done
    
    log_error "All download attempts failed"
    return 1
}

build_kernel_docker() {
    if ! command -v docker &> /dev/null; then
        log_error "Docker not found. Install Docker first."
        return 1
    fi
    
    log_info "Building kernel using Docker..."
    
    # Create Dockerfile
    cat > "$OUTDIR/Dockerfile" << 'EOF'
FROM ubuntu:20.04

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

WORKDIR /kernel
RUN wget https://www.kernel.org/pub/linux/kernel/v5.x/linux-5.10.113.tar.xz
RUN tar -xf linux-5.10.113.tar.xz
WORKDIR /kernel/linux-5.10.113

# Use x86_64 defconfig instead of defconfig
RUN make x86_64_defconfig
RUN echo "CONFIG_SERIAL_8250=y" >> .config
RUN echo "CONFIG_SERIAL_8250_CONSOLE=y" >> .config
RUN echo "CONFIG_DEVTMPFS=y" >> .config
RUN echo "CONFIG_DEVTMPFS_MOUNT=y" >> .config

RUN make -j$(nproc) bzImage

CMD ["cp", "arch/x86/boot/bzImage", "/output/bzImage"]
EOF

    # Build and run Docker container with platform specification
    if docker build --platform linux/amd64 -t kernel-builder "$OUTDIR" && \
       docker run --platform linux/amd64 --rm -v "$OUTDIR":/output kernel-builder; then
        log_success "Kernel built successfully using Docker"
        return 0
    else
        log_error "Docker build failed"
        return 1
    fi
}

build_kernel_local() {
    log_info "Building kernel locally..."
    
    # Check if we have the kernel source
    if [[ ! -d "$PROJECT_ROOT/linux" ]]; then
        log_info "Cloning Linux kernel source..."
        git clone https://github.com/torvalds/linux.git "$PROJECT_ROOT/linux"
    fi
    
    cd "$PROJECT_ROOT/linux"
    
    # Check make version
    local make_version
    make_version=$(make --version | head -1 | grep -o '[0-9]\+\.[0-9]\+' | head -1)
    if [[ $(echo "$make_version < 4.0" | bc -l 2>/dev/null) == 1 ]]; then
        log_error "GNU Make version $make_version is too old. Version 4.0+ required."
        log_info "Try using Docker build instead: ./scripts/kernel.sh docker"
        return 1
    fi
    
    # Build kernel
    log_info "Configuring kernel..."
    make defconfig
    
    log_info "Building kernel..."
    if make -j$(nproc) bzImage; then
        cp arch/x86/boot/bzImage "$KERNEL_FILE"
        log_success "Kernel built successfully"
        return 0
    else
        log_error "Kernel build failed"
        return 1
    fi
}

manual_kernel_placement() {
    log_info "Manual kernel placement"
    echo ""
    echo "Please place a valid bzImage kernel file at: $KERNEL_FILE"
    echo ""
    echo "You can get kernels from:"
    echo "- https://www.kernel.org/pub/linux/kernel/v5.x/"
    echo "- Your Linux distribution"
    echo "- Build your own with: git clone https://github.com/torvalds/linux.git"
    echo ""
    echo "Press Enter when you've placed the kernel file..."
    read -r
    
    if [[ -f "$KERNEL_FILE" ]]; then
        check_kernel_status
    else
        log_error "No kernel file found at $KERNEL_FILE"
        return 1
    fi
}

show_help() {
    cat << EOF
VyomaOS Kernel Management

Usage: $0 [COMMAND]

Commands:
  status     Check current kernel status
  download   Download pre-built kernel (may not work)
  docker     Build kernel using Docker
  local      Build kernel locally (requires Make 4.0+)
  manual     Manual kernel placement
  help       Show this help message

Examples:
  $0 status          # Check if kernel is valid
  $0 docker          # Build kernel using Docker
  $0 manual          # Place kernel manually

Note: Most automated downloads return ELF executables instead of kernels.
      The most reliable method is building locally or using Docker.
EOF
}

main() {
    # Ensure output directory exists
    mkdir -p "$OUTDIR"
    
    # Parse command line arguments
    case "${1:-help}" in
        status)
            check_kernel_status
            ;;
        download)
            if download_kernel; then
                check_kernel_status
            fi
            ;;
        docker)
            if build_kernel_docker; then
                check_kernel_status
            fi
            ;;
        local)
            if build_kernel_local; then
                check_kernel_status
            fi
            ;;
        manual)
            manual_kernel_placement
            ;;
        help|--help|-h)
            show_help
            ;;
        *)
            log_error "Unknown command: $1"
            show_help
            exit 1
            ;;
    esac
}

# Run main function
main "$@"

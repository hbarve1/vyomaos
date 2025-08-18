#!/usr/bin/env bash
# VyomaOS Kernel Module
# Handles Linux kernel download, configuration, and compilation

# Load dependencies
source "$(dirname "${BASH_SOURCE[0]}")/../config.sh"
source "$(dirname "${BASH_SOURCE[0]}")/utils.sh"

# Build the Linux kernel
build_kernel() {
    log_info "Building Linux kernel..."
    
    if [[ -f "$KERNEL_FILE" ]]; then
        log_info "Kernel already exists, skipping build."
        return 0
    fi
    
    # Download kernel source
    if ! download_file "$KERNEL_SOURCE_URL" "$KERNEL_TAR_FILE" "Linux kernel source"; then
        return 1
    fi
    
    # Extract source
    log_info "Extracting kernel source..."
    create_dirs "$OUTDIR"
    tar -xJf "$KERNEL_TAR_FILE" -C "$OUTDIR"
    
    # Configure kernel
    cd "$KERNEL_SOURCE_DIR"
    log_info "Configuring kernel..."
    make defconfig > /dev/null 2>&1
    
    # Apply custom config
    for opt in "${KERNEL_CONFIG_OPTS[@]}"; do
        echo "$opt" >> .config
    done
    
    # Compile kernel
    log_info "Compiling kernel (this may take several minutes)..."
    if make -j"$BUILD_JOBS" bzImage > /dev/null 2>&1; then
        cp arch/x86/boot/bzImage "$KERNEL_FILE"
        log_success "Kernel built successfully."
        return 0
    else
        log_error "Kernel compilation failed."
        return 1
    fi
}

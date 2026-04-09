#!/usr/bin/env bash
# VyomaOS Kernel Module
# Builds Linux kernel using base/kernel.config merged onto tinyconfig.

source "$(dirname "${BASH_SOURCE[0]}")/../config.sh"

build_kernel() {
    [[ -f "$KERNEL_FILE" ]] && { log_info "Kernel already built, skipping."; return 0; }
    log_info "Building kernel..."

    download_file "$KERNEL_SOURCE_URL" "$KERNEL_TAR_FILE" "kernel source" || return 1

    mkdir -p "$OUTDIR"
    tar -xJf "$KERNEL_TAR_FILE" -C "$OUTDIR"

    local kernel_config="$PROJECT_ROOT/base/kernel.config"
    if [[ ! -f "$kernel_config" ]]; then
        log_error "Kernel config not found: $kernel_config"
        return 1
    fi

    cd "$KERNEL_SOURCE_DIR"

    # Merge our config fragment on top of tinyconfig for a minimal build
    make KCONFIG_ALLCONFIG="$kernel_config" tinyconfig

    make -j"$(nproc)" bzImage
    cp arch/x86/boot/bzImage "$KERNEL_FILE"

    log_success "Kernel built: $KERNEL_FILE ($(du -sh "$KERNEL_FILE" | cut -f1))"
}

build_kernel

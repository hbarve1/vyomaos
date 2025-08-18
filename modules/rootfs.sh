#!/usr/bin/env bash
# VyomaOS Root Filesystem Module

# Safer bash settings
set -Eeuo pipefail
IFS=$'\n\t'

source "$(dirname "${BASH_SOURCE[0]}")/../config.sh"

# Get the absolute directory of this script at load time
ROOTFS_MODULE_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# Resolve the directory containing built WASM apps
resolve_apps_build_dir() {
    # Priority: APPS_BUILD_DIR env -> project default
    if [[ -n "${APPS_BUILD_DIR:-}" && -d "${APPS_BUILD_DIR}" ]]; then
        echo "${APPS_BUILD_DIR}"
        return 0
    fi
    local default_dir="${PROJECT_ROOT}/apps/build"
    if [[ -d "${default_dir}" ]]; then
        echo "${default_dir}"
        return 0
    fi
    # As a last resort, try relative to this module
    local rel_dir="$(dirname "${ROOTFS_MODULE_DIR}")/apps/build"
    if [[ -d "${rel_dir}" ]]; then
        echo "${rel_dir}"
        return 0
    fi
    echo ""  # not found
}

create_wasm_runtime() {
    local script_path="${ROOTFS_MODULE_DIR}/scripts/wasmtime"
    local real_wrapper_path="${ROOTFS_MODULE_DIR}/scripts/wasmtime-real-wrapper"
    local wasmtime_binary="${WASMTIME_BINARY:-${HOME}/.wasmtime/bin/wasmtime}"
    
    # Option to include real Wasmtime (disabled by default due to size)
    if [[ "${INCLUDE_REAL_WASMTIME:-false}" == "true" && -f "${wasmtime_binary}" ]]; then
        echo "Installing real Wasmtime runtime..."
        cp "${wasmtime_binary}" "${ROOTFS_DIR}/usr/bin/wasmtime-real"
        chmod +x "${ROOTFS_DIR}/usr/bin/wasmtime-real"
        echo "✅ Real Wasmtime runtime installed ($(du -h "${wasmtime_binary}" | cut -f1))"
        
        # Copy the real wrapper script
        if [[ -f "${real_wrapper_path}" ]]; then
            cp "${real_wrapper_path}" "${ROOTFS_DIR}/usr/bin/wasmtime"
            chmod +x "${ROOTFS_DIR}/usr/bin/wasmtime"
        else
            log_error "Real Wasmtime wrapper not found: ${real_wrapper_path}"
            return 1
        fi
    elif [[ -f "${script_path}" ]]; then
        # Use the runtime wrapper script (default)
        echo "Installing WebAssembly runtime wrapper..."
        cp "${script_path}" "${ROOTFS_DIR}/usr/bin/wasmtime"
        chmod +x "${ROOTFS_DIR}/usr/bin/wasmtime"
        echo "✅ WebAssembly runtime installed"
    else
        log_error "WASM runtime script not found: ${script_path}"
        return 1
    fi
}

create_init_script() {
    local script_path="${ROOTFS_MODULE_DIR}/scripts/init"
    
    if [[ -f "${script_path}" ]]; then
        cp "${script_path}" "${ROOTFS_DIR}/init"
        chmod +x "${ROOTFS_DIR}/init"
    else
        log_error "Init script not found: ${script_path}"
        return 1
    fi
}

include_wasm_apps() {
    local apps_dir
    apps_dir="$(resolve_apps_build_dir)"

    if [[ -z "${apps_dir}" ]]; then
        log_info "No WASM apps directory found; skipping app inclusion"
        return 0
    fi

    if [[ -d "${apps_dir}" ]]; then
        echo "Including WASM applications from ${apps_dir}:"
        # Enable nullglob locally so loop is empty if no matches
        local oldopt
        oldopt=$(shopt -p nullglob || true)
        shopt -s nullglob
        for wasm_file in "${apps_dir}"/*.wasm; do
            if [[ -f "${wasm_file}" ]]; then
                cp "${wasm_file}" "${ROOTFS_DIR}/apps/"
                echo "  $(basename "${wasm_file}") ($(du -h "${wasm_file}" | cut -f1))"
            fi
        done
        # Restore previous nullglob state
        eval "${oldopt:-:}"
    fi
}

build_rootfs() {
    log_info "Building rootfs..."
    
    # Create directory structure
    mkdir -p "${ROOTFS_DIR}"/{bin,proc,sys,dev,usr/bin,apps}
    
    # Create VyomaOS marker file
    echo "VyomaOS $(date)" > "${ROOTFS_DIR}/.vyomaos"
    
    # Setup BusyBox
    download_file "${BUSYBOX_URL}" "${OUTDIR}/busybox" "BusyBox" || return 1
    chmod +x "${OUTDIR}/busybox"
    cp "${OUTDIR}/busybox" "${ROOTFS_DIR}/bin/busybox"
    
    # Create BusyBox symlinks
    for cmd in sh echo mount poweroff wc od head strings tr grep tail sort basename cut; do
        ln -sf /bin/busybox "${ROOTFS_DIR}/bin/${cmd}"
    done
    
    # Create WebAssembly runtime
    create_wasm_runtime || return 1
    
    # Include WASM applications
    include_wasm_apps
    
    # Create init script
    create_init_script || return 1
    
    # Ensure packaging tools exist
    if ! command -v cpio >/dev/null 2>&1; then
        log_error "cpio not found. Please install it (e.g., sudo apt-get install cpio)."
        return 1
    fi

    # Build initramfs without changing caller's cwd
    (
        cd "${ROOTFS_DIR}"
        find . | cpio -o -H newc 2>/dev/null | gzip > "${INITRAMFS_FILE}"
    )
    
    return 0
}

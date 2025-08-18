#!/usr/bin/env bash
# VyomaOS Minimal Utilities

# Download file
download_file() {
    local url="$1" output="$2" description="$3"
    [[ -f "$output" ]] && return 0
    log_info "Downloading $description..."
    curl -L -o "$output" "$url"
}

# Check dependencies
check_system_deps() {
    local missing=()
    for cmd in curl tar gzip make gcc flex bison bc; do
        command -v "$cmd" &>/dev/null || missing+=("$cmd")
    done
    for pkg in libelf-dev libssl-dev libncurses-dev; do
        dpkg -s "$pkg" &>/dev/null || missing+=("$pkg")
    done
    if [[ ${#missing[@]} -gt 0 ]]; then
        log_error "Missing: ${missing[*]}"
        return 1
    fi
    return 0
}

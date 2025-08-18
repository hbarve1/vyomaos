#!/usr/bin/env bash
# VyomaOS Utility Functions
# Common utilities used across modules

# Check if a command exists
command_exists() {
    command -v "$1" &> /dev/null
}

# Check if a package is installed (Debian/Ubuntu)
package_installed() {
    dpkg -s "$1" &> /dev/null 2>&1
}

# Download a file with curl
download_file() {
    local url="$1"
    local output="$2"
    local description="$3"
    
    if [[ -f "$output" ]]; then
        log_info "$description already exists, skipping download."
        return 0
    fi
    
    log_info "Downloading $description..."
    if curl -L -o "$output" "$url"; then
        log_success "$description downloaded."
        return 0
    else
        log_error "Failed to download $description."
        return 1
    fi
}

# Create directory structure
create_dirs() {
    local dirs=("$@")
    for dir in "${dirs[@]}"; do
        mkdir -p "$dir"
    done
}

# Check system dependencies
check_system_deps() {
    local missing=()
    local executables=("curl" "tar" "gzip" "make" "gcc" "flex" "bison" "bc")
    local packages=("libelf-dev" "libssl-dev" "libncurses-dev")
    
    # Check executables
    for exe in "${executables[@]}"; do
        if ! command_exists "$exe"; then
            missing+=("$exe")
        fi
    done
    
    # Check packages
    for pkg in "${packages[@]}"; do
        if ! package_installed "$pkg"; then
            missing+=("$pkg")
        fi
    done
    
    if [[ ${#missing[@]} -gt 0 ]]; then
        log_error "Missing dependencies: ${missing[*]}"
        log_info "Install with: sudo apt-get install ${missing[*]}"
        return 1
    fi
    
    log_success "All dependencies satisfied."
    return 0
}

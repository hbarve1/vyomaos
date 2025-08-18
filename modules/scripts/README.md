# VyomaOS Scripts

This directory contains standalone scripts used by the VyomaOS build system.

## Scripts

### `wasmtime`

The WebAssembly runtime script that gets installed in the root filesystem at `/usr/bin/wasmtime`. This script:

- Validates WASM binary format
- Extracts and displays metadata
- Simulates execution of WebAssembly applications
- Provides user-friendly output with emojis and formatting

### `init`

The init script that gets installed in the root filesystem at `/init`. This script:

- Mounts essential filesystems (proc, sys, devtmpfs)
- Displays the VyomaOS welcome message
- Lists available WebAssembly applications
- Runs the default WebAssembly application
- Initiates system shutdown

## Usage

These scripts are automatically copied to the root filesystem during the build process by the `rootfs.sh` module. They can be edited independently and will be included in the next build.

## Benefits of Separation

1. **Maintainability**: Each script can be edited independently
2. **Version Control**: Changes to scripts are clearly tracked
3. **Testing**: Scripts can be tested individually
4. **Reusability**: Scripts can be shared between different modules
5. **Readability**: No more heredoc syntax cluttering the main module

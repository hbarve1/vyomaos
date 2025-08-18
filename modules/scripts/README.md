# VyomaOS Scripts

This directory contains standalone scripts used by the VyomaOS build system.

## Scripts

### `wasmtime`

The WebAssembly runtime shim that gets installed in the root filesystem at `/usr/bin/wasmtime`. This script:

- Delegates execution to a real Wasmtime binary if available
- Tries `/usr/bin/wasmtime-real`, then `wasmtime` in PATH, then `$HOME/.wasmtime/bin/wasmtime`
- Exits with an error if no runtime is available

### `init`

The init script that gets installed in the root filesystem at `/init`. This script:

- Mounts essential filesystems (proc, sys, devtmpfs)
- Displays the VyomaOS welcome message
- Lists available WebAssembly applications
- Runs each WebAssembly application using the runtime shim
- Initiates system shutdown

## Usage

These scripts are automatically copied to the root filesystem during the build process by the `rootfs.sh` module. They can be edited independently and will be included in the next build.

## Benefits of Separation

1. **Maintainability**: Each script can be edited independently
2. **Version Control**: Changes to scripts are clearly tracked
3. **Testing**: Scripts can be tested individually
4. **Reusability**: Scripts can be shared between different modules
5. **Readability**: No heredoc syntax cluttering the main module

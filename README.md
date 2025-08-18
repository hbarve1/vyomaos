# VyomaOS - Minimal WebAssembly Operating System

A clean, modular Linux-based operating system designed to run WebAssembly applications.

## 🚀 Quick Start

```bash
# Build and run VyomaOS
./vyomaos.sh build
./vyomaos.sh run
```

## 📁 Project Structure

```
vyomaos/
├── vyomaos.sh              # Main entry point
├── config.sh               # System configuration
├── modules/                # Modular components
│   ├── utils.sh            # Common utilities
│   ├── kernel.sh           # Kernel build module
│   ├── rootfs.sh           # Root filesystem module
│   └── qemu.sh             # QEMU virtualization module
├── out/                    # Build artifacts (generated)
└── README.md               # This file
```

## 🏗️ Architecture

```
┌─────────────────────────────────────┐
│              QEMU VM                │
├─────────────────────────────────────┤
│  Linux Kernel 5.10.113             │
├─────────────────────────────────────┤
│  Minimal Root Filesystem            │
│  ├── BusyBox (utilities)            │
│  ├── WebAssembly Runtime            │
│  ├── Sample WASM App                │
│  └── Init Script                    │
└─────────────────────────────────────┘
```

## 🔧 Components

- **Linux Kernel**: Minimal 5.10.113 kernel configured for QEMU
- **BusyBox**: Essential Unix utilities (sh, ls, echo, mount, poweroff, etc.)
- **WASM Runtime**: Simple WebAssembly execution environment
- **Init System**: Custom boot script that runs WASM applications

## � Commands

```bash
./vyomaos.sh build    # Build the complete OS
./vyomaos.sh run      # Boot in QEMU
./vyomaos.sh clean    # Remove build artifacts
./vyomaos.sh help     # Show usage information
```

## ⚙️ Requirements

- **Ubuntu/Debian** Linux system
- **Dependencies**: curl, tar, gzip, make, gcc, flex, bison, bc
- **Development packages**: libelf-dev, libssl-dev, libncurses-dev
- **QEMU**: qemu-system-x86

Install dependencies:
```bash
sudo apt-get update
sudo apt-get install curl tar gzip make gcc flex bison bc \
                     libelf-dev libssl-dev libncurses-dev \
                     qemu-system-x86
```

## � Build Process

1. **Dependency Check**: Validates all required tools and libraries
2. **Kernel Build**: Downloads and compiles Linux kernel from source
3. **Root Filesystem**: Creates minimal filesystem with BusyBox and WASM runtime
4. **Initramfs Creation**: Packages filesystem into bootable archive

## 🎯 Key Features

- **Minimal**: Only essential components included
- **Modular**: Clean separation of concerns
- **Fast Boot**: Optimized for quick startup
- **WASM Ready**: Built-in WebAssembly execution environment
- **Self-Contained**: No external runtime dependencies

## 🧪 Example Output

```
Welcome to VyomaOS!
==================

Starting WebAssembly application...
WebAssembly Runtime
===================
Loading: /app.wasm
Size: 120 bytes

Executing WebAssembly module...
Hello from WebAssembly!
Runtime execution completed.

System shutdown initiated.
```

## � Next Steps

- Replace mock WASM runtime with real interpreter (WASM3, Wasmer)
- Add networking capabilities
- Create package system for WASM applications
- Implement persistent storage
- Add development tools

## � License

Educational and experimental use.

---

**VyomaOS** - Demonstrating minimal OS architecture for WebAssembly applications.

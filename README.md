# VyomaOS - Minimal WebAssembly Operating System

A clean, modular Linux-based operating system designed to run WebAssembly applications. Built with minimal dependencies and a streamlined architecture.

## 🚀 Quick Start

```bash
# Build the complete system
./vyomaos.sh build

# Boot VyomaOS in QEMU
./vyomaos.sh run

# Clean build artifacts
./vyomaos.sh clean
```

## 📁 Project Structure

```
vyomaos/
├── vyomaos.sh              # Main entry point (56 lines)
├── config.sh               # System configuration (66 lines)
├── modules/                # Modular components
│   ├── utils.sh            # Utilities (26 lines)
│   ├── kernel.sh           # Kernel build (27 lines)
│   ├── rootfs.sh           # Root filesystem (51 lines)
│   └── qemu.sh             # QEMU boot (22 lines)
├── apps/                   # WebAssembly applications
│   ├── build.sh            # Build script for all apps
│   ├── hello-world/        # Demo Rust→WASM app
│   ├── calculator/         # Mathematical operations app
│   └── build/              # Compiled WASM binaries
└── out/                    # Build output (auto-generated)
```

**Total codebase: 248 lines** - truly minimal!

## 🦀 WebAssembly Applications

VyomaOS includes a dedicated `apps/` folder for Rust applications that compile to WebAssembly:

```bash
# Build WASM apps and integrate with OS
./build-with-apps.sh

# Or build apps separately
cd apps && ./build.sh

# Then rebuild OS to include apps
./vyomaos.sh clean && ./vyomaos.sh build

# Create new WASM app
mkdir apps/my-app
# See apps/README.md for detailed instructions
```

**Current WASM Applications:**
- `hello-world.wasm` (28KB) - String handling, arithmetic, recursion
- `calculator.wasm` (3.2KB) - Mathematical operations and functions

## 🏗️ Architecture

```
┌─────────────────────────────────────┐
│            QEMU Virtual Machine     │
├─────────────────────────────────────┤
│  Linux Kernel 5.10.113             │
│  • Minimal configuration           │
│  • x86_64 architecture             │
│  • Essential drivers only          │
├─────────────────────────────────────┤
│  Minimal Root Filesystem            │
│  ├── BusyBox (static binary)        │
│  ├── Mock WebAssembly Runtime       │
│  ├── Sample WASM Application        │
│  └── Custom init system             │
└─────────────────────────────────────┘
```

## 🔧 Core Components

- **Linux Kernel 5.10.113**: Minimal configuration optimized for QEMU
- **BusyBox**: Essential Unix utilities (sh, echo, mount, poweroff, wc)
- **WebAssembly Runtime**: Mock implementation demonstrating WASM execution
- **Init System**: Custom boot sequence that runs WebAssembly applications
- **QEMU Virtualization**: Software-based x86_64 emulation

## ⚙️ System Requirements

**Operating System**: Ubuntu/Debian Linux (tested on Ubuntu)

**Dependencies**:
```bash
sudo apt-get update
sudo apt-get install curl tar gzip make gcc flex bison bc 
                     libelf-dev libssl-dev libncurses-dev 
                     qemu-system-x86
```

**Hardware**: 512MB RAM, 2GB disk space

## 📋 Commands

| Command | Description |
|---------|-------------|
| `./vyomaos.sh build` | Build kernel and root filesystem |
| `./vyomaos.sh run` | Boot VyomaOS in QEMU |
| `./vyomaos.sh clean` | Remove all build artifacts |
| `./build-with-apps.sh` | Build WASM apps + integrate with OS |
| `cd apps && ./build.sh` | Build only WebAssembly applications |

## 🛠️ Build Process

1. **System Check**: Validates required dependencies
2. **Kernel Build**: Downloads and compiles Linux 5.10.113 with minimal config
3. **Root Filesystem**: Creates filesystem with BusyBox and WebAssembly runtime
4. **Initramfs**: Packages filesystem into bootable cpio archive
5. **Ready to Boot**: Complete system ready for QEMU

## 🎯 Key Features

- **Ultra Minimal**: Only 248 lines of code total
- **Modular Design**: Clean separation of kernel, filesystem, and boot logic
- **Fast Boot**: Optimized for quick startup (< 10 seconds)
- **WebAssembly Ready**: Built-in execution environment
- **Self-Contained**: No external runtime dependencies
- **Educational**: Clean codebase perfect for learning OS concepts

## 🧪 Example Boot Output

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

## 🔄 Development Workflow

```bash
# Make changes to modules
vim modules/rootfs.sh

# Rebuild and test
./vyomaos.sh clean
./vyomaos.sh build
./vyomaos.sh run
```

## 🚀 Next Steps

- **Real WASM Runtime**: Replace mock with WASM3, Wasmer, or Wasmtime
- **Networking**: Add basic network capabilities
- **Package System**: Create WASM application management
- **Persistent Storage**: Add filesystem persistence
- **Multi-app Support**: Run multiple WebAssembly applications

## 📈 Technical Details

- **Kernel Size**: ~8MB compiled
- **Root Filesystem**: ~2MB initramfs
- **Boot Time**: ~5-10 seconds
- **Memory Usage**: ~32MB minimum
- **QEMU CPU**: qemu64 with TCG acceleration

## 🎓 Educational Value

VyomaOS demonstrates:
- Minimal Linux system construction
- Kernel compilation and configuration
- Custom init system development
- WebAssembly integration concepts
- QEMU virtualization basics
- Modular software architecture

## 📄 License

MIT License - Educational and experimental use.

---

**VyomaOS** - A minimal WebAssembly operating system showcasing clean architecture and educational OS development. modular Linux-based operating system designed to run WebAssembly applications.

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

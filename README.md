# VyomaOS - WebAssembly Operating System

A minimal Linux-based operating system designed to run WebAssembly applications using the Wasmtime runtime.

## 🚀 Quick Start

```bash
# Build VyomaOS
./vyomaos.sh build

# Get a working kernel (choose one method)
./vyomaos.sh kernel docker    # Build using Docker (recommended)
./vyomaos.sh kernel local     # Build locally (requires Make 4.0+)
./vyomaos.sh kernel manual    # Place kernel manually

# Boot VyomaOS
./vyomaos.sh run
```

## 📁 Project Structure

```
vyomaos/
├── vyomaos.sh              # Main entry point script
├── config.sh               # Centralized configuration
├── scripts/
│   ├── build.sh            # Build script
│   ├── kernel.sh           # Kernel management
│   └── run.sh              # QEMU boot script
├── out/                    # Build artifacts
├── docs/                   # Documentation
└── README.md              # This file
```

## 🏗️ Architecture

```
┌─────────────────────────────────────┐
│              QEMU VM                │
├─────────────────────────────────────┤
│  Linux Kernel (bzImage)             │
├─────────────────────────────────────┤
│  Initramfs (rootfs + WASM app)      │
│  ├── /bin/busybox (shell)           │
│  ├── /usr/bin/wasmtime (runtime)    │
│  ├── /wasm_app.wasm (application)   │
│  └── /init (boot script)            │
└─────────────────────────────────────┘
```

## 🔧 Components

- **Linux Kernel**: Minimal kernel for QEMU boot
- **BusyBox**: Static binary providing Unix utilities
- **Wasmtime**: WebAssembly runtime (v16.0.0)
- **WASM App**: Sample base64-encoded WASM application
- **Init Script**: Boot script that runs WASM app as PID 1

## 🚀 Usage

### Main Commands

```bash
# Show help
./vyomaos.sh help

# Check status
./vyomaos.sh status

# Build the OS
./vyomaos.sh build

# Manage kernel
./vyomaos.sh kernel [command]

# Boot in QEMU
./vyomaos.sh run

# Clean build artifacts
./vyomaos.sh clean
```

### Kernel Management

```bash
# Check kernel status
./vyomaos.sh kernel status

# Download pre-built kernel (may not work)
./vyomaos.sh kernel download

# Build kernel using Docker (recommended)
./vyomaos.sh kernel docker

# Build kernel locally (requires Make 4.0+)
./vyomaos.sh kernel local

# Manual kernel placement
./vyomaos.sh kernel manual
```

## ⚠️ Kernel Issue & Solutions

The main challenge is obtaining a proper Linux kernel (`bzImage`) for QEMU. Automated downloads often return ELF executables instead of kernels.

### Solution 1: Docker Build (Recommended)

```bash
# Ensure Docker is installed
docker --version

# Build kernel using Docker
./vyomaos.sh kernel docker
```

### Solution 2: Local Build

```bash
# Check Make version (requires 4.0+)
make --version

# Build kernel locally
./vyomaos.sh kernel local
```

### Solution 3: Manual Placement

```bash
# Get kernel from another source
./vyomaos.sh kernel manual

# Then place your kernel file at: out/bzImage
```

## 🔍 Troubleshooting

### "Kernel is an ELF executable"
- This means you have a LinuxKit binary, not a kernel
- Run `./vyomaos.sh kernel docker` to build a proper kernel

### "QEMU boot fails"
- Ensure you have a proper Linux kernel bzImage
- Check kernel status: `./vyomaos.sh kernel status`

### "Make version too old"
- macOS: Install newer Make via Homebrew
- Or use Docker build: `./vyomaos.sh kernel docker`

### "Missing dependencies"
```bash
# macOS
brew install qemu curl

# Ubuntu
sudo apt-get install qemu-system-x86 curl tar gzip
```

## 📋 Requirements

### System Requirements
- **OS**: macOS, Linux, or Windows with WSL
- **Memory**: 512MB RAM for QEMU
- **Storage**: ~100MB for build artifacts

### Dependencies
- **QEMU**: For virtualization
- **curl**: For downloading components
- **tar/gzip**: For extracting archives
- **Docker**: For kernel building (optional)

## 🎯 Development

### Adding Custom WASM Applications

1. Replace the sample WASM app in `scripts/build.sh`:
```bash
# In create_wasm_app() function
cp your_app.wasm "$OUTDIR/rootfs/wasm_app.wasm"
```

2. Modify the init script if needed:
```bash
# In create_init_script() function
exec /usr/bin/wasmtime /your_app.wasm
```

### Customizing the Build

Edit `config.sh` to modify:
- Component URLs
- QEMU settings
- Build configuration
- Kernel options

## 📝 License

This project is experimental and for educational purposes.

## 🤝 Contributing

1. Fork the repository
2. Create a feature branch
3. Make your changes
4. Test thoroughly
5. Submit a pull request

## 🔗 References

- [Linux Kernel](https://www.kernel.org/)
- [Wasmtime](https://wasmtime.dev/)
- [BusyBox](https://busybox.net/)
- [QEMU](https://www.qemu.org/)

---

**Note**: VyomaOS is designed for educational purposes and demonstrates how to create a minimal OS for running WebAssembly applications.

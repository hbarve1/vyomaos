# VyomaOS - WebAssembly Operating System

A minimal Linux-based operating system that runs WebAssembly applications using the Wasmtime runtime.

## 🚀 Quick Start

```bash
# Build the OS
./build.sh

# Fix kernel issue (if needed)
./get_proper_kernel.sh

# Boot in QEMU
./run.sh
```

## 📁 Project Structure

- **`build.sh`** - Main build script (downloads components, creates rootfs)
- **`run.sh`** - QEMU boot script
- **`get_proper_kernel.sh`** - Interactive kernel fixer
- **`get_kernel.sh`** - Kernel diagnostic tool

## ⚠️ Kernel Issue & Solution

The main issue is getting a proper Linux kernel (`bzImage`) for QEMU. The build script currently creates a placeholder.

### 🔧 How to Fix the Kernel Issue

**Option 1: Use the Kernel Fixer (Recommended)**
```bash
./get_proper_kernel.sh
```

This provides 4 options:
1. **Download pre-built kernel** - Downloads from LinuxKit (but still ELF)
2. **Use Docker to build kernel** - Builds kernel in Docker container
3. **Manual kernel placement** - Place your own kernel file
4. **Skip kernel** - Use placeholder (QEMU will fail)

**Option 2: Manual Kernel Download**
```bash
# Download from kernel.org
curl -L -o out/bzImage https://www.kernel.org/pub/linux/kernel/v5.x/linux-5.10.113.tar.xz

# Or build your own
git clone https://github.com/torvalds/linux.git
cd linux
make defconfig
make -j$(nproc) bzImage
cp arch/x86/boot/bzImage ../out/bzImage
```

**Option 3: Use Docker (if available)**
```bash
# Run the kernel fixer and choose option 2
./get_proper_kernel.sh
# Choose option 2, then run the generated script
cd out && ./build_kernel_docker.sh
```

## 🏗️ Architecture

```
┌─────────────────────────────────────┐
│              QEMU VM                │
├─────────────────────────────────────┤
│  Linux Kernel (bzImage) - NEEDED    │
├─────────────────────────────────────┤
│  Initramfs (rootfs + WASM app)      │
│  ├── /bin/busybox (shell)           │
│  ├── /usr/bin/wasmtime (runtime)    │
│  ├── /wasm_app.wasm (application)   │
│  └── /init (boot script)            │
└─────────────────────────────────────┘
```

## 🔧 Components

- **Kernel**: Linux kernel (needs proper bzImage)
- **BusyBox**: Static binary for Unix utilities
- **Wasmtime**: WebAssembly runtime (v16.0.0)
- **WASM App**: Sample base64-encoded WASM application
- **Init Script**: Boot script that runs WASM app as PID 1

## 🚀 Boot Process

1. **QEMU starts** with kernel and initramfs
2. **Linux boots** and mounts filesystems
3. **Init script runs** and starts Wasmtime
4. **WASM app executes** as PID 1
5. **Fallback to shell** if WASM fails

## ✅ Current Status

- **Build Process**: ✅ Working
- **BusyBox**: ✅ Downloads correctly
- **Wasmtime**: ✅ Downloads correctly
- **Initramfs**: ✅ Creates properly
- **Error Detection**: ✅ Improved
- **User Guidance**: ✅ Clear instructions
- **Kernel**: ⚠️ Needs manual fix (but with clear guidance)

## 🐛 Troubleshooting

### "Kernel is an ELF executable"
- Run `./get_proper_kernel.sh` and choose option 3
- Manually place a proper bzImage kernel file

### "QEMU boot fails"
- Ensure you have a proper Linux kernel bzImage
- Check that the kernel supports initramfs

### "Build tools not found"
- macOS: `xcode-select --install`
- Linux: `sudo apt-get install build-essential`

## 📝 License

This project is experimental and for educational purposes.

## 🤝 Contributing

Feel free to submit issues and enhancement requests!

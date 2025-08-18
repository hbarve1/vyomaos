# VyomaOS WebAssembly Applications

This directory contains Rust applications that compile to WebAssembly for use in VyomaOS.

## 🚀 Quick Start

```bash
# Build all applications
./build.sh

# Built WASM files will be in build/ directory
ls build/
```

## 📁 Applications

### hello-world
A simple demonstration app showing basic WebAssembly exports:
- String handling and memory management
- Basic arithmetic operations
- Recursive function calls

### calculator
Advanced mathematical operations:
- Basic arithmetic (add, subtract, multiply, divide)
- Advanced functions (power, sqrt, sin, cos, log)
- Floating-point calculations

## 🛠️ Development

### Prerequisites

```bash
# Install Rust
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# Add WebAssembly target
rustup target add wasm32-unknown-unknown
```

### Creating a New App

```bash
# Create new app directory
mkdir apps/my-app
cd apps/my-app

# Create Cargo.toml
cat > Cargo.toml << EOF
[package]
name = "my-app"
version = "0.1.0"
edition = "2021"

[lib]
crate-type = ["cdylib"]

[dependencies]
EOF

# Create src/lib.rs with exported functions
mkdir src
cat > src/lib.rs << EOF
#[no_mangle]
pub extern "C" fn my_function() -> i32 {
    42
}
EOF
```

### Building Individual Apps

```bash
cd apps/hello-world
cargo build --target wasm32-unknown-unknown --release

# WASM file will be at:
# target/wasm32-unknown-unknown/release/hello_world.wasm
```

## 📋 Build Output

Built WASM files are placed in `apps/build/` directory:
- `hello-world.wasm` - Hello world demonstration
- `calculator.wasm` - Mathematical operations

## 🔌 Integration with VyomaOS

The WASM applications can be integrated into VyomaOS by:

1. Copying WASM files to the root filesystem during build
2. Updating the WebAssembly runtime to load specific applications
3. Creating a simple application launcher system

## 🎯 Best Practices

- **Keep apps small**: Minimize dependencies for smaller WASM binaries
- **Export C functions**: Use `#[no_mangle]` and `extern "C"` for WebAssembly exports
- **Memory management**: Handle string allocation/deallocation carefully
- **Error handling**: Return error codes or use Result types appropriately

## 🔍 Debugging

```bash
# Check WASM file structure
wasm-objdump -x build/hello-world.wasm

# View exported functions
wasm-objdump -j Export build/hello-world.wasm

# Validate WASM file
wasm-validate build/hello-world.wasm
```

## 📊 File Sizes

Typical WASM file sizes:
- hello-world: ~1-2KB (minimal functionality)
- calculator: ~2-3KB (mathematical operations)

## 🚀 Future Enhancements

- **WASI support**: Add WebAssembly System Interface for file/network access
- **Advanced apps**: Create more complex applications (games, utilities)
- **Runtime integration**: Better integration with VyomaOS runtime
- **Package management**: Create a simple package system for WASM apps

#!/usr/bin/env bash
# VyomaOS Apps Build Script
# Builds all Rust applications to WebAssembly

set -e

APPS_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BUILD_DIR="$APPS_DIR/build"

echo "🦀 Building VyomaOS WebAssembly Applications"
echo "============================================"

# Check if Rust is installed
if ! command -v rustc &> /dev/null; then
    echo "❌ Rust not found. Please install Rust first:"
    echo "   curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh"
    exit 1
fi

# Check if wasm32 target is installed
if ! rustup target list --installed | grep -q "wasm32-unknown-unknown"; then
    echo "📦 Installing WebAssembly target..."
    rustup target add wasm32-unknown-unknown
fi

# Create build directory
mkdir -p "$BUILD_DIR"

# Build each app
for app_dir in "$APPS_DIR"/*; do
    if [[ -d "$app_dir" && -f "$app_dir/Cargo.toml" ]]; then
        app_name=$(basename "$app_dir")
        echo "🔨 Building $app_name..."
        
        cd "$app_dir"
        cargo build --target wasm32-unknown-unknown --release
        
        # Copy WASM file to build directory
        wasm_file="target/wasm32-unknown-unknown/release/${app_name//-/_}.wasm"
        if [[ -f "$wasm_file" ]]; then
            cp "$wasm_file" "$BUILD_DIR/$app_name.wasm"
            echo "✅ Built $app_name.wasm ($(du -h "$BUILD_DIR/$app_name.wasm" | cut -f1))"
        else
            echo "❌ Failed to build $app_name"
        fi
    fi
done

echo ""
echo "📦 WebAssembly Applications:"
ls -la "$BUILD_DIR"/*.wasm 2>/dev/null || echo "No WASM files found"

echo ""
echo "✨ Build complete! WASM files are in: $BUILD_DIR"

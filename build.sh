#!/usr/bin/env bash
# VyomaOS WASM App Integration
# Updates the OS to include the latest WASM applications

set -e

# Build WASM apps first
echo "🦀 Building WebAssembly applications..."
cd apps && ./build.sh && cd ..

echo ""
echo "🔄 Rebuilding VyomaOS with WASM apps..."
./vyomaos.sh clean
./vyomaos.sh build

echo ""
echo "🚀 Ready to run VyomaOS with WebAssembly applications!"
echo "   Run: ./vyomaos.sh run"

./vyomaos.sh run

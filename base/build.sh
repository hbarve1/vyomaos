#!/usr/bin/env bash
# VyomaOS Complete Build Script

set -e

echo "🦀 Building WebAssembly applications..."
cd apps && ./build.sh && cd ..

echo ""
echo "🔄 Building VyomaOS with WASM apps..."
./vyomaos.sh clean
./vyomaos.sh build

echo ""
echo "🚀 Starting VyomaOS..."
./vyomaos.sh run

# Hello World WebAssembly App

A simple Rust application that compiles to WebAssembly, demonstrating basic functionality.

## Building

```bash
# Install Rust and WebAssembly target
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
rustup target add wasm32-unknown-unknown

# Build the WebAssembly module
cargo build --target wasm32-unknown-unknown --release
```

## Functions

- `hello()` - Returns a greeting message
- `add(a, b)` - Adds two integers
- `factorial(n)` - Calculates factorial of n
- `free_string(ptr)` - Frees memory allocated by hello()

## Output

The compiled WASM binary will be located at:
`target/wasm32-unknown-unknown/release/hello_world.wasm`

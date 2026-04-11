# P04T02 — wasi-preview2-config

## Phase

Phase 04 — WASM Runtime

## Goal

Migrate `apps/hello-world` from a WASI Preview 1 `cdylib` export to a standard WASI Preview 2 binary: update `Cargo.toml` to build a `[[bin]]` with `wasm32-wasip2` as the default target, add `.cargo/config.toml`, and rewrite `src/main.rs` as a plain `fn main()` using `println!`.

## File to create / modify

```
apps/hello-world/Cargo.toml
apps/hello-world/src/main.rs
apps/hello-world/.cargo/config.toml
```

## Implementation

### `apps/hello-world/Cargo.toml`

```toml
[package]
name    = "hello-world"
version = "0.1.0"
edition = "2021"

# Binary crate — not a library.
# The old cdylib / extern "C" pattern was WASI Preview 1.
# WASI Preview 2 (wasip2) uses a standard Rust binary entry point.
[[bin]]
name = "hello-world"
path = "src/main.rs"

[profile.release]
opt-level = "z"
lto       = true
codegen-units = 1
strip     = true
```

### `apps/hello-world/.cargo/config.toml`

```toml
[build]
# WASI Preview 2 target — requires Rust 1.78+ and wasm32-wasip2 target installed.
# Install with: rustup target add wasm32-wasip2
target = "wasm32-wasip2"
```

### `apps/hello-world/src/main.rs`

```rust
//! VyomaOS hello-world application
//! Targets wasm32-wasip2 (WASI Preview 2 / Component Model)
//!
//! This is a standard Rust binary. `println!` routes through the WASI
//! `wasi:cli/stdout` interface, which wasmtime maps to the host's stdout.

fn main() {
    println!("Hello from VyomaOS WASM!");
}
```

## Notes

- **Why the change from `cdylib` to `[[bin]]`?** WASI Preview 1 required exporting a `_start` symbol from a `cdylib`. WASI Preview 2 (the Component Model) exports a `wasi:cli/run` interface from a component binary. Rust's `wasm32-wasip2` target handles this automatically when the crate type is `bin` — the toolchain generates a proper component with all WASI interfaces wired.
- **`wasm32-wasip2` vs `wasm32-wasi`**: `wasm32-wasi` targets WASI Preview 1 (`wasm32-wasip1` is the alias). `wasm32-wasip2` targets WASI Preview 2 and emits a WebAssembly Component rather than a core module. Wasmtime 14+ can run both; the supervisor uses `wasmtime run --` which auto-detects the format.
- Remove any existing `[lib]` or `crate-type = ["cdylib"]` lines. If the old `src/lib.rs` file exists, rename or delete it — `cargo build` will error if both a `[[bin]]` src and `[lib]` src exist without explicit path disambiguation.
- The `.cargo/config.toml` is scoped to `apps/hello-world/` and does not affect `supervisor/` (which has its own `.cargo/config.toml` targeting `x86_64-unknown-linux-musl`).
- `strip = true` in the release profile for WASM has limited effect compared to native binaries, but it removes the name section and custom sections that add unnecessary bytes to the `.wasm` file.
- After building, the output is `apps/hello-world/target/wasm32-wasip2/release/hello-world.wasm`. The rootfs build script should copy this to `$ROOTFS/apps/hello-world.wasm`.

### Integrating with `base/modules/rootfs.sh`

Add to the rootfs script after building apps (or invoke from a separate build step):

```sh
# Build hello-world WASM app
HELLO_WASM="$REPO_ROOT/apps/hello-world/target/wasm32-wasip2/release/hello-world.wasm"
if [[ ! -f "$HELLO_WASM" ]]; then
  echo "[rootfs] Building hello-world WASM app"
  cargo build --manifest-path "$REPO_ROOT/apps/hello-world/Cargo.toml" --release
fi
mkdir -p "$ROOTFS/apps"
cp "$HELLO_WASM" "$ROOTFS/apps/hello-world.wasm"
echo "[rootfs] Installed /apps/hello-world.wasm"
```

## Verification

```sh
# 1. Required files exist
test -f apps/hello-world/Cargo.toml
test -f apps/hello-world/src/main.rs
test -f apps/hello-world/.cargo/config.toml

# 2. crate-type cdylib is NOT present (old Preview 1 approach)
grep -v "cdylib" apps/hello-world/Cargo.toml

# 3. [[bin]] section is present
grep -q '^\[\[bin\]\]' apps/hello-world/Cargo.toml

# 4. Target is wasm32-wasip2
grep -q 'wasm32-wasip2' apps/hello-world/.cargo/config.toml

# 5. main.rs uses fn main() not extern "C"
grep -q 'fn main()' apps/hello-world/src/main.rs
grep -v 'extern "C"' apps/hello-world/src/main.rs > /dev/null

# 6. main.rs contains the expected output string
grep -q 'Hello from VyomaOS WASM' apps/hello-world/src/main.rs

# 7. Compiles to a .wasm component
cargo build --manifest-path apps/hello-world/Cargo.toml --release
test -f apps/hello-world/target/wasm32-wasip2/release/hello-world.wasm

# 8. Output file is a WebAssembly module
file apps/hello-world/target/wasm32-wasip2/release/hello-world.wasm \
  | grep -q "WebAssembly"

# 9. wasmtime can run it locally (requires wasmtime on the host)
wasmtime run -- apps/hello-world/target/wasm32-wasip2/release/hello-world.wasm \
  | grep -q "Hello from VyomaOS WASM"
```

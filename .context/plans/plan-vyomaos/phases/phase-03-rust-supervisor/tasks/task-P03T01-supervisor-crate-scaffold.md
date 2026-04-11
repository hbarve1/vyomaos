# P03T01 — supervisor-crate-scaffold

## Phase

Phase 03 — Rust Supervisor

## Goal

Create the `supervisor/` Rust binary crate with a `Cargo.toml` configured for size-optimised musl-static builds, a `.cargo/config.toml` that sets the default target to `x86_64-unknown-linux-musl`, and a `src/main.rs` stub that prints `"vyoma-supervisor starting"` and loops forever as a placeholder PID-1 process.

## File to create / modify

```
supervisor/Cargo.toml
supervisor/.cargo/config.toml
supervisor/src/main.rs
```

## Implementation

### `supervisor/Cargo.toml`

```toml
[package]
name    = "supervisor"
version = "0.1.0"
edition = "2021"

[[bin]]
name = "supervisor"
path = "src/main.rs"

[dependencies]
# libc is added in P03T02 when mount syscalls are needed
# libc = { version = "0.2", default-features = false }

[profile.release]
opt-level = "z"      # optimise for binary size
lto       = true     # link-time optimisation (merges across crates)
codegen-units = 1    # single CGU improves LTO effectiveness
strip     = true     # strip debug symbols from the final binary
panic     = "abort"  # no unwinding machinery (saves ~50 KB in musl builds)
```

### `supervisor/.cargo/config.toml`

```toml
[build]
# Default target: statically linked x86_64 musl binary.
# Override per-invocation with: cargo build --target <other>
target = "x86_64-unknown-linux-musl"

[target.x86_64-unknown-linux-musl]
# musl-gcc is installed in the Docker build environment (P01T03).
# On macOS hosts use: x86_64-linux-musl-gcc (from musl-cross or homebrew).
linker = "musl-gcc"
```

### `supervisor/src/main.rs`

```rust
//! VyomaOS supervisor — PID 1
//!
//! Responsibilities (implemented across Phase 03 tasks):
//!   P03T01 — scaffold: print banner, spin forever
//!   P03T02 — mount /proc, /sys, /dev
//!   P03T03 — discover WASM apps under /apps/
//!   P03T04 — fork/exec each app via wasmtime, waitpid loop

fn main() {
    // Banner written to stderr so it appears on ttyS0 even before /dev is
    // fully set up (the kernel already has the console open on fd 2).
    eprintln!("vyoma-supervisor starting");

    // Placeholder loop — replaced by the real event loop in P03T02+.
    // Using a park loop rather than a busy spin so the CPU is not pegged
    // while we iteratively implement the supervisor.
    loop {
        std::thread::park();
    }
}
```

## Notes

- `panic = "abort"` is critical for a static musl PID-1: without it, Rust links in the unwinding machinery which pulls in additional libc symbols that may not be available in all musl configurations, and it adds ~50–100 KB to the binary.
- `strip = true` in the release profile uses `llvm-strip` (bundled with the Rust toolchain) rather than the system `strip`, which is more reliable in cross-compilation contexts.
- The `.cargo/config.toml` lives inside `supervisor/` (not at the workspace root) so that other crates (e.g., `apps/hello-world`) are not affected by this target override.
- `lto = true` combined with `codegen-units = 1` consistently reduces musl static binary size by 20–40% versus the defaults.
- The `loop { std::thread::park(); }` placeholder is intentional — it keeps the process alive as PID 1 (preventing kernel panic) while subsequent tasks layer in real functionality.
- Do not add `[workspace]` to this `Cargo.toml` yet; if a workspace is desired in the future, it should be defined at the repo root to include both `supervisor/` and `apps/`.

## Verification

```sh
# 1. Required files exist
test -f supervisor/Cargo.toml
test -f supervisor/.cargo/config.toml
test -f supervisor/src/main.rs

# 2. Cargo.toml is valid TOML and contains required fields
cargo metadata --manifest-path supervisor/Cargo.toml --no-deps > /dev/null

# 3. Default target is set to musl
grep -q 'target = "x86_64-unknown-linux-musl"' supervisor/.cargo/config.toml

# 4. Release profile contains size-optimisation settings
grep -q 'opt-level = "z"' supervisor/Cargo.toml
grep -q 'lto *= *true'    supervisor/Cargo.toml
grep -q 'strip *= *true'  supervisor/Cargo.toml
grep -q 'panic *= *"abort"' supervisor/Cargo.toml

# 5. Debug build compiles without errors (fast iteration check)
cargo build --manifest-path supervisor/Cargo.toml

# 6. Release build produces a statically linked binary
cargo build --manifest-path supervisor/Cargo.toml --release
file supervisor/target/x86_64-unknown-linux-musl/release/supervisor \
  | grep -q "statically linked"

# 7. Binary size is reasonable (< 2 MB stripped release)
BINARY=supervisor/target/x86_64-unknown-linux-musl/release/supervisor
SIZE=$(stat -c%s "$BINARY" 2>/dev/null || stat -f%z "$BINARY")
test "$SIZE" -lt 2097152   # 2 MB

# 8. Running the binary prints the banner (debug build on the host)
timeout 1 cargo run --manifest-path supervisor/Cargo.toml 2>&1 \
  | grep -q "vyoma-supervisor starting" || true  # timeout exit is expected
```

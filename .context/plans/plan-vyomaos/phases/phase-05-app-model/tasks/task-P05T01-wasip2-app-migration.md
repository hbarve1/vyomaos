# P05T01 — wasip2-app-migration

## Phase

Phase 05 — App Model

## Goal

Migrate all three existing apps (hello-world, calculator, factorial) from `wasm32-unknown-unknown` + `extern "C"` exports compiled as `cdylib` to `wasm32-wasip2` binaries with a standard `fn main()` entry point so that Wasmtime can execute them directly via its WASI runtime without a host-side JS glue layer.

## File to create / modify

```
apps/hello-world/Cargo.toml
apps/hello-world/src/main.rs           (rename from src/lib.rs)
apps/hello-world/.cargo/config.toml   (new)

apps/calculator/Cargo.toml
apps/calculator/src/main.rs            (rename from src/lib.rs)
apps/calculator/.cargo/config.toml    (new)

apps/factorial/Cargo.toml
apps/factorial/src/main.rs             (rename from src/lib.rs)
apps/factorial/.cargo/config.toml     (new)
```

## Implementation

### `.cargo/config.toml` (identical for all three apps)

```toml
[build]
target = "wasm32-wasip2"
```

This file lives inside each app directory so `cargo build` inside any of the three directories automatically targets `wasm32-wasip2` without requiring a workspace-level override or a CLI flag.

---

### `apps/hello-world/Cargo.toml`

```toml
[package]
name = "hello-world"
version = "0.1.0"
edition = "2021"

[[bin]]
name = "hello-world"
path = "src/main.rs"

[dependencies]
```

Key change: `[lib]` with `crate-type = ["cdylib"]` is replaced by `[[bin]]`. The binary crate type is the correct entry point for a WASI component that Wasmtime runs with `wasmtime run`.

---

### `apps/hello-world/src/main.rs`

```rust
fn main() {
    println!("Hello from VyomaOS!");
    let sum = add(15, 27);
    println!("add(15, 27) = {}", sum);
    let fact = factorial(5);
    println!("factorial(5) = {}", fact);
}

fn add(a: i32, b: i32) -> i32 {
    a + b
}

fn factorial(n: i32) -> i32 {
    if n <= 1 { 1 } else { n * factorial(n - 1) }
}
```

The `#[no_mangle]` / `extern "C"` exports and the raw `*mut c_char` string pointer dance are removed entirely; WASI stdio (`println!`) replaces the CString-returning `hello()` export.

---

### `apps/calculator/Cargo.toml`

```toml
[package]
name = "calculator"
version = "0.1.0"
edition = "2021"

[[bin]]
name = "calculator"
path = "src/main.rs"

[dependencies]
```

---

### `apps/calculator/src/main.rs`

```rust
fn main() {
    println!("Calculator running on VyomaOS");
    println!("add(25.5, 14.3)      = {:.2}", add(25.5, 14.3));
    println!("subtract(100.0, 35.7) = {:.2}", subtract(100.0, 35.7));
    println!("multiply(7.5, 8.0)    = {:.2}", multiply(7.5, 8.0));
    println!("divide(144.0, 12.0)   = {:.2}", divide(144.0, 12.0));
    println!("divide(10.0, 0.0)     = {} (NaN expected)", divide(10.0, 0.0));
    println!("power(2.0, 8.0)       = {:.0}", power(2.0, 8.0));
    println!("sqrt(144.0)           = {:.2}", sqrt_val(144.0));
}

fn add(a: f64, b: f64) -> f64 { a + b }
fn subtract(a: f64, b: f64) -> f64 { a - b }
fn multiply(a: f64, b: f64) -> f64 { a * b }
fn divide(a: f64, b: f64) -> f64 {
    if b != 0.0 { a / b } else { f64::NAN }
}
fn power(base: f64, exp: f64) -> f64 { base.powf(exp) }
fn sqrt_val(x: f64) -> f64 { x.sqrt() }
```

The `sin`, `cos`, `log` symbol names that clashed with `std` intrinsics are no longer exported as bare `#[no_mangle]` functions; the logic is kept as private helpers.

---

### `apps/factorial/Cargo.toml`

```toml
[package]
name = "factorial"
version = "0.1.0"
edition = "2021"

[[bin]]
name = "factorial"
path = "src/main.rs"

[dependencies]
```

---

### `apps/factorial/src/main.rs`

```rust
fn main() {
    println!("Factorial demo on VyomaOS");
    for n in [0, 1, 5, 7, 10] {
        println!("factorial({n})            = {}", factorial(n));
        println!("factorial_iterative({n}) = {}", factorial_iterative(n));
    }
    println!("double_factorial(5)     = {}", double_factorial(5));
    println!("rising_factorial(3, 4)  = {}", rising_factorial(3, 4));
    println!("combinations(5, 2)      = {}", combinations(5, 2));

    let sum = factorial(5)
        + factorial_iterative(5)
        + double_factorial(5)
        + rising_factorial(3, 4)
        + combinations(5, 2);
    assert_eq!(sum, 625, "verification sum mismatch");
    println!("All assertions passed.");
}

fn factorial(n: i32) -> i32 {
    if n <= 1 { 1 } else { n * factorial(n - 1) }
}

fn factorial_iterative(n: i32) -> i32 {
    (2..=n).product()
}

fn double_factorial(n: i32) -> i32 {
    if n <= 1 { 1 } else { n * double_factorial(n - 2) }
}

fn rising_factorial(n: i32, k: i32) -> i32 {
    (0..k).map(|i| n + i).product()
}

fn combinations(n: i32, k: i32) -> i32 {
    if k > n || k < 0 { return 0; }
    let k = k.min(n - k);
    (0..k).fold(1, |acc, i| acc * (n - i) / (i + 1))
}
```

## Notes

- The `wasm32-wasip2` target requires the nightly toolchain channel **or** a stable Rust >= 1.78 that ships the target in `rustup`. Add the target with: `rustup target add wasm32-wasip2`.
- `wasm32-wasip2` generates a WASI Preview 2 component module by default when compiled as a `[[bin]]` crate, which is what Wasmtime >= 18 expects for `wasmtime run --wasm component-model`.
- The old `src/lib.rs` files must be deleted (or renamed) after creating `src/main.rs`; leaving both present causes a compile error because the crate now declares a `[[bin]]` target without a matching `[lib]`.
- The `.cargo/config.toml` target override is scoped to the directory, so the workspace root (if one exists later) is not affected.
- Phase 04 established `wasmtime` as the runtime; the output `.wasm` files from this task are the inputs to the supervisor's `wasmtime run` invocations.

## Verification

```sh
# 0. Ensure target is installed
rustup target add wasm32-wasip2

# 1. Build all three apps
(cd apps/hello-world && cargo build --release 2>&1 | tail -3)
(cd apps/calculator  && cargo build --release 2>&1 | tail -3)
(cd apps/factorial   && cargo build --release 2>&1 | tail -3)

# 2. Confirm .wasm files were produced in the correct target directory
ls -lh apps/hello-world/target/wasm32-wasip2/release/hello-world.wasm
ls -lh apps/calculator/target/wasm32-wasip2/release/calculator.wasm
ls -lh apps/factorial/target/wasm32-wasip2/release/factorial.wasm

# 3. Confirm the binaries are valid WASM (magic bytes \0asm)
for f in \
    apps/hello-world/target/wasm32-wasip2/release/hello-world.wasm \
    apps/calculator/target/wasm32-wasip2/release/calculator.wasm \
    apps/factorial/target/wasm32-wasip2/release/factorial.wasm; do
  xxd "$f" | head -1 | grep -q "0000 6173 6d" && echo "$f: valid WASM"
done

# 4. Run each app with wasmtime and confirm output
wasmtime run apps/hello-world/target/wasm32-wasip2/release/hello-world.wasm \
  | grep -q "Hello from VyomaOS" && echo "hello-world: PASS"

wasmtime run apps/calculator/target/wasm32-wasip2/release/calculator.wasm \
  | grep -q "256" && echo "calculator: PASS"

wasmtime run apps/factorial/target/wasm32-wasip2/release/factorial.wasm \
  | grep -q "All assertions passed" && echo "factorial: PASS"

# 5. Confirm no cdylib artifact remains
! ls apps/hello-world/target/wasm32-wasip2/release/*.so 2>/dev/null && echo "no cdylib: OK"
```

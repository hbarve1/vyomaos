# P06T03 — wit-interface-definitions

## Phase

Phase 06 — Multi-App & IPC

## Goal

Create a `wit/` directory at the repository root containing a shared `math.wit` WIT interface definition that declares `add` and `multiply` operations, and a `apps/calculator/wit/world.wit` that exports that interface as the calculator component's world, establishing the typed contract for inter-component communication.

## File to create / modify

```
wit/math.wit                    (new — shared interface registry)
apps/calculator/wit/world.wit   (new — calculator component world)
apps/calculator/Cargo.toml      (modify — add wit-bindgen dependency)
apps/calculator/src/main.rs     (modify — implement the exported math interface)
```

## Implementation

### `wit/math.wit`

```wit
package vyoma:math@0.1.0;

/// Arithmetic operations provided by a calculator component.
interface math {
    /// Add two signed 32-bit integers.
    add: func(a: s32, b: s32) -> s32;

    /// Multiply two signed 32-bit integers.
    multiply: func(a: s32, b: s32) -> s32;
}
```

This file lives in the repository root `wit/` directory so that any component in the workspace can import or export the `vyoma:math` package without duplicating the interface text.

---

### `apps/calculator/wit/world.wit`

```wit
package vyoma:calculator@0.1.0;

use vyoma:math/math@0.1.0;

/// The calculator world exports the math interface so that
/// consumer components can import and call it via component linking.
world calculator {
    export math;
}
```

The `use` statement references the shared package defined in `wit/math.wit`. The `wac` or `wasm-tools component link` tooling will resolve this reference by scanning the include path that contains both WIT directories.

---

### `apps/calculator/Cargo.toml`

```toml
[package]
name    = "calculator"
version = "0.1.0"
edition = "2021"

[[bin]]
name = "calculator"
path = "src/main.rs"

[dependencies]
wit-bindgen = { version = "0.36", default-features = false, features = ["realloc"] }

[package.metadata.component]
package = "vyoma:calculator"
```

The `wit-bindgen` crate generates Rust bindings from the WIT world definition at build time. The `[package.metadata.component]` key is read by `cargo component` (from the `cargo-component` tool) to identify the component package name.

---

### `apps/calculator/src/main.rs` — implementing the WIT export

```rust
// Generated bindings from apps/calculator/wit/world.wit
// cargo-component generates this module under the `bindings` module.
mod bindings;

use bindings::exports::vyoma::math::math::Guest;

struct Calculator;

impl Guest for Calculator {
    fn add(a: i32, b: i32) -> i32 {
        a + b
    }

    fn multiply(a: i32, b: i32) -> i32 {
        a * b
    }
}

// Register the implementation with the generated component glue
bindings::export!(Calculator with_types_in bindings);

fn main() {
    // Standard WASI entry point — runs when the component is invoked directly
    // rather than via component linking.
    println!("Calculator component ready.");
    println!("add(3, 4)      = {}", Calculator::add(3, 4));       // 7
    println!("multiply(6, 7) = {}", Calculator::multiply(6, 7));  // 42
}
```

**Note:** The `main()` function is only reachable when the binary is executed via `wasmtime run` in standalone mode. When linked as a library component (P06T04), the `add` and `multiply` exports are called directly through the component model ABI.

---

### Build process for WIT components

`cargo component` (from `cargo install cargo-component`) is the recommended tool. It wraps `cargo build` and automatically:

1. Runs `wit-bindgen` against `apps/calculator/wit/world.wit`.
2. Compiles to `wasm32-wasip2`.
3. Wraps the core module in a WASM component envelope.

```sh
# In apps/calculator/
cargo component build --release
# Output: target/wasm32-wasip2/release/calculator.wasm (a component)
```

Alternatively, build manually without `cargo-component`:

```sh
cargo build --target wasm32-wasip2 --release
wasm-tools component new \
    target/wasm32-wasip2/release/calculator.wasm \
    --adapt wasi_snapshot_preview1.reactor.wasm \
    -o target/wasm32-wasip2/release/calculator-component.wasm
```

---

### WIT resolution across directories

When linking (P06T04), pass both WIT directories to the tooling:

```sh
wasm-tools component link \
  --wit wit/ \
  --wit apps/calculator/wit/ \
  consumer.wasm calculator.wasm \
  -o linked.wasm
```

## Notes

- WIT (WebAssembly Interface Types) is the IDL for the WASM Component Model (formerly WASI Preview 2). It replaces ad-hoc `#[no_mangle]` ABI with a typed, versioned contract.
- `s32` in WIT maps to `i32` in Rust. The WIT type system uses signed/unsigned prefixes (`s32`, `u32`, `s64`, `u64`) that `wit-bindgen` translates to the appropriate Rust integer types.
- The `vyoma:math@0.1.0` package name uses the standard WIT namespace syntax `<namespace>:<package>@<semver>`. Namespacing under `vyoma:` prevents collision with other WIT packages in the WASI ecosystem.
- `apps/calculator/wit/` is separate from the root `wit/` because the `world.wit` is specific to the calculator binary (it declares what *this component* exports), while `math.wit` is the shared interface definition.
- The `cargo-component` tool is not the same as `cargo build --target wasm32-wasip2`. `cargo-component` additionally wraps the output in the WASM Component Model envelope. Both approaches produce valid `.wasm` files but only the component-wrapped one can be linked via the component model (P06T04).
- Cross-reference: P06T04 consumes the component produced here to demonstrate runtime linking.

## Verification

```sh
# 1. Confirm WIT files are present and well-formed
test -f wit/math.wit                  && echo "wit/math.wit: present"
test -f apps/calculator/wit/world.wit && echo "apps/calculator/wit/world.wit: present"

# 2. Parse WIT files with wasm-tools (cargo install wasm-tools)
wasm-tools component wit wit/math.wit                  2>&1 | grep -v "^$" | head -5
wasm-tools component wit apps/calculator/wit/world.wit 2>&1 | grep -v "^$" | head -5

# 3. Confirm package declarations
grep -q "package vyoma:math"       wit/math.wit                  && echo "math package: OK"
grep -q "package vyoma:calculator" apps/calculator/wit/world.wit && echo "calculator package: OK"

# 4. Confirm interface and world names
grep -q "interface math"  wit/math.wit                  && echo "math interface: defined"
grep -q "world calculator" apps/calculator/wit/world.wit && echo "calculator world: defined"
grep -q "export math"      apps/calculator/wit/world.wit && echo "math export: present"

# 5. Confirm function signatures in math.wit
grep -q "add: func"      wit/math.wit && echo "add func: defined"
grep -q "multiply: func" wit/math.wit && echo "multiply func: defined"

# 6. Build the calculator as a component (requires cargo-component)
which cargo-component && (
  cd apps/calculator && cargo component build --release 2>&1 | tail -5
  ls -lh target/wasm32-wasip2/release/calculator.wasm && echo "calculator component: built"
) || echo "cargo-component not installed — skipping component build check"

# 7. Verify the WIT can be extracted from the built component
which wasm-tools && test -f apps/calculator/target/wasm32-wasip2/release/calculator.wasm && \
  wasm-tools component wit apps/calculator/target/wasm32-wasip2/release/calculator.wasm \
  | grep -q "add" && echo "WIT embedded in component: verified"
```

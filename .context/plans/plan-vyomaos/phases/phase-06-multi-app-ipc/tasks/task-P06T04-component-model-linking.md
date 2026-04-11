# P06T04 — component-model-linking

## Phase

Phase 06 — Multi-App & IPC

## Goal

Add `wasmtime` (with the `component-model` feature) and `wit-bindgen` to the supervisor's dependencies, then implement a `supervisor/src/component.rs` module that demonstrates runtime component-model linking by instantiating the calculator component and calling its exported `math::add` and `math::multiply` functions from Rust host code, proving that inter-component calls work end-to-end.

## File to create / modify

```
supervisor/Cargo.toml            (modify — add wasmtime + wit-bindgen)
supervisor/src/component.rs      (new)
supervisor/src/main.rs           (modify — call run_component_demo())
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
serde        = { version = "1",  features = ["derive"] }
toml         = "0.8"
wasmtime     = { version = "28", features = ["component-model"] }
wasmtime-wasi = "28"
wit-bindgen  = { version = "0.36", default-features = false }
anyhow       = "1"
```

`wasmtime = { version = "28", features = ["component-model"] }` enables the `wasmtime::component` API. `anyhow` is used for ergonomic `?`-based error propagation in the component setup code.

---

### `supervisor/src/component.rs`

This module contains all component-model logic, keeping it separate from the process-spawning logic in `main.rs`.

```rust
//! component.rs — WASM Component Model linking demo
//!
//! Instantiates the calculator component at runtime and calls
//! its exported `vyoma:math/math` interface functions.

use anyhow::{Context, Result};
use wasmtime::{
    component::{bindgen, Component, Linker},
    Config, Engine, Store,
};
use wasmtime_wasi::{WasiCtx, WasiCtxBuilder, WasiView, ResourceTable};

// ── Generated host-side bindings ──────────────────────────────────────────
//
// The macro reads apps/calculator/wit/world.wit at compile time and generates
// a `Calculator` struct with typed methods for each export.
bindgen!({
    world: "calculator",
    path:  "../../apps/calculator/wit",
});

// ── WASI host state ───────────────────────────────────────────────────────

struct HostState {
    wasi:  WasiCtx,
    table: ResourceTable,
}

impl WasiView for HostState {
    fn ctx(&mut self)   -> &mut WasiCtx      { &mut self.wasi  }
    fn table(&mut self) -> &mut ResourceTable { &mut self.table }
}

// ── Public entry point ────────────────────────────────────────────────────

/// Load the calculator component from `wasm_path`, instantiate it inside
/// the Wasmtime component-model engine, and call `math::add` and
/// `math::multiply` to verify the link is working.
pub fn run_component_demo(wasm_path: &str) -> Result<()> {
    // 1. Configure Wasmtime with component-model support
    let mut config = Config::new();
    config.wasm_component_model(true);
    let engine = Engine::new(&config)?;

    // 2. Load and compile the component
    let component = Component::from_file(&engine, wasm_path)
        .with_context(|| format!("failed to load component: {}", wasm_path))?;

    // 3. Set up a linker and add WASI imports
    let mut linker: Linker<HostState> = Linker::new(&engine);
    wasmtime_wasi::add_to_linker_sync(&mut linker)?;

    // 4. Build WASI context
    let wasi = WasiCtxBuilder::new()
        .inherit_stdio()
        .build();
    let state = HostState { wasi, table: ResourceTable::new() };
    let mut store = Store::new(&engine, state);

    // 5. Instantiate the component
    let (calculator, _instance) =
        Calculator::instantiate(&mut store, &component, &linker)
            .context("failed to instantiate calculator component")?;

    // 6. Call the exported math interface
    let math = calculator.vyoma_math_math();

    let sum = math.call_add(&mut store, 3, 4)
        .context("math::add call failed")?;
    let product = math.call_multiply(&mut store, 6, 7)
        .context("math::multiply call failed")?;

    eprintln!("[component] math::add(3, 4)      = {}", sum);     // 7
    eprintln!("[component] math::multiply(6, 7) = {}", product); // 42

    assert_eq!(sum,     7,  "add result mismatch");
    assert_eq!(product, 42, "multiply result mismatch");

    eprintln!("[component] Component linking demo: PASSED");
    Ok(())
}
```

---

### `supervisor/src/main.rs` — add demo call

Add the module declaration and a call to `run_component_demo` in `main()`, guarded so it only runs when the component `.wasm` file is present (allowing the supervisor to still boot on machines where the component has not yet been built).

```rust
mod component;

// In main(), after mount_pseudo_filesystems() and before reading boot.toml:
const CALCULATOR_COMPONENT: &str =
    "/apps/calculator/target/wasm32-wasip2/release/calculator.wasm";

if std::path::Path::new(CALCULATOR_COMPONENT).exists() {
    if let Err(e) = component::run_component_demo(CALCULATOR_COMPONENT) {
        eprintln!("[supervisor] WARN: component demo failed: {:#}", e);
    }
} else {
    eprintln!("[supervisor] INFO: calculator component not found, skipping demo");
}
```

---

### How the `bindgen!` macro path works

`bindgen!` is a proc-macro that reads `.wit` files at Rust *compile* time from the `path:` argument. The path is relative to the `supervisor/src/` directory where `component.rs` lives, so `../../apps/calculator/wit` resolves to the repository root's `apps/calculator/wit/`.

When the supervisor is compiled inside a Docker container or CI environment, the `apps/` directory must be present at the same relative location. If the paths differ, override with the absolute path or use a build-time `WIT_PATH` environment variable.

---

### Alternative: manual linking without bindgen

For environments where the proc-macro path resolution is inconvenient, the component can be called dynamically using `wasmtime::component::Val`:

```rust
// Dynamic call (no bindgen required):
let instance = linker.instantiate(&mut store, &component)?;
let func = instance
    .get_export(&mut store, None, "vyoma:math/math#add")
    .and_then(|e| e.into_func())
    .context("export not found")?;

let mut results = [wasmtime::component::Val::S32(0)];
func.call(
    &mut store,
    &[wasmtime::component::Val::S32(3), wasmtime::component::Val::S32(4)],
    &mut results,
)?;
eprintln!("dynamic add result: {:?}", results[0]);
```

This approach is useful for ad-hoc introspection but loses type safety.

## Notes

- `wasmtime = "28"` is the latest stable release family as of April 2026 that ships stable `component-model` support; pin to a specific patch version in `Cargo.lock` for reproducible builds.
- `wasmtime-wasi = "28"` must be version-locked to the same major as `wasmtime`; mismatches cause linker errors about conflicting `WasiView` trait bounds.
- The `bindgen!` macro generates the `Calculator` struct, the `VyomaMathMath` accessor struct, and `call_add` / `call_multiply` methods. The exact generated method names follow the pattern `call_<wit-function-name>`.
- `anyhow` is used throughout for ergonomic error propagation; it is not a required dependency — replace with explicit `match` chains if binary size is a concern.
- The `run_component_demo` function is a *demonstration*, not part of the production boot path. The real use case (having one WASM app call another) would use the same `Linker` + component instantiation API but compose multiple components in a single `wasmtime::Store`.
- Cross-reference: P06T03 defines `wit/math.wit` and `apps/calculator/wit/world.wit` that this task consumes via `bindgen!`.

## Verification

```sh
# 1. Confirm Cargo.toml has wasmtime with component-model feature
grep -q 'component-model' supervisor/Cargo.toml && echo "component-model feature: present"
grep -q 'wasmtime'        supervisor/Cargo.toml && echo "wasmtime dep: present"
grep -q 'wit-bindgen'     supervisor/Cargo.toml && echo "wit-bindgen dep: present"
grep -q 'anyhow'          supervisor/Cargo.toml && echo "anyhow dep: present"

# 2. Build the supervisor (will pull wasmtime crate — may take a few minutes)
(cd supervisor && cargo build 2>&1 | tail -10)
test -x supervisor/target/debug/supervisor && echo "supervisor: built OK"

# 3. Confirm component.rs was created
test -f supervisor/src/component.rs && echo "component.rs: present"

# 4. Confirm bindgen! macro invocation and run_component_demo are present
grep -q 'bindgen!'             supervisor/src/component.rs && echo "bindgen! macro: present"
grep -q 'run_component_demo'   supervisor/src/component.rs && echo "run_component_demo: defined"
grep -q 'wasm_component_model' supervisor/src/component.rs && echo "component-model config: present"

# 5. Build calculator as a component (requires cargo-component)
which cargo-component && (
  cd apps/calculator
  cargo component build --release 2>&1 | tail -5
  test -f target/wasm32-wasip2/release/calculator.wasm && echo "calculator component: present"
) || echo "cargo-component not installed — skipping"

# 6. Run the full demo end-to-end (if calculator component exists)
test -f apps/calculator/target/wasm32-wasip2/release/calculator.wasm && (
  # Run supervisor in demo-only mode (not on real hardware — just the component code)
  # Redirect BOOT_CONFIG_PATH to a dummy file to skip boot sequence
  echo '# empty' > /tmp/empty_boot.toml
  BOOT_CONFIG_OVERRIDE=/tmp/empty_boot.toml supervisor/target/debug/supervisor 2>&1 \
    | grep -q "Component linking demo: PASSED" && echo "component demo: PASSED"
) || echo "calculator component not found — build P06T03 first"

# 7. Verify the math results appear in supervisor output
supervisor/target/debug/supervisor 2>&1 | grep "math::add(3, 4)" | grep -q "= 7" \
  && echo "add result verified"
supervisor/target/debug/supervisor 2>&1 | grep "math::multiply(6, 7)" | grep -q "= 42" \
  && echo "multiply result verified"
```

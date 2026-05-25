// T020: Failing tests for cross-arch binary parity.
//
// Verifies that the same WASM binary bytes produce the same runtime behavior
// on different RuntimeConfig instances (simulating different arch targets).
//
// "Same WASM binary, same output" is modelled here as:
//   - Both configs accept the same bytes via `instantiate`
//   - Both return the same exit code for a known binary
//   - Memory usage is reported (not NaN/error)
//
// The test uses the WasmtimeAdapter with /bin/true as a stub runtime,
// mirroring runtime_adapter.rs.  Once wasm3 adapter exists (T055), the
// same test is extended in runtime_parity.rs.

use supervisor::runtime::{Engine, ExecutionMode, ExitCode, RuntimeConfig, WasmRuntime};
use supervisor::manifest::Capabilities;
use supervisor::runtime::wasmtime::WasmtimeAdapter;

// ── helpers ───────────────────────────────────────────────────────────────────

/// Minimal valid WASM module (same bytes as in runtime_adapter.rs).
fn minimal_wasm() -> Vec<u8> {
    // (module (func (export "_start")))
    vec![
        0x00, 0x61, 0x73, 0x6d, // magic: \0asm
        0x01, 0x00, 0x00, 0x00, // version: 1
        0x01, 0x04, 0x01, 0x60, 0x00, 0x00, // type section
        0x03, 0x02, 0x01, 0x00, // function section
        0x07, 0x0a, 0x01, 0x06, 0x5f, 0x73, 0x74, 0x61, 0x72, 0x74, 0x00, 0x00, // export
        0x0a, 0x04, 0x01, 0x02, 0x00, 0x0b, // code section
    ]
}

/// Build a RuntimeConfig for the "x86" target (Wasmtime JIT).
fn x86_config() -> RuntimeConfig {
    RuntimeConfig {
        engine: Engine::Wasmtime,
        mode: ExecutionMode::Jit,
        max_memory_pages: 0,
        fuel_limit: 0,
        wasi_imports: vec![],
    }
}

/// Build a RuntimeConfig for the "arm" target (Wasmtime interpreter mode).
fn arm_config() -> RuntimeConfig {
    RuntimeConfig {
        engine: Engine::Wasmtime,
        mode: ExecutionMode::Interpreter,
        max_memory_pages: 0,
        fuel_limit: 0,
        wasi_imports: vec![],
    }
}

// ── T020-1: same WASM bytes produce Ok on both runtime configs ────────────────

#[test]
fn test_same_wasm_instantiates_on_x86_config() {
    let adapter = WasmtimeAdapter::new(x86_config());
    let caps = Capabilities::default();
    let result = adapter.instantiate("parity-app", &minimal_wasm(), &caps);
    assert!(result.is_ok(), "x86 config must accept minimal WASM bytes");
}

#[test]
fn test_same_wasm_instantiates_on_arm_config() {
    let adapter = WasmtimeAdapter::new(arm_config());
    let caps = Capabilities::default();
    let result = adapter.instantiate("parity-app", &minimal_wasm(), &caps);
    assert!(result.is_ok(), "arm config must accept identical WASM bytes");
}

// ── T020-2: both configs produce the same exit code for the same module ───────

#[test]
fn test_parity_exit_code_x86() {
    let adapter = WasmtimeAdapter::new(x86_config()).with_bin("/bin/true");
    let caps = Capabilities::default();
    let mut inst = adapter.instantiate("parity-app", &minimal_wasm(), &caps)
        .expect("instantiate ok");
    let exit = adapter.execute(inst.as_mut()).expect("execute ok");
    assert_eq!(exit, ExitCode::Success(0), "x86 target: /bin/true exits 0");
}

#[test]
fn test_parity_exit_code_arm() {
    // Uses same stub binary so exit code must match T020-2.
    let adapter = WasmtimeAdapter::new(arm_config()).with_bin("/bin/true");
    let caps = Capabilities::default();
    let mut inst = adapter.instantiate("parity-app", &minimal_wasm(), &caps)
        .expect("instantiate ok");
    let exit = adapter.execute(inst.as_mut()).expect("execute ok");
    assert_eq!(exit, ExitCode::Success(0), "arm target: /bin/true exits 0");
}

// ── T020-3: identical WASM bytes produce identical instance names ─────────────

#[test]
fn test_parity_instance_names_identical() {
    let wasm = minimal_wasm();
    let caps = Capabilities::default();

    let adapter_x86 = WasmtimeAdapter::new(x86_config());
    let inst_x86 = adapter_x86.instantiate("cross-app", &wasm, &caps)
        .expect("x86 instantiate ok");

    let adapter_arm = WasmtimeAdapter::new(arm_config());
    let inst_arm = adapter_arm.instantiate("cross-app", &wasm, &caps)
        .expect("arm instantiate ok");

    assert_eq!(
        inst_x86.name(), inst_arm.name(),
        "instance names must match across runtime configs"
    );
}

// ── T020-4: memory_usage returns 0 before execution on both configs ───────────

#[test]
fn test_parity_memory_usage_before_execute() {
    let wasm = minimal_wasm();
    let caps = Capabilities::default();

    let adapter_x86 = WasmtimeAdapter::new(x86_config());
    let inst_x86 = adapter_x86.instantiate("parity-app", &wasm, &caps)
        .expect("x86 instantiate ok");

    let adapter_arm = WasmtimeAdapter::new(arm_config());
    let inst_arm = adapter_arm.instantiate("parity-app", &wasm, &caps)
        .expect("arm instantiate ok");

    // Both should return the same (0) before any execution.
    assert_eq!(
        adapter_x86.memory_usage(inst_x86.as_ref()),
        adapter_arm.memory_usage(inst_arm.as_ref()),
        "memory_usage must be identical across configs before execution"
    );
}

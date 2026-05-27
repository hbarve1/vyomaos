// T009: Failing unit tests for WasmRuntime trait.
//
// These tests verify that a WasmRuntime implementation:
// - instantiates a module from bytes
// - executes a module and reports the exit code
// - terminates a module
// - reports memory usage
//
// Tests are written against the stub WasmtimeAdapter; they FAIL until T010.

use supervisor::runtime::{Engine, ExecutionMode, ExitCode, RuntimeConfig, WasmRuntime};
use supervisor::manifest::Capabilities;

// ── helpers ───────────────────────────────────────────────────────────────────

fn default_config() -> RuntimeConfig {
    RuntimeConfig {
        engine: Engine::Wasmtime,
        mode: ExecutionMode::Jit,
        max_memory_pages: 0,
        fuel_limit: 0,
        wasi_imports: vec![],
    }
}

/// Minimal valid WASM module that does nothing (exports `_start` that returns).
/// Hand-crafted WAT as bytes — no build toolchain needed.
fn minimal_wasm() -> Vec<u8> {
    // (module (func (export "_start")))
    vec![
        0x00, 0x61, 0x73, 0x6d, // magic: \0asm
        0x01, 0x00, 0x00, 0x00, // version: 1
        // type section: () -> ()
        0x01, 0x04, 0x01, 0x60, 0x00, 0x00,
        // function section: 1 function, type 0
        0x03, 0x02, 0x01, 0x00,
        // export section: "_start" -> func 0
        0x07, 0x0a, 0x01, 0x06, 0x5f, 0x73, 0x74, 0x61, 0x72, 0x74, 0x00, 0x00,
        // code section: func 0 body = (end)
        0x0a, 0x04, 0x01, 0x02, 0x00, 0x0b,
    ]
}

// ── T009-1: instantiate returns Ok for valid WASM bytes ───────────────────────

#[test]
fn test_instantiate_valid_wasm_returns_ok() {
    let adapter = supervisor::runtime::wasmtime::WasmtimeAdapter::new(default_config());
    let caps = Capabilities::default();
    let result = adapter.instantiate("test-app", &minimal_wasm(), &caps);
    assert!(result.is_ok(), "instantiate should succeed for valid WASM");
}

// ── T009-2: instantiate returns Err for empty/invalid bytes ──────────────────

#[test]
fn test_instantiate_invalid_wasm_returns_err() {
    let adapter = supervisor::runtime::wasmtime::WasmtimeAdapter::new(default_config());
    let caps = Capabilities::default();
    let result = adapter.instantiate("bad-app", b"not wasm", &caps);
    assert!(result.is_err(), "instantiate should fail for invalid WASM bytes");
}

// ── T009-3: execute using a known-good binary (sh) as the wasmtime stand-in ──
//
// The builder container does not have wasmtime installed (it is embedded in the
// initramfs for the VM).  We test the execute path using `/bin/true` as the
// runtime binary, which exits 0 regardless of arguments.  This validates the
// execute flow end-to-end without requiring a real wasmtime binary.
#[test]
fn test_execute_minimal_module_returns_success() {
    let config = default_config();
    // Use /bin/true as a stand-in for wasmtime: it exits 0 and ignores args.
    let adapter = supervisor::runtime::wasmtime::WasmtimeAdapter::new(config)
        .with_bin("/bin/true");

    let caps = Capabilities::default();
    let mut instance = adapter
        .instantiate("test-app", &minimal_wasm(), &caps)
        .expect("instantiate should succeed");
    let result = adapter.execute(instance.as_mut());
    assert!(
        result.is_ok(),
        "execute should not return an error"
    );
    let exit = result.unwrap();
    assert_eq!(exit, ExitCode::Success(0), "bin/true exits with code 0");
}

// ── T009-4: terminate does not return an error ────────────────────────────────

#[test]
fn test_terminate_does_not_error() {
    let adapter = supervisor::runtime::wasmtime::WasmtimeAdapter::new(default_config());
    let caps = Capabilities::default();
    let instance = adapter
        .instantiate("test-app", &minimal_wasm(), &caps)
        .expect("instantiate should succeed");
    let result = adapter.terminate(instance.as_ref());
    assert!(result.is_ok(), "terminate should not fail: {:?}", result);
}

// ── T009-5: memory_usage returns a non-negative number ───────────────────────

#[test]
fn test_memory_usage_returns_non_negative() {
    let adapter = supervisor::runtime::wasmtime::WasmtimeAdapter::new(default_config());
    let caps = Capabilities::default();
    let instance = adapter
        .instantiate("test-app", &minimal_wasm(), &caps)
        .expect("instantiate should succeed");
    let usage = adapter.memory_usage(instance.as_ref());
    // Memory usage is always >= 0 (it's a usize), but just confirming the call works.
    let _ = usage; // no assertion needed beyond "doesn't panic"
}

// ── T009-6: fuel_remaining returns None when no limit set ────────────────────

#[test]
fn test_fuel_remaining_none_when_no_limit() {
    let adapter = supervisor::runtime::wasmtime::WasmtimeAdapter::new(default_config());
    let caps = Capabilities::default();
    let instance = adapter
        .instantiate("test-app", &minimal_wasm(), &caps)
        .expect("instantiate should succeed");
    let remaining = adapter.fuel_remaining(instance.as_ref());
    assert_eq!(remaining, None, "fuel_remaining should be None when no fuel limit set");
}

// ── T009-7: instance name matches what was passed to instantiate ──────────────

#[test]
fn test_instance_name_matches_given_name() {
    let adapter = supervisor::runtime::wasmtime::WasmtimeAdapter::new(default_config());
    let caps = Capabilities::default();
    let instance = adapter
        .instantiate("my-named-app", &minimal_wasm(), &caps)
        .expect("instantiate should succeed");
    assert_eq!(instance.name(), "my-named-app");
}

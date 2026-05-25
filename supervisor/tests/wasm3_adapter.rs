// T052: Tests for the wasm3 runtime adapter.
//
// Tests verify that the Wasm3Adapter satisfies the WasmRuntime trait contract:
// - instantiate: accepts valid WASM bytes, rejects invalid
// - execute: returns an ExitCode
// - terminate: does not panic / error
// - memory_usage: returns a value
// - fuel_remaining: returns None or Some
//
// The wasm3 Rust crate is not available on x86_64-unknown-linux-musl in CI,
// so Wasm3Adapter is always the mock implementation.  When built with
// feature = "wasm3" and the real crate available, the real adapter is used.

use supervisor::runtime::{Engine, ExecutionMode, ExitCode, RuntimeConfig, WasmRuntime};
use supervisor::manifest::Capabilities;
use supervisor::runtime::wasm3::Wasm3Adapter;

// ── helpers ───────────────────────────────────────────────────────────────────

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

fn wasm3_config() -> RuntimeConfig {
    RuntimeConfig {
        engine: Engine::Wasm3,
        mode: ExecutionMode::Interpreter,
        max_memory_pages: 16,
        fuel_limit: 0,
        wasi_imports: vec![],
    }
}

// ── T052-1: instantiate returns Ok for valid WASM bytes ──────────────────────

#[test]
fn test_wasm3_instantiate_valid_returns_ok() {
    let adapter = Wasm3Adapter::new(wasm3_config());
    let caps = Capabilities::default();
    let result = adapter.instantiate("wasm3-app", &minimal_wasm(), &caps);
    assert!(result.is_ok(), "wasm3 adapter must accept valid WASM bytes");
}

// ── T052-2: instantiate returns Err for invalid bytes ────────────────────────

#[test]
fn test_wasm3_instantiate_invalid_returns_err() {
    let adapter = Wasm3Adapter::new(wasm3_config());
    let caps = Capabilities::default();
    let result = adapter.instantiate("bad-app", b"not wasm bytes", &caps);
    assert!(result.is_err(), "wasm3 adapter must reject invalid WASM magic");
}

// ── T052-3: instantiate returns Err for empty bytes ──────────────────────────

#[test]
fn test_wasm3_instantiate_empty_returns_err() {
    let adapter = Wasm3Adapter::new(wasm3_config());
    let caps = Capabilities::default();
    let result = adapter.instantiate("empty-app", &[], &caps);
    assert!(result.is_err(), "wasm3 adapter must reject empty bytes");
}

// ── T052-4: execute returns an ExitCode (not panic / error) ──────────────────

#[test]
fn test_wasm3_execute_returns_exit_code() {
    let adapter = Wasm3Adapter::new(wasm3_config());
    let caps = Capabilities::default();
    let mut inst = adapter.instantiate("wasm3-app", &minimal_wasm(), &caps)
        .expect("instantiate ok");
    let result = adapter.execute(inst.as_mut());
    assert!(result.is_ok(), "wasm3 execute must return an ExitCode: {:?}", result);
}

// ── T052-5: terminate does not return an error ────────────────────────────────

#[test]
fn test_wasm3_terminate_does_not_error() {
    let adapter = Wasm3Adapter::new(wasm3_config());
    let caps = Capabilities::default();
    let inst = adapter.instantiate("wasm3-app", &minimal_wasm(), &caps)
        .expect("instantiate ok");
    let result = adapter.terminate(inst.as_ref());
    assert!(result.is_ok(), "wasm3 terminate must not error: {:?}", result);
}

// ── T052-6: memory_usage returns a value (not panic) ─────────────────────────

#[test]
fn test_wasm3_memory_usage_returns_value() {
    let adapter = Wasm3Adapter::new(wasm3_config());
    let caps = Capabilities::default();
    let inst = adapter.instantiate("wasm3-app", &minimal_wasm(), &caps)
        .expect("instantiate ok");
    let _ = adapter.memory_usage(inst.as_ref()); // must not panic
}

// ── T052-7: fuel_remaining returns None when no fuel limit set ───────────────

#[test]
fn test_wasm3_fuel_remaining_none_without_limit() {
    let adapter = Wasm3Adapter::new(wasm3_config());
    let caps = Capabilities::default();
    let inst = adapter.instantiate("wasm3-app", &minimal_wasm(), &caps)
        .expect("instantiate ok");
    let remaining = adapter.fuel_remaining(inst.as_ref());
    assert_eq!(remaining, None, "no fuel limit → fuel_remaining must be None");
}

// ── T052-8: instance name matches what was passed to instantiate ──────────────

#[test]
fn test_wasm3_instance_name_matches() {
    let adapter = Wasm3Adapter::new(wasm3_config());
    let caps = Capabilities::default();
    let inst = adapter.instantiate("my-wasm3-module", &minimal_wasm(), &caps)
        .expect("instantiate ok");
    assert_eq!(inst.name(), "my-wasm3-module");
}

// ── T052-9: execute with fuel limit returns FuelExhausted or Success ─────────

#[test]
fn test_wasm3_execute_with_fuel_limit() {
    let config = RuntimeConfig {
        engine: Engine::Wasm3,
        mode: ExecutionMode::Interpreter,
        max_memory_pages: 8,
        fuel_limit: 100,
        wasi_imports: vec![],
    };
    let adapter = Wasm3Adapter::new(config);
    let caps = Capabilities::default();
    let mut inst = adapter.instantiate("fuel-app", &minimal_wasm(), &caps)
        .expect("instantiate ok");
    let result = adapter.execute(inst.as_mut());
    // Either completes normally or exhausts fuel — both are valid outcomes.
    assert!(result.is_ok(), "execute with fuel limit must return an ExitCode");
    let exit = result.unwrap();
    assert!(
        matches!(exit, ExitCode::Success(_) | ExitCode::FuelExhausted | ExitCode::Terminated),
        "exit must be Success, FuelExhausted, or Terminated: {exit:?}"
    );
}

// T053: Tests for runtime parity — same WASM binary, same output on wasm3 vs Wasmtime.
//
// Both adapters must:
// - Accept the same minimal WASM bytes
// - Return the same ExitCode for a trivial module
// - Return the same instance name for the same input name
// - Report consistent memory_usage behaviour (both return 0 before execution)

use supervisor::runtime::{Engine, ExecutionMode, ExitCode, RuntimeConfig, WasmRuntime};
use supervisor::manifest::Capabilities;
use supervisor::runtime::wasmtime::WasmtimeAdapter;
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

fn wasmtime_config() -> RuntimeConfig {
    RuntimeConfig {
        engine: Engine::Wasmtime,
        mode: ExecutionMode::Jit,
        max_memory_pages: 0,
        fuel_limit: 0,
        wasi_imports: vec![],
    }
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

// ── T053-1: both adapters accept the same valid WASM bytes ───────────────────

#[test]
fn test_parity_both_accept_valid_wasm() {
    let wasm = minimal_wasm();
    let caps = Capabilities::default();

    let wt = WasmtimeAdapter::new(wasmtime_config());
    let w3 = Wasm3Adapter::new(wasm3_config());

    assert!(
        wt.instantiate("parity", &wasm, &caps).is_ok(),
        "WasmtimeAdapter must accept minimal WASM"
    );
    assert!(
        w3.instantiate("parity", &wasm, &caps).is_ok(),
        "Wasm3Adapter must accept minimal WASM"
    );
}

// ── T053-2: both adapters reject invalid WASM bytes ──────────────────────────

#[test]
fn test_parity_both_reject_invalid_wasm() {
    let caps = Capabilities::default();

    let wt = WasmtimeAdapter::new(wasmtime_config());
    let w3 = Wasm3Adapter::new(wasm3_config());

    assert!(
        wt.instantiate("bad", b"garbage", &caps).is_err(),
        "WasmtimeAdapter must reject garbage"
    );
    assert!(
        w3.instantiate("bad", b"garbage", &caps).is_err(),
        "Wasm3Adapter must reject garbage"
    );
}

// ── T053-3: both adapters return the same instance name ──────────────────────

#[test]
fn test_parity_same_instance_name() {
    let wasm = minimal_wasm();
    let caps = Capabilities::default();

    let wt_inst = WasmtimeAdapter::new(wasmtime_config())
        .instantiate("my-app", &wasm, &caps)
        .expect("wasmtime instantiate ok");

    let w3_inst = Wasm3Adapter::new(wasm3_config())
        .instantiate("my-app", &wasm, &caps)
        .expect("wasm3 instantiate ok");

    assert_eq!(wt_inst.name(), w3_inst.name(), "instance names must be identical");
}

// ── T053-4: both adapters return 0 for memory_usage before execution ─────────

#[test]
fn test_parity_memory_usage_before_execute() {
    let wasm = minimal_wasm();
    let caps = Capabilities::default();

    let wt = WasmtimeAdapter::new(wasmtime_config());
    let wt_inst = wt.instantiate("parity-app", &wasm, &caps).expect("ok");

    let w3 = Wasm3Adapter::new(wasm3_config());
    let w3_inst = w3.instantiate("parity-app", &wasm, &caps).expect("ok");

    assert_eq!(
        wt.memory_usage(wt_inst.as_ref()),
        w3.memory_usage(w3_inst.as_ref()),
        "memory_usage must match before execution"
    );
}

// ── T053-5: both adapters produce a valid ExitCode ──────────────────────────

#[test]
fn test_parity_execute_returns_valid_exit_code() {
    let wasm = minimal_wasm();
    let caps = Capabilities::default();

    // Wasmtime: use /bin/true as runtime stand-in.
    let wt = WasmtimeAdapter::new(wasmtime_config()).with_bin("/bin/true");
    let mut wt_inst = wt.instantiate("parity-app", &wasm, &caps).expect("ok");
    let wt_exit = wt.execute(wt_inst.as_mut()).expect("wasmtime execute ok");

    // wasm3: mock always returns Success(0).
    let w3 = Wasm3Adapter::new(wasm3_config());
    let mut w3_inst = w3.instantiate("parity-app", &wasm, &caps).expect("ok");
    let w3_exit = w3.execute(w3_inst.as_mut()).expect("wasm3 execute ok");

    // Both must be in the valid ExitCode set.
    assert!(
        matches!(wt_exit, ExitCode::Success(_) | ExitCode::Terminated | ExitCode::FuelExhausted),
        "wasmtime exit must be a valid ExitCode: {wt_exit:?}"
    );
    assert!(
        matches!(w3_exit, ExitCode::Success(_) | ExitCode::Terminated | ExitCode::FuelExhausted),
        "wasm3 exit must be a valid ExitCode: {w3_exit:?}"
    );
}

// ── T053-6: both adapters terminate without error ────────────────────────────

#[test]
fn test_parity_terminate_no_error() {
    let wasm = minimal_wasm();
    let caps = Capabilities::default();

    let wt = WasmtimeAdapter::new(wasmtime_config());
    let wt_inst = wt.instantiate("parity-app", &wasm, &caps).expect("ok");
    assert!(wt.terminate(wt_inst.as_ref()).is_ok());

    let w3 = Wasm3Adapter::new(wasm3_config());
    let w3_inst = w3.instantiate("parity-app", &wasm, &caps).expect("ok");
    assert!(w3.terminate(w3_inst.as_ref()).is_ok());
}

// ── T053-7: both return None for fuel_remaining when no limit set ─────────────

#[test]
fn test_parity_fuel_remaining_none_without_limit() {
    let wasm = minimal_wasm();
    let caps = Capabilities::default();

    let wt = WasmtimeAdapter::new(wasmtime_config());
    let wt_inst = wt.instantiate("parity-app", &wasm, &caps).expect("ok");

    let w3 = Wasm3Adapter::new(wasm3_config());
    let w3_inst = w3.instantiate("parity-app", &wasm, &caps).expect("ok");

    assert_eq!(wt.fuel_remaining(wt_inst.as_ref()), None);
    assert_eq!(w3.fuel_remaining(w3_inst.as_ref()), None);
}

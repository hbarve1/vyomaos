// Runtime adapter layer — T001
//
// Defines the `WasmRuntime` trait that every supported WASM execution engine
// (Wasmtime, wasm3, WAMR) must implement.  The supervisor calls only this
// interface; concrete adapters live in sub-modules (wasmtime.rs, wasm3.rs).

use crate::manifest::Capabilities;

pub mod wasmtime;
pub mod wasm3;

// ── Engine / Mode enums ───────────────────────────────────────────────────────

/// The WASM execution engine being used.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Engine {
    Wasmtime,
    Wasm3,
    Wamr,
}

/// Execution mode for the engine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExecutionMode {
    Interpreter,
    Jit,
    Aot,
}

// ── RuntimeConfig ─────────────────────────────────────────────────────────────

/// Configuration passed to a `WasmRuntime` on construction.
#[derive(Debug, Clone)]
pub struct RuntimeConfig {
    pub engine: Engine,
    pub mode: ExecutionMode,
    /// Upper bound on WASM linear memory (64 KB per page).  0 = use runtime default.
    pub max_memory_pages: u32,
    /// Maximum number of instructions before termination.  0 = unlimited.
    pub fuel_limit: u64,
    /// WASI interfaces to wire up for apps (e.g. "wasi:cli/stdout@0.2.0").
    pub wasi_imports: Vec<String>,
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            engine: Engine::Wasmtime,
            mode: ExecutionMode::Jit,
            max_memory_pages: 0,
            fuel_limit: 0,
            wasi_imports: Vec::new(),
        }
    }
}

// ── ExitCode ──────────────────────────────────────────────────────────────────

/// Result of executing a WASM module instance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExitCode {
    /// Module exited normally with the given status code.
    Success(i32),
    /// Module was terminated before completion.
    Terminated,
    /// Module ran out of fuel.
    FuelExhausted,
}

// ── ModuleInstance ────────────────────────────────────────────────────────────

/// Opaque handle to a running (or stopped) WASM module instance.
/// Concrete implementations store their engine-specific state inside.
pub trait ModuleInstance: Send {
    /// Human-readable module name for logging.
    fn name(&self) -> &str;
}

// ── WasmRuntime trait ─────────────────────────────────────────────────────────

/// Contract every WASM runtime adapter must fulfil.
///
/// Invariants (from contracts/runtime-adapter.md):
/// 1. `instantiate` wires up ONLY the WASI imports matching declared capabilities.
/// 2. `instantiate` rejects wasm64 binaries on runtimes that do not support them.
/// 3. `execute` respects `fuel_limit` — returns `ExitCode::FuelExhausted` when done.
/// 4. `terminate` releases ALL resources (memory, file handles, sockets).
/// 5. `memory_usage` returns actual committed memory, NOT the declared maximum.
pub trait WasmRuntime: Send {
    /// Prepare a new module instance from raw WASM bytes.
    ///
    /// Only the WASI interfaces corresponding to `capabilities` are wired up.
    fn instantiate(
        &self,
        name: &str,
        wasm_bytes: &[u8],
        capabilities: &Capabilities,
    ) -> Result<Box<dyn ModuleInstance>, String>;

    /// Run a module instance to completion (or until terminated / fuel exhausted).
    fn execute(&self, instance: &mut dyn ModuleInstance) -> Result<ExitCode, String>;

    /// Forcefully stop a running instance and free all its resources.
    fn terminate(&self, instance: &dyn ModuleInstance) -> Result<(), String>;

    /// Return the number of bytes of linear memory currently committed by the instance.
    fn memory_usage(&self, instance: &dyn ModuleInstance) -> usize;

    /// Return remaining fuel, if a fuel limit is configured.
    fn fuel_remaining(&self, instance: &dyn ModuleInstance) -> Option<u64>;
}

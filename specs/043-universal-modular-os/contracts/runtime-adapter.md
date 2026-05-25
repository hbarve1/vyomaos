# Contract: Runtime Adapter

The runtime adapter is the interface between the supervisor and the WASM execution engine. Every supported runtime (Wasmtime, wasm3, WAMR) must implement this contract.

## Trait: `WasmRuntime`

```
fn new(config: RuntimeConfig) → Self
fn instantiate(wasm_bytes: &[u8], capabilities: &CapabilitySet) → Result<ModuleInstance>
fn execute(instance: &mut ModuleInstance) → Result<ExitCode>
fn terminate(instance: &ModuleInstance) → Result<()>
fn memory_usage(instance: &ModuleInstance) → usize
fn fuel_remaining(instance: &ModuleInstance) → Option<u64>
```

## Invariants

1. `instantiate` MUST wire up only the WASI imports corresponding to the declared capabilities — no more, no less
2. `instantiate` MUST reject wasm64 binaries on runtimes that do not support 64-bit memory
3. `execute` MUST respect `fuel_limit` if set — returning an error when fuel is exhausted
4. `terminate` MUST release all resources (memory, file handles, sockets) owned by the instance
5. `memory_usage` MUST return the actual committed memory, not the maximum declared

## RuntimeConfig

| Field | Type | Description |
|-------|------|-------------|
| `engine` | enum | `wasmtime`, `wasm3`, `wamr` |
| `mode` | enum | `interpreter`, `jit`, `aot` |
| `max_memory_pages` | u32 | Upper bound on WASM linear memory (64KB per page) |
| `fuel_limit` | u64 | Max instructions before termination (0 = unlimited) |
| `wasi_imports` | Vec<String> | WASI interfaces to wire up |

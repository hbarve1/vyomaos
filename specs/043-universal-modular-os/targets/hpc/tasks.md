# Tasks: HPC / Supercomputer Target (US4 — P4)

**Parent spec**: `specs/043-universal-modular-os/`
**Branch**: `feat/043-hpc`
**Prerequisite**: `feat/043-core` merged to `develop` first

**Goal**: Deterministic, byte-identical WASM compute jobs across CPU architectures. wasm64 support for memory-intensive workloads.

**Independent Test**: Run same compute WASM job on x86-64 and ARM64 QEMU nodes, compare output byte-for-byte.

---

## Phase 6: User Story 4 — HPC Reproducible Compute Jobs

### Tests

- [ ] T047 [P] Write failing test for wasm64 module loading in `supervisor/tests/wasm64_loading.rs` — verify runtime accepts wasm64 binaries on supported platforms

### Implementation

- [ ] T048 Extend runtime adapter trait to support wasm64 in `supervisor/src/runtime/mod.rs` — add `WasmTarget::Wasm64` variant, reject on runtimes that don't support it
- [ ] T049 Update Wasmtime adapter in `supervisor/src/runtime/wasmtime.rs` — enable 64-bit memory support when wasm64 target detected. Make T047 pass
- [ ] T050 Create sample compute job in `apps/compute-bench/` — deterministic math workload (SHA-256 hash chain) targeting wasm64-wasip2
- [ ] T051 Verify byte-identical output: run `compute-bench.wasm` on two architecture configurations, compare stdout byte-for-byte

**Checkpoint**: wasm64 support operational. Deterministic compute verified across architectures.

---

## Acceptance Criteria

1. `compute-bench.wasm` (wasm64) loaded by Wasmtime adapter without error
2. wasm32 runtime rejects wasm64 binary with clear error (not a silent hang)
3. Same `compute-bench.wasm` run on x86-64 and ARM64 QEMU nodes → byte-identical stdout
4. `cargo test` passes including `wasm64_loading` test

## Key files to touch

- `supervisor/src/runtime/mod.rs` (add `WasmTarget::Wasm64` variant)
- `supervisor/src/runtime/wasmtime.rs` (enable 64-bit memory)
- `apps/compute-bench/src/main.rs` (new SHA-256 chain app)
- `apps/compute-bench/vyoma.toml` (new manifest)
- `apps/compute-bench/Cargo.toml` (new, target wasm64-wasip2)
- `supervisor/tests/wasm64_loading.rs` (new)

## Notes

- wasm64 toolchain support in Rust/LLVM must be verified before starting T048
- If wasm64 toolchain is unavailable, T050 can use wasm32 for the compute benchmark and T047/T048/T049 can use mock wasm64 validation logic

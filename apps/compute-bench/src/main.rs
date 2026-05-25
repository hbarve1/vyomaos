// compute-bench — deterministic SHA-256 hash-chain benchmark (T050)
//
// Runs 10 000 iterations of SHA-256, chaining each output as the next input.
// Seed: 32 zero bytes.
//
// Intended runtime target: wasm64-wasip2 (64-bit linear memory, HPC workloads).
// CI build target:         wasm32-wasip2 (wasm64 toolchain not yet stable).
//
// The algorithm is identical under both targets — only the pointer width
// changes.  This guarantees byte-identical stdout across x86-64 and ARM64
// QEMU nodes (T051 acceptance criterion).
//
// Output: one line — the 64-character lowercase hex SHA-256 of the final state.
//   e.g. "a665a45920422f9d417e4867efdc4fb8a04a1f3fff1fa07e998e86f7f7a27ae3"

mod sha256;

fn main() {
    const ITERATIONS: u32 = 10_000;

    let hash = compute_chain(ITERATIONS);

    // Print hex string to stdout — this is what the supervisor captures and
    // the cross-arch byte-identical check (T051) compares.
    println!("{hash}");
}

/// Run `iterations` rounds of SHA-256, chaining output → input.
/// Seed is 32 zero bytes.
///
/// This function is the canonical compute logic shared with the supervisor-side
/// unit test in `supervisor/tests/wasm64_loading.rs`.
pub fn compute_chain(iterations: u32) -> String {
    let mut state = [0u8; 32];
    for _ in 0..iterations {
        state = sha256::hash(&state);
    }
    to_hex(&state)
}

/// Encode a byte slice as lowercase hex.
fn to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

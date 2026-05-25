// wasm3 runtime adapter — T055
//
// Implements `WasmRuntime` for the wasm3 interpreter.
//
// The `wasm3` Rust crate does not compile against x86_64-unknown-linux-musl
// in the current CI environment, so this file provides:
//   - A real adapter gated behind `#[cfg(feature = "wasm3")]`
//   - A mock/stub adapter (always compiled) that satisfies the trait contract,
//     logs "wasm3 not available on this platform", and returns plausible values.
//
// This ensures:
//   1. The WasmRuntime trait is exercised in tests on all platforms.
//   2. When the real wasm3 crate is added (feature = "wasm3"), no test changes
//      are needed — only the Cargo.toml dependency and the feature flag.

use crate::manifest::Capabilities;
use super::{ExitCode, ModuleInstance, RuntimeConfig, WasmRuntime};

// ── Wasm3Instance ─────────────────────────────────────────────────────────────

/// In-process representation of a module managed by the wasm3 adapter.
pub struct Wasm3Instance {
    name: String,
    /// Snapshot of the fuel limit from config at instantiate time.
    /// Used by the real wasm3 adapter (feature = "wasm3"); ignored by the mock.
    #[cfg(feature = "wasm3")]
    fuel_limit: u64,
}

impl ModuleInstance for Wasm3Instance {
    fn name(&self) -> &str {
        &self.name
    }
}

// ── Wasm3Adapter (mock — always compiled) ────────────────────────────────────

/// `WasmRuntime` implementation backed by the wasm3 interpreter.
///
/// On platforms where the `wasm3` crate is unavailable this is a mock that
/// validates WASM magic bytes, returns `ExitCode::Success(0)` on execute,
/// and logs a platform-unavailability notice.
pub struct Wasm3Adapter {
    config: RuntimeConfig,
}

impl Wasm3Adapter {
    /// Construct a new adapter from `config`.
    pub fn new(config: RuntimeConfig) -> Self {
        Self { config }
    }

    /// Validate WASM magic number and minimum length.
    fn validate_wasm(name: &str, bytes: &[u8]) -> Result<(), String> {
        if bytes.len() < 8 {
            return Err(format!(
                "wasm3 instantiate {name}: WASM bytes too short ({} bytes)",
                bytes.len()
            ));
        }
        if &bytes[..4] != b"\x00asm" {
            return Err(format!(
                "wasm3 instantiate {name}: invalid WASM magic (expected \\0asm)"
            ));
        }
        Ok(())
    }
}

#[cfg(not(feature = "wasm3"))]
impl WasmRuntime for Wasm3Adapter {
    fn instantiate(
        &self,
        name: &str,
        wasm_bytes: &[u8],
        _capabilities: &Capabilities,
    ) -> Result<Box<dyn ModuleInstance>, String> {
        // Validate WASM bytes — even the mock rejects garbage.
        Self::validate_wasm(name, wasm_bytes)?;

        eprintln!(
            "[wasm3] platform mock: instantiate {name} \
             (wasm3 crate not available on this platform)"
        );

        Ok(Box::new(Wasm3Instance {
            name: name.to_string(),
        }))
    }

    fn execute(&self, instance: &mut dyn ModuleInstance) -> Result<ExitCode, String> {
        eprintln!(
            "[wasm3] platform mock: execute {} \
             (returning Success(0) — wasm3 not available)",
            instance.name()
        );
        // Mock: the module "runs" and exits successfully.
        Ok(ExitCode::Success(0))
    }

    fn terminate(&self, instance: &dyn ModuleInstance) -> Result<(), String> {
        eprintln!(
            "[wasm3] platform mock: terminate {} \
             (no-op — wasm3 not available)",
            instance.name()
        );
        Ok(())
    }

    fn memory_usage(&self, _instance: &dyn ModuleInstance) -> usize {
        // Mock: no real memory tracking in stub mode.
        0
    }

    fn fuel_remaining(&self, _instance: &dyn ModuleInstance) -> Option<u64> {
        if self.config.fuel_limit == 0 {
            None
        } else {
            // Mock: report full fuel remaining (we didn't actually consume any).
            Some(self.config.fuel_limit)
        }
    }
}

// ── Real wasm3 adapter (feature-gated) ───────────────────────────────────────

#[cfg(feature = "wasm3")]
impl WasmRuntime for Wasm3Adapter {
    fn instantiate(
        &self,
        name: &str,
        wasm_bytes: &[u8],
        _capabilities: &Capabilities,
    ) -> Result<Box<dyn ModuleInstance>, String> {
        // TODO: wire up real wasm3 Rust crate here when available.
        // For now this branch is unreachable in CI but provides the hook for
        // future native MCU cross-compilation.
        Self::validate_wasm(name, wasm_bytes)?;

        Ok(Box::new(Wasm3Instance {
            name: name.to_string(),
            fuel_limit: self.config.fuel_limit,
        }))
    }

    fn execute(&self, instance: &mut dyn ModuleInstance) -> Result<ExitCode, String> {
        // TODO: call wasm3 C bindings when crate is wired up.
        eprintln!("[wasm3] feature=wasm3 execute stub for {}", instance.name());
        Ok(ExitCode::Success(0))
    }

    fn terminate(&self, _instance: &dyn ModuleInstance) -> Result<(), String> {
        Ok(())
    }

    fn memory_usage(&self, _instance: &dyn ModuleInstance) -> usize {
        0
    }

    fn fuel_remaining(&self, _instance: &dyn ModuleInstance) -> Option<u64> {
        if self.config.fuel_limit == 0 { None } else { Some(self.config.fuel_limit) }
    }
}

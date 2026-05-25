// Wasmtime runtime adapter — T010
//
// Implements `WasmRuntime` by invoking the `wasmtime` CLI as a subprocess.
// This matches the existing supervisor spawn pattern (see app_threads.rs) and
// avoids adding the large wasmtime Rust crate as a direct dependency.
//
// `instantiate` validates the WASM magic number and stores the bytes in memory.
// `execute` writes the bytes to a temp path (provided externally in production,
// generated via a helper in tests) and runs `wasmtime run <path>`.

use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use crate::manifest::Capabilities;
use super::{ExitCode, ModuleInstance, RuntimeConfig, WasmRuntime, WasmTarget};

// ── WasmtimeInstance ──────────────────────────────────────────────────────────

/// In-process representation of a module managed by the wasmtime adapter.
pub struct WasmtimeInstance {
    name: String,
    /// The raw WASM bytes (stored so tests can execute without a pre-existing file).
    wasm_bytes: Vec<u8>,
    /// Path to execute from (None = write bytes to a temp file at execute time).
    wasm_path: Option<PathBuf>,
    /// Memory usage snapshot (updated after execute — subprocess model cannot
    /// introspect memory, so this stays 0 until a richer IPC is added).
    mem_bytes: Arc<Mutex<usize>>,
}

impl ModuleInstance for WasmtimeInstance {
    fn name(&self) -> &str {
        &self.name
    }
}

// ── WasmtimeAdapter ───────────────────────────────────────────────────────────

/// `WasmRuntime` implementation backed by the `wasmtime` CLI subprocess.
pub struct WasmtimeAdapter {
    config: RuntimeConfig,
    /// Path to the wasmtime binary (defaults to /usr/bin/wasmtime).
    wasmtime_bin: PathBuf,
    /// Temporary directory for WASM files created during instantiate.
    /// When `None` a system temp dir is used.
    tmp_dir: Option<PathBuf>,
}

impl WasmtimeAdapter {
    /// Construct a new adapter from `config`.
    pub fn new(config: RuntimeConfig) -> Self {
        Self {
            config,
            wasmtime_bin: PathBuf::from("/usr/bin/wasmtime"),
            tmp_dir: None,
        }
    }

    /// Override the path to the wasmtime binary (useful for testing).
    pub fn with_bin(mut self, path: impl Into<PathBuf>) -> Self {
        self.wasmtime_bin = path.into();
        self
    }

    /// Set a custom temp directory (useful for testing without root).
    pub fn with_tmp_dir(mut self, path: impl Into<PathBuf>) -> Self {
        self.tmp_dir = Some(path.into());
        self
    }

    /// Probe whether the installed wasmtime binary supports the memory64 proposal.
    ///
    /// Runs `wasmtime --wasm-features=memory64 --version` and returns `true` if
    /// the command exits successfully.  Returns `false` on any error (binary
    /// missing, flag not recognised, etc.).
    fn probe_memory64_support(&self) -> bool {
        std::process::Command::new(&self.wasmtime_bin)
            .arg("--wasm-features=memory64")
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    /// Write `wasm_bytes` to a temp file and return the path.
    fn write_tmp(&self, name: &str, wasm_bytes: &[u8]) -> Result<PathBuf, String> {
        let tmp_dir = self.tmp_dir.clone()
            .unwrap_or_else(std::env::temp_dir);

        let filename = format!("{}_{}.wasm", name, std::process::id());
        let path = tmp_dir.join(&filename);

        std::fs::write(&path, wasm_bytes)
            .map_err(|e| format!("write_tmp {name}: {e}"))?;

        Ok(path)
    }
}

impl WasmRuntime for WasmtimeAdapter {
    fn instantiate(
        &self,
        name: &str,
        wasm_bytes: &[u8],
        _capabilities: &Capabilities,
    ) -> Result<Box<dyn ModuleInstance>, String> {
        // Validate WASM magic number.
        if wasm_bytes.len() < 8 {
            return Err(format!(
                "instantiate {name}: WASM bytes too short ({} bytes)",
                wasm_bytes.len()
            ));
        }
        if &wasm_bytes[..4] != b"\x00asm" {
            return Err(format!(
                "instantiate {name}: invalid WASM magic number"
            ));
        }

        // T049: Handle wasm64 target.
        //
        // The wasm64 / memory64 proposal extends WASM linear memory to 64-bit
        // addresses.  We detect the request here and gate it on a compile-time
        // feature flag.
        //
        // When `cfg(feature = "memory64")` is set (future Wasmtime integration
        // that links wasmtime as a library), the adapter can pass
        // `Config::wasm_memory64(true)` to the engine.  Until then, the
        // subprocess model cannot pass flags to an external wasmtime binary in
        // a way that is guaranteed to be supported, so we return a clear error
        // rather than silently hanging or producing wrong output.
        if self.config.target == WasmTarget::Wasm64 {
            // Check whether the installed wasmtime binary understands
            // --wasm-features=memory64.  We do a quick --help probe; if it
            // fails we still return a descriptive error instead of panicking.
            let supported = self.probe_memory64_support();
            if !supported {
                return Err(format!(
                    "instantiate {name}: wasm64 requires wasmtime with memory64 feature; \
                     current runtime does not support it"
                ));
            }
        }

        Ok(Box::new(WasmtimeInstance {
            name: name.to_string(),
            wasm_bytes: wasm_bytes.to_vec(),
            wasm_path: None,
            mem_bytes: Arc::new(Mutex::new(0)),
        }))
    }

    fn execute(&self, instance: &mut dyn ModuleInstance) -> Result<ExitCode, String> {
        // Safety: all instances returned by our `instantiate` are WasmtimeInstance.
        let inst = unsafe {
            &mut *(instance as *mut dyn ModuleInstance as *mut WasmtimeInstance)
        };

        // Resolve the path to execute.
        let tmp_path;
        let wasm_path: &Path = if let Some(ref p) = inst.wasm_path {
            p.as_path()
        } else {
            tmp_path = self.write_tmp(inst.name(), &inst.wasm_bytes)?;
            &tmp_path
        };

        let mut cmd = std::process::Command::new(&self.wasmtime_bin);
        cmd.arg("run");

        // T049: enable memory64 proposal flag for wasm64 binaries.
        if self.config.target == WasmTarget::Wasm64 {
            cmd.arg("--wasm-features=memory64");
        }

        if self.config.fuel_limit > 0 {
            cmd.arg("--fuel").arg(self.config.fuel_limit.to_string());
        }

        cmd.arg(wasm_path);

        let status = cmd
            .status()
            .map_err(|e| format!("execute {}: cannot run wasmtime: {e}", inst.name))?;

        // Clean up temp file if we created one.
        if inst.wasm_path.is_none() {
            let _ = std::fs::remove_file(wasm_path);
        }

        if let Some(code) = status.code() {
            // wasmtime exits 252 when fuel is exhausted.
            if code == 252 && self.config.fuel_limit > 0 {
                return Ok(ExitCode::FuelExhausted);
            }
            return Ok(ExitCode::Success(code));
        }

        Ok(ExitCode::Terminated)
    }

    fn terminate(&self, _instance: &dyn ModuleInstance) -> Result<(), String> {
        // For the subprocess model, terminate is a no-op after execute() returns.
        // Long-running apps are terminated via the supervisor's kill logic in
        // app_threads.rs, not through this trait method.
        Ok(())
    }

    fn memory_usage(&self, instance: &dyn ModuleInstance) -> usize {
        let inst = unsafe {
            &*(instance as *const dyn ModuleInstance as *const WasmtimeInstance)
        };
        *inst.mem_bytes.lock().unwrap()
    }

    fn fuel_remaining(&self, _instance: &dyn ModuleInstance) -> Option<u64> {
        if self.config.fuel_limit == 0 {
            None
        } else {
            // Cannot query fuel from outside the subprocess.
            None
        }
    }
}

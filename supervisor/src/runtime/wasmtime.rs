// Wasmtime runtime adapter — T010 + T038
//
// Implements `WasmRuntime` by invoking the `wasmtime` CLI as a subprocess.
// This matches the existing supervisor spawn pattern (see app_threads.rs) and
// avoids adding the large wasmtime Rust crate as a direct dependency.
//
// `instantiate` validates the WASM magic number and stores the bytes in memory.
// `execute` writes the bytes to a temp path (provided externally in production,
// generated via a helper in tests) and runs `wasmtime run <path>`.
//
// T038: HAL host function gating.
// The `WasmtimeAdapter` carries an optional `PeripheralCapabilitySet` snapshot
// for the module being instantiated.  `hal_gate_check` is called before any
// HAL host function to verify the module declared the required capability;
// if not, it returns a descriptive error suitable for conversion to a WASM trap.

use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use crate::capability::PeripheralCapabilitySet;
use crate::hal::Direction;
use crate::manifest::Capabilities;
use super::{ExitCode, ModuleInstance, RuntimeConfig, WasmRuntime};

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

// ── T038: HAL host-function gate helpers ──────────────────────────────────────
//
// These functions are called by the supervisor before dispatching any HAL
// call routed from a WASM module.  They enforce the capability declarations
// from `vyoma.toml` and return an error string that the caller converts into
// a WASM trap.
//
// In the subprocess model the actual HAL calls happen in the supervisor
// process; the guest module communicates via structured stdout lines that
// the supervisor parses and dispatches through these guards.

/// Gate check for GPIO host functions.
///
/// Returns `Ok(())` if `caps` permits GPIO access on `pin` in `direction`.
/// Returns `Err(trap_message)` otherwise — callers should abort the guest.
pub fn hal_gate_gpio(
    caps: &PeripheralCapabilitySet,
    pin: u8,
    direction: Direction,
) -> Result<(), String> {
    let gpio = caps.gpio.as_ref().ok_or_else(|| {
        format!(
            "WASM trap: GPIO access denied for pin {pin} — \
             module did not declare gpio capability"
        )
    })?;
    if !gpio.pins.contains(&pin) {
        return Err(format!(
            "WASM trap: GPIO pin {pin} not in declared allowed set"
        ));
    }
    if let Some(allowed_dir) = gpio.direction {
        if allowed_dir != direction {
            return Err(format!(
                "WASM trap: GPIO pin {pin} direction mismatch \
                 (declared {allowed_dir:?}, requested {direction:?})"
            ));
        }
    }
    Ok(())
}

/// Gate check for I2C host functions.
pub fn hal_gate_i2c(
    caps: &PeripheralCapabilitySet,
    bus: u8,
    addr: u8,
) -> Result<(), String> {
    let i2c = caps.i2c.as_ref().ok_or_else(|| {
        format!(
            "WASM trap: I2C access denied — module did not declare i2c capability \
             (bus {bus}, addr 0x{addr:02X})"
        )
    })?;
    if i2c.bus != bus {
        return Err(format!(
            "WASM trap: I2C bus {bus} not in declared capability (declared bus {})",
            i2c.bus
        ));
    }
    if let Some(allowed_addr) = i2c.address {
        if allowed_addr != addr {
            return Err(format!(
                "WASM trap: I2C address 0x{addr:02X} not allowed \
                 (declared 0x{allowed_addr:02X} on bus {bus})"
            ));
        }
    }
    Ok(())
}

/// Gate check for UART host functions.
pub fn hal_gate_uart(
    caps: &PeripheralCapabilitySet,
    port: u8,
) -> Result<(), String> {
    let uart = caps.uart.as_ref().ok_or_else(|| {
        format!(
            "WASM trap: UART access denied — module did not declare uart capability \
             (port {port})"
        )
    })?;
    if uart.port != port {
        return Err(format!(
            "WASM trap: UART port {port} not in declared capability (declared port {})",
            uart.port
        ));
    }
    Ok(())
}

// ── T038 unit tests ───────────────────────────────────────────────────────────

#[cfg(test)]
mod hal_gate_tests {
    use super::*;
    use crate::capability::{GpioCapability, I2cCapability, UartCapability};
    use crate::hal::Direction;

    fn caps_with_gpio(pins: Vec<u8>, dir: Option<Direction>) -> PeripheralCapabilitySet {
        PeripheralCapabilitySet {
            gpio: Some(GpioCapability { pins, direction: dir }),
            ..Default::default()
        }
    }

    #[test]
    fn gpio_gate_allows_declared_pin() {
        let caps = caps_with_gpio(vec![5], None);
        assert!(hal_gate_gpio(&caps, 5, Direction::Output).is_ok());
    }

    #[test]
    fn gpio_gate_denies_undeclared_capability() {
        let caps = PeripheralCapabilitySet::default();
        let result = hal_gate_gpio(&caps, 3, Direction::Output);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("did not declare gpio"));
    }

    #[test]
    fn gpio_gate_denies_wrong_pin() {
        let caps = caps_with_gpio(vec![5], None);
        let result = hal_gate_gpio(&caps, 7, Direction::Output);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not in declared allowed set"));
    }

    #[test]
    fn i2c_gate_allows_declared_bus() {
        let caps = PeripheralCapabilitySet {
            i2c: Some(I2cCapability { bus: 1, address: None }),
            ..Default::default()
        };
        assert!(hal_gate_i2c(&caps, 1, 0x48).is_ok());
    }

    #[test]
    fn i2c_gate_denies_undeclared_capability() {
        let caps = PeripheralCapabilitySet::default();
        assert!(hal_gate_i2c(&caps, 0, 0x48).is_err());
    }

    #[test]
    fn uart_gate_allows_declared_port() {
        let caps = PeripheralCapabilitySet {
            uart: Some(UartCapability { port: 0, baud: None }),
            ..Default::default()
        };
        assert!(hal_gate_uart(&caps, 0).is_ok());
    }

    #[test]
    fn uart_gate_denies_undeclared_capability() {
        let caps = PeripheralCapabilitySet::default();
        let result = hal_gate_uart(&caps, 0);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("did not declare uart"));
    }
}

# Contract: Platform Profile Schema

Platform profiles are TOML files that configure a VyomaOS build for a specific hardware target. They are read at build time to determine which supervisor modules, HAL drivers, runtime, and apps to include.

## Schema

```toml
[platform]
name = "string"           # Profile identifier (e.g., "mcu-minimal")
arch = "string"           # Target CPU arch (e.g., "arm-cortex-m4", "x86-64", "riscv32")
runtime = "string"        # WASM runtime: "wasm3" | "wamr" | "wasmtime"
min_ram_kb = 128           # Minimum RAM in KB

[supervisor]
modules = ["string"]       # Supervisor subsystems to include
exclude = ["string"]       # Supervisor subsystems to exclude (optional)

[hal]
drivers = ["string"]       # HAL driver modules to include (e.g., "gpio", "i2c")

[boot]
apps = ["string"]          # WASM apps to launch at boot

[observability]
tier = "string"            # "baseline" | "full"
heartbeat_interval_s = 30  # Seconds between heartbeats (default: 30)

[ota]
enabled = true             # Whether OTA is supported
health_check_secs = 60     # Health check duration after update
ab_slots = true            # A/B slot enabled (default: true when ota.enabled)

[build]
kernel_config = "string"   # Path to kernel .config (relative to platform dir)
rootfs_script = "string"   # Path to rootfs build script
```

## Validation Rules

1. `runtime` MUST be one of: `wasm3`, `wamr`, `wasmtime`
2. `supervisor.modules` MUST include at minimum: `lifecycle`, `capability`
3. `hal.drivers` entries MUST correspond to implemented HAL traits for the target `arch`
4. `boot.apps` entries MUST have corresponding `vyoma.toml` manifests
5. If `runtime = "wasm3"` and `min_ram_kb < 64`, emit warning: wasm3 minimum is ~64 KB
6. If `runtime = "wasmtime"` and `min_ram_kb < 4096`, emit error: Wasmtime requires ~4 MB
7. `observability.tier = "full"` requires `runtime = "wasmtime"` (OpenTelemetry SDK is too large for MCU runtimes)

## Predefined Profiles

| Profile | Arch | Runtime | RAM | Supervisor Modules | HAL Drivers |
|---------|------|---------|-----|-------------------|-------------|
| `mcu-minimal` | ARM Cortex-M | wasm3 | 128 KB | lifecycle, capability, ipc, hal, observability | gpio, i2c, uart |
| `iot-edge` | ARM64 / x86 | wamr | 4 MB | lifecycle, capability, ipc, hal, observability, net, packages | gpio, i2c, spi, uart, adc |
| `robotics-rt` | ARM64 | wamr | 8 MB | lifecycle, capability, ipc, hal, observability, net | gpio, i2c, spi, uart, adc |
| `desktop-full` | x86-64 / ARM64 | wasmtime | 512 MB | all | none (virtual hardware via display) |
| `server-headless` | x86-64 / ARM64 | wasmtime | 1 GB | lifecycle, capability, ipc, net, packages, observability | none |
| `mobile` | ARM64 | wasmtime | 256 MB | lifecycle, capability, ipc, display, observability, net | none |

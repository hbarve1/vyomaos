# Data Model: VyomaOS Universal Modular OS

**Date**: 2026-05-25 | **Spec**: [spec.md](spec.md)

## Entities

### 1. WASM Module

The fundamental unit of software in VyomaOS.

| Attribute | Type | Description |
|-----------|------|-------------|
| `name` | string | Unique identifier (e.g., "sensor-reader") |
| `version` | semver | Semantic version (e.g., "1.2.0") |
| `wasm_target` | enum | `wasm32-wasip2` or `wasm64-wasip2` |
| `hash` | SHA-256 | Content hash for integrity verification |
| `signature` | bytes | Cryptographic signature for authenticity |
| `capabilities` | CapabilitySet | Declared capabilities from vyoma.toml |
| `restart_policy` | enum | `never`, `always`, `on-failure` |
| `state` | enum | `pending`, `running`, `stopped`, `failed`, `updating` |
| `slot` | enum | `A` (primary), `B` (update target) |

**Relationships**: Belongs to a Platform Profile; communicates with other modules via IPC Broker; accesses hardware via HAL.

**State transitions**:
```
pending → running → stopped
                  → failed → (restart_policy=always) → pending
running → updating → running (slot swap success)
                   → running (rollback to previous slot)
```

### 2. Capability Manifest (vyoma.toml)

Per-module configuration declaring required capabilities.

| Section | Type | Description |
|---------|------|-------------|
| `[app]` | table | Module identity: name, version, wasm filename |
| `[capabilities]` | table | System capabilities: stdio, filesystem, network, display, shell, mouse |
| `[capabilities.gpio]` | table | GPIO pin access: pins[], direction |
| `[capabilities.i2c]` | table | I2C bus access: bus, address |
| `[capabilities.spi]` | table | SPI access: bus, cs_pin |
| `[capabilities.uart]` | table | UART access: port, baud |
| `[capabilities.adc]` | table | ADC access: channels[] |
| `[relationships]` | table | (Future) ReBAC declarations: controls, reads_from, administered_by |

**Validation rules**: All declared pins/buses/ports must exist on the target platform's HAL; conflicts (two modules claiming the same pin as output) are rejected at spawn time.

### 3. Platform Profile

Build-time configuration selecting which components are included.

| Attribute | Type | Description |
|-----------|------|-------------|
| `name` | string | Profile identifier (e.g., "mcu-minimal") |
| `arch` | string | Target architecture (e.g., "arm-cortex-m4", "x86-64") |
| `runtime` | enum | `wasm3`, `wamr`, `wasmtime` |
| `min_ram_kb` | integer | Minimum RAM requirement |
| `supervisor.modules` | string[] | Included supervisor subsystems |
| `supervisor.exclude` | string[] | Excluded supervisor subsystems |
| `hal.drivers` | string[] | Included HAL driver modules |
| `boot.apps` | string[] | Apps launched at boot |
| `observability.tier` | enum | `baseline` (logs+heartbeat), `full` (OpenTelemetry) |
| `ota.enabled` | bool | Whether OTA updates are supported |
| `ota.health_check_secs` | integer | Health check duration after update (default: 60) |

### 4. Runtime Adapter

Abstraction over different WASM execution engines.

| Attribute | Type | Description |
|-----------|------|-------------|
| `engine` | enum | `wasmtime`, `wasm3`, `wamr` |
| `mode` | enum | `interpreter`, `jit`, `aot` |
| `max_memory_pages` | integer | Maximum WASM memory pages allowed |
| `fuel_limit` | integer | Instruction fuel limit (0 = unlimited) |
| `wasi_imports` | string[] | WASI interfaces wired up based on capabilities |

**Trait interface** (Rust):
- `fn instantiate(wasm_bytes, capabilities) → ModuleInstance`
- `fn execute(instance) → Result`
- `fn terminate(instance)`
- `fn memory_usage(instance) → bytes`

### 5. OTA Update Record

Tracks update state per module.

| Attribute | Type | Description |
|-----------|------|-------------|
| `module_name` | string | Target module |
| `from_version` | semver | Currently running version |
| `to_version` | semver | New version being deployed |
| `slot` | enum | `A`, `B` |
| `status` | enum | `downloading`, `verifying`, `deploying`, `health-checking`, `complete`, `rolled-back`, `failed` |
| `started_at` | timestamp | Update start time |
| `completed_at` | timestamp | Update completion time |
| `health_checks_passed` | integer | Number of successful health checks |
| `rollback_reason` | string | Reason for rollback (if applicable) |

### 6. Heartbeat Record

Periodic health signal emitted by each running module.

| Attribute | Type | Description |
|-----------|------|-------------|
| `type` | const | "heartbeat" |
| `module` | string | Module name |
| `uptime_s` | integer | Seconds since module start |
| `mem_kb` | integer | Current memory usage in KB |
| `status` | enum | `healthy`, `degraded`, `unhealthy` |
| `last_error` | string | Last error message (if any) |
| `ts` | ISO-8601 | Timestamp |

## Entity Relationships

```
Platform Profile
  ├── selects → Runtime Adapter (wasm3 | wamr | wasmtime)
  ├── includes → HAL Drivers[]
  ├── boots → WASM Module[]
  └── configures → Observability Tier

WASM Module
  ├── declares → Capability Manifest (vyoma.toml)
  ├── executes via → Runtime Adapter
  ├── accesses hardware via → HAL (if capabilities declared)
  ├── communicates via → IPC Broker (supervisor-mediated)
  ├── emits → Heartbeat Records
  └── updated via → OTA Update Records (A/B slot)

Capability Manifest
  ├── grants → System Capabilities (stdio, network, etc.)
  ├── grants → Peripheral Capabilities (GPIO pins, I2C buses, etc.)
  └── (future) declares → Relationships (ReBAC)
```

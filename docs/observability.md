# VyomaOS Observability

**Spec**: `specs/043-universal-modular-os/` | **Date**: 2026-05-25

---

## Overview

Every running module emits periodic health heartbeats that the supervisor collects and can forward to remote monitoring endpoints. On resource-constrained platforms the heartbeat tier is "baseline" (JSON-line to serial). On desktop and server platforms a richer telemetry tier is selectable via the platform profile.

---

## Heartbeat JSON-Line Format

Each heartbeat is a single JSON object on one line (no trailing spaces, terminated by `\n`). The supervisor writes one line per running module at the configured `heartbeat_interval_s` interval (default: 30 s on desktop/server, 60 s on IoT/MCU).

### Fields

| Field | Type | Description |
|-------|------|-------------|
| `type` | `string` | Always `"heartbeat"` |
| `module` | `string` | App name as declared in `vyoma.toml` `[app].name` |
| `uptime_s` | `uint` | Seconds since the module was last spawned |
| `mem_kb` | `uint` | Estimated resident memory usage in KB (0 on MCU where not measurable) |
| `status` | `string` | One of: `"healthy"`, `"degraded"`, `"unhealthy"` |
| `last_error` | `string\|null` | Most recent error message from the module, or `null` |

### Example Lines

```json
{"type":"heartbeat","module":"shell","uptime_s":120,"mem_kb":14,"status":"healthy","last_error":null}
{"type":"heartbeat","module":"http-server","uptime_s":119,"mem_kb":32,"status":"healthy","last_error":null}
{"type":"heartbeat","module":"sensor-reader","uptime_s":3600,"mem_kb":8,"status":"degraded","last_error":"I2C bus timeout on address 0x48"}
```

### Status Values

- `healthy` — module is running and has produced output within the watchdog window (or watchdog is disabled).
- `degraded` — module is running but has reported a recoverable error in its last cycle.
- `unhealthy` — module has exceeded its watchdog timeout, crashed and not restarted, or failed its OTA health check.

---

## Platform Profile Configuration

The observability tier is controlled by the `[observability]` section in the platform profile TOML:

```toml
# Example: supervisor/src/profile/profiles/iot-edge.toml
[observability]
tier = "baseline"
heartbeat_interval_s = 30
```

```toml
# Example: supervisor/src/profile/profiles/desktop-full.toml
[observability]
tier = "full"
heartbeat_interval_s = 30
```

| Tier | What is emitted | Platforms |
|------|----------------|-----------|
| `baseline` | JSON-line heartbeats to stderr / serial | MCU, IoT, Robotics |
| `full` | Heartbeats + structured log events; can be forwarded via network | Mobile, Desktop, Server |

---

## How to Tail Heartbeats

### During Development (QEMU headless run)

```bash
make run | grep HEARTBEAT
```

The supervisor prefixes each heartbeat line with `HEARTBEAT:` before writing to stderr, so it is easy to filter from app output:

```
HEARTBEAT: {"type":"heartbeat","module":"shell","uptime_s":30,...}
HEARTBEAT: {"type":"heartbeat","module":"http-server","uptime_s":30,...}
```

### In QEMU Serial Console

```bash
# Inside the VM:
logf supervisor        # tail supervisor stderr which includes HEARTBEAT lines
```

### Redirecting to a File

```bash
make run 2>heartbeats.log &
tail -f heartbeats.log | grep '"type":"heartbeat"'
```

### Remote Monitoring (network-capable platforms)

On platforms with `network = true` in the supervisor profile, heartbeats can be forwarded to a remote endpoint. The supervisor reads the endpoint from the profile:

```toml
[observability]
tier = "full"
heartbeat_interval_s = 30
export_url = "http://monitor.local:9000/heartbeat"
```

When `export_url` is set, the supervisor POSTs each heartbeat JSON line as `application/x-ndjson` to the configured endpoint.

---

## Implementation Location

The heartbeat subsystem is implemented in:

- `supervisor/src/observability/mod.rs` — module entry point, re-exports
- `supervisor/src/observability/heartbeat.rs` — `Heartbeat` struct, `HeartbeatEmitter`, `ModuleStatus` enum, `to_json_line()` serializer

The `HeartbeatEmitter` tracks per-module state. When `is_due()` returns `true`, the supervisor calls `emit(mem_kb, status)` which returns a `Heartbeat` value that can be serialized with `to_json_line()`.

---

## OpenTelemetry Integration Path (Future)

The `full` observability tier is designed to be the insertion point for OpenTelemetry-compatible telemetry. The planned integration follows this interface:

### Planned Interface

```
Supervisor → HeartbeatEmitter::emit()
           → Heartbeat::to_json_line()        (today: write to stderr)
           → OtelExporter::export(heartbeat)  (future: encode as OTLP)
           → HTTP POST to collector endpoint  (future: gRPC or HTTP/JSON)
```

### OTLP Mapping (Planned)

VyomaOS heartbeats map to OpenTelemetry as follows:

| Heartbeat Field | OTLP Mapping |
|----------------|-------------|
| `module` | `service.name` resource attribute |
| `uptime_s` | Gauge metric `vyoma.module.uptime_seconds` |
| `mem_kb` | Gauge metric `vyoma.module.memory_kb` |
| `status` | Enum as `vyoma.module.status` string attribute on a log record |
| `last_error` | Log record body when non-null, severity `WARN` |

### Activation

OpenTelemetry export will be activated by setting `tier = "otel"` in the platform profile and providing a collector endpoint:

```toml
[observability]
tier = "otel"
heartbeat_interval_s = 30
otel_endpoint = "http://otel-collector:4318"
```

This tier will be implemented as an optional Cargo feature (`--features otel`) to avoid pulling the OTLP SDK into constrained platform builds.

### Not Yet Implemented

OpenTelemetry export is not implemented in the current codebase. The `full` tier emits JSON-line heartbeats and does not yet forward to an OTLP collector. This section describes the interface contract that future implementation must conform to.

---

## See Also

- `supervisor/src/observability/heartbeat.rs` — implementation
- `supervisor/src/profile/profiles/*.toml` — per-platform heartbeat interval configuration
- [docs/ota-updates.md](ota-updates.md) — how health status affects OTA rollback decisions
- [docs/testing-strategy.md](testing-strategy.md) — Layer 8 performance baselines including heartbeat delivery timing

# VyomaOS OTA Updates

**Spec**: `specs/043-universal-modular-os/` | **Date**: 2026-05-25

---

## Overview

VyomaOS uses an A/B slot model for over-the-air (OTA) updates of individual WASM modules. Each app has two slots: slot A (currently active) and slot B (staging). The supervisor deploys a new binary to slot B, runs a health check, and commits slot B as active only if the new version proves healthy within the configured window. If the health check fails, the supervisor automatically rolls back to slot A with no manual intervention.

This model guarantees that a deployed device is never left in an unrecoverable state — slot A is not modified until slot B has been validated.

---

## A/B Slot Mechanism

### Slot States

```
┌─────────┐         ┌─────────┐
│  Slot A │         │  Slot B │
│ (active)│         │(staging)│
└─────────┘         └─────────┘
    ↑                    ↑
currently running    new version being
 binary (safe)        evaluated
```

Each app maintains two binary paths on persistent storage:
- `/data/ota/<app>/slot-a/<app>.wasm` — the known-good version
- `/data/ota/<app>/slot-b/<app>.wasm` — the new candidate version

A file `/data/ota/<app>/active` contains the string `a` or `b` to record which slot is currently active. The supervisor reads this at boot to determine which binary to execute.

### State Machine

```
IDLE
  │ ota-update command received
  ▼
DEPLOYING       ← download + hash-verify new binary → write to slot B
  │
  ▼
HEALTH_CHECK    ← spawn new binary from slot B, monitor for health_check_secs
  │                 ├─ module emits stdout within watchdog window → healthy tick
  │                 └─ module exits / silent timeout → unhealthy tick
  ├─ healthy for full window ──→ COMMIT   ← write "b" to active file, slot A = backup
  └─ unhealthy detected       ──→ ROLLBACK ← terminate slot B, restore slot A
```

---

## IPC Command

OTA updates are triggered via the supervisor IPC broker:

```
@supervisor: ota-update <name> <path>
```

| Parameter | Description |
|-----------|-------------|
| `<name>` | App name as declared in `vyoma.toml` `[app].name` |
| `<path>` | Absolute path to the new `.wasm` binary on the device filesystem |

### Examples

```bash
# Trigger OTA from inside the VM shell (apps/shell.wasm):
@supervisor: ota-update sensor-reader /data/sensor-reader-v2.wasm

# Using echo from a running WASM app's stdout:
println!("@supervisor: ota-update my-app /data/my-app-v2.wasm");
```

The supervisor responds with a structured log event:
```
[ota] deploying sensor-reader: slot B <- /data/sensor-reader-v2.wasm
[ota] sensor-reader: hash verified sha256:abc123...
[ota] sensor-reader: slot B spawned, health check window 60s
```

---

## Health Check Window

The health check window is the duration the supervisor monitors the new module version before committing it as the active slot. It is configurable per platform profile:

```toml
# supervisor/src/profile/profiles/iot-edge.toml
[ota]
enabled = true
health_check_secs = 60
ab_slots = true
```

| Profile | Default `health_check_secs` | OTA enabled |
|---------|---------------------------|-------------|
| `mcu-minimal` | 60 | false (no persistent storage) |
| `iot-edge` | 60 | true |
| `robotics-rt` | 30 | true |
| `mobile` | 60 | true |
| `desktop-full` | 60 | true |
| `server-headless` | 60 | true |

### What "Healthy" Means

During the health check window, the supervisor evaluates the new module as healthy if:

1. The module does not exit (non-zero exit code = immediately unhealthy).
2. The module produces at least one line of stdout within every watchdog interval (if `watchdog_secs` is set in the manifest).
3. The module does not emit `VYOMA_STATUS:unhealthy` on stdout (apps can self-report).

If all conditions hold for the full `health_check_secs` window, the module is committed.

---

## What "Rollback" Means

When a rollback is triggered (health check fails or is explicitly requested), the supervisor:

1. Terminates the slot B process immediately (SIGKILL).
2. Removes the slot B binary from `/data/ota/<app>/slot-b/`.
3. Writes `a` back to `/data/ota/<app>/active`.
4. Restarts the slot A binary (the previous known-good version).
5. Emits a structured log event:

```
[ota] sensor-reader: health check FAILED (exit code 1 after 5s)
[ota] sensor-reader: rolling back to slot A
[ota] sensor-reader: slot A restored, module restarted
```

Slot A is **never modified** during an OTA operation. If the device loses power during deployment to slot B, the next boot reads `active = a` and starts the last known-good version.

---

## Explicit Rollback Command

A rollback can also be triggered manually before the health check window expires:

```
@supervisor: ota-rollback <name>
```

This immediately aborts the health check and restores slot A, equivalent to an automatic rollback.

---

## OTA Update Workflow — Step by Step

### Prerequisites

- The new `.wasm` binary must already be present on the device at an accessible path (e.g., copied to `/data/` via the 9P virtio share or downloaded via a network-capable WASM app).
- The app must have `filesystem = true` or the path must be in a location the supervisor can access directly.

### Full Example

```bash
# Step 1: Build the new version of your app on the host
cd apps/sensor-reader
# ... make changes to src/main.rs ...
cargo build --target wasm32-wasip2 --release

# Step 2: Copy the new binary to the VM's persistent data directory
cp target/wasm32-wasip2/release/sensor-reader.wasm data/sensor-reader-v2.wasm

# Step 3: Start the VM (the data/ directory is mounted at /data inside the VM)
make run

# Step 4: Inside the VM, trigger the OTA update
@supervisor: ota-update sensor-reader /data/sensor-reader-v2.wasm

# Step 5: Watch the supervisor log for health check progress
# (supervisor writes to serial console / stderr)
# [ota] sensor-reader: slot B spawned, health check window 60s
# [ota] sensor-reader: tick 10s healthy
# [ota] sensor-reader: tick 30s healthy
# [ota] sensor-reader: tick 60s healthy — COMMITTING slot B
# [ota] sensor-reader: active = b

# Step 6 (if the new version is bad): automatic rollback
# [ota] sensor-reader: exit code 1 — UNHEALTHY
# [ota] sensor-reader: rolling back to slot A
# [ota] sensor-reader: slot A restored
```

### Verifying the Active Slot

Inside the VM:
```bash
# Check which slot is active for an app (if shell app has filesystem access):
cat /data/ota/sensor-reader/active
# → b  (after successful commit)
# → a  (after rollback or before first OTA)
```

### OTA with Hash Verification

Before deploying to slot B, the supervisor verifies the SHA-256 hash of the new binary. The expected hash can optionally be passed as a third argument:

```
@supervisor: ota-update sensor-reader /data/sensor-reader-v2.wasm sha256:abc123...
```

If the hash does not match, the deployment is rejected immediately (no health check window started, slot A is unaffected):

```
[ota] sensor-reader: hash mismatch — expected sha256:abc123... got sha256:def456...
[ota] sensor-reader: deployment REJECTED, slot A unchanged
```

If no hash is provided, the supervisor computes and logs the hash but does not enforce it:

```
[ota] sensor-reader: hash sha256:abc123... (not verified — no expected hash provided)
```

---

## Partial Transfer Recovery

If the new binary is incomplete (e.g., network transfer interrupted), the supervisor detects this at the hash-verification step or when Wasmtime fails to parse the WASM module header. In either case, the deployment is rejected and slot A is preserved:

```
[ota] sensor-reader: failed to parse wasm module (truncated?) — deployment REJECTED
[ota] sensor-reader: slot A unchanged
```

---

## Implementation Location

- `supervisor/src/ota/mod.rs` — `OtaManager`, update orchestration
- `supervisor/src/ota/ab_slot.rs` — `AbSlot` struct, slot state machine
- `supervisor/src/ota/health_check.rs` — health check monitoring and timeout logic

The OTA subsystem is enabled when `[ota] enabled = true` in the platform profile. On MCU profiles (`mcu-minimal`), OTA is disabled because there is no persistent filesystem for slot storage.

---

## See Also

- `supervisor/src/ota/` — implementation
- [docs/observability.md](observability.md) — heartbeat health status feeds into OTA health check
- [docs/testing-strategy.md](testing-strategy.md) — OTA rollback acceptance test checklist (Section 6)
- `specs/043-universal-modular-os/spec.md` — FR-007, FR-020 functional requirements, SC-008, SC-011 success criteria

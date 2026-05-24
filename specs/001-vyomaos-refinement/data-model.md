# Data Model: VyomaOS Production Refinement

**Branch**: `001-vyomaos-refinement` | **Date**: 2026-05-24

---

## Entity: AppManifest

Parsed representation of a `vyoma.toml` file. Exists at supervisor startup, one per app.

| Field | Type | Constraints |
|---|---|---|
| `app.name` | `String` | Non-empty; unique across all loaded manifests (first-registered wins) |
| `app.version` | `String` | Non-empty; semver format preferred but not enforced |
| `app.wasm` | `String` | Non-empty; resolved relative to manifest directory |
| `app.wasm_sha256` | `Option<String>` | If present: 64-char lowercase hex; verified before spawn |
| `capabilities` | `Capabilities` | See Capabilities entity |
| `window` | `Option<WindowRegion>` | If present: x, y, w, h all ≥ 0 |

**Validation rules**:
- Unknown fields in `[capabilities]` → parse error (enforced by `#[serde(deny_unknown_fields)]`)
- `app.name` collision with already-registered app → reject second, log ERROR
- `wasm_sha256` present but wasm binary missing → reject, log ERROR
- `watchdog_secs` must be `u32` (≥ 0); negative values cannot be represented

**State transitions**:
```
Unloaded → Parsed (manifest file read + deserialized) 
         → Validated (capabilities checked, name unique, wasm exists)
         → Rejected (any validation failure — terminal state for that app)
         → Spawned (wasmtime process launched)
         → Running | Exited | Restarting (lifecycle states, post-spawn)
```

---

## Entity: Capabilities

The set of permissions declared by an app. All fields default to `false`/`0`.

| Field | Type | Default | Effect when true/non-zero |
|---|---|---|---|
| `stdio` | `bool` | `false` | App inherits supervisor stdin/stdout pipes |
| `filesystem` | `bool` | `false` | App gets `/data` 9P mount |
| `network` | `bool` | `false` | App gets WASI socket support |
| `network_port` | `Option<u16>` | `None` | Specific port for network-capable app (0 = any) |
| `display` | `bool` | `false` | App can emit `VYOMA_DRAW:` commands |
| `shell` | `bool` | `false` | App can issue `@supervisor:` commands |
| `mouse` | `bool` | `false` | App receives `VYOMA_INPUT:mouse:` events |
| `watchdog_secs` | `u32` | `0` | Kill app if silent for N seconds; 0 = disabled |

**Invariants**:
- Any field name not in this table → validation error (enforced at parse time)
- `watchdog_secs = 0` is the only valid "disabled" value; no negative sentinel

---

## Entity: StructuredLogLine

One line emitted to stderr by the supervisor. Never written to disk by the supervisor (apps write their own logs to `/data/logs/`).

| Field | Type | Format |
|---|---|---|
| `timestamp` | `String` | ISO 8601: `YYYY-MM-DDTHH:MM:SS.mmmZ` |
| `level` | `enum` | `INFO` / `WARN` / `ERROR` (fixed-width 5 chars with trailing space) |
| `subsystem` | `enum` | `manifest` / `capability` / `lifecycle` / `ipc` / `display` / `input` |
| `app` | `Option<String>` | Present when event relates to a specific app; omitted for system-wide events |
| `message` | `String` | Human-readable description; no newlines; no app payload content |

**Format**: `[{timestamp}] [{level}] [{subsystem}] app={app}  {message}` (app field omitted when None)

**Invariants**:
- Message MUST NOT contain IPC message payload (log injection prevention)
- Message MUST NOT contain user-supplied content from app stdout without sanitization
- Every app spawn, exit, IPC route decision, and capability grant/deny MUST produce exactly one log line

---

## Entity: SmokeTestResult

The pass/fail output of `make test` (the integration smoke test).

| Field | Type | Description |
|---|---|---|
| `outcome` | `enum` | `PASS` or `FAIL` |
| `ready_signal_found` | `bool` | Whether the supervisor "all apps spawned" line appeared in serial output |
| `elapsed_secs` | `f32` | Seconds from QEMU start to ready signal (or timeout) |
| `timeout_secs` | `u32` | Configured timeout (30 by default) |
| `exit_code` | `i32` | 0 on PASS, non-zero on FAIL |

**Outcome rules**:
- `PASS`: `ready_signal_found = true` within `timeout_secs`
- `FAIL (timeout)`: `elapsed_secs >= timeout_secs` with no ready signal
- `FAIL (error)`: QEMU exits non-zero before ready signal
- `FAIL (kernel panic)`: "Kernel panic" string found in serial output

---

## Entity: ManifestValidationError

Structured error produced by `make check-manifests` (pre-spawn manifest scan).

| Field | Type | Description |
|---|---|---|
| `app_path` | `String` | Relative path to the `vyoma.toml` that failed |
| `app_name` | `Option<String>` | App name if parseable from the file |
| `error_kind` | `enum` | `UnknownField` / `MissingField` / `InvalidValue` / `DuplicateName` / `ParseError` |
| `field` | `Option<String>` | Which field triggered the error |
| `message` | `String` | Human-readable description with suggested fix |

**Output format** (one line per error):
```
ERROR apps/bad-app/vyoma.toml [bad-app] UnknownField: capabilities.network_v2 — did you mean 'network'?
ERROR apps/dup-app/vyoma.toml [dup-app] DuplicateName: 'hello-world' already registered by apps/hello-world/vyoma.toml
```

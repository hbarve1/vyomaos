# Kernel & Process Model — Architect Design

> Author: Architect agent
> Target: VyomaOS as macOS-equivalent desktop OS
> Anchor: existing `supervisor/src/main.rs` `AppState`, `AppStatus`, `lifecycle.rs`, `manifest.rs`
> Constraints: Rust PID-1 supervisor, wasm32-wasip2 apps, Wasmtime runtime, no native fork/exec from apps
> Code-size rule: every new `.rs` file must stay ≤500 lines

---

## 1. Process Identity Model

macOS uses reverse-DNS bundle IDs (`com.apple.finder`) keyed at install time and a transient PID at runtime. VyomaOS needs the same split: a **stable identity** (what app is this, regardless of when it ran) and a **runtime handle** (this specific running copy).

### 1.1 AppIdentity (stable, install-time)

Lives in `supervisor/src/identity.rs` (new file, ~180 LOC).

```rust
/// Stable identity of an installed app. Computed once at install/boot,
/// persisted in `/data/registry/<bundle_id>.toml`, and never mutated after.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct AppIdentity {
    /// Reverse-DNS bundle ID. Globally unique. e.g. "os.vyoma.notes".
    /// Validated as: 2-5 dot-separated lowercase alphanumeric segments.
    pub bundle_id: BundleId,

    /// Short human-readable name shown in dock/menu bar/window title.
    /// 1-32 chars. e.g. "Notes".
    pub display_name: String,

    /// Semver string from manifest [app] version. e.g. "1.4.0".
    pub version: String,

    /// SHA-256 of the .wasm binary, hex-lowercase. 64 chars.
    /// Used for code-signing/integrity check at every spawn.
    pub wasm_sha256: WasmDigest,

    /// Absolute path to the .wasm binary inside the VM rootfs.
    /// e.g. "/apps/notes/notes.wasm" or "/data/apps/notes-1.4.0/notes.wasm".
    pub wasm_path: PathBuf,

    /// Absolute path to the parsed manifest (vyoma.toml).
    pub manifest_path: PathBuf,

    /// Capabilities declared at install time. Frozen — cannot be changed
    /// by a running instance.
    pub capabilities: Capabilities,

    /// Source of the install. Determines update channel and trust level.
    pub origin: InstallOrigin,

    /// When this AppIdentity was registered (Unix seconds).
    pub installed_at: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct BundleId(String);

impl BundleId {
    /// Parse + validate. Returns Err if not RDN format.
    pub fn parse(s: &str) -> Result<Self, IdentityError> { /* … */ }
    pub fn as_str(&self) -> &str { &self.0 }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct WasmDigest([u8; 32]);

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum InstallOrigin {
    /// Shipped in the initramfs. Cannot be uninstalled.
    System,
    /// Installed via package manager from /data/apps/.
    User { source_url: Option<String> },
    /// Dev-mode app loaded from a host mount.
    Developer,
}
```

### 1.2 Mapping from WASM binary + manifest

The flow at install:

```
vyoma.toml ── parse_manifest ──► AppManifest
   │
   ├── [app] bundle_id ───────────► BundleId::parse
   ├── [app] name      ───────────► display_name
   ├── [app] version   ───────────► version
   ├── [app] wasm      ───────────► wasm_path resolution
   │                                   sha256(file)  → wasm_sha256
   └── [capabilities]  ───────────► capabilities (frozen)
```

The `bundle_id` field is **added** to the existing `AppMeta` struct in `manifest.rs`. Manifests missing `bundle_id` fall back to `os.vyoma.unsigned.<name>` for backwards compatibility, but emit a warning at boot.

### 1.3 Live instances vs installed apps

Two registries, both owned by the supervisor:

```rust
/// Installed = known to the system, may or may not be running.
/// Persisted at /data/registry/index.toml + per-app TOML files.
pub struct InstallRegistry {
    by_bundle: HashMap<BundleId, AppIdentity>,
}

/// Live = currently executing in a Wasmtime child. Cleared on supervisor
/// restart, rebuilt from auto-launch list.
pub struct InstanceTable {
    by_instance: HashMap<InstanceId, Arc<Mutex<AppState>>>,
    by_bundle:   HashMap<BundleId, Vec<InstanceId>>,  // multi-instance index
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct InstanceId(u64);  // monotonically increasing, starts at 1
```

`InstanceTable::by_bundle` is the **multi-instance index** — for any `BundleId` it returns every live instance, in launch order. Used by the dock for "show all windows of Notes" and by IPC routing `@notes:` to pick a target.

---

## 2. Process Lifecycle State Machine

Current supervisor `AppStatus` is binary: `Running` / `Stopped(i32)`. That's insufficient for a desktop OS. The replacement (`supervisor/src/lifecycle/state.rs`, ~250 LOC) is:

```rust
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AppStatus {
    /// Identity known, not in the process table. Default for installed-but-unlaunched apps.
    NotRunning,

    /// Spawn in flight: wasmtime child created, WASI imports being wired,
    /// app's `_start` not yet returned from its first `VYOMA_DRAW:flush`.
    /// Transitions to Running on first flush, or Crashed on early exit/timeout.
    Launching { since: Instant, child_pid: u32 },

    /// App is executing and serviced by the scheduler. Has a window surface
    /// (if display=true) and a live IPC inbox.
    Running { since: Instant, child_pid: u32 },

    /// App has a window but is not the frontmost app. Still consumes CPU
    /// quota, still receives IPC, but mouse/keyboard events only flow if
    /// they target its window region.
    Background { since: Instant, child_pid: u32 },

    /// App has no visible window and has been parked: no CPU quota, no
    /// timer callbacks, no IPC delivery (queued). Surface is preserved.
    /// Transitions back to Running on user click or programmatic wake.
    Suspended { since: Instant, child_pid: u32, reason: SuspendReason },

    /// Supervisor sent SIGTERM and is waiting up to 2 s for clean exit.
    Terminating { since: Instant, child_pid: u32 },

    /// Clean exit (exit code 0 or any code policy considers normal).
    Terminated { exit_code: i32, at: Instant },

    /// Wasmtime trap, watchdog kill, or non-zero exit not classified as clean.
    /// Carries crash details for the crash reporter.
    Crashed { exit_code: i32, at: Instant, report_id: CrashReportId },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SuspendReason {
    UserMinimized,
    SystemMemoryPressure,
    AppRequested,            // explicit @supervisor:suspend
    QuotaExceeded,           // hit CPU/mem ceiling
}
```

### 2.1 State transitions

| From        | To           | Trigger                                            | Side effects |
|-------------|--------------|----------------------------------------------------|---------------|
| NotRunning  | Launching    | `launch(bundle_id)` IPC or auto-launch list        | spawn wasmtime child, allocate `InstanceId`, create surface placeholder |
| Launching   | Running      | first `VYOMA_DRAW:flush` or 500 ms grace           | mark surface visible, push to z-order, emit `lifecycle.running` event |
| Launching   | Crashed      | child exits before flush, or 5 s timeout            | write crash report, surface destroyed, restart policy evaluated |
| Running     | Background   | another app gains focus                            | drop input subscriptions, IPC still flows, surface kept |
| Background  | Running      | window clicked / `@supervisor: focus <bundle>`     | restore input, raise z-order |
| Running/Bg  | Suspended    | minimize, memory pressure, quota exceed            | snapshot wasm linear-memory page count, deregister timer callbacks, IPC inbox switches to bounded buffer (max 64 msgs) |
| Suspended   | Running      | dock click, programmatic wake, IPC inbox overflow  | re-register timers, drain inbox, raise window |
| any-live    | Terminating  | `kill <bundle>`, app exit request, shutdown        | SIGTERM, 2 s grace, then SIGKILL |
| Terminating | Terminated   | child reaped, exit code in clean set               | release surface, IPC subscriptions, file handles, peripheral leases |
| Terminating | Crashed      | timed out, killed via SIGKILL, or non-clean exit   | crash report written, restart policy evaluated |
| Crashed     | Launching    | restart policy = always or on-crash, backoff elapsed | new `InstanceId` (old one is dead), surface re-created |
| Crashed     | NotRunning   | restart policy = never or backoff exhausted        | crash log retained, no auto-restart |
| Terminated  | NotRunning   | grace period of 30 s for state-restoration handshake elapses | flush state snapshot to `/data/state/<bundle>.toml` |

### 2.2 What happens to each resource on transition

| Resource              | Launching        | Running   | Background | Suspended | Terminating | Crashed/Terminated |
|-----------------------|------------------|-----------|------------|-----------|-------------|---------------------|
| Framebuffer Surface   | allocated, not flushed | live, blitted in compositor | live, blitted unless covered | retained, **not** blitted | retained until child reaped | freed; final frame may persist as snapshot in crash report |
| IPC inbox             | created          | drained per-tick | drained per-tick | bounded buffer (64), no wake | flush, then close | closed, queued messages dropped with NACK |
| File handles (`/data`)| opened lazily    | live      | live       | flushed + closed (re-opened on wake) | flushed | closed |
| Peripheral leases (GPIO/I2C) | acquired   | held      | held       | released (re-acquired on wake) | released | released |
| Timer callbacks       | registered       | dispatched | dispatched | **paused** (deadlines extended by suspend duration) | cancelled | cancelled |
| WASM linear memory    | resident         | resident  | resident   | resident (cannot swap WASM mem under Wasmtime) | resident | freed |
| Mouse/keyboard subs   | none             | full      | hit-test only | none | none | none |

---

## 3. Multi-Instance Model

macOS allows multiple copies of Notes, Safari, etc. VyomaOS supports it via the `InstanceId` / `BundleId` split introduced above plus a manifest opt-out.

### 3.1 Manifest field

In `vyoma.toml` `[app]` table:

```toml
[app]
bundle_id     = "os.vyoma.notes"
multi_instance = true        # default false — same as macOS singletons
max_instances = 16           # optional cap; supervisor enforces
```

Default is `false` because most desktop apps are singletons (System Settings, Finder); the dock + window picker get simpler when there's one window per bundle. Notes, Terminal, Calculator, and the WASM REPL declare `multi_instance = true`.

### 3.2 Supervisor handling of repeat launch

```rust
pub fn handle_launch_request(
    table: &mut InstanceTable,
    registry: &InstallRegistry,
    bundle: &BundleId,
    args: LaunchArgs,
) -> Result<InstanceId, LaunchError> {
    let identity = registry.by_bundle.get(bundle).ok_or(LaunchError::NotInstalled)?;
    let existing = table.by_bundle.get(bundle).cloned().unwrap_or_default();

    if !identity.multi_instance && !existing.is_empty() {
        // Singleton: raise the existing window, deliver args as IPC
        let iid = existing[0];
        focus_instance(table, iid)?;
        deliver_open_args(table, iid, args)?;
        return Ok(iid);
    }

    if existing.len() >= identity.max_instances as usize {
        return Err(LaunchError::InstanceLimit(identity.max_instances));
    }

    spawn_new_instance(table, identity, args)
}
```

### 3.3 Instance IDs vs app IDs in routing

- `BundleId`: stable, install-time, used for capabilities, dock icon, app-store updates.
- `InstanceId`: u64, allocated by supervisor at spawn, used for window events, focus tracking, IPC delivery.

IPC addressing is extended:

```
@<bundle_id>: msg              → broadcast to all live instances
@<bundle_id>#<instance_id>: msg → unicast to one instance
@@focused: msg                 → unicast to whichever instance owns focus
```

The router (`supervisor/src/router.rs`, today routes by app *name*) is rewritten to route by `(BundleId, Option<InstanceId>)`. Window manager events (mouse, keyboard) always carry an `InstanceId` because the framebuffer compositor knows which surface was hit.

### 3.4 Window manager event routing

```rust
pub struct WindowEvent {
    pub instance: InstanceId,           // which window
    pub bundle:   BundleId,             // for capability checks
    pub kind:     WindowEventKind,      // Click | KeyDown | Resize | Close | …
    pub at:       Instant,
}
```

The compositor (`display/compositor.rs`) holds a per-pixel `Vec<InstanceId>` ownership map for the framebuffer (sparse, only window rects). A click at `(x, y)` looks up `instance_id_at(x, y)` and delivers `WindowEvent` to that exact instance's IPC inbox. This is unambiguous even when 4 Notes windows tile the screen.

---

## 4. Background Execution Model

macOS apps run network fetches, timers, and notifications even when not frontmost via `NSBackgroundActivityScheduler`, `NSURLSession` background tasks, and `dispatch_after`. WASM apps under Wasmtime cannot block on syscalls in a backgrounded process, so VyomaOS needs **supervisor-driven background dispatch**.

### 4.1 Data model

`supervisor/src/lifecycle/background.rs` (~200 LOC):

```rust
#[derive(Clone, Debug)]
pub struct BackgroundTask {
    pub id:           BgTaskId,           // u64, allocated per registration
    pub instance:     InstanceId,         // owner; cancelled if instance dies
    pub bundle:       BundleId,           // capability gate
    pub kind:         BackgroundKind,
    pub deadline_ms:  u64,                // monotonic deadline (boot-relative ms)
    pub period_ms:    Option<u64>,        // Some = repeating, None = one-shot
    pub wasm_export:  String,             // exported function to invoke; "" = stdin message
    pub payload:      Vec<u8>,            // ≤ 4 KiB, copied into linear memory
    pub remaining_budget_us: i64,         // CPU budget for this task; if ≤0, dropped
}

#[derive(Clone, Debug)]
pub enum BackgroundKind {
    /// Run once at `deadline_ms`. e.g. notification arming.
    OneShot,
    /// Re-arm every `period_ms`. e.g. "fetch mail every 5 min".
    Periodic,
    /// Long-running: app stays Background, scheduler keeps a slot reserved.
    Service,
    /// Wake on network input on `port`. Supervisor listens, hands off when packet arrives.
    NetworkWake { port: u16 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct BgTaskId(pub u64);
```

### 4.2 How supervisor calls a WASM export without spawning

The supervisor keeps the `wasmtime::Store` per instance **alive** even when the instance is `Suspended` or `Background`. Calling a registered export is a normal Wasmtime function call into the same store; no new process is spawned.

```rust
pub fn dispatch_background(
    store: &mut wasmtime::Store<AppCtx>,
    instance: &wasmtime::Instance,
    task: &BackgroundTask,
) -> Result<(), BgError> {
    // Inject payload into a guest-allocated buffer.
    let alloc = instance.get_typed_func::<u32, u32>(&mut *store, "vyoma_bg_alloc")?;
    let ptr = alloc.call(&mut *store, task.payload.len() as u32)?;
    let memory = instance.get_memory(&mut *store, "memory").unwrap();
    memory.write(&mut *store, ptr as usize, &task.payload)?;

    let entry = instance.get_typed_func::<(u32, u32), ()>(&mut *store, &task.wasm_export)?;
    // Apply per-task fuel budget so a runaway timer can't starve the foreground.
    store.set_fuel(task.remaining_budget_us as u64 * 1_000)?;  // 1 fuel ≈ 1 ns
    entry.call(&mut *store, (ptr, task.payload.len() as u32))?;
    Ok(())
}
```

For apps that don't want function exports, the fallback is to write the message to the app's stdin (current path); the app's main loop handles it next tick.

### 4.3 Registration protocol

Apps register tasks via IPC commands consumed by `ipc_commands/` handlers:

```
@supervisor: bg_register kind=periodic period_ms=300000 export=on_timer payload_b64=…
@supervisor: bg_cancel id=42
@supervisor: bg_list
```

Supervisor replies on the app's stdin:
```
VYOMA_BG: registered id=42
VYOMA_BG: fired id=42 reason=deadline
```

### 4.4 Background task quota per app

Quotas live alongside resource quotas (§6):

```rust
pub struct BackgroundQuota {
    pub max_concurrent_tasks: u8,        // default 8
    pub max_total_budget_ms_per_min: u32,  // default 2000 ms = 3.3% CPU
    pub min_period_ms: u32,              // default 5_000 — no busy timers
    pub allowed_kinds: u8,               // bitset of BackgroundKind variants
}
```

Supervisor refuses new registrations exceeding `max_concurrent_tasks`. Per-minute budget is enforced by deducting `remaining_budget_us` after each dispatch and re-charging once per wall-minute. Exhausted tasks transition to `Suspended` until next charge.

---

## 5. Crash & Restart Policy

### 5.1 Crash detection sources

```rust
#[derive(Clone, Debug)]
pub enum CrashCause {
    /// Wasmtime returned a Trap (unreachable, OOB, divide-by-zero).
    WasmTrap { trap_code: String, backtrace: Vec<String> },

    /// Child exited with a non-zero exit code outside the clean-exit set.
    NonZeroExit { code: i32 },

    /// Watchdog: no `VYOMA_DRAW:flush` or IPC message for watchdog_secs.
    Watchdog { silent_for_ms: u64 },

    /// Fuel exhausted under a hard fuel budget.
    FuelExhausted { task: Option<BgTaskId> },

    /// Memory quota exceeded; Wasmtime refused linear-memory growth.
    MemoryLimit { requested_pages: u32, ceiling: u32 },

    /// SIGTERM grace expired; supervisor used SIGKILL.
    KillTimeout,

    /// External signal from the host kernel (rare; e.g. OOM-killer).
    HostSignal { signo: i32 },
}
```

Detection point in supervisor:
- `WasmTrap` and `FuelExhausted` surface as `Err(wasmtime::Trap)` from `Func::call`.
- `NonZeroExit` from `Child::wait`.
- `Watchdog` from the existing `last_output` timer (kept).
- `MemoryLimit` from the `ResourceLimiter` hook on the store.
- `KillTimeout` from the `Terminating` state's 2 s grace timer.
- `HostSignal` from the SIGCHLD reaper.

### 5.2 Crash log format

`supervisor/src/lifecycle/crash_report.rs` (~220 LOC). One TOML file per crash, written atomically to `/data/crashes/<unix_ts>-<bundle>-<instance>.toml`:

```toml
schema_version = 1
report_id      = "crash-20260529T184412Z-os.vyoma.notes-42"
bundle_id      = "os.vyoma.notes"
instance_id    = 42
wasm_sha256    = "9f3c…"
version        = "1.4.0"
crashed_at     = 1748541852          # unix seconds
uptime_secs    = 312
restart_count  = 0

[cause]
kind         = "wasm_trap"
trap_code    = "unreachable"
backtrace    = [
  "notes::editor::insert_char (notes.wasm+0x1a3c)",
  "notes::main::loop          (notes.wasm+0x0921)",
  "_start                     (notes.wasm+0x0042)",
]

[runtime]
linear_mem_pages = 7
fuel_consumed    = 4823914
open_files       = 2
ipc_inbox_len    = 0

[last_logs]
lines = [
  "[2026-05-29T18:44:12.012Z] [notes] saving draft",
  "[2026-05-29T18:44:12.087Z] [notes] PANIC: out of bounds at index 4096",
]

[recovery]
action      = "restart"           # or "abandon" / "user_prompt"
backoff_ms  = 2000
attempt     = 1
```

The supervisor exposes `@supervisor: crashes list` and `@supervisor: crashes show <report_id>` over IPC for the crash reporter app.

### 5.3 Per-app restart policy

Manifest field replaces the current free-string `restart` in `BootEntry`:

```toml
[lifecycle]
restart = "on-crash"            # never | always | on-crash | exponential-backoff
max_restarts = 5                # 0 = unlimited
backoff_initial_ms = 1000
backoff_max_ms = 60000
state_restoration = true        # if true, signal app it's being relaunched
clean_exit_codes = [0, 75]      # any other non-zero code = crash
```

Parsed into:

```rust
#[derive(Clone, Debug)]
pub struct RestartPolicy {
    pub kind: RestartKind,
    pub max_restarts: u32,
    pub backoff: ExponentialBackoff,
    pub clean_exit_codes: SmallVec<[i32; 4]>,
    pub state_restoration: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RestartKind { Never, Always, OnCrash, ExponentialBackoff }

#[derive(Clone, Debug)]
pub struct ExponentialBackoff {
    pub initial_ms: u32,
    pub max_ms: u32,
    pub current_ms: u32,
    pub attempt: u32,
}
```

Decision matrix (replaces `lifecycle::should_restart`):

| Cause              | Never | Always | OnCrash | ExpBackoff |
|--------------------|-------|--------|---------|------------|
| Clean exit         | no    | yes    | no      | no         |
| Crashed            | no    | yes    | yes     | yes (until max) |
| KillTimeout        | no    | yes    | yes     | yes        |
| User `kill`        | no    | no     | no      | no         |
| Shutdown           | no    | no     | no      | no         |

### 5.4 State restoration handshake

When supervisor decides to relaunch after a crash, it:

1. Reads the crash report's `last_logs` and writes them into `/data/state/<bundle>.last_crash`.
2. Sets env var `VYOMA_RESTORE=1` on the new wasmtime child.
3. The supervisor sends, as the first stdin line after spawn:
   ```
   VYOMA_SYSTEM:restore:bundle=os.vyoma.notes instance=43 prior_instance=42 reason=wasm_trap
   ```
4. The app reads its state snapshot from `/data/state/<bundle>.toml` (apps write this themselves via `@supervisor: state_save <toml>`) and resumes.

Apps that don't support restoration simply ignore the `VYOMA_SYSTEM:restore:` line and cold-start; nothing breaks.

---

## 6. Resource Limits Per App

### 6.1 Quota struct

`supervisor/src/lifecycle/quota.rs` (~220 LOC):

```rust
#[derive(Clone, Debug)]
pub struct ResourceQuota {
    /// Hard ceiling on `memory.grow`. 1 page = 64 KiB. Default 256 pages = 16 MiB.
    pub wasm_memory_pages: u32,

    /// CPU budget in milliseconds per real-world second.
    /// 100 = full single core, 50 = throttled, 1000 = up to 10 cores.
    pub cpu_time_ms_per_sec: u32,

    /// Max open WASI file descriptors against /data.
    pub open_files: u16,

    /// Max simultaneous TCP/UDP sockets (requires capabilities.network).
    pub network_connections: u16,

    /// Max bytes per second for both /data writes and network egress.
    pub io_bytes_per_sec: u32,

    /// Max BackgroundTask quotas (see §4).
    pub background: BackgroundQuota,

    /// Max framebuffer surface area (pixels). Default 4 MP = 2048×2048.
    pub max_surface_pixels: u32,
}

impl Default for ResourceQuota {
    fn default() -> Self {
        Self {
            wasm_memory_pages: 256,
            cpu_time_ms_per_sec: 100,
            open_files: 32,
            network_connections: 8,
            io_bytes_per_sec: 4 * 1024 * 1024,
            background: BackgroundQuota::default(),
            max_surface_pixels: 4 * 1024 * 1024,
        }
    }
}
```

Quotas are declared in the manifest:

```toml
[quota]
wasm_memory_pages   = 1024     # 64 MiB
cpu_time_ms_per_sec = 200      # 2 cores worth
open_files          = 64
network_connections = 32
io_bytes_per_sec    = 16777216
max_surface_pixels  = 8294400  # 4K window allowed
```

System apps (Finder, Dock, MenuBar) declare `system = true` and skip enforcement.

### 6.2 Enforcement mechanism

Three layers, chosen by what Wasmtime/Linux give us cheaply:

| Quota                | Mechanism                                                  |
|----------------------|------------------------------------------------------------|
| `wasm_memory_pages`  | `wasmtime::ResourceLimiter::memory_growing` returns `false` past ceiling. Already supported. |
| `cpu_time_ms_per_sec`| **Fuel + epoch interruption**. Set fuel to `cpu_time_ms_per_sec * 1_000_000` at the start of each wall-second; on epoch tick (10 Hz) check `consume_fuel(0)` and force a yield when budget elapsed. |
| `open_files`         | Wrap WASI `fd_open` host import; reject when count exceeds quota. |
| `network_connections`| Same: wrap socket-open import. |
| `io_bytes_per_sec`   | Token bucket per app, refilled at `io_bytes_per_sec / 100` every 10 ms. WASI read/write block on empty bucket. |
| `max_surface_pixels` | Validate at `VYOMA_DRAW:resize` and at window allocation. |
| Background quotas    | See §4.4. |

We deliberately **don't** use Linux cgroups because:
1. The whole supervisor is PID-1 in a tiny kernel without cgroup v2 support.
2. Wasmtime is in-process; one cgroup per wasmtime child is heavy and breaks fuel-based accounting.

### 6.3 What happens when an app exceeds quota

| Quota               | First exceed action            | Repeated exceed action          |
|---------------------|-------------------------------|----------------------------------|
| wasm_memory_pages   | `memory.grow` returns -1, app gets Wasmtime error | `MemoryLimit` crash if app traps; otherwise app degrades |
| cpu_time_ms_per_sec | Throttle: app sleeps until next wall-second slot | Transition to `Suspended { reason: QuotaExceeded }` after 5 consecutive seconds of exceedance |
| open_files          | `fd_open` returns `EMFILE`     | (no escalation; app handles error) |
| network_connections | `socket` returns `ENFILE`      | (no escalation) |
| io_bytes_per_sec    | Backpressure (read/write blocks) | (no escalation) |
| max_surface_pixels  | resize rejected, app gets `VYOMA_ERROR:surface_too_large` | window forced to old size |
| Background          | Registration refused with error | Active task culled at next dispatch |

All exceed events are logged with `Subsystem::Quota` so the user can see them in the management server.

---

## 7. Supervisor Internal Process Table

The full `AppState` replacement, in `supervisor/src/process_table.rs` (~360 LOC). This is the canonical record for one running instance.

```rust
pub struct AppState {
    // ─── Identity ─────────────────────────────────────────────────────────────
    pub instance_id: InstanceId,
    pub identity:    Arc<AppIdentity>,    // shared with InstallRegistry
    pub display_name_runtime: String,     // may be overridden via window title

    // ─── Lifecycle ────────────────────────────────────────────────────────────
    pub status:          AppStatus,
    pub status_history:  RingBuffer<(AppStatus, Instant), 16>,
    pub spawn_time:      Instant,
    pub last_state_change: Instant,
    pub restart_policy:  RestartPolicy,
    pub restart_count:   u32,
    pub crash_history:   RingBuffer<CrashReportId, 8>,

    // ─── Process plumbing ─────────────────────────────────────────────────────
    pub child_pid:    Option<u32>,
    pub child_stdin:  Option<ChildStdin>,
    pub child_stdout: Option<ChildStdout>,
    pub child_stderr: Option<ChildStderr>,
    pub wasmtime_store_id: StoreId,        // index into supervisor's Store arena
    pub thread_join:  Option<JoinHandle<i32>>,

    // ─── IPC ──────────────────────────────────────────────────────────────────
    pub inbox_tx:     mpsc::Sender<IpcMessage>,
    pub inbox_pending: usize,              // monitored for backpressure
    pub last_sender:  Option<BundleId>,    // for @reply routing
    pub ipc_log_level: LogLevel,

    // ─── Display ──────────────────────────────────────────────────────────────
    pub has_display:  bool,
    pub surface:      Option<Arc<Mutex<Surface>>>,
    pub win_region:   Option<WindowRegion>,
    pub win_z:        u32,
    pub min_size:     (u32, u32),
    pub minimized:    bool,
    pub pre_minimize_region: Option<WindowRegion>,
    pub pending_anim: Option<Animation>,
    pub dirty:        bool,
    pub flush_stats:  FlushStats,

    // ─── Input ────────────────────────────────────────────────────────────────
    pub has_mouse:    bool,
    pub has_keyboard: bool,                // implied by focus
    pub focus_token:  Option<FocusToken>,  // Some when this is the focused instance

    // ─── Capabilities ─────────────────────────────────────────────────────────
    pub capabilities:    Capabilities,
    pub peripheral_leases: Vec<PeripheralLease>,
    pub file_handles:    HashMap<wasi::Fd, FileHandleInfo>,
    pub network_conns:   HashMap<ConnId, NetConnInfo>,

    // ─── Watchdog ─────────────────────────────────────────────────────────────
    pub watchdog_secs:    u32,
    pub last_output:      Arc<Mutex<Instant>>,
    pub watchdog_backoff: Arc<Mutex<u64>>,

    // ─── Resource accounting ─────────────────────────────────────────────────
    pub quota:           ResourceQuota,
    pub cpu_ticks:       u64,
    pub cpu_budget_us:   i64,              // refilled per wall-second
    pub mem_pages:       u32,
    pub io_tokens:       u32,
    pub bg_tasks:        HashMap<BgTaskId, BackgroundTask>,
    pub last_cpu_reset:  Instant,

    // ─── Logging ──────────────────────────────────────────────────────────────
    pub log_buf:         VecDeque<LogLine>,   // capped at LOG_BUF_SIZE
    pub log_subscribers: Vec<mpsc::Sender<LogLine>>,
    pub log_file:        Option<File>,        // /data/logs/<bundle>-<instance>.log

    // ─── Restoration ──────────────────────────────────────────────────────────
    pub state_snapshot:  Option<StateSnapshot>,
    pub launch_args:     LaunchArgs,
}

pub struct FlushStats {
    pub flushes: u64,
    pub last_flush: Instant,
    pub avg_interval_ms: u32,
}

pub struct FocusToken;        // unit type used as a presence marker

pub struct PeripheralLease {
    pub kind: PeripheralKind,
    pub id:   u8,
    pub acquired_at: Instant,
}

pub struct FileHandleInfo { pub path: PathBuf, pub mode: u32, pub opened_at: Instant }
pub struct NetConnInfo    { pub peer: SocketAddr, pub kind: ConnKind, pub opened_at: Instant }

pub struct LogLine {
    pub at: Instant,
    pub level: LogLevel,
    pub text: String,
}
```

This struct is large; it's owned by `Arc<Mutex<AppState>>` and the supervisor's per-tick loop is careful to hold the lock only for short windows (the existing ABBA fix in commit `55fd121` is preserved).

---

## 8. Boot Sequence

Boot is staged so the user sees a desktop within ~3 seconds even on slower hardware. New file: `supervisor/src/boot.rs` (~280 LOC), replacing the inline logic in `main.rs`.

### 8.1 Stages

```
S0  Kernel handoff       (Linux drops to /init = supervisor)
S1  Early init           (parse cmdline, mount /proc /sys /dev, mount 9P /data)
S2  Profile load         (PLATFORM env → /etc/vyoma/profiles/<name>.toml)
S3  Registry load        (read /data/registry/*.toml + /etc/vyoma/apps/*/vyoma.toml)
S4  System app spawn     (unkillable, parallel)
S5  Display init         (open /dev/fb0, init compositor, draw splash)
S6  Auto-launch wave     (user-prefs apps, parallel)
S7  Login complete       (emit "lifecycle: boot_done", start input loops)
S8  On-demand idle       (the rest wait for dock click / IPC launch)
```

### 8.2 App tiers

App tier is declared in the manifest:

```toml
[lifecycle]
tier = "system"          # system | auto | on_demand | login
```

| Tier        | Spawn time | Killable? | Restart on crash | Examples |
|-------------|------------|-----------|------------------|----------|
| `system`    | S4         | no        | always (immediately) | windowserver, dock, menubar, statusbar, ipc-broker-self, crash-reporter |
| `login`     | S6 (post-login) | by user | on-crash | finder, notification-center, spotlight |
| `auto`      | S6         | yes       | per-policy | user-selected at install time (e.g. terminal, notes) |
| `on_demand` | S8 (lazy)  | yes       | per-policy | calculator, settings, app store |

### 8.3 Parallel vs sequential

- **S4 (system apps)** are spawned **in parallel** via a thread pool; they have no inter-dependencies because they all talk to the supervisor, not to each other. The supervisor waits up to 2 s for each to reach `Running`; failures are fatal (system apps are required).
- **S5 (display init)** is **sequential** after S4 because the compositor needs to know which surfaces from S4 are present.
- **S6 (auto-launch wave)** is **parallel** but rate-limited to N=4 concurrent spawns to avoid disk/CPU thrash on slow hardware.
- **S8** is purely event-driven; no spawn until needed.

Why this order:
1. System apps must own the display before any user app draws (avoids flicker).
2. The crash reporter must be alive before user apps spawn so it can collect their first crash.
3. Login-tier apps depend on the windowserver being up, hence after S5.

### 8.4 Concrete boot.toml

```toml
[boot]
profile = "desktop-full"

# Tier = system; supervisor enforces no manifest override
[[apps]]
manifest = "/etc/vyoma/apps/windowserver/vyoma.toml"
tier     = "system"

[[apps]]
manifest = "/etc/vyoma/apps/dock/vyoma.toml"
tier     = "system"

[[apps]]
manifest = "/etc/vyoma/apps/menubar/vyoma.toml"
tier     = "system"

[[apps]]
manifest = "/etc/vyoma/apps/crash-reporter/vyoma.toml"
tier     = "system"

# Login-tier
[[apps]]
manifest = "/etc/vyoma/apps/finder/vyoma.toml"
tier     = "login"

[[apps]]
manifest = "/etc/vyoma/apps/notification-center/vyoma.toml"
tier     = "login"

# User auto-launch list lives at /data/installed.txt (existing path)
```

`/data/installed.txt` is read at S6 and each line spawned as `auto` tier; unknown bundle_ids are logged and skipped.

---

## 9. Implementation Plan

### 9.1 New files

| Path | Purpose | Est. LOC |
|------|---------|----------|
| `supervisor/src/identity.rs` | `AppIdentity`, `BundleId`, `WasmDigest`, `InstallOrigin`, install registry I/O | 180 |
| `supervisor/src/instance.rs` | `InstanceId`, `InstanceTable`, multi-instance routing | 220 |
| `supervisor/src/lifecycle/mod.rs` | re-export hub (replaces flat `lifecycle.rs`) | 30 |
| `supervisor/src/lifecycle/state.rs` | new `AppStatus`, transitions, `RestartPolicy`, `ExponentialBackoff` | 250 |
| `supervisor/src/lifecycle/background.rs` | `BackgroundTask`, dispatch loop, quota enforcement | 200 |
| `supervisor/src/lifecycle/crash_report.rs` | `CrashCause`, TOML serialization, on-disk store | 220 |
| `supervisor/src/lifecycle/quota.rs` | `ResourceQuota`, enforcement hooks, token bucket | 220 |
| `supervisor/src/lifecycle/restoration.rs` | state snapshot save/load, handshake | 150 |
| `supervisor/src/process_table.rs` | full `AppState`, `FlushStats`, helper structs | 360 |
| `supervisor/src/boot.rs` | staged boot orchestrator (S0–S8) | 280 |
| `supervisor/src/router_v2.rs` | `(BundleId, Option<InstanceId>)` routing (replaces `router.rs`) | 240 |

Total new: ~2350 LOC across 11 files, each under the 500-line ceiling.

### 9.2 Modified existing files

| Path | Change | Est. delta |
|------|--------|-----------|
| `supervisor/src/main.rs` | strip `AppState`/`AppStatus`, replace with `process_table::AppState`; delegate boot to `boot::run_stages()` | −180, +40 |
| `supervisor/src/manifest.rs` | add `bundle_id`, `multi_instance`, `max_instances`, `[lifecycle]`, `[quota]` fields; validation | +120 |
| `supervisor/src/lifecycle.rs` | becomes shim re-exporting `lifecycle/state.rs`; old fns kept for now | −20 (mostly delete) |
| `supervisor/src/ipc.rs` | extend addressing to `bundle#instance`; preserve back-compat for name-based | +90 |
| `supervisor/src/ipc_handlers.rs` | `bg_register`, `bg_cancel`, `bg_list`, `state_save`, `crashes list/show` | +160 |
| `supervisor/src/windows.rs` | route window events via `InstanceId`; update `Z_ORDER` to hold `InstanceId` | +60 |
| `supervisor/src/router.rs` | thin wrapper around `router_v2::route()` for back-compat | −40 |
| `supervisor/src/mgmt_protocol.rs` | extend `AppStatus` wire enum to match new variants | +50 |
| `supervisor/src/runtime/wasmtime_adapter.rs` (existing under `runtime/`) | wire `ResourceLimiter`, fuel budgets, epoch interrupts | +110 |

Total modified delta: ~+390 LOC net (after deletions).

### 9.3 Migration order (matches Test-Driven Development)

1. **Identity layer** (`identity.rs` + manifest changes) — pure data, no I/O dependencies; unit tests trivial.
2. **Process table** (`process_table.rs`) — pure struct; replace existing `AppState` field-for-field, keep methods inert.
3. **Lifecycle state machine** (`lifecycle/state.rs`) — replace `AppStatus`; map every transition to an existing call site.
4. **Boot orchestrator** (`boot.rs`) — slice up `main.rs::main` into staged calls; smoke test confirms boot still works.
5. **Multi-instance + router v2** — gated behind `multi_instance = true` manifests so existing apps unaffected.
6. **Quota enforcement** — start with memory + open_files (already supported by Wasmtime); CPU/IO later.
7. **Background tasks** — opt-in; ship with one test app (`apps/bg-timer-demo/`).
8. **Crash reporter app** (new in `apps/crash-reporter/`) — consumes the new structured crash logs.
9. **State restoration** — opt-in flag; system apps use it.

### 9.4 Risks & mitigations

- **Risk**: `process_table::AppState` is large; lock contention could regress the perf gains from commit `c62ac27`/`55fd121`.
  **Mitigation**: split the struct internally into hot (status, region, dirty, surface) vs cold (logs, crash history, bg tasks); use two `Mutex` fields rather than one. Compositor only takes the hot lock.

- **Risk**: Fuel-based CPU budgeting requires re-instantiation tax (Wasmtime fuel is per-store).
  **Mitigation**: epoch interrupts (10 Hz tick) check elapsed wall time and yield via host trap; cheaper than per-call fuel.

- **Risk**: Background tasks running in suspended apps could hold the store lock for too long, blocking foreground draws.
  **Mitigation**: each background dispatch runs on a worker thread, owns its own store handle, and is preempted by epoch interrupt after `cpu_time_ms_per_sec` is exhausted.

- **Risk**: TOML crash reports are slow to write under crash storm.
  **Mitigation**: ring buffer of 256 reports in `/data/crashes/`, oldest evicted; writes are non-blocking via dedicated logger thread.

- **Risk**: `BundleId` change breaks existing `apps/*/vyoma.toml`.
  **Mitigation**: fallback to `os.vyoma.unsigned.<name>` at parse; emit warning; CI gate adds bundle_id to all in-tree manifests in a single follow-up PR.

---

## Appendix A — Protocol strings introduced

| String | Direction | Purpose |
|--------|-----------|---------|
| `VYOMA_SYSTEM:restore:bundle=…` | supervisor → app stdin | Notify of post-crash relaunch |
| `VYOMA_SYSTEM:focus_changed:bundle=… instance=…` | supervisor → app | Lifecycle hint for Background↔Running |
| `VYOMA_SYSTEM:suspend:reason=…` | supervisor → app | About to be suspended; flush state |
| `VYOMA_SYSTEM:resume` | supervisor → app | Back from Suspended |
| `VYOMA_BG:registered id=…` | supervisor → app | bg_register ack |
| `VYOMA_BG:fired id=… reason=…` | supervisor → app | bg deadline reached |
| `VYOMA_BG:cancelled id=…` | supervisor → app | cancellation ack |
| `VYOMA_ERROR:quota:kind=… exceeded_by=…` | supervisor → app | quota breach warning |
| `VYOMA_ERROR:surface_too_large` | supervisor → app | resize rejected |
| `@supervisor: bg_register …` | app → supervisor | register background task |
| `@supervisor: bg_cancel id=…` | app → supervisor | cancel task |
| `@supervisor: state_save <toml>` | app → supervisor | persist state for restoration |
| `@supervisor: crashes list` | app → supervisor | crash reporter query |
| `@supervisor: crashes show <report_id>` | app → supervisor | fetch a crash report |
| `@<bundle>#<instance>: …` | any → app | unicast IPC by instance |
| `@@focused: …` | any → app | unicast IPC to focused instance |

## Appendix B — On-disk layout introduced

```
/etc/vyoma/
  boot.toml                     (tier-aware version of today's)
  profiles/<name>.toml          (unchanged)
  apps/<bundle>/vyoma.toml      (system + login apps, immutable)

/data/
  registry/
    index.toml                  (bundle_id → install file)
    <bundle_id>.toml            (one AppIdentity per installed app)
  installed.txt                 (existing; auto-launch list)
  apps/<bundle>-<version>/      (existing; user-installed wasm)
  state/
    <bundle>.toml               (per-app state snapshot for restoration)
    <bundle>.last_crash         (last crash payload for the app to read)
  crashes/
    <unix_ts>-<bundle>-<iid>.toml
  logs/
    <bundle>-<iid>.log
```

---

End of architect design.

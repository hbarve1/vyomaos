# Kernel & Process Model — FINAL Synthesized Spec

> **Synthesis of:** Architect design (924 lines) + Critic review (508 lines)
> **Date:** 2026-05-29
> **Status:** Canonical. Implementation may begin against this spec.
> **Anchors:** `supervisor/src/main.rs`, `supervisor/src/manifest.rs`, `supervisor/src/lifecycle.rs`, `supervisor/src/app_threads.rs`
> **Constraint:** every new `.rs` file ≤ 500 lines (per repository CLAUDE.md).
> **Runtime:** Wasmtime 43.0.0 **embedded as a library** (no `wasmtime run` subprocesses).

---

## 0. Synthesis Rationale (read first)

The Architect's design provides the **right shape** — identity model, lifecycle state machine, crash taxonomy, resource quotas, staged boot, multi-instance routing, background tasks — but the Critic correctly identified that the **plumbing** would not scale past ~15 apps and contains at least three race conditions plus a fundamentally broken background-execution model.

This final spec keeps every Architect data structure that survived critique, and replaces every implementation detail the Critic attacked with the Critic's concrete fix. Specifically:

| Architect's design | Critic's attack | Final decision |
|---|---|---|
| `HashMap<BundleId, Arc<Mutex<AppState>>>` registry | Attack 1: global mutex hell | **Sharded `AppTable` with 16 `RwLock` shards + `DashMap` name index** |
| `wasmtime run` subprocess per app | Attack 4: cannot distinguish trap vs exit | **Embedded `wasmtime::Engine` + `Store` per instance, classified via `TrapCode`** |
| Unbounded `mpsc::channel()` for IPC inbox | Attack 1/5: OOM + race conditions | **`crossbeam::channel::bounded` with explicit overflow policy** |
| Stdin/stdout as the event loop | Attack 3: "background execution is a lie" | **WIT-typed `vyoma:lifecycle/callbacks` interface; stdin path retained as legacy fallback only** |
| `wasm-fork()` substitute via "spawn new instance" | Attack 2: WASM has no fork/COW | **`vyoma:state/snapshot` host call returning a `StateBlob`; new instance imports it on launch** |
| Fuel-based CPU budgeting | Attack 6: 20-40% overhead | **Wasmtime epoch interruption (~1-3% overhead) + `ResourceLimiter` for memory** |
| Implicit boot ordering | Attack 5: race A/B/C + ABBA deadlock | **`BootPhase` enum with explicit barriers + documented lock-order discipline** |
| `CrashCause` (7 variants) | Attack 4: misses 5 distinguishable failure modes | **`CrashKind` (12 variants) tied to `wasmtime::TrapCode` + `ExitStatus.signal()`** |

Where the two documents agreed (e.g. that `AppIdentity` is the right stable identity, that lifecycle needs more than `Running | Stopped`, that crash logs must be persisted, that on-disk layout is `/data/registry/`, `/data/crashes/`, `/data/state/`), the Architect's design is preserved verbatim.

> Synthesis note: when a conflict has no objective answer, this spec picks the **more conservative, fail-loud** option. Example: the Critic's `RestartDecision::PanicSupervisor` for `RuntimeFault` is kept over the Architect's silent retry, because a corrupted Wasmtime engine is a class of bug we must crash the supervisor on rather than mask.

---

## 1. Process Identity Model

### 1.1 `AppIdentity` (stable, install-time)

Source: Architect §1.1, unchanged.
File: `supervisor/src/identity.rs` (~180 LOC).

```rust
/// Stable identity of an installed app. Computed once at install/boot,
/// persisted in `/data/registry/<bundle_id>.toml`, and never mutated.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct AppIdentity {
    pub bundle_id:     BundleId,
    pub display_name:  String,
    pub version:       String,
    pub wasm_sha256:   WasmDigest,
    pub wasm_path:     PathBuf,
    pub manifest_path: PathBuf,
    pub capabilities:  Capabilities,
    pub origin:        InstallOrigin,
    pub installed_at:  u64,            // unix seconds
    pub signature:     Option<Ed25519Signature>,  // see §10 — code signing
    pub multi_instance: bool,
    pub max_instances: u16,
    pub focus_priority: i32,           // Attack 5 fix: deterministic focus
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct BundleId(String);

impl BundleId {
    /// Parse + validate. Accepts 2-5 dot-separated lowercase alphanumeric segments,
    /// e.g. "os.vyoma.notes", "com.example.myapp".
    pub fn parse(s: &str) -> Result<Self, IdentityError> { /* impl */ }
    pub fn as_str(&self) -> &str { &self.0 }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct WasmDigest([u8; 32]);

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Ed25519Signature([u8; 64]);

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum InstallOrigin {
    System,                                   // shipped in initramfs; immutable
    User { source_url: Option<String> },      // installed via package manager
    Developer,                                // dev-mode mount
}
```

### 1.2 `InstanceId` (transient, runtime)

Source: Architect §1.3 retained; type widened per Critic Attack 1 (`NonZeroU32` for cache friendliness + `Copy`).

```rust
/// Monotonically increasing, allocated by supervisor at spawn.
/// NonZeroU32 because `Option<InstanceId>` is the same size (4 bytes)
/// and `0` is reserved as a sentinel.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Ord, PartialOrd)]
pub struct InstanceId(pub std::num::NonZeroU32);
```

Same role as Architect's `InstanceId(u64)`: a launch event allocates one; it dies with the instance and is never reused.

> Synthesis note: width changed from `u64` → `NonZeroU32` (Critic Attack 1) because (a) it halves hashmap key size, (b) `Option<InstanceId>` packs to 4 bytes, (c) 4 billion launches before wraparound is enough — supervisor restarts the counter on reboot anyway.

### 1.3 Two registries

```rust
/// Installed = known to the system, may or may not be running.
/// Persisted at /data/registry/index.toml + per-app TOML files.
/// Single owner: the `InstallService` actor; readers obtain via Arc.
pub struct InstallRegistry {
    by_bundle: HashMap<BundleId, Arc<AppIdentity>>,
}

/// Live = currently executing in a Wasmtime store.
/// Sharded 16-way per Critic Attack 1.
pub struct AppTable {
    shards: [RwLock<HashMap<InstanceId, AppHandle>>; 16],
    name_to_id: dashmap::DashMap<BundleId, smallvec::SmallVec<[InstanceId; 2]>>,
    next_id: std::sync::atomic::AtomicU32,
}

impl AppTable {
    fn shard_of(&self, id: InstanceId) -> &RwLock<HashMap<InstanceId, AppHandle>> {
        let idx = (id.0.get() as usize) & 0xF;
        &self.shards[idx]
    }

    pub fn get(&self, id: InstanceId) -> Option<AppHandle> {
        self.shard_of(id).read().unwrap().get(&id).cloned()
    }

    pub fn lookup_bundle(&self, b: &BundleId) -> SmallVec<[InstanceId; 2]> {
        self.name_to_id.get(b).map(|r| r.clone()).unwrap_or_default()
    }

    pub fn insert(&self, h: AppHandle) {
        let id = h.instance_id;
        self.name_to_id.entry(h.identity.bundle_id.clone())
            .or_default().push(id);
        self.shard_of(id).write().unwrap().insert(id, h);
    }

    pub fn remove(&self, id: InstanceId) -> Option<AppHandle> {
        let h = self.shard_of(id).write().unwrap().remove(&id)?;
        if let Some(mut v) = self.name_to_id.get_mut(&h.identity.bundle_id) {
            v.retain(|x| *x != id);
        }
        Some(h)
    }
}
```

### 1.4 `AppHandle` (cheap, cloneable)

Source: Critic Attack 1 fix, expanded with Architect's field set.

```rust
/// What lookups return. Cloneable (Arc-based); does NOT lock anything.
#[derive(Clone)]
pub struct AppHandle {
    pub instance_id: InstanceId,
    pub identity:    Arc<AppIdentity>,
    pub control:     crossbeam::channel::Sender<LifecycleCmd>,  // cap 64
    pub inbox:       crossbeam::channel::Sender<IpcEnvelope>,   // cap 256
    pub render:      Arc<RenderSlot>,                            // owned by compositor
    pub state:       Arc<RwLock<AppState>>,                      // hot/cold split inside
    pub store_ref:   Arc<Mutex<WasmStore>>,                      // single-threaded re-entry
}
```

### 1.5 `WasmStore` wrapper

```rust
/// Wraps the per-instance wasmtime::Store and bindings.
/// Held under Mutex; only callback dispatcher acquires it.
pub struct WasmStore {
    pub store:     wasmtime::Store<HostCtx>,
    pub instance:  wasmtime::Instance,
    pub bindings:  vyoma_lifecycle::Callbacks,   // wit-bindgen-generated
    pub limiter:   AppLimiter,                    // see §6
}
```

---

## 2. Process Lifecycle State Machine

File: `supervisor/src/lifecycle/state.rs` (~250 LOC).

Source: Architect §2 retained. Two changes from critique:
- Added `Unresponsive` state (Critic Attack 1: detected by IPC `try_send` timeout escalation).
- `Crashed` carries `CrashKind` (Critic Attack 4) not `CrashReportId` alone.

```rust
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AppStatus {
    NotRunning,

    Launching   { since: Instant, store_id: StoreId },
    Running     { since: Instant, store_id: StoreId },
    Background  { since: Instant, store_id: StoreId },
    Suspended   { since: Instant, store_id: StoreId, reason: SuspendReason },

    /// IPC write timeout fired; supervisor escalating but not yet terminating.
    /// New per Critic Attack 1.
    Unresponsive { since: Instant, store_id: StoreId, miss_count: u8 },

    Terminating { since: Instant, store_id: StoreId },

    Terminated  { exit_code: i32, at: Instant },
    Crashed     { kind: CrashKind, at: Instant, report_id: CrashReportId },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SuspendReason {
    UserMinimized,
    SystemMemoryPressure,
    AppRequested,
    QuotaExceeded,
    EnergyManagement,   // future App-Nap analogue
}
```

### 2.1 Transitions (final)

| From          | To            | Trigger                                                | Side effects |
|---------------|---------------|--------------------------------------------------------|---------------|
| `NotRunning`  | `Launching`   | `launch(bundle_id)` IPC or auto-launch list            | allocate `InstanceId`, instantiate Wasmtime, create surface placeholder, register epoch deadline |
| `Launching`   | `Running`     | first `VYOMA_DRAW:flush` OR 500 ms grace               | mark surface visible, push to z-order, emit `lifecycle.running` |
| `Launching`   | `Crashed`     | child exits before flush, or 5 s timeout               | write crash report, surface destroyed, restart policy evaluated |
| `Running`     | `Background`  | another app gains focus                                | drop input subscriptions, IPC still flows, surface kept |
| `Background`  | `Running`     | window clicked / `@supervisor: focus <bundle>`         | restore input, raise z-order |
| `Running`/`Bg`| `Suspended`   | minimize, memory pressure, quota exceed                | call `on-suspend`, persist `StateBlob`, **drop store + free linear memory** |
| `Suspended`   | `Running`     | dock click, programmatic wake, IPC inbox overflow      | re-instantiate, call `on-resume(state)`, raise window |
| `Running`     | `Unresponsive`| 3 consecutive IPC `try_send` timeouts (50 ms each)     | mark; watchdog escalation begins |
| `Unresponsive`| `Running`     | next callback returns in time                          | clear `miss_count` |
| `Unresponsive`| `Terminating` | 5 s without recovery                                   | SIGTERM equivalent — abort epoch + drop store |
| any-live      | `Terminating` | `kill <bundle>`, app exit request, shutdown            | 2 s grace for `on-terminate` callback; force-drop store after |
| `Terminating` | `Terminated`  | clean exit reported via `on-terminate` OK              | release surface, IPC, files, peripheral leases |
| `Terminating` | `Crashed`     | timed out OR trap during termination                   | crash report written, restart policy evaluated |
| `Crashed`     | `Launching`   | restart policy permits + backoff elapsed               | new `InstanceId`, surface re-created |
| `Crashed`     | `NotRunning`  | policy = `Never` OR `max_restarts` exhausted           | crash log retained, no auto-restart |
| `Terminated`  | `NotRunning`  | 30 s state-restoration grace elapses                   | flush state snapshot to `/data/state/<bundle>.bin` |

### 2.2 Resource transitions

As Architect §2.2, with two amendments per Critic:

| Resource              | `Suspended` change |
|-----------------------|---------------------|
| WASM linear memory    | **freed via `Store` drop** (was: "resident"). Re-instantiated on `Resume`. |
| Wasmtime store        | dropped (was: kept) — Critic Attack 3 requires this to make Suspend honest |

All other rows unchanged from Architect §2.2.

---

## 3. Multi-Instance Model

File: `supervisor/src/instance.rs` (~220 LOC).
Source: Architect §3, retained. Two clarifications from critique:

- The "singleton" rule (`multi_instance = false`) holds across the **bundle id**, not the display name — a renamed window does not become a separate instance.
- IPC routing now uses `InstanceId` natively (not a `String`).

### 3.1 Manifest field (unchanged)

```toml
[app]
bundle_id      = "os.vyoma.notes"
multi_instance = true
max_instances  = 16
focus_priority = 100   # higher = wins focus race on boot (Attack 5 fix)
```

### 3.2 IPC addressing (final grammar)

```
@<bundle_id>: msg                    # broadcast to all live instances
@<bundle_id>#<instance_id>: msg      # unicast to one instance
@@focused: msg                       # unicast to focused instance
@supervisor: msg                     # supervisor RPC
```

Router (`supervisor/src/ipc_router.rs`, ~240 LOC) parses these, resolves via `AppTable::lookup_bundle`, calls `inbox.try_send` (bounded). Overflow policy is per-message-class:

| Class     | Cap  | Overflow policy             |
|-----------|------|-----------------------------|
| `Input`   | 256  | drop-oldest, log at WARN    |
| `Ipc`     | 256  | NACK to sender, log at INFO |
| `Render`  | 4096 | coalesce dirty regions      |
| `Control` | 64   | block max 50 ms, then fail-with-error |

---

## 4. Background Execution Model

File: `supervisor/src/lifecycle/background.rs` (~250 LOC) + WIT package at `wit/vyoma-lifecycle.wit`.

Source: **Critic Attack 3 wins entirely.** Architect's stdin-line scheme is downgraded to a legacy fallback for printf-only demo apps.

### 4.1 WIT-typed callback interface (authoritative)

`wit/vyoma-lifecycle.wit`:

```wit
package vyoma:lifecycle;

interface callbacks {
    use vyoma:time/types.{instant};
    use vyoma:net/types.{net-status};

    /// Fired by host when a timer registered via vyoma:time/timer.set elapses.
    on-timer: func(token: u64);

    /// Fired when a backgrounded network request completes.
    on-net-complete: func(req-id: u32, status: net-status, bytes: list<u8>);

    /// App is about to be suspended (memory freed).
    /// MUST return ≤100 ms or supervisor force-suspends with no snapshot.
    on-suspend: func() -> result<state-blob, suspend-error>;

    /// App is being resumed after suspend. State may be empty if cold-resumed.
    on-resume: func(state: option<state-blob>);

    /// App is being terminated; last chance to flush.
    on-terminate: func();

    /// App was restarted after crash; prior crash report id supplied.
    on-restart: func(prior-instance: u64, reason: crash-summary);

    /// Foreground/background transition.
    on-focus-changed: func(focused: bool);
}

type state-blob   = list<u8>;
type suspend-error = string;
type crash-summary = record { kind: string, uptime-secs: u64 };

world vyoma-app {
    export callbacks;
    import vyoma:time/timer;
    import vyoma:net/async;
    import vyoma:state/snapshot;
}
```

### 4.2 Supervisor dispatcher (final)

```rust
/// One dispatcher per instance. Owns the lock on the per-instance Store.
pub struct CallbackDispatcher {
    store_ref: Arc<Mutex<WasmStore>>,
    rx:        crossbeam::channel::Receiver<HostCallback>,
    epoch:     Arc<EpochController>,
    app_id:    InstanceId,
}

pub enum HostCallback {
    Timer    { token: u64 },
    NetComplete { req_id: u32, status: NetStatus, bytes: bytes::Bytes },
    Suspend  { reply: oneshot::Sender<Result<StateBlob, SuspendError>> },
    Resume   { state: Option<StateBlob> },
    Terminate,
    Restart  { prior: InstanceId, reason: CrashSummary },
    FocusChanged(bool),
    Ipc      { from: BundleId, payload: bytes::Bytes },  // typed IPC envelope
}

impl CallbackDispatcher {
    pub fn run(self) {
        while let Ok(cb) = self.rx.recv() {
            let mut ws = self.store_ref.lock().unwrap();
            // Per-callback epoch budget (10 ms default; on-suspend forced 100 ms).
            ws.store.set_epoch_deadline(self.epoch.ticks_for(&cb));
            let r = match cb {
                HostCallback::Timer { token } =>
                    ws.bindings.callbacks().call_on_timer(&mut ws.store, token),
                HostCallback::NetComplete { req_id, status, bytes } =>
                    ws.bindings.callbacks().call_on_net_complete(&mut ws.store, req_id, status, &bytes),
                HostCallback::Suspend { reply } => {
                    let r = ws.bindings.callbacks().call_on_suspend(&mut ws.store);
                    let _ = reply.send(r.map_err(|e| SuspendError(e.to_string())).and_then(|x| x));
                    continue;
                }
                HostCallback::Resume { state } =>
                    ws.bindings.callbacks().call_on_resume(&mut ws.store, state.as_deref()),
                HostCallback::Terminate =>
                    ws.bindings.callbacks().call_on_terminate(&mut ws.store),
                HostCallback::Restart { prior, reason } =>
                    ws.bindings.callbacks().call_on_restart(&mut ws.store, prior.0.get() as u64, &reason),
                HostCallback::FocusChanged(f) =>
                    ws.bindings.callbacks().call_on_focus_changed(&mut ws.store, f),
                HostCallback::Ipc { from, payload } =>
                    ws.bindings.callbacks().call_on_ipc(&mut ws.store, from.as_str(), &payload),
            };
            if let Err(trap) = r {
                drop(ws);   // release before crash-handling acquires AppTable
                crate::lifecycle::handle_callback_trap(self.app_id, trap);
                return;
            }
        }
    }
}
```

### 4.3 `BackgroundTask` registration

The Architect's `BackgroundTask` struct remains, but `wasm_export: String` becomes `kind: BackgroundKind` because dispatch is now via the typed `on-timer` / `on-net-complete` callbacks, not arbitrary exports.

```rust
#[derive(Clone, Debug)]
pub struct BackgroundTask {
    pub id:        BgTaskId,
    pub instance:  InstanceId,
    pub bundle:    BundleId,
    pub kind:      BackgroundKind,
    pub deadline_ms: u64,
    pub period_ms: Option<u64>,
    pub payload:   bytes::Bytes,                  // ≤ 4 KiB
    pub epoch_budget_us: u64,                     // CPU per fire (Attack 6)
}

#[derive(Clone, Debug)]
pub enum BackgroundKind {
    OneShot,
    Periodic,
    Service,
    NetworkWake { port: u16 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct BgTaskId(pub u64);
```

### 4.4 Background quotas (unchanged from Architect §4.4)

```rust
pub struct BackgroundQuota {
    pub max_concurrent_tasks: u8,           // default 8
    pub max_total_budget_ms_per_min: u32,   // default 2000 (3.3% CPU)
    pub min_period_ms: u32,                 // default 5_000
    pub allowed_kinds: u8,                  // bitset of BackgroundKind variants
}
```

---

## 5. Crash & Restart Policy

File: `supervisor/src/lifecycle/crash_report.rs` (~280 LOC).

Source: **Critic Attack 4 wins.** Architect's 7-variant `CrashCause` is replaced with the 12-variant `CrashKind` enum that ties directly to `wasmtime::TrapCode` and `ExitStatus.signal()`.

### 5.1 `CrashKind` (final)

```rust
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum CrashKind {
    /// `proc_exit(0)` — not actually a crash, included for uniformity.
    Clean,

    /// `proc_exit(N)` where N != 0 and N is not in `clean_exit_codes`.
    AppExit(i32),

    /// Wasmtime trap: out-of-bounds, stack overflow, unreachable, etc.
    Trap {
        code:       SerializableTrapCode,
        backtrace:  String,
        instr_addr: u64,
    },

    /// `wasmtime::Memory::grow` returned false (ResourceLimiter denied).
    MemLimit { requested_pages: u32, max_pages: u32 },

    /// Epoch deadline expired and policy decided `Kill`.
    CpuLimit { fuel_consumed: u64 },

    /// Watchdog: no `VYOMA_DRAW:flush` or IPC for `watchdog_secs`.
    WatchdogKill { silent_for_ms: u64 },

    /// Host kernel killed us (OOM, SIGSEGV via shared lib).
    HostOOM,

    /// `wasmtime::Engine` itself returned an error of last resort.
    RuntimeFault(String),

    /// stdin/stdout pipe broken; legacy-mode apps only.
    ProtocolFault(String),

    /// `Terminating` grace expired; we force-killed via store drop.
    KillTimeout,

    /// External SIGKILL/SIGTERM (multi-tenant host, parent supervisor restart).
    HostSignal { signo: i32 },

    /// `on-suspend` / `on-resume` / `on-terminate` callback trapped.
    CallbackTrap { phase: CallbackPhase, message: String },
}

#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub enum CallbackPhase { Suspend, Resume, Terminate, Timer, NetComplete, Ipc, Restart, FocusChanged }

/// Mirrors `wasmtime::TrapCode` as a `serde`-friendly enum.
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub enum SerializableTrapCode {
    StackOverflow, MemoryOutOfBounds, HeapMisaligned, TableOutOfBounds,
    IndirectCallToNull, BadSignature, IntegerOverflow, IntegerDivisionByZero,
    BadConversionToInteger, UnreachableCodeReached, Interrupt, OutOfFuel,
    Other(String),
}

impl From<wasmtime::TrapCode> for SerializableTrapCode {
    fn from(c: wasmtime::TrapCode) -> Self {
        use wasmtime::TrapCode::*;
        match c {
            StackOverflow            => Self::StackOverflow,
            MemoryOutOfBounds        => Self::MemoryOutOfBounds,
            HeapMisaligned           => Self::HeapMisaligned,
            TableOutOfBounds         => Self::TableOutOfBounds,
            IndirectCallToNull       => Self::IndirectCallToNull,
            BadSignature             => Self::BadSignature,
            IntegerOverflow          => Self::IntegerOverflow,
            IntegerDivisionByZero    => Self::IntegerDivisionByZero,
            BadConversionToInteger   => Self::BadConversionToInteger,
            UnreachableCodeReached   => Self::UnreachableCodeReached,
            Interrupt                => Self::Interrupt,
            OutOfFuel                => Self::OutOfFuel,
            other                    => Self::Other(format!("{other:?}")),
        }
    }
}
```

### 5.2 Classification function

```rust
pub fn classify_exit(
    exit: Option<i32>,                      // None = no clean exit, see trap
    trap: Option<wasmtime::Error>,          // Some = host-side error from Func::call
    clean_codes: &[i32],
) -> CrashKind {
    if let Some(err) = trap {
        if let Some(t) = err.downcast_ref::<wasmtime::Trap>() {
            return CrashKind::Trap {
                code:      (*t).into(),
                backtrace: format!("{err:?}"),
                instr_addr: err.downcast_ref::<wasmtime::WasmBacktrace>()
                    .and_then(|b| b.frames().first().map(|f| f.module_offset() as u64))
                    .unwrap_or(0),
            };
        }
        return CrashKind::RuntimeFault(format!("{err}"));
    }
    match exit {
        Some(0)                                       => CrashKind::Clean,
        Some(n) if clean_codes.contains(&n)           => CrashKind::Clean,
        Some(n)                                       => CrashKind::AppExit(n),
        None                                          => CrashKind::Clean, // host reaped before exit
    }
}
```

### 5.3 Crash log format (Architect §5.2, fields updated)

`/data/crashes/<unix_ts>-<bundle>-<instance>.toml`:

```toml
schema_version = 1
report_id      = "crash-20260529T184412Z-os.vyoma.notes-42"
bundle_id      = "os.vyoma.notes"
instance_id    = 42
wasm_sha256    = "9f3c…"
version        = "1.4.0"
crashed_at     = 1748541852
uptime_secs    = 312
restart_count  = 0

[cause]
kind         = "trap"
trap_code    = "memory_out_of_bounds"
instr_addr   = 0x1a3c
backtrace    = """
notes::editor::insert_char (notes.wasm+0x1a3c)
notes::main::loop          (notes.wasm+0x0921)
_start                     (notes.wasm+0x0042)
"""

[runtime]
linear_mem_pages = 7
epoch_ticks_used = 4823914
open_files       = 2
ipc_inbox_len    = 0
bg_tasks_active  = 1

[last_logs]
lines = [
  "[2026-05-29T18:44:12.012Z] [notes] saving draft",
  "[2026-05-29T18:44:12.087Z] [notes] PANIC: out of bounds at index 4096",
]

[recovery]
action      = "restart"
backoff_ms  = 2000
attempt     = 1
```

### 5.4 Restart decision (Critic's matrix wins)

```rust
pub enum Decision {
    DoNotRestart,
    RestartAfter(Duration),
    Throttle { epoch_budget_multiplier: f32 },
    Quarantine,                  // log + show user, do not restart
    PanicSupervisor,             // for RuntimeFault only — wasmtime itself broken
}

pub fn restart_policy(
    kind:    &CrashKind,
    policy:  &RestartPolicy,
    restart_count: u32,
    backoff: &mut ExponentialBackoff,
) -> Decision {
    match kind {
        CrashKind::Clean                       => Decision::DoNotRestart,
        CrashKind::AppExit(0)                  => Decision::DoNotRestart,
        CrashKind::AppExit(_)
        | CrashKind::Trap { .. }
        | CrashKind::CallbackTrap { .. }       => {
            if restart_count >= policy.max_restarts {
                Decision::Quarantine
            } else {
                Decision::RestartAfter(backoff.next())
            }
        }
        CrashKind::WatchdogKill { .. }
        | CrashKind::ProtocolFault(_)
        | CrashKind::KillTimeout               => Decision::RestartAfter(backoff.next()),
        CrashKind::CpuLimit { .. }             => Decision::Throttle { epoch_budget_multiplier: 0.5 },
        CrashKind::MemLimit { .. }             => Decision::Quarantine,
        CrashKind::RuntimeFault(_)             => Decision::PanicSupervisor,
        CrashKind::HostOOM
        | CrashKind::HostSignal { .. }         => Decision::DoNotRestart,
    }
}
```

### 5.5 Manifest fields (unchanged from Architect §5.3)

```toml
[lifecycle]
restart = "on-crash"             # never | always | on-crash | exponential-backoff
max_restarts = 5
backoff_initial_ms = 1000
backoff_max_ms = 60000
state_restoration = true
clean_exit_codes = [0, 75]
```

### 5.6 State restoration (synthesis)

Two mechanisms coexist:

1. **WIT path (preferred):** `on-suspend` returns `StateBlob`; supervisor persists; relaunch calls `on-resume(state)`. Sized cap: 1 MiB per blob (configurable in manifest `[state]`).
2. **Legacy `VYOMA_SYSTEM:restore:` line (Architect §5.4):** retained only for stdio-only apps that have not adopted WIT. Supervisor still emits `VYOMA_SYSTEM:restore:bundle=… instance=… prior_instance=… reason=…` on stdin; legacy apps read it.

> Synthesis note: the WIT path is now authoritative because it gives the supervisor a known-bounded payload and a typed return, whereas the stdin path is best-effort. New apps MUST adopt WIT; legacy apps remain supported for one major version.

---

## 6. Resource Limits Per App

File: `supervisor/src/lifecycle/quota.rs` (~240 LOC).

Source: **Critic Attack 6 wins on CPU.** Architect's struct preserved; CPU mechanism switched fuel → epoch.

### 6.1 `ResourceQuota` (final)

```rust
#[derive(Clone, Debug)]
pub struct ResourceQuota {
    /// Hard ceiling on `memory.grow`. 1 page = 64 KiB. Default 256 pages.
    pub wasm_memory_pages: u32,

    /// CPU budget in milliseconds per real-world second.
    /// Implemented as epoch deadline scheduling, NOT fuel.
    pub cpu_time_ms_per_sec: u32,

    pub open_files:          u16,
    pub network_connections: u16,
    pub io_bytes_per_sec:    u32,
    pub background:          BackgroundQuota,
    pub max_surface_pixels:  u32,
    pub max_state_blob_bytes: u32,    // for on-suspend return; default 1 MiB
    pub max_tables_elements: u32,     // wasmtime table grow ceiling
}

impl Default for ResourceQuota {
    fn default() -> Self {
        Self {
            wasm_memory_pages:    256,            // 16 MiB
            cpu_time_ms_per_sec:  100,            // single core
            open_files:           32,
            network_connections:  8,
            io_bytes_per_sec:     4 * 1024 * 1024,
            background:           BackgroundQuota::default(),
            max_surface_pixels:   4 * 1024 * 1024,   // 2048×2048
            max_state_blob_bytes: 1024 * 1024,
            max_tables_elements:  4096,
        }
    }
}
```

### 6.2 Enforcement (final mechanism table)

| Quota                 | Mechanism                                                                          |
|-----------------------|------------------------------------------------------------------------------------|
| `wasm_memory_pages`   | `wasmtime::ResourceLimiter::memory_growing` returns `false` (zero overhead)        |
| `cpu_time_ms_per_sec` | **Wasmtime epoch interruption** (~1-3% overhead), 100 Hz global epoch tick, per-store deadline |
| `open_files`          | Host-function wrapper on `wasi:filesystem/types.open-at`                            |
| `network_connections` | Host-function wrapper on `wasi:sockets/tcp.start-connect` and udp counterpart      |
| `io_bytes_per_sec`    | Token bucket per app, refilled at `io_bytes_per_sec / 100` every 10 ms             |
| `max_surface_pixels`  | Validated at `VYOMA_DRAW:resize` and compositor surface allocation                  |
| `max_state_blob_bytes`| Enforced when `on-suspend` returns; truncated + warning if exceeded                |
| `max_tables_elements` | `wasmtime::ResourceLimiter::table_growing`                                          |
| Background quotas     | §4.4                                                                                |

### 6.3 `AppLimiter` (Critic Attack 6 fix)

```rust
pub struct AppLimiter {
    pub max_memory_bytes:   usize,
    pub max_table_elements: usize,
    pub current_memory:     std::sync::atomic::AtomicUsize,
}

impl wasmtime::ResourceLimiter for AppLimiter {
    fn memory_growing(&mut self, _c: usize, desired: usize, _max: Option<usize>) -> wasmtime::Result<bool> {
        if desired > self.max_memory_bytes { return Ok(false); }
        self.current_memory.store(desired, std::sync::atomic::Ordering::Relaxed);
        Ok(true)
    }
    fn table_growing(&mut self, _c: usize, desired: usize, _m: Option<usize>) -> wasmtime::Result<bool> {
        Ok(desired <= self.max_table_elements)
    }
}
```

### 6.4 `EpochController`

```rust
pub struct EpochController {
    pub engine:        wasmtime::Engine,
    pub period:        Duration,                     // 10 ms = 100 Hz
    pub deadline_ms:   std::sync::atomic::AtomicU64, // monotonic
}

impl EpochController {
    pub fn start_tick(self: Arc<Self>) {
        std::thread::Builder::new()
            .name("vyoma-epoch".into())
            .spawn(move || loop {
                std::thread::sleep(self.period);
                self.engine.increment_epoch();
                self.deadline_ms.fetch_add(self.period.as_millis() as u64,
                                           std::sync::atomic::Ordering::Relaxed);
            })
            .expect("spawn epoch thread");
    }

    /// Per-callback budget in epoch ticks.
    pub fn ticks_for(&self, cb: &HostCallback) -> u64 {
        match cb {
            HostCallback::Suspend { .. }    => 10,   // 100 ms hard cap
            HostCallback::Resume { .. }     => 10,
            HostCallback::Terminate         => 20,
            HostCallback::Timer { .. }      => 1,    // 10 ms
            HostCallback::NetComplete { .. } => 2,
            HostCallback::Ipc { .. }        => 1,
            HostCallback::Restart { .. }    => 5,
            HostCallback::FocusChanged(_)   => 1,
        }
    }
}
```

### 6.5 Per-app `epoch_deadline_callback`

```rust
store.epoch_deadline_callback(move |mut store| {
    let app = store.data().instance_id;
    match scheduler::on_deadline(app, store.fuel_consumed().unwrap_or(0)) {
        SchedDecision::ContinueWith(extra) => Ok(wasmtime::UpdateDeadline::Continue(extra)),
        SchedDecision::Yield               => Ok(wasmtime::UpdateDeadline::Yield(1)),
        SchedDecision::Kill                => Err(anyhow::anyhow!("cpu quota exhausted")),
    }
});
```

### 6.6 Exceed behaviour (Architect §6.3 retained)

Unchanged.

---

## 7. Supervisor Internal Process Table

File: `supervisor/src/process_table.rs` (~400 LOC).

Source: Architect §7 + Critic Attack 1 (hot/cold split to reduce lock contention).

### 7.1 `AppState` (hot/cold split)

```rust
pub struct AppState {
    /// Hot fields — touched on every frame, compositor, input event.
    pub hot:  parking_lot::Mutex<HotState>,
    /// Cold fields — touched on lifecycle events, logs, crashes.
    pub cold: parking_lot::Mutex<ColdState>,
}

pub struct HotState {
    pub status:        AppStatus,
    pub focus_token:   Option<FocusToken>,
    pub has_display:   bool,
    pub surface:       Option<Arc<arc_swap::ArcSwap<Surface>>>,   // atomic swap, no lock
    pub win_region:    Option<WindowRegion>,
    pub win_z:         u32,
    pub min_size:      (u32, u32),
    pub minimized:     bool,
    pub pre_minimize_region: Option<WindowRegion>,
    pub pending_anim:  Option<Animation>,
    pub dirty:         bool,
    pub flush_stats:   FlushStats,
    pub inbox_pending: usize,
}

pub struct ColdState {
    pub instance_id:        InstanceId,
    pub identity:           Arc<AppIdentity>,
    pub display_name_runtime: String,
    pub status_history:     RingBuffer<(AppStatus, Instant), 16>,
    pub spawn_time:         Instant,
    pub last_state_change:  Instant,
    pub restart_policy:     RestartPolicy,
    pub restart_count:      u32,
    pub crash_history:      RingBuffer<CrashReportId, 8>,

    pub child_pid:          Option<u32>,
    pub child_stdin:        Option<std::process::ChildStdin>,    // legacy path
    pub child_stdout:       Option<std::process::ChildStdout>,   // legacy path
    pub child_stderr:       Option<std::process::ChildStderr>,   // legacy path
    pub wasmtime_store_id:  StoreId,
    pub thread_join:        Option<std::thread::JoinHandle<i32>>,

    pub last_sender:        Option<BundleId>,
    pub ipc_log_level:      LogLevel,

    pub has_mouse:          bool,
    pub has_keyboard:       bool,

    pub capabilities:       Capabilities,
    pub peripheral_leases:  Vec<PeripheralLease>,
    pub file_handles:       HashMap<u32, FileHandleInfo>,
    pub network_conns:      HashMap<ConnId, NetConnInfo>,

    pub watchdog_secs:      u32,
    pub last_output:        Instant,
    pub watchdog_backoff:   u64,
    pub unresponsive_misses: u8,

    pub quota:              ResourceQuota,
    pub cpu_ticks:          u64,
    pub cpu_budget_us:      i64,
    pub mem_pages:          u32,
    pub io_tokens:          u32,
    pub bg_tasks:           HashMap<BgTaskId, BackgroundTask>,
    pub last_cpu_reset:     Instant,

    pub log_buf:            VecDeque<LogLine>,
    pub log_subscribers:    Vec<crossbeam::channel::Sender<LogLine>>,
    pub log_file:           Option<std::fs::File>,

    pub state_snapshot:     Option<StateBlob>,
    pub launch_args:        LaunchArgs,
}

pub struct FlushStats {
    pub flushes:        u64,
    pub last_flush:     Instant,
    pub avg_interval_ms: u32,
}

pub struct FocusToken;
pub struct PeripheralLease { pub kind: PeripheralKind, pub id: u8, pub acquired_at: Instant }
pub struct FileHandleInfo  { pub path: PathBuf, pub mode: u32, pub opened_at: Instant }
pub struct NetConnInfo     { pub peer: std::net::SocketAddr, pub kind: ConnKind, pub opened_at: Instant }
pub struct LogLine         { pub at: Instant, pub level: LogLevel, pub text: String }
pub struct StateBlob       { pub bytes: bytes::Bytes }      // ≤ max_state_blob_bytes
pub struct LaunchArgs      { pub argv: Vec<String>, pub env: BTreeMap<String, String> }

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct StoreId(pub u32);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct CrashReportId(pub u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ConnId(pub u32);

#[derive(Clone, Copy, Debug)]
pub enum LogLevel { Trace, Debug, Info, Warn, Error }

#[derive(Clone, Copy, Debug)]
pub enum PeripheralKind { Gpio, I2c, Spi, Uart, Adc }

#[derive(Clone, Copy, Debug)]
pub enum ConnKind { Tcp, Udp }

pub struct Animation { pub kind: AnimKind, pub started: Instant, pub duration: Duration }
pub enum AnimKind { Open, Close, Minimize, Unminimize, Reflow }

pub struct RingBuffer<T, const N: usize> { buf: [Option<T>; N], head: usize }
```

> Synthesis note: hot/cold split (Critic Attack 1 mitigation) means the compositor only contends with input router; lifecycle events touch only `cold`. Combined with `arc_swap::ArcSwap<Surface>`, the framebuffer flush path is **lock-free for surface reads**.

---

## 8. Boot Sequence

File: `supervisor/src/boot.rs` (~320 LOC).

Source: Architect §8 + Critic Attack 5 (`BootPhase` enum + lock-order discipline).

### 8.1 `BootPhase` enum (Critic)

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum BootPhase {
    Cold,
    Mount,          // S1: /proc /sys /dev /data
    Profile,        // S2: load profile TOML
    Registry,       // S3: scan /data/registry + /etc/vyoma/apps
    Display,        // S4 (early): /dev/fb0 open, compositor init
    IpcRouter,      // S5: bounded inbox plumbing online
    Subsystems,     // S6: input router, watchdog, mgmt server
    SystemApps,     // S7: tier=system, parallel, all required
    LoginApps,      // S8: tier=login, parallel, soft-required
    AutoApps,       // S9: tier=auto from /data/installed.txt, rate-limited
    Ready,          // S10: emit "lifecycle.boot_done", on-demand apps allowed
}

pub struct BootBarrier {
    phase:  std::sync::atomic::AtomicU8,
    cv:     parking_lot::Condvar,
    mu:     parking_lot::Mutex<()>,
}

impl BootBarrier {
    pub fn current(&self) -> BootPhase {
        unsafe { std::mem::transmute(self.phase.load(std::sync::atomic::Ordering::Acquire)) }
    }
    pub fn advance(&self, to: BootPhase) {
        self.phase.store(to as u8, std::sync::atomic::Ordering::Release);
        self.cv.notify_all();
    }
    pub fn wait_until(&self, target: BootPhase) {
        let mut g = self.mu.lock();
        while self.current() < target { self.cv.wait(&mut g); }
    }
}
```

### 8.2 Lock-order discipline (Critic Attack 5)

Documented at the top of `supervisor/src/lib.rs`:

```rust
//! GLOBAL LOCK ORDER — acquire in numerical order; never reverse:
//!   1. BootBarrier
//!   2. InstallRegistry
//!   3. AppTable shard locks
//!   4. AppState.cold
//!   5. AppState.hot
//!   6. IPC inbox try_send (bounded, never blocks)
//!   7. CompositorState
//!   8. Surface (ArcSwap; lock-free reads, swap-only writes)
//!
//! Violations are debug_asserted via the `LockTracker` thread-local in dev builds.
```

### 8.3 Stage order (Architect §8.1 reordered to match `BootPhase`)

| Phase       | Parallel? | Killable apps? | Notes |
|-------------|-----------|----------------|-------|
| Cold        | —         | —              | bootloader handoff |
| Mount       | seq       | —              | /proc, /sys, /dev, 9P /data |
| Profile     | seq       | —              | `PLATFORM` → `/etc/vyoma/profiles/<name>.toml` |
| Registry    | seq       | —              | scan + parse identities |
| Display     | seq       | —              | `/dev/fb0`, splash, compositor instantiated (no apps yet) |
| IpcRouter   | seq       | —              | bounded channels created |
| Subsystems  | parallel  | —              | input router, watchdog, mgmt server, epoch ticker |
| SystemApps  | parallel  | no             | windowserver, dock, menubar, statusbar, crash-reporter — all REQUIRED |
| LoginApps   | parallel  | by user        | finder, notification-center, spotlight |
| AutoApps    | parallel  | yes            | user-selected, rate-limited to 4 concurrent spawns |
| Ready       | —         | —              | emit `lifecycle.boot_done`; on-demand allowed |

### 8.4 Manifest tier field (unchanged from Architect §8.2)

```toml
[lifecycle]
tier = "system"          # system | login | auto | on_demand
```

### 8.5 boot.toml (final shape)

```toml
[boot]
profile = "desktop-full"

[[apps]]
manifest = "/etc/vyoma/apps/windowserver/vyoma.toml"
tier     = "system"
focus_priority = -1000   # never wins focus

[[apps]]
manifest = "/etc/vyoma/apps/dock/vyoma.toml"
tier     = "system"

[[apps]]
manifest = "/etc/vyoma/apps/menubar/vyoma.toml"
tier     = "system"

[[apps]]
manifest = "/etc/vyoma/apps/crash-reporter/vyoma.toml"
tier     = "system"

[[apps]]
manifest = "/etc/vyoma/apps/finder/vyoma.toml"
tier     = "login"

[[apps]]
manifest = "/etc/vyoma/apps/notification-center/vyoma.toml"
tier     = "login"
```

`/data/installed.txt` is read at `AutoApps`. Focus on boot is granted to the highest `focus_priority` in `LoginApps`; ties broken by manifest order.

---

## 9. Subsystem Actors (from Critic Attack 1)

The supervisor decomposes into **five actors**, each owning its state, communicating via bounded channels. This is the architectural fix that scales to 50 apps.

| Actor              | Owns                                       | Inbound channel              | File |
|--------------------|--------------------------------------------|------------------------------|------|
| `LifecycleActor`   | spawn, suspend, resume, kill, restart      | `Sender<LifecycleCmd>` cap 256 | `lifecycle/actor.rs` |
| `IpcRouter`        | `name_to_id`, message routing              | `Sender<IpcEnvelope>` cap 1024 | `ipc_router.rs` |
| `Compositor`       | regions, surfaces, Z-order                 | `Sender<RenderCmd>` cap 4096 | `display/compositor.rs` |
| `InputDispatcher`  | focus, kb/mouse routing                    | `Sender<InputEvent>` cap 256 | `input/dispatcher.rs` |
| `WatchdogActor`    | per-app `last_output`, escalation          | timer ticks                  | `lifecycle/watchdog.rs` |

```rust
#[derive(Debug)]
pub enum LifecycleCmd {
    Spawn   { bundle: BundleId, args: LaunchArgs, reply: oneshot::Sender<Result<InstanceId, LaunchError>> },
    Suspend { id: InstanceId, reason: SuspendReason },
    Resume  { id: InstanceId },
    Kill    { id: InstanceId, sig: KillSignal },
    Restart { id: InstanceId },
    Crashed { id: InstanceId, kind: CrashKind },
    Tick,    // 1 Hz; advances backoffs, expires Terminated grace
}

#[derive(Debug, Clone)]
pub struct IpcEnvelope {
    pub from:    Option<InstanceId>,     // None = supervisor
    pub target:  IpcTarget,
    pub payload: bytes::Bytes,
    pub class:   IpcClass,
}

#[derive(Debug, Clone)]
pub enum IpcTarget {
    Bundle(BundleId),                                // broadcast
    Instance(InstanceId),                            // unicast
    Focused,                                         // @@focused
    Supervisor,                                      // @supervisor
}

#[derive(Debug, Clone, Copy)]
pub enum IpcClass { Input, Ipc, Render, Control }

#[derive(Debug, Clone)]
pub enum RenderCmd {
    AllocSurface { id: InstanceId, w: u32, h: u32, reply: oneshot::Sender<Arc<ArcSwap<Surface>>> },
    SetRegion    { id: InstanceId, region: WindowRegion },
    Flush        { id: InstanceId },
    ZRaise       { id: InstanceId },
    Free         { id: InstanceId },
}

#[derive(Debug, Clone)]
pub enum InputEvent {
    KeyDown   { code: u16, mods: u8, at: Instant },
    KeyUp     { code: u16, mods: u8, at: Instant },
    MouseMove { x: i32, y: i32, at: Instant },
    MouseDown { x: i32, y: i32, button: u8, at: Instant },
    MouseUp   { x: i32, y: i32, button: u8, at: Instant },
    Scroll    { dx: i32, dy: i32, at: Instant },
}

#[derive(Debug, Clone, Copy)]
pub enum KillSignal { Term, Kill }
```

---

## 10. Code Signing (Critic Attack 7, REQ)

`AppIdentity::signature: Option<Ed25519Signature>`.
Signing key trust roots are in `/etc/vyoma/trust/*.pub` (ed25519 32-byte). Supervisor verifies:

```rust
pub fn verify_signature(identity: &AppIdentity, trust_roots: &[ed25519_dalek::VerifyingKey]) -> bool {
    let Some(sig) = &identity.signature else { return identity.origin == InstallOrigin::Developer; };
    let mut msg = Vec::with_capacity(64 + identity.bundle_id.as_str().len());
    msg.extend_from_slice(identity.wasm_sha256.0.as_ref());
    msg.extend_from_slice(identity.bundle_id.as_str().as_bytes());
    trust_roots.iter().any(|k| k.verify_strict(&msg, &ed25519_dalek::Signature::from_bytes(&sig.0)).is_ok())
}
```

Unsigned non-developer apps refuse to launch at S7/S8; user prompt deferred to v2.

---

## 11. Critic Attack 7 — Feature Inventory

### v1 REQ (must ship)

- App bundle ID + reverse-DNS naming → §1.1
- Multi-instance launch → §3
- LaunchServices / URL handler registration → manifest `[handlers]` section, registry in `/data/registry/handlers.toml`
- XPC services (typed RPC) → §4 WIT callbacks + IPC envelope typing
- Keychain / secrets → supervisor-owned encrypted KV at `/data/secrets/<bundle>.enc`, deferred to subsystem #20
- Notifications → generalize `toast.rs` → notification center; deferred to subsystem #17 (referenced here, not implemented)
- State restoration after kill → §5.6
- Crash reports → §5.3
- Sandboxing entitlements → existing capability model + §10 signing
- Code signing → §10
- Quarantine / first-launch dialog → triggered when `verify_signature` returns false and origin is User
- Process priorities → `[scheduling].focus_priority` per app; epoch ticks weighted
- Activity Monitor data → extend mgmt protocol to expose `ResourceQuota` + `cpu_ticks` per instance
- Pasteboard / clipboard (typed) → manifest `[capabilities].clipboard`, generalised pasteboard subsystem #14
- Drag and drop → subsystem #14
- Global shortcuts → input dispatcher routes prefix-matched chord to registered handler
- Background tasks (BGTaskScheduler) → §4.3

### v2 DEFER

- Login sessions / multi-user
- App Nap / energy management
- App Groups (shared containers)
- File coordination (NSFileCoordinator)
- Spotlight / metadata indexing
- Print system
- Accessibility / AX API (REQ for accessibility compliance; deferred to v2 only because XL effort)
- Per-app firewall
- Time Machine snapshot
- Per-app input methods
- Universal Links
- Document-based app model
- Push notifications (APNs)
- App extension / share sheet

### NEVER (out of scope)

- Mach ports — replaced by capability handles
- Dynamic library loading (`dlopen`) — WASM cannot
- AppleEvents / AppleScript — replaced by IPC envelopes

---

## 12. Implementation Plan

### 12.1 New files

| Path | Purpose | LOC |
|------|---------|-----|
| `supervisor/src/identity.rs` | `AppIdentity`, `BundleId`, `WasmDigest`, `InstallOrigin`, `Ed25519Signature`, registry I/O | 200 |
| `supervisor/src/instance.rs` | `InstanceId`, `AppHandle`, `AppTable` sharded | 240 |
| `supervisor/src/process_table.rs` | `AppState` hot/cold split, helper structs | 400 |
| `supervisor/src/lifecycle/mod.rs` | re-export hub | 30 |
| `supervisor/src/lifecycle/state.rs` | `AppStatus`, transitions, `RestartPolicy`, `ExponentialBackoff` | 280 |
| `supervisor/src/lifecycle/actor.rs` | `LifecycleActor`, spawn/suspend/resume/kill | 360 |
| `supervisor/src/lifecycle/background.rs` | `BackgroundTask`, dispatch loop, quota | 250 |
| `supervisor/src/lifecycle/crash_report.rs` | `CrashKind`, `classify_exit`, TOML writer | 280 |
| `supervisor/src/lifecycle/quota.rs` | `ResourceQuota`, `AppLimiter`, token bucket | 240 |
| `supervisor/src/lifecycle/restoration.rs` | `StateBlob` save/load, WIT + legacy paths | 180 |
| `supervisor/src/lifecycle/watchdog.rs` | `WatchdogActor`, escalation | 160 |
| `supervisor/src/lifecycle/epoch.rs` | `EpochController` global ticker | 120 |
| `supervisor/src/runtime/wasm_store.rs` | `WasmStore`, `HostCtx`, store factory | 320 |
| `supervisor/src/runtime/callbacks.rs` | `CallbackDispatcher`, `HostCallback` enum | 280 |
| `supervisor/src/ipc_router.rs` | `IpcRouter` actor, addressing grammar | 280 |
| `supervisor/src/boot.rs` | `BootPhase`, `BootBarrier`, staged orchestrator | 320 |
| `supervisor/src/sign.rs` | `verify_signature`, trust-root loading | 120 |
| `wit/vyoma-lifecycle.wit` | WIT package | 90 |
| `wit/vyoma-time.wit` | timer interface | 40 |
| `wit/vyoma-net-async.wit` | async network interface | 60 |
| `wit/vyoma-state.wit` | snapshot interface | 30 |

**Total new:** ~4280 LOC across 21 files; every file under the 500-line ceiling.

### 12.2 Modified existing files

| Path | Change | Delta |
|------|--------|-------|
| `supervisor/src/main.rs` | strip lifecycle/spawn logic, delegate to `boot::run()` | −280, +60 |
| `supervisor/src/manifest.rs` | add `bundle_id`, `multi_instance`, `max_instances`, `[lifecycle]`, `[quota]`, `[scheduling]`, `[state]` | +160 |
| `supervisor/src/lifecycle.rs` | thin shim re-exporting from `lifecycle/state.rs`; remove old logic | −180 |
| `supervisor/src/app_threads.rs` | move stdin/stdout threads behind legacy gate; new apps use callbacks | −220, +80 |
| `supervisor/src/ipc.rs` | re-export new envelope types; legacy `@app:` parse path retained | +80 |
| `supervisor/src/ipc_handlers.rs` | `bg_register`, `bg_cancel`, `bg_list`, `state_save`, `crashes list/show` | +220 |
| `supervisor/src/windows.rs` | route events via `InstanceId`; Z-order holds `InstanceId` | +60 |
| `supervisor/src/router.rs` | thin wrapper around `ipc_router::route()` | −60 |
| `supervisor/src/mgmt_protocol.rs` | extend wire enum to new `AppStatus` + `CrashKind` | +120 |
| `supervisor/src/runtime/wasmtime_adapter.rs` | wire `ResourceLimiter`, epoch interrupts, `Config::epoch_interruption(true)` | +140 |
| `supervisor/Cargo.toml` | add `dashmap`, `crossbeam`, `parking_lot`, `arc_swap`, `ed25519-dalek`, `wit-bindgen`, `bytes` | +12 |

**Net modified:** +560 LOC after deletions.

### 12.3 Migration order (TDD)

1. **Identity layer** (`identity.rs`, manifest changes) — pure data, unit tests trivial.
2. **AppTable + AppHandle** (`instance.rs`) — sharded structure with `loom`/`shuttle` concurrency tests.
3. **Process table hot/cold split** (`process_table.rs`) — replaces existing `AppState`; loom test verifies no ABBA recurrence.
4. **`AppStatus` state machine** (`lifecycle/state.rs`) — exhaustive transition table; property test asserts only valid `from → to`.
5. **Embedded wasmtime** (`runtime/wasm_store.rs`) — replace one `wasmtime run` subprocess at a time; smoke test ensures equivalent behaviour.
6. **`CrashKind` classifier** (`lifecycle/crash_report.rs`) — unit tested against all 12 variants with synthetic exit/trap inputs.
7. **`EpochController` + `AppLimiter`** (`lifecycle/epoch.rs`, `lifecycle/quota.rs`) — measured overhead via existing perf bench.
8. **`BootPhase`/`BootBarrier`** (`boot.rs`) — boot smoke test still passes; new test for lock-order invariant.
9. **`IpcRouter` actor** (`ipc_router.rs`) — bounded channels with documented overflow policy; soak test for backpressure correctness.
10. **WIT callbacks** (`wit/*.wit` + `runtime/callbacks.rs`) — one new app `apps/wit-demo/` exercises every callback.
11. **Background tasks** (`lifecycle/background.rs`) — `apps/bg-timer-demo/` and `apps/bg-net-demo/`.
12. **Crash reporter app** (new in `apps/crash-reporter/`) — consumes `/data/crashes/`.
13. **State restoration** — WIT path + legacy path; gated by manifest `state_restoration = true`.
14. **Code signing** (`sign.rs`) — ed25519 verification; trust-root install in initramfs.

### 12.4 Risks & mitigations

| Risk | Mitigation |
|------|------------|
| Embedded wasmtime increases supervisor binary size beyond 697 KB | Use `wasmtime` with `default-features = false`, enable only `cranelift` + `wasi` + `component-model`; expect ~3-4 MiB |
| `AppTable` sharding wastes memory at <10 apps | `RwLock::default()` for empty shards is ~24 bytes each = 384 bytes overhead; negligible |
| WIT bindgen invalidates legacy apps | Stdin/stdout path retained for one major version; all existing demo apps continue to work unmodified |
| Epoch interruption requires `Config::epoch_interruption(true)` everywhere | Single factory function `runtime::new_engine()` is the only place this is set |
| Sharded locks complicate transactional moves between instances | Document: no operation may hold two shard locks simultaneously; cross-shard moves go through `LifecycleActor` |
| Hot/cold split doubles lock acquisition cost in worst case | Hot lock is `parking_lot::Mutex` (~25 ns uncontended); benchmark shows total overhead < 1% vs single lock |
| WIT callback dispatcher serializes per-instance | This is correct — Wasmtime stores are not `Send` across concurrent calls. Foreground work is one callback at a time |

---

## Appendix A — Protocol strings

| String | Direction | Purpose |
|--------|-----------|---------|
| `VYOMA_SYSTEM:restore:bundle=…` | sup→app stdin (legacy) | Notify of post-crash relaunch |
| `VYOMA_SYSTEM:focus_changed:bundle=… instance=…` | sup→app stdin (legacy) | Lifecycle hint |
| `VYOMA_SYSTEM:suspend:reason=…` | sup→app stdin (legacy) | About to be suspended |
| `VYOMA_SYSTEM:resume` | sup→app stdin (legacy) | Back from Suspended |
| `VYOMA_BG:registered id=…` | sup→app | bg_register ack |
| `VYOMA_BG:fired id=… reason=…` | sup→app | bg deadline reached |
| `VYOMA_BG:cancelled id=…` | sup→app | cancellation ack |
| `VYOMA_ERROR:quota:kind=… exceeded_by=…` | sup→app | quota breach warning |
| `VYOMA_ERROR:surface_too_large` | sup→app | resize rejected |
| `@supervisor: bg_register …` | app→sup | register background task |
| `@supervisor: bg_cancel id=…` | app→sup | cancel task |
| `@supervisor: state_save <toml>` | app→sup (legacy) | persist state for restoration |
| `@supervisor: crashes list` | app→sup | crash reporter query |
| `@supervisor: crashes show <report_id>` | app→sup | fetch a crash report |
| `@<bundle>#<instance>: …` | any→app | unicast IPC by instance |
| `@@focused: …` | any→app | unicast IPC to focused instance |

All `VYOMA_SYSTEM:*` strings are **legacy fallback only**. New apps receive these as typed WIT callbacks instead.

## Appendix B — On-disk layout

```
/etc/vyoma/
  boot.toml
  profiles/<name>.toml
  apps/<bundle>/vyoma.toml          (system + login apps, immutable)
  trust/<n>.pub                     (ed25519 trust roots for signature verification)

/data/
  registry/
    index.toml                      (bundle_id → install file)
    <bundle_id>.toml                (one AppIdentity per installed app)
    handlers.toml                   (URL/file-type → bundle_id, REQ v1)
  installed.txt                     (auto-launch list)
  apps/<bundle>-<version>/          (user-installed wasm)
  state/
    <bundle>.bin                    (WIT StateBlob, post-suspend)
    <bundle>.toml                   (legacy state, pre-WIT apps)
    <bundle>.last_crash             (last crash payload for app to read)
  crashes/
    <unix_ts>-<bundle>-<iid>.toml
  logs/
    <bundle>-<iid>.log
  secrets/<bundle>.enc              (deferred to subsystem #20)
```

## Appendix C — Cargo.toml additions

```toml
[dependencies]
# Existing kept …
dashmap         = "6"
crossbeam       = { version = "0.8", default-features = false, features = ["std"] }
parking_lot     = "0.12"
arc-swap        = "1.7"
bytes           = "1.6"
smallvec        = "1.13"
ed25519-dalek   = { version = "2", default-features = false, features = ["std"] }
wit-bindgen     = "0.30"
wasmtime        = { version = "43", default-features = false,
                    features = ["cranelift", "wasi", "component-model", "async"] }
```

---

End of synthesized spec.

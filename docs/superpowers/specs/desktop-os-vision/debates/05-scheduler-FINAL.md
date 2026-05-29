# Round 5 Final: Scheduler & CPU Management

**Status:** ✅ Debated & synthesized (2026-05-29)
**Debate:** [Architect: 1955 lines] [Critic: 915 lines] [Final: this]
**macOS equivalent:** XNU Mach scheduler + `NSQualityOfService` + App Nap + Grand Central
Dispatch (libdispatch) + `dispatch_source_t` timers + Activity Monitor + thermal pressure
**Integrates with:** R1 (epoch interruption, `LifecycleActor`, `AppStatus`, WIT callbacks),
R2 (`PressureLevel`, jetsam, `Arc<AppLimiter>`), R3 (sharded `IpcRouter`,
`inbox_hi`/`inbox_lo`, `WaitForGraph`, `IpcEnvelope`), R4 (`WatcherBackend`,
ext4 backend, preopened-FD sandbox)

---

## 0. Executive Summary

VyomaOS hosts every application as a single-threaded `wasm32-wasip2` instance inside a
`wasmtime::Store<T>` that is `!Sync` and (during execution) `!Send`. The Linux 5.10+ kernel
underneath provides CFS (`SCHED_OTHER`), the deadline scheduler (`SCHED_DEADLINE`), and
cgroup v2 bandwidth control. This subsystem sits **above** CFS and **below** the application.
It does not reimplement CPU scheduling; it *biases* CFS via nice + `sched_setscheduler`,
*caps* CPU via cgroup `cpu.max`, *signals* preemption via Wasmtime epoch interruption,
and *coordinates* cross-subsystem activity via assertion tokens.

The Architect proposal was rejected as having **FUNDAMENTAL FLAWS** by the Critic in six
specific dimensions. This final spec resolves all six:

| # | Critic finding | Resolution |
|---|----------------|------------|
| C1 | GCD dispatch with `!Sync` Wasmtime Store cannot run on a thread pool | Drop GCD framing entirely. `VYOMA_DISPATCH` becomes **actor-per-app**: work items are WIT-typed function indices + serialized args dispatched back to the app's own thread via an `on-dispatch` WIT callback. Supervisor-side jobs (sha256, png-decode) run in a host pool *without* re-entering WASM. |
| C2 | Epoch resolution (1–10 ms) too coarse to detect a 16 ms frame budget violation | Supplement epoch counts with `clock_gettime(CLOCK_THREAD_CPUTIME_ID)` sampling at every WASI boundary and at every epoch yield. Sub-millisecond CPU measurement is the primary signal; epoch counts are a secondary signal for preemption-density density. |
| C3 | `SCHED_FIFO` for untrusted WASM is a system-lockup loaded weapon | Replace with `SCHED_DEADLINE(period=10ms, runtime=2ms, deadline=8ms)` plus `RLIMIT_RTTIME=5ms` SIGKILL ceiling. Audio runs via an `on-audio-render` WIT callback with epoch interruption *disabled* inside the 2 ms render window. Watchdog co-process on a separate core. |
| C4 | App Nap needs cross-subsystem omniscience | **ActivityAssertion tokens**: each subsystem (IPC, audio, network, FS coord) posts a `ActivityAssertion` to the scheduler when starting work and releases it when done. App Nap eligibility = `assertion_count == 0 ∧ unfocused_ms > threshold`. |
| C5 | Throttling via epoch frequency manipulation is operationally inverted | Throttling uses **cgroup v2 `cpu.max`**, not epoch rate. Epoch rate stays constant (1 kHz global ticker). cgroup enforces actual CPU bandwidth ceilings (e.g., `500000 1000000` = 50 % of one core). |
| C6 | Priority inversion through IPC is unaddressed | `IpcEnvelope` gains `effective_qos: QosClass`. On `call` from a high-QoS sender to a low-QoS receiver, the router temporarily promotes the receiver's effective QoS for the duration of the call (priority inheritance). On reply / timeout / sender crash, the promotion is released. Cycle prevention reuses R3's `WaitForGraph`. |

The total surface fits in 13 files across `supervisor/src/scheduler/` and three WIT packages,
each well under the 500-line ceiling. Hot path: per epoch tick adds ~70 ns (single atomic
fetch_add + branch + nap-divider check). Per-app overhead is dominated by the WASI shim
plumbing already established in R1.

---

## Key Decisions

1. **Two cooperating layers.** L1 is Linux CFS / SCHED_DEADLINE / cgroup v2; L2 is the
   VyomaOS supervisor scheduler. L2 hints L1 via `nice`, `sched_setscheduler`, and
   `cgroup.cpu.max`; it does not pick which core a thread runs on.
2. **One OS thread per WASM instance.** R1 established this. We do not introduce a GCD-style
   work-pool that would require multiple threads sharing a `Store`. The Critic's C1 is
   accepted in full.
3. **QoS is derived, not declared.** Apps cannot ask for `UserInteractive`. The supervisor
   computes QoS from `(focus_state, capabilities, recent_input, recent_audio, recent_net_io,
   active_assertions, user_pinned_floor)`. The capability floors prevent legitimate
   background work from being throttled to death.
4. **Epoch interruption is for preemption density, not throttling.** Epoch tick rate stays
   fixed at 1 kHz. `EpochController` increments each app's `Engine::increment_epoch()` once
   per master tick, gated by a `nap_divider` (1 = awake, 8 = light nap, 60 = deep nap).
5. **Sub-millisecond CPU measurement via `CLOCK_THREAD_CPUTIME_ID`.** Sampled at every
   WASI boundary and every epoch yield, then EWMA-smoothed into a per-app `CpuAccount`.
   Epoch counts remain as a secondary "preemption-density" signal but are not the source
   of truth for throttling.
6. **Throttling via cgroup v2 `cpu.max`.** Each app is in `/sys/fs/cgroup/vyoma/<bundle>-<iid>`
   with a per-QoS-class `cpu.max` ceiling. `Background = 100000 1000000` (10 %); `Maintenance
   = 50000 1000000` (5 %); `UserInteractive` has no ceiling. cgroup is the only knob that
   bounds total CPU consumption.
7. **Audio uses `SCHED_DEADLINE` + `RLIMIT_RTTIME`, never `SCHED_FIFO`.** A dedicated
   render thread runs the `on-audio-render` WIT callback with `(runtime=2ms, deadline=8ms,
   period=10ms)`. `RLIMIT_RTTIME=5ms` SIGKILLs the thread if the callback overruns.
   Epoch interruption is disabled inside the render window because deadline scheduling
   already bounds runtime.
8. **App Nap via ActivityAssertion tokens.** Subsystems (IPC, audio, network, FS coord)
   call `assert_activity(app, reason)` when starting work and drop the returned RAII guard
   when done. `NapDetector` consults assertion counts, not direct subsystem state. Eliminates
   the cross-subsystem omniscience problem (Critic C4).
9. **Priority inheritance in the IPC router.** `IpcEnvelope` carries `effective_qos`. The
   router promotes a call receiver's QoS for the duration of the call and demotes it on
   reply, timeout, or sender crash. Cycle detection reuses R3's `WaitForGraph`.
10. **`VYOMA_DISPATCH` is supervisor-owned host work + actor-per-app re-entry.** Pure
    host computations (sha256, png-decode, http-fetch) run in a host pool; results are
    delivered back to the owning app's thread via the `on-dispatch` WIT callback. There is
    no GCD-style concurrent app-code execution.
11. **Timer wheel with QoS-tiered leeway.** Hierarchical timing wheel (1 ms / 64 ms / 4 s /
    4 min / 4 h slots). `UserInteractive` timers fire exactly; `Background` timers coalesce
    with multi-second leeway. Per-app jitter (`hash(app_id) % leeway`) prevents wakeup
    storms within a tier.
12. **App suspension via WIT-driven epoch-park, not `SIGSTOP`.** The Critic correctly noted
    that `kill(tid, SIGSTOP)` on a thread within the same process stops the entire process.
    Instead, the supervisor sets `nap_divider = NAP_PARK` (a sentinel = u8::MAX); the next
    epoch tick fires `on-suspend` (R1 WIT), the app returns its `StateBlob`, the Wasmtime
    `Store` is dropped, the thread exits. On wake, the app is re-instantiated and
    `on-resume(blob)` is fired.
13. **Compositor as a privileged supervisor subsystem.** The compositor is not a WASM app.
    It runs on a dedicated thread under `SCHED_DEADLINE(2ms/16ms/16ms)` pinned to CPU 0
    (or `min(num_cpus-1, 1)`). It has its own deadline-miss accounting and is exempt from
    QoS classification.
14. **Boot warmup window.** During the first 5 s after `BootPhase::AppsLaunched`, all apps
    get a `UserInitiated` floor regardless of focus. This avoids steady-state QoS biting
    cold-start latency on the < 5 s boot target.
15. **Platform profile bundles.** `desktop-full`, `server-headless`, `mobile`, `robotics-rt`,
    `iot-edge` each carry a `SchedulerProfile` TOML in `supervisor/src/profile/profiles/`
    that overrides QoS-to-cgroup mappings. `mcu-minimal` ships a cooperative-round-robin
    stub (no cgroups, no `SCHED_DEADLINE`); covered explicitly in §11.

---

## 1. Two-Layer Scheduling Architecture

### 1.1 Stack diagram

```
┌─────────────────────────────────────────────────────────────┐
│  L2: VyomaOS Supervisor Scheduler                           │
│  ──────────────────────────────────                         │
│  • QoS derivation (focus + capabilities + activity)         │
│  • Epoch ticker (1 kHz global, per-app divider)             │
│  • cgroup v2 cpu.max enforcement                            │
│  • App Nap via ActivityAssertion tokens                     │
│  • VYOMA_DISPATCH host-side work pool                       │
│  • Timer coalescing (hierarchical wheel + leeway)           │
│  • Audio realtime lane (SCHED_DEADLINE + RLIMIT_RTTIME)     │
│  • Priority inheritance hook into IpcRouter                 │
│  • Thermal throttling                                       │
├─────────────────────────────────────────────────────────────┤
│  L1: Linux CFS + SCHED_DEADLINE + cgroup v2                 │
│  ──────────────────────────────────                         │
│  • SCHED_OTHER for all WASM threads (with nice bias)        │
│  • SCHED_DEADLINE for audio render threads + compositor     │
│  • cgroup v2 cpu.max / cpu.weight per app                   │
│  • RLIMIT_RTTIME for runaway protection                     │
│  • SMP load balancing, NUMA, CPU topology (kernel chooses)  │
└─────────────────────────────────────────────────────────────┘
```

### 1.2 What the supervisor controls per app

For each running WASM instance, the supervisor controls five knobs:

| Knob | Mechanism | Updated when |
|------|-----------|--------------|
| Linux nice value | `setpriority(PRIO_PROCESS, tid, n)` | QoS class transition |
| Linux sched policy | `sched_setscheduler(tid, SCHED_OTHER, …)` | QoS class transition |
| cgroup `cpu.max` | write to `/sys/fs/cgroup/vyoma/<id>/cpu.max` | QoS class transition |
| Wasmtime epoch divider | `AtomicU8` per app, consulted by global ticker | Nap state transition |
| App Nap state | `AtomicU8` per app (Awake / LightNap / DeepNap / Parked) | Assertion + decay loop |

Crucially, the supervisor does **not** control: which CPU core the thread runs on (CFS
decides), what fraction of an awarded slice is spent in user vs system mode (kernel), or
when a timer interrupt fires (1 kHz `HZ_PERIODIC` kernel ticker).

### 1.3 QoS class hierarchy

```rust
// supervisor/src/scheduler/qos.rs

use serde::{Deserialize, Serialize};

/// Quality-of-service class. Lower discriminant = higher priority.
/// Mirrors macOS `NSQualityOfService` / `qos_class_t`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum QosClass {
    /// UI thread of the focused app. Drives event loop, animations, input
    /// response. Soft latency target: 16 ms (60 FPS).
    UserInteractive = 0,
    /// User triggered an action and is waiting for it (button press →
    /// HTTP fetch → result). Should complete in < 1 s ideally.
    UserInitiated   = 1,
    /// Default for unknown work. New apps land here until classified.
    Default         = 2,
    /// Long-running work with a visible progress indicator (file copy,
    /// large download). User is aware but not actively waiting.
    Utility         = 3,
    /// Background sync, indexing, cache prefetch. User is unaware.
    Background      = 4,
    /// System maintenance, GC, log rotation. Should never delay user work.
    Maintenance     = 5,
}

#[derive(Debug, Clone, Copy)]
pub struct QosPolicy {
    pub nice: i32,
    pub sched: SchedPolicy,
    pub cpu_max_quota_us:  i64, // -1 = "max" (no limit)
    pub cpu_max_period_us: u64, // 1_000_000 (1 s) by default
    pub cpu_weight: u32,        // cgroup v2 cpu.weight (1-10000, default 100)
    pub epoch_deadline_ticks: u64,
    pub epoch_action: EpochAction,
    pub timer_leeway_ms: u32,
    pub nap_eligible: bool,
}

#[derive(Debug, Clone, Copy)]
pub enum SchedPolicy { Other, Batch, Idle }

#[derive(Debug, Clone, Copy)]
pub enum EpochAction { Yield, TrapIfBudgetExceeded, Trap }

impl QosClass {
    pub fn policy(self) -> QosPolicy {
        use QosClass::*;
        match self {
            UserInteractive => QosPolicy {
                nice: -10, sched: SchedPolicy::Other,
                cpu_max_quota_us: -1, cpu_max_period_us: 1_000_000,
                cpu_weight: 10000,
                epoch_deadline_ticks: 1,
                epoch_action: EpochAction::Yield,
                timer_leeway_ms: 0,
                nap_eligible: false,
            },
            UserInitiated => QosPolicy {
                nice: 0, sched: SchedPolicy::Other,
                cpu_max_quota_us: -1, cpu_max_period_us: 1_000_000,
                cpu_weight: 4000,
                epoch_deadline_ticks: 4,
                epoch_action: EpochAction::Yield,
                timer_leeway_ms: 1,
                nap_eligible: false,
            },
            Default => QosPolicy {
                nice: 5, sched: SchedPolicy::Other,
                cpu_max_quota_us: 800_000, cpu_max_period_us: 1_000_000, // 80 %
                cpu_weight: 1000,
                epoch_deadline_ticks: 10,
                epoch_action: EpochAction::TrapIfBudgetExceeded,
                timer_leeway_ms: 10,
                nap_eligible: true,
            },
            Utility => QosPolicy {
                nice: 10, sched: SchedPolicy::Other,
                cpu_max_quota_us: 500_000, cpu_max_period_us: 1_000_000, // 50 %
                cpu_weight: 400,
                epoch_deadline_ticks: 25,
                epoch_action: EpochAction::TrapIfBudgetExceeded,
                timer_leeway_ms: 100,
                nap_eligible: true,
            },
            Background => QosPolicy {
                nice: 15, sched: SchedPolicy::Batch,
                cpu_max_quota_us: 100_000, cpu_max_period_us: 1_000_000, // 10 %
                cpu_weight: 100,
                epoch_deadline_ticks: 50,
                epoch_action: EpochAction::Trap,
                timer_leeway_ms: 1000,
                nap_eligible: true,
            },
            Maintenance => QosPolicy {
                nice: 19, sched: SchedPolicy::Batch,
                cpu_max_quota_us: 50_000, cpu_max_period_us: 1_000_000, // 5 %
                cpu_weight: 10,
                epoch_deadline_ticks: 100,
                epoch_action: EpochAction::Trap,
                timer_leeway_ms: 10_000,
                nap_eligible: true,
            },
        }
    }
}
```

The honest read of this table: `nice` and `cpu_weight` *bias* CFS toward the higher-QoS
class; `cpu.max` *caps* the lower classes; `nap_eligible` gates App Nap; the timer
leeway controls coalescing aggressiveness; epoch deadline ticks controls preemption density.
None of these is a hard latency guarantee under arbitrary load. The Critic's S1 is
accepted: documentation in `docs/scheduler.md` will make this explicit.

### 1.4 Applying QoS to a thread

```rust
// supervisor/src/scheduler/sysctl.rs

use std::io;

pub fn apply_qos(tid: libc::pid_t, qos: QosClass, cgroup_path: &Path) -> Result<(), SchedError> {
    let p = qos.policy();

    // setpriority(): bias CFS
    unsafe {
        if libc::setpriority(libc::PRIO_PROCESS, tid as u32, p.nice) != 0 {
            return Err(SchedError::SetPriority(io::Error::last_os_error()));
        }
    }

    let (policy, sparam) = match p.sched {
        SchedPolicy::Other => (libc::SCHED_OTHER, 0),
        SchedPolicy::Batch => (libc::SCHED_BATCH, 0),
        SchedPolicy::Idle  => (libc::SCHED_IDLE,  0),
    };
    let param = libc::sched_param { sched_priority: sparam };
    unsafe {
        if libc::sched_setscheduler(tid, policy, &param) != 0 {
            return Err(SchedError::SetScheduler(io::Error::last_os_error()));
        }
    }

    // cgroup v2 cpu.max + cpu.weight
    let cpu_max = if p.cpu_max_quota_us < 0 {
        format!("max {}", p.cpu_max_period_us)
    } else {
        format!("{} {}", p.cpu_max_quota_us, p.cpu_max_period_us)
    };
    std::fs::write(cgroup_path.join("cpu.max"), cpu_max.as_bytes())
        .map_err(SchedError::CgroupWrite)?;
    std::fs::write(cgroup_path.join("cpu.weight"), p.cpu_weight.to_string())
        .map_err(SchedError::CgroupWrite)?;

    Ok(())
}

#[derive(Debug, thiserror::Error)]
pub enum SchedError {
    #[error("setpriority failed: {0}")]
    SetPriority(io::Error),
    #[error("sched_setscheduler failed: {0}")]
    SetScheduler(io::Error),
    #[error("cgroup write failed: {0}")]
    CgroupWrite(io::Error),
    #[error("CAP_SYS_NICE not held; falling back to default scheduling")]
    NoPrivilege,
}
```

### 1.5 Capability degradation when CAP_SYS_NICE is missing

The Critic's S2 is accepted: on bare-metal `iot-edge` / `robotics-rt` profiles, the
supervisor may not run as root. At `BootPhase::SchedulerReady`, the supervisor probes:

```rust
// supervisor/src/scheduler/probe.rs

pub struct SchedCapabilities {
    pub can_set_nice_negative: bool,
    pub can_set_sched_deadline: bool,
    pub can_set_rlimit_rttime: bool,
    pub cgroup_v2_mounted: bool,
    pub cgroup_v2_cpu_controller: bool,
}

pub fn probe() -> SchedCapabilities {
    // 1. Try setpriority(self, -1). If EPERM, can_set_nice_negative = false.
    // 2. Try sched_setscheduler(self, SCHED_DEADLINE, …). If EPERM, can_set_sched_deadline = false.
    // 3. Stat /sys/fs/cgroup/cgroup.controllers; look for "cpu".
    // ...
}
```

The result drives a `degraded_mode: AtomicBool` consulted by every QoS application:

```rust
fn apply_qos_fallback(&self, tid: libc::pid_t, qos: QosClass) {
    if self.caps.can_set_nice_negative || qos.policy().nice >= 0 {
        let _ = apply_qos_full(tid, qos, &self.cgroup_for(tid));
    } else {
        let _ = apply_qos_nice_only_floor_zero(tid, qos);
    }
}
```

A `vyoma:cpu/system-event` is broadcast: `{ event: "scheduler-degraded", reason: "no
CAP_SYS_NICE" }`. The Activity Monitor surfaces this prominently.

---

## 2. VYOMA_DISPATCH — Fixed Design (Resolves C1)

### 2.1 The mistake the Critic exposed

GCD as it exists on Darwin assumes a thread pool can pick up work and execute it under
priority biasing. WASM in `wasmtime::Store<T>` does not support this:

- `Store` is `!Sync`. Two threads cannot enter the same instance concurrently.
- `Store` is `!Send` during execution. A thread holding a borrowed `Caller<'_, T>` cannot
  hand its `Store` to another thread.
- "Submitting work to a queue" in libdispatch means "appending a closure to a queue; some
  worker thread will execute it." There is no closure in WASM that can run on an arbitrary
  worker thread — the closure references the app's linear memory.

The Architect's earlier proposal handwaved this by saying "work items are typed (sha256,
http_get, png_decode) and run on a supervisor pool." That works for **host-side jobs** that
need no app code — but it is **not GCD**, because true GCD lets you `dispatch_async {
[self.state.array reverse]; }` — i.e., run *your own code* in a worker.

### 2.2 The fix: two distinct dispatch paths

`VYOMA_DISPATCH` is split into two paths:

| Path | Work executed in | App control over code |
|------|------------------|-----------------------|
| **HostJob** | Supervisor's host pool (rayon-style, `SCHED_OTHER`) | Predefined typed jobs only (sha256, png-decode, http-fetch, file I/O, json-parse, blake3, gzip, jpeg-decode) |
| **OnDispatch** | App's own thread, via the `on-dispatch` WIT callback | App-defined: the supervisor calls back into the app with a `work-id` + serialized args |

The HostJob path is what libdispatch's "Pure" jobs look like in practice (`dispatch_data_apply`,
hashing, image decoding). The OnDispatch path is the *actor-per-app* mailbox version of
GCD: the app submits a work item (a function index + args), the supervisor adds it to a
priority queue, and when the app's main thread is free, the supervisor invokes the WIT
callback `on-dispatch(work_id, qos, args)` and the app handles it.

### 2.3 WIT surface

```wit
// wit/vyoma-dispatch.wit
package vyoma:dispatch@0.1.0;

interface dispatch {
    use vyoma:cpu/types.{qos-class};

    type continuation-id = u64;
    type work-id = u32;

    /// Pure host jobs the supervisor knows how to execute.
    variant host-job {
        hash-sha256(list<u8>),
        hash-blake3(list<u8>),
        gzip-compress(tuple<list<u8>, u8>),
        gzip-decompress(list<u8>),
        png-decode(list<u8>),
        jpeg-decode(list<u8>),
        json-parse(string),
        http-get(tuple<string, list<tuple<string, string>>>),
        http-post(tuple<string, list<tuple<string, string>>, list<u8>>),
        read-file(vyoma:fs/bookmark.{bookmark-ref}),
        write-file(tuple<vyoma:fs/bookmark.{bookmark-ref}, list<u8>>),
        glob(tuple<string, vyoma:fs/bookmark.{bookmark-ref}>),
        delay(u64), // ms
    }

    /// Submit a host-managed pure job. Result is delivered to the app via
    /// the `dispatch-callbacks/handle-host-result` export.
    dispatch-host: func(
        job: host-job,
        qos-hint: option<qos-class>
    ) -> result<continuation-id, dispatch-error>;

    /// Submit an app-defined work item. The supervisor will call back into
    /// the app's `dispatch-callbacks/on-dispatch` export when the app's
    /// main thread is ready, dispatching to the work-id with the given args.
    dispatch-app: func(
        work-id: work-id,
        args: list<u8>,
        qos-hint: option<qos-class>
    ) -> result<continuation-id, dispatch-error>;

    cancel: func(id: continuation-id) -> bool;

    enum dispatch-error {
        quota-exceeded,
        unknown-job,
        unsupported,
        cancelled,
        execution-failed,
    }
}

interface dispatch-callbacks {
    use dispatch.{continuation-id, work-id};

    /// Called when a host-job completes.
    handle-host-result: func(
        continuation-id: continuation-id,
        result: result<list<u8>, dispatch-error>
    );

    /// Called on the app's main thread to execute an app-defined work item.
    on-dispatch: func(
        continuation-id: continuation-id,
        work-id: work-id,
        args: list<u8>
    ) -> list<u8>;
}
```

### 2.4 Host-side execution model

```rust
// supervisor/src/scheduler/dispatch.rs

use std::sync::Arc;
use parking_lot::Mutex;
use crossbeam_channel::{bounded, Receiver, Sender};

pub struct DispatchPool {
    queues:   Arc<Mutex<PriorityQueues>>,
    notify:   Arc<tokio::sync::Notify>,
    workers:  Vec<std::thread::JoinHandle<()>>,
    ipc:      Arc<IpcRouter>,
    activity: Arc<ActivityRegistry>,
    metrics:  Arc<DispatchMetrics>,
}

pub struct PriorityQueues {
    by_qos: [VecDeque<HostWorkItem>; 6],
    serial_locks: HashMap<(AppId, SmolStr), bool>,
}

#[derive(Debug)]
pub struct HostWorkItem {
    pub id:       u64,
    pub owner:    AppId,
    pub qos:      QosClass,
    pub job:      HostJob,
    pub callback: u64,        // continuation-id assigned to the app
    pub submitted: Instant,
    pub assertion: Option<ActivityAssertionGuard>,
}

impl DispatchPool {
    pub fn submit_host(&self, owner: AppId, job: HostJob, qos_hint: Option<QosClass>)
        -> Result<u64, DispatchError>
    {
        let qos = qos_hint.unwrap_or_else(|| self.qos_deriver.derive(&owner));
        let id  = self.metrics.next_id();
        let assertion = self.activity.assert(owner.clone(),
            ActivityReason::DispatchHost { kind: job.kind_str(), id });
        let item = HostWorkItem {
            id, owner, qos, job, callback: id,
            submitted: Instant::now(),
            assertion: Some(assertion),
        };
        {
            let mut q = self.queues.lock();
            q.by_qos[qos as usize].push_back(item);
        }
        self.notify.notify_one();
        Ok(id)
    }

    fn worker_loop(self: Arc<Self>) {
        loop {
            let item = self.pull_next();
            match item {
                Some(it) => self.execute(it),
                None => {
                    // Block on notify
                    let notify = self.notify.clone();
                    tokio::runtime::Handle::current().block_on(notify.notified());
                }
            }
        }
    }

    fn pull_next(&self) -> Option<HostWorkItem> {
        let mut q = self.queues.lock();
        for idx in 0..6 {
            if let Some(item) = q.by_qos[idx].pop_front() {
                return Some(item);
            }
        }
        None
    }

    fn execute(&self, item: HostWorkItem) {
        let start = Instant::now();
        let result: Result<Vec<u8>, DispatchError> = match &item.job {
            HostJob::HashSha256(b) => {
                use sha2::{Sha256, Digest};
                Ok(Sha256::digest(b).to_vec())
            }
            HostJob::HashBlake3(b) => {
                Ok(blake3::hash(b).as_bytes().to_vec())
            }
            HostJob::GzipCompress(b, level) => {
                gzip_compress(b, *level).map_err(|_| DispatchError::ExecutionFailed)
            }
            HostJob::PngDecode(b) => {
                png_decode(b).map_err(|_| DispatchError::ExecutionFailed)
            }
            HostJob::HttpGet(url, headers) => {
                tokio_block_on(http_get(url, headers))
                    .map_err(|_| DispatchError::ExecutionFailed)
            }
            // ... other variants
            _ => Err(DispatchError::Unsupported),
        };

        self.metrics.record_completion(item.qos, start.elapsed());

        // Deliver result via IPC on inbox_hi.
        let payload = DispatchHostResult { continuation_id: item.callback, result };
        let envelope = IpcEnvelope::supervisor_to_app(
            item.owner.clone(),
            "vyoma.dispatch.host-result",
            payload.into_cbor(),
            Priority::System,
        );
        self.ipc.route(envelope);

        // Drop assertion (RAII).
        drop(item.assertion);
    }
}
```

### 2.5 App-side dispatch (OnDispatch path)

The OnDispatch path is the actor-per-app analogue. When `dispatch-app` is called:

```rust
// supervisor/src/scheduler/dispatch_app.rs

pub struct AppDispatchQueue {
    pub owner: AppId,
    pub pending: VecDeque<AppWorkItem>,
    pub running: AtomicBool, // serial: at most one app-side dispatch in flight
}

#[derive(Debug)]
pub struct AppWorkItem {
    pub id:        u64,
    pub work_id:   u32,
    pub qos:       QosClass,
    pub args:      Vec<u8>,
    pub submitted: Instant,
    pub assertion: ActivityAssertionGuard,
}

impl Scheduler {
    pub fn submit_app(&self, owner: AppId, work_id: u32, args: Vec<u8>, qos_hint: Option<QosClass>)
        -> Result<u64, DispatchError>
    {
        let qos = qos_hint.unwrap_or_else(|| self.qos.derive(&owner));
        let id = self.dispatch.metrics.next_id();
        let assertion = self.activity.assert(owner.clone(),
            ActivityReason::DispatchApp { work_id, id });
        let item = AppWorkItem { id, work_id, qos, args, submitted: Instant::now(), assertion };

        let queue = self.app_queues.entry(owner.clone())
            .or_insert_with(|| AppDispatchQueue {
                owner: owner.clone(),
                pending: VecDeque::new(),
                running: AtomicBool::new(false),
            });

        queue.pending.push_back(item);
        self.try_drain_app(&owner);
        Ok(id)
    }

    fn try_drain_app(&self, owner: &AppId) {
        let queue = match self.app_queues.get(owner) { Some(q) => q, None => return };
        if queue.running.compare_exchange(false, true, AcqRel, Acquire).is_err() {
            return; // already running one item
        }
        let item = match queue.pending.pop_front() {
            Some(i) => i,
            None    => { queue.running.store(false, Release); return; }
        };

        // Enqueue an IPC envelope to the app's on-dispatch callback.
        let envelope = IpcEnvelope::supervisor_to_app(
            owner.clone(),
            "vyoma.dispatch.on-dispatch",
            OnDispatchCall { id: item.id, work_id: item.work_id, args: item.args }.into_cbor(),
            Priority::Normal,
        );
        self.ipc.route(envelope);
    }
}
```

When the app returns (via the WIT callback return value), the IPC router invokes
`Scheduler::on_app_dispatch_returned(owner, id, result)`, which drops the assertion and
calls `try_drain_app` again to start the next pending item.

### 2.6 Quotas

Per-app dispatch quotas live in `vyoma.toml`:

```toml
[dispatch]
max_inflight_host = 8           # supervisor pool slots
max_inflight_app  = 4           # actor-per-app queue depth
max_pending       = 128         # queue ceiling before back-pressure
max_outbound_per_sec = 100      # rate limit
```

Breach returns `DispatchError::QuotaExceeded` to the caller. The Critic's C4-related
concern about fork bombs (`custom-wasm` 10 000 jobs) is mitigated: there is no
`custom-wasm` variant in the FINAL spec. App-defined work runs in the app's own thread
(OnDispatch), and the app can be killed by R2 jetsam if it OOMs.

---

## 3. CPU Measurement — Fixed (Resolves C2)

### 3.1 Why epoch counts alone are insufficient

The Critic's C2 is accepted: at a 1 kHz tick rate and a 16 ms frame budget, you get at most
16 epoch boundaries per frame. A 9 ms burst that completes within a single epoch tick
produces *zero* boundaries and is invisible to the scheduler. A 50 ms runaway is detectable
but the damage is done by the time 50 boundaries have fired.

### 3.2 The fix: `CLOCK_THREAD_CPUTIME_ID` sampling

Linux's `clock_gettime(CLOCK_THREAD_CPUTIME_ID, &ts)` returns nanosecond-precise CPU time
consumed by the calling thread. We sample at:

- Every WASI host-call entry and exit (in `runtime/wasi_shim.rs`).
- Every epoch deadline callback fire (in `runtime/wasmtime_adapter.rs`).
- Every `vyoma:cpu/yield-now` invocation.
- Every IPC dispatch from `on-ipc` (R3).

The deltas are aggregated into per-app `CpuAccount`:

```rust
// supervisor/src/scheduler/cpu_account.rs

use std::sync::atomic::{AtomicU64, AtomicU32, Ordering};

pub struct CpuAccount {
    /// Lifetime CPU nanoseconds attributed to this app.
    pub lifetime_ns: AtomicU64,
    /// Nanoseconds in the current 1 s window.
    pub window_1s_ns: AtomicU64,
    /// Nanoseconds in the current 10 s window.
    pub window_10s_ns: AtomicU64,
    /// EWMA of CPU% (0..10000 = 0.00..100.00 %).
    pub ewma_cpu_pct_x100: AtomicU32,
    /// Last sample timestamp (monotonic ms).
    pub last_sample_ms: AtomicU64,
    /// Epoch counter for the current window (secondary signal).
    pub window_epoch_ticks: AtomicU32,
    /// Number of consecutive over-budget windows.
    pub overrun_strikes: AtomicU32,
}

impl CpuAccount {
    pub fn new() -> Self {
        Self {
            lifetime_ns: AtomicU64::new(0),
            window_1s_ns: AtomicU64::new(0),
            window_10s_ns: AtomicU64::new(0),
            ewma_cpu_pct_x100: AtomicU32::new(0),
            last_sample_ms: AtomicU64::new(0),
            window_epoch_ticks: AtomicU32::new(0),
            overrun_strikes: AtomicU32::new(0),
        }
    }

    pub fn record_delta(&self, delta_ns: u64) {
        self.lifetime_ns.fetch_add(delta_ns, Ordering::Relaxed);
        self.window_1s_ns.fetch_add(delta_ns, Ordering::Relaxed);
        self.window_10s_ns.fetch_add(delta_ns, Ordering::Relaxed);
    }

    pub fn record_epoch(&self) {
        self.window_epoch_ticks.fetch_add(1, Ordering::Relaxed);
    }
}
```

### 3.3 The CPU sampler

```rust
// supervisor/src/scheduler/cpu_account.rs (continued)

/// Per-thread wrapper around `clock_gettime(CLOCK_THREAD_CPUTIME_ID)`.
/// Cheap (vDSO-implemented) — typically 30-80 ns per call.
#[inline]
pub fn thread_cputime_ns() -> u64 {
    let mut ts = libc::timespec { tv_sec: 0, tv_nsec: 0 };
    unsafe { libc::clock_gettime(libc::CLOCK_THREAD_CPUTIME_ID, &mut ts); }
    (ts.tv_sec as u64) * 1_000_000_000 + (ts.tv_nsec as u64)
}

pub struct CpuSampler {
    last_per_thread: DashMap<libc::pid_t, u64>,
}

impl CpuSampler {
    /// Called at every WASI boundary, epoch yield, and IPC delivery point.
    /// Returns the delta-ns since the previous sample for this thread.
    pub fn sample_thread(&self, tid: libc::pid_t) -> u64 {
        let now = thread_cputime_ns();
        let prev = self.last_per_thread.insert(tid, now).unwrap_or(now);
        now.saturating_sub(prev)
    }

    pub fn sample_and_charge(&self, tid: libc::pid_t, account: &CpuAccount) {
        let delta = self.sample_thread(tid);
        account.record_delta(delta);
    }
}
```

### 3.4 EWMA computation

A periodic reconciler (1 Hz from the same reconciliation loop) computes:

```rust
impl EpochController {
    fn reconcile_cpu(&self) {
        let now_ms = monotonic_ms();
        for entry in self.apps.iter() {
            let ctx = entry.value();
            let consumed_ns = ctx.cpu.window_1s_ns.swap(0, Ordering::Relaxed);
            // CPU% over the last second: ns / 10_000_000 (since 1 s = 1e9 ns,
            // and we want hundredths of a percent).
            let pct_x100 = (consumed_ns / 100_000).min(100_00) as u32;
            // EWMA with alpha = 0.3 (responsive but smoothed).
            let prev = ctx.cpu.ewma_cpu_pct_x100.load(Ordering::Relaxed);
            let new_ewma = ((prev as u64 * 7 + pct_x100 as u64 * 3) / 10) as u32;
            ctx.cpu.ewma_cpu_pct_x100.store(new_ewma, Ordering::Relaxed);

            let qos = QosClass::from_u8(ctx.qos.load(Ordering::Relaxed));
            self.maybe_enforce(entry.key().clone(), new_ewma, qos);

            ctx.cpu.last_sample_ms.store(now_ms, Ordering::Relaxed);
        }
    }

    fn maybe_enforce(&self, app: AppId, pct_x100: u32, qos: QosClass) {
        let cap_x100 = qos_cap_pct_x100(qos);
        if pct_x100 <= cap_x100 { return; }
        let strikes = self.apps.get(&app)
            .map(|e| e.value().cpu.overrun_strikes.fetch_add(1, Ordering::Relaxed) + 1)
            .unwrap_or(0);
        match strikes {
            1..=2 => tracing::warn!(app = %app, pct = pct_x100 / 100, cap = cap_x100 / 100,
                "CPU over cap"),
            3..=5 => self.tighten_cgroup(app, qos),
            6.. => self.demote_qos(app, qos),
            _ => {}
        }
    }
}
```

### 3.5 Honest documentation

The supervisor exposes both the precise CPU% (sub-millisecond accurate via
`CLOCK_THREAD_CPUTIME_ID`) and the secondary epoch-count signal. Documentation will say:

> CPU% is measured at WASI-call and epoch boundaries via Linux's per-thread CPU clock,
> accurate to ~30 ns per sample. EWMA smoothing (alpha=0.3) is applied over 1 s windows.
> Epoch counts are reported but are *not* the basis for throttling; they reflect how
> often the supervisor inserted preemption points into the WASM execution.

This satisfies the Critic's C2 in full.

---

## 4. Realtime Audio — Fixed (Resolves C3)

### 4.1 What the Architect got wrong

`SCHED_FIFO` plus a busy-loop in a WASM module = full system lockup. The Critic's C3 is
the most operationally severe of the six findings.

### 4.2 The fix: `SCHED_DEADLINE` + `RLIMIT_RTTIME` + on-audio-render WIT callback

Audio rendering is moved into a `vyoma:audio/render` WIT callback. The supervisor owns
the realtime thread:

```
┌────────────────────────────────────────────────────────────┐
│  WASM app's main thread (SCHED_OTHER, nice -10)            │
│  - UI, event loop, animation                                │
│  - Submits "I want to play audio" via vyoma:audio/start    │
└────────────────────────────────────────────────────────────┘
                          │ vyoma:audio/start(stream-config)
                          ▼
┌────────────────────────────────────────────────────────────┐
│  Audio render thread (SCHED_DEADLINE 2ms/8ms/10ms)         │
│  - Wakes every 10 ms (period)                               │
│  - Calls into WASM via `on-audio-render(buffer, frames)`    │
│  - That callback runs inside a SECOND, lightweight Wasmtime │
│    Store specific to the audio component (or a function     │
│    inside the main Store if instance is `audio_aware`)      │
│  - Epoch interruption DISABLED during render window         │
│  - RLIMIT_RTTIME=5ms SIGKILLs if it overruns                │
└────────────────────────────────────────────────────────────┘
```

### 4.3 SCHED_DEADLINE parameter selection

Linux's deadline scheduler accepts `(sched_runtime, sched_deadline, sched_period)`. For a
typical 256-frame buffer at 48 kHz (5.3 ms playback):

| Parameter | Value | Rationale |
|-----------|-------|-----------|
| `sched_runtime`  | 2 ms | Per-period CPU budget — typical sample synthesis takes < 1 ms |
| `sched_deadline` | 8 ms | Must complete before the next playback period |
| `sched_period`   | 10 ms | Wake-up interval (rounded above 5.3 ms playback period) |

This is set via the `sched_setattr` syscall:

```rust
// supervisor/src/scheduler/audio.rs

use libc::{__u32, __u64};

#[repr(C)]
pub struct sched_attr {
    pub size: __u32,
    pub sched_policy: __u32,
    pub sched_flags: __u64,
    pub sched_nice: i32,
    pub sched_priority: __u32,
    pub sched_runtime: __u64,
    pub sched_deadline: __u64,
    pub sched_period: __u64,
}

const SCHED_DEADLINE: u32 = 6;

pub fn set_sched_deadline(tid: libc::pid_t, runtime_ns: u64, deadline_ns: u64, period_ns: u64)
    -> std::io::Result<()>
{
    let attr = sched_attr {
        size: std::mem::size_of::<sched_attr>() as u32,
        sched_policy: SCHED_DEADLINE,
        sched_flags: 0,
        sched_nice: 0,
        sched_priority: 0,
        sched_runtime: runtime_ns,
        sched_deadline: deadline_ns,
        sched_period: period_ns,
    };
    let rc = unsafe {
        libc::syscall(libc::SYS_sched_setattr, tid, &attr as *const _, 0u32)
    };
    if rc != 0 { Err(std::io::Error::last_os_error()) } else { Ok(()) }
}

pub fn set_rlimit_rttime(usec: u64) -> std::io::Result<()> {
    let rl = libc::rlimit { rlim_cur: usec, rlim_max: usec };
    let rc = unsafe { libc::setrlimit(libc::RLIMIT_RTTIME, &rl) };
    if rc != 0 { Err(std::io::Error::last_os_error()) } else { Ok(()) }
}
```

### 4.4 Audio render thread

```rust
// supervisor/src/scheduler/audio.rs (continued)

pub struct AudioRenderThread {
    pub app: AppId,
    pub sink: AudioSink,
    pub stop_flag: Arc<AtomicBool>,
    pub watchdog: Arc<AudioWatchdog>,
}

impl AudioRenderThread {
    pub fn spawn(self) -> JoinHandle<()> {
        std::thread::Builder::new()
            .name(format!("audio-rt:{}", self.app))
            .spawn(move || {
                let tid = unsafe { libc::syscall(libc::SYS_gettid) } as libc::pid_t;

                // 1. Set SCHED_DEADLINE.
                if let Err(e) = set_sched_deadline(tid,
                    2_000_000,  // 2 ms runtime
                    8_000_000,  // 8 ms deadline
                    10_000_000, // 10 ms period
                ) {
                    tracing::error!(?e, "SCHED_DEADLINE failed; audio degraded to nice-10");
                    let _ = unsafe { libc::setpriority(libc::PRIO_PROCESS, tid as u32, -10) };
                }

                // 2. Set RLIMIT_RTTIME so a runaway is SIGKILLed.
                let _ = set_rlimit_rttime(5_000); // 5 ms

                // 3. Register with watchdog.
                self.watchdog.register(tid, self.app.clone());

                let frames_per_buffer = self.sink.frames_per_buffer as usize;
                let channels          = self.sink.channels as usize;
                let mut buffer = vec![0.0f32; frames_per_buffer * channels];

                while !self.stop_flag.load(Ordering::Relaxed) {
                    // Zero the buffer
                    for s in buffer.iter_mut() { *s = 0.0; }

                    // Invoke the app's on-audio-render WIT callback with epoch
                    // interruption disabled inside the callback. The callback
                    // runs inside the app's audio sub-Store (separate from main).
                    let render_start = thread_cputime_ns();
                    let result = self.invoke_render_callback(&mut buffer, frames_per_buffer);
                    let render_ns = thread_cputime_ns() - render_start;

                    if render_ns > 2_000_000 {
                        // Overran the runtime budget. Log; kernel will throttle us.
                        self.watchdog.record_overrun(self.app.clone(), render_ns);
                    }

                    match result {
                        Ok(()) => {
                            if self.sink.submit(&buffer).is_err() {
                                tracing::warn!(app = %self.app, "audio sink full");
                            }
                        }
                        Err(_) => {
                            // Render error: zero-fill, log, continue.
                            self.sink.submit_silence();
                        }
                    }

                    // SCHED_DEADLINE wakes us at the next period boundary
                    // implicitly via the kernel scheduler.
                }

                self.watchdog.unregister(tid);
            })
            .expect("spawn audio render thread")
    }
}
```

### 4.5 WIT surface for audio render

```wit
// wit/vyoma-audio.wit
package vyoma:audio@0.1.0;

interface audio {
    record stream-config {
        sample-rate: u32,         // 44100, 48000, 96000
        channels: u8,             // 1 = mono, 2 = stereo
        frames-per-buffer: u32,   // 256 typical
        format: sample-format,
    }

    enum sample-format { f32, s16, s24 }

    enum audio-error {
        no-capability,
        sink-unavailable,
        config-invalid,
        deadline-unavailable,
        already-running,
    }

    /// Start an audio output stream. The supervisor will begin invoking
    /// `audio-callbacks/on-audio-render` at the configured rate.
    start: func(cfg: stream-config) -> result<_, audio-error>;
    stop:  func();
}

interface audio-callbacks {
    use audio.{stream-config};

    /// Called from a SCHED_DEADLINE thread every `period` ms. The callback
    /// must fill `frames_per_buffer` * `channels` samples into the host's
    /// shared SHM buffer at `buffer-handle` (R3) and return.
    ///
    /// Hard budget: 2 ms. Overruns are not SIGKILL'd directly (the kernel
    /// will throttle via SCHED_DEADLINE), but RLIMIT_RTTIME=5ms acts as a
    /// final safety net.
    on-audio-render: func(
        buffer-handle: u64,
        frames-to-write: u32,
        timestamp-ms: u64
    ) -> result<u32, audio-error>;
}
```

### 4.6 Epoch interruption disabled inside render

Crucial detail: when the audio render thread invokes the WIT callback, the supervisor sets
the audio-Store's epoch deadline to `u64::MAX` (effectively infinite) so the global
1 kHz ticker does not interrupt the callback. The `SCHED_DEADLINE` runtime budget *is*
the bound. Once the callback returns, the deadline is restored.

```rust
// supervisor/src/scheduler/audio.rs (continued)

impl AudioRenderThread {
    fn invoke_render_callback(&self, buffer: &mut [f32], frames: usize) -> Result<(), AudioError> {
        // 1. Lock the audio sub-Store. Sub-Store is a lightweight Store created
        //    at audio:start time that holds only the audio interface and a SHM
        //    handle for the buffer.
        let mut store = self.sub_store.lock();
        // 2. Disable epoch interruption.
        store.set_epoch_deadline(u64::MAX);
        // 3. Publish the buffer to the SHM region (zero-copy if SurfaceAlias-style).
        let handle = self.shm.publish(buffer);
        // 4. Invoke on-audio-render.
        let result = self.callbacks.on_audio_render
            .call(&mut *store, (handle.id, frames as u32, monotonic_ms()))?;
        // 5. Restore epoch deadline.
        store.set_epoch_deadline(self.normal_deadline);
        match result {
            Ok(_) => Ok(()),
            Err(_) => Err(AudioError::ExecutionFailed),
        }
    }
}
```

### 4.7 The audio watchdog

A separate thread pinned to a different CPU core (`taskset -c $((num_cpus-1))` via
`pthread_setaffinity_np`) monitors `/proc/<tid>/sched` for the render thread:

```rust
// supervisor/src/scheduler/audio_watchdog.rs

pub struct AudioWatchdog {
    threads: DashMap<libc::pid_t, AudioWatchdogEntry>,
}

pub struct AudioWatchdogEntry {
    pub app: AppId,
    pub last_observed_runtime_ns: AtomicU64,
    pub stuck_since_ms: AtomicU64,
}

impl AudioWatchdog {
    pub fn run(self: Arc<Self>) {
        loop {
            std::thread::sleep(Duration::from_millis(100));
            let now_ms = monotonic_ms();
            for entry in self.threads.iter() {
                let tid = *entry.key();
                let runtime_ns = read_sched_runtime(tid);
                let prev = entry.value().last_observed_runtime_ns
                    .swap(runtime_ns, Ordering::Relaxed);
                if runtime_ns == prev {
                    // Thread hasn't run in 100 ms (deadline overrun & throttled).
                    let stuck_at = entry.value().stuck_since_ms
                        .compare_exchange(0, now_ms, AcqRel, Acquire)
                        .unwrap_or_else(|prev| prev);
                    if now_ms.saturating_sub(stuck_at) > 500 {
                        tracing::error!(app = %entry.value().app,
                            "audio render thread stuck > 500 ms; killing");
                        unsafe { libc::kill(tid, libc::SIGKILL); }
                    }
                } else {
                    entry.value().stuck_since_ms.store(0, Ordering::Relaxed);
                }
            }
        }
    }
}
```

### 4.8 Capability gating

Only apps with `[capabilities.audio_realtime]` in `vyoma.toml` get a render thread. The
capability is **distinct** from `audio = true` (which permits non-realtime audio output via
buffered submission). Requesting `audio_realtime` requires a signed manifest (Round 1's
ed25519 signature verification) — adversarial unsigned apps cannot grab `SCHED_DEADLINE`.

```toml
[capabilities]
audio          = true   # buffered playback (no RT)
audio_realtime = true   # SCHED_DEADLINE render callback; requires signed manifest
```

---

## 5. App Nap — Fixed (Resolves C4)

### 5.1 The mistake

The Architect's App Nap depended on the scheduler being a privileged observer of audio,
network, IPC, and FS state. The Critic correctly noted this couples every subsystem to
the scheduler and creates state-explosion bugs.

### 5.2 The fix: ActivityAssertion tokens

Each subsystem (IPC, audio, network, FS coord, dispatch) calls
`ActivityRegistry::assert(app, reason)` when it starts work on behalf of that app, and
drops the returned RAII guard when done. The scheduler queries assertion *counts*, not
subsystem state.

```rust
// supervisor/src/scheduler/activity.rs

use std::sync::Arc;
use parking_lot::Mutex;
use dashmap::DashMap;

#[derive(Debug, Clone)]
pub enum ActivityReason {
    /// Inbound IPC call from another app awaiting our reply.
    IpcInboundCall { from: AppId, envelope_id: u64 },
    /// Outbound IPC call we're awaiting a reply for.
    IpcOutboundCall { to: AppId, envelope_id: u64 },
    /// Network operation in flight (socket read/write/connect).
    NetIo { kind: NetIoKind, socket_id: u64 },
    /// Filesystem coordination lock held.
    FsCoordLock { path: PathBuf, mode: CoordMode },
    /// Audio buffer submitted in last 1 s.
    AudioRecent,
    /// Dispatch host-job in flight.
    DispatchHost { kind: &'static str, id: u64 },
    /// Dispatch app-job in flight.
    DispatchApp { work_id: u32, id: u64 },
    /// Explicit user keep-awake (via vyoma:cpu/keep-awake).
    UserKeepAwake { reason: String, until_ms: u64 },
    /// Timer due within 1 s.
    TimerImminent { timer_id: u64, fires_at_ms: u64 },
    /// Notification displayed in last 30 s.
    NotificationRecent,
}

#[derive(Debug, Clone, Copy)]
pub enum NetIoKind { Read, Write, Connect, Accept }
#[derive(Debug, Clone, Copy)]
pub enum CoordMode { Read, Write }

pub struct ActivityRegistry {
    inner: DashMap<AppId, AppActivitySet>,
    next_id: AtomicU64,
}

pub struct AppActivitySet {
    pub active: Mutex<HashMap<u64, ActivityReason>>,
    pub count: AtomicU32,
    pub last_release_ms: AtomicU64,
}

/// RAII guard that decrements the assertion count on Drop.
pub struct ActivityAssertionGuard {
    registry: Arc<ActivityRegistry>,
    app: AppId,
    id: u64,
}

impl Drop for ActivityAssertionGuard {
    fn drop(&mut self) {
        if let Some(set) = self.registry.inner.get(&self.app) {
            set.active.lock().remove(&self.id);
            set.count.fetch_sub(1, Ordering::AcqRel);
            set.last_release_ms.store(monotonic_ms(), Ordering::Relaxed);
        }
    }
}

impl ActivityRegistry {
    pub fn assert(self: &Arc<Self>, app: AppId, reason: ActivityReason)
        -> ActivityAssertionGuard
    {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let set = self.inner.entry(app.clone()).or_insert_with(|| AppActivitySet {
            active: Mutex::new(HashMap::new()),
            count: AtomicU32::new(0),
            last_release_ms: AtomicU64::new(0),
        });
        set.active.lock().insert(id, reason);
        set.count.fetch_add(1, Ordering::AcqRel);
        ActivityAssertionGuard {
            registry: Arc::clone(self),
            app, id,
        }
    }

    pub fn count(&self, app: &AppId) -> u32 {
        self.inner.get(app)
            .map(|s| s.count.load(Ordering::Acquire))
            .unwrap_or(0)
    }

    pub fn list(&self, app: &AppId) -> Vec<ActivityReason> {
        self.inner.get(app)
            .map(|s| s.active.lock().values().cloned().collect())
            .unwrap_or_default()
    }
}
```

### 5.3 Subsystem hookups

Each subsystem holds an `Arc<ActivityRegistry>` and asserts at obvious lifecycle points:

```rust
// R3 IPC router (extended)
impl IpcRouter {
    pub fn deliver_call(&self, env: IpcEnvelope) {
        let target = env.target.bundle.clone();
        let assertion = self.activity.assert(target.clone(),
            ActivityReason::IpcInboundCall {
                from: env.source.bundle.clone(),
                envelope_id: env.id,
            });
        // Store assertion alongside the pending reply; dropped on reply or timeout.
        self.pending.insert(env.id, PendingReply {
            envelope: env,
            assertion: Some(assertion),
            // ...
        });
    }
}

// Network stack hookup (Round 11 forward reference)
impl NetworkStack {
    pub fn begin_socket_read(&self, app: &AppId, socket_id: u64) -> ActivityAssertionGuard {
        self.activity.assert(app.clone(),
            ActivityReason::NetIo { kind: NetIoKind::Read, socket_id })
    }
}

// FS coordination (R4)
impl CoordinationService {
    pub fn acquire(&self, app: AppId, path: PathBuf, mode: CoordMode) -> CoordLock {
        let assertion = self.activity.assert(app.clone(),
            ActivityReason::FsCoordLock { path: path.clone(), mode });
        CoordLock { path, mode, assertion, /* ... */ }
    }
}
```

### 5.4 NapDetector state machine

```rust
// supervisor/src/scheduler/app_nap.rs

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NapState {
    /// Normal execution. nap_divider = 1.
    Awake,
    /// Light throttle. nap_divider = 8 (~8 ms effective epoch tick).
    LightNap,
    /// Heavy throttle. nap_divider = 60 (~60 ms tick).
    DeepNap,
    /// Instance dropped, StateBlob persisted. Wake = re-instantiate.
    Parked,
}

pub struct NapDetector {
    qos:      Arc<QosDeriver>,
    epoch:    Arc<EpochController>,
    activity: Arc<ActivityRegistry>,
    timers:   Arc<TimerWheel>,
    state:    DashMap<AppId, NapState>,
    last_focus_ms: DashMap<AppId, u64>,
    lifecycle: Arc<LifecycleActor>,
}

impl NapDetector {
    /// Called every 1 s for every non-focused app.
    pub fn reconsider(&self, app: &AppId) {
        let now_ms = monotonic_ms();
        let assertion_count = self.activity.count(app);

        // Hard wakers: any non-zero assertion or explicit keep-awake → Awake.
        if assertion_count > 0 {
            self.transition(app, NapState::Awake);
            return;
        }
        if self.qos.focus().is_focused(app) {
            self.transition(app, NapState::Awake);
            return;
        }
        if self.timers.has_due_within(app, Duration::from_secs(1)) {
            self.transition(app, NapState::Awake);
            return;
        }

        // Decay based on time-since-focus.
        let since_focus = now_ms.saturating_sub(
            self.last_focus_ms.get(app).map(|r| *r).unwrap_or(now_ms)
        );

        let new_state = match since_focus {
            0..=10_000               => NapState::Awake,
            10_001..=60_000          => NapState::LightNap,
            60_001..=600_000         => NapState::DeepNap,
            _ if self.is_parkable(app) => NapState::Parked,
            _                         => NapState::DeepNap,
        };
        self.transition(app, new_state);
    }

    fn transition(&self, app: &AppId, new_state: NapState) {
        let prev = self.state.insert(app.clone(), new_state).unwrap_or(NapState::Awake);
        if prev == new_state { return; }

        match new_state {
            NapState::Awake => {
                self.epoch.set_nap_divider(app, 1);
                if prev == NapState::Parked {
                    // Re-instantiate from StateBlob.
                    self.lifecycle.resume_from_park(app.clone());
                }
            }
            NapState::LightNap => {
                self.epoch.set_nap_divider(app, 8);
            }
            NapState::DeepNap => {
                self.epoch.set_nap_divider(app, 60);
            }
            NapState::Parked => {
                // Fire on-suspend, drop the Store.
                self.lifecycle.park(app.clone());
            }
        }

        tracing::info!(?app, ?prev, ?new_state, "nap state transition");
    }

    fn is_parkable(&self, app: &AppId) -> bool {
        self.lifecycle.declared_suspendable(app)
            && self.activity.count(app) == 0
    }
}
```

### 5.5 Wake triggers (now assertion-driven)

When any subsystem asserts activity on a napping/parked app, the NapDetector is notified:

```rust
impl ActivityRegistry {
    pub fn assert(self: &Arc<Self>, app: AppId, reason: ActivityReason)
        -> ActivityAssertionGuard
    {
        // ... (same as §5.2) ...

        // Notify nap detector immediately. Re-instantiation is async.
        if let Some(nap) = &*self.nap_detector.read() {
            nap.wake_if_napping(&app);
        }

        guard
    }
}

impl NapDetector {
    pub fn wake_if_napping(&self, app: &AppId) {
        let prev = self.state.get(app).map(|r| *r).unwrap_or(NapState::Awake);
        if prev != NapState::Awake {
            self.transition(app, NapState::Awake);
        }
    }
}
```

This is the entire wake-up mechanism. If you assert, the app wakes. If you don't,
it stays napped. The Critic's C4 is fully resolved.

### 5.6 Park semantics (no SIGSTOP)

The Critic correctly noted that `SIGSTOP` on a thread within the supervisor process stops
the entire supervisor. We do not use `SIGSTOP` for app suspension. Instead:

1. The scheduler sets `nap_divider = NAP_PARK_SENTINEL` (= `u8::MAX`).
2. The epoch ticker, on the next tick for this app, calls `engine.increment_epoch()`
   and the epoch callback observes the sentinel, then fires `on-suspend` via R1's WIT
   callback infrastructure.
3. `on-suspend` returns a `StateBlob` which the supervisor persists to
   `/data/state/<bundle>.bin`.
4. The supervisor drops the `Store`. The thread exits cleanly.
5. The `AppHandle` in R1's sharded `AppTable` transitions to
   `AppStatus::Parked { blob_path }`.

On wake:

1. The supervisor spawns a fresh thread, creates a new `Store`, instantiates the WASM
   component, calls `on-resume(blob)`, and the app resumes from its persisted state.
2. The thread joins the regular epoch ticker loop.

This costs ~50 ms typical re-instantiation latency. Trade-off accepted: we recover full
memory cost (Store dropped → linear memory released), at the cost of a half-second
"wake animation" delay. Document this in the user guide as "background apps may take up
to a second to resume."

---

## 6. CPU Throttling — Fixed (Resolves C5)

### 6.1 The mistake

Reducing epoch tick frequency does *not* reduce CPU consumption. It reduces preemption
density (how often the supervisor inspects state mid-execution). The Critic's C5 is
elementary but the Architect got it wrong.

### 6.2 The fix: cgroup v2 cpu.max

Each app's thread is in a per-app cgroup at boot:

```
/sys/fs/cgroup/vyoma/
├── <bundle>-<iid>/
│   ├── cgroup.procs       ← tid written here at thread spawn
│   ├── cpu.max            ← "quota period" pair, written per QoS class
│   ├── cpu.weight         ← cgroup v2 weight (1–10000)
│   └── cpu.stat           ← read-only: usage_usec, nr_periods, nr_throttled, ...
```

QoS class → `cpu.max` mapping table:

| QoS class | `cpu.max` | Effective ceiling | Use |
|-----------|-----------|-------------------|-----|
| UserInteractive | `max 1000000` | unbounded (1 s period) | Focused UI, never throttle |
| UserInitiated | `max 1000000` | unbounded | Recently focused, never throttle |
| Default | `800000 1000000` | 80 % of 1 core | New apps, soft cap |
| Utility | `500000 1000000` | 50 % of 1 core | Visible progress / net I/O |
| Background | `100000 1000000` | 10 % of 1 core | Unfocused > 60 s |
| Maintenance | `50000 1000000` | 5 % of 1 core | GC, log rotation |

### 6.3 cgroup setup

```rust
// supervisor/src/scheduler/cgroup.rs

use std::path::{Path, PathBuf};
use std::io;

pub struct CgroupManager {
    root: PathBuf, // typically /sys/fs/cgroup/vyoma
}

impl CgroupManager {
    /// At BootPhase::SchedulerReady, ensure /sys/fs/cgroup/vyoma exists with
    /// cpu controller enabled.
    pub fn init(root: PathBuf) -> io::Result<Self> {
        std::fs::create_dir_all(&root)?;
        // Enable cpu controller in the subtree.
        let subtree_control = root.join("cgroup.subtree_control");
        std::fs::write(&subtree_control, b"+cpu")?;
        Ok(Self { root })
    }

    pub fn create_app_cgroup(&self, app: &AppHandle) -> io::Result<PathBuf> {
        let p = self.root.join(format!("{}-{}", app.bundle, app.iid));
        std::fs::create_dir_all(&p)?;
        Ok(p)
    }

    pub fn join(&self, cgroup_dir: &Path, tid: libc::pid_t) -> io::Result<()> {
        std::fs::write(cgroup_dir.join("cgroup.procs"), tid.to_string().as_bytes())
    }

    pub fn apply_qos(&self, cgroup_dir: &Path, qos: QosClass) -> io::Result<()> {
        let p = qos.policy();
        let cpu_max = if p.cpu_max_quota_us < 0 {
            format!("max {}", p.cpu_max_period_us)
        } else {
            format!("{} {}", p.cpu_max_quota_us, p.cpu_max_period_us)
        };
        std::fs::write(cgroup_dir.join("cpu.max"), cpu_max.as_bytes())?;
        std::fs::write(cgroup_dir.join("cpu.weight"), p.cpu_weight.to_string().as_bytes())?;
        Ok(())
    }

    pub fn read_stat(&self, cgroup_dir: &Path) -> io::Result<CgroupCpuStat> {
        let s = std::fs::read_to_string(cgroup_dir.join("cpu.stat"))?;
        let mut stat = CgroupCpuStat::default();
        for line in s.lines() {
            let mut it = line.split_whitespace();
            match (it.next(), it.next()) {
                (Some("usage_usec"), Some(v)) => stat.usage_usec = v.parse().unwrap_or(0),
                (Some("user_usec"),  Some(v)) => stat.user_usec  = v.parse().unwrap_or(0),
                (Some("system_usec"),Some(v)) => stat.system_usec= v.parse().unwrap_or(0),
                (Some("nr_periods"), Some(v)) => stat.nr_periods = v.parse().unwrap_or(0),
                (Some("nr_throttled"),Some(v)) => stat.nr_throttled = v.parse().unwrap_or(0),
                (Some("throttled_usec"),Some(v)) => stat.throttled_usec = v.parse().unwrap_or(0),
                _ => {}
            }
        }
        Ok(stat)
    }

    pub fn destroy(&self, cgroup_dir: &Path) -> io::Result<()> {
        std::fs::remove_dir(cgroup_dir)
    }
}

#[derive(Debug, Default, Clone)]
pub struct CgroupCpuStat {
    pub usage_usec: u64,
    pub user_usec: u64,
    pub system_usec: u64,
    pub nr_periods: u64,
    pub nr_throttled: u64,
    pub throttled_usec: u64,
}
```

### 6.4 Throttling actions

When `CpuAccount::ewma_cpu_pct_x100 > qos.cpu_cap_x100`, the reconciler tightens the
cgroup ceiling:

```rust
// supervisor/src/scheduler/epoch_controller.rs (continued)

impl EpochController {
    fn tighten_cgroup(&self, app: AppId, qos: QosClass) {
        let dir = match self.cgroups.cgroup_dir(&app) { Some(d) => d, None => return };
        let p = qos.policy();
        // Halve the quota (relative to QoS baseline).
        let new_quota = if p.cpu_max_quota_us < 0 {
            (500_000_i64).min(p.cpu_max_period_us as i64 / 2)
        } else {
            (p.cpu_max_quota_us / 2).max(10_000)
        };
        let cpu_max = format!("{} {}", new_quota, p.cpu_max_period_us);
        let _ = std::fs::write(dir.join("cpu.max"), cpu_max.as_bytes());
        tracing::warn!(app = %app, ?qos, new_quota_us = new_quota,
            "tightened cgroup cpu.max");
    }
}
```

### 6.5 Reading actual CPU consumption

Beyond per-thread `CLOCK_THREAD_CPUTIME_ID`, the cgroup `cpu.stat` gives us authoritative
aggregate CPU and throttling counts. The `UsageSampler` (Section 10) reads both per-thread
and per-cgroup numbers and reports the maximum.

---

## 7. Priority Inheritance through IPC — Fixed (Resolves C6)

### 7.1 The mistake

The Architect's QoS classes apply to apps but R3's IPC made request/reply the primary
inter-app pattern. A UserInteractive caller blocked on a Background callee → callee
runs at Background priority → caller's UI stalls. Classic Mars Pathfinder bug.

### 7.2 The fix: effective_qos field in IpcEnvelope

R3's `IpcEnvelope` gains a new field:

```rust
// supervisor/src/ipc/envelope.rs (modified)

pub struct IpcEnvelope {
    pub id: u64,
    pub source: IpcSourceId,
    pub target: IpcAddr,
    pub schema: SchemaId,
    pub schema_version: u16,
    pub body: InlineBody,
    pub flags: IpcFlags,
    pub priority: Priority,
    pub trace_id: Option<u128>,
    pub span_id: Option<u64>,
    pub timeout: Option<Duration>,
    pub deadline_ms: Option<u64>,
    /// NEW (R5): The effective QoS class to apply to the receiver while
    /// processing this envelope. Set by the router on Call envelopes when
    /// the sender has higher QoS than the receiver.
    pub effective_qos: Option<QosClass>,
}
```

### 7.3 Promotion on call delivery

```rust
// supervisor/src/ipc/dispatch.rs (R5-extended)

impl IpcRouter {
    fn promote_for_call(&self, env: &mut IpcEnvelope) -> Option<QosPromotionGuard> {
        if !env.is_call() {
            return None;
        }
        let sender_qos = self.scheduler.qos.derive(&env.source.bundle);
        let receiver_qos = self.scheduler.qos.derive(&env.target.bundle);
        if sender_qos >= receiver_qos {
            // Sender QoS is *lower* or *equal* (lower discriminant = higher prio).
            return None;
        }
        // Sender > receiver in priority. Promote receiver.
        env.effective_qos = Some(sender_qos);
        self.scheduler.qos.promote(
            env.target.bundle.clone(),
            sender_qos,
            PromotionReason::IpcInheritance {
                from: env.source.bundle.clone(),
                envelope_id: env.id,
            },
        )
    }
}
```

### 7.4 Promotion mechanics

```rust
// supervisor/src/scheduler/qos.rs (R5-extended)

pub struct QosPromotionGuard {
    deriver: Arc<QosDeriver>,
    app: AppId,
    promoted_to: QosClass,
    promotion_id: u64,
}

impl Drop for QosPromotionGuard {
    fn drop(&mut self) {
        self.deriver.release_promotion(&self.app, self.promotion_id);
    }
}

impl QosDeriver {
    pub fn promote(&self, app: AppId, target_qos: QosClass, reason: PromotionReason)
        -> Option<QosPromotionGuard>
    {
        let id = self.next_promotion_id.fetch_add(1, Ordering::Relaxed);
        let entry = self.promotions.entry(app.clone()).or_default();
        let prev_effective = entry.effective_qos();
        entry.add(id, target_qos, reason);
        let new_effective = entry.effective_qos();
        if new_effective != prev_effective {
            self.apply_to_threads(&app, new_effective);
        }
        Some(QosPromotionGuard {
            deriver: Arc::clone(self),
            app,
            promoted_to: target_qos,
            promotion_id: id,
        })
    }

    pub fn release_promotion(&self, app: &AppId, id: u64) {
        if let Some(entry) = self.promotions.get_mut(app) {
            let prev_effective = entry.effective_qos();
            entry.remove(id);
            let new_effective = entry.effective_qos();
            if new_effective != prev_effective {
                self.apply_to_threads(app, new_effective);
            }
        }
    }
}

pub struct PromotionSet {
    by_id: HashMap<u64, (QosClass, PromotionReason)>,
}

impl PromotionSet {
    /// Effective QoS = min(declared, all active promotions).
    /// Lower discriminant = higher priority, so min() picks the highest priority.
    pub fn effective_qos(&self) -> QosClass {
        self.by_id.values().map(|(q, _)| *q).min().unwrap_or(QosClass::Default)
    }
}
```

### 7.5 Cycle detection via R3's WaitForGraph

R3's `WaitForGraph` already detects A → B → C → A cycles. The promotion path uses the
same graph: when the router promotes B for a call from A, the graph edge `A → B` is
recorded. If a transitive cycle is detected, the call is rejected with
`IpcError::WouldDeadlock` and the promotion is not applied.

### 7.6 Demotion triggers

The promotion guard is dropped (and the receiver demoted) on any of:

- Reply delivered to sender (normal path).
- Sender crash (R3's `on_instance_gone` drains pending replies and drops their
  promotion guards).
- Receiver crash (the call fails with `IpcError::ReceiverGone`).
- Timeout fired (R3's 100 ms tick deadline sweep).
- Sender explicitly cancels via `cancel-call`.

### 7.7 Transitive inheritance

When promoted B itself blocks on a call to C, the router promotes C to B's *effective*
QoS (which is now the inherited UserInteractive). This produces correct transitive
inheritance through any depth of chain. The chain length is bounded by the per-app
`max_inflight = 8` from R3 multiplied by the total live-app count; in practice well
under 100.

---

## 8. Focus-Aware QoS Derivation

### 8.1 Inputs to derivation

The `QosDeriver` consumes:

1. **`focus.is_focused(app)`** from R1's `FocusState`.
2. **`activity.last_*_ms`** snapshot per app.
3. **`assertion.count(app)`** from `ActivityRegistry` (§5).
4. **`caps.declared(app)`** from R1's capability registry.
5. **`runtime.user_pinned_floor(app)`** — user-set "Keep running in background" pin.
6. **`promotions.effective_qos(app)`** — IPC inheritance.

### 8.2 Derivation function

```rust
// supervisor/src/scheduler/qos.rs (continued)

impl QosDeriver {
    pub fn derive(&self, app: &AppId) -> QosClass {
        let declared_floor = self.declared_floor(app);
        let derived = self.derive_unfloored(app);
        let inherited = self.promotions.get(app)
            .map(|p| p.effective_qos())
            .unwrap_or(QosClass::Maintenance);
        // Take the highest priority (lowest discriminant) of: derived, inherited, floor.
        [derived, inherited, declared_floor].iter().copied().min().unwrap()
    }

    fn derive_unfloored(&self, app: &AppId) -> QosClass {
        let caps = self.caps.get(app);
        let act = self.activity.snapshot(app);
        let now = monotonic_ms();

        // 1. Recently rendering audio → UserInteractive (audio render thread is separate;
        //    UI thread still benefits from low-latency for audio app's control panel).
        if caps.audio_realtime && now.saturating_sub(act.last_audio_frame_ms) < 1_000 {
            return QosClass::UserInteractive;
        }

        // 2. Focused window → UserInteractive.
        if self.focus.is_focused(app) {
            return QosClass::UserInteractive;
        }

        // 3. Recently focused (≤ 5 s ago) → UserInitiated.
        let since_focus = now.saturating_sub(act.last_focus_ms);
        if since_focus < 5_000 {
            return QosClass::UserInitiated;
        }

        // 4. Recently took user input (mouse-in-window event) → UserInitiated.
        if now.saturating_sub(act.last_user_input_ms) < 2_000 {
            return QosClass::UserInitiated;
        }

        // 5. Progress visible OR net I/O recent → Utility floor.
        if act.progress_visible
            || now.saturating_sub(act.last_net_io_ms) < 10_000
        {
            return QosClass::Utility;
        }

        // 6. Unfocused > 60 s with no work → Background.
        if since_focus > 60_000 {
            return QosClass::Background;
        }

        // 7. Default.
        QosClass::Utility
    }

    fn declared_floor(&self, app: &AppId) -> QosClass {
        // User-pinned floor from Activity Monitor "Keep running" toggle.
        self.runtime.get(app)
            .and_then(|s| s.user_pinned_floor)
            .unwrap_or(QosClass::Maintenance)
    }
}
```

### 8.3 Decay schedule

A periodic 1 Hz reconciler walks every non-focused app:

```rust
// supervisor/src/scheduler/decay.rs

pub async fn decay_loop(deriver: Arc<QosDeriver>, observer: Arc<FocusObserver>) {
    let mut interval = tokio::time::interval(Duration::from_secs(1));
    loop {
        interval.tick().await;
        for app in deriver.all_apps() {
            if deriver.focus.is_focused(&app) { continue; }
            let new_qos = deriver.derive(&app);
            observer.apply_qos_idempotent(&app, new_qos);
        }
    }
}
```

`apply_qos_idempotent` compares with the current applied QoS and only triggers
`apply_qos(tid, qos)` (which involves syscalls) when the class changes.

### 8.4 Boot warmup window

For the first 5 s after `BootPhase::AppsLaunched`, the deriver returns `max(derived,
UserInitiated)` to allow all apps to JIT-compile + execute init code at higher priority.
This protects the < 5 s boot target.

```rust
impl QosDeriver {
    pub fn derive(&self, app: &AppId) -> QosClass {
        let derived = self.derive_unfloored(app);
        if self.boot_warmup_active() {
            return derived.min(QosClass::UserInitiated);
        }
        // ... usual logic ...
    }

    fn boot_warmup_active(&self) -> bool {
        monotonic_ms().saturating_sub(self.boot_apps_launched_ms.load(Ordering::Relaxed)) < 5_000
    }
}
```

### 8.5 Capability floors

Some capabilities imply a QoS floor regardless of focus/decay:

| Capability | Floor when active |
|------------|-------------------|
| `audio_realtime` (recent render call) | UserInteractive |
| `compositor` (R1 chrome-class) | UserInitiated |
| `notification_daemon` (notifications recent) | Utility |
| `network` with active socket I/O | Utility |
| `user_pinned_floor` (user toggle) | as declared |

---

## 9. Timer Coalescing

### 9.1 Hierarchical timing wheel

We use a 5-level hierarchical wheel similar to `tokio::time::driver`:

| Level | Slot duration | Total range | Slots |
|-------|---------------|-------------|-------|
| 0 | 1 ms | 64 ms | 64 |
| 1 | 64 ms | 4 s | 64 |
| 2 | 4 s | 4 min | 64 |
| 3 | 4 min | 4 h | 64 |
| 4 | 4 h | 11 days | 64 |

```rust
// supervisor/src/scheduler/timers.rs

pub struct TimerWheel {
    levels: [Level; 5],
    epoch_ms: AtomicU64,
    next_id: AtomicU64,
    ipc: Arc<IpcRouter>,
    dispatch: Arc<DispatchPool>,
    activity: Arc<ActivityRegistry>,
}

pub struct Level {
    slot_ms: u64,
    slots: Vec<Mutex<SlotBucket>>,
    current_slot: AtomicUsize,
}

pub struct SlotBucket {
    timers: SmallVec<[TimerEntry; 4]>,
}

#[derive(Debug, Clone)]
pub struct TimerEntry {
    pub id: u64,
    pub owner: AppId,
    pub deadline_ms: u64,
    pub leeway_ms: u32,
    pub interval_ms: u32, // 0 = one-shot
    pub kind: TimerKind,
}

#[derive(Debug, Clone)]
pub enum TimerKind {
    Tick { user_data: u64 },
    Dispatch { job: HostJob, qos_hint: Option<QosClass> },
    Internal { task: InternalTask },
}
```

### 9.2 QoS-tiered leeway

The Critic's S3 is accepted: a flat 10 ms coalesce causes wakeup storms. We adopt
QoS-tiered leeway with per-app jitter:

| QoS | Base leeway | Per-app jitter | Effective coalescing |
|-----|-------------|----------------|----------------------|
| UserInteractive | 0 ms | 0 | exact firing |
| UserInitiated | 1 ms | 0–1 ms | almost exact |
| Default | 10 ms | hash(app) % 10 ms | clustered every 10 ms |
| Utility | 100 ms | hash(app) % 100 ms | clustered every 100 ms |
| Background | 1 s | hash(app) % 1 s | clustered every 1 s |
| Maintenance | 10 s | hash(app) % 10 s | clustered every 10 s |

Apps may also explicitly request a leeway via the WIT API:

```wit
// wit/vyoma-time.wit
interface time {
    type timer-id = u64;

    /// Set a one-shot timer. `at-ms` is monotonic. `leeway-ms` is the
    /// app's preferred slop; the supervisor may extend it based on QoS
    /// (the effective leeway is max(requested, QoS-tier-leeway)).
    set-timer: func(
        at-ms: u64,
        leeway-ms: u32,
        user-data: u64
    ) -> result<timer-id, time-error>;

    set-recurring: func(
        at-ms: u64,
        interval-ms: u32,
        leeway-ms: u32,
        user-data: u64
    ) -> result<timer-id, time-error>;

    cancel-timer: func(id: timer-id) -> bool;

    monotonic-now: func() -> u64;
}
```

### 9.3 Insertion with QoS-derived leeway

```rust
impl TimerWheel {
    pub fn insert(&self, owner: AppId, deadline_ms: u64, requested_leeway_ms: u32,
                  kind: TimerKind) -> u64
    {
        let qos = self.qos.derive(&owner);
        let qos_leeway = qos.policy().timer_leeway_ms;
        let effective_leeway = requested_leeway_ms.max(qos_leeway);
        // Per-app jitter so we don't synchronize within tier.
        let app_hash = hash_app(&owner);
        let jitter = if effective_leeway > 0 {
            app_hash % (effective_leeway as u64)
        } else { 0 };
        let adjusted_deadline = deadline_ms.saturating_add(jitter);

        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let entry = TimerEntry {
            id, owner, deadline_ms: adjusted_deadline,
            leeway_ms: effective_leeway,
            interval_ms: 0, kind,
        };
        self.insert_entry(entry);
        id
    }
}
```

### 9.4 Wakeup budget

A global "wakeups/second" counter is maintained. When the budget (default 1000/s for
desktop, 100/s for mobile) is exceeded, the wheel increases leeway aggressiveness by
one tier across all timers:

```rust
impl TimerWheel {
    pub fn enforce_budget(&self) {
        let recent_wakeups = self.wakeup_counter.swap(0, Ordering::Relaxed);
        if recent_wakeups > self.budget {
            self.leeway_pressure.fetch_add(1, Ordering::Relaxed);
            tracing::warn!(recent_wakeups, budget = self.budget,
                "wakeup budget exceeded; tightening leeway");
        } else {
            self.leeway_pressure.store(0, Ordering::Relaxed);
        }
    }
}
```

### 9.5 Internal supervisor timers

Internal tasks (jetsam scan, log rotation, cgroup stat sampling) use `TimerKind::Internal`
to run on the same wheel, ensuring the supervisor doesn't run its own competing tickers.

---

## 10. CPU Usage Reporting & Activity Monitor

### 10.1 UsageSnapshot

```rust
// supervisor/src/scheduler/usage.rs

#[derive(Debug, Clone, Serialize)]
pub struct UsageSnapshot {
    pub app: AppId,
    pub qos: QosClass,
    pub effective_qos: QosClass, // includes inheritance
    pub nap_state: NapState,

    // From per-thread CLOCK_THREAD_CPUTIME_ID
    pub cpu_pct_1s_x100: u32,
    pub cpu_pct_60s_x100: u32,
    pub cpu_ns_lifetime: u64,

    // From cgroup cpu.stat
    pub cgroup_usage_usec: u64,
    pub cgroup_throttled_usec: u64,
    pub cgroup_nr_throttled: u64,

    // From epoch counters (secondary)
    pub epochs_consumed_1s: u32,
    pub epochs_consumed_60s: u32,
    pub epochs_lifetime: u64,

    // From R2 memory subsystem
    pub rss_bytes: u64,
    pub anon_bytes: u64,
    pub jetsam_score: u32,
    pub memory_pressure_seen: PressureLevel,

    // From activity registry
    pub active_assertions: u32,
    pub assertion_summary: Vec<String>,

    // From dispatch
    pub dispatch_inflight_host: u32,
    pub dispatch_pending_host: u32,
    pub dispatch_inflight_app:  u32,
    pub dispatch_pending_app:   u32,

    // From timers
    pub active_timers: u32,
    pub wakeups_1s: u32,

    // From thermal
    pub thermal_level: ThermalLevel,
}
```

### 10.2 The sampler

```rust
// supervisor/src/scheduler/usage.rs (continued)

pub struct UsageSampler {
    apps: Arc<DashMap<AppId, AppEpochCtx>>,
    threads: Arc<ThreadRegistry>,
    cgroups: Arc<CgroupManager>,
    activity: Arc<ActivityRegistry>,
    dispatch: Arc<DispatchPool>,
    timers: Arc<TimerWheel>,
    snapshots: Arc<ArcSwap<HashMap<AppId, UsageSnapshot>>>,
}

impl UsageSampler {
    pub fn run(self: Arc<Self>) {
        let mut next = Instant::now();
        loop {
            next += Duration::from_secs(1);

            let mut new_snapshots = HashMap::new();
            for entry in self.apps.iter() {
                let app = entry.key().clone();
                let ctx = entry.value();
                let snap = self.sample_one(&app, ctx);
                new_snapshots.insert(app, snap);
            }
            self.snapshots.store(Arc::new(new_snapshots));

            spin_sleep::sleep_until(next);
        }
    }

    fn sample_one(&self, app: &AppId, ctx: &AppEpochCtx) -> UsageSnapshot {
        let qos = QosClass::from_u8(ctx.qos.load(Ordering::Relaxed));
        let eff_qos = self.qos_deriver.derive(app);
        let cgroup_stat = self.cgroups.cgroup_dir(app)
            .and_then(|d| self.cgroups.read_stat(&d).ok())
            .unwrap_or_default();
        let assertion_summary: Vec<String> = self.activity.list(app)
            .iter().map(|r| format!("{:?}", r)).collect();

        UsageSnapshot {
            app: app.clone(),
            qos,
            effective_qos: eff_qos,
            nap_state: self.nap_detector.state(app),
            cpu_pct_1s_x100: ctx.cpu.ewma_cpu_pct_x100.load(Ordering::Relaxed),
            cpu_pct_60s_x100: ctx.cpu.window_10s_ns.load(Ordering::Relaxed) as u32 / 100_000,
            cpu_ns_lifetime: ctx.cpu.lifetime_ns.load(Ordering::Relaxed),
            cgroup_usage_usec: cgroup_stat.usage_usec,
            cgroup_throttled_usec: cgroup_stat.throttled_usec,
            cgroup_nr_throttled: cgroup_stat.nr_throttled,
            epochs_consumed_1s: ctx.account.window_1s.load(Ordering::Relaxed),
            epochs_consumed_60s: ctx.account.window_10s.load(Ordering::Relaxed),
            epochs_lifetime: ctx.account.lifetime.load(Ordering::Relaxed),
            rss_bytes: self.memory.rss(app),
            anon_bytes: self.memory.anon(app),
            jetsam_score: self.memory.jetsam_score(app),
            memory_pressure_seen: self.memory.last_pressure(),
            active_assertions: self.activity.count(app),
            assertion_summary,
            dispatch_inflight_host: self.dispatch.inflight_host(app),
            dispatch_pending_host:  self.dispatch.pending_host(app),
            dispatch_inflight_app:  self.dispatch.inflight_app(app),
            dispatch_pending_app:   self.dispatch.pending_app(app),
            active_timers: self.timers.count_for(app),
            wakeups_1s: self.timers.wakeups_1s(app),
            thermal_level: self.thermal.level(),
        }
    }
}
```

### 10.3 Activity Monitor IPC

```wit
// wit/vyoma-cpu.wit (extended)
package vyoma:cpu@0.1.0;

interface activity-monitor {
    record app-usage {
        bundle:            string,
        instance-id:       u32,
        qos:               qos-class,
        effective-qos:     qos-class,
        nap-state:         nap-state,
        cpu-pct-1s:        f32,
        cpu-pct-60s:       f32,
        rss-mb:            u32,
        active-assertions: u32,
        dispatch-inflight: u32,
        timers-active:     u32,
        wakeups-per-sec:   u32,
        thermal-level:     thermal-level,
    }

    enum qos-class {
        user-interactive, user-initiated, default,
        utility, background, maintenance,
    }
    enum nap-state { awake, light-nap, deep-nap, parked }
    enum thermal-level { nominal, fair, serious, critical }

    list-usage: func() -> list<app-usage>;
    subscribe:  func() -> stream<list<app-usage>>;
}
```

Capability gating: only apps with `[capabilities.activity_monitor]` may bind. Default
distribution includes this only for the bundled `apps/activity-monitor/` and `apps/console/`.

### 10.4 Thermal throttling

```rust
// supervisor/src/scheduler/thermal.rs

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub enum ThermalLevel { Nominal, Fair, Serious, Critical }

pub struct ThermalMonitor {
    qos: Arc<QosDeriver>,
    epoch_ctl: Arc<EpochController>,
    cgroups: Arc<CgroupManager>,
    level: AtomicU8,
    zones: Vec<PathBuf>,
}

impl ThermalMonitor {
    pub fn run(self: Arc<Self>) {
        loop {
            let temp_c = self.peak_temperature_c();
            let new_level = match temp_c {
                t if t < 60 => ThermalLevel::Nominal,
                t if t < 75 => ThermalLevel::Fair,
                t if t < 88 => ThermalLevel::Serious,
                _           => ThermalLevel::Critical,
            };
            let prev = self.level.swap(new_level as u8, Ordering::Relaxed);
            if prev != new_level as u8 {
                tracing::info!(?new_level, temp_c, "thermal level changed");
                self.apply(new_level);
            }
            std::thread::sleep(Duration::from_secs(2));
        }
    }

    fn apply(&self, lvl: ThermalLevel) {
        match lvl {
            ThermalLevel::Nominal | ThermalLevel::Fair => {
                // Restore default cgroup ceilings for all apps.
                self.cgroups.restore_all_to_qos_default();
            }
            ThermalLevel::Serious => {
                // Halve cgroup ceilings for Background and Maintenance classes.
                self.cgroups.scale_quota_for_qos(QosClass::Background,  0.5);
                self.cgroups.scale_quota_for_qos(QosClass::Maintenance, 0.5);
            }
            ThermalLevel::Critical => {
                // Park all parkable apps; tightly cap Utility too.
                self.epoch_ctl.park_all_parkable_at(QosClass::Background);
                self.cgroups.scale_quota_for_qos(QosClass::Utility,    0.5);
                self.cgroups.scale_quota_for_qos(QosClass::Default,    0.7);
            }
        }
    }
}
```

### 10.5 Cooperative yielding API

```wit
// wit/vyoma-cpu.wit (extended)
interface cpu {
    use types.{qos-class};

    /// Voluntarily yield. Next epoch interrupt is suppressed for one tick.
    yield-now: func();

    /// Yield and request not to be re-entered before `at-least-ms` ms.
    yield-for: func(at-least-ms: u32);

    /// Request keep-awake; max 600 s. Returns RAII token. Reason shown in
    /// Activity Monitor.
    keep-awake: func(reason: string, duration-secs: u32) -> result<u64, cpu-error>;
    release-keep-awake: func(token: u64);

    /// Query current effective QoS class.
    current-qos: func() -> qos-class;
}
```

### 10.6 Per-CPU and totals

```rust
#[derive(Debug, Clone, Serialize)]
pub struct SystemUsage {
    pub host_cpu_pct: f32,
    pub host_user_pct: f32,
    pub host_system_pct: f32,
    pub host_idle_pct: f32,
    pub n_apps_running: u32,
    pub n_apps_napping: u32,
    pub n_apps_parked: u32,
    pub total_epochs_per_sec: u64,
    pub total_wakeups_per_sec: u32,
    pub thermal_zone: ThermalLevel,
    pub scheduler_degraded: bool,
}
```

Sampled from `/proc/stat`, the NapDetector state map, and the thermal monitor.

---

## 11. Implementation Files

All files ≤ 500 lines.

| File | LOC | Purpose |
|------|-----|---------|
| `supervisor/src/scheduler/mod.rs` | 220 | Public re-exports, `Scheduler` struct, `start()` wiring, dependency injection |
| `supervisor/src/scheduler/qos.rs` | 480 | `QosClass`, `QosPolicy`, `QosDeriver`, `FocusObserver`, derivation logic |
| `supervisor/src/scheduler/sysctl.rs` | 280 | `apply_qos`, `set_sched_deadline`, `set_rlimit_rttime` syscall wrappers |
| `supervisor/src/scheduler/probe.rs` | 180 | `SchedCapabilities`, capability detection, degraded-mode broadcast |
| `supervisor/src/scheduler/cgroup.rs` | 360 | `CgroupManager`, per-app cgroup lifecycle, `cpu.max` / `cpu.weight` writers |
| `supervisor/src/scheduler/cpu_account.rs` | 240 | `CpuAccount`, `CpuSampler`, `thread_cputime_ns()` |
| `supervisor/src/scheduler/epoch_controller.rs` | 460 | `EpochController`, 1 kHz ticker, nap divider, reconciler, demotion logic |
| `supervisor/src/scheduler/app_nap.rs` | 420 | `NapDetector`, state machine, park/wake hooks, `is_parkable` |
| `supervisor/src/scheduler/activity.rs` | 360 | `ActivityRegistry`, `ActivityAssertionGuard` RAII, `ActivityReason` variants |
| `supervisor/src/scheduler/dispatch.rs` | 490 | `DispatchPool`, host-job execution, priority queues, serial locks |
| `supervisor/src/scheduler/dispatch_app.rs` | 320 | `AppDispatchQueue`, OnDispatch path, IPC envelope construction |
| `supervisor/src/scheduler/timers.rs` | 460 | `TimerWheel`, hierarchical 5-level structure, QoS-tiered leeway, jitter |
| `supervisor/src/scheduler/audio.rs` | 440 | `AudioRenderThread`, `SCHED_DEADLINE` setup, `on-audio-render` invocation |
| `supervisor/src/scheduler/audio_watchdog.rs` | 240 | `AudioWatchdog` on dedicated CPU core, deadline overrun detection |
| `supervisor/src/scheduler/usage.rs` | 460 | `UsageSnapshot`, `UsageSampler`, `SystemUsage`, `/proc/stat` reader |
| `supervisor/src/scheduler/thermal.rs` | 260 | `ThermalMonitor`, zone discovery, level transitions, applied throttling |
| `supervisor/src/scheduler/decay.rs` | 140 | 1 Hz decay loop, `apply_qos_idempotent` |
| `supervisor/src/scheduler/promotion.rs` | 220 | `PromotionSet`, `QosPromotionGuard`, `promote` / `release_promotion` |
| `wit/vyoma-cpu.wit` | 200 | `qos-class`, `nap-state`, `cpu`, `time`, `activity-monitor` interfaces |
| `wit/vyoma-dispatch.wit` | 180 | `dispatch`, `dispatch-callbacks` interfaces (HostJob + OnDispatch) |
| `wit/vyoma-audio.wit` | 120 | `audio`, `audio-callbacks` (`on-audio-render`) interfaces |

**Total new:** ~6,560 LOC across 21 files; every file under the 500-line ceiling.

**Modified (additive):**

| File | Δ LOC | Change |
|------|-------|--------|
| `supervisor/src/main.rs` | +60 | Scheduler boot phase wiring, cgroup mount probe |
| `supervisor/src/boot.rs` | +50 | `BootPhase::CgroupReady`, `SchedulerReady` insertion |
| `supervisor/src/manifest.rs` | +120 | Parse `[capabilities.audio_realtime]`, `[capabilities.activity_monitor]`, `[dispatch]` |
| `supervisor/src/lifecycle/actor.rs` | +90 | `AppStatus::Parked`, park/resume handlers |
| `supervisor/src/lifecycle/restoration.rs` | +60 | StateBlob path for parked apps (`/data/state/parked/<bundle>.bin`) |
| `supervisor/src/ipc/envelope.rs` | +30 | `effective_qos: Option<QosClass>` field |
| `supervisor/src/ipc/router.rs` | +110 | `promote_for_call` integration, demotion on reply/timeout |
| `supervisor/src/ipc/wit_host.rs` | +40 | Inbound call sites assert activity |
| `supervisor/src/runtime/wasmtime_adapter.rs` | +90 | CPU sampling at WASI boundaries, audio-store epoch-disable |
| `supervisor/src/runtime/wasi_shim.rs` | +60 | `sample_thread` at every fast/slow shim call |
| `supervisor/src/observability/heartbeat.rs` | +50 | Scheduler section: per-app QoS, nap, throttle counters |
| `supervisor/src/mgmt_handlers.rs` | +140 | `sched-stat`, `sched-cgroup`, `sched-assertions`, `sched-degraded` |
| `supervisor/src/memory/jetsam.rs` | +30 | Consult `NapState::Parked` for jetsam exclusion (R2 boundary fix) |

**Total modified:** ~930 added LOC across 13 files.

### 11.1 `mod.rs` skeleton

```rust
// supervisor/src/scheduler/mod.rs

pub mod qos;
pub mod sysctl;
pub mod probe;
pub mod cgroup;
pub mod cpu_account;
pub mod epoch_controller;
pub mod app_nap;
pub mod activity;
pub mod dispatch;
pub mod dispatch_app;
pub mod timers;
pub mod audio;
pub mod audio_watchdog;
pub mod usage;
pub mod thermal;
pub mod decay;
pub mod promotion;

pub use qos::{QosClass, QosPolicy, QosDeriver, FocusObserver};
pub use epoch_controller::EpochController;
pub use app_nap::{NapDetector, NapState};
pub use activity::{ActivityRegistry, ActivityAssertionGuard, ActivityReason};
pub use dispatch::{DispatchPool, HostJob, DispatchError};
pub use timers::{TimerWheel, TimerEntry, TimerKind};
pub use thermal::{ThermalMonitor, ThermalLevel};
pub use usage::{UsageSampler, UsageSnapshot, SystemUsage};
pub use cgroup::CgroupManager;
pub use audio::{AudioRenderThread, set_sched_deadline, set_rlimit_rttime};
pub use promotion::{QosPromotionGuard, PromotionReason};

pub struct Scheduler {
    pub epoch:    Arc<EpochController>,
    pub nap:      Arc<NapDetector>,
    pub activity: Arc<ActivityRegistry>,
    pub dispatch: Arc<DispatchPool>,
    pub timers:   Arc<TimerWheel>,
    pub usage:    Arc<UsageSampler>,
    pub thermal:  Arc<ThermalMonitor>,
    pub qos:      Arc<QosDeriver>,
    pub cgroups:  Arc<CgroupManager>,
    pub caps:     Arc<probe::SchedCapabilities>,
}

impl Scheduler {
    pub fn new(deps: SchedulerDeps) -> anyhow::Result<Arc<Self>> {
        let caps = Arc::new(probe::probe());
        let cgroups = Arc::new(CgroupManager::init(
            deps.cgroup_root.unwrap_or_else(|| PathBuf::from("/sys/fs/cgroup/vyoma"))
        )?);
        let activity = Arc::new(ActivityRegistry::new());
        let qos = Arc::new(QosDeriver::new(
            deps.focus.clone(),
            deps.caps.clone(),
            deps.runtime.clone(),
            activity.clone(),
        ));
        let epoch = Arc::new(EpochController::new(qos.clone(), cgroups.clone()));
        let timers = Arc::new(TimerWheel::new(
            deps.ipc.clone(), deps.dispatch_pool_handle.clone(),
            qos.clone(), activity.clone(),
        ));
        let dispatch = Arc::new(DispatchPool::new(
            num_cpus::get() * 2, deps.ipc.clone(),
            qos.clone(), activity.clone(),
        ));
        let nap = Arc::new(NapDetector::new(
            qos.clone(), epoch.clone(), activity.clone(),
            timers.clone(), deps.lifecycle.clone(),
        ));
        // Bind nap detector into activity registry so assert() can wake.
        activity.bind_nap_detector(nap.clone());
        let usage = Arc::new(UsageSampler::new(
            epoch.clone(), deps.threads.clone(),
            cgroups.clone(), activity.clone(),
            dispatch.clone(), timers.clone(),
            qos.clone(), deps.memory.clone(),
        ));
        let thermal = Arc::new(ThermalMonitor::new(
            qos.clone(), epoch.clone(), cgroups.clone(),
        ));

        Ok(Arc::new(Scheduler {
            epoch, nap, activity, dispatch, timers,
            usage, thermal, qos, cgroups, caps,
        }))
    }

    pub fn start(self: &Arc<Self>) {
        let ec = self.epoch.clone();
        std::thread::Builder::new().name("epoch-ticker".into())
            .spawn(move || ec.run()).unwrap();
        let us = self.usage.clone();
        std::thread::Builder::new().name("usage-sampler".into())
            .spawn(move || us.run()).unwrap();
        let tm = self.thermal.clone();
        std::thread::Builder::new().name("thermal-mon".into())
            .spawn(move || tm.run()).unwrap();
        let nd = self.nap.clone();
        let dr = self.qos.clone();
        tokio::spawn(async move {
            decay::decay_loop(dr, nd).await;
        });
    }
}

pub struct SchedulerDeps {
    pub focus:               Arc<FocusState>,
    pub caps:                Arc<CapabilityRegistry>,
    pub runtime:             Arc<AppRuntimeState>,
    pub ipc:                 Arc<IpcRouter>,
    pub lifecycle:           Arc<LifecycleActor>,
    pub threads:             Arc<ThreadRegistry>,
    pub memory:              Arc<MemoryGovernor>,
    pub dispatch_pool_handle: DispatchPoolHandle,
    pub cgroup_root:         Option<PathBuf>,
}
```

### 11.2 Platform profile bundles

```toml
# supervisor/src/profile/profiles/desktop-full.toml (R5 additions)
[scheduler]
qos_warmup_secs = 5
wakeups_budget_per_sec = 1000
audio_realtime = true
nap_eligible_classes = ["default", "utility", "background", "maintenance"]

# supervisor/src/profile/profiles/server-headless.toml
[scheduler]
qos_warmup_secs = 0
wakeups_budget_per_sec = 100
audio_realtime = false
# Servers have no UI; UserInteractive is unused.
default_qos = "default"
nap_eligible_classes = []

# supervisor/src/profile/profiles/mobile.toml
[scheduler]
qos_warmup_secs = 3
wakeups_budget_per_sec = 100
audio_realtime = true
nap_eligible_classes = ["default", "utility", "background", "maintenance"]
# Mobile is aggressive: park parkable apps after 60 s unfocused.
park_threshold_secs = 60

# supervisor/src/profile/profiles/robotics-rt.toml
[scheduler]
qos_warmup_secs = 0
wakeups_budget_per_sec = 10000
audio_realtime = false
# Robotics gets SCHED_DEADLINE for control loops, not audio.
control_loop_runtime_ns = 1000000   # 1 ms
control_loop_deadline_ns = 4000000  # 4 ms
control_loop_period_ns = 10000000   # 10 ms

# supervisor/src/profile/profiles/iot-edge.toml
[scheduler]
qos_warmup_secs = 0
wakeups_budget_per_sec = 100
audio_realtime = false
# iot-edge has tight thermal; aggressive throttling.
thermal_serious_temp_c = 65
thermal_critical_temp_c = 80

# supervisor/src/profile/profiles/mcu-minimal.toml
[scheduler]
# Cooperative round-robin only; no cgroups, no SCHED_DEADLINE.
cooperative_quantum_us = 10000
qos_warmup_secs = 0
audio_realtime = false
nap_eligible_classes = []
```

### 11.3 Integration with R1–R4

- **R1 (lifecycle):** `AppStatus::Parked { blob_path }` added. `WatchdogActor` is paused
  while an app is `Parked` (Critic G8 addressed). `EpochController` replaces R1's stub
  ticker.
- **R2 (jetsam):** `NapState::Parked` apps are *excluded* from live-jetsam scoring (their
  memory is already gone). Parked-blob storage GC runs in a separate pass under R2's
  pressure handler.
- **R3 (IPC):** `IpcEnvelope::effective_qos`, `IpcRouter::promote_for_call`,
  `WaitForGraph` reused for promotion cycle detection. Every inbound call asserts activity
  via `ActivityRegistry`.
- **R4 (filesystem):** `CoordinationService::acquire` returns a `CoordLock` with embedded
  `ActivityAssertionGuard`. The `WatcherBackend` triggers `activity.assert(... watcher
  event ...)` on subscribers, waking napping apps.

### 11.4 Mgmt panel commands (Critic G7)

| Command | Output |
|---------|--------|
| `sched-stat <bundle>` | Current QoS, effective QoS, nap state, CPU% 1s/60s, assertions list, recent throttle events |
| `sched-cgroup <bundle>` | `cpu.max`, `cpu.weight`, `cpu.stat` raw values |
| `sched-assertions <bundle>` | Live assertion list with reasons |
| `sched-degraded` | `SchedCapabilities` probe results |
| `sched-park <bundle>` | Manually park an app (for testing) |
| `sched-wake <bundle>` | Manually wake a parked app |
| `sched-set-floor <bundle> <qos>` | Manually pin floor (admin only) |
| `sched-budget` | Live wakeup budget + leeway pressure |

### 11.5 Emergency escape hatch (Critic G10)

Kernel command line: `vyoma.sched=off` disables QoS-driven cgroup mutation and falls back
to default `SCHED_OTHER` for every thread with no special handling. Useful for debugging
and as a safety valve. The mgmt panel also exposes `sched-emergency-off` (writable only
via the legacy console after authentication).

---

## Critical v1 Requirements

- 6-class QoS taxonomy: `UserInteractive`, `UserInitiated`, `Default`, `Utility`,
  `Background`, `Maintenance`.
- Derived (not declared) QoS, with focus + recent input + capabilities + assertions
  + user-pinned floor + IPC inheritance as inputs.
- Linux thread `nice` + `sched_setscheduler(SCHED_OTHER/BATCH/IDLE)` for L1 hinting.
- cgroup v2 `/sys/fs/cgroup/vyoma/<bundle>-<iid>/cpu.max` + `cpu.weight` for hard CPU
  caps per QoS class.
- 1 kHz global epoch ticker; per-app `nap_divider` controls effective tick rate.
- `CLOCK_THREAD_CPUTIME_ID` sampling at every WASI boundary + epoch yield for
  sub-millisecond CPU accounting.
- `EpochAccount` + `CpuAccount` per app with 1 s / 10 s / lifetime windows + EWMA.
- `ActivityRegistry` with RAII `ActivityAssertionGuard` for cross-subsystem activity
  tracking; no scheduler-side omniscience.
- IPC, audio, network, FS coord, dispatch, timer subsystems all integrate with the
  registry at lifecycle boundaries.
- `NapDetector` consults assertion count + focus + timers; never reads subsystem state
  directly.
- Park semantics via `on-suspend` WIT callback + StateBlob (never `SIGSTOP`).
- `IpcEnvelope::effective_qos` + router-driven priority promotion for call receivers.
- `WaitForGraph` cycle detection extended to promotion chains.
- `VYOMA_DISPATCH` HostJob path: supervisor-side pool, typed jobs (sha256, png, http, fs).
- `VYOMA_DISPATCH` OnDispatch path: actor-per-app, `on-dispatch` WIT callback delivery.
- Hierarchical timing wheel (5 levels: 1 ms / 64 ms / 4 s / 4 min / 4 h).
- QoS-tiered timer leeway with per-app `hash(app)` jitter to prevent wakeup storms.
- Wakeup budget enforcement (default 1000/s desktop, 100/s mobile).
- Audio realtime via `SCHED_DEADLINE(2ms/8ms/10ms)` + `RLIMIT_RTTIME=5ms` (never SCHED_FIFO).
- `on-audio-render` WIT callback inside SCHED_DEADLINE thread; epoch interruption disabled
  during render window.
- Audio watchdog on dedicated CPU core; SIGKILL on > 500 ms stuck render thread.
- Capability gating: `audio_realtime` requires signed manifest (R1 ed25519).
- Boot warmup: first 5 s gives every app a `UserInitiated` floor.
- Thermal monitor reading `/sys/class/thermal/thermal_zone*/temp`, 4 levels (Nominal,
  Fair, Serious, Critical) with cgroup throttling actions.
- `UsageSnapshot` per app (cpu_pct, cgroup_usage, epochs, RSS, jetsam_score, assertions,
  dispatch_inflight, timers, wakeups, thermal_level).
- `vyoma:cpu/activity-monitor` WIT interface, capability-gated.
- Capability degradation: probe CAP_SYS_NICE / cgroup_v2 at boot; fail-soft + broadcast
  system event.
- Platform profile bundles (`desktop-full`, `server-headless`, `mobile`, `robotics-rt`,
  `iot-edge`, `mcu-minimal`); mcu-minimal uses cooperative round-robin stub.
- Mgmt panel commands: `sched-stat`, `sched-cgroup`, `sched-assertions`, `sched-degraded`,
  `sched-park`, `sched-wake`, `sched-set-floor`, `sched-budget`.
- Kernel command line `vyoma.sched=off` emergency escape hatch.
- Heartbeat scheduler section published every 1 s with per-app QoS + nap + throttle
  counters + thermal level + scheduler_degraded flag.

---

## Deferred to v2

- Per-CPU affinity model + CPU topology awareness (Apple Silicon P-cores / E-cores; ARM
  big.LITTLE).
- WASI-call-latency classification table + "main thread checker" warnings for slow WASI
  calls on UserInteractive threads.
- WASI Preview 3 future/stream integration (suspended-on-future as a distinct activity
  signal).
- Per-window QoS within a single multi-window app (blocked on WASM single-threadedness;
  may revisit when component-model threading is stable).
- Memory-pressure-driven jetsamming coordination with R2 (basic exclusion of Parked apps
  is in v1; sophisticated coupling deferred).
- Adaptive QoS tuning based on per-app historical patterns.
- Cross-app dispatch barriers (`dispatch_barrier_async` semantics).
- `dispatch_after` with sub-millisecond precision (uses timer wheel at 1 ms in v1).
- Persistent activity-monitor history (1 s sampling, no long-term storage in v1).
- Energy attribution (joules per app) beyond thermal-level reporting.
- Scheduler hot-reload without restart.
- Operator UI for live scheduler heatmap (CLI mgmt commands only in v1).
- Property-based tests for promotion cycle detection.
- Deterministic scheduling under test seed.
- Per-app cgroup `cpu.pressure` (PSI) observation.
- `RLIMIT_NPROC`-like quotas on dispatch host-job concurrency per app.
- Audio mixer in supervisor (currently the audio render callback writes directly to the
  sink; multi-app audio mixing is deferred to the audio subsystem round).

---

## Explicitly NEVER

- `SCHED_FIFO` for any WASM-hosted callback (use `SCHED_DEADLINE`).
- `SIGSTOP` / `SIGCONT` on threads inside the supervisor process (use park via WIT
  on-suspend).
- Epoch frequency manipulation as a CPU bandwidth control (use cgroup `cpu.max`).
- Apps declaring their own QoS class (it is derived; the closest analog is the
  `user_pinned_floor` administrative toggle).
- App pool / thread pool for executing app code (Wasmtime Store is `!Sync`; one OS thread
  per WASM instance).
- Subscription-based App Nap that requires scheduler to observe every subsystem's state
  (use `ActivityRegistry` assertion tokens).
- Audio realtime without `RLIMIT_RTTIME` ceiling.
- Audio realtime without a separate watchdog on a different CPU core.
- Audio realtime granted to unsigned manifests.
- Priority inheritance without cycle detection (use R3's WaitForGraph).
- Holding a lock across a WIT call (matches R1/R2/R3/R4 lock-order invariant).
- Epoch deadline callback acquiring AppTable shard lock (callback is hot-path; lock-free
  atomics only).
- Killing apps for CPU overrun based on epoch counts alone (use cgroup `cpu.stat` and
  EWMA CPU% as the authoritative signals).
- Promoting QoS based on app self-request (only IPC inheritance and capability floors).
- Dispatch jobs that re-enter the app's main `Store` from a host worker thread (HostJob
  results are delivered via IPC; OnDispatch runs on the app's own thread).
- `custom-wasm` short-lived Store spawning (removed from FINAL — was a fork-bomb vector
  in the Architect draft).
- Sub-millisecond epoch ticking (overhead too high; CPU-time sampling fills the gap).
- Disabling thermal throttling under `desktop-full` (always on, never opt-out).
- Per-app dedicated epoch ticker threads (single global 1 kHz ticker only).
- Polling `/proc/<tid>/stat` as the primary CPU measurement (use
  `CLOCK_THREAD_CPUTIME_ID` + cgroup `cpu.stat`).

---

## Appendix A: Worked Example — Focus Switch via Cmd-Tab

User holds Cmd-Tab. Compositor publishes focus change from app `A` (foreground) to app
`B` (background-deep-napped).

1. **t=0 ms:** Compositor sends `FocusEvent::Lost(A)` and `FocusEvent::Gained(B)` to
   `FocusObserver`.
2. **t=0.1 ms:** `FocusObserver` calls `qos.derive(A)` (returns `UserInitiated`,
   recently-focused floor) and `qos.derive(B)` (returns `UserInteractive`, now-focused).
3. **t=0.2 ms:** `FocusObserver::apply_qos(A, UserInitiated)` is a no-op if A was already
   `UserInteractive` (transition is allowed; just slightly different). For B, the
   `apply_qos` function:
   - Calls `setpriority(PRIO_PROCESS, B_tid, -10)` (-10 = UserInteractive nice).
   - Calls `sched_setscheduler(B_tid, SCHED_OTHER, …)`.
   - Writes `max 1000000` to `/sys/fs/cgroup/vyoma/B-3/cpu.max`.
   - Writes `10000` to `/sys/fs/cgroup/vyoma/B-3/cpu.weight`.
4. **t=0.3 ms:** `NapDetector::wake_if_napping(B)` is called. B was in `DeepNap`. State
   transitions to `Awake`; `nap_divider` set to 1.
5. **t=0.4 ms (only if B was Parked):** `lifecycle.resume_from_park(B)` spawns a thread,
   creates a `Store`, instantiates the WASM component, calls `on-resume(blob)`. Takes
   ~50 ms typical.
6. **t≈50 ms (if parked) or t=1 ms (if napping):** B's thread receives next epoch tick;
   begins executing event loop.
7. **t=16 ms:** B draws first frame. Compositor flushes. Window animation begins.

End-to-end: ~16 ms for transition from a napping unfocused app to drawing a frame;
~80 ms for transition from a parked app. Both meet the perceived snappy-UI target.

## Appendix B: Worked Example — Calendar Background Sync

Calendar app `cal` is unfocused for 6 hours. User opens it.

1. **t=−6 h:** `cal` is napped to `DeepNap` (no recent activity).
2. **At various points during the 6 hours:** The Round 11 push notification service
   receives a server event for `cal`. It calls `activity.assert(cal,
   NotificationRecent)` and immediately wakes `cal` via `dispatch.submit_app(cal,
   handle_sync_event, args)`.
3. **`cal` processes the event:** Updates local cache, may show a notification. Then
   completes the `on-dispatch` return. The assertion guard is dropped; `cal` returns to
   `DeepNap`.
4. **t=0:** User clicks `cal` in dock. Focus event arrives. `cal` becomes
   `UserInteractive`, draws UI showing fresh events.

The calendar updated 6 times during the day without ever being "Background" → "Awake" →
parked. The assertion mechanism plus push-driven dispatch is exactly the `os_activity`
pattern the Critic called out as required.

## Appendix C: Worked Example — Audio Glitch Recovery

Music player `mp` is playing. WASM render callback overruns 2 ms budget on render #1234.

1. **t=0:** `on-audio-render` invoked with epoch interruption disabled.
2. **t=2 ms:** Kernel SCHED_DEADLINE throttle fires; `mp`'s audio thread cannot run again
   until the next period boundary at t=10 ms.
3. **t=2 ms:** `AudioRenderThread` notices `render_ns > 2_000_000` and calls
   `watchdog.record_overrun(mp, render_ns)`.
4. **t=5 ms:** RLIMIT_RTTIME limit (5 ms) approaches. If callback hadn't returned, SIGKILL
   would fire. (It did return at t=2 ms in this example.)
5. **t=2 ms:** Audio thread blocks until next period.
6. **t=10 ms:** Next period. Audio thread wakes, calls `on-audio-render` again. The
   missed buffer caused a single ~5 ms gap (zero-filled by `submit_silence` fallback).
7. **t=10 s:** Watchdog reports < 10 overruns/60 s window. `mp` retains audio_realtime.
8. **Alternative — if > 10 overruns/60 s:** `mp` is downgraded to buffered playback
   (non-realtime); audio_realtime capability revoked for this session.

This is the macOS "audio underrun" recovery path implemented honestly.

## Appendix D: Worked Example — Priority Inversion Resolved

Calculator `calc` (UserInteractive, focused) calls IPC `evaluate(expr)` on a math service
`mathd` (Background, unfocused).

1. **t=0:** `calc` calls `vyoma:ipc/call("mathd", "evaluate", expr)`.
2. **t=0.1 ms:** Router examines envelope. `qos.derive(calc) = UserInteractive`;
   `qos.derive(mathd) = Background`. Inheritance triggers.
3. **t=0.2 ms:** `qos.promote(mathd, UserInteractive, IpcInheritance{from: calc, id})`
   returns a `QosPromotionGuard`. The guard is stored in the pending-reply entry for the
   call.
4. **t=0.2 ms:** `mathd`'s threads have their `nice` set to -10 and cgroup `cpu.max` set
   to `max`. The promotion takes effect on the next scheduling decision.
5. **t=0.3 ms:** Router delivers envelope to `mathd`'s `inbox_hi` (Call envelopes go to
   hi-priority lane).
6. **t=0.4 ms:** `mathd`'s dispatcher invokes `on-ipc` with the envelope. `mathd`
   computes the result.
7. **t=8 ms:** `mathd` calls `reply(envelope_id, result)`. Router delivers reply to
   `calc::inbox_hi`. The promotion guard is dropped; `mathd` demotes back to `Background`
   (cgroup `cpu.max = 100000 1000000`, nice +15).
8. **t=8.1 ms:** `calc` receives reply, completes its UI frame.

Without inheritance: `mathd` runs at Background priority while `calc` waits. Background
cgroup cap of 10 % of one core means `mathd` could take up to ~80 ms to produce the
result, causing `calc` to miss 5 frames.

## Appendix E: Boot Sequence Integration

```
BootPhase::Mounted9P             // R4
BootPhase::DiskImageReady        // R4
BootPhase::FilesystemMounted     // R4
BootPhase::CgroupReady           // R5 NEW — mount /sys/fs/cgroup/vyoma
BootPhase::XattrStoreReady       // R4
BootPhase::CoordReplayed         // R4
BootPhase::SchedulerReady        // R5 NEW — probe caps, start ticker, sampler, thermal
BootPhase::WatcherReady          // R4
BootPhase::IpcReady              // R3
BootPhase::DisplayReady          // R1 / future R11
BootPhase::PanelReady            // R4
BootPhase::AppsLaunched          // R1 — also sets boot_apps_launched_ms for 5 s warmup
```

## Appendix F: Test Strategy

1. **Unit tests in `supervisor/tests/scheduler/`:**
   - `qos_derivation.rs` — focus + activity + assertions → expected class
   - `cpu_account.rs` — EWMA computation, window rollover
   - `nap_state_machine.rs` — transition table coverage
   - `activity_registry.rs` — RAII drop, count, list
   - `promotion.rs` — single + transitive + cycle rejection
   - `timers.rs` — leeway, jitter, cascade, budget
   - `cgroup.rs` — write + read roundtrips, fault tolerance
2. **Integration tests in `supervisor/tests/scheduler_integration/`:**
   - `focus_promotion.rs` — Cmd-Tab transitions, syscall counts
   - `dispatch_host_path.rs` — sha256 result delivery via IPC
   - `dispatch_app_path.rs` — OnDispatch callback chain
   - `audio_underrun.rs` — render thread overrun → SIGKILL via RLIMIT_RTTIME
   - `priority_inheritance.rs` — calc → mathd worked example
   - `app_nap_assertion.rs` — calendar push event keeps app awake
3. **Smoke test extension:** `make smoke` confirms scheduler section appears in
   heartbeat JSON within 5 s of boot.
4. **Property-based (deferred to v2):** Promotion cycle detection under random IPC
   graphs.

---

Hand-off complete. Round 6 (Device Driver Model) begins next.

# Round 5 Architect: Scheduler & CPU Management

**Status:** Draft — Architect proposal (2026-05-29)
**Author:** Architect role
**Subsystem:** CPU scheduling, QoS, App Nap, GCD-equivalent dispatch, timer coalescing
**macOS equivalent:** XNU Mach scheduler + QoS classes (`NSQualityOfService`) + App Nap +
Grand Central Dispatch (libdispatch) + `dispatch_source_t` timers + Activity Monitor + thermal
pressure
**Integrates with:** R1 (epoch interruption, `LifecycleActor`, `AppStatus`), R2 (`PressureLevel`,
jetsam), R3 (sharded `IpcRouter`, `inbox_hi`/`inbox_lo`), R4 (ext4 backend, watcher events)

---

## 0. Executive Summary

VyomaOS is a Linux-hosted, Wasmtime-embedded WASM-first OS. The hardware CPU scheduler is **Linux
CFS** and is not in our control. Our scheduler subsystem sits *above* CFS and *below* the
application: it decides which Linux threads exist, what nice value they carry, what `SCHED_*`
policy they run under, how aggressively Wasmtime preempts each app via epoch deadlines, when an app
is napped, and how application-level "concurrency" requests are fulfilled despite WASI Preview 2
being single-threaded per instance.

We are reproducing six distinct macOS subsystems on a *single* Linux + Wasmtime substrate:

| macOS subsystem               | VyomaOS module                                | Mechanism                                      |
|-------------------------------|-----------------------------------------------|------------------------------------------------|
| Mach scheduler / `pthread`    | `scheduler/qos.rs`                            | nice + `SCHED_OTHER`/`SCHED_BATCH`/`SCHED_FIFO`|
| `NSQualityOfService`          | `scheduler/qos.rs::QosClass`                  | 6-level enum mapped to nice + epoch budget     |
| App Nap                       | `scheduler/app_nap.rs::NapDetector`           | epoch tick rate divider; suspend on deep nap   |
| Grand Central Dispatch        | `scheduler/dispatch.rs::DispatchPool`         | host-side rayon-style thread pool + WIT result |
| `dispatch_source_t` timers    | `scheduler/timers.rs::TimerWheel`             | hashed timing wheel with leeway coalescing     |
| Activity Monitor CPU%         | `scheduler/qos.rs::UsageSampler`              | epoch delta + `/proc/<tid>/stat`                |
| Thermal pressure              | `scheduler/thermal.rs::ThermalMonitor`        | `/sys/class/thermal/thermal_zoneN/temp` polling|
| Realtime audio                | `scheduler/qos.rs::audio_rt_lane()`           | dedicated `SCHED_FIFO` thread per audio app    |

The headline design choices are:

1. **QoS is a derived property, not declared by the app.** The supervisor computes QoS from
   `(focus_state, capabilities, recent_activity, user_intent)`. The app cannot ask for
   `UserInteractive`; it has to *be* interactive (focused, drawing, taking input). This is critical
   to prevent the macOS-style "every app declares itself important" race.
2. **Epoch deadlines are the only preemption knob.** R1 established epoch interruption. We
   ride that mechanism: instead of varying nice across 20 levels, we vary epoch deadline period
   (1 ms for UI, 1 s for Maintenance). Linux still does the actual CPU dispatch.
3. **App Nap is a tick-rate divider, not a freezer.** Napped apps still get epochs, just rarely
   (every 16-100 ms instead of every 1-10 ms). True suspension (`SIGSTOP`) is reserved for
   long-background apps with no soft state.
4. **GCD becomes supervisor-mediated.** Apps post work items to the supervisor; the supervisor
   maintains a thread pool that runs *host code* on behalf of the app (e.g. SHA256, image decode,
   HTTP fetch). The app stays single-threaded; results come back via WIT callbacks and the IPC
   `inbox_hi` (R3) priority lane. We do **not** spawn additional Wasmtime instances of the same
   app.
5. **Timer coalescing is global.** A single `TimerWheel` across all apps, with per-app leeway
   parameter, runs one supervisor thread for all timer wakeups instead of N app threads.
6. **Realtime lane uses `SCHED_FIFO`** for apps with `audio` capability. This is the only
   non-cooperative path; everything else is epoch-cooperative.

The total scheduler code is < 3500 lines spread across 9 files, each well under the 500-line
limit. The hot path adds ~50-100 ns per epoch tick (cmpxchg + branch); per-app overhead is
dominated by the WASI shim plumbing already established in R1.

---

## 1. CPU Scheduling Philosophy

### 1.1 The two layers

VyomaOS has two *cooperating* schedulers:

```
┌─────────────────────────────────────────────────────────────┐
│  L2: VyomaOS Supervisor Scheduler (this round)              │
│  - QoS class assignment per app                             │
│  - Epoch deadline period per app (preemption granularity)   │
│  - Nice value & sched policy per Linux thread               │
│  - App Nap / suspension decisions                           │
│  - GCD dispatch pool (host-side concurrency)                │
│  - Timer coalescing                                          │
│  - Realtime audio lane                                       │
├─────────────────────────────────────────────────────────────┤
│  L1: Linux CFS / SCHED_FIFO / SCHED_BATCH                   │
│  - Picks which thread runs on which core                     │
│  - Honors nice values & sched policies set by L2            │
│  - Handles SMP load balancing, NUMA, CPU affinity            │
└─────────────────────────────────────────────────────────────┘
```

We deliberately do **not** try to reimplement CFS. Trying to schedule sub-millisecond CPU slices
from userspace would be both slower than CFS and would fight CFS for the right decisions. Instead
we *hint* CFS via nice and sched policy.

### 1.2 What the supervisor actually controls

For each running app, the supervisor controls four scheduling knobs:

1. **Linux thread nice value** — affects CFS's `vruntime` accumulation rate. Lower nice = more CPU.
2. **Linux sched policy** — `SCHED_OTHER` (default), `SCHED_BATCH` (long-running, no interactive
   wakeup boost), `SCHED_IDLE` (only runs when nothing else wants CPU), or `SCHED_FIFO` (realtime).
3. **Wasmtime epoch deadline period** — how often the global epoch counter is incremented, which is
   the granularity at which the app yields back to the supervisor. Faster = more responsive to
   suspend/throttle decisions; slower = less overhead.
4. **CPU affinity (optional)** — pin background apps to a single "efficiency" core if the host has
   asymmetric cores (Intel P+E, Apple Silicon-style ARM big.LITTLE). v1 keeps this off.

### 1.3 Why epoch deadlines, not fuel

Wasmtime fuel meters every guest instruction; epoch interruption checks a counter at
function-call/loop-backedge boundaries. R1 already chose epoch for its ~1-3% overhead vs fuel's
~5-15%. The scheduler adds three responsibilities:

- **Per-app epoch period:** UI apps get fine-grained checks (1 ms tick), background apps get coarse
  (100 ms tick). Implemented as a single global epoch ticker thread that increments at the *finest*
  rate (1 ms), with each app's `Engine` configured to fire the interrupt every Nth tick via
  `Config::epoch_interruption(true)` and `Store::set_epoch_deadline(N)`.
- **Per-app deadline action:** UI apps get `EpochDeadline::Yield` (cheap callback); background apps
  get `EpochDeadline::Trap` (force unwind, supervisor decides whether to resume).
- **Aggregate budget enforcement:** if an app burns through too many epochs in a sliding window, we
  raise its deadline (slow it down) or move it to `SCHED_BATCH`.

### 1.4 Acceptance criteria

- Focused UI app gets ≥ 90 % of one core when contended.
- 10 idle background apps consume < 1 % aggregate CPU.
- An app stuck in a tight WASM loop is preempted within 10 ms (UI) or 100 ms (background).
- An app cannot escape its QoS class by spawning work (because it can't — WASI P2 is
  single-threaded; "spawning" goes through our dispatch pool which is QoS-bounded).
- Realtime audio app meets its 10 ms deadline at 95th percentile under load.

---

## 2. QoS Class Hierarchy

### 2.1 The six classes

We adopt macOS's six-class hierarchy directly, with concrete Linux mappings:

```rust
// supervisor/src/scheduler/qos.rs

use serde::{Deserialize, Serialize};

/// Quality-of-service class. Lower discriminant = higher priority.
/// Mirrors macOS `NSQualityOfService` / `qos_class_t`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum QosClass {
    /// UI thread of the focused app. Drives event loop, animations, input
    /// response. Strict latency target: 16 ms (60 FPS).
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

impl QosClass {
    /// Linux nice value. Lower = more CPU. SCHED_OTHER range is [-20, 19].
    pub fn nice(self) -> i32 {
        match self {
            QosClass::UserInteractive => -10,
            QosClass::UserInitiated   =>   0,
            QosClass::Default         =>   5,
            QosClass::Utility         =>  10,
            QosClass::Background      =>  15,
            QosClass::Maintenance     =>  19,
        }
    }

    /// Linux scheduling policy.
    pub fn sched_policy(self) -> SchedPolicy {
        match self {
            QosClass::UserInteractive => SchedPolicy::Other,
            QosClass::UserInitiated   => SchedPolicy::Other,
            QosClass::Default         => SchedPolicy::Other,
            QosClass::Utility         => SchedPolicy::Other,
            QosClass::Background      => SchedPolicy::Batch,
            QosClass::Maintenance     => SchedPolicy::Batch,
        }
    }

    /// Wasmtime epoch deadline in *global ticks* (1 tick = 1 ms).
    /// Smaller = more preemption points, faster supervisor response.
    pub fn epoch_deadline_ticks(self) -> u64 {
        match self {
            QosClass::UserInteractive => 1,   // 1 ms preemption granularity
            QosClass::UserInitiated   => 4,   // 4 ms
            QosClass::Default         => 10,  // 10 ms
            QosClass::Utility         => 25,  // 25 ms
            QosClass::Background      => 50,  // 50 ms
            QosClass::Maintenance     => 100, // 100 ms
        }
    }

    /// What happens when the epoch deadline fires.
    pub fn epoch_action(self) -> EpochAction {
        match self {
            QosClass::UserInteractive | QosClass::UserInitiated => EpochAction::Yield,
            _                                                   => EpochAction::TrapIfBudgetExceeded,
        }
    }

    /// Epochs per second this class is permitted to consume.
    /// A "consumed" epoch means the app was actually executing WASM at the
    /// moment the epoch interrupt fired (not blocked on I/O, not napped).
    pub fn epochs_per_second_budget(self) -> u32 {
        match self {
            QosClass::UserInteractive => 1000, // unlimited within 1 s
            QosClass::UserInitiated   => 800,
            QosClass::Default         => 500,
            QosClass::Utility         => 200,
            QosClass::Background      => 50,
            QosClass::Maintenance     => 20,
        }
    }

    /// CPU% cap (sustained, 10 s window). Burst allowance is +50%.
    pub fn cpu_pct_cap(self) -> u8 {
        match self {
            QosClass::UserInteractive => 100,
            QosClass::UserInitiated   => 80,
            QosClass::Default         => 50,
            QosClass::Utility         => 25,
            QosClass::Background      => 10,
            QosClass::Maintenance     => 5,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum SchedPolicy { Other, Batch, Idle, Fifo(u8 /* prio 1-99 */) }

#[derive(Debug, Clone, Copy)]
pub enum EpochAction { Yield, TrapIfBudgetExceeded, Trap }
```

### 2.2 Class derivation from app state

QoS is *derived*, not declared:

```rust
// supervisor/src/scheduler/qos.rs (continued)

pub struct QosDeriver {
    /// Focus tracker from R1's LifecycleActor.
    focus: Arc<FocusState>,
    /// Per-app last-activity timestamps.
    activity: DashMap<AppId, ActivitySnapshot>,
    /// Capability declarations from each app's vyoma.toml.
    caps: Arc<CapabilityRegistry>,
}

#[derive(Debug, Clone)]
pub struct ActivitySnapshot {
    pub last_user_input_ms:  u64,  // last keystroke/mouse-in-window
    pub last_focus_ms:       u64,  // last time app was foreground
    pub last_audio_frame_ms: u64,  // last audio buffer submitted
    pub last_net_io_ms:      u64,  // last socket read/write
    pub last_draw_flush_ms:  u64,  // last VYOMA_DRAW:flush
    pub progress_visible:    bool, // app announced visible progress
}

impl QosDeriver {
    pub fn derive(&self, app: AppId) -> QosClass {
        let caps = self.caps.get(app);
        let act  = self.activity.get(&app).map(|r| r.clone()).unwrap_or_default();
        let now  = monotonic_ms();

        // Realtime audio is its own lane (Section 9); QoS becomes UserInteractive
        // for the rest of the app's UI work, while the audio thread runs SCHED_FIFO.
        if caps.audio && now.saturating_sub(act.last_audio_frame_ms) < 1_000 {
            return QosClass::UserInteractive;
        }

        // Focused window → top class.
        if self.focus.is_focused(app) {
            return QosClass::UserInteractive;
        }

        // Recently focused (≤ 5 s ago) → user-initiated.
        let since_focus = now.saturating_sub(act.last_focus_ms);
        if since_focus < 5_000 {
            return QosClass::UserInitiated;
        }

        // Showing progress, or actively doing network I/O → Utility floor.
        if act.progress_visible
            || now.saturating_sub(act.last_net_io_ms) < 10_000
        {
            return QosClass::Utility;
        }

        // Unfocused for > 60 s with no visible work → Background.
        if since_focus > 60_000 {
            return QosClass::Background;
        }

        // Otherwise: Utility (still in user's attention orbit).
        QosClass::Utility
    }
}
```

### 2.3 The supervisor cannot trust app self-declarations

Apps cannot ask for a higher QoS class. They can only:

- **Become focused** (which is the user's decision).
- **Show progress** (via `VYOMA_DRAW:progress:`), which floors them at `Utility`.
- **Take user input** (which proves they're interactive).
- **Run audio** (which gates them onto the realtime lane).

This is a deliberate anti-pattern for the macOS bug where every app declares
`NSQualityOfServiceUserInitiated` and starves background work.

### 2.4 Applying QoS to a Linux thread

```rust
// supervisor/src/scheduler/qos.rs (continued)

use nix::sched::{sched_setscheduler, Sched};
use nix::sys::resource::{setpriority, PriorityWhich};

pub fn apply_qos(tid: libc::pid_t, qos: QosClass) -> Result<(), SchedError> {
    let nice  = qos.nice();
    let pol   = qos.sched_policy();

    // setpriority() takes the "who" param. For threads, that's the tid.
    unsafe {
        if libc::setpriority(libc::PRIO_PROCESS, tid as u32, nice) != 0 {
            return Err(SchedError::SetPriority(io::Error::last_os_error()));
        }
    }

    let (policy, sparam) = match pol {
        SchedPolicy::Other     => (libc::SCHED_OTHER,                0),
        SchedPolicy::Batch     => (libc::SCHED_BATCH,                0),
        SchedPolicy::Idle      => (libc::SCHED_IDLE,                 0),
        SchedPolicy::Fifo(p)   => (libc::SCHED_FIFO,                 p as i32),
    };
    let param = libc::sched_param { sched_priority: sparam };
    unsafe {
        if libc::sched_setscheduler(tid, policy, &param) != 0 {
            return Err(SchedError::SetScheduler(io::Error::last_os_error()));
        }
    }
    Ok(())
}
```

`SCHED_FIFO` requires `CAP_SYS_NICE`; the supervisor runs as PID 1 with full capabilities at boot,
which it retains for setting realtime threads on behalf of audio apps.

---

## 3. Per-App CPU Budgeting

### 3.1 Epoch as the unit of charge

We meter CPU in **epochs consumed**, not Linux CPU-time, because:

- An app sleeping on I/O does not consume epochs.
- An app waiting in `inbox_lo` does not consume epochs.
- An app spinning in WASM consumes epochs at the deadline rate.

The supervisor maintains, for each app:

```rust
// supervisor/src/scheduler/epoch_controller.rs

pub struct EpochAccount {
    /// Lifetime epochs consumed.
    pub lifetime: AtomicU64,
    /// Epochs consumed in the current 1 s window.
    pub window_1s: AtomicU32,
    /// Epochs consumed in the current 10 s window.
    pub window_10s: AtomicU32,
    /// Last reset timestamp (ms).
    pub last_reset_ms: AtomicU64,
    /// Number of consecutive over-budget windows.
    pub overrun_strikes: AtomicU8,
}
```

### 3.2 The epoch ticker

A single supervisor thread (`epoch-ticker`) runs at 1 kHz and increments every app's `Engine`
epoch:

```rust
// supervisor/src/scheduler/epoch_controller.rs (continued)

pub struct EpochController {
    apps: Arc<DashMap<AppId, AppEpochCtx>>,
    tick_period: Duration, // 1 ms
}

pub struct AppEpochCtx {
    pub engine: wasmtime::Engine,
    pub deadline_ticks: AtomicU64, // current QoS class's deadline
    pub account: EpochAccount,
    pub qos: AtomicU8, // current QosClass as u8
    pub nap_divider: AtomicU8, // 1=normal, 8=light nap, 60=deep nap
    pub tick_counter: AtomicU64,
}

impl EpochController {
    pub fn run(self: Arc<Self>) {
        let mut next = Instant::now();
        loop {
            next += self.tick_period;
            for entry in self.apps.iter() {
                let ctx = entry.value();
                let n = ctx.tick_counter.fetch_add(1, Ordering::Relaxed);
                let div = ctx.nap_divider.load(Ordering::Relaxed) as u64;
                // Only tick this app's engine every `div` master ticks.
                if div == 1 || (n % div) == 0 {
                    ctx.engine.increment_epoch();
                }
            }
            // Coarse-grained reconciliation every 100 ms.
            if next.duration_since(Instant::now()).is_zero() {
                self.reconcile_budgets();
            }
            spin_sleep::sleep_until(next);
        }
    }

    fn reconcile_budgets(&self) {
        let now_ms = monotonic_ms();
        for entry in self.apps.iter() {
            let ctx = entry.value();
            let last = ctx.account.last_reset_ms.load(Ordering::Relaxed);
            if now_ms - last >= 1_000 {
                let consumed = ctx.account.window_1s.swap(0, Ordering::Relaxed);
                let qos = QosClass::from_u8(ctx.qos.load(Ordering::Relaxed));
                let budget = qos.epochs_per_second_budget();
                if consumed > budget {
                    self.handle_overrun(entry.key().clone(), consumed, budget, qos);
                } else {
                    ctx.account.overrun_strikes.store(0, Ordering::Relaxed);
                }
                ctx.account.last_reset_ms.store(now_ms, Ordering::Relaxed);
            }
        }
    }

    fn handle_overrun(
        &self,
        app: AppId,
        consumed: u32,
        budget: u32,
        qos: QosClass,
    ) {
        let strikes = self.apps.get(&app)
            .map(|e| e.value().account.overrun_strikes.fetch_add(1, Ordering::Relaxed) + 1)
            .unwrap_or(0);
        match strikes {
            1..=2 => {
                // Warning: log it.
                tracing::warn!(app = %app, consumed, budget, "epoch overrun");
            }
            3..=5 => {
                // Throttle: bump the deadline period 2x.
                self.bump_deadline(app, 2);
            }
            6.. => {
                // Demote QoS one notch (Utility → Background → ...).
                self.demote_qos(app, qos);
            }
            _ => {}
        }
    }

    fn bump_deadline(&self, app: AppId, factor: u64) {
        if let Some(entry) = self.apps.get(&app) {
            let old = entry.value().deadline_ticks.load(Ordering::Relaxed);
            entry.value().deadline_ticks.store(old * factor, Ordering::Relaxed);
        }
    }

    fn demote_qos(&self, app: AppId, current: QosClass) {
        let next = match current {
            QosClass::UserInteractive => QosClass::UserInitiated,
            QosClass::UserInitiated   => QosClass::Default,
            QosClass::Default         => QosClass::Utility,
            QosClass::Utility         => QosClass::Background,
            QosClass::Background      => QosClass::Maintenance,
            QosClass::Maintenance     => QosClass::Maintenance,
        };
        // ... apply nice + sched policy + deadline.
        tracing::warn!(app = %app, ?current, ?next, "QoS demoted due to budget overrun");
    }
}
```

### 3.3 Counting consumed epochs

The naive approach — incrementing a per-app counter on every epoch tick — would require
distinguishing "app was running WASM" from "app was blocked." We avoid that by **moving the
counter into the epoch deadline callback** instead:

```rust
// In supervisor/src/runtime/wasmtime_adapter.rs (R1 module, extended)

store.epoch_deadline_callback(move |_store| {
    // This callback only fires when the app was actually executing WASM at
    // the moment the epoch ticker bumped the engine epoch.
    ctx.account.window_1s.fetch_add(1, Ordering::Relaxed);
    ctx.account.window_10s.fetch_add(1, Ordering::Relaxed);
    ctx.account.lifetime.fetch_add(1, Ordering::Relaxed);

    let deadline = ctx.deadline_ticks.load(Ordering::Relaxed);
    Ok(UpdateDeadline::Continue(deadline))
});
```

This is precise: the callback fires exactly once per "I was running WASM and got interrupted"
event. The window counter is therefore a true epoch-consumed count.

### 3.4 Burst allowance

Within any 100 ms window, an app may consume up to **2x** its per-second budget without
penalty. This handles bursty UI workloads (a single 50 ms paint frame doing heavy work).

```rust
// supervisor/src/scheduler/epoch_controller.rs (continued)

impl EpochController {
    fn burst_ok(&self, ctx: &AppEpochCtx, qos: QosClass) -> bool {
        let consumed_100ms = ctx.account.window_1s.load(Ordering::Relaxed);
        let burst_cap = qos.epochs_per_second_budget() / 5; // 200 ms cap, 100 ms burst
        consumed_100ms <= burst_cap * 2
    }
}
```

### 3.5 Kill thresholds

If `overrun_strikes >= 10`, the app is killed with a `BudgetExhausted` reason and reported to
the user via the activity monitor. This is the CPU-side analog of R2's jetsam memory kill.

---

## 4. Focus-Aware Priority

### 4.1 Focus events drive QoS transitions

R1 established the `FocusState` actor. On focus change, the scheduler is notified:

```rust
// supervisor/src/scheduler/qos.rs (continued)

pub enum FocusEvent {
    Gained { app: AppId, at_ms: u64 },
    Lost   { app: AppId, at_ms: u64 },
}

pub struct FocusObserver {
    deriver: Arc<QosDeriver>,
    epoch_ctl: Arc<EpochController>,
    rx: tokio::sync::mpsc::Receiver<FocusEvent>,
}

impl FocusObserver {
    pub async fn run(mut self) {
        while let Some(ev) = self.rx.recv().await {
            match ev {
                FocusEvent::Gained { app, at_ms } => {
                    self.deriver.activity.entry(app.clone())
                        .and_modify(|a| a.last_focus_ms = at_ms);
                    self.apply_qos(app, QosClass::UserInteractive);
                }
                FocusEvent::Lost { app, at_ms } => {
                    self.deriver.activity.entry(app.clone())
                        .and_modify(|a| a.last_focus_ms = at_ms);
                    let new_qos = self.deriver.derive(app.clone());
                    self.apply_qos(app, new_qos);
                }
            }
        }
    }

    fn apply_qos(&self, app: AppId, qos: QosClass) {
        if let Some(tids) = self.thread_registry.tids_for(app.clone()) {
            for tid in tids {
                let _ = apply_qos(tid, qos);
            }
        }
        self.epoch_ctl.set_qos(app, qos);
    }
}
```

### 4.2 Decay schedule

An unfocused app's QoS decays over time:

| Time since focus | QoS class       | Rationale                                  |
|------------------|-----------------|--------------------------------------------|
| 0 s (focused)    | UserInteractive | User is looking at it                      |
| 0-5 s            | UserInitiated   | User probably switched but may come back   |
| 5-60 s           | Utility         | App should keep working but yield to focus |
| > 60 s           | Background      | App is genuinely backgrounded              |

A periodic ticker (1 Hz) reconsiders QoS for every non-focused app:

```rust
// supervisor/src/scheduler/qos.rs (continued)

pub async fn decay_loop(deriver: Arc<QosDeriver>, observer: Arc<FocusObserver>) {
    let mut interval = tokio::time::interval(Duration::from_secs(1));
    loop {
        interval.tick().await;
        for entry in deriver.activity.iter() {
            let app = entry.key().clone();
            if deriver.focus.is_focused(&app) { continue; }
            let new_qos = deriver.derive(app.clone());
            observer.apply_qos(app, new_qos);
        }
    }
}
```

### 4.3 Floors for active background work

Three reasons floor an unfocused app at `Utility`:

1. **`audio` capability with recent audio frame** → also gets realtime lane.
2. **Network I/O in last 10 s** → likely a download/upload the user kicked off.
3. **`progress_visible` flag** → app announced an in-progress task via
   `VYOMA_DRAW:progress:<percent>` or the `vyoma:progress` WIT export.

These floors override the decay schedule. A download that takes 5 minutes does **not** decay to
`Background` after 60 s as long as `last_net_io_ms` keeps refreshing.

### 4.4 Explicitly-backgrounded apps

The user can mark an app "Keep running in background" from the Activity Monitor UI. This sets a
persistent flag in the app's runtime state:

```rust
pub struct AppRuntimeState {
    pub user_pinned_floor: Option<QosClass>, // Some(Utility) → "Keep running"
    // ...
}
```

The deriver respects this:

```rust
impl QosDeriver {
    pub fn derive(&self, app: AppId) -> QosClass {
        let derived = self.derive_unfloored(app.clone());
        if let Some(floor) = self.runtime.get(&app).and_then(|s| s.user_pinned_floor) {
            derived.min(floor) // remember: lower discriminant = higher prio
        } else {
            derived
        }
    }
}
```

---

## 5. App Nap Equivalent

### 5.1 Napable detection

An app is "napable" if **all** of:

- Not focused.
- No audio frame submitted in last 10 s.
- No socket read/write in last 10 s.
- No `progress_visible` flag.
- No timer due within the next 1 s.
- No pending IPC message in `inbox_hi` (R3).
- App did not call `vyoma:cpu/keep-awake()` (Section 5.4).
- App has not displayed a notification in last 30 s.

```rust
// supervisor/src/scheduler/app_nap.rs

pub struct NapDetector {
    deriver: Arc<QosDeriver>,
    timers: Arc<TimerWheel>,
    ipc: Arc<IpcRouter>,
    keep_awake: DashMap<AppId, Instant>, // explicit keep-awake until
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NapState {
    Awake,         // normal
    LightNap,      // epoch tick divider = 8 (8 ms effective tick)
    DeepNap,       // epoch tick divider = 60 (60 ms tick)
    Suspended,     // SIGSTOP issued; app is frozen
}

impl NapDetector {
    pub fn classify(&self, app: &AppId, since_unfocused_ms: u64) -> NapState {
        let act = match self.deriver.activity.get(app) {
            Some(a) => a.clone(),
            None    => return NapState::Awake,
        };
        let now = monotonic_ms();

        // Hard wakers: any of these → Awake.
        if self.has_keep_awake(app) { return NapState::Awake; }
        if now.saturating_sub(act.last_audio_frame_ms) < 1_000 { return NapState::Awake; }
        if act.progress_visible { return NapState::Awake; }
        if self.timers.has_timer_within(app, Duration::from_secs(1)) { return NapState::Awake; }
        if self.ipc.has_hi_pending(app) { return NapState::Awake; }
        if self.deriver.focus.is_focused(app) { return NapState::Awake; }

        // Decay into nap states.
        match since_unfocused_ms {
            0..=10_000              => NapState::Awake,
            10_001..=60_000         => NapState::LightNap,
            60_001..=600_000        => NapState::DeepNap,
            _ => {
                // Eligible for suspension if app exports `vyoma:lifecycle/suspendable=true`
                // and has no soft state warning.
                if self.is_suspendable(app) {
                    NapState::Suspended
                } else {
                    NapState::DeepNap
                }
            }
        }
    }
}
```

### 5.2 Applying nap state

```rust
// supervisor/src/scheduler/app_nap.rs (continued)

impl NapDetector {
    pub fn apply(&self, app: &AppId, state: NapState) {
        let ctx = match self.epoch_ctl.apps.get(app) { Some(c) => c, None => return };
        match state {
            NapState::Awake => {
                ctx.value().nap_divider.store(1, Ordering::Relaxed);
                self.unsuspend(app);
            }
            NapState::LightNap => {
                ctx.value().nap_divider.store(8, Ordering::Relaxed);
                self.unsuspend(app);
            }
            NapState::DeepNap => {
                ctx.value().nap_divider.store(60, Ordering::Relaxed);
                self.unsuspend(app);
            }
            NapState::Suspended => {
                if let Some(tids) = self.threads.tids_for(app.clone()) {
                    for tid in tids {
                        unsafe { libc::kill(tid, libc::SIGSTOP); }
                    }
                }
                // R1: AppStatus becomes Suspended.
                self.lifecycle.set_status(app, AppStatus::Suspended);
            }
        }
    }

    fn unsuspend(&self, app: &AppId) {
        if self.lifecycle.status(app) == AppStatus::Suspended {
            if let Some(tids) = self.threads.tids_for(app.clone()) {
                for tid in tids {
                    unsafe { libc::kill(tid, libc::SIGCONT); }
                }
            }
            self.lifecycle.set_status(app, AppStatus::Background);
        }
    }
}
```

### 5.3 Wake triggers

A suspended/napping app is woken on:

- IPC message delivery (R3): `IpcRouter::deliver()` calls `nap_detector.wake(target_app)`.
- User input event (keyboard, mouse-in-window).
- Network activity (host-side socket readable: requires a `tokio::net` reactor that pings the
  scheduler).
- Timer expiry (Section 6).
- Filesystem watcher event (R4).
- User clicks/taps the app's window.

```rust
impl NapDetector {
    pub fn wake(&self, app: &AppId, reason: WakeReason) {
        tracing::debug!(?app, ?reason, "nap wake");
        self.apply(app, NapState::Awake);
        // Reset the "since unfocused" countdown.
        self.deriver.activity.entry(app.clone()).and_modify(|a| {
            a.last_user_input_ms = monotonic_ms();
        });
    }
}

pub enum WakeReason { Ipc, Input, Net, Timer, Watcher, Focus }
```

### 5.4 App-requested keep-awake

Apps performing background work that the supervisor can't detect (e.g. a pure-WASM compression
job) call:

```wit
// wit/vyoma-cpu.wit

interface cpu {
    // ...

    /// Request that the supervisor not nap this app for `duration-secs`
    /// seconds. The reason is shown in Activity Monitor.
    /// Max duration: 600 s. Caller must re-request to extend.
    /// Returns a token; pass to `release-keep-awake` to release early.
    keep-awake: func(reason: string, duration-secs: u32) -> result<u64, cpu-error>;

    release-keep-awake: func(token: u64);
}
```

The supervisor enforces an upper bound (600 s) and the reason is surfaced in Activity Monitor as
"App is preventing sleep: <reason>", so users can see which app is keeping the system busy. Repeat
offenders (apps requesting keep-awake > 50% of the time) are downgraded in QoS.

### 5.5 Suspendability declaration

Only apps that declare `[lifecycle] suspendable = true` in their `vyoma.toml` get `Suspended`
state. Others bottom out at `DeepNap`. Suspension freezes the entire WASM instance via
`SIGSTOP`, which means:

- File descriptors stay open.
- Memory stays allocated.
- IPC messages pile up in `inbox_lo` (R3).
- No CPU is consumed, period.

This is closer to macOS App Nap's "freeze + freeze descriptors" semantics. The risk: an app
with a half-completed network transaction won't process its socket on wake until it next
schedules a read. Apps that hold realtime obligations (audio, video stream) should never
declare `suspendable = true`.

---

## 6. Grand Central Dispatch Equivalent (VYOMA_DISPATCH)

### 6.1 Why we need it

WASI Preview 2 components are single-threaded: one instance, one execution thread, no
`thread.spawn`. But desktop apps want concurrency: download in background while UI stays
responsive; decode an image while the event loop runs; periodic timer firing without blocking
the main flow.

macOS provides this via libdispatch / GCD: app submits a block to a queue, the kernel runs it on
a worker thread, delivers the result. We provide the *same shape* but the worker thread lives
in the **supervisor**, not in the app.

### 6.2 What dispatch can do

Dispatch work items are **typed**, not arbitrary closures. Apps can dispatch:

1. **Pure host computation**: `sha256(bytes)`, `gzip_compress(bytes, level)`,
   `png_decode(bytes)`, `json_parse(text)` — supervisor runs the work on a rayon-style pool
   and returns the result.
2. **Network fetch**: `http_get(url, headers)`, `http_post(url, body)` — supervisor uses its
   own `hyper`/`reqwest` client, returns body or stream handle.
3. **Filesystem batch**: `read_file(path)`, `write_file(path, bytes)`, `glob(pattern)` —
   supervisor uses R4's VFS layer.
4. **Custom WASM job**: apps can register a *child component* that the supervisor instantiates
   for short-lived dispatch jobs. This is the closest equivalent to "spawn a thread that runs
   my code." See Section 6.7.

Work items are **declarative** because the supervisor needs to execute them safely without
trusting the app to provide native code.

### 6.3 Queue types

```rust
// supervisor/src/scheduler/dispatch.rs

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum QueueKind {
    /// FIFO; one job at a time per app per queue.
    Serial { name: SmolStr },
    /// Parallel up to QoS-derived width.
    Concurrent { name: SmolStr },
    /// Runs back on the app's main thread (event loop) — used for callbacks.
    Main,
}

pub struct DispatchQueue {
    pub kind:    QueueKind,
    pub owner:   AppId,
    pub qos:     QosClass,
    pub pending: VecDeque<WorkItem>,
    pub running: u32, // for Concurrent
}
```

### 6.4 Work item & lifecycle

```rust
// supervisor/src/scheduler/dispatch.rs (continued)

#[derive(Debug)]
pub struct WorkItem {
    pub id:        u64,
    pub owner:     AppId,
    pub queue:     QueueKind,
    pub job:       JobKind,
    pub callback:  CallbackTarget,
    pub submitted: Instant,
    pub deadline:  Option<Instant>, // dispatch_after
    pub barrier:   bool,            // dispatch_barrier
}

#[derive(Debug)]
pub enum JobKind {
    HashSha256 { bytes: Vec<u8> },
    HashBlake3 { bytes: Vec<u8> },
    GzipCompress { bytes: Vec<u8>, level: u8 },
    GzipDecompress { bytes: Vec<u8> },
    PngDecode { bytes: Vec<u8> },
    JpegDecode { bytes: Vec<u8> },
    JsonParse { text: String },
    HttpGet { url: String, headers: Vec<(String, String)> },
    HttpPost { url: String, headers: Vec<(String, String)>, body: Vec<u8> },
    ReadFile { path: BookmarkRef }, // R4 bookmark
    WriteFile { path: BookmarkRef, bytes: Vec<u8> },
    Glob { pattern: String, root: BookmarkRef },
    CustomWasm { component_id: ComponentId, input: Vec<u8> },
    Delay { until: Instant }, // dispatch_after, no actual work
}

#[derive(Debug)]
pub struct CallbackTarget {
    pub app: AppId,
    pub continuation_id: u64, // app-allocated handle for matching reply
}

#[derive(Debug)]
pub enum DispatchResult {
    Ok(Vec<u8>),
    Err(DispatchError),
    Cancelled,
}
```

### 6.5 Pool execution

The supervisor's dispatch pool is sized at `num_cpus * 2`, all running `SCHED_OTHER` at nice 5
by default. Workers pick from a global priority queue partitioned by QoS:

```rust
// supervisor/src/scheduler/dispatch.rs (continued)

pub struct DispatchPool {
    workers: Vec<JoinHandle<()>>,
    queues:  Arc<Mutex<PriorityQueues>>,
    notify:  Arc<Notify>,
}

pub struct PriorityQueues {
    by_qos: [VecDeque<WorkItem>; 6], // indexed by QosClass as u8
    serial_locks: HashMap<(AppId, SmolStr), bool>, // serial queue mutual exclusion
}

impl DispatchPool {
    pub fn submit(&self, mut item: WorkItem) -> Result<(), DispatchError> {
        // Tag with current QoS of the owning app.
        let qos = self.qos_deriver.derive(item.owner.clone());
        let idx = qos as usize;
        let mut q = self.queues.lock();
        q.by_qos[idx].push_back(item);
        drop(q);
        self.notify.notify_one();
        Ok(())
    }

    fn worker_loop(self: Arc<Self>) {
        loop {
            let item = {
                let mut q = self.queues.lock();
                // Pull from highest-priority non-empty queue first.
                let mut found = None;
                for idx in 0..6 {
                    if let Some(it) = q.by_qos[idx].pop_front() {
                        if let QueueKind::Serial { ref name } = it.queue {
                            let key = (it.owner.clone(), name.clone());
                            if q.serial_locks.get(&key).copied().unwrap_or(false) {
                                // Already running; put back at tail.
                                q.by_qos[idx].push_back(it);
                                continue;
                            }
                            q.serial_locks.insert(key, true);
                        }
                        found = Some(it);
                        break;
                    }
                }
                found
            };
            match item {
                Some(it) => self.execute(it),
                None     => self.notify.notified_blocking(),
            }
        }
    }

    fn execute(&self, item: WorkItem) {
        let result = match &item.job {
            JobKind::HashSha256 { bytes } => {
                let h = sha2::Sha256::digest(bytes);
                DispatchResult::Ok(h.to_vec())
            }
            JobKind::GzipCompress { bytes, level } => {
                gzip_compress(bytes, *level).map(DispatchResult::Ok)
                    .unwrap_or_else(|e| DispatchResult::Err(e.into()))
            }
            JobKind::HttpGet { url, headers } => {
                tokio_block_on(self.http.get(url, headers))
                    .map(DispatchResult::Ok)
                    .unwrap_or_else(|e| DispatchResult::Err(e.into()))
            }
            JobKind::CustomWasm { component_id, input } => {
                self.execute_custom_wasm(*component_id, input.clone(), &item.owner)
            }
            // ... other variants
            _ => DispatchResult::Err(DispatchError::Unsupported),
        };

        // Release serial lock if any.
        if let QueueKind::Serial { ref name } = item.queue {
            let mut q = self.queues.lock();
            q.serial_locks.remove(&(item.owner.clone(), name.clone()));
        }

        // Deliver result back to app via inbox_hi (R3).
        let reply = DispatchReply {
            continuation_id: item.callback.continuation_id,
            result,
        };
        self.ipc.deliver_hi(item.callback.app.clone(), reply.into_message());
    }
}
```

### 6.6 WIT surface

```wit
// wit/vyoma-dispatch.wit

interface dispatch {
    use cpu.{qos-class};

    type continuation-id = u64;
    type queue-handle    = u64;

    record queue-spec {
        kind:  queue-kind,
        name:  string,
        qos:   option<qos-class>,
    }

    variant queue-kind { serial, concurrent, main }

    variant job-spec {
        hash-sha256(list<u8>),
        hash-blake3(list<u8>),
        gzip-compress(tuple<list<u8>, u8>),
        gzip-decompress(list<u8>),
        png-decode(list<u8>),
        jpeg-decode(list<u8>),
        json-parse(string),
        http-get(tuple<string, list<tuple<string, string>>>),
        http-post(tuple<string, list<tuple<string, string>>, list<u8>>),
        read-file(bookmark-ref),
        write-file(tuple<bookmark-ref, list<u8>>),
        custom-wasm(tuple<u64, list<u8>>), // (component-id, input)
        delay(u64), // ms
    }

    create-queue: func(spec: queue-spec) -> result<queue-handle, dispatch-error>;

    /// Submit work. Returns a continuation-id; result is delivered to the
    /// app's `dispatch-callback` export when the job completes.
    dispatch-async: func(
        queue: queue-handle,
        job: job-spec
    ) -> result<continuation-id, dispatch-error>;

    /// Cancel a queued work item if not yet started.
    cancel: func(continuation-id) -> bool;

    /// Submit work that will run after the queue's pending items complete
    /// AND after a barrier completes its execution before subsequent items.
    /// Useful for "drain everything, then run cleanup."
    dispatch-barrier: func(
        queue: queue-handle,
        job: job-spec
    ) -> result<continuation-id, dispatch-error>;

    /// Submit work that runs no earlier than `at-ms` ms from now.
    dispatch-after: func(
        queue: queue-handle,
        at-ms: u64,
        job: job-spec
    ) -> result<continuation-id, dispatch-error>;
}

// App must export:
interface dispatch-callbacks {
    use dispatch.{continuation-id};

    handle-result: func(continuation-id: continuation-id, result: result<list<u8>, dispatch-error>);
}
```

### 6.7 Custom WASM dispatch (the escape hatch)

Apps occasionally need to run *their own code* in a worker (e.g. a custom data parser). They
package that code as a separate WASM component declared in their manifest:

```toml
# apps/myapp/vyoma.toml
[[dispatch.components]]
name = "parser-worker"
wasm = "parser-worker.wasm"
qos  = "utility"
memory_max_mb = 64

[dispatch]
max_inflight = 4
```

The supervisor instantiates `parser-worker.wasm` on demand in its own short-lived Wasmtime
`Store`, runs it for ≤ 5 s (configurable), and returns the result. This Store is **isolated**
from the parent app's WASI capabilities: it gets only stdio. If the parser needs more, it must
go through the parent's IPC.

### 6.8 Why not just give apps `thread.spawn`?

- WASI P2 doesn't have it yet (P3 might).
- Multi-threaded WASM requires shared linear memory, which fights every other isolation
  guarantee in this OS.
- Letting the app run host code (rayon, tokio) inside the supervisor address space is a no-go.
- Declarative jobs let us QoS-bound, cancel, and monitor concurrency from the supervisor's
  view of the world.

The trade-off: apps can't run *arbitrary* compute concurrently — only the job types we ship.
This is fine for 90% of use cases. The 10% (custom compute) goes through `custom-wasm`.

---

## 7. Timer Coalescing

### 7.1 The wakeup problem

Each timer wakeup costs ~5-50 µs of kernel scheduling overhead and prevents the host CPU from
entering deep C-states. With 100 apps each running a 1 s timer, naïve scheduling gives 100
wakeups per second; coalesced, it's ~10. This is what macOS's `dispatch_source_t` + leeway
parameter and Linux's `epoll_pwait2 + IORING_TIMEOUT_ABS` achieve.

### 7.2 Hashed timing wheel

We use a hierarchical timing wheel (a la `tokio::time::driver`) with these levels:

| Level | Slot duration | Total range |
|-------|---------------|-------------|
| 0     | 1 ms          | 64 ms       |
| 1     | 64 ms         | 4 s         |
| 2     | 4 s           | 4 min       |
| 3     | 4 min         | 4 h         |
| 4     | 4 h           | 11 days     |

```rust
// supervisor/src/scheduler/timers.rs

pub struct TimerWheel {
    levels: [Level; 5],
    epoch_ms: AtomicU64,
    next_id: AtomicU64,
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
    /// Deliver a `vyoma:time/tick` event to the app.
    Tick { user_data: u64 },
    /// Enqueue a dispatch job at expiry.
    Dispatch { job: Box<JobKind>, queue: QueueKind },
    /// Internal supervisor task (nap re-evaluation, jetsam scan).
    Internal { task: InternalTask },
}
```

### 7.3 Coalescing pass

Every 1 ms tick, the wheel advances level 0 by one slot. When level 0 wraps, level 1 advances
by one slot, cascading down to level 0 in the new range. The key trick: when timers cascade
into level 0, the coalescer **batches** all timers landing within ±leeway of each other into a
single firing event:

```rust
// supervisor/src/scheduler/timers.rs (continued)

impl TimerWheel {
    pub fn advance(&self, now_ms: u64) {
        let last = self.epoch_ms.swap(now_ms, Ordering::Relaxed);
        let elapsed_ms = now_ms.saturating_sub(last);

        // Advance the bottom level and cascade upward.
        for _ in 0..elapsed_ms {
            self.advance_one_ms();
        }
    }

    fn advance_one_ms(&self) {
        let lvl0 = &self.levels[0];
        let slot = lvl0.current_slot.fetch_add(1, Ordering::Relaxed) % lvl0.slots.len();
        let mut bucket = lvl0.slots[slot].lock();
        let due: SmallVec<[TimerEntry; 4]> = bucket.timers.drain(..).collect();
        drop(bucket);

        // Coalesce: group by (deadline_ms ± leeway) into firing batches.
        let mut batches: HashMap<u64, Vec<TimerEntry>> = HashMap::new();
        for t in due {
            let bucket_key = t.deadline_ms / 10; // 10 ms coarse grouping baseline
            batches.entry(bucket_key).or_default().push(t);
        }
        for (_k, group) in batches {
            self.fire_batch(group);
        }

        // Cascade if we wrapped level 0.
        if slot == lvl0.slots.len() - 1 {
            self.cascade_from(1);
        }
    }

    fn cascade_from(&self, lvl_idx: usize) {
        if lvl_idx >= self.levels.len() { return; }
        let lvl = &self.levels[lvl_idx];
        let slot = lvl.current_slot.fetch_add(1, Ordering::Relaxed) % lvl.slots.len();
        let mut bucket = lvl.slots[slot].lock();
        let cascading: SmallVec<[TimerEntry; 4]> = bucket.timers.drain(..).collect();
        drop(bucket);
        for t in cascading {
            self.reinsert(t);
        }
        if slot == lvl.slots.len() - 1 {
            self.cascade_from(lvl_idx + 1);
        }
    }

    fn fire_batch(&self, batch: Vec<TimerEntry>) {
        for t in batch {
            match &t.kind {
                TimerKind::Tick { user_data } => {
                    self.ipc.deliver_hi(t.owner.clone(),
                        TimerEvent { id: t.id, user_data: *user_data }.into_message());
                }
                TimerKind::Dispatch { job, queue } => {
                    self.dispatch.submit(WorkItem {
                        id: self.next_id.fetch_add(1, Ordering::Relaxed),
                        owner: t.owner.clone(),
                        queue: queue.clone(),
                        job: (**job).clone(),
                        callback: CallbackTarget { app: t.owner.clone(), continuation_id: t.id },
                        submitted: Instant::now(),
                        deadline: None,
                        barrier: false,
                    }).ok();
                }
                TimerKind::Internal { task } => self.run_internal(*task),
            }
            // Reschedule if recurring.
            if t.interval_ms > 0 {
                let mut next = t.clone();
                next.deadline_ms += t.interval_ms as u64;
                self.insert(next);
            }
        }
    }
}
```

### 7.4 Leeway semantics

```wit
// wit/vyoma-cpu.wit (continued)

interface time {
    type timer-id = u64;

    /// Set a one-shot timer. `at-ms` is monotonic; `leeway-ms` is the
    /// acceptable slop (the supervisor may fire up to leeway_ms early or
    /// late, choosing whatever firing instant batches best with other timers).
    set-timer: func(
        at-ms: u64,
        leeway-ms: u32,
        user-data: u64
    ) -> result<timer-id, time-error>;

    /// Recurring timer. First fire at `at-ms`, then every `interval-ms`.
    set-recurring: func(
        at-ms: u64,
        interval-ms: u32,
        leeway-ms: u32,
        user-data: u64
    ) -> result<timer-id, time-error>;

    cancel-timer: func(id: timer-id) -> bool;

    /// Monotonic ms since boot.
    monotonic-now: func() -> u64;
}
```

### 7.5 System-wide budget

The wheel maintains a global "wakeups per second" counter. When it exceeds `budget = 1000` (one
wakeup per ms on average across all apps), the wheel **increases coalescing aggressiveness**:

- Apps with leeway ≥ 100 ms get bucketed at 100 ms granularity.
- Apps with leeway ≥ 1 s get bucketed at 500 ms granularity.
- Apps with leeway = 0 (insisted on exact firing) are not coalesced but logged as
  "battery-hungry" and surfaced in Activity Monitor.

---

## 8. CPU Usage Reporting

### 8.1 Two signals: epochs and host CPU time

For each app, the supervisor exports:

```rust
// supervisor/src/scheduler/usage.rs (lives in qos.rs to stay ≤ 500 lines)

#[derive(Debug, Clone, Serialize)]
pub struct UsageSnapshot {
    pub app: AppId,
    pub qos: QosClass,
    pub nap_state: NapState,

    // From /proc/<tid>/stat
    pub host_cpu_pct_1s: f32,
    pub host_cpu_pct_60s: f32,
    pub user_ticks: u64,
    pub system_ticks: u64,

    // From epoch counters
    pub epochs_consumed_1s: u32,
    pub epochs_consumed_60s: u32,
    pub epochs_lifetime: u64,

    // Memory (R2)
    pub rss_bytes: u64,
    pub anon_bytes: u64,
    pub jetsam_score: u32,

    // Dispatch
    pub dispatch_inflight: u32,
    pub dispatch_pending: u32,

    // Timers
    pub active_timers: u32,
    pub wakeups_1s: u32,
}
```

### 8.2 The sampler

A supervisor thread (`usage-sampler`) wakes every 1 s, reads `/proc/<tid>/stat` for each
registered Linux thread, and updates `UsageSnapshot`:

```rust
// supervisor/src/scheduler/usage.rs

pub struct UsageSampler {
    apps: Arc<DashMap<AppId, AppEpochCtx>>,
    threads: Arc<ThreadRegistry>,
    snapshots: Arc<DashMap<AppId, UsageSnapshot>>,
    last_ticks: DashMap<libc::pid_t, u64>,
}

impl UsageSampler {
    pub fn run(self: Arc<Self>) {
        let mut interval = std::time::Instant::now();
        loop {
            interval += Duration::from_secs(1);
            for entry in self.apps.iter() {
                let app = entry.key().clone();
                let ctx = entry.value();
                let tids = self.threads.tids_for(app.clone()).unwrap_or_default();
                let mut user = 0u64;
                let mut sys = 0u64;
                let mut delta_total = 0u64;
                for tid in &tids {
                    if let Some((u, s)) = read_proc_stat(*tid) {
                        user += u;
                        sys += s;
                        let prev = self.last_ticks.get(tid).map(|r| *r).unwrap_or(0);
                        let total = u + s;
                        delta_total += total.saturating_sub(prev);
                        self.last_ticks.insert(*tid, total);
                    }
                }

                let clk_tck = unsafe { libc::sysconf(libc::_SC_CLK_TCK) } as u64;
                let pct = if clk_tck > 0 {
                    (delta_total as f32 / clk_tck as f32) * 100.0
                } else { 0.0 };

                let epochs_1s = ctx.account.window_1s.load(Ordering::Relaxed);

                self.snapshots.entry(app.clone()).and_modify(|s| {
                    s.host_cpu_pct_1s = pct;
                    s.host_cpu_pct_60s = 0.95 * s.host_cpu_pct_60s + 0.05 * pct;
                    s.user_ticks = user;
                    s.system_ticks = sys;
                    s.epochs_consumed_1s = epochs_1s;
                    s.epochs_consumed_60s = (s.epochs_consumed_60s as f32 * 0.95
                        + epochs_1s as f32 * 0.05) as u32;
                }).or_insert_with(|| UsageSnapshot {
                    app: app.clone(),
                    qos: QosClass::from_u8(ctx.qos.load(Ordering::Relaxed)),
                    nap_state: NapState::Awake,
                    host_cpu_pct_1s: pct,
                    host_cpu_pct_60s: pct,
                    user_ticks: user,
                    system_ticks: sys,
                    epochs_consumed_1s: epochs_1s,
                    epochs_consumed_60s: epochs_1s,
                    epochs_lifetime: ctx.account.lifetime.load(Ordering::Relaxed),
                    rss_bytes: 0,
                    anon_bytes: 0,
                    jetsam_score: 0,
                    dispatch_inflight: 0,
                    dispatch_pending: 0,
                    active_timers: 0,
                    wakeups_1s: 0,
                });
            }
            std::thread::sleep(interval.saturating_duration_since(std::time::Instant::now()));
        }
    }
}

fn read_proc_stat(tid: libc::pid_t) -> Option<(u64, u64)> {
    let path = format!("/proc/{}/stat", tid);
    let s = std::fs::read_to_string(path).ok()?;
    // Field 14 = utime, 15 = stime (1-indexed); first ")" delimits comm.
    let rest = s.rsplit_once(')').map(|(_, r)| r)?;
    let parts: Vec<&str> = rest.split_whitespace().collect();
    let utime = parts.get(11)?.parse::<u64>().ok()?;
    let stime = parts.get(12)?.parse::<u64>().ok()?;
    Some((utime, stime))
}
```

### 8.3 Activity Monitor IPC

The Activity Monitor app subscribes to snapshots:

```wit
interface activity-monitor {
    record app-usage {
        app-name:           string,
        qos:                qos-class,
        nap-state:          string,
        cpu-pct:            f32,
        epochs-per-sec:     u32,
        rss-mb:             u32,
        dispatch-inflight:  u32,
        timers-active:      u32,
        wakeups-per-sec:    u32,
    }

    list-usage: func() -> list<app-usage>;

    /// Subscribe to a 1 Hz push stream.
    subscribe: func() -> stream<list<app-usage>>;
}
```

Only apps with the `activity-monitor` capability can call this. By default, only the bundled
Activity Monitor and Console apps have it.

### 8.4 Per-CPU and totals

Total system view:

```rust
pub struct SystemUsage {
    pub host_cpu_pct: f32,
    pub host_user_pct: f32,
    pub host_system_pct: f32,
    pub host_idle_pct: f32,
    pub n_apps_running: u32,
    pub n_apps_napping: u32,
    pub n_apps_suspended: u32,
    pub total_epochs_per_sec: u64,
    pub total_wakeups_per_sec: u32,
    pub thermal_zone: ThermalLevel,
}
```

Read from `/proc/stat` once per second.

---

## 9. Cooperative Yielding

### 9.1 The `vyoma:cpu/yield` API

```wit
interface cpu {
    /// Voluntarily yield the CPU. The supervisor records that this app
    /// preferred to give up its remaining epoch budget for this tick.
    /// Returns immediately; the next epoch interrupt is suppressed.
    yield-now: func();

    /// Yield and request a callback after `at-least-ms` ms. The supervisor
    /// guarantees the app will not be re-entered before then; the actual
    /// wake may be later if other apps are running.
    yield-for: func(at-least-ms: u32);
}
```

### 9.2 Implementation

`yield_now()` sets a per-app flag that the next epoch-deadline callback consults:

```rust
// In epoch deadline callback:
store.epoch_deadline_callback(move |store| {
    let app_ctx = store.data().app_ctx.clone();
    if app_ctx.voluntary_yield.swap(false, Ordering::AcqRel) {
        // Skip this tick — extend deadline by 1 tick to let other apps run.
        let deadline = app_ctx.deadline_ticks.load(Ordering::Relaxed) + 1;
        Ok(UpdateDeadline::Continue(deadline))
    } else {
        let deadline = app_ctx.deadline_ticks.load(Ordering::Relaxed);
        app_ctx.account.window_1s.fetch_add(1, Ordering::Relaxed);
        Ok(UpdateDeadline::Continue(deadline))
    }
});

// vyoma:cpu/yield-now host impl:
fn yield_now(caller: Caller<'_, AppData>) -> Result<()> {
    caller.data().app_ctx.voluntary_yield.store(true, Ordering::Release);
    Ok(())
}
```

`yield_for(at_least_ms)` sets the next epoch deadline to `now + at_least_ms` ticks and returns
normally; the app will not be entered again until the deadline fires.

### 9.3 Anti-abuse: yielding too much

An app yielding constantly burns scheduler overhead. We track yields/sec and if it exceeds 1000,
log a warning. Yielding does not refill the app's epoch budget; an app cannot game its way to
higher CPU by yielding repeatedly.

---

## 10. Realtime Budget for Audio

### 10.1 Why a separate lane

Audio processing has a hard deadline: at 48 kHz with a 256-frame buffer, the app must produce
samples every 5.3 ms or there's an audible glitch. The epoch-deadline approach, even at 1 ms
granularity, has occasional jitter (up to 16 ms) due to CFS load. That's not acceptable.

macOS solves this with `pthread_set_qos_class_self_np(QOS_CLASS_USER_INTERACTIVE)` plus
audio-unit thread time constraints (`thread_policy_set` with `THREAD_TIME_CONSTRAINT_POLICY`).
We use Linux's equivalent: `SCHED_FIFO` with a low priority (1-10 to avoid starving the
kernel).

### 10.2 Audio capability gating

Only apps with `audio = true` in `vyoma.toml` can request the realtime lane. The capability
also exposes the `vyoma:audio` WIT interface (defined in a future audio round). Critically,
the realtime thread is **separate from the main Wasmtime thread**: it runs a *small,
audited* host-side audio mixer that pulls samples from an SPSC ring buffer the WASM app
fills cooperatively.

```
┌────────────────────────────────────────────────────────────┐
│  WASM app's main thread (SCHED_OTHER, nice -10)            │
│  - Runs UI, event loop                                      │
│  - Writes samples into SPSC ring buffer via                 │
│    vyoma:audio/render-callback                              │
└────────────────────────────────────────────────────────────┘
                          │ SPSC ring
                          ▼
┌────────────────────────────────────────────────────────────┐
│  Audio realtime thread (SCHED_FIFO, prio 5)                │
│  - Reads samples from ring at PCM rate                      │
│  - Mixes (multiple apps) and submits to ALSA/PulseAudio    │
│  - If underrun: zero-fill, log, decrement app's audio score│
└────────────────────────────────────────────────────────────┘
```

### 10.3 Pull model with deadline

```rust
// supervisor/src/scheduler/qos.rs (audio_rt_lane fn)

pub fn spawn_audio_rt_lane(
    sink: AudioSink,
    sources: Arc<Mutex<Vec<AudioSource>>>,
) -> JoinHandle<()> {
    std::thread::Builder::new()
        .name("audio-rt".to_string())
        .spawn(move || {
            let tid = unsafe { libc::syscall(libc::SYS_gettid) } as libc::pid_t;
            let _ = apply_qos(tid, QosClass::UserInteractive); // first set nice
            let _ = set_sched_fifo(tid, 5); // then promote to FIFO
            let frame_period = Duration::from_micros(
                (sink.frames_per_buffer as u64 * 1_000_000) / sink.sample_rate as u64
            );
            let mut next = Instant::now();
            loop {
                next += frame_period;
                let mut mixed = vec![0.0f32; sink.frames_per_buffer as usize * sink.channels as usize];
                {
                    let srcs = sources.lock();
                    for src in srcs.iter() {
                        src.pull_into(&mut mixed);
                    }
                }
                if sink.submit(&mixed).is_err() {
                    tracing::error!("audio sink underrun");
                }
                spin_sleep::sleep_until(next);
            }
        }).unwrap()
}

fn set_sched_fifo(tid: libc::pid_t, prio: i32) -> nix::Result<()> {
    let param = libc::sched_param { sched_priority: prio };
    let r = unsafe { libc::sched_setscheduler(tid, libc::SCHED_FIFO, &param) };
    if r != 0 { Err(nix::Error::last()) } else { Ok(()) }
}
```

### 10.4 Underrun policy

Each app has an `audio_underrun_score`. Each underrun caused by that app (detected by an empty
ring buffer at pull time) increments the score. If it exceeds 10 in a 60 s window, the audio
realtime thread is **revoked** for that app (downgraded to SCHED_OTHER + epoch-based delivery,
which means audio will be glitchy but at least the user can close the app).

### 10.5 Why not just give every app SCHED_FIFO

Because SCHED_FIFO at high enough priority can starve the supervisor itself. We restrict to
priority 5 (out of 99), and only apps with audio capability that are *currently emitting
audio*. An app declaring `audio = true` but not actually playing for > 30 s is downgraded
silently.

---

## 11. Implementation File Layout

All files are ≤ 500 lines.

| File                                          | LOC   | Purpose                                       |
|-----------------------------------------------|-------|-----------------------------------------------|
| `supervisor/src/scheduler/mod.rs`             | ~200  | Public re-exports, init, wiring               |
| `supervisor/src/scheduler/qos.rs`             | ~480  | QosClass, deriver, focus observer, audio lane |
| `supervisor/src/scheduler/epoch_controller.rs`| ~450  | Ticker, budget reconciliation, demotion       |
| `supervisor/src/scheduler/app_nap.rs`         | ~380  | NapDetector, suspend/resume, wake routing     |
| `supervisor/src/scheduler/dispatch.rs`        | ~490  | Queues, work items, pool, JobKind execution   |
| `supervisor/src/scheduler/timers.rs`          | ~420  | TimerWheel, coalescing, leeway                |
| `supervisor/src/scheduler/thermal.rs`         | ~250  | Thermal monitoring & throttling               |
| `supervisor/src/scheduler/usage.rs`           | ~310  | UsageSampler, /proc/<tid>/stat parsing        |
| `wit/vyoma-cpu.wit`                           | ~120  | cpu, time interfaces                          |
| `wit/vyoma-dispatch.wit`                      | ~150  | dispatch, dispatch-callbacks interfaces       |

### 11.1 `mod.rs` skeleton

```rust
// supervisor/src/scheduler/mod.rs

pub mod qos;
pub mod epoch_controller;
pub mod app_nap;
pub mod dispatch;
pub mod timers;
pub mod thermal;
pub mod usage;

pub use qos::{QosClass, QosDeriver, FocusObserver};
pub use epoch_controller::EpochController;
pub use app_nap::{NapDetector, NapState};
pub use dispatch::{DispatchPool, DispatchQueue, JobKind, QueueKind};
pub use timers::{TimerWheel, TimerEntry, TimerKind};
pub use thermal::{ThermalMonitor, ThermalLevel};
pub use usage::{UsageSampler, UsageSnapshot, SystemUsage};

pub struct Scheduler {
    pub epoch: Arc<EpochController>,
    pub nap:   Arc<NapDetector>,
    pub dispatch: Arc<DispatchPool>,
    pub timers: Arc<TimerWheel>,
    pub usage: Arc<UsageSampler>,
    pub thermal: Arc<ThermalMonitor>,
    pub qos: Arc<QosDeriver>,
}

impl Scheduler {
    pub fn new(deps: SchedulerDeps) -> anyhow::Result<Arc<Self>> {
        let qos = Arc::new(QosDeriver::new(deps.focus.clone(), deps.caps.clone()));
        let epoch = Arc::new(EpochController::new(qos.clone()));
        let timers = Arc::new(TimerWheel::new(deps.ipc.clone(), deps.dispatch_pool.clone()));
        let dispatch = Arc::new(DispatchPool::new(num_cpus::get() * 2, deps.ipc.clone(), qos.clone()));
        let nap = Arc::new(NapDetector::new(qos.clone(), timers.clone(), deps.ipc.clone(), epoch.clone()));
        let usage = Arc::new(UsageSampler::new(epoch.clone(), deps.threads.clone()));
        let thermal = Arc::new(ThermalMonitor::new(qos.clone(), epoch.clone()));

        Ok(Arc::new(Scheduler { epoch, nap, dispatch, timers, usage, thermal, qos }))
    }

    pub fn start(self: &Arc<Self>) {
        // Spawn long-running threads.
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
        tokio::spawn(async move { nd.run().await; });
    }
}
```

### 11.2 `thermal.rs` outline

```rust
// supervisor/src/scheduler/thermal.rs

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ThermalLevel { Nominal, Fair, Serious, Critical }

pub struct ThermalMonitor {
    qos: Arc<QosDeriver>,
    epoch_ctl: Arc<EpochController>,
    level: AtomicU8,
    zones: Vec<PathBuf>,
}

impl ThermalMonitor {
    pub fn new(qos: Arc<QosDeriver>, epoch_ctl: Arc<EpochController>) -> Self {
        let zones = discover_thermal_zones();
        Self { qos, epoch_ctl, level: AtomicU8::new(0), zones }
    }

    pub fn run(self: Arc<Self>) {
        loop {
            let temp_c = self.peak_temperature();
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

    fn peak_temperature(&self) -> u32 {
        self.zones.iter()
            .filter_map(|p| std::fs::read_to_string(p).ok())
            .filter_map(|s| s.trim().parse::<u32>().ok())
            .map(|millideg| millideg / 1000)
            .max()
            .unwrap_or(50)
    }

    fn apply(&self, lvl: ThermalLevel) {
        match lvl {
            ThermalLevel::Nominal | ThermalLevel::Fair => {
                // no-op or restore
            }
            ThermalLevel::Serious => {
                // Demote all Background/Maintenance to deadline x 2.
                self.epoch_ctl.global_deadline_scale(QosClass::Background, 2);
                self.epoch_ctl.global_deadline_scale(QosClass::Maintenance, 2);
            }
            ThermalLevel::Critical => {
                // Suspend all Background/Maintenance.
                self.epoch_ctl.suspend_all(QosClass::Background);
                self.epoch_ctl.suspend_all(QosClass::Maintenance);
                // Halve epoch budget for Utility too.
                self.epoch_ctl.global_budget_scale(QosClass::Utility, 0.5);
            }
        }
    }
}

fn discover_thermal_zones() -> Vec<PathBuf> {
    let mut zones = Vec::new();
    let dir = match std::fs::read_dir("/sys/class/thermal") { Ok(d) => d, Err(_) => return zones };
    for ent in dir.flatten() {
        let p = ent.path();
        if p.file_name().and_then(|s| s.to_str()).map(|s| s.starts_with("thermal_zone")).unwrap_or(false) {
            let temp_file = p.join("temp");
            if temp_file.is_file() { zones.push(temp_file); }
        }
    }
    zones
}
```

---

## 12. Integration with Rounds 1-4

- **R1 (lifecycle):** `AppStatus::Suspended` is set/cleared by `NapDetector::apply`. The
  `WatchdogActor` is paused while an app is `Suspended`.
- **R1 (epoch):** the `Engine::increment_epoch()` calls now come from `EpochController` only;
  the previous "global 1s ticker" from R1 is replaced.
- **R2 (jetsam):** under `PressureLevel::Critical`, the `MemoryGovernor` calls
  `nap.suspend_all(QosClass::Background)` as a preliminary action before jetsam-killing.
- **R3 (IPC):** dispatch results are delivered to `inbox_hi`; timer events also go to
  `inbox_hi`; ordinary IPC remains on `inbox_lo`. The `WaitForGraph` is consulted before
  marking an app "napable" — if it's the target of any `dispatch_sync`, don't nap.
- **R4 (filesystem):** the `WatcherBackend` calls `nap.wake(app, WakeReason::Watcher)` when a
  watched path changes for a napped app.

---

## 13. Open Questions for the Critic

These are the weaknesses I see in this design and want challenged in Round 5 critique:

1. **Epoch ticker thread is a single point of contention.** A single thread at 1 kHz iterating
   over all apps calling `engine.increment_epoch()` per app each ms. With 100 apps and
   ~50 ns per atomic store, that's ~5 µs per tick = 0.5% of one core. At 1000 apps it's 5%.
   Worse, if `DashMap::iter()` contends with insertion (new app spawning), the ticker stalls.
   Should we use a per-app dedicated timer thread? A single epoch atomic shared across all
   engines? An io_uring timer batch?

2. **Per-app sched policy switches are racy.** Promoting an app from `Background`
   (`SCHED_BATCH`) to `UserInteractive` (`SCHED_OTHER`) on focus gain involves a `setpriority`
   + `sched_setscheduler` round-trip per thread. With multi-window apps having 3-4 threads,
   that's 6-8 syscalls in the focus-event hot path. Focus changes happen on every Cmd-Tab —
   does that mean 80 ms of syscall latency every Cmd-Tab? Should we batch via cgroup CPU
   weights instead?

3. **Dispatch pool resource ownership is fuzzy.** A `JobKind::HttpGet` runs in a supervisor
   worker thread but the network bytes count against *which* memory cap (R2)? Almost certainly
   the owner app's cap, but the supervisor's allocator is doing the allocating. If R2's
   `MemoryGovernor` is per-instance, dispatch jobs need a separate ledger.

4. **`custom-wasm` dispatch is an unbounded fork bomb vector.** An app submits 10 000
   `custom-wasm` jobs each spawning a 64 MB Wasmtime store. Supervisor OOMs. We limit
   `max_inflight` per app, but is per-app fair? An attacker app eats its quota while a
   legitimate app starves. Should we use a global concurrent-instance budget instead?

5. **Timer wheel resolution mismatch with epoch ticker.** Timer wheel level 0 slot is 1 ms;
   epoch ticker is 1 ms. They're separate threads running at the same period — wasteful.
   Should they share a thread? But then a slow timer fire delays epoch ticks. What's the right
   coupling?

6. **App Nap suspension via `SIGSTOP` doesn't reach the supervisor's own per-app threads.**
   The Wasmtime engine thread is owned by the supervisor process and runs the app's bytecode
   — `kill(tid, SIGSTOP)` works on a thread within the same process, but does that actually
   stop the thread, or does it stop the *process* (the whole supervisor)? POSIX says SIGSTOP
   sent to a thread stops the process. So this is **broken**. We need a different
   suspension mechanism: poll a flag in the epoch callback and `park()` the thread? But that
   means the thread holds onto its WASM stack and we can't reclaim memory.

7. **Realtime audio lane has a confused ownership model.** The audio mixer thread is in the
   supervisor address space, pulling from a ring written by a WASM app that runs in another
   thread. If the WASM app's `render-callback` is slow, the ring underflows. We claim "host
   does the mixing," but actual sample generation is in WASM. So the realtime guarantee
   *requires* the WASM thread to also hit deadlines. Have we just moved the problem?

8. **QoS is derived, so apps cannot express intent.** Consider: a calendar app fetching new
   events in the background. The user opened it once today, then unfocused. We classify it
   `Background` after 60 s, throttle the fetch to a crawl, and the user opens it 6 hours later
   to find nothing has synced. The macOS solution is `qos_class_self_set(UTILITY)` plus
   "background app refresh" preference. Our `keep_awake` API is too coarse. Do we need a
   `request_qos_floor(Utility, "syncing events")` API after all?

---

## 14. Summary

This Round 5 proposal lays out a six-layer scheduler that:

- Replaces fuel with **epoch deadlines** keyed to QoS class (1-100 ms preemption granularity).
- Maps **macOS NSQualityOfService** to nice + sched policy + epoch budget.
- Implements **focus-aware decay** with floors for net/audio/progress.
- Provides **App Nap** as a tick-rate divider + optional SIGSTOP suspension (see Open Q 6).
- Reimplements **GCD** as supervisor-mediated typed dispatch with declarative job kinds.
- Coalesces timers with a **hierarchical timing wheel** + leeway.
- Exposes **Activity Monitor**-grade per-app CPU% and epoch counters.
- Reserves a **SCHED_FIFO audio lane** for apps with the audio capability.

The total surface fits in 9 files under the 500-line rule. The hot path adds <100 ns per
epoch tick. Critical gaps acknowledged in Open Questions are the ticker thread fanout, the
SIGSTOP-vs-thread problem, the dispatch fork-bomb, and the "intent" expressiveness of the
QoS class derivation.

Hand-off to the Critic.

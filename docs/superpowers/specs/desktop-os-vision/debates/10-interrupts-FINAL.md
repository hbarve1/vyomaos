# Round 10 FINAL — Interrupt & Exception Handling

**Role:** Synthesis
**Date:** 2026-05-29
**Status:** FINAL — canonical spec
**Resolves:** Architect proposal + 11 blocking Critic issues
**macOS equivalent:** XNU Mach exceptions + BSD signals + IOKit interrupt delivery
+ CoreFoundation runloop — collapsed into one `EventLoop`/`CallbackQueue` pair
inside the VyomaOS supervisor.

---

## 0. Executive Summary

VyomaOS does not ship a kernel. The Linux 5.10 kernel underneath the supervisor
already terminates the entire wall of hardware interrupts (timers, NICs, GPIO,
input devices, USB controllers, MMIO doorbells, IPIs) and exposes them to user
space through file descriptors. The supervisor never sees a hardware vector. It
only consumes synthetic events: `epoll_wait` returns ready file descriptors,
`signalfd` returns signal numbers, the WASM runtime returns traps, and POSIX
signals deliver process-level faults that originated as CPU exceptions
(SIGSEGV, SIGBUS, SIGFPE, SIGILL).

What VyomaOS owns end-to-end is the part above that interface:

1. **Demultiplexing** every event source into a single dispatch fabric.
2. **Routing** each event to exactly one WASM instance via WIT callbacks
   (`on-timer`, `on-ipc`, `on-key`, `on-pointer`, `on-suspend`, `on-resume`,
   `on-audio-render`, `on-terminate`).
3. **Preempting** misbehaved WASM guests deterministically via Wasmtime epoch
   interruption (and an instruction-count cancel hook on wasm3).
4. **Translating** WASM traps and POSIX signals into a stable `CrashKind`
   enum and writing crash records that survive a supervisor restart.
5. **Restarting** failed subsystems via a compile-time `PanicPolicy`
   exhaustive table without taking the supervisor down with them.

This round produces ten Rust files under `supervisor/src/interrupt/`
(each ≤500 LOC per the project rule) and extends the `WasmRuntime` trait
introduced in Round 9. It honors three immovable contracts:

- **No async runtime.** The supervisor must not pull in `tokio` or
  `async-std`. We use mio + native threads + bounded MPSC channels.
- **No allocation at trap time.** Crash records are written from a
  pre-allocated `CrashBuffer` (2 KB per iid) so that a guest OOM
  cannot block a guest panic from being recorded.
- **No host signal-handler logic beyond a self-pipe write.**
  Signal handlers are pure self-pipe primitives. All policy executes
  on the `SignalLoop` thread.

The remainder of the document is the implementation-grade specification a
senior Rust engineer can follow without clarifying questions.

---

## 1. Design Philosophy

### 1.1 The Golden Rule

> **A hardware interrupt becomes a callback. A callback runs to completion.
> A guest gets preempted only by epoch (Wasmtime) or instruction-count
> cancel (wasm3). The supervisor is never directly interrupted; it polls.**

Cooperative cancellation is the only model that keeps the supervisor's
internal data structures consistent. We deliberately reject signal-driven
control flow inside the supervisor process — signals are delivered via
`signalfd` and read like any other file descriptor.

### 1.2 Three Distinct Concepts

VyomaOS draws clean lines among three concepts that older Unix systems
conflate. Confusing them is the single biggest source of correctness bugs
in interrupt handling code; we name them and never re-use the words:

| Concept | What it means in VyomaOS | Where it lives |
|---|---|---|
| **Hardware interrupt** | A vector on the CPU. Handled by Linux. The supervisor never sees one. | Linux kernel |
| **Event** | A file-descriptor ready-notification surfaced by mio. Demultiplexed by an `EventLoop`. | `interrupt/event_loop.rs` |
| **Callback** | A WIT host call into a WASM guest, invoked by a per-app worker thread, drained from the guest's `CallbackQueue`. | `interrupt/callback_queue.rs` + `interrupt/worker.rs` |

Inputs flow strictly in one direction: hardware → kernel → fd → Event →
Callback. Never the reverse.

### 1.3 Why Not Async Rust

Several engineers will reach for `tokio` here. We deliberately do not:

1. **Cold-start size.** `tokio` is ~150 KB of code+rodata on x86_64 musl.
   The supervisor is 697 KB total today; doubling it for one subsystem
   is not acceptable on the `iot-edge` profile (4 MB RAM floor).
2. **Wasmtime is sync.** `Func::call` is a synchronous, native-stack call
   into JIT'd code; we cannot suspend it across `await` points without
   `wasmtime-fiber`, which depends on libucontext and is opt-in.
3. **No.await across a `.lock()`.** The RankedMutex discipline from R8
   (lock order 100–1700) is incompatible with async lock acquisition
   patterns; deadlocks become heisenbugs.
4. **mcu-minimal cannot run async.** wasm3 has no stack-switching support
   and the Cortex-M4 target has no `tokio` build.

We use bare threads + mio + MPSC. Total dependency cost: `mio = "1.0"`
(~50 KB).

### 1.4 Single Source of Truth

Every event source funnels into a worker through one route. There are
no "back doors" — for example, the input subsystem cannot enqueue a
callback directly; it must produce an event on the `InputLoop`, which
calls `CallbackDispatch::enqueue` like every other source.

This is not stylistic. It is the only way to keep the per-iid
`CallbackQueue` ordering well-defined (HPQ then LPQ, FIFO within a
bucket) and the only way to allow `kill_iid` to atomically drain
queued callbacks for a dead guest.

---

## 2. Platform Dispatch Model

### 2.1 Single-Loop vs Multi-Loop

The supervisor must service event sources with wildly different latency
budgets. On `desktop-full` we need audio jitter <4 ms and input latency
<8 ms; simultaneously the `OtaPoll` subsystem may sit idle for hours.
A single thread cannot serve both well, but spawning four threads is
pointless on `mcu-minimal` where there is no SMP.

Therefore VyomaOS uses **platform-gated dispatch**, driven by Cargo
feature flags resolved from the active platform profile.

| Platform | Mode | Loops |
|---|---|---|
| `mcu-minimal` | single-loop | one cooperative loop, SysTick-driven |
| `iot-edge` | single-loop | one `mio::Poll`, normal priority |
| `robotics-rt` | multi-loop | Input(RR60), Timer(FIFO50), Signal(N), General(N) |
| `mobile` | multi-loop | Input(RR60), Timer(FIFO50), Signal(N), General(N) |
| `desktop-full` | multi-loop | Input(RR60), Timer(FIFO50), Signal(N), General(N) |
| `server-headless` | multi-loop | Input(RR60), Timer(FIFO50), Signal(N), General(N) |

Note: `server-headless` is multi-loop even though it has no display. It
runs cron jobs and HTTP servers whose timing requirements still benefit
from a dedicated `TimerLoop`.

### 2.2 EventLoopConfig

```rust
// supervisor/src/interrupt/types.rs

#[derive(Copy, Clone, Debug)]
pub enum SchedPolicy {
    Normal,
    Fifo(u8), // priority 1..=99
    Rr(u8),
    Deadline { runtime_us: u64, deadline_us: u64, period_us: u64 },
}

#[derive(Copy, Clone, Debug)]
pub struct EventLoopConfig {
    pub split_loops:   bool,
    pub input_sched:   SchedPolicy,
    pub timer_sched:   SchedPolicy,
    pub signal_sched:  SchedPolicy,
    pub general_sched: SchedPolicy,
    pub epoch_tick_hz: u32,    // 0 = disabled (mcu-minimal uses cancel hook)
    pub hpq_cap:       usize,  // bounded high-priority queue capacity
    pub lpq_cap:       usize,  // bounded low-priority queue capacity
    pub worker_sched:  WorkerSchedTable,
}

#[derive(Copy, Clone, Debug)]
pub struct WorkerSchedTable {
    pub audio:        SchedPolicy, // SCHED_FIFO 80 on desktop-full/mobile
    pub control_loop: SchedPolicy, // SCHED_FIFO 75 on robotics-rt
    pub ui:           SchedPolicy, // SCHED_OTHER nice -5
    pub background:   SchedPolicy, // SCHED_OTHER nice 5
    pub maintenance:  SchedPolicy, // SCHED_IDLE
}

impl EventLoopConfig {
    pub fn for_profile(p: PlatformProfile) -> Self {
        match p.id() {
            PlatformId::McuMinimal => Self {
                split_loops: false,
                input_sched: SchedPolicy::Normal,
                timer_sched: SchedPolicy::Normal,
                signal_sched: SchedPolicy::Normal,
                general_sched: SchedPolicy::Normal,
                epoch_tick_hz: 0,
                hpq_cap: 16, lpq_cap: 32,
                worker_sched: WorkerSchedTable::cooperative(),
            },
            PlatformId::IotEdge => Self {
                split_loops: false,
                epoch_tick_hz: 100,
                hpq_cap: 32, lpq_cap: 128,
                ..Default::default()
            },
            PlatformId::RoboticsRt => Self {
                split_loops: true,
                input_sched: SchedPolicy::Rr(60),
                timer_sched: SchedPolicy::Fifo(50),
                epoch_tick_hz: 10_000,
                hpq_cap: 64, lpq_cap: 256,
                worker_sched: WorkerSchedTable::rt(),
                ..Default::default()
            },
            PlatformId::Mobile => Self {
                split_loops: true,
                input_sched: SchedPolicy::Rr(60),
                timer_sched: SchedPolicy::Fifo(50),
                epoch_tick_hz: 1_000,
                hpq_cap: 64, lpq_cap: 256,
                ..Default::default()
            },
            PlatformId::DesktopFull => Self {
                split_loops: true,
                input_sched: SchedPolicy::Rr(60),
                timer_sched: SchedPolicy::Fifo(50),
                epoch_tick_hz: 1_000,
                hpq_cap: 64, lpq_cap: 256,
                ..Default::default()
            },
            PlatformId::ServerHeadless => Self {
                split_loops: true,
                input_sched: SchedPolicy::Normal,
                timer_sched: SchedPolicy::Fifo(50),
                epoch_tick_hz: 200,
                hpq_cap: 32, lpq_cap: 256,
                ..Default::default()
            },
        }
    }
}
```

### 2.3 Why Four Loops, Not Two or Eight

We considered:

- **One loop:** cannot meet audio p99 ≤4 ms under input flood.
- **Two loops** (latency + bulk): the same thread cannot do
  SCHED_RR-60 input *and* SCHED_FIFO-50 timer scheduling
  without priority inversion across `epoll_wait`.
- **Eight loops** (one per source): wastes contexts; mio is reentrant
  so we can register many fds on one Poll without contention.

Four is the smallest number that gives each priority class its own
RT scheduling slot and isolates signal handling.

---

## 3. WasmRuntime Trait Extensions

Round 9 introduced `WasmRuntime` as the abstraction over Wasmtime
(desktop/mobile/server/iot/robotics) and wasm3 (mcu-minimal). Round 10
adds three preemption-related methods:

```rust
// supervisor/src/runtime/mod.rs (extension)

pub trait WasmRuntime: Send + Sync {
    // ... existing R9 methods ...

    /// Request the running guest yield ASAP. Implementation-specific:
    /// - Wasmtime: set `EpochDeadline::now()`, guest sees epoch trap at next backedge.
    /// - wasm3: store `Acquire` true in `cancel_flag`; instruction hook polls it.
    fn request_cancel(&self, iid: IidKey, reason: EpochKillReason);

    /// Set the soft-yield epoch slice in milliseconds. Below this slice
    /// length, the EpochTicker tickling thread does not bump the epoch.
    /// Default: 4 ms on multi-loop platforms, 16 ms on iot-edge,
    /// disabled on mcu-minimal.
    fn set_slice_ms(&self, iid: IidKey, ms: u32);

    /// Configure per-instance linear-memory cap (bytes). Enforced at
    /// instantiation and reflected via `wasmtime::Config::max_memory_size`
    /// (Wasmtime) or `m3_SetMaximumStackSize` + a custom growth hook (wasm3).
    fn set_memory_cap(&self, iid: IidKey, bytes: usize);

    /// Drain any in-flight host calls for `iid` and signal trap.
    /// Used by `kill_iid` and by SIGSEGV recovery to ensure the worker's
    /// `Func::call` returns within `kill_timeout_ms`.
    fn force_trap(&self, iid: IidKey, kind: WasmTrap);
}

#[derive(Copy, Clone, Debug)]
pub enum EpochKillReason {
    Watchdog,          // exceeded watchdog_secs from vyoma.toml
    QuotaExceeded,     // ran past CPU quota
    UserKill,          // operator typed `kill <name>` at supervisor console
    PolicyKill,        // PanicPolicy escalated to KillPending
    Shutdown,          // supervisor going down
    OomGuard,          // crossed memory cap
    DeadlineMissed,    // SCHED_DEADLINE overrun (robotics-rt)
}
```

The cap on memory matters because epoch interruption does **not** fire
mid-`memory.copy` or mid-`memory.fill`. A 4-GiB `memory.fill` on a JIT'd
function will block a worker for seconds with no preemption opportunity.
The cap puts a hard upper bound on that delay.

Per-platform memory caps:

| Platform | Per-instance memory cap |
|---|---|
| `mcu-minimal` | 64 KiB |
| `iot-edge` | 256 KiB |
| `robotics-rt` | 512 KiB |
| `mobile` | 64 MiB |
| `desktop-full` | 256 MiB |
| `server-headless` | 512 MiB |

These are upper bounds; individual apps can request smaller via
`vyoma.toml` `[limits].memory_mib`. Apps cannot exceed the platform cap.

---

## 4. The WASM Exception Model

### 4.1 WasmTrap Enum

A WASM guest can fail for sixteen reasons. We enumerate them exhaustively
and never collapse them into "trap":

```rust
// supervisor/src/interrupt/types.rs

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum WasmTrap {
    StackOverflow,           // Wasmtime: Trap::StackOverflow
    MemoryOob,               // out-of-bounds memory access
    HeapMisaligned,          // unaligned atomic
    TableOob,                // out-of-bounds table indirection
    IndirectCallToNull,
    BadSignature,            // call_indirect signature mismatch
    IntegerOverflow,         // i32.div_s INT_MIN / -1
    IntegerDivisionByZero,
    BadConversionToInteger,  // f→i with NaN/Inf/out of range
    UnreachableCodeReached,  // wasm `unreachable`
    Interrupt,               // epoch deadline / cancel flag (cooperative kill)
    AtomicWaitNonShared,
    OutOfFuel,               // fuel metering exhausted (we don't use fuel)
    HostTrap(HostTrapCode),  // returned by a hostcall (e.g., capability denied)
    GuestPanic(u32),         // explicit `proc_exit(N)` with N != 0
    UnknownTrap,             // future Wasmtime variant we don't yet model
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum HostTrapCode {
    CapDenied         = 1,
    QuotaExceeded     = 2,
    Shutdown          = 3,
    SupervisorKilled  = 4,
    BadArgument       = 5,
}
```

`UnknownTrap` exists so that a future Wasmtime upgrade adding a new
`Trap::*` variant we don't pattern-match falls through to a safe path
rather than panicking the worker.

### 4.2 CrashKind Mapping

R1 defined 12 `CrashKind` variants. Round 10 finalizes the mapping
from low-level cause → user-visible crash report:

```rust
// supervisor/src/interrupt/types.rs

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum CrashKind {
    WasmStackOverflow,
    WasmMemoryViolation,
    WasmArithmetic,
    WasmUnreachable,
    WasmIndirect,
    Killed,
    Panic,
    SegFault,
    BusError,
    Aborted,
    OomKill,
    Shutdown,
}

pub fn trap_to_crashkind(t: WasmTrap) -> CrashKind {
    use WasmTrap::*;
    match t {
        StackOverflow                              => CrashKind::WasmStackOverflow,
        MemoryOob | HeapMisaligned                 => CrashKind::WasmMemoryViolation,
        TableOob | IndirectCallToNull | BadSignature
                                                   => CrashKind::WasmIndirect,
        IntegerOverflow | IntegerDivisionByZero
        | BadConversionToInteger                   => CrashKind::WasmArithmetic,
        UnreachableCodeReached                     => CrashKind::WasmUnreachable,
        Interrupt                                  => CrashKind::Killed,
        AtomicWaitNonShared                        => CrashKind::WasmIndirect,
        OutOfFuel                                  => CrashKind::Killed,
        HostTrap(HostTrapCode::QuotaExceeded)      => CrashKind::OomKill,
        HostTrap(HostTrapCode::Shutdown)           => CrashKind::Shutdown,
        HostTrap(HostTrapCode::CapDenied)          => CrashKind::Panic,
        HostTrap(HostTrapCode::SupervisorKilled)   => CrashKind::Killed,
        HostTrap(HostTrapCode::BadArgument)        => CrashKind::Panic,
        GuestPanic(_)                              => CrashKind::Panic,
        UnknownTrap                                => CrashKind::Panic,
    }
}

pub fn signal_to_crashkind(sig: i32) -> CrashKind {
    match sig {
        libc::SIGSEGV => CrashKind::SegFault,
        libc::SIGBUS  => CrashKind::BusError,
        libc::SIGFPE  => CrashKind::WasmArithmetic,
        libc::SIGILL  => CrashKind::Aborted,
        libc::SIGKILL => CrashKind::Killed,
        libc::SIGABRT => CrashKind::Aborted,
        _             => CrashKind::Aborted,
    }
}
```

### 4.3 IidState — Atomic State Machine

The architect proposed a single `AtomicBool yield_requested`. The critic
correctly observed this race-conditions with the worker's read path.
The replacement uses a 5-state CAS machine stored in an `AtomicU8`:

```rust
// supervisor/src/interrupt/types.rs

#[repr(u8)]
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum IidState {
    Running      = 0,
    YieldPending = 1, // EpochTicker requested soft yield; guest has not yet observed it
    Yielded      = 2, // guest hit epoch backedge and trapped; worker has the trap
    KillPending  = 3, // hard kill requested (watchdog/policy/operator); skip yield
    Dead         = 4, // worker confirmed kill; iid is gone
}

pub struct IidEpochState {
    pub state:          AtomicU8,   // IidState
    pub deadline_epoch: AtomicU64,  // wasmtime epoch deadline
    pub cancel_flag:    AtomicBool, // wasm3 instruction-hook polls this
    pub reason:         AtomicU8,   // EpochKillReason
}

impl IidEpochState {
    pub fn try_request_yield(&self, reason: EpochKillReason) -> bool {
        self.reason.store(reason as u8, Release);
        self.state
            .compare_exchange(
                IidState::Running as u8,
                IidState::YieldPending as u8,
                AcqRel, Acquire,
            )
            .is_ok()
    }

    pub fn request_kill(&self, reason: EpochKillReason) -> bool {
        self.reason.store(reason as u8, Release);
        // Transition from any non-Dead state → KillPending.
        let mut cur = self.state.load(Acquire);
        loop {
            if cur == IidState::Dead as u8 {
                return false;
            }
            match self.state.compare_exchange_weak(
                cur, IidState::KillPending as u8, AcqRel, Acquire,
            ) {
                Ok(_)  => { self.cancel_flag.store(true, Release); return true; }
                Err(e) => cur = e,
            }
        }
    }

    pub fn observe_yield(&self) {
        let _ = self.state.compare_exchange(
            IidState::YieldPending as u8,
            IidState::Yielded as u8,
            AcqRel, Acquire,
        );
    }

    pub fn mark_dead(&self) {
        self.state.store(IidState::Dead as u8, Release);
    }
}
```

Transitions form a directed graph:

```
            try_request_yield
   Running ─────────────────────► YieldPending
      │                                │
      │ request_kill                   │ observe_yield
      ▼                                ▼
  KillPending                       Yielded
      │                                │
      │ mark_dead          mark_dead   │
      └───────────────┬────────────────┘
                      ▼
                    Dead
```

`Dead` is terminal. A new iid created from a restart gets a fresh
`IidEpochState`.

---

## 5. EventLoop(s) Implementation

### 5.1 Single-Threaded Variant (mcu-minimal, iot-edge)

`iot-edge` uses one `mio::Poll`; `mcu-minimal` uses a cooperative
`while { poll_all(); dispatch(); }` driven by SysTick (no Linux there).

```rust
// supervisor/src/interrupt/event_loop.rs (single-loop path)

pub struct SingleLoop {
    poll:      mio::Poll,
    events:    mio::Events,
    registry:  Arc<FdRegistry>,
    dispatch:  Arc<CallbackDispatch>,
}

impl SingleLoop {
    pub fn run(&mut self, stop: Arc<AtomicBool>) {
        while !stop.load(Acquire) {
            // 100 ms timeout so we periodically check `stop` and drain control
            // self-pipes that don't get readable for long stretches.
            let _ = self.poll.poll(&mut self.events, Some(Duration::from_millis(100)));
            for ev in self.events.iter() {
                let src = self.registry.lookup(ev.token());
                self.handle_event(src, ev);
            }
        }
    }
}
```

### 5.2 Multi-Threaded Variant (robotics-rt, mobile, desktop-full, server-headless)

Four loops; each is a `SingleLoop` over its own `mio::Poll`, dedicated
to one event class. Loops do not share file descriptors — each loop's
`Poll` only knows the fds registered to it. Loops share the per-iid
`CallbackQueue` set through the `CallbackDispatch` `Arc`.

```rust
// supervisor/src/interrupt/event_loop.rs (multi-loop path)

pub struct MultiLoop {
    pub input:   SingleLoop,
    pub timer:   SingleLoop,
    pub signal:  SingleLoop,
    pub general: SingleLoop,
}

impl MultiLoop {
    pub fn spawn(self, cfg: EventLoopConfig, stop: Arc<AtomicBool>) -> MultiLoopHandle {
        let h_input   = spawn_loop("vyoma-input",   cfg.input_sched,   self.input,   stop.clone());
        let h_timer   = spawn_loop("vyoma-timer",   cfg.timer_sched,   self.timer,   stop.clone());
        let h_signal  = spawn_loop("vyoma-signal",  cfg.signal_sched,  self.signal,  stop.clone());
        let h_general = spawn_loop("vyoma-general", cfg.general_sched, self.general, stop.clone());
        MultiLoopHandle { h_input, h_timer, h_signal, h_general }
    }
}

fn spawn_loop(name: &'static str, sched: SchedPolicy, mut lp: SingleLoop, stop: Arc<AtomicBool>)
    -> JoinHandle<()>
{
    thread::Builder::new()
        .name(name.into())
        .spawn(move || {
            apply_sched_policy(sched).expect("sched_setscheduler");
            // Each loop runs inside `run_subsystem`; panic policy decides
            // whether to escalate or restart.
            let sub = match name {
                "vyoma-input"   => Subsystem::InputLoop,
                "vyoma-timer"   => Subsystem::TimerLoop,
                "vyoma-signal"  => Subsystem::SignalLoop,
                "vyoma-general" => Subsystem::GeneralLoop,
                _ => unreachable!(),
            };
            run_subsystem(sub, || lp.run(stop.clone()))
        })
        .expect("event loop thread")
}
```

### 5.3 Event Sources Registered to Each Loop

| Loop | Registered fds |
|---|---|
| `InputLoop` | `/dev/input/event*` (evdev), libinput sock, virtio-input ring |
| `TimerLoop` | All `timerfd` instances from `TimerManager`, ALSA `pollfd[]`, V4L2 capture buffers ready-fd |
| `SignalLoop` | `signalfd` (TERM/INT/PIPE/USR1/USR2/CHLD/HUP), SIGSEGV self-pipe reader, SIGBUS self-pipe reader |
| `GeneralLoop` | IPC UDS listener fd, per-iid IPC sockets, udev netlink fd, thermal sysfs `pollfd`, OTA timer fd, `inotify` fd for `/data/crashes/raw`, supervisor REPL fd |

### 5.4 SCHED Policy Application

```rust
// supervisor/src/interrupt/event_loop.rs

fn apply_sched_policy(p: SchedPolicy) -> io::Result<()> {
    unsafe {
        let pid = 0; // current thread
        let mut param: libc::sched_param = mem::zeroed();
        let (policy, prio) = match p {
            SchedPolicy::Normal       => (libc::SCHED_OTHER, 0),
            SchedPolicy::Fifo(prio)   => (libc::SCHED_FIFO,  prio as i32),
            SchedPolicy::Rr(prio)     => (libc::SCHED_RR,    prio as i32),
            SchedPolicy::Deadline { .. } => {
                // sched_setattr path; uses raw syscall on x86_64/aarch64
                return apply_sched_deadline(p);
            }
        };
        param.sched_priority = prio;
        if libc::sched_setscheduler(pid, policy, &param) < 0 {
            // RT priorities require CAP_SYS_NICE; stage1 grants this to PID-2.
            return Err(io::Error::last_os_error());
        }
    }
    Ok(())
}
```

### 5.5 Dispatch Path Inside the Loop

```rust
fn handle_event(&self, src: EventSource, ev: &mio::event::Event) {
    match src {
        EventSource::Timerfd { iid, timer_id } => {
            // Drain the timerfd; one read = N expirations.
            let n = read_u64(timer_id.fd);
            let cb = Callback::timer(iid, timer_id, n);
            self.dispatch.enqueue(iid, Priority::Low, cb);
        }
        EventSource::Evdev { node } => {
            let (iid, events) = self.input.decode_into_events(node);
            for ev in coalesce(events) {
                let cb = Callback::input(iid, ev);
                let prio = if ev.is_critical() { Priority::High } else { Priority::Low };
                self.dispatch.enqueue(iid, prio, cb);
            }
        }
        EventSource::Signalfd => self.signal.drain_signalfd(&self.dispatch),
        EventSource::SegvSelfPipe => self.signal.handle_segv_event(&self.dispatch),
        EventSource::IpcSocket { iid } => self.ipc.deliver(iid, &self.dispatch),
        EventSource::Udev => self.udev.process(&self.dispatch),
        // ...
    }
}
```

The handler runs entirely on the loop thread; it must never block.
If a sink (e.g., IPC delivery) needs to block on a `RankedMutex`,
the handler enqueues a `Callback::ipc_pending` and the receiving worker
performs the lock acquisition.

---

## 6. CallbackQueue — Two-Queue Model

### 6.1 HPQ + LPQ Per iid

The architect's single ring buffer with eviction is unsafe: a flood of
mouse moves can evict the watchdog tick, the audio render, or the
`Terminate` callback that's trying to shut the app down. We replace it
with a per-iid `HighPriorityQueue` + `LowPriorityQueue` pair:

```rust
// supervisor/src/interrupt/callback_queue.rs

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Priority {
    High, // watchdog, audio-render, control-loop timer, Terminate, Supervisor.signal
    Low,  // general timers, IPC, input (except critical), suspend/resume
}

pub struct CallbackQueue {
    hpq: ArrayQueue<Callback>, // never evicted; producer blocks on full (rare/fatal)
    lpq: ArrayQueue<Callback>, // evict oldest on overflow
    drop_count: AtomicU64,     // monotonic counter of dropped LPQ items
}

impl CallbackQueue {
    pub fn new(cfg: &EventLoopConfig) -> Self {
        Self {
            hpq: ArrayQueue::new(cfg.hpq_cap),
            lpq: ArrayQueue::new(cfg.lpq_cap),
            drop_count: AtomicU64::new(0),
        }
    }

    pub fn enqueue(&self, prio: Priority, cb: Callback) -> Result<(), EnqueueErr> {
        match prio {
            Priority::High => {
                // Block the producer if HPQ full. HPQ full means the worker is
                // wedged; we want to surface that immediately, not silently lose
                // a Terminate. The producer is an EventLoop thread; blocking it
                // for >50 ms triggers a watchdog escalation on that subsystem.
                self.hpq.push(cb).map_err(|_| EnqueueErr::HpqFull)
            }
            Priority::Low => {
                if let Err(rejected) = self.lpq.push(cb) {
                    // Evict oldest LPQ entry and retry once.
                    let _ = self.lpq.pop();
                    self.drop_count.fetch_add(1, Relaxed);
                    self.lpq.push(rejected).map_err(|_| EnqueueErr::LpqFull)
                } else {
                    Ok(())
                }
            }
        }
    }

    /// Worker drain — HPQ first, then LPQ. Returns None if both empty.
    pub fn pop(&self) -> Option<Callback> {
        self.hpq.pop().or_else(|| self.lpq.pop())
    }
}

#[derive(Debug)]
pub enum EnqueueErr {
    HpqFull,   // worker stuck; escalate to KillPending after watchdog
    LpqFull,   // shouldn't happen given the eviction retry; treat as drop
    DeadIid,
}
```

`ArrayQueue` is from `crossbeam-queue 0.3`. The supervisor already
depends on `crossbeam-channel`; adding `crossbeam-queue` costs <10 KB.

### 6.2 Callback Variants

```rust
#[derive(Debug)]
pub enum Callback {
    Timer  { iid: IidKey, timer_id: TimerId, expirations: u64, deadline_ns: u64 },
    Ipc    { iid: IidKey, from: IidKey, envelope_id: u64 },
    Input  { iid: IidKey, ev: InputEvent },
    Audio  { iid: IidKey, frames_ready: u32, mono_ns: u64 },
    Suspend{ iid: IidKey, reason: SuspendReason },
    Resume { iid: IidKey, reason: ResumeReason },
    HalEvent { iid: IidKey, bus: HalBus, ev: HalEvent }, // from R9
    Terminate { iid: IidKey, reason: TerminateReason },
    Watchdog { iid: IidKey },                            // internal; queued only on HPQ
}

impl Callback {
    pub fn iid(&self) -> IidKey { match self { Callback::Timer{iid,..} | ... => *iid } }
}
```

### 6.3 CallbackDispatch — Lock-Free Routing

```rust
// supervisor/src/interrupt/callback_queue.rs

pub struct CallbackDispatch {
    // RwLock here gates *creation* of new iid queues; the hot path is `read()`
    // followed by `Arc::clone` + atomic `enqueue`.
    queues: RwLock<HashMap<IidKey, Arc<CallbackQueue>>>,
}

impl CallbackDispatch {
    pub fn enqueue(&self, iid: IidKey, prio: Priority, cb: Callback) {
        let queue = {
            let map = self.queues.read();
            match map.get(&iid) {
                Some(q) => q.clone(),
                None => return, // iid dead → silently drop
            }
        };
        match queue.enqueue(prio, cb) {
            Ok(())            => {}
            Err(EnqueueErr::HpqFull) => {
                // Worker is stuck. Escalate.
                metrics::HPQ_FULL.inc(iid);
                supervisor::watchdog::on_hpq_full(iid);
            }
            Err(_) => {} // LPQ drops are tracked via drop_count, no event
        }
    }

    pub fn register_iid(&self, iid: IidKey, cfg: &EventLoopConfig) {
        self.queues.write().insert(iid, Arc::new(CallbackQueue::new(cfg)));
    }

    pub fn unregister_iid(&self, iid: IidKey) {
        self.queues.write().remove(&iid);
    }
}
```

### 6.4 Coalescing — Done at the Producer Side

Coalescing happens before enqueue, on the `InputLoop` thread, so that
the queue's invariants stay simple:

- **Mouse move:** collapse a run of consecutive `MouseMove(x,y)`
  events for the same iid into a single event with the latest
  position. (Implementation: a 1-deep slot in the InputLoop;
  flush on a different-event-type boundary or on every 16 ms tick.)
- **Key repeat:** evdev emits autorepeat at ~30 Hz; collapse N
  repeats of the same keycode into one `on-key-repeat(count: N)`
  callback. (Implementation: maintain `(iid, last_keycode, count)`
  in the InputLoop; flush on key-release, focus change, or 16 ms.)
- **Scroll wheel:** sum vertical and horizontal deltas; emit one
  combined callback per 16 ms.

Coalescing is bounded: at most 16 ms of delay. Critical input
(modifier-key press/release, mouse button press/release, key-up)
is never coalesced; it always emits immediately, on HPQ if the focus
target has set the `Priority::High` input flag.

### 6.5 Capacity Table by Platform

| Platform | HPQ cap | LPQ cap | Rationale |
|---|---|---|---|
| `mcu-minimal` | 16 | 32 | Tiny RAM; only a handful of timers/IPC in flight |
| `iot-edge` | 32 | 128 | Modest I/O; some sensor bursts |
| `robotics-rt` | 64 | 256 | High-rate sensor data and control-loop timers |
| `mobile` | 64 | 256 | UI + sensors + audio |
| `desktop-full` | 64 | 256 | UI + audio + many windows |
| `server-headless` | 32 | 256 | Network bursts dominate |

---

## 7. Epoch Interruption

### 7.1 The Honest Model

Wasmtime epoch interruption is **cooperative preemption at function
entries and loop backedges only**. The compiler inserts an epoch check
at every backedge; on a slice tick, the running guest will yield within
a few thousand instructions — *if it is executing user code*.

Three gaps exist:

1. **Bulk memory ops.** `memory.copy`, `memory.fill`, `memory.init`
   are single instructions in the WASM spec, decoded into native loops
   inside Wasmtime. No epoch check fires mid-instruction. A 256 MiB
   `memory.fill` on `desktop-full` takes ~50 ms on a modern CPU.
   **Mitigation:** per-instance memory cap (Section 3) bounds the worst
   case. On `mobile` (64 MiB cap), worst-case `memory.fill` is ~12 ms,
   within our audio jitter budget if we deprioritize that worker.

2. **Hostcalls.** A WIT hostcall (`vyoma:fs.read`,
   `vyoma:net.send`, etc.) runs supervisor code on the worker thread.
   Wasmtime epoch checks do not interrupt host code. **Mitigation:** all
   potentially-blocking hostcalls follow the **async-friendly hostcall
   discipline**: the host code submits work to a queue and returns
   immediately with `Err(Pending)`. The guest re-tries via `on-timer`
   or `on-ipc` resume. The complete list of blocking hostcalls and their
   pending IDs is documented in `wit/blocking-hostcalls.md`.

3. **wasm3 (mcu-minimal).** Has no epoch facility. **Mitigation:**
   `WasmRuntime` on wasm3 sets `AtomicBool cancel_flag`; wasm3's
   instruction hook (registered via `m3_RegisterInstructionHook`)
   reads the flag every 1000 instructions (~6 µs at 168 MHz Cortex-M4)
   and longjmps to a trampoline that returns `WasmTrap::Interrupt`.

### 7.2 EpochTicker Implementation

```rust
// supervisor/src/interrupt/epoch.rs

pub struct EpochTicker {
    engine:    wasmtime::Engine,
    tick_hz:   u32,
    iid_states: Arc<DashMap<IidKey, Arc<IidEpochState>>>,
    stop:      Arc<AtomicBool>,
    thread:    Option<JoinHandle<()>>,
}

impl EpochTicker {
    pub fn start(engine: wasmtime::Engine, tick_hz: u32) -> Self {
        let iid_states = Arc::new(DashMap::new());
        let stop = Arc::new(AtomicBool::new(false));

        let thread = {
            let engine = engine.clone();
            let iid_states = iid_states.clone();
            let stop = stop.clone();
            thread::Builder::new()
                .name("vyoma-epoch-ticker".into())
                .spawn(move || run_ticker(engine, tick_hz, iid_states, stop))
                .expect("epoch ticker thread")
        };

        Self { engine, tick_hz, iid_states, stop, thread: Some(thread) }
    }
}

fn run_ticker(
    engine: wasmtime::Engine,
    hz: u32,
    iid_states: Arc<DashMap<IidKey, Arc<IidEpochState>>>,
    stop: Arc<AtomicBool>,
) {
    let period = Duration::from_nanos(1_000_000_000 / hz as u64);
    let mut next = Instant::now() + period;
    while !stop.load(Acquire) {
        // Sleep until the next tick boundary; absolute timing prevents drift.
        let now = Instant::now();
        if now < next {
            thread::sleep(next - now);
        }
        next += period;

        // Bump the global engine epoch by 1.
        engine.increment_epoch();

        // For every iid that's running with a deadline at or before this epoch,
        // record YieldPending so the worker can disambiguate yield from kill.
        let cur_epoch = engine.current_epoch();
        for kv in iid_states.iter() {
            let state = kv.value();
            if state.deadline_epoch.load(Acquire) <= cur_epoch {
                state.try_request_yield(EpochKillReason::Watchdog);
            }
        }
    }
}
```

### 7.3 Slice Configuration

A "slice" is how many epoch ticks a callback gets before yield is
requested. Per platform:

| Platform | Tick rate | Default slice ticks | Default slice ms |
|---|---|---|---|
| `mcu-minimal` | N/A (cancel hook) | N/A | N/A |
| `iot-edge` | 100 Hz | 2 | 20 ms |
| `robotics-rt` | 10 kHz | 5 | 0.5 ms (control loops) |
| `mobile` | 1 kHz | 4 | 4 ms (audio-friendly) |
| `desktop-full` | 1 kHz | 4 | 4 ms |
| `server-headless` | 200 Hz | 20 | 100 ms (batch) |

Per-iid slice can be overridden via `vyoma.toml`:

```toml
[scheduling]
slice_ms = 1     # robotics control loop
```

### 7.4 Hard Kill Path

`request_cancel(iid, EpochKillReason::PolicyKill)`:

1. `IidEpochState::request_kill` CAS to `KillPending`.
2. Set `cancel_flag` (wasm3) and `deadline_epoch = current` (Wasmtime).
3. Wait `kill_timeout_ms` (default 200 ms; 50 ms on robotics-rt).
4. If worker has not exited the `Func::call` by then, `WasmRuntime::force_trap`
   detaches the Wasmtime store and the worker observes `WasmTrap::Interrupt`.
5. If the worker thread itself is stuck (e.g., infinite hostcall),
   `pthread_kill(worker, SIGUSR2)`. The worker's `SIGUSR2` handler is a
   `longjmp` back into the worker's main loop with the kill recorded.

The `pthread_kill` path is last-resort: it leaks any Wasmtime store
state. The worker is marked `Dead`; PanicPolicy decides whether
to restart the iid.

### 7.5 Why Not Fuel Metering

Wasmtime supports `Config::consume_fuel(true)` for instruction-count
quotas. We deliberately don't enable it:

- Doubles JIT'd code size (a fuel-check at every backedge AND opcode).
- Epoch interruption already gives us preemption at the same granularity.
- Fuel is a quota, not a guarantee; rebuilding the fuel pool is racy
  with multi-threaded workers.

If future work needs CPU quotas, R5 cgroup `cpu.max` is the answer.

---

## 8. Signal Handling

### 8.1 Three Categories

| Category | Signals | Delivery method |
|---|---|---|
| **Async control** | SIGTERM, SIGINT, SIGHUP, SIGUSR1, SIGUSR2, SIGPIPE, SIGCHLD | `signalfd`, polled by `SignalLoop` |
| **Synchronous fault** | SIGSEGV, SIGBUS, SIGFPE, SIGILL, SIGABRT | minimal `sa_handler` (writes to self-pipe) |
| **Lifecycle** | SIGQUIT, SIGTSTP, SIGCONT | `signalfd` |

### 8.2 Critical Install Order

`Engine::new()` installs Wasmtime's own SIGSEGV handler that translates
`mmap`-protected guard-page faults inside JIT'd code into a clean
`Trap::MemoryOutOfBounds`. If our supervisor installs a SIGSEGV handler
*after* `Engine::new()`, we either overwrite Wasmtime's handler (causing
JIT faults to crash the supervisor) or chain to it incorrectly.

The correct order, enforced in `interrupt/signal.rs`:

```rust
// supervisor/src/interrupt/signal.rs

/// MUST be called BEFORE `Engine::new()`. Installs a minimal sa_handler that
/// writes the signal number to a self-pipe. Does NOT chain via oldact; Wasmtime
/// will register a *host signal handler* via `Config::with_host_signal_handler`
/// in step 2, which gets called BEFORE Wasmtime's WASM-bounds interpretation.
pub fn install_pre_engine_handlers(self_pipe_w: RawFd) {
    PRE_ENGINE_SELF_PIPE.store(self_pipe_w, Release);
    unsafe {
        let mut sa: libc::sigaction = mem::zeroed();
        sa.sa_sigaction = pre_engine_sigsegv as usize;
        sa.sa_flags = libc::SA_SIGINFO | libc::SA_RESETHAND | libc::SA_ONSTACK;
        libc::sigemptyset(&mut sa.sa_mask);
        for sig in [libc::SIGSEGV, libc::SIGBUS, libc::SIGFPE, libc::SIGILL] {
            // Use sigaltstack to survive stack-overflow on the main thread.
            libc::sigaction(sig, &sa, std::ptr::null_mut());
        }
    }
}

extern "C" fn pre_engine_sigsegv(sig: i32, info: *mut libc::siginfo_t, _ctx: *mut libc::c_void) {
    // ASYNC-SIGNAL-SAFE ONLY. No heap, no locking, no Rust panics.
    let fd = PRE_ENGINE_SELF_PIPE.load(Acquire);
    let buf = [sig as u8];
    unsafe { libc::write(fd, buf.as_ptr() as *const _, 1); }
    // SA_RESETHAND restored the default handler; if we return without exiting,
    // Linux will re-raise the signal and kill the process. That's intentional:
    // if this fires in supervisor code (not WASM), we want the process to die
    // and stage1 to restart us cleanly.
}

/// Step 2: after Engine::new(), wire Wasmtime's host signal handler.
pub fn install_wasmtime_host_handler(config: &mut wasmtime::Config) {
    unsafe {
        config.with_host_signal_handler(Box::new(|sig, info, ctx| {
            // Wasmtime calls THIS before its own WASM-bounds interpretation.
            // We forward to the supervisor's self-pipe pathway, then return
            // false to let Wasmtime handle WASM JIT bounds.
            let fd = PRE_ENGINE_SELF_PIPE.load(Acquire);
            let buf = [sig as u8];
            libc::write(fd, buf.as_ptr() as *const _, 1);
            false // not handled; let Wasmtime continue
        }));
    }
}
```

The `SA_RESETHAND` flag automatically restores the default handler on
entry, eliminating recursive signal-in-signal hazards. When stage1
respawns PID-2, it re-installs the handler in `install_pre_engine_handlers`.

### 8.3 signalfd Setup

```rust
// supervisor/src/interrupt/signal.rs

pub fn install_signalfd() -> io::Result<OwnedFd> {
    let mut mask: libc::sigset_t = unsafe { mem::zeroed() };
    unsafe { libc::sigemptyset(&mut mask); }
    for sig in [
        libc::SIGTERM, libc::SIGINT, libc::SIGHUP,
        libc::SIGUSR1, libc::SIGUSR2,
        libc::SIGPIPE, libc::SIGCHLD,
        libc::SIGQUIT,
    ] {
        unsafe { libc::sigaddset(&mut mask, sig); }
    }
    // Block these for the whole process; signalfd will deliver them.
    unsafe { libc::pthread_sigmask(libc::SIG_BLOCK, &mask, std::ptr::null_mut()); }
    let fd = unsafe {
        libc::signalfd(-1, &mask, libc::SFD_NONBLOCK | libc::SFD_CLOEXEC)
    };
    if fd < 0 { return Err(io::Error::last_os_error()); }
    Ok(unsafe { OwnedFd::from_raw_fd(fd) })
}
```

### 8.4 Signal Handlers in SignalLoop

```rust
impl SignalLoopHandler {
    fn drain_signalfd(&self, dispatch: &CallbackDispatch) {
        let mut info: libc::signalfd_siginfo = unsafe { mem::zeroed() };
        loop {
            let n = unsafe {
                libc::read(self.signalfd.as_raw_fd(),
                           &mut info as *mut _ as *mut _,
                           mem::size_of_val(&info))
            };
            if n <= 0 { break; }
            match info.ssi_signo as i32 {
                libc::SIGTERM | libc::SIGINT | libc::SIGQUIT => {
                    self.shutdown.store(true, Release);
                    // Enqueue Terminate to every running iid on HPQ.
                    for iid in self.app_table.live_iids() {
                        dispatch.enqueue(iid, Priority::High,
                            Callback::Terminate { iid, reason: TerminateReason::ShutDown });
                    }
                }
                libc::SIGHUP => self.config_reload.notify(),
                libc::SIGUSR1 => self.metrics_dump.notify(),
                libc::SIGUSR2 => self.log_rotate.notify(),
                libc::SIGPIPE => {} // ignored; per-fd EPIPE is sufficient
                libc::SIGCHLD => self.reap_zombies(),
                _ => log::warn!("unhandled signal: {}", info.ssi_signo),
            }
        }
    }

    fn handle_segv_event(&self, dispatch: &CallbackDispatch) {
        // The self-pipe carries the signal number; we already know it's SIGSEGV
        // or SIGBUS that came from inside a worker. Look up which worker
        // by examining the WasmRuntime's pending-trap state.
        let mut buf = [0u8; 8];
        let _ = nix::unistd::read(self.segv_pipe_r, &mut buf);
        for sig in &buf[..] {
            if *sig == 0 { continue; }
            // For each iid currently in a host signal-handler context,
            // request the WasmRuntime to surface the trap to the worker.
            self.runtime.surface_pending_signal_traps(*sig as i32);
        }
    }

    fn reap_zombies(&self) {
        loop {
            let mut status: i32 = 0;
            let pid = unsafe { libc::waitpid(-1, &mut status, libc::WNOHANG) };
            if pid <= 0 { break; }
            // We rarely have child processes; only the crash-reporter and
            // some driver-shim apps are children. Reap them so they don't
            // become zombies; mark them for respawn if needed.
            self.child_table.on_exit(pid, status);
        }
    }
}
```

### 8.5 SIGCHLD and Per-Process Children

The supervisor runs all apps as in-process WASM. The only OS-level child
processes are:

1. `crash-reporter` WASM app — but it's also in-process. So the only
   actual children are external driver shims (R6 Tier-3) and the
   stage1 → PID-2 relationship (handled by stage1, not the supervisor).
2. Future: optional `runc`-style sandbox for native helper tools.

So SIGCHLD is rare. We still handle it correctly to avoid PID exhaustion
in case a driver shim crashes.

### 8.6 Re-Entry Guard for Signal Handlers

`SA_RESETHAND` is the primary guard. As a defense-in-depth, the
self-pipe write path uses a thread-local counter:

```rust
thread_local! {
    static IN_SIGNAL_HANDLER: Cell<u32> = const { Cell::new(0) };
}
```

`pre_engine_sigsegv` is `extern "C"`; we don't access TLS from it
(TLS access from a signal handler is technically unsafe). Instead, the
counter is for the signal-loop dispatch code that runs *after* the
self-pipe wakeup. If we ever re-enter that path, we log to a static
ring buffer and skip the offending handler.

---

## 9. TimerManager

### 9.1 Per-Platform Backend

| Platform | Backend | Resolution |
|---|---|---|
| `mcu-minimal` | SysTick + per-iid sorted heap | 1 ms |
| All Linux platforms | `timerfd_create(CLOCK_MONOTONIC)` | 1 µs (kernel hrtimer) |

### 9.2 Linux Implementation

```rust
// supervisor/src/interrupt/timer.rs

pub struct TimerManager {
    inner: RankedMutex<TimerInner, RankLevel::TIMER_MGR>, // rank 800
    cfg:   EventLoopConfig,
}

struct TimerInner {
    by_iid:    HashMap<IidKey, Vec<Timer>>,
    by_fd:     HashMap<RawFd, (IidKey, TimerId)>,
    next_id:   u64,
}

pub struct Timer {
    id:         TimerId,
    fd:         OwnedFd,
    period_ns:  u64, // 0 = one-shot
    deadline_ns: u64,
    cb_priority: Priority,
}

impl TimerManager {
    pub fn create(&self, iid: IidKey, period: Duration, prio: Priority)
        -> Result<TimerId, TimerErr>
    {
        let mut g = self.inner.lock();
        if g.by_iid.entry(iid).or_default().len() >= TIMER_PER_IID_QUOTA {
            return Err(TimerErr::QuotaExceeded);
        }
        let fd = unsafe {
            libc::timerfd_create(libc::CLOCK_MONOTONIC, libc::TFD_NONBLOCK | libc::TFD_CLOEXEC)
        };
        if fd < 0 { return Err(TimerErr::Io(io::Error::last_os_error())); }
        let owned = unsafe { OwnedFd::from_raw_fd(fd) };
        let id = TimerId(g.next_id); g.next_id += 1;
        Self::set_timerfd(owned.as_raw_fd(), period, period)?;
        let timer = Timer {
            id,
            fd: owned,
            period_ns: period.as_nanos() as u64,
            deadline_ns: now_mono_ns() + period.as_nanos() as u64,
            cb_priority: prio,
        };
        g.by_fd.insert(timer.fd.as_raw_fd(), (iid, id));
        // Register with TimerLoop's mio::Poll.
        self.timer_loop.register(&timer.fd);
        g.by_iid.get_mut(&iid).unwrap().push(timer);
        Ok(id)
    }

    pub fn cancel(&self, iid: IidKey, id: TimerId) {
        let mut g = self.inner.lock();
        if let Some(timers) = g.by_iid.get_mut(&iid) {
            if let Some(pos) = timers.iter().position(|t| t.id == id) {
                let t = timers.remove(pos);
                g.by_fd.remove(&t.fd.as_raw_fd());
                self.timer_loop.deregister(&t.fd);
                // OwnedFd drop closes the fd.
            }
        }
    }

    pub fn cancel_all(&self, iid: IidKey) {
        let mut g = self.inner.lock();
        if let Some(timers) = g.by_iid.remove(&iid) {
            for t in timers {
                g.by_fd.remove(&t.fd.as_raw_fd());
                self.timer_loop.deregister(&t.fd);
            }
        }
    }
}

const TIMER_PER_IID_QUOTA: usize = 64;
```

### 9.3 mcu-minimal Implementation

```rust
// supervisor/src/interrupt/timer.rs (cfg(feature = "mcu-minimal"))

pub struct TimerManager {
    heap: RefCell<BinaryHeap<ScheduledTimer>>, // single-threaded
    next_id: Cell<u64>,
    systick_hz: u32, // 1000
}

#[derive(Eq, PartialEq)]
struct ScheduledTimer {
    deadline_tick: u64, // monotonic SysTick counter
    iid:           IidKey,
    id:            TimerId,
    period_ticks:  u32,
    prio:          Priority,
}

impl Ord for ScheduledTimer { fn cmp(&self, o: &Self) -> Ordering {
    o.deadline_tick.cmp(&self.deadline_tick) // min-heap
}}

/// Called from the SysTick ISR via a callback table.
pub fn on_systick(tick: u64, mgr: &TimerManager, dispatch: &CallbackDispatch) {
    let mut heap = mgr.heap.borrow_mut();
    while let Some(top) = heap.peek() {
        if top.deadline_tick > tick { break; }
        let due = heap.pop().unwrap();
        dispatch.enqueue(due.iid, due.prio,
            Callback::Timer { iid: due.iid, timer_id: due.id, expirations: 1, deadline_ns: tick * 1_000_000 });
        if due.period_ticks > 0 {
            heap.push(ScheduledTimer { deadline_tick: tick + due.period_ticks as u64, ..due });
        }
    }
}
```

### 9.4 Quota and Quota Errors

Per-iid quota: 64 timers. Exceeding it returns `TimerErr::QuotaExceeded`
to the guest as a `HostTrapCode::QuotaExceeded`. The guest can then call
`vyoma:timer.cancel` and retry. Crash report fields the
`CrashKind::OomKill` if a guest hits the quota and panics.

---

## 10. Crash Reporting

### 10.1 The Two-Stage Pipeline

```
Trap occurs in worker
    │
    ▼
┌─────────────────────────────────────────────────┐
│ Stage 1: Raw record (in-worker, sync, no heap)  │
│  • Write to per-iid CrashBuffer (2 KB, prealloc)│
│  • atomic-write /data/crashes/raw/<uuid>.bin     │
│   (tmp → fsync → rename → fsync parent)         │
└─────────────────────────────────────────────────┘
    │
    │ inotify IN_MOVED_TO
    ▼
┌─────────────────────────────────────────────────┐
│ Stage 2: Symbolication (crash-reporter WASM app)│
│  • Read raw record                              │
│  • Read app's .symbols sidecar                  │
│  • Produce TOML at /data/crashes/<uuid>.toml    │
│  • Delete raw record                            │
└─────────────────────────────────────────────────┘
```

### 10.2 CrashBuffer — Pre-Allocated Per iid

```rust
// supervisor/src/interrupt/crash.rs

pub const CRASH_BUFFER_BYTES: usize = 2048;

pub struct CrashBuffer {
    // Heap-allocated ONCE at iid creation; never freed until iid dies.
    raw: Box<[u8; CRASH_BUFFER_BYTES]>,
    cursor: AtomicUsize,
}

impl CrashBuffer {
    pub fn new() -> Self {
        Self {
            raw: Box::new([0u8; CRASH_BUFFER_BYTES]),
            cursor: AtomicUsize::new(0),
        }
    }

    pub fn write_raw_record(&self, rec: &RawCrashRecord) -> Result<&[u8], CrashErr> {
        // Wire format: bincode-serialized RawCrashRecord, fixed-size = 1600 B.
        let bytes = unsafe { rec.as_bytes() };
        if bytes.len() > CRASH_BUFFER_BYTES { return Err(CrashErr::TooBig); }
        // Cursor reset on every trap; we only store one record at a time.
        unsafe {
            let dst = self.raw.as_ptr() as *mut u8;
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), dst, bytes.len());
        }
        self.cursor.store(bytes.len(), Release);
        let n = bytes.len();
        Ok(unsafe { std::slice::from_raw_parts(self.raw.as_ptr(), n) })
    }
}
```

### 10.3 RawCrashRecord Layout

Fixed-size, `#[repr(C)]`, no heap, no `String`:

```rust
#[repr(C)]
pub struct RawCrashRecord {
    pub magic:        u32,              // 0x56594D43 = "VYMC"
    pub version:      u8,               // 1
    pub _pad:         [u8; 3],
    pub iid:          IidKey,
    pub iid_name_len: u8,
    pub iid_name:     [u8; 31],         // app name; truncated
    pub trap_kind:    u16,              // WasmTrap discriminant
    pub crash_kind:   u16,              // CrashKind discriminant
    pub crash_time_ns: u64,             // CLOCK_REALTIME at trap
    pub mono_ns:      u64,              // CLOCK_MONOTONIC at trap
    pub faulting_pc:  u64,              // WASM module pc, or 0
    pub frame_count:  u8,
    pub frames:       [u64; 32],        // WASM pcs, deepest first
    pub last_ipc_count: u8,
    pub last_ipc:     [IpcHeader; 16],  // 16 × 32 B = 512 B
    pub rss_bytes:    u64,
    pub vss_bytes:    u64,
    pub fd_count:     u32,
    pub timer_count:  u32,
    pub epoch_state:  u8,
    pub epoch_reason: u8,
    pub _reserved:    [u8; 32],
}
// sizeof = 8 + 4 + 32 + 4 + 8 + 8 + 8 + 1 + 32*8 + 1 + 16*32 + 8 + 8 + 4 + 4 + 1 + 1 + 32
//        ≈ 904 bytes; well under 2 KB CrashBuffer.

#[repr(C)]
pub struct IpcHeader {
    pub from: IidKey,
    pub to:   IidKey,
    pub kind: u8,   // 0=request, 1=reply, 2=event
    pub _pad: [u8; 3],
    pub seq:  u64,
    pub len:  u32,
    pub topic_hash: u64,
}
```

### 10.4 Atomic Write Path

```rust
// supervisor/src/interrupt/crash.rs

pub fn write_raw_record_atomic(rec: &RawCrashRecord) -> io::Result<()> {
    let uuid = Uuid::new_v4();
    let dir = Path::new("/data/crashes/raw");
    let tmp = dir.join(format!(".tmp.{uuid}"));
    let fin = dir.join(format!("{uuid}.bin"));

    // 1. open with O_TMPFILE if available; else create tmp file.
    let mut f = OpenOptions::new()
        .write(true).create_new(true).mode(0o600)
        .open(&tmp)?;

    // 2. write fixed-size record.
    let bytes = unsafe {
        std::slice::from_raw_parts(rec as *const _ as *const u8, mem::size_of_val(rec))
    };
    f.write_all(bytes)?;

    // 3. fsync the file.
    f.sync_all()?;
    drop(f);

    // 4. rename tmp → final (atomic on POSIX same-fs).
    std::fs::rename(&tmp, &fin)?;

    // 5. fsync the parent directory so the rename is durable.
    let dirfd = File::open(dir)?;
    dirfd.sync_all()?;
    Ok(())
}
```

### 10.5 Crash Quota Enforcement

```rust
// supervisor/src/interrupt/crash.rs

pub fn enforce_quota() -> io::Result<()> {
    let statfs = nix::sys::statvfs::statvfs("/data")?;
    let data_capacity = statfs.blocks() as u64 * statfs.fragment_size() as u64;
    let quota = std::cmp::max(data_capacity / 20, 10 * 1024 * 1024); // max(5%, 10 MiB)

    let mut entries: Vec<(PathBuf, u64, u64, bool)> = fs::read_dir("/data/crashes")?
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let path = e.path();
            let m = fs::metadata(&path).ok()?;
            let mtime = m.modified().ok()?.duration_since(SystemTime::UNIX_EPOCH).ok()?.as_secs();
            let sticky = is_sticky(&path).unwrap_or(false);
            Some((path, m.len(), mtime, sticky))
        })
        .collect();

    let mut total: u64 = entries.iter().map(|e| e.1).sum();
    if total <= quota { return Ok(()); }

    // Sort by mtime ascending; sticky entries pushed to the end (never deleted
    // unless quota cannot be met).
    entries.sort_by_key(|e| (e.3, e.2));
    for (path, sz, _, sticky) in &entries {
        if total <= quota { break; }
        if *sticky { continue; }
        let _ = fs::remove_file(path);
        total = total.saturating_sub(*sz);
    }
    // Hard cap: also keep at most 50 reports.
    if entries.len() > 50 {
        for (path, _, _, sticky) in &entries[..entries.len() - 50] {
            if !sticky { let _ = fs::remove_file(path); }
        }
    }
    Ok(())
}

fn is_sticky(path: &Path) -> io::Result<bool> {
    // Read the first 256 bytes of the TOML; look for `sticky = true`.
    let mut head = [0u8; 256];
    let mut f = File::open(path)?;
    let n = f.read(&mut head)?;
    Ok(std::str::from_utf8(&head[..n])
        .ok()
        .map(|s| s.contains("sticky = true"))
        .unwrap_or(false))
}
```

### 10.6 Symbolicated TOML Output

```toml
# /data/crashes/0192b3e7-…-toml
schema_version = 1
sticky = false
app_name = "music-player"
app_version = "0.4.2"
iid = "music-player#42"
crash_time_iso = "2026-05-29T14:32:18.943Z"
crash_kind = "WasmMemoryViolation"
trap_kind = "MemoryOob"
faulting_pc = "0x0010_4a3c"
backtrace = [
  { pc = "0x0010_4a3c", function = "decode_mp3_frame", file = "src/decode.rs", line = 187 },
  { pc = "0x0010_4810", function = "decode_next",      file = "src/decode.rs", line = 142 },
  { pc = "0x0010_1004", function = "on-timer",         file = "src/main.rs",   line =  58 },
]
last_ipc = [
  { from = "music-player#42", to = "audio-srv#1", kind = "request", topic = "render", seq = 14401 },
]
resource_state = { rss_mib = 11, vss_mib = 64, fds = 3, timers = 1 }
epoch_state = "Running"
epoch_reason = "Watchdog"
```

### 10.7 Out-of-Process Symbolicator

The `crash-reporter` is a WASM app under `apps/crash-reporter/`:

```rust
// apps/crash-reporter/src/main.rs (sketch)

fn main() -> ! {
    let inotify = vyoma::fs::inotify_init().unwrap();
    inotify.add_watch("/data/crashes/raw", IN_MOVED_TO).unwrap();
    loop {
        for ev in inotify.read_events() {
            let raw_path = format!("/data/crashes/raw/{}", ev.name);
            if let Some(rec) = read_raw_record(&raw_path) {
                let toml_path = format!("/data/crashes/{}.toml", uuid_of(&raw_path));
                let sym = symbolicate(&rec); // reads <app>.symbols sidecar
                fs::write(&toml_path, sym).ok();
                fs::remove_file(&raw_path).ok();
            }
        }
    }
}
```

Failures inside `crash-reporter` do not affect the supervisor's
raw-record path. If `crash-reporter` itself crashes, the raw record
remains in `/data/crashes/raw/` for the next run.

### 10.8 Pre-Computed Symbols Sidecar

The `PackageManager` (R8) extracts DWARF at install time:

```bash
wasm-tools strip -d -o app.wasm.stripped app.wasm
wasm-tools dump app.wasm > app.symbols
```

`.symbols` is line-oriented: `<pc_hex> <name> <file>:<line>`. Stored
alongside the app's `vyoma.toml`. If absent, symbolication falls back
to PC-only backtraces.

---

## 11. PanicPolicy — Compile-Time Subsystem Enum

The architect proposed a `HashMap<&'static str, PanicPolicy>` populated at
boot. That allows a typo to silently default to `Fatal`. We replace it
with a closed `enum` and a `const fn` so the compiler verifies coverage:

```rust
// supervisor/src/interrupt/panic_recovery.rs

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Subsystem {
    EventLoop,    // single-loop variant (mcu-minimal / iot-edge)
    InputLoop,    // multi-loop input thread
    TimerLoop,    // multi-loop timer thread
    SignalLoop,   // multi-loop signal thread
    GeneralLoop,  // multi-loop general thread
    IpcBroker,
    Display,
    Audio,
    TimerManager,
    CrashWriter,
    OtaPoll,
    Worker(QosClass),
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum PanicPolicy {
    /// Catch the panic, log it, recreate the subsystem, continue.
    Restart,
    /// Catch the panic, log it, disable the feature, continue.
    Degrade,
    /// Catch the panic, log it, exit PID-2 to let stage1 cold-restart.
    Fatal,
}

pub const fn panic_policy(s: Subsystem) -> PanicPolicy {
    match s {
        Subsystem::EventLoop      => PanicPolicy::Fatal,    // single-loop death = everything dies
        Subsystem::InputLoop      => PanicPolicy::Restart,
        Subsystem::TimerLoop      => PanicPolicy::Restart,
        Subsystem::SignalLoop     => PanicPolicy::Fatal,    // cannot survive signal-loop death
        Subsystem::GeneralLoop    => PanicPolicy::Restart,
        Subsystem::IpcBroker      => PanicPolicy::Restart,
        Subsystem::Display        => PanicPolicy::Degrade,  // run headless instead
        Subsystem::Audio          => PanicPolicy::Degrade,  // silence audio
        Subsystem::TimerManager   => PanicPolicy::Restart,
        Subsystem::CrashWriter    => PanicPolicy::Degrade,  // log to ring buffer
        Subsystem::OtaPoll        => PanicPolicy::Degrade,  // skip OTA updates
        Subsystem::Worker(_)      => PanicPolicy::Restart,  // restart the iid via app respawn
    }
}
```

`run_subsystem` invokes the closure inside `catch_unwind`:

```rust
pub fn run_subsystem<F>(s: Subsystem, f: F)
where
    F: FnOnce() + std::panic::UnwindSafe,
{
    let result = std::panic::catch_unwind(f);
    if let Err(payload) = result {
        let msg = panic_msg(&payload);
        log::error!("[panic][{:?}] {}", s, msg);
        match panic_policy(s) {
            PanicPolicy::Restart => {
                // The caller of run_subsystem is the supervisor's main loop or
                // a respawn manager; it will see the function returned and
                // reissue the spawn.
                metrics::SUBSYS_RESTART.inc(s);
            }
            PanicPolicy::Degrade => {
                metrics::SUBSYS_DEGRADED.inc(s);
                // Caller may notice this via a `SubsystemHealth` flag.
                health::mark_degraded(s);
            }
            PanicPolicy::Fatal => {
                metrics::SUBSYS_FATAL.inc(s);
                log::error!("[panic][fatal] subsystem {:?} died; exiting PID-2", s);
                // stage1 will respawn us within 200 ms.
                std::process::exit(70);
            }
        }
    }
}
```

The supervisor's main loop wraps each subsystem spawn in
`run_subsystem`. Worker threads' `run_iid_worker` does the same.
Anywhere in supervisor code that is **not** wrapped in
`run_subsystem` and panics will terminate PID-2 unwinding to stage1.
This is intentional: it forces every long-lived subsystem to opt
into a policy explicitly.

---

## 12. Per-App Worker Thread

### 12.1 Thread Per iid

Each running iid has exactly one worker thread:

```rust
// supervisor/src/interrupt/worker.rs

pub struct WorkerHandle {
    pub iid:    IidKey,
    pub queue:  Arc<CallbackQueue>,
    pub state:  Arc<IidEpochState>,
    pub thread: JoinHandle<()>,
    pub stop:   Arc<AtomicBool>,
}

pub fn spawn_iid_worker(
    iid: IidKey,
    qos: QosClass,
    cfg: &EventLoopConfig,
    runtime: Arc<dyn WasmRuntime>,
    dispatch: Arc<CallbackDispatch>,
) -> WorkerHandle {
    let queue = Arc::new(CallbackQueue::new(cfg));
    let state = Arc::new(IidEpochState::new());
    dispatch.register_iid(iid, cfg);

    let stop = Arc::new(AtomicBool::new(false));
    let h_queue = queue.clone();
    let h_state = state.clone();
    let h_stop  = stop.clone();
    let h_qos   = qos;
    let h_iid   = iid;

    let thread = thread::Builder::new()
        .name(format!("vyoma-iid-{}", iid.0))
        .spawn(move || {
            apply_sched_policy(cfg.worker_sched.for_qos(h_qos)).ok();
            run_subsystem(Subsystem::Worker(h_qos), || {
                run_iid_worker(h_iid, runtime, h_queue, h_state, h_stop);
            });
        })
        .expect("worker thread");

    WorkerHandle { iid, queue, state, thread, stop }
}
```

### 12.2 The Worker Loop

```rust
fn run_iid_worker(
    iid: IidKey,
    runtime: Arc<dyn WasmRuntime>,
    queue: Arc<CallbackQueue>,
    state: Arc<IidEpochState>,
    stop: Arc<AtomicBool>,
) {
    // Park on a per-iid Condvar when both HPQ and LPQ are empty.
    let park = WorkerPark::new();
    queue.set_wakeup(park.clone());

    while !stop.load(Acquire) {
        // 1. Drain HPQ first; HPQ entries are not coalesced and always run.
        let cb = match queue.pop() {
            Some(cb) => cb,
            None => { park.wait_with_timeout(Duration::from_millis(100)); continue; }
        };

        // 2. Check kill state BEFORE dispatching.
        match state.state.load(Acquire) {
            x if x == IidState::KillPending as u8 => {
                // Skip everything but Terminate; tear down immediately.
                if !matches!(cb, Callback::Terminate { .. }) { continue; }
            }
            x if x == IidState::Dead as u8 => return,
            _ => {}
        }

        // 3. Dispatch into WASM. This may take up to the slice budget;
        // longer if the guest is doing bulk memory ops or a hostcall.
        let trap_result = runtime.dispatch_callback(iid, &cb);

        // 4. Observe trap result.
        match trap_result {
            Ok(()) => {}
            Err(WasmTrap::Interrupt) => {
                // Soft yield: epoch ticker requested. Mark Yielded; loop continues.
                state.observe_yield();
                // Continue to next callback; the guest will get a fresh slice.
            }
            Err(trap) => {
                // Hard trap: record crash, transition to Dead, stop worker.
                let rec = build_raw_record(iid, trap, &state);
                let _ = write_raw_record_atomic(&rec);
                state.mark_dead();
                metrics::IID_CRASHED.inc(iid);
                // Notify lifecycle manager (R8 BootPhase / app supervisor).
                lifecycle::on_iid_dead(iid, trap_to_crashkind(trap));
                return;
            }
        }
    }

    // Stop requested. Drain HPQ for any Terminate to run on-terminate.
    while let Some(cb) = queue.pop() {
        if let Callback::Terminate { .. } = cb {
            let _ = runtime.dispatch_callback(iid, &cb);
            break;
        }
    }
    state.mark_dead();
    dispatch.unregister_iid(iid);
}
```

### 12.3 WorkerPark — Cheap Per-iid Parking

```rust
pub struct WorkerPark {
    mtx:  Mutex<bool>,        // "queue non-empty" hint
    cv:   Condvar,
}

impl WorkerPark {
    pub fn wait_with_timeout(&self, dur: Duration) {
        let mut g = self.mtx.lock();
        if *g { *g = false; return; }
        let (g2, _) = self.cv.wait_timeout(g, dur);
        drop(g2);
    }
    pub fn wake(&self) {
        let mut g = self.mtx.lock();
        *g = true;
        self.cv.notify_one();
    }
}
```

`CallbackQueue::enqueue` calls `park.wake()` after a successful push.

### 12.4 Worker SCHED Policy by QoS

| QoS class (R5) | SCHED policy on multi-loop platforms |
|---|---|
| `Audio` | SCHED_FIFO 80 |
| `ControlLoop` (robotics-rt only) | SCHED_FIFO 75 |
| `UserInteractive` | SCHED_OTHER nice −5 |
| `UserInitiated` | SCHED_OTHER nice 0 |
| `Utility` | SCHED_OTHER nice +5 |
| `Background` | SCHED_OTHER nice +10 |
| `Maintenance` | SCHED_IDLE |

Workers are pinned to no specific CPU set; cgroup `cpuset` (managed by
the scheduler in R5) handles affinity.

---

## 13. Stage1 Restart Semantics

### 13.1 Cold Reboot Is the Contract

When PID-2 exits (intentionally or via crash), stage1 spawns a fresh
PID-2 process. The new PID-2:

1. Does **not** inherit any per-app state from the previous PID-2.
2. **Does** inherit the persistent `/data` directory (apps' saved files
   survive).
3. Drains every IPC FIFO before starting any app (`unlink + recreate`).
4. Scans `/data/crashes/` to populate metrics and the crash-reporter app.
5. Spawns apps from `boot.toml` in BootPhase order (R8).

Per-app state preservation across restarts is **not** in scope for R10.
A future hibernate subsystem will manage that.

### 13.2 The Panic-Loop Detector in Stage1

Stage1 maintains a 30-second rolling window of PID-2 deaths:

```rust
// stage1/src/main.rs

const PANIC_WINDOW_SECS: u64 = 30;
const PANIC_THRESHOLD:   usize = 5;

fn main() -> ! {
    let mut deaths = VecDeque::<Instant>::new();
    loop {
        let pid2 = spawn_pid2(/* normal | safe */);
        let exit = wait_for_exit(pid2);

        let now = Instant::now();
        deaths.push_back(now);
        while let Some(&t) = deaths.front() {
            if now.duration_since(t).as_secs() > PANIC_WINDOW_SECS {
                deaths.pop_front();
            } else { break; }
        }

        if deaths.len() > PANIC_THRESHOLD {
            // Cold-boot a SAFE-MODE supervisor: skip OTA, skip non-essential
            // apps, no crash-recovery scan, print diagnostic banner.
            spawn_pid2_safe_mode();
            deaths.clear();
        }

        // Cap restart delay to 200 ms total (per R8 spec).
        thread::sleep(Duration::from_millis(50));
    }
}
```

### 13.3 Safe-Mode Supervisor

Triggered conditions:

- Panic-loop detector fires (5 deaths in 30 s)
- `/data/safe-mode` flag file exists (operator override)
- BootPhase R8 panic during stage1 init

Behavior:

- Skip crash-recovery scan (avoid heap-allocating thousands of records).
- Spawn only `essential = true` apps from `boot.toml`.
- Print "SAFE MODE — kernel-bug or storage error suspected" banner to fb0.
- Disable OTA polling.
- Refuse non-essential IPC.

### 13.4 IPC FIFO Drain on Restart

Every WASM-app-to-WASM-app channel from the previous PID-2 had a backing
named FIFO under `/run/vyoma/ipc/`. After a crash, those FIFOs may have
stale partial messages (an iid was mid-write at trap). The new PID-2:

```rust
// supervisor/src/main.rs (BootPhase::Pid2Init)

fn drain_stale_ipc_fifos() -> io::Result<()> {
    let dir = Path::new("/run/vyoma/ipc");
    if !dir.exists() { return Ok(()); }
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        // unlink + recreate the FIFO with mkfifo so any stale buffer is gone.
        fs::remove_file(&path).ok();
        nix::unistd::mkfifo(&path, nix::sys::stat::Mode::from_bits(0o660).unwrap())?;
    }
    Ok(())
}
```

### 13.5 Crash Scan on Startup

```rust
fn scan_crash_dir() {
    let deadline = Instant::now() + Duration::from_secs(5);
    let dir = Path::new("/data/crashes/raw");
    if !dir.exists() { return; }

    let mut count = 0usize;
    for entry in fs::read_dir(dir).unwrap_or_else(|_| return).flatten() {
        if Instant::now() > deadline {
            log::warn!("crash scan deadline hit at {} records; deferring rest", count);
            schedule_background_crash_scan();
            return;
        }
        let path = entry.path();
        let parse_result = std::panic::catch_unwind(|| parse_raw_record(&path));
        match parse_result {
            Ok(Ok(rec)) => { metrics::CRASH_REC.inc(&rec); count += 1; }
            Ok(Err(_)) | Err(_) => {
                // Corrupted record: quarantine.
                let q = Path::new("/data/crashes/corrupt").join(entry.file_name());
                let _ = fs::rename(&path, &q);
            }
        }
    }
}
```

### 13.6 Stage1 Defensive Binary

Per R8, stage1 itself is ~150 LOC, has zero allocations, no dynamic
dispatch, and stores nothing on disk between PID-2 spawns. Its sole job
is the panic-loop detector + restart, plus the very first SIGSEGV
self-pipe install. If stage1 itself crashes, the kernel's
`/proc/sys/kernel/panic_on_oops` setting reboots the machine.

---

## 14. Audio Latency Budget and Measurement

### 14.1 The Budget

| Metric | Target | Platform |
|---|---|---|
| `on-audio-render` callback p50 | ≤2 ms | desktop-full, mobile |
| `on-audio-render` callback p99 | ≤4 ms | desktop-full, mobile |
| `on-audio-render` callback p99.9 | ≤6 ms | desktop-full, mobile |
| Audio period | 5.33 ms (48 kHz / 256 frames) | desktop-full, mobile |

### 14.2 What the Budget Measures

Time from ALSA `pollfd` becoming ready (the moment the kernel signals
"buffer underrun risk in N frames") to the moment `Func::call(on-audio-render, …)`
begins executing.

### 14.3 Engineering Choices to Meet the Budget

1. **Audio timer = HPQ.** Audio callbacks are enqueued at `Priority::High`,
   evicting nothing else (HPQ is unbounded-block).
2. **Audio worker = SCHED_FIFO 80.** Higher than InputLoop (RR 60) so
   audio preempts input on the CPU.
3. **Dedicated TimerLoop.** ALSA `pollfd` lives on the TimerLoop (SCHED_FIFO 50).
4. **No locking in the audio path.** The `runtime.dispatch_callback`
   path for `Callback::Audio` does not take any RankedMutex above
   rank 200; the only locks are: WasmRuntime store per-iid (per-instance,
   uncontended) and the audio backend's lockless ring buffer.

### 14.4 Measurement Plan

The `audio-bench` WASM app:

```rust
// apps/audio-bench/src/main.rs (sketch)

fn on_audio_render(_frames: u32, mono_ns: u64) {
    let recv_ns = wasi::clock_monotonic_now_ns();
    let jitter_ns = recv_ns.saturating_sub(mono_ns);
    HISTOGRAM.observe(jitter_ns);
    if RUN_COUNT.fetch_add(1, Relaxed) == 60_000 { // ~5 min @ 200 Hz
        let p50 = HISTOGRAM.percentile(50);
        let p99 = HISTOGRAM.percentile(99);
        let p999 = HISTOGRAM.percentile(99.9);
        println!("AUDIO_BENCH p50={p50}ns p99={p99}ns p99.9={p999}ns");
        if p99 > 4_000_000 {
            println!("AUDIO_BENCH: FAIL p99={p99}ns > 4ms budget");
            std::process::exit(1);
        }
    }
}
```

CI gate in `make test`:

```
make smoke-audio    # boots VM with audio-bench, captures AUDIO_BENCH line,
                    # fails if p99 > 4 ms.
```

Failure modes that this gate catches:

- A locking regression on the audio path.
- An EventLoop split that accidentally puts ALSA on the GeneralLoop.
- A RankedMutex rank-order regression in `interrupt/timer.rs`.

---

## 15. mcu-minimal Specifics

### 15.1 Why It Is Different

`mcu-minimal` runs on a Cortex-M4 with 128 KB RAM and no Linux kernel.
There is no `signalfd`, no `epoll`, no `mio`, no thread library (the
RTOS is bare-metal), no DRM, no 9P. Everything is built around a single
cooperative loop driven by SysTick.

### 15.2 The Cooperative Loop

```rust
// supervisor/src/interrupt/event_loop.rs (cfg(feature = "mcu-minimal"))

pub struct McuEventLoop {
    dispatch: McuDispatch,
    timer:    TimerManager,      // SysTick-driven
    input:    OptionalRawInput,  // ADC/GPIO-driven
    halr:     HalRegistry,       // from R9
}

impl McuEventLoop {
    pub fn run(&mut self) -> ! {
        loop {
            let now = systick::ticks();
            timer::on_systick(now, &self.timer, &self.dispatch);
            self.input.poll(now, &self.dispatch);
            self.halr.poll(now, &self.dispatch);
            self.dispatch.drain_one_iid();
            cortex_m::asm::wfi(); // wait for next SysTick interrupt
        }
    }
}
```

`McuDispatch::drain_one_iid` picks one iid and runs at most one HPQ
callback + one LPQ callback on it. Multiple iids share the single CPU
core via this cooperative round-robin.

### 15.3 wasm3 Cancel Hook

```rust
// supervisor/src/runtime/wasm3_adapter.rs

static CANCEL_FLAGS: [AtomicBool; MAX_MCU_IIDS] = ... ;

#[no_mangle]
extern "C" fn vyoma_wasm3_instruction_hook(iid_slot: u32) -> i32 {
    if CANCEL_FLAGS[iid_slot as usize].load(Acquire) {
        1 // returns non-zero ⇒ wasm3 aborts current execution.
    } else {
        0
    }
}

impl WasmRuntime for Wasm3Adapter {
    fn request_cancel(&self, iid: IidKey, _reason: EpochKillReason) {
        let slot = self.slot_of(iid);
        CANCEL_FLAGS[slot].store(true, Release);
    }
}
```

Calibrated at 1 instruction-hook per 1000 instructions on Cortex-M4 @
168 MHz; cancel latency ≤100 µs.

### 15.4 HardFault Handler

```rust
// supervisor/src/interrupt/hardfault.rs

#[exception]
unsafe fn HardFault(frame: &ExceptionFrame) -> ! {
    // RAM-resident ring buffer; survives reset on M4 only if backed by
    // standby SRAM. Otherwise we just spin UART output before reset.
    let _ = uart_emergency_println!("[HARDFAULT] pc={:08x} lr={:08x} psr={:08x}",
                                   frame.pc, frame.lr, frame.xpsr);
    cortex_m::peripheral::SCB::sys_reset();
}
```

No TOML crash report on mcu-minimal; the UART ring buffer is the only
artifact.

### 15.5 No Crash Quota; Tiny CrashBuffer

CrashBuffer on mcu-minimal is 256 B (not 2 KB). It holds a compressed
RawCrashRecord (no IPC envelope history; just PC + last 8 frames + RSS).
On reset, the bootloader optionally sends the buffer over UART.

---

## 16. Implementation Files and Line Budgets

```
supervisor/src/interrupt/
├── mod.rs                    180 LOC   re-exports; InterruptSubsystem init
├── types.rs                  450 LOC   WasmTrap, CrashKind, IidState, EventLoopConfig
├── event_loop.rs             490 LOC   SingleLoop, MultiLoop, sched policy
├── callback_queue.rs         420 LOC   HPQ+LPQ, CallbackDispatch, coalescing
├── epoch.rs                  300 LOC   EpochTicker, cancel API
├── signal.rs                 460 LOC   pre-engine handlers, signalfd, SignalLoop
├── timer.rs                  470 LOC   TimerManager (Linux + MCU variants)
├── crash.rs                  490 LOC   RawCrashRecord, CrashBuffer, atomic write, quota
├── panic_recovery.rs         290 LOC   Subsystem enum, panic_policy, run_subsystem
└── worker.rs                 470 LOC   run_iid_worker, WorkerPark, sched

Total: ~4,020 LOC; every file ≤490 LOC (under the 500-LOC project limit).
```

Cargo dependencies added:

| Crate | Version | Cost |
|---|---|---|
| `mio` | `1.0` | ~50 KB |
| `crossbeam-queue` | `0.3` | ~10 KB (incremental, already have `crossbeam-channel`) |
| `nix` | `0.27` (already in supervisor) | 0 |

Total binary size impact: +~80 KB to supervisor (from 697 KB to ~777 KB).
Well within the budget.

WIT additions to `wit/vyoma-app.wit`:

```wit
interface app {
    on-timer:        func(timer-id: u32, expirations: u32);
    on-ipc:          func(from: string, envelope-id: u64);
    on-input:        func(ev: input-event);
    on-audio-render: func(frames: u32, mono-ns: u64);
    on-suspend:      func(reason: suspend-reason);
    on-resume:       func(reason: resume-reason);
    on-key-repeat:   func(keycode: u32, count: u32);
    on-terminate:    func(reason: terminate-reason);
    on-watchdog:     func(); // optional; if implemented, watchdog calls it
}

variant suspend-reason {
    user-requested, low-battery, screen-locked, system-shutdown,
}
variant resume-reason {
    user-requested, on-charger, screen-unlocked,
}
variant terminate-reason {
    shut-down, killed, restart, oom, watchdog-timeout, policy-kill,
}
```

---

## 17. Interaction with Prior Rounds

### 17.1 R1 — WIT Callbacks & Epoch

- R10 formalizes the 9 WIT callbacks (R1 listed 5; R10 adds `on-audio-render`,
  `on-key-repeat`, `on-input`, `on-watchdog`).
- R1's CrashKind enum is consumed by R10's `signal_to_crashkind` /
  `trap_to_crashkind`; R10 finalizes the mapping.
- R10's `IidEpochState` is the implementation of R1's epoch contract.

### 17.2 R5 — Scheduler & QoS

- R5's `QosClass` flows into `Subsystem::Worker(QosClass)`; R10's
  `WorkerSchedTable` is the concrete SCHED policy table.
- R5's `SCHED_DEADLINE` is exposed through `SchedPolicy::Deadline`.
- R5's `cgroup cpu.max` quota is orthogonal to R10's epoch slice; they
  compose: epoch handles per-callback latency, cgroup handles per-iid
  CPU share.

### 17.3 R6 — Device Drivers

- R6's `DeviceManager::on_uevent` is a callback into the `GeneralLoop`
  (udev netlink fd registered there).
- R6's `InputDispatcher::dispatch` is what runs on the `InputLoop`'s
  evdev decode path; it produces `Callback::Input { ev }` for HPQ/LPQ.
- R6's three-tier driver model (kernel-included, supervisor-bundled,
  external) is unaffected; external drivers' SIGCHLD is handled in R10.

### 17.4 R7 — Power Management

- R7's `ThermalGovernor` sends `SIGSTOP` for tier-2 throttle. R10's
  `SignalLoop` does not see SIGSTOP for the supervisor itself (which
  has SIGSTOP unblocked); tier-3 SIGKILL on a misbehaved app is
  observed by R10 via SIGCHLD → `Callback::Terminate(OomKill)`.
- R7's `AssertionRegistry` is consulted before R10 enters epoch
  cancellation; an `ActivityAssertion(prevent-kill)` makes
  `request_cancel(Watchdog)` no-op until the assertion drops.

### 17.5 R8 — Boot Sequence

- R8's `BootPhase` controls subsystem startup ordering; R10's
  `InterruptSubsystem` is started in `BootPhase::CoreServices`,
  before any app.
- R8's `RankedMutex` ranks are reserved as follows for R10:
  - 100  `IidEpochState` per-iid CAS (lock-free; no mutex)
  - 200  `CallbackQueue` per-iid (lock-free; no mutex)
  - 800  `TimerManager` global
  - 900  `CrashDirQuota` global
  - 1000 `CallbackDispatch.queues` map (RwLock)
- R8's `catch_unwind` on every phase composes with R10's `run_subsystem`.
- R8's stage1 — R10 adds the panic-loop detector to it; otherwise unchanged.

### 17.6 R9 — Hardware Abstraction Layer

- R9's per-bus `HalRequest` thread queues are independent of R10's
  EventLoop; HAL completions are surfaced to the appropriate iid's
  `CallbackQueue` as `Callback::HalEvent`.
- R9's GPIO_V2 chardev fd polling is registered with the `GeneralLoop`.

---

## 18. Testing Strategy

### 18.1 Unit Tests (per file, in supervisor/tests/)

| Test file | Scope |
|---|---|
| `test_wasmtrap_mapping.rs` | every `WasmTrap` variant → expected `CrashKind` |
| `test_signal_mapping.rs` | every signal number → expected `CrashKind` |
| `test_iidstate_cas.rs` | concurrent CAS transitions; no lost transitions |
| `test_callback_queue.rs` | HPQ never evicts; LPQ evicts oldest; coalescing |
| `test_panic_policy.rs` | every `Subsystem` variant has a non-default policy |
| `test_crash_record.rs` | atomic write, fsync, partial-write recovery |
| `test_timer_quota.rs` | 65th timer returns QuotaExceeded |
| `test_event_loop_split.rs` | per-profile config produces correct loop topology |
| `test_epoch_ticker.rs` | tick rate ±10% on iot-edge, mobile, desktop-full |

### 18.2 Integration Tests (in QEMU)

| Test name | Verifies |
|---|---|
| `smoke-crash-recovery` | trap an app, observe TOML crash report appears |
| `smoke-audio-jitter` | run `audio-bench` for 5 min, p99 ≤4 ms |
| `smoke-watchdog` | infinite-loop app gets `Killed` within `watchdog_secs + 200 ms` |
| `smoke-panic-loop` | force PID-2 to die 6 times in 30 s, observe safe mode banner |
| `smoke-segv-recovery` | force `unreachable` trap, observe iid restart per policy |
| `smoke-bulk-memory` | guest `memory.fill 256 MiB`, observe yield within slice + cap |
| `smoke-key-repeat-flood` | flood key-repeat, observe coalescing to one callback |

### 18.3 Stress Tests

- **Worker spawn/teardown:** spawn and kill 1000 iids in a loop; verify
  no FD leak (track `/proc/self/fd` count) and no thread leak
  (`/proc/self/task` count).
- **CrashBuffer flood:** trap an app 1000 times in 60 s; verify
  `enforce_quota` keeps `/data/crashes/` under quota; no panic.
- **HPQ pressure:** synthesize 10 K watchdog callbacks to one iid in
  100 ms; observe enqueue is bounded (no unbounded growth) and worker
  drains correctly.

### 18.4 Manual Verification

Console commands available at the supervisor REPL:

```
crash list                          # show recent /data/crashes/*.toml
crash show <uuid>                   # print one record
crash mark-sticky <uuid>            # tag for never-prune
crash purge                         # force quota enforcement now
trap <app>                          # synthetic trap for testing
loop kill <input|timer|signal|general>  # synthetic subsystem panic
panic-policy                        # print the const table
```

---

## 19. Critical Decisions

### 19.1 Never (Out of Scope, Permanently)

- **Async-signal-safe logic beyond a self-pipe write.** Calling Rust
  formatting, allocators, locks, or even most stdlib functions from a
  signal handler is unsound. We never extend the signal handler.
- **Per-app exception ports** (XNU style). The Mach exception-port
  mechanism that allows a per-process exception handler is overkill
  for a single-supervisor model. Apps cannot register custom trap
  handlers; they get `on-terminate` and a crash record after the fact.
- **In-process DWARF symbolication.** Adding `addr2line` or `gimli` to
  the supervisor would double its binary size. Symbolication runs
  out-of-process in the `crash-reporter` WASM app.
- **`tokio` or any async runtime** for the EventLoop. We use mio +
  native threads; this is settled.
- **Fuel metering.** Epoch interruption is the chosen preemption
  mechanism; fuel is not enabled.

### 19.2 Deferred (Future Rounds)

- `crash-reporter` WASM app's UI (settled in R76 — App Framework).
- Sticky crash report UX (which gestures mark sticky; settled in R23 — System Chrome).
- Advanced DWARF with source-level line info per frame requires R76's
  filesystem ABI for reading sidecars.
- Per-app suspended state preservation across PID-2 restart (future
  hibernate subsystem).
- Crash report submission to a remote telemetry endpoint (future R75 — App Distribution).

### 19.3 Critical v1 (Must Ship)

- `WasmTrap` enum and `CrashKind` mapping.
- `EventLoop` (multi-loop on perf platforms).
- `CallbackQueue` (HPQ + LPQ + coalescing).
- `EpochTicker` with per-platform tick rates.
- `RawCrashRecord` and atomic write path.
- `Subsystem` enum and `const fn panic_policy`.
- Stage1 panic-loop detector.
- `audio-bench` and `smoke-audio-jitter` CI gates.

Anything else can ship in a follow-up.

---

## 20. Summary

Round 10 specifies, end-to-end, how VyomaOS turns hardware interrupts
into WASM callbacks and how it handles every category of failure that
can occur in that pipeline. The headline design choices:

1. **Demultiplex via mio, not async.** A single `EventLoop` (mcu/iot)
   or four-loop split (mobile/desktop/server/robotics) hands events to
   per-iid `CallbackQueue`s and per-iid worker threads.

2. **Two-queue model, not eviction.** A `HighPriorityQueue` for
   watchdog/audio/control-loop/Terminate callbacks (never evicted;
   block on full) and a bounded `LowPriorityQueue` for everything else
   (evict oldest on overflow).

3. **Epoch interruption as the only preemption.** Honest documentation
   of the three gaps (bulk memory, hostcalls, wasm3) and explicit
   mitigations (per-platform memory cap, async-friendly hostcalls,
   instruction-count cancel hook on wasm3).

4. **Signal handlers are pure self-pipe writers.** All policy executes
   on the `SignalLoop` thread, never inside a handler. SIGSEGV is
   installed *before* `Engine::new()` with `SA_RESETHAND`; Wasmtime's
   host signal handler is registered via `Config::with_host_signal_handler`.

5. **Out-of-process crash symbolication.** Pre-allocated 2 KB
   `CrashBuffer` per iid; atomic raw-record writes;
   `crash-reporter` WASM app produces the TOML; if it crashes, the
   raw record survives.

6. **Compile-time panic policy.** Closed `Subsystem` enum, `const fn`,
   exhaustive `match`: no typo can silently default to `Fatal`.

7. **Stage1 owns restart, with a panic-loop detector.** PID-2 cold
   restart is the failure mode; safe-mode supervisor kicks in after
   five deaths in thirty seconds.

8. **Audio is a first-class HPQ tenant** with SCHED_FIFO 80 workers
   and a CI-enforced p99 ≤4 ms budget on `desktop-full` and `mobile`.

9. **`WasmRuntime` trait gains `request_cancel`, `set_slice_ms`,
   `set_memory_cap`, `force_trap`** — keeping the runtime abstraction
   clean while supporting both Wasmtime and wasm3.

10. **Ten files, ≤490 LOC each.** No file violates the 500-line rule;
    total cost ~80 KB to the supervisor binary.

This is the final spec. The next round (R11 — Display Server & Compositor)
can assume that input, timer, audio, and IPC callbacks are reliably
dispatched, and that any subsystem crash is contained.

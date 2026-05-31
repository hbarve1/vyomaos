# Round 10 Critique — Interrupt & Exception Handling

**Role:** Critic
**Date:** 2026-05-29
**Verdict:** REJECT AND REDESIGN — the core single-`EventLoop` model and the epoch-only preemption story are not safe for VyomaOS's stated multi-platform targets (mobile audio, robotics-rt, mcu-minimal). Several blocking issues require structural changes, not parameter tweaks.

---

## 1. Verdict summary

- **Single `EventLoop` thread is a soft-real-time killer.** All event sources — input, timers, signals, IPC, thermal — funnel through one epoll handler. A single slow dispatch blocks audio callbacks, watchdog timers, and SIGCHLD reaping simultaneously. There is no path to <1 ms latency on the `mobile` profile and no path to RT scheduling on `robotics-rt` without splitting it.
- **Epoch interruption is mis-sold as "the" preemption mechanism.** Epoch checks happen at WASM function entry and loop backedges *only*. They do not interrupt host-side hostcalls, do not interrupt `memory.copy`/`memory.fill` bulk ops, and do not interrupt blocking WASI calls. The architect's "1 kHz EpochTicker = CPU preemption" claim is false. The system is in fact cooperative whenever WASM is in a hostcall or a bulk memory op, and the design has no story for the gap.
- **SIGSEGV chaining to Wasmtime via stored `oldact` is undefined behavior in the general case.** Wasmtime registers its handler via `sigaction(SIGSEGV, ..., &oldact)` *itself*; if the supervisor saves Wasmtime's `oldact` and forwards into it post-handler, the supervisor is calling into Wasmtime internals at a moment when supervisor invariants may be violated. The handler ordering is wrong and the design needs to install *before* Wasmtime, not after.
- **Stage1 → PID-2 restart leaves persistent and kernel state inconsistent.** `/data/crashes/` (9P, persistent), in-flight FIFO IPC envelopes (kernel queues survive only because they're tied to the FIFO inode), and partial TOML writes after `O_APPEND` without `fsync` cause forensic corruption and silent drops. The design ignores recovery semantics.
- **`CallbackQueue` overflow eviction inverts safety.** Evicting lowest-priority callbacks on overflow means a sustained input flood silently drops watchdog-tick Timer callbacks — the exact callbacks whose loss is detectable only when the watchdog kills the wrong app. This is worse than backpressure.
- **DWARF symbolication at trap time is unsafe.** It reads `.wasm` from 9P, allocates from a possibly-OOM heap, and runs inside the same supervisor process that just trapped. It must be deferred to an out-of-process reporter or precomputed at install time.
- **`PanicPolicy` runtime name → policy `HashMap` lookup forfeits compile-time completeness.** A new subsystem with no entry silently defaults; this is exactly the kind of latent fault that the rest of VyomaOS works hard to prevent at the type level.
- **mcu-minimal uses wasm3, not Wasmtime; the entire `EpochTicker` + `Func::call` story does not apply.** wasm3 has no epoch interruption, no `Store`, no `Engine`. The architect's design *implicitly* assumes Wasmtime on every profile and does not specify a wasm3 interrupt path. mcu-minimal currently has no preemption story at all in this proposal.

---

## 2. BLOCKING issues (must fix before synthesis)

### 2.1 Single `EventLoop` thread is not viable on `mobile` or `robotics-rt`

The architect collapses every external event source — `timerfd`, `signalfd`, `evdev`, `gpiochip`, netlink, thermal pipes, IPC sockets — into one `mio::Poll` consumed by a single thread that also dispatches into per-app `CallbackQueue`s. This single thread is now on the critical path of:

1. Audio frame-ready interrupts (mobile profile, 5.8 ms period at 48 kHz / 256 frames).
2. Robot control-loop tick timers (robotics-rt, 1 kHz hard deadline).
3. Watchdog ticks for every running app.
4. SIGCHLD reaping (which must complete before the next `fork()`/`exec()` cycle).
5. Input event dispatch (target <16 ms input latency for desktop).
6. Thermal throttle events.
7. IPC envelope routing.

Any one of these going slow blocks all the others. Worst case is realistic and easy to construct:

- A long-haul SIGCHLD reap that walks the iid registry under a `Mutex` lock.
- A burst of evdev events from key-repeat plus mouse motion (~1000 events/sec).
- A netlink storm during USB hotplug enumeration.
- A 9P stall that delays a thermal-pipe read.

The architect's response to this is "drain in priority order" — but that orders *within* the queue, not between event sources. The EventLoop reads `mio::Poll` events in whatever order epoll returns them; a single misbehaving source can monopolize iterations.

**This is not fixable by tuning.** It needs structural separation:

- A *dedicated* `InputLoop` thread for evdev (SCHED_RR, real-time priority) — input is the most latency-sensitive desktop path.
- A *dedicated* `TimerLoop` thread for `timerfd` (SCHED_FIFO, audio/robotics-rt parity).
- A *dedicated* `SignalLoop` thread for `signalfd` (normal priority, but isolated so SIGCHLD reap doesn't block timer fire).
- The general `EventLoop` handles only IPC, netlink, thermal, and slow-path events.
- All four threads feed into the same per-app `CallbackQueue` set, with the queue handling cross-thread enqueue via lock-free or `parking_lot::Mutex<VecDeque<_>>` with priority bands.

The architect must justify why a single-threaded EventLoop is acceptable on `mobile` (where audio needs <6 ms p99) and `robotics-rt` (where 1 kHz control loops must not miss), or split the loop. There is no middle ground.

**Quantitative back-of-envelope:** A `mio::Poll` wakeup on Linux costs ~5 µs; epoll-readiness-to-thread-dispatch is ~10 µs more. If the EventLoop processes 100 events per `poll()` batch (realistic under load), that is 1.5 ms just in scheduling overhead before *any* event handler runs. The audio app's `Func::call` then takes 200 µs–2 ms. The total budget consumed before the *next* poll batch is observed is 3–4 ms — most of the 5.33 ms audio period gone. A single misbehaving IPC handler (an unbounded loop in supervisor-side routing, say, because of a malformed envelope) consumes the rest. There is no headroom for a single-loop design.

**Why not a thread-per-event-source design?** The architect may argue thread proliferation is bad for mcu-minimal (RAM cost) and complicates lock discipline. Both concerns are real; both have answers:

- mcu-minimal does not run audio or robotics; its event sources are GPIO, SPI, and one timer. A single-loop design is appropriate *there*, and the design must say so explicitly: "EventLoop is single-threaded on `mcu-minimal` and `iot-edge`; multi-threaded on `mobile`, `robotics-rt`, `desktop-full`, `server-headless`."
- Lock discipline: per-app `CallbackQueue` uses a single producer lock per loop pair. Four loops × one mutex acquire per enqueue = ~200 ns; negligible. No global state lives between the loops.

### 2.2 Epoch interruption is not CPU preemption — the "gap" is enormous

The proposal repeatedly frames `EpochTicker` + 1 kHz increments as the system's CPU preemption story. This is wrong in several ways and the wrongness has safety consequences.

**What epoch interruption actually does:** Wasmtime emits an epoch check at WASM function entry and at loop backedges *only* (controlled by `Config::epoch_interruption(true)`). When the engine's epoch counter exceeds the store's deadline, the *next* such check raises a trap. It is not a signal, not an async interrupt, not a preemption — it is a polled check at compiler-inserted points.

**Real gaps where epoch does nothing:**

1. **Bulk memory ops.** `memory.copy`, `memory.fill`, `memory.init`, `data.drop` are single WASM instructions. A `memory.copy` of 1 GB executes without a single epoch check. Mitigation requires `Config::native_unwind_info(true)` and either a runtime cancellation (which Wasmtime does not provide for bulk ops) or a per-instance memory size cap that prevents the worst case. The architect specifies neither.

2. **Long straight-line code without loops.** WASM-as-target compilers (LLVM, Binaryen) sometimes unroll heavily. An unrolled hash function with no backedges takes zero epoch checks regardless of how many epochs have ticked.

3. **Hostcalls into the supervisor.** When WASM calls a WASI hostcall — `fd_read`, `poll_oneoff`, `clock_time_get`, IPC envelope send — Wasmtime is *not* executing; the supervisor is. Epoch checks do nothing. If the hostcall blocks (a 9P read on a stalled mount, an IPC send into a full queue), the WASM "callback" appears stuck and there is no preemption story.

4. **Trap handlers themselves.** While Wasmtime is running its own SIGSEGV/SIGBUS handler to translate a guard-page fault into a `WasmTrap::HostStackOverflow`, the epoch counter is irrelevant.

5. **wasm3 on mcu-minimal.** wasm3 has no epoch interruption. It is an interpreter; preemption requires either explicit `m3_AbortRuntime()` calls between instructions (which the architect does not specify) or yielding at WASI hostcalls (cooperative only).

**The design must say, explicitly:**

- What is the worst-case CPU monopolization a malicious or buggy app can achieve? (For `memory.copy` of full address space on a `mobile` profile with 256 MiB and 4 KiB pages, that is ≥250 ms unkillable per the public Wasmtime issue tracker. For the audio thread, that is 40+ missed frames.)
- What is the platform-specific maximum allocatable memory per app such that `memory.copy` worst case stays below the audio period?
- What is the wasm3 cancellation path on `mcu-minimal`?
- Are blocking hostcalls (`fd_read` on a real fd) allowed at all, or must everything go through a non-blocking `poll_oneoff` shim?

Without these answers, the "1 kHz preemption" story is decorative.

**Concrete adversarial example:** A WASM app exports `fn evil()` containing:

```wat
(func $evil
  (memory.copy (i32.const 0) (i32.const 1) (i32.const 0x10000000))
  (memory.copy (i32.const 0) (i32.const 1) (i32.const 0x10000000))
  (memory.copy (i32.const 0) (i32.const 1) (i32.const 0x10000000))
  ;; ...repeat 16 times
)
```

This is one WASM function. There are zero loop backedges. There are 16 instructions. Epoch checks fire at function entry only — *one* check, then 4 GiB of memory traffic with no interruption point. Even at 200 GB/s DRAM bandwidth on a modern desktop, that is 20 ms of unkillable CPU per call. On `mobile` ARM at 25 GB/s, it is 160 ms. The audio thread misses 30 frames. The architect must either:

- Cap per-instance linear memory such that 16× the cap stays under the audio period (so cap ≤ 25 MiB on `mobile`), or
- Patch Wasmtime to emit periodic epoch checks inside long bulk ops (not currently a feature; would require upstream work), or
- Run the audio path on a separate `Engine` with a sibling process model so that the misbehaving app cannot block it (which is the *real* fix, and changes the whole supervisor topology).

The architect has not engaged with this and the design is unsound without engagement.

### 2.3 SIGSEGV chaining is racing with Wasmtime's own handler

The architect's design: install a chaining `sa_sigaction` that writes one byte to a self-pipe, then calls `PREV_SIGSEGV`'s `sa_sigaction` (Wasmtime's). This is the wrong order.

**Wasmtime's SIGSEGV handler semantics:** Wasmtime calls `sigaction(SIGSEGV, ..., &oldact)` during `Engine::new()`. It expects its handler to be called *first* on a fault, so that it can decide whether the fault is a legitimate WASM guard-page hit (which it translates to a trap and resumes) or a real fault (which it forwards to the saved `oldact`). The protocol is well-documented and assumes a stack of handlers where Wasmtime is on top.

**Architect's proposal violates this in two ways:**

1. **Install order.** If the supervisor installs *after* Wasmtime (which is what `PREV_SIGSEGV: OnceLock<sigaction>` implies — you save Wasmtime's prior handler), then on a SIGSEGV, the supervisor handler runs *first*. The supervisor writes to a self-pipe, then forwards to Wasmtime's handler. But Wasmtime needs its handler to be the *first* responder for guard-page resolution — running supervisor code (even one byte to a self-pipe) before Wasmtime has decided "is this a WASM trap?" is observable as a syscall and can disturb signal-stack state.

2. **Forwarding semantics.** Saving `sa_sigaction` from `oldact` and calling it manually is only correct if the saved handler does not rely on the `sigaction` *flags* being set up by the kernel (sigreturn frame, sigaltstack, etc.). Wasmtime's handler *does* rely on sigaltstack because guard-page faults can occur on a near-overflowed stack. A manual call from a separate `sa_sigaction` does not preserve sigaltstack semantics correctly across the chain.

**Correct design:**

- The supervisor must install its SIGSEGV handler *before* `Engine::new()`. Then `sigaction(..., &oldact)` in Wasmtime saves the supervisor's handler. Wasmtime handles its WASM guard-page traps; on a real fault, *Wasmtime* forwards to the supervisor's saved handler via the standard `oldact` chain. The supervisor never needs to call Wasmtime's handler.
- The supervisor's handler must be `SA_SIGINFO | SA_ONSTACK | SA_NODEFER` and only do async-signal-safe work: write to a self-pipe and `_exit(128 + signo)` or longjmp out via `sigsetjmp`. It must not touch any non-AS-safe state.
- The current proposal of "minimal sa_handler writes self-pipe byte + aborts" is fine *only* if it runs after Wasmtime (i.e., as the `oldact` Wasmtime calls), which is the opposite of what `PREV_SIGSEGV` implies.

The architect must either (a) re-document the install order so that the supervisor's handler is what Wasmtime saves as its `oldact`, or (b) explain why calling Wasmtime's handler from supervisor code is safe across stack-overflow scenarios.

**Additional concern: signal mask and `SA_NODEFER`.** Wasmtime's handler installs with `SA_NODEFER` so that nested guard-page faults during handler execution are re-entrant. If the supervisor's chaining handler does *not* also use `SA_NODEFER`, the second SIGSEGV during handler dispatch is delivered with the signal masked, and the process aborts with SIGSEGV out of `pthread_sigmask` rather than being handled. The architect's "minimal sa_handler writes 1 byte then aborts" pattern works for the supervisor's own faults but does not interoperate with Wasmtime's guard-page recovery if installed wrong.

**Test plan for this issue:** Construct a WASM app that exhausts its stack via deep recursion. Wasmtime's guard-page mechanism should trap this into `WasmTrap::HostStackOverflow` (recoverable). With the architect's design as written, verify the trap is actually recovered. With the install order wrong, the SIGSEGV will be observed by the supervisor's handler first, the supervisor will abort, stage1 will restart PID-2, and *all apps die*. This is a one-line WASM app that DoSes the entire system.

### 2.4 Stage1 → PID-2 restart corrupts persistent and in-flight state

When PID-2 crashes and stage1 restarts it within 200 ms (per the Round 1 spec), the following state is left in inconsistent forms:

**1. `/data/crashes/` directory (9P, persistent).**

- A crash report TOML write that was mid-`O_APPEND` when PID-2 died is now a truncated file. Without `fsync` + atomic rename pattern, the file may contain a partial section header or a UTF-8 split. The new PID-2 must enumerate `/data/crashes/` and reject corrupted entries on startup.
- The retention bound (200 reports / 50 MiB) is enforced by which process? If the dying PID-2 was mid-eviction, the new PID-2 must re-scan and re-enforce. The architect does not specify this.
- Worse: if the crash that killed PID-2 was itself triggered by a crash-report write (recursive trap), the next PID-2 incarnation will see that crash report on disk and may attempt to read it during its own initialization, triggering the same fault.

**2. In-flight IPC envelopes.**

- VyomaOS uses FIFO + Unix socket IPC. When PID-2 dies, the FIFOs themselves are kernel inodes on tmpfs; their in-kernel buffers contain bytes from before the crash. The new PID-2 cannot know whether those bytes are: (a) a complete envelope, (b) a half-written envelope from a now-dead app, or (c) a complete envelope but addressed to an app the new supervisor does not know about yet.
- The design must specify: on restart, *drain and discard* every FIFO buffer before resuming routing, *or* re-cycle every FIFO inode (unlink + recreate).

**3. timerfd registrations.**

- Per the design, `timerfd_create` is called per timer. The fds are owned by PID-2. When PID-2 dies, the kernel closes them and the timers vanish. After restart, every app's TimerManager state is empty. The architect's `CallbackQueue` is also empty. But the app's *intent* (it scheduled a 5-second watchdog tick) is gone from supervisor memory.
- Apps that survive a PID-2 restart (because they live in Wasmtime stores in PID-2 itself? — wait, no, they die too because the process is killed) ... this is the deeper problem: PID-2 owns every Wasmtime store. When PID-2 restarts, every app dies. So this concern is moot.
- *But* — the design implies stage1 restarts PID-2 to recover from PID-2 panics, e.g., a supervisor-side `unwrap()`. If every app dies on PID-2 restart, then PID-2 panic recovery is equivalent to *system crash*. The architect must say this explicitly: stage1 PID-2 restart is a *cold reboot of all apps*, not a "soft recovery." If the panic was caused by a single subsystem, the `PanicPolicy::Restart` should have caught it before the process died.

**4. Wasmtime per-engine epoch state.**

- Each `Engine` has its own epoch counter. After PID-2 restart, the new Engine starts at epoch 0. This is fine *if* every Store also dies (which it does, because the process died). The concern is non-issue here, but the architect should note it.

**Required fix:**

- Document explicitly that PID-2 restart = full app re-spawn (no app-state preservation).
- Specify the `/data/crashes/` recovery scan on PID-2 startup: validate each TOML, quarantine corrupted ones to `/data/crashes/corrupt/`, do not auto-load any crash report for processing.
- Specify FIFO drain-on-restart for every IPC FIFO before any app is re-spawned.
- Specify atomic crash-report writes: write to `/data/crashes/.tmp.<uuid>`, `fsync`, `rename` to final name, `fsync` parent dir. Without this, the post-crash forensic data is corrupted exactly when you need it.

**Additional restart-loop hazard:** If PID-2 panics during startup (e.g., crash-recovery scan itself trips on a malformed crash report), stage1 restarts within 200 ms, PID-2 panics again on the same file, infinite loop. The crash-recovery scan must be:

- **Fast-failable.** Each crash report parse is wrapped in `catch_unwind` and `Result`; an Err quarantines the file to `corrupt/` and moves on.
- **Bounded.** Maximum scan time at startup is 5 seconds; if not done, defer the rest to a background task post-app-spawn.
- **Idempotent.** Re-running the scan must produce the same quarantine decisions.

Stage1 should additionally implement a "panic-loop detector": if PID-2 dies more than N=5 times within 30 seconds, stage1 boots into a "safe mode" supervisor configuration that disables crash-recovery scan entirely and emits a diagnostic banner. The architect does not specify this; without it, a single bad crash report bricks the system.

### 2.5 `CallbackQueue` priority-eviction-on-overflow inverts safety

The design: bounded 256-capacity queue per app, 8 priority levels, overflow evicts lowest-priority entries to make room.

**Failure scenario:** A key-repeat burst (200 events/sec from a stuck key, repeated for 2 seconds = 400 events of priority=Input=2) plus a normal trickle of Timer=7 (audio heartbeat, every 6 ms) fills the queue. The watchdog Timer fires at second 5 — but the queue is full of Input. The watchdog Timer is priority=7 (lowest), so it gets evicted.

**Consequences:**

- The watchdog tick is silently dropped. The supervisor's per-app watchdog logic now sees "no tick received" and *kills the app* — even though the app was responsive and processing input correctly. The app is killed because its supervisor-side heartbeat timer was evicted from the supervisor-side queue.
- On a robotics-rt app with a 1 kHz control-loop timer (Timer=7), every Input burst can drop control-loop ticks. Loss of control-loop ticks on a robot is a safety event.
- The priority encoding has the highest-priority numbers as *lowest priority* (Timer=7) by the architect's listing. This is confusingly opposite to most schedulers (where 0 is lowest) — at minimum, rename.

**Required fix:**

- Watchdog and control-loop timers must be on a *separate* high-priority queue that does not contend with input/IPC. A unified 256-slot queue cannot satisfy both flood-control and deadline guarantees.
- Overflow must apply backpressure to the *producer*, not silently evict from the *consumer side*. For input, this means dropping at the evdev intake (which is fine; bursts above 200 Hz are user error), not after the input is already in the queue.
- Coalescing must happen at producer side (mouse motion: keep only latest; key-repeat: keep only count, not individual events). The architect's open question #6 acknowledges this; the answer is "yes, mandatory."
- The capacity 256 must be platform-tuned. On mcu-minimal, 32 slots is plenty and 256 is wasteful. On desktop-full with 10 apps, 256 may be enough or may need to be 1024. Bake into platform profile.

**Concrete safety scenario the architect must address:** A `robotics-rt` app runs a 1 kHz control loop driving servo motors. It has a 5 Hz watchdog Timer (priority=Timer=7, lowest under the architect's scheme). A buggy IPC peer floods the robot app with 5000 envelopes/sec (priority=Ipc=4, middle). The 256-slot queue fills with IPC envelopes. Watchdog Timer callbacks at priority=7 are evicted. The supervisor-side watchdog observes "no heartbeat from robot app in 200 ms" and kills it via SIGTERM. The robot's servos lose their PWM signal and fail to a default position — *during operation*. This is a safety event caused entirely by the queue's eviction policy. The architect must redesign before this lands.

### 2.6 DWARF symbolication at crash time is unsafe and unbounded

The architect proposes that the crash report writer reads the WASM binary, parses its DWARF debug info, and produces a symbolicated backtrace into a TOML file under `/data/crashes/`. This runs inside the supervisor process, inside the crash-recovery path.

**Why this is dangerous:**

1. **DWARF parser allocation.** Symbolication libraries (`gimli`, `addr2line`, `wasm-tools`) allocate substantially — tens of MiB for a moderately-sized WASM binary. At trap time, the supervisor heap may be near-OOM, the app's memory was just freed but not necessarily returned to the OS, and additional large allocations can fail. The crash reporter itself can crash with OOM.

2. **9P stall.** Reading the `.wasm` binary from `/data` (or wherever it's mounted) goes through 9P. 9P stalls are observable in practice. The crash reporter blocks the supervisor's normal event loop while waiting.

3. **`.wasm` file may be gone.** During an OTA update (Round 8 OTA A/B slot), the previous `.wasm` file may have been removed from the active slot or rotated. Symbolication against a stale binary produces incorrect frames.

4. **Symbolicating in-process leaks fault state.** If symbolication itself panics (gimli has known issues with malformed DWARF), the panic occurs in the supervisor process during the crash-report path. This is a recursive fault.

**Required fix:**

- **At trap time, write the raw crash record only:** trap kind, faulting WASM pc (file offset), code-section offsets of the last N frames, register state, last 100 IPC envelopes. This is bounded-size and uses no heap allocation beyond pre-allocated buffers.
- **Out-of-process symbolication.** A separate `vyoma-crashd` app (which can be one of the WASM apps under the supervisor, *or* a normal Linux helper spawned by stage1) reads the raw records from `/data/crashes/raw/`, performs symbolication, and writes the human-readable TOML to `/data/crashes/`. If symbolication fails, the raw record is still preserved.
- **Pre-compute the symbol map at install time.** When a WASM app is installed, derive a `.symbols` sidecar file (function offsets → names + source locations) and store it next to the `.wasm`. At crash time, the supervisor uses the pre-computed map, not live DWARF parsing. This is cheap and bounded.

The architect's open question #8 (crash report disk write under ENOSPC/ENOMEM) is correct to raise — but the answer is "do not do allocating work at trap time," not "handle ENOSPC."

### 2.7 `PanicPolicy` table loses compile-time completeness

The architect's `run_subsystem(name: &str, f: impl FnOnce() -> R)` with a `name → PanicPolicy` lookup. Two problems:

1. **`&str` keys with runtime lookup.** A new subsystem added in a future patch has no entry. The lookup falls back to `PanicPolicy::Fatal` (or worse, `PanicPolicy::Degrade`). The choice of fallback is a policy decision but is invisible at the call site; there is no compile error if a subsystem is undeclared.
2. **No `#[must_use]` on the policy.** Even if a developer adds a subsystem, they might add it to the table but mis-spell the name at the call site. The string mismatch is a runtime fault discoverable only when that subsystem panics.

**Required fix:**

```rust
#[derive(Copy, Clone, Debug)]
enum Subsystem {
    EventLoop,
    Ipc,
    Display,
    Input,
    Timer,
    Crash,
    OtaPoll,
    Worker(IidKind),
    // ...
}

const fn policy(s: Subsystem) -> PanicPolicy {
    match s {
        Subsystem::EventLoop => PanicPolicy::Fatal,
        Subsystem::Ipc => PanicPolicy::Restart,
        Subsystem::Display => PanicPolicy::Degrade,
        Subsystem::Input => PanicPolicy::Restart,
        Subsystem::Timer => PanicPolicy::Restart,
        Subsystem::Crash => PanicPolicy::Degrade,
        Subsystem::OtaPoll => PanicPolicy::Degrade,
        Subsystem::Worker(_) => PanicPolicy::Restart,
    }
}

fn run_subsystem<R>(s: Subsystem, f: impl FnOnce() -> R + UnwindSafe) -> Result<R, PanicErr> {
    let pol = policy(s); // const-fn, compile-time
    // ... catch_unwind, then act on pol
}
```

The compiler now enforces that every `Subsystem` variant has a policy. Adding a new subsystem without choosing a policy is a non-exhaustive match error. The architect must adopt this or equivalent.

### 2.8 mcu-minimal has no preemption story at all

The proposal centers on Wasmtime epoch interruption. mcu-minimal uses **wasm3**, not Wasmtime. The differences are not minor:

- wasm3 is an interpreter; it has no JIT-emitted epoch check sites.
- wasm3 exposes `m3_AbortRuntime(runtime)` for cancellation, but cancellation is checked only at instruction dispatch, not preemptively.
- wasm3 has no `Config::epoch_interruption(true)` equivalent.
- The Round 1 / Round 9 specs put MCU runtime overhead at <128 KB RAM total; a 1 kHz signal-based ticker is plausible but burns ~1% CPU just on interrupt entry/exit on Cortex-M4 (where SysTick is the only sub-millisecond timer).

**What the design must specify for mcu-minimal:**

- Cancellation path: at every `n` wasm3 instructions, call `m3_AbortRuntime` if a deadline is past. `n` must be calibrated so cancellation latency is <100 µs (MCU control-loop budget).
- Signal handling: MCU has no `signalfd`. The design must use either a bare-metal interrupt vector (HardFault, MemManage) chained to a panic handler, *or* the architect must declare that signals do not exist on mcu-minimal and exception handling is via bare-metal traps.
- `timerfd`: MCU has no Linux `timerfd`. Use the platform's hardware timer + SysTick. Tier into a unified `Timer` trait that has Linux + bare-metal implementations.
- `CallbackQueue`: 256 capacity is too large; recommend 16.
- `EventLoop`: there is no `epoll` on mcu-minimal. The design must specify the cooperative scheduler used on bare-metal.

The architect's design as written is *desktop-only* despite claiming multi-platform applicability. mcu-minimal needs a sibling design or an explicit deferral.

**A `Runtime` trait that hides the difference is not enough.** The architect's Round 9 introduced `WasmRuntime` trait with Wasmtime and wasm3 adapters. That trait can abstract "load module, get function, call function." It cannot abstract:

- Epoch interruption (wasm3 has no equivalent — needs a different mechanism behind the same trait).
- Trap kind enumeration (wasm3 traps differ from Wasmtime traps in granularity).
- Stack overflow detection (wasm3 stacks are interpreter-managed, not guard-page-protected).

The synthesis must either extend `WasmRuntime` with a `cancel(deadline: Instant)` method that each adapter implements (Wasmtime sets store deadline; wasm3 instruments instruction-count cancellation), or document that the interrupt subsystem has runtime-specific compilation paths gated by `cfg`.

### 2.9 Audio callback latency budget is unmet

The architect does not state the audio callback latency budget anywhere. For `mobile` and `desktop-full` audio, a missed callback = audio glitch. The hard budget at 48 kHz / 256 frames is 5.33 ms; p99 must be ≤4 ms to absorb scheduling jitter.

**Question the design must answer:** When the audio app's WASM `audio_callback` function needs to run, what is the wall-clock path from "PCM buffer half-empty interrupt" to "WASM `audio_callback` begins executing"?

1. ALSA/JACK-equivalent fires a `pollfd` ready event.
2. Single `EventLoop` thread receives it via `mio::Poll`.
3. EventLoop pushes to the audio app's `CallbackQueue` at priority `Input=2`.
4. Audio app's *worker thread* (SCHED_FIFO 50, per design) dequeues.
5. Worker calls `Func::call`.
6. Wasmtime epoch check at function entry confirms no pending interrupt.
7. WASM `audio_callback` runs.

Steps 2–4 cross threads twice (EventLoop → worker). With `mio::Poll` waking on a single thread shared with input/IPC/SIGCHLD/timer/thermal events, the EventLoop can be in the middle of dispatching a 10-ms IPC handler when the audio interrupt fires. Worst-case latency is **10+ ms**, double the budget.

The single-EventLoop bottleneck (issue 2.1) is the cause. The audio thread needs either:

- A dedicated `TimerLoop` with the audio app's worker thread directly woken from it (no EventLoop hop), or
- The audio app's worker thread itself blocks on its own `pollfd` set (a per-app mini-EventLoop for audio apps with the right capability).

The architect must commit to a number and a measurement plan, or the design fails on a stated platform target.

**Measurement plan the synthesis should specify:**

1. Synthetic audio benchmark: an `audio-bench` app that records `clock_monotonic` timestamps at every callback entry, computes inter-callback jitter histogram, dumps to `/data/audio-bench.csv`.
2. Run with a "cold" system (only audio + supervisor) — establishes the floor.
3. Run with a "loaded" system (audio + 5 other apps doing typical work: shell, file browser, network app). p99 should be ≤4 ms.
4. Run with "stress" (an adversary app doing the `memory.copy` trick from issue 2.2). p99 with stress should still be ≤6 ms (graceful degradation, no glitch-class missed frames).
5. Pass criterion goes into the test harness; failure blocks release.

Without quantitative budgets and CI enforcement, the design's audio claims are aspirational only.

### 2.10 Soft yield race (architect's open question 7) is real and worse than stated

The architect identified the `kill_on_next_trap` flag race between `after_call` and `before_call`. But the race is more fundamental: the flag is checked at trap time, but a soft-yielded WASM frame may *resume* (not trap) and run for another 1 ms before the next epoch check. During that 1 ms, the supervisor believes the iid is yielded and may make scheduling decisions inconsistent with reality.

**Required fix:** Use a per-iid atomic state machine with explicit transitions:

```rust
enum IidState {
    Running,
    SoftYieldPending, // set by EpochTicker; observed at next backedge
    SoftYielded,      // set by Wasmtime trap handler after yield-trap
    HardKillPending,  // set by supervisor on policy violation
    Dead,
}
```

Transitions must be CAS-only. Supervisor scheduling decisions must read state *and* hold a per-iid lock that blocks Wasmtime resume. The current design with a single `bool kill_on_next_trap` cannot encode `SoftYieldPending` vs `SoftYielded` and admits the race.

### 2.11 Crash report budget interacts badly with `/data` quota

`/data/crashes/` retention: 200 reports, 50 MiB. But `/data` is the *single shared persistent volume* (per Round 4 filesystem). If `/data` is 100 MiB total and the user's documents are 70 MiB, the crash directory has only 30 MiB headroom — less than 50 MiB. The bound is unenforceable without coordinating with the filesystem quota.

**Required fix:**

- Crash directory should be a separate quota'd subtree, enforced by the supervisor on every write.
- On quota exhaustion: do *not* silently drop new crash reports; rotate aggressively (keep the most recent N regardless of total size; keep one sticky "user-marked" report).
- The architect's open question #5 about a sticky-flag for user-inspected reports is right; the answer is yes, plus quota-aware rotation.

---

## 3. NON-BLOCKING concerns (address in synthesis if time permits)

### 3.1 Per-platform epoch tick rate

The architect's open question #2: 1 kHz on MCU is expensive. Yes — and on mcu-minimal it doesn't even make sense (wasm3, no epoch). On desktop-full, 1 kHz is fine; on robotics-rt where 1 kHz is the *control loop*, the epoch ticker fighting the control loop for SysTick is a layering violation. Per-platform tick rates:

| Platform | Tick rate | Notes |
|---|---|---|
| mcu-minimal | N/A | wasm3 has no epoch; use instruction-count cancel |
| iot-edge | 100 Hz | 10 ms preemption granularity is fine for IoT |
| robotics-rt | 10 kHz | Must be faster than the control loop |
| mobile | 1 kHz | Audio period ~6 ms; 1 kHz is the minimum |
| desktop-full | 1 kHz | Default |
| server-headless | 100 Hz | Long-running workloads, batch-style |

Bake into platform profile TOML.

### 3.2 Timer quota 64 per iid

Architect's open question #4. For a robot with 50+ sensors, 64 is borderline. Recommend per-platform quotas:

- mcu-minimal: 8
- iot-edge: 32
- robotics-rt: 256 (sensors are timer-driven)
- mobile / desktop-full: 64
- server-headless: 128

### 3.3 `signalfd` for SIGCHLD reaping ordering

`signalfd` delivers SIGCHLD as a `signalfd_siginfo` struct with the child PID. The architect must specify whether the EventLoop *reaps* (`waitpid`) immediately upon `signalfd` notification, or queues a callback into a "reap" priority bucket. If queued, reap latency contributes to the time between a wasmtime child crash and the supervisor noticing. Recommend: reap inline in the SignalLoop thread (post-split), update iid state atomically, *then* enqueue notification callbacks.

### 3.4 `SA_RESTART` on inherited signal mask

If the supervisor inherits an `SA_RESTART` mask from stage1 (which it should not, but if), `read()` on the self-pipe transparently restarts after SIGCHLD. Must explicitly `sigaction(..., flags = 0)` (no `SA_RESTART`) on the self-pipe write side, or use a non-blocking self-pipe.

### 3.5 Crash report contents — last 100 IPC envelopes

200 reports × 100 envelopes × ~256 bytes/envelope = 5 MiB just for IPC history. Fine within the 50 MiB budget, but means IPC history must be a fixed-size ring buffer (not allocated per envelope) so the cost is bounded at runtime as well. Architect does not specify.

### 3.6 Worker thread priorities and inheritance

Per-app worker threads at SCHED_FIFO 50. But the EventLoop runs at... what? Not specified. If EventLoop is SCHED_OTHER, every wakeup of a SCHED_FIFO worker is preceded by waking the EventLoop, which is on a lower-priority class. Priority inversion via the wake path. Recommend the EventLoop (and any per-event-source loops post-split) run at SCHED_FIFO 49 (just below worker priority) so they preempt SCHED_OTHER but yield to workers.

### 3.7 `mio::Poll` vs `epoll_wait` directly

`mio` adds a thin layer but also overhead per wakeup (~1 µs). For audio-critical paths, raw `epoll_wait` may be preferable. Non-blocking; mention as a tuning knob.

### 3.8 Crash report TOML format — schema versioning

Crash reports are persistent and may be read by future supervisor versions. The TOML must include a schema version field. Architect does not specify. Recommend: `schema = "vyomaos-crash-v1"` at the top of every report.

### 3.9 Trap during epoch-yield resume

If the soft-yield resume itself traps (e.g., guard-page hit on the very next instruction), what happens to the `IidState::SoftYieldPending`? Must be drained to `Dead` via the trap path, not left dangling.

### 3.10 Cross-platform `evdev` absence

evdev is Linux-specific. On mcu-minimal (bare-metal) and possibly robotics-rt (which may use Xenomai or PREEMPT_RT-on-Linux), the input source differs. The `EventLoop` design hard-codes evdev as a source. Need an abstraction layer (`InputSource` trait) and platform-specific implementations.

### 3.11 File-budget pressure

The architect's 10 files under `supervisor/src/interrupt/`, each <500 LOC, total budget 5000 LOC. Realistic accounting:

- `event_loop.rs`: 400 LOC (only single-loop). Split into four files (input_loop, timer_loop, signal_loop, general_loop) at ~250 LOC each = 1000 LOC after split.
- `callback_queue.rs`: 350 LOC (with coalescing, multi-band).
- `signal.rs`: 300 LOC.
- `epoch.rs`: 250 LOC + per-runtime adapter ~150 LOC = 400 LOC.
- `timer.rs`: 350 LOC.
- `crash.rs`: needs split into `crash_raw.rs` (in-process, no-alloc) and `crash_symbols.rs` (offline symbolicator) ~500 LOC total.
- `panic_recovery.rs`: 200 LOC.
- `worker.rs`: 300 LOC.
- `types.rs`: 200 LOC.
- `mod.rs`: 100 LOC.

Total ~4200 LOC, within budget after the EventLoop split. The synthesis should re-tabulate.

### 3.12 Observability hooks

Every event handler should emit a structured log line (per the existing `observability/` subsystem from Round 9) on dispatch, with iid + event type + queue depth + dispatch latency. This is essential for diagnosing audio glitches and watchdog kills in production. The architect does not mention observability integration; it is the difference between "I can't reproduce" and "here is the iid + timestamp + queue state at the moment of failure."

### 3.13 `EpochTicker` cancellation on shutdown

When PID-2 shuts down cleanly (SIGTERM from stage1 during planned restart), the `EpochTicker` SCHED_FIFO 50 thread must be joined. SCHED_FIFO threads do not respond to `pthread_cancel` cleanly. The architect must specify an explicit `AtomicBool` shutdown flag the ticker polls between ticks, and a `join_timeout` of 100 ms before stage1 escalates to SIGKILL.

---

## 4. What the architect got right

### 4.1 Centralizing event demultiplexing via `mio::Poll`

The *idea* of one event-demux abstraction is correct — `mio::Poll` is the right primitive on Linux. The mistake is making it single-threaded for *all* sources. Split per source category, keep `mio::Poll` per loop. The skeleton survives; the threading model needs work.

### 4.2 `signalfd` over `sa_handler` for non-fatal signals

Using `signalfd` for SIGTERM/INT/PIPE/USR1/USR2/CHLD/HUP is correct and superior to async-signal handlers. It avoids all async-signal-safety constraints for these signals, and queueing semantics are well-defined. Keep this.

### 4.3 Per-iid worker thread

A dedicated worker per app instance is the right model — it isolates WASM execution from the supervisor's I/O path, allows per-iid SCHED_FIFO priority, and makes trap handling local. The cross-thread callback enqueue from EventLoop → worker is the right pattern. The bug is the queue's overflow behavior (issue 2.5), not the thread topology.

### 4.4 `PanicPolicy::{Degrade, Restart, Fatal}` trichotomy

The three-state policy is the right abstraction; the implementation via stringly-typed lookup is the mistake (issue 2.7). Adopt the const-fn match version and this section is good.

---

## 5. Questions for synthesis

1. **Is the EventLoop split (issue 2.1) accepted?** If yes, the synthesis must specify the four-loop topology (Input, Timer, Signal, General) and their respective SCHED_FIFO priorities. If no, synthesis must justify why single-thread is sufficient *with measurements*, not assertions.

2. **What is the audio latency budget commitment?** Synthesis must state the p99 audio-callback wakeup latency target (suggest: ≤4 ms at 48 kHz / 256 frames) and reference a measurement plan in the test strategy.

3. **What is the WASM bulk-op CPU monopolization bound (issue 2.2)?** Synthesis must specify the per-platform memory cap that ensures worst-case `memory.copy` latency stays below the platform's preemption budget.

4. **mcu-minimal preemption story (issue 2.8)?** Either include a wasm3 cancellation design or explicitly defer with rationale.

5. **SIGSEGV handler install order (issue 2.3)?** State explicitly: "supervisor handler installed before `Engine::new()`; Wasmtime saves supervisor's handler as its `oldact`; no manual forwarding from supervisor handler."

6. **`/data/crashes/` recovery semantics (issue 2.4)?** Specify the PID-2 startup scan, corrupted-report quarantine, FIFO drain, atomic write protocol.

7. **CallbackQueue redesign (issue 2.5)?** Confirm: producer-side coalescing + backpressure, separate high-priority queue for watchdog/control-loop timers, no consumer-side eviction.

8. **DWARF symbolication out-of-process (issue 2.6)?** Confirm install-time symbol-map precompute and out-of-process crash-reporter daemon design.

9. **`PanicPolicy` const-fn match (issue 2.7)?** Confirm adoption of compile-time exhaustive policy mapping.

10. **Priority encoding direction?** Today: Terminate=0 (highest), Timer=7 (lowest). Confusing; recommend reversing or using named priorities (`Priority::Critical`, `Priority::High`, ...) without numeric leakage.

11. **Worker thread priority for `EventLoop` (issue 3.6)?** State: EventLoop SCHED_FIFO 49, workers SCHED_FIFO 50.

12. **Per-platform epoch tick rates and timer quotas (issues 3.1, 3.2)?** Bake into platform profile TOML; do not hard-code in supervisor source.

13. **IPC history ring-buffer size (issue 3.5)?** Specify per-iid ring buffer size for last-100-envelopes preservation and confirm it fits the per-platform memory budget.

14. **Soft-yield state machine (issue 2.10)?** Confirm adoption of multi-state atomic transition (Running / SoftYieldPending / SoftYielded / HardKillPending / Dead).

15. **Crash directory quota interaction with `/data` quota (issue 2.11)?** State the relationship: separate quota'd subtree, supervisor-enforced, with sticky-bit support.

---

## Closing assessment

The design has the right *structure* — central event demux, per-app workers, three-tier panic policy, signalfd, epoch-based cooperative preemption — but the *specifics* are not safe for VyomaOS's stated multi-platform targets. The single-threaded EventLoop is the dominant defect: it cannot meet audio latency on `mobile` and cannot meet control-loop deadlines on `robotics-rt`. The epoch-only preemption story is oversold and silently breaks on bulk memory ops, hostcalls, and wasm3-based mcu-minimal. The SIGSEGV chaining is in the wrong direction relative to Wasmtime's expected install order. The CallbackQueue overflow policy is unsafe under sustained input.

These are not synthesizable nits — they require structural redesign of the threading model and an explicit accounting of which platforms the design serves and which it does not.

If the synthesis commits to:

1. EventLoop split into four loops with documented priorities,
2. Audio latency budget with measurement,
3. Per-platform memory caps to bound `memory.copy` worst case,
4. SIGSEGV install-order fix,
5. CallbackQueue producer-side coalescing + separate high-priority bus for watchdog/control-loop timers,
6. Out-of-process symbolication + install-time symbol maps,
7. PID-2 restart recovery semantics for `/data/crashes/` and IPC FIFOs,
8. wasm3 cancellation path (or explicit deferral) for mcu-minimal,
9. `PanicPolicy` const-fn match,

then the round can move to FINAL. Without these, the design is a desktop-only sketch that will fail in `mobile` audio and `robotics-rt` control loops the first time they are exercised.

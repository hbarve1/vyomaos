# Round 10 — Interrupt & Exception Handling

**Role:** Architect
**Date:** 2026-05-29
**Status:** Proposal (awaiting Critic review)
**Scope:** Unified interrupt, signal, trap, panic, and timer subsystem for the VyomaOS supervisor and the WASM apps it hosts. Covers WASM trap → CrashKind mapping, Linux signal handling at PID-2 and stage1, hardware interrupt delivery via a single epoll-based EventLoop, WIT callback queueing for apps, Wasmtime epoch interruption as the canonical CPU preemption mechanism, panic recovery in supervisor subsystems, crash reporting with DWARF symbolicated WASM backtraces, and a per-app TimerManager backed by `timerfd_create(CLOCK_MONOTONIC, ...)`.

---

## 0. Executive summary

Interrupts and exceptions in a conventional Unix kernel are a tangle of separate mechanisms: hardware IRQs handled by ISRs in ring 0, POSIX signals delivered to user-space via the kernel signal mask, page-fault exceptions handled by `mm/fault.c`, software interrupts (`softirq`, `tasklet`), and per-process `SIGCHLD`/`waitpid` reaping. The XNU equivalent layers Mach exception ports, BSD signals, IOKit interrupt event sources, and the CFRunLoop on top. Both stacks took 30+ years to reach steady state, and both still leak abstractions: Linux process death by `SIGKILL` looks different from death by `SIGSEGV`, and macOS Mach exception ports interact with BSD signals via the `ux_exception_handler` trampoline in confusing ways.

VyomaOS does not need most of that machinery. WASM apps have **no native signal concept** — they cannot be interrupted asynchronously in the POSIX sense. Wasmtime gives us exactly two asynchronous-ish primitives: **epoch interruption** (a cooperative preemption point checked at every loop backedge and function call) and **trap unwinding** (a synchronous error that unwinds the guest stack and returns control to the host). Everything that looks asynchronous from the WASM guest's perspective — a timer firing, an input event arriving, an IPC message landing — is actually delivered as a **WIT callback** that the supervisor invokes on a host-managed thread *between* WASM instructions.

This gives us a radically simpler model:

1. **Below the supervisor**, the kernel delivers events through file descriptors (`/dev/input/event*`, `timerfd`, `signalfd`, `gpiochip*`, netlink, epoll). A single `EventLoop` thread owns the epoll set and demultiplexes everything.
2. **Inside the supervisor**, Rust panics are caught at well-defined subsystem boundaries (`catch_unwind`), and POSIX signals to the supervisor itself are funnelled through `signalfd` into the same `EventLoop` — no async-signal-safety hazards.
3. **Above the supervisor**, every app sees a single priority-ordered `CallbackQueue` of WIT calls. The supervisor's per-app worker thread dequeues callbacks, enters the WASM store, invokes the export, handles any resulting trap, and loops.
4. **Wasmtime epoch interruption** is the *only* CPU preemption mechanism. It serves three roles: (a) cooperative time-slicing under SCHED_DEADLINE, (b) hard CPU-limit enforcement, (c) cancellation of stuck callbacks. We distinguish **soft** epoch interrupts (reschedule, do not crash) from **hard** epoch kills (terminate the iid).

The macOS analogue here is the *combination* of: XNU Mach exception ports (for trap delivery from kernel to user-space exception handler task), the BSD signal layer (for `SIGSEGV`, `SIGTERM` to processes), IOKit's `IOInterruptEventSource` (for hardware interrupt delivery to user-space drivers), and CoreFoundation's `CFRunLoop` (for run-loop event demultiplexing). VyomaOS collapses all four into one `EventLoop` + `CallbackQueue` pair, because we have only one process model (WASM-in-Wasmtime) and one execution model (cooperative per-app workers).

Key constraints honoured:

- **Capability secure.** Apps only receive callbacks for events they declared in `vyoma.toml`. No `on-input` if `display=false`; no `on-gpio` if no pins declared.
- **No async-signal-safety footguns.** Real POSIX signal handlers do nothing but `write(self_pipe, &sig, 1)`; everything else runs in `EventLoop`.
- **500-line file limit.** Subsystem split into seven files under `supervisor/src/interrupt/`.
- **Stage1 owns PID-1.** PID-2 (supervisor) crashes via SIGSEGV are *expected* and recoverable; stage1 catches `SIGCHLD`, runs the restart policy from R8, and re-spawns PID-2 within 200 ms.
- **Deterministic crash forensics.** Every WASM trap produces a TOML crash report under `/data/crashes/` containing the trap kind, WASM backtrace (DWARF symbolicated when debug info present), the last 100 IPC envelopes the app saw, and the resident-set memory at crash time.

End-state: a WASM app calling `core::ptr::null::<u8>().read()` produces `WasmTrap::HeapOutOfBounds`, which maps to `CrashKind::MemoryFault`, which writes `/data/crashes/2026-05-29T13:42:11Z-tic-tac-toe-iid-7.toml`, which the package manager surfaces in its "Recent Crashes" UI, and which restarts the app per its `[restart] policy = "on-failure"` manifest field. A keyboard event from `/dev/input/event3` lands in the EventLoop's epoll, gets routed by R6's `InputDispatcher` to the focused app, gets enqueued as an `on-input` callback on that app's `CallbackQueue`, gets dequeued by the per-app worker thread, and is delivered as a WIT call **before** the next `on-timer` callback even if the timer fired earlier — because input has higher priority. All of this happens with no async-signal-safety violations, no race conditions on shutdown, and no supervisor-internal Rust panic that isn't caught and reported.

---

## 1. Design philosophy

### 1.1 The single golden rule

> **Apps never see asynchrony. Apps see a sequence of synchronous WIT callbacks delivered in priority order on one logical thread.**

This is the single most important property of the entire interrupt subsystem. WASM as a programming model is single-threaded by default, and `wasm32-wasip2` apps cannot rely on async-signal-safety or memory barriers because the WASM memory model does not expose either. So everything asynchronous in the underlying Linux kernel — interrupts, signals, hotplug events, timer firings, network packets — gets serialised into a single per-app FIFO of callback invocations, and the per-app worker thread drains that FIFO one callback at a time. From inside the WASM guest, every event is a normal function call.

The cost of this design: a callback that takes 50 ms to run holds up every other callback to that app for 50 ms. That is acceptable because the apps that need low-latency processing (audio, input, real-time control) get **dedicated** per-event worker threads via QoS (R5), and the rest run with reasonable latency budgets enforced by the watchdog (R5/R7).

### 1.2 Trap vs panic vs interrupt — three distinct concepts

VyomaOS code (and our Critic) regularly conflates these. They are three different things:

- **Trap** = a synchronous error in a WASM instance. The guest tried to do something illegal (divide by zero, out-of-bounds load, hit `unreachable`, exceeded its epoch deadline). Wasmtime detects this and returns a `Trap` error from the `Func::call` invocation. The host (us) decides what to do: log it, write a crash report, restart the iid, ignore it.
- **Panic** = a synchronous error in the *supervisor's* Rust code. Some line of supervisor code called `.unwrap()` on a `None`, or hit an `assert!(...)` that failed, or indexed past a slice end. Rust unwinds the host stack until something catches it. Without `catch_unwind`, the supervisor process dies of an unhandled panic and stage1 sees a `SIGCHLD` with abnormal exit code.
- **Interrupt** = an asynchronous event from outside the supervisor's normal execution flow. A POSIX signal sent by `kill`, a hardware event from the kernel, a timer firing. These never *interrupt* the supervisor's CPU directly the way kernel ISRs interrupt user-space; they sit waiting in a kernel queue (signalfd, epoll, timerfd) until the EventLoop polls.

Round 10 handles all three, but the mechanisms are completely different. Treating them uniformly (the way XNU's exception ports tried to) is a known anti-pattern; we keep them distinct on purpose.

### 1.3 Why not async Rust?

We considered building the entire subsystem on `tokio`. Reasons we chose `mio` + dedicated threads instead:

1. **Wasmtime stores are `!Sync`.** A WASM instance must be entered from exactly one thread at a time. Tokio task-stealing on a multi-threaded runtime is wrong for us.
2. **Determinism.** A single-threaded event loop with hand-rolled priority queues is easier to reason about and unit-test than a sea of futures, especially for the Critic.
3. **No allocator churn on the hot path.** Every `tokio::spawn` allocates a future on the heap. Our per-app worker is a tight loop over a `VecDeque<Callback>`.
4. **R8 boot-phase ordering.** Initialising tokio inside `BootPhase::EventLoop` and tearing it down inside `Shutdown::Phase4_TaskDrain` is plausible but more code than `mio::Poll::new()` plus seven explicit `join_handle.join()`s.

We use `mio` (the underlying epoll abstraction tokio is built on) directly, plus standard library threads.

### 1.4 The "epoch as preemption" insight

Wasmtime epoch interruption is described in the upstream docs as a *cooperative* timeout: every WASM function entry and every loop backedge implicitly compares a per-instance deadline against a globally shared `epoch` counter, and traps if the deadline is exceeded. The global counter is bumped by some external clock (we run a dedicated thread doing `engine.increment_epoch()` every N ms).

That description undersells the mechanism. **Epoch interruption is the closest thing WASM has to a hardware timer interrupt.** It is the only way to forcibly stop a runaway computation inside a WASM call without killing the entire host process. We exploit it for:

- **Time-slicing.** Every per-app worker thread sets a deadline of `current_epoch + slice_epochs` before calling into the guest. When the deadline is hit, the trap propagates back to the worker, which checks: "Did I want this app to yield? Yes → reschedule. No → escalate to a hard kill."
- **CPU quota.** The R5 cgroup `cpu.max` budget is enforced *also* via epoch counts. If an app burns through its quota mid-callback, the next epoch bump traps it.
- **Stuck-callback cancellation.** If an `on-input` callback takes longer than 250 ms, the watchdog requests a hard epoch kill on the iid. The current callback unwinds with `Trap::Interrupt`, the iid is restarted per policy.

The clean distinction between soft (reschedule) and hard (kill) epoch interrupts is implemented by a 1-bit flag the worker sets on the `IidEpochState` before calling the guest: "if the next trap arrives, treat it as kill, otherwise treat it as yield." See §6.

### 1.5 Critique of the macOS model

XNU's exception model has a known design flaw: Mach exception messages are delivered to a task's "exception port", and if no handler is registered, they fall back to BSD signal delivery. This means a single program can have its trap intercepted by *three* different layers — its own `mach_msg` handler, then a parent process's exception port, then a BSD signal handler — with non-obvious interleaving. Apple's own `<sys/sysctl.h>` documentation contradicts itself about which takes precedence.

We take the opposite approach: exactly **one** layer decides what happens to a trap. The per-app worker thread that called `Func::call` receives the `Trap` error directly, looks up the iid's restart policy, and acts. There is no second-chance handler, no parent-process inheritance of exception state. This makes the model boring but predictable.

### 1.6 Layering picture

```
┌─────────────────────────────────────────────────────────────┐
│  WASM apps  (wasm32-wasip2 in Wasmtime stores)               │
│   guest sees only synchronous WIT calls + return values      │
└─────────────────────────────────────────────────────────────┘
                       │  WIT host callback dispatch
                       ▼
┌─────────────────────────────────────────────────────────────┐
│  Per-app worker thread  (one per iid)                        │
│   loop { callback = queue.pop(); run(callback); }            │
│   wraps each guest call in `catch_unwind`                    │
│   handles `Trap` errors, escalates to CrashWriter            │
└─────────────────────────────────────────────────────────────┘
                       │  CallbackQueue (priority FIFO)
                       ▼
┌─────────────────────────────────────────────────────────────┐
│  CallbackDispatch  (lock-free MPSC, one per iid)             │
│   producers: TimerManager, IpcBroker, InputDispatcher, ...   │
│   consumer:  the per-app worker                              │
└─────────────────────────────────────────────────────────────┘
                       │  enqueue(Callback)
                       ▼
┌─────────────────────────────────────────────────────────────┐
│  EventLoop  (single thread, owns epoll)                      │
│   demuxes timerfd, signalfd, evdev, netlink, gpiochip,       │
│   thermal pipes, IPC sockets; dispatches to producers        │
└─────────────────────────────────────────────────────────────┘
                       │  epoll_wait → fd
                       ▼
┌─────────────────────────────────────────────────────────────┐
│  Linux kernel  (interrupt sources)                           │
│   /dev/input/event*, timerfd_create, signalfd,               │
│   /dev/gpiochip*, /sys/class/thermal/*/temp, IPC FIFOs       │
└─────────────────────────────────────────────────────────────┘
```

Note the inversion compared to a traditional OS: there is no per-app *receive* loop running inside the guest. Apps are dormant between callbacks. The EventLoop is the only "loop"; everything else is callback execution.

---

## 2. The WASM exception model

### 2.1 Wasmtime trap taxonomy

Wasmtime's `wasmtime::Trap` enum (as of 22.0.0) has the following synchronous trap codes that can fire from inside a `Func::call`:

```rust
// supervisor/src/interrupt/wasm_trap.rs
//
// Mirror of wasmtime::Trap, but defined in terms VyomaOS can persist.
// The wasmtime enum is opaque (it may grow) so we wrap it.

use std::time::Duration;

/// Every trap that a wasm32-wasip2 guest can raise to its host.
///
/// Mirrors `wasmtime::Trap` but with VyomaOS-specific semantics
/// for the variants we treat differently (epoch interruption is split
/// into recoverable and unrecoverable).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WasmTrap {
    /// Guest stack grew past the configured maximum (default 1 MiB).
    /// Recoverable: restart the iid; the app is probably looping.
    StackOverflow,

    /// Linear memory access outside the allocated heap region.
    /// This is the WASM equivalent of SIGSEGV.
    HeapOutOfBounds {
        /// Address the guest tried to access (in WASM linear memory).
        addr: u64,
        /// Length of the access.
        len: u32,
    },

    /// Misaligned access on a memory model that required alignment
    /// (rare for wasm32, but legal under the `relaxed-simd` proposal).
    HeapMisaligned {
        addr: u64,
        required: u32,
    },

    /// `call_indirect` index out of table bounds.
    TableOutOfBounds {
        table_index: u32,
        entry: u32,
        table_size: u32,
    },

    /// `call_indirect` to a null table entry.
    IndirectCallToNull {
        table_index: u32,
        entry: u32,
    },

    /// `call_indirect` to a function with the wrong signature.
    BadSignature {
        table_index: u32,
        entry: u32,
    },

    /// Explicit `unreachable` instruction. In Rust this is what
    /// `unreachable!()` and `panic!()` ultimately compile to. We
    /// treat this as an *assertion failure* (the app intended to
    /// crash) distinct from a memory bug.
    Unreachable,

    /// `idiv` / `rem` with divisor zero.
    IntegerDivByZero,

    /// `idiv` / `rem` with `i32::MIN / -1`. Same as Rust panic.
    IntegerOverflow,

    /// `trunc_s` / `trunc_u` from a float that doesn't fit.
    BadConversionToInt,

    /// Soft epoch interrupt — the worker requested a yield. NOT a
    /// crash. Worker reschedules the callback.
    EpochYield,

    /// Hard epoch kill — watchdog requested termination, or the
    /// app exceeded its CPU quota. Treated as CrashKind::CpuLimit.
    UnrecoverableEpochInterrupt {
        deadline_ms: u64,
        consumed_ms: u64,
        reason: EpochKillReason,
    },

    /// A host function panicked (caught by `catch_unwind` and
    /// turned into a trap). The String is the panic payload.
    HostTrap(String),

    /// Wasmtime ran out of fuel. We don't use fuel mode in
    /// production, but include for completeness so test code
    /// can synthesise it.
    OutOfFuel,

    /// Wasmtime resource limit hit (memory growth refused,
    /// table growth refused).
    ResourceExhausted {
        resource: ResourceKind,
        requested: u64,
        limit: u64,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EpochKillReason {
    /// CPU watchdog declared the app stuck.
    StuckCallback,
    /// cgroup cpu.max budget exhausted (R5).
    QuotaExhausted,
    /// Thermal governor SIGSTOP'd the app for too long (R7).
    ThermalThrottle,
    /// Operator explicitly killed via `kill <name>` (R8 shutdown).
    OperatorKill,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResourceKind {
    LinearMemory,
    Table,
    Stack,
}
```

### 2.2 Mapping to CrashKind

R1 established the 12 `CrashKind` variants. The mapping is exhaustive and lossless:

```rust
// supervisor/src/interrupt/wasm_trap.rs (cont.)

impl WasmTrap {
    pub fn to_crash_kind(&self) -> CrashKind {
        use WasmTrap::*;
        match self {
            StackOverflow          => CrashKind::StackOverflow,
            HeapOutOfBounds { .. } => CrashKind::MemoryFault,
            HeapMisaligned { .. }  => CrashKind::MemoryFault,
            TableOutOfBounds { .. }=> CrashKind::ControlFlowFault,
            IndirectCallToNull{..} => CrashKind::ControlFlowFault,
            BadSignature { .. }    => CrashKind::ControlFlowFault,
            Unreachable            => CrashKind::AssertionFailure,
            IntegerDivByZero       => CrashKind::ArithmeticFault,
            IntegerOverflow        => CrashKind::ArithmeticFault,
            BadConversionToInt     => CrashKind::ArithmeticFault,
            EpochYield             => unreachable!("EpochYield is not a crash"),
            UnrecoverableEpochInterrupt { .. } => CrashKind::CpuLimit,
            HostTrap(_)            => CrashKind::HostPanic,
            OutOfFuel              => CrashKind::CpuLimit,
            ResourceExhausted { resource: ResourceKind::LinearMemory, .. } =>
                CrashKind::MemoryLimit,
            ResourceExhausted { resource: ResourceKind::Table, .. } =>
                CrashKind::ResourceLimit,
            ResourceExhausted { resource: ResourceKind::Stack, .. } =>
                CrashKind::StackOverflow,
        }
    }

    /// True if this trap can be ignored / retried without
    /// restarting the iid. Only EpochYield qualifies today.
    pub fn is_recoverable(&self) -> bool {
        matches!(self, WasmTrap::EpochYield)
    }

    /// True if this trap is the app's own fault (we should
    /// blame it) vs. an environment fault (host OOM, etc.).
    pub fn is_app_fault(&self) -> bool {
        !matches!(self,
            WasmTrap::HostTrap(_) |
            WasmTrap::UnrecoverableEpochInterrupt {
                reason: EpochKillReason::ThermalThrottle, ..
            }
        )
    }
}
```

### 2.3 Converting wasmtime::Trap to WasmTrap

The conversion happens in the per-app worker right after `Func::call` returns an error:

```rust
// supervisor/src/interrupt/wasm_trap.rs (cont.)

use wasmtime::{Trap as WtTrap, TrapCode};

impl From<wasmtime::Error> for WasmTrap {
    fn from(e: wasmtime::Error) -> WasmTrap {
        // Wasmtime 22 returns errors as anyhow::Error; we downcast.
        if let Some(t) = e.downcast_ref::<WtTrap>() {
            return WasmTrap::from(*t);
        }
        // Anything else is an unexpected host error — treat as HostTrap.
        WasmTrap::HostTrap(format!("{e:#}"))
    }
}

impl From<WtTrap> for WasmTrap {
    fn from(t: WtTrap) -> WasmTrap {
        match t {
            WtTrap::StackOverflow         => WasmTrap::StackOverflow,
            WtTrap::MemoryOutOfBounds     => WasmTrap::HeapOutOfBounds { addr: 0, len: 0 },
            WtTrap::HeapMisaligned        => WasmTrap::HeapMisaligned { addr: 0, required: 0 },
            WtTrap::TableOutOfBounds      => WasmTrap::TableOutOfBounds { table_index: 0, entry: 0, table_size: 0 },
            WtTrap::IndirectCallToNull    => WasmTrap::IndirectCallToNull { table_index: 0, entry: 0 },
            WtTrap::BadSignature          => WasmTrap::BadSignature { table_index: 0, entry: 0 },
            WtTrap::IntegerOverflow       => WasmTrap::IntegerOverflow,
            WtTrap::IntegerDivisionByZero => WasmTrap::IntegerDivByZero,
            WtTrap::BadConversionToInteger=> WasmTrap::BadConversionToInt,
            WtTrap::UnreachableCodeReached=> WasmTrap::Unreachable,
            WtTrap::Interrupt             => WasmTrap::EpochYield,
            WtTrap::OutOfFuel             => WasmTrap::OutOfFuel,
            // Wasmtime may add more variants — treat unknown as HostTrap.
            _                             => WasmTrap::HostTrap(format!("unmapped wasmtime trap: {t:?}")),
        }
    }
}
```

Note the impedance issue: Wasmtime's `Trap` doesn't include the address/length of an OOB access; that information lives in the `Trace` field of the `wasmtime::Error`. The conversion above stubs the address as 0; the per-app worker fills it in from the trace before emitting the crash report. See §7.

### 2.4 Trap vs panic distinction inside the WASM guest

A subtle point: Rust code compiled to `wasm32-wasip2` uses `panic = "abort"` by default for the embedded toolchain. A `panic!()` in guest code therefore compiles to a `core::intrinsics::abort()` call, which lowers to the WASM `unreachable` instruction. From our host's point of view, every guest panic looks like `WasmTrap::Unreachable`. There is no way to distinguish "the app called `unreachable!()` intentionally" from "the app called `option.unwrap()` on a `None`" except by looking at the stack trace (and even then only when DWARF is present).

We accept this. The `CrashKind::AssertionFailure` covers both cases. The crash report includes the WASM PC and (if available) the symbolicated source location, which is usually enough to distinguish.

---

## 3. Supervisor signal handling

### 3.1 The signals the supervisor receives

PID-2 (the supervisor) runs as a Linux process and can receive POSIX signals from:

- Stage1 (PID-1): `SIGTERM` on orderly shutdown, `SIGKILL` if PID-2 is unresponsive past 5s.
- Operators: `kill -USR1 $(pidof supervisor)` for a diagnostic dump.
- The kernel: `SIGSEGV` on internal memory corruption, `SIGBUS` on misaligned access, `SIGILL` on a bad instruction, `SIGPIPE` on writing to a closed pipe.
- Wasmtime: `SIGALRM` is *not* used by upstream Wasmtime (epoch is driven by a thread, not a timer signal) but we MUST NOT install a handler that would mask it in case a future Wasmtime version changes.

Stage1 receives a different set:
- `SIGCHLD` when PID-2 dies → R8 restart logic.
- `SIGTERM` from systemd/init when the entire VM shuts down → cascade to children.
- `SIGINT` if running in dev mode under a terminal.

### 3.2 The signalfd-only rule

Real `sa_handler` functions run in async-signal context, which forbids almost every libc call (`malloc`, `printf`, mutex acquisition). Real-world signal-handling bugs are almost always async-signal-safety violations. We rule them out by policy:

> **The supervisor installs `SIG_IGN` or `SIG_DFL` for every signal it cares about, and reads the actual delivery via `signalfd(2)` from the EventLoop.**

This makes signal handling completely indistinguishable from "another fd became readable in the epoll set". No `volatile sig_atomic_t` globals, no `pthread_kill`, no jmp_buf gymnastics.

The exception is `SIGSEGV`/`SIGBUS`/`SIGILL`, which cannot be `signalfd`'d because they re-arm if not handled (the kernel sends them on every retry of the faulting instruction). For these we install a minimal `sa_handler` that writes the signal number to a private self-pipe and calls `abort()`. The EventLoop sees the byte on the self-pipe just before the abort, logs a "PID-2 crashing" line, then the abort terminates the process and stage1 takes over.

### 3.3 SignalHandler type

```rust
// supervisor/src/interrupt/signal.rs

use std::io::Write;
use std::os::unix::io::{AsRawFd, RawFd};
use libc::{c_int, sigaction, sigemptyset, sigfillset, sigset_t, SA_RESTART, SIG_BLOCK, SIG_IGN};

/// The set of signals the supervisor (PID-2) blocks at startup so
/// they can be drained from a signalfd instead of running async
/// handlers.
const BLOCKED_SIGNALS: &[c_int] = &[
    libc::SIGTERM,
    libc::SIGINT,
    libc::SIGPIPE,
    libc::SIGUSR1,
    libc::SIGUSR2,
    libc::SIGCHLD, // wasmtime child processes (if we ever spawn any)
    libc::SIGHUP,
];

/// SIGSEGV/SIGBUS/SIGILL cannot be signalfd'd. They get a tiny
/// handler that writes one byte to a self-pipe and aborts.
const FATAL_SIGNALS: &[c_int] = &[
    libc::SIGSEGV,
    libc::SIGBUS,
    libc::SIGILL,
    libc::SIGFPE,  // shouldn't happen in Rust, but cheap to handle
];

pub struct SignalHandler {
    /// The signalfd that delivers BLOCKED_SIGNALS.
    pub signal_fd: RawFd,
    /// The read end of the self-pipe for FATAL_SIGNALS.
    pub fatal_pipe_rx: RawFd,
}

impl SignalHandler {
    /// Install all signal handling. Called early in BootPhase::Stage1Handoff.
    /// SAFETY: must be called exactly once, before any thread other than
    /// the main thread exists, because sigprocmask is per-thread on Linux.
    pub unsafe fn install() -> std::io::Result<Self> {
        // 1. Block the signals we want to receive via signalfd.
        let mut mask: sigset_t = std::mem::zeroed();
        sigemptyset(&mut mask);
        for &sig in BLOCKED_SIGNALS {
            libc::sigaddset(&mut mask, sig);
        }
        if libc::pthread_sigmask(SIG_BLOCK, &mask, std::ptr::null_mut()) != 0 {
            return Err(std::io::Error::last_os_error());
        }

        // 2. Open a signalfd for them.
        let signal_fd = libc::signalfd(-1, &mask, libc::SFD_NONBLOCK | libc::SFD_CLOEXEC);
        if signal_fd < 0 {
            return Err(std::io::Error::last_os_error());
        }

        // 3. Set up the self-pipe for fatal signals.
        let mut pipefd = [0i32; 2];
        if libc::pipe2(pipefd.as_mut_ptr(), libc::O_CLOEXEC | libc::O_NONBLOCK) != 0 {
            return Err(std::io::Error::last_os_error());
        }
        let fatal_pipe_rx = pipefd[0];
        FATAL_PIPE_TX.store(pipefd[1], std::sync::atomic::Ordering::SeqCst);

        // 4. Install the sa_handler for each fatal signal.
        for &sig in FATAL_SIGNALS {
            let mut act: sigaction = std::mem::zeroed();
            act.sa_sigaction = fatal_handler as usize;
            act.sa_flags = libc::SA_SIGINFO | libc::SA_RESETHAND;
            sigfillset(&mut act.sa_mask);
            if sigaction(sig, &act, std::ptr::null_mut()) != 0 {
                return Err(std::io::Error::last_os_error());
            }
        }

        // 5. Ignore SIGPIPE *globally*. We catch broken-pipe errors
        //    at every write site explicitly.
        let mut act: sigaction = std::mem::zeroed();
        act.sa_sigaction = SIG_IGN;
        sigaction(libc::SIGPIPE, &act, std::ptr::null_mut());

        Ok(SignalHandler { signal_fd, fatal_pipe_rx })
    }

    /// Read pending signals out of the signalfd. Called from EventLoop
    /// when the signal_fd becomes readable.
    pub fn drain(&self) -> Vec<SignalEvent> {
        let mut out = Vec::with_capacity(8);
        let mut buf: libc::signalfd_siginfo = unsafe { std::mem::zeroed() };
        let size = std::mem::size_of::<libc::signalfd_siginfo>();
        loop {
            let n = unsafe {
                libc::read(self.signal_fd, &mut buf as *mut _ as *mut _, size)
            };
            if n == size as isize {
                out.push(SignalEvent::from_siginfo(&buf));
            } else {
                break;
            }
        }
        out
    }
}

static FATAL_PIPE_TX: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(-1);

/// Async-signal-safe SIGSEGV/SIGBUS/SIGILL/SIGFPE handler. Writes
/// the signal number into the self-pipe (so the EventLoop *might*
/// see it) and aborts. After SA_RESETHAND, the next occurrence of
/// the signal will use SIG_DFL.
extern "C" fn fatal_handler(
    sig: c_int,
    _info: *mut libc::siginfo_t,
    _ctx: *mut libc::c_void,
) {
    let fd = FATAL_PIPE_TX.load(std::sync::atomic::Ordering::SeqCst);
    if fd >= 0 {
        let byte = sig as u8;
        unsafe { libc::write(fd, &byte as *const u8 as *const _, 1); }
    }
    // No printf, no malloc — straight to abort.
    unsafe { libc::abort(); }
}

#[derive(Debug, Clone)]
pub enum SignalEvent {
    Term { from_pid: u32 },
    Interrupt,
    Pipe,                  // someone wrote to a broken pipe
    DiagnosticDump,        // SIGUSR1
    LogRotate,             // SIGUSR2
    ChildExit { pid: u32, status: i32 },
    Hangup,
}

impl SignalEvent {
    fn from_siginfo(s: &libc::signalfd_siginfo) -> Self {
        match s.ssi_signo as c_int {
            libc::SIGTERM => SignalEvent::Term { from_pid: s.ssi_pid },
            libc::SIGINT  => SignalEvent::Interrupt,
            libc::SIGPIPE => SignalEvent::Pipe,
            libc::SIGUSR1 => SignalEvent::DiagnosticDump,
            libc::SIGUSR2 => SignalEvent::LogRotate,
            libc::SIGCHLD => SignalEvent::ChildExit {
                pid: s.ssi_pid,
                status: s.ssi_status,
            },
            libc::SIGHUP  => SignalEvent::Hangup,
            _ => unreachable!("unknown signal in signalfd: {}", s.ssi_signo),
        }
    }
}
```

### 3.4 Stage1 signal handling

Stage1 is a separate, tiny binary (R8). It only cares about `SIGCHLD` (PID-2 died) and `SIGTERM` (whole VM is shutting down). Stage1 *can* use a synchronous sa_handler because it doesn't do anything async-signal-unsafe in the handler — it just sets a flag and `waitpid()`s in its main loop.

```rust
// supervisor/src/interrupt/signal.rs (cont.) — stage1 portion

// Stage1's main loop pseudocode:
//
//     let supervisor_pid = spawn_supervisor()?;
//     loop {
//         let mut status: i32 = 0;
//         let dead = libc::waitpid(supervisor_pid, &mut status, 0);
//         if dead == supervisor_pid {
//             log_supervisor_exit(status);
//             apply_restart_policy(status)?;
//             supervisor_pid = spawn_supervisor()?;
//         }
//     }
//
// No signal handlers needed; waitpid blocks until SIGCHLD wakes it.
```

### 3.5 Interaction with Wasmtime signal usage

Wasmtime uses signals internally for its memory-fault catching path: when the guest does an OOB load, the host's mmap'd guard page faults with SIGSEGV, Wasmtime's `signal_handler` catches it, and the trap is converted into a `Trap::MemoryOutOfBounds` returned from `Func::call`. **We must not interfere with this.**

Concretely:
- We do NOT install our own SIGSEGV handler that processes the fault — we install one that only fires for *unrelated* SIGSEGVs (i.e., bugs in our Rust code, not WASM OOB).
- Wasmtime's `Config::signals_based_traps(true)` is the default. We leave it on.
- The Wasmtime handler is installed via `sigaction(SIGSEGV, ...)` with `SA_SIGINFO`. Our `fatal_handler` is installed *after* Wasmtime initialises (because we initialise SignalHandler in BootPhase::Stage1Handoff and Wasmtime in BootPhase::WasmRuntime which is later).

Wait — there's the ordering problem. Wasmtime's signal handler installation happens lazily (the first time `Engine::new()` runs). If we install ours first and Wasmtime installs second, Wasmtime *chains* to ours via `oldact`. If we install second, we'd overwrite theirs and break OOB trap delivery. Solution: we **chain** explicitly. When installing `fatal_handler`, we save the old `sigaction` and call `oldact.sa_sigaction(sig, info, ctx)` first from our handler. This is the standard pattern.

```rust
// Stored chain: { signo → previous sigaction }
static PREV_SIGSEGV: std::sync::OnceLock<sigaction> = std::sync::OnceLock::new();

extern "C" fn fatal_handler(sig: c_int, info: *mut libc::siginfo_t, ctx: *mut libc::c_void) {
    if sig == libc::SIGSEGV {
        if let Some(prev) = PREV_SIGSEGV.get() {
            // Forward to Wasmtime's handler first.
            let f: extern "C" fn(c_int, *mut libc::siginfo_t, *mut libc::c_void)
                = unsafe { std::mem::transmute(prev.sa_sigaction) };
            f(sig, info, ctx);
            // If Wasmtime handled it (e.g., OOB → trap), we never get here.
            // If we get here, the SEGV was outside Wasmtime's purview → abort.
        }
    }
    // ... write to pipe, abort ...
}
```

Both Wasmtime's handler and ours coexist via the chain.

---

## 4. The EventLoop

### 4.1 Responsibilities

The `EventLoop` is the single thread that owns the supervisor's epoll set. It is the choke point for every asynchronous input the supervisor cares about. Its responsibilities:

1. Maintain the epoll set (`Poll` instance from `mio`).
2. Drain ready fds in priority order.
3. Dispatch each event to its handler (a `Producer`).
4. Wake up sleeping producers (HID parser, thermal poller) at their tick intervals.
5. Track per-fd statistics for observability.
6. Shut down cleanly when stage1 sends SIGTERM.

The EventLoop does **not** itself run any business logic. It only demultiplexes. Business logic — "this evdev event is for the focused app" — runs in the producer thread the EventLoop dispatches to.

### 4.2 Data model

```rust
// supervisor/src/interrupt/event_loop.rs

use mio::{Events, Interest, Poll, Token};
use std::collections::HashMap;
use std::os::unix::io::RawFd;
use std::sync::{Arc, RwLock};
use std::time::Duration;

pub struct EventLoop {
    poll: Poll,
    events: Events,

    /// Token → source. Lookup table for what to do when an fd fires.
    sources: HashMap<Token, EventSource>,

    /// Monotonically increasing token allocator.
    next_token: u32,

    /// When set, the next iteration of the loop should clean up and exit.
    shutdown: Arc<std::sync::atomic::AtomicBool>,

    /// Per-source statistics for observability.
    stats: HashMap<Token, EventSourceStats>,
}

/// Everything the EventLoop knows how to wait for.
pub enum EventSource {
    /// Signalfd from §3.
    SignalFd { handler: Arc<SignalHandler> },

    /// Timerfd associated with an iid timer (§8).
    Timer { iid: IidId, timer_id: TimerId },

    /// evdev /dev/input/eventN.
    InputDevice {
        path: std::path::PathBuf,
        parser: Arc<RwLock<EvdevParser>>,
    },

    /// udev netlink for hotplug.
    UdevNetlink { mgr: Arc<DeviceManager /* from R6 */> },

    /// /dev/gpiochipN line event from R9.
    GpioLineEvent { chip: u8, line: u32, iid: IidId },

    /// Thermal pipe from R7's ThermalGovernor.
    ThermalPipe { gov: Arc<ThermalGovernor> },

    /// IPC socket — one fd per app, but receive side only.
    IpcReceive { iid: IidId, sock: RawFd },

    /// Self-pipe for fatal signals from §3.
    FatalPipe { sig_pipe: RawFd },

    /// The watchdog interval timerfd (§8.4).
    WatchdogTick,
}

pub struct EventSourceStats {
    pub events_count: u64,
    pub last_seen_at: std::time::Instant,
    pub avg_dispatch_latency_us: f64,
}
```

### 4.3 The main loop

```rust
impl EventLoop {
    pub fn new(shutdown: Arc<std::sync::atomic::AtomicBool>) -> std::io::Result<Self> {
        Ok(Self {
            poll: Poll::new()?,
            events: Events::with_capacity(128),
            sources: HashMap::new(),
            next_token: 1,
            shutdown,
            stats: HashMap::new(),
        })
    }

    /// Register a new source. Returns its Token for later removal.
    pub fn register(
        &mut self,
        source_fd: RawFd,
        interest: Interest,
        source: EventSource,
    ) -> std::io::Result<Token> {
        let token = Token(self.next_token as usize);
        self.next_token += 1;

        // mio wants a SourceFd wrapper for raw fds.
        let mut sf = mio::unix::SourceFd(&source_fd);
        self.poll.registry().register(&mut sf, token, interest)?;

        self.sources.insert(token, source);
        self.stats.insert(token, EventSourceStats {
            events_count: 0,
            last_seen_at: std::time::Instant::now(),
            avg_dispatch_latency_us: 0.0,
        });
        Ok(token)
    }

    pub fn deregister(&mut self, token: Token, fd: RawFd) -> std::io::Result<()> {
        let mut sf = mio::unix::SourceFd(&fd);
        self.poll.registry().deregister(&mut sf)?;
        self.sources.remove(&token);
        self.stats.remove(&token);
        Ok(())
    }

    /// Run forever. Returns Err only on a fatal mio failure (epoll fd
    /// closed underneath us). Returns Ok on clean shutdown.
    pub fn run(&mut self, ctx: Arc<EventLoopContext>) -> std::io::Result<()> {
        // Poll timeout: 100 ms keeps stat updates fresh and lets us
        // observe the shutdown flag promptly.
        let poll_timeout = Some(Duration::from_millis(100));

        while !self.shutdown.load(std::sync::atomic::Ordering::SeqCst) {
            self.poll.poll(&mut self.events, poll_timeout)?;

            // Snapshot events.events() into a Vec so we can iterate
            // by *priority* rather than by readiness order. This is
            // critical: input must beat IPC must beat timer must
            // beat thermal, regardless of which fd became ready
            // first.
            let mut ready: Vec<&mio::event::Event> = self.events.iter().collect();
            ready.sort_by_key(|e| self.priority_of(e.token()));

            for evt in ready {
                let token = evt.token();
                let now = std::time::Instant::now();
                if let Some(source) = self.sources.get(&token) {
                    self.dispatch(source, evt, &ctx);
                }
                if let Some(stat) = self.stats.get_mut(&token) {
                    stat.events_count += 1;
                    stat.last_seen_at = now;
                }
            }
        }
        // Clean shutdown.
        for (token, _) in self.sources.drain() {
            tracing::debug!("EventLoop draining token {:?}", token);
        }
        Ok(())
    }

    fn priority_of(&self, token: Token) -> u8 {
        match self.sources.get(&token) {
            Some(EventSource::FatalPipe { .. })     => 0,
            Some(EventSource::SignalFd { .. })      => 1,
            Some(EventSource::InputDevice { .. })   => 2,
            Some(EventSource::GpioLineEvent { .. }) => 3,
            Some(EventSource::IpcReceive { .. })    => 4,
            Some(EventSource::UdevNetlink { .. })   => 5,
            Some(EventSource::Timer { .. })         => 6,
            Some(EventSource::WatchdogTick)         => 7,
            Some(EventSource::ThermalPipe { .. })   => 8,
            None                                     => 255,
        }
    }

    fn dispatch(
        &self,
        source: &EventSource,
        evt: &mio::event::Event,
        ctx: &EventLoopContext,
    ) {
        // The dispatch is intentionally non-blocking. The producer
        // does whatever cheap work it can and then enqueues into
        // CallbackQueues. Heavy work (DWARF symbolication, crash
        // writing) lives elsewhere.
        match source {
            EventSource::SignalFd { handler } => {
                for sig in handler.drain() {
                    ctx.handle_signal(sig);
                }
            }
            EventSource::Timer { iid, timer_id } => {
                ctx.timer_manager.handle_fire(*iid, *timer_id);
            }
            EventSource::InputDevice { parser, .. } => {
                parser.write().unwrap().drain_into(&ctx.input_dispatcher);
            }
            EventSource::UdevNetlink { mgr } => {
                mgr.handle_uevent_ready();
            }
            EventSource::GpioLineEvent { chip, line, iid } => {
                ctx.hal_registry.drain_gpio(*chip, *line, *iid);
            }
            EventSource::ThermalPipe { gov } => {
                gov.handle_ready();
            }
            EventSource::IpcReceive { iid, sock } => {
                ctx.ipc_broker.handle_readable(*iid, *sock);
            }
            EventSource::FatalPipe { sig_pipe } => {
                // Try to log before stage1 sees us go away.
                let mut buf = [0u8; 8];
                let _ = unsafe { libc::read(*sig_pipe, buf.as_mut_ptr() as *mut _, 8) };
                tracing::error!(signal = buf[0], "PID-2 hit fatal signal; aborting shortly");
            }
            EventSource::WatchdogTick => {
                ctx.watchdog.tick();
            }
        }
    }
}

pub struct EventLoopContext {
    pub timer_manager: Arc<TimerManager>,
    pub input_dispatcher: Arc<InputDispatcher /* R6 */>,
    pub hal_registry: Arc<HalRegistry /* R9 */>,
    pub ipc_broker: Arc<IpcBroker>,
    pub watchdog: Arc<Watchdog /* R5/R7 */>,
}

impl EventLoopContext {
    fn handle_signal(&self, sig: SignalEvent) {
        match sig {
            SignalEvent::Term { from_pid } => {
                tracing::info!(from_pid, "received SIGTERM; initiating shutdown");
                // R8: cascade through Shutdown::Phase0_AcceptNoNew → Phase1_Quiesce → ...
                self.watchdog.request_shutdown();
            }
            SignalEvent::Interrupt => {
                // In production, treat as SIGTERM. In dev, drop into a repl.
                self.watchdog.request_shutdown();
            }
            SignalEvent::Pipe => {
                tracing::debug!("got SIGPIPE; ignored");
            }
            SignalEvent::DiagnosticDump => self.dump_state(),
            SignalEvent::LogRotate => self.rotate_logs(),
            SignalEvent::ChildExit { pid, status } => {
                // We don't spawn child processes any more (wasmtime is in-process)
                // but log it for forensics.
                tracing::warn!(pid, status, "unexpected SIGCHLD");
            }
            SignalEvent::Hangup => {
                tracing::info!("SIGHUP received; reloading config");
                // Reload /etc/vyoma/boot.toml hot — defer to R8 logic.
            }
        }
    }

    fn dump_state(&self) {
        // Print all in-flight callbacks, all app states, all HAL bindings.
        // Intentionally verbose: this is the post-mortem button.
        tracing::info!("=== DIAGNOSTIC DUMP (SIGUSR1) ===");
        self.input_dispatcher.dump();
        self.timer_manager.dump();
        self.ipc_broker.dump();
        self.hal_registry.dump();
        tracing::info!("=== END DUMP ===");
    }

    fn rotate_logs(&self) {
        // Tell the structured logger to reopen /data/log/supervisor.log.
        tracing::info!("rotating logs (SIGUSR2)");
        // Defer to R1 logging subsystem.
    }
}
```

The 100 ms poll timeout strikes a balance between latency and idle CPU usage. With one app blocked on `on-input`, the latency from key press to dispatch is the evdev fd readability time (~microseconds) plus the EventLoop drain (~microseconds) plus the callback queue wake (~microseconds): well under 1 ms in practice.

### 4.4 Why prioritise dispatch order

Without priority, an unlucky burst on the thermal pipe could starve input events for a polling interval. With it, input events always get processed first in any given epoll wakeup, and a key press that arrived 10 µs *after* a thermal reading still gets dispatched first in the same iteration. This matters for interactive responsiveness; it does not matter for throughput because the EventLoop loops at well above the kernel's wake-up rate.

---

## 5. The CallbackQueue

### 5.1 Per-iid mailbox

Every running app instance (iid, in R1 terminology) has a single `CallbackQueue`. Producers (timer manager, IPC broker, input dispatcher, HAL, power manager) push `Callback` values onto it. The per-app worker pops them.

```rust
// supervisor/src/interrupt/callback_queue.rs

use std::collections::BinaryHeap;
use std::sync::{Arc, Condvar, Mutex};
use std::time::Instant;

/// One callback to invoke on an app. The discriminant determines
/// which WIT export gets called; the payload is the argument.
#[derive(Debug, Clone)]
pub enum Callback {
    Timer {
        timer_id: TimerId,
        elapsed_ms: u64,
        enqueued_at: Instant,
    },
    Ipc {
        envelope: IpcEnvelope,
        enqueued_at: Instant,
    },
    Input {
        event: InputEvent /* R6 */,
        enqueued_at: Instant,
    },
    DeviceEvent {
        event: DeviceEvent /* R6 */,
        enqueued_at: Instant,
    },
    Gpio {
        pin: u8,
        value: bool,
        enqueued_at: Instant,
    },
    Suspend { enqueued_at: Instant },
    Resume { enqueued_at: Instant },
    MemoryWarning {
        level: PressureLevel,
        enqueued_at: Instant,
    },
    /// Forced shutdown notification. The worker delivers this and
    /// then refuses to dequeue anything else.
    Terminate {
        reason: TerminateReason,
        enqueued_at: Instant,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerminateReason {
    ManifestReload,
    OperatorRequest,
    ParentShutdown,
    PolicyViolation,
    Crash,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PressureLevel {
    Normal,
    Warning,
    Critical,
}

impl Callback {
    pub fn priority(&self) -> u8 {
        // Lower is higher priority.
        match self {
            Callback::Terminate { .. }      => 0,
            Callback::Suspend { .. }        => 1,
            Callback::MemoryWarning { .. }  => 2,
            Callback::Input { .. }          => 3,
            Callback::Gpio { .. }           => 3,
            Callback::Ipc { .. }            => 4,
            Callback::DeviceEvent { .. }    => 5,
            Callback::Resume { .. }         => 6,
            Callback::Timer { .. }          => 7,
        }
    }
}

/// A bounded, priority-ordered, condvar-wakeable mailbox. We do not
/// use a `crossbeam` channel because we want O(1) "is the queue
/// non-empty?" and explicit overflow policy.
pub struct CallbackQueue {
    inner: Mutex<CallbackQueueInner>,
    cvar: Condvar,
    capacity: usize,
}

struct CallbackQueueInner {
    /// Indexed by priority bucket [0..8], each bucket a Vec.
    /// We use buckets rather than a real heap because we usually
    /// have <10 callbacks pending and want to preserve FIFO order
    /// *within* a priority bucket.
    buckets: [Vec<Callback>; 8],
    total: usize,
    /// Set when the worker should stop pulling new callbacks.
    closed: bool,
    /// Stats for observability.
    stats: QueueStats,
}

#[derive(Default)]
pub struct QueueStats {
    pub enqueued_total: u64,
    pub dropped_overflow: u64,
    pub max_depth: usize,
    pub coalesced_timers: u64,
}

impl CallbackQueue {
    pub fn new(capacity: usize) -> Self {
        Self {
            inner: Mutex::new(CallbackQueueInner {
                buckets: Default::default(),
                total: 0,
                closed: false,
                stats: Default::default(),
            }),
            cvar: Condvar::new(),
            capacity,
        }
    }

    /// Enqueue. Returns false if dropped (queue full or closed).
    pub fn push(&self, cb: Callback) -> bool {
        let mut g = self.inner.lock().unwrap();
        if g.closed {
            return false;
        }
        if g.total >= self.capacity {
            // Overflow policy: drop oldest *Timer* first, then
            // oldest *DeviceEvent*, then refuse.
            if !g.drop_lowest_priority() {
                g.stats.dropped_overflow += 1;
                return false;
            }
        }
        // Coalesce: if the new callback is a Timer for the same
        // timer_id already pending in the bucket, replace it
        // (semantics: most recent fire wins).
        let prio = cb.priority() as usize;
        if let Callback::Timer { timer_id, .. } = &cb {
            if let Some(existing) = g.buckets[prio].iter_mut()
                .find(|c| matches!(c, Callback::Timer { timer_id: t, .. } if t == timer_id))
            {
                *existing = cb;
                g.stats.coalesced_timers += 1;
                self.cvar.notify_one();
                return true;
            }
        }
        g.buckets[prio].push(cb);
        g.total += 1;
        g.stats.enqueued_total += 1;
        if g.total > g.stats.max_depth { g.stats.max_depth = g.total; }
        self.cvar.notify_one();
        true
    }

    /// Pop the highest priority callback, blocking. Returns None
    /// if the queue is closed.
    pub fn pop_blocking(&self) -> Option<Callback> {
        let mut g = self.inner.lock().unwrap();
        loop {
            if g.closed && g.total == 0 {
                return None;
            }
            if let Some(cb) = g.pop_highest() {
                return Some(cb);
            }
            g = self.cvar.wait(g).unwrap();
        }
    }

    /// Pop with a timeout. Returns None on timeout or shutdown.
    pub fn pop_timeout(&self, dur: std::time::Duration) -> Option<Callback> {
        let mut g = self.inner.lock().unwrap();
        let deadline = Instant::now() + dur;
        loop {
            if g.closed && g.total == 0 { return None; }
            if let Some(cb) = g.pop_highest() { return Some(cb); }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() { return None; }
            let (gn, res) = self.cvar.wait_timeout(g, remaining).unwrap();
            g = gn;
            if res.timed_out() { return None; }
        }
    }

    pub fn close(&self) {
        let mut g = self.inner.lock().unwrap();
        g.closed = true;
        self.cvar.notify_all();
    }

    pub fn stats(&self) -> QueueStats {
        let g = self.inner.lock().unwrap();
        QueueStats {
            enqueued_total: g.stats.enqueued_total,
            dropped_overflow: g.stats.dropped_overflow,
            max_depth: g.stats.max_depth,
            coalesced_timers: g.stats.coalesced_timers,
        }
    }
}

impl CallbackQueueInner {
    fn pop_highest(&mut self) -> Option<Callback> {
        for b in &mut self.buckets {
            if !b.is_empty() {
                self.total -= 1;
                return Some(b.remove(0));
            }
        }
        None
    }

    fn drop_lowest_priority(&mut self) -> bool {
        // Walk from lowest priority bucket and drop one.
        for b in self.buckets.iter_mut().rev() {
            if !b.is_empty() {
                b.pop();
                self.total -= 1;
                return true;
            }
        }
        false
    }
}
```

### 5.2 Priority semantics defended

Why this priority order?

- **Terminate (0):** the only way an app gets a chance to clean up before being killed. Must beat everything.
- **Suspend (1):** must beat input so that an in-flight key press doesn't run after the app was supposed to suspend (R7 semantics).
- **MemoryWarning (2):** apps should drop caches BEFORE handling new input.
- **Input (3) and GPIO (3):** the things users notice. Tied because GPIO edge events in robotics are equivalently latency-sensitive.
- **IPC (4):** inter-app coordination. Must not starve input but must not be starved by background timers.
- **DeviceEvent (5):** hotplug. The user noticing latency on hot-plug is "they plugged in a thumb drive and it appeared on the desktop", which is a multi-second budget.
- **Resume (6):** symmetric with Suspend but pessimistic — let the app drain everything pending before reactivating cleanly.
- **Timer (7):** the lowest priority, because a timer is by definition something the app already knew about. Inputs the app didn't know about should always preempt.

### 5.3 Overflow policy

We use a bounded queue (default 256 callbacks) because an unbounded queue is a memory-DoS surface. When full, we drop the *lowest priority pending* callback. This is essentially "evict timers first", which matches the semantics of a real-time signal queue. If even timers are gone and a new high-priority event arrives, it pushes and the resulting drop-count goes into observability stats; the watchdog raises an alert above a configurable threshold (default 1000/sec).

### 5.4 Coalescing

For `Callback::Timer`, if a timer fires again before the worker has consumed the previous fire, we replace the pending fire with the newer one. This avoids piling up 50 stale `on-timer(id=foo, elapsed_ms=50)` callbacks if the worker was stuck for 2.5 seconds. The semantics match macOS GCD's "leeway" model.

For `Callback::Input`, we do NOT coalesce: every key press is distinct. We do NOT coalesce `Callback::Ipc` either: every message must be delivered.

For `Callback::MemoryWarning`, we coalesce by overwriting any pending one with the higher level.

---

## 6. Wasmtime epoch interruption integration

### 6.1 The epoch tick thread

A dedicated thread bumps the global epoch counter on a fixed cadence. We use 1 kHz (1 ms per epoch) by default; the platform profile can override.

```rust
// supervisor/src/interrupt/epoch.rs

use std::sync::Arc;
use std::time::{Duration, Instant};
use wasmtime::Engine;

pub struct EpochTicker {
    engine: Engine,
    tick_interval: Duration,
    shutdown: Arc<std::sync::atomic::AtomicBool>,
}

impl EpochTicker {
    pub fn spawn(engine: Engine, hz: u32, shutdown: Arc<std::sync::atomic::AtomicBool>) -> std::thread::JoinHandle<()> {
        let interval = Duration::from_nanos(1_000_000_000 / hz as u64);
        let me = EpochTicker { engine, tick_interval: interval, shutdown };
        std::thread::Builder::new()
            .name("vyoma-epoch-ticker".into())
            .stack_size(64 * 1024)
            .spawn(move || me.run())
            .expect("epoch ticker spawn")
    }

    fn run(self) {
        // Pin to a single CPU? Not necessarily — but use SCHED_FIFO
        // at very high priority so we tick on time.
        unsafe { set_sched_fifo(50); }

        let mut next = Instant::now() + self.tick_interval;
        while !self.shutdown.load(std::sync::atomic::Ordering::SeqCst) {
            std::thread::sleep(next.saturating_duration_since(Instant::now()));
            self.engine.increment_epoch();
            next += self.tick_interval;
        }
    }
}

unsafe fn set_sched_fifo(prio: i32) {
    let param = libc::sched_param { sched_priority: prio };
    let _ = libc::sched_setscheduler(0, libc::SCHED_FIFO, &param);
}
```

### 6.2 Per-iid epoch state

Each iid has an `IidEpochState` that the per-app worker manipulates before/after every guest call.

```rust
// supervisor/src/interrupt/epoch.rs (cont.)

use wasmtime::Store;

pub struct IidEpochState {
    /// True = the next trap should be treated as a hard kill.
    /// False = the next trap should be treated as a yield (reschedule).
    pub kill_on_next_trap: std::sync::atomic::AtomicBool,
    /// Wall-clock deadline that the worker requested. Used for crash
    /// report consumed_ms.
    pub started_at: std::sync::Mutex<Option<Instant>>,
    /// Cumulative consumed CPU under this iid (ms). Compared against
    /// the cgroup cpu.max budget.
    pub consumed_cpu_ms: std::sync::atomic::AtomicU64,
}

impl IidEpochState {
    /// Called by the worker just before invoking a WIT export.
    /// `slice_ms` is the maximum time we want this callback to run.
    /// `kill` is true if we want to kill on overrun (watchdog tier 2).
    pub fn before_call<T>(&self, store: &mut Store<T>, slice_ms: u64, kill: bool) {
        self.kill_on_next_trap.store(kill, std::sync::atomic::Ordering::SeqCst);
        *self.started_at.lock().unwrap() = Some(Instant::now());
        // Each epoch is 1 ms, so slice_ms epochs = slice_ms ms.
        store.set_epoch_deadline(slice_ms);
    }

    pub fn after_call<T>(&self, store: &mut Store<T>) {
        if let Some(t0) = self.started_at.lock().unwrap().take() {
            let dt = t0.elapsed().as_millis() as u64;
            self.consumed_cpu_ms.fetch_add(dt, std::sync::atomic::Ordering::Relaxed);
        }
        // Reset deadline to "never" until next call.
        store.set_epoch_deadline(u64::MAX);
    }
}
```

### 6.3 Soft yield vs hard kill in the worker

```rust
// supervisor/src/interrupt/worker.rs

fn run_callback<T>(
    cb: Callback,
    store: &mut Store<T>,
    epoch: &IidEpochState,
    engine_cfg: &EngineCfg,
) -> Result<(), WasmTrap> {
    let slice_ms = engine_cfg.slice_for(&cb);  // e.g. 250 for Input, 1000 for Timer
    let kill = engine_cfg.kill_on_overrun(&cb);
    epoch.before_call(store, slice_ms, kill);
    let result = match cb {
        Callback::Timer { timer_id, elapsed_ms, .. } => {
            invoke_on_timer(store, timer_id, elapsed_ms)
        }
        Callback::Input { event, .. } => invoke_on_input(store, event),
        // ...
    };
    epoch.after_call(store);

    match result {
        Ok(()) => Ok(()),
        Err(e) => {
            let trap = WasmTrap::from(e);
            if matches!(trap, WasmTrap::EpochYield) && !kill {
                // Soft yield: re-enqueue the callback and let other
                // apps run. The worker thread returns to its loop
                // and pops the next callback (which may be this
                // same one if nothing else is pending).
                tracing::trace!("epoch yield; rescheduling");
                Ok(())
            } else if matches!(trap, WasmTrap::EpochYield) && kill {
                // Hard kill: convert to UnrecoverableEpochInterrupt
                // with reason StuckCallback.
                Err(WasmTrap::UnrecoverableEpochInterrupt {
                    deadline_ms: slice_ms,
                    consumed_ms: epoch.consumed_cpu_ms.load(std::sync::atomic::Ordering::Relaxed),
                    reason: EpochKillReason::StuckCallback,
                })
            } else {
                Err(trap)
            }
        }
    }
}
```

### 6.4 The `kill_on_next_trap` flip

The watchdog (R5/R7) flips `kill_on_next_trap` when an iid has been past its deadline for too long. The next epoch tick after the flip causes the in-flight call to trap; the worker observes `kill=true` and treats it as `EpochKillReason::StuckCallback`. There is no race: the flip is `SeqCst` and the worker reads `kill_on_next_trap` only after the trap returns, so it always sees the latest value.

### 6.5 ThermalThrottle path

When R7 declares the system in thermal tier 2 (critical), the ThermalGovernor flips `kill_on_next_trap=true` on every non-essential iid simultaneously, then `engine.increment_epoch()` is called explicitly to force a trap *right now* even if the natural tick hasn't arrived. The workers see kill=true and emit `UnrecoverableEpochInterrupt { reason: ThermalThrottle, .. }`. The iids are then SIGSTOPped (actually, since we're in-process: their workers stop pulling from CallbackQueue) until thermal tier drops back to 1.

---

## 7. WASM stack traces and crash reports

### 7.1 Backtrace extraction

Wasmtime's `wasmtime::WasmBacktrace::capture(&store)` returns a list of `FrameInfo` items, each containing a module name, function name (if exported or named in the name section), function offset, and source location (filename + line, if DWARF debug info was packaged). We capture this *inside* the per-app worker, immediately after the trap, before the `store` is moved or dropped.

```rust
// supervisor/src/interrupt/crash_writer.rs

use wasmtime::{Store, WasmBacktrace, FrameInfo};

#[derive(Debug, Clone)]
pub struct WasmBacktraceCapture {
    pub frames: Vec<WasmFrame>,
}

#[derive(Debug, Clone)]
pub struct WasmFrame {
    pub module: String,
    pub func_name: Option<String>,
    pub func_index: u32,
    pub func_offset: u32,
    pub source_file: Option<String>,
    pub source_line: Option<u32>,
}

impl WasmBacktraceCapture {
    pub fn from_store<T>(store: &Store<T>) -> Self {
        let wb = WasmBacktrace::capture(store);
        let frames = wb.frames().iter().map(|f| WasmFrame {
            module: f.module().name().unwrap_or("(anon)").to_string(),
            func_name: f.func_name().map(str::to_string),
            func_index: f.func_index(),
            func_offset: f.func_offset() as u32,
            source_file: f.symbols().first().and_then(|s| s.file().map(str::to_string)),
            source_line: f.symbols().first().and_then(|s| s.line()),
        }).collect();
        Self { frames }
    }
}
```

### 7.2 The crash report

```rust
#[derive(Debug, Clone)]
pub struct CrashReport {
    pub iid: IidId,
    pub bundle_id: String,
    pub bundle_version: semver::Version,
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub trap: WasmTrap,
    pub crash_kind: CrashKind,
    pub backtrace: WasmBacktraceCapture,
    pub rust_backtrace: Option<String>,  // for HostTrap
    pub memory_stats: MemoryStats,
    pub recent_ipc: Vec<IpcEnvelope>,    // last 100
    pub uptime_ms: u64,
    pub consumed_cpu_ms: u64,
    pub callback_in_flight: String,      // e.g. "on-timer(id=42)"
}

#[derive(Debug, Clone)]
pub struct MemoryStats {
    pub linear_memory_pages: u32,
    pub linear_memory_bytes: u64,
    pub table_entries: u32,
    pub stack_high_water_bytes: u32,
}

impl CrashReport {
    /// Write the report to `/data/crashes/<ts>-<bundle>-<iid>.toml`.
    /// Uses `std::fs::write` not async because crashes are rare and
    /// we want the write to complete before we restart.
    pub fn write(&self, base_dir: &std::path::Path) -> std::io::Result<std::path::PathBuf> {
        std::fs::create_dir_all(base_dir)?;
        let fname = format!(
            "{}-{}-iid-{}.toml",
            self.timestamp.format("%Y-%m-%dT%H-%M-%SZ"),
            self.bundle_id,
            self.iid.0,
        );
        let path = base_dir.join(fname);
        let body = self.to_toml();
        std::fs::write(&path, body)?;
        Ok(path)
    }

    fn to_toml(&self) -> String {
        // Hand-rolled TOML emission because we want maximum stability;
        // the crash report format is a forensic artifact.
        let mut s = String::new();
        use std::fmt::Write;
        let _ = writeln!(s, "# VyomaOS crash report");
        let _ = writeln!(s, "iid       = {}", self.iid.0);
        let _ = writeln!(s, "bundle    = \"{}\"", self.bundle_id);
        let _ = writeln!(s, "version   = \"{}\"", self.bundle_version);
        let _ = writeln!(s, "timestamp = {}", self.timestamp.to_rfc3339());
        let _ = writeln!(s, "kind      = \"{:?}\"", self.crash_kind);
        let _ = writeln!(s, "trap      = \"{:?}\"", self.trap);
        let _ = writeln!(s, "uptime_ms = {}", self.uptime_ms);
        let _ = writeln!(s, "cpu_ms    = {}", self.consumed_cpu_ms);
        let _ = writeln!(s, "callback  = \"{}\"", self.callback_in_flight);
        let _ = writeln!(s);
        let _ = writeln!(s, "[memory]");
        let _ = writeln!(s, "linear_pages   = {}", self.memory_stats.linear_memory_pages);
        let _ = writeln!(s, "linear_bytes   = {}", self.memory_stats.linear_memory_bytes);
        let _ = writeln!(s, "table_entries  = {}", self.memory_stats.table_entries);
        let _ = writeln!(s, "stack_hwm      = {}", self.memory_stats.stack_high_water_bytes);
        let _ = writeln!(s);
        let _ = writeln!(s, "[[backtrace]]");
        for (i, f) in self.backtrace.frames.iter().enumerate() {
            let _ = writeln!(s, "[[backtrace.frame]]");
            let _ = writeln!(s, "index    = {}", i);
            let _ = writeln!(s, "module   = \"{}\"", f.module);
            if let Some(n) = &f.func_name {
                let _ = writeln!(s, "function = \"{}\"", n);
            }
            let _ = writeln!(s, "func_idx = {}", f.func_index);
            let _ = writeln!(s, "offset   = {}", f.func_offset);
            if let Some(file) = &f.source_file {
                let _ = writeln!(s, "source   = \"{}:{}\"", file,
                    f.source_line.unwrap_or(0));
            }
        }
        let _ = writeln!(s);
        let _ = writeln!(s, "[[recent_ipc]]");
        for env in self.recent_ipc.iter().rev().take(100) {
            let _ = writeln!(s,
                "{{ from = \"{}\", to = \"{}\", kind = \"{:?}\", len = {} }}",
                env.from, env.to, env.kind, env.payload_len);
        }
        s
    }
}
```

### 7.3 Crash report retention

`/data/crashes/` is bounded: max 200 reports, max 50 MiB total. The CrashWriter prunes the oldest on each new write. Reports older than 30 days are deleted unconditionally. The package manager surfaces them in its "Recent Crashes" UI keyed by `bundle_id`.

### 7.4 DWARF symbolication policy

Symbolication requires `wasmtime::Config::wasm_backtrace_details(WasmBacktraceDetails::Enable)` and the WASM module to contain a custom `name` section and (optionally) a `.debug_info` section. Both increase binary size: name section ~5%, DWARF ~50%. We make this a per-bundle policy:

- Bundles with `[debug] level = "release"` ship only the name section.
- Bundles with `[debug] level = "release-with-debug-info"` ship DWARF.
- Bundles with `[debug] level = "debug"` ship full DWARF and pretty-printed asserts.

The package manager defaults to `release-with-debug-info` for development bundles and `release` for store-distributed bundles.

---

## 8. TimerManager

### 8.1 Surface

Apps register timers via the WIT interface `vyoma:time/set-timer`:

```wit
interface time {
    type timer-id = u64;

    enum timer-error {
        quota-exceeded,
        invalid-interval,
        leeway-too-large,
    }

    record timer-spec {
        interval-ms: u64,
        leeway-ms: u64,
        repeating: bool,
        /// QoS hint (R5). Higher QoS means tighter leeway honoured.
        qos: qos-class,
    }

    set-timer: func(spec: timer-spec) -> result<timer-id, timer-error>;
    cancel-timer: func(id: timer-id) -> result<_, timer-error>;
}
```

The host implementation uses `timerfd_create(CLOCK_MONOTONIC, TFD_NONBLOCK|TFD_CLOEXEC)` and arms it with `timerfd_settime`. Each timer gets its own timerfd; the EventLoop registers it under an `EventSource::Timer { iid, timer_id }` token.

### 8.2 TimerManager structure

```rust
// supervisor/src/interrupt/timer.rs

use std::collections::HashMap;
use std::os::unix::io::{AsRawFd, OwnedFd, RawFd};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TimerId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct IidId(pub u32);

pub struct TimerManager {
    inner: Mutex<TimerInner>,
    event_loop: Arc<EventLoopHandle>,
    callbacks: Arc<CallbackRouter>,
}

struct TimerInner {
    timers: HashMap<(IidId, TimerId), TimerEntry>,
    next_id_per_iid: HashMap<IidId, u64>,
    per_iid_count: HashMap<IidId, u32>,
}

struct TimerEntry {
    fd: OwnedFd,
    spec: TimerSpec,
    armed_at: Instant,
    /// Token registered with the EventLoop.
    el_token: mio::Token,
}

#[derive(Debug, Clone)]
pub struct TimerSpec {
    pub interval_ms: u64,
    pub leeway_ms: u64,
    pub repeating: bool,
    pub qos: QosClass,
}

const MAX_TIMERS_PER_IID: u32 = 64;
const MIN_INTERVAL_MS: u64 = 1;
const MAX_LEEWAY_RATIO: u64 = 4;  // leeway <= interval * 4

impl TimerManager {
    pub fn new(el: Arc<EventLoopHandle>, callbacks: Arc<CallbackRouter>) -> Self {
        Self {
            inner: Mutex::new(TimerInner {
                timers: HashMap::new(),
                next_id_per_iid: HashMap::new(),
                per_iid_count: HashMap::new(),
            }),
            event_loop: el,
            callbacks,
        }
    }

    pub fn set(&self, iid: IidId, spec: TimerSpec) -> Result<TimerId, TimerError> {
        if spec.interval_ms < MIN_INTERVAL_MS { return Err(TimerError::InvalidInterval); }
        if spec.leeway_ms > spec.interval_ms * MAX_LEEWAY_RATIO {
            return Err(TimerError::LeewayTooLarge);
        }

        let mut g = self.inner.lock().unwrap();
        let count = g.per_iid_count.entry(iid).or_insert(0);
        if *count >= MAX_TIMERS_PER_IID { return Err(TimerError::QuotaExceeded); }

        // Allocate a TimerId and a timerfd.
        let next = g.next_id_per_iid.entry(iid).or_insert(0);
        *next += 1;
        let id = TimerId(*next);

        let fd = unsafe {
            let raw = libc::timerfd_create(
                libc::CLOCK_MONOTONIC,
                libc::TFD_NONBLOCK | libc::TFD_CLOEXEC,
            );
            if raw < 0 { return Err(TimerError::SystemError); }
            OwnedFd::from_raw_fd(raw)
        };
        let ts = libc::itimerspec {
            it_interval: if spec.repeating { ms_to_timespec(spec.interval_ms) } else { zero_ts() },
            it_value: ms_to_timespec(spec.interval_ms),
        };
        let rc = unsafe { libc::timerfd_settime(fd.as_raw_fd(), 0, &ts, std::ptr::null_mut()) };
        if rc < 0 { return Err(TimerError::SystemError); }

        // Register with the EventLoop.
        let token = self.event_loop.register(
            fd.as_raw_fd(),
            mio::Interest::READABLE,
            EventSource::Timer { iid, timer_id: id },
        ).map_err(|_| TimerError::SystemError)?;

        *count += 1;
        g.timers.insert((iid, id), TimerEntry {
            fd, spec, armed_at: Instant::now(), el_token: token,
        });
        Ok(id)
    }

    pub fn cancel(&self, iid: IidId, id: TimerId) -> Result<(), TimerError> {
        let mut g = self.inner.lock().unwrap();
        let entry = g.timers.remove(&(iid, id)).ok_or(TimerError::NotFound)?;
        let _ = self.event_loop.deregister(entry.el_token, entry.fd.as_raw_fd());
        if let Some(c) = g.per_iid_count.get_mut(&iid) { *c = c.saturating_sub(1); }
        Ok(())
    }

    /// Drop all timers belonging to an iid (called on iid teardown).
    pub fn drop_iid(&self, iid: IidId) {
        let mut g = self.inner.lock().unwrap();
        let keys: Vec<_> = g.timers.keys().filter(|(i, _)| *i == iid).copied().collect();
        for k in keys {
            if let Some(entry) = g.timers.remove(&k) {
                let _ = self.event_loop.deregister(entry.el_token, entry.fd.as_raw_fd());
            }
        }
        g.per_iid_count.remove(&iid);
        g.next_id_per_iid.remove(&iid);
    }

    /// Called by the EventLoop when a timerfd becomes readable.
    pub fn handle_fire(&self, iid: IidId, id: TimerId) {
        // Drain the timerfd. It returns u64 expiration count.
        let entry_fd = {
            let g = self.inner.lock().unwrap();
            match g.timers.get(&(iid, id)) {
                Some(e) => e.fd.as_raw_fd(),
                None => return,  // cancelled in flight
            }
        };
        let mut buf = [0u8; 8];
        let n = unsafe { libc::read(entry_fd, buf.as_mut_ptr() as *mut _, 8) };
        if n != 8 { return; }
        let expirations = u64::from_ne_bytes(buf);

        // Compute elapsed_ms since the timer was armed.
        let elapsed_ms = {
            let g = self.inner.lock().unwrap();
            g.timers.get(&(iid, id))
                .map(|e| e.armed_at.elapsed().as_millis() as u64)
                .unwrap_or(0)
        };

        // Enqueue exactly one callback even if expirations > 1.
        // Coalescing in CallbackQueue handles the "missed N fires" case.
        let cb = Callback::Timer {
            timer_id: id,
            elapsed_ms,
            enqueued_at: Instant::now(),
        };
        self.callbacks.enqueue(iid, cb);

        // If non-repeating, drop the timer now.
        let drop_it = {
            let g = self.inner.lock().unwrap();
            g.timers.get(&(iid, id)).map(|e| !e.spec.repeating).unwrap_or(false)
        };
        if drop_it { let _ = self.cancel(iid, id); }
    }

    pub fn dump(&self) {
        let g = self.inner.lock().unwrap();
        for ((iid, id), e) in &g.timers {
            tracing::info!(
                iid = iid.0, timer = id.0,
                interval_ms = e.spec.interval_ms,
                leeway_ms = e.spec.leeway_ms,
                "active timer"
            );
        }
    }
}

fn ms_to_timespec(ms: u64) -> libc::timespec {
    libc::timespec {
        tv_sec: (ms / 1000) as libc::time_t,
        tv_nsec: ((ms % 1000) * 1_000_000) as libc::c_long,
    }
}
fn zero_ts() -> libc::timespec { libc::timespec { tv_sec: 0, tv_nsec: 0 } }
```

### 8.3 Coalescing within leeway

When the EventLoop processes a timer fire, it checks for *other* timers in the same iid whose deadlines fall within ±leeway_ms of *this* fire and, if any, marks them as "co-fired" so the worker only has to dispatch one consolidated `on-timer` (the WIT interface allows passing the set of fired ids, not one). This matches macOS dispatch source leeway. Implementation lives in `TimerManager::handle_fire` extended to consult the iid's full timer set.

### 8.4 Watchdog timer

The watchdog itself uses a timerfd, registered as `EventSource::WatchdogTick`, firing every 1 second. The handler walks all iids, checks per-iid latency budgets (R5 QoS deadlines), and either flips `kill_on_next_trap=true` or emits a `TerminateReason::PolicyViolation` callback.

### 8.5 Quotas defended

64 timers per iid sounds low; in practice a UI app needs maybe 5 (animation frame, idle timer, suspended-state debouncer, watchdog pet, periodic save), a robotics app needs maybe 10 (one per sensor), and a misbehaving app trying to install 1000 timers gets `QuotaExceeded`. We can raise the cap per-platform via `boot.toml` if a use case appears.

---

## 9. Panic handling in supervisor code

### 9.1 Catch-unwind boundaries

R8 established that every subsystem's event-processing loop is wrapped in `catch_unwind`. Round 10 makes this explicit:

```rust
// supervisor/src/interrupt/panic_recovery.rs

use std::any::Any;
use std::panic::{catch_unwind, AssertUnwindSafe, UnwindSafe};

/// A scoped catch_unwind that turns panics into structured errors
/// without killing the supervisor.
pub fn run_subsystem<F, R>(name: &'static str, f: F) -> Result<R, PanicRecord>
where F: FnOnce() -> R + UnwindSafe {
    match catch_unwind(f) {
        Ok(r) => Ok(r),
        Err(payload) => {
            let msg = panic_payload_to_string(&payload);
            tracing::error!(subsystem = name, panic = %msg, "caught panic");
            Err(PanicRecord { subsystem: name, message: msg, at: chrono::Utc::now() })
        }
    }
}

fn panic_payload_to_string(p: &Box<dyn Any + Send>) -> String {
    if let Some(s) = p.downcast_ref::<String>()           { return s.clone(); }
    if let Some(s) = p.downcast_ref::<&'static str>()     { return s.to_string(); }
    "unknown panic payload".into()
}

#[derive(Debug)]
pub struct PanicRecord {
    pub subsystem: &'static str,
    pub message: String,
    pub at: chrono::DateTime<chrono::Utc>,
}
```

### 9.2 Per-subsystem PanicRecovery policy

```rust
pub enum PanicPolicy {
    /// Subsystem is degraded but not restarted. Used for non-critical
    /// subsystems (e.g., ThermalGovernor — fall back to no thermal management).
    Degrade,

    /// Restart the subsystem in place. Used for I/O subsystems that
    /// own a thread pool (e.g., the HID parser).
    Restart { max_restarts_per_hour: u32 },

    /// Treat panic as fatal — abort the supervisor and let stage1
    /// restart it. Used for subsystems where degraded operation is
    /// worse than full restart (e.g., the AssertionRegistry).
    Fatal,
}

pub struct PanicTracker {
    records: std::sync::Mutex<Vec<PanicRecord>>,
    policies: HashMap<&'static str, PanicPolicy>,
}

impl PanicTracker {
    pub fn observe(&self, rec: PanicRecord) -> PanicAction {
        let policy = self.policies.get(rec.subsystem)
            .copied()
            .unwrap_or(PanicPolicy::Degrade);
        let mut records = self.records.lock().unwrap();
        records.push(rec.clone());
        match policy {
            PanicPolicy::Degrade => PanicAction::Continue,
            PanicPolicy::Fatal   => PanicAction::AbortSupervisor,
            PanicPolicy::Restart { max_restarts_per_hour } => {
                let recent = records.iter()
                    .filter(|r| r.subsystem == rec.subsystem)
                    .filter(|r| (chrono::Utc::now() - r.at).num_minutes() < 60)
                    .count();
                if recent > max_restarts_per_hour as usize {
                    PanicAction::AbortSupervisor
                } else {
                    PanicAction::RestartSubsystem
                }
            }
        }
    }
}

pub enum PanicAction { Continue, RestartSubsystem, AbortSupervisor }
```

### 9.3 Default policies

| Subsystem | Policy |
|---|---|
| EventLoop | Fatal — without it nothing works |
| TimerManager | Restart (max 4/hr) |
| IpcBroker | Restart (max 4/hr) |
| InputDispatcher | Restart (max 8/hr) |
| HalRegistry | Fatal — broken HAL = lost peripherals |
| ThermalGovernor | Degrade — without it the SoC throttles in hardware anyway |
| DeviceManager | Restart (max 4/hr) |
| AssertionRegistry | Fatal |
| Per-app worker | Caught at the worker level, becomes a HostTrap on the iid |

### 9.4 Worker panics → HostTrap

When the per-app worker catches a panic *inside* its callback dispatch (i.e., something the supervisor's WIT shim does panicked, not the guest), the panic is converted to a `WasmTrap::HostTrap(payload_string)` and emitted as a crash report against that iid. The worker then loops to process the next callback (it does not die). This contains the blast radius to one iid; other iids keep running.

---

## 10. Hardware interrupt delivery

### 10.1 evdev → InputDispatcher → CallbackQueue

The full path for a keypress on `/dev/input/event3`:

1. User presses key. Kernel evdev driver writes an `input_event` struct to the file's ring buffer and marks the fd readable.
2. EventLoop's epoll wakes; the `EventSource::InputDevice { parser, .. }` is dispatched at priority 2.
3. `EvdevParser::drain_into` reads all available `input_event` structs, parses key codes, and emits `InputEvent` values into the `InputDispatcher` (R6).
4. `InputDispatcher` consults the focus stack (R6 windowing decision): the focused iid receives the event.
5. The event becomes `Callback::Input { event, enqueued_at }` and is `push`ed onto that iid's `CallbackQueue`.
6. The per-app worker `pop_blocking()`s it, calls `on-input` on the WASM store, handles any trap.

Total latency budget: 5 ms from key press to callback delivery, of which ~3 ms is kernel evdev overhead and ~1 ms is our subsystem.

### 10.2 GPIO

R9's HalRegistry exposes `subscribe_line_event(chip, line) -> Receiver<GpioLineEvent>`. Internally, the HAL opens `/dev/gpiochipN`, issues `GPIO_V2_GET_LINE_IOCTL` with `GPIO_V2_LINE_FLAG_EDGE_RISING | EDGE_FALLING`, and registers the resulting line-event fd with the EventLoop under `EventSource::GpioLineEvent`. Edge events flow the same way as evdev: dispatcher reads, callback enqueued, worker drains.

### 10.3 udev hotplug

`DeviceManager` (R6) owns a netlink socket bound to `NETLINK_KOBJECT_UEVENT`. The EventLoop registers the socket under `EventSource::UdevNetlink`. When the socket becomes readable, the DeviceManager parses the uevent message (ASCII KEY=VALUE pairs) and emits a `DeviceEvent` to each iid that subscribed to that device class. The callback enqueue happens with priority 5.

### 10.4 Thermal

R7's ThermalGovernor polls `/sys/class/thermal/thermal_zone*/temp` every 5 seconds. Rather than poll inside the EventLoop (which is single-threaded and shouldn't block on sysfs reads), the governor runs a dedicated thread that posts a small event into a pipe whenever the tier *changes*. The pipe is registered as `EventSource::ThermalPipe`. When the tier crosses the threshold for tier 2 (critical), the governor flips `kill_on_next_trap=true` on the non-essential iids and calls `engine.increment_epoch()` to force traps now.

### 10.5 Network

Network is largely out of scope for this round (R11 covers networking) but the integration point is: each TCP accept produces an fd; if an iid wants `on-data` callbacks, the supervisor registers that fd under `EventSource::IpcReceive { iid, sock }` (the same machinery as IPC). This works because the priority bucket for IPC (4) is appropriate for network data too.

---

## 11. The per-app worker thread

### 11.1 Lifecycle

When an iid is launched (R8 `App` lifecycle), the supervisor:

1. Builds the `wasmtime::Store` with the iid's WASI imports.
2. Instantiates the WASM module.
3. Spawns a dedicated worker thread with `name = format!("vyoma-iid-{}", iid.0)` and `stack_size = 256 KiB`.
4. Hands the worker an `Arc<CallbackQueue>`, the `Store`, the `IidEpochState`, and an `Arc<CrashHandler>`.

The worker runs:

```rust
// supervisor/src/interrupt/worker.rs

pub fn run_iid_worker<T: Send + 'static>(
    iid: IidId,
    mut store: Store<T>,
    queue: Arc<CallbackQueue>,
    epoch: Arc<IidEpochState>,
    crash: Arc<CrashHandler>,
    invoker: Box<dyn CallbackInvoker<T> + Send>,
    cfg: EngineCfg,
) {
    tracing::info!(iid = iid.0, "iid worker started");
    while let Some(cb) = queue.pop_blocking() {
        let is_terminate = matches!(cb, Callback::Terminate { .. });
        let result = panic_recovery::run_subsystem("iid_worker", AssertUnwindSafe(|| {
            run_callback(cb.clone(), &mut store, &epoch, &cfg, invoker.as_ref())
        }));
        match result {
            Ok(Ok(())) => { /* clean */ }
            Ok(Err(trap)) => {
                crash.report(iid, &store, cb.clone(), trap);
                if !matches!(cb, Callback::Terminate { .. }) {
                    // The crash handler decides whether to keep the worker alive.
                    if !crash.is_iid_recoverable(iid) { break; }
                }
            }
            Err(panic_record) => {
                // Panic inside our shim — wrap as HostTrap.
                let trap = WasmTrap::HostTrap(panic_record.message);
                crash.report(iid, &store, cb.clone(), trap);
                if !crash.is_iid_recoverable(iid) { break; }
            }
        }
        if is_terminate { break; }
    }
    queue.close();
    tracing::info!(iid = iid.0, "iid worker exiting");
}

pub trait CallbackInvoker<T> {
    fn invoke(&self, store: &mut Store<T>, cb: &Callback) -> wasmtime::Result<()>;
}
```

### 11.2 Engine config per callback

```rust
pub struct EngineCfg {
    pub input_slice_ms: u64,        // default 250
    pub timer_slice_ms: u64,        // default 1000
    pub ipc_slice_ms: u64,          // default 500
    pub suspend_slice_ms: u64,      // default 100
    pub kill_on_overrun: bool,      // default true
}

impl EngineCfg {
    pub fn slice_for(&self, cb: &Callback) -> u64 {
        match cb {
            Callback::Input { .. } | Callback::Gpio { .. } => self.input_slice_ms,
            Callback::Timer { .. } => self.timer_slice_ms,
            Callback::Ipc { .. } | Callback::DeviceEvent { .. } => self.ipc_slice_ms,
            Callback::Suspend { .. } | Callback::Resume { .. } => self.suspend_slice_ms,
            Callback::MemoryWarning { .. } => self.suspend_slice_ms,
            Callback::Terminate { .. } => self.input_slice_ms,
        }
    }

    pub fn kill_on_overrun(&self, cb: &Callback) -> bool {
        // Suspend MUST complete in slice or app is killed. Input gets
        // a graceful yield. Timers are killed only if cfg says so.
        matches!(cb,
            Callback::Suspend { .. } | Callback::Terminate { .. } | Callback::MemoryWarning { level: PressureLevel::Critical, .. }
        ) || self.kill_on_overrun
    }
}
```

---

## 12. Implementation files & line budgets

```
supervisor/src/interrupt/
├── mod.rs                  ~120 LOC — re-exports, Subsystem trait impl
├── signal.rs               ~480 LOC — SignalHandler, fatal handler, SignalEvent
├── event_loop.rs           ~490 LOC — EventLoop, EventSource, dispatch
├── timer.rs                ~470 LOC — TimerManager, TimerEntry, quota check
├── wasm_trap.rs            ~310 LOC — WasmTrap, EpochKillReason, mappings
├── crash_writer.rs         ~470 LOC — CrashReport, WasmBacktraceCapture, prune
├── callback_queue.rs       ~430 LOC — Callback, CallbackQueue, priorities
├── worker.rs               ~360 LOC — run_iid_worker, EngineCfg
├── panic_recovery.rs       ~250 LOC — run_subsystem, PanicTracker
└── epoch.rs                ~240 LOC — EpochTicker, IidEpochState
```

Every file is well under 500 LOC. Cross-file relationships:
- `mod.rs` ties the subsystem together and exposes `InterruptSubsystem` for `Supervisor`.
- `event_loop.rs` depends on `signal.rs` (signalfd), `timer.rs` (timerfd), `callback_queue.rs` (push).
- `worker.rs` depends on `epoch.rs`, `callback_queue.rs`, `wasm_trap.rs`, `crash_writer.rs`, `panic_recovery.rs`.
- `timer.rs` depends on `event_loop.rs` (registration) and `callback_queue.rs` (enqueue).
- `crash_writer.rs` depends on `wasm_trap.rs`.

---

## 13. Interaction with prior rounds

| Round | Interaction |
|---|---|
| R1 (lifecycle) | `WasmTrap → CrashKind` mapping uses the 12 CrashKind variants defined here; `on-timer`, `on-ipc`, `on-suspend`, `on-resume`, `on-terminate` are the WIT callbacks the worker delivers. |
| R2 (virtual memory) | `WasmTrap::HeapOutOfBounds` includes the linear-memory address from R2's guard-page machinery; `MemoryStats` in the crash report uses R2's resident-set accounting. |
| R3 (IPC) | `Callback::Ipc` carries the `IpcEnvelope`; `IpcBroker` is one of the producers feeding `CallbackQueue`. |
| R5 (scheduler) | `IidEpochState::consumed_cpu_ms` feeds back into the SCHED_DEADLINE budget; `kill_on_next_trap` flips when cgroup cpu.max is hit. `EngineCfg.slice_for` uses QoS. |
| R6 (drivers) | `InputDispatcher` and `DeviceManager` are EventLoop producers; their `EventSource` variants are `InputDevice` and `UdevNetlink`. |
| R7 (power) | `ThermalGovernor` flips `kill_on_next_trap = true` and bumps the epoch to force throttling; `AssertionRegistry` blocks the EventLoop's shutdown path. |
| R8 (boot) | Stage1 catches PID-2's `SIGSEGV` via `SIGCHLD`. EventLoop is initialised in `BootPhase::EventLoop` and torn down in `Shutdown::Phase4_TaskDrain`. `RankedMutex` ordering: `EventLoop.sources` outranks `CallbackQueue.inner` outranks `IidEpochState`. |
| R9 (HAL) | `EventSource::GpioLineEvent` integrates with R9's `HalRegistry`; per-bus thread queues from R9 feed into `CallbackQueue` via the `Hal` producer. |

---

## 14. Testing strategy

### 14.1 Unit tests

- `wasm_trap_tests.rs`: every `WasmTrap` variant round-trips through `to_crash_kind` and back.
- `signal_tests.rs`: simulated `signalfd_siginfo` decoding for all `BLOCKED_SIGNALS`.
- `event_loop_tests.rs`: deterministic priority dispatch (mock sources, check order).
- `timer_tests.rs`: quota enforcement, leeway validation, fire-and-cancel race.
- `callback_queue_tests.rs`: priority ordering, coalescing, overflow eviction.
- `crash_writer_tests.rs`: TOML output stability (golden files), 200-report pruning.
- `panic_recovery_tests.rs`: per-subsystem policy decisions, escalation to Fatal after N panics/hr.
- `epoch_tests.rs`: soft-yield vs hard-kill distinguished by the `kill_on_next_trap` flag.

### 14.2 Integration tests

- `iid_crash_recovery.rs`: spawn a tiny WASM app that traps every Nth callback, verify restart count and crash report.
- `epoch_timeslicing.rs`: two apps in a busy loop, verify both progress under 1 kHz epoch.
- `signal_dispatch.rs`: send each `BLOCKED_SIGNAL` and verify EventLoop saw it.
- `timer_coalescing.rs`: register 10 timers with overlapping leeway, verify coalescing fires.

### 14.3 Fuzz / chaos

- `chaos_panic.rs`: random subsystems inject panics; verify supervisor either degrades or restarts but never deadlocks.

---

## 15. Worked example: button press triggers GPIO callback

End-to-end on `iot-edge` (Raspberry Pi):

1. User presses a button wired to GPIO 17 (manifest declares `gpio_pins = [17]`).
2. Kernel gpiochip0 driver detects the edge; the line-event fd becomes readable.
3. EventLoop's epoll wakes; `EventSource::GpioLineEvent { chip: 0, line: 17, iid }` dispatched at priority 3.
4. `HalRegistry::drain_gpio` reads the `gpio_v2_line_event` struct, extracts `value`, builds `Callback::Gpio { pin: 17, value: true, enqueued_at: now }`.
5. The callback is `push`ed onto iid's `CallbackQueue`. Since the iid was idle, the worker's `pop_blocking` returns immediately.
6. Worker calls `epoch.before_call(&mut store, slice_ms=250, kill=true)`.
7. Worker invokes `on-gpio` via the `CallbackInvoker`. The WASM app's handler runs.
8. App returns normally. Worker calls `epoch.after_call(&mut store)`.
9. Worker loops back to `pop_blocking`. Next callback (if any) dispatched.

Total wall time from button press to `on-gpio` invocation: typically 1–3 ms.

---

## 16. Open questions for the Critic

1. **Priority inversion in CallbackQueue.** If a high-priority `Input` arrives while the worker is processing a long-running `Timer`, the Timer must finish before the Input dispatches. Should we add cooperative cancellation (a polling hostcall the app can check) so apps can voluntarily yield mid-timer? Or accept the priority-inversion latency given typical timer callbacks are short?

2. **Epoch tick rate vs CPU cost.** A 1 kHz ticker is 1000 `engine.increment_epoch()` calls/sec, which is cheap on x86 but possibly visible on mcu-minimal. Should we make the tick rate per-platform (e.g., 100 Hz on mcu-minimal, 1 kHz on desktop-full), or accept variable preemption latency across platforms?

3. **SIGSEGV chain ordering with Wasmtime.** Our `fatal_handler` chains to Wasmtime's prior handler. But if a SEGV arises *during* the chained call (Wasmtime's handler itself faults), we recurse forever. Should we install a guard that detects re-entry and aborts immediately? Is there a cleaner integration via Wasmtime's `Config::with_host_signal_handler`?

4. **Timer quota of 64.** Is this too tight for plausible robotics workloads (one timer per sensor, of which there might be 50+)? Should the cap be per-platform via `boot.toml`, or should we expose a "consolidated tick" API where apps register one timer and a list of sub-deadlines to coalesce themselves?

5. **Crash report bound (200 / 50 MiB).** On a write-heavy system this could lose meaningful forensic data. Should we add a "sticky" flag for crash reports the user has explicitly inspected so they're never pruned? Should pruning be priority-aware (always keep `CrashKind::HostPanic` over `CrashKind::ArithmeticFault`)?

6. **CallbackQueue capacity of 256.** A burst of 1000 input events from a held key would drop most of them. Should the input dispatcher coalesce key-repeat events *before* the CallbackQueue, the way it coalesces evdev SYN events? Or accept dropped events as a backpressure signal?

7. **Soft yield races.** When the watchdog flips `kill_on_next_trap` simultaneously with the worker calling `epoch.after_call`, can a "kill flag" persist into the next callback's `before_call`? The `before_call` always overwrites it, but the *window* between worker code paths is not zero. Is there a real scenario where this matters?

8. **Crash report disk write under memory pressure.** If the system is in `MemoryWarning::Critical`, writing a 50 KiB TOML to `/data/crashes/` may itself fail with `ENOSPC` or `ENOMEM`. Should we have an in-memory ring of recent crash reports as a fallback, only flushed to disk when conditions allow? Or accept that crash reports may be lost on a flailing system?

---

## 17. Summary

Round 10 unifies six previously separate concerns — Wasmtime traps, POSIX signals to the supervisor, hardware interrupt delivery, app-visible callbacks, epoch-based CPU preemption, and Rust panic recovery — into a single, layered subsystem under `supervisor/src/interrupt/`. The keystone is the single-threaded `EventLoop` that demultiplexes every async source through `epoll`, plus the per-iid `CallbackQueue` that re-serialises everything into a synchronous, priority-ordered sequence of WIT calls.

The model has three distinguishing properties:

- **Apps see only synchronous callbacks.** No signal handlers, no interrupt vectors, no async event loops inside the guest. The host does all the asynchrony.
- **Epoch interruption is the single CPU-preemption mechanism.** Time slicing, quota enforcement, and watchdog kill all flow through `engine.increment_epoch()` plus the `kill_on_next_trap` flag. There are no other preemption channels.
- **POSIX signals are funnel-fed through `signalfd`.** No async-signal-safe handler logic except the unavoidable `SIGSEGV/SIGBUS/SIGILL/SIGFPE` minimal handler that writes a byte to a self-pipe and aborts, allowing stage1 (R8) to recover PID-2.

Combined with R1's lifecycle, R5's scheduler, R6's drivers, R7's power management, R8's boot, and R9's HAL, the interrupt subsystem completes the supervisor's runtime model. The Critic's responsibility is to find the holes — priority inversions, signal-chain hazards, quota mismatches, crash-report durability bugs — that this proposal hasn't anticipated, especially in §16's open questions.

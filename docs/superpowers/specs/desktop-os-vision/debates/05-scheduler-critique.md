# Round 5 Critic: Scheduler & CPU Management

**Date:** 2026-05-29
**Round:** 5 of 80
**Subsystem:** Scheduler & CPU Management
**Critic verdict:** FUNDAMENTAL FLAWS

## Executive Summary

The proposed scheduler design for VyomaOS — modeled on macOS Grand Central
Dispatch (GCD), Quality-of-Service (QoS) classes, App Nap, and SCHED_FIFO
real-time bands — is an attractive surface-level mapping but it founders the
moment one looks past the API and asks how the model intersects with the
underlying realities of WyomaOS: (a) every app is a single-threaded
`wasm32-wasip2` instance hosted inside a `wasmtime::Store` that is not `Send`
or `Sync`; (b) the supervisor is a single process that owns all WASM linear
memories in its address space; (c) the only real CPU dispatcher is Linux CFS
running on a 5.10 kernel inside QEMU; and (d) Wasmtime's epoch interruption
fires at ~10ms resolution with semantics that throttle preemption density,
not throughput.

The Architect appears to have ported "macOS desktop semantics" wholesale
without engaging with the WASM execution model. The biggest mismatches:

1. **GCD's concurrent work queues fundamentally assume cheap shared-memory
   thread reuse.** WASM linear memories cannot be entered concurrently from
   two threads. The Wasmtime Store is a `!Sync` object. A
   `dispatch_async(queue_A, work_item)` model collapses into either:
   serialized actor mailboxes (which is just per-app run queues), or full
   Store cloning with shared state via WASI shared-everything-threads
   (unstable, breaks WASIp2 invariants). The proposal handwaves this.

2. **Epoch-based CPU accounting is an order of magnitude too coarse** for
   the latency claims it makes. A 10ms epoch tick cannot distinguish
   "16ms-bounded UI burst" from "runaway compute" — both look like one
   epoch tick of activity. The Architect's "deadline detection" needs
   sub-millisecond accounting which Wasmtime simply does not provide
   without instrumentation.

3. **SCHED_FIFO for audio is a foot-gun** that becomes a system-bricking
   loaded weapon when the FIFO thread is also running untrusted WASM.
   Inside a guest VM under QEMU it may merely hang the VM; on bare metal
   on the iot-edge/robotics-rt profiles it can deadlock the whole platform.
   macOS does not use SCHED_FIFO equivalents — it uses Mach time-constraint
   policies with computation/constraint/period triples that bound starvation.

4. **App Nap detection requires omniscience.** The criterion "no audio, no
   network for 10s" requires the scheduler to be a cross-cutting observer
   of the IPC broker, audio subsystem, and network stack. This couples
   the scheduler to every other subsystem and creates state-explosion bugs
   when one subsystem proxies for another (Round 3 introduced exactly
   this case with mediated capabilities).

5. **Throttling via epoch-frequency manipulation is backwards.** Reducing
   epoch frequency makes apps run LONGER per slice between preemption
   points, not shorter. The Architect appears to confuse "preemption
   density" with "CPU time consumed." This is a fundamental error
   in the operational model.

6. **Priority inversion is not addressed.** Round 3 IPC made request/reply
   the primary inter-app communication mechanism. With QoS classes and no
   priority inheritance, every high-priority caller blocked on a low-priority
   callee will jitter. The Architect's proposal would make this *worse* than
   a naive scheduler because the QoS spread is wider.

The verdict is **FUNDAMENTAL FLAWS** because four of the six pillars of the
proposal (GCD-style concurrent queues, SCHED_FIFO real-time, App Nap
detection, and epoch-based throttling) are either inapplicable to WASM
execution or operationally incorrect. A redesign starting from the WASM
execution model — not from macOS's API surface — is required.

## Critical Issues (blocking)

### C1. GCD-Concurrent-Queues Cannot Express WASM Single-Threadedness

**Issue.** The proposal positions a GCD-equivalent dispatch system as a core
primitive: apps submit work items to global or app-specific queues, the
scheduler picks worker threads from a pool, and the pool runs work items
with priority biasing per QoS class. This is exactly how libdispatch on
Darwin works. It is unworkable for VyomaOS.

**Why.**

- A `wasm32-wasip2` instance lives inside a `wasmtime::Store<T>`. The
  Store owns the instance's linear memory, tables, globals, the trampoline
  table, the call stack, and the epoch counter. `wasmtime::Store` is
  documented as `!Sync` and is also `!Send` once an instance is actively
  executing inside it (the executing thread holds borrows into the
  Store's interior).

- A "work item submitted by App A" is, in the WASM execution model, a
  reference to an exported function in App A's instance plus a set of
  arguments. To call that exported function, you need a thread that
  owns (or can briefly take) App A's Store.

- "Pool of worker threads" means N OS threads each capable of taking
  any Store. To make this work safely you must `Mutex<Store>` every
  Store — which immediately reduces the GCD queue to a single-writer
  mailbox per app. That is not GCD; that is just an actor per app.
  And critically, the supervisor already has exactly one thread per
  app (per CLAUDE.md: "Concurrent scheduler (one thread per app)").
  The Architect's proposal duplicates this with extra abstraction
  layers and no behavioral change.

- Even if you accept the actor-per-app collapse, you've now lost the
  promised property of GCD: that a UserInteractive work item submitted
  by App A jumps the queue ahead of a Background work item submitted
  by App A. With a single owning thread per Store, work items are
  executed FIFO inside the Store (or with epoch-yielded cooperation),
  not by external priority. The QoS class can affect *which app's
  thread* CFS schedules next, but it cannot reorder work *inside*
  an app's instance.

**The Architect's proposal would, in practice, devolve to:** "the supervisor
sets `nice` on each app's existing thread based on the app's declared QoS
class." Which is fine — but it is not "GCD-equivalent dispatch." Calling it
that is misleading and will produce bug reports from developers who expected
GCD semantics.

**Required.** Either (a) drop the GCD framing entirely and document VyomaOS
as a per-app QoS-tiered thread scheduler with no in-process concurrent
queues, or (b) introduce an explicit WASI proposal extension that exposes
"work items" as host-managed continuations the supervisor can execute
in priority order — and gate this behind a capability that admits the
loss of `wasm32-wasip2` purity.

I see no third path.

### C2. Epoch Accounting Is 10× Too Coarse for the Claimed Latency Targets

**Issue.** The proposal cites "UI smoothness < 16ms frame budget" as a
quality target served by the UserInteractive QoS class, and claims that
epoch-based deadline detection can identify runaway computations. Both
claims are unsupported by the granularity of Wasmtime epoch interruption.

**Why.**

- The default Wasmtime epoch tick interval is in the 1–10ms range depending
  on host configuration. In Round 1 we adopted the standard pattern where
  a single background thread ticks the global epoch counter every ~10ms.
  This means a WASM function gets a yield point every ~10ms of wall clock
  in the best case.

- A 16ms frame budget with 10ms epoch resolution means at most 1 yield
  inside the frame, sometimes 0. There is no way to detect "this UI
  callback has consumed 12 of its 16ms budget" with this resolution.
  You only ever see "ran for 0 epoch ticks" or "ran for 1 epoch tick."

- A 9ms-burst-per-16ms-frame app produces wall-clock CPU utilization of
  56% but produces *zero* epoch tick boundaries inside its busy intervals
  (the 9ms is less than one epoch tick). Without instrumentation, the
  scheduler cannot distinguish this acceptable burst from "ran 9ms,
  yielded, didn't run again" — i.e., a healthy idle app.

- Conversely, a 50ms runaway computation produces 5 epoch tick boundaries.
  The scheduler can see it. But by the time it sees 5 boundaries, 50ms of
  UI thread time has been consumed. If this is the UserInteractive thread
  it has *already* missed 3 frames.

- The Architect's "deadline detection" therefore has a fundamental latency
  floor equal to the epoch tick interval, which is ≥ the frame budget the
  detection is supposed to enforce.

**What the proposal needs.** Either (a) sub-millisecond epoch ticking
(comes with very real overhead because each tick requires a memory write
+ memory barrier observable by all running WASM, and at sub-ms rates this
shows up in cache traffic), or (b) host-side timestamping at every WASI
syscall and at every epoch yield, with a "soft deadline" that compares
elapsed wall clock to the budget. The proposal does neither.

**Concrete fix.** Replace "epoch-based deadline detection" with "per-thread
`clock_gettime(CLOCK_THREAD_CPUTIME_ID)` sampling at WASI boundaries,
augmented with epoch-driven yields as preemption points." Document that
sub-frame deadlines are detectable post-hoc only (i.e., "this app missed
its frame budget last frame") not preemptively.

### C3. SCHED_FIFO for Audio Is Unsafe in a WASM Sandbox

**Issue.** The proposal grants SCHED_FIFO scheduling policy to a thread
hosting an audio callback for any app declaring an audio capability.
This is unsafe at multiple layers.

**Why.**

- SCHED_FIFO threads on Linux are non-preemptible by other SCHED_OTHER
  threads of equal or lower priority and they run until they voluntarily
  yield or block. A buggy or adversarial WASM module running inside a
  SCHED_FIFO thread can busy-loop and starve:
  - Other apps' threads
  - The supervisor's own management threads (IPC broker, lifecycle)
  - The Wasmtime epoch-ticker thread itself (if the ticker is SCHED_OTHER)
  - In the worst case, kernel work queues if RT throttling is disabled

- The Linux kernel has `kernel.sched_rt_runtime_us` (default 950000/1000000)
  which caps RT thread CPU to 95%, preventing total system lockup. But
  the remaining 5% is still catastrophic for an interactive system: in
  a 100ms window the entire userspace except the runaway RT thread gets
  5ms total. Mouse cursor freezes. Display compositor stalls.

- Wasmtime's epoch ticking is delivered by a background thread (typically
  SCHED_OTHER). A SCHED_FIFO WASM thread will not be preempted by epoch
  ticks because the ticker thread cannot run. The standard mitigation
  ("use SIGALRM") doesn't help either: SCHED_FIFO threads do receive
  signals, but the signal handler runs on the FIFO thread, and the
  WASM call into the host's signal-induced trap path requires acquiring
  Wasmtime state that may itself be inside the FIFO thread's stack.

- Crucially, **WASM is not exempt from Linux's memory pressure or kernel
  faults.** A WASM linear memory growing past resident memory triggers a
  page fault. The kernel handler will service it, but if there's no
  available memory, the OOM killer may kill our supervisor — and our
  supervisor is PID 1 in the guest VM, so the guest panics.

- macOS does not have a SCHED_FIFO analogue exposed to userspace this
  way. It has Mach `THREAD_TIME_CONSTRAINT_POLICY` which is a *bounded*
  RT policy: each thread declares `computation`, `constraint`, `period`
  (e.g., "I need 2ms of CPU within every 5ms window"). The Mach scheduler
  enforces both upper and lower bounds. The Architect's proposal cites
  this as the model but maps it to Linux SCHED_FIFO, which provides only
  the lower bound, not the upper.

**The correct mapping** of Mach time-constraint policy to Linux is
`SCHED_DEADLINE` (CFS deadline scheduler, available since Linux 3.14),
which takes a triple `(runtime, deadline, period)` and enforces all three.
The proposal does not mention SCHED_DEADLINE. It should be the only RT
policy used for any WASM-hosted callback.

**Even with SCHED_DEADLINE**, the supervisor must:
- Set a CPU time hard limit (`setrlimit RLIMIT_RTTIME`) so a runaway WASM
  callback is SIGKILL'd before it can starve the system
- Maintain a watchdog thread on a different CPU core (CPU affinity required)
  that can detect SCHED_DEADLINE starvation
- Refuse to grant RT scheduling to apps without a declared `audio_realtime`
  capability AND a signed manifest (capability-secure semantics require
  this be controlled, not opt-in for any app declaring "audio")

The proposal does none of this. As-is, the audio scheduling design will
cause platform lockups in development and CVEs in production.

### C4. App Nap Requires Cross-Subsystem Omniscience

**Issue.** The App Nap criterion proposed — "if app has produced no audio
output and no network traffic for 10s and is occluded, throttle aggressively
or suspend" — requires the scheduler to be a privileged observer of subsystems
that the Round 3 IPC critique already showed are deliberately decoupled.

**Why.**

- Audio output activity originates in a future audio subsystem (Round 16+).
  Network activity originates from the network stack (Round 11). Window
  occlusion comes from the compositor (Round 9, Phase 17). The scheduler
  would have to subscribe to events from all of these.

- The proposed mechanism likely involves event broadcasts: "audio
  subsystem publishes 'app_X produced N samples' to scheduler." This
  is fine in isolation but explodes the IPC graph: every leaf subsystem
  is now publishing to the scheduler, and the scheduler holds tracking
  state for every app's activity history across every dimension.

- **The mediated-capability case.** Consider a download manager app
  that holds the network capability and accepts download requests from
  other apps via IPC. A background-tier app submits a download to the
  download manager, then sleeps waiting for completion. From the
  scheduler's perspective:
    - Background app: no audio, no network, occluded → App Nap → suspend
    - Download manager: doing the work, has network activity, is alive
  The download completes, the download manager tries to deliver the
  result via IPC to the background app, but the background app is
  suspended. Either the IPC is dropped (data loss) or it wakes the
  background app (defeating App Nap).

- macOS solves this with **activity tracking**: when app A initiates
  any work that might continue async, it begins an `os_activity` token,
  and downstream subsystems propagate the token so attribution stays
  with A. The scheduler then knows "A has an active activity, even
  if A itself is sleeping" and refuses to nap it. The Architect does
  not mention activity attribution.

- **Edge case: the broker IS the activity proxy.** The Round 3 IPC
  broker is in the supervisor's address space. If we made the broker
  responsible for keeping activity tokens on behalf of waiting apps,
  it can answer "is app A waiting for a reply?" → "yes, exempt it from
  Nap." But this requires every IPC operation to be tagged with a
  cross-subsystem activity ID, which the current spec doesn't have.

**Required.** Either (a) introduce an `os_activity`-style propagated token
spec into Round 3 IPC and Round 11 network as a precondition for App Nap,
or (b) drop App Nap from this round and revisit it after activity tracking
is specified, or (c) restrict App Nap to apps that have NO inbound IPC
subscriptions (which removes its value for most realistic apps).

The Architect's design as-stated is unimplementable without changes to
the IPC and network specs that have already been ratified in Rounds 3
and (planned) 11.

### C5. Throttling Via Epoch Manipulation Is Operationally Inverted

**Issue.** The proposal describes a CPU-cap mechanism: "if an app exceeds
its CPU quota over a moving window, the scheduler reduces its epoch
frequency, throttling its execution." This is backwards. Reducing epoch
frequency *increases* the runtime per slice between preemption opportunities;
it does not reduce CPU consumption.

**Why.**

- Wasmtime epoch interruption works as follows: a single u64 counter
  is incremented periodically by an external ticker thread. WASM
  function prologues check the counter; if it has advanced past a
  threshold, the function yields (via a trap or a cooperative yield
  hook). The frequency of ticking controls how often this check
  results in a yield.

- If you reduce the tick frequency (longer interval between ticks),
  the WASM function executes *more* host wall-clock between yields,
  not less. The total CPU consumed by the app over a 1-second window
  is determined by the CFS scheduler giving its thread CPU time, NOT
  by the epoch frequency. The epoch frequency only controls preemption
  granularity *within* an awarded CPU slice.

- The correct mechanism for CPU throttling on Linux is one of:
    - `cgroup v2 cpu.max` — set max bandwidth (e.g., 50000/100000 = 50% CPU)
    - `setpriority` / `nice` — bias CFS toward giving the thread less CPU
    - `sched_setaffinity` — restrict to fewer CPU cores
    - SIGSTOP/SIGCONT — outright pause the thread

- The Architect appears to have conflated **two distinct mechanisms**:
  (1) epoch interruption (cooperative yield from WASM to host for
  policy decisions), and (2) CPU throttling (preemptive reduction of
  CPU time given by kernel scheduler). They operate at different layers
  and one cannot substitute for the other.

- **An aside on what reducing epoch frequency actually does cause.**
  A long epoch interval increases the latency between "app produced bad
  output and should yield" and "supervisor regains control." For an
  adversarial WASM, this means a longer window to do damage before being
  preemptable. So the proposal not only fails to throttle, it makes the
  system MORE vulnerable to runaway compute, not less.

**Required.** Replace "reduce epoch frequency to throttle" with "place the
runaway thread into a cgroup with `cpu.max` of the desired budget." If
cgroup support is unavailable on the platform (some mcu-minimal or
iot-edge configurations may not have cgroup v2), fall back to `nice +20`
and document the degradation.

The proposal needs a clean separation:
- Epoch interruption = preemption point density for policy injection
- cgroups / nice = CPU bandwidth control

### C6. Priority Inversion in IPC Is Unaddressed

**Issue.** Round 3 ratified request/reply IPC with capability-typed
endpoints. The Architect now layers QoS classes on top of apps but does
not specify how priority is propagated through IPC blocking. The
inevitable result is classic priority inversion.

**Why.**

- Scenario: UserInteractive app U sends an IPC request to Background app B.
  U blocks waiting for the reply (or polls, but either way U cannot complete
  the next UI frame until B replies). B has nice +10 (Background QoS).
  The system has Utility apps running.

- Under CFS, U's nice -10 buys U preferential CPU, but U isn't asking for
  CPU — it's waiting on B. B is competing with Utility apps (which have
  higher CFS weight than B's nice +10). The kernel correctly schedules
  Utility apps over B. B's reply is delayed by 30-100ms. U's UI stutters.

- Result: a UserInteractive app's effective latency is dominated by the
  QoS of the *lowest-priority app it depends on* — the opposite of the
  intent. This is the textbook priority inversion problem solved in
  POSIX with priority inheritance mutexes.

- **The corresponding fix**, priority inheritance, requires the IPC
  broker to: (a) detect when high-priority app U is blocked on a reply
  from low-priority app B, (b) temporarily promote B's effective scheduler
  priority to U's level for the duration of B's reply handling, (c) restore
  B's priority when B replies. This requires the broker to know about
  scheduler priorities and to call `pthread_setschedparam` (or our wrapper)
  on B's host thread. The Architect's proposal does not establish this
  cross-cutting interaction.

- **Worse**: when U times out and abandons the request, the broker must
  remember to demote B. If U crashes mid-wait, the broker must clean up.
  These are the standard problems that have made priority inheritance
  notoriously hard to get right in production OSes (Mars Pathfinder is
  the canonical case study).

- **Worst**: with capability-typed endpoints and capability sharing
  through Round 3, the dependency chain U → B → C → D is possible.
  Priority inheritance must transitively chain through the whole graph.
  The broker must build a dependency DAG, detect cycles (deadlock), and
  apply inheritance along the longest path.

**Required.** Either (a) introduce priority inheritance into the IPC broker
specification with explicit cycle detection and timeout semantics, or
(b) declare that IPC across QoS-class boundaries is non-blocking only
(request/reply must use futures with deadline timeouts, never sync waits),
or (c) accept the inversion as a known limitation and document expected
latency degradation.

The proposal silently ignores the question and the result is that the
QoS system will produce *worse* P99 latency than a flat scheduler when
realistic IPC patterns are used.

## Significant Issues (important)

### S1. Nice -10 Does Not Guarantee 16ms Latency

The proposal repeatedly equates "UserInteractive QoS → nice -10" with
"< 16ms latency for UI." This is wrong in a load-precise way.

CFS is a *fair* scheduler; nice values bias the share but don't bound
the wait. Under load, even a nice -20 thread can wait 30-50ms for CPU
on a 4-core system if 20 other threads (all with various nice values)
are runnable. The 16ms frame budget claim is not a guarantee, it's an
aspiration. Documentation should make this explicit.

The honest framing is: "nice -10 means UserInteractive apps will receive
roughly 4× the CFS time share of nice 0 apps, and roughly 16× the share
of nice +10 apps. Under low system load, this is sufficient for 16ms
frame budgets. Under heavy load, the supervisor should use SCHED_DEADLINE
for any thread that requires hard latency bounds (compositor flush, audio
mixing)."

The supervisor's compositor thread (per recent Phase 17 work) is itself
WASM and would need the same SCHED_DEADLINE treatment, with all the
attendant safety concerns in C3.

### S2. CAP_SYS_NICE Privilege Assumption Is Not Documented

Setting nice values below 0 (i.e., higher priority than default) requires
CAP_SYS_NICE on Linux. Setting RT scheduling policies (SCHED_FIFO,
SCHED_RR, SCHED_DEADLINE) requires CAP_SYS_NICE plus RLIMIT_RTPRIO.

In the VyomaOS guest VM, the supervisor is PID 1 and runs as root, so it
has full capability sets by default. This is fine *inside the VM*. But:

- For the upcoming bare-metal profiles (iot-edge on Raspberry Pi,
  robotics-rt on actual robot controllers), the supervisor may not run
  as root if the target distribution applies modern privilege-reduction
  practices. The proposal assumes CAP_SYS_NICE without documenting the
  assumption or providing a graceful fallback.

- The supervisor should fail-soft: if it cannot set the desired scheduler
  policy, log a warning and continue with default scheduling. The proposal
  does not specify this fallback.

Required: a "scheduler capability degradation" mode that detects available
privileges at startup, picks the best available policy, and announces the
degradation to apps via a system event.

### S3. Timer Coalescing Creates a Wakeup Storm

The proposal correctly identifies timer coalescing (Darwin's `TIMER_LEEWAY`)
as a power-saving and cache-friendly technique. But the proposed
implementation — "coalesce timers within ±10ms by aligning to a global
10ms tick" — creates a wakeup storm at every tick boundary.

If 50 apps each register a 1-second timer (chat clients, status pollers,
RSS readers, etc.), they all align to the same global tick. At time T,
50 WASM instances wake up, all need CPU, all compete. Throughput collapses
into a sawtooth pattern with 950ms idle and 50ms scramble.

macOS handles this with **QoS-tiered timer leeway**:
- UserInteractive: leeway 0 (timers fire precisely)
- UserInitiated: leeway 1ms
- Utility: leeway 1s
- Background: leeway 10s

This staggers wakeups across QoS classes. The Architect's proposal
should adopt this tiered leeway scheme, not a flat 10ms coalesce.

Additionally, the coalescing should jitter the alignment per app (e.g.,
align to `tick + hash(app_id) % leeway`) so even within a QoS tier
apps don't synchronize.

### S4. App Nap Suspension Semantics Are Underspecified

If an app is suspended (SIGSTOP or epoch-park) by App Nap, what happens to:

- Pending IPC messages from other apps?
  - Buffered indefinitely? Bounded buffer with drops? Replied with EAGAIN?
  - The Round 3 spec doesn't address this; the scheduler proposal also doesn't.

- Active filesystem operations on `/data`?
  - 9P virtio operations have an in-flight state. SIGSTOP at the wrong
    moment leaves the 9P connection in a half-acknowledged state.
  - On resume, does the syscall return EINTR? Does the supervisor replay it?

- Network sockets with pending data?
  - TCP socket buffers fill, TCP backpressure stops the peer. If the peer
    times out, the connection drops, and on resume the app sees ECONNRESET.
  - This is a behavior change from non-suspended operation that apps must
    be aware of.

- The wall clock advancement?
  - An app using monotonic clock for animation might wake with a 30-second
    delta from its previous tick. UI animations skip ahead, audio buffers
    underflow.

macOS handles much of this with **assertions**: apps declare "I am performing
file I/O" or "I need a network connection alive" and these assertions
prevent App Nap. The Architect's proposal mentions audio and network as
nap-preventers but doesn't define an assertion API for apps to take
explicit responsibility.

Required: define an `assertion` capability and API. Suspension semantics
must be documented for IPC, FS, network, and timer subsystems.

### S5. The Compositor Is Special and the Proposal Doesn't Acknowledge It

The supervisor hosts the framebuffer compositor (per recent commits
55fd121, c62ac27, 1380b0b — per-window Surface routing and Z-order pass).
The compositor itself is in the supervisor's address space (not a WASM
app yet), but it has real-time-ish requirements: at 60Hz it must flush
every 16ms.

The Architect's QoS classes apply to WASM apps. But the compositor is
not a WASM app. What scheduler policy does it run under? Default? Same
as UserInteractive? Higher?

When the compositor stalls (e.g., the recent ABBA deadlock fix), all UI
freezes — and the scheduler has no awareness. App Nap can't help because
the compositor isn't an app. SCHED_DEADLINE can't help because the
compositor's deadline depends on UI app rendering completion times.

The proposal needs an explicit statement on supervisor-internal subsystem
scheduling, with the compositor as a worked example. Likely the compositor
should run on a dedicated CPU core with SCHED_DEADLINE policy and CPU
affinity, but this needs to be designed, not assumed.

### S6. Multi-Platform Profile Coverage Is Incomplete

The Architect maps the design to "desktop-full" implicitly. But VyomaOS
supports 6 platform profiles (CLAUDE.md):

- `mcu-minimal` (128 KB RAM, wasm3 interpreter): No threads. No CFS.
  No SCHED_DEADLINE. No App Nap. Almost nothing in the proposal applies.
  What is the scheduler model here? Likely cooperative round-robin with
  fixed time slices. The proposal doesn't address it.

- `iot-edge` (4 MB, WAMR AOT): Linux but may not have full cgroup support.
  Audio is uncommon. App Nap is critical for battery life. The proposal
  doesn't differentiate.

- `robotics-rt` (8 MB, WAMR AOT): Hard real-time requirements. SCHED_DEADLINE
  may not be enough — may need Xenomai or PREEMPT_RT kernel. WASM execution
  determinism becomes a key question.

- `mobile`: Touch input adds new QoS considerations. Background app suspension
  matches iOS App Nap.

- `desktop-full`: The Architect's target.

- `server-headless`: No UI thread, no UserInteractive QoS makes sense.
  Different policy entirely (throughput-oriented).

The proposal should either (a) explicitly scope to desktop-full and defer
the others to later rounds, or (b) describe how each profile gets a
different scheduler policy bundle. Silently assuming one model is dishonest.

### S7. The `wasmtime::Engine` and Compilation Cache Cost Is Ignored

Each WASM app instance has compilation cost (one-time JIT) and steady-state
memory. The supervisor runs many small apps. Critical scheduler-adjacent
question: under memory pressure (e.g., on mobile or iot-edge), can the
scheduler unload an app's compiled code while preserving its linear memory?

This is similar to macOS's "app jetsamming" — the kernel can terminate
background apps to reclaim memory. The proposal mentions App Nap (CPU
suspension) but not memory pressure response.

Required: define how the scheduler interacts with memory pressure signals
(Round 2 spec). Probably the right answer is "scheduler nominates candidates
for memory reclamation based on App Nap state" — but this is a cross-cutting
concern that needs to be specified.

### S8. Latency-Sensitive WASI Calls Need Identification

A WASM app calling `clock_gettime` is a fast WASI call. A WASM app calling
`fd_read` on a 9P-backed file may block on host I/O for tens of milliseconds.

If a UserInteractive app makes a blocking WASI call inside its UI frame
budget, the frame is missed. The scheduler can't help — the app voluntarily
yielded into a slow WASI call.

macOS distinguishes "blocking syscalls in main thread of UI app" with
warnings and instrumentation. The Architect's proposal should specify:

- A WASI-call-latency classification table (fast / slow / I/O-bound)
- A policy for UserInteractive apps making slow calls (warn? trap? deadline-enforce?)
- A development-mode "main thread checker" that aborts on slow calls
  from a thread declared UserInteractive

Without this, the QoS classification becomes purely cosmetic — the actual
latency is dominated by WASI call patterns, not scheduling.

### S9. Wasmtime Async Mode vs Sync Mode Is Not Chosen

Wasmtime supports two modes for WASI:
- **Sync mode**: WASM function calls into host syscall, host blocks the
  OS thread, returns to WASM when complete. Simple, but one OS thread
  per concurrent in-flight syscall.
- **Async mode**: WASM function yields to host via Wasmtime's async
  machinery (Future-based), host runs syscall, WASM resumes when ready.
  Many in-flight syscalls per OS thread.

For a per-app-thread supervisor, sync mode is fine — each app gets one
thread, one in-flight syscall at a time. But async mode would allow
"work pool" patterns (one thread runs many WASM instances' WASI calls
concurrently via futures) — which is closer to what the Architect's
"GCD-style" proposal actually wants.

The proposal does not say which mode is used. This is a fundamental
implementation choice that affects every other design decision.

If async mode is chosen, the supervisor can multiplex many app's WASI
calls onto fewer threads, which improves scaling and enables sophisticated
QoS-aware scheduling of WASI completions. If sync mode is chosen, the
"GCD" claims (already invalid per C1) are doubly so.

Required: explicit choice + justification.

## Design Gaps

### G1. No Treatment of WASM Trap Behavior Under Preemption

When epoch interrupts a WASM function, the function's call stack is unwound
(or paused, depending on Wasmtime configuration). What happens to:

- Memory the WASM had borrowed from a WASI host call (e.g., a pointer
  passed to `fd_read` that the host is writing into)?
- Host resources the WASM had acquired (file descriptors, network
  connections held by Wasmtime's WASI implementation)?

If the supervisor's policy is to *resume* the WASM after preemption, this
is straightforward. If the policy is to *cancel* the WASM (e.g., as part
of throttling), cleanup semantics must be specified.

### G2. No Per-CPU Affinity Model

On multi-core hosts, which app threads run on which cores? Currently the
supervisor leaves this to CFS. But:

- The compositor likely benefits from a dedicated core (or one of a pair)
- WASM apps with heavy linear memory benefit from staying on a core that
  has their pages in L2/L3
- App Nap candidates might be banished to a "background core" to leave
  bigger cores for UserInteractive work

macOS handles this with "performance" and "efficiency" cores on Apple
Silicon, and the scheduler routes QoS classes to core types. Even on
Intel macs the kernel uses CPU affinity hints.

The Architect's proposal makes no mention of CPU topology. For
multi-core targets (most desktop and server profiles) this is a
significant gap.

### G3. Missing Per-App CPU Budget Accounting and Quotas

The proposal mentions "CPU caps" but does not specify:

- How CPU usage is measured (`clock_gettime(CLOCK_THREAD_CPUTIME_ID)`?
  cgroup `cpu.stat`? RUSAGE?)
- Over what window (1-second sliding? per-second tumbling? exponential
  decay?)
- What the cap units are (% of one core? % of all cores? raw CPU-seconds?)
- What happens when an app exceeds its quota (warn, throttle, kill?)
- Whether apps can request a higher quota at runtime via IPC

This is foundational: without a clear accounting model, the entire
"CPU management" claim of this round is hollow.

### G4. No Specification of Cooperative Yield API

In sync mode, a WASM app cannot voluntarily yield to other apps without
making a WASI call. macOS apps can call `pthread_yield_np()` or
`dispatch_async` to cooperatively yield. WASM apps in VyomaOS need an
equivalent.

Two options:
- A WASI extension `vyoma.yield()` that returns to the scheduler
- Implicit yield at epoch ticks (already exists)

The Architect's proposal should specify whether an explicit yield API
is needed and what its semantics are (e.g., "yield until any IPC message
arrives" vs "yield for at least N ms" vs "yield immediately").

### G5. No Handling of WASM Component Model / WASI 0.3 Futures

WASIp2 introduces async via the component model (futures and streams).
A WASM app awaiting a future is not in a WASM execution state — it's
suspended at the component model layer. How does the scheduler treat
this?

If the supervisor doesn't recognize suspended-on-future as "idle," it
will give CPU time to threads that have nothing to do. If it does, it
needs to subscribe to the component model's wake-up mechanism.

Round 5 should at least flag this and defer the design to a coordination
round with WASI Preview 3 adoption (probably Round 25+).

### G6. Initial Boot and Cold-Start Scheduling Is Not Discussed

When the system boots, 10+ WASM apps start simultaneously (per CLAUDE.md
current state). Each must JIT-compile, instantiate, run startup code.
What scheduler policy applies during this cold-start period?

Common practice: a higher CPU budget during the first N seconds, then
ramp down to steady-state policies. The Architect's proposal silently
applies steady-state QoS from t=0, which may produce a sluggish boot
experience.

This matters for the < 5-second boot target documented in CLAUDE.md.

### G7. Scheduler Telemetry and Debuggability

How does a developer diagnose "my app is sluggish"? The supervisor's
`ps` command (per CLAUDE.md) shows running apps but not scheduler state.
The Architect should specify:

- A `sched-stat <app>` command showing thread's current QoS class,
  nice value, CPU usage, App Nap state, last yield reason
- Per-app CPU time accounting in `/data/logs/<app>/sched.log`
- A "main thread checker" warning in development mode for slow WASI calls
  on UserInteractive threads
- Integration with the existing Round 0 observability/heartbeat emitter

Without these, the scheduler is a black box and its bugs are unfixable
in the field.

### G8. Interaction with Watchdog Capability

CLAUDE.md documents a `watchdog_secs` capability: "supervisor kills app
if silent for N seconds." This interacts with App Nap (a napped app is
silent by definition) and with throttling (a throttled app may appear
silent if the throttling is severe enough).

The Architect's proposal doesn't reconcile these:
- Does App Nap pause the watchdog timer? (Should: a napped app isn't
  expected to produce output.)
- Does throttling pause the watchdog? (Maybe: depends on whether the
  throttled app is "supposed to be working.")
- If both are active and the watchdog fires, who wins?

Specification required.

### G9. No Treatment of Multi-Window Apps Whose Windows Have Different QoS

The Round 9 compositor (planned) and the current Phase 17 multi-window
work allow one app to have multiple windows. If the user is interacting
with window A (UserInteractive priority) and window B is occluded
(Background priority), what's the app's QoS?

Two options:
- Per-thread QoS: but each WASM app has one thread, so this is impossible
  without restructuring
- Aggregated: highest QoS of any window. This is the macOS default.

The Architect should specify and document the limitation that VyomaOS
apps cannot have per-window QoS due to WASM single-threadedness.

### G10. Scheduler Lacks an Explicit Failure Mode

When the scheduler itself misbehaves — wrong QoS class assignment,
priority inversion deadlock, App Nap suspending a critical app — what's
the recovery?

- A "panic mode" that resets all QoS to default and disables App Nap?
- Logged scheduler state dumps?
- A bypass mechanism (kernel command line param, supervisor flag) to
  disable the scheduler entirely and fall back to vanilla CFS?

For a young system like VyomaOS, an emergency escape hatch is essential.
The Architect should specify one.

## Points of Strength

Despite the fundamental issues, the proposal has some correct intuitions
worth preserving in the synthesis:

- **The QoS class taxonomy** (UserInteractive, UserInitiated, Utility,
  Background) is well-established from macOS/iOS and maps reasonably to
  user mental models. Even if the implementation needs rework, the
  *vocabulary* is appropriate.

- **Recognition that audio needs special treatment** is correct, even if
  SCHED_FIFO is the wrong mechanism. Audio is the canonical case for
  RT scheduling and the proposal at least flags it.

- **The intent to throttle background CPU** is correct from a power and
  thermal management standpoint. Mobile and iot-edge profiles cannot ship
  without aggressive background throttling. The proposal acknowledges
  this even if the mechanism is wrong.

- **App Nap as a concept** maps well to the WASM-app-per-thread model
  in principle: a suspended thread costs zero CPU. The implementation
  challenges (cross-subsystem state, IPC interactions) are surmountable
  with the right design.

- **Citing macOS as the design north star** is the correct strategic
  framing for VyomaOS's "desktop-class capability-secure OS" positioning.
  Even where the macOS mechanism is wrong for our substrate, the macOS
  *user-visible behavior* (snappy UI, polite backgrounds, battery-aware)
  is the right target.

- **Acknowledging the difference between user-interactive and background
  work** at the OS level is structurally important. Linux's default
  fairness is wrong for a desktop OS; the proposal correctly identifies
  this and moves to fix it.

## Synthesis Recommendations

The synthesizer should consolidate this round around the following
revised model:

### Architecture: "Per-App Thread + QoS Bias + WASI-aware Accounting"

1. **One OS thread per WASM app instance.** Already the supervisor's
   model. Document explicitly. Do not introduce GCD-style work pools.
   The "work item" concept either doesn't exist or is implemented inside
   the WASM module via component-model futures (Round 25+).

2. **QoS classes as a thread-policy tuple, not an API.** Each WASM app
   declares its QoS in `vyoma.toml` (or has one assigned dynamically by
   user focus / window state). The supervisor maps QoS → `(nice value,
   scheduler policy, CPU affinity hint, App Nap eligibility, timer leeway)`.

3. **Audio uses SCHED_DEADLINE, not SCHED_FIFO.** Apps declaring an
   `audio_realtime` capability get a separate thread (not their main
   thread) under SCHED_DEADLINE with declared `(runtime, deadline, period)`.
   Subject to RLIMIT_RTTIME ceiling. Watchdog co-process on a separate
   core. Capability requires explicit allowance in manifest with signed
   approval (capability-secure constraint).

4. **CPU accounting via `CLOCK_THREAD_CPUTIME_ID`, not epochs.** Sample
   at WASI boundaries and at epoch yields. Maintain per-app exponentially
   weighted moving average CPU consumption.

5. **CPU throttling via cgroups, not epoch manipulation.** Each app's
   thread is in a cgroup with `cpu.max` set per QoS class. Throttling
   means tightening `cpu.max`. Epoch frequency is *fixed* (e.g., 10ms)
   and used only for preemption-point density, not bandwidth.

6. **Activity tokens for App Nap eligibility.** Round 3 IPC must be
   amended to propagate `os_activity`-style tokens. The scheduler
   consults the token registry rather than direct subsystem state.
   App Nap suspension requires "no active assertions" not just "no
   recent activity."

7. **Priority inheritance in the IPC broker.** When a high-QoS app
   blocks waiting for a reply from a low-QoS app, the broker temporarily
   promotes the callee's QoS. Cycle detection for deadlock prevention.
   Explicit timeout semantics for high-QoS waiters.

8. **Tiered timer leeway.** UserInteractive timers fire precisely;
   Background timers coalesce with multi-second leeway. Per-app jitter
   within tier to avoid wakeup synchronization.

9. **Compositor as a privileged subsystem.** Document that the supervisor's
   compositor runs under SCHED_DEADLINE on a pinned CPU. Not a WASM app,
   not subject to QoS. Has its own deadline-miss accounting.

10. **Platform-profile-specific policy bundles.** desktop-full,
    server-headless, mobile, robotics-rt, iot-edge, mcu-minimal each
    get a documented scheduler policy bundle. mcu-minimal is cooperative
    round-robin only. server-headless drops UserInteractive and tunes
    for throughput.

11. **Emergency escape hatches.** Boot parameter `vyoma.sched=off`
    disables the QoS system and falls back to vanilla CFS. Useful for
    debugging and as a safety valve.

12. **Explicit deferral.** WASM futures / async (Round 25+), per-window
    QoS (Round 9 follow-up), and memory-pressure-driven jetsamming
    (Round 2 follow-up) are deferred to future rounds with explicit
    references.

### What to drop

- The "GCD-style concurrent dispatch queues" framing. Replace with
  "per-app priority bias."
- "Epoch-based throttling." Replace with cgroup-based throttling.
- "SCHED_FIFO for audio." Replace with SCHED_DEADLINE + RLIMIT_RTTIME.
- "Subscription-based App Nap detection." Replace with assertion-based
  App Nap eligibility.

### What to defer

- Per-CPU affinity model (needs CPU topology spec)
- WASI-call-latency classification table (needs WASI Preview 3 alignment)
- Multi-window QoS (needs Round 9 compositor coordination)
- Memory-pressure interaction (needs Round 2 finalization)

### What to elevate

- The QoS vocabulary (UserInteractive, UserInitiated, Utility, Background)
  is the user-facing taxonomy. Adopt as-is.
- Timer leeway tiering by QoS is correct in concept; expand the proposal.
- App Nap as a user-visible behavior promise (with internal mechanism
  rewritten) is desirable.

### Verdict reiterated

**FUNDAMENTAL FLAWS.** The proposal as written cannot be implemented
without a substantive rewrite of four of its six core pillars. The
QoS taxonomy and design intent are sound; the mechanism mapping to
the WASM execution model and Linux substrate is not. The synthesizer
must rebuild around the per-app-thread reality, cgroup-based bandwidth
control, SCHED_DEADLINE for RT, and assertion-based activity tracking
with priority inheritance in the IPC layer.

A revised design along these lines is implementable in the existing
supervisor architecture and consistent with the WASM/Wasmtime constraints
documented in Round 1.

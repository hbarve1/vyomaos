# Kernel & Process Model — Critic Review

**Reviewer:** CRITIC role
**Subject:** VyomaOS supervisor as the foundation of a macOS-equivalent desktop OS
**Scope:** Process model, IPC, crash recovery, scheduling, resource quotas
**Posture:** Brutally specific. Every attack must produce a concrete fix, with Rust types and code references to `supervisor/src/`.

The current design — a single `supervisor` PID-1 binary written in Rust that spawns one `wasmtime` child per app, one OS thread per child for stdin and one for stdout, plus a handful of singleton service threads (input router, mouse, watchdog, screen poller, mgmt server) — is fine for 10 apps and a demo. It is **not** the basis of a macOS-equivalent OS. The faults below explain why, and what to change before any further feature work is layered on top.

---

## Attack 1: Single supervisor bottleneck — global mutex hell

**Problem:** The supervisor pretends to be concurrent because it has many threads, but it serializes nearly every interesting operation behind two global locks: `AppRegistry = Arc<Mutex<HashMap<String, Arc<Mutex<AppState>>>>>` and `Inbox = Arc<Mutex<HashMap<String, mpsc::Sender<String>>>>`. Both are top-level locks acquired on every IPC message, every frame draw, every keyboard event, every restart, every layout reflow. macOS spreads process management across `launchd` (lifecycle), `WindowServer` (compositing), `loginwindow` (session), and the Mach kernel (IPC). VyomaOS funnels all of it through one `Mutex<HashMap>`.

**Evidence (from `supervisor/src/main.rs` and `supervisor/src/app_threads.rs`):**

1. `app_threads::apply_tiling_layout()` takes `registry.lock()` *twice* in one function — once to read app list (line 72), once to write `win_region` (line 92) — with `init_surface_for_region()` reaching into the surface mutex *while still holding the registry lock* (lines 96-102). Add 50 apps with `display=true`, and every spawn/exit holds the global registry lock long enough to allocate 50 surface buffers (potentially MB each) under the lock. This is `O(n)` work under an `O(1)`-expected lock.
2. `app_threads::spawn_io_threads()` (line 191): the reader thread re-acquires `registry_r.lock()` *for every line* of stdout from the app, *twice* in the case of display apps (once to push log_buf, again to read `win_region`). 50 apps each writing a `VYOMA_DRAW:flush` per frame at 60Hz = 3000 lock acquisitions/second on a single global mutex, even before counting `route_or_print()`.
3. `run_watchdog()` (line 449): holds `reg.lock()` for the entire iteration over all apps, then for each app acquires `st.lock()`, then `watchdog_backoff.lock()`, then `last_output.lock()`. A single slow app's `last_output` lock blocks watchdog scanning of every other app.
4. One slow `child_stdin.writeln!` in a `*-writer` thread (line 151) blocks the corresponding `mpsc::Receiver`. The IPC broker has no timeout, no backpressure, no overflow policy — `tx.send()` calls in screen-poll, input-router, and `route_or_print` are unbounded `mpsc::channel()` (line 273). 50 apps each backlogged by 1000 messages = unbounded memory growth, no flow control.
5. The display flush loop (referenced in the recent commits about compositor ABBA deadlock) already proves the lock-order discipline is fragile.

**Fix:**

Replace the central registry with sharded state plus per-subsystem actors.

```rust
// Type-level partitioning: lifecycle, IPC, and rendering own distinct state.
pub struct AppHandle {
    pub id:       AppId,           // newtype, not String; resolves O(1)
    pub control:  Sender<AppCtl>,  // bounded(64), backpressure on overflow
    pub render:   Arc<RenderSlot>, // owned by compositor, not registry
    pub state:    Arc<RwLock<AppState>>,  // RwLock, readers don't block readers
}

pub type AppId = std::num::NonZeroU32;  // 4 bytes, Copy, hashes faster

pub struct AppTable {
    // 16-way sharded hashmap; each shard has its own RwLock.
    shards: [RwLock<HashMap<AppId, AppHandle>>; 16],
    name_to_id: DashMap<String, AppId>,  // lookups don't block lifecycle
}
```

Concrete subsystem split (each owns its own state, communicates via bounded channels):

| Actor | Owns | Channel inbound | Replaces |
|---|---|---|---|
| `LifecycleActor` | spawn/exit/restart, `AppStatus` | `Sender<LifecycleCmd>` cap 256 | `wait_app` loop |
| `IpcRouter` | `inbox` map, message routing | `Sender<IpcEnvelope>` cap 1024 | `route_or_print` |
| `Compositor` | window regions, surfaces, Z order | `Sender<RenderCmd>` cap 4096 | `apply_tiling_layout` + flush |
| `InputDispatcher` | focus, keyboard/mouse routing | `Sender<InputEvent>` cap 256 | `input_keys` + `mouse_input` |
| `Watchdog` | `last_output` per app | timer ticks | `run_watchdog` |

Bounded channels with a documented overflow policy (drop-oldest for input, fail-and-log for IPC, coalesce for redraws) are the single most important change. Today every `mpsc::channel()` in the codebase is unbounded; this *will* OOM under load.

For the "50 apps call IPC simultaneously" case: route via a single `IpcRouter` actor that owns the inbox map. Senders hand it `IpcEnvelope { from: AppId, to: AppId, payload: Bytes }`. The router resolves recipient via `name_to_id` (DashMap, lock-free read), then `try_send` on the recipient's bounded queue. If full, the policy fires (drop / log / kill noisy app). The hot path holds no global mutex.

For the "wasmtime thread deadlocks" case: today there's no detection. A blocked thread silently consumes a stdin pipe and never unblocks `child_stdin.write_all`. Fix: every `child_stdin.write_all` must go through `write_all_timeout(Duration::from_millis(50))` using a worker pool with `nix::poll` or `mio`, not the per-app blocking thread. If the write times out, the app is marked `AppStatus::Unresponsive(since: Instant)` and the watchdog escalates.

For the "slow flush blocks display loop" case: the compositor must own a `SurfaceCache` keyed by `AppId` holding `Arc<Surface>` clones. The flush loop reads surfaces with `RwLock::read()` (no contention with other readers), and slow surface updates from apps go to a private staging surface that swaps atomically when ready (`ArcSwap<Surface>`). No app can stall the compositor.

---

## Attack 2: WASM linear memory ≠ process memory

**Problem:** macOS process model assumes Mach VM: shared regions (`mach_vm_remap`), copy-on-write fork, dynamic growth, mmap'd files, IOSurface for zero-copy display sharing. WASM has none of this. Each WASM module has one `LinearMemory` (a contiguous `Vec<u8>` from the embedder's view), no `mmap`, no shared pages, no COW, no over-commit. wasmtime *can* grow memory up to the declared maximum, but you cannot dynamically raise the maximum after instantiation, and you cannot share pages between modules.

**What breaks in a macOS-like model:**

1. **`fork()`** — no analogue. macOS apps that fork for sandboxed renderers (Safari, Chrome) cannot be modeled. Workaround: every "child process" must be a separately-instantiated WASM module with its own linear memory. A 256 MB app that "forks" actually allocates a fresh 256 MB.
2. **Shared memory IPC** — XPC, Mach ports with `MACH_RCV_OVERWRITE`, POSIX shm. WASM has no syscall path to it; even with `wasm-threads`, sharing is *within* one module, not between modules.
3. **Mmap'd files** — apps cannot `mmap` a file from `/data`. Reads must be `wasi:filesystem/types.read`, copying through linear memory.
4. **GPU IOSurface / drm prime fd handoff** — zero-copy display is structurally impossible. Every `VYOMA_DRAW:` blit copies pixels through the supervisor.
5. **Dynamic memory limits** — manifest declares `max_memory_mb`; instantiation fixes it. A photo editor that wants 4 GB occasionally and 64 MB normally must reserve 4 GB up front.
6. **Address-space separation for plugins** — WASM gives this for free (every module is isolated), but you lose the ability to share pointers, which means every plugin call serializes its arguments.

**Fix (design compensations):**

1. **Define a "WASM process" type explicitly.** Don't try to mimic Unix processes. The unit is `WasmInstance { id: AppId, max_memory: u32 (pages), capabilities: CapSet, peer_links: Vec<PeerLinkId> }`. Document that fork/exec do not exist.

2. **Shared-memory substitute: supervisor-owned `SharedBuffer` capability.**
   ```rust
   pub struct SharedBuffer { id: SbufId, bytes: Arc<RwLock<Vec<u8>>>, owners: SmallVec<[AppId; 4]> }
   // WASI host-function exports:
   //   vyoma:sbuf/types.create(size: u32) -> SbufId
   //   vyoma:sbuf/types.attach(id: SbufId) -> Result<Handle, Denied>
   //   vyoma:sbuf/types.read(h: Handle, offset, len) -> List<u8>      // copies in
   //   vyoma:sbuf/types.write(h: Handle, offset, bytes: List<u8>)     // copies out
   ```
   This is *not* zero-copy — WASM cannot point into supervisor memory — but it is `O(1)` allocation and provides large-buffer semantics for video frames, audio buffers, etc.

3. **File access via streaming, not mmap.** Standard WASI Preview 2 `wasi:filesystem` is already streaming. Document that mmap is not on the roadmap; apps wanting "memory-mapped" semantics get a `SharedBuffer` populated lazily by the supervisor.

4. **GPU: introduce a `GpuSurface` capability.** Supervisor owns DRM dumb buffers; apps issue draw-list commands (the existing `VYOMA_DRAW:` protocol generalised), not pixels. This solves the "every blit copies" problem by moving rasterization to the supervisor's GPU driver path.

5. **Tiered memory limits.** Declare `mem_reserved` (instantiation floor) and `mem_max` (hard cap) in the manifest. Instantiate at `mem_reserved`. Background apps get demoted by re-instantiating from a snapshot at `mem_reserved`; foreground apps grow up to `mem_max` via `wasmtime::Memory::grow`.

6. **State serialization for "fork-like" patterns.** Provide a `vyoma:state/types.snapshot()` host call that serializes app-declared state via `serde_postcard` into supervisor-owned storage. A new instance can be spawned that imports the snapshot. This replaces fork+exec workflows.

---

## Attack 3: "Background execution" is currently a lie

**Problem:** WASI Preview 2 modules execute their `_start` function and exit. There is no event loop *inside* the module — they block on `wasi:io/streams.read` against a pipe or `wasi:clocks/wall-clock.now`. To do anything asynchronously, the app must either (a) block in a `wasi:poll` on a list of pollables, or (b) spin in a `while let Ok(line) = stdin.read_line()` loop. There are no threads inside a `wasm32-wasip2` module (no `wasi-threads`, which would require atomic memory and shared memory — not in P2).

**What the current design glosses over:**
- "Suspended" apps in the macOS sense (App Nap, NSProcessInfo activity tokens) do not exist. An app blocked on `stdin.read_line()` is just blocked; the supervisor cannot wake it on its own timer or deliver a "your download finished" message except by writing to its stdin pipe.
- Timer callbacks: today there are zero. An app wanting "wake me in 5 s" has to spin-block in a stdin-poll loop with a sleep, eating CPU and a wasmtime thread.
- Network completion: the supervisor's `net.rs` apparently brokers TCP via numeric handles (`TCP_CONNS`). The completion model is synchronous request/response.
- Background downloads: nothing in the model lets a backgrounded app continue work; if its stdin pipe isn't being read, the supervisor blocks on stdin writes, deadlocking the whole pipeline.

**Fix — concrete callback-export mechanism:**

Define **WIT-typed callback exports** that the host invokes by *re-entering* the WASM instance on a supervisor thread. wasmtime supports this via `wasmtime::Func::call` from any host thread, provided the instance is single-threaded (we serialize calls per instance with a `Mutex<Store>`).

```wit
// vyoma:lifecycle.wit
package vyoma:lifecycle;

interface callbacks {
    /// Called by host on a timer fire registered via vyoma:time.set-timer.
    on-timer: func(token: u64);

    /// Called when a backgrounded network request completes.
    on-net-complete: func(req-id: u32, status: net-status, bytes: list<u8>);

    /// Called when the app is being suspended; app may persist state, must return < 100ms.
    on-suspend: func() -> result<state-blob, suspend-error>;

    /// Called when a suspended app is resumed with previously-saved state.
    on-resume: func(state: option<state-blob>);

    /// Called when the app is being terminated; last chance to flush.
    on-terminate: func();
}

world vyoma-app {
    export callbacks;
    import vyoma:time/timer;
    import vyoma:net/async;
}
```

Supervisor side (Rust):
```rust
pub struct InstanceHandle {
    store:    Arc<Mutex<wasmtime::Store<HostCtx>>>,
    bindings: vyoma_lifecycle::Callbacks,  // wit-bindgen generated
    queue:    crossbeam::channel::Sender<HostCallback>,
}

enum HostCallback {
    Timer(u64),
    NetComplete { req_id: u32, status: NetStatus, bytes: Bytes },
    Suspend(oneshot::Sender<Result<StateBlob, SuspendError>>),
    Resume(Option<StateBlob>),
}

// One dispatcher thread per app instance. Serializes callbacks into the Store.
fn callback_dispatcher(handle: InstanceHandle, rx: Receiver<HostCallback>) {
    while let Ok(cb) = rx.recv() {
        let mut store = handle.store.lock().unwrap();
        let result = match cb {
            HostCallback::Timer(t) => handle.bindings.callbacks().call_on_timer(&mut *store, t),
            HostCallback::NetComplete { req_id, status, bytes } =>
                handle.bindings.callbacks().call_on_net_complete(&mut *store, req_id, status, &bytes),
            HostCallback::Suspend(reply) => {
                let r = handle.bindings.callbacks().call_on_suspend(&mut *store);
                let _ = reply.send(r.unwrap_or_else(|e| Err(SuspendError::Trap(e.to_string()))));
                continue;
            }
            HostCallback::Resume(s) => handle.bindings.callbacks().call_on_resume(&mut *store, s.as_ref()),
        };
        if let Err(trap) = result {
            // Re-entry trapped — treat as crash. See Attack 4.
            handle_callback_trap(&handle, trap);
            break;
        }
    }
}
```

**Implications this changes about the current code:**
- `spawn_io_threads` (stdin/stdout pipes) becomes a *fallback* for legacy or printf-only apps. New apps export callbacks instead.
- The `*-writer` thread that does `writeln!(stdin, "{msg}")` is replaced by `cb_tx.send(HostCallback::Ipc { ... })` and a callback re-entry.
- "Background execution" means an instance with no foreground priority that still receives `on-timer` and `on-net-complete` callbacks.
- "Suspend" means draining `cb_tx`, calling `on-suspend`, persisting the returned `StateBlob` to `/data/state/<app>.bin`, then dropping the `Store` and freeing linear memory. Restore re-instantiates and delivers `on-resume(state)`.

This is the only honest path. The current design pretending stdin lines are an event loop will not survive contact with NSTimer or `URLSession` semantics.

---

## Attack 4: Crash detection is incomplete and conflated

**Problem:** wasmtime trap categories (out-of-bounds memory, stack overflow, integer overflow, unreachable, fuel exhaustion, host error) all show up as a non-zero process exit when run via `wasmtime run` as a child process. Clean `wasi:cli/exit.exit(0)` produces exit code 0. `wasi:cli/exit.exit(42)` produces 42. The supervisor sees only the exit code and cannot distinguish trap from clean exit from kill from `proc_exit(N)`.

**Evidence (from `supervisor/src/app_threads.rs:378-381`):**
```rust
let exit_code = match child.wait() {
    Ok(s) => { let c = s.code().unwrap_or(-1); ... c }
    Err(e) => { ... -1 }
};
```

That's it. `child.wait()` returns `ExitStatus`, of which the supervisor extracts only `.code()`. It throws away `.signal()` entirely. On Linux a process killed by SIGKILL has `code() == None` and `signal() == Some(9)`; a process that segfaults has `signal() == Some(11)`. wasmtime traps are translated by `wasmtime run` into a non-zero exit code but the *trap reason* (string) is only on stderr, which is `Stdio::inherit()` (line 304) — gone to the supervisor's stderr, not parsed.

**All the ways an app can die — and whether the supervisor distinguishes them:**

| Failure mode | What `child.wait()` returns | Currently distinguished? | Should be |
|---|---|---|---|
| Clean exit `proc_exit(0)` | `code() = Some(0)` | yes (treated as not-restart) | yes |
| App error `proc_exit(N)` | `code() = Some(N)` | partially (restart on != 0) | yes, with reason code |
| Trap: OOB memory | `code() = Some(134 or 137)` | NO — looks like signal | distinct: `CrashKind::MemoryOOB` |
| Trap: stack overflow | `code() = Some(134)` | NO | distinct: `CrashKind::StackOverflow` |
| Trap: integer overflow | `code() = Some(134)` | NO | distinct |
| Trap: unreachable | `code() = Some(134)` | NO | distinct |
| Trap: fuel exhausted | `code() = Some(2 or 134)` | NO | distinct: `CrashKind::CpuLimit` |
| OOM (linear memory grow failed) | varies | NO | distinct: `CrashKind::MemLimit` |
| Killed by SIGKILL (watchdog) | `code() = None, signal() = Some(9)` | NO — turned into -1 | distinct: `CrashKind::WatchdogKill` |
| Killed by OOM-killer (host) | `code() = None, signal() = Some(9)` | NO — indistinguishable from above | distinct: `CrashKind::HostOOM` |
| wasmtime itself crashed | varies, often signal | NO | distinct: `CrashKind::RuntimeFault` |
| Pipe broken (stdout EOF before exit) | `child.wait()` not yet returned | NO — reader loop exits silently | distinct: `CrashKind::ProtocolFault` |

The current code at line 412:
```rust
let should_restart = matches!(entry.restart.as_str(), "always" | "on-failure")
    && (entry.restart == "always" || exit_code != 0);
```
restarts on *any* non-zero exit. A misbehaving app that traps every second on OOB memory gets restarted forever with no backoff (the backoff in `watchdog_next_backoff` only applies to watchdog-kills, not exit-restarts). This is a guaranteed CPU-burn loop after any deterministic crash.

**Fix:**

Stop running apps via `wasmtime run` as a subprocess. **Embed wasmtime directly** in the supervisor as a library; `wasmtime::Trap` gives you the exact `wasmtime::TrapCode` (`MemoryOutOfBounds`, `StackOverflow`, `IntegerOverflow`, `Unreachable`, `OutOfFuel`, …) and `WasmBacktrace`.

```rust
#[derive(Debug, Clone, serde::Serialize)]
pub enum CrashKind {
    Clean,
    AppExit(i32),
    Trap { code: wasmtime::TrapCode, backtrace: String },
    MemLimit { requested_pages: u32, max_pages: u32 },
    CpuLimit { fuel_consumed: u64 },
    WatchdogKill { silent_for_ms: u64 },
    HostOOM,
    RuntimeFault(String),
    ProtocolFault(String),
}

pub struct ExitReport {
    pub app:    AppId,
    pub kind:   CrashKind,
    pub uptime: Duration,
    pub last_lines: SmallVec<[String; 8]>,  // tail of stderr/log
}

pub fn classify_exit(child_status: ExitStatus, trap_log: Option<wasmtime::Trap>) -> CrashKind { ... }

pub fn restart_policy(kind: &CrashKind, restart_count: u32) -> Decision {
    match kind {
        CrashKind::Clean => Decision::DoNotRestart,
        CrashKind::AppExit(0) => Decision::DoNotRestart,
        CrashKind::AppExit(_) | CrashKind::Trap { .. } => {
            // Deterministic trap: exponential backoff, max 5 attempts, then give up.
            if restart_count >= 5 { Decision::Quarantine } else { Decision::RestartAfter(backoff(restart_count)) }
        }
        CrashKind::WatchdogKill { .. } => Decision::RestartAfter(backoff(restart_count)),
        CrashKind::CpuLimit { .. } => Decision::Throttle { fuel_multiplier: 0.5 },
        CrashKind::MemLimit { .. } => Decision::Quarantine,  // tunable per-app
        CrashKind::RuntimeFault(_) => Decision::PanicSupervisor,  // wasmtime itself broken
        CrashKind::ProtocolFault(_) => Decision::RestartAfter(Duration::from_secs(1)),
        CrashKind::HostOOM => Decision::DoNotRestart,  // host is dying, don't pile on
    }
}
```

The supervisor must own this taxonomy and persist `ExitReport`s to `/data/crash-reports/<app>-<ts>.json` for future `vyoma-debug` tooling. Replacing the subprocess model with embedded wasmtime is the prerequisite. It also fixes the "wasmtime per-process memory overhead" issue (each `wasmtime run` invocation rebuilds the JIT engine).

---

## Attack 5: Boot sequence has at least three race conditions

**Problem:** `main.rs` starts a sequence: mount, init display, parse boot.toml, spawn apps in a pass-1 loop, then *while spawning apps*, the input-router, mouse-input, mgmt-server, screen-poll, and watchdog threads are kicked off. Apps' IO threads (`spawn_io_threads`) are started in pass-2. Each spawned app's first action is typically to emit `VYOMA_DRAW:` commands that target `/dev/fb0` via the singleton `display::get()` mutex.

**Race conditions visible in the code:**

**Race A — Display before init:** `app_threads::launch_app_threads` (line 220-225) sends `VYOMA_SYSTEM:screen:WxH` to a freshly-spawned app, then immediately the reader thread can deliver app stdout to `route_or_print` which calls `display::dispatch(...)`. But the initial menu bar / desktop fill (main.rs lines 277-283) ran *before* `Pass 2` started any reader. If 10 apps print `VYOMA_DRAW:fill_rect` simultaneously before `chrome::draw_menubar` returns, the menu bar can be overdrawn. There's no ordering guarantee.

**Race B — Inbox lookup vs spawn:** `apply_tiling_layout` is called inside `launch_app_threads` (line 212) and re-acquires `app_registry.lock()` while `spawn_io_threads` will later acquire it too. The inbox map is written in `spawn_app` (line 274) *before* the app's reader thread exists. If app A starts emitting messages to app B before B's reader is running, the message is delivered to B's pipe via `cb_tx.send` (`tx.send` actually, line 446 of `main.rs`) but no one is reading the receiver. The unbounded `mpsc::channel` accumulates.

**Race C — Focus lottery:** main.rs line 376-381 sets focus to "the first shell app". But shells declare `shell = true` and could legitimately fail to spawn. If two shells are present (during dev/testing) there's no defined ordering — `find()` returns whichever the iterator yields first, which is the boot.toml order. Now consider the input router (spawned at line 386) reading `/dev/tty0` and looking up `focused.lock().unwrap()`. If a keystroke arrives before line 380 executes, the focused entry is `None` and the keystroke is dropped silently.

**Concrete deadlock scenario at boot:**
```
T0: Main thread holds nothing.
T1: Spawning app A (display=true). Calls apply_tiling_layout.
    apply_tiling_layout: acquires registry.lock() at line 72.
    Inside, calls init_surface_for_region which calls Surface::new(ww, content_h).
    Surface::new allocates Vec<u32>; under heap pressure this can call into
    a global heap allocator that takes its own mutex.
T2: Meanwhile, screen-poll thread fires (started at line 429). It detects a
    size change, acquires registry_resize.lock() at line 440 — BLOCKED on T1.
    But it ALSO holds inbox_resize.lock() at line 441 (acquired BEFORE the
    registry lock in screen-poll, in REVERSE order vs. spawn).
T3: Meanwhile, route_or_print from app C's reader thread tries to acquire
    inbox.lock() to deliver an IPC message — BLOCKED on T2.
T4: Meanwhile, T1's apply_tiling_layout finishes its first registry.lock()
    scope (line 84) and wants to acquire it again at line 92 to write
    win_region. It also wants to send VYOMA_SYSTEM via inbox.lock().
    BLOCKED on T2 holding inbox.
```
This is the classic A→B vs B→A lock order inversion. Today it likely doesn't fire because boot is fast and screen size doesn't change in QEMU mid-boot, but in production with hotplugged displays it will.

**Fix:**

1. **Define a strict global lock-order discipline.** Document it in `supervisor/src/lib.rs`:
   ```rust
   //! LOCK ORDER (acquire in this order; never reverse):
   //!   1. AppTable shard locks
   //!   2. Per-app AppState
   //!   3. Inbox sender (bounded, try_send only)
   //!   4. Display framebuffer
   //!   5. Surface buffers
   //! Holding any of the above forbids acquiring a lower-numbered lock.
   ```
   Add `debug_assert!` checks in the dev build using a `LockTracker` thread-local.

2. **Use a phased boot.** Introduce explicit boot phases with barriers:
   ```rust
   enum BootPhase { Mount, Display, Profile, Registry, IpcRouter, Compositor, Subsystems, Apps, Ready }
   pub struct BootBarrier { phase: AtomicU8, wakers: Mutex<Vec<Waker>> }
   ```
   No background thread (screen-poll, watchdog, input) starts before `BootPhase::Ready`. Apps' reader threads do not deliver `route_or_print` outputs until the IpcRouter is in `Ready`.

3. **Eliminate the "two passes over apps" structure.** It exists only because the original code wanted to register all inboxes before any IO. With the actor model from Attack 1, a single `LifecycleCmd::Spawn(manifest)` creates the AppHandle (inbox, state, surface) atomically before any IO starts.

4. **Replace `mpsc::channel` for inbox with `crossbeam::channel::bounded`** with documented overflow policy. Race B becomes impossible: if recipient hasn't started, `try_send` returns `TrySendError::Disconnected` and the sender gets an error it can handle.

5. **Make focus deterministic.** Manifest field `[app] focus_priority: i32`; on boot, pick highest priority among shells. Document the rule.

6. **Per-resource readiness gates.** `/dev/fb0` open, keyboard raw-mode init, mgmt server bind — each gates apps that require them. The boot sequence delivers `VYOMA_SYSTEM:ready` to each app *only* once its required resources are live.

---

## Attack 6: Resource quotas are theatre

**Problem:** The current design has *no* CPU enforcement (manifest has `watchdog_secs` for stuck apps but no fuel/epoch limits) and *no* per-app memory limit beyond what `wasmtime run` defaults to (4 GB max memory). The `draw_ticks` / `last_cpu_reset` fields in `AppState` (lines 103-104) appear to count draw commands but there's no enforcement loop visible. cgroups would require root, container plumbing, and a writable cgroupfs — none of which the current minimal kernel config has enabled.

**Realistic options assessed:**

| Mechanism | Granularity | Cost | Verdict for VyomaOS |
|---|---|---|---|
| Wasmtime fuel | Every instruction | ~20-40% overhead in instruction-heavy code | NO for hot apps, YES for untrusted |
| Wasmtime epoch interruption | Async preemption at backedges/calls | ~1-3% overhead | YES — this is the right tool |
| cgroups v2 cpu.max | Per cgroup, kernel-enforced | ~0% | YES *if* you accept the kernel config cost |
| cgroups v2 memory.max | Per cgroup, kernel-enforced | ~0% | YES, same caveat |
| rlimit RLIMIT_AS | Per process | ~0% | partial — only address space, not RSS |
| Wasmtime ResourceLimiter trait | Linear memory grow hook | ~0% | YES — already supported by wasmtime |
| Manual watchdog (current) | 1Hz polling | ~0% but coarse | only as backstop |

**Fix:**

**CPU — use Wasmtime epoch interruption, not fuel.** Epoch interruption sets a deadline; wasmtime checks it at function entry and loop backedges (a handful of cycles), and yields the instance. The supervisor advances the epoch via a single global timer thread.

```rust
pub struct EpochController {
    engine: wasmtime::Engine,
    deadline_ms: AtomicU64,
}

impl EpochController {
    pub fn start_tick(&self, period: Duration) {
        let engine = self.engine.clone();
        std::thread::spawn(move || loop {
            std::thread::sleep(period);
            engine.increment_epoch();
        });
    }
}

// Per-instance config:
let mut config = wasmtime::Config::new();
config.epoch_interruption(true);
let engine = wasmtime::Engine::new(&config)?;
store.set_epoch_deadline(quota_ticks);
store.epoch_deadline_callback(|mut store| {
    // Called on every deadline hit. Decide: extend, yield (suspend), or trap.
    let app = store.data().app_id;
    match scheduler::decide(app) {
        SchedDecision::Continue(extra) => Ok(wasmtime::UpdateDeadline::Continue(extra)),
        SchedDecision::Yield => Ok(wasmtime::UpdateDeadline::Yield(1)),  // re-enter scheduler
        SchedDecision::Kill => Err(wasmtime::Trap::OutOfFuel.into()),
    }
});
```

This gives ~1-3% overhead, async preemption, and a clean integration point with the scheduler — apps in the background simply get fewer epoch ticks.

**Memory — use Wasmtime's `ResourceLimiter` trait.**
```rust
pub struct AppLimiter {
    pub max_memory_bytes: usize,
    pub max_table_elements: usize,
    pub current_memory: AtomicUsize,
}

impl wasmtime::ResourceLimiter for AppLimiter {
    fn memory_growing(&mut self, current: usize, desired: usize, _max: Option<usize>) -> wasmtime::Result<bool> {
        if desired > self.max_memory_bytes {
            return Ok(false);  // returns OOM trap to the guest
        }
        self.current_memory.store(desired, Ordering::Relaxed);
        Ok(true)
    }
    fn table_growing(&mut self, _c: usize, desired: usize, _m: Option<usize>) -> wasmtime::Result<bool> {
        Ok(desired <= self.max_table_elements)
    }
}

store.limiter(|ctx| &mut ctx.limiter);
```
Zero overhead in the common case, hard enforcement at grow time.

**IO — explicit per-app quotas at the host-function layer.**
- Bytes/sec to stdout: track per-app, drop frames or rate-limit `VYOMA_DRAW:` if exceeded.
- TCP connections: cap via `TCP_CONNS.lock().filter(|c| c.owner == app).count()`.
- Filesystem bytes/sec: track in WASI host bindings.

**Disk — no easy answer without cgroups.** Punt to v2; document the gap.

The key change: the current `watchdog_secs` and `draw_ticks` fields stay as *secondary* signals; the primary enforcement moves into wasmtime config. The Cargo dependency cost is zero (wasmtime is already there).

---

## Attack 7: Missing macOS process model features

Below is the inventory. Status legend: **REQ** = required for a credible v1 desktop, **DEFER** = acceptable to omit until v2, **NEVER** = explicitly out of scope.

| Feature | macOS provider | Currently in VyomaOS? | v1 Status | Effort to add |
|---|---|---|---|---|
| App bundle ID + reverse-DNS naming | `CFBundleIdentifier` | partial (manifest `app.name`) | REQ | S — add `bundle_id: String` to manifest |
| Multi-instance launch | `NSApplicationDelegate` opt-in | NO — `inbox` is keyed by app name (one instance) | REQ | M — `AppId` newtype, instance-keyed inbox |
| LaunchServices / URL handler registration | `LSSetDefaultHandlerForURLScheme` | NO | REQ | M — `[handlers]` section in manifest, registry |
| XPC services | `xpc_connection_t` | NO — only stdio IPC | REQ | L — see Attack 1 IPC router; XPC ≈ typed RPC |
| Mach ports | kernel | NO | NEVER (capability handles replace) | — |
| Login sessions, user accounts | `loginwindow`, opendirectoryd | NO — single user | REQ for multi-user, DEFER for single-user v1 | L if multi-user |
| Keychain / secrets | `Security.framework` | NO | REQ | M — supervisor-owned encrypted KV |
| Notifications | `UNUserNotificationCenter` | partial (`toast.rs` for crash) | REQ | M — generalize toast → notification center |
| App Nap / energy management | kernel + `NSProcessInfo` | NO | DEFER for desktop | L |
| State restoration after kill | `NSWindowRestoration` | NO | REQ | M — see Attack 3 `on-suspend`/`on-resume` |
| Crash reports / `~/Library/Logs/DiagnosticReports` | ReportCrash | NO | REQ | S — see Attack 4 `ExitReport` persistence |
| Sandboxing entitlements | `sandbox_init` | partial (manifest caps) | REQ | manifest model extension |
| App Groups (shared containers) | sandbox + entitlements | NO | DEFER | M |
| File coordination (NSFileCoordinator) | `filecoordinationd` | NO | DEFER | M |
| Spotlight / metadata indexing | `mds`, `mdworker` | NO | DEFER | L |
| Print system (CUPS) | `cupsd` | NO | DEFER | XL |
| Accessibility / AX API | accessibilityd | NO | REQ for compliance | XL |
| Per-app firewall / pf rules | `pfctl` + `socketfilterfw` | NO | DEFER | M |
| Time Machine snapshot integration | APFS + `tmutil` | NO | DEFER | XL |
| Code signing / notarization | `codesign`, Gatekeeper | partial (`wasm_sha256` field) | REQ | M — extend to signature, not just hash |
| Quarantine / first-launch dialog | LaunchServices + Gatekeeper | NO | REQ | S — implement once code signing is in |
| Dynamic library loading | `dlopen` | NO (WASM cannot) | NEVER | — |
| Process priorities / `nice` | scheduler | NO — equal threads | REQ | S — `[scheduling]` in manifest, epoch budget |
| Activity Monitor data | host_statistics / proc | partial (mgmt server) | REQ | S — extend mgmt protocol |
| Per-app input methods | `TextInputSources` | NO | DEFER | L |
| Pasteboard / clipboard | `pbcopy`, NSPasteboard | partial (`CLIPBOARD` static) | REQ | S — generalize to typed pasteboard |
| Drag and drop | NSDraggingSession | NO | REQ | M |
| Global shortcuts | `CGEventTap` | NO | REQ | S — extend input router |
| AppleEvents / Apple Script | `osascript` | NO | NEVER (replaced by IPC) | — |
| Universal Links | LaunchServices | NO | DEFER | M |
| Document-based app model | NSDocument | NO | REQ for productivity apps | L |
| Background tasks (BGTaskScheduler) | duet | NO | REQ | M — combine with Attack 3 callbacks |
| Push notifications (APNs) | `apsd` | NO | DEFER | XL |
| App extension / share sheet | extensionkitd | NO | DEFER | XL |

**Effort scale:** S = days, M = 1-2 weeks, L = 1-2 months, XL = quarter+.

**Critical v1 omissions and their cost:**

1. **Multi-instance (`AppId` vs `app_name`)** — must be fixed before any feature using window-restore, multi-document, or multi-window apps. The `Inbox = HashMap<String, _>` keyed on name *cannot* support a second instance of the same app. ETA: 1 week.
2. **XPC-equivalent typed RPC** — required for any non-trivial app talking to a system service (Files, Calendar). Today there's only stringly-typed `@app:` messages. ETA: 2-3 weeks using `wit-bindgen` and the WIT-typed callbacks from Attack 3.
3. **State restoration** — required for "crash recovery" claim. Without `on-suspend`/`on-resume`, every restart loses everything. ETA: 1-2 weeks once Attack 3 callbacks land.
4. **Crash reports** — required for any kind of in-field debugging. ETA: days, once Attack 4 classification is in.
5. **Code signing (not just SHA-256)** — current `wasm_sha256` is a hash, not a signature. There's no notion of *who* signed the binary. Required for any package-distribution story. ETA: 1 week using ed25519 + a root cert pinned in supervisor.
6. **Notification center** — `toast.rs` only handles crash notifications today; needs to be a system service apps can post to. ETA: 1 week.
7. **Drag and drop + clipboard typing** — `CLIPBOARD: OnceLock<Mutex<String>>` is a single string. Real pasteboard needs typed payloads (image, file URLs, rich text). ETA: 1 week.
8. **Per-app scheduling priority** — directly tied to Attack 6's epoch budgets. Without it, the dock and a CPU-heavy app compete equally. ETA: days once epoch interruption lands.

Total v1 critical-path additions: ~8-10 weeks of focused work, assuming Attacks 1-6 fixes are landed first (those are prerequisites).

---

## Summary scorecard

| Attack | Severity | Blocking? | Fix complexity |
|---|---|---|---|
| 1 — global mutex bottleneck | HIGH | YES — perf wall at ~15 apps | HIGH (architectural refactor) |
| 2 — WASM memory ≠ process memory | HIGH | YES — design assumptions wrong | MEDIUM (new capability primitives) |
| 3 — background execution is fake | CRITICAL | YES — no path to macOS parity | HIGH (WIT-typed callback ABI) |
| 4 — crash detection is conflated | MEDIUM | YES — silent crash loops in prod | MEDIUM (embed wasmtime, not subprocess) |
| 5 — boot race conditions | MEDIUM | NOT YET — will surface with hotplug/multi-display | LOW (lock-order discipline + phased boot) |
| 6 — quotas are theatre | HIGH | YES — one bad app kills the box | LOW-MEDIUM (epoch interruption + ResourceLimiter) |
| 7 — missing macOS features | HIGH (cumulative) | for v1 GA, YES | 8-10 weeks of focused work |

**Recommendation:** Do not layer more chrome, animation, or app features on top of the current supervisor until Attacks 1, 3, 4, and 6 are addressed. The work is largely subtractive (delete `wasmtime run` subprocesses, delete unbounded mpsc, delete global registry mutex) and replaces them with primitives already available in `wasmtime` 43.0.0 and `crossbeam`. None of these fixes require new external dependencies beyond what is already in `supervisor/Cargo.toml`.

The Phase-17 milestone ("10 concurrent apps") is real. Scaling that to 50 with macOS semantics is not a 5x effort — it is an architectural reset. Better to do it now, at ~30 source files, than after a chrome-fidelity push doubles the surface area.

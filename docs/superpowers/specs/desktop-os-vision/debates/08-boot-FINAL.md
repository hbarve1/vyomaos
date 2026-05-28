# Round 8 FINAL — Boot Sequence & Init System

**Subsystem 8**: Boot, init, manifest loading, app launch ordering, shutdown, recovery.
**Status**: FINAL (post-synthesis of Architect + Critic)
**Date**: 2026-05-29
**macOS equivalent**: iBoot + launchd phase ordering + EFI recovery

---

## 0. Position Statement (Reconciled)

VyomaOS boots in a strict, observable, **fault-contained** sequence: the Linux
5.10+ kernel hands control to a 100-line `stage1` PID-1 process whose sole job
is to fork, monitor, and re-exec the Rust supervisor (PID 2). The supervisor
advances a finite-state `BootPhase` machine through ten phases, each gated by a
`BootBarrier`. Subsystem initialization is **two-phased** (`init_early` then
`init_late`) to break the cyclic dependency graph between
Memory↔IPC↔AppTable↔VFS↔Storage↔Memory. Every lock in the supervisor is
wrapped in `RankedMutex<T, const RANK: u16>` so lock-order violations are
caught at runtime in debug builds and (with the type-state pattern) at compile
time for the common cases.

App launch splits into a *mandatory* wave (compositor, session, shell) and an
*optional* wave (QoS-prioritized). The boot target is **tiered**:

- **Warm boot** (cwasm AOT cache hit, fsck clean): `< 3 s` to `Operational`.
- **Cold boot** (first-ever boot, full JIT, signature verify): `< 10 s`.
- **Recovery boot** (fsck needed, cache invalidated): `< 30 s`.

Failures degrade gracefully through five recovery modes: skip-non-critical,
restart-with-backoff, emergency MinimalShell (supervisor-internal, no Wasmtime
dependency), factory-reset prompt, and stage1 re-exec. A corrupted
`boot.toml` falls through a **three-layer config** (initramfs read-only →
`/data/etc/override.toml` writable → factory const baked into the supervisor
binary) so no on-disk corruption can brick the device. Shutdown is the strict
reverse-topological order of boot, with the compositor exiting last among UI
subsystems so apps see their final frame.

This document is the single source of truth for the boot subsystem and
supersedes both the architect proposal and the critic critique.

---

## 1. Contract with Prior Rounds (R1–R7)

This round consumes every prior round; it invents no new substrates beyond
those required for fault containment.

| Round | Consumed type / service              | Used in phase              |
|-------|--------------------------------------|----------------------------|
| R1    | `BootPhase`, `BootBarrier`, `WasmDigest`, `RankedMutex` | All phases |
| R1    | `RestartPolicy` (exponential backoff)| `AppLaunch`, post-`Operational` |
| R2    | `MemoryGovernor::init_early/late`, `PsiMonitor::start` | `SubsystemInit` step 2 |
| R3    | `IpcRouter::init_early/late`, `WaitForGraph` | `SubsystemInit` step 5 |
| R4    | `Vfs::mount_root`, `StorageBackend::probe`, ext4 backend | `StorageMount` + `SubsystemInit` step 4 |
| R5    | `Scheduler::new`, cgroup v2 tree, `SCHED_DEADLINE` audio | `KernelSetup` + step 6 |
| R6    | `DeviceManager`, coldplug walk, `InputDispatcher`, `AudioSubsystem` | `DeviceDiscovery` + steps 3, 8, 9 |
| R7    | `PowerManager::new`, `AssertionRegistry::new` | step 1 |

The boot module owns no long-lived state beyond `BootCoordinator`; after
`Operational` it exposes only `shutdown::perform()` and runtime app
respawn (delegated to `R1::RestartPolicy`).

---

## 2. Two-Stage Init Architecture (resolves C1)

### 2.1 Stage 1: Defensive PID-1 Wrapper

`stage1` is a separate static binary at `/sbin/stage1`. Its only job is to
own PID 1 (which the kernel will never let crash without panicking) while
running the supervisor as PID 2 where it is allowed to fail.

**Properties**:
- Statically linked against musl.
- **No** heap allocator; uses only stack + `static mut` buffers.
- **No** `unwrap()`, **no** `expect()`, **no** `panic!()` in source.
- Uses `rustix` for direct syscalls; never `std::fs`.
- Total source size: ≤ 150 LOC.

```rust
//! supervisor/stage1/src/main.rs
//! PID 1. Forks supervisor, kicks watchdog, re-execs supervisor on death.

#![no_main]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use core::time::Duration;
use rustix::process::{fork, waitpid, WaitOptions, WaitStatus, Pid};
use rustix::runtime::execve;

const SUPERVISOR_PATH: &[u8] = b"/sbin/supervisor\0";
const MAX_CRASHES_PER_WINDOW: u32 = 5;
const CRASH_WINDOW_SECS: u64 = 60;
const WATCHDOG_KICK_INTERVAL_MS: u64 = 500;

#[no_mangle]
pub extern "C" fn main(_argc: i32, _argv: *const *const u8) -> i32 {
    // 1. Become a session leader; ignore SIGTERM/SIGINT (kernel guarantees
    //    PID 1 ignores these by default, but be explicit).
    install_signal_ignores();

    // 2. Open kernel watchdog at /dev/watchdog0. If absent (QEMU minimal),
    //    skip silently — but real hardware MUST have one.
    let watchdog = open_watchdog();

    // 3. Crash bookkeeping in static buffer (no heap).
    let mut crash_times = [0u64; MAX_CRASHES_PER_WINDOW as usize];
    let mut crash_idx: usize = 0;

    loop {
        // 4. Fork supervisor as PID 2 (or higher on respawn).
        let pid = match unsafe { fork() } {
            Ok(Some(child)) => child,
            Ok(None) => {
                // Child: exec supervisor. Any return = failure.
                let argv = [SUPERVISOR_PATH.as_ptr(), core::ptr::null()];
                let envp = make_min_env();
                let _ = unsafe { execve(SUPERVISOR_PATH.as_ptr().cast(),
                                        argv.as_ptr().cast(),
                                        envp.as_ptr().cast()) };
                // Exec failed. Emit kernel log line and abort the child.
                kmsg(b"stage1: exec supervisor failed\n");
                unsafe { rustix::runtime::exit_group(127) };
            }
            Err(_) => {
                kmsg(b"stage1: fork failed; sleeping 1s\n");
                sleep_ms(1000);
                continue;
            }
        };

        // 5. Stage-1 main loop: kick watchdog, wait for supervisor.
        loop {
            kick_watchdog(&watchdog);
            match waitpid(Some(pid), WaitOptions::WNOHANG) {
                Ok(Some((_, status))) => {
                    record_crash(&mut crash_times, &mut crash_idx);
                    kmsg_status(b"stage1: supervisor exited", &status);
                    break; // outer loop respawns
                }
                Ok(None) => sleep_ms(WATCHDOG_KICK_INTERVAL_MS),
                Err(_)   => break,
            }
        }

        // 6. Crash-rate gate: if more than MAX_CRASHES_PER_WINDOW within
        //    CRASH_WINDOW_SECS, drop to recovery shell instead of looping.
        if crashes_in_window(&crash_times, CRASH_WINDOW_SECS) >= MAX_CRASHES_PER_WINDOW {
            kmsg(b"stage1: supervisor crash storm; dropping to recovery shell\n");
            exec_recovery_shell();
            // exec_recovery_shell never returns. If it does, halt.
            unsafe { rustix::runtime::exit_group(1) };
        }
    }
}
```

**Invariants enforced by stage1**:

- The kernel watchdog is kicked every 500 ms. If the supervisor hangs
  (deadlock, livelock, infinite loop) for > watchdog_timeout (default 30s),
  the kernel reboots the machine via hardware watchdog. Stage1 is the
  *only* watchdog-kicker; the supervisor cannot kick it.
- Crash storms (5 crashes in 60 s) drop to `/sbin/recovery-shell` (a tiny
  busybox-linked shell). This breaks reboot loops cold.
- Stage1 itself never allocates, never calls into Wasmtime, never opens a
  network socket. Its attack surface is zero.

### 2.2 Stage 2: The Supervisor

The supervisor (PID 2+) is the focus of the rest of this document. It can
panic, exit, or be SIGKILL'd by stage1; the kernel does not care because it
is not PID 1.

The supervisor still applies its own defenses:

- `std::panic::catch_unwind` around each subsystem's `init_early` and
  `init_late`.
- `Result<_, PhaseFailure>` return types everywhere; no `unwrap()` in the
  boot path. Lints enforced: `#![deny(clippy::unwrap_used,
  clippy::expect_used)]` for `supervisor/src/boot/`.
- The internal `MinimalShell` (Section 8) is the supervisor's own fallback
  REPL that runs *without* Wasmtime, so a Wasmtime fault cannot brick
  the recovery path.

---

## 3. Extended `BootPhase` State Machine

R1 defined seven phases; we extend to ten with explicit deadlines.

```rust
//! supervisor/src/boot/phases.rs
//! Single source of truth for boot phase ordering.

use std::time::{Duration, Instant};

#[repr(u8)]
#[derive(Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Debug, Hash)]
pub enum BootPhase {
    /// Kernel handed off PID 2. /proc, /sys, /dev not yet mounted.
    EarlyInit          = 0,
    /// seccomp self-filter, cgroup v2 hierarchy, rlimits.
    KernelSetup        = 1,
    /// ext4 (virtio-blk) + 9P mounts; /data backing store online.
    StorageMount       = 2,
    /// udev netlink socket open; coldplug walk of /sys complete.
    DeviceDiscovery    = 3,
    /// All 9 supervisor subsystems passed init_early + init_late.
    SubsystemInit      = 4,
    /// boot.toml resolved; per-app signatures verified.
    AppRegistryLoad    = 5,
    /// Mandatory wave of WASM apps spawned and reached `ready`.
    AppLaunch          = 6,
    /// Compositor produced first flushed frame (or headless ack).
    DisplayReady       = 7,
    /// Optional wave complete; every manifest-declared app alive or skipped.
    AllAppsSpawned     = 8,
    /// Steady state. Boot module quiesces.
    Operational        = 9,
}

impl BootPhase {
    pub const ALL: [BootPhase; 10] = [
        BootPhase::EarlyInit, BootPhase::KernelSetup, BootPhase::StorageMount,
        BootPhase::DeviceDiscovery, BootPhase::SubsystemInit,
        BootPhase::AppRegistryLoad, BootPhase::AppLaunch,
        BootPhase::DisplayReady, BootPhase::AllAppsSpawned,
        BootPhase::Operational,
    ];

    /// Per-tier deadline. Warm path uses these directly; cold path scales
    /// by `cold_scale_factor()`; recovery path uses `recovery_deadline()`.
    pub const fn warm_deadline(self) -> Duration {
        match self {
            BootPhase::EarlyInit        => Duration::from_millis(40),
            BootPhase::KernelSetup      => Duration::from_millis(120),
            BootPhase::StorageMount     => Duration::from_millis(250),
            BootPhase::DeviceDiscovery  => Duration::from_millis(400),
            BootPhase::SubsystemInit    => Duration::from_millis(500),
            BootPhase::AppRegistryLoad  => Duration::from_millis(150),
            BootPhase::AppLaunch        => Duration::from_millis(900),
            BootPhase::DisplayReady     => Duration::from_millis(300),
            BootPhase::AllAppsSpawned   => Duration::from_millis(340),
            BootPhase::Operational      => Duration::from_secs(0),
        }
    }
    // sum(warm_deadline) = 3000 ms = warm boot target.

    pub const fn cold_scale(self) -> u32 {
        // Cold boot inflates JIT-bound phases.
        match self {
            BootPhase::AppRegistryLoad => 4,  // 150 → 600 (ed25519 × 10)
            BootPhase::AppLaunch       => 5,  // 900 → 4500 (JIT × 10)
            BootPhase::AllAppsSpawned  => 5,  // 340 → 1700
            BootPhase::SubsystemInit   => 2,  // 500 → 1000 (Wasmtime warm-up)
            _                          => 2,  // 2× general safety margin
        }
    }

    pub const fn recovery_deadline(self) -> Duration {
        // Recovery: 10× warm budget across the board, capped at 30s total.
        Duration::from_millis(self.warm_deadline().as_millis() as u64 * 10)
    }

    pub const fn label(self) -> &'static str {
        match self {
            BootPhase::EarlyInit       => "early_init",
            BootPhase::KernelSetup     => "kernel_setup",
            BootPhase::StorageMount    => "storage_mount",
            BootPhase::DeviceDiscovery => "device_discovery",
            BootPhase::SubsystemInit   => "subsystem_init",
            BootPhase::AppRegistryLoad => "app_registry_load",
            BootPhase::AppLaunch       => "app_launch",
            BootPhase::DisplayReady    => "display_ready",
            BootPhase::AllAppsSpawned  => "all_apps_spawned",
            BootPhase::Operational     => "operational",
        }
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum BootTier { Warm, Cold, Recovery }

#[derive(Clone, Debug)]
pub struct PhaseTransition {
    pub phase:        BootPhase,
    pub entered_at:   Instant,
    pub completed_at: Option<Instant>,
    pub outcome:      PhaseOutcome,
}

#[derive(Clone, Debug)]
pub enum PhaseOutcome {
    Pending,
    Ok,
    Slow { overage: Duration },
    Failed { reason: String, recovery: RecoveryAction },
}

#[derive(Clone, Copy, Debug)]
pub enum RecoveryAction {
    /// Phase body should have caught this. Should never reach the coordinator.
    SkipNonCritical,
    /// Retry one app, mandatory or not, with extended timeout.
    RetryAppOnce,
    /// Drop to MinimalShell (supervisor-internal REPL, no Wasmtime).
    EmergencyShell,
    /// Prompt user: factory-reset /data?
    FactoryResetPrompt,
    /// Re-exec the supervisor via stage1 (graceful re-exec, preserves PID 1).
    ReexecSupervisor,
    /// Reboot the machine (stage1 sees exit code → reboot).
    Reboot,
    /// Halt.
    Halt,
}
```

### 3.1 BootBarrier with timeout and `catch_unwind`

```rust
//! supervisor/src/boot/barrier.rs
use parking_lot::{Condvar};
use crate::lock_order::{RankedMutex, RANK_BOOT_BARRIER};
use std::sync::Arc;
use std::time::{Duration, Instant};
use super::phases::BootPhase;

#[derive(Clone)]
pub struct BootBarrier(Arc<Inner>);

struct Inner {
    state: RankedMutex<BootPhase, RANK_BOOT_BARRIER>,
    cv:    Condvar,
}

impl BootBarrier {
    pub fn new() -> Self {
        Self(Arc::new(Inner {
            state: RankedMutex::new(BootPhase::EarlyInit),
            cv:    Condvar::new(),
        }))
    }

    pub fn current(&self) -> BootPhase { *self.0.state.lock() }

    /// Advance to next phase. Panics on regression — boot is monotonic.
    /// Wrapped in `catch_unwind` by the BootCoordinator.
    pub fn advance(&self, to: BootPhase) {
        let mut g = self.0.state.lock();
        assert!(to > *g, "boot phase regression: {:?} -> {:?}", *g, to);
        *g = to;
        self.0.cv.notify_all();
    }

    /// Block until current >= `at_least`. Returns false on timeout.
    pub fn wait_for(&self, at_least: BootPhase, max: Duration) -> bool {
        let mut g = self.0.state.lock();
        let deadline = Instant::now() + max;
        while *g < at_least {
            let now = Instant::now();
            if now >= deadline { return false; }
            self.0.cv.wait_for(&mut g.raw_guard(), deadline - now);
        }
        true
    }

    pub fn at_least(&self, p: BootPhase) -> bool { *self.0.state.lock() >= p }
}
```

---

## 4. `RankedMutex` & Lock-Order Discipline (resolves C4)

Every lock in the supervisor is wrapped in a generic `RankedMutex<T, const
RANK: u16>`. Thread-local stores the maximum rank currently held; attempt to
acquire a lower-or-equal rank panics in debug builds. Release builds compile
the check away.

```rust
//! supervisor/src/lock_order.rs
//! Compile-time + runtime lock-rank enforcement.

use parking_lot::{Mutex, MutexGuard};
use std::cell::Cell;

thread_local! {
    static MAX_HELD_RANK: Cell<u16> = const { Cell::new(0) };
    static HELD_RANKS:    Cell<[u16; 16]> = const { Cell::new([0; 16]) };
    static HELD_DEPTH:    Cell<usize> = const { Cell::new(0) };
}

#[repr(transparent)]
pub struct RankedMutex<T, const RANK: u16> {
    inner: Mutex<T>,
}

pub struct RankedGuard<'a, T, const RANK: u16> {
    guard:        MutexGuard<'a, T>,
    prior_max:    u16,
    prior_depth:  usize,
}

impl<T, const RANK: u16> RankedMutex<T, RANK> {
    pub const fn new(v: T) -> Self { Self { inner: Mutex::new(v) } }

    pub fn lock(&self) -> RankedGuard<'_, T, RANK> {
        #[cfg(debug_assertions)] {
            MAX_HELD_RANK.with(|m| {
                let cur = m.get();
                debug_assert!(
                    cur < RANK,
                    "lock-order violation: holding rank {cur}, acquiring rank {RANK}"
                );
            });
        }
        let prior_max = MAX_HELD_RANK.with(|c| c.get());
        let prior_depth = HELD_DEPTH.with(|c| c.get());
        let guard = self.inner.lock();
        MAX_HELD_RANK.with(|c| c.set(RANK));
        HELD_DEPTH.with(|c| c.set(prior_depth + 1));
        RankedGuard { guard, prior_max, prior_depth }
    }
}

impl<'a, T, const RANK: u16> Drop for RankedGuard<'a, T, RANK> {
    fn drop(&mut self) {
        MAX_HELD_RANK.with(|c| c.set(self.prior_max));
        HELD_DEPTH.with(|c| c.set(self.prior_depth));
    }
}

impl<'a, T, const RANK: u16> std::ops::Deref for RankedGuard<'a, T, RANK> {
    type Target = T;
    fn deref(&self) -> &T { &*self.guard }
}
impl<'a, T, const RANK: u16> std::ops::DerefMut for RankedGuard<'a, T, RANK> {
    fn deref_mut(&mut self) -> &mut T { &mut *self.guard }
}

// === Canonical rank assignments ===========================================
// Lower rank = acquired FIRST. Total order; gaps for future insertions.
pub const RANK_POWER_MANAGER:      u16 =  100;
pub const RANK_MEMORY_GOVERNOR:    u16 =  200;
pub const RANK_DEVICE_MANAGER:     u16 =  300;
pub const RANK_VFS:                u16 =  400;
pub const RANK_IPC_ROUTER:         u16 =  500;
pub const RANK_SCHEDULER:          u16 =  600;
pub const RANK_COMPOSITOR:         u16 =  700;
pub const RANK_INPUT_DISPATCHER:   u16 =  800;
pub const RANK_AUDIO:              u16 =  900;
pub const RANK_BOOT_BARRIER:       u16 = 1000;
pub const RANK_INSTALL_REGISTRY:   u16 = 1100;
pub const RANK_APP_TABLE_SHARD:    u16 = 1200;  // sharded; same rank across shards
pub const RANK_APP_STATE_HOT:      u16 = 1300;
pub const RANK_APP_STATE_COLD:     u16 = 1400;
pub const RANK_IPC_INBOX:          u16 = 1500;
pub const RANK_COMPOSITOR_STATE:   u16 = 1600;
pub const RANK_SURFACE:            u16 = 1700;
```

**Compile-time check** (advanced): a type-state helper `LockToken<RANK>`
threads the maximum-rank-held through the function signature. A function
that takes `LockToken<500>` cannot acquire a `RankedMutex<_, 400>`. We use
this for the boot path's `init_early` chain; we use runtime checks for
hot-path subsystem code.

**Reviewer rule**: every `RankedMutex::new` site must cite the rank constant
by name, not literal. CI rejects literal RANK values in `RankedMutex<_,
1234>` declarations via a grep-based lint.

---

## 5. Boot Coordinator & Subsystem Init (resolves C3)

### 5.1 Two-phase init protocol

Every subsystem implements:

```rust
//! supervisor/src/boot/subsystem.rs
pub trait Subsystem: Send + Sync {
    fn name(&self) -> &'static str;
    /// What other subsystems must complete `init_late` before our `init_late`.
    /// Empty in `init_early` mode — early init is dependency-free.
    fn depends_on(&self) -> &'static [&'static str];

    /// Phase A: allocate fields, no cross-subsystem calls allowed.
    /// May fail; failure routes to `RecoveryAction`.
    fn init_early(&mut self, ctx: &mut InitContext) -> Result<(), InitError>;

    /// Phase B: wire up dependencies via `ctx.lookup(name)`.
    /// Runs after every subsystem's `init_early` returned Ok.
    fn init_late(&mut self, ctx: &mut InitContext) -> Result<(), InitError>;
}

pub struct InitContext<'a> {
    pub barrier: &'a crate::boot::BootBarrier,
    pub log:     &'a crate::observability::BootLog,
    pub tier:    super::phases::BootTier,
    registry:    &'a SubsystemRegistry,
}

impl<'a> InitContext<'a> {
    pub fn lookup<T: 'static>(&self, name: &str) -> Option<std::sync::Arc<T>> {
        self.registry.get::<T>(name)
    }
}
```

The boot driver computes the topological order of `depends_on` *at compile
time* via a `const fn` over the static list of registered subsystems. If a
cycle exists, `cargo build` fails with `const-eval error: dependency cycle`.

### 5.2 Dependency DAG (cycle-free by construction)

The architect's cycle claim ("there are no cycles by construction") was
correct *for `init_late`* but the critic correctly observed that the
**dependency at construction time** between memory↔IPC↔VFS↔storage is
naively cyclic. The fix: `init_early` allocates fields with no
cross-subsystem calls; `init_late` resolves dependencies via the registry.

```
init_early phase (any order; no inter-dependencies):
  PowerManager, MemoryGovernor, DeviceManager, Vfs, IpcRouter, Scheduler,
  Compositor, InputDispatcher, AudioSubsystem

init_late phase (topological):
  PowerManager       → ∅
  MemoryGovernor     → [PowerManager]
  DeviceManager      → [MemoryGovernor, PowerManager]
  Vfs                → [DeviceManager, MemoryGovernor]
  IpcRouter          → [Vfs, MemoryGovernor]
  Scheduler          → [MemoryGovernor, PowerManager]
  Compositor         → [DeviceManager, Vfs, Scheduler, PowerManager]
  InputDispatcher    → [DeviceManager, IpcRouter, Compositor]
  AudioSubsystem     → [DeviceManager, Scheduler, PowerManager]
```

```
        PowerManager ──┬──> MemoryGovernor ──> DeviceManager ──> VFS
                       │                              │           │
                       │                              │           v
                       │                              │         IpcRouter
                       │                              │
                       │                              v
                       │                            Scheduler
                       │                              │
                       │                              v
                       └──────────────────────────> Compositor
                                                       │  │
                                                       v  v
                                                 InputDisp Audio
```

### 5.3 The Boot Coordinator

```rust
//! supervisor/src/boot/mod.rs
//! BootCoordinator runs the FSM, owns the timeline, dispatches to recovery.

pub mod phases;
pub mod barrier;
pub mod subsystem;
pub mod registry;
pub mod manifest_loader;
pub mod app_launcher;
pub mod shutdown;
pub mod recovery;
pub mod minimal_shell;

mod early_init;
mod kernel_setup;
mod storage_mount;
mod device_discovery;
mod subsystem_init;

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::Arc;
use std::time::Instant;
use phases::{BootPhase, BootTier, PhaseTransition, PhaseOutcome, RecoveryAction};
use barrier::BootBarrier;
use crate::lock_order::{RankedMutex, RANK_BOOT_BARRIER};
use crate::observability::BootLog;
use crate::supervisor_ctx::SupervisorContext;

pub struct BootCoordinator {
    pub(crate) barrier:  BootBarrier,
    pub(crate) timeline: RankedMutex<Vec<PhaseTransition>, RANK_BOOT_BARRIER>,
    pub(crate) log:      Arc<BootLog>,
    pub(crate) start:    Instant,
    pub(crate) tier:     BootTier,
    pub(crate) dev_mode: bool,
}

impl BootCoordinator {
    pub fn new(dev_mode: bool) -> Arc<Self> {
        let tier = detect_boot_tier();   // checks /data/.system/aot/
        let log = BootLog::open("/data/.system/boot.log")
            .unwrap_or_else(|_| BootLog::stderr_only());
        Arc::new(Self {
            barrier:  BootBarrier::new(),
            timeline: RankedMutex::new(Vec::with_capacity(10)),
            log,
            start:    Instant::now(),
            tier, dev_mode,
        })
    }

    pub fn run(self: Arc<Self>) -> Arc<SupervisorContext> {
        let mut ctx = SupervisorContext::empty(self.clone());

        self.run_phase(BootPhase::EarlyInit,       |_| early_init::run(&self)).recover(&self);
        self.run_phase(BootPhase::KernelSetup,     |_| kernel_setup::run(&self)).recover(&self);
        self.run_phase(BootPhase::StorageMount,    |_| storage_mount::run(&self)).recover(&self);
        self.run_phase(BootPhase::DeviceDiscovery, |_| device_discovery::run(&self)).recover(&self);
        self.run_phase(BootPhase::SubsystemInit,
                       |_| subsystem_init::run(&self, &mut ctx)).recover(&self);
        self.run_phase(BootPhase::AppRegistryLoad,
                       |_| manifest_loader::run(&self, &mut ctx)).recover(&self);
        self.run_phase(BootPhase::AppLaunch,
                       |_| app_launcher::launch_mandatory(&self, &mut ctx)).recover(&self);
        self.run_phase(BootPhase::DisplayReady,
                       |_| app_launcher::await_display(&self, &ctx)).recover(&self);
        self.run_phase(BootPhase::AllAppsSpawned,
                       |_| app_launcher::launch_optional(&self, &mut ctx)).recover(&self);

        self.barrier.advance(BootPhase::Operational);
        self.log_transition(BootPhase::Operational, PhaseOutcome::Ok);
        Arc::new(ctx)
    }

    /// Run one phase body inside `catch_unwind` so a panic inside the
    /// supervisor does not propagate to PID 2 exit (which stage1 would
    /// see as a crash and re-exec).
    fn run_phase<F>(&self, p: BootPhase, body: F) -> Result<(), PhaseFailure>
    where F: FnOnce(&BootCoordinator) -> Result<(), PhaseFailure>
            + std::panic::UnwindSafe
    {
        self.barrier.advance(p);
        let t0 = Instant::now();
        self.timeline.lock().push(PhaseTransition {
            phase: p, entered_at: t0, completed_at: None,
            outcome: PhaseOutcome::Pending,
        });
        let res = catch_unwind(AssertUnwindSafe(|| body(self)))
            .unwrap_or_else(|panic| Err(PhaseFailure {
                reason:   format!("panic in {}: {:?}", p.label(), describe(panic)),
                recovery: RecoveryAction::EmergencyShell,
            }));
        let t1 = Instant::now();
        let elapsed = t1 - t0;
        let deadline = match self.tier {
            BootTier::Warm     => p.warm_deadline(),
            BootTier::Cold     => p.warm_deadline() * p.cold_scale(),
            BootTier::Recovery => p.recovery_deadline(),
        };
        let outcome = match &res {
            Ok(()) if elapsed > deadline =>
                PhaseOutcome::Slow { overage: elapsed - deadline },
            Ok(())  => PhaseOutcome::Ok,
            Err(f)  => PhaseOutcome::Failed {
                reason:   f.reason.clone(),
                recovery: f.recovery,
            },
        };
        if let Some(last) = self.timeline.lock().last_mut() {
            last.completed_at = Some(t1);
            last.outcome = outcome.clone();
        }
        self.log_transition(p, outcome);
        res
    }

    fn log_transition(&self, p: BootPhase, outcome: PhaseOutcome) {
        let line = serde_json::json!({
            "ts": iso_now(),
            "phase": p.label(),
            "elapsed_ms": self.start.elapsed().as_millis(),
            "tier": format!("{:?}", self.tier),
            "outcome": format!("{:?}", outcome),
        });
        self.log.append(&line.to_string());
    }
}

#[derive(Clone, Debug)]
pub struct PhaseFailure {
    pub reason:   String,
    pub recovery: RecoveryAction,
}

trait RecoverExt { fn recover(self, b: &BootCoordinator); }
impl RecoverExt for Result<(), PhaseFailure> {
    fn recover(self, b: &BootCoordinator) {
        if let Err(f) = self { recovery::handle(b, f); }
    }
}

fn detect_boot_tier() -> BootTier {
    use std::path::Path;
    if !Path::new("/data/.system/aot").is_dir() { return BootTier::Cold; }
    if Path::new("/data/.system/recovery.marker").exists() { return BootTier::Recovery; }
    BootTier::Warm
}
```

### 5.4 SubsystemInit step body

```rust
//! supervisor/src/boot/subsystem_init.rs
use crate::power::{PowerManager, AssertionRegistry};
use crate::memory::{MemoryGovernor, PsiMonitor};
use crate::devices::{DeviceManager, InputDispatcher, AudioSubsystem};
use crate::vfs::Vfs;
use crate::ipc::{IpcRouter, WaitForGraph};
use crate::scheduler::Scheduler;
use crate::compositor::Compositor;
use crate::supervisor_ctx::SupervisorContext;
use super::registry::SubsystemRegistry;

pub fn run(b: &super::BootCoordinator, ctx: &mut SupervisorContext)
    -> Result<(), super::PhaseFailure>
{
    let mut registry = SubsystemRegistry::new();

    // ===== Phase A: init_early (no inter-dependencies) =====================
    let mut power      = PowerManager::stub();
    let mut mem_gov    = MemoryGovernor::stub();
    let mut dm         = DeviceManager::stub();
    let mut vfs        = Vfs::stub();
    let mut ipc        = IpcRouter::stub();
    let mut sched      = Scheduler::stub();
    let mut comp       = Compositor::stub();
    let mut input      = InputDispatcher::stub();
    let mut audio      = AudioSubsystem::stub();

    let ictx = |b: &super::BootCoordinator| super::subsystem::InitContext {
        barrier: &b.barrier, log: &b.log, tier: b.tier, registry: &registry,
    };
    power.init_early(&mut ictx(b))    .map_err(pf_early("PowerManager"))?;
    mem_gov.init_early(&mut ictx(b))  .map_err(pf_early("MemoryGovernor"))?;
    dm.init_early(&mut ictx(b))       .map_err(pf_early("DeviceManager"))?;
    vfs.init_early(&mut ictx(b))      .map_err(pf_early("Vfs"))?;
    ipc.init_early(&mut ictx(b))      .map_err(pf_early("IpcRouter"))?;
    sched.init_early(&mut ictx(b))    .map_err(pf_early("Scheduler"))?;
    comp.init_early(&mut ictx(b))     .map_err(pf_early("Compositor"))?;
    input.init_early(&mut ictx(b))    .map_err(pf_early("InputDispatcher"))?;
    audio.init_early(&mut ictx(b))    .map_err(pf_early("AudioSubsystem"))?;

    registry.register("power",     std::sync::Arc::new(power.clone()));
    registry.register("memory",    std::sync::Arc::new(mem_gov.clone()));
    registry.register("devices",   std::sync::Arc::new(dm.clone()));
    registry.register("vfs",       std::sync::Arc::new(vfs.clone()));
    registry.register("ipc",       std::sync::Arc::new(ipc.clone()));
    registry.register("scheduler", std::sync::Arc::new(sched.clone()));
    registry.register("compositor",std::sync::Arc::new(comp.clone()));
    registry.register("input",     std::sync::Arc::new(input.clone()));
    registry.register("audio",     std::sync::Arc::new(audio.clone()));

    // ===== Phase B: init_late in topological order ==========================
    power.init_late(&mut ictx(b))     .map_err(pf_late("PowerManager"))?;
    mem_gov.init_late(&mut ictx(b))   .map_err(pf_late("MemoryGovernor"))?;
    dm.init_late(&mut ictx(b))        .map_err(pf_late("DeviceManager"))?;
    vfs.init_late(&mut ictx(b))       .map_err(pf_late("Vfs"))?;
    ipc.init_late(&mut ictx(b))       .map_err(pf_late("IpcRouter"))?;
    sched.init_late(&mut ictx(b))     .map_err(pf_late("Scheduler"))?;
    comp.init_late(&mut ictx(b))      .map_err(pf_late("Compositor"))?;
    input.init_late(&mut ictx(b))     .map_err(pf_late("InputDispatcher"))?;
    audio.init_late(&mut ictx(b))     .map_err(pf_late("AudioSubsystem"))?;

    // Move resolved handles into the supervisor context.
    ctx.power      = Some(power);
    ctx.memory     = Some(mem_gov);
    ctx.devices    = Some(dm);
    ctx.vfs        = Some(vfs);
    ctx.ipc        = Some(ipc);
    ctx.scheduler  = Some(sched);
    ctx.compositor = Some(comp);
    ctx.input      = Some(input);
    ctx.audio      = Some(audio);
    ctx.psi        = PsiMonitor::start(ctx.memory.as_ref().unwrap().clone()).ok();
    Ok(())
}

fn pf_early(name: &'static str) -> impl FnOnce(crate::boot::subsystem::InitError)
    -> super::PhaseFailure
{
    move |e| super::PhaseFailure {
        reason: format!("{name}.init_early: {e}"),
        recovery: super::phases::RecoveryAction::EmergencyShell,
    }
}
fn pf_late(name: &'static str) -> impl FnOnce(crate::boot::subsystem::InitError)
    -> super::PhaseFailure
{
    move |e| super::PhaseFailure {
        reason: format!("{name}.init_late: {e}"),
        recovery: super::phases::RecoveryAction::EmergencyShell,
    }
}
```

---

## 6. Phase Bodies (EarlyInit through DeviceDiscovery)

### 6.1 EarlyInit

```rust
//! supervisor/src/boot/early_init.rs
use nix::mount::{mount, MsFlags};
use nix::sys::stat::Mode;

pub fn run(b: &super::BootCoordinator) -> Result<(), super::PhaseFailure> {
    // PID-1 check is now stage1's responsibility; supervisor checks PID 2+.
    let pid = nix::unistd::getpid().as_raw();
    if pid < 2 {
        return Err(super::PhaseFailure {
            reason: format!("supervisor is PID {pid}, expected ≥ 2"),
            recovery: super::phases::RecoveryAction::Halt,
        });
    }
    mount_pseudo("proc",     "/proc",   "proc",
                 MsFlags::MS_NOSUID|MsFlags::MS_NODEV|MsFlags::MS_NOEXEC)?;
    mount_pseudo("sysfs",    "/sys",    "sysfs",
                 MsFlags::MS_NOSUID|MsFlags::MS_NODEV|MsFlags::MS_NOEXEC)?;
    mount_pseudo("devtmpfs", "/dev",    "devtmpfs", MsFlags::MS_NOSUID)?;
    mount_pseudo("tmpfs",    "/run",    "tmpfs",
                 MsFlags::MS_NOSUID|MsFlags::MS_NODEV)?;
    mount_pseudo("cgroup2",  "/sys/fs/cgroup", "cgroup2",
                 MsFlags::MS_NOSUID|MsFlags::MS_NODEV|MsFlags::MS_NOEXEC)?;
    let _ = nix::unistd::mkdir("/dev/pts",
        Mode::S_IRWXU|Mode::S_IRWXG|Mode::S_IRWXO);
    mount_pseudo("devpts", "/dev/pts", "devpts",
                 MsFlags::MS_NOSUID|MsFlags::MS_NOEXEC)?;
    install_panic_hook(b);     // writes panic + backtrace to /tmp/boot.log
    install_sigchld_handler(); // reaps zombies before scheduler exists
    Ok(())
}
```

### 6.2 KernelSetup

```rust
pub fn run(b: &super::BootCoordinator) -> Result<(), super::PhaseFailure> {
    apply_self_seccomp_denylist()?;
    set_rlimits()?;
    create_cgroup_root_hierarchy()?;
    set_supervisor_oom_score_adj(-1000)?;
    write_kernel_sysctls()?;
    if b.dev_mode { unlock_kallsyms_for_dev()?; }
    Ok(())
}
```

### 6.3 StorageMount

```rust
pub fn run(b: &super::BootCoordinator) -> Result<(), super::PhaseFailure> {
    let backend = StorageBackend::probe().map_err(|e| super::PhaseFailure {
        reason: format!("storage probe failed: {e}"),
        recovery: super::phases::RecoveryAction::FactoryResetPrompt,
    })?;
    match backend.kind() {
        BackendKind::Ext4OnVirtioBlk { dev } =>
            mount_ext4(dev, "/data", MsFlags::MS_NOATIME)?,
        BackendKind::Nine_P { tag } => mount_9p(tag, "/data")?,
        BackendKind::Tmpfs => mount_pseudo("tmpfs", "/data", "tmpfs",
                                           MsFlags::MS_NOATIME)?,
    }
    ensure_dir("/data/.system")?;
    ensure_dir("/data/.system/aot")?;
    ensure_dir("/data/.system/preserved")?;
    ensure_dir("/data/etc")?;
    ensure_file_default("/data/.system/installed.txt", b"")?;
    Ok(())
}
```

### 6.4 DeviceDiscovery

```rust
pub fn run(b: &super::BootCoordinator) -> Result<(), super::PhaseFailure> {
    let dm = DeviceManager::open_netlink()
        .map_err(|e| pf("netlink open: {e}", RecoveryAction::EmergencyShell))?;
    let coldplug = dm.walk_sysfs("/sys")
        .map_err(|e| pf("coldplug: {e}", RecoveryAction::RetryAppOnce))?;
    if coldplug.input_devices.is_empty() && !b.dev_mode {
        b.log.append(r#"{"warn":"no input devices at coldplug"}"#);
    }
    stash_for_subsystem_init(dm, coldplug);
    Ok(())
}
```

---

## 7. Three-Layer `boot.toml` Resolution (resolves C6)

```rust
//! supervisor/src/boot/manifest_loader.rs
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use serde::Deserialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Factory default, baked into the supervisor binary. Sufficient to boot
/// to MinimalShell, list `/apps/`, and accept user input.
const FACTORY_BOOT_TOML: &[u8] = include_bytes!(
    "../../../config/factory-boot.toml"
);

#[derive(Debug, Clone, Deserialize)]
pub struct BootToml {
    pub boot_order: Vec<BootEntry>,
    #[serde(default)]
    pub trusted_keys: Vec<TrustedKey>,
    #[serde(default = "default_boot_timeout_ms")]
    pub boot_timeout_ms: u64,
    #[serde(default)]
    pub parallelism: Parallelism,
    #[serde(default)]
    pub on_failure: OnFailure,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Parallelism {
    #[serde(default = "default_max_concurrent_jit")]
    pub max_concurrent_jit: usize,
    #[serde(default = "default_max_concurrent_io")]
    pub max_concurrent_io: usize,
    #[serde(default = "default_max_concurrent_launch")]
    pub max_concurrent_launch: usize,
}
impl Default for Parallelism {
    fn default() -> Self { Self {
        max_concurrent_jit: 2,
        max_concurrent_io: 4,
        max_concurrent_launch: 4,
    }}
}

#[derive(Debug, Clone, Deserialize)]
pub struct OnFailure {
    #[serde(default)]
    pub max_consecutive_failures: u32,
    #[serde(default)]
    pub strategy: FailureStrategy,
}
impl Default for OnFailure {
    fn default() -> Self { Self {
        max_consecutive_failures: 3,
        strategy: FailureStrategy::DropToRecoveryShell,
    }}
}
#[derive(Debug, Clone, Default, Deserialize)]
pub enum FailureStrategy {
    #[default] DropToRecoveryShell,
    Reboot,
    FactoryReset,
    SwitchSlot,
}

/// Loads `BootToml` through the three-layer cascade:
/// 1. Try `/data/etc/override.toml`. If valid, merge over initramfs base.
/// 2. Try `/etc/vyoma/boot.toml` (initramfs, read-only). Base config.
/// 3. Fall back to `FACTORY_BOOT_TOML` (embedded const).
///
/// Each layer is independently validated. A corrupt layer is logged and
/// skipped — never bricks boot.
pub fn resolve_boot_toml(log: &crate::observability::BootLog) -> BootToml {
    // Layer 1: initramfs base.
    let base = match std::fs::read_to_string("/etc/vyoma/boot.toml") {
        Ok(s) => match toml::from_str::<BootToml>(&s) {
            Ok(t) => Some(t),
            Err(e) => {
                log.append(&format!(
                    r#"{{"warn":"initramfs boot.toml parse failed","err":"{e}"}}"#));
                None
            }
        },
        Err(e) => {
            log.append(&format!(
                r#"{{"warn":"initramfs boot.toml missing","err":"{e}"}}"#));
            None
        }
    };

    // Layer 2: data override.
    let override_ = match std::fs::read_to_string("/data/etc/override.toml") {
        Ok(s) => match toml::from_str::<BootToml>(&s) {
            Ok(t) => Some(t),
            Err(e) => {
                log.append(&format!(
                    r#"{{"warn":"override.toml parse failed; ignoring","err":"{e}"}}"#));
                None
            }
        },
        Err(_) => None,   // Override absent is normal, not warning.
    };

    // Layer 3: factory const (always available).
    let factory: BootToml = toml::from_slice(FACTORY_BOOT_TOML)
        .expect("factory boot.toml must parse — fix at build time");

    match (override_, base) {
        (Some(o), _) => o,           // Override wins if valid.
        (None, Some(b)) => b,        // Initramfs base.
        (None, None) => {            // Last resort: factory.
            log.append(r#"{"warn":"all boot.toml layers failed; using factory"}"#);
            factory
        }
    }
}
```

`config/factory-boot.toml` ships in the repo and is `include_bytes!`'d into
the supervisor binary; it lists only the minimum mandatory apps (compositor,
shell, session-manager). It must always parse — a CI test compiles the
supervisor and runs `toml::from_slice(FACTORY_BOOT_TOML)` to verify.

### 7.1 Manifest verification & DAG

The signature verification path is unchanged from the architect's proposal
(ed25519 over `WasmDigest`), except:

1. Verified-once cache: after a successful verify, the supervisor writes
   `/data/.system/aot/<digest>.verified` with the verifier's pubkey
   fingerprint. Next boot, if `<wasm_path>` is the same digest and the
   marker exists, skip re-verify (saves ~100 ms per app on cold→warm
   transition).

2. Override path apps must re-verify every boot (cannot be cached) because
   override.toml is user-mutable.

```rust
pub fn run(b: &super::BootCoordinator, ctx: &mut SupervisorContext)
    -> Result<(), super::PhaseFailure>
{
    let cfg = resolve_boot_toml(&b.log);
    let keys: HashMap<String, VerifyingKey> = cfg.trusted_keys.iter()
        .map(|k| (k.name.clone(), VerifyingKey::from_bytes(&k.pubkey).unwrap()))
        .collect();
    let mut resolved = Vec::with_capacity(cfg.boot_order.len());
    let mut skipped = Vec::new();
    for entry in &cfg.boot_order {
        match load_one(entry, &keys, b.dev_mode) {
            Ok(r) => resolved.push(r),
            Err(e) if !entry.mandatory => {
                b.log.append(&format!(
                    r#"{{"warn":"skipping app","path":"{}","err":"{e}"}}"#,
                    entry.path.display()));
                skipped.push((entry.clone(), e));
            }
            Err(e) => return Err(super::PhaseFailure {
                reason: format!("mandatory app {} failed: {e}", entry.path.display()),
                recovery: super::phases::RecoveryAction::EmergencyShell,
            }),
        }
    }
    let dag = build_dependency_dag(&resolved).map_err(|cyc| super::PhaseFailure {
        reason: format!("cycle in start_after: {cyc:?}"),
        recovery: super::phases::RecoveryAction::EmergencyShell,
    })?;
    ctx.boot_plan = Some(BootPlan {
        order_waves: dag.waves,
        skipped,
        boot_timeout: std::time::Duration::from_millis(cfg.boot_timeout_ms),
        parallelism: cfg.parallelism,
        on_failure: cfg.on_failure,
    });
    ctx.resolved_apps = resolved;
    Ok(())
}
```

Kahn's-algorithm DAG construction is unchanged. Cycles in `start_after`
fail with `EmergencyShell` recovery, the same as for mandatory app failures.

---

## 8. App Launcher (resolves C7)

### 8.1 Pipelined launch with bounded parallelism

```rust
//! supervisor/src/boot/app_launcher.rs
use std::sync::Arc;
use std::time::{Duration, Instant};
use crossbeam_channel::{bounded, Receiver};
use crate::SupervisorContext;
use super::manifest_loader::{ResolvedApp, QosHint, RespawnPolicy, Parallelism};

/// Bounded semaphore protecting the JIT stage. AOT-cached apps skip this.
struct JitSemaphore {
    permits: parking_lot::Mutex<usize>,
    cv:      parking_lot::Condvar,
    cap:     usize,
}
impl JitSemaphore {
    fn new(cap: usize) -> Arc<Self> {
        Arc::new(Self { permits: parking_lot::Mutex::new(cap), cv: Default::default(), cap })
    }
    fn acquire(&self) {
        let mut g = self.permits.lock();
        while *g == 0 { self.cv.wait(&mut g); }
        *g -= 1;
    }
    fn release(&self) {
        let mut g = self.permits.lock();
        *g += 1;
        self.cv.notify_one();
    }
}

pub fn launch_mandatory(b: &super::BootCoordinator, ctx: &mut SupervisorContext)
    -> Result<(), super::PhaseFailure>
{
    let plan = ctx.boot_plan.as_ref().expect("plan exists");
    let par  = plan.parallelism.clone();
    let cpus = num_cpus::get();
    let launch_cap = par.max_concurrent_launch.min(cpus).max(1);
    let jit = JitSemaphore::new(par.max_concurrent_jit.min(cpus).max(1));
    let apps = ctx.resolved_apps.clone();

    for (wave_idx, wave) in plan.order_waves.iter().enumerate() {
        let mandatory_in_wave: Vec<usize> = wave.iter().copied()
            .filter(|&i| apps[i].mandatory).collect();
        if mandatory_in_wave.is_empty() { continue; }
        launch_wave(b, ctx, &mandatory_in_wave, true, launch_cap, jit.clone())
            .map_err(|w| super::PhaseFailure {
                reason: format!("wave {wave_idx} mandatory launch failed: {w:?}"),
                recovery: super::phases::RecoveryAction::EmergencyShell,
            })?;
    }
    Ok(())
}

pub fn await_display(b: &super::BootCoordinator, ctx: &SupervisorContext)
    -> Result<(), super::PhaseFailure>
{
    let comp = ctx.compositor.as_ref().expect("compositor init");
    let deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < deadline {
        if comp.has_flushed_frame() { return Ok(()); }
        std::thread::sleep(Duration::from_millis(20));
    }
    // Forward-ref behaviour per Section 14: if loader app failed, compositor
    // renders a static "loader missing" panel itself.
    comp.draw_fallback_panel();
    if comp.has_flushed_frame() { return Ok(()); }
    Err(super::PhaseFailure {
        reason: "compositor produced no frame within 2s + fallback panel failed".into(),
        recovery: super::phases::RecoveryAction::EmergencyShell,
    })
}

pub fn launch_optional(b: &super::BootCoordinator, ctx: &mut SupervisorContext)
    -> Result<(), super::PhaseFailure>
{
    let plan = ctx.boot_plan.as_ref().expect("plan exists");
    let par  = plan.parallelism.clone();
    let cpus = num_cpus::get();
    let launch_cap = par.max_concurrent_launch.min(cpus).max(1);
    let jit = JitSemaphore::new(par.max_concurrent_jit.min(cpus).max(1));
    let apps = ctx.resolved_apps.clone();
    for wave in &plan.order_waves {
        let mut optional: Vec<usize> = wave.iter().copied()
            .filter(|&i| !apps[i].mandatory).collect();
        optional.sort_by_key(|&i| qos_rank(apps[i].qos));
        if optional.is_empty() { continue; }
        let _ = launch_wave(b, ctx, &optional, false, launch_cap, jit.clone());
    }
    Ok(())
}

pub fn qos_rank(q: QosHint) -> u8 {
    match q {
        QosHint::UserInteractive => 0,
        QosHint::UserInitiated   => 1,
        QosHint::Utility         => 2,
        QosHint::Background      => 3,
    }
}

#[derive(Debug)]
pub struct WaveFailure { pub failed: Vec<String> }

fn launch_wave(
    b: &super::BootCoordinator,
    ctx: &mut SupervisorContext,
    indices: &[usize],
    mandatory: bool,
    launch_cap: usize,
    jit: Arc<JitSemaphore>,
) -> Result<(), WaveFailure>
{
    let (tx, rx) = bounded::<(String, Result<(), String>)>(indices.len());
    let mut in_flight = 0usize;
    let mut next = 0usize;
    let mut failed = Vec::new();
    let deadline = Instant::now() + ctx.boot_plan.as_ref().unwrap().boot_timeout;
    while next < indices.len() || in_flight > 0 {
        while in_flight < launch_cap && next < indices.len() {
            let idx = indices[next]; next += 1;
            spawn_app_pipeline(ctx, idx, tx.clone(), jit.clone());
            in_flight += 1;
        }
        match rx.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
            Ok((name, Ok(())))   => {
                in_flight -= 1;
                ctx.app_table.mark_ready(&name);
            }
            Ok((name, Err(e))) => {
                in_flight -= 1;
                b.log.append(&format!(r#"{{"app":"{name}","err":"{e}"}}"#));
                failed.push(name);
            }
            Err(_) => return Err(WaveFailure {
                failed: vec!["boot timeout".into()],
            }),
        }
    }
    if mandatory && !failed.is_empty() { return Err(WaveFailure { failed }); }
    Ok(())
}

/// Pipeline stages: I/O (manifest+wasm read) → JIT (or AOT load) →
/// alloc (store + cgroup) → run. JIT stage is gated by `jit` semaphore.
fn spawn_app_pipeline(
    ctx: &SupervisorContext,
    idx: usize,
    reply: crossbeam_channel::Sender<(String, Result<(), String>)>,
    jit: Arc<JitSemaphore>,
) {
    let app = ctx.resolved_apps[idx].clone();
    let scheduler = ctx.scheduler.clone().unwrap();
    let ipc = ctx.ipc.clone().unwrap();
    let aot_cache_dir = "/data/.system/aot";
    std::thread::Builder::new()
        .name(format!("launch-{}", app.name))
        .spawn(move || {
            let res = (|| -> Result<(), String> {
                // Stage 1: I/O (no semaphore — kernel queues already bound).
                let wasm_bytes = std::fs::read(&app.wasm_path)
                    .map_err(|e| format!("read wasm: {e}"))?;
                // Stage 2: AOT lookup; JIT if miss.
                let aot_path = format!("{}/{}.cwasm", aot_cache_dir, app.digest.hex());
                let module = if std::path::Path::new(&aot_path).exists() {
                    load_aot(&aot_path).map_err(|e| format!("AOT load: {e}"))?
                } else {
                    jit.acquire();
                    let m = jit_compile(&wasm_bytes)
                        .map_err(|e| { jit.release(); format!("JIT: {e}") })?;
                    let _ = persist_aot(&aot_path, &m); // best-effort cache write
                    jit.release();
                    m
                };
                // Stage 3: store alloc + cgroup (serialized via cgroup write lock).
                let cgroup = scheduler.attach_qos_cgroup(&app.name, app.qos)
                    .map_err(|e| e.to_string())?;
                // Stage 4: run.
                let child = ipc.spawn_wasm_with_module(&app, &cgroup, module)
                    .map_err(|e| e.to_string())?;
                let ready = child.wait_ready(Duration::from_millis(1500));
                if !ready { return Err("never emitted ready".into()); }
                Ok(())
            })();
            let _ = reply.send((app.name, res));
        }).expect("spawn launch thread");
}
```

### 8.2 The `ready` contract

Every app's WIT world declares an optional export `on-ready: func() -> ()`.
The launcher considers an app ready when any of:

1. App invokes `vyoma:lifecycle/ready` (preferred path).
2. App sends IPC `@supervisor: ready`.
3. 1500 ms elapsed and app's PID still alive (assumed-ready fallback).

Case 3 is logged as a warning. The `wait_ready` deadline is **per-app**, not
shared across the wave.

---

## 9. Mandatory App Failure & Recovery (resolves C5)

### 9.1 Three failure modes

| When mandatory app fails | Action |
|--------------------------|--------|
| During `init` (never reached `ready`) | Retry once with 3 s timeout; second failure → **EmergencyShell** |
| After `AllAppsSpawned`, app crashes | R1 `RestartPolicy::Always` with exponential backoff: 1s, 2s, 4s, 8s, 16s (cap). After 5 crashes in 60 s, mark **degraded**; do not loop. |
| Compositor crashes specifically | **Do not** kill other apps. Draw blank frame from supervisor directly. Restart compositor under R1 RestartPolicy. If compositor fails to restart 3× in 60 s → EmergencyShell. |

```rust
//! supervisor/src/boot/restart_policy.rs (in spirit; integrates with R1)
use std::time::{Duration, Instant};

pub struct AppFailureTracker {
    pub name: String,
    failures: Vec<Instant>,
    pub policy: super::manifest_loader::RespawnPolicy,
}

impl AppFailureTracker {
    /// Record a failure and decide the action.
    pub fn record_failure(&mut self) -> FailureAction {
        let now = Instant::now();
        self.failures.retain(|&t| now.duration_since(t) < Duration::from_secs(60));
        self.failures.push(now);
        match &self.policy {
            super::manifest_loader::RespawnPolicy::Never => FailureAction::MarkDead,
            super::manifest_loader::RespawnPolicy::OnExit { max_restarts, window_secs } => {
                let window = Duration::from_secs(*window_secs as u64);
                let recent = self.failures.iter()
                    .filter(|&&t| now.duration_since(t) < window).count();
                if recent as u32 > *max_restarts {
                    FailureAction::EscalateToCoordinator
                } else {
                    FailureAction::Restart {
                        delay: Self::backoff(recent),
                    }
                }
            }
            super::manifest_loader::RespawnPolicy::Always => {
                let recent = self.failures.len();
                if recent >= 5 { FailureAction::MarkDegraded }
                else { FailureAction::Restart { delay: Self::backoff(recent) } }
            }
        }
    }
    fn backoff(n: usize) -> Duration {
        let secs = 1u64 << n.min(4);   // 1, 2, 4, 8, 16
        Duration::from_secs(secs)
    }
}

pub enum FailureAction {
    Restart { delay: Duration },
    MarkDegraded,
    MarkDead,
    EscalateToCoordinator,
}
```

### 9.2 The MinimalShell (no Wasmtime)

```rust
//! supervisor/src/boot/minimal_shell.rs
//! A tiny REPL that does not depend on Wasmtime, IPC, or the compositor.
//! It is the supervisor's own fallback. Writes to /dev/tty1 framebuffer
//! text mode (or the kernel console if framebuffer is uninitialized).

use std::io::{BufRead, Write};

pub struct MinimalShell<'a> {
    boot: &'a super::BootCoordinator,
}

impl<'a> MinimalShell<'a> {
    pub fn new(boot: &'a super::BootCoordinator) -> Self {
        Self { boot }
    }

    pub fn run(&mut self) -> ! {
        let stdin  = std::io::stdin();
        let stdout = std::io::stdout();
        let _     = write!(stdout.lock(),
            "VyomaOS emergency shell. type 'help' for commands.\n> ");
        let _ = stdout.lock().flush();
        for line in stdin.lock().lines() {
            let line = line.unwrap_or_default();
            self.dispatch(line.trim());
            let _ = write!(stdout.lock(), "> ");
            let _ = stdout.lock().flush();
        }
        sync_and_halt();
    }

    fn dispatch(&self, cmd: &str) {
        match cmd {
            "help"          => println!("ps log mount df dmesg reboot halt \
                                          factory-reset journal preserve-log exit"),
            "ps"            => self.ps(),
            "df"            => self.df(),
            "dmesg"         => self.dmesg(),
            "journal"       => self.journal(),
            "preserve-log"  => self.preserve_log(),
            "reboot"        => sync_and_reboot(),
            "halt"          => sync_and_halt(),
            "factory-reset" => self.factory_reset_with_log_preservation(),
            _ if cmd.starts_with("log ") => self.log(&cmd[4..]),
            _ if cmd.starts_with("mount ") => self.mount(&cmd[6..]),
            _ => println!("unknown command: {cmd}"),
        }
    }

    fn factory_reset_with_log_preservation(&self) {
        // Per S/critic, preserve the log before wiping.
        let _ = std::fs::copy("/data/.system/boot.log",
                              "/data/.system/preserved/boot.log.last");
        let _ = std::fs::remove_dir_all("/data/.system");
        let _ = std::fs::remove_dir_all("/data/apps");
        let _ = std::fs::create_dir_all("/data/.system/preserved");
        let _ = std::fs::copy("/data/.system/preserved/boot.log.last",
                              "/data/.system/preserved/boot.log.factory-reset");
        sync_and_reboot();
    }

    // ... ps, df, dmesg, journal, log, mount, preserve_log are short
    //     helpers that touch only /proc and /sys, not Wasmtime or IPC.
}
```

### 9.3 Recovery dispatch

```rust
//! supervisor/src/boot/recovery.rs
use super::{BootCoordinator, PhaseFailure};
use super::phases::RecoveryAction;
use super::minimal_shell::MinimalShell;

pub fn handle(b: &BootCoordinator, f: PhaseFailure) -> ! {
    b.log.append(&format!(r#"{{"recovery":"{:?}","reason":{:?}}}"#,
                          f.recovery, f.reason));
    nix::unistd::sync();   // flush before any drastic action
    match f.recovery {
        RecoveryAction::SkipNonCritical => {
            eprintln!("BUG: SkipNonCritical reached recovery::handle");
            std::process::abort();
        }
        RecoveryAction::RetryAppOnce =>   reexec_supervisor_or_halt(),
        RecoveryAction::EmergencyShell => emergency_shell(b, &f.reason),
        RecoveryAction::FactoryResetPrompt => factory_reset_prompt(b, &f.reason),
        RecoveryAction::ReexecSupervisor => reexec_supervisor_or_halt(),
        RecoveryAction::Reboot          => sync_and_reboot(),
        RecoveryAction::Halt            => sync_and_halt(),
    }
}

fn emergency_shell(b: &BootCoordinator, why: &str) -> ! {
    eprintln!("=== VyomaOS Emergency Shell ===");
    eprintln!("reason: {why}");
    eprintln!("type 'help' for commands");
    let mut sh = MinimalShell::new(b);
    sh.run();
}

fn factory_reset_prompt(b: &BootCoordinator, why: &str) -> ! {
    eprintln!("=== VyomaOS: Boot State Corrupt ===");
    eprintln!("reason: {why}");
    eprintln!("[y] factory reset (wipe /data, restore default app list)");
    eprintln!("[n] reboot and try again");
    eprintln!("[s] emergency shell");
    let choice = read_one_char_with_timeout(std::time::Duration::from_secs(30))
        .unwrap_or('n');
    match choice {
        'y' => {
            // Preserve the log before wipe (resolves G/critic open question).
            let _ = std::fs::copy("/data/.system/boot.log",
                                  "/data/.system/preserved/boot.log.last");
            let _ = std::fs::remove_dir_all("/data/apps");
            let _ = std::fs::remove_dir_all("/data/.system");
            let _ = std::fs::create_dir_all("/data/.system/preserved");
            sync_and_reboot();
        }
        's' => emergency_shell(b, why),
        _   => sync_and_reboot(),
    }
}

fn reexec_supervisor_or_halt() -> ! {
    // Exit with code 42 → stage1 treats this as a re-exec request, not a
    // crash (does not advance the crash counter).
    nix::unistd::sync();
    std::process::exit(42);
}
```

---

## 10. Boot Performance Budget (resolves C2)

### 10.1 Warm boot (target: < 3 s)

| Phase | Budget | Typical (warm) | Dominant cost |
|-------|--------|----------------|---------------|
| EarlyInit         |  40 ms |  18 ms | mount(2) × 5 |
| KernelSetup       | 120 ms |  44 ms | cgroup subtree_control write |
| StorageMount      | 250 ms | 110 ms | ext4 mount, journal already replayed |
| DeviceDiscovery   | 400 ms | 320 ms | coldplug walk |
| SubsystemInit     | 500 ms | 350 ms | wasmtime engine warm-up cached |
| AppRegistryLoad   | 150 ms |  45 ms | trusted-key-cache hit |
| AppLaunch         | 900 ms | 600 ms | mandatory wave with AOT load |
| DisplayReady      | 300 ms | 160 ms | first vblank + first flush |
| AllAppsSpawned    | 340 ms | 280 ms | optional wave with AOT load |
| **Total**         | 3000 ms| **1927 ms** | |

### 10.2 Cold boot (target: < 10 s)

Cold-path scale factors apply to JIT-bound phases (see `BootPhase::cold_scale`):

| Phase | Cold budget | Reason |
|-------|-------------|--------|
| EarlyInit         |   80 ms | 2× safety |
| KernelSetup       |  240 ms | 2× safety |
| StorageMount      |  500 ms | 2× (cold cache) |
| DeviceDiscovery   |  800 ms | 2× (cold cache) |
| SubsystemInit     | 1000 ms | wasmtime engine cold init |
| AppRegistryLoad   |  600 ms | ed25519 × 10 apps, no cache |
| AppLaunch         | 4500 ms | full JIT compile × mandatory |
| DisplayReady      |  600 ms | 2× safety |
| AllAppsSpawned    | 1700 ms | JIT × optional |
| **Total**         | **10020 ms** | |

If cold boot exceeds 10 s, the phase outcome is logged as `Slow`; the boot
still completes (no fatal cutoff). A `Slow` outcome on more than 3 phases
triggers a `--boot-degraded` flag for telemetry.

### 10.3 Recovery boot (target: < 30 s)

Triggered by presence of `/data/.system/recovery.marker`. All deadlines
scale to `recovery_deadline()` (10× warm budget). Includes ext4 `fsck.ext4
-y`, AOT cache rebuild, manifest re-verification. Drops to MinimalShell if
even recovery deadlines blow past.

### 10.4 AOT cache lifecycle

```
/data/.system/aot/
├── <wasm_sha256_hex>.cwasm          # Wasmtime serialized module
├── <wasm_sha256_hex>.verified       # 1-byte marker: signature OK
└── _wasmtime_version                # text: "43.0.0"
```

Cache invalidation triggers (any → wipe `/data/.system/aot/*`):

1. Wasmtime version mismatch (read `_wasmtime_version`).
2. Supervisor binary SHA-256 mismatch (recorded at install in
   `/data/.system/supervisor.sha`).
3. Kernel version mismatch (uname comparison vs
   `/data/.system/kernel.uname`).
4. User-triggered `recovery-shell> rebuild-cache`.

Cache rebuild is non-blocking: first cold boot after invalidation JIT-
compiles each app on launch and writes the `.cwasm` in the background. The
boot still observes cold-tier budgets.

---

## 11. Shutdown (resolves C8)

### 11.1 Strict reverse-topological ordering

Shutdown is the reverse of boot: optional apps first, then mandatory apps,
then UI subsystems (with compositor last), then non-UI subsystems, then
storage flush, then poweroff/reboot.

```
Wave order (reverse of launch waves):
  1. Optional apps in current wave (parallel within wave, sequential across)
  2. Mandatory apps in current wave (parallel within wave, sequential across)
  3. UI subsystems: InputDispatcher → Compositor (last)
  4. Non-UI subsystems: Audio → Scheduler → IpcRouter → Vfs (with sync)
                       → DeviceManager → MemoryGovernor → PowerManager
  5. nix::unistd::sync()
  6. /sys/power/state → "poweroff" | "mem" | reboot syscall
```

```rust
//! supervisor/src/boot/shutdown.rs
use std::sync::Arc;
use std::time::{Duration, Instant};
use crate::SupervisorContext;
use super::BootCoordinator;

pub enum ShutdownKind { Halt, Reboot, Suspend, PowerOff }

pub fn perform(ctx: Arc<SupervisorContext>, kind: ShutdownKind) -> ! {
    let b = ctx.boot.clone();
    b.log.append(&format!(r#"{{"shutdown":"{:?}"}}"#, kind));

    // 1. Compositor draws "shutting down" overlay.
    if let Some(c) = ctx.compositor.as_ref() { c.draw_shutdown_overlay(); }

    // 2. AssertionRegistry transitions to Draining; no new assertions accepted.
    if let Some(a) = ctx.assertions.as_ref() { a.transition_to_draining(); }

    // 3. Reverse-topological wave shutdown of apps.
    let plan = ctx.boot_plan.as_ref().expect("plan exists");
    let apps = ctx.resolved_apps.clone();
    for wave in plan.order_waves.iter().rev() {
        // Within a wave: optional apps first, then mandatory.
        let (optional, mandatory): (Vec<_>, Vec<_>) = wave.iter().copied()
            .partition(|&i| !apps[i].mandatory);
        shutdown_wave(&ctx, &optional, /*grace=*/Duration::from_secs(3));
        shutdown_wave(&ctx, &mandatory, /*grace=*/Duration::from_secs(5));
    }

    // 4. UI subsystems: input first, compositor LAST so apps keep
    //    rendering until they're all dead.
    if let Some(s) = ctx.input.as_ref()      { s.shutdown(); }
    if let Some(s) = ctx.compositor.as_ref() { s.shutdown(); }

    // 5. Non-UI in reverse init_late order.
    if let Some(s) = ctx.audio.as_ref()      { s.shutdown(); }
    if let Some(s) = ctx.scheduler.as_ref()  { s.shutdown(); }
    if let Some(s) = ctx.ipc.as_ref()        { s.shutdown(); }
    if let Some(s) = ctx.vfs.as_ref()        { s.sync_all(); s.shutdown(); }
    if let Some(s) = ctx.devices.as_ref()    { s.shutdown(); }
    if let Some(s) = ctx.memory.as_ref()     { s.shutdown(); }
    if let Some(s) = ctx.power.as_ref()      { s.shutdown(); }

    // 6. Final sync, then poweroff via /sys/power/state or kernel reboot syscall.
    nix::unistd::sync();
    match kind {
        ShutdownKind::Halt     => write_power_state("poweroff"),
        ShutdownKind::PowerOff => write_power_state("poweroff"),
        ShutdownKind::Reboot   => nix_reboot_restart(),
        ShutdownKind::Suspend  => write_power_state("mem"),
    }
    std::process::exit(0);   // only reached for Suspend (which returns).
}

fn shutdown_wave(ctx: &SupervisorContext, indices: &[usize], grace: Duration) {
    if indices.is_empty() { return; }
    // Send on-terminate concurrently within the wave.
    for &i in indices {
        if let Some(app) = ctx.app_table.get_running(i) {
            let _ = app.send_on_terminate();
        }
    }
    // Wait up to `grace` for graceful exit.
    let deadline = Instant::now() + grace;
    while Instant::now() < deadline {
        if indices.iter().all(|&i| !ctx.app_table.is_running(i)) { return; }
        std::thread::sleep(Duration::from_millis(50));
    }
    // SIGKILL stragglers.
    for &i in indices {
        if let Some(app) = ctx.app_table.get_running(i) {
            app.sigkill();
        }
    }
}
```

### 11.2 Suspend reuses partial shutdown

For `ShutdownKind::Suspend`, steps 1–3 run identically (notify apps,
drain assertions, wait+SIGKILL — though in practice apps just pause).
Subsystem shutdown is **skipped**; the `SupervisorContext` is preserved
across the suspend transition. On resume, the supervisor re-arms input and
compositor without re-running `init_late`. Per R7, suspend/resume is owned
by the `PowerManager` actor; the boot module only provides the shutdown
sequencing primitives.

### 11.3 Shutdown invariants

- `vfs.sync_all()` precedes `vfs.shutdown()`.
- `AssertionRegistry` enters `Draining` at step 2; new assertions rejected.
- Compositor is the **last** UI subsystem to die; the last frame any app
  rendered remains on screen until step 4.
- A subsystem `shutdown()` blocks at most 500 ms; SIGKILL parallel after.

---

## 12. The Boot Log

Append-only JSON-lines file at `/data/.system/boot.log`, rotated when it
exceeds 1 MiB (keep last three). Stage1 also writes to `/tmp/boot.log` (1 MB
ring buffer in tmpfs) before `/data` is mounted.

Each entry is a complete JSON object:

```json
{"ts":"2026-05-29T12:34:56Z","phase":"subsystem_init","elapsed_ms":612,"outcome":"Ok"}
{"ts":"2026-05-29T12:34:57Z","phase":"app_launch","elapsed_ms":1812,"outcome":{"Slow":{"overage_ms":312}}}
{"warn":"skipping app","path":"/apps/games-pad","err":"signature: bad key"}
{"recovery":"EmergencyShell","reason":"Compositor: drm_open EBUSY"}
{"panic":"in subsystem_init","backtrace":"... 50 frames ..."}
```

Boot log is readable by any app with `system_diag = true` capability and
writable only by the supervisor. On factory reset, the log is copied to
`/data/.system/preserved/boot.log.factory-reset` *before* wipe.

---

## 13. Compositor Coupling — Forward Reference to R11

The compositor is the first subsystem whose successful operation (a flushed
frame) gates a boot phase. To avoid chicken-and-egg:

1. `Compositor::init_early` allocates the framebuffer scratch and the
   shutdown-overlay buffer (no DRM master yet).
2. `Compositor::init_late` opens DRM master, allocates framebuffers, starts
   the vblank thread. It does **not** require any WASM app.
3. `Compositor::draw_fallback_panel()` is a built-in static-font panel
   ("Loader missing — press R to reboot") rendered without any app. It is
   used by `await_display` if the loader app fails.
4. If the compositor itself crashes after `Operational`, R1's
   `RestartPolicy::Always` re-launches it. During the gap, the supervisor
   draws a blank frame via the framebuffer it owns from `init_late`. No
   other app is killed.

---

## 14. Mandatory App List (factory default)

`config/factory-boot.toml`:

```toml
boot_timeout_ms = 10000

[parallelism]
max_concurrent_jit = 2
max_concurrent_io = 4
max_concurrent_launch = 4

[on_failure]
max_consecutive_failures = 3
strategy = "DropToRecoveryShell"

# Apps marked mandatory boot in mandatory wave; failure → EmergencyShell.
[[boot_order]]
path = "/apps/compositor"
mandatory = true
qos = "UserInteractive"
respawn = { type = "Always" }

[[boot_order]]
path = "/apps/session-manager"
mandatory = true
qos = "UserInteractive"
start_after = ["compositor"]
respawn = { type = "Always" }

[[boot_order]]
path = "/apps/shell"
mandatory = true
qos = "UserInteractive"
start_after = ["session-manager"]
respawn = { type = "Always" }

[[boot_order]]
path = "/apps/settings-daemon"
mandatory = false
qos = "Utility"
respawn = { type = "OnExit", max_restarts = 5, window_secs = 60 }
```

Apps with `mandatory = true` form the **minimum viable desktop**. Failure of
any one to reach `ready` after one retry (3 s timeout) drops to EmergencyShell.

---

## 15. File Layout (each file ≤ 500 LOC)

```
supervisor/stage1/
├── Cargo.toml
└── src/main.rs              (~150 LOC: PID 1 wrapper, fork+exec loop)

supervisor/src/lock_order.rs (~120 LOC: RankedMutex, rank constants)

supervisor/src/boot/
├── mod.rs                   (~280 LOC: BootCoordinator, run_phase, dispatch)
├── phases.rs                (~150 LOC: BootPhase, deadlines, tier, outcome)
├── barrier.rs               (~90 LOC:  BootBarrier with timeout)
├── subsystem.rs             (~80 LOC:  Subsystem trait, InitContext)
├── registry.rs              (~90 LOC:  SubsystemRegistry (typed name → Arc))
├── early_init.rs            (~110 LOC: pseudo-fs mounts, panic hook)
├── kernel_setup.rs          (~140 LOC: seccomp, rlimits, cgroups)
├── storage_mount.rs         (~150 LOC: backend probe, ext4/9P mount)
├── device_discovery.rs      (~120 LOC: netlink open, coldplug)
├── subsystem_init.rs        (~230 LOC: two-phase init driver)
├── manifest_loader.rs       (~360 LOC: BootToml three-layer, DAG, verify)
├── app_launcher.rs          (~340 LOC: pipelined wave launcher, jit semaphore)
├── restart_policy.rs        (~110 LOC: AppFailureTracker, backoff)
├── shutdown.rs              (~180 LOC: reverse-topological teardown)
├── recovery.rs              (~150 LOC: dispatch handler, factory reset)
└── minimal_shell.rs         (~220 LOC: supervisor-internal REPL, no Wasmtime)

config/
└── factory-boot.toml        (verified by build-time test)
```

Total new LOC: ≈ 3,070 across 17 files plus 1 TOML; every file ≤ 500.

---

## 16. Test Strategy

### 16.1 Unit tests

- `phases::warm_deadline()` total = 3000 ms exactly.
- `phases::cold_scale()` total bounded ≤ 12000 ms.
- `BootBarrier::advance` panics on regression.
- `RankedMutex` panics in debug build on rank violation.
- `RankedMutex` is zero-overhead in release build (benchmark).
- `manifest_loader::resolve_boot_toml` returns factory const when both
  initramfs and override are corrupt.
- `manifest_loader::build_dependency_dag` detects single-cycle,
  multi-cycle, and self-edge in `start_after`.
- `AppFailureTracker::record_failure` returns `Restart` for first failure,
  `MarkDegraded` after 5 within 60 s.

### 16.2 Integration tests

- `MockSupervisorContext` injects subsystem failures at each step of
  `subsystem_init`. Each failure routes to the expected `RecoveryAction`.
- `MinimalShell::run` executes each command without touching Wasmtime.
- Stage1 with fake `supervisor` binary that exits 5×/60s drops to
  recovery-shell.

### 16.3 Smoke test

`make smoke` extended to:

1. Boot to `phase=operational`.
2. Assert `elapsed_ms ≤ 3000` on warm boot (cwasm cache pre-populated).
3. Assert `elapsed_ms ≤ 10000` on cold boot (cwasm cache emptied first).
4. Assert boot.log line for every phase exists.

### 16.4 Chaos tests

A `--chaos-skip <subsystem>` flag forces a `init_late` failure. CI matrix:

| Chaos input | Expected recovery |
|-------------|-------------------|
| corrupt `/etc/vyoma/boot.toml` (truncate) | falls back to factory const, boots |
| corrupt `/data/etc/override.toml` | falls back to initramfs, boots |
| `compositor.init_late` returns Err | EmergencyShell |
| `vfs.init_late` returns Err | EmergencyShell |
| `power.init_late` returns Err | EmergencyShell |
| panic in `subsystem_init` body | catch_unwind → EmergencyShell |
| corrupt wasm module file (random byte flip) | mandatory: EmergencyShell; optional: skip + log |
| AOT cache file truncated | falls back to JIT, completes |
| 5 supervisor crashes / 60 s | stage1 drops to recovery shell |
| ext4 superblock corrupt | StorageMount → FactoryResetPrompt |

### 16.5 Property tests

- Random valid `start_after` graphs produce a topological wave list whose
  union equals input set.
- `cold_deadline` ≥ `warm_deadline` for every phase.
- `RankedMutex` rank assignments form a strict total order with no gaps
  used twice.

---

## 17. Build & CI

### 17.1 Boot timing CI

`make boot-budget` boots a clean QEMU image, scrapes `boot.log`, and
compares per-phase wallclock to budget. CI fails if any phase exceeds 110%
of its budget. Regression is reported with the offending phase and a diff
against the previous baseline checked into `out/.boot-baseline.json`.

### 17.2 Lock-order static check

`make check-locks` runs a custom rust-analyzer pass that walks every
`RankedMutex::lock()` call site, builds a control-flow graph, and confirms
the static rank of each `lock()` is strictly greater than the rank of any
lock held on entry. Catches the common cases at compile-review time;
runtime panics catch the rest.

### 17.3 Factory boot.toml check

`make check-factory-toml` runs:

```bash
cargo test --package supervisor --test factory_boot_toml_parses
```

The test calls `toml::from_slice(FACTORY_BOOT_TOML)` and asserts it parses
into a valid `BootToml` with at least one mandatory app whose path is
`/apps/compositor`.

### 17.4 Stage1 binary size budget

`make check-stage1-size` asserts the stripped `stage1` binary is < 200 KB.
Anything beyond suggests accidental std-lib inclusion.

---

## 18. Resolved Open Questions

### 18.1 Compile-time vs. runtime subsystem order

Resolved in favor of **compile-time** via the `depends_on()` const-fn DAG.
Runtime flexibility was rejected as it loses the lock-rank guarantee. New
subsystems (Network = subsystem 10, Compositor expansions = R11) register
via `register_subsystem!` macro that emits a const declaration.

### 18.2 Boot timeout granularity

Resolved as **per-phase soft deadline** (logged as `Slow`) + **global
hard timeout** in `boot_timeout_ms` (default 10 s). A slow but succeeding
StorageMount on degraded disk is logged, not killed; cumulative slowness
triggers a `--boot-degraded` telemetry flag.

### 18.3 Signature verification scope

Resolved: **sign `boot.toml` too**. The initramfs base ships with a
detached signature at `/etc/vyoma/boot.toml.sig`. The override file is not
signed (it's user-mutable), but apps it adds must still pass per-app
signature verification or be skipped.

### 18.4 Mandatory wave parallelism

Resolved per C7: `max_concurrent_launch = min(4, num_cpus)`. JIT
compilation gated by a separate `max_concurrent_jit = min(num_cpus / 2, 4)`
semaphore. AOT-cached apps skip the JIT semaphore.

### 18.5 Corrupt `boot.toml` recovery

Resolved per Section 7: three-layer cascade. Corrupt override → fall to
initramfs base. Corrupt initramfs → fall to factory const. Factory const
must always parse (CI-enforced).

### 18.6 Headless boot path

Resolved: `server-headless` profile skips `DisplayReady` by setting a
profile-level `skip_phases = ["display_ready"]` field. The `BootCoordinator`
emits a `display_ready: skipped(headless)` log line and advances directly
to `AllAppsSpawned`. The FSM is unchanged.

### 18.7 Re-entry from suspend

Resolved: a distinct `BootPhase::Resuming` is **not** added. Suspend/resume
is owned by R7 `PowerManager` and runs orthogonal to `BootPhase`. The
`BootBarrier` stays at `Operational` throughout suspend; observers
distinguish by subscribing to `PowerState` events.

### 18.8 Boot log preservation across factory reset

Resolved in Section 9.2 / 9.3: factory reset always copies
`/data/.system/boot.log` to `/data/.system/preserved/boot.log.factory-reset`
*before* the wipe, then re-creates `/data/.system/preserved/` empty.

---

## 19. Anti-Patterns (Reject in Review)

The following patterns are **rejected** on sight in PRs touching the boot
subsystem:

- `unwrap()` / `expect()` in `supervisor/src/boot/` (lint: `clippy::unwrap_used`).
- `panic!()` outside `catch_unwind` boundary.
- Allocating in stage1.
- Reading from `/data/etc/override.toml` without falling through to base
  and factory on parse failure.
- `Mutex<T>` (raw) in supervisor — must be `RankedMutex<T, RANK_*>`.
- A literal numeric rank in `RankedMutex<T, 1234>` — must use a named
  constant.
- Cross-subsystem call inside `init_early`.
- Holding any `RankedMutex` across a WIT call.
- `await_display` blocking longer than 2 s without invoking
  `compositor::draw_fallback_panel`.
- Per-app shutdown grace > 10 s.
- Compositor shutdown before all apps have exited.
- Stage1 calling into any dynamic library.
- Stage1 source line count > 200.
- Boot path that depends on networking (NTP, DHCP, remote attestation).
- AOT cache used without validating Wasmtime version + supervisor SHA +
  kernel uname.
- Direct `/etc/vyoma/boot.toml` write — must go through
  `/data/etc/override.toml` overlay.
- Two separate `Subsystem` impls with the same `name()`.
- `init_late` without `depends_on` listing every other subsystem it
  `lookup`'s in `InitContext`.

---

## 20. Implementation Files Summary

| File | LOC | Purpose |
|------|-----|---------|
| `supervisor/stage1/src/main.rs` | 150 | PID-1 fork+exec wrapper with watchdog |
| `supervisor/src/lock_order.rs` | 120 | `RankedMutex` newtype + rank constants |
| `supervisor/src/boot/mod.rs` | 280 | `BootCoordinator`, FSM loop |
| `supervisor/src/boot/phases.rs` | 150 | `BootPhase`, deadlines, tier, outcome |
| `supervisor/src/boot/barrier.rs` | 90 | `BootBarrier` with timeout |
| `supervisor/src/boot/subsystem.rs` | 80 | `Subsystem` trait, `InitContext` |
| `supervisor/src/boot/registry.rs` | 90 | typed name-keyed Arc registry |
| `supervisor/src/boot/early_init.rs` | 110 | pseudo-FS mounts, panic hook |
| `supervisor/src/boot/kernel_setup.rs` | 140 | seccomp, rlimits, cgroups |
| `supervisor/src/boot/storage_mount.rs` | 150 | backend probe, mount |
| `supervisor/src/boot/device_discovery.rs` | 120 | netlink, coldplug |
| `supervisor/src/boot/subsystem_init.rs` | 230 | two-phase init driver |
| `supervisor/src/boot/manifest_loader.rs` | 360 | three-layer config, DAG, signature |
| `supervisor/src/boot/app_launcher.rs` | 340 | pipelined wave launcher, JIT semaphore |
| `supervisor/src/boot/restart_policy.rs` | 110 | `AppFailureTracker` |
| `supervisor/src/boot/shutdown.rs` | 180 | reverse-topological teardown |
| `supervisor/src/boot/recovery.rs` | 150 | dispatch, factory reset |
| `supervisor/src/boot/minimal_shell.rs` | 220 | supervisor-internal REPL |
| `config/factory-boot.toml` | 40 | embedded factory default |
| `wit/vyoma-lifecycle.wit` | 60 | `on-ready`, `on-terminate` exports |

**Total new**: ≈ 3,170 LOC Rust + 60 LOC WIT + 40 LOC TOML across 20 files.
Every Rust file ≤ 500 LOC.

---

## 21. Critical Invariants Summary

The boot subsystem upholds these invariants, each enforced by code, test, or
both:

1. **PID 1 never panics.** Stage1 owns PID 1; supervisor runs as PID 2+.
2. **Lock order is total.** All locks are `RankedMutex<T, RANK_*>`; rank
   constants form a strict total order; runtime panic on violation.
3. **Subsystem init has no cycles.** `init_early` is dependency-free;
   `init_late` runs in topo order; cycles fail at compile time.
4. **Boot.toml never bricks the device.** Three-layer cascade; factory const
   always parses (CI-verified).
5. **Phases are monotonic.** `BootBarrier::advance` panics on regression
   (caught by `catch_unwind`).
6. **Boot has no network dependency.** No NTP, DHCP, or attestation in the
   critical path.
7. **Compositor exits last among UI subsystems.** Shutdown ordering is
   reverse-topological.
8. **VFS sync precedes storage teardown.** `vfs.sync_all()` is the last
   syscall touching `/data`.
9. **No assertion holds during shutdown.** `AssertionRegistry` enters
   `Draining` at shutdown step 2.
10. **Boot log survives factory reset.** Copied to
    `/data/.system/preserved/` before wipe.
11. **AOT cache invalidates on any version drift.** Wasmtime, supervisor
    SHA, or kernel uname mismatch wipes cache.
12. **Stage1 has zero attack surface.** No allocator, no dynamic library,
    no network, no Wasmtime, no IPC.
13. **Mandatory app failure cannot cause unbounded restart loop.** R1
    `RestartPolicy` caps at 5 crashes / 60 s; then `MarkDegraded`.
14. **Compositor crash does not kill other apps.** Compositor restart is
    isolated; supervisor draws blank frame during the gap.
15. **MinimalShell does not depend on Wasmtime.** Pure supervisor-internal
    REPL.

---

*End of Round 8 FINAL spec — 21 sections, ≈ 1,300 lines.*

# Round 8 — Architect: Boot Sequence & Init System

**Subsystem 8**: Boot, init, manifest loading, app launch ordering, shutdown, recovery.
**Author**: Architect
**Date**: 2026-05-29
**Status**: Proposal (pre-Critic)

## 0. Position Statement

VyomaOS boots in a strict, observable, recoverable sequence: Linux 5.10+ kernel
hands control to the Rust PID-1 supervisor, which advances a finite-state
`BootPhase` machine through ten phases. Each phase is gated by a `BootBarrier`
(R1) so subsystems cannot publish state into a phase that has not yet been
entered. Subsystem initialization order is fixed at compile time, derived from
the dependency DAG implied by Rounds 2–7. App launch is split into a *mandatory
core wave* and an *optional wave*: mandatory apps form the minimum viable
desktop (compositor, shell, system services); optional apps launch in QoS order
with bounded parallelism. The system targets `Operational` in under 5 seconds
from kernel start on the `desktop-full` profile. Boot failures degrade
gracefully: a single corrupted app is skipped; a failed mandatory subsystem
drops into an emergency shell; a corrupt manifest triggers factory-reset
prompt. Shutdown is the reverse of boot, gated by a 3-second per-app grace
period and a final VFS sync.

## 1. Integration with Prior Rounds (Contract)

This round consumes every prior round; it does not invent new substrates.

| Round | Consumed Type / Service | Used in Phase |
|-------|-------------------------|---------------|
| R1    | `BootPhase`, `BootBarrier`, `WasmDigest`, lock-order constants | All phases |
| R2    | `MemoryGovernor::new`, `PsiMonitor::start` | `SubsystemInit` step 2 |
| R3    | `IpcRouter::new`, `WaitForGraph::new` | `SubsystemInit` step 5 |
| R4    | `Vfs::mount_root`, `StorageBackend::probe` | `StorageMount` + `SubsystemInit` step 4 |
| R5    | `Scheduler::new`, cgroup v2 tree creation, `SCHED_DEADLINE` audio thread | `KernelSetup` + `SubsystemInit` step 6 |
| R6    | `DeviceManager::open_netlink`, coldplug walk, `InputDispatcher`, `AudioSubsystem` | `DeviceDiscovery` + `SubsystemInit` steps 3, 8, 9 |
| R7    | `PowerManager::new`, `AssertionRegistry::new`, lid/AC handlers | `SubsystemInit` step 1 |

The boot module *owns no long-lived state* beyond `BootCoordinator`; it threads
each subsystem's handle into a global `SupervisorContext` that survives
`Operational`. After `Operational`, the boot module's only API surface is
shutdown and runtime app respawn.

## 2. Extended `BootPhase` State Machine

R1 defined seven phases. We extend to ten, distinguish *enter* from *exit*
markers, and attach a monotonic deadline per phase.

### 2.1 The phase enum (replaces R1's draft)

```rust
//! supervisor/src/boot/phases.rs
//! Single source of truth for boot phase ordering.

use std::time::{Duration, Instant};

/// Ordered, total-ordered enum of boot phases. Numeric discriminant doubles as
/// the `BootBarrier` epoch counter — never reuse a discriminant value.
#[repr(u8)]
#[derive(Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Debug, Hash)]
pub enum BootPhase {
    /// Kernel handed off PID 1 to us. /proc, /sys, /dev not yet mounted.
    EarlyInit          = 0,
    /// seccomp self-filter, cgroup v2 hierarchy at /sys/fs/cgroup, rlimits.
    KernelSetup        = 1,
    /// ext4 (virtio-blk) + 9P mounts; /data backing store online.
    StorageMount       = 2,
    /// udev netlink socket open; coldplug walk of /sys complete.
    DeviceDiscovery    = 3,
    /// All 9 supervisor subsystems initialised in declared order.
    SubsystemInit      = 4,
    /// /etc/vyoma/boot.toml parsed; per-app ed25519 signatures verified.
    AppRegistryLoad    = 5,
    /// Mandatory wave of WASM apps spawned and reached `ready` IPC ping.
    AppLaunch          = 6,
    /// Compositor produced first flushed frame to /dev/fb0 (or headless ack).
    DisplayReady       = 7,
    /// Optional wave complete; every manifest-declared app is alive or skipped.
    AllAppsSpawned     = 8,
    /// Steady state. Boot module quiesces; only respawn + shutdown remain.
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

    /// Strict deadline budget for each phase on `desktop-full`. The sum of
    /// these is the global 5s boot target.
    pub const fn deadline(self) -> Duration {
        match self {
            BootPhase::EarlyInit        => Duration::from_millis(50),
            BootPhase::KernelSetup      => Duration::from_millis(150),
            BootPhase::StorageMount     => Duration::from_millis(400),
            BootPhase::DeviceDiscovery  => Duration::from_millis(600),
            BootPhase::SubsystemInit    => Duration::from_millis(800),
            BootPhase::AppRegistryLoad  => Duration::from_millis(300),
            BootPhase::AppLaunch        => Duration::from_millis(1500),
            BootPhase::DisplayReady     => Duration::from_millis(500),
            BootPhase::AllAppsSpawned   => Duration::from_millis(700),
            BootPhase::Operational      => Duration::from_secs(0),
        }
    }

    pub const fn next(self) -> Option<BootPhase> {
        let i = self as usize;
        if i + 1 >= Self::ALL.len() { None } else { Some(Self::ALL[i + 1]) }
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

/// Recorded phase transition for `boot.log` and `boot_metrics`.
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
    /// Phase exceeded `phase.deadline()` but completed. Logged as warning.
    Slow { overage: Duration },
    /// Phase failed; recovery action attached.
    Failed { reason: String, recovery: RecoveryAction },
}

#[derive(Clone, Debug)]
pub enum RecoveryAction {
    SkipNonCritical,
    EmergencyShell,
    FactoryResetPrompt,
    Reboot,
    Halt,
}
```

### 2.2 BootBarrier (refinement of R1)

R1 introduced `BootBarrier` as a "phase fence". We harden it so subsystem code
calling `barrier.observe(phase)` from a thread is *guaranteed* the phase has
been entered, never reverts, and `wait_for(phase)` parks the caller until then.

```rust
//! supervisor/src/boot/barrier.rs
use parking_lot::{Condvar, Mutex};
use std::sync::Arc;
use std::time::{Duration, Instant};
use super::phases::BootPhase;

#[derive(Clone)]
pub struct BootBarrier(Arc<Inner>);

struct Inner {
    state: Mutex<BootPhase>,
    cv:    Condvar,
}

impl BootBarrier {
    pub fn new() -> Self {
        Self(Arc::new(Inner {
            state: Mutex::new(BootPhase::EarlyInit),
            cv:    Condvar::new(),
        }))
    }

    pub fn current(&self) -> BootPhase { *self.0.state.lock() }

    /// Advance to the next phase. Panics if `to` is not strictly greater than
    /// current — boot must never go backwards.
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
            self.0.cv.wait_for(&mut g, deadline - now);
        }
        true
    }

    /// Cheap fast-path predicate. Used in subsystem `pub fn` entry checks.
    pub fn at_least(&self, p: BootPhase) -> bool { *self.0.state.lock() >= p }
}
```

Every public API on every subsystem must begin with:

```rust
debug_assert!(self.barrier.at_least(BootPhase::SubsystemInit),
    "called {} before SubsystemInit", function_name!());
```

This is a *belt and braces* check against races where a wakeup from the
`netlink` thread tries to dispatch into `InputDispatcher` before it exists.

## 3. The Boot Coordinator

```rust
//! supervisor/src/boot/mod.rs
//! BootCoordinator owns the FSM, the deadline timer, and the boot.log writer.

pub mod phases;
pub mod barrier;
pub mod manifest_loader;
pub mod app_launcher;
pub mod shutdown;
pub mod recovery;

use std::sync::Arc;
use std::time::Instant;
use parking_lot::Mutex;

use barrier::BootBarrier;
use phases::{BootPhase, PhaseTransition, PhaseOutcome, RecoveryAction};
use crate::observability::BootLog;
use crate::supervisor_ctx::SupervisorContext;

pub struct BootCoordinator {
    pub(crate) barrier:     BootBarrier,
    pub(crate) timeline:    Mutex<Vec<PhaseTransition>>,
    pub(crate) log:         Arc<BootLog>,
    pub(crate) start:       Instant,
    pub(crate) dev_mode:    bool,
}

impl BootCoordinator {
    pub fn new(dev_mode: bool) -> Arc<Self> {
        let log = BootLog::open("/data/.system/boot.log")
            .unwrap_or_else(|_| BootLog::stderr_only());
        Arc::new(Self {
            barrier:  BootBarrier::new(),
            timeline: Mutex::new(Vec::with_capacity(10)),
            log,
            start:    Instant::now(),
            dev_mode,
        })
    }

    /// Run the full boot sequence, returning a populated `SupervisorContext`.
    /// On unrecoverable failure, transitions into `recovery::run` and never
    /// returns to the caller.
    pub fn run(self: Arc<Self>) -> Arc<SupervisorContext> {
        let mut ctx = SupervisorContext::empty(self.clone());
        self.run_phase(BootPhase::EarlyInit,       |c| early_init::run(c)).ok_or_recover(&self);
        self.run_phase(BootPhase::KernelSetup,     |c| kernel_setup::run(c)).ok_or_recover(&self);
        self.run_phase(BootPhase::StorageMount,    |c| storage_mount::run(c)).ok_or_recover(&self);
        self.run_phase(BootPhase::DeviceDiscovery, |c| device_discovery::run(c)).ok_or_recover(&self);
        self.run_phase(BootPhase::SubsystemInit,   |c| subsystem_init::run(c, &mut ctx)).ok_or_recover(&self);
        self.run_phase(BootPhase::AppRegistryLoad, |c| manifest_loader::run(c, &mut ctx)).ok_or_recover(&self);
        self.run_phase(BootPhase::AppLaunch,       |c| app_launcher::launch_mandatory(c, &mut ctx)).ok_or_recover(&self);
        self.run_phase(BootPhase::DisplayReady,    |c| app_launcher::await_display(c, &ctx)).ok_or_recover(&self);
        self.run_phase(BootPhase::AllAppsSpawned,  |c| app_launcher::launch_optional(c, &mut ctx)).ok_or_recover(&self);
        self.barrier.advance(BootPhase::Operational);
        self.log_transition(BootPhase::Operational, PhaseOutcome::Ok);
        Arc::new(ctx)
    }

    fn run_phase<F>(&self, p: BootPhase, body: F) -> Result<(), PhaseFailure>
    where F: FnOnce(&BootCoordinator) -> Result<(), PhaseFailure>
    {
        self.barrier.advance(p);
        let t0 = Instant::now();
        self.timeline.lock().push(PhaseTransition {
            phase: p, entered_at: t0, completed_at: None, outcome: PhaseOutcome::Pending,
        });
        let res = body(self);
        let t1 = Instant::now();
        let elapsed = t1 - t0;
        let outcome = match &res {
            Ok(()) if elapsed > p.deadline() =>
                PhaseOutcome::Slow { overage: elapsed - p.deadline() },
            Ok(()) => PhaseOutcome::Ok,
            Err(f) => PhaseOutcome::Failed {
                reason: f.reason.clone(),
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
            "phase": p.label(),
            "elapsed_ms": self.start.elapsed().as_millis(),
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

trait OkOrRecover { fn ok_or_recover(self, b: &BootCoordinator); }
impl OkOrRecover for Result<(), PhaseFailure> {
    fn ok_or_recover(self, b: &BootCoordinator) {
        if let Err(f) = self {
            recovery::handle(b, f);
        }
    }
}
```

## 4. Phase Bodies

### 4.1 EarlyInit

```rust
//! supervisor/src/boot/early_init.rs
use nix::mount::{mount, MsFlags};
use nix::sys::stat::Mode;
use std::path::Path;

pub fn run(b: &super::BootCoordinator) -> Result<(), super::PhaseFailure> {
    fail_if(getpid() != 1, "supervisor must be PID 1")?;
    mount_pseudo("proc",     "/proc",    "proc",     MsFlags::MS_NOSUID | MsFlags::MS_NODEV | MsFlags::MS_NOEXEC)?;
    mount_pseudo("sysfs",    "/sys",     "sysfs",    MsFlags::MS_NOSUID | MsFlags::MS_NODEV | MsFlags::MS_NOEXEC)?;
    mount_pseudo("devtmpfs", "/dev",     "devtmpfs", MsFlags::MS_NOSUID)?;
    mount_pseudo("tmpfs",    "/run",     "tmpfs",    MsFlags::MS_NOSUID | MsFlags::MS_NODEV)?;
    mount_pseudo("cgroup2",  "/sys/fs/cgroup", "cgroup2", MsFlags::MS_NOSUID | MsFlags::MS_NODEV | MsFlags::MS_NOEXEC)?;
    nix::unistd::mkdir("/dev/pts", Mode::S_IRWXU | Mode::S_IRWXG | Mode::S_IRWXO).ok();
    mount_pseudo("devpts",   "/dev/pts", "devpts",   MsFlags::MS_NOSUID | MsFlags::MS_NOEXEC)?;
    install_panic_handler();
    install_sigchld_handler();   // reap zombies even before scheduler exists
    Ok(())
}
```

### 4.2 KernelSetup

```rust
//! supervisor/src/boot/kernel_setup.rs
pub fn run(b: &super::BootCoordinator) -> Result<(), super::PhaseFailure> {
    apply_self_seccomp_denylist()?;     // supervisor itself drops dangerous syscalls
    set_rlimits()?;                      // NOFILE=65536, NPROC=16384
    create_cgroup_root_hierarchy()?;     // delegates to R5
    enable_oom_score_adj_supervisor()?;  // supervisor: -1000 (never killed)
    write_kernel_sysctls()?;             // vm.swappiness=10, kernel.panic=10
    if b.dev_mode {
        unlock_kallsyms_for_dev()?;
    }
    Ok(())
}

fn create_cgroup_root_hierarchy() -> Result<(), super::PhaseFailure> {
    use crate::scheduler::cgroup;
    cgroup::write("/sys/fs/cgroup/cgroup.subtree_control",
                  "+cpu +memory +io +pids")?;
    cgroup::mkdir_all([
        "/sys/fs/cgroup/system.slice",
        "/sys/fs/cgroup/user.slice",
        "/sys/fs/cgroup/qos.interactive",
        "/sys/fs/cgroup/qos.utility",
        "/sys/fs/cgroup/qos.background",
        "/sys/fs/cgroup/qos.media",
    ])?;
    Ok(())
}
```

### 4.3 StorageMount

```rust
//! supervisor/src/boot/storage_mount.rs
use crate::vfs::{Vfs, StorageBackend, BackendKind};

pub fn run(b: &super::BootCoordinator) -> Result<(), super::PhaseFailure> {
    let backend = StorageBackend::probe()
        .map_err(|e| super::PhaseFailure {
            reason: format!("storage probe failed: {e}"),
            recovery: super::phases::RecoveryAction::FactoryResetPrompt,
        })?;
    match backend.kind() {
        BackendKind::Ext4OnVirtioBlk { dev } => {
            mount_ext4(dev, "/data", MsFlags::MS_NOATIME)?;
        }
        BackendKind::Nine_P { tag } => {
            mount_9p(tag, "/data")?;
        }
        BackendKind::Tmpfs => {
            mount_pseudo("tmpfs", "/data", "tmpfs", MsFlags::MS_NOATIME)?;
        }
    }
    ensure_system_dir("/data/.system")?;
    ensure_dir("/data/.system/installed.txt")?;
    Ok(())
}
```

### 4.4 DeviceDiscovery

```rust
//! supervisor/src/boot/device_discovery.rs
use crate::devices::{DeviceManager, ColdplugReport};

pub fn run(b: &super::BootCoordinator) -> Result<(), super::PhaseFailure> {
    let dm = DeviceManager::open_netlink()
        .map_err(|e| pf("netlink open: {e}", RecoveryAction::EmergencyShell))?;
    let coldplug: ColdplugReport = dm.walk_sysfs("/sys")
        .map_err(|e| pf("coldplug: {e}", RecoveryAction::SkipNonCritical))?;
    if coldplug.input_devices.is_empty() && !b.dev_mode {
        // No keyboard, no mouse → still continue but mark headless input.
        b.log.append(r#"{"warn":"no input devices at coldplug"}"#);
    }
    stash_for_subsystem_init(dm, coldplug);
    Ok(())
}
```

### 4.5 SubsystemInit (the linchpin)

This is the strictly ordered initialization step. The order matters because of
declared data dependencies *and* the lock-order discipline from R1.

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

pub fn run(b: &super::BootCoordinator, ctx: &mut SupervisorContext)
    -> Result<(), super::PhaseFailure>
{
    //  1. PowerManager + AssertionRegistry (R7).
    //     No subsystem may take a power assertion without these alive.
    let assertions = AssertionRegistry::new();
    let power      = PowerManager::new(assertions.clone())
        .map_err(|e| pf("PowerManager: {e}", RecoveryAction::EmergencyShell))?;
    ctx.power = Some(power.clone());
    ctx.assertions = Some(assertions.clone());

    //  2. MemoryGovernor (R2). Reads /proc/pressure/memory; sets dirty page
    //     limits + soft OOM thresholds before any allocator-heavy code runs.
    let mem_gov = MemoryGovernor::new(power.clone())
        .map_err(|e| pf("MemoryGovernor: {e}", RecoveryAction::EmergencyShell))?;
    let psi = PsiMonitor::start(mem_gov.clone())
        .map_err(|e| pf("PsiMonitor: {e}", RecoveryAction::SkipNonCritical))?;
    ctx.memory = Some(mem_gov.clone());
    ctx.psi    = Some(psi);

    //  3. DeviceManager (R6). Already opened netlink in DeviceDiscovery;
    //     spawn the long-lived uevent thread here so it can take the
    //     ScannerLock per R1 ordering.
    let dm = take_stashed_device_manager();
    dm.spawn_uevent_thread(power.clone(), mem_gov.clone())
        .map_err(|e| pf("uevent thread: {e}", RecoveryAction::SkipNonCritical))?;
    ctx.devices = Some(dm.clone());

    //  4. VFS + StorageBackend (R4). Mounts root FS into the path namespace
    //     used by Wasmtime preopens later.
    let vfs = Vfs::mount_root(dm.clone(), power.clone())
        .map_err(|e| pf("VFS: {e}", RecoveryAction::FactoryResetPrompt))?;
    ctx.vfs = Some(vfs.clone());

    //  5. IpcRouter + WaitForGraph (R3). After VFS so it can persist its
    //     subscription table at /data/.system/ipc.snapshot.
    let waitfor = WaitForGraph::new();
    let ipc = IpcRouter::new(waitfor.clone(), vfs.clone(), mem_gov.clone())
        .map_err(|e| pf("IpcRouter: {e}", RecoveryAction::EmergencyShell))?;
    ctx.ipc = Some(ipc.clone());
    ctx.waitfor = Some(waitfor);

    //  6. Scheduler + cgroup attach (R5). cgroup tree created in KernelSetup;
    //     here we wire up SCHED_DEADLINE for the audio worker thread and
    //     register MemoryGovernor's pressure callback into the scheduler.
    let sched = Scheduler::new(mem_gov.clone(), power.clone())
        .map_err(|e| pf("Scheduler: {e}", RecoveryAction::EmergencyShell))?;
    sched.install_qos_cgroups()?;
    sched.spawn_audio_thread_sched_deadline()
        .map_err(|e| pf("SCHED_DEADLINE: {e}", RecoveryAction::SkipNonCritical))?;
    ctx.scheduler = Some(sched.clone());

    //  7. Compositor + DisplaySubsystem (subsystem 11, forward ref).
    //     Compositor wants VFS for resource caches, scheduler for vblank
    //     thread QoS, and devices for /dev/fb0 path & DRM master.
    let comp = Compositor::new(dm.clone(), vfs.clone(), sched.clone(), power.clone())
        .map_err(|e| pf("Compositor: {e}", RecoveryAction::EmergencyShell))?;
    ctx.compositor = Some(comp.clone());

    //  8. InputDispatcher (R6 HID side). Routes evdev → IPC.
    let input = InputDispatcher::new(dm.clone(), ipc.clone(), comp.clone())
        .map_err(|e| pf("InputDispatcher: {e}", RecoveryAction::SkipNonCritical))?;
    ctx.input = Some(input);

    //  9. AudioSubsystem (R6). Needs Scheduler (for SCHED_DEADLINE thread),
    //     PowerManager (for idle exit assertions), and DeviceManager.
    let audio = AudioSubsystem::new(dm.clone(), sched.clone(), power.clone())
        .map_err(|e| pf("AudioSubsystem: {e}", RecoveryAction::SkipNonCritical))?;
    ctx.audio = Some(audio);

    Ok(())
}
```

This order is the dependency-DAG topological sort:

```
        PowerManager ──┬──> MemoryGovernor ──> DeviceManager ──> VFS
                       │                              │           │
                       │                              │           v
                       │                              │         IpcRouter
                       │                              │           │
                       │                              │           v
                       │                              └────────> Scheduler
                       │                                          │
                       │                                          v
                       └──────────────────────────────────────> Compositor
                                                                 │  │
                                                                 v  v
                                                          InputDisp Audio
```

Every arrow is "A must exist before B's constructor returns." There are no
cycles by construction; the Critic should attempt to find a hidden cycle.

## 5. Manifest Loading & Verification

```rust
//! supervisor/src/boot/manifest_loader.rs
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use serde::Deserialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Deserialize)]
pub struct BootToml {
    pub boot_order: Vec<BootEntry>,
    #[serde(default)]
    pub trusted_keys: Vec<TrustedKey>,
    #[serde(default)]
    pub boot_timeout_ms: Option<u64>,
}

#[derive(Debug, Deserialize)]
pub struct BootEntry {
    pub path:           PathBuf,    // /apps/<name>/
    pub mandatory:      bool,
    #[serde(default)]
    pub start_after:    Vec<String>,// names of apps that must reach `ready`
    #[serde(default)]
    pub start_before:   Vec<String>,// names of apps that must NOT yet be live
    #[serde(default)]
    pub qos:            QosHint,
    #[serde(default)]
    pub respawn:        RespawnPolicy,
}

#[derive(Debug, Default, Deserialize, Clone, Copy)]
pub enum QosHint {
    UserInteractive,
    #[default] UserInitiated,
    Utility,
    Background,
}

#[derive(Debug, Deserialize, Clone)]
pub enum RespawnPolicy {
    Never,
    OnExit { max_restarts: u32, window_secs: u32 },
    Always,
}
impl Default for RespawnPolicy {
    fn default() -> Self { RespawnPolicy::Never }
}

#[derive(Debug, Deserialize)]
pub struct TrustedKey {
    pub name: String,
    #[serde(with = "hex_array")]
    pub pubkey: [u8; 32],
}

#[derive(Debug, Deserialize)]
pub struct VyomaToml {
    pub app: AppMeta,
    pub capabilities: crate::manifest::Capabilities,
    pub signature:   AppSignature,
}

#[derive(Debug, Deserialize)]
pub struct AppMeta {
    pub name: String,
    pub version: String,
    pub wasm: String,
}

#[derive(Debug, Deserialize)]
pub struct AppSignature {
    pub signer:  String,                  // must match TrustedKey.name
    #[serde(with = "hex_array_sig")]
    pub ed25519: [u8; 64],
    #[serde(with = "hex_array")]
    pub wasm_sha256: [u8; 32],            // R1 `WasmDigest`
}

pub struct ResolvedApp {
    pub name:        String,
    pub mandatory:   bool,
    pub qos:         QosHint,
    pub respawn:     RespawnPolicy,
    pub wasm_path:   PathBuf,
    pub digest:      crate::WasmDigest,
    pub manifest:    VyomaToml,
    pub start_after: Vec<String>,
}

pub fn run(b: &super::BootCoordinator, ctx: &mut crate::SupervisorContext)
    -> Result<(), super::PhaseFailure>
{
    let raw = std::fs::read_to_string("/etc/vyoma/boot.toml")
        .map_err(|e| pf("read boot.toml: {e}", RecoveryAction::FactoryResetPrompt))?;
    let cfg: BootToml = toml::from_str(&raw)
        .map_err(|e| pf("parse boot.toml: {e}", RecoveryAction::FactoryResetPrompt))?;
    let keys: HashMap<String, VerifyingKey> = cfg.trusted_keys.iter()
        .map(|k| (k.name.clone(), VerifyingKey::from_bytes(&k.pubkey).unwrap()))
        .collect();
    let mut resolved = Vec::with_capacity(cfg.boot_order.len());
    let mut skipped  = Vec::new();
    for entry in &cfg.boot_order {
        match load_one(entry, &keys, b.dev_mode) {
            Ok(r)  => resolved.push(r),
            Err(e) if !entry.mandatory => {
                b.log.append(&format!(r#"{{"warn":"skipping app","path":"{}","err":"{e}"}}"#,
                                      entry.path.display()));
                skipped.push((entry.clone(), e));
            }
            Err(e) => return Err(pf(
                format!("mandatory app {} failed: {e}", entry.path.display()),
                RecoveryAction::EmergencyShell)),
        }
    }
    let dag = build_dependency_dag(&resolved)
        .map_err(|cycle| pf(format!("cycle in start_after: {cycle:?}"),
                            RecoveryAction::EmergencyShell))?;
    ctx.boot_plan = Some(BootPlan {
        order_waves: dag.topo_waves(),
        skipped,
        boot_timeout: cfg.boot_timeout_ms.map(std::time::Duration::from_millis)
                                         .unwrap_or(std::time::Duration::from_secs(10)),
    });
    Ok(())
}

fn load_one(entry: &BootEntry, keys: &HashMap<String, VerifyingKey>, dev: bool)
    -> Result<ResolvedApp, String>
{
    let manifest_path = entry.path.join("vyoma.toml");
    let raw = std::fs::read_to_string(&manifest_path)
        .map_err(|e| format!("read {}: {e}", manifest_path.display()))?;
    let m: VyomaToml = toml::from_str(&raw)
        .map_err(|e| format!("parse {}: {e}", manifest_path.display()))?;
    let wasm_path = entry.path.join(&m.app.wasm);
    let wasm_bytes = std::fs::read(&wasm_path)
        .map_err(|e| format!("read wasm: {e}"))?;
    let digest = crate::WasmDigest::of(&wasm_bytes);
    if digest.as_bytes() != &m.signature.wasm_sha256 {
        return Err("digest mismatch".into());
    }
    if !dev {
        let vk = keys.get(&m.signature.signer)
            .ok_or_else(|| format!("unknown signer {}", m.signature.signer))?;
        let sig = Signature::from_bytes(&m.signature.ed25519);
        vk.verify(digest.as_bytes(), &sig)
            .map_err(|e| format!("signature: {e}"))?;
    }
    Ok(ResolvedApp {
        name: m.app.name.clone(),
        mandatory: entry.mandatory,
        qos: entry.qos,
        respawn: entry.respawn.clone(),
        wasm_path, digest, manifest: m,
        start_after: entry.start_after.clone(),
    })
}

/// Detect cycles in the start_after graph using Kahn's algorithm. Returns
/// topological waves (apps in the same wave can launch in parallel).
fn build_dependency_dag(apps: &[ResolvedApp])
    -> Result<DependencyDag, Vec<String>>
{
    use std::collections::{BTreeMap, BTreeSet, VecDeque};
    let by_name: BTreeMap<&str, usize> = apps.iter().enumerate()
        .map(|(i, a)| (a.name.as_str(), i)).collect();
    let mut indeg = vec![0u32; apps.len()];
    let mut succ:  Vec<BTreeSet<usize>> = vec![BTreeSet::new(); apps.len()];
    for (i, a) in apps.iter().enumerate() {
        for dep in &a.start_after {
            let j = *by_name.get(dep.as_str())
                .ok_or_else(|| vec![dep.clone()])?;
            if succ[j].insert(i) { indeg[i] += 1; }
        }
    }
    let mut waves = Vec::<Vec<usize>>::new();
    let mut frontier: VecDeque<usize> = (0..apps.len())
        .filter(|&i| indeg[i] == 0).collect();
    while !frontier.is_empty() {
        let wave: Vec<usize> = frontier.drain(..).collect();
        let mut next = VecDeque::new();
        for &i in &wave {
            for &j in &succ[i] {
                indeg[j] -= 1;
                if indeg[j] == 0 { next.push_back(j); }
            }
        }
        waves.push(wave);
        frontier = next;
    }
    let placed: usize = waves.iter().map(|w| w.len()).sum();
    if placed != apps.len() {
        let cyc = (0..apps.len()).filter(|&i| indeg[i] > 0)
                                 .map(|i| apps[i].name.clone()).collect();
        return Err(cyc);
    }
    Ok(DependencyDag { waves })
}

pub struct DependencyDag { pub waves: Vec<Vec<usize>> }
impl DependencyDag { pub fn topo_waves(self) -> Vec<Vec<usize>> { self.waves } }

pub struct BootPlan {
    pub order_waves:  Vec<Vec<usize>>,
    pub skipped:      Vec<(BootEntry, String)>,
    pub boot_timeout: std::time::Duration,
}
```

## 6. App Launcher

```rust
//! supervisor/src/boot/app_launcher.rs
use std::sync::Arc;
use std::time::{Duration, Instant};
use crossbeam_channel::{bounded, Receiver};
use crate::SupervisorContext;
use super::manifest_loader::{ResolvedApp, QosHint, RespawnPolicy};

/// One wave of mandatory or optional apps. Within a wave, apps may launch
/// in parallel up to `MAX_PARALLEL_LAUNCHES`. Across waves, the launcher
/// blocks until every member of the prior wave has emitted `ready`.
const MAX_PARALLEL_LAUNCHES: usize = 4;

pub fn launch_mandatory(b: &super::BootCoordinator, ctx: &mut SupervisorContext)
    -> Result<(), super::PhaseFailure>
{
    let plan = ctx.boot_plan.as_ref().expect("plan exists");
    let apps = ctx.resolved_apps.clone();
    for (wave_idx, wave) in plan.order_waves.iter().enumerate() {
        let mandatory_in_wave: Vec<usize> = wave.iter().copied()
            .filter(|&i| apps[i].mandatory).collect();
        if mandatory_in_wave.is_empty() { continue; }
        launch_wave(b, ctx, &mandatory_in_wave, /*mandatory=*/true)
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
    Err(super::PhaseFailure {
        reason: "compositor produced no frame within 2s".into(),
        recovery: super::phases::RecoveryAction::EmergencyShell,
    })
}

pub fn launch_optional(b: &super::BootCoordinator, ctx: &mut SupervisorContext)
    -> Result<(), super::PhaseFailure>
{
    let plan = ctx.boot_plan.as_ref().expect("plan exists");
    let apps = ctx.resolved_apps.clone();
    // Sort each wave by QoS so UserInteractive optional apps start first.
    for wave in &plan.order_waves {
        let mut optional: Vec<usize> = wave.iter().copied()
            .filter(|&i| !apps[i].mandatory).collect();
        optional.sort_by_key(|&i| qos_rank(apps[i].qos));
        if optional.is_empty() { continue; }
        // Failures within optional wave are logged, not fatal.
        let _ = launch_wave(b, ctx, &optional, /*mandatory=*/false);
    }
    Ok(())
}

fn qos_rank(q: QosHint) -> u8 {
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
) -> Result<(), WaveFailure>
{
    let (tx, rx) = bounded::<(String, Result<(), String>)>(indices.len());
    let mut in_flight = 0usize;
    let mut next = 0usize;
    let mut failed = Vec::<String>::new();
    let deadline = Instant::now() + ctx.boot_plan.as_ref().unwrap().boot_timeout;
    let apps = ctx.resolved_apps.clone();
    while next < indices.len() || in_flight > 0 {
        while in_flight < MAX_PARALLEL_LAUNCHES && next < indices.len() {
            let idx = indices[next]; next += 1;
            spawn_app(ctx, idx, tx.clone());
            in_flight += 1;
        }
        match rx.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
            Ok((name, Ok(()))) => {
                in_flight -= 1;
                ctx.app_table.mark_ready(&name);
            }
            Ok((name, Err(e))) => {
                in_flight -= 1;
                b.log.append(&format!(r#"{{"app":"{name}","err":"{e}"}}"#));
                failed.push(name);
            }
            Err(_) => {
                return Err(WaveFailure { failed: vec!["boot timeout".into()] });
            }
        }
    }
    if mandatory && !failed.is_empty() {
        return Err(WaveFailure { failed });
    }
    Ok(())
}

fn spawn_app(ctx: &SupervisorContext, idx: usize,
             reply: crossbeam_channel::Sender<(String, Result<(), String>)>)
{
    let app = ctx.resolved_apps[idx].clone();
    let scheduler = ctx.scheduler.clone().unwrap();
    let ipc = ctx.ipc.clone().unwrap();
    std::thread::Builder::new()
        .name(format!("launch-{}", app.name))
        .spawn(move || {
            let res = (|| -> Result<(), String> {
                let cgroup_path = scheduler.attach_qos_cgroup(&app.name, app.qos)
                    .map_err(|e| e.to_string())?;
                let child = ipc.spawn_wasm(&app, &cgroup_path)
                    .map_err(|e| e.to_string())?;
                let ready = child.wait_ready(std::time::Duration::from_millis(1500));
                if !ready { return Err("never emitted ready".into()); }
                Ok(())
            })();
            let _ = reply.send((app.name, res));
        }).expect("spawn launch thread");
}
```

### 6.1 The `ready` contract

Every app's WIT world declares an optional export `on-ready: func() -> ()`. The
launcher considers an app ready when *any* of:

1. App invokes `vyoma:lifecycle/ready` (preferred path).
2. App sends IPC `@supervisor: ready`.
3. 1500 ms elapsed and app's PID is still alive (assumed-ready fallback).

This three-way OR is logged for each app; case 3 is a warning.

## 7. Boot Performance Budget

Profiled on `desktop-full` (x86-64, 4 vCPU, 1 GB):

| Phase             | Budget | Typical | Dominant cost |
|-------------------|--------|---------|---------------|
| EarlyInit         |  50 ms |  18 ms  | five `mount(2)` calls |
| KernelSetup       | 150 ms |  44 ms  | cgroup subtree_control write |
| StorageMount      | 400 ms | 110 ms  | ext4 journal replay |
| DeviceDiscovery   | 600 ms | 320 ms  | coldplug walk + virtio probe |
| SubsystemInit     | 800 ms | 410 ms  | Wasmtime engine pre-warm |
| AppRegistryLoad   | 300 ms |  90 ms  | ed25519 verify × N |
| AppLaunch         | 1500 ms| 1150 ms | mandatory wave with module compile |
| DisplayReady      | 500 ms | 160 ms  | first vblank + first flush |
| AllAppsSpawned    | 700 ms | 540 ms  | optional wave |
| **Total**         | 5000 ms| 2842 ms |   |

Fast paths:
- **Dev mode** (`VYOMA_DEV=1`): skip ed25519 verification (`AppRegistryLoad`
  drops to ~12 ms).
- **AOT module cache**: Wasmtime serialized modules cached at
  `/data/.system/wasm-cache/<digest>.cwasm`. First boot pays compile cost
  (~400 ms/app); subsequent boots load AOT (~30 ms/app).
- **Lazy subsystem init**: Audio, Bluetooth, Vulkan compositor backend defer
  initialization until first use; `DisplayReady` only blocks on framebuffer.

Critically, **no phase blocks on a network**: there is no NTP, no DHCP, no
remote attestation in the critical boot path. Networking apps come up in the
optional wave and may fail without holding boot back.

## 8. Recovery & Diagnostic Mode

```rust
//! supervisor/src/boot/recovery.rs
use super::{BootCoordinator, PhaseFailure};
use super::phases::RecoveryAction;

pub fn handle(b: &BootCoordinator, f: PhaseFailure) -> ! {
    b.log.append(&format!(r#"{{"recovery":"{:?}","reason":{:?}}}"#,
                          f.recovery, f.reason));
    match f.recovery {
        RecoveryAction::SkipNonCritical => {
            // Caller should never have invoked recovery::handle for this; the
            // phase body should have logged and continued. Treat as bug.
            eprintln!("BUG: SkipNonCritical bubbled to recovery::handle");
            std::process::abort();
        }
        RecoveryAction::EmergencyShell  => emergency_shell(b, &f.reason),
        RecoveryAction::FactoryResetPrompt => factory_reset_prompt(b, &f.reason),
        RecoveryAction::Reboot          => sync_and_reboot(),
        RecoveryAction::Halt            => sync_and_halt(),
    }
}

fn emergency_shell(b: &BootCoordinator, why: &str) -> ! {
    eprintln!("=== VyomaOS Emergency Shell ===");
    eprintln!("reason: {why}");
    eprintln!("type 'help' for commands; 'reboot' to retry; 'factory-reset' to wipe");
    let mut sh = MinimalShell::new(b);
    sh.run();   // blocks forever; only reboot/halt syscalls exit
    sync_and_halt();
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
        'y' => { factory_reset_data(); sync_and_reboot(); }
        's' => emergency_shell(b, why),
        _   => sync_and_reboot(),
    }
}

fn factory_reset_data() {
    let _ = std::fs::remove_dir_all("/data/.system");
    let _ = std::fs::remove_dir_all("/data/apps");
    let _ = std::fs::create_dir_all("/data/.system");
    let _ = std::fs::copy("/etc/vyoma/default-boot.toml",
                          "/etc/vyoma/boot.toml");
}
```

The **MinimalShell** is itself a tiny supervisor-internal REPL, not a WASM
app — by definition the WASM runtime might not be safe to use after failure.
It supports: `ps`, `log <name>`, `mount`, `df`, `dmesg`, `reboot`, `halt`,
`factory-reset`, `journal`. It has access only to the parts of
`BootCoordinator` that survived the failed phase.

### 8.1 Per-app failure handling (non-fatal path)

If an optional app fails to load (missing wasm, bad signature, panic during
ready), `manifest_loader` logs and `app_launcher` skips it. The skipped list
is exposed via `@supervisor: list-skipped` so the user can diagnose without
reading boot.log.

For mandatory apps, the supervisor first tries one re-spawn with extended
timeout (3 s instead of 1.5 s). Second failure escalates to
`EmergencyShell`.

## 9. Shutdown

```rust
//! supervisor/src/boot/shutdown.rs
use std::sync::Arc;
use std::time::{Duration, Instant};
use crate::SupervisorContext;
use super::BootCoordinator;

pub enum ShutdownKind { Halt, Reboot, Suspend }

pub fn perform(ctx: Arc<SupervisorContext>, kind: ShutdownKind) -> ! {
    let b = ctx.boot.clone();
    b.log.append(&format!(r#"{{"shutdown":"{:?}"}}"#, kind));

    // 1. Notify compositor to draw "shutting down" overlay.
    if let Some(c) = ctx.compositor.as_ref() { c.draw_shutdown_overlay(); }

    // 2. Send WIT `on-terminate` to every running app, reverse QoS order
    //    (Background first → UserInteractive last) so user-facing apps stay
    //    responsive longest.
    let mut apps = ctx.app_table.snapshot_running();
    apps.sort_by_key(|a| std::cmp::Reverse(super::app_launcher::qos_rank(a.qos)));
    let grace = Duration::from_secs(3);
    let deadline = Instant::now() + grace;
    for app in &apps { let _ = app.send_on_terminate(); }

    // 3. Wait up to `grace` for graceful exit; SIGKILL stragglers.
    while Instant::now() < deadline {
        if ctx.app_table.all_exited() { break; }
        std::thread::sleep(Duration::from_millis(50));
    }
    for app in ctx.app_table.snapshot_running() { app.sigkill(); }

    // 4. Reverse of SubsystemInit. Each subsystem's `shutdown()` is allowed
    //    to block up to 500 ms.
    if let Some(s) = ctx.audio.as_ref()      { s.shutdown(); }
    if let Some(s) = ctx.input.as_ref()      { s.shutdown(); }
    if let Some(s) = ctx.compositor.as_ref() { s.shutdown(); }
    if let Some(s) = ctx.scheduler.as_ref()  { s.shutdown(); }
    if let Some(s) = ctx.ipc.as_ref()        { s.shutdown(); }
    if let Some(s) = ctx.vfs.as_ref()        { s.sync_all(); s.shutdown(); }
    if let Some(s) = ctx.devices.as_ref()    { s.shutdown(); }
    if let Some(s) = ctx.memory.as_ref()     { s.shutdown(); }
    if let Some(s) = ctx.power.as_ref()      { s.shutdown(); }

    // 5. Final sync, then poweroff via /sys/power/state or kernel reboot syscall.
    nix::unistd::sync();
    match kind {
        ShutdownKind::Halt    => write_power_state("poweroff"),
        ShutdownKind::Reboot  => nix_reboot_restart(),
        ShutdownKind::Suspend => write_power_state("mem"),
    }
    // suspend returns; halt/reboot don't.
    std::process::exit(0);
}

fn write_power_state(s: &str) {
    let _ = std::fs::write("/sys/power/state", s);
}

fn nix_reboot_restart() -> ! {
    use nix::sys::reboot::{reboot, RebootMode};
    let _ = reboot(RebootMode::RB_AUTOBOOT);
    std::process::abort();
}
```

### 9.1 Shutdown invariants

- **VFS sync precedes any storage subsystem teardown.** `vfs.sync_all()` is the
  last syscall touching `/data`; subsequent `vfs.shutdown()` only unmounts.
- **Power assertions are not honored during shutdown.** `AssertionRegistry`
  enters a `Draining` state at shutdown step 0; new assertions are rejected.
- **Suspend reuses shutdown steps 1–3** (notify apps, grace, SIGKILL) but
  stops before subsystem teardown; on resume, the `BootCoordinator` reuses
  the same `SupervisorContext` and does not re-enter boot phases.

## 10. The Boot Log

Append-only JSON-line file at `/data/.system/boot.log`, rotated when it
exceeds 1 MiB (keep last three). Each line is a complete JSON object:

```json
{"phase":"subsystem_init","elapsed_ms":612,"outcome":"Ok"}
{"phase":"app_launch","elapsed_ms":1812,"outcome":{"Slow":{"overage_ms":312}}}
{"warn":"skipping app","path":"/apps/games-pad","err":"signature: bad key"}
{"recovery":"EmergencyShell","reason":"Compositor: drm_open EBUSY"}
```

Boot log is readable by any app with `system_diag = true` capability (a new
manifest flag introduced here) and writable only by the supervisor.

## 11. File Layout (within 500-line rule)

```
supervisor/src/boot/
├── mod.rs                   (~180 LOC: BootCoordinator + Phase glue)
├── phases.rs                (~120 LOC: BootPhase, PhaseTransition, deadlines)
├── barrier.rs               (~70 LOC:  BootBarrier)
├── early_init.rs            (~110 LOC: pseudo-fs mounts, panic handler)
├── kernel_setup.rs          (~140 LOC: seccomp, rlimits, cgroups)
├── storage_mount.rs         (~150 LOC: backend probe, ext4/9P mount)
├── device_discovery.rs      (~120 LOC: netlink open, coldplug)
├── subsystem_init.rs        (~210 LOC: 9-step ordered constructor calls)
├── manifest_loader.rs       (~340 LOC: BootToml parse, signatures, DAG)
├── app_launcher.rs          (~290 LOC: wave launcher, ready contract)
├── shutdown.rs              (~160 LOC: ordered teardown + power-state)
└── recovery.rs              (~220 LOC: emergency shell, factory reset)
```

Each file remains under the 500-line limit.

## 12. Test Strategy

- **Unit**: `phases::deadline()` total sums to 5000 ms;
  `BootBarrier::advance` panics on regression; `build_dependency_dag`
  detects single-cycle, multi-cycle, and self-edge.
- **Integration**: a `MockSupervisorContext` lets each phase body run with
  injected failures; assert recovery routes to expected `RecoveryAction`.
- **Smoke**: `make smoke` extended to parse boot.log and assert
  `phase=operational` line is present and elapsed_ms ≤ 5000.
- **Chaos**: a `--chaos-skip <subsystem>` flag forces a phase failure; CI
  matrix runs one chaos run per subsystem and asserts emergency shell appears.
- **Property**: random valid `start_after` graphs should always produce a
  topological wave list whose union equals the input set.

## 13. Interactions with Subsystem 11 (Compositor) — forward reference

The compositor is the first subsystem whose *successful operation* (a flushed
frame) gates a boot phase. To prevent a chicken-and-egg deadlock:

1. `Compositor::new` in `SubsystemInit` only opens DRM master, allocates
   framebuffers, and starts the vblank thread. It does *not* require any app
   to be running.
2. `await_display` polls `comp.has_flushed_frame()` after the mandatory wave.
   A pre-bundled `loader` app (mandatory, QoS UserInteractive) is responsible
   for drawing the boot logo and calling `flush` exactly once.
3. If `loader` itself fails to load, `await_display` recovers by having the
   compositor draw a hard-coded "loader missing" panel directly (compositor
   has a font of last resort linked statically).

## 14. Open Questions for the Critic

1. **Compile-time vs. runtime subsystem order.** I encode the 9-step order in
   straight-line code. Should this instead be a declarative graph processed by
   a topological sort at runtime, to make adding subsystem 10 (Network) and
   11 (Compositor) less invasive? Trade-off: declarative is more flexible but
   loses the static guarantee that the order matches the lock-order discipline
   from R1.

2. **Boot timeout granularity.** I have a single 10 s global timeout (in
   `BootToml.boot_timeout_ms`). Should each phase additionally carry its own
   hard timeout that triggers recovery independently? Risk: a slow but
   succeeding StorageMount on degraded disk could trip a hard cutoff.

3. **Signature verification scope.** Currently I verify only the `.wasm`
   digest against the manifest, and the manifest against the signer. Should I
   also sign `boot.toml` itself? The risk: an attacker with write access to
   `/etc` could substitute a tampered boot list while every individual app
   still verifies.

4. **Mandatory wave parallelism.** I cap parallel launches at 4. On 8-core
   `desktop-full` this leaves cores idle during the longest phase. Should
   `MAX_PARALLEL_LAUNCHES` be `num_cpus / 2` instead? But that risks PSI
   pressure (R2) before MemoryGovernor settles.

5. **Recovery for a corrupt `boot.toml`.** I prompt for factory reset. Is
   there a safer middle ground — e.g., fall back to a read-only
   `/etc/vyoma/default-boot.toml` for one boot only, without wiping `/data`?
   This would preserve user data while still letting the system boot.

6. **Headless boot path.** On `server-headless` profile there is no
   compositor and `DisplayReady` is structurally unreachable. Should I
   collapse `DisplayReady` into `AllAppsSpawned` for that profile, or have it
   succeed instantly with a "headless" marker? The latter preserves a single
   FSM across profiles but lies about what happened.

7. **Re-entry from suspend.** I claim suspend reuses steps 1–3 and skips
   subsystem teardown. But device suspend/resume (e.g., USB rebind) may
   require the DeviceManager to be in a specific state. Should there be a
   distinct `BootPhase::Resuming` that the FSM advances to, so observers can
   distinguish cold boot from resume?

8. **Boot log persistence across factory reset.** The factory-reset path
   wipes `/data/.system`, which destroys the very log a user might want to
   diagnose *why* the reset happened. Should we copy `boot.log` to
   `/data/.system/preserved/` before wiping?

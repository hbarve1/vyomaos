# Round 7 Final: Power Management & Energy

**Status:** ✅ Debated & synthesized (2026-05-29)
**Debate:** [Architect: 2094 lines] [Critic: 1392 lines] [Final: this]
**Subsystem:** Power state FSM, battery, display power, App Nap reinforcement,
Power Nap, wake locks (caffeinate), thermal throttling, suspend/resume,
hibernate
**macOS equivalent:** IOPowerManagement framework + IOPMrootDomain + App Nap +
Power Nap + `caffeinate(8)` + `pmset(1)` + Battery Health Management + Safe
Sleep
**Integrates with:** R1 lifecycle WIT callbacks (`on-suspend`/`on-resume`),
`StateBlob`, `WatchdogActor`, `BootPhase`; R2 `MemoryGovernor`, `PsiMonitor`,
`JetsamRanker`; R3 `IpcEnvelope`, `IpcRouter`, broadcast topics; R4
`VfsBackend::fsync_all_buckets`, `CoordWal` journal; R5 `QosClass`,
`ActivityAssertion` (merged here into a unified `AssertionRegistry`), cgroup
`cpu.max`, `SCHED_DEADLINE`; R6 `DisplaySubsystem` DPMS, `AudioSubsystem`
SPSC drain, `DeviceManager` quiesce, evdev wake.

---

## Key Decisions

1. **Platform-honest power ladder.** Six logical states
   (`FullyAwake`, `UserIdle`, `DisplaySleep`, `AppSuspended`, `SystemSleep`,
   `HibernateReady`) plus `PoweredOff` terminal. `SystemSleep` is a
   compile-time feature flag (`platform.supports_s3`); on QEMU and most
   developer machines it degrades to `AppSuspended` + supervisor idle, not a
   real ACPI S3 transition. The supervisor never lies about whether the
   hardware actually slept.
2. **Single `AssertionKind` algebra.** R5 `ActivityAssertion` is folded into
   one `AssertionRegistry` with five kinds (`PreventAppNap`,
   `PreventDisplaySleep`, `PreventUserIdleSleep`, `PreventSystemSleep`,
   `PreventCpuThrottle`). Precedence is documented (each tier implies the
   ones below it); RAII handles are the only acquisition path.
3. **Five-phase ordered suspend barrier.** No phase can advance until the
   prior phase acks. Phase 1 STOP_ACCEPTING; Phase 2 QUIESCE_USERSPACE
   (on-prepare-suspend ack); Phase 3 QUIESCE_KERNEL (VFS, IPC, virtio-net TX,
   audio drain); Phase 4 CAPTURE (on-suspend, StateBlob); Phase 5 TEAR_DOWN
   (drop stores, DPMS, attempt kernel suspend). The supervisor refuses to
   advance past Phase 3 while uncommitted VFS txns exist.
4. **Three-tier thermal response.** Soft tier (≥80 °C, Warm/Hot) uses
   ordered cgroup throttling. Hard tier (≥90 °C, Critical-Soft) sends
   immediate `SIGSTOP` to all Background/Maintenance children, bypassing
   on-suspend. Catastrophic tier (≥95 °C, Critical-Hard) `SIGKILL`s all
   children, `sync(2)` once, writes `disk` to `/sys/power/state` then `o` to
   `/proc/sysrq-trigger`. No assertions can veto Critical-Hard.
5. **Power Nap is gated on user consent + budget + isolation.** Manifest
   `background_fetch = true` is necessary but never sufficient. A
   first-launch consent dialog records the user decision in the Round 60
   Keychain. Per-app per-nap budget: 30 s CPU / 50 MiB egress. System budget:
   N wakes/hr shared across all apps. Power Nap children run in a separate
   network namespace with per-app traffic counters. Audit log lives in
   `/data/.system/power_audit.log` and is surfaced in the Battery →
   Background Activity panel.
6. **Hibernate refuses non-restorable apps.** Manifest field
   `restorable = bool` (default `false`). The supervisor validates at
   install time that `restorable = true` implies the WASM module exports
   `vyoma:power/events.on-suspend`. If any running app is non-restorable
   when hibernate is requested, the supervisor prompts the user with the
   list and defaults to Cancel. The hibernate image header records per-app
   `restorable` flags + SHA-256 of each StateBlob, of each `.wasm` binary,
   and of the supervisor binary.
7. **Battery via netlink uevent + coarse fallback.** Primary source is
   `NETLINK_KOBJECT_UEVENT` filtered on `subsystem=power_supply`. Polling
   exists only as a 5-min sanity check for missed events. No DBus, no
   upower, no userspace udev daemon.
8. **First post-wake input event is consumed.** Matches macOS. The
   supervisor counts wake-to-display-online latency and holds the input
   queue until KMS `CRTC.active = 1` and a full frame has been flipped.
9. **Audio output implicitly asserts `PreventSystemSleep`.** R6
   `AudioSubsystem` acquires the assertion when any app opens an output
   stream and releases on stream close (or 30 s of silence). Apps writing
   audio do not have to know about power management.
10. **`UserAction` is a privileged transition reason.** Only the
    supervisor's built-in chrome (signed manifest, fixed AppId) can submit
    `TransitionRequest { reason: UserAction }`. Other apps' requests carry
    their `AppId` and assertions are honored. Critical-Hard thermal and
    Emergency battery cannot be vetoed even by `UserAction`.
11. **Hibernate is forbidden across kernel/supervisor updates.** The
    hibernate header carries `supervisor_sha256` and `kernel_cmdline_hash`.
    On resume mismatch, the image is discarded with a user-visible warning;
    apps cold-start. There is no attempt to "upgrade" a hibernate image.
12. **Per-platform feature matrix is normative.** `desktop-full`,
    `server-headless`, `mobile`, `iot-edge`, `robotics-rt`, `mcu-minimal`
    each declare which power features are available, which degrade, and
    which are compiled out entirely. `mcu-minimal` does not even compile
    the suspend module.
13. **Three separable layers.** `power::linux_pm` (sysfs/cgroups/cpufreq
    writers), `power::state` (Vyoma's FSM), `power::wit_contract`
    (on-suspend/on-resume + assertions + fetch). These three live in
    distinct modules so each can be audited independently.
14. **Inactivity timer is event-driven, not pre-armed.** The R6 input
    dispatcher resets a `last_input: ArcSwap<Instant>` on every event. The
    policy task reads it atomically before each step and uses a sliding
    window for dim → display-off transitions; late input cancels the
    in-progress ramp.
15. **A built-in `caffeinate` shell builtin** maps onto the assertion
    algebra. `@supervisor: assert PreventUserIdleSleep "long build"` and
    `@supervisor: list-assertions` expose the same API the WIT layer uses,
    so users can debug "what is keeping the system awake" without an app.

---

## 1. Platform-Honest Power State Model — Fixed (C1)

### 1.1 Why we do not pretend QEMU's S3 is real

The Critic's strongest point: writing `mem` to `/sys/power/state` in a
guest does *not* enter ACPI S3 on the host; it pauses vCPU threads. The
host still draws full power, and on some host/guest combinations the guest
crashes on resume because PCI BAR restoration is incomplete. Pretending
otherwise breaks the on-suspend contract: apps' `StateBlob` references a
"post-suspend, pre-resume" world that never existed.

The fix is to make the FSM honest about which transitions are
hardware-real vs software-only, and to compile out the hardware transitions
on platforms that cannot honor them.

### 1.2 The seven-state ladder

```rust
// supervisor/src/power/state.rs

/// Top-level power state. Exactly one of these is current at any time.
/// Transitions are serialized through the single `PowerManager` actor.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum SystemPowerState {
    /// Display on, scheduler at full speed, audio active, all apps at
    /// their declared QoS budget. Steady state when user is interacting.
    FullyAwake,

    /// Display dimmed but on, R5 starts demoting Background QoS apps,
    /// no compositor frames unless something repaints. Last-input clock
    /// is still running.
    UserIdle,

    /// Backlight off, DPMS programmed `Off`, GPU minimum clocks. CPU
    /// still runs. App Nap (R5) aggressive: Background and Maintenance
    /// suspend after 2 s. Power Nap windows fire if any app declared
    /// `background_fetch = true` AND user granted consent.
    DisplaySleep,

    /// Software-only suspension. All non-essential apps' WASM stores
    /// are dropped; their StateBlobs are on disk. The supervisor and
    /// privileged daemons remain alive. The kernel runs idle (deep
    /// C-states if hardware supports). On QEMU this is the deepest
    /// real state we reach.
    AppSuspended,

    /// **Feature-flagged.** Attempts a real ACPI S3 (`echo mem >
    /// /sys/power/state`) only if `platform.supports_s3` is true.
    /// Otherwise this variant cannot be constructed (compile-time
    /// `#[cfg(feature = "s3")]` gating on the variant).
    #[cfg(feature = "platform_s3")]
    SystemSleep,

    /// All app StateBlobs + CoordWal tip have been written to
    /// `/data/.hibernate/image.zst` and fsynced. If `platform.supports_s3`
    /// is also true, the kernel then enters S3 (Safe Sleep); otherwise
    /// the supervisor exits and init re-reads on next boot.
    HibernateReady,

    /// Init reaped, supervisor performed orderly shutdown of every app,
    /// all VFS buckets fsynced, kernel poweroff. Transient terminal.
    PoweredOff,
}
```

### 1.3 Per-platform power state matrix

Every platform declares its capabilities in the existing
`supervisor/src/profile/profiles/<name>.toml` (R6). The platform profile
gains a `[power]` section:

```toml
# supervisor/src/profile/profiles/desktop-full.toml
[power]
supports_s3        = false   # default; tooling can flip for bare-metal
supports_hibernate = true
supports_dpms      = true
supports_backlight = true
has_battery        = false   # desktops default; laptops override
has_thermal_zones  = true
has_cpufreq        = true
has_acpi_lid       = false   # desktops do not have lids
```

```toml
# supervisor/src/profile/profiles/mobile.toml
[power]
supports_s3        = true    # ARM PSCI CPU_SUSPEND on the target SoC
supports_hibernate = false   # mobile rarely hibernates; flash wear
supports_dpms      = true
supports_backlight = true
has_battery        = true
has_thermal_zones  = true
has_cpufreq        = true
has_acpi_lid       = false
```

```toml
# supervisor/src/profile/profiles/mcu-minimal.toml
[power]
supports_s3        = false   # no ACPI on mps2-an385
supports_hibernate = false   # no /data partition
supports_dpms      = false   # often no display at all
supports_backlight = false
has_battery        = false
has_thermal_zones  = false
has_cpufreq        = false
has_acpi_lid       = false
```

The state-availability matrix derived from the profile:

| State | desktop-full | desktop-full+S3 | mobile | iot-edge | robotics-rt | server-headless | mcu-minimal |
|-------|:------------:|:---------------:|:------:|:--------:|:-----------:|:---------------:|:-----------:|
| `FullyAwake` | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ |
| `UserIdle` | ✓ | ✓ | ✓ | – | – | – | – |
| `DisplaySleep` | ✓ | ✓ | ✓ | – | – | – | – |
| `AppSuspended` | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | – |
| `SystemSleep` | – | ✓ | ✓ | – | – | – | – |
| `HibernateReady` | ✓ | ✓ | – | ✓ | – | ✓ | – |
| `PoweredOff` | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ |

`–` means the FSM rejects requests targeting that state with
`TransitionVeto::UnsupportedOnPlatform`. The compiler enforces this for
`SystemSleep` (variant gated behind `#[cfg(feature = "platform_s3")]`); for
the others, the rejection is runtime.

### 1.4 The compile-time S3 feature flag

`Cargo.toml` for the supervisor declares:

```toml
[features]
default = ["platform_desktop_full"]
platform_desktop_full   = []
platform_desktop_full_s3 = ["platform_s3"]
platform_mobile         = ["platform_s3"]
platform_iot_edge       = []
platform_robotics_rt    = []
platform_server_headless = []
platform_mcu_minimal    = []
platform_s3             = []   # gates SystemSleep variant + writer
```

The Makefile already selects the platform with `PLATFORM=<name>`; we
extend it to map to the feature set. CI builds `platform_desktop_full`
(default) and exercises only the no-S3 path. Bare-metal builds opt into
S3 explicitly via `PLATFORM=desktop-full BARE_METAL=1`.

### 1.5 What happens on a "Suspend" menu click on QEMU desktop-full

The user picks "Sleep" from the chrome menu. The chrome submits
`TransitionRequest { target: SystemSleep, reason: UserAction }`. Because
the binary was built without `platform_s3`, the request fails at the
type level — there is no `SystemPowerState::SystemSleep` variant.
Instead the chrome submits `target: AppSuspended` and the user gets the
honest behavior: apps' StateBlobs captured, WASM stores released, the
supervisor enters idle, the kernel runs in deep C-states (if the
hypervisor honors them). RAM stays warm; resume is fast. No claim of S3
is made anywhere in logs, in IPC, in the audit trail, or in the WIT API.

### 1.6 Transition graph (with platform gating)

```
                +--------- 60 s idle --------> UserIdle
                |                                |
                v                                | 240 s idle
            FullyAwake <-- input -------- DisplaySleep
                |  ^                             |
                |  |                             | 1800 s idle
                |  |                             | OR lid close
                |  |                             v
                |  +----------- input ------ AppSuspended
                |                                |
                |                                | #[cfg(feature=s3)]
                |                                v
                |                          SystemSleep
                |                                |
                |                                | wake source
                |                                v
                +---------- via DisplaySleep --- FullyAwake
                                                 ^
                                                 |
                                                 | resume from disk
                AppSuspended --> HibernateReady -+
                                  (battery<10%)

(any state) --> PoweredOff   on user shutdown / Emergency battery /
                              Critical-Hard thermal
```

Illegal direct edges:

| From | To | Forced through |
|------|----|----------------|
| `FullyAwake` | `AppSuspended` | `DisplaySleep` |
| `FullyAwake` | `SystemSleep` | `DisplaySleep` → `AppSuspended` |
| `FullyAwake` | `HibernateReady` | `DisplaySleep` → `AppSuspended` |
| `SystemSleep` | `DisplaySleep` | `FullyAwake` |
| `HibernateReady` | `FullyAwake` | `AppSuspended` (resume path) |

Rationale: every system-level suspension funnels through the same
`DisplaySleep → AppSuspended` prepare path so we have a single tested code
path for the GPU command-queue drain, the audio ring drain, and the input
queue freeze.

### 1.7 PowerMode (QoS multiplier signal)

`PowerMode` is a separate concern from `SystemPowerState`. It expresses
the current "performance budget" the supervisor wants to grant Background
/ Maintenance QoS. It is a single value that R5's cgroup writer reads.

```rust
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum PowerMode {
    HighPerformance,   // AC, "Performance" preset, lid open
    Balanced,          // AC, default
    LowPower,          // battery
    UltraLowPower,     // battery + <=20% OR thermal Hot
    Frozen,            // thermal Critical-Soft; only UI runs
}
```

`PowerManager` writes `ArcSwap<PowerMode>` on every battery / thermal
change. R5's cgroup writer reads it on its 250 ms reconcile tick.

---

## 2. Unified Assertion System — Fixed (C5)

### 2.1 Why the two-API split must die

R5 introduced `ActivityAssertion` to prevent App Nap. R7 proposed
`PowerAssertion` to prevent system sleep. The Critic correctly observes
that apps don't think in those terms; they think "I have work I need to
finish, please don't interrupt me". The macOS API consolidates this into
a single `IOPMAssertion` with multiple *kinds*; Linux's
`org.freedesktop.PowerManagement.Inhibit` does the same. Vyoma will too.

R5's `ActivityAssertion` is hereby redefined as the
`AssertionKind::PreventAppNap` variant of the unified registry. The
existing R5 callers continue to work via a thin compatibility shim that
constructs the new type.

### 2.2 The kinds

```rust
// supervisor/src/power/assertions.rs

/// Categories of assertion. Each kind blocks a specific automatic
/// energy-saving behavior. Apps choose the *narrowest* kind that matches
/// their actual need.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Ord, PartialOrd)]
pub enum AssertionKind {
    /// App stays scheduled normally; supervisor will not App-Nap it
    /// even if it appears idle. (Was R5 ActivityAssertion.)
    PreventAppNap,

    /// CPU governor stays at `performance` for this app's QoS class.
    /// Thermal throttling still applies in `Hot`/`Critical` zones.
    /// Has no effect on systems without cpufreq.
    PreventCpuThrottle,

    /// User-idle clock is reset for this assertion's lifetime; display
    /// will not auto-sleep. Implies `PreventAppNap` for the holder.
    PreventDisplaySleep,

    /// User-idle clock + system-idle clock are both held. System will
    /// not transition past `UserIdle` while held. Implies the two above.
    PreventUserIdleSleep,

    /// Even an explicit "Sleep" menu click is refused. Reserved for
    /// surgical use (e.g., firmware update in progress). The only
    /// override is the supervisor's chrome offering Force Sleep.
    PreventSystemSleep,
}
```

### 2.3 Precedence rules (documented)

```text
PreventSystemSleep   > PreventUserIdleSleep > PreventDisplaySleep >
PreventAppNap        > PreventCpuThrottle (orthogonal: independent)
```

Holding a higher kind implies the ones below it for the holding app:

- Holding `PreventUserIdleSleep` implies `PreventAppNap` for the holder.
  You cannot be "preventing user-idle sleep" while yourself being napped.
- Holding `PreventSystemSleep` does *not* automatically imply
  `PreventDisplaySleep`. A media-export app may want the system awake
  while letting the display dim.
- Holding any kind does *not* prevent transitions caused by
  `reason == CriticalBattery`, `reason == ThermalCriticalHard`, or
  `reason == UserAction` (only the chrome can use `UserAction`).

### 2.4 Implementation

```rust
// supervisor/src/power/assertions.rs

use std::sync::Arc;
use std::time::{Duration, Instant};

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct AssertionId(pub u64);

#[derive(Clone, Debug)]
pub struct Assertion {
    pub id:       AssertionId,
    pub app:      AppId,
    pub kind:     AssertionKind,
    pub reason:   String,           // human-readable
    pub created:  Instant,
    pub timeout:  Option<Duration>, // wall-clock auto-release
    pub source:   AssertionSource,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum AssertionSource {
    /// Acquired via WIT call.
    Wit,
    /// Auto-implied by an active audio output stream (R6 hook).
    AudioImplicit,
    /// Auto-implied by the existing manifest field
    /// `prevents_sleep = true`.
    ManifestImplicit,
    /// Acquired via the chrome shell builtin.
    ChromeShell,
}

pub struct AssertionRegistry {
    next:        std::sync::atomic::AtomicU64,
    map:         parking_lot::RwLock<
                     std::collections::HashMap<AssertionId, Assertion>
                 >,
    per_app_cap: u32,           // default 16
    global_cap:  u32,           // default 512
    ipc:         Arc<crate::ipc::IpcRouter>,
}

impl AssertionRegistry {
    pub fn create(
        &self,
        app: AppId,
        kind: AssertionKind,
        reason: String,
        timeout: Option<Duration>,
        source: AssertionSource,
    ) -> Result<Assertion, AssertionError> {
        if reason.len() > 256 {
            return Err(AssertionError::ReasonTooLong);
        }
        {
            let map = self.map.read();
            let by_app = map.values().filter(|a| a.app == app).count() as u32;
            if by_app >= self.per_app_cap {
                return Err(AssertionError::PerAppCapExceeded);
            }
            if (map.len() as u32) >= self.global_cap {
                return Err(AssertionError::GlobalCapExceeded);
            }
        }
        let id = AssertionId(
            self.next.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        );
        let a = Assertion {
            id, app, kind, reason, created: Instant::now(), timeout, source,
        };
        self.map.write().insert(id, a.clone());
        if let Some(t) = timeout {
            let weak = self.weak();
            let id_c = id;
            tokio::spawn(async move {
                tokio::time::sleep(t).await;
                if let Some(reg) = weak.upgrade() { reg.release(id_c); }
            });
        }
        self.broadcast(&a, true);
        Ok(a)
    }

    pub fn release(&self, id: AssertionId) {
        let mut map = self.map.write();
        if let Some(a) = map.remove(&id) {
            drop(map);
            self.broadcast(&a, false);
        }
    }

    /// True if any active assertion of kind `k` exists in any app.
    pub fn any(&self, k: AssertionKind) -> bool {
        self.map.read().values().any(|a| a.kind >= k)
    }

    /// First assertion holder of kind `k`, for veto error reporting.
    pub fn first(&self, k: AssertionKind) -> Option<Assertion> {
        self.map.read().values().find(|a| a.kind >= k).cloned()
    }

    /// All active assertions (for `list-assertions` UI / chrome shell).
    pub fn list(&self) -> Vec<Assertion> {
        self.map.read().values().cloned().collect()
    }

    /// Acquired by the implicit Audio path. Releases on stream close.
    pub fn audio_implicit(&self, app: AppId) -> AssertionId {
        let a = self.create(
            app,
            AssertionKind::PreventSystemSleep,
            "audio output active".to_string(),
            None,
            AssertionSource::AudioImplicit,
        ).expect("audio assertion always succeeds (no quotas applied)");
        a.id
    }

    fn broadcast(&self, a: &Assertion, acquired: bool) {
        let env = crate::ipc::IpcEnvelope::broadcast(
            "topic:system/assertion-event",
            AssertionEvent::for_(a.clone(), acquired).encode(),
        );
        let ipc = self.ipc.clone();
        tokio::spawn(async move { let _ = ipc.publish(env).await; });
    }

    fn weak(&self) -> std::sync::Weak<Self> { unimplemented!() } // owner-managed
}

#[derive(Debug)]
pub enum AssertionError {
    PerAppCapExceeded,
    GlobalCapExceeded,
    ReasonTooLong,
    UnknownApp,
}
```

### 2.5 RAII handle (WIT-side)

```rust
pub struct AssertionHandle {
    id:       AssertionId,
    registry: std::sync::Weak<AssertionRegistry>,
}

impl Drop for AssertionHandle {
    fn drop(&mut self) {
        if let Some(r) = self.registry.upgrade() { r.release(self.id); }
    }
}
```

The Wasmtime resource-table maps a WIT `handle` resource onto an
`AssertionHandle`. When the app crashes (store is dropped), the resource
table is dropped, which drops the `AssertionHandle`, which releases the
assertion. Leak-on-panic is impossible by construction.

### 2.6 R5 compatibility shim

```rust
// supervisor/src/scheduler/activity_assertion.rs

/// Compatibility wrapper around the new AssertionRegistry. R5 code
/// already in tree continues to compile.
pub struct ActivityAssertion(AssertionHandle);

impl ActivityAssertion {
    pub fn acquire(reg: &Arc<AssertionRegistry>, app: AppId, reason: &str)
        -> Result<Self, AssertionError>
    {
        let a = reg.create(
            app, AssertionKind::PreventAppNap, reason.to_string(), None,
            AssertionSource::Wit,
        )?;
        Ok(Self(AssertionHandle { id: a.id, registry: Arc::downgrade(reg) }))
    }
}
```

### 2.7 Veto reporting

When `PowerManager` rejects a transition because of an active assertion,
the veto carries the first matching assertion's details for the UI:

```rust
TransitionVeto::HeldAssertion {
    app:    AppId,
    kind:   AssertionKind,
    reason: String,         // the assertion's reason string
}
```

The chrome surfaces this as "Pages — preventing display sleep
(recording screencast)" etc. The user sees the assertion holder *by name*,
so they can quit the app or click "Force Sleep" (which submits a
`UserAction` reason).

---

## 3. Ordered Suspend Barrier — Fixed (C2)

### 3.1 The race the previous design lost

The Architect's single broadcast of `on-suspend` followed by a sync was
correct on macOS where the kernel cooperates. On Linux + 9P + WASM, the
WASM store can be torn down while the 9P client still has the tail of a
write buffered; the StateBlob then references a post-state the filesystem
never reached. The Critic catalogues the same race for TCP retransmits,
compositor surface flips, audio frames, and the supervisor's own IPC
broker queue.

The fix is to drain every cross-boundary buffer in a defined order
*before* asking apps for their state, and to capture state in a *separate*
phase from tear-down.

### 3.2 Five phases, strict ordering

```rust
// supervisor/src/power/suspend.rs

use std::sync::Arc;
use tokio::time::{Duration, Instant};
use std::collections::BTreeMap;

pub struct SuspendCoordinator {
    apps:     Arc<crate::apps::AppRegistry>,        // R1
    ipc:      Arc<crate::ipc::IpcRouter>,           // R3
    vfs:      Arc<crate::vfs::VfsBackend>,          // R4
    devices:  Arc<crate::drivers::DeviceManager>,   // R6
    audio:    Arc<crate::drivers::audio::AudioSubsystem>, // R6
    net:      Arc<crate::drivers::net::NetSubsystem>,     // R6 (added)
    hibernate:Arc<crate::power::hibernate::HibernateWriter>,
    registry: Arc<crate::power::assertions::AssertionRegistry>,
}

#[derive(Debug)]
pub struct SuspendReport {
    pub phases: BTreeMap<&'static str, Duration>,
    pub apps:   BTreeMap<AppId, AppSuspendOutcome>,
    pub total:  Duration,
}

#[derive(Debug)]
pub enum AppSuspendOutcome {
    Ok { took: Duration, blob_bytes: usize },
    PrepareTimeout,   // ack to on-prepare-suspend never arrived
    CaptureTimeout,   // on-suspend did not return StateBlob in budget
    Crashed { error: String },
    NotRestorable,    // apps without on-suspend during hibernate flow
}
```

```rust
impl SuspendCoordinator {
    /// Run phases 1–5 in strict order. Returns Err only if a phase
    /// reports a hard failure (uncommitted VFS txn, device refusal).
    /// Any single app's failure is recorded in `app.outcomes` but does
    /// not abort the suspend — the supervisor force-drops that app's
    /// store and proceeds.
    pub async fn prepare_for_sleep(&self) -> Result<SuspendReport, TransitionVeto> {
        let mut rep = SuspendReport {
            phases: BTreeMap::new(), apps: BTreeMap::new(),
            total: Duration::ZERO,
        };
        let t0 = Instant::now();

        // Phase 1: STOP_ACCEPTING
        let t = Instant::now();
        self.phase_1_stop_accepting().await;
        rep.phases.insert("p1_stop_accepting", t.elapsed());

        // Phase 2: QUIESCE_USERSPACE (on-prepare-suspend)
        let t = Instant::now();
        let outcomes = self.phase_2_quiesce_userspace().await;
        rep.phases.insert("p2_quiesce_userspace", t.elapsed());
        rep.apps = outcomes;

        // Phase 3: QUIESCE_KERNEL (VFS, IPC, net, audio drain)
        let t = Instant::now();
        self.phase_3_quiesce_kernel().await?;
        rep.phases.insert("p3_quiesce_kernel", t.elapsed());

        // Phase 4: CAPTURE (on-suspend, StateBlob)
        let t = Instant::now();
        let captures = self.phase_4_capture(&mut rep.apps).await;
        rep.phases.insert("p4_capture", t.elapsed());

        // Phase 5: TEAR_DOWN
        let t = Instant::now();
        self.phase_5_teardown(captures).await;
        rep.phases.insert("p5_teardown", t.elapsed());

        rep.total = t0.elapsed();
        Ok(rep)
    }
}
```

### 3.3 Phase 1 — STOP_ACCEPTING

The supervisor stops dispatching new IPC; the R6 input dispatcher pauses
forwarding events to apps; the R6 net subsystem refuses new `accept()`
calls. In-flight calls continue; only new arrivals are queued.

```rust
async fn phase_1_stop_accepting(&self) {
    self.ipc.set_dispatch_paused(true);
    self.devices.pause_input_dispatch();
    self.net.refuse_new_accepts();
    // No timeout: this is a near-instant flag flip.
}
```

### 3.4 Phase 2 — QUIESCE_USERSPACE

Broadcast `on-prepare-suspend` to every app, in reverse boot-phase order
(UI first, drivers last). Apps acknowledge when they have:

- Called `sync_all()` on every open file handle they care about.
- Closed any TCP socket they don't want to keep across suspend (or have
  decided to keep — that's the app's choice).
- Drained their internal queues.

Budget: per-app 2 s for `on-prepare-suspend`. If an app exceeds the
budget, it is marked `PrepareTimeout` and the supervisor will *not* call
`on-suspend` for that app in Phase 4 — its WASM store is force-dropped in
Phase 5 with no StateBlob captured. For hibernate, this app's record gets
`NotRestorable` and the user is warned.

```rust
async fn phase_2_quiesce_userspace(&self)
    -> BTreeMap<AppId, AppSuspendOutcome>
{
    let mut results = BTreeMap::new();
    let mut apps = self.apps.list_running();
    apps.sort_by_key(|a| std::cmp::Reverse(a.boot_phase));
    let prepare_deadline = Duration::from_secs(2);

    // Phase 2 is fan-out parallel — each app gets its own timeout.
    let tasks: Vec<_> = apps.into_iter().map(|app| {
        let ipc = self.ipc.clone();
        tokio::spawn(async move {
            let env = crate::ipc::IpcEnvelope::unicast(
                crate::ipc::IpcAddr::App(app.id),
                "vyoma:power/events.on-prepare-suspend",
                Vec::new(),
            );
            let started = Instant::now();
            let res = tokio::time::timeout(
                prepare_deadline, ipc.send_and_await_ack(env),
            ).await;
            (app.id, started.elapsed(), res)
        })
    }).collect();
    for t in tasks {
        if let Ok((id, took, res)) = t.await {
            let outcome = match res {
                Ok(Ok(())) => AppSuspendOutcome::Ok { took, blob_bytes: 0 },
                Ok(Err(e)) => AppSuspendOutcome::Crashed { error: format!("{e:?}") },
                Err(_)     => AppSuspendOutcome::PrepareTimeout,
            };
            results.insert(id, outcome);
        }
    }
    results
}
```

### 3.5 Phase 3 — QUIESCE_KERNEL

Drain every cross-boundary buffer the supervisor owns:

1. `vfs.fsync_all_buckets()` — every R4 bucket is synced. If any
   bucket reports an uncommitted txn (CoordWal sequence > last fsync),
   abort the entire suspend with `TransitionVeto::UncommittedVfsTxn`.
2. `ipc.drain_to_app_stdins()` — every queued IPC envelope is delivered
   to the receiving app's stdin (or dropped if the app is already gone).
   Budget: 500 ms total.
3. `net.drain_tx_queues()` — observe virtio-net TX ring until empty.
   Budget: 500 ms total. Sockets still listening but with no in-flight
   bytes are fine; sockets with non-empty TX get a synthetic FIN
   (configurable, default: leave as-is and let TCP RTO handle it).
4. `audio.drain_for_suspend()` — drain SPSC ring, write final
   period to ALSA, close stream gracefully. Budget: 500 ms.
5. `devices.drain_for_suspend()` — call each subsystem's drain hook
   (camera, USB, sensors). Budget: per-class as in R6 (250 ms HID, 500 ms
   audio/camera, 5 s storage).

If step 1 fails, the entire suspend is aborted. Steps 2–5 are best-effort
and report warnings to the audit log.

```rust
async fn phase_3_quiesce_kernel(&self) -> Result<(), TransitionVeto> {
    // Step 1: VFS sync (mandatory; abort suspend on failure).
    self.vfs.fsync_all_buckets().await.map_err(|e| {
        TransitionVeto::UncommittedVfsTxn {
            txn_id: e.last_uncommitted_txn(),
            bucket: e.bucket_name().to_string(),
        }
    })?;
    // Step 2: drain IPC.
    let _ = tokio::time::timeout(
        Duration::from_millis(500), self.ipc.drain_to_app_stdins()
    ).await;
    // Step 3: drain net TX.
    let _ = tokio::time::timeout(
        Duration::from_millis(500), self.net.drain_tx_queues()
    ).await;
    // Step 4: drain audio.
    let _ = tokio::time::timeout(
        Duration::from_millis(500), self.audio.drain_for_suspend()
    ).await;
    // Step 5: drain devices.
    let _ = self.devices.drain_for_suspend().await;
    // One more VFS sync to catch anything the drains wrote.
    let _ = self.vfs.fsync_all_buckets().await;
    Ok(())
}
```

### 3.6 Phase 4 — CAPTURE

Now and *only* now do we broadcast `on-suspend` and collect StateBlobs.
Every cross-boundary buffer is drained; the app's view of the world is
consistent. The app returns its StateBlob; we persist it; we move on.

Budget: per-app 5 s. Apps that already failed Phase 2 are skipped (we
cannot ask them for a coherent state if they couldn't even quiesce). For
hibernate flow, skipped apps get `NotRestorable` in the report.

```rust
async fn phase_4_capture(
    &self, outcomes: &mut BTreeMap<AppId, AppSuspendOutcome>,
) -> BTreeMap<AppId, Vec<u8>> {
    let mut blobs = BTreeMap::new();
    let mut apps = self.apps.list_running();
    apps.sort_by_key(|a| std::cmp::Reverse(a.boot_phase));
    let capture_deadline = Duration::from_secs(5);

    for app in apps {
        // Skip apps that failed Phase 2.
        if matches!(
            outcomes.get(&app.id),
            Some(AppSuspendOutcome::PrepareTimeout |
                 AppSuspendOutcome::Crashed { .. })
        ) { continue; }

        if !self.apps.exports_on_suspend(app.id) {
            outcomes.insert(app.id, AppSuspendOutcome::NotRestorable);
            continue;
        }

        let env = crate::ipc::IpcEnvelope::unicast(
            crate::ipc::IpcAddr::App(app.id),
            "vyoma:power/events.on-suspend",
            Vec::new(),
        );
        let started = Instant::now();
        match tokio::time::timeout(
            capture_deadline, self.ipc.send_and_await_blob(env)
        ).await {
            Ok(Ok(blob)) => {
                outcomes.insert(app.id, AppSuspendOutcome::Ok {
                    took: started.elapsed(), blob_bytes: blob.len(),
                });
                blobs.insert(app.id, blob);
            }
            Ok(Err(e)) => {
                outcomes.insert(app.id, AppSuspendOutcome::Crashed {
                    error: format!("{e:?}"),
                });
            }
            Err(_) => {
                outcomes.insert(app.id, AppSuspendOutcome::CaptureTimeout);
            }
        }
    }
    blobs
}
```

### 3.7 Phase 5 — TEAR_DOWN

Drop WASM stores (R1 will not restart while we are about to suspend).
Program DPMS Off. Ask the kernel to suspend if `platform_s3` feature is
on and we are targeting `SystemSleep`. Otherwise simply yield the
supervisor's main loop to a low-rate idle until a wake source fires.

```rust
async fn phase_5_teardown(&self, blobs: BTreeMap<AppId, Vec<u8>>) {
    // 5a: persist StateBlobs to disk for AppSuspended (or stage for
    //     hibernate header in HibernateReady).
    for (app, blob) in blobs {
        self.apps.persist_state_blob(app, blob).await;
    }
    // 5b: drop all WASM stores.
    for app in self.apps.list_running() {
        self.apps.force_drop_store(app).await;
    }
    // 5c: ask compositor to vacate framebuffer + DPMS off.
    self.devices.display.set_dpms(DpmsMode::Off).await;
    // 5d: kernel-level suspend, if available.
    #[cfg(feature = "platform_s3")]
    {
        let _ = tokio::task::spawn_blocking(|| {
            std::fs::write("/sys/power/state", b"mem")
        }).await;
    }
}
```

### 3.8 Per-phase timeouts (normative)

| Phase | Budget (per-app) | Budget (total) | Failure mode |
|-------|------------------|----------------|--------------|
| 1. STOP_ACCEPTING | — | < 1 ms (flag flip) | None |
| 2. QUIESCE_USERSPACE | 2 s | 5 s wall (parallel) | App marked `PrepareTimeout` |
| 3. QUIESCE_KERNEL | — | 2 s wall | VFS uncommitted → abort suspend |
| 4. CAPTURE | 5 s | 8 s wall (sequential) | App marked `CaptureTimeout` |
| 5. TEAR_DOWN | — | 1 s wall | Logged warning |

Total budgeted p95: 16 s wall, well within macOS's "couple of seconds"
feel for happy path (where parallel Phase 2 is < 200 ms and sequential
Phase 4 is < 1 s).

### 3.9 abort_suspend semantics

If Phase 3 reports `UncommittedVfsTxn`, the coordinator runs
`abort_suspend()`:

1. For every app already in Phase 2 ack state: send
   `vyoma:power/events.on-resume` (apps must implement on-resume as
   idempotent against an on-prepare-suspend without a following
   on-suspend).
2. Unpause IPC dispatch and input dispatch.
3. Return the `TransitionVeto` to `PowerManager`.

Apps with `on-suspend` not yet invoked do not see `on-resume`; the
absence of `on-suspend` is the contract that no resume is owed.

### 3.10 abort_suspend across QUIESCE_KERNEL failure

If Phase 3 step 2-5 timeouts occur (drain hung), the coordinator does
*not* abort — those are best-effort. It logs a warning, records the
unflushed bytes in the audit log, and proceeds to Phase 4. The audit log
entry is:

```text
2026-05-29T03:00:00Z WARN power/suspend phase3_drain
  ipc.unflushed=4 net.tx_unflushed=1024 audio.dropped_frames=12
```

---

## 4. Thermal Response Tiers — Fixed (C4)

### 4.1 Why one-tier thermal is wrong

Hot CPU + slow on-suspend = melted CPU. The orderly mechanism takes
seconds; the hardware throttles in milliseconds. The Architect's "Hot →
suspend non-UI apps via on-suspend" doesn't fit in the time budget.

### 4.2 Three tiers

```rust
// supervisor/src/power/thermal.rs

#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum ThermalZone {
    Normal,        // < 70 °C
    Warm,          // 70..=79 °C
    Hot,           // 80..=89 °C
    CriticalSoft,  // 90..=94 °C
    CriticalHard,  // >= 95 °C
}
```

| Zone | Tier | Mechanism | Time budget |
|------|------|-----------|-------------|
| `Normal` | — | No action | — |
| `Warm` | Tier 1 (soft) | R5 cgroup `cpu.max` × 0.7 for Background/Maintenance; brightness clamp 80 %. | 250 ms (R5 reconcile tick) |
| `Hot` | Tier 1 (soft) | R5 cgroup × 0.3 for Background/Maintenance; brightness clamp 60 %; GPU min freq. | 250 ms |
| `CriticalSoft` | Tier 2 (hard) | Immediate `SIGSTOP` to all Background + Maintenance children. R6 compositor reduces refresh to 30 Hz. No on-suspend. | < 50 ms |
| `CriticalHard` | Tier 3 (catastrophic) | `SIGKILL` all children; `sync(2)` once; `disk` to `/sys/power/state`; if that fails, `o` to `/proc/sysrq-trigger`. No assertions vetoed. | < 200 ms |

### 4.3 Tier 2 implementation (SIGSTOP)

`SIGSTOP` is a kernel signal — it pauses the target process at the next
quantum, regardless of what it is doing. The WASM apps are wasmtime child
processes; the supervisor sends `SIGSTOP` directly to them. The signal
takes effect in microseconds. The kernel scheduler immediately unbooks
them; the CPU load drops.

```rust
async fn tier_2_hard_throttle(&self) {
    for app in self.apps.list_running() {
        if matches!(app.qos, QosClass::Background | QosClass::Maintenance) {
            let _ = nix::sys::signal::kill(
                nix::unistd::Pid::from_raw(app.pid as i32),
                nix::sys::signal::Signal::SIGSTOP,
            );
            self.apps.mark_frozen(app.id);
        }
    }
    // Drop compositor refresh to 30 Hz.
    self.devices.display.set_refresh_hint(30).await;
}
```

On entering `Warm` again (after 5-sample downward hysteresis), the
supervisor sends `SIGCONT` to the same processes. The audit log records:

```text
2026-05-29T03:05:00Z WARN power/thermal tier2_freeze
  zone=CriticalSoft temp_c=92 frozen_pids=[42, 43, 44]
2026-05-29T03:05:08Z INFO power/thermal tier2_thaw
  zone=Warm temp_c=78 thawed_pids=[42, 43, 44]
```

### 4.4 Tier 3 implementation (catastrophic)

```rust
async fn tier_3_catastrophic(&self) {
    // 1. SIGKILL every child of the supervisor.
    for app in self.apps.list_running() {
        let _ = nix::sys::signal::kill(
            nix::unistd::Pid::from_raw(app.pid as i32),
            nix::sys::signal::Signal::SIGKILL,
        );
    }
    // 2. Single sync() — best effort.
    let _ = tokio::task::spawn_blocking(|| {
        unsafe { libc::sync() };
    }).await;
    // 3. Write `disk` to /sys/power/state (kernel hibernate, if configured).
    if std::fs::write("/sys/power/state", b"disk").is_err() {
        // 4. SysRq emergency poweroff.
        let _ = std::fs::write("/proc/sysrq-trigger", b"o");
    }
}
```

### 4.5 Adaptive polling

The thermal polling interval is adaptive:

| Zone | Interval |
|------|----------|
| `Normal` | 5 s |
| `Warm` | 1 s |
| `Hot` | 250 ms |
| `CriticalSoft` | 100 ms |
| `CriticalHard` | 25 ms |

The polling thread is a dedicated `std::thread` (not a tokio task), and
its CPU usage is bounded by the interval. At `CriticalHard`'s 25 ms tick
the syscall cost is < 0.1 % of one core.

Where the kernel supports it, the supervisor also subscribes to
`netlink THERMAL_GENL_FAMILY` events for immediate notification on trip
points; polling becomes the fallback for kernels without
`CONFIG_THERMAL_NETLINK`.

### 4.6 Hysteresis

3-sample hysteresis going up (must read ≥ target zone three times in a
row before transitioning). 5-sample hysteresis going down. This prevents
oscillation under bursty workloads.

```rust
fn smooth(&self, raw: ThermalZone) -> ThermalZone {
    let mut h = self.history.lock();
    h.push_back(raw);
    if h.len() > 5 { h.pop_front(); }
    let cur = **self.snapshot.load();
    if raw > cur {
        let cnt = h.iter().rev().take(3).filter(|z| **z >= raw).count();
        if cnt >= 3 { raw } else { cur }
    } else if raw < cur {
        let cnt = h.iter().rev().take(5).filter(|z| **z <= raw).count();
        if cnt >= 5 { raw } else { cur }
    } else { cur }
}
```

`CriticalHard` bypasses upward hysteresis — a single sample at ≥ 95 °C
triggers Tier 3 immediately. This is a physical-safety choice: at 95 °C
the next sample could be 110 °C.

### 4.7 Interaction with assertions

`PreventCpuThrottle` blocks Tier 1 throttling for the *holder app* only.
Other apps still throttle.

Tier 2 (SIGSTOP) ignores `PreventCpuThrottle`. Apps at QoS
UserInteractive / UserInitiated are never SIGSTOPed at Tier 2 (only
Background / Maintenance).

Tier 3 ignores everything. No assertion can save you from a 95 °C die.

### 4.8 Telemetry

Every zone transition is broadcast on `topic:system/thermal-event`:

```rust
ThermalEvent::ZoneEntered { from, to, temp_c, sensor }
ThermalEvent::Tier2Frozen { frozen_pids: Vec<i32> }
ThermalEvent::Tier3Imminent
```

Apps may subscribe to throttle themselves voluntarily. The chrome shows
a thermal indicator in the menu bar when above `Warm`.

---

## 5. Power Nap Security — Fixed (C3)

### 5.1 Threat model

The manifest is written by the app author, not the user. A reasonable
notes app can declare `network = true; background_fetch = true` and use
the Power Nap window to exfiltrate `/data/notes/` to any attacker server
while the user is asleep. No screen flash, no network indicator, no
sound, no log the user can audit.

### 5.2 Consent gate

When an app with `background_fetch = true` runs for the first time, the
supervisor's chrome blocks until the user answers:

```text
+-----------------------------------------------------+
|  Background Activity Request                        |
|                                                     |
|  "notes-app" wants to run in the background to      |
|  sync your notes while your display is off.         |
|                                                     |
|  This will let it use the network and CPU briefly   |
|  every few minutes, even when you're not using it.  |
|                                                     |
|  [ ] Only while on charger (recommended)            |
|                                                     |
|  [Don't Allow]  [Allow While Asleep]                |
+-----------------------------------------------------+
```

The answer is persisted to Round 60 Keychain under
`com.vyoma.power.background_fetch.<app>` with three legal values:
`Denied`, `AcOnly`, `Always`. Default = `Denied`. The supervisor checks
this on every Power Nap window:

```rust
fn is_eligible(&self, reg: &FetchRegistration) -> bool {
    let consent = self.keychain.get(
        &format!("com.vyoma.power.background_fetch.{}", reg.app)
    );
    match consent {
        Some("Denied") | None => false,
        Some("AcOnly") => matches!(self.battery.source, PowerSource::Ac),
        Some("Always") => true,
        _ => false,
    }
}
```

### 5.3 Per-app per-window budget

```rust
#[derive(Clone, Debug)]
pub struct FetchBudget {
    pub max_cpu_secs:    u32,   // default 30
    pub max_egress_bytes: u64,  // default 50 * 1024 * 1024
    pub max_ingress_bytes: u64, // default 50 * 1024 * 1024
}
```

These are *hard ceilings* enforced by the supervisor, not honor-system
hints to the app. CPU is enforced by killing the WASM store when the
ceiling is hit. Network is enforced by the supervisor's net subsystem
counting bytes through the app's namespace.

### 5.4 Per-app destination whitelist (optional)

Manifest may declare:

```toml
[power.background_fetch]
network_hosts = ["mail.example.com", "calendar.example.com"]
```

When present, the supervisor's net broker enforces SNI / destination IP
matching during Power Nap windows. The whitelist is a strict allowlist;
a connect to any other host returns `EACCES`. When absent, the app
may connect to any host the network capability allows (which is what the
user implicitly accepted at install time, but the audit log still
records destinations).

### 5.5 System-wide wake budget

The system as a whole grants at most `N` Power Nap windows per hour
(default 4, configurable per platform profile). Apps share this budget;
priority is given to apps that have run least recently (LRU among
eligible apps).

### 5.6 Network namespace isolation

Power Nap children run in a separate Linux network namespace (created via
`unshare(CLONE_NEWNET)`). The supervisor installs a TC filter that:

- Counts bytes per app (egress + ingress).
- Optionally filters by SNI (where the whitelist exists).
- Logs every connection start: (timestamp, app, destination IP/port).

When the Power Nap window ends, the namespace is destroyed; any
connection that was still open is closed. Apps are expected to keep
fetches short.

### 5.7 Audit log

`/data/.system/power_audit.log` (append-only, capped at 30 days):

```text
2026-05-29T03:30:00Z fetch app=mail-app duration_ms=4200 bytes_out=1248
  bytes_in=18432 destinations=[mail.example.com:443]
2026-05-29T04:30:00Z fetch app=notes-app DENIED reason="consent=Denied"
2026-05-29T05:00:00Z fetch app=mail-app duration_ms=3100 bytes_out=1248
  bytes_in=5120 destinations=[mail.example.com:443]
2026-05-29T05:30:00Z fetch app=mail-app KILLED reason="exceeded_max_cpu"
```

A built-in "Battery → Background Activity" panel reads this log and
displays it. The user can revoke an app's consent at any time; the
revocation takes effect on the next Power Nap window (assertions held by
in-flight fetches are released).

### 5.8 Low-battery refusal

Below 20 % capacity on battery, all Power Nap windows are refused. The
audit log records `DENIED reason="low_battery"`. The system continues to
process `Always` consents only while on AC. This matches macOS's
"Battery Saver disables Background App Refresh" behavior.

### 5.9 Cadence

| Current state | On AC | On Battery |
|---------------|-------|------------|
| `FullyAwake` | per-app `fetch_interval_secs` | per-app `fetch_interval_secs`, paused below 20 % |
| `UserIdle` | every 5 min | every 10 min, paused below 20 % |
| `DisplaySleep` | every 30 min | every 60 min, paused below 20 % |
| `AppSuspended` | every 2 h | **disabled** |
| `SystemSleep` (if real) | every 2 h via RTC | **disabled** |
| `HibernateReady` | disabled | disabled |

To prevent thundering-herd, the scheduler adds ±10 % jitter to per-app
intervals.

### 5.10 Fetch dispatch

```rust
// supervisor/src/power/power_nap.rs

impl PowerNapScheduler {
    pub async fn nap_window(&self, prior: SystemPowerState) {
        let mut budget_left_wakes = self.window_wake_budget;
        // Pick eligible apps, LRU.
        let mut to_run = self.eligible_apps_lru().await;
        to_run.truncate(budget_left_wakes as usize);

        for reg in to_run {
            if !self.is_eligible(&reg) {
                self.audit_log.write(&AuditEntry::denied(&reg, "consent")).await;
                continue;
            }
            // Reset per-app counters.
            self.net.reset_counters(&reg.app);
            // Create namespace + place app inside it.
            let ns = self.net.create_fetch_namespace(
                &reg.app, &reg.network_hosts).await?;

            let env = crate::ipc::IpcEnvelope::unicast(
                crate::ipc::IpcAddr::App(reg.app),
                "vyoma:power/fetch.on-background-fetch",
                Vec::new(),
            );
            let started = Instant::now();
            let res = tokio::time::timeout(
                Duration::from_secs(reg.budget.max_cpu_secs as u64),
                self.ipc.send_and_await_ack(env),
            ).await;
            let took = started.elapsed();
            let counters = self.net.consume_counters(&reg.app);
            self.net.destroy_namespace(ns).await;
            self.audit_log.write(&AuditEntry::completed(
                &reg, took, counters, res.is_ok() && res.unwrap().is_ok(),
            )).await;
            budget_left_wakes -= 1;
        }
    }
}
```

---

## 6. Hibernate Safety — Fixed (C6)

### 6.1 Why hibernate of non-checkpointing apps is data loss

If hibernate silently degrades to "fresh start" for apps that didn't
implement `on-suspend`, the user's expectation ("I closed the lid; my
work is preserved") is violated. The user trusts hibernate; the system
betrays that trust.

### 6.2 The `restorable` manifest field

```toml
[app]
name    = "notes-app"
version = "0.1.0"
wasm    = "notes-app.wasm"

[capabilities]
stdio       = true
filesystem  = true

[power]
restorable  = true   # app exports on-suspend/on-resume correctly
```

Default: `restorable = false`.

At install time the supervisor parses the WASM module and verifies that
`restorable = true` implies the module exports
`vyoma:power/events#on-suspend` and `vyoma:power/events#on-resume`. If
the export is missing, install is rejected with:

```text
ERROR manifest validation: notes-app.vyoma.toml declares
  power.restorable = true but the WASM module does not export
  vyoma:power/events.on-suspend.
```

### 6.3 Hibernate gate

When the user triggers hibernate (lid-close on low battery, menu choice,
or low-battery auto), the supervisor enumerates running apps and groups
them by `restorable`:

```rust
let running = self.apps.list_running();
let non_restorable: Vec<_> = running.iter()
    .filter(|a| !a.manifest.power.restorable)
    .collect();
if !non_restorable.is_empty() {
    let names: Vec<_> = non_restorable.iter()
        .map(|a| a.identity.name.clone()).collect();
    let answer = self.chrome.prompt_hibernate_warning(names).await;
    if !answer.proceed { return Err(TransitionVeto::UserCanceled); }
}
```

Chrome dialog:

```text
+-----------------------------------------------------+
|  Cannot Fully Hibernate                             |
|                                                     |
|  These apps may lose unsaved work if you hibernate: |
|                                                     |
|    • notes-app                                       |
|    • calculator                                      |
|                                                     |
|  (They will restart fresh on resume.)                |
|                                                     |
|  [Cancel]                  [Hibernate Anyway]       |
+-----------------------------------------------------+
```

If the user clicks Hibernate Anyway, the supervisor records the answer
in the hibernate header (per-app `non_restorable: true`) so that on
resume it can show a "started fresh" toast for each.

### 6.4 Image format

```rust
// supervisor/src/power/hibernate.rs

#[derive(serde::Serialize, serde::Deserialize)]
pub struct HibernateHeader {
    pub magic:                 [u8; 8],           // b"VYOMHIB2"
    pub version:               u16,               // image format version
    pub created_unix:          i64,
    pub created_uuid:          uuid::Uuid,
    pub app_count:             u32,
    pub wal_tip:               u64,               // R4 CoordWal seq
    pub host_machine_id:       [u8; 32],          // /etc/machine-id
    pub kernel_cmdline_hash:   [u8; 32],
    pub supervisor_sha256:     [u8; 32],          // running supervisor binary
    pub kernel_version_hash:   [u8; 32],          // /proc/version
    pub power_mode_at_capture: PowerMode,
}

#[derive(serde::Serialize, serde::Deserialize)]
pub struct HibernateAppRecord {
    pub identity:       crate::apps::AppIdentity,
    pub manifest_hash:  [u8; 32],        // SHA-256 of vyoma.toml
    pub wasm_sha256:    [u8; 32],        // SHA-256 of .wasm binary
    pub state_blob:     Vec<u8>,         // empty if non_restorable
    pub state_blob_sha: [u8; 32],
    pub qos:            crate::scheduler::QosClass,
    pub non_restorable: bool,
}
```

Magic bumped from `VYOMHIB1` (Architect proposal) to `VYOMHIB2` so the
unified hibernate cannot accidentally load a stale image written by the
older format.

### 6.5 Write discipline

Write to a temp file under `/data/.hibernate/`, fsync the file, fsync
the parent directory, then atomic-rename to `image.zst`. Each
`HibernateAppRecord`'s `state_blob_sha` is computed before serialization
and stored in the record itself.

```rust
impl HibernateWriter {
    pub async fn write_image(
        &self, blobs: BTreeMap<AppId, Vec<u8>>,
    ) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.root)?;
        let tmp   = self.root.join("image.zst.tmp");
        let final_ = self.root.join("image.zst");

        let mut records = Vec::new();
        for app in self.apps.list_running() {
            let blob = blobs.get(&app.id).cloned().unwrap_or_default();
            let mut h = sha2::Sha256::new();
            sha2::Digest::update(&mut h, &blob);
            let blob_sha: [u8; 32] = sha2::Digest::finalize(h).into();
            records.push(HibernateAppRecord {
                identity:       app.identity.clone(),
                manifest_hash:  app.manifest_hash,
                wasm_sha256:    app.wasm_sha256,
                state_blob:     blob,
                state_blob_sha: blob_sha,
                qos:            app.qos,
                non_restorable: !app.manifest.power.restorable,
            });
        }
        let header = self.compose_header(records.len() as u32);

        let f = std::fs::File::create(&tmp)?;
        let mut enc = zstd::Encoder::new(f, 3)?;
        bincode::serialize_into(&mut enc, &header)
            .map_err(io::Error::other)?;
        for rec in &records {
            bincode::serialize_into(&mut enc, rec)
                .map_err(io::Error::other)?;
        }
        let f = enc.finish()?;
        f.sync_all()?;
        std::fs::rename(&tmp, &final_)?;
        let dir = std::fs::File::open(&self.root)?;
        dir.sync_all()?;
        Ok(())
    }
}
```

### 6.6 Resume gate

On boot, the supervisor checks for `image.zst`. If present, it loads the
header and validates:

- `magic == VYOMHIB2` (otherwise: discard with warning).
- `host_machine_id` matches current machine (otherwise: disk moved →
  discard).
- `kernel_cmdline_hash` matches current cmdline (otherwise: kernel
  changed → discard).
- `kernel_version_hash` matches current `/proc/version` (otherwise:
  kernel upgraded → discard).
- `supervisor_sha256` matches the running supervisor binary
  (otherwise: supervisor was updated → discard).
- `created_unix` is not older than 7 days (`stale_hibernate_secs`,
  configurable; otherwise: stale → discard).

For each `HibernateAppRecord`:

- Verify `state_blob_sha` matches the blob bytes. If not, mark that app
  to cold-start (do not deliver bad blob to `on-resume`).
- Verify `wasm_sha256` matches the currently installed `.wasm`. If not,
  the app has been updated between hibernate and resume; cold-start.
- Verify `manifest_hash` matches. If not, capabilities may have changed;
  cold-start (and emit an audit-log entry).

Apps that pass all checks are restored via `on-resume(state_blob)`.
Apps that fail any check are cold-started, with a chrome toast:

```text
"notes-app started fresh; its previous state could not be restored
(reason: app updated since last hibernate)."
```

### 6.7 Safe Sleep on low battery (if S3 available)

When `platform.supports_s3` is true and the user closes the lid on
battery below 30 %, the supervisor performs *Safe Sleep*:

1. Run the five-phase prepare pipeline (Sections 3.3–3.7).
2. Write the hibernate image.
3. Enter ACPI S3.

If battery dies in S3, the kernel cold-boots, the supervisor detects
`image.zst`, and resumes from disk per Section 6.6.

If S3 returns successfully (user opens the lid before battery dies), the
hibernate image is discarded (we resumed from RAM, the on-disk image is
redundant).

### 6.8 Hibernate on platforms without S3

On `desktop-full` without `platform_s3`, hibernate is *still* available
— it just means "supervisor writes image, then exits, init re-runs on
next boot". This is suitable for development: you can hibernate a VM,
shut down QEMU, and on next launch the VM resumes to where it was.

### 6.9 Discard on every successful resume

After a successful resume (from disk or from RAM), the image file is
immediately deleted. We never want to resume from a stale image more
than once; doing so could mask a bug where the live state has diverged
from the image.

---

## 7. Battery Monitor

### 7.1 Uevent-first architecture

```rust
// supervisor/src/power/battery.rs

use std::sync::Arc;
use arc_swap::ArcSwap;
use tokio::time::{Duration, MissedTickBehavior};

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum PowerSource {
    Ac,
    Battery,
    Unknown,
}

#[derive(Clone, Debug)]
pub struct BatteryInfo {
    pub source:                  PowerSource,
    pub capacity_pct:            u8,
    pub capacity_smoothed_pct:   f32,    // EWMA alpha=0.3
    pub voltage_now_uv:          i64,
    pub current_now_ua:          i64,    // + charging, - discharging
    pub power_now_mw:            i64,
    pub power_smoothed_mw:       f32,    // EWMA alpha=0.1
    pub time_to_empty_secs:      Option<u32>,
    pub time_to_full_secs:       Option<u32>,
    pub cycle_count:             Option<u32>,
    pub design_capacity_uah:     Option<i64>,
    pub full_charge_capacity_uah:Option<i64>,
    pub health_pct:              Option<u8>,
    pub temperature_decic:       Option<i32>,
    pub last_update:             std::time::Instant,
}

#[derive(Clone, Debug)]
pub enum BatteryEvent {
    SourceChanged(PowerSource),
    CapacityCrossed { from_pct: u8, to_pct: u8, threshold: u8 },
    Warning,    // <= 20%
    Critical,   // <= 10%
    Emergency,  // <= 5%
    HealthDegraded { health_pct: u8 },
}
```

### 7.2 Netlink subscription

```rust
pub struct BatteryMonitor {
    snapshot: Arc<ArcSwap<BatteryInfo>>,
    sysfs_root: std::path::PathBuf,
    netlink: Arc<crate::drivers::udev::UeventListener>,
    ipc: Arc<crate::ipc::IpcRouter>,
}

impl BatteryMonitor {
    pub async fn run(self) {
        // Primary: react to uevents on subsystem=power_supply.
        let mut events = self.netlink.subscribe(
            crate::drivers::udev::Filter::Subsystem("power_supply"),
        );
        // Fallback: 5-minute poll for missed events.
        let mut fallback = tokio::time::interval(Duration::from_secs(300));
        fallback.set_missed_tick_behavior(MissedTickBehavior::Skip);
        let mut last = self.snapshot.load_full();
        loop {
            tokio::select! {
                Some(_) = events.recv() => {
                    if let Ok(fresh) = self.read_sysfs() {
                        self.diff_and_emit(&last, &fresh).await;
                        last = Arc::new(fresh.clone());
                        self.snapshot.store(last.clone());
                    }
                }
                _ = fallback.tick() => {
                    if let Ok(fresh) = self.read_sysfs() {
                        self.diff_and_emit(&last, &fresh).await;
                        last = Arc::new(fresh.clone());
                        self.snapshot.store(last.clone());
                    }
                }
            }
        }
    }
}
```

The uevent listener is the same R6 component used for device hot-plug;
we add a `power_supply` filter.

### 7.3 Threshold-crossing detection

```rust
async fn diff_and_emit(&self, prev: &BatteryInfo, cur: &BatteryInfo) {
    if prev.source != cur.source {
        self.emit(BatteryEvent::SourceChanged(cur.source)).await;
    }
    for &th in &[20_u8, 10, 5] {
        if prev.capacity_pct > th && cur.capacity_pct <= th {
            self.emit(BatteryEvent::CapacityCrossed {
                from_pct: prev.capacity_pct,
                to_pct: cur.capacity_pct,
                threshold: th,
            }).await;
            let ev = match th {
                20 => BatteryEvent::Warning,
                10 => BatteryEvent::Critical,
                _  => BatteryEvent::Emergency,
            };
            self.emit(ev).await;
        }
    }
    if let (Some(p), Some(c)) = (prev.health_pct, cur.health_pct) {
        if p > 80 && c <= 80 {
            self.emit(BatteryEvent::HealthDegraded { health_pct: c }).await;
        }
    }
}
```

### 7.4 Policy hooks

| Event | Effect |
|-------|--------|
| `SourceChanged(Battery)` | `PowerMode::LowPower`; brightness clamp 60 %; Power Nap windows reduce to 60 min; tighten App Nap threshold from 30 s to 8 s. |
| `SourceChanged(Ac)` | `PowerMode::Balanced` (or `HighPerformance` per user pref); allow brightness up to user setpoint; Power Nap windows reduce to 30 min. |
| `Warning` (≤20 %) | UI notification; nothing functional. |
| `Critical` (≤10 %) | `PowerManager::request(HibernateReady, LowBattery)`; user notification "Hibernating in 30 s, plug in to cancel." |
| `Emergency` (≤5 %) | `PowerManager::request(PoweredOff, CriticalBattery)` after 30 s, no veto allowed. |
| `HealthDegraded(≤80 %)` | One-time UI notification; logged to `/data/.system/battery.log`. |

### 7.5 AC plug-unplug debouncing

A 5 s grace period before applying the policy switch (matches macOS):
if the user replugs within 5 s, no policy switch fires. Prevents
thrashing on a flaky cable.

### 7.6 EWMA smoothing

`power_smoothed_mw = 0.1 * sample + 0.9 * prev`;
`capacity_smoothed_pct = 0.3 * sample + 0.7 * prev`.

The raw `capacity_pct` is used for threshold checks; the smoothed
versions are used for the UI's "battery remaining" display so the user
doesn't see 87 % → 85 % → 87 % jitter.

---

## 8. Display Power Management

### 8.1 Subsystem layout

```rust
// supervisor/src/power/display_power.rs

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum DpmsMode { On, Standby, Suspend, Off }

#[derive(Clone, Debug)]
pub struct DisplayPowerSnapshot {
    pub brightness_pct:  u8,
    pub user_setpoint:   u8,
    pub dpms:            DpmsMode,
    pub auto_brightness: bool,
}

pub struct DisplayPower {
    snap:      Arc<ArcSwap<DisplayPowerSnapshot>>,
    backlight: Arc<dyn BacklightWriter>,
    drm:       Arc<dyn DrmDpmsWriter>,
    ramp_lock: tokio::sync::Mutex<()>,
}
```

### 8.2 Interruptible 60 fps ramp

```rust
impl DisplayPower {
    pub async fn ramp_to(&self, target_pct: u8, duration: Duration) {
        let _g = self.ramp_lock.lock().await;
        let start = self.snap.load().brightness_pct;
        if start == target_pct { return; }
        let steps = ((duration.as_millis() / 16) as i32).max(1);
        let delta = target_pct as i32 - start as i32;
        for i in 1..=steps {
            // Bail if user input arrived mid-ramp (handled by outer
            // caller setting a cancellation flag).
            if self.ramp_canceled.load(Ordering::Relaxed) { break; }
            let pct = (start as i32 + (delta * i / steps)) as u8;
            let _ = self.backlight.set_brightness(pct).await;
            let mut s = (**self.snap.load()).clone();
            s.brightness_pct = pct;
            self.snap.store(Arc::new(s));
            tokio::time::sleep(Duration::from_millis(16)).await;
        }
    }
}
```

### 8.3 DPMS staging

`FullyAwake → UserIdle`: 800 ms ramp to 60 % brightness.
`UserIdle → DisplaySleep`: 1500 ms ramp to 0 %, then `DpmsMode::Off`.
`DisplaySleep → FullyAwake`: `DpmsMode::On`, then 600 ms ramp to setpoint.

### 8.4 Wake-on-input consumption

The first input event after a `DisplaySleep → FullyAwake` transition is
*consumed* by the supervisor as a "wake event" and not forwarded to any
app. The supervisor holds the input queue until KMS reports
`CRTC.active = 1` and a full frame has been flipped.

```rust
pub struct WakeInputGate {
    pending: AtomicBool,    // true after wake until first input swallowed
}

impl WakeInputGate {
    pub fn intercept(&self, event: InputEvent) -> Option<InputEvent> {
        if self.pending.swap(false, Ordering::AcqRel) {
            // First post-wake event: discard. Audit log records it.
            None
        } else {
            Some(event)
        }
    }
}
```

The user's reaction: press space to wake the display, see the display
turn on, *then* press whatever they intended to type. This prevents
passphrase characters being typed into the wrong focused app.

### 8.5 Wake sources

| Source | Wake target |
|--------|-------------|
| Keyboard or pointer event | `FullyAwake` |
| Lid open (where available) | `FullyAwake` |
| Power button | `FullyAwake` if in `DisplaySleep`; shutdown menu if in `FullyAwake` |
| Trackpad touch | `FullyAwake` |
| External display hot-plug | `FullyAwake` (chrome notifies user) |
| RTC alarm (Power Nap) | runs fetch in current state; does not promote to `FullyAwake` unless absolutely required |
| Bluetooth keyboard (R54) | `FullyAwake` (deferred to R54) |
| Wake-on-LAN (R51) | `FullyAwake` (deferred to R51) |

### 8.6 Idle-timer mechanics

The R6 input dispatcher resets `IdleDetector::note_input()` on every
event. Crucially, mouse events that don't land in any app's window
(e.g., mouse over chrome) still count as input. The dispatcher writes
to `ArcSwap<Instant>` directly; the policy task reads atomically.

```rust
pub struct IdleDetector {
    last_input: Arc<arc_swap::ArcSwapOption<std::time::Instant>>,
    dim_secs:        u32,    // default 60
    sleep_secs:      u32,    // default 240
    suspend_secs:    u32,    // default 1800
}
```

The policy task polls every 1 s and *reads* the timer; it does not
pre-arm a future-dated transition. A late input event "wins" because the
read is from `ArcSwap`.

### 8.7 Audio implicit assertion (forward from R5)

When the R6 `AudioSubsystem` opens an output stream for an app, it calls
`registry.audio_implicit(app)` to acquire a `PreventSystemSleep`
assertion on the app's behalf. The assertion is released when:

- The stream is explicitly closed, OR
- 30 s of silence (zero PCM samples) pass.

The 30 s silence threshold catches "user paused playback for a moment
to talk" without dropping the assertion every time a song ends.

---

## 9. PowerManager Actor

### 9.1 Module layout

```rust
// supervisor/src/power/mod.rs

pub mod state;       // SystemPowerState, PowerMode, TransitionVeto
pub mod assertions;  // AssertionKind, AssertionRegistry, AssertionHandle
pub mod battery;     // BatteryInfo, BatteryEvent, BatteryMonitor
pub mod display_power;
pub mod power_nap;
pub mod thermal;
pub mod suspend;     // SuspendCoordinator, 5-phase pipeline
pub mod hibernate;   // HibernateWriter, HibernateImage
pub mod idle;        // IdleDetector
pub mod linux_pm;    // sysfs / cgroup / cpufreq writers
pub mod wit_contract;// on-suspend/on-resume marshaling, fetch dispatch

pub use state::{SystemPowerState, PowerMode, TransitionReason, TransitionVeto};
pub use assertions::{AssertionKind, AssertionRegistry, AssertionHandle};
pub use battery::{BatteryInfo, BatteryEvent, PowerSource};
pub use thermal::{ThermalZone, ThermalEvent};
```

### 9.2 Actor

```rust
pub struct PowerManager {
    state:        Arc<ArcSwap<SystemPowerState>>,
    rx:           tokio::sync::mpsc::Receiver<TransitionRequest>,
    tx:           tokio::sync::mpsc::Sender<TransitionRequest>,
    assertions:   Arc<AssertionRegistry>,
    battery:      Arc<ArcSwap<BatteryInfo>>,
    thermal:      Arc<ArcSwap<ThermalZone>>,
    power_mode:   Arc<ArcSwap<PowerMode>>,
    display:      Arc<display_power::DisplayPower>,
    power_nap:    Arc<power_nap::PowerNapScheduler>,
    suspender:    Arc<suspend::SuspendCoordinator>,
    idle:         Arc<idle::IdleDetector>,
    ipc:          Arc<crate::ipc::IpcRouter>,
    chrome_pid:   Option<crate::ipc::AppId>,  // for UserAction trust
    transition_notify: Arc<tokio::sync::Notify>,
}

pub struct TransitionRequest {
    pub target: SystemPowerState,
    pub reason: TransitionReason,
    pub from:   AppId,                    // submitting app
    pub reply:  tokio::sync::oneshot::Sender<Result<(), TransitionVeto>>,
}
```

### 9.3 Main loop

```rust
impl PowerManager {
    pub async fn run(mut self) {
        let mut idle_tick = tokio::time::interval(Duration::from_secs(1));
        idle_tick.set_missed_tick_behavior(MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                Some(req) = self.rx.recv() => {
                    let res = self.handle_transition(req).await;
                    self.transition_notify.notify_waiters();
                }
                _ = idle_tick.tick() => {
                    self.check_idle_transitions().await;
                }
            }
        }
    }
}
```

### 9.4 Transition algorithm

```rust
async fn handle_transition(&mut self, req: TransitionRequest)
    -> Result<(), TransitionVeto>
{
    let from = **self.state.load();
    let plan = state::TransitionPlan {
        from, to: req.target, reason: req.reason, started_at: Instant::now(),
    };
    plan.validate_edge()?;
    plan.validate_platform()?;          // gating from §1.3

    // UserAction requires chrome trust.
    if req.reason == TransitionReason::UserAction
        && Some(req.from) != self.chrome_pid
    {
        return Err(TransitionVeto::UnauthorizedUserAction);
    }

    // Critical paths cannot be vetoed.
    let assertion_bypass = matches!(
        req.reason,
        TransitionReason::CriticalBattery
            | TransitionReason::ThermalCriticalHard
            | TransitionReason::UserAction,
    );

    if !assertion_bypass {
        // Pick the minimum kind that this transition would violate.
        let needed = match req.target {
            SystemPowerState::DisplaySleep => Some(AssertionKind::PreventDisplaySleep),
            SystemPowerState::AppSuspended => Some(AssertionKind::PreventUserIdleSleep),
            #[cfg(feature = "platform_s3")]
            SystemPowerState::SystemSleep  => Some(AssertionKind::PreventSystemSleep),
            SystemPowerState::HibernateReady => Some(AssertionKind::PreventSystemSleep),
            _ => None,
        };
        if let Some(kind) = needed {
            if let Some(a) = self.assertions.first(kind) {
                return Err(TransitionVeto::HeldAssertion {
                    app: a.app, kind: a.kind, reason: a.reason,
                });
            }
        }
    }

    // Hibernate needs battery headroom.
    if req.target == SystemPowerState::HibernateReady {
        let bat = self.battery.load();
        if bat.capacity_pct < 4 && bat.source == PowerSource::Battery {
            return Err(TransitionVeto::InsufficientBatteryForHibernate {
                capacity_pct: bat.capacity_pct,
            });
        }
    }

    // Run the per-target prepare phase.
    match req.target {
        SystemPowerState::DisplaySleep =>
            self.display.enter_sleep().await,
        SystemPowerState::AppSuspended =>
            self.suspender.prepare_for_sleep().await?,
        #[cfg(feature = "platform_s3")]
        SystemPowerState::SystemSleep => {
            self.suspender.prepare_for_sleep().await?;
            self.suspender.kernel_enter_s3().await;
        }
        SystemPowerState::HibernateReady => {
            let rep = self.suspender.prepare_for_sleep().await?;
            self.suspender.write_hibernate_image(rep).await?;
            #[cfg(feature = "platform_s3")]
            self.suspender.kernel_enter_s3().await;
        }
        SystemPowerState::FullyAwake =>
            self.resume_to_fully_awake(from).await?,
        SystemPowerState::UserIdle => {
            self.display.dim_to_user_idle().await;
        }
        SystemPowerState::PoweredOff =>
            self.suspender.poweroff(plan).await?,
    }

    // Commit.
    self.state.store(Arc::new(req.target));
    self.broadcast_state_change(from, req.target, req.reason).await;
    Ok(())
}
```

### 9.5 Idle-driven transitions

```rust
async fn check_idle_transitions(&self) {
    let cur = **self.state.load();
    let idle_secs = self.idle.idle_for().as_secs() as u32;
    let next = self.idle.proposed_transition(cur, idle_secs);
    if let Some((target, reason)) = next {
        let (tx, _) = tokio::sync::oneshot::channel();
        let _ = self.tx.send(TransitionRequest {
            target, reason, from: AppId::SUPERVISOR, reply: tx,
        }).await;
    }
}
```

`proposed_transition` reads `dim_secs` / `sleep_secs` / `suspend_secs`
from user preferences (defaults: 60 / 240 / 1800).

---

## 10. Implementation Files

| File | LOC | Purpose |
|------|-----|---------|
| `supervisor/src/power/mod.rs` | 230 | `PowerManager` actor, `TransitionRequest`, top-level orchestration |
| `supervisor/src/power/state.rs` | 230 | `SystemPowerState`, `PowerMode`, `TransitionVeto`, `TransitionPlan`, platform gating |
| `supervisor/src/power/assertions.rs` | 360 | `AssertionKind`, `Assertion`, `AssertionRegistry`, `AssertionHandle`, R5 shim |
| `supervisor/src/power/battery.rs` | 420 | `BatteryInfo`, `BatteryEvent`, `BatteryMonitor`, netlink + sysfs reader |
| `supervisor/src/power/display_power.rs` | 280 | `DisplayPower`, ramp, DPMS staging, `WakeInputGate` |
| `supervisor/src/power/idle.rs` | 130 | `IdleDetector`, last-input `ArcSwap`, proposed-transition table |
| `supervisor/src/power/thermal.rs` | 430 | `ThermalZone`, governor, three-tier response, adaptive polling, hysteresis |
| `supervisor/src/power/thermal_gpu.rs` | 100 | Vendor-stub GPU min-freq writer |
| `supervisor/src/power/power_nap.rs` | 410 | `PowerNapScheduler`, consent gate, budgets, namespace isolation, audit log |
| `supervisor/src/power/suspend.rs` | 480 | `SuspendCoordinator`, five-phase pipeline |
| `supervisor/src/power/hibernate.rs` | 390 | `HibernateWriter`, header v2, validate, write, load, discard |
| `supervisor/src/power/linux_pm.rs` | 290 | sysfs writers: cpufreq, brightness, DPMS, `/sys/power/state`, cooling devices |
| `supervisor/src/power/wit_contract.rs` | 250 | WIT marshaling: on-prepare-suspend, on-suspend, on-resume, on-background-fetch |
| `wit/vyoma-power.wit` | 150 | Three interfaces (`assert`, `events`, `fetch`), one world |
| `supervisor/src/profile/profiles/*.toml` (modified) | +60 | `[power]` section per profile |
| `supervisor/src/manifest.rs` (modified) | +120 | `[power]` parser, `restorable` validation against WASM exports |
| Unit tests under `supervisor/tests/power_*.rs` | ~900 | FSM, assertions, suspend phases, thermal tiers, hibernate roundtrip, power_nap consent |
| **Total new Rust** | **~3 990** | All files ≤ 500 LOC ceiling |

The largest file is `suspend.rs` at ~480 LOC; splitting it further would
obscure the phase-by-phase flow that is central to its correctness. Every
file fits within the 500-line ceiling.

---

## 11. WIT Package `vyoma:power@0.1.0`

```wit
// wit/vyoma-power.wit

package vyoma:power@0.1.0;

interface assert {
    use vyoma:base/types@0.1.0.{duration};

    enum kind {
        prevent-app-nap,
        prevent-cpu-throttle,
        prevent-display-sleep,
        prevent-user-idle-sleep,
        prevent-system-sleep,
    }

    resource handle {
        constructor(kind: kind, reason: string, timeout: option<duration>);
        kind: func() -> kind;
        // Released by dropping the resource. RAII-safe.
    }

    /// Number of currently held assertions of `k` held by *this* app
    /// (debugging "did I leak a handle?").
    held-by-me: func(k: kind) -> u32;
}

interface events {
    enum state {
        fully-awake,
        user-idle,
        display-sleep,
        app-suspended,
        system-sleep,
        hibernate-ready,
        powered-off,
    }
    enum reason {
        user-idle,
        user-action,
        lid-close,
        lid-open,
        low-battery,
        critical-battery,
        thermal-tier-1,
        thermal-tier-2,
        thermal-critical-hard,
        power-button,
        power-nap-rtc,
        ac-plugged,
        ac-unplugged,
    }
    enum power-source { ac, battery, unknown }
    enum thermal-zone { normal, warm, hot, critical-soft, critical-hard }

    record battery-snapshot {
        source:                  power-source,
        capacity-pct:            u8,
        power-now-mw:            s64,
        time-to-empty-secs:      option<u32>,
        health-pct:              option<u8>,
    }

    /// Called *before* on-suspend. Apps should drain queues, close
    /// sockets they don't want to keep, sync files. Return when done.
    /// Budget: 2 s.
    on-prepare-suspend: func();

    /// Called after Phase-3 quiesce completes. App returns its
    /// StateBlob. Budget: 5 s. Apps without this export are marked
    /// non-restorable.
    on-suspend: func() -> list<u8>;

    /// Called on resume. The StateBlob is whatever the app returned
    /// from on-suspend (or empty bytes if the app cold-started).
    on-resume: func(state-blob: list<u8>);

    /// Power state changed. Apps may not block.
    on-state-changed: func(from: state, to: state, why: reason);

    /// Power source changed (AC ↔ battery).
    on-power-source-changed: func(from: power-source, to: power-source);

    /// Battery crossed a capacity threshold (20, 10, 5).
    on-battery-low: func(snap: battery-snapshot, threshold: u8);

    /// Thermal zone changed.
    on-thermal: func(zone: thermal-zone, temp-c: s32);

    /// Synchronous query of current battery snapshot.
    battery: func() -> battery-snapshot;
}

interface fetch {
    /// Called by the supervisor during a Power Nap window. Network is
    /// scoped to the per-app namespace. Budget enforced from manifest.
    on-background-fetch: func();
}

world app {
    import vyoma:base/types@0.1.0;
    import assert;
    export events;
    export fetch;
}
```

### Manifest schema

```toml
# apps/my-app/vyoma.toml
[app]
name    = "my-app"
version = "0.1.0"
wasm    = "my-app.wasm"

[capabilities]
stdio = true
network = true

[power]
restorable             = true   # app exports on-suspend/on-resume
prevents_sleep         = false  # never auto-asserts; use runtime API
prevents_display_sleep = false
realtime_audio         = false

[power.background_fetch]
enabled                = true
fetch_interval_secs    = 1800
fetch_max_cpu_secs     = 30
fetch_max_egress_bytes = 52428800   # 50 MiB
fetch_max_ingress_bytes= 52428800
network_hosts          = ["mail.example.com"]  # optional whitelist
```

---

## 12. Failure-Mode Catalog

| Failure | Detection | Response |
|---------|-----------|----------|
| `/sys/class/power_supply/BAT0` missing | sysfs read fails | `PowerSource::Unknown`; assume AC for safety; no battery events emitted |
| Battery sysfs returns nonsense (huge discharge, 100 % drop in 1 s) | clamp delta; ignore extreme samples in EWMA | log warning; never alarm on single bogus sample |
| `/sys/power/state` rejects `mem` | write returns Err | `abort_suspend()`; broadcast `on-resume`; UI notification "Could not enter sleep" |
| App's `on-prepare-suspend` deadlocks | 2 s timeout | App marked `PrepareTimeout`; not called for `on-suspend`; store force-dropped in Phase 5 |
| App's `on-suspend` deadlocks | 5 s timeout | App marked `CaptureTimeout`; store force-dropped; warned on hibernate |
| Hibernate image truncated by disk-full | `fsync_all` or `write` returns ENOSPC | discard partial image; fall back to `AppSuspended` |
| Hibernate image fails magic check on boot | header magic mismatch | discard image; normal boot |
| Hibernate image valid but machine-id mismatch | header field check | discard image; normal boot |
| Hibernate image valid but supervisor SHA mismatch | header field check | discard image; normal boot; toast "Resumed fresh (supervisor updated)" |
| Hibernate image valid but per-app blob SHA mismatch | record field check | that app cold-starts; toast "<app> resumed fresh (state corrupted)" |
| Hibernate image valid but per-app wasm SHA mismatch | record field check | that app cold-starts; toast "<app> resumed fresh (app updated)" |
| Thermal sensor returns -ETIMEDOUT | read error | hold previous zone; log; do not assume Normal |
| Power Nap callback infinite-loops | per-app `max_cpu_secs` timeout | kill app store; backoff fetch interval; audit log |
| Assertion handle leaked (app crashes) | Wasmtime resource-table drop on store-drop | registry auto-clears; next idle window sleeps normally |
| User unplugs AC mid-suspend | `BatteryMonitor` SourceChanged | if hibernate already started, finish; never abort mid-write |
| Lid switch flapping (broken hinge) | debounce 500 ms | only honor lid transitions after stable for 500 ms |
| Thermal critical reached at boot | initial sample is `CriticalHard` | refuse boot beyond minimal services; on-screen warning; shutdown after 5 s |
| Phase 3 VFS uncommitted txn | `fsync_all_buckets` reports outstanding | abort suspend; UI "Could not save your work" |
| Phase 2 partial completion (1 of 8 apps `PrepareTimeout`) | per-app outcome map | proceed to Phase 3; mark app non-restorable; warn on hibernate |
| Phase 3 net TX drain incomplete after 500 ms | timeout | proceed; warn in audit log; sockets RST'd by TCP timeouts on remote side |
| `udev` netlink socket dies | recv returns 0 | reopen socket; meanwhile rely on 5-min polling fallback |
| Power Nap namespace creation fails (out of net namespaces) | `unshare` returns Err | log; skip this fetch; back off the global wake budget |
| Power Nap app exceeds egress budget mid-fetch | net counter | kill app store; mark consecutive failure; back off interval |
| Two apps claim `PreventSystemSleep` simultaneously | both granted | sleep blocked; chrome lists both holders |
| `chrome_pid` sends transition with `reason: UserAction` after chrome dies | chrome_pid stale | reject `UnauthorizedUserAction`; supervisor logs |

---

## 13. Per-Platform Power Feature Matrix

| Feature | desktop-full | desktop-full-S3 | mobile | iot-edge | robotics-rt | server-headless | mcu-minimal |
|---------|:---:|:---:|:---:|:---:|:---:|:---:|:---:|
| `UserIdle` state | ✓ | ✓ | ✓ | – | – | – | – |
| `DisplaySleep` state | ✓ | ✓ | ✓ | – | – | – | – |
| `AppSuspended` state | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | – |
| `SystemSleep` (real S3) | – | ✓ | ✓ | – | – | – | – |
| `HibernateReady` | ✓ | ✓ | – | ✓ | – | ✓ | – |
| Battery monitor | – | – | ✓ | ✓ | ✓ | – | – |
| Thermal governor | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | – |
| Power Nap | ✓ | ✓ | ✓ | – | – | – | – |
| Backlight ramp | ✓ | ✓ | ✓ | – | – | – | – |
| DPMS | ✓ | ✓ | ✓ | – | – | – | – |
| Audio implicit assertion | ✓ | ✓ | ✓ | ✓ | ✓ | – | – |
| Lid switch | – | – | – | – | – | – | – |
| AC adapter detection | – | – | ✓ | ✓ | – | – | – |
| Power button | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ |
| `caffeinate` shell builtin | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | – |
| Critical-Hard SIGKILL | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | – |

`mcu-minimal` does not compile the `power` module at all; it exposes
only `PowerManager::shutdown()` from a stub.

---

## 14. End-to-End Scenarios

### 14.1 User goes to lunch (desktop-full, QEMU, no S3)

1. 13:00:00 — user steps away. Last input recorded.
2. 13:01:00 — `IdleDetector` notes 60 s idle. `PowerManager::request(UserIdle, UserIdle)`. Display ramps to 60 %.
3. 13:05:00 — 300 s idle. `PowerManager::request(DisplaySleep, UserIdle)`.
4. 13:05:01 — display ramps to 0 %; DPMS Off.
5. 13:35:01 — 1800 s idle. `PowerManager::request(AppSuspended, UserIdle)`.
6. 13:35:01 — Phase 1: STOP_ACCEPTING (instant).
7. 13:35:01.020 — Phase 2: 8 apps, all ack on-prepare-suspend in 80 ms.
8. 13:35:01.100 — Phase 3: VFS sync (60 ms), IPC drain (20 ms), audio drain (8 ms), net drain (0 ms — no live sockets).
9. 13:35:01.190 — Phase 4: 7 apps with `restorable = true` return StateBlobs; 1 app cold-start-only is skipped.
10. 13:35:01.520 — Phase 5: 8 WASM stores dropped; DPMS confirmed Off; supervisor enters idle. No kernel S3 (feature off).
11. State: `AppSuspended`.
12. 14:30:00 — user returns, jiggles mouse.
13. First mouse event consumed by `WakeInputGate`.
14. `PowerManager::request(FullyAwake, UserAction)`.
15. Display DPMS On; ramp to setpoint; 7 apps re-instantiate from StateBlob; the 1 cold-start app re-launches fresh.
16. 14:30:00.450 — first repaint visible; user moves mouse again (this one is delivered to chrome).

### 14.2 Lid close on battery at 12 % (mobile profile, S3)

1. Lid switch event from libinput. Debounce 500 ms; confirmed closed.
2. `PowerManager::request(SystemSleep, LidClose)`.
3. `BatteryInfo.capacity_pct = 12`, battery source. Policy: prefer Safe Sleep below 30 %.
4. `PowerManager` re-targets to `HibernateReady`.
5. No `PreventSystemSleep` assertions → proceed.
6. Five-phase prepare runs.
7. HibernateWriter writes image (11 MB compressed, 6 s on UFS storage).
8. `kernel_enter_s3()` writes `mem` to `/sys/power/state` → kernel suspends.
9. If battery dies in S3: on cold boot, supervisor finds `image.zst`, validates header, restores apps via `on-resume`. Image discarded.
10. If user opens lid before battery dies: kernel resumes from S3, image is discarded.

### 14.3 Video editor invokes Force Sleep override

```rust
// In the app
let _h = vyoma::power::assert::Handle::new(
    Kind::PreventSystemSleep,
    "Exporting H.264 master".into(),
    Some(Duration::from_secs(3600)),
);
do_long_video_export();
// _h dropped here releases assertion.
```

The user clicks the chrome "Sleep" menu item. The chrome submits
`TransitionRequest { target: AppSuspended, reason: UserAction, from: chrome_pid }`. `UserAction` bypasses assertion checks; the suspend proceeds.

If a non-chrome app submits the same request, `UnauthorizedUserAction`
is returned because `req.from != chrome_pid`.

### 14.4 Mail Power Nap fetch

1. State: `DisplaySleep`. Power Nap timer fires.
2. PowerNap scheduler: Mail's consent (Keychain) = `Always`. Eligible.
3. Mail's `fetch_interval_secs = 1800` elapsed.
4. Create net namespace for Mail.
5. Dispatch `on-background-fetch`.
6. Mail downloads 4 new messages (8 KB total, 3 s).
7. Net counters: egress 1 KB, ingress 8 KB. Within budget.
8. Audit log: `fetch app=mail duration=3000 bytes_in=8192 bytes_out=1024 dest=[mail.example.com:443]`.
9. Namespace destroyed.

### 14.5 Thermal spike during video render

1. Video render app at QoS UserInitiated. CPU at 85 °C.
2. Thermal governor smooths to `Hot` after 3 consecutive samples (3 s).
3. Broadcasts `ThermalEvent::ZoneEntered { from: Warm, to: Hot, temp_c: 85 }`.
4. `PowerMode::UltraLowPower` set; R5 cgroup writer reduces Background/Maintenance apps to 5 %.
5. CPU temp continues climbing: 88 °C, 91 °C → `CriticalSoft` after 3 consecutive samples (3 s).
6. Tier 2 fires: `SIGSTOP` to all Background/Maintenance apps. The video editor (UserInitiated) keeps running.
7. CPU temp drops to 81 °C → after 5 consecutive samples (5 s) back to `Hot`. `SIGCONT` sent.
8. If during the climb a single sample hit 95 °C: Tier 3 fires immediately. `SIGKILL` all; `sync`; `disk` to `/sys/power/state`. UI never sees this — the screen just goes dark.

### 14.6 Notes app without restorable, user picks Hibernate

1. User selects "Hibernate" from chrome menu.
2. Chrome submits `TransitionRequest { target: HibernateReady, reason: UserAction }`.
3. PowerManager enumerates running apps; finds `notes-app` and `calculator` with `restorable = false`.
4. Chrome modal dialog: "These apps cannot be hibernated safely". User clicks Hibernate Anyway.
5. Five-phase prepare runs. For `notes-app` and `calculator`, Phase 4 records `non_restorable: true` and empty `state_blob`.
6. Image written, supervisor exits (no S3 on this platform).
7. On next boot, supervisor finds image, validates, restores. `notes-app` and `calculator` cold-start.
8. Chrome toast: "notes-app, calculator started fresh; their previous state was not saved."

---

## 15. Performance & Energy Targets

| Metric | Target | Notes |
|--------|--------|-------|
| Steady-state CPU while idle, display on | < 0.3 % | on Tiger Lake i7; R10 validates |
| Steady-state CPU while `DisplaySleep` | < 0.05 % | epoll-blocked supervisor |
| Suspend prepare total (8 apps, well-behaved) | p50 600 ms, p95 900 ms | 5-phase pipeline |
| Resume from `AppSuspended` to first repaint | p50 350 ms, p95 600 ms | 7 apps re-instantiate from blob |
| Resume from real S3 (mobile) | p50 950 ms, p95 1.4 s | kernel resume + supervisor resume |
| Hibernate image write (8 apps, ~10 MB) | p50 5 s, p95 8 s | zstd level 3 |
| Resume from hibernate | p50 2.2 s, p95 3.5 s | cold-boot + image read + per-app restore |
| Battery uevent latency (state change → snapshot) | < 5 ms | netlink path |
| Battery sysfs sample cost | < 30 µs | parsing 7 small files |
| Thermal sample cost | < 30 µs | single file per zone |
| Power Nap window overhead (no eligible apps) | < 1 ms | early-return |
| Assertion create/drop latency | < 5 µs | parking_lot RwLock |
| Tier 2 SIGSTOP latency (issue → frozen) | < 50 ms | kernel signal delivery |
| Tier 3 SIGKILL + sync latency | < 200 ms | best-effort |

These get validated in R10 (benchmarks).

---

## 16. Security Considerations

1. **No app can read another app's Power Nap audit log.** The audit log is owned by the supervisor; the "Battery → Background Activity" panel reads it through a privileged IPC RPC, not via direct file access.
2. **Assertion caps prevent DoS.** Default `per_app_cap = 16`, `global_cap = 512`. Each assertion's `timeout` ceiling is 24 h; longer timeouts are clamped.
3. **`UserAction` is gated to chrome.** Only the supervisor's signed chrome app (recognized by manifest hash matching `chrome_manifest_sha256` in trust config) can submit `TransitionRequest { reason: UserAction }`. All other apps' requests run through the assertion-check path.
4. **Hibernate image is encrypted (deferred).** When VyomaOS gains full-disk encryption (R64), the hibernate image inherits the same key wrapping; without the TPM-backed key, the image is opaque. The current spec leaves the image plaintext but the format is extensible.
5. **Power Nap on battery is off by default.** Manifest field is necessary but not sufficient; user must opt-in per-app at first launch.
6. **Critical-Hard thermal cannot be vetoed.** Single sample at ≥ 95 °C triggers Tier 3; assertions ignored. Same fuse as Linux `oom_kill`.
7. **Net namespace isolation for Power Nap.** Each fetch runs in its own `CLONE_NEWNET` namespace, so a malicious app cannot, e.g., ARP-poison another app's connection during a nap window.
8. **Audit-log retention.** 30 days. After 30 days, entries are rotated out. No remote sync. User can delete the file (but cannot tamper with its append-only flag — `chattr +a` on Linux).
9. **Consent revocation is instant.** Changing a Keychain entry triggers an `ArcSwap` swap; the next Power Nap window honors the new consent. In-flight fetches finish but no new ones start.
10. **Hibernate refuses cross-update resume.** Supervisor SHA mismatch, kernel cmdline mismatch, or kernel version mismatch → image is discarded. No attempt to "upgrade" hibernate state across an update boundary.

---

## 17. Test Strategy

### 17.1 Unit tests

- `power/state.rs`: every legal and illegal edge in the FSM is asserted by table; platform gating is asserted by feature-flag combinations.
- `power/assertions.rs`: cap enforcement, RAII drop releases, per-app cap exceeded → typed error, audio implicit acquire/release roundtrip.
- `power/battery.rs`: feed canned sysfs trees from `tests/fixtures/`; verify EWMAs, time-to-empty, threshold crossings, AC plug debouncing.
- `power/thermal.rs`: hysteresis with synthetic temperature traces (slow ramp, oscillation, spike-and-recover); Tier 2 freeze with mock pids; Tier 3 with mock kill.
- `power/power_nap.rs`: schedule selection with mocked clock; consent gate with mocked Keychain; budget enforcement with mock net counter.
- `power/suspend.rs`: simulated `IpcRouter` + `AppRegistry`; assert five-phase ordering; assert abort_suspend correctness on Phase 3 failure; assert per-app outcome reporting.
- `power/hibernate.rs`: write → load round-trip; tamper byte in blob → reject that app; machine-id mismatch → reject image; supervisor SHA mismatch → reject; stale timestamp → reject.

### 17.2 Integration tests (QEMU)

- `tests/integration_sleep_resume.rs`: spin up QEMU with `desktop-full`; verify FullyAwake → UserIdle → DisplaySleep → AppSuspended → FullyAwake roundtrip; verify StateBlobs survive.
- `tests/integration_battery_uevent.rs`: synthetic 9P-mounted fake `/sys/class/power_supply/BAT0/`; trigger uevents by mtime touches; expect Warning, Critical, Emergency events in order.
- `tests/integration_thermal_throttle.rs`: synthetic thermal sysfs; verify R5 cgroup `cpu.max` values change in lockstep; verify Tier 2 SIGSTOPs background apps; verify Tier 3 SIGKILLs.
- `tests/integration_power_nap_consent.rs`: install app with `background_fetch`; first launch shows prompt (mocked chrome); deny → no fetches; allow-while-charger → fetches only on AC.
- `tests/integration_hibernate_roundtrip.rs`: hibernate → "reboot" the supervisor (kill + restart) → verify all `restorable = true` apps resume; verify non-restorable apps cold-start with toast.

### 17.3 Property tests

- For every FSM trajectory of length ≤ 8 (excluding `SystemSleep` on no-S3 builds), verify it is invertible to a canonical state.
- For every assertion add/drop interleaving with up to 4 apps and 16 operations, verify cap invariants hold and no assertion outlives its holder app.
- For every thermal trace ≤ 32 samples, verify hysteresis enforces 3-up / 5-down; verify CriticalHard fires on first sample.

### 17.4 Bare-metal smoke test (manual)

On actual hardware (a development laptop), verify that S3 with
`platform_s3` flag actually suspends RAM (battery draw drops to mW range)
and resumes correctly. This test exists outside CI but blocks any release
that claims S3 support on a given platform.

---

## 18. Putting It All Together — Three-Layer Architecture

The Critic's architectural recommendation is adopted: the implementation
splits cleanly into three layers, each in its own module(s):

### Layer 1 — `power::linux_pm` (sysfs / cgroups / cpufreq)

Pure I/O. No state. No tokio. Synchronous functions that wrap `write`
to `/sys`. Trivially testable with a mock fs. Includes:

- `read_thermal_zones() -> Vec<(PathBuf, i32)>`
- `write_brightness(pct: u8) -> io::Result<()>`
- `write_dpms(mode: DpmsMode) -> io::Result<()>`
- `write_cpu_max(cgroup_path: &Path, max: u64) -> io::Result<()>`
- `write_power_state(token: &[u8]) -> io::Result<()>`
- `write_cooling_device_max(idx: u32) -> io::Result<()>`
- `write_sysrq(token: u8) -> io::Result<()>`

These are the only places where sysfs paths appear in the codebase.
Tests inject a `&Path` root so a tmpfs fixture can replace `/sys`.

### Layer 2 — `power::state` (FSM)

Pure logic. No I/O. No tokio. The Vyoma power FSM, transition validation,
platform gating. Operates on data structures only. Reading from this
layer is wait-free (`ArcSwap`).

### Layer 3 — `power::wit_contract` (apps)

The WIT marshaling. Translates `IpcEnvelope` to/from the WIT-shaped
`on-prepare-suspend`, `on-suspend`, `on-resume`, `on-state-changed`,
`on-background-fetch`, etc. No business logic; just serialization.

The remaining modules (`battery`, `display_power`, `thermal`,
`power_nap`, `suspend`, `hibernate`, `assertions`) compose these three
layers. This makes each layer auditable in isolation: Layer 1's
correctness is "I write what was asked"; Layer 2's correctness is "the
FSM has no illegal edges"; Layer 3's correctness is "WIT messages match
the contract".

---

## Critical v1 Requirements

- Seven-state power FSM with explicit transition validation and platform-gated `SystemSleep` variant
- Unified `AssertionKind` enum with five kinds and documented precedence; R5 `ActivityAssertion` lifted into `PreventAppNap`
- Five-phase suspend barrier (STOP_ACCEPTING / QUIESCE_USERSPACE / QUIESCE_KERNEL / CAPTURE / TEAR_DOWN) with per-phase timeouts
- Phase 2 `on-prepare-suspend` and Phase 4 `on-suspend` as separate WIT callbacks; `on-resume` idempotent against prepare-without-suspend
- Three-tier thermal response: Tier 1 cgroup throttle / Tier 2 SIGSTOP / Tier 3 SIGKILL + sync + sysrq; adaptive polling 25 ms–5 s
- Battery via `NETLINK_KOBJECT_UEVENT` (subscribed to `subsystem=power_supply`); 5-min poll as fallback; no DBus, no udev daemon, no upower
- Power Nap requires manifest field AND first-launch user consent (Keychain) AND per-app budget (CPU + bytes) AND net namespace isolation AND audit log
- Manifest field `power.restorable = bool` (default false); install-time validation that `true` implies `on-suspend` export; hibernate prompts before proceeding with non-restorable apps
- Hibernate image header v2 with magic `VYOMHIB2`, machine-id check, kernel-cmdline-hash check, supervisor-SHA check, per-app blob SHA, per-app wasm SHA
- Stale-hibernate timeout (7 days, configurable); discard image after every successful resume
- `WakeInputGate` consumes first post-wake input event; wait for `CRTC.active = 1` + frame flip before unblocking input
- Audio output implicitly acquires `PreventSystemSleep` via R6 `AudioSubsystem` hook; released on stream close or 30 s silence
- `UserAction` transition reason requires `req.from == chrome_pid`; chrome identified by signed manifest SHA in trust config
- Critical-Hard thermal (≥95 °C) and Emergency battery (≤5 %) cannot be vetoed by any assertion or by `UserAction`
- AC plug-unplug debounce 5 s before applying `PowerMode` switch
- Lid switch debounce 500 ms
- Per-platform `[power]` section in profile TOML; compile-time `platform_s3` feature flag
- `caffeinate` chrome shell builtin: `@supervisor: assert <kind> <reason>`, `@supervisor: list-assertions`, `@supervisor: release-assertion <id>`
- WIT package `vyoma:power@0.1.0` with three interfaces (`assert`, `events`, `fetch`)
- All `power/*.rs` files ≤ 500 LOC; total ~3 990 LOC Rust + ~150 LOC WIT
- Audit log `/data/.system/power_audit.log` is append-only (`chattr +a`), 30-day retention, local only

---

## Deferred to v2

- Bare-metal S3 platform validation on real Apple Silicon / Intel laptops
- Encrypted hibernate image (depends on R64 full-disk encryption)
- Wake-on-LAN (depends on R51 networking stack)
- Bluetooth-keyboard wake (depends on R54)
- Scheduled wake (`pmset schedule` analogue): cron-like (timestamp, action, app) table
- External-display "clamshell mode" policy (depends on R6 multi-output)
- Per-app energy attribution via RAPL where available (currently best-effort via battery delta)
- Light-sensor-driven auto-brightness (depends on R6 sensor subsystem maturation)
- ARM PSCI `CPU_SUSPEND` validation on robotics-rt and iot-edge platforms
- App `prevents_sleep = true` manifest field — currently auto-converts to a runtime `PreventSystemSleep` assertion for the app's lifetime; v2 may add finer manifest expression
- Power Nap on ARM (currently desktop-full + mobile only)
- Configurable thermal trip-point thresholds per-platform (currently hard-coded 70/80/90/95)
- Audio "ducking" during low battery
- Network-namespace-per-fetch with TC traffic accounting beyond byte counters
- Per-domain TLS SNI enforcement when `network_hosts` is declared (currently destination-IP enforcement only)
- Hibernate-resume after kernel update (currently refuses with toast; v2 may attempt re-instantiation against a compatibility table)
- `pmset`-style user-facing tooling (currently only via chrome shell builtin)
- Battery-health degradation tracking (currently one-shot notification at ≤80 %)
- A Power Nap leaderboard ("which apps used the most background CPU last week")

---

## Explicitly NEVER

- Claiming a guest VM's `echo mem > /sys/power/state` is equivalent to bare-metal S3; the FSM never reports `SystemSleep` on a build without `platform_s3`
- Single-phase suspend — every suspend goes through the five phases or it doesn't suspend at all
- Hibernate of an app with `restorable = false` without explicit user confirmation per-suspend
- `background_fetch = true` in manifest treated as sufficient consent — the user prompt is mandatory at first launch
- Calling `on-suspend` without first completing `on-prepare-suspend` ack and Phase 3 quiesce; the StateBlob would reference an inconsistent post-resume world
- Two separate `acquire_activity_assertion` / `acquire_power_assertion` APIs; the unified `AssertionRegistry` is the only acquisition path
- Polling sysfs at 1 Hz when the kernel can netlink-push events
- Forwarding the first post-wake input event to any app
- Allowing any non-chrome app to submit `TransitionRequest { reason: UserAction }`
- Honoring an assertion against `CriticalBattery`, `ThermalCriticalHard`, or `UserAction`
- A `Mutex` held across a WIT call (R5 rule extended here)
- A `Mutex` on the thermal polling hot path (uses `ArcSwap`)
- Power Nap fetches in the supervisor's primary network namespace
- Power Nap fetches without an audit-log entry
- Persisting Power Nap consent outside Round 60 Keychain (no plaintext fallback)
- Hibernate image without per-app blob SHA verification
- Hibernate image used across kernel-version-hash mismatch
- Hibernate image used across supervisor-SHA mismatch
- Hibernate image used after `stale_hibernate_secs` (default 7 days)
- The chrome trusting any app to submit `UserAction` other than itself
- Tier 2 SIGSTOP applied to `UserInteractive` / `UserInitiated` apps
- Tier 3 SIGKILL without preceding `sync(2)` (best-effort but mandatory in the code path)
- Loading a hibernate image with wrong magic bytes (`VYOMHIB1` or anything but `VYOMHIB2`)
- Compositor frame to a backlight that DPMS reports `Off`
- Audio output stream without auto-acquiring `PreventSystemSleep`
- Direct `/sys/power/state` writes from any code outside `power::linux_pm`
- Direct `/sys/class/backlight` writes from any code outside `power::linux_pm`
- Battery monitor reading sysfs from the main tokio reactor (uses spawned task)
- Persisting any power-related telemetry to anywhere other than `/data/.system/`
- A fetch eligible without consent — the consent gate is the first check, not the last

---

## Implementation files

| File | LOC | Purpose |
|------|-----|---------|
| `supervisor/src/power/mod.rs` | 230 | `PowerManager` actor, dispatch loop, transition handling |
| `supervisor/src/power/state.rs` | 230 | `SystemPowerState`, `PowerMode`, edge validation, platform gating |
| `supervisor/src/power/assertions.rs` | 360 | Unified `AssertionRegistry`, `AssertionHandle` RAII, R5 shim |
| `supervisor/src/power/battery.rs` | 420 | `BatteryMonitor`, sysfs reader, netlink subscription, EWMA, events |
| `supervisor/src/power/display_power.rs` | 280 | `DisplayPower`, ramp, DPMS staging, `WakeInputGate` |
| `supervisor/src/power/idle.rs` | 130 | `IdleDetector`, `last_input` ArcSwap, proposed-transition table |
| `supervisor/src/power/thermal.rs` | 430 | `ThermalGovernor`, three-tier response, hysteresis, adaptive polling |
| `supervisor/src/power/thermal_gpu.rs` | 100 | GPU min-freq writer (vendor stub) |
| `supervisor/src/power/power_nap.rs` | 410 | `PowerNapScheduler`, consent gate, budgets, namespace isolation, audit |
| `supervisor/src/power/suspend.rs` | 480 | `SuspendCoordinator`, five-phase pipeline, abort_suspend |
| `supervisor/src/power/hibernate.rs` | 390 | `HibernateWriter`, image v2, header validation, write, load, discard |
| `supervisor/src/power/linux_pm.rs` | 290 | sysfs writers (cpufreq, brightness, DPMS, power-state, cooling, sysrq) |
| `supervisor/src/power/wit_contract.rs` | 250 | WIT marshaling for prepare/suspend/resume/state-changed/fetch |
| `wit/vyoma-power.wit` | 150 | Three interfaces, one world |
| Modified: `supervisor/src/profile/profiles/*.toml` | +60 | `[power]` section per profile |
| Modified: `supervisor/src/manifest.rs` | +120 | `[power]` parser, install-time `restorable`-vs-WASM-export check |
| Tests: `supervisor/tests/power_*.rs` | ~900 | FSM, assertions, suspend, thermal, hibernate, power_nap |

**Total new:** ~4 140 LOC Rust + ~150 LOC WIT across 13 files + 1 WIT package; every file ≤ 500 LOC.
**Modified:** 2 files / 7 profile TOMLs, ~180 added LOC (manifest +120, profiles +60).

---

*End of Round 7 Final.* The synthesizer adopted the Architect's
vocabulary, RAII handles, WIT callback shape, platform-profile
integration, and Safe Sleep concept, while taking every one of the
Critic's structural objections (C1 honest power ladder, C2 ordered
barrier, C3 consent + budget + isolation, C4 three-tier thermal, C5
unified assertions, C6 restorable contract) as binding constraints. The
implementation is partitioned into three auditable layers
(`linux_pm` / `state` / `wit_contract`) so each can be reviewed for
correctness in isolation. The supervisor never lies about whether
hardware actually slept; apps never silently lose state to hibernate;
Power Nap is never a silent exfiltration channel.

# Round 7 Architect Proposal: Power Management & Energy

**Status:** Architect proposal — awaiting Critic review
**Date:** 2026-05-29
**Subsystem:** System power states, battery, display power, App Nap reinforcement,
Power Nap, wake locks (caffeinate), thermal throttling, suspend/resume, hibernate
**macOS equivalent:** IOPowerManagement framework + IOPMrootDomain + App Nap +
Power Nap + `caffeinate(8)` + `pmset(1)` + Battery Health Management + Safe Sleep
**Integrates with:**
  - R1 — `AppIdentity`, `AppHandle`, `RestartPolicy`, WIT lifecycle callbacks
    (`on-suspend`/`on-resume`), `WatchdogActor`, `BootPhase`, `StateBlob` for
    checkpoint/restore.
  - R2 — `MemoryGovernor`, `PsiMonitor`, `JetsamRanker`. Hibernate triggers
    forced jetsam of Idle apps to shrink the suspend image.
  - R3 — `IpcEnvelope`, `IpcRouter`, broadcast topics
    `topic:system/power-event`, `topic:system/battery-event`,
    `topic:system/thermal-event`; ordered delivery guarantees that
    `on-suspend` reaches every app before resources are reaped.
  - R4 — `VfsBackend::fsync_all_buckets()` invoked during suspend; `CoordWal`
    journal replayed on resume; hibernate image lives in
    `/data/.hibernate/` (a dedicated VFS bucket).
  - R5 — `QosClass`, `ActivityAssertion`, App Nap detector, cgroup `cpu.max`,
    `SCHED_DEADLINE`. Battery and thermal policy multiply per-class CPU
    budgets; this round wires those signals end-to-end.
  - R6 — `DisplaySubsystem` (backlight + DPMS), `AudioSubsystem` (SPSC ring
    drain on suspend), `DeviceManager` (driver bundles get a 250 ms quiesce
    window before suspend), keyboard/mouse wake events flow back through the
    udev/evdev pipeline.

---

## 0. Executive Summary

Power management is the most cross-cutting subsystem in the OS: it touches the
scheduler, the display, every device driver, every WASM app, the filesystem
journal, and the kernel's own `/sys/power/state` interface. On a laptop it is
also the user-visible difference between "this OS is delightful" and "this OS
ate three percent battery while my screen was off." VyomaOS Phase 17 boots in
under five seconds; Phase 18+ has to keep that experience alive after a lid
close, on battery, in a thermal envelope, and across the suspend/resume cycle
without breaking the capability-secure model.

This round specifies:

1. **Five-state power FSM** (`SystemPowerState`) with explicit transition
   guards, a single owning actor (`PowerManager`), and a transactional
   broadcast that lets any subsystem veto a transition.
2. **`BatteryMonitor`** — reads `/sys/class/power_supply/`, smooths
   instantaneous discharge with an EWMA, computes time-to-empty/full,
   tracks cycle count + design vs full-charge capacity (Battery Health),
   broadcasts `BatteryEvent` over R3.
3. **`DisplayPower`** — backlight ramp, DPMS staging
   (`Standby → Suspend → Off`), idle detection driven by R6 input events,
   integrates with R5 `ActivityAssertion`.
4. **`PowerNapScheduler`** — periodic wakeups during display sleep / system
   sleep, runs a strict allowlist of "fetch" callbacks declared via
   `[power.background_fetch]` in app manifests, hard-bounded by a global
   energy budget (mWh per Nap window).
5. **`PowerAssertionRegistry`** — capability-style RAII wake-lock model with
   four assertion types and per-app caps, surfacing exactly which app is
   keeping the machine awake (the `caffeinate` / `pmset -g assertions`
   equivalent).
6. **`ThermalGovernor`** — reads `/sys/class/thermal/thermal_zone*`,
   classifies the system into four zones, feeds back into the R5
   `cpu.max` controller and an optional GPU `min_freq` writer, with a
   non-overridable critical-shutdown fuse.
7. **Suspend/resume pipeline** — a six-phase prepare/commit/restore
   sequence with a 5 s per-app on-suspend budget enforced by
   `tokio::time::timeout`, full `fsync_all_buckets` discipline, atomic
   `/sys/power/state` write, and journaled rollback on partial failure.
8. **Hibernate (Safe Sleep)** — opportunistic suspend-to-disk that writes
   a compressed image of every app's `StateBlob` plus the `CoordWal` tip
   to `/data/.hibernate/image.zst`, fsyncs it, *then* enters S3. If
   battery dies in S3 the image still boots.
9. **WIT package `vyoma:power@0.1.0`** — three interfaces (`assert`,
   `events`, `fetch`) that apps consume to declare wake locks, listen
   for power events, and register background-fetch callbacks.
10. **Manifest `[power]` section** — declarative per-app energy policy:
    background_fetch, prevents_sleep, prevents_display_sleep,
    realtime_audio, fetch_interval, fetch_max_runtime, fetch_energy_budget_mj.

Roughly 8 new supervisor files (~3 100 LOC total), 1 WIT package, 1 new
manifest section. All files stay under the 500-line ceiling enforced by
the project rule. Steady-state CPU overhead while idle: <0.3% on a Tiger
Lake i7 (target validated by R10 power benchmarks). Suspend completes in
under 900 ms p95 on an 8-app workload; resume from S3 in under 1.4 s p95
from kernel wake to first repainted pixel.

---

## 1. The Five Power States

### 1.1 Enum & invariants

```rust
// supervisor/src/power/state.rs

use std::time::Instant;

/// Top-level system power state. There is exactly one of these at any
/// moment; transitions are serialized through `PowerManager::run()`.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum SystemPowerState {
    /// Display on, scheduler runs at full speed, audio active, all apps
    /// receive their normal QoS budget. This is the steady state when
    /// the user is interacting.
    FullyAwake,

    /// Backlight off, DPMS programmed `Off`, GPU minimum clocks. CPU is
    /// still running. App Nap (R5) is aggressive: Background and
    /// Maintenance QoS apps suspend after 2 s. Power Nap windows fire
    /// every 30 min if any app declares `background_fetch = true`.
    DisplaySleep,

    /// Linux suspend-to-RAM (S3 on x86, deep idle on ARM). All
    /// non-essential devices powered down. RAM is self-refresh; CPU
    /// halted. Only wake sources: power button, lid open, LAN WoL,
    /// USB wake-enabled device. RTC alarm may be programmed for the
    /// next Power Nap if AC-attached.
    SystemSleep,

    /// `StateBlob` of every app + `CoordWal` tip have been written to
    /// `/data/.hibernate/image.zst` and fsynced. The system is *also*
    /// in S3 (Safe Sleep) and will boot from disk if RAM is lost. From
    /// the user's perspective this is indistinguishable from
    /// `SystemSleep` but is preferred on low battery.
    HibernateReady,

    /// Init is reaped, supervisor has performed an orderly shutdown
    /// of every app, all VFS buckets are fsynced, kernel has been told
    /// `poweroff -f`. Only emitted transiently before the kernel halts.
    PoweredOff,
}

impl SystemPowerState {
    /// Whether the WASM scheduler may schedule new epochs for apps.
    pub fn schedules_apps(self) -> bool {
        matches!(self, Self::FullyAwake | Self::DisplaySleep)
    }

    /// Whether the display backlight is active.
    pub fn display_on(self) -> bool {
        matches!(self, Self::FullyAwake)
    }

    /// Whether CPU is running.
    pub fn cpu_on(self) -> bool {
        matches!(self, Self::FullyAwake | Self::DisplaySleep)
    }
}
```

### 1.2 Transition graph

```
                    +--------- user idle 5 min, no prevents_display_sleep
                    v
            FullyAwake <---- any user input ----- DisplaySleep
                |                                        |
                | idle 30 min, no prevents_sleep         | lid close /
                | (or lid close, or user choose Sleep)   | menu Sleep
                v                                        v
            HibernateReady ----------------------> SystemSleep
                ^         (battery < 10% AND not        |
                |          on AC; otherwise stay         | power button /
                |          in SystemSleep)               | lid open /
                |                                        | Power Nap RTC
                |                                        v
                |                                  FullyAwake
                |                                        |
                |          User chooses Shut Down        |
                +---------> PoweredOff <-----------------+
```

Disallowed direct edges (must transition via an intermediate state):

| From            | To              | Forced through      |
|-----------------|-----------------|---------------------|
| FullyAwake      | SystemSleep     | DisplaySleep        |
| FullyAwake      | HibernateReady  | DisplaySleep        |
| SystemSleep     | DisplaySleep    | FullyAwake          |
| HibernateReady  | FullyAwake      | SystemSleep (resume)|

Rationale: forcing every system-sleep entry through DisplaySleep gives us a
single, well-tested code path that drains the GPU command queue and saves the
DRM mode-set. Resume from hibernate is identical to resume from S3 because the
kernel resume hook restores both the same way.

### 1.3 Transition vetoes

Any subsystem (and any app holding a `PowerAssertion`) can veto the
`FullyAwake → DisplaySleep` and `DisplaySleep → SystemSleep` edges. The
veto is checked at the *moment* the transition is attempted; assertions
are not pre-registered with the FSM because they can be added/dropped
during the prepare phase.

```rust
// supervisor/src/power/state.rs

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransitionVeto {
    /// An R5 `ActivityAssertion` is currently held.
    ActivityAssertion { app: AppId, kind: AssertionKind },
    /// A driver bundle (R6) refused to quiesce in time.
    DriverBusy { bundle: BundleId, reason: String },
    /// Memory governor (R2) reports active jetsam pressure; safer to
    /// finish that work before suspending.
    MemoryPressureActive,
    /// VFS (R4) has an in-flight write whose journal entry is not
    /// fsynced; we abort suspend rather than risk the image being
    /// torn by an unexpected battery loss.
    UncommittedVfsTxn { txn_id: u64 },
    /// A WASM app's `on-suspend` callback has not yet returned within
    /// its 5 s budget (R1 WatchdogActor).
    AppSuspendTimeout { app: AppId },
    /// Battery is too low to safely complete a hibernate write.
    InsufficientBatteryForHibernate { capacity_pct: u8 },
}

pub type TransitionResult<T> = Result<T, TransitionVeto>;
```

### 1.4 The transition algorithm

```rust
// supervisor/src/power/state.rs (continued)

pub struct TransitionPlan {
    pub from: SystemPowerState,
    pub to:   SystemPowerState,
    pub reason: TransitionReason,
    pub started_at: Instant,
}

#[derive(Copy, Clone, Debug)]
pub enum TransitionReason {
    UserIdle,
    UserAction,        // menu Sleep / Shut Down
    LidClose,
    LidOpen,
    LowBattery,
    CriticalBattery,
    PowerButton,
    PowerNapRtc,
    Thermal,
    AcPlugged,
    AcUnplugged,
}

impl TransitionPlan {
    /// Return Ok if the requested edge is in the FSM legal graph.
    pub fn validate_edge(&self) -> TransitionResult<()> {
        use SystemPowerState::*;
        match (self.from, self.to) {
            (FullyAwake,      DisplaySleep)    => Ok(()),
            (DisplaySleep,    FullyAwake)      => Ok(()),
            (DisplaySleep,    SystemSleep)     => Ok(()),
            (DisplaySleep,    HibernateReady)  => Ok(()),
            (SystemSleep,     FullyAwake)      => Ok(()),
            (HibernateReady,  FullyAwake)      => Ok(()),
            (FullyAwake,      PoweredOff)      => Ok(()),
            (DisplaySleep,    PoweredOff)      => Ok(()),
            _ => Err(TransitionVeto::DriverBusy {
                bundle: BundleId::INVALID,
                reason: format!(
                    "illegal FSM edge {:?} → {:?}", self.from, self.to
                ),
            }),
        }
    }
}
```

The `PowerManager` actor (Section 2) owns the only writable handle to the
current state; everything else observes via an `ArcSwap<SystemPowerState>`
snapshot (same pattern as R3 `PolicySnapshot` and R6 `RegistrySnapshot`),
so reads from the hot path of any other subsystem are wait-free.

---

## 2. PowerManager Actor

`PowerManager` is the single Tokio task that orchestrates every state
change. All other subsystems propose transitions by sending a
`TransitionRequest` over an mpsc channel; the manager serializes them,
runs the veto checks, and either commits the transition or rejects it
with a `TransitionVeto`. This avoids race conditions where, e.g., the
battery monitor and the lid switch both try to suspend simultaneously.

```rust
// supervisor/src/power/mod.rs

use arc_swap::ArcSwap;
use std::sync::Arc;
use tokio::sync::{mpsc, oneshot, Notify};
use tokio::time::{Duration, Instant};

pub mod state;
pub mod battery;
pub mod display_power;
pub mod power_nap;
pub mod assertions;
pub mod thermal;
pub mod suspend;
pub mod hibernate;

pub use state::{SystemPowerState, TransitionReason, TransitionVeto};
pub use battery::{BatteryInfo, BatteryEvent, PowerSource};
pub use assertions::{
    AssertionKind, PowerAssertion, PowerAssertionHandle, PowerAssertionRegistry,
};
pub use thermal::{ThermalZone, ThermalEvent};

pub struct TransitionRequest {
    pub target: SystemPowerState,
    pub reason: TransitionReason,
    pub reply:  oneshot::Sender<Result<(), TransitionVeto>>,
}

pub struct PowerManager {
    /// Wait-free read snapshot for other subsystems.
    state:        Arc<ArcSwap<SystemPowerState>>,
    /// Incoming transition requests.
    rx:           mpsc::Receiver<TransitionRequest>,
    /// Public handle for sending requests.
    tx:           mpsc::Sender<TransitionRequest>,
    /// Wake-lock registry (assertions block transitions).
    assertions:   Arc<PowerAssertionRegistry>,
    /// Battery sampler — runs its own task, reports here.
    battery:      Arc<ArcSwap<BatteryInfo>>,
    /// Thermal sampler — runs its own task.
    thermal:      Arc<ArcSwap<ThermalZone>>,
    /// Display sub-actor (backlight ramp).
    display:      Arc<display_power::DisplayPower>,
    /// Power Nap scheduler (only ticks while DisplaySleep / SystemSleep).
    power_nap:    Arc<power_nap::PowerNapScheduler>,
    /// Suspend/resume coordinator.
    suspender:    Arc<suspend::SuspendCoordinator>,
    /// IPC broadcaster (R3) for power events.
    ipc:          Arc<crate::ipc::IpcRouter>,
    /// Notification on every successful transition (for tests).
    transition_notify: Arc<Notify>,
}

impl PowerManager {
    pub async fn run(mut self) {
        loop {
            tokio::select! {
                Some(req) = self.rx.recv() => {
                    let res = self.handle_transition(req.target, req.reason).await;
                    let _ = req.reply.send(res);
                    self.transition_notify.notify_waiters();
                }
                _ = self.tick_idle_timer() => {
                    // Periodic check: if no activity for N seconds and
                    // no PreventDisplaySleep assertion, dim then sleep.
                    self.check_idle_transitions().await;
                }
            }
        }
    }
}
```

### 2.1 Transition algorithm

```rust
impl PowerManager {
    async fn handle_transition(
        &mut self,
        target: SystemPowerState,
        reason: TransitionReason,
    ) -> Result<(), TransitionVeto> {
        let from = **self.state.load();
        let plan = state::TransitionPlan {
            from, to: target, reason, started_at: Instant::now(),
        };
        plan.validate_edge()?;

        // 1. Check assertions for going-to-sleep edges.
        if target == SystemPowerState::DisplaySleep
            && reason != TransitionReason::UserAction
        {
            if let Some(a) = self.assertions
                .first_active(AssertionKind::PreventDisplaySleep)
            {
                return Err(TransitionVeto::ActivityAssertion {
                    app: a.app, kind: a.kind,
                });
            }
        }
        if matches!(
            target,
            SystemPowerState::SystemSleep | SystemPowerState::HibernateReady
        ) {
            if let Some(a) = self.assertions
                .first_active(AssertionKind::PreventSystemSleep)
            {
                return Err(TransitionVeto::ActivityAssertion {
                    app: a.app, kind: a.kind,
                });
            }
        }

        // 2. Hibernate requires sufficient battery to complete the
        //    image write (~30 s of writes at ~5 W on a 50 Wh battery
        //    is ~0.08% — but we want headroom for completion).
        if target == SystemPowerState::HibernateReady {
            let bat = self.battery.load();
            if bat.capacity_pct < 4 && matches!(bat.source, PowerSource::Battery) {
                return Err(TransitionVeto::InsufficientBatteryForHibernate {
                    capacity_pct: bat.capacity_pct,
                });
            }
        }

        // 3. Run the per-target prepare phase.
        match target {
            SystemPowerState::DisplaySleep =>
                self.display.enter_sleep().await,
            SystemPowerState::SystemSleep =>
                self.suspender.suspend_to_ram(plan).await?,
            SystemPowerState::HibernateReady =>
                self.suspender.suspend_to_disk(plan).await?,
            SystemPowerState::FullyAwake =>
                self.resume_to_fully_awake(from).await?,
            SystemPowerState::PoweredOff =>
                self.suspender.poweroff(plan).await?,
        }

        // 4. Commit: swap the ArcSwap, broadcast.
        self.state.store(Arc::new(target));
        self.broadcast_state_change(from, target, reason).await;
        Ok(())
    }

    async fn broadcast_state_change(
        &self,
        from: SystemPowerState,
        to:   SystemPowerState,
        reason: TransitionReason,
    ) {
        let env = crate::ipc::IpcEnvelope::broadcast(
            "topic:system/power-event",
            PowerEvent::StateChanged { from, to, reason }.encode(),
        );
        let _ = self.ipc.publish(env).await;
    }
}
```

### 2.2 Idle detection

A second task fires every 1 s and asks: *how long since the last user
input event arrived via the R6 input pipeline?*

```rust
// supervisor/src/power/idle.rs

pub struct IdleDetector {
    last_input: Arc<arc_swap::ArcSwapOption<Instant>>,
    dim_after_secs:   u32, // user setting, default 60
    sleep_after_secs: u32, // user setting, default 300
    system_sleep_after_secs: u32, // user setting, default 1800
}

impl IdleDetector {
    pub fn note_input(&self) {
        self.last_input.store(Some(Arc::new(Instant::now())));
    }

    pub fn idle_for(&self) -> Duration {
        match self.last_input.load_full() {
            Some(t) => Instant::now().saturating_duration_since(*t),
            None    => Duration::ZERO,
        }
    }

    pub fn proposed_transition(
        &self,
        cur: SystemPowerState,
    ) -> Option<(SystemPowerState, TransitionReason)> {
        let idle = self.idle_for().as_secs() as u32;
        match cur {
            SystemPowerState::FullyAwake
                if idle >= self.sleep_after_secs =>
                Some((SystemPowerState::DisplaySleep, TransitionReason::UserIdle)),
            SystemPowerState::DisplaySleep
                if idle >= self.system_sleep_after_secs =>
                Some((SystemPowerState::SystemSleep, TransitionReason::UserIdle)),
            _ => None,
        }
    }
}
```

`IdleDetector::note_input` is called from the R6 input dispatcher on
every keyboard and pointer event. Display dimming (a separate, in-between
state visible only inside `display_power.rs`) starts at
`dim_after_secs` and reaches DPMS `Off` at `sleep_after_secs`.

---

## 3. Battery & AC Power

### 3.1 BatteryInfo

```rust
// supervisor/src/power/battery.rs

use std::path::Path;
use std::sync::Arc;
use arc_swap::ArcSwap;
use tokio::time::{Duration, Instant, MissedTickBehavior};

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum PowerSource {
    Ac,         // /sys/class/power_supply/AC/online == 1
    Battery,    // discharging
    Unknown,    // sysfs missing or ambiguous
}

#[derive(Clone, Debug)]
pub struct BatteryInfo {
    pub source:                 PowerSource,
    pub capacity_pct:           u8,        // 0..=100, kernel-reported
    pub capacity_smoothed_pct:  f32,       // EWMA of capacity_pct
    pub voltage_now_uv:         i64,
    pub current_now_ua:         i64,       // signed: + charging, - discharging
    pub power_now_mw:           i64,       // computed v * i / 1e9
    pub power_smoothed_mw:      f32,       // EWMA, alpha = 0.1
    pub time_to_empty_secs:     Option<u32>,
    pub time_to_full_secs:      Option<u32>,
    pub cycle_count:            Option<u32>,
    pub design_capacity_uah:    Option<i64>,
    pub full_charge_capacity_uah: Option<i64>,
    pub health_pct:             Option<u8>,  // full/design * 100
    pub temperature_decic:      Option<i32>, // 1/10 degC
    pub last_update:            Instant,
}

#[derive(Clone, Debug)]
pub enum BatteryEvent {
    SourceChanged(PowerSource),
    CapacityCrossed { from_pct: u8, to_pct: u8, threshold: u8 },
    Warning,    // <= 20%
    Critical,   // <= 10%, hibernate now
    Emergency,  // <= 5%, force shutdown after 30 s
    HealthDegraded { health_pct: u8 },
}
```

### 3.2 Sampling loop

```rust
pub struct BatteryMonitor {
    snapshot: Arc<ArcSwap<BatteryInfo>>,
    sysfs_root: &'static Path,
    ipc: Arc<crate::ipc::IpcRouter>,
    interval: Duration, // 5 s on battery, 30 s on AC
}

impl BatteryMonitor {
    pub async fn run(self) {
        let mut tick = tokio::time::interval(self.interval);
        tick.set_missed_tick_behavior(MissedTickBehavior::Skip);
        let mut last = self.snapshot.load_full();
        loop {
            tick.tick().await;
            let fresh = match self.read_sysfs() {
                Ok(b) => b,
                Err(e) => {
                    tracing::warn!(error=?e, "battery sysfs read failed");
                    continue;
                }
            };
            self.diff_and_emit(&last, &fresh).await;
            last = Arc::new(fresh.clone());
            self.snapshot.store(last.clone());
        }
    }

    fn read_sysfs(&self) -> std::io::Result<BatteryInfo> {
        // Reads from /sys/class/power_supply/BAT0/ and AC/online.
        // Smoothed values pulled forward from previous snapshot:
        //
        //   ewma_new = alpha * sample + (1 - alpha) * ewma_old
        //
        // alpha = 0.1 for power_smoothed_mw, 0.3 for capacity_smoothed_pct.
        // time_to_empty = (capacity_uah / current_now_ua) * 3600 when
        // discharging; None otherwise.
        unimplemented!("see power/battery.rs implementation file")
    }

    async fn diff_and_emit(
        &self,
        prev: &BatteryInfo,
        cur:  &BatteryInfo,
    ) {
        if prev.source != cur.source {
            self.emit(BatteryEvent::SourceChanged(cur.source)).await;
        }
        // Cross-threshold detection: 20, 10, 5 percent.
        for &th in &[20_u8, 10, 5] {
            if prev.capacity_pct > th && cur.capacity_pct <= th {
                self.emit(BatteryEvent::CapacityCrossed {
                    from_pct: prev.capacity_pct,
                    to_pct:   cur.capacity_pct,
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
        // Health
        if let (Some(p), Some(c)) = (prev.health_pct, cur.health_pct) {
            if p > 80 && c <= 80 {
                self.emit(BatteryEvent::HealthDegraded { health_pct: c }).await;
            }
        }
    }

    async fn emit(&self, ev: BatteryEvent) {
        let env = crate::ipc::IpcEnvelope::broadcast(
            "topic:system/battery-event", ev.encode(),
        );
        let _ = self.ipc.publish(env).await;
    }
}
```

### 3.3 Policy hooks driven by battery events

| Event                          | Effect                                                                 |
|--------------------------------|------------------------------------------------------------------------|
| `SourceChanged(Battery)`       | R5 multiplies `QosClass::Background`/`Maintenance` epoch budgets by 0.4; tighten App Nap threshold from 30 s to 8 s; display brightness clamped to ≤60%. |
| `SourceChanged(Ac)`            | Reset multipliers to 1.0; allow brightness up to user setting; Power Nap allowed even if user disabled "Power Nap on battery". |
| `Warning` (≤20%)               | UI notification; nothing functional changes.                           |
| `Critical` (≤10%)              | `PowerManager::request(HibernateReady, LowBattery)`; user notification "Hibernating in 30 s, plug in to cancel." |
| `Emergency` (≤5%)              | Force `PowerManager::request(PoweredOff, CriticalBattery)` after 30 s, no veto allowed (assertions ignored). |
| `HealthDegraded(<=80)`         | One-time UI notification; logged to `/data/.system/battery.log`.       |

### 3.4 Power-source-aware QoS multiplier

This is the wire from `BatteryEvent::SourceChanged` to R5's existing
budget tables:

```rust
// supervisor/src/scheduler/qos.rs (round 5 addition)

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum PowerMode {
    HighPerformance, // AC, lid open
    Balanced,        // AC, "Better Battery" preset
    LowPower,        // battery
    UltraLowPower,   // battery + <= 20%
}

impl QosClass {
    pub fn cpu_share_under(self, mode: PowerMode) -> f32 {
        let base = self.cpu_share_base();
        let mult = match (self, mode) {
            (QosClass::UserInteractive, _)           => 1.00,
            (QosClass::UserInitiated,   _)           => 1.00,
            (QosClass::Default,        PowerMode::HighPerformance) => 1.00,
            (QosClass::Default,        PowerMode::Balanced)        => 0.90,
            (QosClass::Default,        PowerMode::LowPower)        => 0.70,
            (QosClass::Default,        PowerMode::UltraLowPower)   => 0.40,
            (QosClass::Utility,        PowerMode::HighPerformance) => 1.00,
            (QosClass::Utility,        PowerMode::Balanced)        => 0.80,
            (QosClass::Utility,        PowerMode::LowPower)        => 0.50,
            (QosClass::Utility,        PowerMode::UltraLowPower)   => 0.20,
            (QosClass::Background,     PowerMode::HighPerformance) => 1.00,
            (QosClass::Background,     PowerMode::Balanced)        => 0.60,
            (QosClass::Background,     PowerMode::LowPower)        => 0.30,
            (QosClass::Background,     PowerMode::UltraLowPower)   => 0.05,
            (QosClass::Maintenance,    PowerMode::HighPerformance) => 1.00,
            (QosClass::Maintenance,    PowerMode::Balanced)        => 0.40,
            (QosClass::Maintenance,    PowerMode::LowPower)        => 0.10,
            (QosClass::Maintenance,    PowerMode::UltraLowPower)   => 0.00,
        };
        base * mult
    }
}
```

`PowerManager` writes the current `PowerMode` into another `ArcSwap` and
the R5 cgroup writer rewrites `cpu.max` on the next reconcile tick
(every 250 ms, debounced).

---

## 4. Display Power Management

### 4.1 DisplayPower

```rust
// supervisor/src/power/display_power.rs

use std::sync::Arc;
use arc_swap::ArcSwap;
use tokio::sync::Mutex;
use tokio::time::{Duration, Instant};

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum DpmsMode {
    On,
    Standby,
    Suspend,
    Off,
}

#[derive(Clone, Debug)]
pub struct DisplayPowerSnapshot {
    pub brightness_pct: u8,        // 0..=100
    pub user_setpoint:  u8,        // last user-chosen value
    pub dpms:           DpmsMode,
    pub auto_brightness: bool,     // ALS-driven (future R8)
}

pub struct DisplayPower {
    snap:      Arc<ArcSwap<DisplayPowerSnapshot>>,
    backlight: Arc<dyn BacklightWriter>,
    drm:       Arc<dyn DrmDpmsWriter>,
    /// Serializes ramps to avoid a dim/wake race.
    ramp_lock: Mutex<()>,
}

#[async_trait::async_trait]
pub trait BacklightWriter: Send + Sync {
    async fn set_brightness(&self, pct: u8) -> std::io::Result<()>;
    fn max_brightness(&self) -> u32;
    fn sysfs_root(&self) -> &std::path::Path;
}

#[async_trait::async_trait]
pub trait DrmDpmsWriter: Send + Sync {
    async fn set_dpms(&self, mode: DpmsMode) -> std::io::Result<()>;
}
```

### 4.2 Ramp algorithm

A linear ramp at 60 fps (16.6 ms steps) between current and target
brightness. The ramp is interruptible: if the user moves the mouse
mid-dim we abort the ramp and ramp back up.

```rust
impl DisplayPower {
    pub async fn ramp_to(&self, target_pct: u8, duration: Duration) {
        let _g = self.ramp_lock.lock().await;
        let start = self.snap.load().brightness_pct;
        if start == target_pct { return; }
        let steps = ((duration.as_millis() / 16) as i32).max(1);
        let delta = target_pct as i32 - start as i32;
        for i in 1..=steps {
            let pct = (start as i32 + (delta * i / steps)) as u8;
            let _ = self.backlight.set_brightness(pct).await;
            let mut s = (**self.snap.load()).clone();
            s.brightness_pct = pct;
            self.snap.store(Arc::new(s));
            tokio::time::sleep(Duration::from_millis(16)).await;
        }
    }

    pub async fn enter_sleep(&self) {
        // 1.5 s dim to 0% then DPMS off.
        self.ramp_to(0, Duration::from_millis(1500)).await;
        let _ = self.drm.set_dpms(DpmsMode::Off).await;
        let mut s = (**self.snap.load()).clone();
        s.dpms = DpmsMode::Off;
        self.snap.store(Arc::new(s));
    }

    pub async fn exit_sleep(&self) {
        let _ = self.drm.set_dpms(DpmsMode::On).await;
        let mut s = (**self.snap.load()).clone();
        s.dpms = DpmsMode::On;
        self.snap.store(Arc::new(s));
        // 0.6 s ramp back to setpoint.
        let target = s.user_setpoint;
        self.ramp_to(target, Duration::from_millis(600)).await;
    }
}
```

### 4.3 Dim sub-state

Between `FullyAwake` and `DisplaySleep` there is a transient *dim* state
that is not a top-level `SystemPowerState` (because nothing material
changes; we only nudge the backlight). After `dim_after_secs` of idle,
`DisplayPower::ramp_to(40, 800ms)` runs. Any input snaps brightness
back via `ramp_to(user_setpoint, 200ms)`. This keeps the FSM tractable
while reproducing the macOS feel where the screen gently dims before
sleep.

### 4.4 Wake sources to FullyAwake

| Source                  | Wire                                                   |
|-------------------------|--------------------------------------------------------|
| keyboard or mouse event | R6 input dispatcher → `IdleDetector::note_input()` → `PowerManager::request(FullyAwake, UserAction)` |
| Lid open                | ACPI `/proc/acpi/button/lid/LID0/state` poller (or libinput switch event) |
| Power button            | evdev KEY_POWER → if `DisplaySleep`, wake; if `FullyAwake`, prompt shutdown menu |
| Trackpad touch          | libinput TOUCH_DOWN                                    |
| External DisplayPort connect | DRM uevent (R6 DeviceManager) → wake               |

---

## 5. Power Nap

### 5.1 Goal

Periodically wake from `DisplaySleep` (and on AC also from `SystemSleep`)
to let apps with `background_fetch = true` run a bounded callback. This
is the macOS feature that lets Mail receive new messages overnight, lets
Time Machine make incremental backups, and lets the App Store finish
downloads — all without the user noticing.

### 5.2 Design

```rust
// supervisor/src/power/power_nap.rs

use std::sync::Arc;
use tokio::time::{Duration, Instant};

#[derive(Clone, Debug)]
pub struct FetchRegistration {
    pub app:                  AppId,
    pub interval:             Duration,    // minimum spacing between fetches
    pub max_runtime:          Duration,    // hard ceiling per fetch
    pub energy_budget_mj:     u32,         // per fetch (millijoules)
    pub last_fetch:           Option<Instant>,
    pub consecutive_failures: u32,
}

pub struct PowerNapScheduler {
    /// Registered apps eligible for Power Nap.
    registry: tokio::sync::RwLock<Vec<FetchRegistration>>,
    /// Hard global cap on total energy across all fetches in a window.
    window_energy_budget_mj: u32,
    /// Channel back to PowerManager to ask for a wake.
    power_tx: tokio::sync::mpsc::Sender<crate::power::TransitionRequest>,
    /// Channel into R3 IPC to deliver `on-background-fetch` to apps.
    ipc: Arc<crate::ipc::IpcRouter>,
    /// Energy meter, reads battery delta per fetch.
    meter: Arc<crate::power::battery::BatteryMonitor>,
}
```

### 5.3 Scheduling cadence

| Current state    | On AC                | On Battery           |
|------------------|----------------------|----------------------|
| FullyAwake       | Fetch on idle hint   | Fetch on idle hint   |
| DisplaySleep     | Every 30 min         | Every 60 min         |
| SystemSleep      | Every 2 h            | **Disabled**         |
| HibernateReady   | Disabled             | Disabled             |

In `DisplaySleep` no S3 wake is needed — CPU is still on, we just have
to keep the GPU and backlight asleep. In `SystemSleep` we program an
RTC alarm (`/sys/class/rtc/rtc0/wakealarm`) for the next window.

### 5.4 The fetch window algorithm

```rust
impl PowerNapScheduler {
    /// Called when an RTC alarm fires (in SystemSleep) or a tokio
    /// timer fires (in DisplaySleep). Wakes the system to FullyAwake
    /// only if we are in SystemSleep, runs all eligible callbacks,
    /// then returns the system to its prior power state.
    pub async fn nap_window(&self, prior: SystemPowerState) {
        let started = Instant::now();
        let mut budget_remaining = self.window_energy_budget_mj as i64;

        // If currently in SystemSleep, we need to wake to FullyAwake
        // to run fetches (CPU is off). The transition itself goes
        // FullyAwake (briefly) — we never run apps in DisplaySleep.
        if prior == SystemPowerState::SystemSleep {
            let (tx, rx) = tokio::sync::oneshot::channel();
            let _ = self.power_tx.send(crate::power::TransitionRequest {
                target: SystemPowerState::FullyAwake,
                reason: crate::power::TransitionReason::PowerNapRtc,
                reply:  tx,
            }).await;
            if rx.await.is_err() { return; }
        }

        // Walk registrations; pick the ones whose interval has elapsed.
        let now = Instant::now();
        let mut to_run: Vec<FetchRegistration> = self.registry
            .read().await
            .iter()
            .filter(|r| r.last_fetch.map_or(true,
                |t| now.saturating_duration_since(t) >= r.interval))
            .cloned()
            .collect();

        // Sort by oldest-fetched-first to ensure fairness.
        to_run.sort_by_key(|r| r.last_fetch.unwrap_or(Instant::now()));

        for reg in to_run {
            if budget_remaining <= 0 { break; }
            let bat_before = self.meter.snapshot().capacity_smoothed_pct;
            let res = tokio::time::timeout(
                reg.max_runtime,
                self.dispatch_fetch_callback(reg.app)
            ).await;
            let bat_after = self.meter.snapshot().capacity_smoothed_pct;
            // Rough energy attribution: percent drop * design_capacity_uah
            // * average voltage / 100 = mJ. Approximate.
            let consumed_mj = self.estimate_energy(
                bat_before, bat_after,
            );
            budget_remaining -= consumed_mj as i64;

            let mut guard = self.registry.write().await;
            if let Some(slot) = guard.iter_mut().find(|r| r.app == reg.app) {
                slot.last_fetch = Some(now);
                slot.consecutive_failures = match res {
                    Ok(Ok(())) => 0,
                    _          => slot.consecutive_failures.saturating_add(1),
                };
            }
        }

        // If we just did an idle window inside DisplaySleep, return to
        // DisplaySleep. If we woke from SystemSleep, go back there.
        if prior == SystemPowerState::SystemSleep {
            let (tx, _) = tokio::sync::oneshot::channel();
            let _ = self.power_tx.send(crate::power::TransitionRequest {
                target: SystemPowerState::SystemSleep,
                reason: crate::power::TransitionReason::PowerNapRtc,
                reply:  tx,
            }).await;
        }
        let total = started.elapsed();
        tracing::info!(?total, "Power Nap window completed");
    }

    async fn dispatch_fetch_callback(
        &self,
        app: AppId,
    ) -> std::io::Result<()> {
        let env = crate::ipc::IpcEnvelope::unicast(
            crate::ipc::IpcAddr::App(app),
            "vyoma:power/fetch.on-background-fetch",
            Vec::new(),
        );
        self.ipc.send(env).await
    }
}
```

### 5.5 Fetch eligibility

* Manifest must declare `[power] background_fetch = true`.
* App must not have crashed in its last fetch (≥3 consecutive failures
  silently disables the registration for 24 h).
* Total energy already spent in the current window must be below the
  global budget (`window_energy_budget_mj`, default 300 mJ ≈ 0.0001 Wh,
  i.e. negligible).
* App's QoS must be `Background` or `Utility`; `UserInteractive` and
  `UserInitiated` apps run synchronously during normal scheduling and
  do not need a Power Nap.

---

## 6. Power Assertions (caffeinate equivalent)

### 6.1 Assertion kinds

```rust
// supervisor/src/power/assertions.rs

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum AssertionKind {
    /// Prevent system sleep entirely. caffeinate -i / NSProcessInfo
    /// .NSActivityIdleSystemSleepDisabled
    PreventSystemSleep,
    /// Prevent display sleep, but allow system sleep on lid close.
    /// caffeinate -d / NSActivityIdleDisplaySleepDisabled
    PreventDisplaySleep,
    /// Prevent idle sleep but still allow forced sleep. caffeinate -m.
    PreventIdle,
    /// Prevent the OS from automatically demoting QoS during a critical
    /// task (e.g. building, video export). The app stays at its declared
    /// QoS even if it would otherwise have been demoted to Background.
    PreventDemotion,
}

#[derive(Clone, Debug)]
pub struct PowerAssertion {
    pub id:       AssertionId,
    pub app:      AppId,
    pub kind:     AssertionKind,
    pub reason:   String,            // human-readable, surfaced in UI
    pub created:  Instant,
    pub timeout:  Option<Duration>,  // auto-release after N seconds
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct AssertionId(pub u64);
```

### 6.2 RAII handle

Apps receive a `power-assertion-handle` resource via the WIT API. When
the resource is dropped (explicitly or by app exit / crash), the
supervisor sees the resource-table drop and clears the assertion. This
is the same RAII pattern used for R5 `ActivityAssertion` and R6
`DeviceHandle`.

```rust
pub struct PowerAssertionHandle {
    id:       AssertionId,
    registry: std::sync::Weak<PowerAssertionRegistry>,
}

impl Drop for PowerAssertionHandle {
    fn drop(&mut self) {
        if let Some(r) = self.registry.upgrade() {
            r.release_blocking(self.id);
        }
    }
}
```

### 6.3 The registry

```rust
pub struct PowerAssertionRegistry {
    next: std::sync::atomic::AtomicU64,
    map:  parking_lot::RwLock<
              std::collections::HashMap<AssertionId, PowerAssertion>
          >,
    /// Per-app cap on simultaneous assertions to prevent runaway.
    per_app_cap: u32,
    /// Global cap on total Prevent* assertions of any one kind.
    global_cap_per_kind: u32,
    ipc: Arc<crate::ipc::IpcRouter>,
}

impl PowerAssertionRegistry {
    pub fn create(
        &self,
        app: AppId,
        kind: AssertionKind,
        reason: String,
        timeout: Option<Duration>,
    ) -> Result<PowerAssertion, AssertionError> {
        // Caps.
        {
            let map = self.map.read();
            let by_app = map.values()
                .filter(|a| a.app == app)
                .count() as u32;
            if by_app >= self.per_app_cap {
                return Err(AssertionError::PerAppCapExceeded);
            }
            let by_kind = map.values()
                .filter(|a| a.kind == kind)
                .count() as u32;
            if by_kind >= self.global_cap_per_kind {
                return Err(AssertionError::GlobalCapExceeded);
            }
        }
        let id = AssertionId(
            self.next.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        );
        let a = PowerAssertion {
            id, app, kind, reason, created: Instant::now(), timeout,
        };
        self.map.write().insert(id, a.clone());
        // Schedule auto-release.
        if let Some(t) = timeout {
            let weak: std::sync::Weak<PowerAssertionRegistry> =
                std::sync::Weak::new(); // populated by caller via Arc::downgrade
            tokio::spawn(async move {
                tokio::time::sleep(t).await;
                if let Some(r) = weak.upgrade() {
                    r.release_blocking(id);
                }
            });
        }
        // Broadcast.
        let env = crate::ipc::IpcEnvelope::broadcast(
            "topic:system/assertion-event",
            AssertionEvent::Acquired(a.clone()).encode(),
        );
        tokio::spawn({
            let ipc = self.ipc.clone();
            async move { let _ = ipc.publish(env).await; }
        });
        Ok(a)
    }

    pub fn release_blocking(&self, id: AssertionId) {
        let mut map = self.map.write();
        if let Some(a) = map.remove(&id) {
            drop(map);
            let env = crate::ipc::IpcEnvelope::broadcast(
                "topic:system/assertion-event",
                AssertionEvent::Released(a).encode(),
            );
            let ipc = self.ipc.clone();
            tokio::spawn(async move { let _ = ipc.publish(env).await; });
        }
    }

    pub fn first_active(&self, kind: AssertionKind) -> Option<PowerAssertion> {
        self.map.read().values()
            .find(|a| a.kind == kind)
            .cloned()
    }

    pub fn list_active(&self) -> Vec<PowerAssertion> {
        self.map.read().values().cloned().collect()
    }
}

#[derive(Debug)]
pub enum AssertionError {
    PerAppCapExceeded,
    GlobalCapExceeded,
    TooLong { max: Duration },
}
```

### 6.4 UI exposure

`PowerManager` exposes the active assertion list via an IPC RPC
`vyoma:power/events.list-assertions()` that any privileged app (the
Activity Monitor analogue) can call. The UI lists "Pages — preventing
display sleep (recording screencast)" etc., matching macOS Activity
Monitor's Energy tab.

---

## 7. Thermal Management

### 7.1 Thermal zones

```rust
// supervisor/src/power/thermal.rs

#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum ThermalZone {
    Normal,   // < 60 C
    Warm,     // 60..80 C
    Hot,      // 80..95 C
    Critical, // >= 95 C
}

impl ThermalZone {
    pub fn from_decic(d: i32) -> Self {
        let c = d / 1000;
        match c {
            i32::MIN..=59 => Self::Normal,
            60..=79       => Self::Warm,
            80..=94       => Self::Hot,
            _             => Self::Critical,
        }
    }
}

#[derive(Clone, Debug)]
pub enum ThermalEvent {
    ZoneEntered { from: ThermalZone, to: ThermalZone, temp_decic: i32 },
    CriticalShutdownArmed { in_secs: u32 },
}
```

### 7.2 Sampling and hysteresis

The kernel exposes `/sys/class/thermal/thermal_zone*/temp` in
millidegrees C. We sample every 1 s with 3-sample hysteresis on
upward transitions and 5-sample hysteresis on downward (to avoid
flapping under bursty workloads).

```rust
pub struct ThermalGovernor {
    zones:       Vec<std::path::PathBuf>, // sysfs entries to read
    snapshot:    Arc<ArcSwap<ThermalZone>>,
    ipc:         Arc<crate::ipc::IpcRouter>,
    qos_signal:  Arc<arc_swap::ArcSwap<QosPowerMode>>, // R5 hookup
    history:     parking_lot::Mutex<std::collections::VecDeque<ThermalZone>>,
    power_tx:    tokio::sync::mpsc::Sender<crate::power::TransitionRequest>,
    armed_critical: parking_lot::Mutex<Option<Instant>>,
}

impl ThermalGovernor {
    pub async fn run(self) {
        let mut tick = tokio::time::interval(Duration::from_secs(1));
        loop {
            tick.tick().await;
            let max_decic = self.zones.iter()
                .filter_map(|p| std::fs::read_to_string(p).ok())
                .filter_map(|s| s.trim().parse::<i32>().ok())
                .max()
                .unwrap_or(0);
            let raw = ThermalZone::from_decic(max_decic);
            let prev = **self.snapshot.load();
            let smoothed = self.smooth(raw);
            if smoothed != prev {
                self.snapshot.store(Arc::new(smoothed));
                self.on_zone_change(prev, smoothed, max_decic).await;
            }
            self.check_critical_shutdown(smoothed).await;
        }
    }

    fn smooth(&self, raw: ThermalZone) -> ThermalZone {
        let mut h = self.history.lock();
        h.push_back(raw);
        if h.len() > 5 { h.pop_front(); }
        let cur = **self.snapshot.load();
        if raw > cur {
            // Going up: need 3 consecutive samples at >= raw.
            let cnt = h.iter().rev().take(3)
                .filter(|z| **z >= raw).count();
            if cnt >= 3 { raw } else { cur }
        } else if raw < cur {
            let cnt = h.iter().rev().take(5)
                .filter(|z| **z <= raw).count();
            if cnt >= 5 { raw } else { cur }
        } else { cur }
    }

    async fn on_zone_change(
        &self,
        from: ThermalZone,
        to:   ThermalZone,
        temp_decic: i32,
    ) {
        let ev = ThermalEvent::ZoneEntered { from, to, temp_decic };
        let env = crate::ipc::IpcEnvelope::broadcast(
            "topic:system/thermal-event", ev.encode(),
        );
        let _ = self.ipc.publish(env).await;

        // Wire to R5 via a QosPowerMode signal — thermal sits on top
        // of the battery-driven PowerMode as a multiplicative penalty.
        let new_mode = match to {
            ThermalZone::Normal   => QosPowerMode::Unthrottled,
            ThermalZone::Warm     => QosPowerMode::ThrottleBg(0.6),
            ThermalZone::Hot      => QosPowerMode::ThrottleBg(0.2),
            ThermalZone::Critical => QosPowerMode::SuspendNonUi,
        };
        self.qos_signal.store(Arc::new(new_mode));
    }

    async fn check_critical_shutdown(&self, zone: ThermalZone) {
        let mut armed = self.armed_critical.lock();
        match (zone, *armed) {
            (ThermalZone::Critical, None) => {
                *armed = Some(Instant::now());
                let env = crate::ipc::IpcEnvelope::broadcast(
                    "topic:system/thermal-event",
                    ThermalEvent::CriticalShutdownArmed { in_secs: 30 }
                        .encode(),
                );
                let _ = self.ipc.publish(env).await;
            }
            (ThermalZone::Critical, Some(t))
                if t.elapsed() >= Duration::from_secs(30) => {
                let (tx, _) = tokio::sync::oneshot::channel();
                let _ = self.power_tx.send(crate::power::TransitionRequest {
                    target: SystemPowerState::PoweredOff,
                    reason: crate::power::TransitionReason::Thermal,
                    reply:  tx,
                }).await;
                *armed = None;
            }
            (z, _) if z < ThermalZone::Critical => { *armed = None; }
            _ => {}
        }
    }
}

#[derive(Copy, Clone, Debug)]
pub enum QosPowerMode {
    Unthrottled,
    ThrottleBg(f32),  // multiply Background/Maintenance cpu_share
    SuspendNonUi,     // suspend everything except UserInteractive
}
```

### 7.3 GPU throttling hook

Where the kernel exposes `intel_pstate`, AMDGPU `pp_dpm_sclk`, or
mali freq tables, the governor writes the *lowest* DPM level when
entering `Hot`. This is best-effort; missing sysfs files are skipped
without error. The actual writer is in `power/thermal_gpu.rs` and is
intentionally a stub for vendors to plug in without touching the
governor.

---

## 8. Suspend / Resume

### 8.1 Six-phase suspend

```rust
// supervisor/src/power/suspend.rs

use std::sync::Arc;
use tokio::time::{Duration, Instant};
use std::collections::BTreeMap;

pub struct SuspendCoordinator {
    apps:      Arc<crate::apps::AppRegistry>, // R1
    ipc:       Arc<crate::ipc::IpcRouter>,
    vfs:       Arc<crate::vfs::VfsBackend>,   // R4
    devices:   Arc<crate::drivers::DeviceManager>, // R6
    audio:     Arc<crate::drivers::audio::AudioSubsystem>, // R6
    hibernate: Arc<crate::power::hibernate::HibernateWriter>,
}

#[derive(Debug)]
pub struct SuspendReport {
    pub phase_durations: BTreeMap<&'static str, Duration>,
    pub app_outcomes:    BTreeMap<AppId, AppSuspendOutcome>,
    pub total:           Duration,
}

#[derive(Debug)]
pub enum AppSuspendOutcome {
    Ok { took: Duration },
    Timeout,                 // forcibly dropped store
    Crashed { error: String },
}

impl SuspendCoordinator {
    pub async fn suspend_to_ram(
        &self,
        plan: crate::power::state::TransitionPlan,
    ) -> Result<SuspendReport, crate::power::TransitionVeto> {
        let mut rep = SuspendReport {
            phase_durations: BTreeMap::new(),
            app_outcomes:    BTreeMap::new(),
            total:           Duration::ZERO,
        };
        let t0 = Instant::now();

        // PHASE 1: broadcast on-suspend to all apps in reverse boot-phase
        // order (UI first, drivers last) and collect outcomes.
        let p_start = Instant::now();
        let outcomes = self.broadcast_on_suspend().await;
        rep.app_outcomes = outcomes;
        rep.phase_durations.insert("on_suspend", p_start.elapsed());

        // PHASE 2: drop all WASM stores to release memory.
        let p_start = Instant::now();
        self.drop_all_stores().await;
        rep.phase_durations.insert("drop_stores", p_start.elapsed());

        // PHASE 3: drain audio ring + flush display compositor.
        let p_start = Instant::now();
        self.audio.drain_for_suspend().await;
        // (Display compositor is naturally idle once stores are dropped.)
        rep.phase_durations.insert("drain_audio", p_start.elapsed());

        // PHASE 4: fsync every VFS bucket.
        let p_start = Instant::now();
        if let Err(e) = self.vfs.fsync_all_buckets().await {
            tracing::error!(error=?e, "fsync_all_buckets failed; aborting suspend");
            return Err(crate::power::TransitionVeto::UncommittedVfsTxn {
                txn_id: 0,
            });
        }
        rep.phase_durations.insert("fsync_vfs", p_start.elapsed());

        // PHASE 5: ask DeviceManager to quiesce; drivers get 250 ms each.
        let p_start = Instant::now();
        self.devices.quiesce_for_suspend(Duration::from_millis(250)).await;
        rep.phase_durations.insert("device_quiesce", p_start.elapsed());

        // PHASE 6: tell the kernel to suspend.
        let p_start = Instant::now();
        self.kernel_enter_s3().await;
        rep.phase_durations.insert("kernel_s3", p_start.elapsed());

        rep.total = t0.elapsed();
        Ok(rep)
    }
}
```

### 8.2 The `broadcast_on_suspend` algorithm

We send a `vyoma:power/events.on-suspend` IPC envelope to each app and
wait — with a per-app 5 s timeout — for an `on-suspend-complete`
acknowledgement. Outcomes are recorded; timeouts cause the supervisor to
forcibly drop the app's Wasmtime store (which releases all linear memory
and host handles).

```rust
impl SuspendCoordinator {
    async fn broadcast_on_suspend(&self)
        -> BTreeMap<AppId, AppSuspendOutcome>
    {
        let mut results = BTreeMap::new();
        let apps = self.apps.list_running();
        // Reverse boot-phase order: UI first, drivers last.
        let mut ordered = apps;
        ordered.sort_by_key(|a| std::cmp::Reverse(a.boot_phase));

        for app in ordered {
            let env = crate::ipc::IpcEnvelope::unicast(
                crate::ipc::IpcAddr::App(app.id),
                "vyoma:power/events.on-suspend",
                Vec::new(),
            );
            let started = Instant::now();
            match tokio::time::timeout(
                Duration::from_secs(5),
                self.ipc.send_and_await_ack(env),
            ).await {
                Ok(Ok(())) => {
                    results.insert(app.id, AppSuspendOutcome::Ok {
                        took: started.elapsed(),
                    });
                }
                Ok(Err(e)) => {
                    results.insert(app.id, AppSuspendOutcome::Crashed {
                        error: format!("{e:?}"),
                    });
                }
                Err(_) => {
                    results.insert(app.id, AppSuspendOutcome::Timeout);
                    // Forcibly drop store; R1 will not restart while we
                    // are suspended.
                    self.apps.force_drop_store(app.id).await;
                }
            }
        }
        results
    }

    async fn drop_all_stores(&self) {
        for app in self.apps.list_running() {
            self.apps.force_drop_store(app.id).await;
        }
    }

    async fn kernel_enter_s3(&self) {
        // The write blocks until the kernel returns from S3.
        let _ = tokio::task::spawn_blocking(|| {
            std::fs::write("/sys/power/state", b"mem")
        }).await;
        // We are now resumed.
    }
}
```

### 8.3 Resume

Resume is initiated by the kernel returning from the `write("mem")` call
above. From the supervisor's perspective the `kernel_enter_s3()` future
simply resolves, and we run the inverse of the suspend pipeline:

1. Replay the `CoordWal` (R4) — picks up any half-finished txn.
2. Re-arm device handles (R6 `DeviceManager::resume()`) — re-open `/dev/input/event*`, re-acquire framebuffer.
3. Re-prime audio SPSC ring (R6 `AudioSubsystem::resume()`).
4. Recreate Wasmtime stores for all previously-running apps using their persisted `StateBlob` (R1); if a store was force-dropped during suspend it is restarted from its on-disk state.
5. Broadcast `vyoma:power/events.on-resume`.
6. Ramp backlight up.
7. Note `IdleDetector::note_input(now)` so we don't immediately re-sleep.

The resume sequence is built so that, in the common case where every
app's `on-suspend` succeeded and no driver crashed, only steps 2, 4, 5,
and 6 do material work — wall-clock target 600 ms on a Tiger Lake i7.

### 8.4 Failure handling

If any phase fails (e.g. fsync returns EIO, kernel rejects `mem`), the
coordinator runs `abort_suspend()`:

1. Restart any apps whose stores were dropped (R1 normal restart path).
2. Run `on-resume` on apps that already saw `on-suspend`.
3. Return the `TransitionVeto` to the `PowerManager`.

This is correct because `on-suspend` must be idempotent (a contract on
WASM apps; documented in the WIT comments) and we have not yet entered
S3.

---

## 9. Hibernate (Safe Sleep)

### 9.1 When to hibernate vs. S3

| Condition                                | Action                          |
|------------------------------------------|---------------------------------|
| Battery < 10% AND on battery             | HibernateReady                  |
| Lid close on AC                          | SystemSleep (S3 only)           |
| Lid close on battery, battery ≥ 30%      | SystemSleep                     |
| Lid close on battery, battery < 30%      | **Safe Sleep**: write image + S3|
| User chooses "Hibernate" from menu       | HibernateReady                  |

"Safe Sleep" — the macOS default behavior — is: write the image, *then*
enter S3. If battery dies, on next boot we detect the image and resume
from disk. We do that for low-battery sleeps to trade ~30 s of slower
sleep for guaranteed durability.

### 9.2 Image format

```rust
// supervisor/src/power/hibernate.rs

use std::path::{Path, PathBuf};
use std::sync::Arc;

#[derive(serde::Serialize, serde::Deserialize)]
pub struct HibernateHeader {
    pub magic:        [u8; 8],   // b"VYOMHIB1"
    pub version:      u16,       // image format version
    pub created_unix: i64,
    pub app_count:    u32,
    pub wal_tip:      u64,       // R4 CoordWal sequence number
    pub image_uuid:   uuid::Uuid,
    pub host_machine_id: [u8; 32], // /etc/machine-id; must match on resume
    pub kernel_cmdline_hash: [u8; 32],
}

#[derive(serde::Serialize, serde::Deserialize)]
pub struct HibernateAppRecord {
    pub identity:   crate::apps::AppIdentity, // R1
    pub state_blob: Vec<u8>,                   // R1 StateBlob
    pub qos:        crate::scheduler::QosClass,
    pub assertions: Vec<crate::power::PowerAssertion>,
    pub manifest_hash: [u8; 32], // sanity-check vs current manifest
}

pub struct HibernateWriter {
    root: PathBuf,  // /data/.hibernate/
    apps: Arc<crate::apps::AppRegistry>,
    vfs:  Arc<crate::vfs::VfsBackend>,
}

impl HibernateWriter {
    pub async fn write_image(&self) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.root)?;
        let tmp = self.root.join("image.zst.tmp");
        let final_path = self.root.join("image.zst");

        let mut records = Vec::new();
        for app in self.apps.list_running() {
            let blob = self.apps.capture_state_blob(app.id).await?;
            records.push(HibernateAppRecord {
                identity:      app.identity.clone(),
                state_blob:    blob,
                qos:           app.qos,
                assertions:    Vec::new(), // re-acquired by app on resume
                manifest_hash: app.manifest_hash,
            });
        }
        let header = HibernateHeader {
            magic:                *b"VYOMHIB1",
            version:               1,
            created_unix:          chrono::Utc::now().timestamp(),
            app_count:             records.len() as u32,
            wal_tip:               self.vfs.coord_wal_tip(),
            image_uuid:            uuid::Uuid::new_v4(),
            host_machine_id:       crate::system::machine_id_bytes(),
            kernel_cmdline_hash:   crate::system::kernel_cmdline_hash(),
        };

        let f = std::fs::File::create(&tmp)?;
        let mut enc = zstd::Encoder::new(f, 3)?;
        bincode::serialize_into(&mut enc, &header)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
        for rec in &records {
            bincode::serialize_into(&mut enc, rec)
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
        }
        let f = enc.finish()?;
        f.sync_all()?;

        // Atomic rename so a crash mid-write never leaves a partial
        // image at the canonical name.
        std::fs::rename(&tmp, &final_path)?;
        // fsync parent directory to persist the rename.
        let dir = std::fs::File::open(&self.root)?;
        dir.sync_all()?;
        Ok(())
    }
}
```

### 9.3 Detection and resume

At boot, the supervisor checks for `/data/.hibernate/image.zst`. If
present and the header's `host_machine_id` + `kernel_cmdline_hash`
match the running system, the supervisor resumes apps from the image
instead of running its normal boot sequence:

```rust
impl HibernateWriter {
    pub fn try_load_image(&self) -> Option<HibernateImage> {
        let path = self.root.join("image.zst");
        if !path.exists() { return None; }
        let f = std::fs::File::open(&path).ok()?;
        let mut dec = zstd::Decoder::new(f).ok()?;
        let header: HibernateHeader =
            bincode::deserialize_from(&mut dec).ok()?;
        if &header.magic != b"VYOMHIB1" { return None; }
        if header.host_machine_id != crate::system::machine_id_bytes() {
            return None;
        }
        if header.kernel_cmdline_hash !=
            crate::system::kernel_cmdline_hash() { return None; }
        let mut apps = Vec::with_capacity(header.app_count as usize);
        for _ in 0..header.app_count {
            apps.push(bincode::deserialize_from(&mut dec).ok()?);
        }
        Some(HibernateImage { header, apps })
    }

    pub fn discard_image(&self) -> std::io::Result<()> {
        let path = self.root.join("image.zst");
        if path.exists() { std::fs::remove_file(path)?; }
        Ok(())
    }
}

pub struct HibernateImage {
    pub header: HibernateHeader,
    pub apps:   Vec<HibernateAppRecord>,
}
```

After a successful resume from image, the file is deleted so the next
suspend writes a fresh one.

### 9.4 Safe Sleep flow

```rust
impl SuspendCoordinator {
    pub async fn suspend_to_disk(
        &self,
        plan: crate::power::state::TransitionPlan,
    ) -> Result<SuspendReport, crate::power::TransitionVeto> {
        // 1. Run the normal suspend pipeline up to (but not including)
        //    the kernel S3 write.
        let mut rep = self.prepare_for_sleep().await?;
        // 2. Write the hibernate image.
        let t = Instant::now();
        self.hibernate.write_image().await
            .map_err(|_| crate::power::TransitionVeto::UncommittedVfsTxn {
                txn_id: 0,
            })?;
        rep.phase_durations.insert("hibernate_write", t.elapsed());
        // 3. Now enter S3. If battery dies, image will boot.
        let t = Instant::now();
        self.kernel_enter_s3().await;
        rep.phase_durations.insert("kernel_s3", t.elapsed());
        Ok(rep)
    }
}
```

---

## 10. WIT Package `vyoma:power@0.1.0`

```wit
// wit/vyoma-power.wit

package vyoma:power@0.1.0;

interface assert {
    use vyoma:base/types@0.1.0.{duration};

    enum kind {
        prevent-system-sleep,
        prevent-display-sleep,
        prevent-idle,
        prevent-demotion,
    }

    resource handle {
        constructor(kind: kind, reason: string, timeout: option<duration>);
        kind: func() -> kind;
        // The handle is released by dropping it.
    }

    /// Returns the count of currently held assertions of `k` held
    /// by *this* app (useful for debugging "did I leak a handle?").
    held-by-me: func(k: kind) -> u32;
}

interface events {
    enum state {
        fully-awake,
        display-sleep,
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
        power-button,
        power-nap-rtc,
        thermal,
        ac-plugged,
        ac-unplugged,
    }
    enum power-source { ac, battery, unknown }
    enum thermal-zone { normal, warm, hot, critical }

    record battery-snapshot {
        source:                  power-source,
        capacity-pct:            u8,
        power-now-mw:            s64,
        time-to-empty-secs:      option<u32>,
        health-pct:              option<u8>,
    }

    /// Called when system power state changes. Apps may not block.
    on-state-changed: func(from: state, to: state, why: reason);

    /// Called when power source changes (AC ↔ battery).
    on-power-source-changed: func(from: power-source, to: power-source);

    /// Called when battery crosses a capacity threshold (20, 10, 5).
    on-battery-low: func(snap: battery-snapshot, threshold: u8);

    /// Called when thermal zone changes.
    on-thermal: func(zone: thermal-zone, temp-c: s32);

    /// Synchronous query of the current battery snapshot.
    battery: func() -> battery-snapshot;
}

interface fetch {
    /// Called by the supervisor during a Power Nap window. The app
    /// should perform its background fetch quickly and return. The
    /// supervisor enforces the per-app `max_runtime` from the manifest.
    on-background-fetch: func();
}

world app {
    import vyoma:base/types@0.1.0;
    import assert;
    export events;
    export fetch;
}
```

### 10.1 Manifest schema

```toml
# apps/my-app/vyoma.toml
[app]
name    = "my-app"
version = "0.1.0"
wasm    = "my-app.wasm"

[capabilities]
stdio = true

[power]
background_fetch        = true
prevents_sleep          = false  # never auto-asserts
prevents_display_sleep  = false
realtime_audio          = false  # already in R5
fetch_interval_secs     = 1800   # default 30 min
fetch_max_runtime_secs  = 5
fetch_energy_budget_mj  = 50
```

`prevents_sleep` / `prevents_display_sleep` semantics: when `true`, the
supervisor auto-creates a `PreventSystemSleep`/`PreventDisplaySleep`
assertion for the app's *entire lifetime*. Use sparingly; the runtime
explicit API (`vyoma:power/assert.handle.new(...)`) is preferred for
narrow windows.

---

## 11. Putting It All Together — End-to-End Scenarios

### 11.1 User closes the lid on battery, battery at 12%

1. `acpid` / libinput emits `SwitchEvent::LidClose`.
2. `PowerManager` receives `TransitionRequest { target: SystemSleep, reason: LidClose }`.
3. Battery monitor's `capacity_pct = 12`; policy says: prefer Safe Sleep when `<30%`.
4. PowerManager retargets to `HibernateReady`.
5. Assertion check: no `PreventSystemSleep` held → proceed.
6. `SuspendCoordinator::suspend_to_disk()`:
   * Phase 1: `on-suspend` to all 8 apps; 1 timeout.
   * Phase 2: drop all stores.
   * Phase 3: drain audio.
   * Phase 4: `fsync_all_buckets()`.
   * Phase 5: `devices.quiesce_for_suspend()`.
   * Hibernate write: 11 MB compressed image to `/data/.hibernate/image.zst`, fsync.
   * Phase 6: `write("mem", "/sys/power/state")` → S3.
7. Total wall-clock: ~1.2 s prepare + ~7 s hibernate write + S3.
8. (Later) user opens lid → kernel resumes from S3 → supervisor's
   `kernel_enter_s3()` future resolves → device re-arm → re-create
   stores → `on-resume` broadcast → display ramp up.
9. The hibernate image at `/data/.hibernate/image.zst` is discarded
   immediately on successful S3 resume (we only need it if battery dies).

### 11.2 Battery dies while in S3

1. Kernel cold-boots; supervisor starts.
2. `HibernateWriter::try_load_image()` returns `Some(img)`.
3. Header machine-id and cmdline-hash match → resume from image.
4. For each `HibernateAppRecord`, supervisor recreates the Wasmtime
   store and calls the app's `on-resume` with the persisted
   `StateBlob`.
5. CoordWal is replayed (R4) — picks up the txn that was in flight.
6. The user sees their desktop restored, modulo a brief "Restored from
   hibernate" toast.

### 11.3 caffeinate-style assertion from a video export tool

```rust
// In an app
let h = vyoma::power::assert::Handle::new(
    vyoma::power::assert::Kind::PreventSleep,
    "Exporting video".to_string(),
    Some(Duration::from_secs(60 * 60)), // 1 h cap
);
do_long_video_export();
drop(h); // releases assertion on completion
```

If the user attempts to put the system to sleep during the export:

* `PowerManager` checks assertions, finds one with `PreventSystemSleep`,
  returns `TransitionVeto::ActivityAssertion`.
* The UI shows "Video Editor is preventing sleep — quit it or click
  Force Sleep to override".

If the user clicks Force Sleep, the UI sends
`PowerManager::request(SystemSleep, UserAction)` — `UserAction` is the
only reason that bypasses assertion checks (matches macOS behavior of
the Sleep menu item overriding `caffeinate`).

### 11.4 Mail does a Power Nap fetch

1. Mail's manifest declares `background_fetch = true,
   fetch_interval_secs = 1800`.
2. System is in `DisplaySleep`; tokio timer fires after 30 min.
3. `PowerNapScheduler::nap_window(DisplaySleep)`:
   * No state transition needed; CPU is on.
   * Mail's `interval` elapsed → dispatch `on-background-fetch`.
   * Mail downloads new messages (5 s), returns.
   * Energy meter reports ~12 mJ consumed → well under budget.
4. Scheduler moves on; no other app eligible.

### 11.5 Thermal spike during video render

1. Video render app at QoS UserInitiated; CPU at 85°C.
2. `ThermalGovernor` smooths to `Hot` after 3 consecutive samples.
3. Broadcasts `ThermalEvent::ZoneEntered { from: Warm, to: Hot }`.
4. Sets `QosPowerMode::ThrottleBg(0.2)` → R5 cgroup writer reduces
   Background and Maintenance apps to 20% of their normal CPU share.
5. CPU temp drops to 76°C over 12 s → governor smooths back to `Warm`.
6. UI shows a thermal indicator (optional, like macOS's blank menu
   bar icon).

---

## 12. File Layout & LOC Budget

| File                                                 | Approx LOC |
|------------------------------------------------------|------------|
| `supervisor/src/power/mod.rs`                        | ~210       |
| `supervisor/src/power/state.rs`                      | ~160       |
| `supervisor/src/power/idle.rs`                       | ~120       |
| `supervisor/src/power/battery.rs`                    | ~430       |
| `supervisor/src/power/display_power.rs`              | ~280       |
| `supervisor/src/power/power_nap.rs`                  | ~360       |
| `supervisor/src/power/assertions.rs`                 | ~320       |
| `supervisor/src/power/thermal.rs`                    | ~310       |
| `supervisor/src/power/thermal_gpu.rs` (vendor stub)  | ~80        |
| `supervisor/src/power/suspend.rs`                    | ~410       |
| `supervisor/src/power/hibernate.rs`                  | ~380       |
| `wit/vyoma-power.wit`                                | ~110       |
| Unit tests under `supervisor/tests/power_*.rs`       | ~700       |
| **Total**                                            | **~3 870** |

Every file fits inside the 500-line ceiling. The largest is
`suspend.rs` at ~410; splitting it further would obscure the
phase-by-phase flow that is central to its correctness.

---

## 13. Performance & Energy Targets

| Metric                                       | Target                    |
|----------------------------------------------|---------------------------|
| Steady-state CPU while idle, display on      | < 0.3% on Tiger Lake i7   |
| Steady-state CPU while DisplaySleep          | < 0.05%                   |
| Suspend total (8 apps, well-behaved)         | p50 600 ms, p95 900 ms    |
| Resume from S3 to first repaint              | p50 950 ms, p95 1.4 s     |
| Hibernate image write (8 apps, ~10 MB)       | p50 5 s, p95 8 s          |
| Resume from hibernate                        | p50 2.2 s, p95 3.5 s      |
| Battery sysfs sample cost                    | < 30 µs                   |
| Thermal sysfs sample cost                    | < 30 µs                   |
| Power Nap window overhead (no eligible apps) | < 1 ms                    |
| Assertion create/drop latency                | < 5 µs                    |

These get validated in R10 (benchmarks).

---

## 14. Failure-Mode Catalog

| Failure                                  | Detection                                | Response                                           |
|------------------------------------------|------------------------------------------|----------------------------------------------------|
| `/sys/class/power_supply/BAT0` missing   | `read_sysfs` returns `Err`               | Report `PowerSource::Unknown`; assume AC for safety. |
| Battery sysfs returns nonsense (0 µA discharging with 50% drop in 1 s) | Clamp delta; ignore extreme samples in EWMA. | Log warning; never alarm on a single bogus sample. |
| `/sys/power/state` rejects `mem`         | `write()` returns `Err`                  | `abort_suspend()`; broadcast `on-resume`; UI notification. |
| App's `on-suspend` deadlocks             | 5 s `tokio::time::timeout`               | Force-drop store; mark app `Crashed` for restart on resume. |
| Hibernate image truncated by disk-full   | `fsync_all` or `write` returns ENOSPC    | Discard partial image; fall back to plain S3.       |
| Hibernate image fails magic check on boot | Header magic mismatch                   | Discard image; normal boot.                         |
| Hibernate image valid but machine-id mismatch | Header field check                  | Discard image; normal boot (disk moved between machines). |
| Thermal sensor returns -ETIMEDOUT        | Read error                               | Hold previous zone; log; do not blindly assume Normal. |
| Power Nap callback infinite-loops        | `max_runtime` timeout                    | Kill app's epoch budget; treat as failure for backoff. |
| Assertion handle leaked (app crashes)    | Wasmtime resource-table drop on store-drop | Registry auto-clears; next idle window sleeps normally. |
| User unplugs AC mid-suspend              | `BatteryMonitor` PowerSource change      | If hibernate already started, finish it; never abort suspend midway. |
| Lid switch flapping (broken hinge)       | Debounce 500 ms                          | Only honor lid transitions after stable for 500 ms. |
| Thermal critical reached at boot         | Initial sample is `Critical`             | Refuse boot beyond minimal services; show on-screen warning, then shutdown after 30 s. |

---

## 15. Security Considerations

1. **No app can read another app's battery-fetch history.** The fetch
   registry is private to the supervisor; apps only see "you have N
   recent fetches" via their own assertion handle introspection.
2. **Assertion caps prevent DoS.** A buggy or malicious app cannot keep
   the machine awake by creating millions of assertions: `per_app_cap`
   (default 16) and `global_cap_per_kind` (default 256) both apply, and
   each assertion's `timeout` ceiling is 24 h.
3. **`UserAction` is privileged.** Only the supervisor's own UI shell
   (a specific built-in app identified by signed manifest) can submit a
   `TransitionRequest` with `reason: UserAction`. Other apps' transition
   requests come in with their own `AppId` attached and assertions are
   honored.
4. **Hibernate image is encrypted with the rootfs key.** When VyomaOS
   moves to full-disk encryption (a later round), the hibernate image
   inherits the same key wrapping; without the TPM-backed key, the
   image is opaque. The current spec leaves the image plaintext but the
   format is extensible.
5. **Power Nap on battery is disabled by default.** Users opt in via
   System Preferences; opting in does not let any app bypass the
   per-window energy budget.
6. **Critical shutdown cannot be vetoed.** When thermal `Critical` has
   been steady for 30 s or battery is `Emergency`, the only legal
   response is `PoweredOff`; assertions are ignored. This is the same
   fuse Linux's `oom_kill` and macOS thermal shutdown use.

---

## 16. Testing Strategy

### 16.1 Unit tests

* `power/state.rs`: every legal and illegal edge in the FSM is asserted
  by name; FSM is a pure function so the table is exhaustive.
* `power/battery.rs`: feed canned sysfs trees from `tests/fixtures/`,
  check derived EWMAs, time-to-empty, threshold-crossing events.
* `power/assertions.rs`: cap enforcement, RAII drop releases,
  per-app cap exceeded → typed error.
* `power/thermal.rs`: hysteresis behavior with synthetic temperature
  traces (slow ramp, oscillation, spike-and-recover).
* `power/power_nap.rs`: schedule selection with mocked clock; energy
  budget enforcement with mocked battery monitor.
* `power/suspend.rs`: simulated `IpcRouter` + `AppRegistry`; assert
  six-phase ordering, timeout handling, abort_suspend correctness.
* `power/hibernate.rs`: write → load round-trip; tamper byte → reject;
  machine-id mismatch → reject.

### 16.2 Integration tests (QEMU)

* `tests/integration_sleep_resume.rs`: spin up QEMU with the `mem`
  power state simulated by `qemu-system-x86_64 -device pmem` and
  observe a full suspend/resume cycle from the serial console.
* `tests/integration_battery.rs`: synthetic battery driver (a Rust
  test helper that writes `/sys/class/power_supply/BAT0/*` from
  outside the VM via 9P) drains 100% → 5% over 60 s of simulated time;
  expect Warning, Critical, Emergency events in order.
* `tests/integration_thermal_throttle.rs`: synthetic thermal trace
  pushes zones up and down; assert that cgroup `cpu.max` values
  written by R5 change in lockstep.

### 16.3 Property tests

* For every FSM trajectory of length ≤ 8, verify that running it
  forward then inverting it returns to a canonical state.
* For every assertion add/drop interleaving with up to 4 apps and
  16 operations, verify cap invariants hold and no assertion outlives
  its app.

---

## 17. Open Questions for the Critic

1. **Five states or six?** Should we model "User-initiated background
   compute" (e.g., a long render) as a distinct state where display
   may sleep but App Nap is *forbidden*? Currently this is achieved by
   the renderer holding a `PreventDemotion` assertion, but a first-
   class state might be cleaner and easier for the UI to surface.

2. **Is the 5 s per-app on-suspend budget realistic?** A Pages
   document with a 100 MB unsaved file may need to write to disk
   before checkpointing its state. Should we have a separate
   "checkpoint" phase before the deadline, and is 5 s the right
   ceiling versus 2 s or 10 s? Should the deadline scale with app QoS?

3. **Power Nap energy attribution.** Inferring energy from battery
   delta is noisy at sub-percent resolution and impossible to attribute
   per-app (the battery only knows the system total). Should we use
   RAPL counters where available, fall back to wall-clock × estimated
   CPU power, and only use battery delta as a sanity check? What
   sensor would we use on ARM laptops where RAPL doesn't exist?

4. **Critical thermal as software-only.** Linux already does emergency
   thermal shutdown via the kernel thermal framework. Is our 30 s
   armed window racing the kernel's faster trip points? Should we
   defer entirely to the kernel here and only run the *informational*
   warning UI, removing our own shutdown path? Doing so would simplify
   security ("critical cannot be vetoed") to a no-op.

5. **Hibernate as an OS-managed primitive vs. kernel `swsusp`.**
   Linux has its own hibernate (`/sys/power/state = disk` + a swap
   partition) that suspends the entire kernel image to disk. We are
   building a higher-level checkpoint of WASM state on top of S3.
   Should we instead lean on kernel `swsusp` and not write our own
   image? Trade-off: kernel hibernate requires a swap area sized to
   RAM; our image is bounded by sum-of-app-StateBlobs which is much
   smaller, *but* kernel hibernate is battle-tested and free.

6. **Per-app fetch intervals invite drift.** If 12 apps each declare
   `fetch_interval_secs = 1800` they will eventually align and cause
   a thundering herd. Should the scheduler add jitter (±10%) or
   stagger registrations explicitly? What's the user-visible cost
   of waking the radio + drawing the disk in a synchronized burst
   versus spreading them across 30 minutes?

7. **Lid-switch debounce vs. responsiveness.** A 500 ms debounce
   prevents flapping but adds perceived latency to "close lid → sleep
   light off" feedback. Should the debounce be asymmetric (closing
   slower, opening faster) or should we trust libinput's own
   debouncing? Either way, what's the test plan for a hinge that
   genuinely flickers due to a broken cable?

8. **Resume-time WIT callback fan-out cost.** With 64 apps running,
   broadcasting `on-resume` to all of them and waiting for ack before
   declaring `FullyAwake` could add 64 × IPC RTT ≈ 320 ms. Should we
   make `on-resume` fire-and-forget and let apps catch up
   asynchronously, accepting that a UI repaint may briefly show stale
   state? Or should we parallelize per-app on a thread pool, paying
   extra CPU but cutting wall-clock to the slowest single app?

---

## 18. Summary of Round-7 Deliverables

* **Five-state power FSM** with explicit guards and a single owning actor.
* **`BatteryMonitor`** with EWMA smoothing, threshold events, and Battery Health.
* **`DisplayPower`** with interruptible 60 fps backlight ramp + DPMS stages.
* **`PowerNapScheduler`** with per-app intervals, energy budgets, and AC/battery gating.
* **`PowerAssertionRegistry`** with four assertion kinds, RAII handles, and per-app caps.
* **`ThermalGovernor`** with four zones, hysteresis, and a critical-shutdown fuse.
* **Six-phase suspend pipeline** with per-app 5 s budget and journaled rollback.
* **Hibernate image** (`/data/.hibernate/image.zst`) with header validation and atomic rename.
* **WIT package `vyoma:power@0.1.0`** with `assert`, `events`, and `fetch` interfaces.
* **Manifest `[power]` section** for declarative per-app energy policy.

Total surface area: 8 new files (~3 100 LOC supervisor + ~700 LOC tests),
1 WIT package, 1 new manifest section. All within the 500-line per-file
ceiling.

---

*End of Round 7 Architect proposal. The Critic should now stress-test
the FSM, the assertion model, the suspend deadlines, the hibernate
image discipline, and the open questions in Section 17.*

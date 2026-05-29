# Round 2 Final: Virtual Memory & Address Space

**Status:** ✅ Debated & synthesized (2026-05-29)
**Debate:** [Architect: 1237 lines] [Critic: 747 lines] [Final: this]
**Subsystem:** Virtual Memory & Address Space (macOS equivalent: `vm_allocate` / `mmap` / `malloc` / `NSCache` / address space layout)
**Builds on Round 1:** sharded `AppTable`, `AppState` hot/cold split, `AppLimiter` (`ResourceLimiter`), `StateBlob` (≤ 1 MiB), `CrashKind` taxonomy, `InstanceId`, `BootPhase` lock-order discipline, WIT callbacks.
**Runtime invariants:** Wasmtime 43.0.0 embedded as library; one Linux supervisor process; `wasm32-wasip2` apps (no shared linear memory); 500-line ceiling per `.rs` file.

This document is the **authoritative implementation spec** for VyomaOS's virtual memory and address-space subsystem. It supersedes the architect proposal `02-virtual-memory.md` and incorporates every blocking fix in `02-virtual-memory-critique.md` (C1–C6) plus the synthesis recommendations R1–R22.

---

## Key Decisions

1. **PSI is the source of truth for memory pressure, not `/proc/meminfo` polling.** The supervisor opens `/proc/pressure/memory`, registers a "some 150 ms / 1 s" trigger, and waits on it via `epoll`. Polling becomes a fallback only when PSI is unavailable (kernel < 4.20 or `psi=0` boot flag). This is the single most important architectural inversion: **pressure is an event, not a number** (resolves C4).

2. **`MemoryGovernor` reservations are RAII tokens, never overshoot-and-backout.** `try_reserve()` returns `Option<ReservationToken<'_>>`. The token holds the budget; `commit()` makes the reservation permanent; `Drop` releases on panic or unwind. No window in which `wasm_used_bytes > wasm_budget_bytes` is ever observable (resolves C2).

3. **`AppLimiter` is lifted into `AppState.hot` as `Arc<AppLimiter>`.** The `Store` shares the same `Arc`; `WasmMemoryStat::read` reads atomic fields from any thread without touching the Store lock (resolves C1).

4. **Suspension grant: in-flight `Suspending` instances get an unconditional `state_blob_bytes + 256 KiB` admission window.** `on-suspend` can allocate to serialize state even under `PressureLevel::Critical`. The grant is reclaimed when the store is dropped (resolves C3).

5. **`SharedBufferKind::Surface` is zero-copy via aliasing.** When an app declares a `Surface` shared buffer, the `Vec<u8>` backing the supervisor-side `Surface.back` IS the shared buffer. Compositor reads it directly; no second copy. Video at 60 Hz is feasible (resolves C6).

6. **Per-instance cap, governor reservation, pressure admission are merged into a single atomic decision** inside `AppLimiter::memory_growing`. The decision is computed under lock-free atomics in < 100 ns on the hot path; an `AtomicBool is_foreground` on the limiter eliminates the `Arc<im::HashSet>` clone-per-grow cost (resolves S1).

7. **RSS budget at the 512 MiB `desktop-full` floor supports 12 concurrent apps with Wasmtime PoolingAllocator** and shared `tokio::task::spawn_blocking` driver pool (no per-app 8 MiB stack). The 50-app target is reserved for systems with ≥ 2 GiB RAM. `CLAUDE.md` is updated accordingly (resolves C5).

8. **Jetsam scoring uses a `profile.toml`-configurable weight table; `Suspended` apps are removed from the live-scoring pool** and compacted in a separate "old StateBlob GC" pass. This prevents the "select-victims-that-yield-no-memory" pathology (resolves S14).

9. **Static supervisor allocations (font atlas, framebuffer, image cache) are pre-registered with the governor** at `InitDisplay` boot phase so the governor's `wasm_used_bytes` reflects supervisor overhead from boot onwards (resolves S8).

10. **`WasmMemoryView` is replaced with thin free functions over `Memory::read` / `Memory::write`.** No raw `data_ptr` exposure, no snapshot-size guard pattern (which was unsound in release builds), no LOC waste. Wasmtime's `&mut Store` borrow already proves no concurrent grow (resolves S5).

11. **Batched re-sampling suspend loop with a per-tick cap of 5 instances** replaces "suspend everyone selected, then re-check on next tick". `MemAvailable` is re-read after each suspension; the loop exits as soon as the target watermark is reached (resolves S3).

12. **`on-memory-warning` WIT callback has a fully defined contract:** synchronous on driver thread, 100 ms budget, returns bytes-released, fires only on level transitions (resolves S13).

13. **Pull-only streaming gains `read-at(offset, len)` for random-access workloads** (video demuxers, MP4 `moov` atom seek-back). Implementation uses `pread()`; mmap-backed fast path optional (resolves S11).

14. **`SharedBufferKind::Surface` cap is raised to 64 MiB** to match max surface size. Other kinds (`AudioBuffer`, `ImageData`, `Custom`) remain at 16 MiB (resolves S12).

15. **Long-running app fragmentation is addressed by an explicit "soft restart" policy.** When `current_pages / reported_live_bytes > 4×` for > 1 h, the supervisor signals `on-soft-restart`, persists `StateBlob`, terminates, and re-spawns. This breaks the WASM "memory only grows" ratchet (resolves S10).

---

## 1. WASM Memory Model

### 1.1 The wasm32 linear memory primitive

A `wasm32-wasip2` module has exactly one memory primitive: a single contiguous linear memory of `u8` indexed by `i32`, grown by `memory.grow` in 64 KiB pages, capped at 4 GiB. `wasip2` does **not** include wasm-threads, so there is no `SharedArrayBuffer` and no cross-instance shared linear memory. Each `wasmtime::Memory` is private to one `wasmtime::Store`.

VyomaOS does not give apps `mach_vm_*` / Linux `mmap`. The supervisor arbitrates physical memory; bounds-checks are encoded by Wasmtime (Cranelift) at compile time.

```rust
// supervisor/src/runtime/wasm_runtime.rs
pub const WASM_PAGE_BYTES:               usize = 65_536;     // 64 KiB
pub const WASM_MAX_PAGES_HARD:           u32   = 65_536;     // 4 GiB (WASM spec)
pub const WASM_MAX_PAGES_DEFAULT:        u32   = 2_048;      // 128 MiB default
pub const WASM_MAX_PAGES_FOREGROUND_MAX: u32   = 16_384;     // 1 GiB ceiling
pub const WASM_MAX_TABLE_ELEMENTS:       u32   = 16_384;
pub const SUSPENSION_GRANT_BYTES:        usize = 1_048_576 + 262_144;   // 1 MiB + 256 KiB scratch
```

### 1.2 Manifest declaration

```toml
[memory]
initial_pages         = 16        # initial linear memory (default 16 = 1 MiB)
max_pages             = 2048      # hard cap (default 2048 = 128 MiB)
state_blob_bytes      = 1_048_576 # ≤ 1 MiB (Round 1 default)
allow_shared_buffers  = true
pin_in_pressure       = false     # if true, jetsam will not select this app
elevated_state        = false     # allows state_blob > 1 MiB, signed-only

# Optional per-app override of streaming read-ahead.
[memory.streaming]
chunk_size            = 262_144   # 256 KiB default
read_ahead            = 2         # number of chunks
```

Validation (`manifest.rs::validate_memory()`):
- `initial_pages ≤ max_pages`
- `max_pages ≤ WASM_MAX_PAGES_FOREGROUND_MAX`
- `state_blob_bytes ≤ 1_048_576` unless `elevated_state = true` AND `InstallOrigin ∈ {System, Developer}`
- `chunk_size ∈ [4 KiB, 4 MiB]`
- `read_ahead ∈ [0, 8]`

### 1.3 `AppLimiter` — shared between Store and AppState.hot

Round 1 introduced `AppLimiter` as the per-store limiter. The synthesis lifts it into `AppState.hot` via `Arc<AppLimiter>`. Both the `Store` (via `Store::data().limiter`) and host-side observers reference the same atomic fields.

```rust
// supervisor/src/resource_limiter.rs
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use wasmtime::ResourceLimiter;

pub struct AppLimiter {
    pub instance_id:        InstanceId,
    pub bundle_id:          BundleId,
    pub max_pages:          u32,
    pub current_pages:      AtomicU64,        // mirrors growth; written from Store thread
    pub peak_pages:         AtomicU64,
    pub grow_denied_count:  AtomicU64,
    pub grow_attempted:     AtomicU64,        // counts every memory.grow call
    pub last_attempted_pages: AtomicU64,      // for crash report enrichment (resolves S15)

    /// Cached foreground state. Updated by focus manager via
    /// MemoryGovernor::set_foreground; read once per grow.
    pub is_foreground:      AtomicBool,

    /// Cached lifecycle state for grow decisions; updated by LifecycleActor.
    /// 0 = NotStarted, 1 = Foreground, 2 = Background, 3 = Suspending,
    /// 4 = Suspended, 5 = Stopping, 6 = Crashed.
    pub lifecycle_tag:      std::sync::atomic::AtomicU8,

    /// Suspension grant remaining bytes. Set when LifecycleActor begins suspending;
    /// drained by memory_growing.
    pub suspension_grant:   AtomicU64,

    pub governor:           Arc<MemoryGovernor>,
}

impl AppLimiter {
    pub fn new(
        instance_id: InstanceId,
        bundle_id:   BundleId,
        max_pages:   u32,
        initial_pages: u32,
        governor:    Arc<MemoryGovernor>,
    ) -> Self {
        Self {
            instance_id, bundle_id, max_pages, governor,
            current_pages:        AtomicU64::new(initial_pages as u64),
            peak_pages:           AtomicU64::new(initial_pages as u64),
            grow_denied_count:    AtomicU64::new(0),
            grow_attempted:       AtomicU64::new(0),
            last_attempted_pages: AtomicU64::new(initial_pages as u64),
            is_foreground:        AtomicBool::new(false),
            lifecycle_tag:        std::sync::atomic::AtomicU8::new(0),
            suspension_grant:     AtomicU64::new(0),
        }
    }

    #[inline]
    pub fn set_foreground(&self, flag: bool) {
        self.is_foreground.store(flag, Ordering::Release);
    }

    #[inline]
    pub fn set_lifecycle(&self, tag: u8) {
        self.lifecycle_tag.store(tag, Ordering::Release);
    }

    /// Open a suspension grant window. Called by LifecycleActor immediately before
    /// invoking on-suspend. Reclaimed when the store is dropped.
    pub fn open_suspension_grant(&self, bytes: u64) {
        self.suspension_grant.store(bytes, Ordering::Release);
        self.set_lifecycle(3 /* Suspending */);
    }

    fn try_consume_grant(&self, bytes: u64) -> bool {
        let mut current = self.suspension_grant.load(Ordering::Acquire);
        loop {
            if current < bytes { return false; }
            match self.suspension_grant.compare_exchange_weak(
                current, current - bytes, Ordering::AcqRel, Ordering::Acquire) {
                Ok(_) => return true,
                Err(actual) => current = actual,
            }
        }
    }
}

impl ResourceLimiter for AppLimiter {
    fn memory_growing(
        &mut self,
        current: usize,
        desired: usize,
        _maximum: Option<usize>,
    ) -> wasmtime::Result<bool> {
        self.grow_attempted.fetch_add(1, Ordering::Relaxed);

        let cur_pages   = (current / WASM_PAGE_BYTES) as u64;
        let want_pages  = (desired / WASM_PAGE_BYTES) as u64;
        let delta_pages = want_pages.saturating_sub(cur_pages);
        let delta_bytes = (delta_pages as usize) * WASM_PAGE_BYTES;

        self.last_attempted_pages.store(want_pages, Ordering::Relaxed);

        // 1. Per-instance cap.
        if want_pages > self.max_pages as u64 {
            self.grow_denied_count.fetch_add(1, Ordering::Relaxed);
            return Ok(false);
        }

        // 2. Suspension grant first — bypasses governor and pressure checks
        //    so that on-suspend can serialize state. Reclaimed on store drop.
        let lifecycle = self.lifecycle_tag.load(Ordering::Acquire);
        if lifecycle == 3 /* Suspending */ && self.try_consume_grant(delta_bytes as u64) {
            self.current_pages.store(want_pages, Ordering::Relaxed);
            let _ = self.peak_pages.fetch_max(want_pages, Ordering::Relaxed);
            return Ok(true);
        }

        // 3. Pressure check (cached level, atomic load).
        let pressure = self.governor.pressure_level();
        match pressure {
            PressureLevel::Imminent => {
                self.grow_denied_count.fetch_add(1, Ordering::Relaxed);
                return Ok(false);
            }
            PressureLevel::Critical if !self.is_foreground.load(Ordering::Acquire) => {
                self.grow_denied_count.fetch_add(1, Ordering::Relaxed);
                return Ok(false);
            }
            _ => {}
        }

        // 4. Global governor reservation (RAII token; auto-released on Drop).
        let Some(token) = self.governor.try_reserve(delta_bytes) else {
            self.grow_denied_count.fetch_add(1, Ordering::Relaxed);
            return Ok(false);
        };

        // 5. Commit: from here we cannot fail; the token sticks.
        self.current_pages.store(want_pages, Ordering::Relaxed);
        let _ = self.peak_pages.fetch_max(want_pages, Ordering::Relaxed);
        token.commit();
        Ok(true)
    }

    fn memory_grow_failed(&mut self, err: anyhow::Error) -> wasmtime::Result<()> {
        // Wasmtime calls this only on host-side allocator failure (mmap returned ENOMEM).
        // Distinct from us returning Ok(false). Crash classifier maps this to
        // CrashKind::HostOOM (Round 1 §5.1).
        tracing::error!(?err, instance = ?self.instance_id, "memory grow host-allocator failed");
        Err(err.context("host-side allocator exhausted"))
    }

    fn table_growing(&mut self, _current: usize, desired: usize, _max: Option<usize>) -> wasmtime::Result<bool> {
        Ok(desired <= WASM_MAX_TABLE_ELEMENTS as usize)
    }
}
```

### 1.4 `WasmMemoryStat` — genuinely lock-free read

```rust
// supervisor/src/memory/stat.rs
pub struct WasmMemoryStat {
    pub current_pages:        u32,
    pub peak_pages:           u32,
    pub byte_capacity:        usize,
    pub grow_denied:          u64,
    pub grow_attempted:       u64,
    pub last_attempted_pages: u32,
}

impl WasmMemoryStat {
    /// Read directly from the Arc<AppLimiter> in AppState.hot.
    /// Genuinely lock-free; no Store contact required.
    pub fn read(limiter: &AppLimiter) -> Self {
        let current = limiter.current_pages.load(Ordering::Acquire);
        Self {
            current_pages:        current as u32,
            peak_pages:           limiter.peak_pages.load(Ordering::Acquire) as u32,
            byte_capacity:        (current as usize) * WASM_PAGE_BYTES,
            grow_denied:          limiter.grow_denied_count.load(Ordering::Relaxed),
            grow_attempted:       limiter.grow_attempted.load(Ordering::Relaxed),
            last_attempted_pages: limiter.last_attempted_pages.load(Ordering::Relaxed) as u32,
        }
    }
}
```

This exposes **capacity** (how many pages the app has reserved), not **occupancy**. Occupancy is opaque to the host; if an app wants to report it, it calls `vyoma:memory/report.report-heap`.

### 1.5 Allocators

The supervisor does not pick the allocator — the app does. Three options ship in our SDK:

| Allocator   | Crate                    | Overhead    | When to use                            |
|-------------|--------------------------|-------------|----------------------------------------|
| `dlmalloc`  | `dlmalloc-rs`            | ~3 % space  | Default. Reasonable for any app.       |
| `wee_alloc` | `wee_alloc`              | ~1 KiB code | Tiny apps; no `free` of large blocks.  |
| `buddy`     | `buddy_system_allocator` | ~5 % space  | Apps with bounded allocation patterns. |

Apps declare `#[global_allocator]`. The allocator calls `memory.grow` directly when its free lists are exhausted; the supervisor sees only the grow event. Fragmentation handling is per §8 (soft restart).

---

## 2. MemoryGovernor — Fixed Design (resolves C2, S9)

The governor is the single source of truth for "can this grow?" and "should this be evicted?". Its hot path is lock-free atomics; its slow path acquires read locks on the `AppTable` only.

### 2.1 `ReservationToken` RAII

```rust
// supervisor/src/memory/governor.rs
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use arc_swap::ArcSwap;

pub struct MemoryGovernor {
    /// WASM linear memory budget across all stores.
    /// Default = platform_ram_floor * 0.50 on small platforms; 75 % on desktop-full.
    wasm_budget_bytes:   AtomicU64,
    wasm_used_bytes:     AtomicU64,

    /// Pre-registered supervisor static allocations (atlas, framebuffer, caches).
    /// Counted separately so we never subtract them by mistake.
    static_reserved_bytes: AtomicU64,

    /// Shared-buffer registry budget. Separate budget so a SHM storm cannot
    /// starve linear-memory growth and vice versa.
    shm_budget_bytes:    AtomicU64,
    shm_used_bytes:      AtomicU64,

    /// Host-side I/O buffer budget (streaming chunks). Resolves G8.
    host_io_budget_bytes: AtomicU64,
    host_io_used_bytes:   AtomicU64,

    /// Cached pressure level; updated only by MemoryPressureMonitor.
    pressure_level:      arc_swap::ArcSwap<PressureLevel>,

    /// Set of foreground InstanceIds. ONLY used by mgmt API; the hot grow
    /// path reads AppLimiter.is_foreground directly (resolves S1).
    foreground_set:      ArcSwap<im::HashSet<InstanceId>>,

    /// Reverse map InstanceId → Arc<AppLimiter> for set_foreground/set_lifecycle.
    /// DashMap so updates are concurrent and lock-free for readers.
    limiters_by_instance: dashmap::DashMap<InstanceId, Arc<AppLimiter>>,

    /// Reference to AppTable (Round 1) for victim selection.
    app_table:           Arc<AppTable>,

    /// Caches we can trim.
    glyph_cache:         Arc<GlyphCache>,
    image_cache:         Arc<ImageDecodeCache>,
    shm_registry:        Arc<SharedBufferRegistry>,

    /// Pressure event sender to LifecycleActor.
    pressure_tx:         crossbeam::channel::Sender<PressureEvent>,

    /// Counters for telemetry.
    jetsam_events:       AtomicU64,
    suspend_events:      AtomicU64,
    soft_restart_events: AtomicU64,

    /// Per-second rolling jetsam window (for the heartbeat jetsam_storm flag).
    jetsam_ts_ring:      parking_lot::Mutex<smallvec::SmallVec<[std::time::Instant; 16]>>,
}

/// RAII reservation: holds bytes against the wasm budget. Drop releases if
/// not committed. Panic-safe: if the holder unwinds, budget is released.
#[must_use = "ReservationToken must be either committed or dropped"]
pub struct ReservationToken<'g> {
    governor:  &'g MemoryGovernor,
    bytes:     u64,
    committed: bool,
}

impl<'g> ReservationToken<'g> {
    /// Make the reservation permanent. After commit(), the budget is held
    /// until released explicitly via MemoryGovernor::release_committed().
    pub fn commit(mut self) {
        self.committed = true;
    }

    pub fn bytes(&self) -> u64 { self.bytes }
}

impl Drop for ReservationToken<'_> {
    fn drop(&mut self) {
        if !self.committed {
            self.governor.wasm_used_bytes.fetch_sub(self.bytes, Ordering::AcqRel);
        }
    }
}

impl MemoryGovernor {
    /// Try to reserve `bytes` against the wasm budget. Returns a token that
    /// must be either commit()ed or dropped (auto-release).
    pub fn try_reserve(&self, bytes: usize) -> Option<ReservationToken<'_>> {
        let bytes = bytes as u64;
        if bytes == 0 {
            return Some(ReservationToken { governor: self, bytes: 0, committed: false });
        }
        let budget = self.wasm_budget_bytes.load(Ordering::Acquire);

        // CAS loop: never overshoot the budget visibly.
        let mut current = self.wasm_used_bytes.load(Ordering::Acquire);
        loop {
            let new = current.checked_add(bytes)?;
            if new > budget { return None; }
            match self.wasm_used_bytes.compare_exchange_weak(
                current, new, Ordering::AcqRel, Ordering::Acquire,
            ) {
                Ok(_) => return Some(ReservationToken { governor: self, bytes, committed: false }),
                Err(actual) => current = actual,
            }
        }
    }

    /// Release bytes that were previously committed (e.g. on instance teardown).
    pub fn release_committed(&self, bytes: u64) {
        self.wasm_used_bytes.fetch_sub(bytes, Ordering::AcqRel);
    }

    /// Pre-register a static supervisor allocation (font atlas, framebuffer, etc.).
    /// Counts against both static_reserved and wasm_used (so budget is honest).
    pub fn pre_register(&self, label: &'static str, bytes: usize) {
        let bytes = bytes as u64;
        self.static_reserved_bytes.fetch_add(bytes, Ordering::AcqRel);
        self.wasm_used_bytes.fetch_add(bytes, Ordering::AcqRel);
        tracing::info!(label, bytes, "governor: pre-registered static allocation");
    }

    #[inline]
    pub fn pressure_level(&self) -> PressureLevel {
        **self.pressure_level.load()
    }

    /// Called by FocusManager on focus change. Updates the cached bool on the
    /// AppLimiter directly — single atomic store, no Arc clone.
    pub fn set_foreground(&self, id: InstanceId, flag: bool) {
        if let Some(limiter) = self.limiters_by_instance.get(&id) {
            limiter.set_foreground(flag);
        }
        // Update the mgmt-visible set as well.
        let mut new_set = (**self.foreground_set.load()).clone();
        if flag { new_set.insert(id); } else { new_set.remove(&id); }
        self.foreground_set.store(Arc::new(new_set));
    }

    pub fn set_lifecycle(&self, id: InstanceId, tag: u8) {
        if let Some(limiter) = self.limiters_by_instance.get(&id) {
            limiter.set_lifecycle(tag);
        }
    }

    pub fn register_limiter(&self, id: InstanceId, limiter: Arc<AppLimiter>) {
        self.limiters_by_instance.insert(id, limiter);
    }

    pub fn unregister_limiter(&self, id: InstanceId) {
        if let Some((_, l)) = self.limiters_by_instance.remove(&id) {
            // Release the limiter's current_pages from the wasm budget.
            let bytes = l.current_pages.load(Ordering::Acquire) * WASM_PAGE_BYTES as u64;
            self.release_committed(bytes);
        }
    }

    /// Telemetry snapshot.
    pub fn snapshot(&self) -> GovernorSnapshot {
        GovernorSnapshot {
            wasm_budget:           self.wasm_budget_bytes.load(Ordering::Acquire),
            wasm_used:             self.wasm_used_bytes.load(Ordering::Acquire),
            static_reserved:       self.static_reserved_bytes.load(Ordering::Acquire),
            shm_budget:            self.shm_budget_bytes.load(Ordering::Acquire),
            shm_used:              self.shm_used_bytes.load(Ordering::Acquire),
            host_io_budget:        self.host_io_budget_bytes.load(Ordering::Acquire),
            host_io_used:          self.host_io_used_bytes.load(Ordering::Acquire),
            pressure:              self.pressure_level(),
            jetsam_events_total:   self.jetsam_events.load(Ordering::Relaxed),
            suspend_events_total:  self.suspend_events.load(Ordering::Relaxed),
            soft_restart_total:    self.soft_restart_events.load(Ordering::Relaxed),
            jetsam_events_last_5s: self.recent_jetsam_count(std::time::Duration::from_secs(5)),
        }
    }

    fn recent_jetsam_count(&self, window: std::time::Duration) -> u32 {
        let now = std::time::Instant::now();
        let ring = self.jetsam_ts_ring.lock();
        ring.iter().filter(|t| now.duration_since(**t) <= window).count() as u32
    }

    pub(crate) fn record_jetsam(&self) {
        self.jetsam_events.fetch_add(1, Ordering::Relaxed);
        let mut ring = self.jetsam_ts_ring.lock();
        if ring.len() == 16 { ring.remove(0); }
        ring.push(std::time::Instant::now());
    }
}

#[derive(Clone, Copy, Debug)]
pub struct GovernorSnapshot {
    pub wasm_budget:           u64,
    pub wasm_used:             u64,
    pub static_reserved:       u64,
    pub shm_budget:            u64,
    pub shm_used:              u64,
    pub host_io_budget:        u64,
    pub host_io_used:          u64,
    pub pressure:              PressureLevel,
    pub jetsam_events_total:   u64,
    pub suspend_events_total:  u64,
    pub soft_restart_total:    u64,
    pub jetsam_events_last_5s: u32,
}
```

**Properties:**
- Never overshoots: budget overshoot is impossible because CAS rejects any addition that would push `wasm_used > budget`.
- Panic-safe: if a thread panics between `try_reserve` and `commit`, `Drop` releases.
- No spurious telemetry: heartbeats and pressure monitor can read `wasm_used_bytes` and never see a transient over-budget value.
- Non-leaking: every committed reservation is eventually released via `release_committed` when the limiter's instance terminates.

### 2.2 Budget configuration

The governor's budgets come from `profile.toml`:

```toml
# supervisor/src/profile/profiles/desktop-full.toml
[memory]
wasm_budget_fraction       = 0.55    # 55 % of platform RAM floor on small systems
wasm_budget_fraction_large = 0.75    # 75 % when total RAM > 2 GiB
shm_budget_bytes           = 67_108_864     # 64 MiB
host_io_budget_bytes       = 16_777_216     # 16 MiB
pressure_warn_avail_mib    = 200
pressure_crit_avail_mib    = 50
pressure_imminent_avail_mib = 20
pressure_hysteresis_mib    = 50
max_victims_per_tick       = 5
```

At boot, `LoadProfile` reads these; `InitRegistries` constructs the governor with the resolved values.

---

## 3. Memory Telemetry — Fixed Design (resolves C1)

The architect's `WasmMemoryStat::read(&AppLimiter)` claimed to be lock-free but couldn't actually reach the limiter without touching the Store. The fix: **lift `AppLimiter` into `AppState.hot` as `Arc<AppLimiter>`** so it is reachable from any thread via the sharded `AppTable`.

### 3.1 AppState.hot extension

```rust
// supervisor/src/process_table.rs (modified)
pub struct AppStateHot {
    pub limiter:        Arc<AppLimiter>,    // NEW: shared with Store
    pub surface:        ArcSwap<Surface>,   // Round 1: per-window surface
    pub last_focus:     std::time::Instant,
    pub reported_heap:  parking_lot::Mutex<Option<ReportedHeap>>,    // NEW: from report-heap
    // ... rest from Round 1 ...
}

#[derive(Clone, Copy, Debug)]
pub struct ReportedHeap {
    pub live_bytes:   u64,
    pub free_bytes:   u64,
    pub reported_at:  std::time::Instant,
}
```

The `Store` references the same `Arc<AppLimiter>` via its `HostState`. Wasmtime's `ResourceLimiter` trait takes `&mut self`, but we install it via a closure (`Store::limiter(|s| &mut *s.limiter)`) where `s.limiter` is `&mut AppLimiter` derived from `Arc::get_mut` — except we don't need `get_mut` because every field on `AppLimiter` is atomic. The `&mut self` requirement of `ResourceLimiter` is satisfied by a thin shim that derefs through the `Arc`:

```rust
// supervisor/src/runtime/wasm_store.rs
pub struct HostState {
    pub limiter: Arc<AppLimiter>,
    // ... rest ...
}

impl HostState {
    /// Wasmtime's limiter closure requires &mut ResourceLimiter.
    /// We return a wrapper that derefs through the Arc; all fields are atomic
    /// so &mut on the wrapper is sound.
    pub fn limiter_view(&mut self) -> LimiterView<'_> {
        LimiterView { inner: &self.limiter }
    }
}

pub struct LimiterView<'a> { inner: &'a Arc<AppLimiter> }

impl<'a> ResourceLimiter for LimiterView<'a> {
    fn memory_growing(&mut self, current: usize, desired: usize, max: Option<usize>)
        -> wasmtime::Result<bool>
    {
        // Construct a fresh AppLimiter shim that delegates to the Arc.
        // Since AppLimiter's memory_growing is &mut self but only mutates atomics,
        // we can call it through &self via raw-pointer cast OR by making
        // AppLimiter::memory_growing take &self. We choose the latter.
        self.inner.memory_growing_atomic(current, desired, max)
    }
    fn table_growing(&mut self, current: usize, desired: usize, max: Option<usize>)
        -> wasmtime::Result<bool>
    {
        self.inner.table_growing_atomic(current, desired, max)
    }
    fn memory_grow_failed(&mut self, err: anyhow::Error) -> wasmtime::Result<()> {
        self.inner.memory_grow_failed_atomic(err)
    }
}
```

`AppLimiter::memory_growing_atomic` is the `&self`-flavored version; the `&mut self` `ResourceLimiter` impl on the bare `AppLimiter` delegates to it. This satisfies Wasmtime's trait while keeping the `Arc` shareable.

### 3.2 Reading from anywhere

The mgmt thread, the heartbeat emitter, the pressure monitor, the jetsam scorer — all of them resolve the `Arc<AppLimiter>` via:

```rust
fn read_stat(app_table: &AppTable, id: InstanceId) -> Option<WasmMemoryStat> {
    let shard = app_table.shard_for(id);
    let guard = shard.read();             // read lock on shard, brief
    let state = guard.get(&id)?;
    let stat = WasmMemoryStat::read(&state.hot.limiter);
    drop(guard);
    Some(stat)
}
```

The shard read lock is held only for the duration of `Arc::clone` (or simply field access). No Store contact required; no deadlock possible.

### 3.3 `report-heap` WIT call

Apps can volunteer occupancy:

```wit
// supervisor/wit/vyoma-memory.wit
interface report {
    report-heap: func(live-bytes: u64, free-bytes: u64);
}
```

Implementation:

```rust
// supervisor/src/shm/wit_host.rs (excerpt)
pub fn report_heap(
    store: &mut wasmtime::Store<HostState>,
    live: u64,
    free: u64,
) -> wasmtime::Result<()> {
    let id = store.data().instance_id;
    let table = &store.data().app_table;
    if let Some(state) = table.with_instance_mut(id, |s| {
        *s.hot.reported_heap.lock() = Some(ReportedHeap {
            live_bytes: live, free_bytes: free,
            reported_at: std::time::Instant::now(),
        });
    }) { return Ok(()); }
    Ok(())
}
```

The supervisor uses `reported_heap` for: (a) soft-restart fragmentation detection (§8), (b) jetsam score live-bytes weighting, (c) mgmt observability.

---

## 4. Pressure Monitor — Fixed Design (resolves C4)

The architect's 250 ms `/proc/meminfo` polling has a worst-case 200 ms blind window in which a media app can drive the system into OOM. The fix: **PSI (Pressure Stall Information) is the primary signal**; polling is a fallback for kernels < 4.20.

### 4.1 PSI trigger

Linux 5.10+ supports `/proc/pressure/memory`:

```
$ cat /proc/pressure/memory
some avg10=0.00 avg60=0.00 avg300=0.00 total=12345
full avg10=0.00 avg60=0.00 avg300=0.00 total=678
```

We write a trigger:

```
some 150000 1000000
```

…which means "notify when any task is stalled on memory for ≥ 150 ms within any 1 s window". Then `poll()` / `epoll()` on the fd; `POLLPRI` fires on threshold.

### 4.2 `MemoryPressureMonitor`

```rust
// supervisor/src/memory/pressure_monitor.rs
use std::os::unix::io::{AsRawFd, OwnedFd};
use std::sync::Arc;
use tokio::io::unix::AsyncFd;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum PressureLevel {
    Normal   = 0,
    Warning  = 1,
    Critical = 2,
    Imminent = 3,
}

#[derive(Clone, Copy, Debug)]
pub struct PressureThresholds {
    pub warn_avail_bytes:     u64,
    pub crit_avail_bytes:     u64,
    pub imminent_avail_bytes: u64,
    pub hysteresis_bytes:     u64,
    pub max_victims_per_tick: u32,
    pub psi_some_us:          u64,    // 150_000 = 150 ms
    pub psi_window_us:        u64,    // 1_000_000 = 1 s
}

#[derive(Clone, Copy, Debug)]
pub struct PressureEvent {
    pub from: PressureLevel,
    pub to:   PressureLevel,
    pub mem_available_bytes: u64,
    pub at: std::time::Instant,
}

pub struct MemoryPressureMonitor {
    pub governor:       Arc<MemoryGovernor>,
    pub thresholds:     PressureThresholds,
    pub callbacks:      Arc<PressureCallbackRegistry>,
    pub jetsam:         Arc<JetsamDirector>,
    pub epoch_writer:   crossbeam::channel::Sender<PressureEvent>,
    psi_some_fd:        Option<AsyncFd<OwnedFd>>,
}

impl MemoryPressureMonitor {
    /// Open /proc/pressure/memory, write the trigger, register with epoll.
    /// Falls back to polling on EOPNOTSUPP/ENOENT.
    pub fn open_psi(thresholds: &PressureThresholds) -> std::io::Result<Option<AsyncFd<OwnedFd>>> {
        use std::os::unix::fs::OpenOptionsExt;
        let file = match std::fs::OpenOptions::new()
            .read(true).write(true)
            .custom_flags(libc::O_NONBLOCK)
            .open("/proc/pressure/memory")
        {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e),
        };
        let trigger = format!("some {} {}\0",
            thresholds.psi_some_us, thresholds.psi_window_us);
        use std::io::Write;
        let mut f = file;
        f.write_all(trigger.as_bytes())?;
        let fd: OwnedFd = f.into();
        Ok(Some(AsyncFd::with_interest(fd, tokio::io::Interest::PRIORITY)?))
    }

    pub async fn run(self: Arc<Self>) {
        if let Some(psi) = &self.psi_some_fd {
            // PSI mode (preferred).
            loop {
                match psi.readable_priority().await {
                    Ok(mut guard) => {
                        guard.clear_ready();
                        self.handle_psi_event().await;
                    }
                    Err(e) => {
                        tracing::error!(?e, "PSI epoll error; falling back to polling");
                        self.run_polling().await;
                        return;
                    }
                }
            }
        } else {
            self.run_polling().await;
        }
    }

    /// Polling fallback. 250 ms cadence retained for non-PSI kernels.
    async fn run_polling(self: Arc<Self>) {
        let mut ticker = tokio::time::interval(std::time::Duration::from_millis(250));
        loop {
            ticker.tick().await;
            let avail = Self::read_mem_available_bytes();
            self.maybe_transition(avail).await;
        }
    }

    async fn handle_psi_event(&self) {
        // PSI told us something is stalled. Read MemAvailable and classify.
        let avail = Self::read_mem_available_bytes();
        self.maybe_transition(avail).await;
    }

    async fn maybe_transition(&self, avail: u64) {
        let new_level = self.classify(avail);
        let old_level = self.governor.pressure_level();
        if new_level != old_level && self.transition_allowed(old_level, new_level, avail) {
            self.governor.pressure_level.store(Arc::new(new_level));
            let _ = self.epoch_writer.send(PressureEvent {
                from: old_level, to: new_level,
                mem_available_bytes: avail,
                at: std::time::Instant::now(),
            });
            self.dispatch(old_level, new_level).await;
        }
    }

    fn classify(&self, avail: u64) -> PressureLevel {
        if avail < self.thresholds.imminent_avail_bytes { PressureLevel::Imminent }
        else if avail < self.thresholds.crit_avail_bytes { PressureLevel::Critical }
        else if avail < self.thresholds.warn_avail_bytes { PressureLevel::Warning }
        else { PressureLevel::Normal }
    }

    fn transition_allowed(&self, from: PressureLevel, to: PressureLevel, avail: u64) -> bool {
        if to > from { return true; }    // escalations always allowed
        // Downgrades require hysteresis to prevent flapping.
        let need = match to {
            PressureLevel::Normal   => self.thresholds.warn_avail_bytes     + self.thresholds.hysteresis_bytes,
            PressureLevel::Warning  => self.thresholds.crit_avail_bytes     + self.thresholds.hysteresis_bytes,
            PressureLevel::Critical => self.thresholds.imminent_avail_bytes + self.thresholds.hysteresis_bytes,
            PressureLevel::Imminent => 0,
        };
        avail >= need
    }

    async fn dispatch(&self, _from: PressureLevel, to: PressureLevel) {
        match to {
            PressureLevel::Normal => {
                self.governor.shm_registry.clear_throttle();
            }
            PressureLevel::Warning => {
                self.callbacks.broadcast_warning(to as u8).await;
                self.governor.glyph_cache.trim_to(self.governor.glyph_cache.bytes_cap() / 2);
                self.governor.image_cache.trim_to(self.governor.image_cache.bytes_cap() / 2);
            }
            PressureLevel::Critical => {
                self.callbacks.broadcast_warning(to as u8).await;
                // Trim caches hard.
                self.governor.glyph_cache.clear();
                self.governor.image_cache.trim_to(0);
                // Evict orphaned shared buffers.
                self.governor.shm_registry.evict_orphans();
                // Batched re-sampling suspension loop (R11, resolves S3).
                self.jetsam.handle_critical().await;
            }
            PressureLevel::Imminent => {
                // Jetsam until target watermark is reached.
                self.jetsam.handle_imminent().await;
            }
        }
    }

    fn read_mem_available_bytes() -> u64 {
        // Linux: parse /proc/meminfo's MemAvailable.
        match std::fs::read_to_string("/proc/meminfo") {
            Ok(s) => {
                for line in s.lines() {
                    if let Some(rest) = line.strip_prefix("MemAvailable:") {
                        if let Some(kib_str) = rest.split_whitespace().next() {
                            if let Ok(kib) = kib_str.parse::<u64>() {
                                return kib * 1024;
                            }
                        }
                    }
                }
                0
            }
            Err(_) => 0,
        }
    }
}
```

### 4.3 Per-level actions summary

| Level    | Actions                                                                                  |
|----------|------------------------------------------------------------------------------------------|
| Normal   | Clear throttles                                                                          |
| Warning  | Broadcast `on-memory-warning` (Warning); trim caches to 50%                              |
| Critical | Broadcast `on-memory-warning` (Critical); clear glyph cache; trim image cache to 0; evict orphaned SHM; batched suspend (≤ 5 victims/tick, re-sample after each) |
| Imminent | Jetsam loop: pick highest-score victim, terminate, sleep 50 ms, re-sample; repeat until target |

### 4.4 Hysteresis behavior

`PressureLevel::Critical` requires a 50 MiB recovery margin to drop back to `Warning`. The batched suspend loop (R11) re-reads `MemAvailable` after each suspension, so the loop exits as soon as the watermark is reached — preventing the "suspend everyone, then over-recover" antipattern.

---

## 5. Jetsam Eviction — Fixed Design (resolves C3, S3, S14, S7)

### 5.1 The eviction–serialize circularity

The fundamental bug: `on-suspend` runs in the wasm instance and may call `malloc → memory.grow`. Under `PressureLevel::Critical`, the architect's hook returned `Ok(false)` for non-foreground apps — but suspending apps are by definition not foreground, so `memory.grow` returned `-1`, the allocator panicked, the app crashed with `CrashKind::Trap`, and `StateBlob` was never written.

The fix: **Suspension grant**. When `LifecycleActor` decides to suspend instance X, it sets `X.lifecycle = Suspending` AND grants X an emergency allocation window of `state_blob_bytes + 256 KiB`. `AppLimiter::memory_growing` consumes from this grant unconditionally (bypassing governor and pressure checks) while the grant has remaining bytes.

```rust
// supervisor/src/memory/jetsam.rs
use std::sync::Arc;
use std::time::{Duration, Instant};

pub struct JetsamDirector {
    pub governor:        Arc<MemoryGovernor>,
    pub app_table:       Arc<AppTable>,
    pub lifecycle_tx:    crossbeam::channel::Sender<LifecycleCommand>,
    pub callbacks:       Arc<PressureCallbackRegistry>,
    pub weights:         JetsamWeights,
    pub max_per_tick:    u32,
    pub target_avail:    u64,    // crit_avail + hysteresis
    pub jetsam_target:   u64,    // imminent recovery watermark
}

#[derive(Clone, Copy, Debug)]
pub struct JetsamWeights {
    pub foreground:           i64,    // -100_000_000 default
    pub foreground_lifecycle: i64,    // -50_000_000
    pub background_lifecycle: i64,    // -5_000_000
    pub stopping_lifecycle:   i64,    // -10_000_000  (R19: NEVER interrupt stop)
    pub crashed_lifecycle:    i64,    // +10_000_000
    pub not_started_lifecycle: i64,   // +5_000_000
    pub system_origin:        i64,    // -30_000_000
    pub developer_origin:     i64,    // +5_000_000
    pub user_origin:          i64,    // 0
    pub pin:                  i64,    // -100_000_000
    pub focus_priority_scale: i64,    // 100_000
    pub idle_secs_cap:        i64,    // 3600
    pub idle_secs_scale:      i64,    // 1_000
    pub mib_cost_scale:       i64,    // 1_000
}

impl Default for JetsamWeights {
    fn default() -> Self {
        Self {
            foreground:           -100_000_000,
            foreground_lifecycle:  -50_000_000,
            background_lifecycle:   -5_000_000,
            stopping_lifecycle:    -10_000_000,
            crashed_lifecycle:      10_000_000,
            not_started_lifecycle:   5_000_000,
            system_origin:         -30_000_000,
            developer_origin:        5_000_000,
            user_origin:                     0,
            pin:                  -100_000_000,
            focus_priority_scale:      100_000,
            idle_secs_cap:                3600,
            idle_secs_scale:             1_000,
            mib_cost_scale:              1_000,
        }
    }
}

/// Snapshot of AppState taken at scoring time.
#[derive(Clone, Debug)]
pub struct AppStateSnapshot {
    pub instance:        InstanceId,
    pub bundle:          BundleId,
    pub lifecycle:       LifecycleState,
    pub origin:          InstallOrigin,
    pub focus_priority:  u8,
    pub pin_in_pressure: bool,
    pub last_focus:      Instant,
    pub mem_stat:        WasmMemoryStat,
    pub reported_heap:   Option<ReportedHeap>,
}

/// Higher score = more eligible for eviction. Suspended apps are NOT in this set
/// (handled by a separate compaction pass; resolves S14).
pub fn jetsam_score(
    snap:     &AppStateSnapshot,
    weights:  &JetsamWeights,
    now:      Instant,
    focus_id: Option<InstanceId>,
) -> i64 {
    let mut score: i64 = 0;

    if Some(snap.instance) == focus_id {
        score += weights.foreground;
    }

    match snap.lifecycle {
        LifecycleState::Foreground            => score += weights.foreground_lifecycle,
        LifecycleState::Background            => score += weights.background_lifecycle,
        LifecycleState::Suspending { .. }     => return i64::MIN, // never evict in flight
        LifecycleState::Suspended  { .. }     => return i64::MIN, // separate compaction pass
        LifecycleState::Stopping              => score += weights.stopping_lifecycle, // R19: protect
        LifecycleState::Crashed { .. }        => score += weights.crashed_lifecycle,
        LifecycleState::NotStarted            => score += weights.not_started_lifecycle,
    }

    match snap.origin {
        InstallOrigin::System      => score += weights.system_origin,
        InstallOrigin::User { .. } => score += weights.user_origin,
        InstallOrigin::Developer   => score += weights.developer_origin,
    }

    score += (snap.focus_priority as i64) * weights.focus_priority_scale;

    let idle_secs = now.duration_since(snap.last_focus).as_secs() as i64;
    let idle_secs = idle_secs.min(weights.idle_secs_cap);
    score += idle_secs * weights.idle_secs_scale;

    // Memory cost: prefer live bytes if reported (more honest than capacity).
    let cost_bytes = snap.reported_heap.map(|h| h.live_bytes)
        .unwrap_or(snap.mem_stat.byte_capacity as u64);
    score += (cost_bytes / (1024 * 1024)) as i64 * weights.mib_cost_scale;

    if snap.pin_in_pressure {
        score += weights.pin;
    }

    score
}

impl JetsamDirector {
    /// Snapshot AppTable, score, sort. Returns up to `max` candidates.
    /// (R16, resolves S7.)
    pub fn select_victims(&self, max: u32) -> Vec<(InstanceId, i64)> {
        let mut snapshots = Vec::with_capacity(64);
        self.app_table.iter_snapshot(|state| {
            // Skip non-evictable lifecycles up front to avoid pointless scoring.
            if matches!(state.cold.lifecycle,
                LifecycleState::Suspending { .. } |
                LifecycleState::Suspended  { .. })
            {
                return;
            }
            snapshots.push(AppStateSnapshot {
                instance:        state.instance,
                bundle:          state.cold.identity.bundle_id.clone(),
                lifecycle:       state.cold.lifecycle.clone(),
                origin:          state.cold.identity.origin.clone(),
                focus_priority:  state.cold.identity.focus_priority,
                pin_in_pressure: state.cold.identity.pin_in_pressure,
                last_focus:      state.hot.last_focus,
                mem_stat:        WasmMemoryStat::read(&state.hot.limiter),
                reported_heap:   *state.hot.reported_heap.lock(),
            });
        });
        let now = Instant::now();
        let focus_id = self.governor.focused_instance();
        let mut scored: Vec<(InstanceId, i64)> = snapshots.iter()
            .map(|s| (s.instance, jetsam_score(s, &self.weights, now, focus_id)))
            .collect();
        scored.sort_by_key(|(_, s)| std::cmp::Reverse(*s));
        scored.truncate(max as usize);
        scored
    }

    /// Critical-pressure batched suspend (R11, resolves S3).
    pub async fn handle_critical(&self) {
        let target = self.target_avail;
        for _ in 0..self.max_per_tick {
            let avail = MemoryPressureMonitor::read_mem_available_bytes();
            if avail >= target { break; }
            let victims = self.select_victims(1);
            let Some((id, _)) = victims.first() else { break; };
            self.suspend(*id).await;
            // PSI/poll loop will pick up the new MemAvailable on next event.
        }
    }

    pub async fn handle_imminent(&self) {
        let target = self.jetsam_target;
        for _ in 0..self.max_per_tick {
            let avail = MemoryPressureMonitor::read_mem_available_bytes();
            if avail >= target { break; }
            let victims = self.select_victims(1);
            let Some((id, _)) = victims.first() else { break; };
            self.jetsam(*id).await;
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    /// Cooperative suspension: emit grant, call on-suspend, capture StateBlob,
    /// drop store. (Resolves C3.)
    async fn suspend(&self, id: InstanceId) {
        // 1. Open the suspension grant on the limiter (1 MiB + 256 KiB scratch).
        self.governor.open_suspension_grant(id, SUSPENSION_GRANT_BYTES as u64);
        // 2. Tell LifecycleActor to call on-suspend on the driver thread.
        let _ = self.lifecycle_tx.send(LifecycleCommand::SuspendForMemoryPressure(id));
        // 3. LifecycleActor will: invoke on-suspend, persist StateBlob, drop store,
        //    and call governor.unregister_limiter(id) which releases all committed bytes.
        self.governor.suspend_events.fetch_add(1, Ordering::Relaxed);
    }

    async fn jetsam(&self, id: InstanceId) {
        let _ = self.lifecycle_tx.send(LifecycleCommand::JetsamKill(id));
        self.governor.record_jetsam();
    }
}

impl MemoryGovernor {
    pub fn open_suspension_grant(&self, id: InstanceId, bytes: u64) {
        if let Some(limiter) = self.limiters_by_instance.get(&id) {
            limiter.open_suspension_grant(bytes);
        }
    }
    pub fn focused_instance(&self) -> Option<InstanceId> {
        self.foreground_set.load().iter().next().copied()
    }
}
```

### 5.2 Streaming StateBlob (alternative path)

For apps whose state exceeds 1 MiB but does not warrant `elevated_state = true`, a streaming WIT path lets the app push state in chunks without growing its own memory:

```wit
// supervisor/wit/vyoma-state.wit (extended)
interface state-stream {
    /// Begin streaming a state blob. Returns a sequence number.
    begin: func(total-bytes: u64) -> result<u32, state-error>;
    /// Push a chunk. Chunks accumulate into a supervisor-side buffer.
    push: func(seq: u32, chunk: list<u8>) -> result<_, state-error>;
    /// Commit the blob. Supervisor persists to /data/state/<bundle>.bin.
    commit: func(seq: u32) -> result<_, state-error>;
    /// Cancel and free the buffer.
    cancel: func(seq: u32);
}
```

Apps that choose `state-stream` over the synchronous `on-suspend → StateBlob` path can serialize larger working sets (subject to a 16 MiB streaming cap) without consuming their own linear memory growth budget.

### 5.3 Suspended-apps compaction pass

Suspended apps hold only their persisted `StateBlob` in `/data/state/<bundle>.bin`. They consume no WASM linear memory (store dropped). However, they DO consume `AppState.cold` slot in the `AppTable`. Under sustained pressure, a separate compaction pass GCs Suspended apps older than 30 minutes:

```rust
// supervisor/src/memory/jetsam.rs
impl JetsamDirector {
    /// Runs on a 5-minute timer. Removes Suspended apps whose suspended_at
    /// is older than `compact_after`. Their StateBlob persists on disk
    /// and a fresh launch will resume from it.
    pub async fn compact_suspended(&self, compact_after: Duration) {
        let now = Instant::now();
        let mut to_remove = Vec::new();
        self.app_table.iter_snapshot(|state| {
            if let LifecycleState::Suspended { since } = state.cold.lifecycle {
                if now.duration_since(since) > compact_after {
                    to_remove.push(state.instance);
                }
            }
        });
        for id in to_remove {
            let _ = self.lifecycle_tx.send(LifecycleCommand::CompactSuspended(id));
        }
    }
}
```

This makes the live-app jetsam pool exclusively running apps; killing one always yields real RSS.

---

## 6. Shared Memory (VYOMA_SHM) — Fixed Design (resolves C6, S12)

Two-copy `list<u8>` cannot sustain 60 Hz video. The fix: **`SharedBufferKind::Surface` aliases the supervisor-side `Surface.back` buffer**; the WIT call returns a borrow handle that the app writes through directly, with no intermediate copy.

### 6.1 SharedBuffer types

```rust
// supervisor/src/shm/shared_buffer.rs
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SharedBufferId(pub std::num::NonZeroU64);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SharedBufferKind {
    /// Aliased to a window's Surface.back. Zero-copy write path; compositor reads.
    /// Cap: 64 MiB (matches max surface 4096×4096 RGBA).
    Surface,
    /// SPSC ring of audio frames; producer = app, consumer = mixer. 16 MiB cap.
    AudioBuffer,
    /// Read-only after publish(). 16 MiB cap. For shared decoded images.
    ImageData,
    /// Custom data sharing under capability check. 16 MiB cap.
    Custom,
}

impl SharedBufferKind {
    pub const fn max_bytes(self) -> usize {
        match self {
            Self::Surface     => 64 * 1024 * 1024,   // R13, resolves S12
            Self::AudioBuffer => 16 * 1024 * 1024,
            Self::ImageData   => 16 * 1024 * 1024,
            Self::Custom      => 16 * 1024 * 1024,
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct SharedBufferFlags {
    pub read_only_after_publish: bool,
    pub zero_on_drop:            bool,
    pub pinned:                  bool,
}

/// Storage backing for the buffer. Surface is aliased to a Window's surface;
/// other kinds get a dedicated heap allocation.
pub enum SharedBufferStorage {
    /// Owned Vec; for AudioBuffer, ImageData, Custom.
    Owned(parking_lot::RwLock<Vec<u8>>),
    /// Aliased to a Surface.back. The Surface itself is Arc-shared with the
    /// compositor; writes go directly into Surface.back.
    SurfaceAlias {
        surface: Arc<crate::display::surface::Surface>,
        /// Offset and length within Surface.back permitted to this buffer.
        offset:  usize,
        len:     usize,
    },
}

pub struct SharedBuffer {
    pub id:        SharedBufferId,
    pub owner:     InstanceId,
    pub readers:   parking_lot::Mutex<smallvec::SmallVec<[InstanceId; 4]>>,
    pub kind:      SharedBufferKind,
    pub storage:   SharedBufferStorage,
    pub size:      usize,
    pub created:   std::time::Instant,
    pub last_write: parking_lot::Mutex<std::time::Instant>,
    pub flags:     SharedBufferFlags,
    pub published: std::sync::atomic::AtomicBool,
}

impl SharedBuffer {
    /// Direct write into the backing storage. For SurfaceAlias this writes
    /// straight into Surface.back; the compositor reads from there with no
    /// further copy.
    pub fn write_at(&self, offset: usize, bytes: &[u8]) -> Result<(), ShmError> {
        if self.flags.read_only_after_publish &&
           self.published.load(std::sync::atomic::Ordering::Acquire) {
            return Err(ShmError::ReadOnly);
        }
        let end = offset.checked_add(bytes.len()).ok_or(ShmError::OutOfBounds)?;
        if end > self.size { return Err(ShmError::OutOfBounds); }
        match &self.storage {
            SharedBufferStorage::Owned(rw) => {
                let mut v = rw.write();
                v[offset..offset + bytes.len()].copy_from_slice(bytes);
            }
            SharedBufferStorage::SurfaceAlias { surface, offset: base, .. } => {
                let mut back = surface.back.write();
                back[base + offset .. base + offset + bytes.len()]
                    .copy_from_slice(bytes);
                surface.dirty.store(true, std::sync::atomic::Ordering::Release);
                surface.epoch.fetch_add(1, std::sync::atomic::Ordering::AcqRel);
            }
        }
        *self.last_write.lock() = std::time::Instant::now();
        Ok(())
    }

    pub fn read_at(&self, offset: usize, out: &mut [u8]) -> Result<(), ShmError> {
        let end = offset.checked_add(out.len()).ok_or(ShmError::OutOfBounds)?;
        if end > self.size { return Err(ShmError::OutOfBounds); }
        match &self.storage {
            SharedBufferStorage::Owned(rw) => {
                let v = rw.read();
                out.copy_from_slice(&v[offset..offset + out.len()]);
            }
            SharedBufferStorage::SurfaceAlias { surface, offset: base, .. } => {
                let back = surface.back.read();
                out.copy_from_slice(&back[base + offset .. base + offset + out.len()]);
            }
        }
        Ok(())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ShmError {
    #[error("buffer not found")]                       NotFound,
    #[error("permission denied")]                      Denied,
    #[error("size {0} exceeds per-buffer cap")]        TooLarge(usize),
    #[error("registry budget exhausted")]              BudgetExhausted,
    #[error("offset+len out of bounds")]               OutOfBounds,
    #[error("buffer is read-only after publish")]      ReadOnly,
    #[error("kind does not support requested op")]     UnsupportedKind,
}
```

### 6.2 Registry

```rust
// supervisor/src/shm/registry.rs
pub struct SharedBufferRegistry {
    by_id:       dashmap::DashMap<SharedBufferId, Arc<SharedBuffer>>,
    next_id:     std::sync::atomic::AtomicU64,
    per_owner:   dashmap::DashMap<InstanceId, u32>,
    /// Reference to governor for budget accounting.
    governor:    Arc<MemoryGovernor>,
    /// Per-buffer caps drawn from SharedBufferKind::max_bytes().
    /// Per-owner cap (number of buffers).
    per_owner_cap: u32,    // default 8
}

impl SharedBufferRegistry {
    pub fn create_owned(
        &self,
        owner: InstanceId,
        size:  usize,
        kind:  SharedBufferKind,
        flags: SharedBufferFlags,
    ) -> Result<SharedBufferId, ShmError> {
        if size > kind.max_bytes() { return Err(ShmError::TooLarge(size)); }
        if matches!(kind, SharedBufferKind::Surface) {
            return Err(ShmError::UnsupportedKind);   // use create_surface_alias
        }
        let owner_count = self.per_owner.entry(owner).or_insert(0);
        if *owner_count >= self.per_owner_cap { return Err(ShmError::BudgetExhausted); }

        // Reserve from the SHM budget.
        let prev = self.governor.shm_used_bytes.fetch_add(size as u64, Ordering::AcqRel);
        if prev + (size as u64) > self.governor.shm_budget_bytes.load(Ordering::Acquire) {
            self.governor.shm_used_bytes.fetch_sub(size as u64, Ordering::AcqRel);
            return Err(ShmError::BudgetExhausted);
        }

        let id = SharedBufferId(
            std::num::NonZeroU64::new(self.next_id.fetch_add(1, Ordering::AcqRel) + 1).unwrap());
        let buf = Arc::new(SharedBuffer {
            id, owner, kind, size, flags,
            readers:    parking_lot::Mutex::new(smallvec::SmallVec::new()),
            storage:    SharedBufferStorage::Owned(parking_lot::RwLock::new(vec![0u8; size])),
            created:    std::time::Instant::now(),
            last_write: parking_lot::Mutex::new(std::time::Instant::now()),
            published:  std::sync::atomic::AtomicBool::new(false),
        });
        self.by_id.insert(id, buf);
        *owner_count += 1;
        Ok(id)
    }

    /// Create a Surface-aliased buffer. The buffer's storage IS the Window's
    /// Surface.back. Zero-copy compositor path (R6, resolves C6).
    pub fn create_surface_alias(
        &self,
        owner:   InstanceId,
        surface: Arc<crate::display::surface::Surface>,
        flags:   SharedBufferFlags,
    ) -> Result<SharedBufferId, ShmError> {
        let size = (surface.width as usize) * (surface.height as usize) * 4;
        if size > SharedBufferKind::Surface.max_bytes() {
            return Err(ShmError::TooLarge(size));
        }
        // Surface storage is NOT counted against shm_budget — it's already
        // counted as a per-window surface in the layout. This is intentional:
        // aliasing should not double-charge.
        let id = SharedBufferId(
            std::num::NonZeroU64::new(self.next_id.fetch_add(1, Ordering::AcqRel) + 1).unwrap());
        let buf = Arc::new(SharedBuffer {
            id, owner,
            kind: SharedBufferKind::Surface,
            size, flags,
            readers:    parking_lot::Mutex::new(smallvec::SmallVec::new()),
            storage:    SharedBufferStorage::SurfaceAlias { surface, offset: 0, len: size },
            created:    std::time::Instant::now(),
            last_write: parking_lot::Mutex::new(std::time::Instant::now()),
            published:  std::sync::atomic::AtomicBool::new(false),
        });
        self.by_id.insert(id, buf);
        *self.per_owner.entry(owner).or_insert(0) += 1;
        Ok(id)
    }

    pub fn attach_reader(&self, buf: SharedBufferId, reader: InstanceId) -> Result<(), ShmError> {
        let b = self.by_id.get(&buf).ok_or(ShmError::NotFound)?;
        let mut r = b.readers.lock();
        if !r.contains(&reader) { r.push(reader); }
        Ok(())
    }

    pub fn evict_orphans(&self) {
        let mut to_remove = Vec::new();
        for entry in self.by_id.iter() {
            // Orphan = owner gone AND no readers AND not pinned.
            let owner_alive = self.per_owner.contains_key(&entry.value().owner);
            let no_readers = entry.value().readers.lock().is_empty();
            if !owner_alive && no_readers && !entry.value().flags.pinned {
                to_remove.push(*entry.key());
            }
        }
        for id in to_remove { let _ = self.destroy_internal(id); }
    }

    fn destroy_internal(&self, id: SharedBufferId) -> Result<(), ShmError> {
        if let Some((_, b)) = self.by_id.remove(&id) {
            if let SharedBufferStorage::Owned(_) = b.storage {
                self.governor.shm_used_bytes.fetch_sub(b.size as u64, Ordering::AcqRel);
            }
            if let Some(mut count) = self.per_owner.get_mut(&b.owner) {
                if *count > 0 { *count -= 1; }
            }
        }
        Ok(())
    }

    pub fn clear_throttle(&self) { /* placeholder for future throttle policy */ }
}
```

### 6.3 Audio path: SPSC ring (R6 alternative)

For audio, two-copy `list<u8>` adds jitter. We provide a dedicated SPSC ring kind:

```rust
// supervisor/src/shm/audio_ring.rs
pub struct AudioRing {
    pub frames:  crossbeam::queue::SegQueue<AudioFrame>,   // lock-free SPSC
    pub frame_bytes: usize,
}

#[derive(Clone)]
pub struct AudioFrame {
    pub samples:   Box<[u8]>,
    pub timestamp: u64,
}
```

The WIT path is `vyoma:audio/ring/push-frame(frame: list<u8>, ts: u64)` — one `Vec` allocation per frame on the WIT side; the supervisor pushes a single `AudioFrame` onto the lock-free queue. Mixer thread drains. No registry-managed `SharedBuffer` involvement.

### 6.4 WIT interface — Shared memory

```wit
// supervisor/wit/vyoma-memory.wit
package vyoma:memory@0.1.0;

interface shared {
    type shared-buffer-id = u64;

    enum shared-buffer-kind {
        surface,
        audio-buffer,
        image-data,
        custom,
    }

    record shared-buffer-flags {
        read-only-after-publish: bool,
        zero-on-drop:            bool,
    }

    variant shm-error {
        not-found,
        denied,
        too-large(u32),
        budget-exhausted,
        out-of-bounds,
        read-only,
        unsupported-kind,
    }

    /// For Surface kind, callers pass the window-id; supervisor aliases the
    /// window's Surface.back into the buffer (zero-copy).
    create-surface-buffer: func(window-id: u32, flags: shared-buffer-flags)
        -> result<shared-buffer-id, shm-error>;

    /// For owned kinds (AudioBuffer, ImageData, Custom).
    create-owned: func(
        size:  u32,
        kind:  shared-buffer-kind,
        flags: shared-buffer-flags,
    ) -> result<shared-buffer-id, shm-error>;

    attach: func(id: shared-buffer-id) -> result<_, shm-error>;

    write: func(
        id:     shared-buffer-id,
        offset: u32,
        data:   list<u8>,
    ) -> result<_, shm-error>;

    read: func(
        id:     shared-buffer-id,
        offset: u32,
        len:    u32,
    ) -> result<list<u8>, shm-error>;

    publish: func(id: shared-buffer-id) -> result<_, shm-error>;
    destroy: func(id: shared-buffer-id) -> result<_, shm-error>;
}

interface report {
    report-heap: func(live-bytes: u64, free-bytes: u64);
}
```

For `Surface`-aliased buffers, the `write` call does a single `memcpy` from wasm-linear-memory directly into `Surface.back` — saving the architect's second copy.

### 6.5 Stdout fallback (legacy)

For apps still using Phase 11/12 stdout protocol:

```
VYOMA_SHM:create:<size>:<kind>:<flags>            → VYOMA_SHM_OK:<id>
VYOMA_SHM:create_surface:<window_id>:<flags>      → VYOMA_SHM_OK:<id>
VYOMA_SHM:attach:<id>                              → VYOMA_SHM_OK:<id>
VYOMA_SHM:write:<id>:<offset>:<base64_bytes>      → VYOMA_SHM_OK
VYOMA_SHM:read:<id>:<offset>:<len>                → VYOMA_SHM_DATA:<base64>
VYOMA_SHM:publish:<id>                             → VYOMA_SHM_OK
VYOMA_SHM:destroy:<id>                             → VYOMA_SHM_OK
```

Base64 path is bandwidth-bounded (≤ 32 KiB per message) and intentionally inefficient; serious bytes use WIT.

---

## 7. RSS Budget — Reconciled (resolves C5)

The architect's 834 MiB estimate breaches the 512 MiB `desktop-full` RAM floor. We resolve this by:

1. **Lowering the small-system concurrency target** from "50 apps" to "12 apps" on 512 MiB.
2. **Adopting Wasmtime PoolingAllocator with `MAP_NORESERVE`** so the 50 × 128 MiB linear-memory reservation is virtual, not RSS.
3. **Replacing per-app dedicated 8 MiB driver threads with a shared `tokio::task::spawn_blocking` pool** of CPU-count workers.

### 7.1 Budget at 12 concurrent apps, 512 MiB RAM floor

| Region                                          | Bytes        |
|-------------------------------------------------|--------------|
| Linux kernel                                    | ~50 MiB      |
| Tmpfs root, busybox, wasmtime runtime           | ~30 MiB      |
| Supervisor binary (text+data, musl static)      | ~0.7 MiB     |
| Page cache / slab residual                      | ~30 MiB      |
| Thread stacks (4 tokio workers + 4 blocking)    | ~64 MiB      |
| Framebuffer (mmap, shared w/ GPU)               | ~5 MiB       |
| Surfaces (12 windows × 1.875 MiB back-only*)    | ~22.5 MiB    |
| WASM linear memory (avg 8 MiB × 12, RSS)        | ~96 MiB      |
| WASM Store overhead (12 × 250 KiB)              | ~3 MiB       |
| Compiled code (shared via Engine)               | ~5 MiB       |
| Font glyph cache + atlas                        | ~24 MiB      |
| Image decode cache (cap 16 MiB on small sys)    | ~16 MiB      |
| IPC ring buffers                                | ~1 MiB       |
| Shared buffers                                  | ~8 MiB       |
| Rust global heap residual                       | ~16 MiB      |
| **Estimated RSS total**                         | **~371 MiB** |

*Hidden windows drop the front buffer; only back retained.

371 MiB ≤ 512 MiB − 110 MiB (kernel + tmpfs + slab) = 402 MiB userspace ceiling. **Fits.**

### 7.2 Budget at 50 concurrent apps, 2 GiB RAM (medium-desktop)

| Region                                       | Bytes        |
|----------------------------------------------|--------------|
| Linux kernel                                 | ~80 MiB      |
| Tmpfs root, supervisor binary                | ~30 MiB      |
| Page cache / slab                            | ~80 MiB      |
| Thread stacks (8 tokio + 16 blocking)        | ~96 MiB      |
| Framebuffer                                  | ~5 MiB       |
| Surfaces (avg 30 visible windows × 3.75 MiB) | ~113 MiB     |
| WASM linear memory (avg 12 MiB × 50, RSS)    | ~600 MiB     |
| WASM Store overhead (50 × 250 KiB)           | ~12.5 MiB    |
| Compiled code                                | ~10 MiB      |
| Font glyph cache + atlas                     | ~24 MiB      |
| Image decode cache                           | ~64 MiB      |
| IPC + router                                 | ~4 MiB       |
| Shared buffers                               | ~32 MiB      |
| Rust global heap                             | ~32 MiB      |
| **Estimated RSS total**                      | **~1183 MiB**|

1183 MiB ≤ 2048 − 200 = 1848 MiB userspace. **Fits.**

### 7.3 Wasmtime PoolingAllocator config

```rust
// supervisor/src/runtime/wasm_runtime.rs
pub fn build_engine(profile: &PlatformProfile) -> wasmtime::Result<wasmtime::Engine> {
    let mut config = wasmtime::Config::new();
    config.epoch_interruption(true);
    config.async_support(true);
    config.consume_fuel(false);    // Round 1: epochs replace fuel

    let mut pool = wasmtime::PoolingAllocationConfig::default();
    pool.total_memories(profile.max_concurrent_apps as u32);
    pool.total_tables(profile.max_concurrent_apps as u32);
    pool.max_memory_size((WASM_MAX_PAGES_DEFAULT as usize) * WASM_PAGE_BYTES);
    pool.max_table_elements(WASM_MAX_TABLE_ELEMENTS as usize);

    // Lazy commit: virtual reservation, physical pages on demand.
    pool.linear_memory_keep_resident(0);
    pool.table_keep_resident(0);

    config.allocation_strategy(wasmtime::InstanceAllocationStrategy::Pooling(pool));
    wasmtime::Engine::new(&config)
}
```

On `desktop-full` with `max_concurrent_apps = 50`, this reserves 50 × 128 MiB = 6.4 GiB of *virtual* address space at startup. RSS is bounded by `wasm_budget_bytes`. On the 512 MiB floor system, `max_concurrent_apps = 12` and the virtual reservation is 1.5 GiB — well under the 47-bit user-virtual range.

### 7.4 Shared driver-thread pool

Round 1's architecture spawned a dedicated thread per running app (one app = one driver). At 50 apps × 8 MiB stack = 400 MiB just for stacks. We replace this with `tokio::task::spawn_blocking` against a pool sized at `2 × num_cpus`:

```rust
// supervisor/src/runtime/driver_pool.rs
pub struct DriverPool {
    runtime: tokio::runtime::Runtime,
}

impl DriverPool {
    pub fn build(num_workers: usize) -> std::io::Result<Self> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(num_workers)
            .max_blocking_threads(2 * num_workers)
            .thread_stack_size(2 * 1024 * 1024)    // 2 MiB per thread, not 8
            .enable_all()
            .build()?;
        Ok(Self { runtime })
    }

    /// Run a Store call on the blocking pool. Each call enters Wasmtime
    /// via call_async; epoch interruption fires on the supervisor's epoch tick.
    pub fn spawn_app_driver<F>(&self, f: F) -> tokio::task::JoinHandle<()>
    where F: FnOnce() + Send + 'static
    {
        self.runtime.spawn_blocking(f)
    }
}
```

At 16 worker threads × 2 MiB = 32 MiB. Each Store call is short (one host call or one epoch slice); the blocking pool is shared across all apps.

### 7.5 `CLAUDE.md` update

The synthesis updates the `desktop-full` platform row in CLAUDE.md to clarify:

> `desktop-full` | x86-64 workstation (default) | Wasmtime JIT | 512 MB floor (≤ 12 concurrent apps); ≥ 2 GiB recommended for 50-app target

---

## 8. Large Object Handling

### 8.1 Font atlas

The font atlas is 2048×2048 RGBA = 16 MiB, allocated **once** during `InitDisplay` and pre-registered with the governor:

```rust
// supervisor/src/text/atlas.rs
pub struct FontAtlas {
    pub width:  u32,    // 2048
    pub height: u32,    // 2048
    pub bytes:  parking_lot::RwLock<Box<[u8]>>,
    pub residency: parking_lot::Mutex<lru::LruCache<GlyphKey, AtlasSlot>>,
}

impl FontAtlas {
    pub fn new(governor: &MemoryGovernor) -> Self {
        let size = 2048 * 2048 * 4;
        let atlas = Self {
            width: 2048, height: 2048,
            bytes: parking_lot::RwLock::new(vec![0u8; size].into_boxed_slice()),
            residency: parking_lot::Mutex::new(lru::LruCache::new(
                std::num::NonZeroUsize::new(8192).unwrap())),
        };
        governor.pre_register("font_atlas", size);   // R10, resolves S8
        atlas
    }
}
```

**One atlas, system-wide.** When the atlas fills, LRU eviction reclaims slots. Per-app private atlases are NOT supported; this keeps RAM bounded. Apps with exotic font needs (BiDi, CJK, emoji) share the system atlas; rendering goes through `vyoma:text/draw` which manages atlas residency.

### 8.2 Framebuffer

```rust
// supervisor/src/display/framebuffer.rs
pub struct Framebuffer {
    fd:          std::os::unix::io::OwnedFd,
    pub width:   u32,
    pub height:  u32,
    pub stride:  u32,
    pub bpp:     u32,
    pub bytes:   std::ptr::NonNull<u8>,    // mmap; SAFETY documented in comment
    pub len:     usize,
}

// SAFETY: Framebuffer is Send + Sync because:
// (a) mmap is shared with the GPU which serializes via DRM page-flip events;
// (b) writes from the compositor are serialized by Compositor's flush lock.
unsafe impl Send for Framebuffer {}
unsafe impl Sync for Framebuffer {}
```

Allocated once during `InitDisplay`; pre-registered with `governor.pre_register("framebuffer", w*h*4)`.

### 8.3 Streaming API — pull + read-at (R15, resolves S11)

```wit
// supervisor/wit/vyoma-fs.wit (excerpt)
package vyoma:fs@0.2.0;

interface streaming {
    type stream-handle = u64;

    record open-options {
        path:       string,
        chunk-size: u32,    // [4 KiB, 4 MiB], default 256 KiB
        read-ahead: u32,    // [0, 8], default 2
    }

    variant fs-error {
        not-found,
        permission-denied,
        io-error(string),
        invalid-offset,
        budget-exhausted,
        closed,
    }

    open:  func(opts: open-options) -> result<stream-handle, fs-error>;

    /// Sequential pull. Returns up to chunk-size bytes; None on EOF.
    pull:  func(h: stream-handle) -> result<option<list<u8>>, fs-error>;

    /// Random-access read. Backed by pread() or mmap; does NOT mutate stream
    /// position and does NOT invalidate read-ahead.
    read-at: func(h: stream-handle, offset: u64, len: u32)
        -> result<list<u8>, fs-error>;

    /// Sequential seek; invalidates read-ahead.
    seek:  func(h: stream-handle, offset: u64) -> result<_, fs-error>;
    close: func(h: stream-handle);
}
```

Implementation:

```rust
// supervisor/src/fs/streaming.rs
pub struct StreamingHandle {
    file:        tokio::fs::File,
    /// If true, supervisor mmap'd the file; read-at uses memcpy from mapping.
    mmap:        Option<memmap2::Mmap>,
    chunk_size:  usize,
    read_ahead:  u32,
    cursor:      u64,
    governor:    Arc<MemoryGovernor>,
    ahead_buf:   tokio::sync::Mutex<std::collections::VecDeque<Vec<u8>>>,
}

impl StreamingHandle {
    /// pread-based read-at; does not mutate cursor.
    pub async fn read_at(&self, offset: u64, len: u32) -> Result<Vec<u8>, FsError> {
        // Reserve host_io budget for the response buffer.
        let bytes = len as usize;
        if !self.governor.try_reserve_host_io(bytes) { return Err(FsError::BudgetExhausted); }
        let buf = if let Some(mmap) = &self.mmap {
            let end = (offset as usize).checked_add(bytes).ok_or(FsError::InvalidOffset)?;
            if end > mmap.len() { return Err(FsError::InvalidOffset); }
            mmap[offset as usize..end].to_vec()
        } else {
            use std::os::unix::fs::FileExt;
            let std_file = self.file.try_clone().await?.into_std().await;
            let mut buf = vec![0u8; bytes];
            let n = tokio::task::spawn_blocking(move || std_file.read_at(&mut buf, offset))
                .await
                .map_err(|e| FsError::IoError(e.to_string()))?
                .map_err(|e| FsError::IoError(e.to_string()))?;
            buf.truncate(n);
            buf
        };
        self.governor.release_host_io(bytes);
        Ok(buf)
    }
}
```

Read-ahead lives in `ahead_buf` (governor host_io budget). On `seek`, the deque is cleared and re-warmed at the new offset.

### 8.4 Large in-app buffers

If an app needs a single buffer > 4 MiB inside its linear memory, the allocator (`dlmalloc`) issues `memory.grow` to satisfy. `MemoryGovernor` admits or denies based on policy (§1.3, §2.1). No special path; the suspension grant is the only bypass.

---

## 9. Memory Safety

### 9.1 Inter-app isolation

**Can app A read app B's linear memory?** No.

Each app has its own `wasmtime::Store`; each `Store` contains its own `wasmtime::Memory`. WASM bounds-checks are encoded at compile time by Cranelift into either explicit `cmp+jne` per access or virtual-memory traps (signals backend: Wasmtime reserves 8 GiB per memory, OOB lands in unmapped pages → SIGSEGV → trap).

There is no instruction in `wasm32` that can address memory outside its `Memory`. There is no host import we expose that takes a raw host pointer. The only paths bytes cross from A to B:

- IPC messages (Round 1): bytes copied at the WIT boundary into supervisor `IpcEnvelope`.
- Shared buffers (§6): for `Surface` kind, A writes into a buffer that B can read; for `Custom`, capability-checked.
- File system (`/data` 9P): mediated by the kernel.

### 9.2 Supervisor → WASM-store writes

The architect's `WasmMemoryView` snapshot-guard pattern was redundant: Wasmtime's `Memory::write(&mut Store, offset, &[u8])` already does the bounds check internally, and the `&mut Store` borrow proves no concurrent grow. The synthesis replaces the guard with thin free functions (R12, resolves S5):

```rust
// supervisor/src/runtime/memory_io.rs
use wasmtime::{Memory, Store};

#[derive(Debug, thiserror::Error)]
pub enum WasmIoError {
    #[error("offset+len overflow")]                 Overflow,
    #[error("offset {offset}..{} exceeds memory size {cap}", offset + len)]
    OutOfBounds { offset: usize, len: usize, cap: usize },
    #[error("wasmtime: {0}")]                       Wasmtime(#[from] anyhow::Error),
}

pub fn write_to_wasm<T>(
    store:  &mut Store<T>,
    memory: Memory,
    offset: u32,
    bytes:  &[u8],
) -> Result<(), WasmIoError> {
    let offset = offset as usize;
    let end = offset.checked_add(bytes.len()).ok_or(WasmIoError::Overflow)?;
    let cap = memory.data_size(&*store);
    if end > cap { return Err(WasmIoError::OutOfBounds { offset, len: bytes.len(), cap }); }
    memory.write(store, offset, bytes).map_err(Into::into)
}

pub fn read_from_wasm<T>(
    store:  &Store<T>,
    memory: Memory,
    offset: u32,
    out:    &mut [u8],
) -> Result<(), WasmIoError> {
    let offset = offset as usize;
    let end = offset.checked_add(out.len()).ok_or(WasmIoError::Overflow)?;
    let cap = memory.data_size(store);
    if end > cap { return Err(WasmIoError::OutOfBounds { offset, len: out.len(), cap }); }
    memory.read(store, offset, out).map_err(Into::into)
}
```

A code-review rule (enforced by `cargo deny` custom lint) prohibits calling `Memory::data_mut` / `Memory::data_ptr` anywhere except inside `memory_io.rs`.

### 9.3 Stack overflow

Wasmtime detects WASM stack overflow via guard pages (signals backend) or explicit stack-check (default). It traps with `TrapCode::StackOverflow`. The supervisor's classifier maps to `CrashKind::StackOverflow`:

```rust
// supervisor/src/lifecycle/crash.rs (additions)
pub enum CrashKind {
    // ... Round 1 variants ...
    StackOverflow  { fn_index: Option<u32> },
    Jetsamed       { pressure: PressureLevel, score: i64 },
    ShmExhausted   { requested: usize, budget_remaining: u64 },
    SoftRestart    { reason: SoftRestartReason },
}

#[derive(Clone, Copy, Debug)]
pub enum SoftRestartReason {
    Fragmentation { current_pages: u32, reported_live_mib: u32 },
    Manual,
}
```

`RestartDecision::for_kind(StackOverflow)` is "restart with exponential backoff" (load-dependent bug, may recur). For `Jetsamed`, the policy is "restart only if foreground reactivates" — no automatic restart in the background.

### 9.4 Compositor/eviction race (resolves S6)

The architect's surface eviction protocol was vague. Synthesis:

```rust
// supervisor/src/display/surface.rs (extended)
pub struct Surface {
    pub width:  u32,
    pub height: u32,
    pub back:   parking_lot::RwLock<Box<[u8]>>,
    /// Front buffer; arc_swap so the compositor reads lock-free.
    /// On hidden/minimized, eviction stores `Arc::new(EMPTY_FRONT)`.
    pub front:  arc_swap::ArcSwap<Box<[u8]>>,
    pub dirty:  std::sync::atomic::AtomicBool,
    pub epoch:  std::sync::atomic::AtomicU64,
}

pub static EMPTY_FRONT: once_cell::sync::Lazy<Box<[u8]>> =
    once_cell::sync::Lazy::new(|| Box::new([]));

impl Surface {
    /// Swap back → front. Called by the app on flush.
    pub fn swap(&self) {
        let back = self.back.read().clone();    // single clone
        self.front.store(Arc::new(back));
        self.epoch.fetch_add(1, Ordering::AcqRel);
        self.dirty.store(false, Ordering::Release);
    }

    /// Drop the front buffer (hidden/minimized window). Compositor will see
    /// an empty front next time and skip blitting.
    pub fn drop_front(&self) {
        self.front.store(Arc::new(EMPTY_FRONT.clone()));
        self.dirty.store(false, Ordering::Release);
    }
}
```

Compositor reads `self.front.load()` which is `arc_swap::Guard<Arc<Box<[u8]>>>` — held for the duration of the blit. If eviction stores `EMPTY_FRONT` mid-blit, the compositor's guard keeps the OLD front alive until the guard drops. No use-after-free; current frame draws correctly; next frame draws nothing. Resolves S6.

`epoch` is monotonically increasing; compositor cache invalidation uses `compare_exchange` semantics (Acquire on read, Release on write). `dirty` uses `Release` on write, `Acquire` on read.

### 9.5 Re-entrant grow inside host call

Wasmtime documents that calling `Memory::grow` from inside a host call while holding `&mut Store` is OK, but it invalidates any prior `data_ptr()`. Our `write_to_wasm` reads `memory.data_size(&*store)` immediately before `memory.write`; there is no opportunity for a re-entrant grow between the size check and the write because both happen under the same `&mut store` borrow.

For host functions that grow on behalf of the app (rare, only `vyoma:state/stream` which buffers), the grow happens BEFORE any subsequent write, and we re-read `data_size` after.

### 9.6 Use-after-free of SHM handles

Handles are `u64`. When an app's `Instance` terminates, the supervisor:

1. Walks `SharedBufferRegistry.per_owner[instance]` and decrements per-buffer reader counts where this instance is a reader.
2. Decrements `per_owner_cap` count.
3. For owned buffers: the buffer becomes orphaned (`owner_alive = false`). The next `evict_orphans()` reclaims it.
4. For Surface-aliased buffers: the underlying `Surface` is also owned by the now-dead instance's window; the WindowManager drops the Surface, the `Arc<SharedBuffer>` weak references resolve to dropped, registry removes on next sweep.

Any stashed `u64` in another app's memory is just an integer; the next `attach()` / `read()` / `write()` returns `ShmError::NotFound`.

### 9.7 Engine memory accounting (resolves G2)

Cranelift compilation can spike to 50+ MiB per module. We:

1. Bound concurrent compiles to 1 module at a time (`tokio::sync::Semaphore` with permits = 1).
2. Block new module installs under `PressureLevel::Critical` (return `InstallError::PressureCritical`).
3. Pre-register a `wasmtime_compile_arena` scratch budget of 64 MiB with the governor.

```rust
// supervisor/src/runtime/compile_arena.rs
pub struct CompileArena {
    pub permit:   tokio::sync::Semaphore,
    pub budget:   AtomicU64,    // 64 MiB
}

impl CompileArena {
    pub async fn acquire_for_install(&self, governor: &MemoryGovernor)
        -> Result<tokio::sync::SemaphorePermit<'_>, CompileError>
    {
        if matches!(governor.pressure_level(), PressureLevel::Critical | PressureLevel::Imminent) {
            return Err(CompileError::PressureBackoff);
        }
        self.permit.acquire().await.map_err(|_| CompileError::Closed)
    }
}
```

---

## 10. Debug & Observability API

### 10.1 Stdout `mgmt:` commands

```
mgmt: memstat <bundle>       → MEMSTAT:<bundle>:<current_pages>:<peak_pages>:<grow_denied>:<reported_live>:<reported_free>
mgmt: memmap                 → MEMMAP:<json blob>
mgmt: pressure <level>       → PRESSURE_OK:<level>    (gated: see R21)
mgmt: shmstat                → SHMSTAT:<count>:<total_bytes>:<budget>:<orphans>
mgmt: jetsam-rank            → JETSAM_RANK:<json list of (bundle, score, bytes)>
mgmt: governor               → GOVERNOR:<json snapshot>
mgmt: soft-restart <bundle>  → SOFT_RESTART_OK
```

### 10.2 WIT management interface

```wit
// supervisor/wit/vyoma-mgmt.wit (memory section)
interface memory-mgmt {
    record memory-snapshot {
        bundle:               string,
        current-pages:        u32,
        peak-pages:           u32,
        byte-capacity:        u64,
        grow-denied:          u64,
        grow-attempted:       u64,
        last-attempted-pages: u32,    // for crash-report enrichment (S15)
        reported-live-bytes:  option<u64>,
        reported-free-bytes:  option<u64>,
    }

    record region-stat {
        label:        string,
        virt-bytes:   u64,
        rss-bytes:    u64,
    }

    record memory-map {
        binary:           region-stat,
        framebuffer:      region-stat,
        font-cache:       region-stat,
        font-atlas:       region-stat,
        image-cache:      region-stat,
        shared-buffers:   region-stat,
        ipc-buffers:      region-stat,
        thread-stacks:    region-stat,
        wasm-stores-total: region-stat,
        compile-arena:    region-stat,
        global-heap:      region-stat,
        estimated-rss:    u64,
    }

    record governor-snapshot {
        wasm-budget:           u64,
        wasm-used:             u64,
        static-reserved:       u64,
        shm-budget:            u64,
        shm-used:              u64,
        host-io-budget:        u64,
        host-io-used:          u64,
        pressure:              u8,
        jetsam-events-total:   u64,
        jetsam-events-last-5s: u32,
        suspend-events-total:  u64,
        soft-restart-total:    u64,
    }

    memstat:      func(bundle: string)  -> option<memory-snapshot>;
    memmap:       func()                -> memory-map;
    governor:     func()                -> governor-snapshot;
    jetsam-rank:  func()                -> list<tuple<string, s64, u64>>;
    set-pressure: func(level: u8)       -> bool;    // gated; R21
}
```

### 10.3 `on-memory-warning` callback (resolves S13)

```wit
// supervisor/wit/vyoma-callbacks.wit (extended)
interface callbacks {
    /// Called on level transition Warning ↔ Critical. NOT called every tick.
    /// Synchronous on the driver thread, 100 ms budget. Returns bytes-released
    /// (the app's best estimate; advisory only). Re-entrant safe: an app
    /// already in Suspending state receives no callback.
    on-memory-warning: func(level: u8, available-mib: u32)
        -> result<u64, callback-error>;

    /// Called when the supervisor decides to soft-restart this app due to
    /// fragmentation. The app should serialize critical state to StateBlob
    /// and return promptly. Budget: 500 ms.
    on-soft-restart: func(reason: u8) -> result<_, callback-error>;
}
```

Implementation contract:

- Fires on level transitions only (Normal→Warning, Warning→Critical, and the recovery downgrades).
- Total budget: 100 ms. Enforced by `tokio::time::timeout`.
- Returns `bytes_released` for telemetry (not enforcement).
- An app whose lifecycle tag is `Suspending` or `Suspended` is skipped.
- Re-entrant: if a transition happens while a callback is in flight, the new transition's callback is queued (single-slot queue per app; if a transition supersedes, the queued slot is overwritten).

### 10.4 Heartbeat extension (resolves G12)

```json
{
  "ts": 1730000000000,
  "memory": {
    "pressure_level": "Normal",
    "mem_available_mib": 612,
    "total_wasm_pages": 6400,
    "total_surface_bytes": 197283840,
    "shared_buffer_count": 12,
    "shared_buffer_bytes": 18874368,
    "jetsam_events_total": 0,
    "jetsam_events_5s": 0,
    "jetsam_storm": false,
    "suspend_events_total": 4,
    "soft_restart_total": 0,
    "warning_callbacks_sent": 4,
    "host_io_used_mib": 2
  }
}
```

`jetsam_storm` is true when `jetsam_events_5s >= 5`. Observability layer can alert on the flag.

### 10.5 Crash report enrichment (resolves S15)

When `CrashKind::MemLimit`, `CrashKind::Jetsamed`, `CrashKind::StackOverflow`, or `CrashKind::ShmExhausted` fires, the crash report (Round 1 §5) is enriched:

```toml
# /data/crashes/<id>.toml
[crash]
bundle  = "com.example.notes"
instance = 12
kind    = "MemLimit"
at      = "2026-05-29T12:34:56Z"

[crash.memory_snapshot]
current_pages         = 127
peak_pages            = 128
byte_capacity         = 8323072
grow_denied           = 1
grow_attempted        = 87
last_attempted_pages  = 256    # the failing grow target (resolves S15)
reported_live_bytes   = 4194304
reported_free_bytes   = 1048576
pressure_at_crash     = "Critical"
governor_wasm_used    = 268435456
governor_wasm_budget  = 281474976
```

Diagnosis becomes trivial: "127 pages was the state; the app attempted to grow to 256; system was in Critical pressure; budget was 96% used → policy denied".

### 10.6 `set-pressure` capability gate (resolves G11)

```toml
# vyoma.toml of a dev/test app
[capabilities]
mgmt = true
mgmt.allow_pressure_injection = true   # narrow gate (R21)
```

Only manifests carrying this flag may invoke `set-pressure`. The flag is rejected during manifest validation if `InstallOrigin != Developer || System`.

---

## 11. Implementation Files

All paths under `supervisor/src/` unless noted. Every file ≤ 500 LOC per the CLAUDE.md ceiling.

| File | LOC | Purpose |
|------|-----|---------|
| `memory/mod.rs` | 80 | module wiring; re-exports `PressureLevel`, `MemoryGovernor`, `WasmMemoryStat`, `ReservationToken` |
| `memory/governor.rs` | 460 | `MemoryGovernor`, `ReservationToken` RAII, `try_reserve`, `pre_register`, `set_foreground`, `set_lifecycle`, snapshot |
| `memory/pressure.rs` | 140 | `PressureLevel`, `PressureThresholds`, `PressureEvent` types + classify/transition_allowed |
| `memory/pressure_monitor.rs` | 380 | `MemoryPressureMonitor` PSI + polling fallback + dispatch dispatcher |
| `memory/jetsam.rs` | 480 | `JetsamDirector`, `JetsamWeights`, `AppStateSnapshot`, `jetsam_score`, suspend/jetsam/compact loops |
| `memory/layout.rs` | 320 | `SupervisorMemoryLayout`, `RegionStat`, `/proc/self/smaps` reader for `mgmt: memmap` |
| `memory/stat.rs` | 80 | `WasmMemoryStat` reader (over `Arc<AppLimiter>`) |
| `memory/callback_registry.rs` | 220 | `PressureCallbackRegistry` — broadcasts `on-memory-warning` to driver threads with timeout |
| `shm/mod.rs` | 50 | module wiring |
| `shm/shared_buffer.rs` | 280 | `SharedBuffer`, `SharedBufferKind`, `SharedBufferStorage` (Owned vs SurfaceAlias), `SharedBufferFlags`, lifecycle |
| `shm/registry.rs` | 460 | `SharedBufferRegistry` — create_owned, create_surface_alias, attach, write, read, publish, destroy, evict_orphans |
| `shm/audio_ring.rs` | 180 | SPSC ring path for audio frames (lock-free crossbeam SegQueue) |
| `shm/wit_host.rs` | 320 | WIT `vyoma:memory/shared` + `vyoma:memory/report` host implementation |
| `runtime/memory_io.rs` | 120 | `write_to_wasm`, `read_from_wasm` free functions (replaces `WasmMemoryView`) |
| `runtime/compile_arena.rs` | 140 | Cranelift concurrent-compile limiter; pressure-aware install gate |
| `runtime/driver_pool.rs` | 160 | `DriverPool` — shared `tokio::spawn_blocking` pool for app drivers |
| `display/surface.rs` (modified) | +60 | `Surface::drop_front`, ArcSwap front buffer with `EMPTY_FRONT` sentinel |
| `resource_limiter.rs` (modified) | +180 | extend `AppLimiter` with `is_foreground`, `lifecycle_tag`, `suspension_grant`, `grow_attempted`, `last_attempted_pages`, `LimiterView` shim |
| `lifecycle/crash.rs` (modified) | +80 | add `StackOverflow`, `Jetsamed`, `ShmExhausted`, `SoftRestart` to `CrashKind`; restart policy updates |
| `manifest.rs` (modified) | +120 | parse + validate `[memory]` block; `pin_in_pressure`, `allow_shared_buffers`, `elevated_state`, `[memory.streaming]`, `mgmt.allow_pressure_injection` |
| `observability/heartbeat.rs` (modified) | +70 | emit memory section + jetsam rate + storm flag (§10.4) |
| `mgmt_handlers.rs` (modified) | +110 | memstat / memmap / governor / jetsam-rank / soft-restart / set-pressure |
| `boot.rs` (modified) | +40 | reorder phases: LoadProfile → InitRegistries (governor) → InitDisplay (pre-register atlas+fb) → SpawnPressureMonitor → LaunchInstalled |

**Totals:**

- 16 new files: ~3,870 LOC.
- 7 modified files: ~660 added LOC.
- Every file ≤ 500 LOC.

WIT files (new or extended):

| File | Purpose |
|------|---------|
| `wit/vyoma-memory.wit` (new) | `vyoma:memory/shared` (create-surface-buffer, create-owned, attach, write, read, publish, destroy) + `vyoma:memory/report` (report-heap) |
| `wit/vyoma-state.wit` (extend) | `state-stream` interface (begin, push, commit, cancel) for streaming StateBlob |
| `wit/vyoma-callbacks.wit` (extend) | `on-memory-warning`, `on-soft-restart` |
| `wit/vyoma-mgmt.wit` (extend) | `memory-mgmt` (memstat, memmap, governor, jetsam-rank, set-pressure) |
| `wit/vyoma-fs.wit` (extend) | `streaming` adds `read-at` |

### 11.1 Test plan

| Test file | Scope |
|-----------|-------|
| `supervisor/tests/memory_limiter_test.rs` | `AppLimiter::memory_growing` per-instance cap, suspension grant, pressure admission |
| `supervisor/tests/governor_token_test.rs` | `try_reserve` returns Some/None correctly; commit retains; drop releases; panic-safety |
| `supervisor/tests/governor_concurrency_test.rs` | 32 threads concurrent `try_reserve` against tight budget; assert no over-budget observable |
| `supervisor/tests/pressure_psi_test.rs` | PSI fd opens, trigger written, fake epoll event drives transition; falls back to polling on fd unavailable |
| `supervisor/tests/pressure_hysteresis_test.rs` | available-memory series → no flapping |
| `supervisor/tests/jetsam_score_test.rs` | golden tests for `jetsam_score` across 16 scenarios (foreground, suspended, pinned, etc.) |
| `supervisor/tests/jetsam_select_snapshot_test.rs` | `select_victims` consistency under concurrent AppTable writes |
| `supervisor/tests/jetsam_critical_batch_test.rs` | batched suspend loop respects max_per_tick + re-samples after each |
| `supervisor/tests/suspension_grant_test.rs` | on-suspend can allocate after grant opens; grant reclaimed on store drop; no leak |
| `supervisor/tests/shm_registry_test.rs` | create_owned + create_surface_alias + attach + write + read + destroy + lifetime + orphan eviction |
| `supervisor/tests/shm_surface_alias_test.rs` | write through Surface-aliased SHM lands directly in Surface.back (no extra copy) |
| `supervisor/tests/audio_ring_test.rs` | SPSC ring lock-free producer/consumer at 48 kHz / 1024-frame buffers |
| `supervisor/tests/memory_io_test.rs` | `write_to_wasm` / `read_from_wasm` reject overflow + OOB |
| `supervisor/tests/lock_order_test.rs` | every pairing in reverse order checked via `parking_lot` deadlock detector |
| `supervisor/tests/wit_memory_host_test.rs` | end-to-end WIT shared-buffer roundtrip between two synthetic instances |
| `supervisor/tests/heartbeat_memory_test.rs` | memory section appears in heartbeat JSON, jetsam_storm transitions true/false |
| `supervisor/tests/crash_report_memory_test.rs` | MemLimit crash report includes last_attempted_pages, pressure_at_crash, governor snapshot |
| `supervisor/tests/soft_restart_test.rs` | fragmentation ratio crosses threshold → on-soft-restart called; StateBlob preserved; new instance gets state |
| `supervisor/tests/pressure_callback_timeout_test.rs` | on-memory-warning callback budget 100 ms enforced; timeout records but does not crash app |

### 11.2 Lock-order discipline (extends Round 1)

```
1. Engine                                 (no locks)
2. MemoryGovernor atomics                 (lock-free)
3. AppLimiter atomics                     (lock-free)
4. AppTable shard RwLocks                 (RwLock; brief)
5. AppState.cold                          (RwLock; per-instance)
6. AppState.hot                           (RwLock; per-instance)
7. SharedBufferRegistry.by_id (DashMap)   (per-entry; never held across WASM call)
8. SharedBuffer.storage RwLock            (always last in chain)
9. Surface.back RwLock / Surface.front ArcSwap   (only via the compositor or SHM write)
10. ipc inbox bounded queues              (lock-free crossbeam)
```

The `lock_order_test.rs` attempts every reverse pairing with `parking_lot` deadlock detection enabled; failure is a CI break.

### 11.3 Boot ordering (resolves S8)

```
KernelHandoff      → no memory action
MountData          → open /proc/meminfo, /proc/pressure/memory
LoadProfile        → read [memory] from profile.toml
InitRegistries     → construct MemoryGovernor (budgets known)
                     construct SharedBufferRegistry, AppTable
InitDisplay        → mmap framebuffer; governor.pre_register("framebuffer", w*h*4)
                     allocate font atlas; governor.pre_register("font_atlas", 16 MiB)
                     allocate font glyph cache; governor.pre_register("glyph_cache", 8 MiB)
                     allocate image decode cache; governor.pre_register("image_cache", 16/32 MiB)
SpawnPressureMonitor → opens PSI fd; spawns tokio task
SpawnCompositor    → ArcSwap surface readers ready
LaunchInstalled    → for each app, build Arc<AppLimiter>; governor.register_limiter
```

The governor exists before any static allocation occurs that we care to budget. All pre-registered allocations show up in both `static_reserved_bytes` and `wasm_used_bytes`.

---

## Critical v1 Requirements

These must ship in v1.0 of the VyomaOS desktop OS:

1. **PSI-based pressure monitor** with polling fallback (Linux < 4.20 only).
2. **`MemoryGovernor` with RAII `ReservationToken`** — never overshoots budget, panic-safe.
3. **`Arc<AppLimiter>` in `AppState.hot`** — telemetry readable from any thread without Store contact.
4. **Suspension grant** — `on-suspend` can allocate `state_blob_bytes + 256 KiB` even under Critical.
5. **`SharedBufferKind::Surface` zero-copy aliasing** — Surface.back IS the SHM buffer; video at 60 Hz achievable.
6. **Per-instance `max_pages` cap** + **global wasm budget** + **pressure admission** — three guards composed in one atomic decision.
7. **Pre-registration of static allocations** at `InitDisplay` so atlas + framebuffer count against budget from boot.
8. **Batched re-sampling suspend loop** (`max_per_tick = 5`); MemAvailable re-read after each suspension.
9. **Jetsam scoring with `profile.toml`-configurable weights**; Suspended apps excluded from live scoring.
10. **`on-memory-warning` callback** with 100 ms budget, fires on level transitions only.
11. **Streaming I/O `read-at(offset, len)`** via `pread()` for random-access media workloads.
12. **`AtomicBool is_foreground` on `AppLimiter`** updated by focus manager — no Arc clone per grow.
13. **`MemAvailable` parsed from `/proc/meminfo`** for classification; PSI is the wakeup signal.
14. **Soft-restart on fragmentation**: `current_pages / reported_live_bytes > 4×` for > 1 h triggers `on-soft-restart`.
15. **Wasmtime PoolingAllocator** with `linear_memory_keep_resident(0)` — virtual reservation, RSS on demand.
16. **Shared `tokio::spawn_blocking` driver pool** (size `2 × num_cpus`, 2 MiB stack) — no per-app dedicated thread.
17. **Crash report enrichment** with `WasmMemoryStat`, `last_attempted_pages`, `pressure_at_crash`, `GovernorSnapshot`.
18. **Compile arena** (single-permit semaphore + pressure gate); blocks new module installs under Critical.
19. **Lock-order test** (`lock_order_test.rs`) — every reverse pairing fails under `parking_lot` deadlock detector.
20. **Heartbeat memory section** with `jetsam_storm` flag (`jetsam_events_5s >= 5`).

---

## Deferred to v2

- **GPU VRAM accounting** — virtio-gpu has its own VRAM concept; the spec treats the GPU as a black box covered by the framebuffer mmap. Subsystem #12 (GPU Acceleration Layer) will model `gpu_vram_used`.
- **NUMA awareness** — desktop is single-socket; revisit only for multi-socket server targets.
- **Persistent memory (pmem)** — out of scope.
- **Memory ballooning to host hypervisor** — VyomaOS is not assumed to be a guest of another VyomaOS.
- **Per-process address space sharing (wasm-threads)** — depends on `wasi-threads` proposal stabilization in `wasip3` or later.
- **Memory compression (Linux zram, macOS-style swap compressor)** — explicit roadmap item but not v1.
- **Adaptive jetsam-weight learning** — weights are static-configurable in v1; v2 could adjust based on observed thrash patterns.
- **Per-window surface compression** for occluded windows (lossy RGB565 fallback) — defer until profiling demands it.
- **Mmap-backed `read-at`** — v1 ships `pread()` only; v2 may auto-mmap files > 16 MiB for read-heavy workloads.
- **`mgmt: vm-allocate-trace`** — per-grow audit trail for profiling tools (Instruments equivalent). Useful for diagnostics, out-of-scope for v1.

---

## Explicitly NEVER

- **Direct `mmap` / `mach_vm_*` exposed to apps.** WASM linear memory is the only memory primitive apps see.
- **Cross-instance shared linear memory.** `wasip2` does not include wasm-threads; we will not invent a non-standard extension.
- **Raw host pointers crossing the WIT boundary.** Even SHM is identified by `u64` handles. No `pointer-to-host-memory` WIT type.
- **`Memory::data_mut` / `Memory::data_ptr` outside `runtime/memory_io.rs`.** Code-review rule; CI lint enforces.
- **Overshoot-and-backout reservation patterns.** All budget reservations are RAII tokens.
- **Per-app dedicated 8 MiB driver thread.** Shared blocking pool with 2 MiB stack.
- **Per-app private font atlas.** One system-wide 16 MiB atlas; LRU eviction.
- **Killing an app to free memory it doesn't hold.** Suspended apps are NOT live-jetsam candidates; only the compaction pass touches them.
- **Polling `/proc/meminfo` as the primary signal.** PSI is mandatory on supported kernels; polling is fallback only.
- **WASM `memory.shrink`.** Spec does not exist; we will not patch Wasmtime. Use soft-restart instead.
- **Allowing growth during Imminent pressure.** Never. Foreground or otherwise.
- **Holding the AppTable shard lock across a WIT call.** Lock-order discipline forbids it; tests enforce.

---

## Appendix A — Worked Examples

### A.1 Foreground app cold-starts and allocates 32 MiB

1. App spawns. `LifecycleActor` calls `governor.register_limiter(id, Arc::clone(&limiter))`.
2. `governor.set_foreground(id, true)` (focus manager).
3. App's allocator calls `memory.grow 512` (32 MiB).
4. Wasmtime calls `LimiterView::memory_growing(current=1MiB, desired=33MiB)`.
5. `AppLimiter::memory_growing_atomic`:
   - `grow_attempted += 1`
   - per-instance cap: `513 ≤ 2048` ✓
   - lifecycle = `Foreground` (1) → no grant path
   - pressure = `Normal` → admission ok
   - `governor.try_reserve(32 MiB)`:
     - CAS: `wasm_used 64 MiB → 96 MiB`, ≤ `wasm_budget 280 MiB` ✓
     - returns `Some(ReservationToken { bytes: 32 MiB, committed: false })`
   - `current_pages.store(513)`, `peak_pages = max(513, prev)`
   - `token.commit()` — reservation sticks.
   - return `Ok(true)`.
6. Wasmtime grows the memory; app sees `memory.grow` return `1` (previous page count).

### A.2 Background app allocates under Critical pressure (DENIED gracefully)

1. App's lifecycle = `Background` (2). `is_foreground = false`.
2. Pressure crosses to `Critical`.
3. `MemoryPressureMonitor::dispatch` broadcasts `on-memory-warning(Critical, 45 MiB)` to all apps.
4. The Background app's `on-memory-warning` runs on its driver thread (within 100 ms budget) and trims an internal cache, freeing some bytes via its allocator's `free` calls (no `memory.shrink`; bytes remain reserved but free-listed).
5. Later, app tries `memory.grow 16` (1 MiB).
6. `AppLimiter::memory_growing_atomic`:
   - per-instance cap: ok
   - lifecycle = `Background`, no grant
   - pressure = `Critical` AND `is_foreground == false` → return `Ok(false)`
7. Wasm sees `memory.grow == -1`. App's allocator returns null from `malloc`. App handles gracefully (returns error to caller).
8. App does NOT crash; it just cannot grow.

### A.3 Critical pressure triggers suspension; on-suspend succeeds

1. `Pressure: Critical`. `JetsamDirector::handle_critical()`:
   - Loop iteration 1: `MemAvailable = 45 MiB`, target = 100 MiB. `select_victims(1)` → app X (Background, idle 30 min, 64 MiB live).
   - `JetsamDirector::suspend(X)`:
     - `governor.open_suspension_grant(X, 1.25 MiB)` → `limiter.suspension_grant = 1.25 MiB`, `lifecycle_tag = 3`.
     - `lifecycle_tx.send(SuspendForMemoryPressure(X))`.
   - `LifecycleActor` receives, sets `X.lifecycle = Suspending`, calls `on-suspend` on driver thread.
   - In `on-suspend`, app calls `serde_json::to_vec(&state)` → allocator calls `memory.grow 8` (512 KiB).
   - `memory_growing_atomic`:
     - lifecycle = 3 (Suspending) → `try_consume_grant(512 KiB)`: remaining = 768 KiB ✓
     - `current_pages.store(new)`, `peak_pages = max`.
     - return `Ok(true)`.
   - `on-suspend` returns `StateBlob(state_bytes)`.
   - `LifecycleActor` persists to `/data/state/<bundle>.bin`.
   - `LifecycleActor` drops the Store. `governor.unregister_limiter(X)` releases X's committed bytes (32 MiB).
2. Loop re-reads `MemAvailable = 78 MiB`. Still under target.
3. Iteration 2: same flow on app Y.
4. After 3 iterations, `MemAvailable = 130 MiB`. Loop exits.
5. `PressureLevel` transitions back to `Warning` (hysteresis: needs 100 + 50 = 150 MiB to fully recover to Normal).

### A.4 Video app uses Surface-aliased SHM at 60 Hz

1. App creates a window 1920×1080. WindowManager allocates a `Surface { back: 1920*1080*4 = 8 MiB, front: ArcSwap<8 MiB> }`. Governor pre-registered the `Surface` cost.
2. App calls `vyoma:memory/shared/create-surface-buffer(window_id, flags)`.
3. `SharedBufferRegistry::create_surface_alias`:
   - Resolves window's `Arc<Surface>`.
   - Creates `SharedBuffer` with `storage = SurfaceAlias { surface, offset: 0, len: 8 MiB }`.
   - Returns `SharedBufferId(N)`.
4. Each frame, app decodes 8 MiB RGBA into its WASM linear memory, then calls `vyoma:memory/shared/write(id=N, offset=0, data)`.
5. `SharedBuffer::write_at(0, &data[..])`:
   - `SurfaceAlias` branch: writes directly into `surface.back[0..8 MiB]`. **Single memcpy** (wasm linear memory → supervisor `surface.back`).
   - `surface.dirty = true`, `surface.epoch += 1`.
6. App calls `vyoma:display/flush`. Compositor reads `surface.back`, blits to framebuffer. **Second memcpy** (surface.back → framebuffer).
7. Total per-frame copies: **2** (wasm→Surface, Surface→framebuffer). At 8 MiB × 2 × 60 Hz = 960 MiB/s. Achievable on x86 with cache-line streaming stores.

The architect's original 3-copy design would have added another 8 MiB copy (`wasm → SHM → Surface → framebuffer`). The alias optimization saves 480 MiB/s of memory bandwidth per video stream.

### A.5 Fragmentation triggers soft restart

1. App runs for 6 hours. `current_pages = 1024` (64 MiB), `reported_live_bytes = 12 MiB` (from `report-heap`).
2. Ratio: 64 / 12 = 5.3× > 4×.
3. Soft-restart monitor (1-minute tick) marks the app for restart.
4. After 1 hour of sustained ratio > 4×, supervisor:
   - Calls `on-soft-restart(SoftRestartReason::Fragmentation { current_pages: 1024, reported_live_mib: 12 })`.
   - App returns `StateBlob` (≤ 1 MiB).
   - Supervisor persists, drops Store, releases 64 MiB to governor.
   - Supervisor spawns a fresh instance of the same bundle.
   - New instance's `on-resume` receives the saved `StateBlob`.
5. App is back online with fresh linear memory (1 MiB initial); fragmentation reset.

---

## Appendix B — Resolution Index

| Issue | Severity | Resolution | Spec Location |
|-------|----------|-----------|---------------|
| C1: `WasmMemoryStat` not lock-free | Blocking | `Arc<AppLimiter>` in `AppState.hot` | §3.1 |
| C2: `try_reserve` race | Blocking | `ReservationToken` RAII via CAS loop | §2.1 |
| C3: Eviction-serialize circularity | Blocking | Suspension grant: `state_blob_bytes + 256 KiB` | §5.1, A.3 |
| C4: 250 ms polling too slow | Blocking | PSI primary, polling fallback | §4.1, §4.2 |
| C5: 834 MiB > 512 MiB floor | Blocking | 12-app target on 512 MiB; Pooling allocator; shared driver pool | §7 |
| C6: Two-copy SHM | Blocking | `SharedBufferKind::Surface` aliasing | §6.1, A.4 |
| S1: Foreground Arc-clone per grow | Significant | `AtomicBool is_foreground` on AppLimiter | §1.3, §2.1 |
| S2: Font atlas / glyph cache budget | Significant | One system-wide atlas, pre-registered | §8.1, §11.3 |
| S3: Hysteresis flapping via suspend loop | Significant | Batched re-sampling loop, max 5/tick | §5.1, A.3 |
| S4: Background-grow under Critical breaks on-suspend | Significant | Suspension grant bypasses (same as C3 fix) | §1.3, §5.1 |
| S5: `WasmMemoryView` snapshot unsound | Significant | Replaced with `memory_io.rs` free fns | §9.2 |
| S6: Compositor/eviction race | Significant | ArcSwap front + EMPTY_FRONT sentinel + acq/rel | §9.4 |
| S7: select_for_suspend consistency | Significant | Snapshot AppTable before scoring | §5.1 |
| S8: Font atlas bypasses governor | Significant | Boot reorder + `pre_register` | §11.3 |
| S9: Cold-start storm non-determinism | Significant | Same as C2 fix (RAII tokens) | §2.1 |
| S10: Allocator fragmentation | Significant | Soft-restart policy + `on-soft-restart` | §1.5, A.5 |
| S11: Streaming seek invalidates ra | Significant | `read-at(offset, len)` via `pread()` | §8.3 |
| S12: 16 MiB SHM cap too small | Significant | Surface cap = 64 MiB | §6.1 |
| S13: `on-memory-warning` undefined | Significant | Full contract documented | §10.3 |
| S14: jetsam selects zero-memory victims | Significant | Suspended excluded; compact pass | §5.1, §5.3 |
| S15: Crash report unclear | Significant | `last_attempted_pages` + governor snapshot | §10.5 |
| G1: WASM stack accounting | Gap | Pooling allocator's fixed stack budget; counted in `wasm_stores_total` region | §7.3, §10.2 |
| G2: Engine compile memory | Gap | `compile_arena` 64 MiB, pressure gate | §9.7 |
| G3: Kernel page cache | Gap | Out of scope; governor reads MemAvailable which is post-cache | §4.2 |
| G4: 9P virtio queue depth | Gap | Bounded by virtio queue size (fixed by drivers); host_io budget covers app-visible buffers | §8.3 |
| G5: Drop Store mid-call | Gap | LifecycleActor uses tokio's task abort + Store drop in driver thread context | §5.1 |
| G6: PoolingAllocator vs on-demand | Gap | Pooling chosen, documented | §7.3 |
| G7: WASM table memory | Gap | `table_growing` enforced; tables capped at 16K elements; counted in store overhead | §1.3 |
| G8: Host-side I/O buffers | Gap | `host_io_budget_bytes` separate budget | §2.1, §8.3 |
| G9: Stopping vs jetsam | Gap | `stopping_lifecycle: -10_000_000` (NEVER interrupt) | §5.1 |
| G10: jetsam + epoch increment | Gap | JetsamDirector triggers epoch via `lifecycle_tx` → LifecycleActor → Store::terminate via epoch | §5.1 |
| G11: set-pressure gate | Gap | `mgmt.allow_pressure_injection` manifest flag | §10.6 |
| G12: jetsam rate metric | Gap | `jetsam_events_5s` + `jetsam_storm` flag in heartbeat | §10.4 |

---

## Closing Note

This synthesis takes the architect's coherent structural design and patches every critical fault: PSI replaces polling, RAII tokens replace overshoot-and-backout, suspension grants break the eviction-serialize cycle, Surface aliasing makes 60 Hz video tractable, and the 50-app target is honestly partitioned across RAM-floor budgets. The 500-line ceiling holds across all 16 new files. Every recommendation R1–R22 is closed against an explicit spec location. The four BLOCKING issues C1–C6 are resolved with concrete Rust code.

The remaining open question for future rounds is wasm-threads adoption: once `wasip3` ships with `shared-memory`, the SHM `Owned` storage variant can be re-implemented as a `Memory::shared` import and the second copy disappears for non-Surface kinds too. For now, the SPSC audio ring and Surface aliasing cover the bandwidth-critical paths; everything else is fine with one memcpy.

The most important architectural choice in this round: **pressure is an event, not a number to poll**. PSI makes the rest of the design tractable; without it, polling lag would invalidate every other guarantee. This single inversion — from polling to subscription — is what separates a desktop OS that survives a media-app burst from one that gets OOM-killed by its own kernel.

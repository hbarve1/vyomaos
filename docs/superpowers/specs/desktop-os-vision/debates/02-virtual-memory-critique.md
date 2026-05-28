# Round 2 Critic: Virtual Memory & Address Space

**Date:** 2026-05-29
**Round:** 2 of 80
**Subsystem:** Virtual Memory & Address Space
**Critic verdict:** FUNDAMENTAL FLAWS

## Executive Summary

The architect has produced a structurally coherent design that fits well within the Round 1 framework (sharded `AppTable`, `AppLimiter`, `StateBlob`, `CrashKind`), but the spec contains at least **four fundamental flaws** that make it unimplementable as written: (1) a self-contradictory "lock-free" `WasmMemoryStat` that the architect demonstrably *cannot* read without touching the `Store` it explicitly disclaims, (2) a `MemoryGovernor::try_reserve` that the architect themselves flags as racy and which leaks budget under any preemption window, (3) the **eviction–serialize circularity** — `on-suspend` requires the app to allocate inside the very linear memory the governor is trying to free, and (4) a **250 ms /proc/meminfo polling cadence** that demonstrably cannot react in time to wasm-internal allocator bursts. Additionally, the RSS budget at 50 apps (~834 MiB) **does not fit** the `desktop-full` 512 MiB RAM floor cited in `CLAUDE.md`, and the foreground-set lookup, font atlas one-shot allocation, two-copy SHM, and pressure hysteresis all have correctness or performance flaws that must be fixed before synthesis. The most critical single change before merge: replace the polling-based `MemoryPressureMonitor` with PSI (kernel cgroup memory pressure stall information) and rework `MemoryGovernor::try_reserve` to use a proper reservation-token RAII type.

## Critical Issues (blocking)

### C1. `WasmMemoryStat` is **not** lock-free — the "never touches linear memory" claim is misleading at best, incorrect at worst

The architect writes (§1.5):

> Crucially: this never reads the app's linear memory. We expose **capacity** (how many pages the app has reserved), not **occupancy**.

This is technically true for `current_pages` and `peak_pages` (they are mirrored in `AtomicU64` on the `AppLimiter`), but the implementation in §1.5 is still wrong on two counts:

1. **The `AppLimiter` is owned by the `Store`.** Wasmtime stores the `ResourceLimiter` either as `Store::limiter(|s| ...)` (closure-based) or as `Store::limiter_async`, both of which require a `&mut Store<T>` to install or modify. The `AppLimiter` is reachable from the `Store::data()` chain. Reading `current_pages` outside a `Store` lock is only safe **if** the architect commits to wrapping `AppLimiter` in an `Arc<>` and exposing it through a sidecar table — which the spec does not say. The phrase "`limiter: &AppLimiter`" in `WasmMemoryStat::read` assumes a `&AppLimiter` is reachable without locking, but the `AppLimiter` is owned by the `Store` and the Round 1 design puts the `Store` behind the app's driver thread.

2. **Even if `AppLimiter` is in an `Arc`**, the architect implicitly assumes Round 1's hot/cold split exposes it. Round 1 §6 introduces `AppLimiter` as a per-store limiter, not a sidecar. The architect must either (a) lift `AppLimiter` out of the `Store` and into the `AppState.hot` field with `Arc<AppLimiter>`, or (b) provide a `MemoryStatHandle` snapshot type that is published into `AppState.hot` after each grow. Neither is in the spec.

**Concrete failure mode:** `mgmt: memstat <bundle>` is described as a stdout command served by the supervisor's main loop. The main loop would call `WasmMemoryStat::read(limiter)`, but there is no thread-safe path from the main loop to the `Store`-owned `AppLimiter`. Either the call deadlocks (driver thread holds the `Store` mutably during a `memory.grow`) or it returns stale data.

**Severity:** Blocking. The whole §7 debug API rests on this, and `MemoryGovernor::try_reserve`'s correctness check also reads from this surface.

### C2. `MemoryGovernor::try_reserve` is fundamentally racy and the architect knows it

The implementation in §8:

```rust
pub fn try_reserve(&self, bytes: usize) -> bool {
    let bytes = bytes as u64;
    let prev = self.wasm_used_bytes.fetch_add(bytes, Ordering::AcqRel);
    if prev + bytes > self.wasm_budget_bytes.load(Ordering::Relaxed) {
        self.wasm_used_bytes.fetch_sub(bytes, Ordering::AcqRel);
        false
    } else { true }
}
```

This is the "overshoot-and-backout" pattern and it has **three** problems:

1. **Visible over-budget windows.** Between `fetch_add` and `fetch_sub`, a third observer (the pressure monitor, the heartbeat emitter, a concurrent `try_reserve`) sees `wasm_used_bytes > wasm_budget_bytes`. If the heartbeat emits during this window, telemetry shows a spurious budget breach. If the pressure monitor reads in this window, it may classify pressure incorrectly and **kill an app for transient over-commit that never actually happened**. The architect Open Question #2 acknowledges the leak but downplays it; the killing-an-app consequence is not mentioned.

2. **Permanent leak on panic between add and sub.** If the thread is killed (panic in `parse_meminfo_field`, OOM-killer hits the supervisor, etc.) between `fetch_add` and the `fetch_sub` backout, the budget is permanently leaked. Over a long uptime (the architect's stated goal is "24-hour desktop"), this can leak enough to deadlock new grows.

3. **Concurrent contention amplification.** Under N concurrent grow attempts where total demand exceeds budget by ε, all N threads `fetch_add`, all see the over-budget condition, and all `fetch_sub`. The CAS loop the architect denies having is **emergent**: every thread does useless work. Worse, if one thread is slow to `fetch_sub`, the other threads' over-budget check fires on a fictitious total. This is the kind of bug that doesn't show in unit tests but melts under contention from a `gtk-broadway`-style window animation that does sustained grows.

**Concrete failure mode:** A user opens 30 apps that each immediately allocate 32 MiB on startup. With `wasm_budget_bytes = 384 MiB`, the spec says 12 should succeed and 18 fail. With the racy `try_reserve`, the actual outcome is non-deterministic — between 8 and 15 succeed depending on scheduling — and the pressure monitor may observe a fictitious 960 MiB used during the storm.

**Severity:** Blocking. Replace with a proper reservation token.

### C3. **Eviction–serialize circularity** in `on-suspend`

The Round 1 lifecycle says `on-suspend` is invoked before the store is dropped, giving the app a chance to write `StateBlob` (≤ 1 MiB). The architect (§4.5) reuses this path under `PressureLevel::Critical`:

> `Critical` → standard `Foreground/Background → Suspended` transition (Round 1 §3 already drops the store and persists `StateBlob`).

But **`on-suspend` runs inside the wasm instance**, which means:

1. The app's allocator may need to call `malloc` to serialize state (think `serde_json::to_vec` or `bincode::serialize_into(&mut Vec<u8>, ...)`).
2. `malloc` may call `memory.grow` because the allocator's free lists are exhausted.
3. The `memory_growing` hook (§1.3) under `PressureLevel::Critical` returns `false` if `!self.is_foreground()` — and the app is by definition NOT foreground at this point because it's being suspended.
4. `memory.grow` returns -1; the allocator panics or aborts.
5. The app crashes with `CrashKind::Trap` BEFORE `on-suspend` completes; `StateBlob` is never written.
6. On restore, the app comes back with no state.

This is a **textbook circular dependency**: you cannot evict memory by asking the app to serialize itself if serialization needs memory you won't grant.

**Mitigations the architect did NOT specify:**

- A "suspension grant" of N pages temporarily admitted in `memory_growing` when the lifecycle state is `Stopping` or `Suspending`.
- A pre-allocated supervisor-side serialization buffer the app can `wit/streaming-write` into without growing its own memory.
- A two-phase suspend: first warning (app trims), then suspend (app serializes from pre-reserved scratch).

**Concrete failure mode:** Under `Critical`, the supervisor selects 5 victims, calls `on-suspend` on each, all 5 fail to allocate, all 5 trap, none persist state, all 5 lose user data. The supervisor records "5 apps suspended successfully" because the store was dropped, but a 6th app's allocation that triggered the eviction can now succeed — masking the data loss as a transient pressure event.

**Severity:** Blocking. The Round 1 contract that "suspend preserves state" is violated under exactly the conditions that motivate suspension.

### C4. 250 ms `/proc/meminfo` polling cannot keep up with wasm-internal bursts

The architect (Open Question #3) flags this themselves:

> A burst allocator inside a wasm app can move 64 MiB in 50 ms; we might miss the warning window.

The actual situation is worse than the architect admits:

1. **A single grow is unbounded between sample ticks.** Wasm `memory.grow` is a single instruction; the `AppLimiter::memory_growing` hook fires per grow but only checks the **governor's cached** `pressure_level`. Between two pressure samples (250 ms), an app can attempt to grow N times, and every grow is admitted as long as `wasm_budget_bytes` is not exhausted — even if `MemAvailable` has crashed to 5 MiB in the interim.

2. **`/proc/meminfo` is a file read.** On a loaded system, reading `/proc/meminfo` takes 50–500 µs. The architect's 250 ms cadence means a worst-case-200 ms blind window where multiple Critical-eligible events can fire.

3. **No backpressure on the hot path.** The `memory_growing` hook never re-reads `MemAvailable` directly; it consults the *cached* `PressureLevel`. So an app can drive the system into OOM without any wasm-side resistance, because the cached level is stale.

4. **PSI (`/proc/pressure/memory`) is the right tool**, and Linux 5.10 (the VyomaOS kernel version) supports it. PSI provides edge-triggered notifications via `epoll` when the time spent stalled on memory crosses a threshold within a window. The architect dismisses this in Open Question #3 as a "should we?" — but the answer is unambiguously yes; polling is the wrong primitive.

**Concrete failure mode:** A media app decodes a 4K frame, the decoder's allocator grows by 64 MiB in a single tick. Between sample N (Normal, 250 ms ago) and sample N+1 (Critical, now), the supervisor's heap also grew because the compositor allocated a new surface. `MemAvailable` drops below the Imminent threshold. Sample N+1 fires, jetsam runs, but in the 250 ms gap the kernel OOM-killer has already killed the supervisor (PID 1). System reboots.

**Severity:** Blocking. PSI is mandatory; polling is at best a fallback for non-PSI kernels.

### C5. RSS budget at 50 apps does NOT fit the `desktop-full` 512 MiB RAM floor

The architect's table (§2.8) totals **834 MiB**. `CLAUDE.md` states:

> `desktop-full` | x86-64 workstation (default) | Wasmtime JIT | 512 MB

The architect waves at this with:

> This sits comfortably under the 1 GiB platform floor for `mobile` and easily inside the 512 MiB → 8 GiB range of `desktop-full`.

This is **mathematically false**. 834 MiB > 512 MiB. The RAM floor is the *minimum required RAM*, not the upper limit at which the system is comfortable. At 512 MiB total RAM:

- Linux kernel: ~50 MiB
- Tmpfs root, busybox, wasmtime runtime, supervisor binary: ~80 MiB
- Page cache + dentries + slab: ~50 MiB minimum
- **Remaining for VyomaOS userspace: ~330 MiB**

The 50-apps target is therefore infeasible on the floor configuration, and the architect's "comfortably" claim is unfounded. Additionally, the 50 apps target itself appears to be aspirational with no reasoning — the per-app `Store` overhead alone (250 KiB) × 50 = 12.5 MiB is a lower bound, but linear memory at 8 MiB average is a guess. The current shipped supervisor (per `CLAUDE.md`) runs 10 concurrent apps; the spec jumps to 50 with no intermediate analysis.

The thread stacks line is also suspicious: **16 threads × 8 MiB = 128 MiB**. The architect notes "one Tokio worker per CPU + N app driver threads (one per RUNNING instance)". At 50 running apps, that is at least 50 driver threads × 8 MiB = 400 MiB of stack reservation. Either:
- (a) The reservation is mostly virtual and not RSS (then say so), or
- (b) The "16 threads" line is for a 16-app world and contradicts the 50-app target.

**Severity:** Blocking. The architect must reconcile the per-app driver thread model with the RAM floor, or revise the 50-apps target down to something the platform actually supports.

### C6. The two-copy `VYOMA_SHM` is **unworkable** for video/audio at desktop frame rates

The architect (Open Question #5) flags this:

> For a 16 MiB surface at 60 Hz this is ~ 2 GiB/s of memcpy.

Even ignoring the cost (`memcpy` at 10 GiB/s is achievable on modern x86), the two-copy model has correctness problems:

1. **Latency budget.** At 60 Hz, the frame budget is 16.7 ms. A 16 MiB copy at 4 GiB/s (realistic with cache misses) is 4 ms — **24 % of the frame budget gone to copying** even before the compositor blits. For audio, 1024-frame buffers at 48 kHz must be delivered every 21 ms; a 4 KiB copy is fine, but 16 audio streams × 4 KiB × 2 copies = 128 KiB per cycle, which adds jitter.

2. **The WIT `list<u8>` ABI is not zero-copy.** The architect calls this out but understates the impact. `list<u8>` is canonical ABI: Wasmtime allocates supervisor memory, copies from wasm linear memory in, then on `read` allocates wasm linear memory and copies out. Neither side gets a borrowed slice.

3. **For surfaces specifically, this is a regression from the current commit log:** `2490ad6` "feat(display): per-window Surface buffer + blit_surface compositor primitive" already has a `Surface` in supervisor memory (§2.3 of the spec). The spec does not say whether the per-window `Surface` IS the SHM buffer or is a separate copy. If separate, that's **three** copies (wasm → SHM → Surface → framebuffer).

4. **No proposed escape hatch.** The architect's "do we need wasm-threads?" Open Question #5 has no answer. Without an answer, video/audio apps are architecturally impossible at desktop frame rates.

**Concrete failure mode:** A user runs a YouTube-clone app. The video decoder allocates a 1920×1080 RGBA buffer (~ 8 MiB) and pushes a new frame every 16.7 ms. Per the spec, this means: wasm → SHM (8 MiB copy, ~ 2 ms), SHM read into Surface (8 MiB, ~ 2 ms), Surface blit to framebuffer (~ 2 ms). 6 ms of pure copying per frame, plus decoder work — frame rate caps at ~ 30 Hz on a system that should easily do 60.

**Severity:** Blocking for any media use case. Must either accept a wasm-shared-memory backing or specify a "supervisor surface ID = SHM ID" alias.

## Significant Issues (important)

### S1. Foreground-set lookup is O(1) but `Arc` clone-per-grow is wasteful and racy with focus changes

The architect's `is_foreground()`:

```rust
self.governor.foreground_set().contains(&self.instance_id)
```

uses `arc_swap::ArcSwap<im::HashSet<InstanceId>>`. Per Open Question #1, this clones an `Arc` on every grow.

Problems:

1. **Cost is real but undersold.** `Arc::clone` is one atomic increment (~ 10 cycles), but at 100 grows/s/app × 50 apps = 5000 atomic increments/s on a hot cache line. Under heavy allocation (game start, IDE warm-up), this can spike to 50,000/s and the atomic becomes a contended cache line.

2. **Consistency with focus changes is unspecified.** Who writes to `foreground`? The spec says "focus manager" but does not specify the WindowManager → MemoryGovernor channel. Race window: focus changes from A to B; A is still in `foreground` for one swap; A's grow is admitted while B's is denied (because B isn't in the set yet). Inverted at the next swap.

3. **The fix the architect proposes** ("cache foreground-ness on `AppLimiter` itself and update via a channel from the focus manager") is the right one, but introduces a new lock-order concern: the focus manager → AppLimiter write happens under the WindowManager lock; if AppLimiter writes go under a Store lock, you can deadlock.

**Recommended:** Cache `is_foreground: AtomicBool` on `AppLimiter`; updates pushed by the focus manager via a `Box<dyn Fn(bool) + Send + Sync>` callback registered at app spawn. No lock-order risk because the update is a single atomic store.

### S2. Font atlas 16 MiB allocated "once at startup" — what about 50 apps loading custom fonts?

§5.1 says the font atlas is 2048×2048 RGBA = 16 MiB, allocated once. But §2.5 says glyph cache is *also* 8 MiB. The atlas is the GPU-uploadable bitmap; the glyph cache is "rasterized glyph dictionary". Total: 24 MiB for text.

Problems:

1. **One atlas for the whole system?** If app A and app B both use a custom Hebrew font and app C uses Cyrillic + Devanagari, do they share the atlas? If yes, what's the eviction policy when the atlas is full? If no, where do per-app atlases live? The spec doesn't say.

2. **50 apps × custom font.** If each app uses `cosmic-text` or similar with its own atlas, 50 × 16 MiB = 800 MiB just for atlases. The spec's "one atlas, 16 MiB" implies system-wide sharing, but doesn't specify the eviction protocol when the atlas overflows.

3. **Boot-time allocation order.** §9 says `InitDisplay` allocates the font atlas; §1.1 / §4 says the `MemoryGovernor` budgets are set in `LoadProfile` *after* `InitDisplay`. So the 16 MiB atlas allocates **before** the governor exists, meaning the atlas is exempt from the very budget that should account for it. If the governor later sets `wasm_budget_bytes = 384 MiB` on a 512 MiB system, the 16 MiB already spent on the atlas is invisible to the budget.

### S3. Pressure hysteresis can still oscillate via the eviction-recovery loop

§4.6 says hysteresis prevents flapping by requiring 50 MiB recovery margin. But the recovery itself is driven by jetsam:

1. Pressure crosses Critical → suspend 1 app, recover 32 MiB.
2. `MemAvailable` is now 32 + previous (say 40) = 72 MiB. Still under `crit + hysteresis` (50 + 50 = 100 MiB).
3. Spec dispatches Critical again on next tick → suspend another app.
4. Repeat until enough margin is gained.

This is correct behavior in one sense (recover until safe) but pathological in another:

- **Each tick suspends one app**, but the cost of a `Suspended → Foreground` restore is much higher than the cost of the suspension (state deserialize, allocator warm-up). If memory recovers later, all the suspended apps stay suspended; they don't auto-restore.
- **No batching.** A Critical event should select N victims to reach the target watermark in one batch, not one per tick. The architect's `select_for_suspend(Default::default())` is described but its batching behavior is unspecified — does it return one victim or many?

Looking at §8 again:

```rust
let victims = self.governor.select_for_suspend(Default::default());
for v in victims {
    self.governor.suspend_instance(v).await;
}
```

This *looks* like batched suspension, but:

- The `for` loop awaits each suspend. The first suspend can take 100+ ms (StateBlob write); the loop is sequential.
- After each suspend, `MemAvailable` should be re-checked to avoid over-suspending. The spec doesn't say.
- Under sustained pressure, the loop completes, ticker fires again, *another* batch is selected. Hysteresis does not prevent over-suspending.

**Recommended:** Batched, re-sampling suspend loop with a per-batch cap (e.g. ≤ 5 apps/tick) and explicit re-measurement after each suspension.

### S4. Background app cannot grow under Critical → buggy interaction with `on-suspend`

Already covered in C3 from one angle. Different angle: §1.3 step 4 says:

```rust
PressureLevel::Critical if !self.is_foreground() => {
    return Ok(false);
}
```

But §4 §4.5 says Critical triggers a `Background → Suspended` transition that runs `on-suspend`. The app, during its own suspension, IS background. So its grow is denied. This makes graceful suspension impossible at the exact time it matters.

Compounding the issue: an app that has just been resumed from suspend (`Suspended → Background`) needs to rehydrate state. Rehydration may require growing memory. If the system is still at Critical, the resumed app cannot grow, so it cannot rehydrate, so it traps. The eviction-restore loop is broken.

### S5. `WasmMemoryView` snapshot_size assumption can be silently violated

§6.5:

> A debug assertion in `WasmMemoryView::write` checks that `memory.data_size(&store) == self.snapshot_size`; if it diverges the supervisor panics with a stack trace. This is a developer-only check disabled in release.

Two problems:

1. **Disabling the check in release** is exactly when you need it. Wasmtime's documentation is explicit: holding a reference to wasm memory across a grow is UB. The cost of the check is one load + compare; saving that in release is penny-wise pound-foolish.

2. **The check is too weak.** The wasm app cannot grow during a host call (single-threaded execution within the Store), but the supervisor *can* call `Memory::grow` itself if it wants to make space for a write (the architect does not say this is forbidden). If supervisor code grows memory while holding a `WasmMemoryView`, the view is invalidated and the check fires only on the next `write`. But pointers obtained via `Memory::data_ptr` before the assertion fired are already dangling.

**Recommended:** Make `WasmMemoryView` *not* expose raw `data_ptr`. Always read/write through `Memory::read`/`Memory::write` (which re-check bounds internally). The "guard" then becomes a marker type, not a memory accessor; rename it to `WasmMemoryAccess` or just delete it and use `Memory` directly.

### S6. Compositor/eviction race with `ArcSwap<Surface>` is *not* obviously safe

The git log shows `Surface` is used; the spec §2.3 describes the structure. Under critical pressure, the spec says hidden surfaces drop their `front`. But:

1. **The compositor reads `front` under `ArcSwap`.** If the eviction path stores a new `Surface` with `front: vec![]`, the compositor reading the old `front` is fine (Arc keeps it alive) but the *next* compositor frame sees an empty front and the window flashes black. Is this intentional? The spec doesn't say.

2. **The `Surface.dirty: AtomicBool` and `epoch: AtomicU64` need explicit sequencing.** If eviction clears `dirty` but not `epoch`, the compositor may skip a frame that should have been drawn (because dirty=false) but the app's next `flush` may not re-set dirty before the compositor's next read. Memory ordering on these atomics is not specified.

3. **The "share scratch front buffer" optimization** (§2.3 item 2) is described in one sentence with no implementation. When does the compositor decide a window is fully occluded? Is this per-frame or hysteretic? If per-frame, the surface allocation churns on every window state change.

### S7. `select_for_suspend` iterates the sharded AppTable — what's the consistency model?

§4.4: "Victim selection iterates the `AppTable` (sharded, Round 1 §1), reads each `AppState` under its `RwLock`."

Problems:

1. **16 shards × N apps per shard.** Iterating means acquiring 16 read locks in sequence. If any shard has a writer waiting (e.g., spawn of a new app), the iteration blocks.

2. **AppState views may be inconsistent across shards.** App A in shard 3 is read at T=0 with `lifecycle=Foreground`. By T=1ms shard 7 is read; in between, app A's focus changed and its lifecycle is now Background. The `jetsam_score` for A reflects an obsolete state. This can flip the ordering of victim selection.

3. **`now: Instant` is captured once.** The architect captures `now` at the start of iteration; idle computation uses this. If iteration takes 5 ms, the most recently focused app gets a stale `last_focus`. Probably fine, but uncited.

**Recommended:** Snapshot a `Vec<(InstanceId, AppStateSnapshot)>` before scoring. Trade memory for consistency.

### S8. `MemoryGovernor` boot ordering allows the font atlas to bypass the budget

Already touched in S2. Per §9 `BootPhase` table:

- `LoadProfile` reads `[memory]` block to construct `MemoryGovernor` budgets.
- `InitDisplay` allocates the font atlas.
- `InitRegistries` constructs `MemoryGovernor`.

But `InitDisplay` precedes `InitRegistries`. So the atlas is allocated **before** the governor exists. The governor's `wasm_used_bytes` therefore starts at 0 even though the supervisor has already spent 16 MiB on atlas, ~5 MiB on framebuffer, etc. Either:

- (a) Move governor construction before display init (introduces chicken-and-egg if display config is in profile).
- (b) Pre-register the static allocations with the governor on construction.

The spec does neither.

### S9. Per-instance cap + global governor: pathological behavior under simultaneous grow

§1.3 admission flow: per-instance cap → global try_reserve → pressure check.

Imagine 10 apps each at 100 / 128 pages with a budget of 600 pages free. All 10 call `memory.grow 60` simultaneously:

- Each passes per-instance (100 + 60 = 160 ≤ 128 → false). Wait, 160 > 128, so per-instance denies. OK, choose 28 instead.
- Each passes per-instance (100 + 28 = 128 ≤ 128 → true).
- All 10 call `try_reserve(28 * 64 KiB) = try_reserve(1.75 MiB)`.
- Total demand: 17.5 MiB. Budget free: say 12 MiB.
- Per the racy `try_reserve` (C2), the actual outcome is non-deterministic. Some random subset succeeds.
- The denied apps see `memory.grow == -1`. Their allocators may abort. Apps die.

**This is exactly the workload (cold-start storm) where determinism matters most.** The spec gives no guidance on grow ordering. A cold-start "thundering herd" can kill multiple apps non-deterministically.

### S10. `dlmalloc`/`wee_alloc` fragmentation over 24-hour uptime is unaddressed

The architect (§1.4) defaults to `dlmalloc`. Known issues:

1. **`dlmalloc-rs` is unmaintained-ish.** Last release 2024-03; bug fixes are slow. The crate is a port of Doug Lea's classic dlmalloc, which is designed for processes with eventual exit. WASM apps in VyomaOS are expected to run for 24+ hours.

2. **Fragmentation in WASM is worse than native.** WASM has no `madvise(MADV_DONTNEED)` — pages once committed never return to the OS. The supervisor's grow accounting tracks pages *allocated*, not pages *in use*. An app that allocates 64 MiB, frees 60 MiB, then allocates 1 MiB more sees:
   - In native: kernel reclaims the freed pages via `MADV_DONTNEED`; RSS drops.
   - In WASM: linear memory cap stays at 64 MiB; `current_pages = 1024`.
   - `MemoryGovernor.wasm_used_bytes` shows 64 MiB used; reality is 5 MiB live.

3. **`peak_pages` is the right metric, but the governor budgets on `current_pages`.** This is a one-way ratchet: an app's accounted memory only goes up.

4. **No proposal for `memory.shrink`.** WASM has no shrink instruction. The only way to release WASM linear memory back to the system is to terminate the instance. This means every long-running app eventually accumulates fragmentation overhead.

**Recommended:** Document this as a known limitation; add a "soft restart" mechanism for apps where the supervisor terminates and respawns after RSS exceeds K × peak_live_estimate.

### S11. Streaming API is pull-only — video seek is broken

§5.3:

```wit
pull:  func(h: stream-handle) -> result<option<list<u8>>, fs-error>;
seek:  func(h: stream-handle, offset: u64) -> result<_, fs-error>;
```

`seek` exists but has caveats the architect didn't address:

1. **`seek` invalidates `read-ahead`.** The supervisor pre-reads `read-ahead` chunks. On `seek`, those are wasted. Frequent seeking (video scrubbing) burns supervisor I/O bandwidth.

2. **No random-access primitive.** Apps that need true random access (an MP4 demuxer reading the moov atom at end-of-file then jumping back) need many seeks. Each seek round-trips through wasm → host → BufReader.

3. **No mmap-equivalent.** The architect ruled out mmap for apps. But the *supervisor* can mmap the file and expose chunks via the streaming protocol. The spec doesn't say whether the host-side `BufReader` is on a `File` or an `Mmap`. For media access patterns, mmap-backed is critical.

**Recommended:** Add a `Range` variant to the streaming API: `read-at: func(h, offset: u64, len: u32) -> result<list<u8>, fs-error>`. Host implements via `pread()` or mmap-backed.

### S12. Per-buffer 16 MiB cap is too small for desktop graphics

§3.2: "Per-buffer cap: 16 MiB (covers a 2048×2048 RGBA atlas)."

But a 4K display (3840×2160) at RGBA = 32 MiB. A 5K display = 50 MiB. A high-DPI 1440×900 *retina* (2880×1800) surface = 20 MiB. The cap is below the working set for a single full-screen surface on common displays.

The spec elsewhere mentions max surface 4096×4096 (= 64 MiB). The shared-buffer cap (16 MiB) is **smaller than the max surface cap (64 MiB)**, but `SharedBufferKind::Surface` is the path apps use to share surfaces with the compositor. Internal inconsistency.

### S13. `WIT on-memory-warning` callback semantics undefined

§4.3:

> `Warning` → `on-memory-warning` callback (new WIT, but existing dispatch infrastructure).

What does the app return? What's the timeout? What if the app is mid-`memory.grow` when the callback fires? Is the callback synchronous (blocks the supervisor's pressure dispatch loop) or async (queued to driver thread)? The spec is silent.

Round 1 documented the WIT callback contract carefully; this spec breaks that pattern by declaring a new callback in one sentence with no signature, return contract, or budget.

### S14. `jetsam_score` magic numbers without normalization

§4.4 weights:

- Foreground: -100,000,000
- Foreground lifecycle: -50,000,000
- System origin: -30,000,000
- Pin flag: -100,000,000
- focus_priority × 100,000
- idle_secs × 1,000
- MiB × 1,000

These all add up in `i64`. Open Question #7 asks if these are defensible; they are not, because:

1. **No upper bound proof.** What if `idle_secs` is 7,200 (2 hours, exceeds the 1-hour cap; actually it IS capped to 1h = 3600). At max idle = 3600 × 1000 = 3.6M, weight is smaller than foreground (50M); reasonable. But what if MiB cost = 1024 MiB? 1024 × 1000 = 1.024M. So a 1 GiB suspended app has score -49M (suspended → 0, but plus 1M MiB cost = +1M, vs. background -5M). Wait: suspended is 0 not -5M, so a suspended 1 GiB app score is 1M; background 1 MiB app is -5M + 1k = -4.999M. Suspended apps are evicted first — but they have NO RUNNING WASM STATE to free! Killing a suspended app frees only the StateBlob (≤ 1 MiB).

   **The scoring system literally selects victims that yield no memory.**

2. **No platform-specific scaling.** On `iot-edge` (4 MiB RAM floor), the MiB scale × 1000 should be much larger to differentiate small differences. The same constants apply across all profiles.

3. **No anti-thrash bias.** An app suspended 5 seconds ago is the most likely candidate to be needed back; reviving it costs more than killing a different one. The score doesn't account for "recently suspended → expensive to evict from suspension".

**Recommended:** Make weights `profile.toml`-configurable; reject `Suspended` from victim list entirely (or add a "fully evict suspended" pass separate from the live-app scoring).

### S15. Crash report `memory_snapshot` lifecycle unspecified

§7.4: "Crash report gains a `memory_snapshot` field with `WasmMemoryStat` at the moment of crash."

But:

- If the crash is `MemLimit`, the snapshot is taken *after* the failed grow. By then, `current_pages` reflects the state pre-attempted-grow (because we returned false before storing). The snapshot doesn't show the user "127 attempted to grow to 256"; it shows "127 was the state". The architect's example narrative ("app died with 127 pages, peak 128, requested 1 more") requires capturing the *attempted* size separately. The spec doesn't say so.

- For `Jetsamed`: the score is captured. But the score is computed by the supervisor using a snapshot of state; recomputation in the report won't match. Is the score logged at decision time or at report time?

## Design Gaps

### G1. No story for **wasm stack memory accounting**

`StackOverflow` is added as a `CrashKind`, but the wasm stack is allocated by Wasmtime separately from the linear memory. The default stack size is 1 MiB per Wasmtime call. At 50 instances × multiple concurrent host calls, this stack memory is invisible to the governor. No accounting, no budget.

### G2. No story for **Wasmtime engine memory**

The architect mentions "compiled code: shared via Engine, ~5 MiB total" but does not account for:

- Compilation memory during module load. Cranelift can spike to 50+ MiB per module compile.
- Engine's instance-template memory.
- Trap-handler signal stack.

If a user installs a new module, the supervisor compiles it; under critical pressure, this can OOM the supervisor.

### G3. No story for **kernel page cache pressure**

`MemAvailable` includes evictable page cache. If the supervisor is reading lots of files (logs, configs), page cache grows. Under pressure, the kernel evicts page cache; supervisor experiences I/O latency. The spec does not consider page cache as a "free" resource and does not control supervisor I/O under pressure.

### G4. No story for **`/data` (9P) memory cost**

§5.3 streaming I/O reads files via 9P virtio. Each in-flight 9P request consumes virtio queue slots. No accounting for virtio queue depth × per-request buffer.

### G5. No story for **dropping a `Store` mid-call**

`MemoryGovernor::suspend_instance` says "call WIT on-suspend, save StateBlob, drop store". But if the app is currently inside a host call (e.g., blocked on a long stdout write), the store cannot be dropped from another thread. Wasmtime requires the `Store` owner thread to drop. Coordination protocol not specified. Open Question #6 acknowledges this; no answer provided.

### G6. No story for **Wasmtime pooling allocator vs. on-demand**

Wasmtime supports two allocation backends: `PoolingAllocator` (pre-allocates a fixed pool of `Store`s for low-latency instantiation) and on-demand. The choice affects RSS dramatically:

- Pooling: 50 stores × 64 MiB reservation = 3.2 GiB virtual (committed lazily).
- On-demand: each store allocates its own; no fixed reservation.

The spec does not say which is used. Either choice has different memory characteristics.

### G7. No story for **WASM table memory**

§1.3 includes `table_growing` but only mentions `MAX_TABLE_ELEMENTS`. Tables hold function references; large tables (for dynamic dispatch via `call_indirect`) can be MiB-scale. No accounting in `MemoryGovernor`.

### G8. No story for **host-side allocations triggered by wasm calls**

When an app calls `vyoma:fs/streaming/pull`, the supervisor allocates a `Vec<u8>` of up to `chunk_size` (default 256 KiB). At 50 apps × 4 outstanding pulls = 50 MiB of supervisor-side buffers, none budgeted. The governor only sees wasm linear memory and SHM.

### G9. No specification of **what happens to `LifecycleState::Stopping` apps**

§4.4 scoring gives `Stopping` a high positive score (+10M), preferring to finish them. But "stopping" means an `on-stop` callback is in progress. Killing an app mid-stop is racy. The spec does not say if jetsam can interrupt `on-stop`.

### G10. No specification of **how pressure interacts with `Engine::increment_epoch`**

Round 1 §4 uses Wasmtime epoch interruption to terminate runaway apps. The spec does not address whether jetsam triggers epoch increment or relies on `store.terminate()`. Different semantics → different latencies → different memory-free timings.

### G11. `mgmt: set-pressure` is a test-only command but lacks an obvious safeguard

§7.2 includes `set-pressure: func(level: u8) -> bool` "requires mgmt capability". But setting pressure to `Imminent` from a debug shell would jetsam half the system. Who can invoke this? Is it gated behind a `[debug] allow_pressure_injection = true` manifest flag? The spec is silent.

### G12. Heartbeat `jetsam_events_total` is monotonic; no rate observation

§7.3 emits `jetsam_events_total`. To detect jetsam storms, an observer needs to differentiate. The spec doesn't expose a rate or windowed counter.

## Points of Strength

Despite the issues above, the architect did several things well:

### P1. Reusing Round 1 lifecycle for eviction transitions

§4.5 explicitly does not invent new states; it uses the existing `Foreground → Background → Suspended → Stopping → Crashed` machinery. This is correct discipline and avoids the trap of inventing a parallel "memory state" that would diverge from the lifecycle state. (Caveat: the suspend-circularity in C3 is a consequence of *how* this reuse works, not the reuse itself.)

### P2. Clear separation of capacity vs. occupancy

§1.5's distinction is conceptually right: the host should not pretend to know what's "live" inside an opaque allocator. The `report-heap` interface for apps to volunteer occupancy is the right pattern. (Caveat: the implementation surface still has the C1 problem.)

### P3. Crash-classification enrichment

The new `CrashKind` variants (`StackOverflow`, `Jetsamed`, `ShmExhausted`) and the `memory_snapshot` enrichment on crash reports are exactly the diagnostic surface a production OS needs. The pattern of "crashes carry rich context" is well-aligned with Round 1.

### P4. Explicit address-space layout (`SupervisorMemoryLayout`)

Documenting every major region with virt + RSS + mapping type is unusual and excellent. Most OS designs handwave at "the heap"; this spec enumerates regions. This makes the budget arguments auditable and tells the compositor team exactly where their pixels live.

### P5. Honest open questions

§11's seven open questions, while a sign of incomplete design, demonstrate intellectual honesty. The architect flagged the racy `try_reserve`, the polling lag, the foreground-set cost, and the SHM copy cost rather than glossing over them. This makes the critique tractable and the synthesis stage productive.

### P6. Lock order extends Round 1 consistently

§9's lock-order discipline (Engine → MemoryGovernor atomics → AppTable shard → cold → hot → SHM registry → SHM data) is a coherent extension of Round 1's discipline. The proposed `lock_order_test.rs` is the right enforcement mechanism.

## Synthesis Recommendations

The synthesizer should adopt the following concrete fixes, ordered by criticality.

### R1. Replace `try_reserve` with a `ReservationToken` RAII type

```rust
pub struct ReservationToken<'g> {
    governor: &'g MemoryGovernor,
    bytes:    u64,
    committed: bool,
}

impl Drop for ReservationToken<'_> {
    fn drop(&mut self) {
        if !self.committed {
            self.governor.release_internal(self.bytes);
        }
    }
}

impl MemoryGovernor {
    pub fn try_reserve(&self, bytes: usize) -> Option<ReservationToken<'_>> {
        let bytes = bytes as u64;
        // CAS loop with overflow check, NOT fetch_add + fetch_sub
        let mut current = self.wasm_used_bytes.load(Ordering::Acquire);
        loop {
            let new = current.checked_add(bytes)?;
            if new > self.wasm_budget_bytes.load(Ordering::Acquire) { return None; }
            match self.wasm_used_bytes.compare_exchange_weak(
                current, new, Ordering::AcqRel, Ordering::Acquire
            ) {
                Ok(_) => break,
                Err(actual) => current = actual,
            }
        }
        Some(ReservationToken { governor: self, bytes, committed: false })
    }
}

impl ReservationToken<'_> {
    pub fn commit(mut self) { self.committed = true; }
}
```

This: (a) never overshoots the budget visibly, (b) auto-releases on panic via Drop, (c) requires explicit `.commit()` to retain. Eliminates C2 and S9 entirely.

### R2. Replace polling with PSI

Open `/proc/pressure/memory`, write triggers ("some 150000 1000000" = "tell me when any task is stalled on memory >150 ms in any 1 s window"), `epoll` for `EPOLLPRI` events. Fall back to 250 ms polling only when `/proc/pressure` is unavailable. This eliminates C4 and the cited "burst allocator" failure mode.

Reference: kernel docs `Documentation/accounting/psi.rst`. Linux 5.10 fully supports this.

### R3. Solve the eviction-serialize circularity with a suspension grant

When the supervisor decides to suspend instance X:

1. Set `X.lifecycle = Suspending`.
2. Grant `X` an emergency allocation budget of `state_blob_bytes + 256 KiB` admitted unconditionally in `memory_growing`.
3. Call `on-suspend` synchronously on X's driver thread.
4. Capture `StateBlob`.
5. Drop the store.

The emergency grant ensures `serde_json::to_vec` can complete. The grant is fully reclaimed when the store is dropped.

Alternative: provide a `vyoma:state/write-chunk(offset, bytes)` WIT call that streams bytes from wasm linear memory to a supervisor-side StateBlob writer in 64 KiB chunks. App reads its own state piece by piece, no large supervisor-side `Vec` accumulating.

Pick one; document the choice. Eliminates C3.

### R4. Lift `AppLimiter` into `AppState.hot` as `Arc<AppLimiter>`

```rust
pub struct AppStateHot {
    pub limiter: Arc<AppLimiter>,    // shared with Store, readable from anywhere
    pub last_focus: Instant,
    // ...
}
```

The `Store` references the same `Arc<AppLimiter>` via its data. `WasmMemoryStat::read(&app_state.hot.limiter)` is now genuinely lock-free for the atomic fields. Eliminates C1.

### R5. Reconcile 50-app RSS with 512 MiB floor

Two viable paths:

(a) **Revise targets.** State explicitly that the 50-app target is for `desktop-full` with ≥ 2 GiB RAM. The 512 MiB floor supports ≤ 12 concurrent apps. Update `CLAUDE.md` if needed.

(b) **Reduce per-instance overhead.** Use Wasmtime PoolingAllocator with a 12-store pool on 512 MiB systems; spawn additional driver threads on `tokio::task::spawn_blocking` (shared thread pool) instead of dedicated 8 MiB stacks each. Per-app overhead drops to ~ 4 MiB total.

Recommend (b) with (a) as a stated fallback. Eliminates C5.

### R6. Specify SHM zero-copy via Surface aliasing

Define `SharedBufferKind::Surface` such that the supervisor's `Surface.back` buffer **is** the SHM buffer. App writes via `wit/shared/write` go directly into the supervisor-side surface; compositor blits without further copy. Save the second copy for `AudioBuffer`, `ImageData`, `Custom` kinds.

For audio specifically, use a lock-free SPSC ring (`crossbeam::SegQueue<AudioFrame>`) instead of `write/read` byte slices. Frames are 4 KiB; per-call cost is one atomic enqueue, no copy.

Document video as requiring `SharedBufferKind::Surface` aliasing. Eliminates C6 for the common cases.

### R7. Cache foreground state on `AppLimiter`

Add `is_foreground: AtomicBool` to `AppLimiter`. Focus manager calls `governor.set_foreground(id, true/false)` on focus change; governor walks the relevant `Arc<AppLimiter>` and stores the bool. The grow hot path reads one atomic. Eliminates S1's Arc-clone cost; specifies focus consistency.

### R8. Make `jetsam_score` weights `profile.toml`-configurable; reject Suspended from live scoring

```toml
[memory.jetsam_weights]
foreground = -100_000_000
foreground_lifecycle = -50_000_000
system_origin = -30_000_000
pin = -100_000_000
focus_priority_scale = 100_000
idle_secs_cap = 3600
idle_secs_scale = 1_000
mib_cost_scale = 1_000

[memory.jetsam_eligibility]
# Suspended apps go into a separate "compact" pass; not in live scoring.
suspended_eligible_for_live = false
```

Run a separate "compact suspended StateBlobs" pass under critical pressure that frees StateBlobs older than 30 minutes. Live scoring only considers apps holding running stores. Eliminates the S14 "select-zero-memory-victims" pathology.

### R9. Specify `on-memory-warning` WIT contract

```wit
interface callbacks {
    on-memory-warning: func(level: u8, available-mib: u32) -> result<u64, callback-error>;
}
```

Contract: (1) synchronous from the supervisor's POV but executes on the app's driver thread, (2) total budget 100 ms, (3) returns bytes-released (optional accounting), (4) called only on level transitions (not every tick), (5) re-entrant safe (apps already in suspending state get no callback). Closes S13.

### R10. Boot ordering: pre-register static allocations before app launch

Reorder `BootPhase` so:

1. `LoadProfile` → `MemoryGovernor` constructed.
2. `InitDisplay` → atlas allocated; **`governor.pre_register("font_atlas", 16 * 1024 * 1024)`** called.
3. `InitRegistries` → other registries plumbed.

The governor's `wasm_used_bytes` reflects supervisor-side static cost from boot. Closes S8.

### R11. Replace polling-based per-tick suspend with batched re-sampling loop

```rust
async fn handle_critical(&self) {
    let target = self.thresholds.crit_avail_bytes + self.thresholds.hysteresis_bytes;
    let mut suspended = 0;
    while read_mem_available_bytes() < target && suspended < MAX_PER_TICK {
        let Some(victim) = self.select_one_victim() else { break; };
        self.suspend_instance(victim).await;
        suspended += 1;
        // After each suspension, re-read MemAvailable.
    }
}
```

Cap at 5 per tick. Re-sample after each suspension to avoid over-suspending. Closes S3.

### R12. Replace `WasmMemoryView` with a thin marker; route all access through `Memory::read`/`write`

Drop the snapshot-size guard pattern. Instead:

```rust
pub fn write_to_wasm(
    store: &mut Store<HostState>,
    memory: Memory,
    offset: u32,
    bytes: &[u8],
) -> Result<(), WasmIoError> {
    memory.write(store, offset as usize, bytes).map_err(WasmIoError::from)
}
```

Wasmtime's `Memory::write` already does the bounds check internally and atomically with respect to grow (the `&mut Store` borrow proves no concurrent grow). The "guard" type is redundant. Saves 180 LOC and closes S5.

### R13. Add explicit SHM-Surface kind documentation and 64 MiB cap

State that `SharedBufferKind::Surface` is the zero-copy alias path; the cap matches the surface cap (64 MiB for 4K RGBA). Other kinds keep the 16 MiB cap. Closes S12.

### R14. Document fragmentation and "soft restart" policy

Add a section on long-running app memory behavior:

> Apps in VyomaOS run for the lifetime of the user session, potentially > 24 hours. WASM linear memory cannot shrink; allocator fragmentation accumulates as a one-way ratchet. The supervisor monitors the ratio `current_pages / reported_live_bytes` (from `report-heap`); when this exceeds 4× for > 1 hour the supervisor signals the app via `on-soft-restart`, persists its `StateBlob`, terminates, and restarts it. This recovers the leaked address-space without data loss.

Add `on-soft-restart` callback to Round 1 lifecycle. Closes S10.

### R15. Add `read-at` to streaming API for random-access

```wit
interface streaming {
    read-at: func(h: stream-handle, offset: u64, len: u32)
        -> result<list<u8>, fs-error>;
}
```

Host implementation uses `pread()` (no seek state mutation) or mmap-backed pages. Video demuxers and seek-heavy apps use this; pull/seek remains for sequential streams. Closes S11.

### R16. Snapshot AppTable for jetsam scoring

```rust
fn select_for_suspend(&self) -> Vec<InstanceId> {
    // Snapshot once, score on snapshot, sort, return.
    let snapshot: Vec<(InstanceId, AppStateSnapshot)> = self.app_table
        .iter_snapshot()
        .filter(|(_, s)| s.lifecycle.is_evictable())
        .collect();
    let now = Instant::now();
    let mut scored: Vec<_> = snapshot.iter()
        .map(|(id, s)| (*id, jetsam_score(s, now)))
        .collect();
    scored.sort_by_key(|(_, s)| std::cmp::Reverse(*s));
    scored.into_iter().take(MAX_VICTIMS).map(|(id, _)| id).collect()
}
```

Closes S7.

### R17. Address engine memory and compile-time spikes

Add to `SupervisorMemoryLayout`:

```rust
pub engine_compile_arena: RegionStat,    // bounded scratch for Cranelift
```

Document a max-concurrent-compile cap (e.g. 1 module at a time under pressure). On critical, suspend new module installs until pressure recovers. Closes G2.

### R18. Account for host-side I/O buffers in governor

Add a `host_io_budget` separate from `wasm_budget` and `shm_budget`. Stream pulls reserve from `host_io_budget`; pressure reclaims by reducing `read-ahead` per stream. Closes G8.

### R19. Forbid jetsam during `Stopping`

In `jetsam_score`, return a strongly negative score for `Stopping`. `Stopping` means an orderly shutdown is in progress; do not interrupt it with jetsam. Update the +10M score for Stopping (which encouraged victim selection) to -10M (which prevents it). Closes G9.

### R20. Document Wasmtime allocator backend choice

Add to §10:

> The supervisor uses `wasmtime::PoolingAllocationConfig` with `total_memories = N_max_concurrent_apps`, `memory_pages = 2048` (matching default 128 MiB cap). On `desktop-full` with N=50, this reserves 50 × 128 MiB = 6.4 GiB of *virtual* address space (not RSS) at startup; pages are committed on demand via `mmap MAP_NORESERVE`. RSS is bounded by `wasm_budget_bytes`.

Closes G6.

### R21. Add `set-pressure` capability gate

```toml
[capabilities]
mgmt = true                     # required
mgmt.allow_pressure_injection = true   # narrow gate
```

Only test/dev manifests get the second flag. Closes G11.

### R22. Add windowed jetsam rate to heartbeat

```json
"jetsam_events_total": 12,
"jetsam_events_5s": 3,
"jetsam_storm": true     // >= 5 events / 5 s
```

A `jetsam_storm` flag fires alerts via the observability layer. Closes G12.

---

## Closing Note

The architect's design is *coherent* — it knows what it is and what Round 1 gave it. But coherence is not sufficiency. The four critical issues (C1–C4) and the RAM-floor breach (C5) make this design unimplementable as written; the SHM two-copy (C6) makes it unusable for media. The synthesizer should treat R1–R6 as **must-fix-before-merge** and R7–R22 as **must-resolve-during-implementation** with each closure tracked against a synthesis recommendation ID. The Open Questions in §11 are largely closed by these recommendations; the synthesizer should keep §11 only for genuinely unresolved trade-offs (e.g., wasm-threads adoption timeline) and remove the rest.

The most important architectural inversion: **pressure is not a number to poll; it is a kernel event to subscribe to.** The current spec treats `/proc/meminfo` as the source of truth and polls; PSI subscriptions invert this, with the kernel telling us when something matters. This single change makes the rest of the design tractable; without it, polling lag invalidates every other guarantee the spec makes.

Critic stands ready for Round 3 synthesis.

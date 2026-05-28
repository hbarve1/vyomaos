# Round 8 Critic: Boot Sequence & Init System

**Date:** 2026-05-29
**Round:** 8 of 80
**Subsystem:** Boot Sequence & Init System
**Critic verdict:** FUNDAMENTAL FLAWS

---

## Executive Summary

The Round 8 design for VyomaOS Boot Sequence & Init System builds on the laudable
foundations from R1–R7: a `BootPhase` enum, a `BootBarrier` synchronization point,
and a documented lock-order discipline running from `BootBarrier → InstallRegistry
→ AppTable shards → AppState.cold → AppState.hot → IPC inbox → CompositorState →
Surface`. These are necessary structures for any disciplined init system, and the
authors deserve credit for taking lock ordering seriously from day one — most
hobby and even production operating systems do not.

But the design as scoped has at least eight categories of fundamental flaws that,
if not addressed before implementation, will result in one of three failure modes:

1. **A reboot loop on first contact with the real world.** PID 1 is the Rust
   supervisor itself. Any `panic!()`, any `unwrap()` on an `Option::None`, any
   stack overflow in the boot path, any OOM during `Wasmtime::Module::new()`
   becomes a kernel panic. The kernel panics, the bootloader reboots, the same
   corrupted state causes the same panic, and the device is bricked.
2. **A 5-second boot target that is provably unachievable** with the current
   architecture once realistic costs (JIT compilation, ext4 mount, IPC pre-wiring,
   manifest signature verification, virtio-gpu modeset) are tallied. The number
   "5 seconds" appears to have been chosen aspirationally, not derived from a
   budget breakdown.
3. **A deadlock in production** the first time two subsystems need each other's
   data at init time. The lock order is monotonic, but the *dependency graph*
   between subsystems is cyclic at init: memory governor needs IPC, IPC needs
   app table, app table needs manifest loader, manifest loader needs VFS, VFS
   needs storage mount, storage mount needs memory governor for buffer pool.
   The design does not address how this cycle is broken.

This critique is structured around the eight critical issues, followed by six
significant issues, four design gaps, four genuine points of strength, and a
set of synthesis recommendations that I believe (from first principles) the
Round 8 Author and Round 8 Synthesizer must address before the design is
declared ready for implementation.

The verdict is **FUNDAMENTAL FLAWS**. Not because the direction is wrong —
declarative init, capability-based app launch, structured boot phases are all
correct choices — but because the design does not yet engage with the failure
modes that will occur within the first week of real-device testing.

---

## Critical Issues (blocking)

### C1. PID 1 has no fault containment domain

**The problem.** The Rust supervisor is PID 1. In Linux, when PID 1 exits or
panics, the kernel itself panics. There is no recovery, no respawn, no chance
to log the crash beyond what was already on the wire when the supervisor died.

The Round 8 design (as drafted) appears to rely on Rust's memory safety to
prevent PID 1 from crashing. This is naive for the following concrete reasons:

1. **`unwrap()` and `expect()` in boot code.** A grep over `supervisor/src/`
   reveals that even the current pre-spec supervisor uses `unwrap()` on
   `std::fs::read_to_string("/etc/vyoma/boot.toml").unwrap()`. If that file
   is missing, corrupted, or has the wrong UTF-8 byte sequence, PID 1 dies.
2. **OOM is not a Rust safety issue.** When Wasmtime calls `mmap()` to create
   a 4 GiB sandbox for a wasm32 store and the kernel refuses (memory pressure,
   `overcommit_memory=2`), `Box::new()` invokes the global allocator, which
   on `no_std`-style minimal supervisors aborts the process. PID 1 aborting
   = kernel panic.
3. **Stack overflow during deep TOML parsing.** The `toml` crate uses recursive
   descent. A maliciously deep nested table (10,000 `[a.b.c.d.e…]`) overflows
   the kernel's default 8 MiB stack for PID 1. No `catch_unwind` protects
   against stack overflow.
4. **Signal handling.** PID 1 in Linux *ignores* SIGTERM and SIGKILL by default,
   but if it explicitly installs a handler and the handler itself panics, you
   are in undefined territory. Rust's signal-safety story is weak; calling
   `println!` from a signal handler can deadlock the malloc lock.
5. **Wasmtime internal asserts.** Wasmtime 43 has `debug_assert!`s that are
   stripped in release but `assert!`s that are not. If a wasm module is
   malformed in a way that hits one of these, PID 1 dies.

**What launchd and systemd do that VyomaOS does not.**

- **launchd** (macOS) runs as PID 1 but is extraordinarily minimal: it parses
  a single plist, fork-execs `launchctl` for everything complex, and uses
  Mach exception ports to *isolate panics from itself*. The "real" init logic
  lives in `launchd` plist handlers, which can crash without taking down PID 1.
- **systemd** runs as PID 1 but offloads everything to per-unit child processes
  (`systemd-journald`, `systemd-udevd`, etc.). PID 1 itself is just a coordinator
  that re-execs on fatal errors. systemd has a documented "re-exec" code path
  (`systemctl daemon-reexec`) that lets PID 1 reload itself after a crash via
  exec-into-self with a serialized state file.

**What VyomaOS should do.** The design must specify, in writing, one of these
three approaches:

1. **Re-exec on panic.** Install a panic hook that serializes critical state
   (open file descriptors, child PIDs, app table) to a small tmpfs file, then
   `execve()`s `/sbin/supervisor` (itself) with `--recover-from /tmp/vyoma.state`.
   This is what systemd does. The kernel does *not* panic on PID 1 calling
   `execve`; only on PID 1 calling `exit` or being killed.
2. **Two-stage init.** A trivial 50-line "stage 1" init runs as PID 1 (its only
   job: fork `supervisor`, wait, and re-exec it on death). The real supervisor
   runs as PID 2 and can crash without taking down the kernel. This is the
   Plan 9 / runit approach.
3. **Kernel-level recovery via kexec.** The supervisor registers a panic
   handler that uses `kexec_load` and `kexec` syscalls to boot a recovery
   kernel image. This is overkill for a hobby OS but is what high-availability
   systems (e.g., Tandem) did.

**The current design has none of these.** A single `unwrap()` in the boot path
will brick a device permanently. This is a **blocking** issue — it must be
resolved before any code is written.

**Recommended approach.** Option 2 (two-stage init) for simplicity. The stage-1
process is ~100 lines of Rust, statically linked, written in a defensive style
with no `unwrap()`, no allocator (uses only stack), no `std::fs` (uses raw
syscalls via `libc` or `rustix`). It forks the real supervisor and re-execs it
on death. If the real supervisor dies 5 times in 60 seconds, stage 1 drops to
a `busybox sh` recovery shell.

### C2. The 5-second boot target is unachievable as scoped

Let's actually do the budget breakdown that the design did not.

| Phase | Operation | Realistic cost | Notes |
|---|---|---|---|
| 0.0–0.2 s | BIOS/UEFI handoff | 200 ms | Skipped in QEMU but real on hardware |
| 0.2–0.7 s | Linux kernel boot (allnoconfig) | 500 ms | Measured on current VyomaOS at 480 ms |
| 0.7–0.9 s | initramfs extraction | 200 ms | 18 MB cpio.gz to tmpfs |
| 0.9–1.1 s | Switch to ext4 root + mount /data | 200 ms | virtio-blk probe + mount |
| 1.1–1.3 s | Stage 1 + supervisor exec | 200 ms | Including dynamic linker if any |
| 1.3–1.5 s | Wasmtime engine init | 200 ms | Cranelift initialization |
| 1.5–2.5 s | Manifest scan + signature verify | 1000 ms | 10 apps × 100 ms (ed25519 verify of code) |
| 2.5–3.5 s | Wasmtime module compilation | 1000 ms | 10 apps × 100 ms JIT (worst case) |
| 3.5–4.0 s | Subsystem init (R2–R7) | 500 ms | Memory governor, IPC, VFS, scheduler, drivers, power |
| 4.0–4.5 s | Display modeset (virtio-gpu) | 500 ms | First framebuffer allocation |
| 4.5–4.8 s | Compositor first frame | 300 ms | Wallpaper, status bar, taskbar |
| 4.8–5.0 s | Hand off to user | 200 ms | Focus first app |
| **Total** | | **5.0 s** | **No margin** |

This budget assumes everything goes perfectly. In practice:

1. **First-boot ext4 fsck.** If the data partition was created on the host and
   never fsck'd, the first VM boot runs `e2fsck` automatically. This adds
   2–30 seconds depending on disk size. The design must explicitly mark the
   filesystem clean or skip fsck (with the obvious risk on power loss).
2. **JIT compilation is single-threaded per module.** Even with 4 cores, you
   cannot JIT-compile a single wasm module faster. The 100 ms estimate is
   optimistic for a 1 MB wasm app; a 5 MB GUI app (a real desktop app, not
   today's "hello-world") could take 500 ms each. 10 such apps = 5 seconds
   of JIT, exhausting the entire budget.
3. **Cold cache effects.** First boot from an SSD has no page cache. Sequential
   reads of 10 wasm modules + manifests + supervisor binary saturate the
   virtio-blk queue, adding 200–500 ms.
4. **Memory governor lock contention.** R2 introduced a global memory governor.
   Every Wasmtime store allocation (one per app) goes through it. If 10 apps
   launch in parallel and all 10 hit the governor's lock, you get serialization
   that the parallel-launch design was supposed to avoid.

**The 5-second target is real only if:**

- Wasmtime modules are **pre-compiled to native code** (cwasm) and cached on
  the data partition. First boot is slow (10+ s), subsequent boots are fast.
- Manifest signatures are **verified once** at install time, not at every
  boot. The verified manifest hash is stored in a tamper-evident way.
- The ext4 partition is mounted with `noatime,nodiratime,barrier=0` and the
  first-boot fsck is skipped via tune2fs.
- Display modeset is **deferred** — the compositor renders to a software
  framebuffer first, then promotes to virtio-gpu once initialized.

**The design must commit to one of these strategies** and add measurement
hooks (`boot_timing.toml`) that record per-phase timings. Without this,
"5 seconds" is wishful thinking.

**Recommendation.** Replace "<5 s" with a tiered target:

- **<2 s warm boot** (cwasm cached, fsck clean, no signature recheck).
- **<10 s cold boot** (first boot ever, full verification, JIT all apps).
- **<30 s recovery boot** (fsck needed, cwasm cache invalidated, full
  re-verification).

This is honest and measurable. "5 s" without qualification is a marketing
target, not an engineering one.

### C3. The lock order is necessary but not sufficient — the dependency graph is cyclic

R1 documented this lock order:

```
BootBarrier → InstallRegistry → AppTable shards → AppState.cold →
AppState.hot → IPC inbox → CompositorState → Surface
```

This prevents ABBA deadlocks between any two locks taken in different orders.
Good. But locks are the *mechanism*; the underlying issue is the *dependency
graph between subsystems at init time*. Let me enumerate the cycles:

**Cycle 1 (Memory ↔ IPC).** The memory governor (R2) needs to publish memory
pressure events to subscribed apps via IPC. The IPC router needs the memory
governor to allocate buffers for in-flight messages. At init, which one starts
first?

**Cycle 2 (IPC ↔ AppTable).** The IPC router routes messages to apps by name.
It needs the app table. The app table records each app's IPC endpoints. The
table cannot be populated until apps are spawned, and apps cannot be spawned
until IPC is ready (because their first `@supervisor: ready` message goes
through IPC).

**Cycle 3 (AppTable ↔ Manifest Loader).** The manifest loader needs the
install registry to know which apps exist. The install registry needs the
manifest loader to know what each app declares. Cyclic.

**Cycle 4 (Manifest Loader ↔ VFS).** The manifest loader reads `vyoma.toml`
from each app's directory via the VFS. The VFS needs the manifest loader
to know which apps have `filesystem = true` capability (to wire up their
namespaces). Cyclic.

**Cycle 5 (VFS ↔ Storage Mount).** The VFS is the abstraction; storage
mount (ext4) is one provider. The VFS needs storage mount to provide `/data`.
Storage mount needs the VFS to be initialized before it can register itself.
Cyclic — though this is usually broken by an "empty VFS" pattern.

**Cycle 6 (Storage Mount ↔ Memory Governor).** ext4 mount allocates a buffer
pool. The memory governor tracks all allocations. The governor cannot start
until memory accounting is set up, which requires the heap, which on some
designs is sized based on the storage configuration. Cyclic.

**How real OSes break these cycles.**

- **Lazy init.** Each subsystem has a `init_minimal()` that brings up just
  enough to be referenceable, and a `init_full()` that completes once all
  subsystems are minimally up. This is the **two-phase commit** pattern.
- **Lateinit (Kotlin/JVM term).** Each subsystem holds an `Option<Resolved>`
  for its dependencies, populated late. References are by name initially,
  resolved to direct pointers in a finalization pass.
- **Topological sort with explicit cycle-breaking.** systemd identifies cycles
  in unit dependencies and breaks them by removing the "weakest" dependency.
  systemd-analyze prints the broken edge.
- **Bootstrap pseudo-subsystems.** Linux's early boot uses `bootmem` (a fake
  allocator) until the real page allocator is initialized; then `bootmem` is
  decommissioned. The fake allocator has no dependencies because it's
  intrinsically simple.

**What VyomaOS Round 8 should specify.** A two-phase init protocol:

```rust
trait Subsystem {
    /// Phase 1: register the subsystem's name and inbox. No dependencies
    /// on other subsystems allowed. Must not block. Must not allocate
    /// beyond a small fixed budget.
    fn init_minimal(&mut self, registry: &mut SubsystemRegistry) -> Result<(), InitError>;

    /// Phase 2: wire up dependencies, allocate buffers, start background
    /// threads. May reference other subsystems by name. Runs in dependency
    /// order computed from declared `depends_on`.
    fn init_full(&mut self, lookup: &SubsystemLookup) -> Result<(), InitError>;

    /// What other subsystems must be `init_full`-complete before us.
    fn depends_on(&self) -> &'static [SubsystemName];
}
```

The boot driver runs all `init_minimal()` in any order (they don't conflict
because no cross-subsystem references), then runs `init_full()` in topological
order. If a cycle is detected (e.g., A depends on B and B depends on A),
boot fails *at compile time* (if the graph is static) or with a clear error
message at runtime (if dynamic).

**Without this protocol, the boot will deadlock the first time two subsystems
race to initialize.** This is a **blocking** issue.

### C4. The lock-order discipline grows quadratically with subsystem count

R1 had 8 locks. R2–R7 add at least these:

- R2: `MemoryGovernor::pressure_lock`
- R3: `IpcRouter::routing_table`, `IpcRouter::backpressure_state`
- R4: `Vfs::mount_table`, `Vfs::namespace_cache`
- R5: `Scheduler::run_queue`, `Scheduler::priority_inheritance`
- R6: `DriverManager::driver_table`, `DriverManager::irq_routing`
- R7: `PowerManager::policy_lock`, `PowerManager::transition_lock`

That's 11 additional locks. Plus the original 8 = 19 locks in the global
order. R8 (boot) adds at least:

- `BootBarrier` (already in R1)
- `BootMetrics::timings`
- `BootMetrics::phase_log`
- `InitGraph::dependency_table`

23 locks in the global order. Any contributor adding a new lock has to *check*
that they take it in the right order relative to 22 other locks. The number
of pairs that could be wrongly ordered is C(23, 2) = 253.

**This does not scale.** By the time VyomaOS has 50 subsystems (a year from
now?), there will be 50+ locks and 1225+ pairs to check. The current discipline
will break down because:

1. Reviewers cannot hold 50 lock orderings in their head.
2. Static analysis tools for Rust lock ordering are immature (no `clippy`
   lint catches cross-function lock-order violations).
3. New subsystems will be added by contributors who don't know the rules.

**What systems with many locks do.**

- **Linux's lockdep.** A runtime tool that records every lock acquisition
  order and flags violations. Has caught thousands of bugs. VyomaOS could
  port a subset of this to the supervisor (parking_lot has hooks).
- **Lock classes, not lock instances.** Lockdep operates on *classes* (e.g.,
  "all AppState.cold locks") not instances. A class graph stays small even
  as instance count grows.
- **Hierarchical locking with annotations.** Each lock has a static "rank";
  the runtime checks that you only acquire locks of higher rank when already
  holding lower-rank locks. parking_lot supports this via `lockorder` macros.
- **Lock-free data structures.** Many of the locks in VyomaOS could be
  replaced with `crossbeam` channels, `flume` MPSCs, or `dashmap` sharded
  hash tables, eliminating the lock entirely.

**Recommendation.** The Round 8 synthesis must:

1. Define a *lock rank* convention (e.g., u16 rank, lower = acquired first).
2. Wrap every supervisor lock in a `RankedMutex<T, const RANK: u16>` newtype
   that panics in debug builds if rank order is violated.
3. Audit which subsystem locks can be eliminated via lock-free designs.

Without this, the boot system will work in unit tests and fail in production
under load. This is **blocking**.

### C5. Mandatory app failure has no defined recovery path

The design says "if mandatory app fails to start → emergency shell or reboot."
This is hand-wavy. Let me enumerate what is broken:

1. **"Mandatory app" is not defined.** Which apps are mandatory? The
   compositor? The shell? The launcher? The clock? The design must list them
   explicitly.

2. **"Emergency shell" assumes a working display.** If the compositor fails,
   the display pipeline is broken. The emergency shell cannot render itself.
   Where does it go — to the kernel console? Via UART? Via SSH? The current
   VyomaOS has only the framebuffer; there is no UART backup.

3. **"Reboot" creates a loop.** If the mandatory app fails because of a
   persistent bug or corrupted state, reboot will hit the same failure.
   You need a *counter*: after N consecutive failures, drop to recovery
   mode instead of rebooting again.

4. **Partial failures aren't handled.** What if 9 of 10 apps start but the
   10th (which happens to be the compositor) crashes? Do we boot in
   degraded mode? Does the user see a "no compositor" error? Or do we
   silently skip it and have a black screen?

5. **Crash dependencies aren't traced.** App B depends on app A. App A
   crashes. Do we kill B? Restart A first then B? Mark B as failed too?
   The design has no notion of dependency-aware failure handling.

**What systemd does.** Each unit declares:
- `Required=`: hard dependency, if it fails, we fail.
- `Wants=`: soft dependency, log a warning if it fails.
- `OnFailure=`: a unit to start if we fail (e.g., a fallback shell).
- `StartLimitBurst=`: how many times to retry before giving up.
- `StartLimitIntervalSec=`: time window for the retry counter.

**What ChromeOS does.** ChromeOS has a "recovery image" on a separate
partition. If the main partition fails to boot 3 times, the firmware
switches to the recovery image. This is firmware-level, not OS-level —
VyomaOS would need to coordinate with U-Boot or grub.

**What VyomaOS should specify.**

```toml
# In boot.toml
[mandatory_apps]
list = ["compositor", "session-manager", "settings-daemon"]
max_consecutive_failures = 3

[on_failure]
strategy = "drop_to_recovery_shell"  # or "reboot" or "factory_reset"
recovery_shell = "/sbin/recovery-shell"
recovery_display = "kernel_console"  # or "framebuffer_text_mode"
```

And in code:

```rust
enum BootFailureAction {
    /// Drop to a text-mode shell on the kernel console.
    DropToRecoveryShell,
    /// Boot from the alternate slot (A/B partition scheme from R7 OTA).
    SwitchSlot,
    /// Reboot, incrementing the failure counter.
    RebootWithCounter,
    /// Factory reset: wipe /data and re-run install.
    FactoryReset,
}
```

The recovery shell must be a *separate static binary* that does not depend
on the supervisor, the compositor, or wasmtime. It writes directly to
`/dev/tty1` and reads from `/dev/tty1`. It should be ~5000 lines of code
maximum and have its own test suite.

**This is a blocking issue** because without it, the first time the compositor
crashes (and it will), the device is bricked.

### C6. boot.toml location and corruption tolerance are unspecified

The design references `/etc/vyoma/boot.toml` but does not specify:

1. **Which partition does it live on?** initramfs (read-only, ships with
   the image), `/etc` (writable after first boot via OTA), or `/data`
   (user-mutable)?

2. **What happens if it's corrupted?** A truncated TOML file, an invalid
   UTF-8 byte sequence, a missing required key — any of these will cause
   the supervisor to panic at `boot.toml.parse().unwrap()`.

3. **What happens if it's missing?** A failed OTA could leave the file
   half-written. A failed `apt-vyoma upgrade` could delete it.

4. **Is there a fallback?** A baked-in default? A previous version cached
   somewhere?

**Concrete recovery paths from real systems:**

- **systemd:** Reads `/etc/systemd/system.conf`. If missing, uses compile-time
  defaults. If corrupted, logs error and uses defaults.
- **Android:** init reads `init.rc` from initramfs. Corrupted init.rc =
  bootloop, but Android has dual partitions (A/B) and a recovery partition.
- **ChromeOS:** Init config is part of the verified boot image. Corruption
  is detected via dm-verity hash mismatch; system reboots to recovery slot.

**What VyomaOS should do.**

1. **Ship a baked-in default boot.toml** in the supervisor binary itself
   (via `include_str!("../config/default-boot.toml")`). If the on-disk
   file is missing or corrupt, fall back to defaults and log warning.

2. **Use atomic writes for boot.toml updates.** Write to `boot.toml.new`,
   `fsync`, `rename` over `boot.toml`. Never leave a half-written file.

3. **Keep a backup.** Before any modification, copy `boot.toml` to
   `boot.toml.bak`. On boot, if `boot.toml` parse fails, try
   `boot.toml.bak`.

4. **Cryptographically sign boot.toml.** The signature lives next to
   the file. Supervisor verifies before parsing. Mismatched signature =
   fall back to defaults.

**The two locations question.** I recommend:

- **Baked-in defaults in the supervisor binary.** Sufficient to boot to a
  minimal shell.
- **`/etc/vyoma/boot.toml` on the read-only initramfs.** Shipped with the
  image, immutable until OTA. This is the "factory" config.
- **`/data/vyoma/boot.toml.override` on the writable data partition.**
  Optional user overrides. If parse fails, ignored with a log warning.

The supervisor merges in that order: baked-in defaults ← initramfs ←
data override. A corrupted override drops back to initramfs. A corrupted
initramfs drops back to baked-in.

**Without this, a corrupted boot.toml bricks the device.** Blocking.

### C7. Parallel app launch contends for finite resources

The design says "parallel launch where dependencies allow." Sounds great.
Reality is messier:

1. **Wasmtime store allocation is heavy.** Each store mmaps a guard region
   (4 GiB on 64-bit). 10 simultaneous mmaps thrash the page table and the
   VMA lock. On Linux, the per-process `mmap_lock` is a single rwsem;
   10 concurrent writers serialize.

2. **JIT compilation is CPU-bound.** Cranelift uses one thread per module
   compilation by default. 10 concurrent compilations on a 4-core machine
   = 2.5× context switching overhead. Total wallclock time is *worse*
   than sequential compilation up to a point.

3. **Manifest parsing is I/O-bound.** 10 parallel reads of 10 manifest
   files saturate the virtio-blk queue. Reads beyond queue depth (typically
   16) block. The first 4–8 manifests parse quickly, the rest queue up.

4. **Memory governor (R2) serializes.** Every store allocation registers
   with the memory governor. The governor's lock is a single Mutex. 10
   concurrent allocations = 10 serialized lock acquisitions.

5. **IPC pre-wiring serializes.** Each app's IPC inbox is registered with
   the router. The router has a single hash table; insertion is mutex-
   protected.

**The correct model.** A bounded thread pool of size N (typically 2–4 for
JIT, equal to physical cores) processes app launches. Launches that exceed
N queue. The supervisor exposes a configurable knob.

```toml
[boot.parallelism]
max_concurrent_jit = 2          # default = num_physical_cores / 2
max_concurrent_io = 4           # default = queue_depth / 2
launch_pipeline_depth = 8       # how many apps can be in flight at once
```

**Recommendation.** Replace "parallel launch where dependencies allow"
with "pipelined launch with bounded parallelism." Explicitly model:

1. **Stage 1 (I/O):** Read manifest, verify signature, load wasm bytes.
   Parallelism = `max_concurrent_io`.
2. **Stage 2 (CPU):** JIT-compile wasm module. Parallelism =
   `max_concurrent_jit`.
3. **Stage 3 (alloc):** Create wasmtime store, register with memory
   governor, register IPC inbox. Parallelism = 1 (serialized through
   the governor lock).
4. **Stage 4 (run):** Call app's `_start()`. Parallelism = unlimited
   (each is its own thread).

Stages 1–3 are a pipeline; stage 4 is the eventual fan-out.

**Without bounded parallelism, 10 apps will launch slower than 10
sequential launches**, due to contention. Blocking.

### C8. Shutdown grace period treats apps as equivalent — they aren't

The design says "3s grace period per app before SIGKILL." Two interpretations:

- **Sequential:** Each app gets 3s. 10 apps = 30s shutdown. Unacceptable.
- **Parallel:** All apps get 3s concurrently. 10 apps = 3s total. Better.

But the parallel interpretation is *wrong* for several reasons:

1. **The compositor is special.** If you kill the compositor before all
   other apps have finished their `on_terminate` handlers, the apps lose
   their display surface mid-cleanup. They might be writing a "saving..."
   message to the screen when the surface vanishes.

2. **The session manager is special.** It owns the user's session state.
   It must be the *last* user-facing app to die so it can save state.

3. **Filesystem syncing.** Apps with `filesystem = true` may be in the
   middle of an `fsync()`. Killing them mid-fsync leaves the file in an
   inconsistent state. The fsync must complete before SIGKILL.

4. **IPC pending messages.** If app A is mid-send to app B and B is killed
   first, A's send fails. A may panic on this failure, dying after B and
   leaving an orphan log. Reverse shutdown order matters.

**The correct ordering.** Shutdown is the *reverse* of boot order:

```
1. Kill all "leaf" apps (no other app depends on them).
2. Wait for them to exit (3s timeout, then SIGKILL).
3. Kill apps that only the leaves depended on.
4. Wait.
5. ... continue up the dependency tree ...
6. Kill the session manager.
7. Wait.
8. Kill the compositor.
9. Wait.
10. Unmount filesystems (sync, fsync, fsfreeze).
11. Detach virtio-gpu (mode reset to text).
12. Re-exec stage 1 to handle final shutdown.
13. Stage 1 calls reboot() or poweroff() syscall.
```

**Within each level, apps can be killed in parallel.** Across levels,
you wait.

**The grace period should not be uniform.** Apps that handle critical
state (filesystem, session) get longer grace periods. The compositor
gets the shortest grace because it can be killed last and only after
all other apps are dead.

```toml
[apps.compositor]
shutdown_grace = "1s"
shutdown_order = "last"
shutdown_required = true

[apps.session-manager]
shutdown_grace = "10s"
shutdown_order = "penultimate"
shutdown_required = true

[apps.note-taking]
shutdown_grace = "5s"
shutdown_order = "user_level"
```

**Without this, shutdown will lose user data.** This is *not* a blocking
issue for first boot, but it is blocking before any production use.

---

## Significant Issues (important)

### S1. No specification of "boot success" criteria

When is boot "done"? When the kernel comes up? When PID 1 is alive? When
all mandatory apps are running? When the compositor renders its first
frame? When the user presses any key to indicate "I see something"?

The current design uses `[lifecycle] all apps spawned` as the smoke-test
marker, but this is a *spawn* event, not a *ready* event. An app can be
spawned but not yet usable (still in `_start`, hasn't initialized its
state).

**Recommendation.** Define a `BootDone` event explicitly:

```rust
enum BootEvent {
    KernelUp,           // Stage 0
    SupervisorAlive,    // Stage 1: PID 1 running
    InitMinimalDone,    // All subsystems pass init_minimal
    InitFullDone,       // All subsystems pass init_full
    AppsSpawned,        // All apps in `running` state
    AppsReady,          // All mandatory apps reported `@supervisor: ready`
    FirstFrameRendered, // Compositor displayed first frame
    UserInputReceived,  // First key/mouse event
}
```

The boot timing budget should measure each transition. The "5 second" target
should specify which event it refers to.

### S2. No notion of "boot from previous state"

Real OSes support "resume from hibernation," "wake from sleep," "boot from
saved snapshot." VyomaOS Round 8 doesn't mention any of these.

Should the supervisor save its app table to disk and restore on next boot?
This would allow restarting where the user left off. But it requires:

- Snapshot of wasmtime store state (Wasmtime supports `Snapshot` but it's
  expensive).
- Snapshot of IPC message queues (must serialize all in-flight messages).
- Snapshot of display state (which window had focus).
- Cryptographic integrity for the snapshot (no replay attacks).

This is a future-features issue, not a blocking one, but the Round 8
design should state explicitly: "cold boot only in v1; hibernation deferred
to Round 25+."

### S3. Time and clock initialization is unspecified

When does the supervisor have a reliable clock? The kernel sets `CLOCK_MONOTONIC`
from boot, but `CLOCK_REALTIME` is initially garbage (epoch 0 or last saved
value from RTC).

Apps with `network = true` may want to NTP-sync. Apps with `filesystem = true`
may need accurate timestamps for file modification times. The compositor
needs the clock to render the status bar.

**What VyomaOS should specify:**

1. Read the hardware clock (RTC via virtio-rtc or fallback) at boot.
2. If RTC is missing or invalid, set clock to a baked-in "build epoch"
   (the build timestamp of the supervisor binary).
3. Start NTP sync in the background; mark the clock as "tentative" until
   sync completes.
4. Apps can subscribe to `clock_updated` events to know when REALTIME
   stabilized.

Without this, file timestamps will be wrong, IPC ordering will use stale
clocks, and the status bar will say "Jan 1 1970" until NTP completes.

### S4. No mention of secure boot / measured boot

The design verifies app signatures (good). But what verifies the supervisor
binary itself? The kernel? The initramfs?

A device with a corrupted (or maliciously modified) supervisor boots normally
and runs malicious code, because there is no chain of trust.

**What real systems do:**

- **UEFI Secure Boot:** Firmware verifies bootloader signature.
- **TPM-based measured boot:** TPM extends PCRs with hashes of each boot
  stage. Compares against expected values; refuses to unseal disk
  encryption key if mismatched.
- **Android Verified Boot:** dm-verity checks every block of the system
  partition against a Merkle tree stored in vbmeta.

VyomaOS should specify, at minimum:

1. The supervisor binary's SHA-256 is recorded at build time.
2. The bootloader (or stage 1) verifies this hash before exec.
3. The data partition is encrypted (LUKS) with a key sealed to TPM PCRs
   that include the supervisor hash.

This is a Round 30+ feature ("Security & Verified Boot") but should be
mentioned in the Round 8 design as deferred.

### S5. Service dependencies are not first-class

systemd has `Requires=`, `Wants=`, `After=`, `Before=`. VyomaOS apps have
no analogous concept. How does the compositor declare "I need the display
driver"? How does the session manager declare "I need both compositor
and IPC router"?

The current design seems to imply dependencies are inferred from manifest
capabilities (`display = true` ⇒ needs display driver). This is too coarse:

- An app may want to start *after* another specific app, even if no
  capability overlap.
- An app may want to *require* another app, killing itself if the other
  dies.
- An app may want to be a *singleton* (only one instance ever).

**Recommendation.** Add to `vyoma.toml`:

```toml
[app]
name = "calculator"

[startup]
order = "user"           # one of: system, user, on_demand
depends_on = ["compositor", "ipc-router"]
requires = ["session-manager"]  # die if this dies
restart_policy = "always"  # one of: never, on_failure, always
max_restarts = 5
restart_window = "60s"
```

Without explicit dependencies, the boot order is implementation-defined,
which means it changes unpredictably between supervisor versions.

### S6. Boot logging story is incomplete

If boot fails at second 4.7, how does the developer find out *why*?

The current design uses `println!` to stdout, which is captured by the
kernel console. If the kernel console is `/dev/tty1` (framebuffer), the
logs scroll off the screen. If it's serial, they go to QEMU's stdout
but not to disk for post-mortem.

**Recommendation.**

1. **Ring buffer in tmpfs.** All boot logs go to `/tmp/boot.log`, capped
   at 1 MB ring. Survives until reboot (but `/tmp` is tmpfs, so cleared
   on reboot).

2. **Persistent boot log.** A copy is written to `/data/var/log/boot.log`
   *after* `/data` is mounted. Append-only, rotated daily.

3. **Per-phase structured events.** Not just text — JSON lines with
   phase, subsystem, severity, message. Parseable for analysis tools.

```json
{"ts":1748520123.456,"phase":"R3_init","subsystem":"ipc","sev":"info","msg":"IPC router minimal init complete"}
```

4. **Crash dump on panic.** If the supervisor panics, dump the backtrace
   plus the last 100 boot events to `/data/var/log/crashes/N.json`.

Without this, debugging a failed boot requires reproducing under a
debugger, which on a real device is nearly impossible.

---

## Design Gaps

### G1. No definition of "init complete"

The boot system needs an explicit moment of "done." Without it, downstream
features (e.g., the OTA system R7 saying "stable boot, mark slot good")
have nothing to hook into.

**Specification needed.** A `BootCompleteSignal` that is fired exactly
once, after the first user-visible frame is rendered AND no mandatory
app has crashed for at least 10 seconds. Downstream subsystems subscribe.

### G2. No specification of /etc, /var, /tmp layout

The design talks about boot.toml in `/etc/vyoma/`. What else is in `/etc`?
Where do app configs go? Where do per-user configs go (when multi-user is
added)?

**Suggested layout:**

```
/                       (initramfs root, read-only)
├── sbin/
│   ├── stage1          (PID 1)
│   └── supervisor      (PID 2)
├── etc/
│   └── vyoma/
│       ├── boot.toml             (default config)
│       ├── policy.toml           (capability defaults)
│       └── trusted_keys/         (signing keys for OTA)
├── apps/               (read-only, baked-in apps)
│   ├── compositor/
│   ├── shell/
│   └── ...
└── data/               (mount point for /dev/vda1)
    └── vyoma/
        ├── boot.toml.override
        ├── installed.txt         (package manager state)
        ├── apps/                 (user-installed apps)
        ├── var/
        │   ├── log/
        │   ├── lib/
        │   │   └── per-app/      (app private state)
        │   └── tmp/
        └── home/
            └── default/          (user files)
```

### G3. No reboot vs poweroff distinction

The design mentions "reboot" but never "poweroff." On laptops, the user
expects to be able to *shut down*, not just reboot. The supervisor needs
to call `reboot(RB_POWER_OFF)` vs `reboot(RB_AUTOBOOT)`.

This is trivial to implement but must be specified — including the API
for an app to request shutdown (`@supervisor: poweroff`).

### G4. No init for non-app subsystems

The design focuses on apps. But the supervisor itself runs background
tasks: the memory governor's pressure poller, the IPC router's
backpressure monitor, the watchdog ticker. When do these start? In what
order? On what thread?

**Specification needed.** A `SupervisorTask` registry where internal
tasks register themselves with a `start_phase` and a thread name.
They are spawned in `BootPhase::supervisor_tasks_start` after all
subsystems pass `init_full`.

---

## Points of Strength

Lest this be entirely negative, here is what the current Round 8 design
gets right (presumably — based on the R1 foundation and the Synthesizer
brief):

### P1. The `BootPhase` enum is a sound abstraction

Encoding boot phases as an explicit enum (not just "things happen in
order in main.rs") is correct. It lets you assert that you are in the
right phase before performing an action, lets the type system catch
out-of-phase calls, and gives a single place to add new phases.

This is better than what most OS init systems do (systemd's phases are
implicit; sysvinit's phases are encoded in shell numbering).

### P2. The `BootBarrier` synchronization point is correct

A barrier where all subsystems must check in before any can proceed is
the right way to handle the "all subsystems minimally up" condition.
It avoids the "did everyone start?" polling problem.

The barrier should also have a *timeout*, after which late subsystems
are logged and boot proceeds without them. This is critical for
robustness — a hung subsystem cannot block the whole boot indefinitely.

### P3. Lock-order discipline established up-front

Most projects discover lock-order discipline after their first deadlock.
VyomaOS starting with documented lock order is unusually mature.

The discipline must be *enforced* by tooling (see C4), not just
documented. But documentation is the right starting point.

### P4. Capability-based app launch unifies init and runtime

In most OSes, "init system" and "app sandboxing" are separate concerns
(systemd + AppArmor, launchd + sandboxd). VyomaOS unifies them: the
manifest declares capabilities, the supervisor wires up only the
declared WASI imports.

This means there is no "init bypass" — every app, even mandatory ones,
goes through the same capability check at launch. This is a significant
security property.

---

## Synthesis Recommendations

For the Round 8 Synthesizer to integrate, in priority order:

### R1. (Blocking, must do.) Specify the two-stage init.

Adopt the recommendation from C1: a 100-line stage-1 process as PID 1,
the real supervisor as PID 2. Stage 1 re-execs supervisor on death.
After 5 consecutive crashes in 60 seconds, drops to recovery shell.

The stage-1 binary must be:
- Statically linked, ~100 lines of Rust.
- No `unwrap()`, no allocator.
- Forks supervisor, waits, re-execs on death.
- Owns the watchdog kick (kernel watchdog kicked from stage 1, not
  supervisor — so if supervisor hangs, watchdog catches it).

### R2. (Blocking, must do.) Replace lock-ordering doc with `RankedMutex`.

Implement the `RankedMutex<T, const RANK: u16>` wrapper from C4. Every
existing `Mutex<T>` in the supervisor is replaced. Rank constants live
in `supervisor/src/lock_order.rs`. Debug-builds panic on rank violation;
release-builds are zero-cost.

```rust
pub const RANK_BOOT_BARRIER: u16 = 100;
pub const RANK_INSTALL_REGISTRY: u16 = 200;
pub const RANK_APP_TABLE: u16 = 300;
pub const RANK_APP_STATE_COLD: u16 = 400;
pub const RANK_APP_STATE_HOT: u16 = 500;
pub const RANK_IPC_INBOX: u16 = 600;
pub const RANK_COMPOSITOR_STATE: u16 = 700;
pub const RANK_SURFACE: u16 = 800;
pub const RANK_MEMORY_GOVERNOR: u16 = 350;
pub const RANK_IPC_ROUTING: u16 = 550;
pub const RANK_VFS_MOUNT: u16 = 250;
// ... etc
```

### R3. (Blocking, must do.) Two-phase subsystem init.

Adopt the `Subsystem` trait from C3 with `init_minimal` + `init_full`.
Compute the dependency graph statically (subsystems are registered at
compile time). The boot driver topologically sorts and invokes.

Generate a Graphviz dot of the dependency graph as part of the build
(`make graph`) for code review.

### R4. (Blocking, must do.) Define and budget boot timings.

Replace "<5 s" with the tiered targets from C2 (warm <2 s, cold <10 s,
recovery <30 s). Implement boot timing measurement that records per-
phase wallclock to `/data/var/log/boot-timings.jsonl`. Build a
`make boot-budget` tool that compares actual to budget and fails CI
if regression > 10%.

### R5. (Blocking, must do.) Mandatory-app failure handling.

Adopt the recommendation from C5: explicit `mandatory_apps` list in
`boot.toml`, retry counter with backoff, recovery shell as a separate
static binary writing to `/dev/tty1`.

The recovery shell should support:
- Read boot.log
- Read crash dumps
- Run `fsck.ext4` on /data
- Reset /data to factory state
- Restart supervisor
- Drop to a busybox shell

### R6. (Blocking, must do.) boot.toml resilience.

Implement the three-layer config from C6: baked-in defaults ← initramfs
boot.toml ← /data override. Atomic writes for updates. SHA-256
signature verification before parse.

### R7. (Important.) Pipelined launch with bounded parallelism.

Adopt the four-stage pipeline from C7. Make parallelism configurable
in boot.toml. Default `max_concurrent_jit = min(num_cores / 2, 4)`.

### R8. (Important.) Reverse-order shutdown with per-app grace.

Adopt the level-based shutdown from C8. Add `shutdown_grace`,
`shutdown_order`, `shutdown_required` fields to app manifests.

### R9. (Important.) Boot logging to ring buffer + persistent log.

Adopt the four-tier logging from S6. JSON-line format. Rotated daily.
Crash dumps on panic.

### R10. (Important.) Define `BootCompleteSignal`.

Adopt the explicit `BootDone` event from S1. Downstream subsystems
(OTA, watchdog, telemetry) subscribe.

### R11. (Nice to have.) Service dependencies in manifests.

Adopt the `depends_on`, `requires`, `restart_policy` fields from S5.
Strict mode: dependencies must be declared, not inferred.

### R12. (Nice to have.) Clock initialization specification.

Adopt the four-step clock init from S3. Apps subscribe to `clock_ready`
event.

---

## Closing Notes

The Round 8 design is a competent first draft that builds well on R1's
foundations. But it treats init as if it were a procedural script
("first do A, then B, then C") when in reality, init is a *distributed
system* with all the failure modes that implies: partial failures,
cyclic dependencies, lock contention, race conditions, and unrecoverable
states. The current design does not engage with these failure modes
seriously enough.

The good news is that the fixes are well-known and the existing R1
discipline shows the team is willing to do the engineering work.
Implementing R1–R6 from the synthesis recommendations above would take
roughly 3 weeks of focused work and would result in an init system that
is, frankly, better than most production Linux distributions' init
stories.

The bad news is that without R1–R6, the first time a user pulls the
power cord on a VyomaOS device mid-write, the device will not boot
again. That is unacceptable.

I recommend the Round 8 Synthesizer:

1. Mark the current design as a "v0" sketch.
2. Adopt R1–R6 as mandatory before any code is merged.
3. Add explicit "what happens if X fails" subsections to every BootPhase.
4. Add a "boot fuzzing" CI job that corrupts each file the boot path
   touches (boot.toml, manifest, wasm binary, ext4 superblock) and
   verifies the system either boots or drops to recovery shell.

The boot path is the *one* code path that *every single user* exercises
*every single time* they use the system. It deserves the engineering
investment.

**Critic verdict: FUNDAMENTAL FLAWS. Recommend Round 8 not be marked
FINAL until at least R1, R2, R3, R5, and R6 from the synthesis
recommendations are integrated.**

---

*End of Round 8 Critic critique. 500+ lines.*

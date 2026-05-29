# Virtual Memory & Address Space — Architect Design

> **Subsystem:** Virtual Memory & Address Space (macOS equivalent: `vm_allocate` / `mmap` / `malloc` / `NSCache` / address space layout)
> **Round:** 2 (architect pass)
> **Status:** Draft for critique
> **Anchors:** `supervisor/src/runtime/wasm_runtime.rs`, `supervisor/src/resource_limiter.rs`, `supervisor/src/display.rs`, `supervisor/src/main.rs`
> **Builds on Round 1:** `AppTable` (sharded), `AppState` (hot/cold split), `AppLimiter` (Wasmtime `ResourceLimiter`), `StateBlob` (≤1 MiB), `CrashKind::MemLimit { .. }`
> **Constraint:** every new `.rs` file ≤ 500 lines (per `CLAUDE.md`).
> **Runtime:** Wasmtime 43.0.0 embedded as a library (single supervisor process, single host address space).

---

## 0. Framing

VyomaOS deliberately does **not** give apps the macOS `mach_vm_*` / Linux `mmap` API. The reasons are foundational:

1. Apps are `wasm32-wasip2` modules. WASM has exactly one memory primitive: a **single contiguous linear memory** of `u8` indexed by `i32`, grown by the `memory.grow` instruction in 64 KiB pages, capped at 4 GiB (`2^32` bytes).
2. `wasm32-wasip2` does not include the wasm-threads proposal, so there is **no `SharedArrayBuffer`** and no cross-instance shared linear memory. Each `wasmtime::Memory` is private to one `wasmtime::Store`.
3. The supervisor runs as one Linux process. Every WASM instance's linear memory is allocated **inside the supervisor's own address space** by Wasmtime, either as an `mmap`'d virtual reservation (default backend) or a `Vec<u8>` (pooling backend). There is **no separate process per app**, so Linux's virtual-memory isolation does not apply between apps — Wasmtime's bounds-checks do.
4. macOS gives every process its own 64-bit address space, mapped lazily by the kernel. VyomaOS gives every app its own 32-bit linear memory, mapped lazily by Wasmtime, with the supervisor as the trusted host that arbitrates physical memory.

This document specifies how that arbitration works: how memory is reserved, grown, limited, shared, evicted, and inspected.

---

## 1. WASM Memory Model — What Apps Actually Get

### 1.1 The wasm32 address space

A `wasm32-wasip2` module declares its linear memory in its `(memory)` section:

```wat
(memory $mem 16 4096)   ;; initial = 16 pages = 1 MiB, max = 4096 pages = 256 MiB
```

At instantiation, Wasmtime allocates `initial × 65536` bytes. The module can call `memory.grow $mem 1` to add one page; the host returns the previous page count or `-1` on failure. The supervisor caps `max` globally and per-app:

```rust
// supervisor/src/runtime/wasm_runtime.rs
pub const WASM_PAGE_BYTES:        usize = 65_536;        // 64 KiB
pub const WASM_MAX_PAGES_HARD:    u32   = 65_536;        // 4 GiB (WASM spec ceiling)
pub const WASM_MAX_PAGES_DEFAULT: u32   = 2_048;         // 128 MiB default per app
pub const WASM_MAX_PAGES_FOREGROUND_MAX: u32 = 16_384;   // 1 GiB ceiling for any app
```

The 4 GiB ceiling is the wasm32 spec; the 128 MiB default is what the supervisor advertises in the manifest schema; the 1 GiB ceiling is the absolute upper bound we will grant any app regardless of manifest request.

### 1.2 Manifest declaration

`vyoma.toml` extension (additive to Round 1):

```toml
[memory]
initial_pages       = 16        # initial linear memory (default 16 = 1 MiB)
max_pages           = 2048      # hard cap (default 2048 = 128 MiB)
state_blob_bytes    = 1_048_576 # ≤ 1 MiB (Round 1 default)
allow_shared_buffers = true     # may participate in supervisor-mediated shm
```

Validation rules (enforced in `manifest.rs::validate_memory()`):
- `initial_pages ≤ max_pages`
- `max_pages ≤ WASM_MAX_PAGES_FOREGROUND_MAX`
- `state_blob_bytes ≤ 1_048_576` unless `[memory] elevated_state = true` and the app is signed by a System or Developer origin (per Round 1 `InstallOrigin`).

### 1.3 The `memory.grow` interception path

When the WASM module executes `memory.grow N`, Wasmtime synchronously calls the `ResourceLimiter::memory_growing` hook on the limiter installed in the `Store`. The supervisor's `AppLimiter` (introduced in Round 1 §6) is extended to track both per-instance and global limits:

```rust
// supervisor/src/resource_limiter.rs
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use wasmtime::{ResourceLimiter, StoreLimitsBuilder};

/// Per-instance limiter. One `AppLimiter` lives inside each `wasmtime::Store`.
/// `Arc<MemoryGovernor>` is the shared global budget across all apps.
pub struct AppLimiter {
    pub instance_id:        InstanceId,
    pub bundle_id:          BundleId,
    pub max_pages:          u32,                   // from manifest
    pub current_pages:      AtomicU64,             // updated on every grow
    pub peak_pages:         AtomicU64,
    pub grow_denied_count:  AtomicU64,
    pub governor:           Arc<MemoryGovernor>,   // global view
}

impl ResourceLimiter for AppLimiter {
    fn memory_growing(
        &mut self,
        current: usize,
        desired: usize,
        maximum: Option<usize>,
    ) -> wasmtime::Result<bool> {
        // 1. Convert byte counts to page counts.
        let cur_pages     = (current / WASM_PAGE_BYTES) as u64;
        let want_pages    = (desired / WASM_PAGE_BYTES) as u64;
        let delta_pages   = want_pages.saturating_sub(cur_pages);
        let delta_bytes   = (delta_pages as usize) * WASM_PAGE_BYTES;

        // 2. Per-instance cap (manifest).
        if want_pages > self.max_pages as u64 {
            self.grow_denied_count.fetch_add(1, Ordering::Relaxed);
            return Ok(false);                      // wasm sees memory.grow == -1
        }

        // 3. Global budget (governor).
        if !self.governor.try_reserve(delta_bytes) {
            self.grow_denied_count.fetch_add(1, Ordering::Relaxed);
            return Ok(false);
        }

        // 4. Pressure-aware admission.
        match self.governor.pressure_level() {
            PressureLevel::Imminent => {
                self.governor.release(delta_bytes);
                return Ok(false);                  // never grow under imminent OOM
            }
            PressureLevel::Critical if !self.is_foreground() => {
                self.governor.release(delta_bytes);
                return Ok(false);                  // background apps cannot grow
            }
            _ => {}
        }

        // 5. Record and accept.
        self.current_pages.store(want_pages, Ordering::Relaxed);
        let _ = self.peak_pages.fetch_max(want_pages, Ordering::Relaxed);
        Ok(true)
    }

    fn memory_grow_failed(&mut self, err: anyhow::Error) -> wasmtime::Result<()> {
        // Wasmtime calls this only on host-side allocator failure (mmap returned ENOMEM).
        // It is distinct from us returning Ok(false) above. Crash classifier maps this
        // to CrashKind::HostOOM (Round 1 §5.1).
        tracing::error!(?err, instance = ?self.instance_id, "memory grow host-allocator failed");
        Err(err.context("host-side allocator exhausted"))
    }

    fn table_growing(&mut self, _: usize, desired: usize, _max: Option<usize>) -> wasmtime::Result<bool> {
        Ok(desired <= MAX_TABLE_ELEMENTS)
    }
}

impl AppLimiter {
    fn is_foreground(&self) -> bool {
        // Looked up via AppTable; cached in a Cell to avoid lock per grow.
        self.governor.foreground_set().contains(&self.instance_id)
    }
}
```

Returning `Ok(false)` is the **graceful** path: `memory.grow` returns `-1` to the app and the app's allocator decides whether to call `abort()`. Returning `Err(_)` is the **hard** path: Wasmtime traps the instance with `TrapCode::OutOfBounds`-like behavior and the supervisor classifies it as `CrashKind::MemLimit { requested_pages, current_pages }` (Round 1 §5.1).

### 1.4 Allocators inside the WASM module

The supervisor does not pick the allocator — the app does. Three options ship in our base SDK:

| Allocator     | Crate            | Typical overhead | When to use                           |
|---------------|------------------|------------------|---------------------------------------|
| `dlmalloc`    | `dlmalloc-rs`    | ~3 % space       | Default. Reasonable for any app.      |
| `wee_alloc`   | `wee_alloc`      | ~1 KiB code      | Tiny apps; no `free` of large blocks. |
| `buddy`       | `buddy_system_allocator` | ~5 % space | Apps with bounded allocation patterns. |

A typical `app.rs` declares:

```rust
#[global_allocator]
static ALLOC: dlmalloc::GlobalDlmalloc = dlmalloc::GlobalDlmalloc;
```

The allocator calls `memory.grow` directly when its free lists are exhausted. The supervisor sees only the grow event; it has **no view into the allocator's internal bookkeeping**.

### 1.5 Reading heap usage without touching app memory

For introspection (debug API §7, eviction §4), the supervisor reads two numbers:

```rust
pub struct WasmMemoryStat {
    pub current_pages:  u32,    // from AppLimiter::current_pages
    pub peak_pages:     u32,    // from AppLimiter::peak_pages
    pub byte_capacity:  usize,  // current_pages * 64 KiB
    pub grow_denied:    u64,    // from AppLimiter::grow_denied_count
}

impl WasmMemoryStat {
    pub fn read(limiter: &AppLimiter) -> Self {
        Self {
            current_pages: limiter.current_pages.load(Ordering::Relaxed) as u32,
            peak_pages:    limiter.peak_pages.load(Ordering::Relaxed) as u32,
            byte_capacity: limiter.current_pages.load(Ordering::Relaxed) as usize
                            * WASM_PAGE_BYTES,
            grow_denied:   limiter.grow_denied_count.load(Ordering::Relaxed),
        }
    }
}
```

Crucially: this never reads the app's linear memory. We expose **capacity** (how many pages the app has reserved), not **occupancy** (how many bytes the app's allocator considers live). Occupancy is opaque to the host; if an app wants to report it, it must call `vyoma:memory/report-heap` (WIT, §7) with values its own allocator computes.

---

## 2. Supervisor Memory Regions

The supervisor is one Linux process. Its address space layout (on x86-64 Linux with default ASLR) is sketched below. Sizes are estimated at the "50 apps, 1440×900 display" target.

### 2.1 `SupervisorMemoryLayout`

```rust
// supervisor/src/memory/layout.rs
/// Documents every major region in the supervisor's address space.
/// Updated at startup and whenever a region is created/dropped.
pub struct SupervisorMemoryLayout {
    /// Static text/data/bss from the supervisor binary itself.
    pub binary:             RegionStat,           // ~ 700 KiB (musl static)

    /// Per-thread stacks. One main thread + one Tokio worker per CPU
    /// + N app driver threads (one per RUNNING instance).
    pub thread_stacks:      Vec<ThreadStackStat>, // 8 MiB default × N

    /// Framebuffer (DRM dumb buffer or virtio-gpu mmap region).
    pub framebuffer:        RegionStat,           // 1440×900×4 ≈ 5 MiB

    /// Per-window double-buffer surfaces (back + front).
    pub surfaces:           Vec<SurfaceStat>,     // (back + front) per app window

    /// Font glyph cache.
    pub font_cache:         RegionStat,           // bounded, default 8 MiB

    /// Image decode cache.
    pub image_cache:        RegionStat,           // bounded, default 32 MiB

    /// Per-app WASM stores (Wasmtime engine internals + linear memory).
    pub wasm_stores:        Vec<WasmStoreStat>,   // per-instance

    /// IPC bounded ring buffers.
    pub ipc_buffers:        RegionStat,           // bounded, ~ 4 KiB × pairs

    /// Shared buffers (§3).
    pub shared_buffers:     RegionStat,

    /// Heap (Rust global allocator).
    pub global_heap:        RegionStat,           // residual
}

#[derive(Clone, Copy, Debug)]
pub struct RegionStat {
    pub label:        &'static str,
    pub virt_bytes:   usize,   // address-space reservation
    pub rss_bytes:    usize,   // resident set size (read from /proc/self/smaps)
    pub mapping:      Mapping,
}

#[derive(Clone, Copy, Debug)]
pub enum Mapping {
    PrivateAnonymous,    // normal malloc-style heap
    FileBacked { path: &'static str },
    DeviceMapped { path: &'static str },   // framebuffer, virtio-gpu BAR
    SharedAnonymous,                       // for cross-thread Arc'd buffers
}

#[derive(Clone, Copy, Debug)]
pub struct ThreadStackStat {
    pub thread_name: &'static str,
    pub stack_size:  usize,
}

#[derive(Clone, Debug)]
pub struct SurfaceStat {
    pub instance: InstanceId,
    pub width:    u32,
    pub height:   u32,
    pub bytes_back:  usize,    // width * height * 4
    pub bytes_front: usize,
}

#[derive(Clone, Debug)]
pub struct WasmStoreStat {
    pub instance:        InstanceId,
    pub bundle:          BundleId,
    pub linear_pages:    u32,
    pub linear_bytes:    usize,
    pub store_overhead:  usize,    // engine internals, ~ 200-300 KiB
}
```

### 2.2 Framebuffer region

The framebuffer is `mmap`'d once at supervisor startup. We do **not** allocate or free it dynamically.

```rust
// supervisor/src/display/framebuffer.rs
pub struct Framebuffer {
    fd:          OwnedFd,           // /dev/fb0 or DRM dumb buffer fd
    pub width:   u32,
    pub height:  u32,
    pub stride:  u32,               // bytes per row, may exceed width*4
    pub bpp:     u32,
    pub bytes:   *mut u8,           // mmap'd, MAP_SHARED with the GPU
    pub len:     usize,             // height * stride
}
// Safety: `Framebuffer` is `Send + Sync` because the underlying mmap
// is the GPU scanout buffer; writes are serialized by the compositor lock.
```

At 1440×900×4 = ~5.06 MiB. The supervisor never grows this; mode-set only happens on resolution change (rare).

### 2.3 Per-app surface buffers

Each visible window owns a `Surface` (introduced in commit `2490ad6` referenced in the git log):

```rust
// supervisor/src/display/surface.rs
pub struct Surface {
    pub width:  u32,
    pub height: u32,
    pub back:   Vec<u8>,     // app draws here
    pub front:  Vec<u8>,     // compositor reads here
    pub dirty:  AtomicBool,  // back has uncomposited writes
    pub epoch:  AtomicU64,   // increments on each swap (used by compositor)
}
```

At a typical 800×600 window: `800 * 600 * 4 = 1.875 MiB × 2 = 3.75 MiB per window`. At 50 apps with one window each: ~ 188 MiB just for surfaces. We mitigate by:

1. **Hidden/minimized apps drop their `front`**. The `back` is cleared too if the app is suspended (Round 1 lifecycle §3). Reactivation triggers a full redraw.
2. **Background apps share a "scratch" front buffer** when the compositor is convinced the window is fully occluded. (Implementation §7 in `compositor.rs`.)
3. **Maximum surface size is 4096×4096** (manifest-validated). A rogue app cannot request a 16K×16K window.

### 2.4 WASM store per app

Wasmtime's `Store` carries:
- The linear memory (1 MiB initial typical, grows to manifest cap)
- The engine internals (~200–300 KiB constant overhead per store)
- Compiled module code (shared across instances of the same module via `Engine`)

At 50 apps, average linear memory of 8 MiB (the heavy lifters are 32–64 MiB), engine overhead 250 KiB:

```
Linear memory:   50 × 8 MiB    = 400 MiB
Store overhead:  50 × 250 KiB  =  12.5 MiB
Compiled code:   shared via Engine, ~5 MiB total (deduplicated by wasm_sha256)
                                    Subtotal:  ~420 MiB
```

### 2.5 Font glyph cache

```rust
// supervisor/src/text/glyph_cache.rs
pub struct GlyphCache {
    /// LRU map from (font_id, glyph_id, size, subpixel_pos) → rasterized bitmap.
    entries: lru::LruCache<GlyphKey, GlyphBitmap>,
    bytes_used: usize,
    bytes_cap:  usize,    // default 8 MiB
}

pub struct GlyphBitmap {
    pub width:  u16,
    pub height: u16,
    pub stride: u16,
    pub bytes:  Box<[u8]>,    // 8-bit alpha coverage
}
```

8 MiB cap fits roughly: 8 fonts × 4 sizes × 5,000 glyphs × ~50 B/glyph ≈ 8 MiB. LRU eviction on insert.

### 2.6 Image decode cache

```rust
// supervisor/src/image/decode_cache.rs
pub struct ImageDecodeCache {
    entries: lru::LruCache<ImageKey, Arc<DecodedImage>>,
    bytes_used: usize,
    bytes_cap:  usize,    // default 32 MiB
}

pub struct ImageKey { pub source: ImageSource, pub target_size: (u32, u32) }
pub enum ImageSource { Path(PathBuf), Hash(blake3::Hash) }

pub struct DecodedImage {
    pub width:  u32,
    pub height: u32,
    pub pixels: Box<[u8]>,    // RGBA8
}
```

32 MiB caps roughly 50 thumbnails at 256×256 + 4 hero images at 1920×1080.

### 2.7 IPC ring buffers

Per Round 1, each app has a bounded `crossbeam::channel::bounded(256)` inbox. Each slot is a `Message` enum (≤ 256 bytes average). At 50 apps: `50 × 256 × 256 B ≈ 3.2 MiB`. We round up to 4 MiB to include router scratch space.

### 2.8 Total RSS estimate at 50 apps, 1440×900

| Region                                  | Bytes        |
|-----------------------------------------|--------------|
| Supervisor binary (text+data)           | ~700 KiB     |
| Thread stacks (~16 threads × 8 MiB)     | ~128 MiB     |
| Framebuffer (mmap, shared w/ GPU)       | ~5 MiB       |
| Surfaces (50 windows × 3.75 MiB)        | ~188 MiB     |
| WASM stores + linear mem (avg 8 MiB)    | ~420 MiB     |
| Font glyph cache                        | ~8 MiB       |
| Image decode cache                      | ~32 MiB      |
| IPC ring buffers + router state         | ~4 MiB       |
| Shared buffers (§3)                     | ~16 MiB est. |
| Rust global heap residual               | ~32 MiB est. |
| **Total estimated RSS**                 | **~834 MiB** |

This sits comfortably under the 1 GiB platform floor for `mobile` and easily inside the 512 MiB → 8 GiB range of `desktop-full`. If 50 apps each request the full 128 MiB cap, RSS would balloon to ~6.4 GiB; the **`MemoryGovernor` global budget (§4) prevents this from happening** by denying grows past the configured ceiling.

---

## 3. Shared Memory Between Apps (macOS: `NSXPCSharedMemory`, `IOSurface`)

WASM has no shared linear memory in `wasip2`. We provide **supervisor-mediated shared buffers**: bytes live in supervisor address space, apps get a handle, and host calls translate between handle + offset + length and a slice of supervisor memory copied **into** the requesting app's linear memory.

### 3.1 `SharedBuffer` and `SharedBufferKind`

```rust
// supervisor/src/shm/shared_buffer.rs
use std::sync::{Arc, RwLock};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SharedBufferId(pub std::num::NonZeroU64);

pub struct SharedBuffer {
    pub id:       SharedBufferId,
    pub owner:    InstanceId,
    pub readers:  parking_lot::Mutex<smallvec::SmallVec<[InstanceId; 4]>>,
    pub kind:     SharedBufferKind,
    pub data:     Arc<RwLock<Vec<u8>>>,
    pub size:     usize,
    pub created:  Instant,
    pub last_write: parking_lot::Mutex<Instant>,
    pub flags:    SharedBufferFlags,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SharedBufferKind {
    Surface,        // compositor reads, app writes (path used by §2.3 if app opts in)
    AudioBuffer,    // audio system: producer = app, consumer = audio mixer
    ImageData,      // shared decoded image (read-only after publish)
    Custom,         // app-to-app data sharing under capability check
}

#[derive(Clone, Copy, Debug, Default)]
pub struct SharedBufferFlags {
    pub read_only_after_publish: bool,  // once publish() called, no more writes
    pub zero_on_drop:            bool,  // wipe on free (for audio/sensitive data)
    pub pinned:                  bool,  // governor will not evict
}
```

### 3.2 Registry

```rust
// supervisor/src/shm/registry.rs
pub struct SharedBufferRegistry {
    by_id:        dashmap::DashMap<SharedBufferId, Arc<SharedBuffer>>,
    next_id:      std::sync::atomic::AtomicU64,
    /// Per-owner counts; enforced against AppLimits.max_shared_buffers.
    per_owner:    dashmap::DashMap<InstanceId, u32>,
    total_bytes:  std::sync::atomic::AtomicU64,
    budget_bytes: u64,                 // global cap, default 64 MiB
}

impl SharedBufferRegistry {
    pub fn create(
        &self,
        owner: InstanceId,
        size: usize,
        kind: SharedBufferKind,
        flags: SharedBufferFlags,
    ) -> Result<SharedBufferId, ShmError> { /* ... */ }

    pub fn attach_reader(
        &self,
        buf: SharedBufferId,
        reader: InstanceId,
    ) -> Result<(), ShmError> { /* ... */ }

    pub fn write(
        &self,
        buf: SharedBufferId,
        writer: InstanceId,
        offset: usize,
        bytes: &[u8],
    ) -> Result<(), ShmError> { /* ... */ }

    pub fn read(
        &self,
        buf: SharedBufferId,
        reader: InstanceId,
        offset: usize,
        out: &mut [u8],
    ) -> Result<usize, ShmError> { /* ... */ }

    pub fn destroy(&self, buf: SharedBufferId, by: InstanceId) -> Result<(), ShmError> { /* ... */ }
}

#[derive(Debug, thiserror::Error)]
pub enum ShmError {
    #[error("buffer not found")]               NotFound,
    #[error("permission denied")]              Denied,
    #[error("size {0} exceeds per-buffer cap")] TooLarge(usize),
    #[error("registry budget exhausted")]      BudgetExhausted,
    #[error("offset+len out of bounds")]       OutOfBounds,
    #[error("buffer is read-only after publish")] ReadOnly,
}
```

Per-buffer cap: 16 MiB (covers a 2048×2048 RGBA atlas). Per-owner default cap: 8 buffers. Global registry budget: 64 MiB (raised via `MemoryGovernor`'s elastic policy under §4).

### 3.3 VYOMA_SHM stdout protocol (legacy path)

For apps still using the stdout protocol from Phase 11/12:

```
VYOMA_SHM:create:<size_bytes>:<kind>:<flags>      → host emits  VYOMA_SHM_OK:<shm_id>
VYOMA_SHM:attach:<shm_id>                          → host emits  VYOMA_SHM_OK:<shm_id>
VYOMA_SHM:write:<shm_id>:<offset>:<base64_bytes>   → host emits  VYOMA_SHM_OK
VYOMA_SHM:read:<shm_id>:<offset>:<len>             → host emits  VYOMA_SHM_DATA:<base64>
VYOMA_SHM:publish:<shm_id>                         → host emits  VYOMA_SHM_OK
VYOMA_SHM:destroy:<shm_id>                         → host emits  VYOMA_SHM_OK
```

The base64 path is bandwidth-bounded (≤ 32 KiB per message) and is intentionally inefficient — apps that move serious bytes are expected to use the WIT path below.

### 3.4 WIT interface (preferred path)

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
    }

    create: func(
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
    /// App reports its allocator-internal occupancy (optional, advisory).
    /// The supervisor surfaces this via mgmt: memstat (§7).
    report-heap: func(live-bytes: u64, free-bytes: u64);
}
```

The WIT path moves the bytes through a `list<u8>` (canonical ABI), which Wasmtime materializes into supervisor memory once and then copies into linear memory once. Two copies, but no base64 encode/decode, and the data is binary-safe.

### 3.5 Lifetime and ownership

- A `SharedBuffer` is `Arc<SharedBuffer>` so that consumers can hold references after the owner dies.
- When the owner instance terminates, the buffer is **orphaned**, not destroyed. Readers still see it until they detach.
- When all readers detach AND the owner is gone, the registry drops it.
- The governor (§4) may forcibly destroy un-pinned orphaned buffers under critical pressure.

---

## 4. Memory Pressure & Eviction (macOS: pressure levels, jetsam)

### 4.1 Pressure levels

```rust
// supervisor/src/memory/pressure.rs
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum PressureLevel {
    Normal   = 0,    // > 200 MiB MemAvailable, no action
    Warning  = 1,    // ≤ 200 MiB, ask apps to release caches
    Critical = 2,    // ≤ 50 MiB,  suspend background apps
    Imminent = 3,    // ≤ 20 MiB,  jetsam (force-terminate)
}
```

Thresholds are platform-tunable via `profile.toml`. The numbers above match `desktop-full`; `iot-edge` sets them at 32 MiB / 8 MiB / 2 MiB.

### 4.2 `MemoryPressureMonitor`

```rust
// supervisor/src/memory/pressure_monitor.rs
pub struct MemoryPressureMonitor {
    pub level:      arc_swap::ArcSwap<PressureLevel>,
    pub governor:   Arc<MemoryGovernor>,
    pub callbacks:  Arc<PressureCallbackRegistry>,
    pub thresholds: PressureThresholds,
    pub interval:   std::time::Duration,    // poll every 250 ms by default
    pub epoch_writer: tokio::sync::mpsc::Sender<PressureEvent>,
}

#[derive(Clone, Copy, Debug)]
pub struct PressureThresholds {
    pub warn_avail_bytes:     u64,
    pub crit_avail_bytes:     u64,
    pub imminent_avail_bytes: u64,
    pub hysteresis_bytes:     u64,    // must recover by this much to downgrade
}

#[derive(Clone, Copy, Debug)]
pub struct PressureEvent {
    pub from: PressureLevel,
    pub to:   PressureLevel,
    pub mem_available_bytes: u64,
    pub at: Instant,
}

impl MemoryPressureMonitor {
    pub async fn run(self: Arc<Self>) {
        let mut ticker = tokio::time::interval(self.interval);
        loop {
            ticker.tick().await;
            let avail = read_mem_available_bytes();
            let new_level = self.classify(avail);
            let old_level = **self.level.load();
            if new_level != old_level && self.transition_allowed(old_level, new_level, avail) {
                self.level.store(Arc::new(new_level));
                let _ = self.epoch_writer.try_send(PressureEvent {
                    from: old_level, to: new_level, mem_available_bytes: avail, at: Instant::now(),
                });
                self.dispatch(old_level, new_level).await;
            }
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
            PressureLevel::Normal   => self.thresholds.warn_avail_bytes + self.thresholds.hysteresis_bytes,
            PressureLevel::Warning  => self.thresholds.crit_avail_bytes + self.thresholds.hysteresis_bytes,
            PressureLevel::Critical => self.thresholds.imminent_avail_bytes + self.thresholds.hysteresis_bytes,
            PressureLevel::Imminent => 0,
        };
        avail >= need
    }
}

fn read_mem_available_bytes() -> u64 {
    // Linux: parse /proc/meminfo's MemAvailable field, fall back to MemFree+Buffers+Cached.
    parse_meminfo_field("MemAvailable").unwrap_or(0) * 1024
}
```

### 4.3 Per-level actions

```rust
impl MemoryPressureMonitor {
    async fn dispatch(&self, from: PressureLevel, to: PressureLevel) {
        match to {
            PressureLevel::Normal => {
                // Release any throttling.
                self.governor.clear_throttle();
            }
            PressureLevel::Warning => {
                // Ask all apps to release caches via WIT on-memory-warning.
                self.callbacks.broadcast_warning().await;
                // Trim host-side caches.
                self.governor.trim_caches(TrimAggressiveness::Light);
            }
            PressureLevel::Critical => {
                // 1. Snapshot lowest-priority background apps and drop their stores.
                let victims = self.governor.select_for_suspend(Default::default());
                for v in victims {
                    self.governor.suspend_instance(v).await;
                }
                // 2. Trim caches hard.
                self.governor.trim_caches(TrimAggressiveness::Aggressive);
                // 3. Force-evict un-pinned shared buffers.
                self.governor.evict_orphaned_shm();
            }
            PressureLevel::Imminent => {
                // Jetsam in priority order until we recover or run out of victims.
                let target_avail = self.thresholds.crit_avail_bytes + self.thresholds.hysteresis_bytes;
                self.governor.jetsam_until(target_avail).await;
            }
        }
    }
}
```

### 4.4 `jetsam_score` and victim selection

```rust
// supervisor/src/memory/jetsam.rs
/// Higher score = more eligible for eviction.
/// Score combines: foreground-ness, priority, recency, memory cost, pin flags.
pub fn jetsam_score(
    state:    &AppState,                 // Round 1 hot/cold split
    cost:     &WasmMemoryStat,           // §1.5
    now:      Instant,
    focus_id: Option<InstanceId>,
) -> i64 {
    let mut score: i64 = 0;

    // (a) Foreground is sacred: huge negative weight.
    if Some(state.instance) == focus_id {
        score -= 100_000_000;
    }

    // (b) Round 1 lifecycle: Suspended apps have already been swapped out.
    match state.cold.lifecycle {
        LifecycleState::Foreground            => score -= 50_000_000,
        LifecycleState::Background            => score -= 5_000_000,
        LifecycleState::Suspended { .. }      => score -= 0,        // already cheap to kill
        LifecycleState::Stopping              => score += 10_000_000, // prefer to finish it
        LifecycleState::Crashed { .. }        => score += 10_000_000,
        LifecycleState::NotStarted            => score += 5_000_000,
    }

    // (c) System apps are protected; user apps less so; developer mounts least.
    match state.cold.identity.origin {
        InstallOrigin::System          => score -= 30_000_000,
        InstallOrigin::User { .. }     => score -= 0,
        InstallOrigin::Developer       => score += 5_000_000,
    }

    // (d) Manifest focus_priority (Round 1 §1.1): higher = more important.
    score -= state.cold.identity.focus_priority as i64 * 100_000;

    // (e) Recency: idle background apps are more evictable.
    let idle_secs = now.duration_since(state.hot.last_focus).as_secs() as i64;
    score += idle_secs.min(60 * 60) * 1_000;       // capped at 1 hour worth

    // (f) Memory cost: bigger apps preferred under critical.
    score += (cost.byte_capacity / (1024 * 1024)) as i64 * 1_000;

    // (g) Pin flag (manifest [memory] pin_in_pressure = true).
    if state.cold.identity.pin_in_pressure {
        score -= 100_000_000;
    }

    score
}
```

Victim selection iterates the `AppTable` (sharded, Round 1 §1), reads each `AppState` under its `RwLock`, computes `jetsam_score`, sorts descending, and returns the top-N until we project recovery to the target watermark.

### 4.5 Eviction transitions are Round 1 transitions

Critically, **none of these actions invent new lifecycle states**. They reuse Round 1:

- `Warning` → `on-memory-warning` callback (new WIT, but existing dispatch infrastructure).
- `Critical` → standard `Foreground/Background → Suspended` transition (Round 1 §3 already drops the store and persists `StateBlob`).
- `Imminent` → standard `* → Stopping → Crashed { kind: CrashKind::Jetsamed, .. }` (we add a new `CrashKind` variant; see §6.3).

### 4.6 Hysteresis and flapping

`PressureLevel::Critical` requires a 50 MiB recovery margin to drop back to `Warning` (default `hysteresis_bytes = 50 * 1024 * 1024`). Without this, freeing memory by killing one app would immediately resurrect the warning state on the next sample tick, churning callbacks.

---

## 5. Large Object Allocation (macOS: large pages, huge pages)

### 5.1 Font atlas (2048×2048 RGBA = 16 MiB)

Allocated **once** at supervisor startup via `Box<[u8; 16 * 1024 * 1024]>` and then placed behind `Arc<FontAtlas>`. Lives in `supervisor/src/text/atlas.rs`. Tracked in `SupervisorMemoryLayout.font_cache` (note: glyph cache and atlas are separate; the atlas is the GPU-uploadable bitmap, the glyph cache is the LRU rasterized-glyph dictionary).

We do **not** use Linux huge pages explicitly (`MADV_HUGEPAGE`); the allocation is contiguous within Wasmtime's reserved heap and the kernel may opportunistically use transparent huge pages.

### 5.2 Framebuffer double-buffer

Already covered in §2.2: one `mmap` from DRM, plus a single `back_buffer: Vec<u8>` of the same size for the compositor to draw into before flipping. On supported drivers we use page-flipping (`drmModePageFlip`); on virtio-gpu fbcon we `memcpy` from back to front on every flush.

### 5.3 Large file streaming for media apps

Apps cannot `mmap`. For media apps that previously would have `mmap`'d a 4 GiB MP4, we instead expose a streaming API:

```wit
// supervisor/wit/vyoma-fs.wit (excerpt)
interface streaming {
    type stream-handle = u64;

    record open-options {
        path:        string,
        chunk-size:  u32,    // host returns at most this many bytes per pull
        read-ahead:  u32,    // host pre-reads this many chunks
    }

    open:  func(opts: open-options) -> result<stream-handle, fs-error>;
    pull:  func(h: stream-handle) -> result<option<list<u8>>, fs-error>;
    seek:  func(h: stream-handle, offset: u64) -> result<_, fs-error>;
    close: func(h: stream-handle);
}
```

The supervisor opens the file once, uses an internal `BufReader`, and ships chunks (default 256 KiB) to the app on demand. The supervisor's read-ahead lives in supervisor memory (subject to `MemoryGovernor`), so a media app never inflates its WASM linear memory by more than `chunk_size`.

### 5.4 Large allocations within an app

If an app needs a single buffer > 4 MiB inside its linear memory, the allocator (`dlmalloc` etc.) issues a `memory.grow` to satisfy it. The `MemoryGovernor` admits or denies based on policy. We do not special-case large allocations beyond pressure-aware admission in `AppLimiter::memory_growing` (§1.3, step 4).

---

## 6. Memory Safety Guarantees

### 6.1 Inter-app isolation

**Can app A read app B's linear memory?** No.

Each app has its own `wasmtime::Store`, and each `Store` contains its own `wasmtime::Memory`. WASM bounds-checks are encoded at compile time by Wasmtime's compiler (Cranelift) into either:

- **Explicit bounds checks** (`cmp` + `jne` on every memory access) for the default backend, or
- **Virtual memory traps** (`signals` backend): Wasmtime reserves 8 GiB of virtual address space per memory, the wasm `i32` index is zero-extended to `i64`, and any out-of-range access lands in unmapped pages that SIGSEGV. Wasmtime installs a signal handler that catches the SEGV and converts it to a WASM trap.

There is no instruction in `wasm32` that can address memory not belonging to the executing `Memory`. There is no `import` we expose that takes a raw host pointer. Therefore app A cannot read or write app B's memory **even if it tries**. The only path bytes cross from A to B is via supervisor-mediated mechanisms: IPC messages (Round 1) and shared buffers (§3), both of which copy at the WIT boundary.

### 6.2 Supervisor → WASM-store writes

**Can a buggy supervisor code path write into a WASM store's linear memory?** Yes — by design, since host functions can write to wasm memory. But we guard against accidental bugs.

```rust
// supervisor/src/runtime/memory_guard.rs
/// A safe view into a WASM instance's linear memory, scoped to one host call.
/// All writes go through this guard; raw &mut [u8] is never exposed.
pub struct WasmMemoryView<'s> {
    store:    &'s mut wasmtime::Store<HostState>,
    memory:   wasmtime::Memory,
    /// Bounds captured at guard creation; checked before every write.
    snapshot_size: usize,
}

impl<'s> WasmMemoryView<'s> {
    pub fn new(store: &'s mut wasmtime::Store<HostState>, memory: wasmtime::Memory) -> Self {
        let snapshot_size = memory.data_size(&store);
        Self { store, memory, snapshot_size }
    }

    /// Write `bytes` at `offset`. Returns Err if the write would exceed
    /// the snapshotted size — the host MUST NOT grow the memory during a host call.
    pub fn write(&mut self, offset: u32, bytes: &[u8]) -> Result<(), MemoryGuardError> {
        let offset = offset as usize;
        let end = offset.checked_add(bytes.len()).ok_or(MemoryGuardError::Overflow)?;
        if end > self.snapshot_size {
            return Err(MemoryGuardError::OutOfBounds { offset, len: bytes.len(), cap: self.snapshot_size });
        }
        self.memory.write(&mut self.store, offset, bytes)
            .map_err(MemoryGuardError::WasmtimeWrite)
    }

    pub fn read(&self, offset: u32, out: &mut [u8]) -> Result<(), MemoryGuardError> {
        let offset = offset as usize;
        let end = offset.checked_add(out.len()).ok_or(MemoryGuardError::Overflow)?;
        if end > self.snapshot_size {
            return Err(MemoryGuardError::OutOfBounds { offset, len: out.len(), cap: self.snapshot_size });
        }
        self.memory.read(&self.store, offset, out)
            .map_err(MemoryGuardError::WasmtimeRead)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum MemoryGuardError {
    #[error("offset+len arithmetic overflow")]      Overflow,
    #[error("write {offset}..{} exceeds capacity {cap}", offset + len)]
    OutOfBounds { offset: usize, len: usize, cap: usize },
    #[error("wasmtime write error: {0}")]            WasmtimeWrite(anyhow::Error),
    #[error("wasmtime read error: {0}")]             WasmtimeRead(anyhow::Error),
}
```

Every host function that touches wasm memory takes a `WasmMemoryView`, not raw pointers. Code review enforces this. A unit test asserts that we never call `Memory::data_mut` outside `WasmMemoryView`.

### 6.3 Stack overflow in WASM

Wasmtime detects WASM stack overflow by either:

1. **Explicit stack-check** at the start of each function (default; ~3 % overhead), or
2. **Guard pages** at the bottom of the host-allocated wasm stack (faster, signals backend).

Either way, Wasmtime traps with `TrapCode::StackOverflow`. We extend the Round 1 `CrashKind` enum:

```rust
// supervisor/src/lifecycle/crash.rs (additions)
pub enum CrashKind {
    // ... Round 1 variants ...
    StackOverflow { fn_index: Option<u32> },
    Jetsamed     { pressure: PressureLevel, score: i64 },   // new for §4
    ShmExhausted { requested: usize, budget_remaining: u64 }, // new for §3
}

// In the trap classifier:
fn classify_trap(trap: wasmtime::Trap, ctx: &TrapContext) -> CrashKind {
    match trap {
        wasmtime::Trap::StackOverflow => CrashKind::StackOverflow {
            fn_index: ctx.faulting_fn_index,
        },
        // ... rest of Round 1 mapping ...
    }
}
```

`RestartDecision` for `StackOverflow`: same as `Trap` (restart with backoff, since it is likely a load-dependent bug).

### 6.4 Use-after-free of shared buffer handles

When an app's `Instance` terminates, the supervisor walks `SharedBufferRegistry.per_owner` and detaches it from every buffer it owns or reads. Any handle the app might have stashed in its WASM linear memory is just a `u64`; the next call referencing it returns `ShmError::NotFound`. No dangling pointers, because there are no pointers — only opaque integer handles validated by the registry.

### 6.5 Re-entrant grow during host call

Wasmtime documents that calling `Memory::grow` from inside a host call while holding a `WasmMemoryView` invalidates the view's pointer. Our `WasmMemoryView` captures `snapshot_size` and **never** calls `grow` between read/write operations. A debug assertion in `WasmMemoryView::write` checks that `memory.data_size(&store) == self.snapshot_size`; if it diverges the supervisor panics with a stack trace. This is a developer-only check disabled in release.

---

## 7. Debug / Profiling API (macOS: Instruments, vmmap)

### 7.1 Stdout protocol additions

```
mgmt: memstat <bundle>       → MEMSTAT:<bundle>:<current_pages>:<peak_pages>:<grow_denied>:<supervisor_bytes>
mgmt: memmap                  → MEMMAP:<json blob of SupervisorMemoryLayout>
mgmt: pressure <level>        → PRESSURE_OK:<level>     (test-only; manifest must allow)
mgmt: shmstat                 → SHMSTAT:<count>:<total_bytes>:<budget>
mgmt: jetsam-rank             → JETSAM_RANK:<json list of (bundle_id, score, bytes)>
```

### 7.2 WIT management interface

```wit
// supervisor/wit/vyoma-mgmt.wit (memory section)
interface memory-mgmt {
    record memory-snapshot {
        bundle:               string,
        current-pages:        u32,
        peak-pages:           u32,
        byte-capacity:        u64,
        grow-denied:          u64,
        reported-live-bytes:  option<u64>,   // from report-heap
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
        image-cache:      region-stat,
        shared-buffers:   region-stat,
        ipc-buffers:      region-stat,
        thread-stacks:    region-stat,
        wasm-stores-total: region-stat,
        global-heap:      region-stat,
        estimated-rss:    u64,
    }

    memstat:    func(bundle: string) -> option<memory-snapshot>;
    memmap:     func() -> memory-map;
    set-pressure: func(level: u8) -> bool;    // requires mgmt capability
    jetsam-rank: func() -> list<tuple<string, s64, u64>>;
}
```

### 7.3 Heartbeat extension

The Round 1 observability heartbeat (`supervisor/src/observability/`) gains a memory section:

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
    "warning_callbacks_sent": 4
  }
}
```

Emitted every 1 s in the existing heartbeat cadence.

### 7.4 Crash report enrichment

When `CrashKind::MemLimit`, `CrashKind::Jetsamed`, or `CrashKind::StackOverflow` fires, the crash report (Round 1 §5) gains a `memory_snapshot` field with `WasmMemoryStat` at the moment of crash. This lets `/data/crashes/<id>.toml` show: "app died with 127 pages allocated, peak 128, requested 1 more". Diagnosis becomes trivial.

---

## 8. The `MemoryGovernor`

Glue type that ties §1, §3, §4 together. Lives in `supervisor/src/memory/governor.rs`.

```rust
pub struct MemoryGovernor {
    /// Global byte ceiling across all WASM stores. Default = 75 % of platform RAM floor.
    pub wasm_budget_bytes:   AtomicU64,
    pub wasm_used_bytes:     AtomicU64,

    /// Shared buffer registry budget.
    pub shm_budget_bytes:    AtomicU64,
    pub shm_used_bytes:      AtomicU64,

    /// Foreground instance set (for grow admission decisions).
    foreground: arc_swap::ArcSwap<im::HashSet<InstanceId>>,

    /// App table reference for victim selection.
    app_table:  Arc<AppTable>,

    /// Pressure monitor reference.
    pressure:   Arc<MemoryPressureMonitor>,

    /// Caches we can trim.
    glyph_cache: Arc<GlyphCache>,
    image_cache: Arc<ImageDecodeCache>,
    shm_registry: Arc<SharedBufferRegistry>,

    /// Jetsam counters for telemetry.
    jetsam_events: AtomicU64,
    suspend_events: AtomicU64,
}

impl MemoryGovernor {
    pub fn try_reserve(&self, bytes: usize) -> bool {
        let bytes = bytes as u64;
        let prev = self.wasm_used_bytes.fetch_add(bytes, Ordering::AcqRel);
        if prev + bytes > self.wasm_budget_bytes.load(Ordering::Relaxed) {
            self.wasm_used_bytes.fetch_sub(bytes, Ordering::AcqRel);
            false
        } else { true }
    }

    pub fn release(&self, bytes: usize) {
        self.wasm_used_bytes.fetch_sub(bytes as u64, Ordering::AcqRel);
    }

    pub fn pressure_level(&self) -> PressureLevel { **self.pressure.level.load() }
    pub fn foreground_set(&self) -> arc_swap::Guard<Arc<im::HashSet<InstanceId>>> { self.foreground.load() }

    pub fn select_for_suspend(&self, opts: SelectOptions) -> Vec<InstanceId> { /* iterate AppTable, sort by jetsam_score */ }
    pub async fn suspend_instance(&self, id: InstanceId) -> Result<(), SuspendError> { /* call WIT on-suspend, save StateBlob, drop store */ }
    pub async fn jetsam_until(&self, target_avail: u64) { /* loop: pick highest score, terminate, sleep, re-sample */ }

    pub fn trim_caches(&self, level: TrimAggressiveness) {
        match level {
            TrimAggressiveness::Light => {
                self.glyph_cache.trim_to(self.glyph_cache.bytes_cap / 2);
                self.image_cache.trim_to(self.image_cache.bytes_cap / 2);
            }
            TrimAggressiveness::Aggressive => {
                self.glyph_cache.clear();
                self.image_cache.trim_to(0);
            }
        }
    }

    pub fn evict_orphaned_shm(&self) { self.shm_registry.evict_orphans(); }
    pub fn clear_throttle(&self) { /* no-op for now; placeholder for future throttle policy */ }
}
```

The governor is shared (`Arc<MemoryGovernor>`) across:
- Every `AppLimiter` (so `memory_growing` can ask).
- The `MemoryPressureMonitor` (so it can act).
- The `SharedBufferRegistry` (so allocation checks budget).
- The `mgmt` handlers (so debug API reads it).

---

## 9. Boot Sequencing

Per Round 1 `BootPhase`, memory subsystem initialization fits:

| BootPhase | Memory action |
|-----------|---------------|
| `KernelHandoff` | nothing |
| `MountData` | open `/proc/meminfo` for monitor |
| `LoadProfile` | read `[memory]` block from `profile.toml` → `PressureThresholds`, `MemoryGovernor` budgets |
| `InitDisplay` | mmap framebuffer; allocate font atlas; record in `SupervisorMemoryLayout` |
| `InitRegistries` | construct `AppTable`, `SharedBufferRegistry`, `MemoryGovernor`, `MemoryPressureMonitor` |
| `SpawnPressureMonitor` | spawn the monitor's tokio task |
| `LaunchInstalled` | for each app: build `AppLimiter` with `Arc<MemoryGovernor>` |

Lock-order discipline (Round 1 §11 extended):

```
1. Engine (no locks)
2. MemoryGovernor atomics (lock-free)
3. AppTable shard locks
4. AppState.cold
5. AppState.hot
6. SharedBufferRegistry.by_id (DashMap; never held across WASM call)
7. SharedBuffer.data (RwLock; always last)
```

A subsystem-level test (`supervisor/tests/lock_order_test.rs`) attempts every pairing in reverse order under `parking_lot` deadlock detection.

---

## 10. Implementation Files

All paths under `supervisor/src/`. Total: 11 new files + 4 modifications. Every file ≤ 500 LOC.

| File | LOC budget | Purpose |
|------|------------|---------|
| `memory/mod.rs` | 60 | module wiring; re-exports `PressureLevel`, `MemoryGovernor`, `WasmMemoryStat` |
| `memory/governor.rs` | 380 | `MemoryGovernor` + `try_reserve` + `trim_caches` + selection logic |
| `memory/pressure.rs` | 120 | `PressureLevel`, `PressureThresholds`, `PressureEvent` types |
| `memory/pressure_monitor.rs` | 320 | async monitor task + meminfo parser + hysteresis + dispatch |
| `memory/jetsam.rs` | 220 | `jetsam_score` + victim selection + termination loop |
| `memory/layout.rs` | 280 | `SupervisorMemoryLayout`, `RegionStat`, `/proc/self/smaps` reader |
| `shm/mod.rs` | 40 | module wiring |
| `shm/shared_buffer.rs` | 240 | `SharedBuffer`, `SharedBufferKind`, `SharedBufferFlags`, lifecycle |
| `shm/registry.rs` | 420 | `SharedBufferRegistry` + create/attach/write/read/destroy + budget |
| `shm/wit_host.rs` | 280 | WIT `vyoma:memory/shared` host implementation (delegates to registry) |
| `runtime/memory_guard.rs` | 180 | `WasmMemoryView`, `MemoryGuardError`, bounds-checked host accessors |

Modifications:

| File | Δ LOC | Why |
|------|-------|-----|
| `resource_limiter.rs` | +120 | extend `AppLimiter::memory_growing` with governor + pressure check (§1.3) |
| `lifecycle/crash.rs` | +60 | add `StackOverflow`, `Jetsamed`, `ShmExhausted` to `CrashKind`; restart policy update |
| `manifest.rs` | +90 | parse + validate `[memory]` block; `pin_in_pressure`, `allow_shared_buffers` |
| `observability/heartbeat.rs` | +50 | emit memory section per §7.3 |

`mgmt` command additions live in the existing `mgmt_handlers.rs` (already < 500 LOC after Round 1; +80 LOC for memstat/memmap/jetsam-rank/set-pressure).

WIT definitions:

| File | Purpose |
|------|---------|
| `wit/vyoma-memory.wit` | `vyoma:memory@0.1.0` — `shared` interface, `report` interface |
| `wit/vyoma-mgmt.wit` (extend) | `memory-mgmt` interface for §7.2 |
| `wit/vyoma-callbacks.wit` (extend) | `on-memory-warning` callback |

### 10.1 Test plan

| Test file | Scope |
|-----------|-------|
| `supervisor/tests/memory_limiter_test.rs` | `AppLimiter::memory_growing` correctness across all admission paths |
| `supervisor/tests/governor_budget_test.rs` | concurrent `try_reserve` from 16 threads; budget never over-committed |
| `supervisor/tests/pressure_hysteresis_test.rs` | simulate available-memory series, assert no flapping |
| `supervisor/tests/jetsam_score_test.rs` | golden tests for `jetsam_score` across 12 scenarios |
| `supervisor/tests/shm_registry_test.rs` | create/attach/write/read/destroy + lifetime + orphan eviction |
| `supervisor/tests/memory_guard_test.rs` | `WasmMemoryView` rejects out-of-bounds + overflow + post-grow accesses |
| `supervisor/tests/lock_order_test.rs` | assertion that no thread acquires locks in violation of §9 ordering |
| `supervisor/tests/wit_memory_host_test.rs` | end-to-end WIT shared-buffer roundtrip between two synthetic instances |
| `supervisor/tests/heartbeat_memory_test.rs` | memory section appears in heartbeat JSON, fields correct |

### 10.2 Out of scope (deferred to Round 3 / next subsystem)

- **GPU memory accounting:** virtio-gpu has its own VRAM concept; this spec treats the GPU as a black box that the framebuffer mmap covers. A future subsystem ("Display & Compositor") will model `gpu_vram_used`.
- **NUMA awareness:** desktop platform is single-socket; revisit only if VyomaOS targets multi-socket servers.
- **Persistent memory (pmem):** out of scope.
- **Memory ballooning to the host hypervisor:** out of scope (we are not a guest of another VyomaOS).

---

## 11. Open questions for the Critic

These are points the Architect is least confident about; the critique should attack them first.

1. **Foreground set lookup cost in `memory_growing`.** Today the check is `governor.foreground_set().contains(&id)`. With `arc_swap` + `im::HashSet`, this is lock-free read but allocates an `Arc` clone on every grow. At 100 grows/s/app × 50 apps = 5000 clones/s. Is this acceptable, or should we cache foreground-ness on `AppLimiter` itself and update via a channel from the focus manager?

2. **`MemoryGovernor::try_reserve` is naive CAS.** Under contention from concurrent grows, the `fetch_add`/`fetch_sub` overshoot pattern leaks the reservation if a thread is preempted between add and check. The leak is bounded but exists. Is this acceptable, or do we need a real reservation token?

3. **`MemoryAvailable` polling at 250 ms.** A burst allocator inside a wasm app can move 64 MiB in 50 ms; we might miss the warning window. Should we hook into Linux's `cgroup memory.pressure` PSI events for sub-tick reactivity?

4. **`StateBlob` size during eviction.** Round 1 capped state blobs at 1 MiB. Under critical pressure we may need to evict apps with multi-megabyte state (e.g. a notes app with a working document). Do we raise the cap, compress, or accept data loss?

5. **Shared buffer copy cost.** Every `read`/`write` through WIT does two copies (wasm → host, host → wasm). For a 16 MiB surface at 60 Hz this is ~ 2 GiB/s of memcpy. Is this acceptable on `desktop-full`, or do we need a Wasmtime-host shared-memory backing (which requires the wasm-threads proposal)?

6. **Suspending an app drops its `Store` — but `Store` ownership lives in the app's driver thread (Round 1 §4).** Coordinating "drop the store" from the governor thread requires the driver thread to cooperate. Is `Arc<Notify>` from tokio sufficient, or do we need a more rigid command channel?

7. **`jetsam_score` weights are magic numbers.** Are the scale ratios (foreground = −5×10⁷, system = −3×10⁷, system manifest priority = ×10⁵, MiB cost = ×10³) defensible, or should they be configurable via `profile.toml`?

8. **Image decode cache eviction during compositor frame.** If the compositor is mid-flush and `trim_caches(Aggressive)` runs, it may evict an `Arc<DecodedImage>` the compositor still holds. The `Arc` keeps the bytes alive, but the LRU map releases its strong reference. Is this race correctness-preserving? (I think yes, but want the Critic to confirm.)

---

## 12. Summary

This design takes Round 1's per-instance `Store` + `AppLimiter` + sharded `AppTable` and layers on:

- A **global `MemoryGovernor`** that is the single source of truth for "can this grow?" — backed by lock-free atomics for the hot path.
- A **`MemoryPressureMonitor`** that maps `/proc/meminfo` to four pressure levels with hysteresis and dispatches actions reusing Round 1 lifecycle transitions (warning callback, suspend, jetsam).
- A **`SharedBufferRegistry`** that gives apps an explicit, capability-checked mechanism for sharing bytes — no wasm-threads required, two-copy WIT path with a stdout fallback.
- A **`WasmMemoryView` guard** that makes accidental supervisor → wasm-memory bugs hard to write.
- **Debug + observability surface** (`mgmt: memstat`, `memmap`, heartbeat memory section) that makes "where did all my RAM go?" answerable in production.

Total: 11 new files (~2,540 LOC), 4 modifications (~320 added LOC). Every file under the 500-line rule. Subsystem fits in a single Round 1 `BootPhase` insertion and adds three new `CrashKind` variants without disturbing existing crash classification.

The estimated supervisor RSS at the 50-apps / 1440×900 target is **~ 834 MiB**, dominated by per-app linear memories and per-window surfaces. Both have clear knobs (`max_pages` per app, surface eviction on hide) and clear emergency valves (jetsam by score).

Critic — please attack §11 first.

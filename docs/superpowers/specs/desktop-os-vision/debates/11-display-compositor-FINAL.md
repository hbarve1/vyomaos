# Round 11 FINAL — Display Server & Compositor

**Role:** Synthesis
**Date:** 2026-05-29
**Status:** FINAL — canonical spec for VyomaOS Display Server & Compositor
**Resolves:** Architect proposal (in-supervisor compositor, sharded WindowRegistry, protocol v2, FontProvider, thermal throttling) + 8 blocking Critic issues
**macOS equivalent:** WindowServer / SkyLight compositor + CoreAnimation render server

---

## 0. Executive Summary

VyomaOS's Display Server and Compositor are co-located in the PID-2 supervisor as a dedicated **compositor thread** (SCHED_FIFO priority 40), not a separate user-space process. The WASM sandbox boundary already provides the same isolation that a separate WindowServer process gives macOS; running the compositor in-supervisor eliminates two context switches and one IPC hop per draw command — critical for a 60 fps budget on a 256 MB mobile profile.

The compositor accepts a **versioned, framed binary protocol (VYOMA_DRAW v2)** over a *dedicated display pipe fd* (WASI fd 3), separate from stdout. This kills three classes of bugs simultaneously: command injection from log lines, ambiguous framing under partial writes, and missing back-pressure when the compositor falls behind. The legacy v1 textual stdout protocol is retained for compatibility with pre-Round-11 apps.

The window model uses a **sharded `WindowRegistry`**: a brief Mutex over metadata (lifecycle ops only — create, close, raise, lower), a per-window `RwLock<Surface>` for the pixel buffer, and a per-window lock-free `AtomicDamageRect` for damage tracking. The compositor pass snapshots metadata in ~1 µs, releases the metadata lock, then takes per-window read locks during blit. Parser threads run draw commands under per-window read locks (uncontended 99.9% of the time) without ever touching the metadata lock. The framebuffer Mutex is acquired last and last only, preserving a strict lock-order invariant **`Surface < FB`** that is debug-asserted in every flush path.

Window **chrome** (titlebar, borders, traffic-light buttons) lives inside the window's own `Surface` — specifically in the top `CHROME_H = 28` px strip. This replaces today's `chrome.rs::draw_chrome_onto` / `repaint_all_borders` functions, which paint directly into the framebuffer outside surface damage tracking (and therefore go missing on focus changes during damage culling). With chrome in the surface, every chrome repaint is naturally included in the per-window damage union, and the compositor only needs to blit the affected window.

The shared-buffer fast path (apps writing directly into an aliased pixel buffer) is implemented via **Wasmtime's `MemoryCreator` API** (`Config::with_host_memory(...)`). A `SurfaceMemoryCreator` allocates the WASM linear memory backed by the same `Vec<u8>` that the compositor reads from. A single-buffer model with a `generation: AtomicU64` is used for v1; double-buffering is deferred. This fast path is gated to **desktop-full and mobile profiles only** — on iot-edge/robotics-rt/mcu-minimal, the conventional `VYOMA_DRAW_V2` pipe path is the only path.

VSync is driven by **`timerfd(CLOCK_MONOTONIC)`** with an absolute-deadline `it_interval`, not by `thread::sleep`. The compositor reads the timerfd in its main loop; the kernel's `missed_count` value tells the compositor how many ticks were missed, allowing graceful catch-up without time drift. DRM hardware vblank (`DRM_IOCTL_WAIT_VBLANK`) is attempted at startup and used when available; on virtio-gpu (QEMU), it silently falls back to the timerfd path. Per-platform Hz: desktop-full/mobile 60, iot-edge/robotics-rt 30, mcu-minimal 15 (no off-screen compositing), server-headless 1.

**Per-platform Surface pixel formats** are explicit: `Bgra32` on desktop-full / server-headless, `Rgb565` on iot-edge / robotics-rt, `Mono1bpp` on mcu-minimal (where there is no off-screen Surface at all — apps direct-draw into the framebuffer through the protocol shim). Per-platform max Surface dimensions cap memory pressure: 4096×4096 desktop, 1920×1080 mobile, 800×600 iot-edge, 640×480 robotics-rt, 320×240 mcu-minimal.

**Window creation is explicit and eager.** For legacy v1 apps, the supervisor synthesizes the default window inside `launch_app_threads()` *after* `apply_tiling_layout()` returns and *before* any IO threads are spawned — there is no first-draw race. For v2 apps, `create_window` is the required first command before any drawing; the parser FSM (`Uninitialized → WindowCreated → InFrame → Idle`) rejects out-of-order commands.

**Thermal throttling** uses Round 7's `ThermalGovernor`. The compositor reads an `AtomicU8 skip_mask` once per vsync tick — at nominal it always composites; at warm tier it composites every other tick (30 fps effective); at hot tier it composites every fourth tick (15 fps effective). No mode change required; the timerfd keeps ticking, the compositor just no-ops some ticks.

**Z-order** is a `Vec<IidKey>` sorted back-to-front, not an integer field. Raise = remove + push-back. Lower = remove + insert(0). O(N) with N ≤ 50 in practice; no integer overflow, no drift.

Implementation is split across 12+ files under `supervisor/src/display/`, each under 500 LOC per the repository's hard limit.

---

## 1. Design Philosophy

### 1.1 In-Supervisor Compositor — Why Not a Separate Process?

The Apple model is a separate `WindowServer` process. The Wayland model is a separate `compositor` process. Both impose at least one extra context switch and one extra IPC hop per draw command. The reason both architectures accept that overhead is that the trust boundary in their world is the **process boundary**: a graphics app is a regular ELF binary that can do anything in its address space, so the only way to enforce "this app cannot read another app's pixels" is to put compositing logic in a different process.

In VyomaOS, every app runs inside a Wasmtime instance with a strict, capability-bound set of imports. An app cannot make arbitrary syscalls, cannot escape its sandbox memory, cannot mmap arbitrary fds. **The sandbox boundary already provides the isolation the separate WindowServer process is supposed to provide.** Co-locating the compositor in PID-2 with the supervisor:

1. Eliminates IPC marshalling of every draw command (zero serialization for in-process calls).
2. Eliminates the cross-process context switch on every frame submission.
3. Enables direct sharing of `Vec<u8>` buffers between app input parsing and compositor logic without a memcpy.
4. Simplifies trust: there is exactly one trusted Rust process (PID 2). Every WASM app is sandboxed equally.

The cost of co-location is one: a compositor bug *could* corrupt supervisor state. We mitigate by isolating compositor mutable state behind RwLocks/Mutexes with a documented lock order, running compositor logic on a dedicated thread (separate from IPC broker), and budgeting the compositor pass at ≤8 ms hard ceiling on desktop-full (50% of frame budget at 60 Hz).

### 1.2 Trust Boundary and Threat Model

| Threat | Mitigation |
|---|---|
| App writes garbage to display fd | Framed binary protocol with `[u16 len][u8 verb][payload]`; verbs validated; invalid → drop frame, log, continue |
| App tries to overwrite another app's Surface | Surfaces are keyed by `IidKey`; protocol commands operate only on the app's own selected window; no cross-app addressing |
| App floods the display fd | Pipe buffer (64 KB) provides flow control via `EAGAIN`/blocking; supervisor never drops frames silently |
| App triggers infinite loop in compositor (e.g., huge rect) | All rects clipped to surface bounds at protocol parse time; surface dimensions enforced by platform cap; compositor pass bounded by surface count × surface dimensions |
| App writes `@supervisor: ...` IPC commands disguised as draw commands | Display fd is separate from stdout; supervisor's IPC parser does not read fd 3 |
| App reads another app's pixels | Apps have no mechanism to read framebuffer or peer surfaces; only their own selected window is addressable |
| Compositor bug crashes supervisor | Compositor thread runs in same address space as supervisor; this is accepted risk, mitigated by fuzz tests + strict bounds on all protocol parsing |

### 1.3 macOS Equivalence Map

| macOS concept | VyomaOS equivalent |
|---|---|
| WindowServer process | Compositor thread in PID-2 (in-process) |
| `CGSWindow` opaque handle | `IidKey` (supervisor-internal) + `u32 local_id` (app-visible) |
| `CGSContext` / Quartz drawing | VYOMA_DRAW v2 protocol over fd 3 |
| `CALayer` backing store | Per-window `Surface` (BGRA off-screen buffer) |
| `CARenderServer` compositing | `compositor_pass()` function (snapshot → blit Z-order) |
| `CVDisplayLink` vsync | `timerfd(CLOCK_MONOTONIC)` (+ DRM vblank when available) |
| `IOSurface` shared memory | `SurfaceMemoryCreator` via Wasmtime `MemoryCreator` |
| WindowServer kept-alive heartbeat | Compositor thread has its own watchdog; supervisor restart on hang |

### 1.4 Relationship to Linux Display Stacks

VyomaOS is **not** Wayland and **not** X11. Both assume client processes communicate via UNIX sockets with explicit framing protocols. VyomaOS apps are WASM modules in the same address space as the compositor; their "messages" are pipe writes on fd 3 that are parsed without any deserialization library. There is no Wayland compositor library, no XCB, no DBus.

Where VyomaOS *does* use Linux primitives: `/dev/fb0` for the framebuffer, optional DRM/KMS ioctls for hardware vblank, `timerfd` for vsync ticks, `mmap` for the framebuffer aperture. All Linux interaction is encapsulated in `supervisor/src/display/framebuffer.rs` and `supervisor/src/display/vsync.rs`.

---

## 2. WindowRegistry — Sharded Locking Model

### 2.1 Motivation

The simplest design — a single `Mutex<WindowRegistry>` holding all window state — fails under two workloads simultaneously:

- **Compositor pass**: 3–8 ms holding the mutex while reading each surface and blitting → no parser can submit a draw command for 3–8 ms each frame.
- **Parser threads**: 10 apps × ~150 commands/frame = 1500 lock acquisitions per frame. With a 60 Hz target, that's 90 000 lock acquisitions per second on the same mutex, with each acquisition behind a multi-millisecond compositor hold.

Result: parser threads stall, draw command pipelines back up, IPC also stalls because parsers share threads with IPC routing. Convoy. Death.

### 2.2 Shard Design

The registry is split into **three independent shards**:

| Shard | Type | Purpose | Hold duration |
|---|---|---|---|
| Metadata | `Mutex<HashMap<IidKey, WindowMeta>>` | Window lifecycle (create/close), Z-order list, focus | Microseconds only (no I/O held) |
| Surfaces | `HashMap<IidKey, Arc<RwLock<Surface>>>` | Per-window pixel buffer (BGRA / RGB565) | Read lock per blit (~1–3 ms); write lock per resize (~10 µs) |
| Damage | `HashMap<IidKey, Arc<AtomicDamageRect>>` | Per-window dirty region | Lock-free CAS — no holds |

The Surfaces and Damage maps are themselves behind an `RwLock<HashMap<...>>` for the rare add/remove operation; lookup-then-clone-the-Arc takes the outer read lock for nanoseconds, then operates on the per-window inner lock.

```rust
// supervisor/src/display/window_registry.rs

use std::collections::HashMap;
use std::sync::{Arc, Mutex, RwLock};

pub struct WindowRegistry {
    /// Lifecycle metadata. Acquired briefly for create/close/raise/lower/focus.
    /// NEVER held across an I/O or blit operation.
    metadata: Mutex<RegistryMetadata>,

    /// Per-window pixel buffers. Outer RwLock is held for the duration of an
    /// Arc-clone (nanoseconds). The cloned Arc<RwLock<Surface>> is then used
    /// independently; the read lock is the hot path (draw, blit), the write
    /// lock is the cold path (resize).
    surfaces: RwLock<HashMap<IidKey, Arc<RwLock<Surface>>>>,

    /// Per-window damage rectangles. Lock-free via atomics; no waiting.
    damage: RwLock<HashMap<IidKey, Arc<AtomicDamageRect>>>,

    /// Per-window chrome-dirty flag. Set by WM on focus change / title update.
    /// Compositor checks; if true, repaints chrome strip, then clears.
    chrome_dirty: RwLock<HashMap<IidKey, Arc<std::sync::atomic::AtomicBool>>>,
}

pub struct RegistryMetadata {
    /// All known windows; ground truth for existence.
    pub windows: HashMap<IidKey, WindowMeta>,
    /// Z-order from back (index 0) to front (last). NEVER an integer.
    pub z_order: Vec<IidKey>,
    /// Currently focused (key) window, if any.
    pub key_window: Option<IidKey>,
    /// Window receiving mouse input (may differ from key on click-through).
    pub mouse_focus: Option<IidKey>,
}

pub struct WindowMeta {
    pub iid: IidKey,
    pub app_name: String,
    pub local_id: u32,        // App-visible window id (per-app namespace)
    pub bounds: WindowBounds, // x, y, w, h on the screen (content area; chrome adds CHROME_H)
    pub title: String,
    pub global_alpha: f32,    // [0.0, 1.0]
    pub visible: bool,
    pub minimized: bool,
    pub fullscreen: bool,
    pub resizable: bool,
    pub created_ns: u64,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct WindowBounds {
    pub x: i32,
    pub y: i32,
    pub w: u32, // content width (excludes chrome border)
    pub h: u32, // content height (excludes titlebar + bottom border)
}
```

### 2.3 IidKey and local_id

```rust
/// Globally unique window identifier within the supervisor.
/// Wraps the app's IID + a per-app monotonic counter.
#[derive(Copy, Clone, Hash, PartialEq, Eq, Debug)]
pub struct IidKey {
    pub app_iid: u32,    // App instance id (R1)
    pub window_seq: u16, // Monotonic per app; 0 = default v1 window
}

/// App-visible window handle returned by create_window.
/// 32-bit so it survives WIT i32 marshalling cleanly.
/// Mapped per-app to an IidKey via app parser state.
pub type LocalWindowId = u32;
```

Each app's parser holds a `HashMap<LocalWindowId, IidKey>` so the app addresses windows by an opaque 32-bit integer while the supervisor internally addresses them by an `IidKey` that survives app restarts (it doesn't, but the type is correct for future cross-restart persistence).

### 2.4 AtomicDamageRect — Lock-Free Damage Tracking

```rust
use std::sync::atomic::{AtomicI32, Ordering};

/// Per-window damage rectangle, accumulated lock-free by parser threads
/// and consumed once per compositor pass.
///
/// Encoded as four atomics: top-left (x,y) and exclusive bottom-right (x2,y2).
/// Empty (clean) state encoded as x = i32::MAX, x2 = i32::MIN.
pub struct AtomicDamageRect {
    x:  AtomicI32,
    y:  AtomicI32,
    x2: AtomicI32, // exclusive
    y2: AtomicI32, // exclusive
}

impl AtomicDamageRect {
    pub const fn clean() -> Self {
        Self {
            x:  AtomicI32::new(i32::MAX),
            y:  AtomicI32::new(i32::MAX),
            x2: AtomicI32::new(i32::MIN),
            y2: AtomicI32::new(i32::MIN),
        }
    }

    /// Union the given rectangle into the damage region.
    /// Called from parser threads (potentially many concurrent).
    /// Lock-free; uses fetch_min / fetch_max.
    pub fn union_in(&self, x: i32, y: i32, x2: i32, y2: i32) {
        // Use Relaxed ordering for individual fields; compositor uses an
        // AcqRel barrier when reading via `take()` to establish happens-before.
        self.x.fetch_min(x, Ordering::Relaxed);
        self.y.fetch_min(y, Ordering::Relaxed);
        self.x2.fetch_max(x2, Ordering::Relaxed);
        self.y2.fetch_max(y2, Ordering::Relaxed);
    }

    /// Snapshot and reset to clean. Returns None if no damage.
    /// Called once per compositor pass per window.
    pub fn take(&self) -> Option<Rect> {
        // Acquire the current values, then CAS each back to sentinel.
        // The order is x2/y2 first, then x/y, so that a racing union_in
        // either is fully included or fully missed (never half).
        let x2 = self.x2.swap(i32::MIN, Ordering::AcqRel);
        let y2 = self.y2.swap(i32::MIN, Ordering::AcqRel);
        let x  = self.x.swap(i32::MAX, Ordering::AcqRel);
        let y  = self.y.swap(i32::MAX, Ordering::AcqRel);

        if x2 <= x || y2 <= y {
            None // clean
        } else {
            Some(Rect { x, y, w: (x2 - x) as u32, h: (y2 - y) as u32 })
        }
    }

    /// Mark the entire surface dirty.
    pub fn full(&self, w: i32, h: i32) {
        self.union_in(0, 0, w, h);
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
}

impl Rect {
    pub fn union(&self, other: Rect) -> Rect {
        let x  = self.x.min(other.x);
        let y  = self.y.min(other.y);
        let x2 = (self.x + self.w as i32).max(other.x + other.w as i32);
        let y2 = (self.y + self.h as i32).max(other.y + other.h as i32);
        Rect { x, y, w: (x2 - x) as u32, h: (y2 - y) as u32 }
    }

    pub fn intersect(&self, other: Rect) -> Option<Rect> {
        let x  = self.x.max(other.x);
        let y  = self.y.max(other.y);
        let x2 = (self.x + self.w as i32).min(other.x + other.w as i32);
        let y2 = (self.y + self.h as i32).min(other.y + other.h as i32);
        if x2 > x && y2 > y { Some(Rect { x, y, w: (x2 - x) as u32, h: (y2 - y) as u32 }) } else { None }
    }

    pub fn is_empty(&self) -> bool {
        self.w == 0 || self.h == 0
    }
}
```

### 2.5 Lock Order Invariant

```text
Per-window Surface RwLock < Framebuffer Mutex (always)
Per-window Surface RwLock < Damage atomics (always; damage is lock-free anyway)
Metadata Mutex MUST NOT be held when acquiring Surface or FB locks.
```

This is enforced by:

- **API design**: no public function on `WindowRegistry` exposes the metadata MutexGuard. All metadata operations return owned data (clones).
- **Debug assertions**: `Framebuffer::flush_dirty` checks a thread-local `IN_COMPOSITE_PASS: Cell<bool>` and panics if entered while any Surface write lock is held.
- **Documentation**: top of `mod.rs` includes a `// LOCK ORDER:` block with the rules.
- **CI test**: a deadlock-detector test that exercises 1000 windows × 16 threads for 5 seconds and aborts on any cycle detection.

### 2.6 WindowRegistry Public API

```rust
impl WindowRegistry {
    pub fn new() -> Self { /* ... */ }

    // ─── Lifecycle (acquires metadata lock) ────────────────────────────────

    pub fn create_window(
        &self,
        app_iid: u32,
        app_name: &str,
        local_id: LocalWindowId,
        bounds: WindowBounds,
        title: String,
        format: SurfaceFormat,
    ) -> Result<IidKey, RegistryError>;

    pub fn close_window(&self, key: IidKey) -> Result<(), RegistryError>;

    pub fn raise(&self, key: IidKey);
    pub fn lower(&self, key: IidKey);

    pub fn set_focus(&self, key: IidKey);
    pub fn focused(&self) -> Option<IidKey>;

    pub fn set_title(&self, key: IidKey, title: String);
    pub fn set_bounds(&self, key: IidKey, bounds: WindowBounds) -> Result<(), RegistryError>;
    pub fn set_alpha(&self, key: IidKey, alpha: f32);
    pub fn set_visible(&self, key: IidKey, visible: bool);

    // ─── Surface access (no metadata lock held) ────────────────────────────

    /// Acquire a clone of the Arc<RwLock<Surface>>. Outer lock held for ~ns.
    pub fn surface(&self, key: IidKey) -> Option<Arc<RwLock<Surface>>>;

    /// Acquire damage atomic for unioning. Outer lock held for ~ns.
    pub fn damage(&self, key: IidKey) -> Option<Arc<AtomicDamageRect>>;

    pub fn chrome_dirty(&self, key: IidKey) -> Option<Arc<std::sync::atomic::AtomicBool>>;

    // ─── Compositor snapshot ───────────────────────────────────────────────

    /// Acquire metadata briefly and snapshot what the compositor needs.
    /// Returns a Vec sorted back-to-front in Z-order.
    pub fn snapshot_for_composite(&self) -> Vec<CompositeEntry>;
}

pub struct CompositeEntry {
    pub key: IidKey,
    pub bounds: WindowBounds,
    pub alpha: f32,
    pub visible: bool,
    pub surface: Arc<RwLock<Surface>>,
    pub damage: Arc<AtomicDamageRect>,
    pub chrome_dirty: Arc<std::sync::atomic::AtomicBool>,
}

#[derive(Debug)]
pub enum RegistryError {
    UnknownWindow,
    UnknownLocalId,
    InvalidBounds { reason: &'static str },
    LocalIdInUse,
    PlatformCapExceeded { limit_w: u32, limit_h: u32 },
}
```

The `snapshot_for_composite` function is the single critical-path entry on the compositor side; it acquires metadata, surfaces, damage, and chrome_dirty maps' outer locks briefly, builds a `Vec<CompositeEntry>` (~50 entries max), and returns. Total hold time: <10 µs on a desktop-full profile, dominated by HashMap iteration and Arc clones.

---

## 3. WindowRecord — Full Rust Definition

The `WindowMeta` shown above is what lives in the metadata shard. The "full" window record is what callers conceptually deal with — assembled on demand from the three shards:

```rust
// supervisor/src/display/types.rs

/// A window's complete state, assembled from the three registry shards.
/// This struct is never stored; it is a temporary view used by the
/// compositor and WM.
pub struct WindowRecord<'a> {
    pub meta: &'a WindowMeta,
    pub surface: Arc<RwLock<Surface>>,
    pub damage: Arc<AtomicDamageRect>,
    pub chrome_dirty: Arc<std::sync::atomic::AtomicBool>,
}

/// Surface pixel format, selected per platform profile.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum SurfaceFormat {
    /// 4 bytes per pixel, B/G/R/A byte order, premultiplied alpha.
    /// Used on desktop-full and server-headless.
    Bgra32,

    /// 2 bytes per pixel, R5G6B5 little-endian. No alpha.
    /// Used on iot-edge and robotics-rt to save 50% of surface memory.
    Rgb565,

    /// 1 bit per pixel, 8 pixels per byte, MSB first.
    /// Used on mcu-minimal. NOTE: on mcu-minimal there is no off-screen
    /// Surface; this format exists only for direct-FB writes.
    Mono1bpp,
}

impl SurfaceFormat {
    pub const fn bytes_per_pixel(self) -> usize {
        match self {
            Self::Bgra32   => 4,
            Self::Rgb565   => 2,
            Self::Mono1bpp => 0, // bit-packed; computed differently
        }
    }

    pub const fn stride_for(self, width: u32) -> u32 {
        match self {
            Self::Bgra32   => width * 4,
            Self::Rgb565   => width * 2,
            Self::Mono1bpp => (width + 7) / 8,
        }
    }

    pub fn supports_alpha(self) -> bool {
        matches!(self, Self::Bgra32)
    }
}

/// Per-window pixel buffer.
pub struct Surface {
    pub buf:    Vec<u8>,
    pub width:  u32,
    pub height: u32,    // includes CHROME_H strip at top
    pub stride: u32,
    pub format: SurfaceFormat,
    /// Generation counter incremented on `swap` (shared-buffer path).
    /// Used by the compositor to detect frame completion.
    pub generation: u64,
}

impl Surface {
    pub fn new(width: u32, height: u32, format: SurfaceFormat) -> Self {
        let stride = format.stride_for(width);
        let size = (stride as usize) * (height as usize);
        Self {
            buf:    vec![0u8; size],
            width,
            height,
            stride,
            format,
            generation: 0,
        }
    }

    /// Resize in place. Caller MUST hold the write lock.
    pub fn resize(&mut self, new_w: u32, new_h: u32) {
        self.width = new_w;
        self.height = new_h;
        self.stride = self.format.stride_for(new_w);
        let size = (self.stride as usize) * (new_h as usize);
        self.buf.resize(size, 0);
    }
}
```

### 3.1 Chrome Strip Convention

```rust
pub const CHROME_H: u32 = 28; // titlebar 24 + 2 px border top + 2 px border bottom

/// When an app draws at app-local (x, y), the supervisor writes pixel
/// at surface coords (x, y + CHROME_H). The chrome strip occupies
/// surface y in [0, CHROME_H).
///
/// Total surface dimensions = (bounds.w, bounds.h + CHROME_H).
///
/// On the screen, the window is positioned at (bounds.x, bounds.y),
/// where bounds.y refers to the TOP of the chrome strip.
```

This is enforced in the protocol dispatcher: every draw command's `y` coordinate is incremented by `CHROME_H` before reaching `Surface` mutation. Damage rects are similarly translated. The app sees a coordinate system where (0,0) is the top-left of the content area; it has no knowledge that there is a chrome strip above it.

---

## 4. VYOMA_DRAW Protocol v2

### 4.1 Transport — Dedicated Display Pipe

For apps that declare `display = true` in `vyoma.toml`, the supervisor creates an additional pipe at spawn time:

```rust
// supervisor/src/app_threads.rs (modified)

let (display_read, display_write) = nix::unistd::pipe2(OFlag::O_CLOEXEC)?;
// display_write fd is duplicated into the child process as fd 3.
// display_read is owned by the supervisor's display dispatcher thread.

// WASI preopens are extended:
//   fd 0: stdin (IPC inbox + supervisor messages)
//   fd 1: stdout (IPC outbox + lifecycle markers, but NOT VYOMA_DRAW)
//   fd 2: stderr (logs)
//   fd 3: display channel (v2 protocol only)
```

The WASI ABI exposes fd 3 as `wasi:io/streams/output-stream`. For Rust apps, a helper crate (`vyoma_display`) provides a typed binding:

```rust
// In a WASM app (Rust):
use vyoma_display::v2::{Display, WindowBuilder};

fn main() {
    let mut display = Display::open().expect("no display channel");
    let mut win = display.create_window(WindowBuilder::new(800, 600).title("Hello"));

    win.begin_frame();
    win.fill_rect(0, 0, 800, 600, 0x1E1E2EFF);
    win.draw_text(10, 10, "Hello, world!", 0xFFFFFFFF, FontSize::Medium);
    win.damage_rect(0, 0, 800, 600);
    win.end_frame();
}
```

The crate writes framed binary on fd 3 under the hood; apps don't have to think about framing.

### 4.2 Wire Format

Every message on the display pipe is a single frame:

```text
[u16 LE total_len][u8 verb][payload...]
```

- `total_len` is the count of bytes *after* itself, including the verb byte. Max 4096 (4 KB). Larger payloads use a CHUNK verb (`v2.set_pixels` for shared buffers is the only such path).
- `verb` is a single byte; see table below.
- `payload` is verb-specific; little-endian integers, UTF-8 strings prefixed with `u16 len`.

If `total_len > 4096`, the supervisor logs the offending app, closes the display fd, and signals the app via stdin with `VYOMA_DISPLAY_EVENT:protocol_error:frame_too_large`. The app may continue running (its `display` capability is revoked) or exit; supervisor does not kill it.

### 4.3 Verb Table

| Verb | Code | Payload | Direction |
|---|---|---|---|
| `negotiate` | 0x01 | `u16 client_version` | App → Sup |
| `create_window` | 0x10 | `u32 local_id, u32 w, u32 h, u16 title_len, [u8; title_len]` | App → Sup |
| `select_window` | 0x11 | `u32 local_id` | App → Sup |
| `set_title` | 0x12 | `u16 title_len, [u8; title_len]` | App → Sup |
| `set_bounds` | 0x13 | `i32 x, i32 y, u32 w, u32 h` | App → Sup |
| `set_alpha` | 0x14 | `f32 alpha` | App → Sup |
| `close_window` | 0x15 | `u32 local_id` | App → Sup |
| `begin_frame` | 0x20 | (empty) | App → Sup |
| `end_frame` | 0x21 | (empty) | App → Sup |
| `damage_rect` | 0x22 | `i32 x, i32 y, u32 w, u32 h` | App → Sup |
| `swap` | 0x23 | (empty; shared-buffer path) | App → Sup |
| `fill_rect` | 0x30 | `i32 x, i32 y, u32 w, u32 h, u32 rgba` | App → Sup |
| `draw_text` | 0x31 | `i32 x, i32 y, u32 rgba, u8 size, u16 len, [u8; len]` | App → Sup |
| `draw_image` | 0x32 | `i32 x, i32 y, u32 w, u32 h, u32 fmt, [u8; w*h*bpp]` | App → Sup |
| `blit_surface` | 0x33 | `u32 src_local_id, i32 src_x, i32 src_y, u32 w, u32 h, i32 dst_x, i32 dst_y` | App → Sup |
| `set_clip` | 0x40 | `i32 x, i32 y, u32 w, u32 h` | App → Sup |
| `clear_clip` | 0x41 | (empty) | App → Sup |
| `set_pixels` | 0x50 | `u32 offset, [u8; total_len-5]` | App → Sup (chunk) |

### 4.4 Reverse Channel — Display Events to App

The supervisor delivers display events to the app's stdin as text lines (compatible with v1 apps that already read stdin for IPC):

```text
VYOMA_DISPLAY_EVENT:window_created:<local_id>
VYOMA_DISPLAY_EVENT:window_closed:<local_id>
VYOMA_DISPLAY_EVENT:resized:<local_id>:<w>,<h>
VYOMA_DISPLAY_EVENT:focused:<local_id>
VYOMA_DISPLAY_EVENT:blurred:<local_id>
VYOMA_DISPLAY_EVENT:protocol_error:<reason>
VYOMA_DISPLAY_EVENT:negotiated:<server_version>
```

Apps that don't care about display events just ignore these lines. The `vyoma_display` helper crate parses them into a typed `enum DisplayEvent`.

### 4.5 Version Negotiation

```text
App:        negotiate(client_version = 2)
Supervisor: VYOMA_DISPLAY_EVENT:negotiated:2
```

If supervisor supports versions 1..=N and app requests V:

- `V <= N`: supervisor replies `negotiated:V`, uses protocol V.
- `V > N`: supervisor replies `negotiated:N`, app is expected to downgrade.

If the app sends any other verb before `negotiate`, the supervisor defaults to protocol v2 (the current latest) and proceeds.

### 4.6 Parser State Machine

```rust
// supervisor/src/display/protocol_v2.rs

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum ParserState {
    /// Just connected; no window yet. Only `negotiate` and `create_window` allowed.
    Uninitialized,
    /// Has at least one window but no frame open.
    Idle,
    /// Inside `begin_frame ... end_frame`. Draw commands allowed.
    InFrame,
}

pub struct ProtocolParser {
    state:        ParserState,
    selected:     Option<IidKey>,
    app_iid:      u32,
    app_name:     String,
    local_map:    HashMap<LocalWindowId, IidKey>,
    pending_buf:  Vec<u8>, // accumulator for partial frames
    server_version: u16,
    clip:         Option<Rect>, // current clip rectangle, surface-local
}

impl ProtocolParser {
    pub fn push_bytes(&mut self, bytes: &[u8], registry: &WindowRegistry, font: &dyn FontProvider) {
        self.pending_buf.extend_from_slice(bytes);
        while let Some(frame) = self.try_extract_frame() {
            if let Err(e) = self.dispatch(&frame, registry, font) {
                self.report_error(e);
            }
        }
    }

    fn try_extract_frame(&mut self) -> Option<Vec<u8>> {
        if self.pending_buf.len() < 2 { return None; }
        let len = u16::from_le_bytes([self.pending_buf[0], self.pending_buf[1]]) as usize;
        if len > 4096 {
            self.protocol_error("frame too large");
            self.pending_buf.clear();
            return None;
        }
        if self.pending_buf.len() < 2 + len { return None; }
        let mut frame = self.pending_buf.split_off(2 + len);
        std::mem::swap(&mut frame, &mut self.pending_buf);
        // `frame` now contains [u16 len][verb][payload]; strip prefix.
        Some(frame[2..].to_vec())
    }
    // ...
}
```

### 4.7 Dispatch Logic

```rust
fn dispatch(&mut self, frame: &[u8], reg: &WindowRegistry, font: &dyn FontProvider) -> Result<(), ProtoErr> {
    if frame.is_empty() { return Err(ProtoErr::EmptyFrame); }
    let verb = frame[0];
    let body = &frame[1..];

    match (self.state, verb) {
        (ParserState::Uninitialized, 0x01) => self.handle_negotiate(body),
        (ParserState::Uninitialized, 0x10) => self.handle_create_window(body, reg),
        (ParserState::Uninitialized, _) => Err(ProtoErr::NoWindow),

        (_, 0x10) => self.handle_create_window(body, reg),
        (_, 0x11) => self.handle_select_window(body),
        (_, 0x12) => self.handle_set_title(body, reg),
        (_, 0x13) => self.handle_set_bounds(body, reg),
        (_, 0x14) => self.handle_set_alpha(body, reg),
        (_, 0x15) => self.handle_close_window(body, reg),

        (ParserState::Idle, 0x20) => self.handle_begin_frame(),
        (ParserState::InFrame, 0x21) => self.handle_end_frame(reg),
        (ParserState::InFrame, 0x22) => self.handle_damage_rect(body, reg),
        (ParserState::InFrame, 0x23) => self.handle_swap(reg),
        (ParserState::InFrame, 0x30) => self.handle_fill_rect(body, reg),
        (ParserState::InFrame, 0x31) => self.handle_draw_text(body, reg, font),
        (ParserState::InFrame, 0x32) => self.handle_draw_image(body, reg),
        (ParserState::InFrame, 0x33) => self.handle_blit_surface(body, reg),
        (ParserState::InFrame, 0x40) => self.handle_set_clip(body),
        (ParserState::InFrame, 0x41) => { self.clip = None; Ok(()) },
        (ParserState::InFrame, 0x50) => self.handle_set_pixels(body, reg),

        // Drawing outside InFrame: ignored with warning.
        (_, 0x30..=0x4F) => Err(ProtoErr::DrawOutsideFrame),
        _ => Err(ProtoErr::UnknownVerb(verb)),
    }
}

#[derive(Debug)]
pub enum ProtoErr {
    EmptyFrame,
    UnknownVerb(u8),
    NoWindow,
    UnknownLocalId,
    DrawOutsideFrame,
    InvalidPayload(&'static str),
    SurfaceCapExceeded,
}
```

### 4.8 v1 Backwards Compatibility

Apps without `display_protocol = "v2"` in their `vyoma.toml` capabilities section continue to use the legacy text protocol on stdout. The supervisor's stdout router (`supervisor/src/router.rs`) sniffs lines for `VYOMA_DRAW:`, parses them with the v1 parser (`supervisor/src/draw_cmd.rs`), and translates each into the corresponding v2 verb internally. v1 apps get a single synthesized default window (local_id = 0) with the bounds assigned by `apply_tiling_layout`.

```toml
# apps/legacy-clock/vyoma.toml
[capabilities]
stdio   = true
display = true
# (no display_protocol field → defaults to "v1")
```

```toml
# apps/modern-editor/vyoma.toml
[capabilities]
stdio            = true
display          = true
display_protocol = "v2"   # opt in
```

---

## 5. Compositor Pipeline

### 5.1 Compositor Thread Lifecycle

```rust
// supervisor/src/display/compositor.rs

pub struct Compositor {
    registry:   Arc<WindowRegistry>,
    fb:         Arc<Framebuffer>,
    vsync:      VsyncSource,
    thermal:    Arc<crate::power::ThermalGovernor>, // R7
    skip_mask:  Arc<std::sync::atomic::AtomicU8>,
    cursor:     Arc<CursorState>,
    chrome_compose: ChromeCompose,
    stop_flag:  Arc<std::sync::atomic::AtomicBool>,
    stats:      Arc<CompositorStats>,
}

pub struct CompositorStats {
    pub frames_total:     std::sync::atomic::AtomicU64,
    pub frames_dropped:   std::sync::atomic::AtomicU64,
    pub frames_skipped_thermal: std::sync::atomic::AtomicU64,
    pub blit_us_total:    std::sync::atomic::AtomicU64,
    pub last_frame_us:    std::sync::atomic::AtomicU64,
}

impl Compositor {
    pub fn spawn(
        registry: Arc<WindowRegistry>,
        fb: Arc<Framebuffer>,
        thermal: Arc<crate::power::ThermalGovernor>,
        cursor: Arc<CursorState>,
        font: Arc<dyn FontProvider>,
        platform_hz: u32,
    ) -> CompositorHandle {
        let vsync = VsyncSource::open(platform_hz);
        let stop_flag = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let skip_mask = Arc::new(std::sync::atomic::AtomicU8::new(1)); // every tick
        let stats = Arc::new(CompositorStats::default());

        let compositor = Compositor {
            registry,
            fb,
            vsync,
            thermal: thermal.clone(),
            skip_mask: skip_mask.clone(),
            cursor,
            chrome_compose: ChromeCompose::new(font),
            stop_flag: stop_flag.clone(),
            stats: stats.clone(),
        };

        let builder = std::thread::Builder::new()
            .name("vyoma-compositor".into())
            .stack_size(512 * 1024);

        let join = builder.spawn(move || {
            crate::sched::set_sched_fifo(40); // R5
            compositor.run();
        }).expect("compositor thread spawn");

        CompositorHandle { join, stop_flag, skip_mask, stats }
    }
    // ...
}

pub struct CompositorHandle {
    pub join:      std::thread::JoinHandle<()>,
    pub stop_flag: Arc<std::sync::atomic::AtomicBool>,
    pub skip_mask: Arc<std::sync::atomic::AtomicU8>,
    pub stats:     Arc<CompositorStats>,
}
```

### 5.2 Main Loop

```rust
impl Compositor {
    fn run(&self) {
        let mut tick: u64 = 0;
        while !self.stop_flag.load(Ordering::Relaxed) {
            let missed = self.vsync.wait_tick();
            tick = tick.wrapping_add(1 + missed);
            self.stats.frames_total.fetch_add(1 + missed, Ordering::Relaxed);

            // Thermal-aware skip: skip_mask is read fresh each tick.
            // skip_mask = 1 → composite every tick (60 fps).
            // skip_mask = 2 → composite every other tick (30 fps).
            // skip_mask = 4 → composite every fourth tick (15 fps).
            let mask = self.skip_mask.load(Ordering::Relaxed) as u64;
            if mask == 0 || tick % mask != 0 {
                self.stats.frames_skipped_thermal.fetch_add(1, Ordering::Relaxed);
                continue;
            }

            // ─── Composite pass ────────────────────────────────────────────
            let t0 = crate::display::animator::now_ns();
            self.run_composite_pass();
            let dt = crate::display::animator::now_ns().saturating_sub(t0);
            self.stats.last_frame_us.store(dt / 1000, Ordering::Relaxed);
            self.stats.blit_us_total.fetch_add(dt / 1000, Ordering::Relaxed);
        }
    }
}
```

### 5.3 The Composite Pass (Lock-Free)

```rust
// supervisor/src/display/compositor_pass.rs

impl Compositor {
    fn run_composite_pass(&self) {
        // PHASE 1: Snapshot metadata (acquires metadata mutex briefly).
        // Total hold time: <10 µs.
        let entries = self.registry.snapshot_for_composite();
        if entries.is_empty() {
            return;
        }

        // PHASE 2: Repaint chrome strips for any window whose chrome_dirty flag is set.
        // Each repaint takes a per-window write lock on the Surface (short — ~10–50 µs).
        for entry in &entries {
            if entry.chrome_dirty.swap(false, Ordering::AcqRel) {
                self.chrome_compose.repaint_chrome(
                    &entry.meta_clone(),       // see snapshot_for_composite
                    &entry.surface,
                );
                // Mark the chrome strip dirty so the blit covers it.
                entry.damage.union_in(0, 0, entry.bounds.w as i32, CHROME_H as i32);
            }
        }

        // PHASE 3: Collect screen-space damage rects.
        let mut screen_damage = Rect { x: 0, y: 0, w: 0, h: 0 };
        let mut had_damage = false;
        let mut per_window_damage: Vec<(usize, Rect)> = Vec::with_capacity(entries.len());

        for (idx, entry) in entries.iter().enumerate() {
            if !entry.visible { continue; }
            if let Some(rect) = entry.damage.take() {
                // Translate from surface-local to screen-local coords.
                let screen_rect = Rect {
                    x: entry.bounds.x + rect.x,
                    y: entry.bounds.y + rect.y,
                    w: rect.w,
                    h: rect.h,
                };
                per_window_damage.push((idx, screen_rect));
                screen_damage = if had_damage { screen_damage.union(screen_rect) } else { screen_rect };
                had_damage = true;
            }
        }

        // PHASE 4: If cursor moved, add cursor damage.
        let cursor_damage = self.cursor.take_damage();
        if let Some(cd) = cursor_damage {
            screen_damage = if had_damage { screen_damage.union(cd) } else { cd };
            had_damage = true;
        }

        if !had_damage {
            return; // nothing to composite
        }

        // PHASE 5: Clip screen_damage to FB bounds.
        let fb_bounds = Rect { x: 0, y: 0, w: self.fb.width, h: self.fb.height };
        let Some(damage) = screen_damage.intersect(fb_bounds) else { return; };

        // PHASE 6: Blit all visible windows that intersect the damage region,
        // in Z-order (back to front). Per-window read lock on Surface; FB
        // back-buffer Mutex acquired LAST (lock order: Surface < FB).
        debug_assert!(!crate::display::IN_COMPOSITE_PASS.with(|c| c.get()),
                      "lock order violation: nested composite pass");
        crate::display::IN_COMPOSITE_PASS.with(|c| c.set(true));

        let fb_guard = self.fb.back.lock().unwrap(); // acquire FB ONCE per pass
        let mut fb_back = fb_guard;

        for entry in &entries {
            if !entry.visible { continue; }
            let win_screen_rect = Rect {
                x: entry.bounds.x,
                y: entry.bounds.y,
                w: entry.bounds.w + 0,
                h: entry.bounds.h + CHROME_H,
            };
            if win_screen_rect.intersect(damage).is_none() { continue; }

            let surface = entry.surface.read().unwrap();
            blit_surface_to_fb(
                &surface,
                entry.bounds.x,
                entry.bounds.y,
                entry.alpha,
                &mut fb_back,
                self.fb.width,
                self.fb.height,
                damage,
            );
            drop(surface); // explicit
        }

        // PHASE 7: Composite cursor on top.
        self.cursor.composite(&mut fb_back, self.fb.width, self.fb.height, damage);

        drop(fb_back);
        crate::display::IN_COMPOSITE_PASS.with(|c| c.set(false));

        // PHASE 8: Flush damage region from back-buffer to /dev/fb0.
        self.fb.flush_dirty(damage);
    }
}
```

### 5.4 Blit Function (Single Surface → FB)

```rust
// supervisor/src/display/blit.rs

pub fn blit_surface_to_fb(
    src: &Surface,
    dst_x: i32,
    dst_y: i32,
    alpha: f32,
    fb_back: &mut [u8],
    fb_w: u32,
    fb_h: u32,
    clip: Rect,
) {
    let src_w = src.width as i32;
    let src_h = src.height as i32;
    let fb_w_i = fb_w as i32;
    let fb_h_i = fb_h as i32;

    // Compute destination rect in screen coords.
    let x0 = dst_x.max(clip.x).max(0);
    let y0 = dst_y.max(clip.y).max(0);
    let x1 = (dst_x + src_w).min(clip.x + clip.w as i32).min(fb_w_i);
    let y1 = (dst_y + src_h).min(clip.y + clip.h as i32).min(fb_h_i);
    if x1 <= x0 || y1 <= y0 { return; }

    let alpha_byte = (alpha.clamp(0.0, 1.0) * 255.0) as u8;

    match src.format {
        SurfaceFormat::Bgra32 => blit_bgra32(src, x0, y0, x1, y1, dst_x, dst_y, alpha_byte, fb_back, fb_w),
        SurfaceFormat::Rgb565 => blit_rgb565_to_bgra32(src, x0, y0, x1, y1, dst_x, dst_y, fb_back, fb_w),
        SurfaceFormat::Mono1bpp => { /* never reached: mcu-minimal has no Surface */ },
    }
}

fn blit_bgra32(
    src: &Surface,
    x0: i32, y0: i32, x1: i32, y1: i32,
    src_origin_x: i32, src_origin_y: i32,
    alpha_byte: u8,
    fb_back: &mut [u8],
    fb_w: u32,
) {
    let src_stride = src.stride as usize;
    let fb_stride  = fb_w as usize * 4;

    for dst_y in y0..y1 {
        let sy = (dst_y - src_origin_y) as usize;
        let src_row = &src.buf[sy * src_stride..(sy + 1) * src_stride];
        let fb_row_off = dst_y as usize * fb_stride;

        for dst_x in x0..x1 {
            let sx = (dst_x - src_origin_x) as usize;
            let s = &src_row[sx * 4..sx * 4 + 4];
            let sa = ((s[3] as u32) * (alpha_byte as u32) / 255) as u8;
            if sa == 0 { continue; }
            let fb_off = fb_row_off + dst_x as usize * 4;
            let d = &mut fb_back[fb_off..fb_off + 4];
            if sa == 255 {
                d.copy_from_slice(s);
            } else {
                // Premultiplied OVER (src is premultiplied; the global_alpha
                // multiplies both color and alpha because we re-premultiply).
                let inv = 255 - sa as u32;
                d[0] = ((s[0] as u32 * alpha_byte as u32 / 255) + d[0] as u32 * inv / 255) as u8;
                d[1] = ((s[1] as u32 * alpha_byte as u32 / 255) + d[1] as u32 * inv / 255) as u8;
                d[2] = ((s[2] as u32 * alpha_byte as u32 / 255) + d[2] as u32 * inv / 255) as u8;
                d[3] = (sa as u32 + d[3] as u32 * inv / 255).min(255) as u8;
            }
        }
    }
}

fn blit_rgb565_to_bgra32(
    src: &Surface,
    x0: i32, y0: i32, x1: i32, y1: i32,
    src_origin_x: i32, src_origin_y: i32,
    fb_back: &mut [u8],
    fb_w: u32,
) {
    // R5G6B5 LE → BGRA8. No alpha; treat as opaque.
    let src_stride = src.stride as usize;
    let fb_stride  = fb_w as usize * 4;
    for dst_y in y0..y1 {
        let sy = (dst_y - src_origin_y) as usize;
        let src_row = &src.buf[sy * src_stride..(sy + 1) * src_stride];
        let fb_row_off = dst_y as usize * fb_stride;
        for dst_x in x0..x1 {
            let sx = (dst_x - src_origin_x) as usize;
            let lo = src_row[sx * 2] as u16;
            let hi = src_row[sx * 2 + 1] as u16;
            let px = (hi << 8) | lo;
            let r = ((px >> 11) & 0x1F) as u8 * 8;
            let g = ((px >> 5)  & 0x3F) as u8 * 4;
            let b = ( px        & 0x1F) as u8 * 8;
            let fb_off = fb_row_off + dst_x as usize * 4;
            fb_back[fb_off]     = b;
            fb_back[fb_off + 1] = g;
            fb_back[fb_off + 2] = r;
            fb_back[fb_off + 3] = 0xFF;
        }
    }
}
```

### 5.5 Framebuffer Module

```rust
// supervisor/src/display/framebuffer.rs

pub struct Framebuffer {
    /// mmap'd /dev/fb0
    front: std::sync::Mutex<MmapMut>,
    /// CPU back-buffer; same layout as front.
    pub back: std::sync::Mutex<Vec<u8>>,
    pub width: u32,
    pub height: u32,
    pub stride: u32,
    pub bpp: u32, // bits per pixel (always 32 on desktop)
}

impl Framebuffer {
    pub fn open(path: &str) -> Result<Arc<Self>, FbError> {
        let file = std::fs::OpenOptions::new().read(true).write(true).open(path)?;
        let var = unsafe { fb_ioctl::FBIOGET_VSCREENINFO(file.as_raw_fd())? };
        let stride = var.xres * (var.bits_per_pixel / 8);
        let size = (stride as usize) * (var.yres as usize);
        let front = unsafe { memmap2::MmapOptions::new().len(size).map_mut(&file)? };
        let back = vec![0u8; size];
        Ok(Arc::new(Self {
            front: std::sync::Mutex::new(front),
            back:  std::sync::Mutex::new(back),
            width: var.xres,
            height: var.yres,
            stride,
            bpp: var.bits_per_pixel,
        }))
    }

    /// Copy a damage rect from back to front. NOTE: caller must NOT hold
    /// any Surface lock (lock order: Surface < FB). Debug-asserted.
    pub fn flush_dirty(&self, damage: Rect) {
        debug_assert!(
            !crate::display::IN_COMPOSITE_PASS.with(|c| c.get())
            || crate::display::FB_INSIDE_COMPOSITE.with(|c| c.get()),
            "FB flush_dirty called inside composite pass without explicit marker"
        );
        let back = self.back.lock().unwrap();
        let mut front = self.front.lock().unwrap();
        let row_bytes = (damage.w * (self.bpp / 8)) as usize;
        let stride = self.stride as usize;
        for y in 0..damage.h as i32 {
            let row = (damage.y + y) as usize;
            if row >= self.height as usize { break; }
            let col_off = (damage.x as usize) * (self.bpp / 8) as usize;
            let src = &back[row * stride + col_off..row * stride + col_off + row_bytes];
            let dst = &mut front[row * stride + col_off..row * stride + col_off + row_bytes];
            dst.copy_from_slice(src);
        }
    }
}

#[derive(Debug)]
pub enum FbError {
    Io(std::io::Error),
    Ioctl(nix::errno::Errno),
}

impl From<std::io::Error> for FbError { fn from(e: std::io::Error) -> Self { Self::Io(e) } }
impl From<nix::errno::Errno> for FbError { fn from(e: nix::errno::Errno) -> Self { Self::Ioctl(e) } }
```

### 5.6 Lock-Order Detection (Debug Build Only)

```rust
// supervisor/src/display/mod.rs (top)

thread_local! {
    /// Set to true while the compositor pass is running on this thread.
    /// Used by debug assertions to catch reentrant locking.
    pub static IN_COMPOSITE_PASS: std::cell::Cell<bool> = std::cell::Cell::new(false);

    /// Set to true when the FB lock is intentionally acquired inside
    /// the composite pass; cleared when released. Used to bypass the
    /// "FB outside composite" debug check.
    pub static FB_INSIDE_COMPOSITE: std::cell::Cell<bool> = std::cell::Cell::new(false);
}
```

---

## 6. Chrome-in-Surface Model

### 6.1 Why Chrome Moves into the Surface

The pre-Round-11 design had `chrome.rs::draw_chrome_onto(fb, app, focused)` and `repaint_all_borders` writing titlebar bytes directly into `fb.back`. This created two problems:

1. **Damage tracking missed chrome**: when an app's `AtomicDamageRect` was unioned for a content-only repaint, the screen damage rect did not include the chrome strip. After a focus change, the supervisor needed to call `repaint_all_borders` and force a full-screen damage union to make the change visible — a 1080p full-flush is the entire frame budget on iot-edge.

2. **Lock ordering issue**: chrome.rs took `fb.back.lock()` independently of the compositor's pass, opening up an opportunity for deadlock with the surface RwLock.

By making chrome part of the window's Surface, both problems disappear: chrome repaints are surface writes (covered by the per-window write lock), and the chrome strip is part of the surface that's blitted into the screen-damage region.

### 6.2 ChromeCompose Module

```rust
// supervisor/src/display/chrome_compose.rs

use std::sync::Arc;

pub struct ChromeCompose {
    font: Arc<dyn FontProvider>,
}

impl ChromeCompose {
    pub fn new(font: Arc<dyn FontProvider>) -> Self {
        Self { font }
    }

    /// Repaint the top CHROME_H rows of the window's Surface with the chrome
    /// (titlebar, traffic-light buttons, border).
    /// Caller MUST hold the per-window Surface write lock (or the caller
    /// must be the compositor thread that is about to acquire it).
    pub fn repaint_chrome(&self, meta: &WindowMeta, surface: &Arc<RwLock<Surface>>) {
        let mut surf = surface.write().unwrap();
        let w = surf.width;
        let stride = surf.stride;

        // Background color depends on focus.
        let bg = if meta.is_focused {
            BGRA32::new(0x2A, 0x2A, 0x32, 0xFF) // active dark
        } else {
            BGRA32::new(0x1A, 0x1A, 0x22, 0xFF) // inactive darker
        };

        // Fill chrome strip background.
        let pixel = bg.to_array();
        for y in 0..CHROME_H {
            let row_off = (y as usize) * stride as usize;
            for x in 0..w {
                let off = row_off + (x as usize) * 4;
                surf.buf[off..off + 4].copy_from_slice(&pixel);
            }
        }

        // Draw 3 traffic-light buttons on the left.
        const BTN_R: i32 = 6;
        const BTN_Y: i32 = (CHROME_H / 2) as i32;
        let btns = [
            (12,  BGRA32::new(0xFF, 0x60, 0x57, 0xFF)), // close
            (32,  BGRA32::new(0xFE, 0xBC, 0x2E, 0xFF)), // min
            (52,  BGRA32::new(0x28, 0xC8, 0x40, 0xFF)), // max/zoom
        ];
        for (cx, color) in btns {
            draw_filled_circle(&mut surf, cx, BTN_Y, BTN_R, color);
        }

        // Draw centered title.
        let title_color = if meta.is_focused {
            BGRA32::new(0xFF, 0xFF, 0xFF, 0xFF)
        } else {
            BGRA32::new(0xA0, 0xA0, 0xA8, 0xFF)
        };
        self.font.render_into_surface(
            &mut surf,
            &meta.title,
            (w / 2) as i32 - (meta.title.len() as i32 * 4),
            ((CHROME_H / 2) as i32) - 8,
            FontSize::Medium,
            title_color,
        );
    }
}

#[derive(Copy, Clone)]
pub struct BGRA32 { pub b: u8, pub g: u8, pub r: u8, pub a: u8 }
impl BGRA32 {
    pub const fn new(r: u8, g: u8, b: u8, a: u8) -> Self { Self { b, g, r, a } }
    pub const fn to_array(self) -> [u8; 4] { [self.b, self.g, self.r, self.a] }
}

fn draw_filled_circle(surf: &mut Surface, cx: i32, cy: i32, r: i32, color: BGRA32) {
    let px = color.to_array();
    let stride = surf.stride as usize;
    for dy in -r..=r {
        let y = cy + dy;
        if y < 0 || y >= surf.height as i32 { continue; }
        let row_off = (y as usize) * stride;
        let dx_max = ((r * r - dy * dy) as f32).sqrt() as i32;
        for dx in -dx_max..=dx_max {
            let x = cx + dx;
            if x < 0 || x >= surf.width as i32 { continue; }
            let off = row_off + (x as usize) * 4;
            surf.buf[off..off + 4].copy_from_slice(&px);
        }
    }
}
```

### 6.3 Chrome Damage Trigger

The Window Manager (R21) sets `chrome_dirty = true` on the affected window for any of these events:

- Focus change (key window changes).
- Title update via `set_title`.
- Window resize completed (new chrome strip width).
- Window enters/exits drag state.
- Tile state change (full-screen, half-tile, etc.).

The compositor checks this flag at the top of every composite pass and repaints + damages the chrome strip.

```rust
// In ChromeCompose::repaint_chrome, after the strip is drawn:
//   The caller (Compositor::run_composite_pass) adds a damage rect:
//       entry.damage.union_in(0, 0, entry.bounds.w as i32, CHROME_H as i32);
// This ensures the chrome strip is blitted as part of this frame.
```

### 6.4 Coordinate Translation in the Protocol Dispatcher

```rust
impl ProtocolParser {
    fn handle_fill_rect(&mut self, body: &[u8], reg: &WindowRegistry) -> Result<(), ProtoErr> {
        if body.len() < 20 { return Err(ProtoErr::InvalidPayload("fill_rect")); }
        let x  = i32::from_le_bytes(body[0..4].try_into().unwrap());
        let y  = i32::from_le_bytes(body[4..8].try_into().unwrap());
        let w  = u32::from_le_bytes(body[8..12].try_into().unwrap());
        let h  = u32::from_le_bytes(body[12..16].try_into().unwrap());
        let rgba = u32::from_le_bytes(body[16..20].try_into().unwrap());

        let key = self.selected.ok_or(ProtoErr::NoWindow)?;
        let surface = reg.surface(key).ok_or(ProtoErr::UnknownLocalId)?;

        // App coords have origin at content-area top-left.
        // Surface coords have origin at chrome-strip top-left.
        let surf_y = y + CHROME_H as i32;

        {
            let mut surf = surface.write().unwrap();
            crate::display::compositor::fill_rect_in_surface(
                &mut surf, x, surf_y, w, h, rgba, self.clip,
            );
        }

        let damage = reg.damage(key).ok_or(ProtoErr::UnknownLocalId)?;
        damage.union_in(x, surf_y, x + w as i32, surf_y + h as i32);
        Ok(())
    }
    // ... similarly for draw_text, draw_image, blit_surface
}
```

Note: `surface.write()` acquires the surface write lock briefly per draw command. Under heavy traffic from one app, this is uncontested; under heavy traffic from multiple apps to *different* windows, it remains uncontested because each app has its own surface. The only contention point is the compositor pass's read lock against an in-flight draw command — and even there, parker latency is in the microseconds because each draw is microseconds.

We could relax the per-draw-write-lock further by buffering draw commands per app and applying them in batch under one lock acquisition, but this adds complexity without clear benefit at the current target of <500 draws per frame per app. Deferred to optimization phase if profiling shows it.

---

## 7. VSync and Frame Scheduling

### 7.1 VsyncSource — timerfd First, DRM Fallback

```rust
// supervisor/src/display/vsync.rs

use std::os::unix::io::RawFd;

pub struct VsyncSource {
    kind: VsyncKind,
    hz: u32,
}

enum VsyncKind {
    /// Hardware vblank via DRM_IOCTL_WAIT_VBLANK on `/dev/dri/card0`.
    /// Used when running on bare metal or KVM with real GPU passthrough.
    DrmVblank { drm_fd: RawFd },

    /// Fallback for virtio-gpu and headless: timerfd with absolute deadline.
    /// `it_interval = 1e9 / hz` ns. Reads return the missed_count.
    Timerfd { fd: RawFd },
}

impl VsyncSource {
    pub fn open(hz: u32) -> Self {
        // Try DRM first.
        if let Some(fd) = try_open_drm_vblank() {
            crate::log::info!("vsync: using DRM hardware vblank, hz={}", hz);
            return Self { kind: VsyncKind::DrmVblank { drm_fd: fd }, hz };
        }
        // Fall back to timerfd.
        let fd = timerfd_create(hz);
        crate::log::info!("vsync: using timerfd CLOCK_MONOTONIC, hz={}", hz);
        Self { kind: VsyncKind::Timerfd { fd }, hz }
    }

    /// Block until the next vsync tick. Returns the number of *missed* ticks
    /// (0 = on-schedule; 1+ = catch-up needed). The compositor uses this to
    /// log frame drops without changing its behaviour — it still runs exactly
    /// one composite pass per call.
    pub fn wait_tick(&self) -> u64 {
        match &self.kind {
            VsyncKind::DrmVblank { drm_fd } => {
                drm_wait_vblank_once(*drm_fd);
                0 // DRM doesn't tell us about misses; assume 0
            }
            VsyncKind::Timerfd { fd } => {
                let mut buf = [0u8; 8];
                // Blocking read; kernel writes the missed_count.
                let n = unsafe { libc::read(*fd, buf.as_mut_ptr() as *mut _, 8) };
                if n != 8 { return 0; }
                let missed = u64::from_le_bytes(buf);
                missed.saturating_sub(1) // missed = ticks since last read; subtract the one we're handling
            }
        }
    }
}

fn timerfd_create(hz: u32) -> RawFd {
    let fd = unsafe { libc::timerfd_create(libc::CLOCK_MONOTONIC, 0) };
    if fd < 0 { panic!("timerfd_create failed: {}", std::io::Error::last_os_error()); }
    let period_ns: i64 = (1_000_000_000i64 / hz as i64).max(1);
    let spec = libc::itimerspec {
        it_interval: libc::timespec { tv_sec: period_ns / 1_000_000_000, tv_nsec: period_ns % 1_000_000_000 },
        it_value:    libc::timespec { tv_sec: period_ns / 1_000_000_000, tv_nsec: period_ns % 1_000_000_000 },
    };
    let ret = unsafe { libc::timerfd_settime(fd, 0, &spec, std::ptr::null_mut()) };
    if ret != 0 { panic!("timerfd_settime failed: {}", std::io::Error::last_os_error()); }
    fd
}

fn try_open_drm_vblank() -> Option<RawFd> {
    // Try /dev/dri/card0 — open + initial DRM_IOCTL_WAIT_VBLANK probe.
    let path = std::ffi::CString::new("/dev/dri/card0").unwrap();
    let fd = unsafe { libc::open(path.as_ptr(), libc::O_RDWR | libc::O_CLOEXEC) };
    if fd < 0 { return None; }

    // Probe DRM_IOCTL_WAIT_VBLANK with type = ABSOLUTE, sequence = 1.
    // If it returns success, DRM vblank is functional.
    let mut req: drm_wait_vblank = unsafe { std::mem::zeroed() };
    req.request.type_ = DRM_VBLANK_RELATIVE;
    req.request.sequence = 1;
    let ret = unsafe { libc::ioctl(fd, DRM_IOCTL_WAIT_VBLANK, &mut req) };
    if ret == 0 {
        Some(fd)
    } else {
        unsafe { libc::close(fd); }
        None
    }
}

fn drm_wait_vblank_once(fd: RawFd) {
    let mut req: drm_wait_vblank = unsafe { std::mem::zeroed() };
    req.request.type_ = DRM_VBLANK_RELATIVE;
    req.request.sequence = 1;
    let _ = unsafe { libc::ioctl(fd, DRM_IOCTL_WAIT_VBLANK, &mut req) };
}

#[repr(C)]
union drm_wait_vblank {
    request: drm_wait_vblank_request,
    reply: drm_wait_vblank_reply,
}

#[repr(C)]
#[derive(Copy, Clone)]
struct drm_wait_vblank_request {
    type_: u32,
    sequence: u32,
    signal: u64,
}

#[repr(C)]
#[derive(Copy, Clone)]
struct drm_wait_vblank_reply {
    type_: u32,
    sequence: u32,
    tval_sec: i64,
    tval_usec: i64,
}

const DRM_VBLANK_RELATIVE: u32 = 0x1;
const DRM_IOCTL_WAIT_VBLANK: libc::c_ulong = 0xc018_643a;
```

### 7.2 Per-Platform Hz Selection

| Platform | Hz | Notes |
|---|---|---|
| desktop-full | 60 | Standard target; 16.67 ms budget |
| mobile | 60 | Same as desktop; 16.67 ms budget |
| iot-edge | 30 | 33 ms budget; smaller surfaces; less work |
| robotics-rt | 30 | Same as iot-edge; UI is supervisory, not critical |
| mcu-minimal | 15 | Direct-draw; ~66 ms ok for status displays |
| server-headless | 1 | UI refresh rare; conserves CPU; can be lowered to 0.1 |

Configured in `supervisor/src/profile/profiles/<name>.toml`:

```toml
[display]
hz = 60
surface_format = "bgra32"
max_surface_w  = 4096
max_surface_h  = 4096
shared_buffer  = true
```

### 7.3 Frame Scheduler — When Surface Resize Is Atomic

A resize requested mid-frame is not safe to apply during a composite pass (would change the surface dimensions while a read lock is held). Resize requests are queued into a `pending_resize: AtomicBool` per window, and applied at the *start* of the next composite pass under the write lock.

```rust
// supervisor/src/display/frame_scheduler.rs

pub struct FrameScheduler {
    pending: Mutex<Vec<PendingResize>>,
}

pub struct PendingResize {
    pub key: IidKey,
    pub new_w: u32,
    pub new_h: u32,
}

impl FrameScheduler {
    pub fn enqueue_resize(&self, key: IidKey, w: u32, h: u32) {
        self.pending.lock().unwrap().push(PendingResize { key, new_w: w, new_h: h });
    }

    pub fn apply_pending(&self, reg: &WindowRegistry) {
        let mut q = self.pending.lock().unwrap();
        let drain: Vec<PendingResize> = q.drain(..).collect();
        drop(q);
        for r in drain {
            if let Some(surf) = reg.surface(r.key) {
                let mut s = surf.write().unwrap();
                s.resize(r.new_w, r.new_h + CHROME_H);
                if let Some(d) = reg.damage(r.key) {
                    d.full(r.new_w as i32, (r.new_h + CHROME_H) as i32);
                }
            }
            if let Some(cd) = reg.chrome_dirty(r.key) {
                cd.store(true, Ordering::Release);
            }
        }
    }
}
```

The compositor calls `frame_scheduler.apply_pending(&registry)` as PHASE 0 of `run_composite_pass`, before snapshot.

---

## 8. SharedBuffer Path — Wasmtime MemoryCreator

### 8.1 When to Use SharedBuffer

The protocol path (verbs over fd 3) is sufficient for apps that paint <1500 commands per frame. For high-throughput painters (terminal at 1ms/frame, video players, image viewers), each draw command's fd write + supervisor parse adds latency. The SharedBuffer path lets the app write directly into the pixel buffer that the compositor reads from.

This is opt-in per app: `vyoma.toml` must declare `shared_buffer = true`. Only desktop-full and mobile profiles support it (other platforms lack the Wasmtime configuration knobs or have surface formats that don't map cleanly to WASM linear memory).

### 8.2 SurfaceMemoryCreator

```rust
// supervisor/src/display/shared_surface.rs

use std::sync::{Arc, Mutex};
use wasmtime::{LinearMemory, MemoryCreator, MemoryType};

pub struct SurfaceMemoryCreator {
    surface: Arc<RwLock<Surface>>,
    /// Generation counter; incremented on `swap` verb.
    generation: Arc<std::sync::atomic::AtomicU64>,
}

unsafe impl Send for SurfaceMemoryCreator {}
unsafe impl Sync for SurfaceMemoryCreator {}

unsafe impl MemoryCreator for SurfaceMemoryCreator {
    fn new_memory(
        &self,
        ty: &MemoryType,
        min: usize,
        max: Option<usize>,
        reserved: Option<usize>,
        guard: usize,
    ) -> Result<Box<dyn LinearMemory>, String> {
        let mut surf = self.surface.write().unwrap();
        // Ensure surface buf is at least `min` bytes (multiple of wasm page = 64 KiB).
        let needed = ((min + 65535) / 65536) * 65536;
        if surf.buf.len() < needed {
            surf.buf.resize(needed, 0);
        }

        // SAFETY: we keep the Arc<RwLock<Surface>> alive in the returned
        // LinearMemory; Wasmtime drops the LinearMemory before dropping
        // the Engine, so the surface outlives the WASM memory.
        let ptr = surf.buf.as_mut_ptr();
        let len = surf.buf.len();
        Ok(Box::new(SurfaceLinearMemory {
            surface: self.surface.clone(),
            generation: self.generation.clone(),
            ptr,
            len,
            max: max.unwrap_or(len),
        }))
    }
}

pub struct SurfaceLinearMemory {
    surface: Arc<RwLock<Surface>>,
    generation: Arc<std::sync::atomic::AtomicU64>,
    ptr: *mut u8,
    len: usize,
    max: usize,
}

unsafe impl Send for SurfaceLinearMemory {}
unsafe impl Sync for SurfaceLinearMemory {}

unsafe impl LinearMemory for SurfaceLinearMemory {
    fn byte_size(&self) -> usize { self.len }
    fn maximum_byte_size(&self) -> Option<usize> { Some(self.max) }
    fn grow_to(&mut self, new_size: usize) -> Result<(), String> {
        let mut surf = self.surface.write().unwrap();
        if new_size > self.max { return Err("exceeds max".into()); }
        surf.buf.resize(new_size, 0);
        self.ptr = surf.buf.as_mut_ptr();
        self.len = surf.buf.len();
        Ok(())
    }
    fn as_ptr(&self) -> *mut u8 { self.ptr }
}
```

### 8.3 Engine Wiring

```rust
// supervisor/src/spawn.rs (modified for shared-buffer apps)

fn build_engine_for_shared_buffer(surface: Arc<RwLock<Surface>>) -> wasmtime::Engine {
    let generation = Arc::new(std::sync::atomic::AtomicU64::new(0));
    let creator = SurfaceMemoryCreator { surface, generation: generation.clone() };
    let mut config = wasmtime::Config::new();
    config.with_host_memory(Arc::new(creator));
    config.wasm_memory64(false);
    config.async_support(false);
    config.epoch_interruption(true); // R5
    wasmtime::Engine::new(&config).expect("shared-buffer engine")
}
```

### 8.4 Frame Submission via `swap`

The app draws into its linear memory (which *is* the Surface buffer), then submits a `swap` verb on fd 3 with an empty payload:

```text
[u16 len=1][verb=0x23]
```

On receipt, the supervisor:

1. Increments `generation`.
2. Marks the entire surface dirty (`damage.full(w, h)`) — single-buffer semantics: the supervisor doesn't know which sub-region the app touched. (A future v2.1 verb `swap_with_damage:x,y,w,h` would let the app declare a partial damage region.)
3. Returns immediately to the parser loop; the next composite pass picks it up.

```rust
fn handle_swap(&mut self, reg: &WindowRegistry) -> Result<(), ProtoErr> {
    let key = self.selected.ok_or(ProtoErr::NoWindow)?;
    let surface = reg.surface(key).ok_or(ProtoErr::UnknownLocalId)?;
    let damage = reg.damage(key).ok_or(ProtoErr::UnknownLocalId)?;

    // Don't take any lock on the surface; we trust the generation counter.
    // Mark full damage (with chrome offset accounted for at composite time).
    let (w, h) = { let s = surface.read().unwrap(); (s.width, s.height) };
    damage.full(w as i32, h as i32);

    if let Some(gen_atomic) = &self.shared_generation {
        gen_atomic.fetch_add(1, Ordering::AcqRel);
    }
    Ok(())
}
```

### 8.5 Tearing Acknowledgement

Single-buffer means the compositor's read lock on the surface can be acquired while the app is mid-write (the app is not under any lock on the WASM memory side; Wasmtime doesn't synchronize linear memory access with host code). This can cause **tearing** — the compositor reads a half-updated pixel buffer.

For v1, this is acceptable. Apps that need tear-free rendering use the protocol path (verbs). Apps that use SharedBuffer are explicitly opting into "fast but tear-prone" mode.

Future double-buffering (v2.x): the SurfaceLinearMemory would expose two buffers, with `swap` atomically flipping which one is "front" and which is "back". The compositor only reads from "front". Deferred.

### 8.6 Permission Gating

```toml
# apps/terminal/vyoma.toml
[capabilities]
stdio            = true
display          = true
display_protocol = "v2"
shared_buffer    = true   # opt in to fast path
```

```rust
// supervisor/src/manifest.rs
#[derive(Debug, Deserialize)]
pub struct Capabilities {
    // ... existing ...
    #[serde(default)]
    pub shared_buffer: bool,
    #[serde(default)]
    pub display_protocol: Option<String>, // "v1" or "v2"
}

// At spawn:
if app.caps.shared_buffer {
    if !platform.supports_shared_buffer() {
        return Err(SpawnError::CapabilityUnsupported("shared_buffer"));
    }
    let surface = registry.surface(window_key).unwrap();
    let engine = build_engine_for_shared_buffer(surface);
    // ... use engine instead of the standard one
}
```

---

## 9. Per-Platform Surface Format and Dimension Caps

### 9.1 Format Table

| Platform | Format | Bytes/px | Reason |
|---|---|---|---|
| desktop-full | `Bgra32` | 4 | High-fidelity desktop with alpha blending; matches FB format |
| mobile | `Bgra32` | 4 | Same as desktop; alpha blending needed for UI |
| server-headless | `Bgra32` | 4 | Format consistency; rare composite passes |
| iot-edge | `Rgb565` | 2 | Smaller embedded displays; saves 50% surface memory; no alpha overhead |
| robotics-rt | `Rgb565` | 2 | Same as iot-edge; HUD-style overlays don't need alpha |
| mcu-minimal | `Mono1bpp` | 0.125 | OLED/e-ink style; no off-screen buffer at all |

### 9.2 Dimension Caps

| Platform | max_surface_w | max_surface_h |
|---|---|---|
| desktop-full | 4096 | 4096 |
| mobile | 1920 | 1080 |
| server-headless | 1920 | 1080 |
| iot-edge | 800 | 600 |
| robotics-rt | 640 | 480 |
| mcu-minimal | 320 | 240 |

These are enforced at `create_window` parse time:

```rust
fn handle_create_window(&mut self, body: &[u8], reg: &WindowRegistry) -> Result<(), ProtoErr> {
    let local_id = u32::from_le_bytes(body[0..4].try_into().unwrap());
    let w = u32::from_le_bytes(body[4..8].try_into().unwrap());
    let h = u32::from_le_bytes(body[8..12].try_into().unwrap());
    let title_len = u16::from_le_bytes(body[12..14].try_into().unwrap()) as usize;
    let title = std::str::from_utf8(&body[14..14 + title_len])
        .map_err(|_| ProtoErr::InvalidPayload("utf8 title"))?
        .to_string();

    let profile = crate::profile::current();
    if w > profile.display.max_surface_w || h > profile.display.max_surface_h {
        return Err(ProtoErr::SurfaceCapExceeded);
    }

    let bounds = WindowBounds { x: 0, y: 0, w, h };
    let key = reg.create_window(
        self.app_iid, &self.app_name, local_id, bounds, title,
        profile.display.surface_format,
    ).map_err(|_| ProtoErr::InvalidPayload("create"))?;
    self.local_map.insert(local_id, key);
    self.selected = Some(key);
    self.state = ParserState::Idle;
    Ok(())
}
```

### 9.3 mcu-minimal Special Case

On mcu-minimal:

- No `Compositor` thread is spawned (no off-screen Surface to composite).
- The parser handles `fill_rect` / `draw_text` by writing **directly into the framebuffer back-buffer** (which is the only buffer).
- Z-order is undefined: last-writer-wins semantically (the WM enforces non-overlapping tiling at create-window time).
- `begin_frame` / `end_frame` are accepted but have no effect on flushing; the parser flushes the dirty region of the FB after each `end_frame`.
- `swap` is rejected (no shared buffer support).

```rust
// supervisor/src/display/direct_draw.rs (mcu-minimal only)

pub fn dispatch_direct(verb: u8, body: &[u8], fb: &Framebuffer) {
    match verb {
        0x30 => direct_fill_rect(body, fb),
        0x31 => direct_draw_text(body, fb),
        0x21 => fb.flush_dirty(track_damage_for_app()),
        _ => { /* ignore */ },
    }
}
```

---

## 10. Window Lifecycle — Explicit and Race-Free

### 10.1 V1 App Default-Window Synthesis (Eager)

```rust
// supervisor/src/app_threads.rs (modified)

pub fn launch_app_threads(app: &AppHandle, registry: &Arc<WindowRegistry>) -> AppIo {
    // 1. Apply tiling layout to assign initial bounds.
    let bounds = crate::wm::apply_tiling_layout(app, registry);

    // 2. Synthesize default window EAGERLY for v1 apps.
    if app.caps.display && app.caps.display_protocol.as_deref().unwrap_or("v1") == "v1" {
        let profile = crate::profile::current();
        let key = registry.create_window(
            app.iid,
            &app.name,
            0, // default local_id for v1
            bounds,
            app.name.clone(),
            profile.display.surface_format,
        ).expect("default window allocation");
        // Stash key in app state so the v1 parser can find it.
        app.state.lock().unwrap().default_window_key = Some(key);
    }

    // 3. NOW spawn IO threads. Any draw command they deliver will find
    //    the default window already in the registry. No race.
    let stdout_thread = spawn_stdout_reader(app, registry);
    let stderr_thread = spawn_stderr_reader(app);
    let display_fd_thread = if app.caps.display && app.caps.display_protocol.as_deref() == Some("v2") {
        Some(spawn_display_fd_reader(app, registry))
    } else {
        None
    };

    AppIo { stdout: stdout_thread, stderr: stderr_thread, display_fd: display_fd_thread }
}
```

### 10.2 V2 App First-Command Contract

The first command from a v2 app MUST be `create_window` (after optional `negotiate`). Any draw command before `create_window` is rejected:

```rust
ParserState::Uninitialized => match verb {
    0x01 => self.handle_negotiate(body), // negotiate first (optional)
    0x10 => self.handle_create_window(body, reg),
    _    => Err(ProtoErr::NoWindow),
}
```

The supervisor logs the violation and signals the app via stdin:

```text
VYOMA_DISPLAY_EVENT:protocol_error:draw_before_create_window
```

The app's `display` capability is **not** revoked; the parser remains in `Uninitialized` and waits for a valid `create_window`. This is forgiving for bug-fixing.

### 10.3 Close Window

```rust
fn handle_close_window(&mut self, body: &[u8], reg: &WindowRegistry) -> Result<(), ProtoErr> {
    let local_id = u32::from_le_bytes(body[0..4].try_into().unwrap());
    let key = *self.local_map.get(&local_id).ok_or(ProtoErr::UnknownLocalId)?;
    reg.close_window(key).map_err(|_| ProtoErr::UnknownLocalId)?;
    self.local_map.remove(&local_id);
    if self.selected == Some(key) {
        // Auto-select next available window, or fall back to Uninitialized.
        self.selected = self.local_map.values().next().copied();
        if self.selected.is_none() {
            self.state = ParserState::Uninitialized;
        }
    }
    // Signal the app.
    self.emit_event(format!("VYOMA_DISPLAY_EVENT:window_closed:{}", local_id));
    Ok(())
}
```

### 10.4 Atomic Resize

Resize is split into two phases:

1. **Request**: `set_bounds(x, y, w, h)` updates the metadata bounds, but does NOT resize the Surface buffer immediately (the compositor might be mid-blit using the old dimensions).
2. **Apply**: At the top of the next composite pass, the FrameScheduler applies all pending resizes under each surface's write lock.

```rust
impl WindowRegistry {
    pub fn set_bounds(&self, key: IidKey, new_bounds: WindowBounds) -> Result<(), RegistryError> {
        let (old_w, old_h) = {
            let mut meta = self.metadata.lock().unwrap();
            let win = meta.windows.get_mut(&key).ok_or(RegistryError::UnknownWindow)?;
            let old = (win.bounds.w, win.bounds.h);
            win.bounds = new_bounds;
            old
        };
        if (old_w, old_h) != (new_bounds.w, new_bounds.h) {
            crate::display::FRAME_SCHEDULER.enqueue_resize(key, new_bounds.w, new_bounds.h);
        }
        // chrome_dirty is also set on bounds change (since chrome strip width changes)
        if let Some(cd) = self.chrome_dirty(key) {
            cd.store(true, Ordering::Release);
        }
        Ok(())
    }
}
```

---

## 11. Thermal Throttle Integration (Round 7)

```rust
// supervisor/src/display/thermal_bridge.rs

use std::sync::atomic::{AtomicU8, Ordering};

pub struct DisplayThermalBridge {
    skip_mask: Arc<AtomicU8>,
}

impl DisplayThermalBridge {
    pub fn new(skip_mask: Arc<AtomicU8>) -> Self { Self { skip_mask } }

    /// Called by the ThermalGovernor whenever the thermal tier changes.
    pub fn on_tier_change(&self, tier: crate::power::ThermalTier) {
        let mask = match tier {
            crate::power::ThermalTier::Nominal => 1,  // every tick
            crate::power::ThermalTier::Warm    => 2,  // 30 fps
            crate::power::ThermalTier::Hot     => 4,  // 15 fps
            crate::power::ThermalTier::Critical => 8, // 7.5 fps
        };
        self.skip_mask.store(mask, Ordering::Release);
        crate::log::info!("display: thermal tier {:?} → skip_mask={}", tier, mask);
    }
}
```

Registration at compositor spawn:

```rust
// supervisor/src/main.rs
let bridge = Arc::new(DisplayThermalBridge::new(compositor_handle.skip_mask.clone()));
thermal_governor.register_observer(bridge);
```

Note: the compositor still runs (reads timerfd, processes events) every tick — it just skips the *blit phase* on throttled ticks. This keeps damage atomics accumulating correctly; on the next non-skipped tick, the full accumulated damage is flushed at once.

---

## 12. Font System Interface (R13 Integration Point)

The display server does not own fonts. Round 13 will define a full font system; for Round 11, we define the *interface* that the display server uses, with a default 8×16 bitmap implementation for v1.

```rust
// supervisor/src/display/font_iface.rs

/// Font size enum matching VYOMA_DRAW text size codes.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum FontSize {
    Small,  // 4×8
    Medium, // 8×16
    Large,  // 16×32
}

impl FontSize {
    pub fn from_byte(b: u8) -> Option<Self> {
        match b {
            b's' => Some(Self::Small),
            b'm' => Some(Self::Medium),
            b'l' => Some(Self::Large),
            _ => None,
        }
    }

    pub fn pixel_width(self) -> u32 {
        match self { Self::Small => 4, Self::Medium => 8, Self::Large => 16 }
    }

    pub fn pixel_height(self) -> u32 {
        match self { Self::Small => 8, Self::Medium => 16, Self::Large => 32 }
    }
}

/// The display server uses any type implementing FontProvider to render text.
/// Round 13 will provide a TrueType-backed implementation; Round 11 ships a
/// bitmap default.
pub trait FontProvider: Send + Sync {
    /// Render `text` into the given Surface starting at (x, y) (surface coords).
    /// Clips to surface bounds. Returns the (advance_x, advance_y) consumed.
    fn render_into_surface(
        &self,
        surface: &mut Surface,
        text: &str,
        x: i32,
        y: i32,
        size: FontSize,
        color: BGRA32,
    ) -> (i32, i32);

    /// Compute the rendered width of `text` at the given size.
    fn measure(&self, text: &str, size: FontSize) -> (i32, i32);

    /// Return an opaque token identifying the current font set; used for
    /// caching glyph atlases. Changes when the user installs/uninstalls fonts.
    fn version(&self) -> u64;
}

/// Default v1 implementation: 8×16 monospace bitmap, ASCII-only.
pub struct BitmapFontProvider {
    glyphs_4x8:   &'static [[u8; 4]; 128],
    glyphs_8x16:  &'static [[u8; 16]; 128],
    glyphs_16x32: &'static [[u8; 64]; 128],
}

impl BitmapFontProvider {
    pub fn new() -> Self {
        Self {
            glyphs_4x8:   crate::display::font::FONT_4X8,
            glyphs_8x16:  crate::display::font::FONT_8X16,
            glyphs_16x32: crate::display::font::FONT_16X32,
        }
    }
}

impl FontProvider for BitmapFontProvider {
    fn render_into_surface(&self, surface: &mut Surface, text: &str, x: i32, y: i32,
                           size: FontSize, color: BGRA32) -> (i32, i32) {
        let glyph_w = size.pixel_width() as i32;
        let glyph_h = size.pixel_height() as i32;
        let mut cur_x = x;
        for byte in text.bytes() {
            if byte < 32 || byte >= 128 { continue; } // ASCII printable only
            self.composite_glyph(surface, byte, cur_x, y, size, color);
            cur_x += glyph_w;
            if cur_x + glyph_w > surface.width as i32 { break; }
        }
        (cur_x - x, glyph_h)
    }

    fn measure(&self, text: &str, size: FontSize) -> (i32, i32) {
        let n = text.bytes().filter(|b| (32..128).contains(b)).count() as i32;
        (n * size.pixel_width() as i32, size.pixel_height() as i32)
    }

    fn version(&self) -> u64 { 1 } // bitmap never changes
}

impl BitmapFontProvider {
    fn composite_glyph(&self, surface: &mut Surface, ch: u8, x: i32, y: i32,
                       size: FontSize, color: BGRA32) {
        // Reuse existing composite_glyph from supervisor/src/display/compositor.rs
        crate::display::compositor::composite_glyph(surface, ch, x, y, size, color);
    }
}
```

When Round 13 lands, the supervisor's font registration swaps out `BitmapFontProvider` for a `CoreTextLikeFontProvider`:

```rust
// In main.rs init:
let font_provider: Arc<dyn FontProvider> = if profile.font.use_truetype {
    Arc::new(crate::fonts::TrueTypeProvider::open(&profile.font.search_paths)?)
} else {
    Arc::new(BitmapFontProvider::new())
};
```

---

## 13. Implementation Files and Line Budgets

All files must be ≤500 LOC per the repository's hard limit. The Round 11 split is:

| File | LOC budget | Purpose |
|---|---|---|
| `supervisor/src/display/mod.rs` | 200 | Module root; lock-order docs; thread-local sentinels; re-exports |
| `supervisor/src/display/framebuffer.rs` | 250 | `Framebuffer` struct, `open`, `flush_dirty`, fb_ioctl wrappers |
| `supervisor/src/display/surface.rs` | 200 | `Surface`, `SurfaceFormat`, `BGRA32`, `Rect` |
| `supervisor/src/display/window_registry.rs` | 450 | `WindowRegistry`, `WindowMeta`, sharded API |
| `supervisor/src/display/atomic_damage.rs` | 100 | `AtomicDamageRect` |
| `supervisor/src/display/compositor.rs` | 300 | `Compositor` struct, spawn, main loop |
| `supervisor/src/display/compositor_pass.rs` | 350 | `run_composite_pass`, phase orchestration |
| `supervisor/src/display/blit.rs` | 400 | `blit_surface_to_fb`, format-specific paths |
| `supervisor/src/display/protocol_v2.rs` | 480 | Framed parser, FSM, dispatch table |
| `supervisor/src/display/protocol_v1.rs` | 250 | Legacy text protocol bridge to v2 verbs |
| `supervisor/src/display/dispatch.rs` | 350 | Verb handlers (fill_rect, draw_text, ...) |
| `supervisor/src/display/vsync.rs` | 300 | `VsyncSource`, timerfd + DRM probe |
| `supervisor/src/display/frame_scheduler.rs` | 150 | `FrameScheduler`, pending resize apply |
| `supervisor/src/display/chrome_compose.rs` | 350 | Chrome strip painting into Surface |
| `supervisor/src/display/shared_surface.rs` | 300 | `SurfaceMemoryCreator`, `SurfaceLinearMemory` |
| `supervisor/src/display/font_iface.rs` | 250 | `FontProvider` trait, `BitmapFontProvider` |
| `supervisor/src/display/font.rs` | 480 | Bitmap glyph tables (4×8, 8×16, 16×32) |
| `supervisor/src/display/cursor.rs` | 200 | `CursorState`, cursor compositing (existing, kept) |
| `supervisor/src/display/animator.rs` | 100 | `now_ns()`, `now_ms()` (existing, kept) |
| `supervisor/src/display/fb_ioctl.rs` | 80 | `FBIOGET_VSCREENINFO` (existing, kept) |
| `supervisor/src/display/direct_draw.rs` | 250 | mcu-minimal direct-FB shim (gated) |
| `supervisor/src/display/types.rs` | 150 | Shared types: `IidKey`, `WindowBounds`, `Rect` |
| `supervisor/src/display/thermal_bridge.rs` | 80 | R7 ThermalObserver impl |
| `supervisor/src/display/stats.rs` | 100 | `CompositorStats`, telemetry export |

Total: ~5170 LOC across 24 files. All under 500.

---

## 14. Interaction with Prior Rounds

### 14.1 R1 — WIT Callbacks

R1 defined the WIT interface `vyoma:display/window-callbacks` with `on-display-ready(width, height)`. R11 implements this by emitting the event after the default window is created and the first composite pass completes successfully. Apps that don't use the WIT bindings ignore it; apps that do receive the actual usable content area (which already accounts for chrome offset).

### 14.2 R2 — SharedBuffer Concept

R2 sketched the SharedBuffer concept abstractly. R11 makes it concrete via Wasmtime's `MemoryCreator` API. The `shared_buffer = true` capability is the gate; the SurfaceMemoryCreator is the implementation.

### 14.3 R5 — Scheduler & QoS

R5 defined `QosClass::UserInteractive` and SCHED_FIFO. R11 spawns the compositor thread at `SCHED_FIFO priority 40`:

- Below audio (`SCHED_FIFO 80`, R67)
- Below input (`SCHED_RR 60`, R10)
- Above normal IPC routing threads (default `SCHED_OTHER 0`)

This ordering ensures input is processed before its visual feedback is composited (single-tick latency target), while audio retains highest priority for glitch-free playback.

### 14.4 R6 — DRM/virtio-gpu Driver

R6's `DisplayDevice` HAL trait is the abstraction R11 uses for framebuffer access. The concrete `Fb0DisplayDevice` (Linux fb0) is what `Framebuffer::open` wraps. Hardware DRM support lives in the same trait implementation; the VsyncSource probes for DRM vblank as part of `open`.

### 14.5 R7 — Thermal Governor

R7 defined `ThermalGovernor` with tiers Nominal/Warm/Hot/Critical. R11 registers `DisplayThermalBridge` as an observer; on tier change, it updates the compositor's `skip_mask` atomic. No mode switch; the compositor naturally skips ticks.

### 14.6 R8 — BootPhase

R8 placed display init in `BootPhase::Display`, after `BootPhase::Hardware` (which initializes `/dev/fb0` via the HAL). R11 details what runs in this phase:

1. `Framebuffer::open("/dev/fb0")` — mmap front buffer.
2. `WindowRegistry::new()` — empty registry.
3. `VsyncSource::open(platform_hz)` — timerfd or DRM.
4. `FontProvider` instantiation (Bitmap for v1; TrueType later).
5. `Compositor::spawn(...)` — dedicated thread starts ticking.
6. Phase transitions to `BootPhase::Apps`.

### 14.7 R9 — HAL Registry

`DisplayDevice` is registered in `HalRegistry::display`. The compositor accesses it via `HalRegistry::current().display.framebuffer()` — a stable API even if the underlying device implementation changes (fb0, DRM, virtio-gpu).

### 14.8 R10 — Multi-Threaded EventLoop

R10 defined `TimerLoop` and `InputLoop` as dedicated threads. R11's compositor thread is *not* a TimerLoop callback — it's a peer thread that owns its own timerfd. Reason: the compositor is a hot path; piping events through CallbackQueue HPQ + indirection costs ~10 µs per tick that we can spend on actual blitting.

InputLoop, however, delivers input events to the WM, which mutates registry metadata (focus, raise, hover) — and the compositor picks up those changes on the next snapshot. There is no synchronization between input and composite beyond the registry's own sharded locking.

---

## 15. Migration from Current Display Code

### 15.1 What Stays

- `supervisor/src/display/cursor.rs` — kept as-is. Cursor lives outside windows; cursor compositing is the last step of the composite pass.
- `supervisor/src/display/animator.rs` — kept. `now_ns()` / `now_ms()` used throughout.
- `supervisor/src/display/fb_ioctl.rs` — kept. `FBIOGET_VSCREENINFO` is correct.
- `supervisor/src/display/font.rs` — kept as the bitmap glyph table backend. Repurposed as the data store for `BitmapFontProvider`.
- The R10 EventLoop / CallbackQueue / InputLoop — unchanged. Display is its own subsystem.

### 15.2 What Changes

| Current code | Replacement |
|---|---|
| `display/mod.rs` (500 LOC monolith) | Split into `mod.rs`, `framebuffer.rs`, several other modules |
| `display/compositor.rs` (314 LOC, primitives only) | Renamed → `display/primitives.rs` (`composite_glyph`, `blend_over`); compositor *thread* lives in new `compositor.rs` |
| `display/surface.rs` (168 LOC) | Becomes `display/surface.rs` (200 LOC) with format enum + per-platform stride |
| `chrome.rs` (currently writes into FB) | Functions move to `display/chrome_compose.rs`, write into per-window Surface instead |
| `draw_cmd.rs` (v1 parser) | Moves to `display/protocol_v1.rs`; emits v2 verbs internally |
| `router.rs` (sniffs VYOMA_DRAW from stdout) | Continues to sniff v1; new fd-3 reader thread handles v2 |
| `app_threads.rs` (no eager window creation) | Modified to eagerly create default window for v1 apps before spawning IO threads |
| `apply_tiling_layout` (in `wm.rs`) | Adjusted to return bounds rather than directly mutate registry; registry create happens after layout |

### 15.3 Migration Order (Tasks)

1. **Add new files; don't delete old.** Create `display/window_registry.rs`, `atomic_damage.rs`, `compositor.rs` (new), `compositor_pass.rs`, `protocol_v2.rs`, `vsync.rs`, `chrome_compose.rs`, `frame_scheduler.rs`, `shared_surface.rs`, `font_iface.rs`. Old code untouched.
2. **Wire compositor thread alongside existing code.** Compositor reads new WindowRegistry; if registry is empty, runs zero-cost skip. Existing chrome.rs continues to paint into FB. Verify no visible regression.
3. **Migrate first app to v2.** Pick `gui-demo`. Add `display_protocol = "v2"` to its `vyoma.toml`. Update its WASM source to use `vyoma_display` helper crate. Verify it still renders.
4. **Migrate v1 apps' default-window synthesis to be eager.** Modify `app_threads.rs` to create the registry entry before spawning IO threads. v1 apps now route through the v1→v2 bridge in `protocol_v1.rs`. Old `chrome.rs::repaint_all_borders` is *kept* but no longer called by the WM.
5. **Move chrome painting into Surface.** Replace `chrome.rs` calls with `chrome_compose.rs` writes; the WM sets `chrome_dirty = true` instead of calling chrome.rs directly.
6. **Delete dead code.** Remove unused functions from `chrome.rs`; remove direct-FB writes from WM.

Each migration step lands as a separate commit and is verifiable via `make smoke`.

---

## 16. Testing Strategy

### 16.1 Unit Tests

Each module has companion tests in `supervisor/tests/display_*.rs`:

- `display_atomic_damage.rs` — concurrent unioning from 16 threads; verify `take()` returns the correct union.
- `display_window_registry.rs` — concurrent create/close/raise/lower; verify Z-order invariants.
- `display_protocol_v2.rs` — parser FSM transitions; partial-frame accumulation; malformed frame rejection.
- `display_protocol_v1.rs` — legacy text protocol → v2 verb translation.
- `display_blit.rs` — pixel-accurate blit tests (each format × clipping × alpha).
- `display_chrome_compose.rs` — chrome strip layout (button positions, title centering).
- `display_vsync.rs` — timerfd tick rate measurement (±5% tolerance).
- `display_shared_surface.rs` — Wasmtime MemoryCreator allocation; grow_to.

### 16.2 Integration / Smoke Tests

```bash
make smoke-display
```

Runs:

1. `make build` for `desktop-full` profile.
2. Boot QEMU with `display=virtio-gpu`.
3. Wait for `[display] compositor thread ready` log line.
4. Launch `gui-demo` (v2 protocol app).
5. Capture framebuffer dump via QEMU monitor.
6. Verify the dump contains the expected pattern (non-zero pixel coverage, chrome strip detected).
7. Send focus-change event via `@supervisor: focus other-app`.
8. Recapture; verify chrome color changed.

Pass criteria: all steps complete within 30 s; framebuffer dumps differ in expected ways.

### 16.3 Performance Tests

```bash
make bench-compositor
```

Runs a microbenchmark inside QEMU:

- Spawns 10 windows on desktop-full.
- Each window submits 100 fill_rect commands per frame.
- Measures: avg composite pass duration, frame drop rate over 1000 frames.

Pass criteria:
- Avg composite pass <8 ms on desktop-full at 1920×1080.
- Frame drop rate <1% over 1000 frames at 60 Hz.

### 16.4 Lock-Order Test

```bash
make test-lock-order
```

A debug-build-only test:

- Spawns 16 parser threads, each acquiring random surface locks and then attempting FB lock.
- Compositor pass runs in a loop.
- After 10 seconds, the test inspects a deadlock detector (custom: tracks lock acquisition timestamps per thread; any thread that has held a lock >2 s without making progress is flagged).
- Test fails if any thread is flagged.

### 16.5 Fuzz Tests

A `cargo fuzz` target on `ProtocolParser::push_bytes`:

- Random bytes streamed in chunks.
- Parser must never panic, never enter infinite loop, never produce out-of-bounds writes.
- Run for 24h in CI weekly.

---

## 17. Critical Decisions Summary

### 17.1 Critical for v1

- Sharded WindowRegistry (metadata Mutex + per-window RwLock<Surface> + lock-free AtomicDamageRect).
- Dedicated compositor thread, SCHED_FIFO priority 40, with timerfd-driven vsync.
- VYOMA_DRAW v2 framed binary protocol over fd 3, separate from stdout.
- v1 backwards compatibility via stdout sniffing and verb translation.
- Chrome lives in Surface top strip (`CHROME_H = 28`); chrome.rs deleted.
- Per-platform `SurfaceFormat` (Bgra32 / Rgb565 / Mono1bpp) and dimension caps.
- Eager default-window synthesis for v1 apps before IO threads spawn.
- FontProvider trait with BitmapFontProvider as v1 default.
- DRM vblank probe at startup; graceful fallback to timerfd.

### 17.2 Deferred to Later Rounds

- SharedBuffer double-buffering (single-buffer with `swap` verb is v1; tearing accepted).
- Hardware KMS plane overlay for HW-accelerated cursor / video planes (Round 12).
- GPU-accelerated compositor pass (Round 12, Metal-equivalent layer).
- Multi-display support and virtual displays (Round 19).
- HiDPI / per-display scale factors (Round 20).
- TrueType / variable fonts (Round 13).
- Damage region merging across windows (Round 16 with animation engine).
- ColorSync (Round 15).
- Hot font reload via FontProvider version change (Round 13).
- Subpixel anti-aliasing (Round 13).
- Cursor hot-spot animation (Round 36).

### 17.3 Explicitly Never

- **A separate WindowServer process.** WASM sandboxing is the trust boundary; an extra process adds context-switch overhead without security benefit.
- **A single global lock on the WindowRegistry.** Convoy under load; sharded locking is mandatory.
- **`thread::sleep` for vsync timing.** ±5 ms jitter; drift over time; never use this.
- **VYOMA_DRAW commands over stdout in v2.** Injection-vulnerable; separate fd is mandatory.
- **Per-window draw commands without explicit `select_window`.** Removes ambiguity when an app has multiple windows.
- **Holding metadata Mutex during blit.** Compositor would block all parser threads for 3–8 ms; unacceptable.
- **Direct framebuffer writes from chrome painting.** Damage tracking would miss chrome updates; everything goes through Surface.
- **Integer Z-order field with global counter.** Overflow over long sessions; vec-based ordering is correct.
- **Synchronous response to draw commands from the supervisor side.** All draw commands are fire-and-forget; the only synchronous control point is `end_frame` (which still doesn't block on the compositor).

---

## 18. Summary

VyomaOS's Display Server runs entirely inside the PID-2 supervisor as a dedicated compositor thread (SCHED_FIFO priority 40), eliminating two context switches and a marshalling layer per draw command versus a separate-process architecture. WASM sandboxing provides the isolation that an out-of-process WindowServer would otherwise enforce.

The window state is sharded into three locks: a brief metadata Mutex (lifecycle ops only, ~µs holds), per-window `RwLock<Surface>` for pixel buffers, and lock-free `AtomicDamageRect` for damage tracking. The compositor pass snapshots metadata, releases the lock, and processes each window under its own read lock with the framebuffer Mutex taken once at the end — enforcing a strict `Surface < FB` lock order that is debug-asserted at every flush.

The display protocol (VYOMA_DRAW v2) is framed binary on a dedicated WASI fd 3, separate from stdout. This eliminates command injection from log lines and provides natural back-pressure through pipe buffering. The legacy v1 stdout protocol is bridged transparently for backwards compatibility.

Chrome (titlebar, traffic-light buttons, borders) is painted into the top `CHROME_H = 28` px strip of each window's own Surface — not directly into the framebuffer. This ensures chrome updates are correctly tracked by per-window damage and naturally included in the screen-damage union; the old `chrome.rs::repaint_all_borders` direct-FB function is deleted.

VSync is driven by `timerfd(CLOCK_MONOTONIC)` with an absolute-deadline `it_interval`, falling back gracefully when DRM hardware vblank is unavailable (virtio-gpu, headless). The `missed_count` reported by the kernel lets the compositor detect and log frame drops without time drift.

The SharedBuffer fast path uses Wasmtime's `MemoryCreator` API to alias the WASM app's linear memory directly onto the Surface buffer (`Config::with_host_memory(SurfaceMemoryCreator)`), gated to desktop-full and mobile profiles and the `shared_buffer = true` capability. Single-buffer with a `swap` verb increments a generation counter; double-buffering is deferred to a future minor version.

Per-platform pixel formats (`Bgra32` / `Rgb565` / `Mono1bpp`) and dimension caps prevent embedded profiles from exhausting RAM on large surfaces. On mcu-minimal, there is no off-screen Surface at all — apps direct-draw into the framebuffer through a protocol shim.

Thermal throttling (Round 7) feeds an `AtomicU8 skip_mask` that the compositor reads each vsync tick; warm/hot/critical tiers result in 30/15/7.5 fps composition without changing modes or losing damage state.

Z-order is a `Vec<IidKey>` sorted back-to-front; raise/lower are O(N) with N ≤ 50, eliminating integer overflow concerns.

Window creation is explicit and race-free: v1 apps get their default window synthesized eagerly during `launch_app_threads()` after tiling layout and before IO thread spawn; v2 apps must call `create_window` before any draw command and have their parser FSM enforce the contract (`Uninitialized → WindowCreated → InFrame → Idle`).

Implementation is split across 24 files under `supervisor/src/display/`, each under the repository's 500-LOC ceiling, totalling ~5170 LOC. The font system interfaces through a `FontProvider` trait; Round 11 ships a `BitmapFontProvider` (4×8, 8×16, 16×32 ASCII bitmaps); Round 13 will swap in a TrueType-backed implementation.

This design unblocks the Window Manager (R21), Spaces (R25–R27), and all subsequent UI subsystems. The next round (R12) builds on this surface model with GPU acceleration: turning `Surface` buffers into GPU textures and replacing the CPU blit with a shader pass.

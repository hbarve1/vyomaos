# Round 11 — Display Server & Compositor

**Role:** Architect
**Date:** 2026-05-29
**Status:** Proposal (awaiting Critic review)
**Scope:** macOS-fidelity WindowServer/SkyLight equivalent for VyomaOS — per-window surfaces, vsync-driven compositor, Z-order blit with damage tracking, VYOMA_DRAW Protocol v2, SharedBuffer zero-copy path, and integration with the HAL, interrupt subsystem, thermal governor, QoS scheduler, and boot ordering established in prior rounds.

---

## 0. Executive summary

macOS solves windowing via **WindowServer** (formerly SkyLight on iOS, now unified): a separate root-privileged process that owns the GPU, maintains a tree of *CALayer* objects per app, and composites the screen at vsync. Apps render into IOSurfaces that WindowServer maps and blits in Z-order through CoreAnimation. The trust model is "apps render pixels, WindowServer owns the display".

VyomaOS cannot literally copy that model: our apps are `wasm32-wasip2` modules executed inside the supervisor's address space by Wasmtime; there is no IPC boundary between the app and the renderer in the macOS sense (an `extern "C"` Mach RPC). Instead, the supervisor *is* both the kernel-like trust anchor *and* the host of every app. The natural design therefore puts the compositor inside the supervisor as a dedicated thread set:

```
supervisor process (PID 2)
├─ App threads (1 per WASM module)         ← Wasmtime stores
│    write VYOMA_DRAW lines to a per-app pipe
├─ Parser threads (1 per app)              ← already exist (app_threads.rs)
│    consume the pipe, mutate the per-app Surface
├─ Compositor thread (1, UserInteractive)  ← new (this round)
│    drains damage queue, blits surfaces in Z-order, flushes FB
└─ Vsync source thread (1, Realtime)       ← new (this round)
     reads DRM vblank events / falls back to monotonic timer
```

The compositor never blocks on a WASM hostcall and never holds the framebuffer mutex across a WASM call. App threads only ever mutate their *own* Surface; the compositor reads everyone's Surface during the composite pass. We use the **typestate** discipline already present in `display::mod.rs` (`dirty_top`, `dirty_bottom`) and extend it to **per-window damage rectangles** so that 99 % of frames cost less than 100 KB of bandwidth to /dev/fb0.

Key choices:
- **In-supervisor compositor** (not a separate WASM app, not a separate Linux process).
- **One Surface per window**, not one per app. Apps may own multiple windows.
- **VYOMA_DRAW Protocol v2** is line-oriented (forward-compatible with v1) and adds: window lifecycle, damage tracking, opacity, frame delimiters, and a versioned negotiation handshake.
- **SharedBuffer zero-copy path** is opt-in (capability `shared_buffer = true`) and uses Wasmtime's linear-memory aliasing trick: the WASM module's pages 0–N alias a `wasmtime::SharedMemory` that is *also* the Surface back buffer. No memcpy ever runs on the hot path.
- **Vsync** drives composite at 60 Hz on desktop, 30 Hz when ThermalGovernor signals tier 1, 15 Hz at tier 2.
- **Per-window damage rectangles** are unioned per frame; only the union is composited; only the union is blitted to /dev/fb0.

The full Rust type signatures, file split, and the test plan live below. The compositor lives in `supervisor/src/display/` with five new files (each well under 500 lines) plus three extensions to existing files.

---

## 1. Design philosophy: in-supervisor compositor vs separate process

### 1.1 Why not a separate Linux process

macOS-style separation of WindowServer from app processes solves three problems:
1. **Trust:** a crashing app cannot corrupt the framebuffer.
2. **Scheduling:** WindowServer has higher real-time priority than client apps.
3. **Resource ownership:** the GPU is one device, owned by one process.

In VyomaOS:
1. **Trust** is already enforced by WASM sandboxing — a WASM app *cannot* corrupt host memory by design. The supervisor's address space is safe even when colocated with apps.
2. **Scheduling** is solved by the QoS round (R5): the compositor thread runs at `QosClass::UserInteractive` with a real-time SCHED_FIFO priority above all app threads.
3. **Resource ownership** is solved by the HAL round (R9): `DisplayDevice` is acquired once at boot by the supervisor; apps never touch /dev/fb0 directly.

A separate Linux process would add Mach-style IPC overhead (~5 µs round-trip on virtio-vsock, ~1 µs on AF_UNIX), would require a second seccomp jail, and would fragment our memory budget on `mcu-minimal`. We get none of those costs and all of macOS's safety guarantees by keeping the compositor as a thread of PID 2.

### 1.2 Why not run the compositor as a WASM app

Tempting (more dogfooding) but rejected:
- WASM cannot call `mmap` on /dev/fb0 or DRM ioctls without hostcalls that defeat the abstraction.
- The compositor needs to inspect every other app's Surface — a per-app capability boundary that WASM modules cannot legally cross.
- Wasmtime's startup latency (~10 ms per instance) is too high for a vsync-critical path.
- Bootstrap chicken-and-egg: BootPhase::Display (R8) needs to run before any user app instantiation.

### 1.3 What this gives us

```
┌───────────────────────────────────────────────────────────────────┐
│                   supervisor process (Rust, PID 2)                │
│                                                                   │
│  ┌──────────┐ ┌──────────┐ ┌──────────┐ ┌─────────────────────┐  │
│  │  app A   │ │  app B   │ │  app C   │ │   compositor        │  │
│  │ (WASM)   │ │ (WASM)   │ │ (WASM)   │ │   thread            │  │
│  │  thread  │ │  thread  │ │  thread  │ │  (QoS=UI, prio 80)  │  │
│  │          │ │          │ │          │ │                     │  │
│  │  draws → │ │  draws → │ │  draws → │ │  reads Surfaces     │  │
│  │ Surface  │ │ Surface  │ │ Surface  │ │  blits → FB.back    │  │
│  └────┬─────┘ └────┬─────┘ └────┬─────┘ └──────────┬──────────┘  │
│       │            │            │                    │             │
│       │            │            │                    ▼             │
│       │            │            │      ┌─────────────────────┐    │
│       │            │            │      │ Framebuffer.flush() │    │
│       │            │            │      │ → /dev/fb0 (mmap)   │    │
│       │            │            │      └─────────────────────┘    │
│       │            │            │                    ▲             │
│       ▼            ▼            ▼                    │             │
│   ┌──────────────────────────────────┐  ┌────────────┴──────┐    │
│   │ WindowRegistry  (Mutex)          │  │  Vsync source     │    │
│   │  IidKey → WindowRecord           │  │  thread (RT 95)   │    │
│   │  Z-order, focus, damage          │  └───────────────────┘    │
│   └──────────────────────────────────┘                            │
└───────────────────────────────────────────────────────────────────┘
```

---

## 2. Window registry and `WindowRecord`

A window is a stable, addressable rectangle owned by an app, with a Surface, Z-order position, and lifecycle state. The supervisor maintains a global `WindowRegistry` accessed only via a `Mutex` (or `parking_lot::RwLock` once we're past v1).

```rust
// supervisor/src/display/window_registry.rs

use crate::display::surface::Surface;
use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard, OnceLock};

/// Stable per-window identifier (instantiation key).
/// One app may own many windows; each gets a unique `IidKey`.
#[derive(Copy, Clone, Hash, Eq, PartialEq, Debug)]
pub struct IidKey(pub u64);

impl IidKey {
    pub fn new() -> Self {
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT: AtomicU64 = AtomicU64::new(1);
        Self(NEXT.fetch_add(1, Ordering::Relaxed))
    }
}

/// Window lifecycle state.
#[derive(Copy, Clone, Eq, PartialEq, Debug)]
pub enum WindowState {
    /// Created, not yet visible (during open animation).
    Materialising,
    /// Visible on screen.
    Visible,
    /// Minimised to the dock; surface preserved.
    Minimised,
    /// Closing (during close animation); surface freed when animation completes.
    Dematerialising,
    /// Animation finished, ready for GC.
    Closed,
}

/// Damage rectangle in window-local coordinates (0..width, 0..height).
#[derive(Copy, Clone, Debug, Default)]
pub struct DamageRect {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

impl DamageRect {
    pub fn empty() -> Self { Self { x: 0, y: 0, w: 0, h: 0 } }
    pub fn is_empty(&self) -> bool { self.w == 0 || self.h == 0 }

    /// Union two rectangles into the smallest enclosing rectangle.
    pub fn union(self, other: Self) -> Self {
        if self.is_empty() { return other; }
        if other.is_empty() { return self; }
        let x0 = self.x.min(other.x);
        let y0 = self.y.min(other.y);
        let x1 = (self.x + self.w).max(other.x + other.w);
        let y1 = (self.y + self.h).max(other.y + other.h);
        Self { x: x0, y: y0, w: x1 - x0, h: y1 - y0 }
    }

    /// Clip to a rectangle's bounds.
    pub fn clip_to(self, max_w: u32, max_h: u32) -> Self {
        if self.is_empty() { return self; }
        let x1 = (self.x + self.w).min(max_w);
        let y1 = (self.y + self.h).min(max_h);
        if x1 <= self.x || y1 <= self.y { return Self::empty(); }
        Self { x: self.x, y: self.y, w: x1 - self.x, h: y1 - self.y }
    }
}

/// One window in the compositor's view of the world.
pub struct WindowRecord {
    pub iid:           IidKey,
    pub owner_app:     String,            // matches AppState.name
    pub title:         String,
    pub bounds:        ScreenRect,        // position on screen (display coords)
    pub z_order:       i32,               // higher = front; -1 reserved for cursor layer
    pub state:         WindowState,
    pub surface:       Surface,           // owned per-window pixel buffer
    pub global_alpha:  u8,                // 0..255, applied at composite time
    pub damage:        DamageRect,        // accumulated since last composite, window-local
    pub is_key:        bool,              // receives keyboard focus
    pub is_front:      bool,              // top of stacking order excluding floating panels
    pub uses_shared:   bool,              // SharedBuffer zero-copy path
    pub proto_version: ProtoVersion,
    pub last_frame_ms: u64,               // wall-clock of last begin_frame
}

#[derive(Copy, Clone, Debug)]
pub struct ScreenRect {
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
}

#[derive(Copy, Clone, Eq, PartialEq, Debug)]
pub enum ProtoVersion { V1, V2 }

/// Registry of every live window.  Stable across vsyncs.
pub struct WindowRegistry {
    windows:      HashMap<IidKey, WindowRecord>,
    z_order_cache: Vec<IidKey>,    // sorted ascending by z_order, refreshed on lifecycle
    key_window:   Option<IidKey>,
    front_window: Option<IidKey>,
}

static REGISTRY: OnceLock<Mutex<WindowRegistry>> = OnceLock::new();

pub fn registry() -> &'static Mutex<WindowRegistry> {
    REGISTRY.get_or_init(|| Mutex::new(WindowRegistry::new()))
}

impl WindowRegistry {
    pub fn new() -> Self {
        Self {
            windows: HashMap::new(),
            z_order_cache: Vec::new(),
            key_window: None,
            front_window: None,
        }
    }

    pub fn create_window(&mut self, owner_app: String, title: String,
                         bounds: ScreenRect, proto_version: ProtoVersion,
                         uses_shared: bool) -> IidKey {
        let iid = IidKey::new();
        let surface = Surface::new(bounds.w, bounds.h);
        let z = self.next_top_z();
        let rec = WindowRecord {
            iid, owner_app, title, bounds, z_order: z,
            state: WindowState::Materialising,
            surface,
            global_alpha: 255,
            damage: DamageRect { x: 0, y: 0, w: bounds.w, h: bounds.h },
            is_key: true, is_front: true,
            uses_shared, proto_version,
            last_frame_ms: 0,
        };
        self.windows.insert(iid, rec);
        self.set_key(iid);
        self.set_front(iid);
        self.rebuild_z_cache();
        iid
    }

    /// Iterate visible windows back-to-front for compositing.
    pub fn iter_back_to_front(&self) -> impl Iterator<Item = &WindowRecord> {
        self.z_order_cache.iter()
            .filter_map(move |iid| self.windows.get(iid))
            .filter(|w| matches!(w.state,
                WindowState::Materialising
              | WindowState::Visible
              | WindowState::Dematerialising))
    }

    pub fn get_mut(&mut self, iid: IidKey) -> Option<&mut WindowRecord> {
        self.windows.get_mut(&iid)
    }

    pub fn set_key(&mut self, iid: IidKey) { /* clears prev, sets new */ }
    pub fn set_front(&mut self, iid: IidKey) { /* raises to top z */ }
    pub fn raise(&mut self, iid: IidKey) { /* z_order = next_top_z() */ }
    pub fn close(&mut self, iid: IidKey) { /* transitions to Dematerialising */ }
    pub fn gc_closed(&mut self) { /* drops surfaces for Closed windows */ }

    fn next_top_z(&self) -> i32 {
        self.windows.values().map(|w| w.z_order).max().unwrap_or(0) + 1
    }

    fn rebuild_z_cache(&mut self) {
        let mut v: Vec<_> = self.windows.keys().copied().collect();
        v.sort_by_key(|k| self.windows[k].z_order);
        self.z_order_cache = v;
    }
}
```

### 2.1 Why a Mutex around the whole registry?

The compositor pass holds the registry mutex for the duration of one frame (~3–8 ms). During the pass, app parser threads that want to mutate their Surface or push a damage rectangle must wait. This is acceptable because:
- Parser threads are I/O bound (they're blocked on the pipe from the WASM app most of the time)
- The compositor pass is short and bounded (we'll prove this in the perf round R29)
- The 16.6 ms vsync interval guarantees a parser-side wait of at most ~10 ms in the worst case

For v2 we may upgrade to a per-window mutex (or RCU-style swap-pointer) but v1 keeps the design simple.

### 2.2 Why not store the Surface separately from WindowRecord?

We considered a side table (`HashMap<IidKey, Surface>`) so that the registry lock could be released before the slow Surface blits. Rejected because:
- The compositor must check `state`, `z_order`, `bounds`, `damage`, *and* `surface` for every window during the same pass — splitting them invites torn reads.
- A reader-writer lock (`RwLock`) lets parser threads read window metadata concurrently with compositor pass without contention.

We accept a single `Mutex` for v1 and revisit in R29.

---

## 3. VYOMA_DRAW Protocol v2

### 3.1 Compatibility with v1

V1 commands continue to work unchanged for every existing app. New v2 commands are gated on a version handshake at app startup:

```
APP  -> SUP : VYOMA_DRAW:hello:proto=2,features=window,damage,opacity,shared
SUP  -> APP : (via callback on stdin) VYOMA_DISPLAY_READY:proto=2,w=1280,h=720,scale=1
```

V1 apps that never send `hello:` are implicitly version 1: the compositor synthesises a "default window" sized to the full screen for them.

### 3.2 New commands in v2

| Command | Direction | Purpose |
|---------|-----------|---------|
| `VYOMA_DRAW:hello:<key=value,...>` | app → sup | Protocol/feature negotiation |
| `VYOMA_DRAW:create_window:<iid_local>,<x>,<y>,<w>,<h>,<title>` | app → sup | Register a new window |
| `VYOMA_DRAW:select_window:<iid_local>` | app → sup | Subsequent draw commands target this window |
| `VYOMA_DRAW:resize_window:<w>,<h>` | app → sup | Request resize on current window |
| `VYOMA_DRAW:move_window:<x>,<y>` | app → sup | Request move on current window |
| `VYOMA_DRAW:set_opacity:<0-255>` | app → sup | Window global alpha |
| `VYOMA_DRAW:begin_frame` | app → sup | Start a frame on current window |
| `VYOMA_DRAW:damage_rect:<x>,<y>,<w>,<h>` | app → sup | Mark dirty region (window-local) |
| `VYOMA_DRAW:end_frame` | app → sup | Frame complete; compositor may flip |
| `VYOMA_DRAW:close_window` | app → sup | Begin close animation |
| `VYOMA_DRAW:draw_text_scaled:<x>,<y>,<rgba>,<px>,<text>` | app → sup | Variable-size text (forwards to font system R13) |

The bare `VYOMA_DRAW:flush` from v1 becomes equivalent to `begin_frame` + `damage_rect:0,0,W,H` + `end_frame` (i.e., dirties everything).

### 3.3 Deferred to v3

- `present_layer:<layer_id>` — multi-layer compositing (Core-Animation-style sublayers). Out of v1 scope; designed in R12 (Animations) using the existing `animator.rs` machinery.
- Hardware overlay plane assignment (DRM KMS planes) — requires GPU work, planned for R29 perf push.
- GPU-accelerated text — depends on R13 font system.
- Color profile / HDR — explicitly skipped (see §13).

### 3.4 Parser state machine

```rust
// supervisor/src/display/protocol_v2.rs

#[derive(Debug)]
pub enum ProtoCmd {
    Hello       { proto: u8, features: Vec<String> },
    CreateWin   { local_id: u32, x: i32, y: i32, w: u32, h: u32, title: String },
    SelectWin   { local_id: u32 },
    ResizeWin   { w: u32, h: u32 },
    MoveWin     { x: i32, y: i32 },
    SetOpacity  { alpha: u8 },
    BeginFrame,
    DamageRect  { x: u32, y: u32, w: u32, h: u32 },
    EndFrame,
    CloseWin,
    FillRect    { x: u32, y: u32, w: u32, h: u32, rgba: u32 },
    DrawText    { x: u32, y: u32, rgba: u32, size: TextSize, text: String },
    DrawTextWrap{ x: u32, y: u32, max_w: u32, rgba: u32, size: TextSize, text: String },
    DrawTextPx  { x: u32, y: u32, rgba: u32, px: u32, text: String },
    Flush,                        // v1 fallback
}

#[derive(Copy, Clone, Debug)]
pub enum TextSize { Small, Medium, Large }

pub struct AppParserState {
    pub proto:        ProtoVersion,
    pub current_iid:  Option<IidKey>,
    pub local_to_iid: HashMap<u32, IidKey>,  // app's local handle → registry key
    pub default_iid:  Option<IidKey>,        // synthesised window for v1 apps
}

/// Parse a single line.  Malformed lines return Err; caller logs and continues.
pub fn parse_line(line: &str) -> Result<ProtoCmd, ProtoError> {
    let stripped = line.strip_prefix("VYOMA_DRAW:")
        .ok_or(ProtoError::NotADrawCmd)?;
    let (verb, rest) = stripped.split_once(':')
        .unwrap_or((stripped, ""));
    match verb {
        "hello"        => parse_hello(rest),
        "create_window"=> parse_create(rest),
        "select_window"=> parse_select(rest),
        // ... etc
        "fill_rect"    => parse_fill_rect(rest),
        "draw_text"    => parse_draw_text(rest),
        "draw_text_wrap"=> parse_draw_text_wrap(rest),
        "draw_text_scaled"=> parse_draw_text_scaled(rest),
        "flush"        => Ok(ProtoCmd::Flush),
        other          => Err(ProtoError::UnknownVerb(other.to_string())),
    }
}

#[derive(Debug)]
pub enum ProtoError {
    NotADrawCmd,
    UnknownVerb(String),
    BadArgs(&'static str),
    NotInFrame,           // damage_rect/end_frame outside begin_frame
    NoCurrentWindow,
    WindowNotFound(u32),
}
```

### 3.5 Error policy

Malformed commands NEVER kill the app. Specification:
1. Parser logs at `WARN`: `display: app=<name> line=<n> err=<msg> raw=<line>`
2. Parser increments a per-app `bad_lines_total` counter
3. If `bad_lines_total > 1024` in a 60-second window, supervisor sends a structured stderr message and disables that app's display capability for the rest of its lifetime (the app keeps running, just stops getting pixels).

This matches the "supervisor never crashes apps for protocol misbehaviour" principle from R3 and the manifest validation discipline from R8.

---

## 4. Compositor pipeline

### 4.1 Per-frame steps (compositor thread)

```rust
// supervisor/src/display/compositor_pass.rs

pub struct CompositorPass {
    fb: &'static Mutex<Framebuffer>,
    reg: &'static Mutex<WindowRegistry>,
    last_composite_ms: u64,
    fps_counter: FpsCounter,
}

impl CompositorPass {
    pub fn new() -> Self {
        Self {
            fb: display::get().expect("display not initialised"),
            reg: window_registry::registry(),
            last_composite_ms: 0,
            fps_counter: FpsCounter::new(),
        }
    }

    /// One full composite pass.  Returns elapsed micros for thermal feedback.
    pub fn run_once(&mut self) -> u64 {
        let t0 = monotonic_us();
        let mut fb  = self.fb.lock().unwrap();
        let mut reg = self.reg.lock().unwrap();

        // 1. Collect per-window damage in screen coords; compute screen-union.
        let screen_damage = self.collect_screen_damage(&reg, fb.width, fb.height);
        if screen_damage.is_empty() && !reg.cursor_moved_since_last_frame() {
            return monotonic_us() - t0; // nothing to do
        }

        // 2. Restore pixels under cursor so its trail does not become permanent.
        fb.restore_under_cursor();

        // 3. Clear screen-damage region on the FB back buffer to the wallpaper colour.
        self.paint_background(&mut fb, &screen_damage);

        // 4. Composite each visible window in Z-order, clipped to its intersection
        //    with screen-damage.
        for win in reg.iter_back_to_front() {
            self.composite_window(&mut fb, win, &screen_damage);
        }

        // 5. Composite system chrome (menu bar, dock) — owned by Window Manager (R21).
        //    For v1 we emit a stub call here.
        crate::chrome::composite_global(&mut fb, &screen_damage);

        // 6. Paint cursor sprite on top.
        fb.draw_cursor();

        // 7. Flush dirty rows to /dev/fb0.
        fb.flush_region(&screen_damage);

        // 8. Clear per-window damage; mark composited windows as having shown a frame.
        for win in reg.iter_back_to_front_mut() {
            win.damage = DamageRect::empty();
        }

        let elapsed = monotonic_us() - t0;
        self.fps_counter.tick(elapsed);
        elapsed
    }

    fn collect_screen_damage(&self, reg: &WindowRegistry, fb_w: u32, fb_h: u32)
                             -> DamageRect {
        let mut acc = DamageRect::empty();
        for win in reg.iter_back_to_front() {
            if win.damage.is_empty() { continue; }
            // Transform window-local damage into screen coords.
            let sx = win.bounds.x + win.damage.x as i32;
            let sy = win.bounds.y + win.damage.y as i32;
            let sw = win.damage.w;
            let sh = win.damage.h;
            let screen_rect = clip_rect_to_screen(sx, sy, sw, sh, fb_w, fb_h);
            acc = acc.union(screen_rect);
        }
        acc
    }

    fn composite_window(&self, fb: &mut Framebuffer, win: &WindowRecord,
                        screen_damage: &DamageRect) {
        if matches!(win.state, WindowState::Minimised | WindowState::Closed) { return; }
        if win.global_alpha == 0 { return; }
        // Skip if window doesn't intersect screen damage.
        if !rect_intersects(&win.bounds, screen_damage) { return; }
        // Occlusion: skip if a higher-z opaque window completely covers us
        // (computed once per composite, see §4.4).
        // ...
        let dx = win.bounds.x.max(0) as u32;
        let dy = win.bounds.y.max(0) as u32;
        surface::blit_surface(
            &mut fb.back, &win.surface,
            dx, dy, win.global_alpha,
            fb.stride, fb.width, fb.height,
        );
    }
}
```

### 4.2 Damage tracking

Two layers of damage:
- **Window-local damage**: accumulated per-window between `begin_frame` and `end_frame` (or implicit between flushes for v1 apps). Stored in `WindowRecord.damage`.
- **Screen damage**: per-frame union of all window-local damage transformed into screen coordinates. Computed on each composite pass.

Damage is conservative — better to over-redraw than miss pixels. Damage may be reduced by the optional `damage_rect` hint from the app; if the app never emits `damage_rect`, the supervisor falls back to dirtying the entire window surface (matching v1 behaviour).

### 4.3 Z-order blit

For each visible window in z-order ascending (back-to-front):
1. Skip if window state is Minimised or Closed.
2. Skip if `global_alpha == 0`.
3. Skip if window bounds do not intersect the current screen-damage rectangle.
4. Otherwise, `blit_surface(fb.back, &win.surface, dx, dy, win.global_alpha, ...)`.

The existing `surface::blit_surface` is used unchanged — opaque windows hit the row-level memcpy fast path; semi-transparent windows hit the per-pixel blend path.

### 4.4 Occlusion culling

If two opaque windows W1 (back) and W2 (front) cover the exact same region, blitting W1 is wasted work. The optimisation:

```rust
fn build_occlusion_mask(reg: &WindowRegistry, screen_damage: DamageRect)
                        -> OcclusionMask {
    // Walk windows front-to-back: each opaque window subtracts its
    // intersection-with-screen-damage from a remaining damage rectangle set.
    // Result: for each window, the visible (uncovered) sub-rectangles.
    // For v1 we use a single bounding rect per window; v2 may use a region.
}
```

Implementation note: a true region (set of non-overlapping rectangles) is complex. For v1 we ship a "single-rect occlusion": if a higher-z window with `global_alpha == 255` and an opaque-content surface completely covers a lower-z window, the lower window is skipped entirely. This catches the common case (Finder fully behind Safari) without the complexity of region algebra. Region algebra arrives in R29 perf round.

### 4.5 Frame budget

Target: 16.66 ms (60 Hz) per composite pass, with a hard ceiling of 12 ms reserved for compositing alone (leaving 4 ms for vsync IRQ latency, mutex acquire, and the FB flush memcpy).

Budget breakdown on a 1280×720 desktop, 5 visible windows averaging 600×400:
- Mutex acquire (FB + REG): ~20 µs
- Damage collection (5 windows): ~5 µs
- Background paint of damage union: ~600 µs (200 KB at ~3 GB/s)
- Blit 5 windows: ~5 × 1 ms = 5 ms (semi-realistic; opaque fast path: 240 KB each)
- Chrome composite: ~500 µs
- Cursor restore + draw: ~50 µs
- FB flush memcpy: ~1.5 ms (worst case: full screen, 3.5 MB at 3 GB/s)
- **Total worst case: ~8 ms** — comfortably under budget at 60 Hz.

Sub-budget on `mcu-minimal`: target 15 Hz (66 ms) with single window. Compositor still meets target because there's only one tiny surface.

### 4.6 Frame drop policy

If `run_once` exceeds 14 ms three frames in a row, the compositor logs `display: thermal-degraded` and signals ThermalGovernor (R7) to bump the throttle tier. The compositor halves its frame rate (60 → 30 → 15 Hz) by vsync skipping until elapsed budget recovers below 8 ms three frames in a row.

---

## 5. SharedBuffer zero-copy path

### 5.1 Motivation

Video players, games, and any app that fills its window each frame doesn't benefit from a per-pixel parser. The current path is:
```
WASM app → println! → pipe → parser thread → parse u32 RGBA → write into Surface
```
That's ~50 ns per pixel just from parse overhead. For a 1280×720 surface at 60 Hz, that's 2.7 ms of CPU per frame just parsing text — bigger than our entire composite budget.

The SharedBuffer path:
```
WASM app linear memory pages 0..N == Surface.buf  (mmap'd, no copy)
WASM app writes BGRA bytes directly to memory
WASM app sends 1-byte signal: VYOMA_DRAW:swap (4 bytes through the pipe)
Compositor reads from the OTHER half of the double-buffer
```

### 5.2 Wasmtime mechanism

`wasmtime::SharedMemory` already allows host-shared linear memory. The trick:

```rust
// supervisor/src/runtime/wasmtime_shared.rs

use wasmtime::{Memory, SharedMemory, Store};

pub fn setup_shared_surface(
    engine: &wasmtime::Engine,
    surface_back: Arc<Mutex<Vec<u8>>>,    // alias of WindowRecord.surface.buf
    width: u32,
    height: u32,
) -> anyhow::Result<SharedMemory> {
    let nbytes = (width * height * 4) as u64;
    let npages = ((nbytes + 65535) / 65536).max(1);
    let mem_ty = wasmtime::MemoryType::shared(npages as u32, npages as u32);
    let mem = SharedMemory::new(engine, mem_ty)?;
    // Initial fill from surface (e.g., for restore-on-resize).
    {
        let surf = surface_back.lock().unwrap();
        unsafe {
            std::ptr::copy_nonoverlapping(
                surf.as_ptr(),
                mem.data().as_ptr() as *mut u8,
                surf.len().min(nbytes as usize));
        }
    }
    Ok(mem)
}
```

The Surface's `buf` is the *same backing store* as the SharedMemory's data slice. When the app writes pixel bytes to `wasm_memory[42]`, the compositor reads the same byte from `surface.buf[42]`. No syscalls, no copies.

### 5.3 Double-buffering

We allocate **two** Surfaces per shared-buffer window: a *front* (read by compositor) and a *back* (written by app). The app writes pixels into the back; on completion it emits `VYOMA_DRAW:swap`; the supervisor swaps the buffer pointers atomically (under registry lock). The compositor on its next pass reads from the new front.

```rust
pub struct SharedSurface {
    pub front: SharedMemory,   // compositor reads
    pub back:  SharedMemory,   // app writes
    pub width: u32,
    pub height: u32,
    pub front_idx: AtomicU8,   // 0 or 1; flipped on swap
}

impl SharedSurface {
    pub fn swap(&mut self) {
        std::mem::swap(&mut self.front, &mut self.back);
        self.front_idx.fetch_xor(1, Ordering::AcqRel);
    }
}
```

### 5.4 Signal mechanism

Three options considered:
1. **`VYOMA_DRAW:swap` line** through the existing pipe. Latency: ~30 µs. Simple. **Chosen for v1.**
2. **WASI futex on a shared atomic word.** Latency: <1 µs. Requires WASI Preview 3. **Defer.**
3. **Wasmtime hostcall.** Latency: ~3 µs. Requires custom WIT. **Defer to v2.**

The 30 µs of `swap` latency is invisible at 60 Hz (16.6 ms frame budget) and matches macOS's IOSurface flip latency.

### 5.5 Capability gating

Only apps that declare both capabilities get a SharedBuffer:

```toml
[capabilities]
display       = true
shared_buffer = true   # New in v2
```

The manifest validator enforces this. Apps without `shared_buffer = true` fall back to the parser path automatically — no opt-in change in source needed.

### 5.6 Security model

The shared memory is bidirectional but trust is asymmetric:
- The app can corrupt its own window's pixels (which it could already do via fill_rect).
- The app **cannot** read other apps' pixels — its SharedMemory only maps its own Surface, never the framebuffer.
- The app **cannot** read the framebuffer — only the supervisor's compositor thread has /dev/fb0 mapped.
- Therefore zero-copy preserves the capability boundary; WASM linear memory remains the trust boundary, exactly as before.

### 5.7 Memory budget

A 1280×720 RGBA surface is 3.5 MB. Double-buffered: 7 MB per window. On a desktop with 8 GB RAM and 16 concurrent shared-buffer windows: 112 MB. On `mcu-minimal` (128 KB RAM ceiling): SharedBuffer is unsupported (manifest validator rejects `shared_buffer = true` on that platform).

---

## 6. Multi-window Z-order, occlusion, and AppState extension

### 6.1 AppState extension

Existing `AppState` has a `surface: Option<Surface>` field. We deprecate that in favour of:

```rust
// supervisor/src/app_state.rs (edit)

pub struct AppState {
    pub name: String,
    // ... existing fields ...

    /// Windows owned by this app, indexed by app-local handle.
    pub windows: HashMap<u32, IidKey>,
    /// If true, this app uses the SharedBuffer path for all its windows.
    pub uses_shared_buffer: bool,
}
```

Windows are owned by the `WindowRegistry` (not by `AppState`) so that a crashed parser thread doesn't lose pixels. `AppState.windows` is just a lookup table from the app's local `iid_local: u32` (its own handle space) to the global `IidKey`. When the app exits, the supervisor closes all its registered windows; when individual windows close (via `close_window`), they're removed from the local lookup.

### 6.2 Z-order policy

- Newly created windows go to the top (highest Z).
- `select_window` does NOT raise — apps can update background windows.
- `move_window` does NOT raise — programmatic moves shouldn't steal focus.
- Window Manager (R21) issues `RegistryCmd::Raise(iid)` in response to user input (click, ⌘Tab, dock click).
- A close animation keeps the window in the registry until the animation completes; visible during animation, then GC'd.

### 6.3 Damage on Z-order change

When a window is raised or lowered, all windows previously occluded by it (and now revealed) need their full visible area added to screen damage. We compute this by:
1. Snapshot bounds of the raised/lowered window.
2. For every other window, if its bounds intersect that rectangle and its Z-order put it behind the moved window, mark its damage as full-window.

For v1 this is a coarse "if a window's Z changes, mark all overlapping windows fully dirty". R29 will refine.

### 6.4 Occlusion culling implementation

```rust
// supervisor/src/display/compositor_pass.rs (continued)

struct Occluded { iid: IidKey, fully_covered: bool }

fn compute_occlusion(reg: &WindowRegistry, screen_damage: DamageRect)
                     -> Vec<Occluded> {
    // Walk front-to-back; track unobscured area as a single rect for v1.
    let mut unobscured = screen_damage;
    let mut result = Vec::new();
    let front_to_back: Vec<_> = reg.iter_back_to_front().collect();
    for win in front_to_back.iter().rev() {
        if unobscured.is_empty() {
            // Everything below is fully covered.
            result.push(Occluded { iid: win.iid, fully_covered: true });
            continue;
        }
        let win_screen = ScreenRect {
            x: win.bounds.x, y: win.bounds.y, w: win.bounds.w, h: win.bounds.h,
        };
        if win.global_alpha == 255 && surface_is_opaque(win) {
            // This opaque window subtracts from the remaining unobscured rect.
            unobscured = unobscured.subtract_rect(&win_screen);
        }
        result.push(Occluded { iid: win.iid, fully_covered: false });
    }
    result
}
```

`DamageRect::subtract_rect` for v1 is conservative: it returns the original rect unchanged unless the subtraction produces another rectangle. This is sufficient for the common case (overlapping centered windows of similar size) and avoids region algebra.

### 6.5 Window lifecycle FSM

```
Materialising ──(open anim end)──> Visible
Visible       ──(minimise)─────────> Minimised
Minimised     ──(restore)──────────> Visible
Visible       ──(close)────────────> Dematerialising
Dematerialising ──(close anim end)──> Closed
Closed        ──(gc tick)──────────> (dropped)
```

Transitions are issued either by VYOMA_DRAW commands (app-driven), by the Window Manager (R21), or by the Animator (R12) at animation completion.

---

## 7. Display protocol parsing and thread model

### 7.1 Current model

`supervisor/src/app_threads.rs` (existing) already spawns one reader thread per app to drain its stdout. The current parser hands lines off to a `display::handle_line` function. We extend this with v2 dispatch but keep the threading unchanged.

### 7.2 Per-app parser state

Each parser thread owns its `AppParserState`. When it sees a non-display line (e.g. `@target: msg`), it forwards to IPC routing as today. When it sees `VYOMA_DRAW:`, it parses, then dispatches:

```rust
// supervisor/src/display/dispatch.rs

pub fn dispatch(
    state: &mut AppParserState,
    app_name: &str,
    line: &str,
) {
    let cmd = match protocol_v2::parse_line(line) {
        Ok(c) => c,
        Err(e) => {
            log_protocol_error(app_name, &e, line);
            return;
        }
    };

    let reg = window_registry::registry();
    match cmd {
        ProtoCmd::Hello { proto, features } => {
            state.proto = if proto >= 2 { ProtoVersion::V2 } else { ProtoVersion::V1 };
            send_display_ready_callback(app_name, &features);
        }
        ProtoCmd::CreateWin { local_id, x, y, w, h, title } => {
            let mut r = reg.lock().unwrap();
            let iid = r.create_window(app_name.into(), title,
                ScreenRect { x, y, w, h }, state.proto, /*shared=*/false);
            state.local_to_iid.insert(local_id, iid);
            state.current_iid = Some(iid);
        }
        ProtoCmd::SelectWin { local_id } => {
            state.current_iid = state.local_to_iid.get(&local_id).copied();
        }
        ProtoCmd::BeginFrame => {
            // No-op for the registry; just clears damage if app doesn't emit damage_rect.
        }
        ProtoCmd::DamageRect { x, y, w, h } => {
            if let Some(iid) = state.current_iid {
                let mut r = reg.lock().unwrap();
                if let Some(win) = r.get_mut(iid) {
                    win.damage = win.damage.union(DamageRect { x, y, w, h });
                }
            }
        }
        ProtoCmd::EndFrame => {
            // Compositor picks up next vsync.
        }
        ProtoCmd::FillRect { x, y, w, h, rgba } => {
            // V1 compatibility: also apply to current window's Surface.
            let mut r = reg.lock().unwrap();
            let iid = state.current_iid.or(state.default_iid);
            if let Some(iid) = iid {
                if let Some(win) = r.get_mut(iid) {
                    win.surface.fill_rect(x, y, w, h, rgba);
                    win.damage = win.damage.union(DamageRect { x, y, w, h });
                }
            }
        }
        // ... other commands ...
        ProtoCmd::Flush => {
            // V1: mark entire current window damaged.
            let mut r = reg.lock().unwrap();
            let iid = state.current_iid.or(state.default_iid);
            if let Some(iid) = iid {
                if let Some(win) = r.get_mut(iid) {
                    win.damage = DamageRect { x: 0, y: 0, w: win.surface.width, h: win.surface.height };
                }
            }
        }
    }
}
```

### 7.3 Default-window synthesis for v1 apps

The first time a v1 app (one that hasn't sent `hello:`) emits any non-hello command, the dispatcher auto-creates a "default window" sized to the full screen and assigns it as `state.default_iid`. The app gets pixels on screen without knowing about v2.

### 7.4 Lock discipline

Parser threads hold the registry mutex only for the duration of one command's mutation (microseconds). Compositor thread holds it for the duration of a composite pass (milliseconds). Vsync source thread never touches the registry.

To prevent a parser thread starving the compositor:
- Parser holds registry mutex < 100 µs per command.
- A flood of commands from one app would still throttle the compositor. We mitigate by having `parse_line` operate on a `String` first (no lock), then a short critical section to apply. This is already how the code above is structured.

If the parser thread ever blocks for >5 ms holding the lock (debug-detected via `parking_lot` lock instrumentation in test builds), it's a bug to fix immediately.

---

## 8. VSync and frame scheduling

### 8.1 VSync source

The compositor needs a steady tick at 60 Hz (or whatever the panel refresh rate is). Two sources, picked at boot:

#### Source A: DRM page-flip events (preferred)

```rust
// supervisor/src/display/vsync.rs

use std::os::fd::AsRawFd;
use libc::{drm_event, DRM_EVENT_VBLANK};

pub struct DrmVsyncSource {
    fd: std::os::fd::RawFd,
}

impl DrmVsyncSource {
    pub fn open() -> std::io::Result<Self> {
        // Open /dev/dri/card0 (DRM master claimed by supervisor at BootPhase::Display).
        let dri = std::fs::OpenOptions::new().read(true).write(true).open("/dev/dri/card0")?;
        let fd = dri.as_raw_fd();
        // ... ioctl DRM_IOCTL_MODE_PAGE_FLIP with DRM_MODE_PAGE_FLIP_EVENT ...
        Ok(Self { fd })
    }
    /// Block until the next vblank arrives.  Returns the vblank timestamp.
    pub fn wait(&self) -> std::io::Result<u64> {
        let mut buf = [0u8; 32];
        let n = unsafe { libc::read(self.fd, buf.as_mut_ptr() as *mut _, buf.len()) };
        if n <= 0 { return Err(std::io::Error::last_os_error()); }
        let evt: &drm_event = unsafe { &*(buf.as_ptr() as *const drm_event) };
        if evt.type_ == DRM_EVENT_VBLANK {
            // Parse the tv_sec/tv_usec from the rest of the event.
            Ok(decode_vblank_us(&buf))
        } else {
            self.wait()
        }
    }
}
```

#### Source B: Monotonic timer (fallback)

For headless or non-DRM virtio paths:

```rust
pub struct TimerVsyncSource {
    period_us: u64,
    next_us: u64,
}

impl TimerVsyncSource {
    pub fn new(refresh_hz: u32) -> Self {
        Self {
            period_us: 1_000_000 / refresh_hz as u64,
            next_us: monotonic_us() + 1_000_000 / refresh_hz as u64,
        }
    }
    pub fn wait(&mut self) -> u64 {
        let now = monotonic_us();
        if now < self.next_us {
            std::thread::sleep(std::time::Duration::from_micros(self.next_us - now));
        }
        self.next_us = self.next_us.max(now) + self.period_us;
        monotonic_us()
    }
}
```

### 8.2 Unified Vsync trait

```rust
pub trait VsyncSource: Send {
    fn wait(&mut self) -> u64;       // returns monotonic-us of the vsync
    fn refresh_hz(&self) -> u32;     // 60 / 30 / 15 / ...
    fn skip_next(&mut self);         // halve frame rate for one frame (thermal)
}

pub enum AnyVsync {
    Drm(DrmVsyncSource),
    Timer(TimerVsyncSource),
}

impl VsyncSource for AnyVsync { /* dispatch */ }
```

### 8.3 Compositor thread main loop

```rust
// supervisor/src/display/frame_scheduler.rs

pub fn spawn_compositor_thread(qos: QosClass) -> std::thread::JoinHandle<()> {
    std::thread::Builder::new()
        .name("vyoma-compositor".into())
        .spawn(move || {
            crate::scheduler::set_thread_qos(qos);  // R5: UserInteractive
            let mut pass = CompositorPass::new();
            let mut vsync: AnyVsync = open_vsync_source();
            let mut thermal_skip_mask = 0u32;
            loop {
                let t_vsync = vsync.wait();
                if should_skip(thermal_skip_mask) { continue; }
                let elapsed_us = pass.run_once();
                thermal_skip_mask = update_thermal_decision(thermal_skip_mask, elapsed_us);
            }
        })
        .expect("spawn compositor")
}
```

### 8.4 Vsync source thread separate from compositor thread?

Considered: a dedicated vsync source thread that just reads the DRM fd and pings a condvar that wakes the compositor. Rejected for v1 because:
- DRM read blocks anyway, equivalent to the compositor thread blocking.
- Adds a context switch per frame (~5 µs) for no benefit.
- One fewer thread to QoS-tag.

If vsync source ever needs to be at a different QoS (e.g. Realtime while compositor is UserInteractive), we'll split. For now, single thread.

### 8.5 R10 integration: vsync as a timerfd event

If R10 (interrupt subsystem) lands a `TimerLoop` that owns all timerfds, vsync can register as a TimerLoop callback:

```rust
// Future: instead of own thread, register with TimerLoop
timer_loop.subscribe_vsync(QosClass::UserInteractive, |t_us| {
    compositor::run_once();
});
```

For v1 we use a dedicated thread; we'll migrate to TimerLoop subscription in R29.

---

## 9. Thermal throttle integration

ThermalGovernor (R7) emits tiers 0/1/2. Compositor reaction:

| Tier | Compositor behaviour |
|------|----------------------|
| 0 (cool) | 60 Hz vsync, full composite quality |
| 1 (warm) | 30 Hz vsync (skip every other), no animation easing |
| 2 (hot)  | 15 Hz vsync (3 in 4 skipped), drop semi-transparent windows to opaque |

```rust
// supervisor/src/display/thermal.rs

pub fn thermal_listener(tier_rx: crossbeam_channel::Receiver<ThermalTier>) {
    while let Ok(tier) = tier_rx.recv() {
        let skip_mask = match tier {
            ThermalTier::Cool => 0u32,
            ThermalTier::Warm => 0b01,
            ThermalTier::Hot  => 0b1110,
        };
        THERMAL_SKIP_MASK.store(skip_mask, Ordering::Release);
    }
}

fn should_skip(_unused: u32) -> bool {
    static FRAME_IDX: AtomicU32 = AtomicU32::new(0);
    let mask = THERMAL_SKIP_MASK.load(Ordering::Acquire);
    let idx = FRAME_IDX.fetch_add(1, Ordering::Relaxed);
    (mask >> (idx & 0b11)) & 1 == 1
}

static THERMAL_SKIP_MASK: AtomicU32 = AtomicU32::new(0);
```

The compositor thread reads `THERMAL_SKIP_MASK` once per frame; switching tiers is lock-free.

---

## 10. Window chrome and decoration

### 10.1 Division of responsibility

| Subsystem | Owns |
|-----------|------|
| **Display Server** (R11, this round) | Compositor pass; per-window Surface allocation; the act of painting the titlebar bitmap into a window's Surface; cursor compositing |
| **Window Manager** (R21, future) | Window layout policy (tiling, free-floating); which window has focus; menubar contents; dock contents; close/min/max button bounds & hit-testing |
| **Chrome** (existing `chrome.rs`) | The bitmap design of titlebars, buttons, shadows |

Concretely: Display Server provides a `composite_chrome(win: &WindowRecord, fb: &mut Framebuffer)` helper that the Window Manager calls in its policy code. For v1, the chrome composite is invoked from inside the compositor pass directly (since there's no Window Manager yet).

### 10.2 Per-window chrome dimensions

```rust
pub const TITLEBAR_H:    u32 = 24;
pub const BUTTON_SIZE:   u32 = 12;
pub const BUTTON_GUTTER: u32 = 8;   // distance between buttons
pub const BORDER_W:      u32 = 1;

pub struct ChromeLayout {
    pub close_btn:    Rect,
    pub min_btn:      Rect,
    pub max_btn:      Rect,
    pub title_text:   Rect,
    pub content_area: Rect,
}

impl ChromeLayout {
    pub fn for_window(bounds: ScreenRect) -> Self { /* compute from bounds */ }
}
```

### 10.3 Drawing pass

```rust
pub fn composite_chrome(win: &WindowRecord, fb: &mut Framebuffer) {
    let layout = ChromeLayout::for_window(win.bounds);
    let bar_color = if win.is_key {
        helpers::titlebar_color(true)
    } else {
        helpers::titlebar_color(false)
    };
    fb.fill_rect(win.bounds.x as u32, win.bounds.y as u32,
                 win.bounds.w, TITLEBAR_H, bar_color);
    // Buttons...
    draw_button(fb, &layout.close_btn, button_color(ButtonKind::Close, win.is_key));
    draw_button(fb, &layout.min_btn,   button_color(ButtonKind::Min,   win.is_key));
    draw_button(fb, &layout.max_btn,   button_color(ButtonKind::Max,   win.is_key));
    // Title...
    fb.draw_text(layout.title_text.x, layout.title_text.y,
                 &win.title, 0xFFFFFFFF, font::FontSize::Medium);
}
```

Note: chrome is painted *into the framebuffer back-buffer* outside the window's Surface — surfaces only contain *content* pixels. This matches macOS: the window content layer and the titlebar are independent. App resize requests don't change the titlebar; they change the Surface and bounds, then the chrome pass repaints.

---

## 11. Font system interface

R13 (Font System & Typography) is a future round. For v1 we expose a minimal interface that R13 can swap in for fontdue-based scalable rendering.

```rust
// supervisor/src/display/font_iface.rs

pub trait FontProvider: Send + Sync {
    fn measure(&self, text: &str, size_px: u32) -> (u32, u32);
    fn render_into(&self, surface: &mut Surface, x: u32, y: u32,
                   text: &str, size_px: u32, rgba: u32);
}

/// V1 default: 8×16 bitmap font, only sizes {s, m, l}.
pub struct BitmapFontProvider;

impl FontProvider for BitmapFontProvider {
    fn measure(&self, text: &str, size_px: u32) -> (u32, u32) {
        let chars = text.chars().count() as u32;
        let (gw, gh) = match size_px {
            0..=10  => (font::glyph_dims(font::FontSize::Small)),
            11..=20 => (font::glyph_dims(font::FontSize::Medium)),
            _       => (font::glyph_dims(font::FontSize::Large)),
        };
        (gw * chars, gh)
    }
    fn render_into(&self, surface: &mut Surface, x: u32, y: u32,
                   text: &str, _size_px: u32, rgba: u32) {
        surface.draw_text_bitmap(x, y, text, rgba);
    }
}
```

R13 will replace `BitmapFontProvider` with a `FontduFontProvider` wired to `composite_glyph` (already in compositor.rs). The trait stays stable; the implementation swaps. This delivers `draw_text_scaled` for v2 without re-architecting the display pipeline.

---

## 12. Implementation files and line budgets

### 12.1 Existing files (extend)

| File | Existing lines | New lines | Final | Notes |
|------|----------------|-----------|-------|-------|
| `display/mod.rs` | 500 | ~50 | ~550 — **must split** | Add `flush_region`, public registry handle, vsync init |
| `display/compositor.rs` | 314 | unchanged | 314 | Used by Surface and compositor_pass |
| `display/surface.rs` | 168 | ~40 | ~208 | Add SharedSurface, swap logic |
| `display/cursor.rs` | 88 | unchanged | 88 | |
| `display/animator.rs` | 88 | unchanged | 88 | |
| `display/fb_ioctl.rs` | 48 | ~30 | ~78 | Add DRM page-flip ioctls |
| `display/helpers.rs` | (existing) | ~20 | <500 | Add `clip_rect_to_screen`, `rect_intersects` |

`display/mod.rs` is already at 500 — adding any new lines violates the 500-line rule. We split:

```
display/mod.rs            (200 lines)  — module reexports + init + get + screen_size
display/framebuffer.rs    (300 lines)  — Framebuffer struct, drawing primitives
display/fb_flush.rs       (150 lines)  — flush + flush_region + cursor restore
```

### 12.2 New files

| File | Approx lines | Purpose |
|------|--------------|---------|
| `display/window_registry.rs` | ~350 | `IidKey`, `WindowRecord`, `WindowRegistry`, `DamageRect`, `ScreenRect`, lifecycle FSM |
| `display/protocol_v2.rs` | ~400 | `ProtoCmd`, `ProtoError`, `parse_line` and per-verb parsers |
| `display/dispatch.rs` | ~250 | `AppParserState`, `dispatch()` — applies parsed commands to registry/surface |
| `display/compositor_pass.rs` | ~400 | `CompositorPass`, damage collection, Z-order blit, occlusion, chrome composite, FpsCounter |
| `display/vsync.rs` | ~200 | `VsyncSource` trait, `DrmVsyncSource`, `TimerVsyncSource`, `AnyVsync` |
| `display/frame_scheduler.rs` | ~150 | `spawn_compositor_thread`, vsync skip logic, frame-budget telemetry |
| `display/thermal.rs` | ~80 | Thermal tier listener, skip-mask atomic |
| `display/chrome_compose.rs` | ~150 | `ChromeLayout`, `composite_chrome`, `draw_button` |
| `display/font_iface.rs` | ~80 | `FontProvider` trait, `BitmapFontProvider` |
| `display/shared_surface.rs` | ~200 | `SharedSurface`, double-buffer swap, Wasmtime SharedMemory wiring |

Total: 10 new files, ~2,260 lines. Largest single file: 400 lines (`compositor_pass.rs`, `protocol_v2.rs`). All under 500.

### 12.3 Public module surface

```rust
// supervisor/src/display/mod.rs (after split)

mod cursor;
mod fb_ioctl;
mod helpers;
mod compositor;
mod framebuffer;
mod fb_flush;
mod window_registry;
mod protocol_v2;
mod dispatch;
mod compositor_pass;
mod vsync;
mod frame_scheduler;
mod thermal;
mod chrome_compose;
mod font_iface;
mod shared_surface;
pub mod animator;
pub mod surface;

pub use framebuffer::Framebuffer;
pub use cursor::{CursorState, CURSOR_W, CURSOR_H, CURSOR_MASK};
pub use window_registry::{registry, WindowRegistry, WindowRecord, IidKey,
                          DamageRect, ScreenRect, WindowState, ProtoVersion};
pub use dispatch::{AppParserState, dispatch};
pub use frame_scheduler::spawn_compositor_thread;
pub use thermal::set_thermal_tier;
pub use font_iface::{FontProvider, BitmapFontProvider};
```

`init`, `get`, `screen_size`, `set_cursor_pos`, `enable_cursor` stay in `mod.rs` as before.

---

## 13. Interaction with prior rounds

### 13.1 R1 — WIT callbacks

`on-display-ready` callback fires once after BootPhase::Display completes. For v2 apps it carries the negotiated protocol version and the screen size:

```wit
interface display {
    record display-info {
        width: u32,
        height: u32,
        refresh-hz: u32,
        proto-version: u8,
        scale-factor: f32,
    }
    on-display-ready: func(info: display-info);
}
```

### 13.2 R2 — SharedBuffer zero-copy

Covered in §5. The `shared_buffer = true` capability lights up the SharedSurface path. Wasmtime's `SharedMemory` is the underlying mechanism. The app sees no API change in its WIT bindings; the difference is invisible except for higher throughput.

### 13.3 R5 — QoS classes

Compositor thread runs at `QosClass::UserInteractive` (SCHED_FIFO, priority 80). Parser threads run at `QosClass::Default` (SCHED_OTHER, nice 0). The vsync source thread runs at `QosClass::Realtime` (SCHED_FIFO, priority 95) — because missing a vblank is unrecoverable.

Per R5's policy of inheriting QoS from caller intent: the compositor explicitly raises its own QoS on spawn rather than inheriting from main, since main runs at Default.

### 13.4 R6 — DRM/virtio-gpu driver

The DRM driver from R6 exposes `DrmDevice` with `claim_master()`, `set_mode()`, `page_flip()` methods. The Display Server calls `claim_master()` during BootPhase::Display, sets a mode matching the panel's native resolution, and registers for page-flip events. On bare metal this hits the real DRM driver; under QEMU it hits the virtio-gpu DRM driver.

```rust
fn init_with_drm() -> std::io::Result<()> {
    let dev = crate::hal::display::open()?;       // R9 HAL trait
    dev.claim_master()?;
    let mode = dev.preferred_mode()?;
    dev.set_mode(mode)?;
    init();                                       // existing init from display/mod.rs
    Ok(())
}
```

### 13.5 R7 — ThermalGovernor

Covered in §9. The compositor subscribes to a `Receiver<ThermalTier>` from the governor. Frame rate scales 60/30/15 with tier 0/1/2.

### 13.6 R8 — BootPhase ordering

Display initialises in `BootPhase::Display`, which runs after `BootPhase::Hal` (so the HAL display driver is up) and before `BootPhase::Apps` (so no app spawns until pixels work):

```rust
BootPhase::Display => {
    display::init_with_drm()?;
    display::spawn_compositor_thread(QosClass::UserInteractive);
    display::set_thermal_tier(thermal::current_tier());
}
```

### 13.7 R9 — HAL DisplayDevice

```rust
// hal/display.rs (defined in R9)
pub trait DisplayDevice: Send + Sync {
    fn open(&self) -> Result<(), HalError>;
    fn dimensions(&self) -> (u32, u32);
    fn map_framebuffer(&self) -> Result<*mut u8, HalError>;
    fn claim_master(&self) -> Result<(), HalError>;
    fn preferred_mode(&self) -> Result<DisplayMode, HalError>;
    fn set_mode(&self, mode: DisplayMode) -> Result<(), HalError>;
    fn page_flip_event_fd(&self) -> Result<RawFd, HalError>;
}
```

The existing `open_fb()` in `display/mod.rs` becomes a `LinuxFbDisplayDevice` implementation of this trait. On bare metal a new `BareMetalDrmDisplayDevice` is selected; on QEMU we still hit `/dev/fb0` via the Linux DRM fbcon emulation.

### 13.8 R10 — Interrupt subsystem & callback dispatch

- **Vsync event:** Initially a dedicated thread (§8.4). After R10 ships, vsync subscribes to TimerLoop as a `TimerEvent::Vsync` source.
- **HPQ (high-priority queue):** The compositor flush is the canonical HPQ workload. R10's HPQ runner thread is the same compositor thread; the FpsCounter is the HPQ's perf instrumentation.
- **Input → display:** Mouse events from R10's InputLoop dirty the cursor sprite area; the compositor's next pass redraws.

### 13.9 R12 — Window animations

R12 will hook into `WindowState::Materialising` and `WindowState::Dematerialising`. The existing `animator.rs` already has the Animation struct. R12 adds:
- Per-frame sample of the animation, write the resulting scale/alpha into `WindowRecord.global_alpha` and a forthcoming `transform: Affine2D` field.
- Animation completion → state transition to Visible or Closed.

The Display Server stays out of animation policy; it just composites whatever Surface and alpha the registry says.

### 13.10 R13 — Font system

Covered in §11. Display Server depends on the `FontProvider` trait; R13 implements `FontduFontProvider`.

### 13.11 R21 — Window manager

Covered in §10.1. Window Manager is policy; Display Server is mechanism. They share the registry: Window Manager mutates `is_key`, `is_front`, `z_order`, `bounds`; Display Server reads them.

---

## 14. Testing strategy

### 14.1 Unit tests (in-process, no QEMU)

| File | Tests |
|------|-------|
| `tests/display_protocol_v2.rs` | parse every v2 command; reject malformed; v1 backward-compat |
| `tests/display_damage.rs` | DamageRect union/clip/subtract correctness |
| `tests/display_window_registry.rs` | create/raise/close lifecycle; Z-order ordering; key/front tracking |
| `tests/display_compositor_pass.rs` | given fixture registry + heap-backed FB, composite produces expected pixels |
| `tests/display_occlusion.rs` | fully-covered windows skipped; partial overlap composited |
| `tests/display_shared_surface.rs` | double-buffer swap, atomic flip |
| `tests/display_thermal.rs` | tier transitions adjust skip mask correctly |

All of these use `Framebuffer::new_for_test` (already exists) — no /dev/fb0 required.

### 14.2 Integration test: PPM screenshot diff

For each canonical scene (single window, two overlapping windows, semi-transparent overlay, cursor on chrome), run the compositor pass against a heap-backed Framebuffer, dump to PPM via the existing `Framebuffer::screenshot`, and compare against a golden PPM in `tests/golden/`.

```rust
#[test]
fn two_window_overlap_matches_golden() {
    let (mut fb, _) = Framebuffer::new_for_test(800, 600);
    let mut reg = WindowRegistry::new();
    let _ = reg.create_window("a".into(), "A".into(),
        ScreenRect { x: 100, y: 100, w: 400, h: 300 }, ProtoVersion::V2, false);
    let _ = reg.create_window("b".into(), "B".into(),
        ScreenRect { x: 200, y: 150, w: 400, h: 300 }, ProtoVersion::V2, false);
    let mut pass = CompositorPass::for_test(&mut fb, &mut reg);
    pass.run_once_test();
    fb.screenshot("/tmp/two_window.ppm").unwrap();
    assert_ppm_eq("/tmp/two_window.ppm", "tests/golden/two_window.ppm", /*tol=*/4);
}
```

### 14.3 Smoke test extension

Existing `make smoke` will be extended with:
- A `gui-test-app` that emits a deterministic v2 frame (background fill + text + small rect at known position) and exits.
- After boot, supervisor screenshots and compares to golden.

### 14.4 Benchmark target

A `make bench-compositor` target spins up the in-process compositor with 5 mock windows full of fill_rect commands and reports:
- Mean composite-pass duration
- p99 composite-pass duration
- Frame drop rate at 60 Hz

Targets: mean < 4 ms, p99 < 10 ms, frame drops < 0.1 % on the reference desktop.

---

## 15. Open questions for the Critic

1. **Single registry mutex vs per-window:** §2.1 picks a single Mutex for v1. Will this starve under 50+ window load? Should we go to RwLock or per-window lock in v1?

2. **DamageRect single bbox vs region:** §6.4 ships single-bounding-rect occlusion. A region-based implementation (set of non-overlapping rects) is ~300 lines of code and probably needed before we ship a real desktop with many windows. Should we bite that bullet now or defer?

3. **VYOMA_DRAW v2 over the same pipe as v1 vs separate channel:** §3 keeps everything on the existing stdout pipe. A separate Unix-domain socket (or WASI Preview 2 stream resource) would give us framed binary commands instead of text and avoid the parse cost. Cleaner but a new IPC channel.

4. **SharedBuffer's `swap` signal latency:** §5.4 picks the line-based `VYOMA_DRAW:swap` signal (~30 µs). For 60 Hz video this is fine but for 120/240 Hz games we'd want a futex. Do we need futex now or after R29?

5. **Compositor thread vs separate vsync thread:** §8.4 collapses to one thread for v1. Will we regret this on multi-display systems (R19) where two vsync sources need to fire at different rates? Probably yes — but is that v2 work?

6. **Occlusion with semi-transparent windows:** §6.4 only culls when the front window is fully opaque (`global_alpha == 255` AND content is opaque). For windows with rounded corners, the corners themselves are semi-transparent, defeating culling on every window. Should we treat "≥99 % opaque" as culled for v1?

7. **Chrome painting inside compositor pass:** §10 paints chrome into the FB back buffer outside of the Surface. This means a chrome-only repaint (e.g. focus change) still triggers a full window-area FB write. Should chrome live in its own Surface per window?

8. **What about resize?** The current design has the app issue `resize_window:<w>,<h>`, the supervisor allocates a fresh larger Surface, and the app then redraws. The window appears blank for one frame mid-resize. Should we keep the old surface visible until the first end_frame at the new size?

9. **What happens when an app dies mid-frame?** Parser thread exits cleanly; registry should treat all the app's windows as Dematerialising and run the close animation. Is the cleanup path robust? Should the watchdog (R4) kick this off explicitly or let GC handle it?

10. **Are we sure DRM page-flip events arrive on /dev/dri/card0 under virtio-gpu in QEMU?** I believe yes (verified by the existing virtio-gpu DRM driver tests), but worth checking before depending on it for vsync.

11. **Cursor compositing order vs chrome:** Currently cursor draws last (on top of everything including chrome). When dragging a window, the cursor stays above the titlebar — correct. When clicking a button, the cursor's sprite occludes the button briefly — also correct macOS behaviour. Confirm this is what we want.

12. **Multi-resolution / DPI scaling:** spec-045 (already in the tree) adds multi-resolution. How does that interact with our `scale-factor: f32` field in the WIT? Currently we always treat surfaces as 1×. The Critic should weigh in whether v1 should ship 1× only or include 2× support.

---

## 16. Summary

The Display Server & Compositor for VyomaOS v1 is an in-supervisor subsystem comprising:

- A **WindowRegistry** indexed by `IidKey`, holding `WindowRecord`s with bounds, Z-order, state, Surface, and damage rectangle. (§2)
- **VYOMA_DRAW Protocol v2** extending v1 with window lifecycle, damage tracking, opacity, frame delimiters, scaled text, and versioned negotiation — with full backward compatibility. (§3)
- A **dedicated compositor thread** at `QosClass::UserInteractive`, driven by a DRM page-flip vsync source (or fallback timer) at 60 Hz, that on each vblank: collects damage, paints background, blits windows back-to-front with occlusion culling, composites chrome, draws cursor, and flushes only the damaged region to /dev/fb0. (§4, §8)
- A **SharedBuffer zero-copy path** for high-throughput apps via Wasmtime's `SharedMemory`, with double-buffering and a `swap` signal — opt-in via the `shared_buffer = true` manifest capability. (§5)
- **Per-window damage tracking and occlusion culling** so that idle scenes cost near zero CPU and 100 % opaque-front windows fully cull the back. (§4.2, §6.4)
- **ThermalGovernor integration** that halves frame rate at tier 1 (warm) and quarters it at tier 2 (hot). (§9)
- **Clean separation from Window Manager (R21):** Display Server owns the composite pass; WM owns layout policy and focus. (§10)
- A **FontProvider trait** that R13 will fill in with fontdue; v1 ships the existing 8×16 bitmap font behind the same trait. (§11)
- **10 new files**, none over 500 lines, with one split of the existing `display/mod.rs` to keep it under budget. (§12)

The design preserves all of macOS WindowServer's user-visible behaviours (per-window surface, Z-order, alpha, focus, cursor, vsync-pacing) without paying for a separate process; the WASM trust boundary already gives us safety. We explicitly skip CoreAnimation layer trees, GPU compositor planes, multi-monitor, and HDR for v1 — all routed to later rounds (R12, R19, R29).

Awaiting Critic feedback on the 12 open questions in §15.

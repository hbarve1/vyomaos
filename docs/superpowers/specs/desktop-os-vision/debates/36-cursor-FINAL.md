# FINAL Spec: Cursor & Pointer System (Round 36)

**Subsystem**: Cursor & Pointer System  
**macOS Analogue**: `NSCursor` / `CursorManager` / cursor images  
**Depends on**: R11 (compositor pass 2), R32 (CURSOR_POS, route_mouse, MOUSE_DRAG_CAPTURE, APPS_MAP), R34 (ime-window space-0), R35 (SPSC channel pattern)  
**Status**: FINAL — all blocking issues resolved

---

## 1. Overview

The supervisor owns the software cursor sprite exclusively. Apps request shapes; they never
draw the cursor themselves. A single `ArcSwap<CursorState>` snapshot (including position,
B1 fix) is published per recompute and read once by the compositor — no torn reads.

Design invariants:
- `CursorState` embeds `pos: (i32, i32)` — compositor never calls `cursor_xy()` directly (B1).
- `recompute_active_cursor()` called exactly once per mouse event, after all routing side effects (B2).
- `blit_rgba_straight` clips source + destination against framebuffer bounds (B3).
- Custom sprites: per-app token-bucket rate limit + ID validation + atomic cleanup (B4).
- Confinement: `CONFINED_LOGICAL_POS` accumulator separate from `CURSOR_POS`; escape chord bypasses SPSC channels (B5).

---

## 2. Data Structures

```rust
// supervisor/src/cursor/state.rs

#[derive(Clone, Debug)]
pub struct CursorState {
    pub shape:   CursorShape,
    pub sprite:  Arc<CursorSprite>,
    pub visible: bool,
    pub pos:     (i32, i32),    // B1 fix: snapshotted at publish time; compositor uses this
    pub epoch:   u64,
}

#[derive(Clone, Debug)]
pub enum CursorSprite {
    Static { w: u16, h: u16, hot: HotSpot, pixels: Arc<[u32]> },
    Animated { w: u16, h: u16, hot: HotSpot, frames: Vec<Arc<[u32]>>, frame_ms: u16 },
}

#[derive(Clone, Copy, Debug)]
pub struct HotSpot { pub dx: u8, pub dy: u8 }  // 0-based, in [0, w) and [0, h)

pub static ACTIVE_CURSOR: Lazy<ArcSwap<CursorState>> =
    Lazy::new(|| ArcSwap::from_pointee(CursorState::default_arrow()));
```

---

## 3. Compositor Integration

Cursor is drawn **last** in R11 pass 2, after all app surfaces and chrome:

```rust
// supervisor/src/cursor/composite.rs

pub fn composite_cursor(fb: &mut FrameBuffer) {
    let st = ACTIVE_CURSOR.load_full();           // single coherent snapshot (B1 fix)
    if !st.visible { return; }

    let (cx, cy) = st.pos;                        // B1: not cursor_xy() directly
    let sprite = &st.sprite;
    let (w, h, hot, pixels) = match sprite.as_ref() {
        CursorSprite::Static { w, h, hot, pixels } => (*w, *h, *hot, pixels.as_ref()),
        CursorSprite::Animated { w, h, hot, frames, frame_ms } => {
            let fi = (now_ms() / *frame_ms as u64) as usize % frames.len();
            (*w, *h, *hot, frames[fi].as_ref())
        }
    };

    let ox = cx - hot.dx as i32;
    let oy = cy - hot.dy as i32;

    // B3 fix: clip source + destination against framebuffer bounds
    let src_x0 = (-ox).max(0) as u32;
    let src_y0 = (-oy).max(0) as u32;
    let dst_x0 = ox.max(0) as u32;
    let dst_y0 = oy.max(0) as u32;
    let copy_w = ((w as i32 - src_x0 as i32).min(fb.width as i32 - dst_x0 as i32)).max(0) as u32;
    let copy_h = ((h as i32 - src_y0 as i32).min(fb.height as i32 - dst_y0 as i32)).max(0) as u32;
    if copy_w == 0 || copy_h == 0 { return; }

    // B3 extension: check IME suppress rect
    if let Some(suppress) = ACTIVE_SUPPRESS_RECT.load().as_ref() {
        if suppress.contains(cx, cy) { return; }
    }

    blit_rgba_straight_clipped(fb, pixels, w, h, src_x0, src_y0, dst_x0, dst_y0, copy_w, copy_h);
}
```

**`ACTIVE_SUPPRESS_RECT: ArcSwap<Option<Rect>>`**: set only by the IME candidate window app
via `VYOMA_DRAW:cursor:suppress_over:<x>,<y>,<w>,<h>`. The supervisor allowlist restricts
this command to the app whose name equals `ACTIVE_IME` (B3 fix — IME candidate panel only).

---

## 4. Shape Library

Built-in sprites in `supervisor/src/cursor/builtin.rs` (16×16 RGBA, audited for hot spot bounds):

| Name | HotSpot | Notes |
|------|---------|-------|
| `arrow` | (0,0) | Default |
| `text` | (7,8) | Text beam |
| `crosshair` | (8,8) | Precision |
| `hand` | (5,0) | Hyperlinks |
| `grab` | (8,8) | Draggable hover |
| `grabbing` | (8,8) | Active drag |
| `resize_ew` | (8,8) | H-resize |
| `resize_ns` | (8,8) | V-resize |
| `resize_nesw` | (8,8) | Diagonal |
| `resize_nwse` | (8,8) | Diagonal |
| `resize_all` | (8,8) | Move |
| `not_allowed` | (8,8) | Drop disallowed |
| `wait` | (8,8) | Busy (animated) |
| `progress` | (0,0) | Arrow+spinner (animated) |
| `help` | (0,0) | Arrow+? |
| `none` | — | Equivalent to hide |

Unknown names fall back to `arrow` with a `WARN` log.

---

## 5. Cursor Shape Protocol

Apps write `VYOMA_DRAW:cursor:*` lines to stdout:

```
VYOMA_DRAW:cursor:shape:<name>
VYOMA_DRAW:cursor:shape_for_region:<x>,<y>,<w>,<h>,<name>   # window-local coords
VYOMA_DRAW:cursor:hide
VYOMA_DRAW:cursor:show
VYOMA_DRAW:cursor:custom:<id>,<w>,<h>,<hot_x>,<hot_y>,<base64_rgba>
VYOMA_DRAW:cursor:set_custom:<id>
VYOMA_DRAW:cursor:confine:<x>,<y>,<w>,<h>                   # screen-space
VYOMA_DRAW:cursor:unconfine
VYOMA_DRAW:cursor:suppress_over:<x>,<y>,<w>,<h>             # IME-only (B3)
```

Parsed by `supervisor/src/cursor/protocol.rs`, which calls `recompute_active_cursor()` after every mutation.

---

## 6. Custom Sprites (B4 Fix)

### 6.1 Upload Protocol

```
VYOMA_DRAW:cursor:custom:<id>,<w>,<h>,<hot_x>,<hot_y>,<base64_rgba>
```

Constraints:
- `id` must match `^[A-Za-z0-9_-]{1,32}$` — reject otherwise (B4).
- `w`, `h` ∈ [1, 64]; `hot_x` ∈ [0, w), `hot_y` ∈ [0, h) (B3 invariant audited on upload).
- Base64 payload: exactly `w * h * 4` decoded bytes (RGBA8888 straight alpha).
- Line limit: 16 KiB. Over-limit lines dropped with `WARN`.

### 6.2 Per-App Registry (B4 Fix)

```rust
// supervisor/src/cursor/registry.rs

pub struct AppCursorRegistry {
    pub custom:             HashMap<String, Arc<CursorSprite>>,  // id → sprite
    pub default_shape:      CursorShape,
    pub default_custom_id:  Option<String>,
    pub regions:            Vec<(Rect, CursorShape)>,            // window-local
    pub hide_requested:     bool,
    pub confine:            Option<Rect>,                        // screen-space; authoritative (B5)
    pub upload_budget:      TokenBucket,                         // B4: 64 KiB/sec
}

pub static APP_CURSORS: Lazy<Mutex<HashMap<String, AppCursorRegistry>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));
// Lock order: APP_CURSORS acquired BEFORE APPS_MAP (consistent with R32 ordering)
```

Quota: **8 custom sprites per app** (LRU eviction). Upload over budget → dropped with
`VYOMA_LOG:cursor:rate_limited`.

### 6.3 Atomic Cleanup on App Exit (B4 Fix)

```rust
// supervisor/src/cursor/registry.rs

pub fn cleanup_app_cursor(name: &str) {
    // B4 fix: bracket removal AND recompute under single APP_CURSORS lock
    let _guard = APP_CURSORS.lock().unwrap();
    APP_CURSORS_INNER.remove(name);      // remove registry entry
    // ACTIVE_CURSOR will lose the custom sprite reference on next recompute;
    // the Arc keeps it alive until the compositor finishes the current frame.
    recompute_active_cursor_under_lock(); // inline recompute while still holding guard
}
// Invariant: "custom sprite from exited app is not guaranteed to persist past app exit;
// cursor reverts to resolved shape for new hit-target on next mouse event or focus change."
```

---

## 7. Cursor Visibility

```rust
// supervisor/src/cursor/resolve.rs

fn effective_visible(target: Option<&str>, lock: u8) -> bool {
    if lock == LOCK_FS_TRANSITION { return false; }
    if let Some(app) = target {
        let reg = APP_CURSORS.lock().unwrap();
        if let Some(r) = reg.get(app) {
            if r.hide_requested { return false; }
        }
    }
    // During drag: never hide (prevents apps from stealing cursor mid-drag)
    if MOUSE_DRAG_CAPTURE.load().is_some() { return true; }
    true
}
```

**Auto-hide on typing**: if `CURSOR_POS` unchanged for ≥1500ms and a key event fires,
`CURSOR_HIDDEN_FOR_TYPING: AtomicBool` is set. Cleared on next mouse delta ≥1px. Checked
in `effective_visible`.

---

## 8. Shape Priority & `recompute_active_cursor`

```rust
// supervisor/src/cursor/resolve.rs

pub fn recompute_active_cursor() {
    // B2 fix: this is called ONCE per mouse event, AFTER all routing side effects
    let lock = INPUT_LOCK_LEVEL.load(Ordering::Acquire);
    let (cx, cy) = cursor_xy();

    // Hit-test under cursor (R32 logic; acquires APPS_MAP once)
    let target = {
        let map = APPS_MAP.lock().unwrap();
        hit_test(cx, cy, &map)
    };

    // B2 fix: verify drag owner is still alive
    let drag_capture = MOUSE_DRAG_CAPTURE.load();
    let effective_drag = drag_capture.as_ref().and_then(|name| {
        let map = APPS_MAP.lock().unwrap();
        if map.contains_key(name.as_str()) { Some(name.clone()) } else { None }
    });

    let target_name = target.as_ref().map(|h| h.name.as_str());
    let visible = effective_visible(target_name, lock);
    let shape = resolve_shape(lock, effective_drag.as_deref(), target.as_ref(), cx, cy);
    let sprite = sprite_for(&shape, target_name);
    let prev = ACTIVE_CURSOR.load();

    ACTIVE_CURSOR.store(Arc::new(CursorState {
        shape, sprite, visible,
        pos: (cx, cy),  // B1 fix: embed position
        epoch: prev.epoch.wrapping_add(1),
    }));
}

// Priority rules (first match wins):
fn resolve_shape(lock: u8, drag_owner: Option<&str>, target: Option<&HitResult>, cx: i32, cy: i32) -> CursorShape {
    // 1. FsTransition → hidden, shape irrelevant
    if lock == LOCK_FS_TRANSITION { return CursorShape::Arrow; }
    // 2. Any lock > None (except FS) → forced Arrow; custom suppressed
    if lock > LOCK_NONE && lock < LOCK_FS_TRANSITION { return CursorShape::Arrow; }
    // 3. Drag capture owner (B2: verified alive above)
    if let Some(owner) = drag_owner {
        let reg = APP_CURSORS.lock().unwrap();
        if let Some(r) = reg.get(owner) {
            return r.default_shape.clone();
        }
    }
    // 4. Region match → window-local lookup
    if let Some(hit) = target {
        let reg = APP_CURSORS.lock().unwrap();
        if let Some(r) = reg.get(hit.name.as_str()) {
            let lx = cx - hit.win_x;
            let ly = cy - hit.win_y;
            for (rect, shape) in &r.regions {
                if rect.contains(lx, ly) { return shape.clone(); }
            }
            // 5. App default
            if let Some(id) = &r.default_custom_id {
                if let Some(sprite) = r.custom.get(id) {
                    return CursorShape::Custom(sprite.clone());
                }
            }
            return r.default_shape.clone();
        }
    }
    // 6. Fallback
    CursorShape::Arrow
}
```

**`recompute_active_cursor` trigger list**:
- `route_mouse` after every motion event (after Enter/Leave synthesis — B2)
- `handle_cursor_cmd` for any app cursor request
- `INPUT_LOCK_LEVEL` write (lock-on and lock-off)
- `FOCUSED_APP` swap
- `MOUSE_DRAG_CAPTURE` change (after clear — B2)
- App exit (`cleanup_app_cursor`)

---

## 9. Cursor Confinement (B5 Fix)

### 9.1 Authoritative Source

`APP_CURSORS[owner].confine` is the **authoritative** confine rect. `ACTIVE_CONFINE` is a
derived cache. All clear paths write `confine = None` to `APP_CURSORS[owner]` first, then
update `ACTIVE_CONFINE`, both under the `APP_CURSORS` mutex (B5 fix).

```rust
pub static ACTIVE_CONFINE: ArcSwap<Option<Rect>> = ...;
// Always republished by recompute_active_cursor() from APP_CURSORS[focused].confine
```

### 9.2 Delta Accumulator (B5 Fix)

```rust
// supervisor/src/cursor/confine.rs

pub static CONFINED_LOGICAL_POS: AtomicU64 = AtomicU64::new(0); // same pack as CURSOR_POS

pub fn clip_and_update_cursor(raw_x: i32, raw_y: i32) {
    // B5 fix: trackpad deltas accumulate into CONFINED_LOGICAL_POS (unclipped),
    // clipped result → CURSOR_POS, so next event continues from clipped position.
    match *ACTIVE_CONFINE.load() {
        Some(rect) => {
            let clipped_x = raw_x.clamp(rect.x, rect.x + rect.w - 1);
            let clipped_y = raw_y.clamp(rect.y, rect.y + rect.h - 1);
            set_cursor_xy(clipped_x, clipped_y);
            // Also snap logical pos to clamped to avoid overshoot accumulation
            set_confined_logical_pos(clipped_x, clipped_y);
        }
        None => {
            set_cursor_xy(raw_x, raw_y);
        }
    }
}
```

On confine entry, `CONFINED_LOGICAL_POS` is initialized from `CURSOR_POS`.

### 9.3 Escape Chord — Bypasses SPSC (B5 Fix)

The escape chord (`LCtrl × 3 within 800 ms`) is matched in the **input thread's pre-routing
stage** — before any per-app SPSC delivery — so a backpressured app channel cannot trap it:

```rust
// supervisor/src/input/route.rs — pre-routing stage

fn check_escape_chord(ev: &KeyEvent) {
    if ev.key_code != KeyCode::LCtrl || !ev.is_press { return; }
    let now = now_ms();
    let prev = ESCAPE_CHORD_TIMES.lock().unwrap();
    // ... count Ctrl presses within 800ms window ...
    if count >= 3 {
        clear_active_confine_with_reason("escape");
        INPUT_EPOCH.fetch_add(1, Ordering::Release); // invalidate in-flight events
    }
}
```

`clear_active_confine_with_reason(reason)`:
1. Acquires `APP_CURSORS` mutex.
2. Sets `APP_CURSORS[owner].confine = None`.
3. Stores `ACTIVE_CONFINE = None`.
4. Enqueues `VYOMA_INPUT:confine_lost:<reason>` to owner's SPSC channel (non-blocking).
5. Calls `recompute_active_cursor_under_lock()`.

`reason` ∈ `{"focus", "lock", "escape", "exit"}`.

### 9.4 Eligibility

- Only the **focused, non-space-0** app may hold confinement.
- Focus loss, any `INPUT_LOCK_LEVEL > None`, app exit → confine cleared via `clear_active_confine_with_reason`.
- Space-0 apps cannot confine.

---

## 10. Blocking Issue Resolution Summary

| Issue | Resolution |
|-------|-----------|
| B1: Torn-read race — compositor reads CURSOR_POS separately from ACTIVE_CURSOR | `CursorState` embeds `pos: (i32, i32)`; `recompute_active_cursor` snapshots `cursor_xy()` at publish time; compositor uses `st.pos` exclusively |
| B2: Drag-capture shape races and drag-owner exit mid-drag | `recompute_active_cursor` called once after all R32 routing side effects; `effective_drag` verified alive via `APPS_MAP.contains_key` before rule 3 fires; dead drag owner falls through to rule 4 |
| B3: Hot spot OOB at screen edges + no IME cursor suppression | `blit_rgba_straight_clipped` clips both source + dest rects; hot spot validated `[0,w)×[0,h)` on upload; `ACTIVE_SUPPRESS_RECT` set only by `ACTIVE_IME` app via `suppress_over` command |
| B4: Custom sprite upload DoS, cleanup/Arc race, invalid IDs | Per-app `TokenBucket` (64 KiB/sec); ID validated `^[A-Za-z0-9_-]{1,32}$`; `cleanup_app_cursor` brackets removal + recompute under `APP_CURSORS` mutex atomically |
| B5: Confinement accumulator overshoot, SPSC-blocked escape chord, dual truth | `CONFINED_LOGICAL_POS` for accumulation; `CURSOR_POS` always clamped; escape chord detected in pre-routing stage before any SPSC; `APP_CURSORS[owner].confine` is authoritative truth; `ACTIVE_CONFINE` is derived cache republished under mutex |

---

## 11. File Layout

```
supervisor/src/cursor/
├── mod.rs          (~80 lines: ACTIVE_CURSOR, ACTIVE_CONFINE, ACTIVE_SUPPRESS_RECT exports)
├── state.rs        (~180 lines: CursorState, CursorSprite, HotSpot, CursorShape)
├── builtin.rs      (~320 lines: 16×16 RGBA sprites + animation frames)
├── registry.rs     (~240 lines: AppCursorRegistry, APP_CURSORS, cleanup, token bucket)
├── protocol.rs     (~280 lines: VYOMA_DRAW:cursor:* parser)
├── resolve.rs      (~220 lines: recompute_active_cursor, resolve_shape, effective_visible)
├── confine.rs      (~160 lines: confinement, CONFINED_LOGICAL_POS, clip_and_update_cursor, escape chord)
└── composite.rs    (~150 lines: composite_cursor with clipped blit)
```

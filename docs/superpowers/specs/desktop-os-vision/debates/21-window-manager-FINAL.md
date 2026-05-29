# FINAL Spec: Window Manager & Spaces (Round 21)

**Subsystem**: Window Manager & Spaces  
**macOS Analogue**: WindowServer spaces, `NSWindow` positioning, Mission Control Spaces  
**Depends on**: R11 (Surface buffers, vsync RwLock), R20 (DisplayConfig, logical pts), R18 (WIT gating)  
**Status**: FINAL — all blocking issues resolved

---

## 1. Overview

The Window Manager adds logical window geometry (position, size, z-order, title),
virtual desktop Spaces, and auto-tiling layout on top of the existing framebuffer
compositor. All window positions are in logical points (R20). No mouse dragging in
this round — deferred to R32 (Mouse & Gestures).

---

## 2. Window Properties (additions to AppState)

```rust
// supervisor/src/app_state.rs — new fields added to AppState

pub struct AppState {
    // existing fields...
    pub surface: Surface,
    pub hidpi_aware: bool,

    // --- new WM fields ---
    pub win_x: u32,           // logical points
    pub win_y: u32,           // logical points
    pub win_w: u32,           // logical points
    pub win_h: u32,           // logical points
    pub win_z: PerSpaceZ,     // per-space z-indices (B2 fix)
    pub win_space: u8,        // assigned space: 0 = all spaces, 1-9 = specific space
    pub win_visible: bool,
    pub win_focused: bool,
    pub win_minimized: bool,
    pub win_fullscreen: bool,
    pub win_manual_layout: bool,  // true = user set position/size manually
    pub win_title: String,        // defaults to manifest app name
    pub pending_resize: Option<(u32, u32)>,  // deferred resize: (new_w, new_h) in logical pts
}

impl AppState {
    pub fn new(manifest: &AppManifest, /* ... */) -> Self {
        // ...
        win_title: manifest.app.name.clone(),
        win_z: PerSpaceZ::default(),
        // ...
    }
}
```

### 2.1 PerSpaceZ (B2 fix — per-space z-indices)

```rust
// supervisor/src/wm/mod.rs

/// Per-space z-indices. Space-0 apps have a single canonical z (used as base).
pub struct PerSpaceZ {
    /// key = space number (1-9); value = z-index in that space
    pub by_space: HashMap<u8, u32>,
}

impl PerSpaceZ {
    pub fn get(&self, space: u8) -> u32 {
        self.by_space.get(&space).copied().unwrap_or(0)
    }
    pub fn set(&mut self, space: u8, z: u32) {
        self.by_space.insert(space, z);
    }
}
```

This replaces the single `win_z: u32`. Compacting space 1 only modifies `by_space[1]`;
space-2 z-indices are unaffected.

---

## 3. Spaces

```rust
// supervisor/src/wm/spaces.rs

pub struct SpaceRegistry {
    pub active: u8,         // 1-9; default 1
    pub count: u8,          // how many spaces exist (1-9)
    pub pending_switch: std::sync::atomic::AtomicU8,  // 0 = no pending switch; N = switch to N
}
```

Max 9 spaces. Default: 1 space at startup.

### 3.1 Space Commands

| IPC Command | Effect |
|-------------|--------|
| `space_add` | Add a space (up to 9); new space is empty |
| `space_del <N>` | Delete space N; move its apps to space 1; switch to space 1 if N was active |
| `space <N>` | Switch to space N (B3 fix — see section 5) |
| `assign <app> <S>` | Assign app to space S (0 = all spaces) |

---

## 4. Z-Order

### 4.1 Z-Index Rules

- Focused window in the active space has the highest z-index in that space
- When `focus <app>` is called: bring to front by setting `win_z.set(active_space, MAX + 1)` then compact
- Space-0 windows are always composited **after** (on top of) all space-N windows (B4 fix — Model A)

### 4.2 Compaction Algorithm

Z-indices are compacted to prevent unbounded growth. Called after: focus change, window
add, window remove (including process death), space switch.

```rust
// supervisor/src/wm/zorder.rs

pub fn compact_z_order(apps: &mut HashMap<String, AppState>, space: u8) {
    // Collect non-space-0 apps in this space, sorted by current z
    let mut space_n_apps: Vec<&str> = apps.values()
        .filter(|a| a.win_space == space && a.win_visible)
        .sorted_by_key(|a| a.win_z.get(space))
        .map(|a| a.name.as_str())
        .collect();

    // Assign z = 10 * (i+1) for each
    for (i, name) in space_n_apps.iter().enumerate() {
        if let Some(app) = apps.get_mut(*name) {
            app.win_z.set(space, (i as u32 + 1) * 10);
        }
    }

    // Space-0 apps: compact above all space-N apps (B4 fix)
    let base = space_n_apps.len() as u32 * 10 + 10;
    let mut space_0_apps: Vec<&str> = apps.values()
        .filter(|a| a.win_space == 0 && a.win_visible)
        .sorted_by_key(|a| a.win_z.get(0))
        .map(|a| a.name.as_str())
        .collect();

    for (i, name) in space_0_apps.iter().enumerate() {
        if let Some(app) = apps.get_mut(*name) {
            // Space-0 uses space=0 key for canonical z; accessed as the overlay layer
            app.win_z.set(0, base + (i as u32 + 1) * 10);
        }
    }
}
```

**Space-0 always-on-top**: the compositor iterates space-N apps first (by ascending z),
then space-0 apps (by their canonical `win_z.get(0)`). This is a model-level guarantee —
no space-N app can be above a space-0 app regardless of numeric z values.

### 4.3 Compaction Triggers

1. Focus changes (`focus <app>`)
2. Window added to a space
3. Window removed from a space (`kill`, `minimize`)
4. **App process exits** (lifecycle handler calls `wm::compact_z_order` for the app's space)
5. Space switch (compact for new active space)

---

## 5. Space Switching (B3 fix — atomic compositor handoff)

### 5.1 `PENDING_SPACE_SWITCH` Mechanism

```rust
// supervisor/src/wm/spaces.rs

/// AtomicU8: 0 = no pending switch; N (1-9) = switch to space N requested.
pub static PENDING_SPACE_SWITCH: AtomicU8 = AtomicU8::new(0);
```

### 5.2 IPC Handler (does NOT hold vsync_lock)

```rust
// supervisor/src/ipc_handlers.rs

fn handle_space_switch(target: u8) {
    // Validate target
    if target < 1 || target > spaces.count { return; }
    // Signal to compositor — does NOT acquire vsync_lock
    PENDING_SPACE_SWITCH.store(target, Ordering::Release);
    // Broadcast space_changed notification to all display apps
    // NOTE: spaces().lock() is NOT held during broadcast — no lock-order inversion
    broadcast_space_changed(target);
}
```

### 5.3 Compositor Tick (atomically applies the switch)

At the **start** of each compositor tick, before acquiring any surface locks:

```rust
// supervisor/src/compositor.rs — top of vsync_tick()

let pending = PENDING_SPACE_SWITCH.swap(0, Ordering::AcquireRelease);
if pending != 0 {
    // Atomically perform the space switch
    let _write = vsync_lock.write();  // holds write-lock for entire switch operation
    spaces.active = pending;
    fb.fill(BACKGROUND_COLOR);       // clear back-buffer to prevent ghost pixels
    // Mark all new-space apps dirty (forces full redraw)
    for app in apps.values_mut().filter(|a| a.win_space == pending || a.win_space == 0) {
        app.surface_dirty = true;
    }
    compact_z_order(&mut apps, pending);
    // write-lock drops here
}
// Normal compositor pass follows
```

**Lock sequence**: `PENDING_SPACE_SWITCH` atomic write (IPC thread) → `PENDING_SPACE_SWITCH` atomic read (compositor thread) → `vsync_lock.write()` (compositor thread) → updates. The IPC thread never holds `vsync_lock` during space switch. No deadlock possible.

**Ghost pixels eliminated**: `fb.fill(BACKGROUND_COLOR)` inside the write-lock guarantees the old space's pixels are cleared before the new space's surfaces are blitted.

---

## 6. Surface Reallocation (B1 fix — deferred resize)

### 6.1 Problem

`resize` IPC arrives on an app-output-reading thread. That thread may already hold
`vsync_lock.read()` (from processing a `VYOMA_DRAW:flush` earlier in the same line batch).
Acquiring `vsync_lock.write()` from within a thread holding `vsync_lock.read()` deadlocks.

### 6.2 Fix: Deferred Resize via `pending_resize`

The IPC resize handler sets `pending_resize` on the AppState (no lock needed for this
write since `pending_resize` is checked only by the compositor thread after
vsync_lock.write() is acquired):

```rust
// supervisor/src/ipc_handlers.rs

fn handle_resize(app_name: &str, new_w: u32, new_h: u32) {
    if let Some(app) = apps.get_mut(app_name) {
        // No vsync_lock required here — pending_resize is drained by compositor
        app.pending_resize = Some((new_w, new_h));
        app.win_manual_layout = true;
    }
}
```

### 6.3 Compositor Drains `pending_resize` Between Frames

Between the space-switch check and the surface blit loop, the compositor processes
pending resizes while holding `vsync_lock.write()`:

```rust
// supervisor/src/compositor.rs — after space switch check, before blit loop

{
    let _write = vsync_lock.write();  // full write-lock for all pending resizes
    for app in apps.values_mut() {
        if let Some((new_w, new_h)) = app.pending_resize.take() {
            let sf = config.scale_factor as u32;
            let phys_w = new_w.saturating_mul(sf);
            let phys_h = new_h.saturating_mul(sf);
            // Atomically: reallocate surface AND update geometry fields
            app.surface = Surface::new(phys_w, phys_h);
            app.win_w = new_w;
            app.win_h = new_h;
            // win_region is kept in sync (logical → physical)
            app.win_region = Some((
                app.win_x.saturating_mul(sf),
                app.win_y.saturating_mul(sf),
                phys_w, phys_h,
            ));
            app.surface_dirty = true;
        }
    }
    // write-lock drops here — blit loop immediately follows with read-lock
}
```

**Atomicity**: `surface`, `win_w`, `win_h`, and `win_region` are updated in the same
`vsync_lock.write()` critical section. No code outside this section reads these fields
for rendering purposes without first acquiring `vsync_lock`.

---

## 7. Auto-Tiling (B5 fix — layers model)

### 7.1 Auto-Tiling Rule

**When any app in the active space has `win_manual_layout = true`, auto-tiling is
globally disabled for that space.** Non-manual apps are tiled into the full logical
screen; manual apps float above (higher z-index from compaction).

This is the "layers" model:
- Bottom layer: auto-tiled apps (fill the full screen grid)
- Top layer: manually positioned apps (z-order above auto-tiled)

### 7.2 Tiling Algorithm

```rust
// supervisor/src/wm/tiling.rs

pub fn compute_tiling(n: usize, sw: u32, sh: u32) -> Vec<(u32, u32, u32, u32)> {
    match n {
        0 => vec![],
        1 => vec![(0, 0, sw, sh)],
        2 => vec![(0, 0, sw/2, sh), (sw/2, 0, sw - sw/2, sh)],
        3 => vec![(0, 0, sw/2, sh), (sw/2, 0, sw - sw/2, sh/2), (sw/2, sh/2, sw - sw/2, sh - sh/2)],
        4 => {
            let hw = sw/2; let hh = sh/2;
            vec![(0,0,hw,hh),(hw,0,sw-hw,hh),(0,hh,hw,sh-hh),(hw,hh,sw-hw,sh-hh)]
        }
        n => {
            // N≥5: equal-width columns
            let col_w = sw / n as u32;
            (0..n).map(|i| {
                let x = i as u32 * col_w;
                let w = if i == n-1 { sw - x } else { col_w };
                (x, 0, w, sh)
            }).collect()
        }
    }
}

pub fn apply_tiling_layout(apps: &mut HashMap<String, AppState>, config: &DisplayConfig, space: u8) {
    let has_manual = apps.values().any(|a| (a.win_space == space) && a.win_manual_layout && a.win_visible);
    // Auto-tiling only applies when no manual-layout windows exist in this space
    let auto_apps: Vec<&str> = if has_manual {
        return;  // layers model: any manual window disables auto-tiling for whole space
    } else {
        apps.values()
            .filter(|a| (a.win_space == space || a.win_space == 0) && a.win_visible && !a.win_minimized)
            .sorted_by_key(|a| a.spawn_order)   // stable sort by boot.toml order
            .map(|a| a.name.as_str())
            .collect()
    };

    let rects = compute_tiling(auto_apps.len(), config.logical_width, config.logical_height);
    for (name, rect) in auto_apps.iter().zip(rects.iter()) {
        if let Some(app) = apps.get_mut(*name) {
            app.win_x = rect.0; app.win_y = rect.1;
            app.win_w = rect.2; app.win_h = rect.3;
            app.pending_resize = Some((rect.2, rect.3));  // trigger surface realloc
            // win_manual_layout remains false — auto-tiled apps retain auto status
        }
    }
}
```

**Tiling order stability**: apps are sorted by `spawn_order` (index in boot.toml). New
apps appended at the end. This order is stable across recomputes — existing apps do not
swap positions.

### 7.3 `reset_tiling <app>` Command

Sets `win_manual_layout = false` for the specified app. If all apps in the space have
`win_manual_layout = false`, auto-tiling is re-enabled and `apply_tiling_layout` is
called. Formally add this to the WM command set (was missing from command table).

---

## 8. WIT Interface `vyoma:window-manager@1.0.0`

```wit
package vyoma:window-manager@1.0.0;

record window-info {
    app-id: u32,
    title: string,
    x: u32,  y: u32,  w: u32,  h: u32,  // logical points
    z: u32,
    space: u8,
    focused: bool,
    minimized: bool,
    manual-layout: bool,
}

interface window-manager {
    move-window:   func(app-id: u32, x: u32, y: u32) -> result<_, string>;
    resize-window: func(app-id: u32, w: u32, h: u32) -> result<_, string>;
    set-title:     func(app-id: u32, title: string)   -> result<_, string>;
    minimize:      func(app-id: u32) -> result<_, string>;
    restore:       func(app-id: u32) -> result<_, string>;
    set-space:     func(app-id: u32, space: u8) -> result<_, string>;
    focus-window:  func(app-id: u32) -> result<_, string>;
    list-windows:  func() -> result<list<window-info>, string>;
    focused-window: func() -> result<u32, string>;   // returns app-id
    reset-tiling:  func(app-id: u32) -> result<_, string>;
}

world window-manager-world {
    import window-manager;
}
```

---

## 9. VYOMA_WM: Stdout Protocol

Commands (app → supervisor):
```
VYOMA_WM:move:<app_id>,<x>,<y>
VYOMA_WM:resize:<app_id>,<w>,<h>
VYOMA_WM:title:<app_id>,<text>
VYOMA_WM:minimize:<app_id>
VYOMA_WM:restore:<app_id>
VYOMA_WM:space:<app_id>,<N>
VYOMA_WM:focus:<app_id>
VYOMA_WM:list
VYOMA_WM:reset_tiling:<app_id>
```

Push notifications (supervisor → app stdin):
```
VYOMA_WM:moved:<x>,<y>
VYOMA_WM:resized:<w>,<h>
VYOMA_WM:focused
VYOMA_WM:blurred
VYOMA_WM:space_changed:<N>
```

`list` response: one line per window, sorted by space ascending then z ascending:
```
VYOMA_WM_WINDOW:<app_id>,<title>,<x>,<y>,<w>,<h>,<z>,<space>,<focused>,<minimized>
VYOMA_WM_END
```

---

## 10. Compositor Integration

```rust
// supervisor/src/compositor.rs — updated vsync_tick()

fn vsync_tick(/* ... */) {
    // Step 1: Check pending space switch (B3 fix)
    let pending = PENDING_SPACE_SWITCH.swap(0, Ordering::AcquireRelease);
    if pending != 0 {
        let _write = vsync_lock.write();
        spaces.active = pending;
        fb.fill(BACKGROUND_COLOR);
        mark_new_space_dirty(&mut apps, pending);
        compact_z_order(&mut apps, pending);
    }

    // Step 2: Drain pending resizes (B1 fix)
    {
        let _write = vsync_lock.write();
        drain_pending_resizes(&mut apps, &config);
    }

    // Step 3: Compositor blit pass (read-lock)
    {
        let _read = vsync_lock.read();
        // Space-N apps first (ascending z in active space)
        let mut space_n: Vec<_> = apps.values()
            .filter(|a| (a.win_space == spaces.active) && a.win_visible && !a.win_minimized)
            .sorted_by_key(|a| a.win_z.get(spaces.active))
            .collect();
        for app in &space_n {
            blit_surface(&mut fb, app, &config);
        }
        // Space-0 apps last (always on top, B4 fix)
        let mut space_0: Vec<_> = apps.values()
            .filter(|a| a.win_space == 0 && a.win_visible && !a.win_minimized)
            .sorted_by_key(|a| a.win_z.get(0))
            .collect();
        for app in &space_0 {
            blit_surface(&mut fb, app, &config);
        }
    }

    // Step 4: DRM blit (write-lock)
    {
        let _write = vsync_lock.write();
        fb.blit_to_device();
    }
}
```

`blit_surface` accepts `(x, y)` offset from `app.win_x * scale_factor` and `app.win_y * scale_factor`.

---

## 11. Platform Matrix

| Feature | desktop-full | mobile | server-headless | others |
|---------|-------------|--------|----------------|--------|
| Spaces | yes (1-9) | no (1) | no (1) | no |
| Auto-tiling | yes | yes (full-screen only) | no | no |
| `move`/`resize` | yes | no | no | no |
| `minimize`/`restore` | yes | no (full-screen only) | no | no |
| `virtual_display` integration | yes | yes | yes | no |

Mobile: one app = full screen, no spaces, no manual layout. The active app is the focused one; switching apps is done via R24 (Dock/App Switcher).

---

## 12. File Layout

```
supervisor/src/wm/
├── mod.rs           (PerSpaceZ, SpaceRegistry, PENDING_SPACE_SWITCH)
├── commands.rs      (handle_move, handle_resize, handle_minimize, etc.)
├── spaces.rs        (space_add, space_del, space_switch, compact_z_order)
├── tiling.rs        (compute_tiling, apply_tiling_layout, reset_tiling)
└── focus.rs         (focus transfer, z-order bring-to-front)

supervisor/src/app_state.rs   (new WM fields, PerSpaceZ, pending_resize)
supervisor/src/compositor.rs  (updated vsync_tick with deferred resize + space switch)
supervisor/src/wit_handlers.rs (window-manager WIT closures)
```

---

## 13. Blocking Issue Resolution Summary

| Issue | Resolution |
|-------|-----------|
| B1: Surface realloc race | `pending_resize` field set by IPC thread; drained by compositor in `vsync_lock.write()` between frames; all geometry fields updated atomically in one critical section |
| B2: Z-order per-space non-determinism | `win_z: PerSpaceZ` (`HashMap<u8, u32>`); compacting space N only modifies `by_space[N]`; space-0 uses `by_space[0]` canonical z |
| B3: Space switch atomicity | `PENDING_SPACE_SWITCH: AtomicU8` written by IPC thread (no lock); checked at compositor tick start; `vsync_lock.write()` held for: active update + `fb.fill` + dirty marking; lock-order inversion avoided by releasing `spaces.lock()` before broadcast |
| B4: Space-0 z-order contradiction | Model A adopted: space-0 windows always composited after all space-N windows; `compact_z_order` assigns z = base + offset where base > max space-N z |
| B5: Partial auto-tiling fragmentation | Layers model: any `win_manual_layout = true` in a space disables auto-tiling globally for that space; auto-tiled apps tile into full screen; manual apps float above |

---

## 14. Open Questions (deferred)

1. **Mouse dragging**: `move`/`resize` are command-only in R21. R32 (Mouse & Gestures) adds drag-to-move and drag-to-resize.
2. **`win_region` deprecation**: Technical debt noted in AppState; migrate callers to `physical_rect()` helper in R22.
3. **`win_title` in compositor chrome**: Used by R23 (Menu Bar) and R24 (Dock). No visual rendering of title in R21.
4. **HiDPI + tiling**: `compute_tiling` returns logical points; R20 `DisplayConfig.logical_width/height` is the correct input. Already wired.

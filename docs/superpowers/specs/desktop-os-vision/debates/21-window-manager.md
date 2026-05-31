# Round 21 — Window Manager & Spaces (Architect)

**Status**: Draft
**Round**: 21
**Subsystem**: Window Manager & Spaces
**Analogue**: macOS WindowServer (spaces, Mission Control, `NSWindow` tiling, Cmd+number switching)
**Author**: Architect
**Date**: 2026-05-29

---

## 1. Overview and Motivation

VyomaOS through Round 20 treats the framebuffer as a shared canvas that apps paint on
in Z-order. Every running display app writes `VYOMA_DRAW:` commands that the supervisor
composites into one physical rectangle. The layout is fixed by the tiling algorithm in
`supervisor/src/windows.rs`: when an app starts, `apply_tiling_layout` divides the screen
into a grid and each app gets one cell. This is functional for a demonstration OS but it
creates three structural problems as VyomaOS moves toward real desktop use.

**Problem 1: No logical window abstraction.** Apps and windows are the same thing: one
WASM process equals one screen tile. There is no concept of a window title, a minimized
state independent of process life, or a window that belongs to a named space. The tiling
algorithm knows nothing about per-window policy; it just divides a rectangle by N.

**Problem 2: Spaces do not exist.** On macOS, Mission Control lets a user organise windows
into numbered spaces — virtual desktops — and switch between them with Ctrl+1 through
Ctrl+9. In the current VyomaOS model every display app is always composited; there is no
mechanism to say "show apps A and B on space 1, apps C and D on space 2." The compositor
always blits every non-minimized surface.

**Problem 3: Z-order is global and unmanaged.** `AppState.win_z` holds a raw integer and
`Z_ORDER` holds a `Vec<String>` of app names sorted by that integer. When `focus <app>` is
called, the app is moved to the end of `Z_ORDER` (top of the stack). There is no compaction
algorithm, no policy for what happens to z-indices when apps are removed, and no definition
of how spaces interact with the global z-order vector.

Round 21 introduces a **Window Manager** (WM) subsystem that resolves these three problems.
The WM adds per-window position and space metadata to `AppState`, defines a `SpaceRegistry`
that tracks which space is active, updates the compositor pass to filter by space and apply
window offsets, and provides both an IPC command protocol and a WIT interface for apps to
query and manipulate the window state. Mouse dragging is explicitly deferred to R32; window
positions in v1 are set only through supervisor commands.

### Relationship to Prior Rounds

| Round | Contribution reused or extended |
|-------|---------------------------------|
| R11 | Per-window `Surface` in `AppState`; `blit_surface` with `(dx, dy)` offsets |
| R19 | Virtual displays; compositor already has a flush pass per display |
| R20 | `DisplayConfig.width_pts` / `height_pts` as the logical coordinate space |
| R11 | `Z_ORDER: OnceLock<Mutex<Vec<String>>>` for current front-to-back ordering |

### What R21 Does Not Do

- No mouse dragging or pointer-driven window repositioning (R32).
- No window chrome (title bar, close/minimise/maximise buttons) — that is R23.
- No dock or taskbar — that is R24.
- No Expose / Mission Control animation — that is R27.
- No cross-space window transitions or animations.
- No fractional-point window positions. All coordinates are integers in logical points.

---

## 2. Window Model

### 2.1 Window as a Logical Wrapper Around an App

Every running WASM app that has `display = true` in its `vyoma.toml` corresponds to exactly
one window. The window is not a separate entity; its properties are fields on `AppState`.
Non-display apps (e.g., `http-server`, background daemons) have no window and are unaffected
by the WM.

A window has the following properties, all stored directly on `AppState`:

```rust
// New fields added to AppState in supervisor/src/main.rs

/// Window position in logical points (top-left corner).
/// Initialised to (0, 0); updated by apply_tiling_layout or `move` command.
win_x: i32,
win_y: i32,

/// Window size in logical points.
/// Mirrors win_region width/height at 1x; differs at 2x (see §2.4).
/// Initialised to (0, 0); set by apply_tiling_layout.
win_w: u32,
win_h: u32,

/// Z-index within the active space. Higher = closer to the viewer.
/// Range [0, u32::MAX]. Compacted by the WM on focus change (see §4).
win_z: u32,  // already exists; semantics refined in §4

/// Window title string, up to 128 bytes, UTF-8.
/// Default: the app name from the manifest.
win_title: String,

/// Whether the window is visible. false after `minimize`; true after `restore`.
/// Already exists as `minimized: bool`; the WM adds the inverse `visible` alias.
minimized: bool,   // already exists

/// Whether this window currently has keyboard focus.
/// Derived from the global `FocusedApp` state; not stored redundantly on AppState.
// (focused is queried by comparing app name to focused.lock().unwrap())

/// Whether the window is in fullscreen mode.
/// In v1 fullscreen means: win_x=0, win_y=0, win_w=display_pts_w, win_h=display_pts_h.
/// The auto-tiling algorithm treats a fullscreen window as if it is the only window.
win_fullscreen: bool,

/// Which space this window belongs to. 0 = all spaces. 1–9 = specific space.
win_space: u8,

/// Whether auto-tiling is locked for this window.
/// Set to true when a manual `move` or `resize` command is issued.
/// Set to false when `reset_tiling` is issued for the app.
win_manual_layout: bool,
```

### 2.2 Fields That Already Exist and Are Reused

`AppState` already carries `win_region: Option<(u32, u32, u32, u32)>` (physical pixel
rectangle assigned by tiling), `win_z: u32`, and `minimized: bool`. The new fields
`win_x`, `win_y`, `win_w`, `win_h` are **logical-point equivalents** of the physical pixel
values in `win_region`. The two representations coexist during the R21→R20 transition period
and the compositor uses physical-pixel values during blit. The conversion formula is:

```
win_region.x = win_x * scale_factor
win_region.y = win_y * scale_factor
win_region.w = win_w * scale_factor
win_region.h = win_h * scale_factor
```

After R21 is fully integrated, the `win_region` tuple may be deprecated in favour of
deriving physical bounds from the logical fields. Until then both are kept in sync by the
`apply_tiling_layout` and the `wm::apply_window_geometry` helper.

### 2.3 Invariants

1. A window's `win_space` is in the range [0, 9]. The value 0 means "show on every space."
2. A minimized window (`minimized = true`) is never composited regardless of its space.
3. A fullscreen window (`win_fullscreen = true`) fills the logical screen. At most one
   window per space can be fullscreen at a time; the WM enforces this by clearing the flag
   on all other windows in the space when a new window goes fullscreen.
4. `win_w` and `win_h` are always >= `min_size.0` and `min_size.1` respectively.
5. `win_z` values within a space are always distinct integers after any compaction pass.

### 2.4 Logical Points and the HiDPI Relationship

All window coordinates stored in `AppState` are in **logical points** as defined by R20.
At 1x (standard display): 1 point = 1 physical pixel. At 2x (HiDPI): 1 point = 2 physical
pixels. The WM stores and operates in logical points throughout; only the compositor
converts to physical pixels before writing to the framebuffer.

The `DisplayConfig` struct from R20 provides `width_pts()` and `height_pts()` which the
tiling and WM algorithms use as the canonical screen dimensions.

---

## 3. Spaces (Virtual Desktops)

### 3.1 Space Model

A **Space** is an ordered, numbered grouping of windows. Only the windows belonging to the
active space (plus windows assigned to space 0, which appear on every space) are composited
on each frame.

Space numbering:
- Space numbers are in the range [1, 9].
- The system always has at least one space (space 1).
- The active space is the one whose windows are currently displayed on the physical
  framebuffer.
- When the system boots, the active space is space 1 and all apps start on space 1.

### 3.2 SpaceRegistry Struct

A new module `supervisor/src/wm/spaces.rs` introduces `SpaceRegistry`:

```rust
pub struct SpaceRegistry {
    /// Currently active space number (1–9).
    pub active: u8,
    /// Bitmask of allocated spaces. Bit i set = space (i+1) exists.
    /// Space 1 is always allocated (bit 0 always set).
    allocated: u16,
}

impl SpaceRegistry {
    pub fn new() -> Self {
        Self { active: 1, allocated: 0b0000_0001 }
    }

    /// Returns true if space N is allocated.
    pub fn exists(&self, n: u8) -> bool {
        n >= 1 && n <= 9 && (self.allocated >> (n - 1)) & 1 == 1
    }

    /// Allocates the next available space number, up to 9.
    /// Returns the new space number or Err if all 9 are full.
    pub fn add(&mut self) -> Result<u8, &'static str> {
        for n in 1u8..=9 {
            if !self.exists(n) {
                self.allocated |= 1 << (n - 1);
                return Ok(n);
            }
        }
        Err("maximum of 9 spaces already allocated")
    }

    /// Removes space N. Callers must first re-assign its windows to space 1.
    /// Space 1 cannot be deleted.
    pub fn del(&mut self, n: u8) -> Result<(), &'static str> {
        if n == 1 { return Err("space 1 cannot be deleted"); }
        if !self.exists(n) { return Err("space does not exist"); }
        self.allocated &= !(1 << (n - 1));
        if self.active == n { self.active = 1; }
        Ok(())
    }

    /// Returns sorted list of allocated space numbers.
    pub fn list(&self) -> Vec<u8> {
        (1u8..=9).filter(|&n| self.exists(n)).collect()
    }
}
```

`SpaceRegistry` is stored in a `OnceLock<Mutex<SpaceRegistry>>` global, parallel to
`Z_ORDER`:

```rust
static SPACES: OnceLock<Mutex<SpaceRegistry>> = OnceLock::new();
pub fn spaces() -> &'static Mutex<SpaceRegistry> {
    SPACES.get_or_init(|| Mutex::new(SpaceRegistry::new()))
}
```

### 3.3 App-to-Space Assignment

Each app has `win_space: u8` on its `AppState`. On spawn, the supervisor sets
`win_space = spaces().lock().unwrap().active` — apps start on the currently active space.
Apps can be reassigned via the `assign <app> <space>` command (§5.8).

### 3.4 Space Switching

When `@supervisor: space <N>` is received:

1. Verify N is in [1, 9] and `spaces().exists(N)` is true. Reply error if not.
2. Acquire `spaces().lock()` and set `active = N`.
3. Release the lock.
4. Trigger a full compositor repaint (see §6.4).
5. Broadcast `VYOMA_WM:space_changed:<N>` to every app that has `display = true`
   (regardless of their own `win_space`), via their IPC inbox.

The space switch completes **asynchronously** from the compositor's perspective: the
supervisor sets `active = N` and marks the compositor dirty; the next compositor tick will
render the new space. There is no blocking wait on the vsync_lock during the space switch
command handler itself. This avoids a potential deadlock if the command arrives from an app
thread that is also holding a surface lock.

### 3.5 Space 0: Pinned Windows

Apps assigned to space 0 appear on every space. They are composited after all space-specific
windows (i.e., above them in z-order relative to the overall stack). See §4.4 for the
precise z-ordering rule for space-0 windows.

---

## 4. Window Z-Order

### 4.1 Current State (R11)

`Z_ORDER: OnceLock<Mutex<Vec<String>>>` holds a list of app names in ascending z-order:
index 0 is the bottommost window, last index is the topmost (focused) window. On focus
change, the app is moved to the end of the vector. The per-app `win_z: u32` field mirrors
the position in this vector but has not been kept strictly consistent.

### 4.2 R21 Z-Order Model

R21 replaces the implicit position-in-vector ordering with an explicit integer `win_z`
field that is authoritative. The `Z_ORDER` vector becomes a sorted cache recomputed on
each compaction. The algorithm is:

**Compaction algorithm** (called `wm::compact_z_order`, in `supervisor/src/wm/z_order.rs`):

```
Input: &AppRegistry, space_n: u8 (the space being operated on)
Output: nothing (mutates AppState.win_z for all apps in space_n + space-0 apps)

1. Collect all app names whose win_space == space_n or win_space == 0.
2. Sort by current win_z ascending (stable sort to preserve relative order when equal).
3. Reassign win_z values: first app gets win_z=10, next gets win_z=20, ..., step=10.
   (Step of 10 reserves room for future insertions without immediate recompaction.)
4. The focused app (from FocusedApp global) gets win_z = (count * 10) — it is last.
5. Update Z_ORDER vector to the sorted name list.
```

Compaction is called:
- After `focus <app>` changes the focused app.
- After any window is added or removed from a space.
- After `assign <app> <space>` reassigns a window.

The step-10 strategy means up to 9 focus changes can occur before a recompaction overflow
is theoretically possible at u32::MAX, but since compaction is called on every focus change,
the values never grow unbounded.

### 4.3 Focused Window Z-Order Rule

The focused window always has the highest z-index among windows in the active space (and
space-0 windows). This is guaranteed by the compaction step 4 above. When a new app is
spawned into the active space, it receives the focus and is placed at the top.

### 4.4 Space-0 Windows in the Z-Order

Space-0 windows (pinned to all spaces) are included in the compaction pass for every space.
Their z-indices are interleaved with the active space's windows based on whatever `win_z`
value they were given when last compacted. In most cases they should be positioned above
the active space's regular windows to function as "always on top" overlays (e.g., a system
notification window), but this is a per-window policy: the `assign <app> 0` command simply
moves the window to space 0; the operator or future chrome layer can subsequently call
`focus <app>` to bring it to the top.

There is no automatic "space-0 windows are always above space-N windows" rule. This is a
deliberate design choice: it allows space-0 windows to be background wallpaper apps (low
z-index) or overlay notification apps (high z-index) without the WM imposing a policy.

---

## 5. Window Commands

All window management commands arrive via the existing IPC `@supervisor:` protocol.
The parser in `supervisor/src/ipc_commands/` gains a new `wm.rs` handler file.

### 5.1 `move <app> <x> <y>`

Set the window's logical position.

```
@supervisor: move shell 100 50
```

- Parse `x` and `y` as `i32` (windows can be partially off-screen; compositor clips).
- Update `AppState.win_x = x`, `AppState.win_y = y`.
- Set `AppState.win_manual_layout = true` (disables auto-tiling for this window).
- Update `win_region` to `(x * sf, y * sf, win_w * sf, win_h * sf)` where `sf` is the
  active display scale factor from `DisplayConfig`.
- Trigger compositor repaint.
- Push `VYOMA_WM:moved:<x>,<y>` to the app's inbox.

### 5.2 `resize <app> <w> <h>`

Set the window's logical size. This is the most complex command because it may require
Surface reallocation.

```
@supervisor: resize shell 800 600
```

Procedure (see §6.3 for the Surface reallocation protocol):

1. Parse `w` and `h` as `u32`; clamp to `[min_size.0, display_pts_w]` and
   `[min_size.1, display_pts_h]` respectively.
2. Compute new physical size: `pw = w * sf`, `ph = h * sf`.
3. If the new physical size equals the current Surface size, skip reallocation.
4. Otherwise, perform safe Surface reallocation (§6.3).
5. Update `AppState.win_w = w`, `AppState.win_h = h`.
6. Update `win_region.2 = pw`, `win_region.3 = ph`.
7. Set `AppState.win_manual_layout = true`.
8. Trigger compositor repaint.
9. Push `VYOMA_WM:resized:<w>,<h>` (in logical points) to the app's inbox.

### 5.3 `minimize <app>`

Hide the window from the compositor.

```
@supervisor: minimize shell
```

- Set `AppState.minimized = true`.
- Store current region in `AppState.pre_minimize_region` (already exists from R18).
- Trigger compositor repaint.

The app process continues running; it simply does not contribute to the composed frame.
Push notifications are still delivered to minimized apps.

### 5.4 `restore <app>`

Restore a minimized window.

```
@supervisor: restore shell
```

- If `minimized == false`, no-op (reply "already visible").
- Set `AppState.minimized = false`.
- Restore `win_region` from `pre_minimize_region` if set.
- Trigger compositor repaint.

### 5.5 `space_add`

Create a new space.

```
@supervisor: space_add
```

- Call `SpaceRegistry::add()`.
- On success, reply with the new space number: `space_added:<N>`.
- On failure (9 spaces already exist), reply error.

### 5.6 `space_del <N>`

Delete space N. All windows assigned to space N are moved to space 1.

```
@supervisor: space_del 3
```

1. Validate N: must be 2–9, must exist.
2. Re-assign all apps with `win_space == N` to `win_space = 1`.
3. Call `SpaceRegistry::del(N)`.
4. If the active space was N, switch to space 1 (triggers compositor repaint).
5. Call `compact_z_order` for space 1.
6. Reply `space_deleted:<N>`.

### 5.7 `space <N>`

Switch the active space. Described in §3.4.

### 5.8 `assign <app> <space>`

Assign a window to a space. `<space>` is 0–9; 0 = all spaces.

```
@supervisor: assign shell 2
@supervisor: assign overlay 0
```

1. Validate `space` is in [0, 9].
2. If `space >= 1`, verify `SpaceRegistry::exists(space)`.
3. Update `AppState.win_space = space`.
4. Call `compact_z_order` for the target space and (if different) the source space.
5. Trigger compositor repaint.
6. Push `VYOMA_WM:space_changed:<space>` to the app.

### 5.9 `title <app> <text>`

Set the window title string.

```
@supervisor: title shell "System Shell"
```

- Parse `text` as a UTF-8 string up to 128 bytes. Truncate silently if longer.
- Update `AppState.win_title = text`.
- No compositor repaint required (title is not rendered until R23 chrome).
- Reply `ok`.

### 5.10 `focus <app>` (updated)

The existing `focus` IPC command is updated to call `compact_z_order` after changing the
focused app, ensuring the newly focused window is at the highest z-index.

Additions to the existing focus handler:

1. After updating `FocusedApp`, call `wm::compact_z_order(&registry, active_space)`.
2. Push `VYOMA_WM:focused` to the newly focused app's inbox.
3. Push `VYOMA_WM:blurred` to the previously focused app's inbox (if any).

---

## 6. Compositor Integration

### 6.1 Current Compositor Pass (R11)

The existing compositor flush pass in `display/mod.rs` iterates over `Z_ORDER`, reads each
app's `Surface`, and calls `blit_surface(fb_back, surface, dx, dy, alpha, fb_stride, ...)`.
The `dx, dy` values come from `win_region.0` and `win_region.1` respectively (physical
pixel positions from the tiling layout).

### 6.2 R21 Compositor Pass Changes

The compositor pass is updated with two new filtering steps:

**Step A: Space filter.** Before blitting a window's Surface, check:
```rust
let in_active_space = app_state.win_space == 0
    || app_state.win_space == spaces().lock().unwrap().active;
let should_composite = in_active_space && !app_state.minimized;
```
Skip windows that fail this test.

**Step B: Logical-to-physical coordinate conversion.** Replace the raw `win_region`
tuple as the blit origin with the computed physical coordinates:
```rust
let sf = display_config.scale_factor as u32;
let dx = (app_state.win_x.max(0) as u32) * sf;
let dy = (app_state.win_y.max(0) as u32) * sf;
```
Windows with `win_x < 0` or `win_y < 0` are partially off-screen; `blit_surface`'s
existing bounds-clamping logic handles this correctly already (the `dx` saturating_sub
path).

**Step C: Sort order.** The iteration order is determined by `Z_ORDER` (ascending z),
which is already maintained sorted by the compaction algorithm. No additional sort is
needed at compositor time.

### 6.3 Surface Reallocation Protocol

When `resize <app> <w> <h>` changes a window's logical size, the app's `Surface` must be
reallocated to the new physical dimensions. The challenge is that the compositor thread may
be mid-blit of the old Surface simultaneously (see Critique §B1 for detailed analysis).

The safe reallocation protocol:

```
1. Acquire vsync_lock.write() — this blocks until any in-progress compositor blit completes.
   The compositor takes vsync_lock.read() during its blit pass.
2. With write lock held:
   a. Create new_surface = Surface::new(pw, ph).
   b. Replace AppState.surface with Arc<Mutex<Surface>> wrapping new_surface.
   c. Update win_w, win_h, win_region.
3. Release vsync_lock.write().
4. The compositor's next read-lock acquire will see the new Surface.
```

The `vsync_lock` is `Arc<RwLock<()>>` (established in R11). Taking a write lock for
reallocation ensures mutual exclusion with the compositor's read lock during the blit.

This is the only correct approach given the current lock topology. Alternative approaches
(e.g., double-buffering the Surface pointer) would require an atomic pointer swap, which
is possible in Rust but adds complexity without meaningful benefit since resize is a rare,
user-initiated event.

### 6.4 Full Compositor Repaint on Space Switch

When the active space changes, the previous space's windows disappear and the new space's
windows appear. A full repaint is required. The mechanism:

```rust
// In the space switch handler (wm/commands.rs):
// 1. Update SpaceRegistry.active.
// 2. Set APP_DIRTY for every app in the new active space and every space-0 app.
// 3. The compositor's dirty-check on the next tick will fire a full redraw.
```

The `APP_DIRTY: OnceLock<Mutex<HashMap<String, bool>>>` global (already exists from R18)
is used to signal the compositor. Setting all windows dirty ensures the background is also
redrawn (no ghost pixels from the previous space).

Additionally, the compositor must clear the framebuffer back-buffer to the desktop
background colour before compositing the new space's windows. This is a `fill_rect` of the
full physical screen dimensions with the background colour (default: `0x1E1E2EFF`).

### 6.5 `blit_surface` Offset Handling

The `blit_surface` function signature in `display/surface.rs` already accepts `dx: u32` and
`dy: u32` as physical pixel offsets. The existing implementation correctly clips the blit
region to the framebuffer bounds. No changes to `blit_surface` itself are required; R21
only changes how the caller computes `dx` and `dy` (using logical-point win_x/win_y
multiplied by scale_factor instead of the raw win_region tuple).

---

## 7. WIT Interface `vyoma:window-manager@1.0.0`

The WIT interface lives in `wit/window-manager.wit`. It is imported by apps that want
to programmatically query or manipulate windows at link time.

```wit
package vyoma:window-manager@1.0.0;

interface window-manager {
    /// A window descriptor returned by list-windows.
    record window-info {
        name:       string,
        title:      string,
        x:          s32,
        y:          s32,
        width:      u32,
        height:     u32,
        z-index:    u32,
        space:      u8,
        minimized:  bool,
        focused:    bool,
        fullscreen: bool,
    }

    /// Move window to (x, y) in logical points.
    move-window:    func(app: string, x: s32, y: s32) -> result<_, string>;

    /// Resize window to (w, h) in logical points.
    resize-window:  func(app: string, w: u32, h: u32) -> result<_, string>;

    /// Set window title.
    set-title:      func(app: string, title: string) -> result<_, string>;

    /// Minimize window (hide from compositor).
    minimize:       func(app: string) -> result<_, string>;

    /// Restore minimized window.
    restore:        func(app: string) -> result<_, string>;

    /// Assign window to a space (0 = all spaces, 1–9 = specific space).
    set-space:      func(app: string, space: u8) -> result<_, string>;

    /// Return list of all windows known to the WM.
    list-windows:   func() -> list<window-info>;

    /// Return the name of the currently focused window, or None.
    focused-window: func() -> option<string>;

    /// Switch to the named space.
    switch-space:   func(n: u8) -> result<_, string>;

    /// Return list of allocated space numbers.
    list-spaces:    func() -> list<u8>;
}

world window-manager-world {
    import window-manager;
}
```

This WIT interface is gated by the `shell` capability: only apps with `shell = true` in
`vyoma.toml` may bind the `vyoma:window-manager` import. Apps without `shell = true` that
attempt to call these functions receive an immediate WASI trap. This follows the R18
link-time capability gating pattern.

---

## 8. VYOMA_WM: Stdout Protocol

For apps that use the stdout draw protocol rather than WIT, the same operations are
available via `VYOMA_WM:` prefixed lines printed to stdout. The supervisor's line router
(in `router.rs`) dispatches lines matching `VYOMA_WM:` to the WM command handler, parallel
to how `VYOMA_DRAW:` lines are dispatched to the draw command handler.

### 8.1 Command Format

```
VYOMA_WM:move:<app>,<x>,<y>
VYOMA_WM:resize:<app>,<w>,<h>
VYOMA_WM:title:<app>,<text>
VYOMA_WM:minimize:<app>
VYOMA_WM:restore:<app>
VYOMA_WM:focus:<app>
VYOMA_WM:assign:<app>,<space>
VYOMA_WM:space:<N>
VYOMA_WM:space_add
VYOMA_WM:space_del:<N>
```

### 8.2 Query Commands (Request/Response)

Because stdout is fire-and-forget for draw commands, queries use the existing IPC
request/response model: the app sends a query to `@supervisor:` and receives a reply on
its stdin.

```
@supervisor: wm_list_windows
→ (reply on stdin, newline-separated)
window:shell,0,0,960,540,10,1,false,true,false
window:gui-demo,960,0,960,540,20,1,false,false,false

@supervisor: wm_focused
→ focused:shell

@supervisor: wm_list_spaces
→ spaces:1,2,3
```

### 8.3 Response Format

`wm_list_windows` reply: one line per window:
```
window:<name>,<x>,<y>,<w>,<h>,<z>,<space>,<minimized>,<focused>,<fullscreen>
```

All values are integers or `true`/`false` booleans. Coordinates are in logical points.

---

## 9. Focus Management

### 9.1 Focus State Sources

Focus in VyomaOS is held in the global `FocusedApp: Arc<Mutex<Option<String>>>`. The WM
does not replace this; it adds behaviour triggered by focus changes.

### 9.2 When a Window Gains Focus

1. The `focus <app>` IPC command (or `VYOMA_WM:focus:<app>`) updates `FocusedApp`.
2. The WM handler calls `wm::compact_z_order(&registry, active_space)`, which moves the
   focused app to the highest z-index in the current space.
3. The WM pushes `VYOMA_WM:focused` to the newly focused app's IPC inbox (stdin).
4. If a different app was previously focused, the WM pushes `VYOMA_WM:blurred` to it.
5. The compositor sees the updated Z_ORDER on its next tick and composites the focused
   window on top.

### 9.3 Keyboard Input Target

The existing `input_keys.rs` routes keypresses to `FocusedApp`. No changes are needed; the
WM and input router share the same `FocusedApp` global.

### 9.4 Focus on Space Switch

When the active space changes to space N:

- If the current focused app belongs to space N (or space 0), it retains focus.
- If the current focused app belongs to a different space, focus is transferred to the
  topmost window in space N (the app with the highest `win_z` in space N).
- If space N has no windows, `FocusedApp` is set to `None`.

This logic runs inside the `space <N>` command handler, after `SpaceRegistry.active` is
updated but before the compositor repaint is triggered.

---

## 10. Tiling Layout (Default)

### 10.1 Existing Tiling Engine

`supervisor/src/windows.rs` implements `compute_tiling(n, sw, sh)` which divides an
`sw × sh` screen into a grid of `n` rectangles. The algorithm: columns = ceil(sqrt(n)),
rows = ceil(n / cols). The last row's cells expand to fill the screen width.

### 10.2 R21 Changes to Tiling

The tiling engine is updated to:

1. Accept `sw, sh` in **logical points** (from `DisplayConfig.width_pts()` and
   `height_pts()`) rather than physical pixels.
2. Skip apps that have `win_manual_layout = true` — their positions are fixed.
3. Only tile apps belonging to the **active space** (or space 0 if pinned).
4. After computing logical-point regions, convert to physical pixels for `win_region`
   using `win_x * sf`, etc.

The `apply_tiling_layout` function in `app_threads.rs` (which calls `compute_tiling` and
writes results back to `AppState`) is updated to apply these filters and use logical
coordinates.

### 10.3 Tiling Modes

| App count (in active space, non-manual) | Layout |
|-----------------------------------------|--------|
| 0 | Nothing drawn (blank screen) |
| 1 | Full logical screen (0, 0, pts_w, pts_h) |
| 2 | 50/50 vertical split (left: 0,0,pts_w/2,pts_h; right: pts_w/2,0,pts_w/2,pts_h) |
| 3–4 | 2×2 grid (cells: pts_w/2 × pts_h/2) |
| 5–6 | 2×3 grid (2 columns, 3 rows) |
| 7–9 | 3×3 grid |

This is unchanged from the existing algorithm; R21 only changes the input coordinate space
from physical pixels to logical points.

### 10.4 Manual Layout and Auto-Tiling Interaction

When a user issues `move <app>` or `resize <app>`, `win_manual_layout` is set to `true`
for that app. On the next app add/remove event, the auto-tiling pass skips apps with
`win_manual_layout = true` and only re-tiles the non-manual apps. This is the **partial
auto-tiling** model.

Rationale: the alternative (always overwriting manual positions on every app add/remove)
is disruptive and breaks user intent. The alternative (disabling auto-tiling entirely after
any manual move) is too conservative and breaks the new-app-join experience. Partial
auto-tiling threads the needle: manually positioned windows stay put; newly added windows
are tiled into the remaining space.

A user can re-enable auto-tiling for a specific window with `@supervisor: reset_tiling
<app>`, which sets `win_manual_layout = false` and triggers a full re-tile.

---

## 11. Window Events

Apps receive push notifications via their IPC inbox (stdin) when WM state changes affect
them. The event format follows the `VYOMA_WM:` prefix convention.

### 11.1 Event Table

| Event | Trigger | Payload |
|-------|---------|---------|
| `VYOMA_WM:moved:<x>,<y>` | `move` command applied to this app | Logical point position |
| `VYOMA_WM:resized:<w>,<h>` | `resize` command applied to this app | Logical point size |
| `VYOMA_WM:focused` | This app gained keyboard focus | none |
| `VYOMA_WM:blurred` | This app lost keyboard focus | none |
| `VYOMA_WM:space_changed:<N>` | Active space switched (sent to ALL display apps) | New space number |
| `VYOMA_WM:minimized` | This app was minimized | none |
| `VYOMA_WM:restored` | This app was restored from minimize | none |
| `VYOMA_WM:fullscreen:<1\|0>` | Fullscreen state changed for this app | 1=entered, 0=exited |

### 11.2 Delivery Guarantee

Events are delivered via the `send_reply(target, msg, inbox)` function, which pushes a
string onto the app's `mpsc::Sender<String>`. This is a best-effort, in-order, single-
writer delivery model. There is no acknowledgement mechanism. Apps that do not read stdin
will have their inbox backpressure up to the channel capacity (default: 64 messages from
the `mpsc::channel` default), after which `send_reply` silently drops the message.

This matches the existing IPC delivery model and is acceptable for WM events, which are
low-frequency user-driven actions.

---

## 12. Platform Matrix

The WM subsystem is conditional on platform profile. The `supervisor/src/wm/` module is
compiled on all profiles but behaviour is gated by the active profile at runtime.

| Platform | Window Manager Behaviour |
|----------|--------------------------|
| `desktop-full` | Full WM: spaces (up to 9), move/resize, Z-order, tiling, focus events |
| `mobile` | Simplified: no spaces, all windows are full-screen, no manual move/resize; focus switching only |
| `server-headless` | No display; WM module present but all commands are no-ops |
| `iot-edge` | No display by default; WM disabled |
| `robotics-rt` | No display; WM disabled |
| `mcu-minimal` | No WM (no display, no OS-level windowing) |

### 12.1 Mobile Profile Detail

On `mobile`, the WM enforces:
- Every display app is automatically fullscreen: `win_x=0, win_y=0, win_w=pts_w, win_h=pts_h`.
- `move` and `resize` commands are rejected with error "not supported on mobile profile".
- `space_add` and space switching are rejected similarly.
- Only one app is visible at a time (apps are "stacked"; the topmost is composited; others
  are effectively minimized behind it).
- Focus switching via `focus <app>` raises an app to the top of the stack.

This mirrors iOS/iPadOS behaviour where apps are full-screen and switching is managed by
the system rather than the user dragging windows.

### 12.2 Profile Detection

The active platform profile is available via `std::env::var("PLATFORM")` or the
`DISPLAY_PROFILE` global (already set in `main.rs`). The WM command handler checks
`DISPLAY_PROFILE.get()` at the start of each command and applies the platform-specific
restrictions.

---

## 13. File Layout

The WM subsystem is organised as a new `supervisor/src/wm/` directory. The 500-line per-
file limit applies; each concern is isolated.

```
supervisor/src/wm/
├── mod.rs          (~80 lines)   — pub re-exports; compact_z_order; apply_window_geometry
├── spaces.rs       (~120 lines)  — SpaceRegistry struct + SPACES global
├── z_order.rs      (~100 lines)  — compact_z_order algorithm; Z_ORDER maintenance
├── commands.rs     (~200 lines)  — IPC command parser: move, resize, minimize, restore,
│                                   space_add, space_del, space, assign, title, focus update
├── tiling.rs       (~60 lines)   — R21 updates to apply_tiling_layout (space-aware, pts)
└── events.rs       (~60 lines)   — push_wm_event helper; event type enum
```

Existing files modified:

| File | Change |
|------|--------|
| `supervisor/src/main.rs` | Add `win_x, win_y, win_w, win_h, win_title, win_fullscreen, win_space, win_manual_layout` to `AppState`; add `SPACES` global |
| `supervisor/src/display/mod.rs` | Update compositor pass: space filter, logical→physical dx/dy |
| `supervisor/src/windows.rs` | Update `apply_tiling_layout` to use logical points and skip manual windows |
| `supervisor/src/ipc_handlers.rs` | Route `move`, `resize`, `minimize`, `restore`, `space_add`, `space_del`, `space`, `assign`, `title`, `wm_list_windows`, `wm_focused`, `wm_list_spaces` to `wm::commands` |
| `supervisor/src/router.rs` | Add `VYOMA_WM:` prefix dispatch to `wm::commands` |

---

## 14. Data Flow Summary

```
App stdout
  "VYOMA_WM:move:shell,100,50"
        │
  router.rs  ──── VYOMA_WM: prefix ────▶  wm/commands.rs::handle_wm_stdout(line)
                                                │
                                    parse "move shell 100 50"
                                                │
                                    AppState.win_x=100, win_y=50
                                    win_manual_layout=true
                                    update win_region (×sf)
                                    mark APP_DIRTY[shell]=true
                                    push VYOMA_WM:moved:100,50 to shell inbox

  display/mod.rs compositor tick
        │
  acquire vsync_lock.read()
        │
  for name in Z_ORDER (ascending z):
      app = registry[name]
      if app.win_space != 0 && app.win_space != active_space: continue
      if app.minimized: continue
      dx = app.win_x.max(0) as u32 * sf
      dy = app.win_y.max(0) as u32 * sf
      surface = app.surface.lock()
      blit_surface(fb_back, &surface, dx, dy, alpha, ...)
        │
  fb.flush()   (swap back-buffer to DRM/virtio-gpu)
        │
  release vsync_lock.read()
```

---

## 15. Migration Notes for Existing Apps

All existing apps are unaffected by default. On boot:

- Every app that was already running on space 1 (the only space at boot) continues to
  function as before.
- `win_space = 1` for all apps at spawn time.
- `win_manual_layout = false` for all apps; auto-tiling runs as before.
- `win_title` is initialised to the app name from the manifest.

Apps do not need to handle `VYOMA_WM:` events; they are delivered but ignorable. The only
change an existing app might observe is the addition of `VYOMA_WM:focused` and
`VYOMA_WM:blurred` on its stdin, which any well-written app will handle via the
"ignore unknown protocol lines" convention.

---

## 16. Testing Requirements

### 16.1 Unit Tests (supervisor/tests/)

New test file: `supervisor/tests/wm_spaces.rs`

| Test | Validates |
|------|-----------|
| `test_space_add_del` | SpaceRegistry allocates/deallocates correctly up to 9 |
| `test_space_del_moves_windows` | `space_del` reassigns windows to space 1 |
| `test_compact_z_order_simple` | 3 apps get z=10,20,30; focused app gets z=30 |
| `test_compact_z_order_focus_change` | After focus change, compacted values are dense multiples of 10 |
| `test_tiling_skips_manual` | `apply_tiling_layout` skips apps with `win_manual_layout=true` |
| `test_compositor_space_filter` | Compositor skips apps in inactive spaces |
| `test_space_switch_focus_transfer` | Focus transfers to topmost window in new space |

### 16.2 Integration

Existing smoke test (`make smoke`) verifies `[lifecycle] all apps spawned`. The WM does
not change the lifecycle model, so smoke test should pass without modification. A new smoke
test variant (`make smoke-spaces`) can be added in a future CI round.

---

## Appendix A: Command Quick Reference

```
@supervisor: move <app> <x> <y>          — position window (logical pts)
@supervisor: resize <app> <w> <h>        — resize window (logical pts, reallocates Surface)
@supervisor: minimize <app>              — hide window
@supervisor: restore <app>               — show window
@supervisor: focus <app>                 — focus + bring to front
@supervisor: title <app> <text>          — set window title
@supervisor: assign <app> <space>        — move to space (0=all)
@supervisor: space <N>                   — switch active space
@supervisor: space_add                   — create new space
@supervisor: space_del <N>               — delete space, move windows to space 1
@supervisor: reset_tiling <app>          — re-enable auto-tiling for app
@supervisor: wm_list_windows             — list all windows (reply on stdin)
@supervisor: wm_focused                  — get focused app name (reply on stdin)
@supervisor: wm_list_spaces              — list space numbers (reply on stdin)
```

# Data Model: Interactive Window Management

## New / Modified Entities

### DragState (new, in `mouse_input.rs`)

Tracks an in-progress title-bar drag. Stored as a module-level `static DRAG_STATE: OnceLock<Mutex<Option<DragState>>>`.

| Field | Type | Description |
|-------|------|-------------|
| `app_name` | `String` | Name of the app whose title bar initiated the drag |
| `cursor_start` | `(i32, i32)` | Cursor position at left-button press |
| `win_start` | `(u32, u32, u32, u32)` | `win_region` (x, y, w, h) of the app at drag start |

**State transitions:**
- `None` → `Some(DragState)`: Left button pressed over a title bar (not a traffic-light dot).
- `Some` → position updates: Every `EV_SYN` while `btn_held & 1 != 0`.
- `Some` → `None`: Left button released.

---

### AppState (extended, in `main.rs`)

Two fields added to the existing `AppState` struct:

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `minimized` | `bool` | `false` | Whether the window is currently minimized to a strip |
| `pre_minimize_region` | `Option<(u32,u32,u32,u32)>` | `None` | Saved `win_region` before minimization; restored on un-minimize |

**State transitions for `minimized`:**
- `false` → `true`: User clicks yellow traffic-light dot.
- `true` → `false`: User clicks the minimized strip.

**Invariants:**
- `pre_minimize_region` is `Some(…)` if and only if `minimized == true`.
- `win_region` holds the strip position (240×28) when `minimized == true`.

---

## Pure Functions (new, in `windows.rs`)

### `nearest_tiled_slot`

```
fn nearest_tiled_slot(
    win_pos: (u32, u32),       // top-left of dragged window after drop
    n_apps: usize,             // number of running display apps (for grid computation)
    sw: u32,                   // screen width
    sh: u32,                   // screen height
    menubar_h: u32,            // pixels reserved for the global menu bar
) -> Option<(u32, u32, u32, u32)>
```

Returns the grid slot whose top-left corner is within 40 px Chebyshev distance of `win_pos`, or `None` if all slots are farther. Grid computed via `compute_tiling_with_hints(n_apps, sw, sh - menubar_h, &[(0,0);n_apps])` with each slot's y offset by `menubar_h`.

**Testable properties:**
- For a 1-app grid on 1440×900 with menubar=24: returns `Some((0, 24, 1440, 876))` for any `win_pos` within 40 px of (0, 24).
- Returns `None` for a position >40 px from all slots.
- Is pure (no side effects, no statics).

---

## Unchanged Entities

- `TrafficLight` enum (`Close`, `Minimize`, `Maximize`) — values reused as-is.
- `win_region: Option<(u32,u32,u32,u32)>` in `AppState` — still holds the current display region; during minimization it is overwritten with the strip coordinates.
- `MOUSE_DRAG_START: OnceLock<Mutex<Option<(i32,i32)>>>` — continues to hold cursor start position for the existing button-held check; `DRAG_STATE` adds the per-drag window info alongside it.

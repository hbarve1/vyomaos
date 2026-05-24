# Data Model: Windowing System and Mouse Input

**Feature**: 002-windowing-mouse-input
**Date**: 2026-05-24

---

## Entities

### WindowRegion (existing, in `manifest.rs`)

Represents a rectangular area of the framebuffer assigned to one app.

| Field | Type | Source | Description |
|-------|------|--------|-------------|
| `x` | `u32` | supervisor-assigned | Left edge in screen pixels |
| `y` | `u32` | supervisor-assigned | Top edge in screen pixels |
| `w` | `u32` | supervisor-assigned | Width in pixels |
| `h` | `u32` | supervisor-assigned | Height in pixels |
| `title` | `Option<String>` | manifest | Display name hint (not rendered — border-only decoration) |
| `width` | `Option<u32>` | manifest | App-preferred minimum width |
| `height` | `Option<u32>` | manifest | App-preferred minimum height |

**Lifecycle**: Created by `parse_manifest()`; `x/y/w/h` fields overwritten by supervisor's `compute_tiling()` on every display-app spawn or exit. The manifest fields `width`/`height` are read-only hints; `title` is reserved for future use.

**Validation rules**:
- `x + w ≤ screen_width` (enforced by tiling engine)
- `y + h ≤ screen_height` (enforced by tiling engine)
- If app declares `min_width > available_col_width`: supervisor assigns full row to that app (best-effort)

---

### AppState::win_region (existing field, `main.rs`)

Runtime representation of the assigned window. Separate from the manifest struct so the supervisor can update coordinates without re-parsing the manifest.

| Field | Type | Description |
|-------|------|-------------|
| `win_region` | `Option<(u32, u32, u32, u32)>` | `(x, y, w, h)` in screen coordinates; `None` for non-display apps or queued apps beyond slot limit |

**State transitions**:
- `None` → `Some(...)`: when a display app spawns and is assigned a tile
- `Some(old)` → `Some(new)`: when the layout reflows after a spawn/exit
- `Some(...)` → `None`: when the app's tile is reclaimed (app exits, or >9 display apps)

---

### CursorState (new, to be added to `display.rs:Framebuffer`)

Tracks the software mouse cursor position and the pixels saved beneath it.

| Field | Type | Description |
|-------|------|-------------|
| `cx` | `i32` | Current cursor X in screen pixels (clamped 0..screen_width-1) |
| `cy` | `i32` | Current cursor Y in screen pixels (clamped 0..screen_height-1) |
| `visible` | `bool` | True if a mouse device was detected at startup |
| `saved_under` | `[u8; 12 * 19 * 4]` | BGRA pixels saved from back-buffer before cursor was drawn |
| `drawn` | `bool` | True if cursor is currently composited into back-buffer (needs restore before next app redraw) |

**State transitions**:
- `drawn=false` → `drawn=true`: when `draw_cursor()` writes sprite into back-buffer
- `drawn=true` → `drawn=false`: when `restore_under_cursor()` restores saved pixels (called before any app draw command that touches the cursor area, or before `flush()`)

---

### FocusState (existing, `type FocusedApp = Arc<Mutex<Option<String>>>`)

Tracks which app currently receives keyboard input.

| Field | Type | Description |
|-------|------|-------------|
| inner | `Option<String>` | Name of focused app, or `None` if no apps are running |

**State transitions**:
- `None` → `Some(name)`: first display/shell app spawns
- `Some(prev)` → `Some(next)`: user clicks on a different window
- `Some(name)` → `Some(other)`: focused app exits; supervisor auto-transfers focus
- `Some(name)` → `None`: last app exits

---

### MouseEvent (protocol-level, not a Rust struct)

Delivered as a line on the target app's stdin:

```
VYOMA_INPUT:mouse:move:<x>,<y>
VYOMA_INPUT:mouse:click:<x>,<y>:<button>
```

| Field | Type | Constraints |
|-------|------|-------------|
| `x` | `i32` | App-local coordinate (0 = left edge of window) |
| `y` | `i32` | App-local coordinate (0 = top edge of window) |
| `button` | `&str` | One of: `left`, `right`, `middle` |

**Delivery conditions**:
- App must have `capabilities.mouse = true`
- Cursor must be inside the app's `win_region`
- `move` events: dispatched on every EV_SYN where position changed, regardless of button state
- `click` events: dispatched on `EV_KEY BTN_* value=1` (button press), not release

---

## Entity Relationships

```
BootEntry ──────────────────────────── AppManifest
    │                                        │
    │ has_one                                 │ has_one (optional)
    ▼                                        ▼
AppState ◄──── win_region ──────── WindowRegion
    │                                  (x,y,w,h from compute_tiling)
    │ has_one
    ▼
FocusState (shared singleton)
    └── determines keyboard routing

Framebuffer ──── CursorState (new)
    │               (cx, cy, saved_under, drawn)
    └── back Vec<u8> (write target for all drawing + cursor compositing)
```

---

## State Invariants

1. At most one app has keyboard focus at any time (`FocusedApp` is `Mutex<Option<String>>`).
2. The number of apps with `win_region = Some(...)` never exceeds 9.
3. `CursorState::drawn == true` only when cursor pixels are in the back-buffer; `flush()` must never be called while `drawn=true` without first calling `restore_under_cursor()`.
4. All `win_region` assignments satisfy: no two regions overlap; union of all regions covers the full screen (when 1–9 display apps are running).
5. A non-display app (`capabilities.display = false`) always has `win_region = None`.

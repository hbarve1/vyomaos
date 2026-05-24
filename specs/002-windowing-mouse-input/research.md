# Research Findings: Windowing System and Mouse Input

**Feature**: 002-windowing-mouse-input
**Date**: 2026-05-24

## 1. Linux Mouse Input via evdev

### Decision
Use `/dev/input/eventX` (evdev protocol) with blocking `read()` on a dedicated thread.

### Rationale
- VyomaOS kernel config already includes `CONFIG_INPUT=y`, `CONFIG_INPUT_EVDEV=y`, `CONFIG_VIRTIO_INPUT=y`
- `/dev/input/mice` requires `CONFIG_INPUT_MOUSEDEV=y` (PS/2 emulation) — not present, and undesirable for a minimal kernel
- `/dev/input/eventX` works directly with virtio-input (QEMU `-device virtio-mouse-pci`)
- Blocking `read()` on a dedicated thread is correct: no CPU spin, kernel wakes the thread on arrival

### Alternatives Considered
- **PS/2 `/dev/input/mice`**: Requires `CONFIG_INPUT_MOUSEDEV` — adds kernel complexity; rejected
- **O_NONBLOCK tight loop**: Wastes CPU at 100% between events; rejected
- **libinput**: Requires udev, not compatible with static musl supervisor; rejected

### Protocol (evdev input_event, 64-bit Linux)
```
struct input_event {
    i64 tv_sec    // bytes 0–7
    i64 tv_usec   // bytes 8–15
    u16 type      // bytes 16–17
    u16 code      // bytes 18–19
    i32 value     // bytes 20–23
}  // = 24 bytes total
```

Relevant types/codes already used in supervisor:
- `EV_REL (2)` + `REL_X (0)` / `REL_Y (1)`: relative movement (physical mouse)
- `EV_ABS (3)` + `ABS_X (0)` / `ABS_Y (1)`: absolute position (virtio-mouse-pci, range 0–32767)
- `EV_KEY (1)` + `BTN_LEFT (0x110)` / `BTN_RIGHT (0x111)` / `BTN_MIDDLE (0x112)`: buttons
- `EV_SYN (0)`: synchronisation event — flush accumulated deltas

**Existing implementation**: `open_mouse_device()` and the mouse-input thread in `supervisor/src/main.rs` already implement this correctly. No kernel config changes needed.

---

## 2. Software Cursor Compositing over /dev/fb0

### Decision
Maintain a saved-pixel backup under the cursor sprite; restore on next draw cycle.

### Rationale
- The existing `Framebuffer` struct uses a `back` Vec<u8> as a back-buffer; `flush()` blits back → front atomically
- Save-under approach: before drawing cursor into `back`, save the 16×16 pixels beneath it; after next app `flush()` naturally overwrites cursor position in back, redraw cursor on top
- This avoids tearing and requires no per-app changes
- X11 software cursors use the same technique
- Hardware overlay planes (DRM atomic modesetting) would avoid the CPU copy but require DRM API, not available via `/dev/fb0` mmap

### Cursor Sprite
- **Size**: 12×19 pixels (standard arrow) — fits in `[[u8; 3]; 19]` where each row is packed 3 bytes
- **Format**: Hardcoded `const CURSOR_SPRITE: &[[u8; 3]; 19]` — 1-bit mask (set = white, clear = transparent)
- **Hotspot**: (0, 0) — top-left pixel of sprite is the pointer tip

### Alternatives Considered
- **Save-under per app (flush-triggered restore)**: Complex; requires knowing which app will flush next; rejected in favor of simpler restore-before-draw approach
- **Always redraw from app back-buffer**: Requires tracking which pixels belong to which app — too complex; rejected
- **Dedicated overlay thread at 60 fps**: Wastes CPU repainting when nothing moves; rejected

### Thread Safety
- All `Framebuffer` accesses are serialised through `Mutex<Framebuffer>` in `static FB`
- Cursor state (position + saved pixels) lives inside `Framebuffer` struct — automatically serialised
- No concurrent writes to `/dev/fb0`; existing architecture is correct

---

## 3. Tiled Window Layout Algorithm

### Decision
`columns = ceil(sqrt(n))`, `rows = ceil(n / columns)`, filled left-to-right top-to-bottom; last row's empty slots expand remaining windows to fill row width.

### Rationale
- Spec explicitly mandates this algorithm (Assumptions §)
- Produces balanced grids for n=1..9: 1→1×1, 2→2×1, 3→2×2, 4→2×2, 6→3×2, 9→3×3
- No manual configuration required
- Empty-slot expansion ensures 100% screen coverage (SC-001)

### Layout Table
| n | cols | rows | grid |
|---|------|------|------|
| 1 | 1    | 1    | 1×1 full-screen |
| 2 | 2    | 1    | side-by-side |
| 3 | 2    | 2    | 2+1 (last row 1 tile spans 2 cols) |
| 4 | 2    | 2    | 2×2 |
| 5 | 3    | 2    | 3+2 (last row: 2 tiles each 1.5x normal width) |
| 6 | 3    | 2    | 3×2 |
| 7 | 3    | 3    | 3+3+1 (last row spans 3) |
| 8 | 3    | 3    | 3+3+2 |
| 9 | 3    | 3    | 3×3 |

### Implementation Strategy
- New function `compute_tiling(n, screen_w, screen_h) -> Vec<(u32,u32,u32,u32)>` in `supervisor/src/windows.rs`
- Returns one `(x, y, w, h)` tuple per display app, in registration order
- Called from main thread on every display-app spawn or exit
- Regions assigned to `AppState::win_region` fields by iterating the display app list in registration order

---

## 4. Existing Implementation Status

### Already Implemented (no changes needed)
| Component | Location | Status |
|-----------|----------|--------|
| evdev device discovery | `main.rs:open_mouse_device()` | ✅ complete |
| evdev event parsing loop | `main.rs` mouse-input thread | ✅ complete |
| ABS/REL coordinate handling | `main.rs` mouse-input thread | ✅ complete |
| Z-order stack | `main.rs:Z_ORDER` | ✅ complete |
| Keyboard focus on click | `main.rs:dispatch_mouse()` | ✅ complete |
| `mouse` capability field | `manifest.rs:Capabilities` | ✅ complete |
| `[window]` manifest section | `manifest.rs:WindowRegion` | ✅ complete (has min_w/min_h) |
| `fill_rect` clipping to win_region | `main.rs:handle_draw_command()` | ✅ complete |
| `draw_text` clipping to win_region | `main.rs:handle_draw_command()` | ✅ complete |
| Framebuffer back-buffer | `display.rs:Framebuffer::back` | ✅ complete |
| Kernel input drivers | `base/kernel.config` | ✅ complete |

### Needs New Implementation
| Component | Gap | Where |
|-----------|-----|--------|
| Tiled layout engine | Regions currently read from manifest, not computed | New `supervisor/src/windows.rs` |
| Dynamic reflow on spawn/exit | No recompute call exists | `main.rs` spawn/exit paths |
| Mouse cursor sprite | Not rendered at all | `display.rs` (cursor state + draw) |
| Mouse event format update | Current: `VYOMA_INPUT:mouse:lx,ly,btn` → Spec: `move:x,y` / `click:x,y:button` | `main.rs:dispatch_mouse()` |
| Window border style | Current: 20px title bar + close button → Spec: 2px inset border only | `main.rs:handle_draw_command()` |
| Dynamic screen size for mouse clamp | Current: `const SCREEN_W: i32 = 1440` → use `display::screen_size()` | `main.rs` mouse-input thread |
| Focus transfer on exit | Not implemented | `main.rs:wait_app()` |

---

## 5. Key Design Decisions Confirmed

- **No new kernel config changes**: `CONFIG_VIRTIO_INPUT=y` already present
- **No QEMU flags changes**: `-device virtio-mouse-pci` already part of run-gui targets
- **Single rendering thread**: Mutex<Framebuffer> serialises all writes; cursor state is safe inside the struct
- **Supervisor-assigned regions only**: Apps no longer set their own x/y/w/h in `[window]`; supervisor overwrites with computed tiling. Apps may still declare `width`/`height` as min-size hints.
- **Mouse event format change is not backward-compatible**: FR-013 requires existing no-mouse apps to be unaffected (they are — they never receive mouse events). Mouse-capable apps need to update their event parsing.

# Implementation Plan: Interactive Window Management

**Branch**: `042-interactive-window-management` | **Date**: 2026-05-25 | **Spec**: [spec.md](spec.md)

**Input**: Feature specification from `specs/042-interactive-window-management/spec.md`

## Summary

Implement window drag-to-move (real-time `win_region` update on mouse motion while title bar is held), snap-back to the nearest tiled slot on release (Chebyshev ≤ 40 px), a functional close button (SIGKILL + reflow + focus transfer — partially implemented, missing the reflow/focus steps), and minimize/restore (240×28 strip at screen bottom, pre-minimize region saved in AppState).

All logic lives in the supervisor (`supervisor/src/`). No WASM app changes are required. Five files are modified; one new test file is added.

## Technical Context

**Language/Version**: Rust stable (`x86_64-unknown-linux-musl`, `wasm32-wasip2` — no WASM changes)

**Primary Dependencies**: `libc` (SIGKILL), `supervisor::windows` (tiling), `crate::display` (fb flush), `crate::chrome` (draw_titlebar, repaint_all_borders)

**Storage**: No persistent storage changes; state is in-memory `AppState`

**Testing**: `cargo test --target x86_64-unknown-linux-musl`, `RUSTFLAGS=-D warnings`

**Target Platform**: Linux (virtio-gpu, evdev mouse)

**Project Type**: OS supervisor (Rust binary, PID 1)

**Performance Goals**: Drag updates ≤ 33 ms per mouse event (SC-001); close ≤ 200 ms (SC-002); minimize/restore ≤ 100 ms (SC-003)

**Constraints**: No file > 500 lines; zero new warnings; all existing tests must pass

**Scale/Scope**: 5 files modified, 1 new test file, ~200 lines net added

## Constitution Check

| Principle | Status | Notes |
|-----------|--------|-------|
| I. Capability-Secure | ✅ Pass | No capability model changes; `mouse_input.rs` only reacts to events already gated by `has_mouse` and `has_display` |
| II. Test-First | ✅ Pass | `drag_test.rs` with `nearest_tiled_slot`, drag clamp, and minimize-state tests written before implementation |
| III. Minimal Surface Area | ✅ Pass | ~200 lines added; no new dependencies; `DragState` and two `AppState` fields are the only new data |
| IV. Hermetic Builds | ✅ Pass | No build system changes; all targets remain the same |
| V. Explicit Over Implicit | ✅ Pass | `minimized` and `pre_minimize_region` are explicit fields; no magic detection |
| VI. Observability First | ✅ Pass | Drag start/end, snap, close, minimize, restore all emit `log_info!` |

**Quality Gates:**
- `cargo check` zero new warnings: required before every commit
- Smoke test (`make build && make run`): required before PR
- All existing supervisor tests must continue passing
- `wc -l supervisor/src/*.rs` — no file > 500 lines

## Project Structure

### Documentation (this feature)

```text
specs/042-interactive-window-management/
├── plan.md              ← this file
├── research.md          ← Phase 0 output
├── data-model.md        ← Phase 1 output
├── quickstart.md        ← Phase 1 output
├── contracts/
│   └── window-state-contract.md
└── tasks.md             ← Phase 2 output (/speckit-tasks)
```

### Source Code Changes

```text
supervisor/src/
├── main.rs              # Add minimized: bool, pre_minimize_region: Option<…> to AppState
├── mouse_input.rs       # DRAG_STATE global; drag update loop; close fix; minimize; strip-click
├── windows.rs           # Add nearest_tiled_slot() pure function
└── draw_cmd.rs          # Early return on st.minimized

supervisor/tests/
└── drag_test.rs         # New: nearest_tiled_slot tests, drag clamp tests, minimize invariant tests
```

**No changes to**: `chrome.rs`, `display/`, `ipc_handlers.rs`, `ipc_commands/`, `app_threads.rs`, `toast.rs`, `lib.rs`, any WASM app

## Implementation Design

### 1. `nearest_tiled_slot` (windows.rs)

New pure function:

```rust
pub fn nearest_tiled_slot(
    win_pos: (u32, u32),
    n_apps: usize,
    sw: u32,
    sh: u32,
    menubar_h: u32,
) -> Option<(u32, u32, u32, u32)> {
    if n_apps == 0 { return None; }
    let usable_h = sh.saturating_sub(menubar_h);
    let hints = vec![(0u32, 0u32); n_apps];
    let slots: Vec<(u32,u32,u32,u32)> = compute_tiling_with_hints(n_apps, sw, usable_h, &hints)
        .into_iter()
        .map(|(x, y, w, h)| (x, y + menubar_h, w, h))
        .collect();
    let (px, py) = win_pos;
    slots.into_iter().min_by_key(|&(sx, sy, _, _)| {
        let dx = (px as i32 - sx as i32).unsigned_abs();
        let dy = (py as i32 - sy as i32).unsigned_abs();
        dx.max(dy)
    }).filter(|&(sx, sy, _, _)| {
        let dx = (px as i32 - sx as i32).unsigned_abs();
        let dy = (py as i32 - sy as i32).unsigned_abs();
        dx.max(dy) <= 40
    })
}
```

### 2. AppState fields (main.rs)

```rust
struct AppState {
    // ... existing fields ...
    minimized:           bool,
    pre_minimize_region: Option<(u32, u32, u32, u32)>,
}
```

Initialise both to `false` / `None` in the two `AppState` construction sites.

### 3. DRAG_STATE global (mouse_input.rs)

```rust
struct DragState {
    app_name:     String,
    cursor_start: (i32, i32),
    win_start:    (u32, u32, u32, u32),
}

static DRAG_STATE: OnceLock<Mutex<Option<DragState>>> = OnceLock::new();
fn drag_state() -> &'static Mutex<Option<DragState>> {
    DRAG_STATE.get_or_init(|| Mutex::new(None))
}
```

### 4. Drag initiation (mouse_input.rs — EV_SYN on click path)

When `pending_click_mask & 1 != 0` (left button pressed), after setting `MOUSE_DRAG_START`:

```rust
// Find title-bar app under cursor (excluding traffic-light area)
let dragged = {
    let reg = registry.lock().unwrap();
    let mut found = None;
    for name in &z_snap {
        let Some(st) = reg.get(name) else { continue };
        let st = st.lock().unwrap();
        let Some((wx, wy, ww, _)) = st.win_region else { continue };
        if cy >= wy as i32 && cy < (wy + TITLEBAR_H) as i32
            && cx >= wx as i32 && cx < (wx + ww) as i32
            && traffic_light_hit(cx, cy, wx, wy).is_none() {
            found = Some((name.clone(), st.win_region.unwrap()));
            break;
        }
    }
    found
};
if let Some((name, region)) = dragged {
    *drag_state().lock().unwrap() = Some(DragState {
        app_name: name, cursor_start: (cx, cy), win_start: region,
    });
}
```

### 5. Drag update (mouse_input.rs — EV_SYN on position-changed path)

Replace the existing log-only drag block:

```rust
if btn_held & 1 != 0 {
    let ds = drag_state().lock().unwrap().clone();
    if let Some(ds) = ds {
        let (dx, dy) = drag_delta(ds.cursor_start.0, ds.cursor_start.1, cx, cy);
        let (wx, wy, ww, wh) = ds.win_start;
        let new_x = (wx as i32 + dx).clamp(0, (screen_w - ww as i32).max(0)) as u32;
        let new_y = (wy as i32 + dy).clamp(MENUBAR_H as i32, (screen_h - wh as i32).max(MENUBAR_H as i32)) as u32;
        let new_region = (new_x, new_y, ww, wh);
        // Update registry
        {
            let reg = registry.lock().unwrap();
            if let Some(st_arc) = reg.get(&ds.app_name) {
                st_arc.lock().unwrap().win_region = Some(new_region);
            }
        }
        // Repaint title bar at new position (partial blit)
        if let Some(fb_lock) = display::get() {
            let focused_name = focused.lock().unwrap().clone();
            let is_focused = focused_name.as_deref() == Some(ds.app_name.as_str());
            let mut fb = fb_lock.lock().unwrap();
            // Clear old title-bar rows
            fb.fill_rect(wx, wy, ww, TITLEBAR_H, 0x0D1117FF);
            // Paint new title bar
            draw_titlebar(&mut *fb, new_x, new_y, ww, is_focused, false, &ds.app_name);
            fb.flush();
        }
        log_info!(Subsystem::Input, Some(ds.app_name.as_str()),
            "drag: ({wx},{wy}) → ({new_x},{new_y})");
    }
}
```

### 6. Drag release + snap (mouse_input.rs — pending_release_mask path)

After clearing `MOUSE_DRAG_START`:

```rust
let ds_finished = drag_state().lock().unwrap().take();
if let Some(ds) = ds_finished {
    // Snap-back check
    let (wx, wy, ww, wh) = {
        let reg = registry.lock().unwrap();
        reg.get(&ds.app_name)
            .and_then(|st| st.lock().unwrap().win_region)
            .unwrap_or(ds.win_start)
    };
    let n_display = {
        let reg = registry.lock().unwrap();
        reg.values().filter(|st| {
            let st = st.lock().unwrap();
            st.has_display && matches!(st.status, AppStatus::Running)
        }).count()
    };
    let snap = supervisor::windows::nearest_tiled_slot(
        (wx, wy), n_display, screen_w as u32, screen_h as u32,
        crate::chrome::MENUBAR_H,
    );
    if let Some(snapped) = snap {
        let reg = registry.lock().unwrap();
        if let Some(st_arc) = reg.get(&ds.app_name) {
            st_arc.lock().unwrap().win_region = Some(snapped);
        }
        log_info!(Subsystem::Input, Some(ds.app_name.as_str()), "drag: snapped to tiled slot");
    }
}
```

### 7. Close button fix (mouse_input.rs — TrafficLight::Close arm)

The existing code already sends SIGKILL. The exit watcher in `app_threads.rs` handles reflow and focus transfer on natural death. Because SIGKILL causes the wasmtime child to exit, the watcher fires automatically — **no additional code is required** in the close arm. The existing code already produces the correct outcome through the existing exit path.

Verification: `app_threads.rs` → `wait_app()` → calls `apply_tiling_layout` + `toast::auto_transfer_focus` on app exit.

### 8. Minimize (mouse_input.rs — TrafficLight::Minimize arm)

```rust
TrafficLight::Minimize => {
    let strip_pos: Option<(u32, u32)> = {
        let reg = registry.lock().unwrap();
        let minimized_count = reg.values()
            .filter(|st| st.lock().unwrap().minimized)
            .count() as u32;
        let strip_x = minimized_count * 240;
        // Wrap to second row if necessary
        let row = strip_x / screen_w as u32;
        let col_x = strip_x % screen_w as u32;
        let strip_y = screen_h as u32 - 28 - row * 28;
        reg.get(&name).map(|st| {
            let mut st = st.lock().unwrap();
            st.pre_minimize_region = st.win_region;
            st.minimized = true;
            st.win_region = Some((col_x, strip_y, 240, 28));
            (col_x, strip_y)
        })
    };
    if let Some((sx, sy)) = strip_pos {
        // Paint the title-bar strip at the minimized position
        if let Some(fb_lock) = display::get() {
            let focused_name = focused.lock().unwrap().clone();
            let is_focused = focused_name.as_deref() == Some(name.as_str());
            draw_titlebar(&mut *fb_lock.lock().unwrap(), sx, sy, 240, is_focused, false, &name);
            fb_lock.lock().unwrap().flush();
        }
        log_info!(Subsystem::Display, Some(name.as_str()), "minimized to strip ({sx},{sy})");
    }
}
```

### 9. Strip click detection (mouse_input.rs — btn != 0 path, before traffic-light check)

Before the existing traffic-light block, check if the click lands on a minimized strip:

```rust
// Minimized-strip restore: check before traffic-light hit-test
if btn != 0 {
    let restore_target: Option<String> = {
        let reg = app_registry.lock().unwrap();
        reg.iter().find_map(|(n, st)| {
            let st = st.lock().unwrap();
            if !st.minimized { return None; }
            let Some((sx, sy, sw, sh)) = st.win_region else { return None };
            if cx >= sx as i32 && cy >= sy as i32
                && cx < (sx + sw) as i32 && cy < (sy + sh) as i32 {
                Some(n.clone())
            } else { None }
        })
    };
    if let Some(name) = restore_target {
        let reg = app_registry.lock().unwrap();
        if let Some(st_arc) = reg.get(&name) {
            let mut st = st_arc.lock().unwrap();
            if let Some(prev) = st.pre_minimize_region.take() {
                st.win_region = Some(prev);
                st.minimized = false;
                // Repaint at restored position
                drop(st);
                drop(reg);
                if let Some(fb_lock) = display::get() {
                    let is_focused = focused.lock().unwrap().as_deref() == Some(name.as_str());
                    let win = {
                        let reg = app_registry.lock().unwrap();
                        reg.get(&name).and_then(|st| st.lock().unwrap().win_region)
                    };
                    if let Some((wx, wy, ww, _)) = win {
                        draw_titlebar(&mut *fb_lock.lock().unwrap(), wx, wy, ww, is_focused, false, &name);
                        fb_lock.lock().unwrap().flush();
                    }
                }
                log_info!(Subsystem::Display, Some(name.as_str()), "restored from minimized strip");
            }
        }
        return;
    }
}
```

### 10. Draw command guard (draw_cmd.rs)

At the top of `handle_draw_command`, after resolving `win`:

```rust
// Skip draw commands for minimized windows (content area is hidden)
{
    let reg = app_registry.lock().unwrap();
    if reg.get(sender).map(|st| st.lock().unwrap().minimized).unwrap_or(false) {
        return;
    }
}
```

## Complexity Tracking

No constitution violations. No extra complexity beyond what the spec requires.

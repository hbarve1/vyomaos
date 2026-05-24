# Quickstart: Windowing System and Mouse Input

**Feature**: 002-windowing-mouse-input

---

## Integration Scenario 1: Boot with Multiple Display Apps

**Goal**: Verify tiled layout and border rendering.

1. Build and boot with at least 3 display apps:
   ```
   make build && make run-gui DISPLAY_BACKEND=cocoa
   ```
2. Observe the screen:
   - With 3 apps: 2 windows on top row, 1 full-width on bottom
   - Each window has a 2 px inset border (accent color on focused, dim on unfocused)
   - No title bar; no close button; app content fills the full region
3. Kill one app via shell: `kill gui-demo`
4. Observe within 500 ms: remaining 2 apps reflow to side-by-side layout

---

## Integration Scenario 2: Mouse Cursor Tracking

**Goal**: Verify cursor sprite renders and tracks physical mouse.

1. Boot with `make run-gui` (requires `-device virtio-mouse-pci` in QEMU invocation)
2. Move the physical mouse or VM mouse
3. Observe:
   - A 12×19 white arrow cursor appears and moves smoothly
   - Cursor crosses window boundaries without disappearing
   - Cursor stops at screen edges (no wrap)
4. Hover the cursor over an actively-drawing window; verify cursor remains on top

---

## Integration Scenario 3: Mouse Event Routing

**Goal**: Verify `VYOMA_INPUT:mouse:move` and `click` events with correct local coordinates.

1. Build and run the `mouse-demo` app (to be created as a test app):
   ```toml
   # apps/mouse-demo/vyoma.toml
   [capabilities]
   display = true
   mouse   = true
   ```
   The app prints received mouse events in its window.

2. Click inside the mouse-demo window:
   - App receives `VYOMA_INPUT:mouse:click:<x>,<y>:left`
   - `x` and `y` should match the position of the click relative to the window's top-left corner
   - Verify: click at window left edge produces x≈0

3. Move mouse inside the window:
   - App receives `VYOMA_INPUT:mouse:move:<x>,<y>` at ≥30 Hz
   - Moving to window right edge: x approaches `window_width - 1`

4. Click outside the window:
   - App receives no events

---

## Integration Scenario 4: Keyboard Focus Follows Click

**Goal**: Verify click-to-focus and border indicator.

1. Boot with two keyboard-interactive apps side by side
2. Click on app A's window:
   - App A's border changes to accent color (focused)
   - App B's border changes to dim color
3. Type a key: input routes to app A only
4. Click on app B:
   - Borders swap; app B now receives keyboard input

---

## Unit Test Scenarios (supervisor/tests/)

### Tiling layout correctness

```rust
use supervisor::windows::compute_tiling;

#[test]
fn test_4_apps_2x2() {
    let regions = compute_tiling(4, 1024, 768);
    assert_eq!(regions.len(), 4);
    // Top-left region at (0,0)
    assert_eq!(regions[0], (0, 0, 512, 384));
    // Top-right at (512,0)
    assert_eq!(regions[1], (512, 0, 512, 384));
    // Full screen coverage
    let area: u32 = regions.iter().map(|(_, _, w, h)| w * h).sum();
    assert_eq!(area, 1024 * 768);
}

#[test]
fn test_3_apps_last_row_expands() {
    let regions = compute_tiling(3, 1024, 768);
    // cols=2, rows=2; last row has 1 app → spans full width
    assert_eq!(regions[2].0, 0);   // x=0
    assert_eq!(regions[2].2, 1024); // w=full width
}
```

### Mouse event format

```rust
// In existing dispatch_mouse tests: verify new format strings
assert!(event.starts_with("VYOMA_INPUT:mouse:move:"));
assert!(click_event.starts_with("VYOMA_INPUT:mouse:click:"));
assert!(click_event.ends_with(":left") || click_event.ends_with(":right") || click_event.ends_with(":middle"));
```

### No-overlap invariant

```rust
#[test]
fn no_two_regions_overlap() {
    for n in 1..=9 {
        let regions = compute_tiling(n, 1024, 768);
        for i in 0..regions.len() {
            for j in (i+1)..regions.len() {
                assert!(!regions_overlap(regions[i], regions[j]),
                    "regions {i} and {j} overlap for n={n}");
            }
        }
    }
}
```

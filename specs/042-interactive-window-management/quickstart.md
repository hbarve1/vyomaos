# Quickstart: Interactive Window Management

## Testing Drag-to-Move

1. `make build && make run-gui DISPLAY_BACKEND=cocoa` (or `sdl`)
2. Wait for the system to boot — you should see `shell`, `ticker`, `gui-demo`, and `mouse-demo` in their tiled windows.
3. Hold the left mouse button on the `shell` title bar (the top 28 px strip) and drag it 100 px to the right.
4. Confirm the window follows the cursor and lands at the new position.
5. Release near the original tiled slot (within 40 px) — confirm snap-back.
6. Release far from any slot — confirm the window stays where dropped.

## Testing Close Button

1. Boot as above with at least two display apps visible.
2. Click the red dot (leftmost traffic-light) on any window.
3. Confirm the window disappears within 200 ms.
4. Confirm the remaining window expands to fill the vacated space.
5. If the closed app had focus, confirm keyboard input routes to the next app.

## Testing Minimize / Restore

1. Boot as above.
2. Click the yellow dot (middle traffic-light) on any window.
3. Confirm the window collapses to a 240×28 strip at the bottom of the screen.
4. The strip shows the app name; the app continues running.
5. Click the strip — confirm the window restores to its prior size and position.

## Running Unit Tests

```bash
make unit-test
# or from inside the Docker shell:
cargo test --target x86_64-unknown-linux-musl
```

Expect ≥ 5 new tests in `supervisor/tests/drag_test.rs` to pass.

## Checking the 500-Line Rule

```bash
wc -l supervisor/src/*.rs supervisor/src/**/*.rs
```

No file should exceed 500 lines after this feature lands.

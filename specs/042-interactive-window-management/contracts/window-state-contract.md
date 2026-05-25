# Contract: Window State

This contract describes the invariants the supervisor maintains for per-window state,
extended by the interactive window management feature.

## win_region Invariants

- `win_region` is `Some(x, y, w, h)` for every running display app.
- When `minimized == true`, `win_region` holds the 240×28 strip position (not the full window).
- `win_region` is `None` for apps without `display = true` capability.

## Drag Invariants

- During a drag, `win_region` for the dragged app is updated on every mouse sync event (≤ 33 ms latency).
- `win_region.x` is clamped to `[0, screen_w - win_w]` at all times.
- `win_region.y` is clamped to `[MENUBAR_H, screen_h - win_h]` at all times.
- Drag affects only the dragged app's `win_region`; all other apps' regions are unchanged.

## Snap-Back Contract

- If the dragged window's final `(x, y)` is within 40 px Chebyshev distance of a tiled grid slot, the window snaps to that slot's exact `(x, y, w, h)`.
- The Chebyshev distance is `max(|Δx|, |Δy|)` where Δx and Δy are distances from the window's top-left corner to the slot's top-left corner.
- Snap affects only the dragged window; no other window is moved.

## Minimize Strip Layout

- Minimized strips are 240 px wide × 28 px tall.
- Strips are laid out left-to-right at the screen bottom: strip N starts at `x = N * 240`, `y = screen_h - 28`.
- If `N * 240 >= screen_w`, the strip wraps to `x = (N * 240) % screen_w`, `y = screen_h - 28 - 28 * (N * 240 / screen_w)`.
- `pre_minimize_region` holds the exact `win_region` at minimize time; restore writes it back verbatim.

## Close-Button Post-Kill Contract

After `SIGKILL` is sent:
1. The tiling layout is recomputed for the remaining `N-1` display apps.
2. Keyboard focus is transferred via `auto_transfer_focus()`.
3. The closed app's region on the framebuffer is cleared (fill with background colour).

These steps execute in the app-exit path (`app_threads.rs`), which already runs on app death. The traffic-light close sends the kill signal; the existing exit watcher handles steps 1–3.

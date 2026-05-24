# Contract: Tiled Window Layout

**Version**: 1.0

## Overview

The supervisor auto-computes non-overlapping tiled window regions for all running display-capable apps whenever a display app spawns or exits. Apps do not declare their own screen position; they only declare preferred minimum dimensions as hints.

## Tiling Algorithm

```
columns = ceil(sqrt(n))
rows    = ceil(n / columns)
```

Where `n` = number of currently running display apps (1 ≤ n ≤ 9).

**Base cell dimensions** (before last-row expansion):
```
cell_w = screen_width  / columns
cell_h = screen_height / rows
```

**Region assignment** (left-to-right, top-to-bottom, 0-indexed):
```
for i in 0..n:
    col = i % columns
    row = i / columns
    x   = col * cell_w
    y   = row * cell_h
    w   = cell_w
    h   = cell_h
```

**Last-row expansion** — if the last row has fewer tiles than `columns`, each tile in that row expands to fill the remaining width equally:
```
last_row_start = (rows - 1) * columns
last_row_count = n - last_row_start
expanded_w     = screen_width / last_row_count   (integer division)
// Apply expanded_w to the last last_row_count tiles
// Assign last tile: x = expanded_w * (last_row_count - 1), w = screen_width - that x
// (absorbs remainder pixels to prevent gaps)
```

## Reference Grid Table

| n | cols | rows | Grid description |
|---|------|------|-----------------|
| 1 | 1 | 1 | Full screen |
| 2 | 2 | 1 | Two equal halves side-by-side |
| 3 | 2 | 2 | 2 top + 1 bottom (bottom spans full width) |
| 4 | 2 | 2 | 2×2 equal grid |
| 5 | 3 | 2 | 3 top + 2 bottom (bottom tiles wider) |
| 6 | 3 | 2 | 3×2 equal grid |
| 7 | 3 | 3 | 3+3+1 (last tile full row) |
| 8 | 3 | 3 | 3+3+2 |
| 9 | 3 | 3 | 3×3 equal grid |

## Invariants

1. **Coverage**: Union of all regions = full screen area (zero unclaimed pixels)
2. **Non-overlapping**: No two regions share a pixel
3. **Completeness**: Every display app gets exactly one region (for n ≤ 9)
4. **Pixel-accuracy**: Integer arithmetic; remainder pixels absorbed by rightmost/bottommost tile
5. **Minimum dimensions**: If an app declares `[window] width = W` and `W > cell_w`, supervisor assigns that app a full-row slot (best-effort; may not be achievable if all apps have large minimums)

## Reflow Timing

- Triggered: on every display-app spawn or exit
- Deadline: regions updated and borders repainted within 500 ms of the trigger event
- During reflow: existing app content in the back-buffer is preserved; regions are reassigned then borders redrawn; next `flush()` from any app picks up the new coordinates

## Apps Beyond Slot Limit

- Maximum visible slots: 9
- Apps #10, #11, … are spawned normally (they receive stdio, IPC, etc.) but get `win_region = None`
- Their `VYOMA_DRAW:` output is silently discarded
- When a visible app exits, the first queued app is assigned its slot and begins rendering

## App-Declared Hints (manifest `[window]` section)

```toml
[window]
width  = 320   # preferred minimum width in pixels
height = 240   # preferred minimum height in pixels
title  = "My App"  # reserved; not rendered in this phase
```

The supervisor reads `width` and `height` from the manifest at spawn time and attempts to honour them during layout. `title` is stored but not rendered (no title bar in this phase).

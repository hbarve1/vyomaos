# VyomaOS P18–P30 Roadmap Design

**Date:** 2026-04-11
**Status:** approved
**Current phase:** P17 (real-time shell input via raw TTY)

---

## Goal

Advance VyomaOS from a functional WASM OS prototype (P17) to a capable, windowed, secure, self-updating OS with system apps, mouse input, and app distribution. Progress one phase at a time; independent phases parallelize via git worktrees.

---

## Waves and Phases

### Wave 1 — Usability (P18–P20, fully parallel)

Three independent improvements that make the system more usable without touching shared code paths.

#### P18 — Shell UX: arrow keys + command history

**Problem:** Shell discards ESC sequences; no history means retyping commands.

**Changes:**
- `supervisor/src/main.rs` — input thread already reads raw bytes. Extend ESC sequence parser: `\x1b[A` (↑) and `\x1b[B` (↓) forwarded as-is to focused app instead of being discarded.
- `apps/shell/src/main.rs` — add `history: Vec<String>` (cap 50), `hist_idx: usize`. On `\x1b[A` recall `history[hist_idx-1]` into input buffer; on `\x1b[B` go forward. On Enter push non-empty command to history, reset `hist_idx`. Redraw prompt line on every change (already works).

**Acceptance criteria:**
- `↑` recalls last command into prompt
- `↑↑↑` navigates back through up to 50 entries
- `↓` moves forward; reaching end clears prompt
- `Enter` on empty line is no-op (no crash, no empty history entry)
- History is in-memory only (lost on shell restart)

**Files touched:** `supervisor/src/main.rs`, `apps/shell/src/main.rs`

---

#### P19 — Watchdog: silent-app detection + restart with backoff

**Problem:** An app that crashes or hangs silently (no more stdout) is restarted only if `restart=always`. There's no timeout-based detection for hung apps.

**Changes:**
- `vyoma.toml` manifest — add optional `watchdog_secs: u32` field (0 = disabled). If set, supervisor kills + restarts the app if it produces no stdout for that many seconds.
- `supervisor/src/main.rs` — per-app `last_output: Instant` timestamp updated by reader thread. Watchdog thread (one global, 1-second tick) scans all apps: if `last_output.elapsed() > watchdog_secs` and app is running, kill it; let the restart policy re-spawn it. Exponential backoff: `min(watchdog_secs * 2^restarts, 300)` seconds before next restart attempt.
- Capability manifest: `watchdog_secs` is optional, default 0 (disabled).

**Acceptance criteria:**
- App with `watchdog_secs = 5` that stops printing is killed after 5 seconds
- Restart count increments; backoff doubles each time (5s → 10s → 20s → …, cap 300s)
- Apps with `watchdog_secs = 0` are unaffected
- `ps` output shows `[watchdog]` tag next to apps with watchdog enabled

**Files touched:** `supervisor/src/main.rs`, optionally `apps/ticker/vyoma.toml` (add `watchdog_secs = 10` as demo)

---

#### P20 — Font scaling: multiple glyph sizes

**Problem:** The current 8×16 font is small at 1440×900. No way to render larger text.

**Changes:**
- `supervisor/src/font8x16.rs` — rename to `font.rs`. Add `pub enum FontSize { Small, Medium, Large }` mapping to glyph dimensions 8×8, 8×16, 16×32.
- Scale-up strategy: 16×32 is the 8×16 font pixel-doubled (each bit → 2×2 block). No new font data needed.
- `supervisor/src/display.rs` — extend `draw_text` signature with a size parameter. Update VYOMA_DRAW protocol: `VYOMA_DRAW:draw_text:<x>,<y>,<rgba>,<size>,<text>` where size = `s`/`m`/`l`. Backwards compat: supervisor parser checks if 4th comma-field is one of `s`/`m`/`l`; if not, treats the whole remainder as text with default size `m`. This means existing `draw_text:<x>,<y>,<rgba>,<text>` lines continue to work unchanged.
- `apps/gui-demo/src/main.rs` — update header title to use size `l`; status text stays `m`.
- `apps/shell/src/main.rs` — prompt text stays `m`.

**Acceptance criteria:**
- `VYOMA_DRAW:draw_text:100,100,0xFFFFFFFF,l,Hello` renders at 16×32
- `VYOMA_DRAW:draw_text:100,100,0xFFFFFFFF,m,Hello` renders at 8×16 (unchanged)
- `VYOMA_DRAW:draw_text:100,100,0xFFFFFFFF,s,Hello` renders at 8×8
- Existing apps that omit size field still work (default `m`)
- gui-demo header "VyomaOS" visibly larger

**Files touched:** `supervisor/src/font.rs` (rename), `supervisor/src/display.rs`, `apps/gui-demo/src/main.rs`, `apps/shell/src/main.rs`

---

### Wave 2 — Windowing (P21–P23, sequential)

#### P21 — Window regions: per-app declared viewports

**Problem:** Dashboard (Y=0–440) and shell (Y=450–900) are hardcoded in app source. No way to rearrange without editing apps.

**Changes:**
- `vyoma.toml` manifest — add optional `[window]` section: `x`, `y`, `w`, `h` (pixel coords). If absent, app gets full screen.
- Supervisor `display.rs` — store per-app window rect. Clip all VYOMA_DRAW commands to that rect: offset `x`/`y` by window origin, drop pixels outside bounds.
- Apps `gui-demo` and `shell` — remove hardcoded W/DASH_H/shell region constants; use full declared window dimensions from their own vyoma.toml.

**Acceptance criteria:**
- Moving gui-demo window in vyoma.toml moves the dashboard on screen without code changes
- Two apps can draw to non-overlapping regions simultaneously
- Overlapping windows: last writer wins (no compositor blending needed yet)

**Files touched:** `supervisor/src/main.rs`, `supervisor/src/display.rs`, `apps/gui-demo/vyoma.toml`, `apps/shell/vyoma.toml`, `apps/gui-demo/src/main.rs`, `apps/shell/src/main.rs`

---

#### P22 — Mouse input: virtio-input pointer events

**Problem:** No mouse support; all interaction is keyboard-only.

**Changes:**
- `base/kernel.config` — add `CONFIG_INPUT_EVDEV=y`, `CONFIG_VIRTIO_INPUT=y`.
- QEMU `run-gui` target — add `-device virtio-mouse-pci`.
- Supervisor — new input thread reads `/dev/input/event*` (evdev), decodes `EV_REL`/`EV_ABS`/`EV_KEY` events, tracks global cursor position (x, y), dispatches `VYOMA_INPUT:mouse:<x>,<y>,<btn>` to the app whose window rect contains the cursor (if it has `mouse: true` capability). On headless boot where no `/dev/input/event*` exists, thread logs a warning and exits silently — no crash.
- New capability field: `mouse: bool`.
- New VYOMA_INPUT protocol (stdout of supervisor → stdin of app): `VYOMA_INPUT:mouse:<x>,<y>,<buttons_bitmask>`
- `apps/gui-demo` — add `mouse = true`; highlight card under cursor.

**Acceptance criteria:**
- Mouse movement in QEMU window updates cursor tracking
- Click on a gui-demo app card highlights it
- Apps without `mouse = true` receive no mouse events

**Files touched:** `base/kernel.config`, `Makefile`, `supervisor/src/main.rs`, `apps/gui-demo/src/main.rs`, `apps/gui-demo/vyoma.toml`

---

#### P23 — TUI widget primitives

**Problem:** Every app redraws the entire frame from scratch. No shared primitives for borders, scrollable lists, text wrapping.

**Changes:**
- Extend VYOMA_DRAW protocol:
  - `VYOMA_DRAW:rect_border:<x>,<y>,<w>,<h>,<rgba>` — single-pixel border rectangle
  - `VYOMA_DRAW:draw_text_wrap:<x>,<y>,<max_w>,<rgba>,<size>,<text>` — word-wrap text into max_w
  - `VYOMA_DRAW:clear_region:<x>,<y>,<w>,<h>` — fill with black (convenience)
- `supervisor/src/display.rs` — implement all three.
- Update `apps/shell` to use `rect_border` for panel borders and `clear_region` for redraw.

**Acceptance criteria:**
- Shell panel has a visible 1px border drawn via `rect_border`
- Long text wraps correctly in shell output
- `clear_region` avoids ghost text on redraw

---

### Wave 3 — System Apps (P24–P26, parallel)

#### P24 — File manager app

New WASM app. Reads `/data` directory via WASI filesystem. Renders a scrollable file list using `VYOMA_DRAW:draw_text` + `rect_border`. Keyboard: `↑`/`↓` navigate, `Enter` views file content (first 20 lines).

Capabilities: `filesystem = true`, `display = true`, `shell = true` (keyboard), `[window]` region.

---

#### P25 — Text editor app

New WASM app. Opens a file path argument (from shell `run text-editor /data/foo.txt`). Line buffer, cursor, insert/delete. Saves on `Ctrl+S`. Basic only — no syntax highlighting.

Capabilities: `filesystem = true`, `display = true`, `shell = true`.

---

#### P26 — System monitor app

New WASM app. Queries `@supervisor: ps-raw` every second. Renders per-app uptime, restart count, watchdog status. Replaces or supplements `gui-demo` dashboard with richer data.

Capabilities: `display = true`, `[window]` region.

---

### Wave 4 — Security & Distribution (P27–P30, mostly sequential)

#### P27 — App namespaces

Per-app Linux mount namespace (`clone(CLONE_NEWNS)`) + PID namespace (`CLONE_NEWPID`). Each wasmtime child sees only its declared mounts. Requires `CONFIG_NAMESPACES=y`, `CONFIG_PID_NS=y`, `CONFIG_MNT_NS=y` in kernel.

#### P28 — Signed app bundles

SHA256 manifest field `wasm_sha256` in vyoma.toml. Supervisor verifies binary at load time; refuses to start if hash mismatches. Enables safe package installation from untrusted sources.

#### P29 — Multi-resolution display

Supervisor reads actual `FBIOGET_VSCREENINFO` resolution at boot and broadcasts `VYOMA_SYSTEM:screen:<w>,<h>` to all display apps before first draw. Apps use this to scale their layouts instead of hardcoding 1440×900.

#### P30 — OTA hot-swap

`@supervisor: update <app> <url>` command. Supervisor downloads WASM binary via HTTP (using the http-server app's socket, or a new built-in downloader), verifies SHA256, replaces `/data/apps/<app>/<app>.wasm`, and restarts the app. No reboot needed.

---

## Parallelization Map

```
Wave 1 (now):
  worktree/p18-shell-ux   ← apps/shell/src/main.rs + supervisor input parser
  worktree/p19-watchdog   ← supervisor/src/main.rs (watchdog thread only)
  worktree/p20-font-scale ← supervisor/src/font.rs + display.rs + apps headers

Wave 2 (after Wave 1 merged):
  worktree/p21-windowing  ← sequential
  worktree/p22-mouse      ← after p21
  worktree/p23-widgets    ← after p21

Wave 3 (after Wave 2 merged):
  worktree/p24-file-mgr   ← parallel with p25, p26
  worktree/p25-text-edit  ← parallel with p24, p26
  worktree/p26-sysmon     ← parallel with p24, p25

Wave 4 (after Wave 3 merged):
  sequential: p27 → p28 → p29 → p30
```

## Commit convention

`feat: PXX — <description>`
`fix: PXX — <description>`
`test: PXX — <description>`

## Testing strategy

- **Unit:** `cargo test` in Docker for supervisor and each app (where logic is pure)
- **Integration:** `make build && make run` in QEMU headless; capture serial output; assert expected lines appear
- **Per-phase gate:** build must pass + serial output must show all apps started before marking phase complete

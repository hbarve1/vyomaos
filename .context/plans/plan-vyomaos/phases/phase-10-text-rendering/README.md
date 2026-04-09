# Phase 10 — Text Rendering

## Goal

Extend the VYOMA_DRAW protocol with a `draw_text` command so WASM apps can render readable labels, status lines, and counters directly on the framebuffer. The supervisor embeds a compact 8×16 bitmap font as a const byte array — no file I/O, no external font crate. `gui-demo` is updated to label every visual block with the app name and live data (boot count, IPC message count, etc.).

## Gate

```sh
make run-gui DISPLAY_BACKEND=cocoa
# QEMU window shows:
#   - "VyomaOS" title text in the header bar
#   - App name labels inside each coloured block
#   - Boot count number rendered on screen
#   - All text pixel-perfect, no garbled characters
```

## Why this phase before networking

The GUI currently displays meaningful shapes but no text — the dashboard communicates nothing to a first-time viewer. Text rendering closes the visual story in one small, well-scoped phase with zero new kernel, QEMU, or build-system changes. It also unblocks Phase 12 (interactive shell needs text output).

## Architecture

```
WASM app stdout:
  VYOMA_DRAW:draw_text:x,y,text,rgba

Supervisor reader thread (supervisor/src/display.rs):
  parse → font lookup → blit 8×16 glyph pixels to mmaped fb0

Font data (supervisor/src/font8x16.rs):
  pub const FONT: &[u8; 128 * 16] = &[ ... ];
  // 2048 bytes, ASCII 0x00–0x7F
  // Each char = 16 bytes (one per row), MSB = leftmost pixel
```

## Protocol addition

```
VYOMA_DRAW:draw_text:<x>,<y>,<rgba_decimal>,<text>
```

- `x`, `y` — top-left pixel of the first character
- `rgba_decimal` — colour as decimal 0xRRGGBBAA u32
- `text` — remainder of the line after the third comma (may contain commas)

## Dependencies

- Phase 09 (VYOMA_DRAW fill_rect/flush infrastructure in place)
- No new kernel config, no new QEMU flags, no new Cargo deps

## Tasks

### P10T01 — Embed 8×16 Bitmap Font
[tasks/task-P10T01-embed-font.md](tasks/task-P10T01-embed-font.md)

Encode the classic 8×16 Linux console font as `pub const FONT: &[u8; 2048]` in a new `supervisor/src/font8x16.rs` module. Cover full printable ASCII (0x20–0x7E) plus space.

### P10T02 — draw_text Command in Display Module
[tasks/task-P10T02-draw-text-command.md](tasks/task-P10T02-draw-text-command.md)

Add `Framebuffer::draw_text(x, y, text, rgba)` to `supervisor/src/display.rs`. Parse `VYOMA_DRAW:draw_text:` in `handle_draw_command`. Text is clipped at framebuffer edges; non-printable ASCII is rendered as a blank glyph.

### P10T03 — Update gui-demo with Labels
[tasks/task-P10T03-gui-demo-labels.md](tasks/task-P10T03-gui-demo-labels.md)

Rewrite `apps/gui-demo/src/main.rs` to render the OS title, per-app block labels, and boot metadata using `VYOMA_DRAW:draw_text`. Read `boot_count` from `/data/boot_count.txt` if `filesystem = true` is granted, or hard-code `"boot #?"` otherwise.

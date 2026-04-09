# P10T02 — draw_text Command in Display Module

## Phase
Phase 10 — Text Rendering

## Goal
Add `Framebuffer::draw_text(x, y, text, rgba)` to `supervisor/src/display.rs` and wire it into `handle_draw_command` so that apps can render text via `VYOMA_DRAW:draw_text:x,y,rgba,text`.

## Files to modify

```
supervisor/src/display.rs   — add draw_text method, import font8x16
supervisor/src/main.rs      — add draw_text arm in handle_draw_command
```

## Protocol

```
VYOMA_DRAW:draw_text:<x>,<y>,<rgba_decimal>,<text_payload>
```

The `text_payload` is everything after the third comma — this allows text that itself contains commas. Parse with `splitn(4, ',')`.

## Implementation

### `supervisor/src/display.rs` — draw_text method

```rust
use super::font8x16::{FONT, GLYPH_W, GLYPH_H, FIRST_CHAR, LAST_CHAR};

impl Framebuffer {
    /// Render a string at pixel position (x, y).
    /// Characters outside printable ASCII are drawn as blank glyphs.
    /// Text is clipped at the right and bottom framebuffer edges.
    pub fn draw_text(&mut self, x: u32, y: u32, text: &str, rgba: u32) {
        if self.bpp != 32 { return; }
        let r = ((rgba >> 24) & 0xFF) as u8;
        let g = ((rgba >> 16) & 0xFF) as u8;
        let b = ((rgba >>  8) & 0xFF) as u8;
        let fg = [b, g, r, 0xFF_u8]; // BGRA

        let mut cx = x;
        for ch in text.chars() {
            if cx + GLYPH_W > self.width { break; }

            let glyph_idx = if ch as u32 >= FIRST_CHAR as u32
                            && ch as u32 <= LAST_CHAR as u32 {
                (ch as usize - FIRST_CHAR as usize) * GLYPH_H as usize
            } else {
                0 // blank glyph for out-of-range chars
            };

            for row in 0..GLYPH_H {
                let scan_y = y + row;
                if scan_y >= self.height { break; }
                let byte = FONT[glyph_idx + row as usize];
                for bit in 0..GLYPH_W {
                    if byte & (0x80 >> bit) != 0 {
                        let px = cx + bit;
                        let off = (scan_y * self.stride + px * 4) as usize;
                        if off + 4 <= self.buf_len {
                            unsafe {
                                std::ptr::copy_nonoverlapping(
                                    fg.as_ptr(), self.buf.add(off), 4,
                                );
                            }
                        }
                    }
                }
            }
            cx += GLYPH_W;
        }
    }
}
```

### `supervisor/src/main.rs` — handle_draw_command extension

```rust
if let Some(args) = cmd.strip_prefix("draw_text:") {
    // format: x,y,rgba,text (text may contain commas)
    let parts: Vec<&str> = args.splitn(4, ',').collect();
    if let [xs, ys, cs, text] = parts.as_slice() {
        if let (Ok(x), Ok(y), Ok(rgba)) = (
            xs.parse::<u32>(), ys.parse::<u32>(), cs.parse::<u32>()
        ) {
            fb_lock.lock().unwrap().draw_text(x, y, text, rgba);
        }
    } else {
        eprintln!("vyoma-display: [{sender}] bad draw_text args: {args}");
    }
    return;
}
```

## Notes

- Text is rendered in foreground colour only (no background fill behind glyphs). Apps should call `fill_rect` first to clear the area if needed.
- Newlines in `text` are not handled — the caller is responsible for splitting multi-line text into separate `draw_text` calls with incremented `y` values.

## Verification

```sh
# After make run-gui: verify text appears in QEMU window
# gui-demo (updated in P10T03) will call draw_text; check for readable labels
```

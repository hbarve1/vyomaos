// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.
//! VyomaOS framebuffer display module (Linux only)
//!
//! Opened once at supervisor startup.  Reader threads call `fill_rect` /
//! `flush` when they detect `VYOMA_DRAW:` protocol lines from display-capable
//! apps.  If `/dev/fb0` is absent (headless boot) every call is a silent
//! no-op.
//!
//! Protocol (one command per stdout line from a display-capable WASM app):
//!   VYOMA_DRAW:fill_rect:<x>,<y>,<w>,<h>,<rgba_decimal>
//!   VYOMA_DRAW:draw_text:<x>,<y>,<rgba_decimal>,<text>
//!   VYOMA_DRAW:flush
mod cursor;
mod fb_ioctl;
mod helpers;
mod screenshot;
pub use cursor::{CursorState, CURSOR_W, CURSOR_H, CURSOR_MASK};
// blend_alpha and titlebar_color are used by tests and helper modules;
// the binary path does not call them directly but they are part of the public API.
#[allow(unused_imports)]
pub use helpers::{blend_alpha, titlebar_color, border_color, app_accent_color, format_fps, wrap_words};
use fb_ioctl::{FBIOGET_VSCREENINFO, FbVarScreeninfo};
use super::font;
use std::{
    fs::OpenOptions,
    io,
    os::unix::io::AsRawFd,
    sync::{Mutex, OnceLock},
    thread,
    time::Duration,
};
// ── Dirty-region tracking ─────────────────────────────────────────────────────
/// Bounding scanline range for back-buffer writes since the last flush.
/// Empty state: `y0 > y1` (use `DirtyRect::empty(height)`).
#[derive(Clone, Copy)]
pub struct DirtyRect {
    pub y0: u32,
    pub y1: u32,
}

impl DirtyRect {
    pub fn empty(height: u32) -> Self { Self { y0: height, y1: 0 } }
    pub fn is_empty(&self) -> bool { self.y0 > self.y1 }
    pub fn expand(&mut self, row_start: u32, row_end_inclusive: u32) {
        self.y0 = self.y0.min(row_start);
        self.y1 = self.y1.max(row_end_inclusive);
    }
}
// ── Framebuffer handle ────────────────────────────────────────────────────────
pub struct Framebuffer {
    _file: std::fs::File, // keeps the fd alive
    pub width: u32,
    pub height: u32,
    stride: u32,          // bytes per scanline
    bpp: u32,
    buf: *mut u8,
    buf_len: usize,
    pub back: Vec<u8>,    // back-buffer; blitted to buf on flush()
    pub dirty: DirtyRect, // modified scanline range since last flush
    pub cursor: CursorState,
    mmaped: bool,         // false for test-only heap-allocated instances
}
// All mutable access is serialised through `Mutex<Framebuffer>`.
unsafe impl Send for Framebuffer {}
static FB: OnceLock<Mutex<Framebuffer>> = OnceLock::new();
// ── Public API ────────────────────────────────────────────────────────────────
/// Open `/dev/fb0`, mmap the pixel buffer.
/// Retries up to 5× with 200 ms delay to handle DRM async init at boot.
/// Returns `true` if the display is ready.
pub fn init() -> bool {
    for attempt in 0..5u32 {
        match open_fb() {
            Ok(fb) => {
                let _ = FB.set(Mutex::new(fb));
                return true;
            }
            Err(e) => {
                if attempt < 4 {
                    thread::sleep(Duration::from_millis(200));
                } else {
                    eprintln!("vyoma-display: /dev/fb0 unavailable ({e}); GUI disabled");
                }
            }
        }
    }
    false
}
/// Return the global framebuffer, or `None` if GUI is not available.
pub fn get() -> Option<&'static Mutex<Framebuffer>> {
    FB.get()
}
/// Return the actual framebuffer resolution read via FBIOGET_VSCREENINFO.
/// Returns `None` on headless boots where `/dev/fb0` was not opened.
pub fn screen_size() -> Option<(u32, u32)> {
    FB.get().map(|m| {
        let fb = m.lock().unwrap();
        (fb.width, fb.height)
    })
}
/// Update cursor position (clamped to screen bounds).
pub fn set_cursor_pos(cx: i32, cy: i32) {
    if let Some(m) = FB.get() {
        let mut fb = m.lock().unwrap();
        let max_x = fb.width  as i32 - 1;
        let max_y = fb.height as i32 - 1;
        fb.cursor.cx = cx.clamp(0, max_x);
        fb.cursor.cy = cy.clamp(0, max_y);
    }
}
/// Enable cursor visibility (called once a mouse device is found).
pub fn enable_cursor() {
    if let Some(m) = FB.get() {
        let mut fb = m.lock().unwrap();
        fb.cursor.visible = true;
        let (cx, cy) = (fb.cursor.cx, fb.cursor.cy);
        eprintln!("cursor: sprite enabled at ({cx},{cy})");
    }
}
/// Blit only the cursor sprite region (fast path for mouse motion).
#[allow(dead_code)]
pub fn flush_cursor_only() {
    if let Some(m) = FB.get() { m.lock().unwrap().flush_cursor_only(); }
}
// ── Framebuffer open + mmap ───────────────────────────────────────────────────
fn open_fb() -> io::Result<Framebuffer> {
    let file = OpenOptions::new().read(true).write(true).open("/dev/fb0")?;
    let fd = file.as_raw_fd();
    let mut var: FbVarScreeninfo = unsafe { std::mem::zeroed() };
    let (width, height, bpp) = if unsafe {
        libc::ioctl(fd, FBIOGET_VSCREENINFO,
                    &mut var as *mut FbVarScreeninfo as *mut libc::c_void)
    } >= 0 && var.xres > 0 {
        (var.xres, var.yres, var.bits_per_pixel)
    } else {
        eprintln!("vyoma-display: FBIOGET_VSCREENINFO failed, using 1024×768 defaults");
        (1024, 768, 32)
    };
    let stride = width * bpp.max(8) / 8;
    let buf_len = (stride * height) as usize;
    let buf = unsafe {
        libc::mmap(
            std::ptr::null_mut(),
            buf_len,
            libc::PROT_READ | libc::PROT_WRITE,
            libc::MAP_SHARED,
            fd,
            0,
        )
    };

    if buf == libc::MAP_FAILED {
        return Err(io::Error::last_os_error());
    }

    eprintln!(
        "vyoma-display: {}×{} {}bpp stride={} ({} KiB mmaped)",
        width, height, bpp, stride, buf_len / 1024
    );

    let back = vec![0u8; buf_len];
    let dirty = DirtyRect::empty(height);
    let cursor = CursorState {
        cx:          (width / 2) as i32,
        cy:          (height / 2) as i32,
        visible:     false,
        saved_under: vec![0u8; (CURSOR_W * CURSOR_H * 4) as usize],
        drawn:       false,
    };
    Ok(Framebuffer { _file: file, width, height, stride, bpp, buf: buf as *mut u8, buf_len, back, dirty, cursor, mmaped: true })
}

impl Drop for Framebuffer {
    fn drop(&mut self) {
        if self.mmaped {
            unsafe { libc::munmap(self.buf as *mut libc::c_void, self.buf_len); }
        } else {
            use std::alloc::{dealloc, Layout};
            if self.buf_len > 0 {
                let layout = Layout::from_size_align(self.buf_len, 4).unwrap();
                unsafe { dealloc(self.buf, layout); }
            }
        }
    }
}
// ── Drawing primitives ────────────────────────────────────────────────────────
impl Framebuffer {
    /// Fill a rectangle with an RGBA colour (0xRRGGBBAA, big-endian).
    /// virtio-gpu framebuffer is XRGB8888 little-endian → stored as [B, G, R, X].
    pub fn fill_rect(&mut self, x: u32, y: u32, w: u32, h: u32, rgba: u32) {
        if self.bpp != 32 {
            return;
        }
        let r = ((rgba >> 24) & 0xFF) as u8;
        let g = ((rgba >> 16) & 0xFF) as u8;
        let b = ((rgba >>  8) & 0xFF) as u8;
        let pixel = [b, g, r, 0xFF_u8]; // BGRA on-disk

        let x1 = (x + w).min(self.width);
        let y1 = (y + h).min(self.height);

        if y1 > y {
            self.dirty.expand(y, y1 - 1);
        }

        for row in y..y1 {
            let base = (row * self.stride + x * 4) as usize;
            let end  = (row * self.stride + x1 * 4) as usize;
            if end > self.buf_len {
                break;
            }
            for off in (base..end).step_by(4) {
                self.back[off..off + 4].copy_from_slice(&pixel);
            }
        }
    }

    /// Render a string at pixel position (x, y) using the embedded bitmap font.
    /// Supports three sizes: Small (8×8), Medium (8×16), Large (16×32).
    /// Characters outside printable ASCII (0x20–0x7E) are drawn as blank glyphs.
    /// Text is clipped at the right and bottom framebuffer edges.
    pub fn draw_text(&mut self, x: u32, y: u32, text: &str, rgba: u32, size: font::FontSize) {
        if self.bpp != 32 { return; }
        let r = ((rgba >> 24) & 0xFF) as u8;
        let g = ((rgba >> 16) & 0xFF) as u8;
        let b = ((rgba >>  8) & 0xFF) as u8;
        let fg = [b, g, r, 0xFF_u8]; // BGRA little-endian

        let (glyph_w, glyph_h) = font::glyph_dims(size);
        let y_end = (y + glyph_h - 1).min(self.height.saturating_sub(1));
        if y < self.height {
            self.dirty.expand(y, y_end);
        }
        let mut cx = x;

        for ch in text.chars() {
            if cx + glyph_w > self.width { break; }

            let glyph_base = if (ch as u32) >= font::FIRST_CHAR as u32
                             && (ch as u32) <= font::LAST_CHAR as u32 {
                (ch as usize - font::FIRST_CHAR as usize) * font::GLYPH_H as usize
            } else {
                0 // blank glyph for out-of-range chars
            };

            match size {
                font::FontSize::Medium => {
                    for row in 0..font::GLYPH_H {
                        let scan_y = y + row;
                        if scan_y >= self.height { break; }
                        let byte = font::FONT[glyph_base + row as usize];
                        for bit in 0..font::GLYPH_W {
                            if byte & (0x80 >> bit) != 0 {
                                let px = cx + bit;
                                let off = (scan_y * self.stride + px * 4) as usize;
                                if off + 4 <= self.buf_len {
                                    self.back[off..off + 4].copy_from_slice(&fg);
                                }
                            }
                        }
                    }
                }
                font::FontSize::Small => {
                    // 8×8: sample every other row of the 8×16 glyph (rows 0,2,4,...14)
                    for out_row in 0..8u32 {
                        let src_row = out_row * 2;
                        let scan_y = y + out_row;
                        if scan_y >= self.height { break; }
                        let byte = font::FONT[glyph_base + src_row as usize];
                        for out_bit in 0..8u32 {
                            if byte & (0x80 >> out_bit) != 0 {
                                let px = cx + out_bit;
                                let off = (scan_y * self.stride + px * 4) as usize;
                                if off + 4 <= self.buf_len {
                                    self.back[off..off + 4].copy_from_slice(&fg);
                                }
                            }
                        }
                    }
                }
                font::FontSize::Large => {
                    // 16×32: pixel-double the 8×16 glyph (each bit → 2×2 block)
                    for src_row in 0..font::GLYPH_H {
                        let byte = font::FONT[glyph_base + src_row as usize];
                        for rep in 0..2u32 {
                            let scan_y = y + src_row * 2 + rep;
                            if scan_y >= self.height { break; }
                            for src_bit in 0..font::GLYPH_W {
                                if byte & (0x80 >> src_bit) != 0 {
                                    for rep_x in 0..2u32 {
                                        let px = cx + src_bit * 2 + rep_x;
                                        let off = (scan_y * self.stride + px * 4) as usize;
                                        if off + 4 <= self.buf_len {
                                            self.back[off..off + 4].copy_from_slice(&fg);
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
            cx += glyph_w;
        }
    }

    /// Save pixels under the cursor hotspot into `cursor.saved_under`, then
    /// paint the arrow sprite (white fill with 1px black outline) at `cursor.cx/cy`.
    /// No-op when `!cursor.visible` or bpp != 32.
    pub fn draw_cursor(&mut self) {
        if !self.cursor.visible || self.bpp != 32 {
            return;
        }
        let cx = self.cursor.cx as u32;
        let cy = self.cursor.cy as u32;

        let mut saved_idx = 0usize;
        for row in 0..CURSOR_H {
            let py = cy + row;
            if py >= self.height { break; }
            for col in 0..CURSOR_W {
                let px = cx + col;
                if px >= self.width { break; }
                let off = (py * self.stride + px * 4) as usize;
                if off + 4 <= self.buf_len && saved_idx + 4 <= self.cursor.saved_under.len() {
                    self.cursor.saved_under[saved_idx..saved_idx + 4]
                        .copy_from_slice(&self.back[off..off + 4]);
                }
                saved_idx += 4;
            }
        }
        self.cursor.drawn = true;

        for row in 0..CURSOR_H as usize {
            let py = cy + row as u32;
            if py >= self.height { break; }
            let mask = CURSOR_MASK[row];
            for col in 0..CURSOR_W as usize {
                if mask & (0x8000 >> col) == 0 { continue; }
                let px = cx + col as u32;
                if px >= self.width { break; }
                let off = (py * self.stride + px * 4) as usize;
                if off + 4 > self.buf_len { continue; }
                let is_edge = col == 0 || row == 0
                    || (col > 0 && CURSOR_MASK[row] & (0x8000 >> (col - 1)) == 0)
                    || (row > 0 && CURSOR_MASK[row - 1] & (0x8000 >> col) == 0)
                    || (col + 1 < CURSOR_W as usize && CURSOR_MASK[row] & (0x8000 >> (col + 1)) == 0)
                    || (row + 1 < CURSOR_H as usize && CURSOR_MASK[row + 1] & (0x8000 >> col) == 0);
                let pixel: [u8; 4] = if is_edge {
                    [0x00, 0x00, 0x00, 0xFF] // black outline (BGRA)
                } else {
                    [0xFF, 0xFF, 0xFF, 0xFF] // white fill
                };
                self.back[off..off + 4].copy_from_slice(&pixel);
            }
        }
    }

    /// Restore pixels that were saved by the most recent `draw_cursor()` call.
    /// No-op when `cursor.drawn` is false.
    pub fn restore_under_cursor(&mut self) {
        if !self.cursor.drawn { return; }
        let cx = self.cursor.cx as u32;
        let cy = self.cursor.cy as u32;
        let mut saved_idx = 0usize;
        for row in 0..CURSOR_H {
            let py = cy + row;
            if py >= self.height { break; }
            for col in 0..CURSOR_W {
                let px = cx + col;
                if px >= self.width { break; }
                let off = (py * self.stride + px * 4) as usize;
                if off + 4 <= self.buf_len && saved_idx + 4 <= self.cursor.saved_under.len() {
                    self.back[off..off + 4]
                        .copy_from_slice(&self.cursor.saved_under[saved_idx..saved_idx + 4]);
                }
                saved_idx += 4;
            }
        }
        self.cursor.drawn = false;
    }

    /// Blit back-buffer to the mmap'd framebuffer (front-buffer).
    /// Composites the cursor sprite on top before blitting, then restores the
    /// back-buffer so subsequent draw ops see a clean canvas.
    /// Only the dirty scanline range is copied to avoid blitting the full frame.
    pub fn flush(&mut self) {
        self.restore_under_cursor();
        self.draw_cursor();

        // Ensure cursor sprite rows are included in the dirty region so the
        // partial blit always covers the cursor even if no app content changed.
        if self.cursor.visible && self.bpp == 32 {
            let cy = self.cursor.cy as u32;
            let cy_end = (cy + CURSOR_H - 1).min(self.height.saturating_sub(1));
            self.dirty.expand(cy, cy_end);
        }

        if !self.dirty.is_empty() {
            let y0    = self.dirty.y0 as usize;
            let y1    = (self.dirty.y1 as usize + 1).min(self.height as usize);
            let start = y0 * self.stride as usize;
            let end   = y1 * self.stride as usize;
            if end <= self.buf_len {
                unsafe {
                    std::ptr::copy_nonoverlapping(
                        self.back.as_ptr().add(start),
                        self.buf.add(start),
                        end - start,
                    );
                }
            }
            self.dirty = DirtyRect::empty(self.height);
        }

        self.restore_under_cursor();
    }

    /// Blit cursor sprite region only; does not consume the dirty rect.
    #[allow(dead_code)]
    pub fn flush_cursor_only(&mut self) {
        if !self.cursor.visible || self.bpp != 32 { return; }
        self.restore_under_cursor();
        self.draw_cursor();
        let cx = self.cursor.cx as u32;
        let cy = self.cursor.cy as u32;
        let x1 = (cx + CURSOR_W).min(self.width);
        let y1 = (cy + CURSOR_H).min(self.height);
        for row in cy..y1 {
            let start = (row * self.stride + cx * 4) as usize;
            let end   = (row * self.stride + x1 * 4) as usize;
            if end <= self.buf_len { unsafe {
                std::ptr::copy_nonoverlapping(
                    self.back.as_ptr().add(start), self.buf.add(start), end - start);
            }}
        }
        self.restore_under_cursor();
    }

    /// Draw a 1-pixel border rectangle (no fill).
    pub fn rect_border(&mut self, x: u32, y: u32, w: u32, h: u32, rgba: u32) {
        if w == 0 || h == 0 { return; }
        self.fill_rect(x, y, w, 1, rgba);              // top
        self.fill_rect(x, y + h - 1, w, 1, rgba);      // bottom
        self.fill_rect(x, y, 1, h, rgba);              // left
        self.fill_rect(x + w - 1, y, 1, h, rgba);      // right
    }

    /// Fill a region with solid black (clear).
    pub fn clear_region(&mut self, x: u32, y: u32, w: u32, h: u32) {
        self.fill_rect(x, y, w, h, 0x000000FF);
    }

    /// Construct a Framebuffer backed by heap memory (no /dev/fb0 required).
    /// Used by integration tests.
    #[doc(hidden)]
    #[allow(dead_code)]
    pub fn new_for_test(width: u32, height: u32) -> (Self, Vec<u8>) {
        use std::alloc::{alloc_zeroed, Layout};
        let bpp = 32u32;
        let stride = width * 4;
        let buf_len = (stride * height) as usize;
        let layout = Layout::from_size_align(buf_len, 4).unwrap();
        let buf = unsafe { alloc_zeroed(layout) };
        let back = vec![0u8; buf_len];
        let dirty = DirtyRect::empty(height);
        let cursor = CursorState {
            cx:          (width / 2) as i32,
            cy:          (height / 2) as i32,
            visible:     true,
            saved_under: vec![0u8; (CURSOR_W * CURSOR_H * 4) as usize],
            drawn:       false,
        };
        let file = std::fs::File::open("/dev/null").unwrap();
        let fb = Self { _file: file, width, height, stride, bpp, buf, buf_len, back, dirty, cursor, mmaped: false };
        (fb, vec![0u8; buf_len])
    }

    /// Draw word-wrapped text. Each line is `glyph_h` pixels tall.
    /// `max_w` is the available width in pixels; wraps at character boundaries.
    pub fn draw_text_wrap(
        &mut self,
        x: u32,
        y: u32,
        max_w: u32,
        text: &str,
        rgba: u32,
        size: font::FontSize,
    ) {
        if self.bpp != 32 { return; }
        let (glyph_w, glyph_h) = font::glyph_dims(size);
        let max_chars = if glyph_w > 0 { (max_w / glyph_w) as usize } else { 0 };
        for (i, line) in wrap_words(text, max_chars).into_iter().enumerate() {
            let ly = y + i as u32 * glyph_h;
            if ly + glyph_h > self.height { break; }
            self.draw_text(x, ly, &line, rgba, size);
        }
    }
}

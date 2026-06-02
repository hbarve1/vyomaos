// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

//! VyomaOS framebuffer display module (Linux only).
//! Silent no-op when `/dev/fb0` is absent (headless boot).

mod cursor;
mod fb_ioctl;
mod helpers;
mod compositor;
pub mod animator;
pub mod surface;
#[allow(unused_imports)] pub use surface::{Surface, blit_surface};

pub use cursor::{CursorState, CURSOR_W, CURSOR_H, CURSOR_MASK};
#[allow(unused_imports)] pub use helpers::{blend_alpha, titlebar_color, border_color, app_accent_color, format_fps, wrap_words};
use fb_ioctl::{FBIOGET_VSCREENINFO, FbVarScreeninfo};
use super::font;
#[allow(unused_imports)] pub use compositor::{composite_glyph, blit_image, draw_rounded_rect, rounded_rect_coverage, composite_frame, CompositeEntry};
use compositor::{blend_over, read_bgra, write_bgra};

use std::{
    fs::OpenOptions,
    io,
    os::unix::io::AsRawFd,
    sync::{Mutex, OnceLock},
    thread,
    time::Duration,
};

use crate::lock_or_recover;

pub struct Framebuffer {
    _file: std::fs::File, // keeps the fd alive
    pub width: u32,
    pub height: u32,
    pub stride: u32,      // bytes per scanline
    bpp: u32,
    buf: *mut u8,
    buf_len: usize,
    pub back: Vec<u8>,    // back-buffer; blitted to buf on flush()
    pub cursor: CursorState,
    mmaped: bool,         // false for test-only heap-allocated instances
    dirty_top: u32,       // first dirty row since last flush (> dirty_bottom = clean)
    dirty_bottom: u32,    // one past last dirty row
}

// All mutable access is serialised through `Mutex<Framebuffer>`.
unsafe impl Send for Framebuffer {}

static FB: OnceLock<Mutex<Framebuffer>> = OnceLock::new();

// ── Public API ────────────────────────────────────────────────────────────────

/// Open `/dev/fb0`, mmap the pixel buffer. Retries up to 5x.
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
        let fb = lock_or_recover(&m);
        (fb.width, fb.height)
    })
}

/// Update cursor position (clamped to screen bounds).
pub fn set_cursor_pos(cx: i32, cy: i32) {
    if let Some(m) = FB.get() {
        let mut fb = lock_or_recover(&m);
        let max_x = fb.width  as i32 - 1;
        let max_y = fb.height as i32 - 1;
        fb.cursor.cx = cx.clamp(0, max_x);
        fb.cursor.cy = cy.clamp(0, max_y);
    }
}

/// Enable cursor visibility (called once a mouse device is found).
pub fn enable_cursor() {
    if let Some(m) = FB.get() {
        let mut fb = lock_or_recover(&m);
        fb.cursor.visible = true;
        let (cx, cy) = (fb.cursor.cx, fb.cursor.cy);
        eprintln!("cursor: sprite enabled at ({cx},{cy})");
    }
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
        libc::mmap(std::ptr::null_mut(), buf_len,
            libc::PROT_READ | libc::PROT_WRITE, libc::MAP_SHARED, fd, 0)
    };

    if buf == libc::MAP_FAILED {
        return Err(io::Error::last_os_error());
    }

    eprintln!(
        "vyoma-display: {}×{} {}bpp stride={} ({} KiB mmaped)",
        width, height, bpp, stride, buf_len / 1024
    );

    let back = vec![0u8; buf_len];
    let cursor = CursorState {
        cx:          (width / 2) as i32,
        cy:          (height / 2) as i32,
        visible:     false,
        saved_under: vec![0u8; (CURSOR_W * CURSOR_H * 4) as usize],
        drawn:       false,
    };
    Ok(Framebuffer { _file: file, width, height, stride, bpp, buf: buf as *mut u8, buf_len, back, cursor, mmaped: true, dirty_top: height, dirty_bottom: 0 })
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
    /// Fill a rectangle with an RGBA colour (packed 0xRRGGBBAA). Alpha-blended.
    pub fn fill_rect(&mut self, x: u32, y: u32, w: u32, h: u32, rgba: u32) {
        if self.bpp != 32 { return; }
        let a = (rgba & 0xFF) as u8;
        let x1 = (x + w).min(self.width);
        let y1 = (y + h).min(self.height);

        // Fast path: fully opaque fill — no blending needed
        if a == 255 {
            let r = ((rgba >> 24) & 0xFF) as u8;
            let g = ((rgba >> 16) & 0xFF) as u8;
            let b = ((rgba >>  8) & 0xFF) as u8;
            let pixel = [b, g, r, 0xFF_u8];
            for row in y..y1 {
                let base = (row * self.stride + x * 4) as usize;
                let end  = (row * self.stride + x1 * 4) as usize;
                if end > self.buf_len { break; }
                for off in (base..end).step_by(4) {
                    self.back[off..off + 4].copy_from_slice(&pixel);
                }
            }
        } else {
            // Alpha-blended fill using Porter-Duff "over"
            for row in y..y1 {
                let base = (row * self.stride + x * 4) as usize;
                let end  = (row * self.stride + x1 * 4) as usize;
                if end > self.buf_len { break; }
                for off in (base..end).step_by(4) {
                    let dst = read_bgra(&self.back, off);
                    let blended = blend_over(rgba, dst);
                    write_bgra(&mut self.back, off, blended);
                }
            }
        }

        if y1 > y {
            self.dirty_top    = self.dirty_top.min(y);
            self.dirty_bottom = self.dirty_bottom.max(y1);
        }
    }

    /// Render a string at (x, y) using the embedded bitmap font. Clips at edges.
    pub fn draw_text(&mut self, x: u32, y: u32, text: &str, rgba: u32, size: font::FontSize) {
        if self.bpp != 32 { return; }
        let r = ((rgba >> 24) & 0xFF) as u8;
        let g = ((rgba >> 16) & 0xFF) as u8;
        let b = ((rgba >>  8) & 0xFF) as u8;
        let fg = [b, g, r, 0xFF_u8]; // BGRA little-endian

        let (glyph_w, _glyph_h) = font::glyph_dims(size);
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

        let (_, glyph_h) = font::glyph_dims(size);
        let y1 = (y + glyph_h).min(self.height);
        if y1 > y {
            self.dirty_top    = self.dirty_top.min(y);
            self.dirty_bottom = self.dirty_bottom.max(y1);
        }
    }

    /// Save pixels under cursor hotspot then paint the arrow sprite.
    pub fn draw_cursor(&mut self) {
        if !self.cursor.visible || self.bpp != 32 { return; }
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

    /// Restore pixels saved by the most recent `draw_cursor()` call.
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

    /// Blit dirty rows + cursor rows from back-buffer to the mmap'd framebuffer.
    pub fn flush(&mut self) {
        self.restore_under_cursor();
        self.draw_cursor();

        // Expand dirty bounds to include the cursor sprite rows.
        let cur_top = self.cursor.cy.max(0) as u32;
        let cur_bot = (self.cursor.cy as u32 + CURSOR_H).min(self.height);
        let row_top = self.dirty_top.min(cur_top);
        let row_bot = self.dirty_bottom.max(cur_bot);

        if row_top < row_bot {
            let start = (row_top * self.stride) as usize;
            let end   = ((row_bot * self.stride) as usize).min(self.buf_len);
            unsafe {
                std::ptr::copy_nonoverlapping(
                    self.back.as_ptr().add(start),
                    self.buf.add(start),
                    end - start,
                );
            }
        }

        // Reset dirty tracking; restore cursor pixels in back-buffer.
        self.dirty_top    = self.height;
        self.dirty_bottom = 0;
        self.restore_under_cursor();
    }

    /// Composite surfaces onto the back-buffer and flip to the mmap'd fb.
    #[allow(dead_code)]
    pub fn composite_and_flip(
        &mut self,
        bg_color: u32,
        surfaces: &[compositor::CompositeEntry<'_>],
    ) {
        let (w, h) = (self.width, self.height);
        self.fill_rect(0, 0, w, h, bg_color);
        compositor::composite_frame(
            &mut self.back, surfaces, self.stride, self.width, self.height,
        );
        // Mark entire screen dirty so flush copies everything.
        self.dirty_top = 0;
        self.dirty_bottom = self.height;
        self.flush();
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

    /// Construct a heap-backed Framebuffer for tests (no /dev/fb0 required).
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
        let cursor = CursorState {
            cx:          (width / 2) as i32,
            cy:          (height / 2) as i32,
            visible:     true,
            saved_under: vec![0u8; (CURSOR_W * CURSOR_H * 4) as usize],
            drawn:       false,
        };
        let file = std::fs::File::open("/dev/null").unwrap();
        let fb = Self { _file: file, width, height, stride, bpp, buf, buf_len, back, cursor, mmaped: false, dirty_top: height, dirty_bottom: 0 };
        (fb, vec![0u8; buf_len])
    }

    /// Write the current back-buffer as a raw PPM (P6) file to `path`.
    pub fn screenshot(&self, path: &str) -> Result<(), String> {
        use std::io::Write as IoWrite;
        let mut rgb = Vec::with_capacity(3 * (self.width * self.height) as usize);
        for row in 0..self.height {
            for col in 0..self.width {
                let off = (row * self.stride + col * 4) as usize;
                if off + 4 <= self.back.len() {
                    let b = self.back[off];
                    let g = self.back[off + 1];
                    let r = self.back[off + 2];
                    rgb.push(r);
                    rgb.push(g);
                    rgb.push(b);
                }
            }
        }
        let header = format!("P6\n{} {}\n255\n", self.width, self.height);
        let mut f = std::fs::File::create(path)
            .map_err(|e| format!("create {path}: {e}"))?;
        f.write_all(header.as_bytes()).map_err(|e| format!("write: {e}"))?;
        f.write_all(&rgb).map_err(|e| format!("write: {e}"))?;
        Ok(())
    }

    /// Draw word-wrapped text within `max_w` pixels.
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

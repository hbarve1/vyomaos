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

use super::font;

use std::{
    fs::OpenOptions,
    io,
    os::unix::io::AsRawFd,
    sync::{Mutex, OnceLock},
    thread,
    time::Duration,
};

// ── Linux framebuffer ioctl ───────────────────────────────────────────────────

const FBIOGET_VSCREENINFO: libc::Ioctl = 0x4600;

// fb_var_screeninfo — purely u32 fields (+ bitfield sub-structs of u32),
// so no cross-platform alignment surprises on x86_64.
#[repr(C)]
struct FbBitfield {
    offset: u32,
    length: u32,
    msb_right: u32,
}

#[repr(C)]
struct FbVarScreeninfo {
    xres: u32,
    yres: u32,
    xres_virtual: u32,
    yres_virtual: u32,
    xoffset: u32,
    yoffset: u32,
    bits_per_pixel: u32,
    grayscale: u32,
    red: FbBitfield,
    green: FbBitfield,
    blue: FbBitfield,
    transp: FbBitfield,
    nonstd: u32,
    activate: u32,
    height: u32,  // physical mm — not pixel height
    width: u32,   // physical mm — not pixel width
    accel_flags: u32,
    pixclock: u32,
    left_margin: u32,
    right_margin: u32,
    upper_margin: u32,
    lower_margin: u32,
    hsync_len: u32,
    vsync_len: u32,
    sync: u32,
    vmode: u32,
    rotate: u32,
    colorspace: u32,
    reserved: [u32; 4],
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
        // QEMU virtio-gpu default resolution
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

    Ok(Framebuffer { _file: file, width, height, stride, bpp, buf: buf as *mut u8, buf_len })
}

impl Drop for Framebuffer {
    fn drop(&mut self) {
        unsafe { libc::munmap(self.buf as *mut libc::c_void, self.buf_len); }
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

        for row in y..y1 {
            let base = (row * self.stride + x * 4) as usize;
            let end  = (row * self.stride + x1 * 4) as usize;
            if end > self.buf_len {
                break;
            }
            for off in (base..end).step_by(4) {
                unsafe {
                    std::ptr::copy_nonoverlapping(pixel.as_ptr(), self.buf.add(off), 4);
                }
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
                    // Original 8×16 rendering (unchanged)
                    for row in 0..font::GLYPH_H {
                        let scan_y = y + row;
                        if scan_y >= self.height { break; }
                        let byte = font::FONT[glyph_base + row as usize];
                        for bit in 0..font::GLYPH_W {
                            if byte & (0x80 >> bit) != 0 {
                                let px = cx + bit;
                                let off = (scan_y * self.stride + px * 4) as usize;
                                if off + 4 <= self.buf_len {
                                    unsafe {
                                        std::ptr::copy_nonoverlapping(fg.as_ptr(), self.buf.add(off), 4);
                                    }
                                }
                            }
                        }
                    }
                }
                font::FontSize::Small => {
                    // 8×8: sample every other row of the 8×16 glyph (rows 0,2,4,...14)
                    for out_row in 0..8u32 {
                        let src_row = out_row * 2; // source rows 0,2,4,...14
                        let scan_y = y + out_row;
                        if scan_y >= self.height { break; }
                        let byte = font::FONT[glyph_base + src_row as usize];
                        for out_bit in 0..8u32 {
                            if byte & (0x80 >> out_bit) != 0 {
                                let px = cx + out_bit;
                                let off = (scan_y * self.stride + px * 4) as usize;
                                if off + 4 <= self.buf_len {
                                    unsafe {
                                        std::ptr::copy_nonoverlapping(fg.as_ptr(), self.buf.add(off), 4);
                                    }
                                }
                            }
                        }
                    }
                }
                font::FontSize::Large => {
                    // 16×32: pixel-double the 8×16 glyph (each bit → 2×2 block)
                    for src_row in 0..font::GLYPH_H {
                        let byte = font::FONT[glyph_base + src_row as usize];
                        for rep in 0..2u32 { // each source row drawn twice
                            let scan_y = y + src_row * 2 + rep;
                            if scan_y >= self.height { break; }
                            for src_bit in 0..font::GLYPH_W {
                                if byte & (0x80 >> src_bit) != 0 {
                                    for rep_x in 0..2u32 { // each bit drawn twice
                                        let px = cx + src_bit * 2 + rep_x;
                                        let off = (scan_y * self.stride + px * 4) as usize;
                                        if off + 4 <= self.buf_len {
                                            unsafe {
                                                std::ptr::copy_nonoverlapping(fg.as_ptr(), self.buf.add(off), 4);
                                            }
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

    /// Flush — virtio-gpu with DRM fbdev emulation propagates writes
    /// immediately on mmap.  This is a protocol no-op kept for completeness.
    pub fn flush(&self) {}

    /// Draw a 1-pixel border rectangle (no fill).
    pub fn rect_border(&mut self, x: u32, y: u32, w: u32, h: u32, rgba: u32) {
        if w == 0 || h == 0 {
            return;
        }
        self.fill_rect(x, y, w, 1, rgba);              // top
        self.fill_rect(x, y + h - 1, w, 1, rgba);      // bottom
        self.fill_rect(x, y, 1, h, rgba);              // left
        self.fill_rect(x + w - 1, y, 1, h, rgba);      // right
    }

    /// Fill a region with solid black (clear).
    pub fn clear_region(&mut self, x: u32, y: u32, w: u32, h: u32) {
        self.fill_rect(x, y, w, h, 0x000000FF);
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
        if self.bpp != 32 {
            return;
        }
        let (glyph_w, glyph_h) = font::glyph_dims(size);
        let max_chars = if glyph_w > 0 {
            (max_w / glyph_w) as usize
        } else {
            0
        };
        for (i, line) in wrap_words(text, max_chars).into_iter().enumerate() {
            let ly = y + i as u32 * glyph_h;
            if ly + glyph_h > self.height {
                break;
            }
            self.draw_text(x, ly, &line, rgba, size);
        }
    }
}

/// Word-wrap `text` so each line is at most `max_chars` wide.
/// Long single words are placed on their own line without truncation.
/// If `max_chars` is 0, returns the full text as a single line.
/// SYNC: algorithm duplicated in supervisor/tests/display_test.rs — keep in lockstep.
pub fn wrap_words(text: &str, max_chars: usize) -> Vec<String> {
    if max_chars == 0 {
        return vec![text.to_string()];
    }
    let mut lines: Vec<String> = Vec::new();
    let mut current = String::new();
    for word in text.split(' ').filter(|w| !w.is_empty()) {
        if current.is_empty() {
            current.push_str(word);
        } else if current.len() + 1 + word.len() <= max_chars {
            current.push(' ');
            current.push_str(word);
        } else {
            lines.push(std::mem::take(&mut current));
            current.push_str(word);
        }
    }
    lines.push(current);  // always push, even if empty — ensures empty input returns vec![""]
    lines
}

/// VyomaOS GUI demo
///
/// Draws a static frame to the supervisor framebuffer via the VYOMA_DRAW
/// stdout protocol.  The supervisor's reader thread detects the protocol
/// prefix, decodes the commands, and writes pixels to /dev/fb0.
///
/// Protocol:
///   VYOMA_DRAW:fill_rect:<x>,<y>,<w>,<h>,<rgba_decimal>
///   VYOMA_DRAW:flush
///
/// RGBA is a decimal u32 in big-endian byte order: 0xRRGGBBAA.
fn main() {
    // QEMU virtio-gpu default resolution — supervisor reads the real size
    // via FBIOGET_VSCREENINFO, but we compose the layout against this baseline.
    let w = 1024u32;
    let h = 768u32;

    eprintln!("gui-demo: composing frame ({}×{})", w, h);

    // ── Background: deep navy ─────────────────────────────────────────────────
    fill(0, 0, w, h, 0x0D1117FF);

    // ── Header bar ────────────────────────────────────────────────────────────
    fill(0, 0, w, 52, 0x161B22FF);
    // Header accent line
    fill(0, 52, w, 3, 0x58A6FFFF);

    // ── VyomaOS logo block (top-left of header) ───────────────────────────────
    fill(16, 10, 32, 32, 0x3FB950FF); // green square

    // ── Main content panel ────────────────────────────────────────────────────
    fill(24, 72, w - 48, h - 112, 0x161B22FF);
    // Panel border
    fill(24, 72, w - 48, 2, 0x30363DFF);
    fill(24, 72, 2, h - 112, 0x30363DFF);

    // ── Coloured section blocks (representing running apps) ───────────────────
    let row_y = 96u32;
    let col_w = (w - 96) / 3;
    // hello-world block — teal
    fill(32, row_y, col_w - 8, 100, 0x1F6FEBFF);
    // calculator block — purple
    fill(32 + col_w, row_y, col_w - 8, 100, 0x8957E5FF);
    // storage-demo block — orange
    fill(32 + col_w * 2, row_y, col_w - 8, 100, 0xE3B341FF);

    // ── IPC section ───────────────────────────────────────────────────────────
    fill(32, row_y + 120, (w - 96) / 2 - 8, 80, 0x3FB950FF); // ping — green
    fill(32 + (w - 96) / 2, row_y + 120, (w - 96) / 2 - 8, 80, 0xF78166FF); // pong — red

    // ── Footer ────────────────────────────────────────────────────────────────
    fill(0, h - 56, w, 3, 0x30363DFF);
    fill(0, h - 53, w, 53, 0x0D1117FF);
    // Status dot — green = running
    fill(16, h - 36, 12, 12, 0x3FB950FF);

    flush();
    eprintln!("gui-demo: frame complete");
}

// ── Display protocol helpers ──────────────────────────────────────────────────

#[inline]
fn fill(x: u32, y: u32, w: u32, h: u32, rgba: u32) {
    println!("VYOMA_DRAW:fill_rect:{x},{y},{w},{h},{rgba}");
}

#[inline]
fn flush() {
    println!("VYOMA_DRAW:flush");
}

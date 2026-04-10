/// VyomaOS GUI demo — labelled dashboard
///
/// Draws a status dashboard to the supervisor framebuffer via VYOMA_DRAW.
/// Protocol:
///   VYOMA_DRAW:fill_rect:<x>,<y>,<w>,<h>,<rgba_decimal>
///   VYOMA_DRAW:draw_text:<x>,<y>,<rgba_decimal>,<text>
///   VYOMA_DRAW:flush
///
/// RGBA is a decimal u32: 0xRRGGBBAA (alpha byte ignored by supervisor).
fn main() {
    let w = 1440u32;
    let h = 900u32;

    let boot_count = read_boot_count();

    eprintln!("gui-demo: composing frame ({}x{}) boot#{}", w, h, boot_count);

    // ── Background ────────────────────────────────────────────────────────────
    fill(0, 0, w, h, 0x0D1117FF);

    // ── Header bar ────────────────────────────────────────────────────────────
    fill(0, 0, w, 52, 0x161B22FF);
    fill(0, 52, w, 3, 0x58A6FFFF); // accent line

    // Logo block
    fill(16, 10, 32, 32, 0x3FB950FF);

    // Title and boot counter
    text(58, 18, 0xFFFFFFFF, "VyomaOS");
    let boot_label = format!("boot #{}", boot_count);
    text(w - 100, 18, 0x8B949EFF, &boot_label);

    // ── Main panel ────────────────────────────────────────────────────────────
    fill(24, 72, w - 48, h - 128, 0x161B22FF);
    fill(24, 72, w - 48, 2, 0x30363DFF); // top border
    fill(24, 72, 2, h - 128, 0x30363DFF); // left border

    // Section label
    text(32, 78, 0x8B949EFF, "running apps");

    // ── App blocks — row 1 ───────────────────────────────────────────────────
    let row1_y = 100u32;
    let col_w = (w - 96) / 3;

    // hello-world — teal
    fill(32, row1_y, col_w - 8, 90, 0x1F6FEBFF);
    text(40, row1_y + 8,  0xFFFFFFFF, "hello-world");
    text(40, row1_y + 26, 0xE6EDF3FF, "wasm32-wasip2");
    text(40, row1_y + 62, 0xFFFFFFFF, "done");

    // calculator — purple
    fill(32 + col_w, row1_y, col_w - 8, 90, 0x8957E5FF);
    text(40 + col_w, row1_y + 8,  0xFFFFFFFF, "calculator");
    text(40 + col_w, row1_y + 26, 0xE6EDF3FF, "wasm32-wasip2");
    text(40 + col_w, row1_y + 62, 0xFFFFFFFF, "done");

    // storage-demo — orange
    fill(32 + col_w * 2, row1_y, col_w - 8, 90, 0xE3B341FF);
    text(40 + col_w * 2, row1_y + 8,  0x0D1117FF, "storage-demo");
    text(40 + col_w * 2, row1_y + 26, 0x0D1117FF, "9P virtio fs");
    text(40 + col_w * 2, row1_y + 62, 0x0D1117FF, "done");

    // ── IPC blocks — row 2 ───────────────────────────────────────────────────
    let row2_y = 210u32;
    let half_w = (w - 96) / 2;

    // ping — green
    fill(32, row2_y, half_w - 8, 80, 0x3FB950FF);
    text(40, row2_y + 8,  0x0D1117FF, "ping");
    text(40, row2_y + 26, 0x0D1117FF, "IPC: 3 msgs sent");
    text(40, row2_y + 44, 0x0D1117FF, "target: pong");

    // pong — red-orange
    fill(32 + half_w, row2_y, half_w - 8, 80, 0xF78166FF);
    text(40 + half_w, row2_y + 8,  0x0D1117FF, "pong");
    text(40 + half_w, row2_y + 26, 0x0D1117FF, "IPC: 3 msgs handled");
    text(40 + half_w, row2_y + 44, 0x0D1117FF, "source: ping");

    // ── gui-demo self-label ───────────────────────────────────────────────────
    let row3_y = 310u32;
    fill(32, row3_y, w - 64, 60, 0x21262DFF);
    text(40, row3_y + 8,  0x58A6FFFF, "gui-demo");
    text(40, row3_y + 26, 0x8B949EFF, "display:yes  VYOMA_DRAW protocol  framebuffer: /dev/fb0");

    // ── Footer ────────────────────────────────────────────────────────────────
    fill(0, h - 52, w, 2, 0x30363DFF);
    fill(0, h - 50, w, 50, 0x0D1117FF);
    fill(16, h - 34, 12, 12, 0x3FB950FF); // green status dot
    text(36, h - 34, 0x8B949EFF, "running  |  7 apps  |  supervisor: Rust musl PID 1  |  runtime: Wasmtime WASI P2");

    flush();
    eprintln!("gui-demo: frame drawn");
}

fn read_boot_count() -> u64 {
    std::fs::read_to_string("/data/boot_count.txt")
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0)
}

// ── VYOMA_DRAW protocol helpers ───────────────────────────────────────────────

#[inline]
fn fill(x: u32, y: u32, w: u32, h: u32, rgba: u32) {
    println!("VYOMA_DRAW:fill_rect:{x},{y},{w},{h},{rgba}");
}

#[inline]
fn text(x: u32, y: u32, rgba: u32, s: &str) {
    println!("VYOMA_DRAW:draw_text:{x},{y},{rgba},{s}");
}

#[inline]
fn flush() {
    println!("VYOMA_DRAW:flush");
}

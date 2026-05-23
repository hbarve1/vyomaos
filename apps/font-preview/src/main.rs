use std::io::{self, BufRead, Write};

const W: u32 = 1040;
const H: u32 = 680;
const C_BG: u32    = 0x0D1117FF;
const C_HEADER: u32 = 0x161B22FF;
const C_BORDER: u32 = 0x30363DFF;
const C_TEXT: u32  = 0xE6EDF3FF;
const C_DIM: u32   = 0x8B949EFF;
const C_HINT: u32  = 0x6E7681FF;
const C_SEL: u32   = 0x58A6FFFF;

fn fill(x: u32, y: u32, w: u32, h: u32, rgba: u32) {
    println!("VYOMA_DRAW:fill_rect:{x},{y},{w},{h},{rgba:#010x}");
}
fn flush() {
    println!("VYOMA_DRAW:flush");
    let _ = io::stdout().flush();
}

struct Section {
    label:  &'static str,
    size:   char,
    color:  u32,
    row_h:  u32,
    cols:   u32,
    cell_w: u32,
}

const SECTIONS: &[Section] = &[
    Section { label: "Small (s)  — 8×16 bitmap",  size: 's', color: C_DIM,  row_h: 18, cols: 48, cell_w: 18 },
    Section { label: "Medium (m) — 16×32 scaled",  size: 'm', color: C_TEXT, row_h: 34, cols: 24, cell_w: 20 },
    Section { label: "Large (l)  — 24×48 scaled",  size: 'l', color: C_SEL,  row_h: 50, cols: 16, cell_w: 26 },
];

fn draw(scroll: usize) {
    fill(0, 0, W, H, C_BG);

    // Header
    fill(0, 0, W, 36, C_HEADER);
    fill(0, 36, W, 1, C_BORDER);
    println!("VYOMA_DRAW:draw_text:{},{},{:#010x},m,Font Preview — VYOMA_DRAW font sizes S / M / L", 16, 10, C_TEXT);
    println!("VYOMA_DRAW:draw_text:{},{},{:#010x},m,↑↓: scroll  Esc: close", W - 200, 10, C_HINT);

    let printable: Vec<char> = (32u8..=126u8).map(|b| b as char).collect();

    let mut y = 40i32 - (scroll as i32 * 2);

    for section in SECTIONS {
        if y + 24 > H as i32 { break; }
        if y > H as i32 { break; }

        // Section header
        if y >= 0 {
            fill(0, y as u32, W, 24, 0x21262DFF);
            fill(0, y as u32, 4, 24, section.color);
            println!("VYOMA_DRAW:draw_text:{},{},{:#010x},m,{}", 12, y + 4, section.color, section.label);
            println!("VYOMA_DRAW:draw_text:{},{},{:#010x},m,{} chars", W - 80, y + 4, C_HINT, printable.len());
        }
        y += 26;

        // Render chars in rows
        let rows = (printable.len() as u32 + section.cols - 1) / section.cols;
        for row in 0..rows {
            if y > H as i32 { break; }
            if y + section.row_h as i32 >= 0 {
                let start = (row * section.cols) as usize;
                let end = ((row + 1) * section.cols as u32) as usize;
                let end = end.min(printable.len());
                let line: String = printable[start..end].iter().collect();
                println!("VYOMA_DRAW:draw_text:{},{},{:#010x},{},{line}", 12, y, section.color, section.size);
            }
            y += section.row_h as i32;
        }
        y += 12;
    }

    // Total height info
    println!("VYOMA_DRAW:draw_text:{},{},{:#010x},m,Showing 95 printable ASCII characters (U+0020–U+007E)", 16, H - 16, C_HINT);

    flush();
}

fn main() {
    let stdin = io::stdin();
    let mut scroll = 0usize;

    println!("@supervisor: raise font-preview");
    let _ = io::stdout().flush();

    draw(scroll);

    for line in stdin.lock().lines() {
        let raw = match line { Ok(l) => l, Err(_) => break };
        if raw.starts_with("REPLY:") { continue; }

        match raw.as_str() {
            "\x03" | "\x1b" => {
                fill(0, 0, W, H, 0x0D1117FF);
                println!("VYOMA_DRAW:flush");
                let _ = io::stdout().flush();
                std::process::exit(0);
            }
            "\x1b[A" => {
                if scroll > 0 { scroll -= 2; }
                draw(scroll);
            }
            "\x1b[B" => {
                scroll += 2;
                draw(scroll);
            }
            _ => {}
        }
    }
}

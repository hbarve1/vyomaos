//! VyomaOS text editor (P25)
//!
//! Startup: prompts for a file path (or creates new file).
//! Edit mode: full cursor movement, insert/delete, line split/merge.
//! Ctrl+C → save and quit.  Ctrl+W → save without quitting.
//! Full-screen window; launched on demand via `run text-editor` from shell.

use std::fs;
use std::io::{BufRead, Write};

// ── Layout ────────────────────────────────────────────────────────────────────

const W: u32 = 1440;
const H: u32 = 900;
const PX: u32 = 32;
const PY: u32 = 32;
const PW: u32 = W - 64;
const PH: u32 = H - 64;
const TITLE_H: u32 = 28;
const STATUS_H: u32 = 20;
const INNER_X: u32 = PX + 14;
const INNER_Y: u32 = PY + TITLE_H + 6;
const INNER_W: u32 = PW - 28;
const INNER_H: u32 = PH - TITLE_H - STATUS_H - 12;
const LINE_H: u32 = 18;
const MAX_VISIBLE: usize = (INNER_H / LINE_H) as usize;
const LNUM_W: u32 = 44;          // line-number gutter width
const TEXT_X: u32 = INNER_X + LNUM_W + 6;
const TEXT_W: u32 = INNER_W - LNUM_W - 6;

// ── Colours ───────────────────────────────────────────────────────────────────

const C_BG: u32 = 0x0D1117FF;
const C_PANEL: u32 = 0x161B22FF;
const C_TITLE: u32 = 0x21262DFF;
const C_ACCENT: u32 = 0x58A6FFFF;
const C_WHITE: u32 = 0xFFFFFFFF;
const C_DIM: u32 = 0x8B949EFF;
const C_GREEN: u32 = 0x3FB950FF;
const C_YELLOW: u32 = 0xE3B341FF;
const C_CURLINE: u32 = 0x1C2128FF;  // cursor line highlight
const C_GUTTER: u32 = 0x0D1117FF;   // line number background
const C_SEP: u32 = 0x30363DFF;      // gutter separator

// ── State machine ─────────────────────────────────────────────────────────────

enum Mode {
    /// User types a file path to open or create.
    PathInput { buf: String },

    /// Active editing session.
    Edit {
        path: String,
        lines: Vec<String>,
        cl: usize,   // cursor line
        cc: usize,   // cursor col (byte offset, kept ≤ line.len())
        scroll: usize,
        modified: bool,
    },
}

// ── Main ──────────────────────────────────────────────────────────────────────

fn main() {
    let mut mode = Mode::PathInput { buf: String::new() };
    draw(&mode);

    let stdin = std::io::stdin();
    for raw in stdin.lock().lines() {
        let raw = match raw { Ok(l) => l, Err(_) => break };
        if handle(&raw, &mut mode) { break; }
        draw(&mode);
    }
}

// ── Input dispatch — returns true to exit ─────────────────────────────────────

fn handle(raw: &str, mode: &mut Mode) -> bool {
    match mode {
        Mode::PathInput { buf } => handle_path(raw, buf, mode),
        Mode::Edit { path, lines, cl, cc, scroll, modified } => {
            handle_edit(raw, path, lines, cl, cc, scroll, modified)
        }
    }
}

fn handle_path(raw: &str, buf: &mut String, mode: &mut Mode) -> bool {
    match raw {
        "\x03" => return true,
        "\x7f" => { buf.pop(); }
        "" => {
            let path = buf.trim().to_string();
            if path.is_empty() { return false; }
            let content = fs::read_to_string(&path).unwrap_or_default();
            let lines = if content.is_empty() {
                vec![String::new()]
            } else {
                content.lines().map(|s| s.to_string()).collect()
            };
            *mode = Mode::Edit { path, lines, cl: 0, cc: 0, scroll: 0, modified: false };
        }
        s if single_printable(s) => buf.push_str(s),
        _ => {}
    }
    false
}

fn handle_edit(
    raw: &str,
    path: &mut String,
    lines: &mut Vec<String>,
    cl: &mut usize,
    cc: &mut usize,
    scroll: &mut usize,
    modified: &mut bool,
) -> bool {
    match raw {
        "\x03" => {
            // Ctrl+C — save then quit
            if *modified { save(path, lines); }
            return true;
        }
        "\x17" => {
            // Ctrl+W — save in place
            save(path, lines);
            *modified = false;
        }
        "\x1b[A" => {
            // ↑
            if *cl > 0 {
                *cl -= 1;
                clamp_col(cc, &lines[*cl]);
                scroll_up(cl, scroll);
            }
        }
        "\x1b[B" => {
            // ↓
            if *cl + 1 < lines.len() {
                *cl += 1;
                clamp_col(cc, &lines[*cl]);
                scroll_down(cl, scroll);
            }
        }
        "\x1b[C" => {
            // →
            if *cc < lines[*cl].len() {
                *cc += 1;
            } else if *cl + 1 < lines.len() {
                *cl += 1;
                *cc = 0;
                scroll_down(cl, scroll);
            }
        }
        "\x1b[D" => {
            // ←
            if *cc > 0 {
                *cc -= 1;
            } else if *cl > 0 {
                *cl -= 1;
                *cc = lines[*cl].len();
                scroll_up(cl, scroll);
            }
        }
        "\x7f" => {
            // Backspace
            if *cc > 0 {
                lines[*cl].remove(*cc - 1);
                *cc -= 1;
                *modified = true;
            } else if *cl > 0 {
                let tail = lines.remove(*cl);
                *cl -= 1;
                *cc = lines[*cl].len();
                lines[*cl].push_str(&tail);
                scroll_up(cl, scroll);
                *modified = true;
            }
        }
        "" => {
            // Enter — split line at cursor
            let tail = lines[*cl].split_off(*cc);
            *cl += 1;
            lines.insert(*cl, tail);
            *cc = 0;
            scroll_down(cl, scroll);
            *modified = true;
        }
        s if single_printable(s) => {
            lines[*cl].insert_str(*cc, s);
            *cc += s.len();
            *modified = true;
        }
        _ => {}
    }
    false
}

// ── Drawing ───────────────────────────────────────────────────────────────────

fn draw(mode: &Mode) {
    fill(0, 0, W, H, C_BG);
    fill(PX, PY, PW, PH, C_PANEL);

    match mode {
        Mode::PathInput { buf } => draw_path_input(buf),
        Mode::Edit { path, lines, cl, cc, scroll, modified } => {
            draw_editor(path, lines, *cl, *cc, *scroll, *modified);
        }
    }

    flush();
}

fn draw_path_input(buf: &str) {
    fill(PX, PY, PW, TITLE_H, C_TITLE);
    fill(PX, PY + TITLE_H, PW, 2, C_ACCENT);
    text(PX + 10, PY + 6, C_ACCENT, "Text Editor");
    border(PX, PY, PW, PH, C_ACCENT);

    let cy = PY + PH / 2 - 40;
    text(INNER_X, cy, C_DIM, "Open or create a file in /data");
    let by = cy + 26;
    fill(INNER_X, by, INNER_W, 22, C_TITLE);
    border(INNER_X, by, INNER_W, 22, C_ACCENT);
    text(INNER_X + 6, by + 3, C_WHITE, buf);
    // cursor block
    let cx = INNER_X + 6 + buf.len() as u32 * 8;
    fill(cx, by + 3, 8, 14, C_ACCENT);

    let sy = PY + PH - STATUS_H;
    fill(PX, sy, PW, STATUS_H, C_TITLE);
    text(INNER_X, sy + 3, C_DIM, "Enter path and press Enter   Ctrl+C cancel");
}

fn draw_editor(
    path: &str,
    lines: &[String],
    cl: usize,
    cc: usize,
    scroll: usize,
    modified: bool,
) {
    // Title bar
    fill(PX, PY, PW, TITLE_H, C_TITLE);
    let accent = if modified { C_YELLOW } else { C_GREEN };
    fill(PX, PY + TITLE_H, PW, 2, accent);
    let title = if modified {
        format!("● {path}")
    } else {
        path.to_string()
    };
    text(PX + 10, PY + 6, accent, &title);
    border(PX, PY, PW, PH, accent);

    // Gutter + content area
    clear_region(INNER_X, INNER_Y, INNER_W, INNER_H);
    fill(INNER_X, INNER_Y, LNUM_W, INNER_H, C_GUTTER);
    fill(INNER_X + LNUM_W, INNER_Y, 1, INNER_H, C_SEP);

    for (i, line) in lines.iter().skip(scroll).take(MAX_VISIBLE).enumerate() {
        let abs_ln = scroll + i;
        let iy = INNER_Y + i as u32 * LINE_H;

        // Cursor-line highlight
        if abs_ln == cl {
            fill(INNER_X, iy, INNER_W, LINE_H, C_CURLINE);
        }

        // Line number
        text(INNER_X + 2, iy + 1, C_DIM, &format!("{:>4}", abs_ln + 1));

        // Content — sanitise control chars for display
        let display: String = line
            .chars()
            .map(|c| if c.is_control() { '·' } else { c })
            .collect();
        text_wrap(TEXT_X, iy + 1, TEXT_W, C_WHITE, &display);

        // Cursor block on cursor line
        if abs_ln == cl {
            let cursor_px = TEXT_X + (cc.min(line.len()) as u32) * 8;
            fill(cursor_px, iy + 1, 8, 14, C_ACCENT);
            // Re-draw the char under cursor in contrasting colour
            if cc < line.len() {
                let ch: String = line.chars().nth(cc).into_iter().collect();
                text(cursor_px, iy + 1, C_BG, &ch);
            }
        }
    }

    // Status bar
    let sy = PY + PH - STATUS_H;
    fill(PX, sy, PW, STATUS_H, C_TITLE);
    fill(PX, sy, PW, 1, C_SEP);
    let save_hint = if modified { "Ctrl+W save  " } else { "" };
    text(
        INNER_X,
        sy + 3,
        C_DIM,
        &format!(
            "{save_hint}Ctrl+C save+quit   Ln {}/{} Col {}",
            cl + 1,
            lines.len(),
            cc + 1
        ),
    );
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn save(path: &str, lines: &[String]) {
    let content = lines.join("\n");
    let _ = fs::write(path, content);
}

fn clamp_col(cc: &mut usize, line: &str) {
    if *cc > line.len() {
        *cc = line.len();
    }
}

fn scroll_up(cl: &usize, scroll: &mut usize) {
    if *cl < *scroll {
        *scroll = *cl;
    }
}

fn scroll_down(cl: &usize, scroll: &mut usize) {
    if *cl >= *scroll + MAX_VISIBLE {
        *scroll = cl.saturating_sub(MAX_VISIBLE - 1);
    }
}

fn single_printable(s: &str) -> bool {
    s.len() == 1 && s.bytes().next().map(|b| (0x20..=0x7E).contains(&b)).unwrap_or(false)
}

// ── VYOMA_DRAW helpers ────────────────────────────────────────────────────────

#[inline] fn fill(x: u32, y: u32, w: u32, h: u32, rgba: u32) {
    println!("VYOMA_DRAW:fill_rect:{x},{y},{w},{h},{rgba}");
}
#[inline] fn text(x: u32, y: u32, rgba: u32, s: &str) {
    println!("VYOMA_DRAW:draw_text:{x},{y},{rgba},m,{s}");
}
#[inline] fn border(x: u32, y: u32, w: u32, h: u32, rgba: u32) {
    println!("VYOMA_DRAW:rect_border:{x},{y},{w},{h},{rgba}");
}
#[inline] fn clear_region(x: u32, y: u32, w: u32, h: u32) {
    println!("VYOMA_DRAW:clear_region:{x},{y},{w},{h}");
}
#[inline] fn text_wrap(x: u32, y: u32, max_w: u32, rgba: u32, s: &str) {
    println!("VYOMA_DRAW:draw_text_wrap:{x},{y},{max_w},{rgba},m,{s}");
}
#[inline] fn flush() {
    println!("VYOMA_DRAW:flush");
    let _ = std::io::stdout().flush();
}

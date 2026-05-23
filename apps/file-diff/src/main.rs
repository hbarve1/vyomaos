use std::io::{self, BufRead, Write};

const W: u32 = 1360;
const H: u32 = 760;
const HEADER_H: u32 = 48;
const STATUS_H: u32 = 28;
const LINE_H: u32 = 18;
const CHAR_W: u32 = 8;
const GUTTER_W: u32 = 48;
const VISIBLE_LINES: usize = ((H - HEADER_H - STATUS_H - LINE_H) / LINE_H) as usize;
const MAX_FILE_LINES: usize = 500;
const MAX_COLS: usize = ((W - GUTTER_W - 12) / CHAR_W) as usize;

const C_BG: u32     = 0x0D1117FF;
const C_HEADER: u32 = 0x21262DFF;
const C_BORDER: u32 = 0x30363DFF;
const C_TEXT: u32   = 0xE6EDF3FF;
const C_HINT: u32   = 0x6E7681FF;
const C_ORANGE: u32 = 0xFFA657FF;
const C_GREEN: u32  = 0x3FB950FF;
const C_RED: u32    = 0xFF7B72FF;
const C_SEL: u32    = 0x58A6FFFF;

const C_ADD_BG: u32 = 0x0D2B0DFF;
const C_DEL_BG: u32 = 0x2B0D0DFF;

fn fill(x: u32, y: u32, w: u32, h: u32, rgba: u32) {
    println!("VYOMA_DRAW:fill_rect:{x},{y},{w},{h},{rgba:#010x}");
}
fn text(x: u32, y: u32, rgba: u32, s: &str) {
    println!("VYOMA_DRAW:draw_text:{x},{y},{rgba:#010x},m,{s}");
}
fn flush() {
    println!("VYOMA_DRAW:flush");
    let _ = io::stdout().flush();
}

#[derive(Clone, PartialEq)]
enum DiffKind { Same, Add, Del }

struct DiffLine {
    kind:  DiffKind,
    num_a: Option<usize>,
    num_b: Option<usize>,
    text:  String,
}

fn lcs_diff(a: &[String], b: &[String]) -> Vec<DiffLine> {
    let m = a.len();
    let n = b.len();
    // dp[i][j] = LCS length for a[i..] vs b[j..]
    let mut dp = vec![vec![0u16; n + 1]; m + 1];
    for i in (0..m).rev() {
        for j in (0..n).rev() {
            dp[i][j] = if a[i] == b[j] {
                dp[i + 1][j + 1].saturating_add(1)
            } else {
                dp[i + 1][j].max(dp[i][j + 1])
            };
        }
    }
    let mut out = Vec::new();
    let (mut i, mut j) = (0usize, 0usize);
    while i < m || j < n {
        if i < m && j < n && a[i] == b[j] {
            out.push(DiffLine { kind: DiffKind::Same, num_a: Some(i+1), num_b: Some(j+1), text: a[i].clone() });
            i += 1; j += 1;
        } else if j < n && (i >= m || dp[i+1][j] >= dp[i][j+1]) {
            out.push(DiffLine { kind: DiffKind::Add, num_a: None, num_b: Some(j+1), text: b[j].clone() });
            j += 1;
        } else {
            out.push(DiffLine { kind: DiffKind::Del, num_a: Some(i+1), num_b: None, text: a[i].clone() });
            i += 1;
        }
    }
    out
}

fn load_file(name: &str) -> Result<Vec<String>, String> {
    let path = if name.starts_with('/') { name.to_string() } else { format!("/data/{}", name) };
    std::fs::read_to_string(&path)
        .map(|s| s.lines().take(MAX_FILE_LINES).map(|l| l.to_string()).collect())
        .map_err(|e| format!("{}: {}", path, e))
}

fn trunc(s: &str) -> &str {
    if s.len() > MAX_COLS { &s[..MAX_COLS] } else { s }
}

enum Phase {
    PromptA { input: String },
    PromptB { path_a: String, lines_a: Vec<String>, input: String },
    View    { path_a: String, path_b: String, diff: Vec<DiffLine>, scroll: usize },
    Err     { msg: String },
}

fn draw(ph: &Phase) {
    fill(0, 0, W, H, C_BG);
    fill(0, 0, W, HEADER_H, C_HEADER);
    fill(0, HEADER_H - 1, W, 1, C_BORDER);
    text(16, 16, C_ORANGE, "File Diff");
    text(140, 16, C_HINT, "Compare /data/ files — LCS unified diff");
    text(560, 16, C_HINT, "↑↓:line  PgUp/Dn:page  Home/End  Ctrl+C:back");

    match ph {
        Phase::PromptA { input } => {
            let label = "File A (name in /data/): ";
            let lx = 16 + label.len() as u32 * CHAR_W;
            text(16, HEADER_H + 24, C_HINT, label);
            text(lx, HEADER_H + 24, C_TEXT, input);
            fill(lx + input.len() as u32 * CHAR_W, HEADER_H + 22, 2, LINE_H, C_SEL);
            status("Type filename, press Enter");
        }
        Phase::PromptB { path_a, input, .. } => {
            let label = "File B (name in /data/): ";
            let lx = 16 + label.len() as u32 * CHAR_W;
            text(16, HEADER_H + 8, C_HINT, &format!("File A: {}", path_a));
            text(16, HEADER_H + 30, C_HINT, label);
            text(lx, HEADER_H + 30, C_TEXT, input);
            fill(lx + input.len() as u32 * CHAR_W, HEADER_H + 28, 2, LINE_H, C_SEL);
            status("Type second filename, press Enter to diff");
        }
        Phase::View { path_a, path_b, diff, scroll } => {
            let total = diff.len();
            let sc = (*scroll).min(total.saturating_sub(VISIBLE_LINES));
            let end = (sc + VISIBLE_LINES).min(total);

            // Column header row
            let hdr = format!("--- {}   +++ {}", path_a, path_b);
            text(4, HEADER_H + 2, C_HINT, trunc(&hdr));

            let content_top = HEADER_H + LINE_H;
            for (row, idx) in (sc..end).enumerate() {
                let dl = &diff[idx];
                let y = content_top + row as u32 * LINE_H;

                let (row_bg, pfx_col, pfx) = match dl.kind {
                    DiffKind::Add  => (C_ADD_BG, C_GREEN, "+"),
                    DiffKind::Del  => (C_DEL_BG, C_RED,   "-"),
                    DiffKind::Same => (C_BG,     C_HINT,  " "),
                };
                fill(0, y, W, LINE_H, row_bg);

                let gut = match (&dl.num_a, &dl.num_b) {
                    (Some(a), Some(b)) => format!("{:3}:{:<3} ", a, b),
                    (None,    Some(b)) => format!("   :{:<3} ", b),
                    (Some(a), None)    => format!("{:3}:    ", a),
                    (None,    None)    => "        ".to_string(),
                };
                text(2, y + 2, C_HINT, &gut);
                text(GUTTER_W, y + 2, pfx_col, pfx);
                let line_col = match dl.kind {
                    DiffKind::Add  => C_GREEN,
                    DiffKind::Del  => C_RED,
                    DiffKind::Same => C_TEXT,
                };
                text(GUTTER_W + CHAR_W + 4, y + 2, line_col, trunc(&dl.text));
            }

            let adds  = diff.iter().filter(|l| l.kind == DiffKind::Add).count();
            let dels  = diff.iter().filter(|l| l.kind == DiffKind::Del).count();
            let same  = diff.iter().filter(|l| l.kind == DiffKind::Same).count();
            status(&format!("+{} added  -{} removed  {} unchanged  |  {}/{} lines",
                adds, dels, same, sc + 1, total));
        }
        Phase::Err { msg } => {
            text(16, HEADER_H + 30, C_RED, msg);
            status("Press any key to try again");
        }
    }
    flush();
}

fn status(s: &str) {
    let sb_y = H - STATUS_H;
    fill(0, sb_y, W, STATUS_H, C_HEADER);
    fill(0, sb_y, W, 1, C_BORDER);
    text(8, sb_y + 6, C_HINT, s);
}

fn main() {
    let stdin = io::stdin();
    let mut ph = Phase::PromptA { input: String::new() };

    println!("@supervisor: raise file-diff");
    let _ = io::stdout().flush();
    draw(&ph);

    for line in stdin.lock().lines() {
        let raw = match line { Ok(l) => l, Err(_) => break };
        if raw.starts_with("REPLY:") { continue; }

        ph = handle(ph, &raw);
        draw(&ph);
    }
}

fn handle(ph: Phase, raw: &str) -> Phase {
    match ph {
        Phase::Err { .. } => Phase::PromptA { input: String::new() },

        Phase::PromptA { mut input } => match raw {
            "\x03" => { fill(0, 0, W, H, C_BG); flush(); std::process::exit(0); }
            "\x7f" => { input.pop(); Phase::PromptA { input } }
            "" => {
                let name = input.trim().to_string();
                if name.is_empty() { return Phase::PromptA { input }; }
                match load_file(&name) {
                    Ok(la) => Phase::PromptB { path_a: name, lines_a: la, input: String::new() },
                    Err(e) => Phase::Err { msg: e },
                }
            }
            s if s.len() == 1 => {
                let b = s.as_bytes()[0];
                if b >= 0x20 && b < 0x7f { input.push(b as char); }
                Phase::PromptA { input }
            }
            _ => Phase::PromptA { input },
        },

        Phase::PromptB { path_a, lines_a, mut input } => match raw {
            "\x03" => Phase::PromptA { input: String::new() },
            "\x7f" => { input.pop(); Phase::PromptB { path_a, lines_a, input } }
            "" => {
                let name = input.trim().to_string();
                if name.is_empty() { return Phase::PromptB { path_a, lines_a, input }; }
                match load_file(&name) {
                    Ok(lb) => {
                        let diff = lcs_diff(&lines_a, &lb);
                        Phase::View { path_a, path_b: name, diff, scroll: 0 }
                    }
                    Err(e) => Phase::Err { msg: e },
                }
            }
            s if s.len() == 1 => {
                let b = s.as_bytes()[0];
                if b >= 0x20 && b < 0x7f { input.push(b as char); }
                Phase::PromptB { path_a, lines_a, input }
            }
            _ => Phase::PromptB { path_a, lines_a, input },
        },

        Phase::View { path_a, path_b, diff, mut scroll } => {
            let total = diff.len();
            let max_scroll = total.saturating_sub(VISIBLE_LINES);
            match raw {
                "\x03" => Phase::PromptA { input: String::new() },
                "\x1b[A"  => { scroll = scroll.saturating_sub(1); }
                "\x1b[B"  => { scroll = (scroll + 1).min(max_scroll); }
                "\x1b[5~" => { scroll = scroll.saturating_sub(VISIBLE_LINES); }
                "\x1b[6~" => { scroll = (scroll + VISIBLE_LINES).min(max_scroll); }
                "\x1b[H"  => { scroll = 0; }
                "\x1b[F"  => { scroll = max_scroll; }
                _ => {}
            }
            Phase::View { path_a, path_b, diff, scroll }
        }
    }
}

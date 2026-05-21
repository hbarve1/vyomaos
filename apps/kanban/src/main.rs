use std::io::{self, BufRead, Write};

const W: u32 = 1100;
const H: u32 = 760;
const HEADER_H: u32 = 48;
const STATUS_H: u32 = 36;
const CHAR_W: u32 = 8;

const COL_COUNT: usize = 3;
const COL_W: u32 = (W - 16) / COL_COUNT as u32; // ~361
const COL_GAD: u32 = 4;
const COL_HEADER_H: u32 = 32;
const CARD_H: u32 = 56;
const CARD_GAP: u32 = 4;
const CONTENT_Y: u32 = HEADER_H + COL_HEADER_H + 4;

const C_BG: u32      = 0x0D1117FF;
const C_HEADER: u32  = 0x21262DFF;
const C_BORDER: u32  = 0x30363DFF;
const C_TEXT: u32    = 0xE6EDF3FF;
const C_HINT: u32    = 0x6E7681FF;
const C_ORANGE: u32  = 0xFFA657FF;
const C_GREEN: u32   = 0x3FB950FF;
const C_RED: u32     = 0xFF7B72FF;
const C_SEL: u32     = 0x58A6FFFF;
const C_CARD: u32    = 0x161B22FF;
const C_YELLOW: u32  = 0xD29922FF;

fn fill(x: u32, y: u32, w: u32, h: u32, rgba: u32) {
    println!("VYOMA_DRAW:fill_rect:{x},{y},{w},{h},{rgba:#010x}");
}
fn text(x: u32, y: u32, rgba: u32, s: &str) {
    println!("VYOMA_DRAW:draw_text:{x},{y},{rgba:#010x},m,{s}");
}
fn border(x: u32, y: u32, w: u32, h: u32, rgba: u32) {
    println!("VYOMA_DRAW:rect_border:{x},{y},{w},{h},{rgba:#010x}");
}
fn flush() {
    println!("VYOMA_DRAW:flush");
    let _ = io::stdout().flush();
}

#[derive(Clone)]
struct Card {
    title:    String,
    priority: u8, // 1=high 2=med 3=low
}

impl Card {
    fn new(title: &str, priority: u8) -> Self {
        Card { title: title.to_string(), priority }
    }
    fn priority_color(&self) -> u32 {
        match self.priority { 1 => C_RED, 2 => C_YELLOW, _ => C_HINT }
    }
    fn priority_label(&self) -> &'static str {
        match self.priority { 1 => "HIGH", 2 => "MED", _ => "LOW" }
    }
}

struct Board {
    cols:       [Vec<Card>; 3],
    col_idx:    usize,
    card_idx:   [usize; 3],
    adding:     bool,
    add_input:  String,
}

impl Board {
    fn new() -> Self {
        let cols = [
            vec![
                Card::new("Design WASM ABI",       1),
                Card::new("Implement GPU driver",   1),
                Card::new("Write networking stack", 2),
                Card::new("Add TLS support",        2),
                Card::new("Refactor IPC broker",    3),
            ],
            vec![
                Card::new("Build compositor",       1),
                Card::new("Fix keyboard routing",   2),
            ],
            vec![
                Card::new("Boot in < 5 seconds",   3),
            ],
        ];
        Board { cols, col_idx: 0, card_idx: [0, 0, 0], adding: false, add_input: String::new() }
    }

    fn cur_card(&self) -> Option<&Card> {
        let col = &self.cols[self.col_idx];
        if col.is_empty() { return None; }
        let ci = self.card_idx[self.col_idx].min(col.len().saturating_sub(1));
        col.get(ci)
    }

    fn clamp_cursor(&mut self) {
        for c in 0..COL_COUNT {
            let n = self.cols[c].len();
            if n == 0 { self.card_idx[c] = 0; }
            else { self.card_idx[c] = self.card_idx[c].min(n - 1); }
        }
    }

    fn move_card_right(&mut self) {
        let col = self.col_idx;
        if self.cols[col].is_empty() { return; }
        let ci = self.card_idx[col].min(self.cols[col].len() - 1);
        let card = self.cols[col].remove(ci);
        let next_col = (col + 1) % COL_COUNT;
        self.cols[next_col].push(card);
        self.col_idx = next_col;
        self.card_idx[next_col] = self.cols[next_col].len() - 1;
        self.clamp_cursor();
    }

    fn delete_card(&mut self) {
        let col = self.col_idx;
        if self.cols[col].is_empty() { return; }
        let ci = self.card_idx[col].min(self.cols[col].len() - 1);
        self.cols[col].remove(ci);
        self.clamp_cursor();
    }

    fn cycle_priority(&mut self) {
        let col = self.col_idx;
        if self.cols[col].is_empty() { return; }
        let ci = self.card_idx[col].min(self.cols[col].len() - 1);
        let p = &mut self.cols[col][ci].priority;
        *p = if *p >= 3 { 1 } else { *p + 1 };
    }

    fn save(&self) -> Result<(), String> {
        let mut out = String::new();
        for (c, col) in self.cols.iter().enumerate() {
            for card in col {
                out.push_str(&format!("{},{},{}\n", c, card.priority, card.title));
            }
        }
        std::fs::write("/data/kanban.csv", out).map_err(|e| e.to_string())
    }

    fn load(&mut self) -> Result<(), String> {
        let s = std::fs::read_to_string("/data/kanban.csv").map_err(|e| e.to_string())?;
        for col in &mut self.cols { col.clear(); }
        for line in s.lines() {
            let parts: Vec<&str> = line.splitn(3, ',').collect();
            if parts.len() < 3 { continue; }
            let col: usize = parts[0].parse().unwrap_or(0);
            let pri: u8    = parts[1].parse().unwrap_or(3);
            let title = parts[2].to_string();
            if col < COL_COUNT {
                self.cols[col].push(Card { title, priority: pri });
            }
        }
        self.clamp_cursor();
        Ok(())
    }
}

const COL_NAMES: [&str; 3] = ["Todo", "Doing", "Done"];
const COL_COLORS: [u32; 3] = [C_ORANGE, C_SEL, C_GREEN];

fn col_x(col: usize) -> u32 { 8 + col as u32 * (COL_W + COL_GAD) }

fn draw(board: &Board, status: &str) {
    fill(0, 0, W, H, C_BG);
    fill(0, 0, W, HEADER_H, C_HEADER);
    fill(0, HEADER_H - 1, W, 1, C_BORDER);
    text(16, 16, C_ORANGE, "Kanban Board");
    text(200, 16, C_HINT, "←→:col  ↑↓:card  Enter:move→  N:add  Del:remove  P:priority  Ctrl+W:save");

    // Column headers
    for c in 0..COL_COUNT {
        let cx = col_x(c);
        let hdr_col = if c == board.col_idx { COL_COLORS[c] } else { C_HINT };
        fill(cx, HEADER_H, COL_W - COL_GAD, COL_HEADER_H, C_CARD);
        if c == board.col_idx {
            border(cx, HEADER_H, COL_W - COL_GAD, COL_HEADER_H, hdr_col);
        }
        let cnt = board.cols[c].len();
        let label = format!("{} ({})", COL_NAMES[c], cnt);
        text(cx + 8, HEADER_H + 8, hdr_col, &label);
    }

    // Cards
    let avail_h = H - HEADER_H - COL_HEADER_H - STATUS_H - 8;
    let max_visible = (avail_h / (CARD_H + CARD_GAP)) as usize;

    for c in 0..COL_COUNT {
        let cx = col_x(c);
        let col = &board.cols[c];
        let cur_ci = board.card_idx[c].min(col.len().saturating_sub(1));
        // Scroll so selected card is visible
        let scroll = if col.is_empty() { 0 }
                     else { cur_ci.saturating_sub(max_visible - 1) };

        for (vis_i, card_i) in (scroll..).take(max_visible).enumerate() {
            if card_i >= col.len() { break; }
            let card = &col[card_i];
            let cy = CONTENT_Y + vis_i as u32 * (CARD_H + CARD_GAP);
            let is_sel = c == board.col_idx && card_i == cur_ci;
            let bg = if is_sel { 0x1C2D4EFF } else { C_CARD };
            fill(cx, cy, COL_W - COL_GAD, CARD_H, bg);
            if is_sel {
                border(cx - 1, cy - 1, COL_W - COL_GAD + 2, CARD_H + 2, C_SEL);
            } else {
                border(cx, cy, COL_W - COL_GAD, CARD_H, C_BORDER);
            }
            // Priority badge
            let pri_col = card.priority_color();
            fill(cx + 4, cy + 4, 36, 16, pri_col);
            text(cx + 6, cy + 6, C_BG, card.priority_label());
            // Title
            let max_t = (COL_W - 16) as usize / CHAR_W as usize;
            let title_d = if card.title.len() > max_t { &card.title[..max_t] } else { &card.title };
            let tc = if is_sel { C_TEXT } else { C_HINT };
            text(cx + 4, cy + 26, tc, title_d);
            // Card index
            text(cx + COL_W - 36, cy + 6, C_HINT, &format!("{}/{}", card_i + 1, col.len()));
        }
    }

    // Status / input bar
    let sb_y = H - STATUS_H;
    fill(0, sb_y, W, STATUS_H, C_HEADER);
    fill(0, sb_y, W, 1, C_BORDER);

    if board.adding {
        text(8, sb_y + 10, C_HINT, "New card title: ");
        let inp = &board.add_input;
        text(8 + 16 * CHAR_W, sb_y + 10, C_TEXT, inp);
        fill(8 + (16 + inp.len() as u32) * CHAR_W, sb_y + 8, 2, 20, C_SEL);
    } else if !status.is_empty() {
        text(8, sb_y + 10, C_HINT, status);
    } else {
        let total: usize = board.cols.iter().map(|c| c.len()).sum();
        text(8, sb_y + 10, C_HINT, &format!("{} total cards  |  Todo:{} Doing:{} Done:{}",
            total, board.cols[0].len(), board.cols[1].len(), board.cols[2].len()));
    }
    flush();
}

fn main() {
    let stdin = io::stdin();
    let mut board = Board::new();
    let mut status = String::new();

    println!("@supervisor: raise kanban");
    let _ = io::stdout().flush();
    draw(&board, &status);

    for line in stdin.lock().lines() {
        let raw = match line { Ok(l) => l, Err(_) => break };
        if raw.starts_with("REPLY:") { continue; }

        status.clear();

        if board.adding {
            match raw.as_str() {
                "\x03" | "\x1b" => { board.adding = false; board.add_input.clear(); }
                "\x7f" => { board.add_input.pop(); }
                "" => {
                    let title = board.add_input.trim().to_string();
                    if !title.is_empty() {
                        board.cols[board.col_idx].push(Card::new(&title, 3));
                        board.card_idx[board.col_idx] = board.cols[board.col_idx].len() - 1;
                        status = format!("Added: {}", title);
                    }
                    board.adding = false;
                    board.add_input.clear();
                }
                s if s.len() == 1 => {
                    let b = s.as_bytes()[0];
                    if b >= 0x20 && b < 0x7f { board.add_input.push(b as char); }
                }
                _ => {}
            }
        } else {
            match raw.as_str() {
                "\x03" => { fill(0, 0, W, H, C_BG); flush(); std::process::exit(0); }
                "\x1b[C" => { board.col_idx = (board.col_idx + 1) % COL_COUNT; }
                "\x1b[D" => { board.col_idx = if board.col_idx == 0 { COL_COUNT - 1 } else { board.col_idx - 1 }; }
                "\x1b[A" => {
                    let c = board.col_idx;
                    if board.card_idx[c] > 0 { board.card_idx[c] -= 1; }
                }
                "\x1b[B" => {
                    let c = board.col_idx;
                    if board.card_idx[c] + 1 < board.cols[c].len() { board.card_idx[c] += 1; }
                }
                "" => { board.move_card_right(); status = "Card moved →".to_string(); }
                "n" | "N" => { board.adding = true; }
                "\x7f" => { board.delete_card(); status = "Card deleted".to_string(); }
                "p" | "P" => { board.cycle_priority(); }
                "\x17" => { // Ctrl+W
                    match board.save() {
                        Ok(())  => status = "Saved to /data/kanban.csv".to_string(),
                        Err(e)  => status = format!("Save error: {}", e),
                    }
                }
                "\x0c" => { // Ctrl+L
                    match board.load() {
                        Ok(())  => status = "Loaded from /data/kanban.csv".to_string(),
                        Err(e)  => status = format!("Load error: {}", e),
                    }
                }
                _ => {}
            }
        }
        draw(&board, &status);
    }
}

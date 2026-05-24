// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

use std::io::{self, BufRead, Write};

const W: i32 = 1200;
const H: i32 = 720;
const LINE_H: i32 = 18;
const GUTTER_W: i32 = 44;
const PANEL_W: i32 = W / 2;
const CONTENT_Y: i32 = 60;

const C_BG: u32     = 0x0D1117FF;
const C_HEADER: u32 = 0x21262DFF;
const C_BORDER: u32 = 0x30363DFF;
const C_TEXT: u32   = 0xE6EDF3FF;
const C_HINT: u32   = 0x6E7681FF;
const C_GREEN: u32  = 0x3FB950FF;
const C_RED: u32    = 0xFF7B72FF;
const C_YELLOW: u32 = 0xD29922FF;
const C_CARD: u32   = 0x161B22FF;
const C_SEL: u32    = 0x58A6FFFF;
const C_ADD_BG: u32 = 0x0A2910FF;
const C_REM_BG: u32 = 0x290A0AFF;
const C_CHG_BG: u32 = 0x292010FF;

fn fill(x: i32, y: i32, w: i32, h: i32, c: u32) {
    if w > 0 && h > 0 { println!("VYOMA_DRAW:fill_rect:{},{},{},{},{}", x, y, w, h, c); }
}
fn text(x: i32, y: i32, c: u32, s: &str) {
    if !s.is_empty() { println!("VYOMA_DRAW:draw_text:{},{},{},m,{}", x, y, c, s); }
}
fn flush() {
    println!("VYOMA_DRAW:flush");
    let _ = io::stdout().flush();
}

fn trunc(s: &str, max: usize) -> &str {
    if s.len() <= max { s } else { &s[..max] }
}

#[derive(Clone, Copy, PartialEq)]
enum DiffKind { Same, Added, Removed, Changed }

#[derive(Clone)]
struct DiffRow {
    left:  Option<String>,
    right: Option<String>,
    kind:  DiffKind,
}

fn lcs_diff_raw(old: &[&str], new: &[&str]) -> Vec<DiffRow> {
    let m = old.len();
    let n = new.len();
    let mut dp = vec![vec![0u16; n + 1]; m + 1];
    for i in (0..m).rev() {
        for j in (0..n).rev() {
            dp[i][j] = if old[i] == new[j] {
                dp[i + 1][j + 1].saturating_add(1)
            } else {
                dp[i + 1][j].max(dp[i][j + 1])
            };
        }
    }
    let mut rows: Vec<DiffRow> = Vec::new();
    let (mut i, mut j) = (0usize, 0usize);
    while i < m || j < n {
        if i < m && j < n && old[i] == new[j] {
            rows.push(DiffRow { left: Some(old[i].to_string()), right: Some(new[j].to_string()), kind: DiffKind::Same });
            i += 1; j += 1;
        } else if j < n && (i >= m || dp[i][j + 1] >= dp[i + 1][j]) {
            rows.push(DiffRow { left: None, right: Some(new[j].to_string()), kind: DiffKind::Added });
            j += 1;
        } else {
            rows.push(DiffRow { left: Some(old[i].to_string()), right: None, kind: DiffKind::Removed });
            i += 1;
        }
    }
    rows
}

fn merge_changes(raw: Vec<DiffRow>) -> Vec<DiffRow> {
    let mut result: Vec<DiffRow> = Vec::new();
    let mut k = 0;
    while k < raw.len() {
        match raw[k].kind {
            DiffKind::Same | DiffKind::Added | DiffKind::Changed => {
                result.push(raw[k].clone());
                k += 1;
            }
            DiffKind::Removed => {
                let mut rm: Vec<String> = Vec::new();
                while k < raw.len() && raw[k].kind == DiffKind::Removed {
                    rm.push(raw[k].left.clone().unwrap_or_default());
                    k += 1;
                }
                let mut add: Vec<String> = Vec::new();
                while k < raw.len() && raw[k].kind == DiffKind::Added {
                    add.push(raw[k].right.clone().unwrap_or_default());
                    k += 1;
                }
                let pairs = rm.len().min(add.len());
                for p in 0..pairs {
                    result.push(DiffRow { left: Some(rm[p].clone()), right: Some(add[p].clone()), kind: DiffKind::Changed });
                }
                for p in pairs..rm.len() {
                    result.push(DiffRow { left: Some(rm[p].clone()), right: None, kind: DiffKind::Removed });
                }
                for p in pairs..add.len() {
                    result.push(DiffRow { left: None, right: Some(add[p].clone()), kind: DiffKind::Added });
                }
            }
        }
    }
    result
}

fn lcs_diff(old: &[&str], new: &[&str]) -> Vec<DiffRow> {
    merge_changes(lcs_diff_raw(old, new))
}

// ── Diff pair data ────────────────────────────────────────────────────────────

static PAIR_NAMES: [&str; 5] = [
    "Refactor: extract helper",
    "Add error handling",
    "Rename + simplify",
    "Add logging",
    "Loop optimization",
];

static OLD0: &[&str] = &[
    "fn process(data: &[u8]) -> String {",
    "    let mut result = String::new();",
    "    for b in data {",
    "        if *b >= 32 && *b < 127 {",
    "            result.push(*b as char);",
    "        }",
    "    }",
    "    result.trim().to_string()",
    "}",
];
static NEW0: &[&str] = &[
    "fn is_printable(b: u8) -> bool {",
    "    b >= 32 && b < 127",
    "}",
    "",
    "fn process(data: &[u8]) -> String {",
    "    data.iter()",
    "        .filter(|&&b| is_printable(b))",
    "        .map(|&b| b as char)",
    "        .collect::<String>()",
    "        .trim()",
    "        .to_string()",
    "}",
];

static OLD1: &[&str] = &[
    "fn load_config(path: &str) -> Config {",
    "    let content = fs::read_to_string(path).unwrap();",
    "    let config = toml::from_str(&content).unwrap();",
    "    config",
    "}",
    "",
    "fn main() {",
    "    let cfg = load_config(\"config.toml\");",
    "    run(cfg);",
    "}",
];
static NEW1: &[&str] = &[
    "fn load_config(path: &str) -> Result<Config, Box<dyn Error>> {",
    "    let content = fs::read_to_string(path)?;",
    "    let config = toml::from_str(&content)?;",
    "    Ok(config)",
    "}",
    "",
    "fn main() {",
    "    let cfg = load_config(\"config.toml\")",
    "        .expect(\"failed to load config\");",
    "    run(cfg);",
    "}",
];

static OLD2: &[&str] = &[
    "struct UserRecord {",
    "    id: u32,",
    "    name: String,",
    "    email: String,",
    "}",
    "",
    "fn get_record(id: u32) -> UserRecord {",
    "    DB.lock().get(&id).cloned().unwrap()",
    "}",
    "",
    "fn delete_record(id: u32) {",
    "    DB.lock().remove(&id);",
    "}",
];
static NEW2: &[&str] = &[
    "struct User {",
    "    id:    u32,",
    "    name:  String,",
    "    email: String,",
    "}",
    "",
    "fn find_user(id: u32) -> Option<User> {",
    "    DB.lock().get(&id).cloned()",
    "}",
    "",
    "fn remove_user(id: u32) {",
    "    DB.lock().remove(&id);",
    "}",
];

static OLD3: &[&str] = &[
    "fn save(path: &str, data: &[u8]) {",
    "    let mut f = File::create(path).unwrap();",
    "    f.write_all(data).unwrap();",
    "}",
    "",
    "fn load(path: &str) -> Vec<u8> {",
    "    fs::read(path).unwrap()",
    "}",
];
static NEW3: &[&str] = &[
    "fn save(path: &str, data: &[u8]) {",
    "    log::info!(\"saving {} bytes -> {}\", data.len(), path);",
    "    let mut f = File::create(path).unwrap();",
    "    f.write_all(data).unwrap();",
    "    log::debug!(\"save ok\");",
    "}",
    "",
    "fn load(path: &str) -> Vec<u8> {",
    "    log::info!(\"loading {}\", path);",
    "    fs::read(path).unwrap()",
    "}",
];

static OLD4: &[&str] = &[
    "fn sum_evens(nums: &[i64]) -> i64 {",
    "    let mut total = 0i64;",
    "    for i in 0..nums.len() {",
    "        if nums[i] % 2 == 0 {",
    "            total += nums[i];",
    "        }",
    "    }",
    "    total",
    "}",
    "",
    "fn max_val(nums: &[i64]) -> i64 {",
    "    let mut m = i64::MIN;",
    "    for i in 0..nums.len() {",
    "        if nums[i] > m { m = nums[i]; }",
    "    }",
    "    m",
    "}",
];
static NEW4: &[&str] = &[
    "fn sum_evens(nums: &[i64]) -> i64 {",
    "    nums.iter().filter(|&&n| n % 2 == 0).sum()",
    "}",
    "",
    "fn max_val(nums: &[i64]) -> i64 {",
    "    nums.iter().copied().max().unwrap_or(i64::MIN)",
    "}",
];

// ── App ───────────────────────────────────────────────────────────────────────

struct App {
    pair:   usize,
    rows:   Vec<DiffRow>,
    scroll: usize,
}

impl App {
    fn new() -> Self {
        let mut app = App { pair: 0, rows: Vec::new(), scroll: 0 };
        app.load_pair();
        app
    }

    fn load_pair(&mut self) {
        let (old, new): (&[&str], &[&str]) = match self.pair {
            0 => (OLD0, NEW0),
            1 => (OLD1, NEW1),
            2 => (OLD2, NEW2),
            3 => (OLD3, NEW3),
            _ => (OLD4, NEW4),
        };
        self.rows = lcs_diff(old, new);
        self.scroll = 0;
    }

    fn stats(&self) -> (usize, usize, usize, usize) {
        let (mut a, mut r, mut c, mut s) = (0, 0, 0, 0);
        for row in &self.rows {
            match row.kind {
                DiffKind::Added   => a += 1,
                DiffKind::Removed => r += 1,
                DiffKind::Changed => c += 1,
                DiffKind::Same    => s += 1,
            }
        }
        (a, r, c, s)
    }

    fn visible_lines() -> usize {
        ((H - CONTENT_Y - 16) / LINE_H) as usize
    }

    fn draw(&self) {
        let vis = Self::visible_lines();
        let max_chars = ((PANEL_W - GUTTER_W - 8) / 8) as usize;

        fill(0, 0, W, H, C_BG);

        // Header
        fill(0, 0, W, 32, C_HEADER);
        text(12, 8, C_TEXT, &format!("Code Diff  [{}/{}]  {}", self.pair + 1, PAIR_NAMES.len(), PAIR_NAMES[self.pair]));
        text(W - 376, 8, C_HINT, "Tab=next  \u{2191}\u{2193}=scroll  Q=quit");

        // Stats bar
        let (added, removed, changed, same) = self.stats();
        fill(0, 32, W, 28, C_CARD);
        text(12,  38, C_GREEN,  &format!("+{} added",     added));
        text(120, 38, C_RED,    &format!("-{} removed",   removed));
        text(260, 38, C_YELLOW, &format!("~{} changed",   changed));
        text(400, 38, C_HINT,   &format!("{} unchanged",  same));

        // Column headers
        let mid = PANEL_W;
        text(mid - 72, 38, C_HINT, "OLD");
        fill(mid - 1, 32, 2, H - 32, C_BORDER);
        text(mid + 12, 38, C_HINT, "NEW");

        // Content rows
        let start = self.scroll;
        let end   = (start + vis).min(self.rows.len());

        // Compute initial line numbers by counting non-blank left/right up to scroll
        let mut left_ln  = 1usize;
        let mut right_ln = 1usize;
        for row in &self.rows[..start] {
            if row.left.is_some()  { left_ln  += 1; }
            if row.right.is_some() { right_ln += 1; }
        }

        for (idx, row) in self.rows[start..end].iter().enumerate() {
            let ry = CONTENT_Y + idx as i32 * LINE_H;
            let bg = match row.kind {
                DiffKind::Added   => C_ADD_BG,
                DiffKind::Removed => C_REM_BG,
                DiffKind::Changed => C_CHG_BG,
                DiffKind::Same    => C_BG,
            };
            // Row backgrounds
            fill(0,   ry, PANEL_W, LINE_H, bg);
            fill(mid, ry, PANEL_W, LINE_H, bg);
            // Gutters
            fill(0,   ry, GUTTER_W, LINE_H, C_CARD);
            fill(mid, ry, GUTTER_W, LINE_H, C_CARD);

            // Left side
            if let Some(ref l) = row.left {
                let tc = match row.kind {
                    DiffKind::Removed => C_RED,
                    DiffKind::Changed => C_YELLOW,
                    _ => C_TEXT,
                };
                text(4,                ry + 1, C_HINT, &format!("{:3}", left_ln));
                text(GUTTER_W + 4,     ry + 1, tc,    trunc(l, max_chars));
                left_ln += 1;
            }

            // Right side
            if let Some(ref r) = row.right {
                let tc = match row.kind {
                    DiffKind::Added   => C_GREEN,
                    DiffKind::Changed => C_YELLOW,
                    _ => C_TEXT,
                };
                text(mid + 4,              ry + 1, C_HINT, &format!("{:3}", right_ln));
                text(mid + GUTTER_W + 4,   ry + 1, tc,    trunc(r, max_chars));
                right_ln += 1;
            }
        }

        // Scrollbar
        let total = self.rows.len();
        if total > vis {
            let area_h = (H - CONTENT_Y - 16) as usize;
            let thumb_h = ((area_h * vis) / total).max(16);
            let thumb_y = CONTENT_Y as usize + (area_h * self.scroll) / total;
            fill(W - 6, CONTENT_Y, 6, area_h as i32, C_CARD);
            fill(W - 6, thumb_y as i32, 6, thumb_h as i32, C_BORDER);
        }

        // Bottom bar
        fill(0, H - 16, W, 16, C_HEADER);
        text(12, H - 14, C_HINT,
             &format!("{} rows  scroll {}/{}  pair {}/{}",
                 total, self.scroll,
                 total.saturating_sub(vis),
                 self.pair + 1, PAIR_NAMES.len()));

        flush();
    }

    fn handle(&mut self, line: &str) {
        let vis = Self::visible_lines();
        match line {
            "\t" => {
                self.pair = (self.pair + 1) % PAIR_NAMES.len();
                self.load_pair();
                self.draw();
            }
            "\x1b[A" => {
                if self.scroll > 0 { self.scroll -= 1; }
                self.draw();
            }
            "\x1b[B" => {
                let max_scroll = self.rows.len().saturating_sub(vis);
                if self.scroll < max_scroll { self.scroll += 1; }
                self.draw();
            }
            "q" | "Q" | "\x03" => std::process::exit(0),
            _ => {}
        }
    }
}

fn main() {
    let stdin = io::stdin();
    let mut app = App::new();
    app.draw();
    let _ = io::stdout().flush();

    for line in stdin.lock().lines() {
        let line = match line { Ok(l) => l, Err(_) => break };
        app.handle(&line);
        let _ = io::stdout().flush();
    }
}

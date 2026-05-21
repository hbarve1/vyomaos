use std::io::{self, BufRead, Write};

const W: u32 = 960;
const H: u32 = 640;
const HEADER_H: u32 = 48;

const C_BG: u32     = 0x0D1117FF;
const C_HEADER: u32 = 0x21262DFF;
const C_BORDER: u32 = 0x30363DFF;
const C_TEXT: u32   = 0xE6EDF3FF;
const C_HINT: u32   = 0x6E7681FF;
const C_ORANGE: u32 = 0xFFA657FF;
const C_GREEN: u32  = 0x3FB950FF;
const C_RED: u32    = 0xFF7B72FF;
const C_SEL: u32    = 0x58A6FFFF;
const C_CARD: u32   = 0x161B22FF;

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

fn lcg(s: u64) -> u64 {
    s.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407)
}

const COIN_NAMES:  [&str; 5] = ["BTC", "ETH", "SOL", "ADA", "DOT"];
// Prices stored ×10000 (so 1 unit = $0.0001)
const BASE_PRICES: [u64; 5]  = [
    650_000_000, // BTC $65,000
    35_000_000,  // ETH $3,500
    1_600_000,   // SOL $160
    6_000,       // ADA $0.60
    85_000,      // DOT $8.50
];

const SPK_LEN:  usize = 40;
const SPK_W:    u32   = 120;
const SPK_H:    u32   = 40;
const ROW_H:    u32   = 100;
const ROW_Y0:   u32   = HEADER_H + 16;

fn fmt_price(p: u64) -> String {
    let dollars = p / 10_000;
    let frac    = p % 10_000;
    if dollars >= 1000 {
        format!("${}", dollars)
    } else if dollars >= 10 {
        format!("${}.{:02}", dollars, frac / 100)
    } else {
        format!("${}.{:04}", dollars, frac)
    }
}

// Returns percent change in units of 0.01% (basis points × 100)
fn pct_change(current: u64, base: u64) -> i64 {
    if base == 0 { return 0; }
    (current as i64 - base as i64) * 10_000 / base as i64
}

fn fmt_pct(bp100: i64) -> String {
    let sign = if bp100 >= 0 { "+" } else { "-" };
    let abs  = bp100.unsigned_abs();
    format!("{}{}.{:02}%", sign, abs / 100, abs % 100)
}

struct Coin {
    price:    u64,
    base:     u64,
    history:  [u64; SPK_LEN],
    hist_len: usize,
    hist_pos: usize,
}

impl Coin {
    fn new(base: u64) -> Self {
        Coin { price: base, base, history: [base; SPK_LEN], hist_len: 1, hist_pos: 0 }
    }

    fn update(&mut self, seed: u64) {
        // Random walk ±0.5% max per tick (±5 basis points)
        let r = (seed >> 40) as i64 % 11 - 5; // -5..5
        let new_price = ((self.price as i64 * (10_000 + r)) / 10_000) as u64;
        self.price = new_price.max(self.base / 10);
        self.hist_pos = (self.hist_pos + 1) % SPK_LEN;
        self.history[self.hist_pos] = self.price;
        if self.hist_len < SPK_LEN { self.hist_len += 1; }
    }

    fn reset(&mut self) {
        self.price = self.base;
        self.history = [self.base; SPK_LEN];
        self.hist_len = 1;
    }

    fn pct(&self) -> i64 { pct_change(self.price, self.base) }
}

fn draw_sparkline(coin: &Coin, x: u32, y: u32) {
    if coin.hist_len < 2 { return; }
    let mut min = u64::MAX;
    let mut max = 0u64;
    for i in 0..coin.hist_len {
        let v = coin.history[(coin.hist_pos + SPK_LEN - coin.hist_len + 1 + i) % SPK_LEN];
        if v < min { min = v; }
        if v > max { max = v; }
    }
    let range = if max == min { 1 } else { max - min };
    let bar_w = SPK_W / coin.hist_len as u32;
    let bar_w = bar_w.max(2);
    let color = if coin.pct() >= 0 { C_GREEN } else { C_RED };

    fill(x, y, SPK_W, SPK_H, C_CARD);

    for i in 0..coin.hist_len {
        let v = coin.history[(coin.hist_pos + SPK_LEN - coin.hist_len + 1 + i) % SPK_LEN];
        let bx = x + i as u32 * SPK_W / coin.hist_len as u32;
        let bh = ((v - min) * (SPK_H - 2) as u64 / range + 2) as u32;
        let by = y + SPK_H - bh;
        fill(bx, by, bar_w.min(SPK_W / SPK_LEN as u32 + 1), bh, color);
    }
}

#[derive(PartialEq)]
enum SortMode { ByName, ByChange }

struct Ticker {
    coins: Vec<Coin>,
    seed:  u64,
    sort:  SortMode,
    ticks: u32,
}

impl Ticker {
    fn new() -> Self {
        Ticker {
            coins: BASE_PRICES.iter().map(|&b| Coin::new(b)).collect(),
            seed:  0xC5EC1234567890ABu64,
            sort:  SortMode::ByName,
            ticks: 0,
        }
    }

    fn tick(&mut self) {
        self.ticks += 1;
        for coin in &mut self.coins {
            self.seed = lcg(self.seed);
            coin.update(self.seed);
        }
    }

    fn reset(&mut self) {
        for coin in &mut self.coins { coin.reset(); }
    }

    fn sorted_indices(&self) -> Vec<usize> {
        let mut idx: Vec<usize> = (0..self.coins.len()).collect();
        if self.sort == SortMode::ByChange {
            idx.sort_by(|&a, &b| {
                self.coins[b].pct().cmp(&self.coins[a].pct())
            });
        }
        idx
    }
}

fn draw(t: &Ticker) {
    fill(0, 0, W, H, C_BG);
    fill(0, 0, W, HEADER_H, C_HEADER);
    fill(0, HEADER_H - 1, W, 1, C_BORDER);
    text(16, 16, C_ORANGE, "Crypto Ticker");
    text(200, 16, C_HINT, &format!("Tick #{}", t.ticks));
    let sort_label = if t.sort == SortMode::ByChange { "Sort: % Change" } else { "Sort: Name" };
    text(400, 16, C_HINT, sort_label);
    text(620, 16, C_HINT, "R:reset  Tab:sort");

    // Column headers
    let hy = ROW_Y0 - 16;
    text(20,  hy, C_HINT, "Coin");
    text(100, hy, C_HINT, "Price");
    text(260, hy, C_HINT, "Change");
    text(420, hy, C_HINT, "Sparkline (40 ticks)");
    text(560, hy, C_HINT, "Alert");

    let indices = t.sorted_indices();
    for (display_row, &ci) in indices.iter().enumerate() {
        let coin = &t.coins[ci];
        let ry = ROW_Y0 + display_row as u32 * ROW_H;
        let pct = coin.pct();
        let is_alert = pct.abs() > 500; // > 5% from base (500 = 5.00%)

        // Row background
        let row_bg = if is_alert {
            if pct > 0 { 0x0D2B0DFF } else { 0x2B0D0DFF }
        } else {
            C_CARD
        };
        fill(0, ry, W, ROW_H - 4, row_bg);
        border(0, ry, W, ROW_H - 4, C_BORDER);

        // Coin name
        text(20, ry + 8, C_SEL, COIN_NAMES[ci]);

        // Price
        let price_str = fmt_price(coin.price);
        text(100, ry + 8, C_TEXT, &price_str);

        // % change
        let pct_str = fmt_pct(pct);
        let pct_color = if pct > 0 { C_GREEN } else if pct < 0 { C_RED } else { C_HINT };
        text(260, ry + 8, pct_color, &pct_str);

        // High / Low
        let high = fmt_price(*coin.history.iter().max().unwrap_or(&coin.price));
        let low  = fmt_price(*coin.history.iter().min().unwrap_or(&coin.price));
        text(260, ry + 28, C_HINT, &format!("H:{} L:{}", high, low));

        // Sparkline
        draw_sparkline(coin, 420, ry + 8);

        // Alert
        if is_alert {
            let alert_color = if pct > 0 { C_GREEN } else { C_RED };
            let alert_str = if pct > 0 { "▲ SURGE" } else { "▼ DROP" };
            text(560, ry + 8, alert_color, alert_str);
        }

        // Base price for reference
        text(20, ry + 28, C_HINT, &format!("Base: {}", fmt_price(coin.base)));
    }

    flush();
}

fn main() {
    let stdin = io::stdin();
    let mut ticker = Ticker::new();

    println!("@supervisor: raise crypto-ticker");
    let _ = io::stdout().flush();
    println!("@supervisor: ping");
    let _ = io::stdout().flush();
    draw(&ticker);

    for line in stdin.lock().lines() {
        let raw = match line { Ok(l) => l, Err(_) => break };
        if raw.starts_with("REPLY:") {
            ticker.tick();
            println!("@supervisor: ping");
            let _ = io::stdout().flush();
            draw(&ticker);
            continue;
        }

        match raw.as_str() {
            "\x1b" | "\x03" => { fill(0, 0, W, H, C_BG); flush(); std::process::exit(0); }
            "r" | "R" => { ticker.reset(); }
            "\t" => {
                ticker.sort = if ticker.sort == SortMode::ByName {
                    SortMode::ByChange
                } else {
                    SortMode::ByName
                };
            }
            _ => {}
        }
        draw(&ticker);
    }
}

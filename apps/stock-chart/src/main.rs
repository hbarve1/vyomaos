use std::io::{self, BufRead, Write};

const W: u32 = 1200;
const H: u32 = 760;
const HEADER_H: u32 = 48;
const STATUS_H: u32 = 28;
const CHAR_W: u32 = 8;

const STOCK_PANEL_W: u32 = 120;
const CHART_X: u32 = STOCK_PANEL_W;
const CHART_W: u32 = W - STOCK_PANEL_W;
const PRICE_AXIS_W: u32 = 72;
const VOL_H: u32 = 60;
const CHART_CONTENT_W: u32 = CHART_W - PRICE_AXIS_W;
const CHART_Y: u32 = HEADER_H;
const CHART_CONTENT_H: u32 = H - HEADER_H - STATUS_H - VOL_H;

const MAX_CANDLES: usize = 100;
const CANDLE_W: u32 = {
    let w = CHART_CONTENT_W / MAX_CANDLES as u32;
    if w < 4 { 4 } else { w }
};
const VISIBLE_CANDLES: usize = (CHART_CONTENT_W / CANDLE_W) as usize;

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
const C_GRID: u32    = 0x1C2128FF;

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

fn lcg(s: u64) -> u64 {
    s.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407)
}

#[derive(Clone, Copy)]
struct Candle {
    open:   i64,
    high:   i64,
    low:    i64,
    close:  i64,
    volume: u64,
}

struct Stock {
    name:       &'static str,
    start:      i64, // price ×100 cents
    candles:    Vec<Candle>,
    seed:       u64,
}

impl Stock {
    fn new(name: &'static str, start_cents: i64, seed: u64) -> Self {
        let mut s = Stock { name, start: start_cents, candles: Vec::new(), seed };
        // Seed with 40 initial candles
        for _ in 0..40 {
            s.next_candle();
        }
        s
    }

    fn next_candle(&mut self) {
        let prev_close = self.candles.last().map(|c| c.close).unwrap_or(self.start);

        self.seed = lcg(self.seed);
        let delta = ((self.seed % 201) as i64 - 100) * prev_close / 1000; // ±10%
        let open = prev_close;
        let close = (prev_close + delta).max(1);

        self.seed = lcg(self.seed);
        let range = ((self.seed % 50) as i64 + 5) * prev_close / 1000;
        let high = open.max(close) + range;
        let low  = open.min(close).saturating_sub(range).max(1);

        self.seed = lcg(self.seed);
        let volume = (self.seed % 10_000 + 1_000) * (1 + (delta.abs() / (prev_close / 100 + 1)) as u64);

        if self.candles.len() >= MAX_CANDLES { self.candles.remove(0); }
        self.candles.push(Candle { open, high, low, close, volume });
    }

    fn current_price(&self) -> i64 {
        self.candles.last().map(|c| c.close).unwrap_or(self.start)
    }

    fn pct_change(&self) -> i64 {
        let first = self.candles.first().map(|c| c.open).unwrap_or(self.start);
        let last = self.current_price();
        (last - first) * 10000 / first.max(1)
    }
}

fn fmt_price(cents: i64) -> String {
    format!("${}.{:02}", cents / 100, cents.abs() % 100)
}

fn draw(stocks: &[Stock], selected: usize) {
    fill(0, 0, W, H, C_BG);
    fill(0, 0, W, HEADER_H, C_HEADER);
    fill(0, HEADER_H - 1, W, 1, C_BORDER);
    text(16, 16, C_ORANGE, "Stock Chart");
    text(160, 16, C_HINT, "1-5 or ↑↓:switch stock  R:reset  Ctrl+C:exit");

    // Stock list panel
    fill(0, HEADER_H, STOCK_PANEL_W, H - HEADER_H - STATUS_H, C_CARD);
    fill(STOCK_PANEL_W - 1, HEADER_H, 1, H - HEADER_H - STATUS_H, C_BORDER);

    for (i, s) in stocks.iter().enumerate() {
        let sy = HEADER_H + 8 + i as u32 * 52;
        if i == selected {
            fill(0, sy - 2, STOCK_PANEL_W - 1, 50, 0x1C2D4EFF);
        }
        let pct = s.pct_change();
        let pct_col = if pct >= 0 { C_GREEN } else { C_RED };
        let price_str = fmt_price(s.current_price());
        let pct_str = format!("{}{}.{}%", if pct >= 0 { "+" } else { "-" }, pct.abs() / 100, pct.abs() % 100);
        text(8, sy, if i == selected { C_TEXT } else { C_HINT }, s.name);
        text(8, sy + 18, pct_col, &price_str);
        text(8, sy + 34, pct_col, &pct_str);
    }

    // Chart area background
    fill(CHART_X, CHART_Y, CHART_W, H - HEADER_H - STATUS_H, C_BG);

    let s = &stocks[selected];
    let n = s.candles.len();
    if n == 0 { flush(); return; }

    let vis = n.min(VISIBLE_CANDLES);
    let start = n.saturating_sub(vis);
    let shown = &s.candles[start..n];

    // Find price range for scaling
    let (price_min, price_max) = shown.iter().fold((i64::MAX, i64::MIN), |(lo, hi), c| {
        (lo.min(c.low), hi.max(c.high))
    });
    let price_range = (price_max - price_min).max(1);

    let vol_max = shown.iter().map(|c| c.volume).max().unwrap_or(1);

    let price_to_y = |p: i64| -> u32 {
        let ratio = (price_max - p) as u64 * CHART_CONTENT_H as u64 / price_range as u64;
        CHART_Y + ratio.min(CHART_CONTENT_H as u64 - 1) as u32
    };

    // Horizontal grid lines (5 levels)
    for lvl in 0..=4 {
        let p = price_min + (price_range * lvl / 4);
        let gy = price_to_y(p);
        fill(CHART_X, gy, CHART_CONTENT_W, 1, C_GRID);
        let label = fmt_price(p);
        text(CHART_X + CHART_CONTENT_W + 2, gy.saturating_sub(6), C_HINT, &label);
    }

    // Draw candles
    for (i, c) in shown.iter().enumerate() {
        let cx = CHART_X + i as u32 * CANDLE_W;
        let is_green = c.close >= c.open;
        let col = if is_green { C_GREEN } else { C_RED };

        // Wick
        let wy_top = price_to_y(c.high);
        let wy_bot = price_to_y(c.low);
        let wick_x = cx + CANDLE_W / 2;
        fill(wick_x, wy_top, 1, wy_bot.saturating_sub(wy_top).max(1), col);

        // Body
        let body_top = price_to_y(c.open.max(c.close));
        let body_bot = price_to_y(c.open.min(c.close));
        let body_h = body_bot.saturating_sub(body_top).max(1);
        let bx = (cx + 1).min(CHART_X + CHART_CONTENT_W - 1);
        let bw = (CANDLE_W - 2).max(1);
        fill(bx, body_top, bw, body_h, col);
    }

    // Volume bars
    let vol_y = CHART_Y + CHART_CONTENT_H;
    fill(CHART_X, vol_y, CHART_CONTENT_W, VOL_H, C_BG);
    fill(CHART_X, vol_y, CHART_CONTENT_W, 1, C_BORDER);

    for (i, c) in shown.iter().enumerate() {
        let vx = CHART_X + i as u32 * CANDLE_W;
        let vh = (c.volume * (VOL_H - 4) as u64 / vol_max).max(1) as u32;
        let vy = vol_y + VOL_H - 2 - vh;
        let is_green = c.close >= c.open;
        let vc = if is_green { 0x1A4A1AFF } else { 0x4A1A1AFF };
        fill(vx + 1, vy, (CANDLE_W - 2).max(1), vh, vc);
    }

    // Status bar
    let sb_y = H - STATUS_H;
    fill(0, sb_y, W, STATUS_H, C_HEADER);
    fill(0, sb_y, W, 1, C_BORDER);
    let pct = s.pct_change();
    let info = format!("{}  {}  {}  Hi:{} Lo:{} Vol:{}  {} candles",
        s.name,
        fmt_price(s.current_price()),
        if pct >= 0 { format!("+{}.{}%", pct/100, pct%100) } else { format!("-{}.{}%", pct.abs()/100, pct.abs()%100) },
        fmt_price(shown.iter().map(|c| c.high).max().unwrap_or(0)),
        fmt_price(shown.iter().map(|c| c.low).min().unwrap_or(0)),
        shown.iter().map(|c| c.volume).sum::<u64>() / 1000,
        vis,
    );
    text(8, sb_y + 6, C_HINT, &info);
    flush();
}

fn main() {
    let stdin = io::stdin();
    let mut stocks = vec![
        Stock::new("VYMA",  15000, 0x7793_ABCDEF1234),
        Stock::new("WASM",   8500, 0x7453_12345678AB),
        Stock::new("RUST",  24000, 0x8057_FEDCBA9876),
        Stock::new("KRNL",  35000, 0x5241_0123456789),
        Stock::new("CLUD",  12800, 0x4C4C_89ABCDEF01),
    ];
    let mut selected = 0usize;

    println!("@supervisor: raise stock-chart");
    let _ = io::stdout().flush();
    println!("@supervisor: ping");
    let _ = io::stdout().flush();
    draw(&stocks, selected);

    for line in stdin.lock().lines() {
        let raw = match line { Ok(l) => l, Err(_) => break };

        if raw.starts_with("REPLY:") {
            // Add one candle to all stocks each tick
            for s in &mut stocks { s.next_candle(); }
            println!("@supervisor: ping");
            let _ = io::stdout().flush();
            draw(&stocks, selected);
            continue;
        }

        match raw.as_str() {
            "\x03" => { fill(0, 0, W, H, C_BG); flush(); std::process::exit(0); }
            "\x1b[A" | "k" => { if selected > 0 { selected -= 1; } }
            "\x1b[B" | "j" => { if selected + 1 < stocks.len() { selected += 1; } }
            "1" => { selected = 0; }
            "2" => { selected = 1; }
            "3" => { selected = 2; }
            "4" => { selected = 3; }
            "5" => { selected = 4; }
            "r" | "R" => {
                for s in &mut stocks {
                    s.candles.clear();
                    s.seed = lcg(s.seed);
                    for _ in 0..40 { s.next_candle(); }
                }
            }
            _ => {}
        }
        draw(&stocks, selected);
    }
}

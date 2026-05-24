// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

use std::io::{self, BufRead, Write};

const W: i32 = 880;
const H: i32 = 640;

const C_BG: u32     = 0x0D1117FF;
const C_HEADER: u32 = 0x21262DFF;
const C_BORDER: u32 = 0x30363DFF;
const C_TEXT: u32   = 0xE6EDF3FF;
const C_HINT: u32   = 0x6E7681FF;
const C_ORANGE: u32 = 0xFFA657FF;
const C_SEL: u32    = 0x58A6FFFF;
const C_GREEN: u32  = 0x3FB950FF;
const C_RED: u32    = 0xFF7B72FF;
const C_YELLOW: u32 = 0xD29922FF;
const C_CARD: u32   = 0x161B22FF;
const C_PURPLE: u32 = 0xBC8CFFFF;

// Weather condition index
const SUNNY:   u8 = 0;
const CLOUDY:  u8 = 1;
const RAINY:   u8 = 2;
const STORMY:  u8 = 3;
const SNOWY:   u8 = 4;
const FOGGY:   u8 = 5;
const WINDY:   u8 = 6;
const PARTLY:  u8 = 7; // partly cloudy

const COND_NAMES: [&str; 8] = ["Sunny","Cloudy","Rainy","Stormy","Snowy","Foggy","Windy","Partly Cloudy"];

fn fill(x: i32, y: i32, w: i32, h: i32, c: u32) {
    println!("VYOMA_DRAW:fill_rect:{},{},{},{},{}", x, y, w, h, c);
}
fn text(x: i32, y: i32, c: u32, s: &str) {
    println!("VYOMA_DRAW:draw_text:{},{},{},m,{}", x, y, c, s);
}
fn border(x: i32, y: i32, w: i32, h: i32, c: u32) {
    println!("VYOMA_DRAW:rect_border:{},{},{},{},{}", x, y, w, h, c);
}
fn flush() {
    println!("VYOMA_DRAW:flush");
    let _ = io::stdout().flush();
}

fn lcg(s: u64) -> u64 {
    s.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407)
}

fn lcg_range(s: u64, lo: i32, hi: i32) -> (u64, i32) {
    let ns = lcg(s);
    let v = lo + ((ns >> 33) as i32 % (hi - lo + 1)).abs();
    (ns, v)
}

#[derive(Clone)]
struct DayForecast {
    cond:     u8,
    hi:       i32, // celsius
    lo:       i32,
    humidity: u8,  // percent
    wind:     u8,  // km/h
    precip:   u8,  // mm
}

#[derive(Clone)]
struct HourData {
    temp: i32,
    cond: u8,
}

struct App {
    seed:     u64,
    days:     Vec<DayForecast>,
    hours:    Vec<HourData>, // 24 hours for today
    sim_hour: usize,         // current simulated hour (0-23)
    view_day: usize,         // 0 = today, 1-6 = future days
    ticks:    u64,
    city:     &'static str,
}

const CITIES: [&str; 5] = ["New York", "London", "Tokyo", "Sydney", "Mumbai"];

impl App {
    fn new() -> Self {
        let seed = 0x57EA7_1234_5678u64;
        let mut app = App {
            seed, days: Vec::new(), hours: Vec::new(),
            sim_hour: 8, view_day: 0, ticks: 0, city: CITIES[0],
        };
        app.generate();
        app
    }

    fn generate(&mut self) {
        let mut s = self.seed;
        self.days.clear();
        self.hours.clear();

        // Generate 7 days
        let base_temp = 18i32;
        for d in 0..7usize {
            s = lcg(s);
            let cond = ((s >> 33) % 8) as u8;
            let (ns, hi) = lcg_range(s, base_temp - 5, base_temp + 15);
            s = ns;
            let (ns, lo) = lcg_range(s, hi - 12, hi - 3);
            s = ns;
            let (ns, humidity) = lcg_range(s, 30, 90);
            s = ns;
            let (ns, wind) = lcg_range(s, 5, 50);
            s = ns;
            let precip = if cond >= RAINY { 2 + ((s >> 33) % 20) as u8 } else { 0 };
            s = lcg(s);
            let _ = d;
            self.days.push(DayForecast { cond, hi, lo, humidity: humidity as u8, wind: wind as u8, precip });
        }

        // Generate 24 hours for today
        let today = &self.days[0];
        let base_h = today.lo;
        let range = (today.hi - today.lo).max(1);
        for hr in 0..24usize {
            // temperature curve: low at 6am, peak at 2pm
            let peak_fac = if hr < 6 { 0i32 }
                           else if hr < 14 { ((hr - 6) as i32 * 100) / 8 }
                           else if hr < 20 { (100 - (hr - 14) as i32 * 12).max(0) }
                           else { 0 };
            let temp = base_h + (range * peak_fac) / 100;
            s = lcg(s);
            let cond = if ((s >> 33) % 4) == 0 { ((s >> 20) % 8) as u8 } else { today.cond };
            self.hours.push(HourData { temp, cond });
        }
        self.seed = s;
    }

    fn cond_color(cond: u8) -> u32 {
        match cond {
            SUNNY   => C_YELLOW,
            CLOUDY  => 0x8899AAFF,
            RAINY   => C_SEL,
            STORMY  => C_PURPLE,
            SNOWY   => 0xCCDDEEFF,
            FOGGY   => 0x777777FF,
            WINDY   => C_GREEN,
            PARTLY  => C_ORANGE,
            _       => C_TEXT,
        }
    }

    fn draw_sun(cx: i32, cy: i32, r: i32, c: u32) {
        fill(cx - r, cy - r, r * 2, r * 2, c);
        // Rays: 8 lines radiating out
        let ray = r + 6;
        let offsets: [(i32, i32); 4] = [(0, 1), (1, 0), (1, 1), (1, -1)];
        for (dx, dy) in &offsets {
            fill(cx + dx * (r + 2), cy + dy * (r + 2), 4, 4, c);
            fill(cx - dx * (r + 2), cy - dy * (r + 2), 4, 4, c);
            let _ = ray;
        }
    }

    fn draw_cloud(cx: i32, cy: i32, c: u32) {
        fill(cx - 20, cy - 6, 40, 12, c);
        fill(cx - 10, cy - 14, 24, 12, c);
        fill(cx - 4,  cy - 18, 14, 10, c);
    }

    fn draw_rain(cx: i32, cy: i32, c: u32) {
        App::draw_cloud(cx, cy, 0x8899AAFF);
        for i in 0..5i32 {
            fill(cx - 16 + i * 8, cy + 10, 2, 8, c);
        }
    }

    fn draw_snow(cx: i32, cy: i32, c: u32) {
        App::draw_cloud(cx, cy, 0x8899AAFF);
        for i in 0..5i32 {
            fill(cx - 14 + i * 7, cy + 10, 4, 4, c);
        }
    }

    fn draw_storm(cx: i32, cy: i32) {
        App::draw_cloud(cx, cy, 0x444455FF);
        // Lightning bolt
        fill(cx - 2, cy + 8,  6, 8, C_YELLOW);
        fill(cx - 6, cy + 14, 6, 8, C_YELLOW);
    }

    fn draw_icon(cx: i32, cy: i32, cond: u8) {
        match cond {
            SUNNY   => App::draw_sun(cx, cy, 12, C_YELLOW),
            CLOUDY  => App::draw_cloud(cx, cy, 0x8899AAFF),
            RAINY   => App::draw_rain(cx, cy, C_SEL),
            STORMY  => App::draw_storm(cx, cy),
            SNOWY   => App::draw_snow(cx, cy, 0xCCDDEEFF),
            FOGGY   => {
                for i in 0..3i32 { fill(cx - 20, cy - 6 + i * 8, 40, 4, 0x777777FF); }
            }
            WINDY   => {
                for i in 0..3i32 { fill(cx - 20, cy - 4 + i * 6, 30 + i * 4, 3, C_GREEN); }
            }
            PARTLY  => {
                App::draw_sun(cx - 8, cy - 4, 10, C_YELLOW);
                App::draw_cloud(cx + 6, cy + 4, 0x8899AAFF);
            }
            _ => {}
        }
    }

    fn draw(&self) {
        fill(0, 0, W, H, C_BG);
        fill(0, 0, W, 32, C_HEADER);
        text(12, 8, C_TEXT, &format!("Weather Dashboard  —  {}", self.city));
        text(W - 240, 8, C_HINT, "←→=day  R=refresh  Q=quit");

        let today = &self.days[0];
        let vday  = &self.days[self.view_day];
        let ch    = if self.view_day < self.hours.len() / 24 + 1 {
            &self.hours[self.sim_hour]
        } else {
            &self.hours[0]
        };

        // ── Current conditions (top-left panel) ──────────────────────────────
        fill(12, 38, 300, 180, C_CARD);
        border(12, 38, 300, 180, C_BORDER);
        text(20, 46, C_HINT, "NOW");
        // Large temperature
        let temp_str = format!("{}°C", ch.temp);
        text(20, 64, App::cond_color(ch.cond), &temp_str);
        text(100, 64, C_TEXT, COND_NAMES[ch.cond as usize]);
        // Animated icon
        App::draw_icon(240, 90, ch.cond);
        text(20, 98, C_HINT, &format!("Hi {:2}°  Lo {:2}°", today.hi, today.lo));
        text(20, 116, C_HINT, &format!("Humidity: {}%", today.humidity));
        text(20, 134, C_HINT, &format!("Wind:     {} km/h", today.wind));
        text(20, 152, C_HINT, &format!("Precip:   {} mm", today.precip));
        text(20, 170, C_HINT, &format!("Hour:     {:02}:00", self.sim_hour));

        // ── Hourly temperature strip ──────────────────────────────────────────
        fill(12, 228, W - 24, 120, C_CARD);
        border(12, 228, W - 24, 120, C_BORDER);
        text(20, 236, C_HINT, "24-HOUR FORECAST");

        let bar_area_w = W - 60;
        let bar_w = bar_area_w / 24;
        let min_t = self.hours.iter().map(|h| h.temp).min().unwrap_or(0);
        let max_t = self.hours.iter().map(|h| h.temp).max().unwrap_or(20);
        let t_range = (max_t - min_t).max(1);

        for hr in 0..24usize {
            let h = &self.hours[hr];
            let bx = 20 + hr as i32 * bar_w;
            let max_bar_h = 60i32;
            let bar_h = ((h.temp - min_t) * max_bar_h / t_range).max(4);
            let by = 228 + 120 - 24 - bar_h;
            let bc = App::cond_color(h.cond);
            let is_cur = hr == self.sim_hour;
            fill(bx, by, bar_w - 2, bar_h, if is_cur { C_ORANGE } else { bc });
            if is_cur { border(bx - 1, by - 1, bar_w, bar_h + 2, C_ORANGE); }
            if hr % 6 == 0 {
                text(bx, 228 + 120 - 18, C_HINT, &format!("{:02}", hr));
            }
        }
        // Temp scale
        text(W - 44, 228 + 10, C_HINT, &format!("{}°", max_t));
        text(W - 44, 228 + 100, C_HINT, &format!("{}°", min_t));

        // ── 7-day forecast row ────────────────────────────────────────────────
        fill(12, 358, W - 24, 130, C_CARD);
        border(12, 358, W - 24, 130, C_BORDER);
        text(20, 366, C_HINT, "7-DAY FORECAST");
        let day_names = ["Today","Tue","Wed","Thu","Fri","Sat","Sun"];
        let day_w = (W - 32) / 7;
        for d in 0..7usize {
            let dx = 16 + d as i32 * day_w;
            let df = &self.days[d];
            let is_sel = d == self.view_day;
            if is_sel {
                fill(dx - 2, 380, day_w, 100, C_HEADER);
                border(dx - 2, 380, day_w, 100, C_SEL);
            }
            let dc = App::cond_color(df.cond);
            text(dx, 382, if is_sel { C_ORANGE } else { C_HINT }, day_names[d]);
            App::draw_icon(dx + day_w / 2, 410, df.cond);
            text(dx, 438, dc, COND_NAMES[df.cond as usize].get(..6).unwrap_or(COND_NAMES[df.cond as usize]));
            text(dx, 454, C_SEL, &format!("{:2}°/{:2}°", df.hi, df.lo));
        }

        // ── Detail panel (selected day) ────────────────────────────────────────
        let px = 12;
        fill(px, 498, W - 24, 130, C_CARD);
        border(px, 498, W - 24, 130, C_BORDER);
        let dn = if self.view_day == 0 { "Today".to_string() } else { day_names[self.view_day].to_string() };
        text(px + 8, 506, C_HINT, &format!("DETAIL: {}", dn));
        App::draw_icon(px + 40, 540, vday.cond);
        text(px + 80, 514, App::cond_color(vday.cond), COND_NAMES[vday.cond as usize]);
        text(px + 80, 530, C_TEXT,  &format!("Hi {}°C  Lo {}°C", vday.hi, vday.lo));
        text(px + 80, 546, C_HINT,  &format!("Humidity {}%   Wind {} km/h", vday.humidity, vday.wind));
        text(px + 80, 562, C_SEL,   &format!("Precipitation {} mm", vday.precip));

        // UV / AQI simulation based on condition
        let uv  = if vday.cond == SUNNY { 8 } else if vday.cond == PARTLY { 5 } else { 2 };
        let aqi = if vday.cond == FOGGY { 75 } else if vday.cond == STORMY { 50 } else { 25 };
        let uvc  = if uv >= 7 { C_RED } else if uv >= 4 { C_ORANGE } else { C_GREEN };
        let aqic = if aqi >= 50 { C_ORANGE } else { C_GREEN };
        text(px + 300, 514, uvc,  &format!("UV Index: {}", uv));
        text(px + 300, 530, aqic, &format!("AQI:      {}", aqi));
        text(px + 300, 546, C_HINT, &format!("Pressure: {} hPa", 1013 + vday.lo % 10));
        text(px + 300, 562, C_HINT, &format!("Sunrise: 06:{:02}", 15 + vday.lo % 30));
        text(px + 450, 514, C_HINT, &format!("Sunset:  19:{:02}", 30 + vday.hi % 20));
        text(px + 450, 530, C_HINT, &format!("Moon: {}", ["New","Crescent","Quarter","Gibbous","Full"][self.view_day % 5]));

        // Tick counter / city selector hint
        text(12, H - 20, C_HINT, &format!("Time: {:02}:00  Tick: {}  C=next city  R=refresh  Q=quit", self.sim_hour, self.ticks));

        flush();
    }

    fn handle(&mut self, line: &str) {
        if line == "REPLY:pong" {
            self.ticks += 1;
            if self.ticks % 3 == 0 {
                self.sim_hour = (self.sim_hour + 1) % 24;
            }
            self.draw();
            println!("@supervisor: ping");
            return;
        }
        match line {
            "\x1b[D" => { if self.view_day > 0 { self.view_day -= 1; } self.draw(); }
            "\x1b[C" => { if self.view_day < 6 { self.view_day += 1; } self.draw(); }
            "r" | "R" => {
                self.seed = lcg(self.seed);
                self.generate();
                self.view_day = 0;
                self.draw();
            }
            "c" | "C" => {
                let idx = CITIES.iter().position(|&c| c == self.city).unwrap_or(0);
                self.city = CITIES[(idx + 1) % CITIES.len()];
                self.seed = lcg(self.seed + self.city.as_bytes()[0] as u64);
                self.generate();
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
    println!("@supervisor: ping");
    let _ = io::stdout().flush();

    for line in stdin.lock().lines() {
        let line = match line { Ok(l) => l, Err(_) => break };
        app.handle(&line);
        let _ = io::stdout().flush();
    }
}

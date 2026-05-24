// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

use std::io::{self, BufRead, Write};

const W: u32 = 760;
const H: u32 = 480;
const C_BG: u32     = 0x0D1117FF;
const C_ACCENT: u32 = 0x58A6FFFF;
const C_DIM: u32    = 0x8B949EFF;
const C_TITLE: u32  = 0xFFFFFFFF;
const C_BORDER: u32 = 0x30363DFF;
const C_HINT: u32   = 0x6E7681FF;
const C_ERR: u32    = 0xFF7B72FF;
const C_OK: u32     = 0x3FB950FF;
const C_WARM: u32   = 0xFFA657FF;
const C_COOL: u32   = 0x79C0FFFF;

const WEATHER_PATH: &str = "/data/weather.toml";

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

#[derive(Clone, Default)]
struct DayWeather {
    date:      String,
    condition: String,
    temp_hi:   i32,
    temp_lo:   i32,
    humidity:  i32,
}

fn load_weather() -> Vec<DayWeather> {
    let Ok(s) = std::fs::read_to_string(WEATHER_PATH) else { return Vec::new() };
    let mut days: Vec<DayWeather> = Vec::new();
    let mut cur = DayWeather::default();
    let mut in_day = false;

    for line in s.lines() {
        let line = line.trim();
        if line == "[[day]]" {
            if in_day && !cur.date.is_empty() {
                days.push(cur.clone());
            }
            cur = DayWeather::default();
            in_day = true;
        } else if let Some(v) = line.strip_prefix("date = ") {
            cur.date = v.trim_matches('"').to_string();
        } else if let Some(v) = line.strip_prefix("condition = ") {
            cur.condition = v.trim_matches('"').to_string();
        } else if let Some(v) = line.strip_prefix("temp_hi = ") {
            cur.temp_hi = v.trim().parse().unwrap_or(0);
        } else if let Some(v) = line.strip_prefix("temp_lo = ") {
            cur.temp_lo = v.trim().parse().unwrap_or(0);
        } else if let Some(v) = line.strip_prefix("humidity = ") {
            cur.humidity = v.trim().parse().unwrap_or(0);
        }
    }
    if in_day && !cur.date.is_empty() {
        days.push(cur);
    }
    days
}

fn condition_icon(cond: &str) -> &'static str {
    let lower = cond.to_ascii_lowercase();
    if lower.contains("sun") || lower.contains("clear") { "[*]" }
    else if lower.contains("cloud") { "[~]" }
    else if lower.contains("rain") { "[/]" }
    else if lower.contains("snow") { "[#]" }
    else if lower.contains("storm") || lower.contains("thunder") { "[!]" }
    else if lower.contains("fog") || lower.contains("mist") { "[=]" }
    else { "[?]" }
}

fn temp_color(t: i32) -> u32 {
    if t >= 30 { 0xFF4444FF }
    else if t >= 20 { C_WARM }
    else if t >= 10 { C_OK }
    else { C_COOL }
}

fn draw(days: &[DayWeather], idx: usize, status: &str) {
    fill(0, 0, W, H, C_BG);
    text(20, 14, C_ACCENT, "Weather");
    fill(0, 34, W, 1, C_BORDER);

    if days.is_empty() {
        text(20, 80, C_DIM, "No weather data found.");
        text(20, 110, C_HINT, "Create /data/weather.toml with [[day]] entries.");
        text(20, H - 18, C_HINT, "r: refresh   Ctrl+C: quit");
        flush();
        return;
    }

    let day = &days[idx];

    // Navigation indicator
    let nav = format!("[{}/{}]", idx + 1, days.len());
    let nav_x = W - nav.len() as u32 * 8 - 20;
    text(nav_x, 14, C_DIM, &nav);

    // Date + icon
    let icon = condition_icon(&day.condition);
    let hdr = format!("{icon}  {}", day.date);
    text(20, 50, C_TITLE, &hdr);

    // Condition
    text(20, 80, C_ACCENT, &day.condition);

    // Temperature block
    border(20, 110, 340, 100, C_BORDER);
    fill(21, 111, 338, 98, 0x161B22FF);
    text(30, 122, C_DIM, "High");
    text(30, 148, temp_color(day.temp_hi), &format!("{}°C", day.temp_hi));
    text(180, 122, C_DIM, "Low");
    text(180, 148, temp_color(day.temp_lo), &format!("{}°C", day.temp_lo));
    text(30, 178, C_DIM, &format!("feels like {}/{}°C", day.temp_hi, day.temp_lo));

    // Humidity block
    border(380, 110, 340, 100, C_BORDER);
    fill(381, 111, 338, 98, 0x161B22FF);
    text(390, 122, C_DIM, "Humidity");
    let hum_bar_w = (day.humidity.clamp(0, 100) as u32) * 280 / 100;
    fill(390, 150, 280, 14, 0x21262DFF);
    let hum_color = if day.humidity > 80 { C_COOL } else if day.humidity > 60 { C_ACCENT } else { C_OK };
    if hum_bar_w > 0 { fill(390, 150, hum_bar_w, 14, hum_color); }
    text(390, 172, hum_color, &format!("{}%", day.humidity));

    // Mini forecast for surrounding days
    let forecast_y = 230u32;
    fill(0, forecast_y - 6, W, 1, C_BORDER);
    text(20, forecast_y, C_DIM, "Forecast:");
    let vis = 5usize.min(days.len());
    for i in 0..vis {
        let fx = 20 + i as u32 * 140;
        let fy = forecast_y + 20;
        let day_i = i;
        let is_cur = day_i == idx;
        if is_cur {
            fill(fx, fy - 2, 130, 60, C_HINT.wrapping_sub(0x30000000));
            border(fx, fy - 2, 130, 60, C_BORDER);
        }
        let d = &days[day_i];
        let date_short: String = d.date.chars().skip(5).take(5).collect();
        text(fx + 4, fy + 2, if is_cur { C_TITLE } else { C_DIM }, &date_short);
        text(fx + 4, fy + 18, C_HINT, condition_icon(&d.condition));
        text(fx + 4, fy + 36, temp_color(d.temp_hi), &format!("{}°", d.temp_hi));
    }

    fill(0, H - 30, W, 1, C_BORDER);
    if !status.is_empty() {
        let sc = if status.starts_with("Error") { C_ERR } else { C_OK };
        text(20, H - 18, sc, status);
    } else {
        text(20, H - 18, C_HINT, "←→: prev/next day   r: refresh   Ctrl+C: quit");
    }
    flush();
}

fn main() {
    let stdin = io::stdin();
    let mut days = load_weather();
    let mut idx = 0usize;
    let mut status = String::new();

    draw(&days, idx, &status);

    for line in stdin.lock().lines() {
        let raw = match line { Ok(l) => l, Err(_) => break };
        if raw.starts_with("REPLY:") { continue; }

        match raw.as_str() {
            "\x03" => {
                fill(0, 0, W, H, C_BG);
                flush();
                std::process::exit(0);
            }
            "\x1b[D" => {
                if idx > 0 { idx -= 1; }
                status.clear();
                draw(&days, idx, &status);
            }
            "\x1b[C" => {
                if !days.is_empty() && idx + 1 < days.len() { idx += 1; }
                status.clear();
                draw(&days, idx, &status);
            }
            "r" | "R" => {
                days = load_weather();
                idx = idx.min(days.len().saturating_sub(1));
                status = if days.is_empty() {
                    "No data found in /data/weather.toml".to_string()
                } else {
                    format!("Refreshed: {} day(s) loaded", days.len())
                };
                draw(&days, idx, &status);
            }
            _ => {}
        }
    }
}

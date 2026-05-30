// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

//! Spotlight — full-screen search overlay.
//! Searches apps, files (/data), and supervisor commands.
//! Type to filter, arrow keys to navigate, Enter to act, Escape to close.

mod search;

use search::{Category, SearchResult};
use std::io::{self, BufRead, Write};

const W: u32 = 1440;
const H: u32 = 876;

// Search box geometry (centered)
const BOX_W: u32 = 720;
const BOX_X: u32 = (W - BOX_W) / 2;
const BOX_Y: u32 = 160;
const BOX_H: u32 = 52;

// Results list
const RESULTS_X: u32 = BOX_X;
const RESULTS_Y: u32 = BOX_Y + BOX_H + 16;
const RESULTS_W: u32 = BOX_W;
const ROW_H: u32 = 40;
const HEADER_H: u32 = 32;
const MAX_VISIBLE: usize = 20;

// Colors
const C_OVERLAY: u32 = 0x0D1117CC; // dark overlay ~80% alpha
const C_BOX_BG: u32 = 0x21262DFF; // search box bg
const C_BOX_BORDER: u32 = 0x3B82F6FF; // blue border accent
const C_SEL: u32 = 0x1D4ED8FF; // selection highlight
const C_TEXT: u32 = 0xE6EDF3FF; // primary text
const C_HINT: u32 = 0x8B949EFF; // placeholder / hint
const C_DIM: u32 = 0x6E7681FF; // dim text
const C_HEADER: u32 = 0x58A6FFFF; // category header blue
const C_CLEAR: u32 = 0x00000000; // transparent (for close)

// ── Drawing helpers ──────────────────────────────────────────────────

fn fill(x: u32, y: u32, w: u32, h: u32, rgba: u32) {
    println!("VYOMA_DRAW:fill_rect:{x},{y},{w},{h},{rgba}");
}

fn fill_r(x: u32, y: u32, w: u32, h: u32, rgba: u32, r: u32) {
    println!("VYOMA_DRAW:fill_rect_r:{x},{y},{w},{h},{rgba},{r}");
}

fn text_m(x: u32, y: u32, rgba: u32, s: &str) {
    println!("VYOMA_DRAW:draw_text:{x},{y},{rgba},m,{s}");
}

fn text_s(x: u32, y: u32, rgba: u32, s: &str) {
    println!("VYOMA_DRAW:draw_text:{x},{y},{rgba},s,{s}");
}

fn text_l(x: u32, y: u32, rgba: u32, s: &str) {
    println!("VYOMA_DRAW:draw_text:{x},{y},{rgba},l,{s}");
}

fn flush() {
    println!("VYOMA_DRAW:flush");
    let _ = io::stdout().flush();
}

// ── File scanner ─────────────────────────────────────────────────────

/// Recursively collect file paths under `/data`, up to `limit` entries.
fn scan_files(root: &str, limit: usize) -> Vec<String> {
    let mut files = Vec::new();
    let mut dirs = vec![root.to_string()];
    while let Some(dir) = dirs.pop() {
        if files.len() >= limit {
            break;
        }
        let entries = match std::fs::read_dir(&dir) {
            Ok(e) => e,
            Err(_) => continue,
        };
        for entry in entries {
            if files.len() >= limit {
                break;
            }
            let entry = match entry {
                Ok(e) => e,
                Err(_) => continue,
            };
            let path = entry.path();
            let path_str = path.to_string_lossy().to_string();
            if path.is_dir() {
                dirs.push(path_str);
            } else {
                files.push(path_str);
            }
        }
    }
    files
}

// ── Rendering ────────────────────────────────────────────────────────

/// Compute which rows to display, inserting category headers.
struct DisplayRow {
    kind: DisplayKind,
}

enum DisplayKind {
    Header(Category),
    Result(usize), // index into results vec
}

fn build_display_rows(results: &[SearchResult]) -> Vec<DisplayRow> {
    let mut rows = Vec::new();
    let mut last_cat: Option<Category> = None;
    for (i, r) in results.iter().enumerate() {
        if last_cat != Some(r.category) {
            rows.push(DisplayRow {
                kind: DisplayKind::Header(r.category),
            });
            last_cat = Some(r.category);
        }
        rows.push(DisplayRow {
            kind: DisplayKind::Result(i),
        });
    }
    rows
}

/// Count selectable (non-header) rows visible.
fn selectable_count(rows: &[DisplayRow], max: usize) -> usize {
    rows.iter()
        .take(max)
        .filter(|r| matches!(r.kind, DisplayKind::Result(_)))
        .count()
}

/// Map a selection index (0-based among selectable rows) to a result index.
fn selection_to_result(rows: &[DisplayRow], sel: usize) -> Option<usize> {
    let mut count = 0;
    for row in rows {
        if let DisplayKind::Result(idx) = row.kind {
            if count == sel {
                return Some(idx);
            }
            count += 1;
        }
    }
    None
}

fn draw(query: &str, results: &[SearchResult], sel: usize) {
    // Full-screen frosted overlay
    fill_r(0, 0, W, H, C_OVERLAY, 0);

    // Title
    let title = "Spotlight Search";
    let title_w = title.len() as u32 * 16;
    let title_x = (W - title_w) / 2;
    text_l(title_x, 100, C_TEXT, title);

    // Search box with rounded corners and accent border
    fill_r(BOX_X - 2, BOX_Y - 2, BOX_W + 4, BOX_H + 4, C_BOX_BORDER, 12);
    fill_r(BOX_X, BOX_Y, BOX_W, BOX_H, C_BOX_BG, 10);

    // Search icon hint + text
    let display_text = if query.is_empty() {
        "Search apps, files, commands...".to_string()
    } else {
        format!("{query}|")
    };
    let text_color = if query.is_empty() { C_HINT } else { C_TEXT };
    text_m(BOX_X + 16, BOX_Y + 18, text_color, &display_text);

    // Build display rows with category headers
    let rows = build_display_rows(results);

    // Draw rows
    let mut y = RESULTS_Y;
    let mut sel_idx = 0; // tracks selectable index
    let mut drawn = 0;

    for row in &rows {
        if drawn >= MAX_VISIBLE {
            break;
        }
        match &row.kind {
            DisplayKind::Header(cat) => {
                // Category header
                text_s(RESULTS_X + 8, y + 10, C_HEADER, cat.label());
                y += HEADER_H;
                drawn += 1;
            }
            DisplayKind::Result(idx) => {
                let r = &results[*idx];
                let is_sel = sel_idx == sel;

                if is_sel {
                    fill_r(RESULTS_X, y, RESULTS_W, ROW_H, C_SEL, 8);
                }

                let name_color = if is_sel { C_TEXT } else { C_HINT };
                text_m(RESULTS_X + 24, y + 12, name_color, &r.name);

                // Right-align detail
                let detail_len = r.detail.len().min(60) as u32;
                let detail_w = detail_len * 8;
                let detail_x = RESULTS_X + RESULTS_W - detail_w - 16;
                text_m(detail_x, y + 12, C_DIM, &r.detail);

                // Category badge on far right (small)
                let badge = match r.category {
                    Category::App => "APP",
                    Category::File => "FILE",
                    Category::Command => "CMD",
                };
                let badge_x = RESULTS_X + RESULTS_W - (badge.len() as u32 * 4) - 8;
                text_s(badge_x, y + 4, C_DIM, badge);

                y += ROW_H;
                sel_idx += 1;
                drawn += 1;
            }
        }
    }

    if results.is_empty() && !query.is_empty() {
        text_m(RESULTS_X + 24, RESULTS_Y + 12, C_HINT, "No results found");
    }

    // Footer
    let hint = "Enter: Open  |  Up/Down: Navigate  |  Esc: Close";
    let hint_w = hint.len() as u32 * 8;
    let hint_x = (W - hint_w) / 2;
    text_m(hint_x, H - 36, C_DIM, hint);

    // Result count
    let count_str = format!("{} results", results.len());
    text_m(RESULTS_X, H - 36, C_DIM, &count_str);

    flush();
}

// ── Actions ──────────────────────────────────────────────────────────

fn execute_result(result: &SearchResult) {
    let stdout = io::stdout();
    let mut out = stdout.lock();
    match result.category {
        Category::App => {
            let _ = writeln!(out, "@supervisor: run {}", result.detail);
            let _ = writeln!(out, "@supervisor: focus {}", result.detail);
        }
        Category::File => {
            // Open file in text-editor
            let _ = writeln!(out, "@supervisor: focus text-editor");
        }
        Category::Command => {
            let _ = writeln!(out, "@supervisor: {}", result.detail);
        }
    }
    let _ = out.flush();
}

fn close_overlay() {
    fill(0, 0, W, H, C_CLEAR);
    flush();
    std::process::exit(0);
}

// ── Main loop ────────────────────────────────────────────────────────

fn main() {
    let stdin = io::stdin();
    let mut query = String::new();
    let mut cursor: usize = 0;

    // Raise spotlight to front
    println!("@supervisor: raise spotlight");
    let _ = io::stdout().flush();

    // Scan /data for files (cache once at startup)
    let file_list = scan_files("/data", 500);

    // Initial draw
    let results = search::search(&query, &file_list);
    let rows = build_display_rows(&results);
    let _max_sel = selectable_count(&rows, MAX_VISIBLE);
    draw(&query, &results, cursor);

    for line in stdin.lock().lines() {
        let raw = match line {
            Ok(l) => l,
            Err(_) => break,
        };

        // Skip supervisor replies
        if raw.starts_with("REPLY:") {
            continue;
        }

        match raw.as_str() {
            // Escape or Ctrl+C
            "\x1b" | "\x03" => close_overlay(),
            // Backspace
            "\x7f" => {
                query.pop();
                cursor = 0;
            }
            // Up arrow
            "\x1b[A" => {
                if cursor > 0 {
                    cursor -= 1;
                }
            }
            // Down arrow
            "\x1b[B" => {
                let r = search::search(&query, &file_list);
                let rows = build_display_rows(&r);
                let max = selectable_count(&rows, MAX_VISIBLE);
                if cursor + 1 < max {
                    cursor += 1;
                }
            }
            // Enter
            "" => {
                let r = search::search(&query, &file_list);
                let rows = build_display_rows(&r);
                if let Some(idx) = selection_to_result(&rows, cursor) {
                    execute_result(&r[idx]);
                }
                close_overlay();
            }
            // Printable character
            ch if ch.len() == 1 => {
                let c = ch.chars().next().unwrap();
                if c.is_ascii_graphic() || c == ' ' {
                    query.push(c);
                    cursor = 0;
                }
            }
            _ => {}
        }

        let results = search::search(&query, &file_list);
        let rows = build_display_rows(&results);
        let max_sel = selectable_count(&rows, MAX_VISIBLE);
        if cursor >= max_sel && max_sel > 0 {
            cursor = max_sel - 1;
        }
        draw(&query, &results, cursor);
    }
}

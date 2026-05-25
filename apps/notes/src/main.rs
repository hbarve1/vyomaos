// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

mod app;

use std::io::{self, BufRead, Write};

const W: u32 = 1100;
const H: u32 = 760;
const HEADER_H: u32 = 48;
const STATUS_H: u32 = 32;
const SIDEBAR_W: u32 = 260;
const CHAR_W: u32 = 8;
const CHAR_H: u32 = 16;
const LINE_H: u32 = 18;

const C_BG: u32      = 0x0D1117FF;
const C_HEADER: u32  = 0x21262DFF;
const C_BORDER: u32  = 0x30363DFF;
const C_TEXT: u32    = 0xE6EDF3FF;
const C_HINT: u32    = 0x6E7681FF;
const C_ORANGE: u32  = 0xFFA657FF;
const C_SEL: u32     = 0x58A6FFFF;
const C_CARD: u32    = 0x161B22FF;
const C_SEL_BG: u32  = 0x1C2D4EFF;

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

fn main() {
    let stdin = io::stdin();
    let mut app = app::App::new();

    println!("@supervisor: raise notes");
    let _ = io::stdout().flush();
    app::draw(&app);

    for line in stdin.lock().lines() {
        let raw = match line { Ok(l) => l, Err(_) => break };
        if raw.starts_with("REPLY:") { continue; }

        app.status.clear();

        // Search input mode
        if app.searching {
            match raw.as_str() {
                "\x03" | "\x1b" => {
                    app.searching = false;
                    app.search_buf.clear();
                    app.search = None;
                }
                "" => {
                    app.searching = false;
                    if app.search_buf.is_empty() {
                        app.search = None;
                    } else {
                        app.search = Some(app.search_buf.clone());
                    }
                    app.search_buf.clear();
                }
                "\x7f" => { app.search_buf.pop(); }
                s if s.len() == 1 => {
                    let b = s.as_bytes()[0];
                    if b >= 0x20 && b < 0x7f { app.search_buf.push(b as char); }
                }
                _ => {}
            }
            app::draw(&app);
            continue;
        }

        // Add-note title input mode
        if app.adding {
            match raw.as_str() {
                "\x03" | "\x1b" => { app.adding = false; app.add_buf.clear(); }
                "\x7f" => { app.add_buf.pop(); }
                "" => {
                    let title = app.add_buf.trim().to_string();
                    if !title.is_empty() {
                        app.notes.push(app::Note::new(&title, ""));
                        app.sel = app.notes.len() - 1;
                        app.cursor_row = 0;
                        app.cursor_col = 0;
                        app.scroll_ed = 0;
                        app.focus = app::Focus::Editor;
                        app.status = format!("Created: {}", title);
                    }
                    app.adding = false;
                    app.add_buf.clear();
                }
                s if s.len() == 1 => {
                    let b = s.as_bytes()[0];
                    if b >= 0x20 && b < 0x7f { app.add_buf.push(b as char); }
                }
                _ => {}
            }
            app::draw(&app);
            continue;
        }

        // Normal mode
        match raw.as_str() {
            "\x03" => { fill(0, 0, W, H, C_BG); flush(); std::process::exit(0); }
            "\t" => {
                app.focus = if app.focus == app::Focus::List { app::Focus::Editor } else { app::Focus::List };
            }
            "\x0e" => { // Ctrl+N
                app.adding = true;
            }
            "\x04" => { // Ctrl+D
                if !app.notes.is_empty() {
                    let name = app.notes[app.sel].title.clone();
                    app.notes.remove(app.sel);
                    if app.sel >= app.notes.len() && app.sel > 0 { app.sel -= 1; }
                    if app.notes.is_empty() {
                        app.notes.push(app::Note::new("Untitled", ""));
                        app.sel = 0;
                    }
                    app.cursor_row = 0;
                    app.cursor_col = 0;
                    app.scroll_ed = 0;
                    app.status = format!("Deleted: {}", name);
                }
            }
            "\x17" => { // Ctrl+W
                match app.save() {
                    Ok(()) => app.status = "Saved to /data/notes.csv".to_string(),
                    Err(e) => app.status = format!("Save error: {}", e),
                }
            }
            "\x0c" => { // Ctrl+L
                match app.load() {
                    Ok(()) => app.status = "Loaded from /data/notes.csv".to_string(),
                    Err(e) => app.status = format!("Load error: {}", e),
                }
            }
            "\x06" => { // Ctrl+F
                app.searching = true;
                app.search_buf.clear();
            }
            _ => {
                if app.focus == app::Focus::List {
                    let indices = app.filtered_indices();
                    let pos = indices.iter().position(|&i| i == app.sel).unwrap_or(0);
                    match raw.as_str() {
                        "\x1b[A" => {
                            if pos > 0 { app.sel = indices[pos - 1]; }
                            else if !indices.is_empty() { app.sel = indices[0]; }
                        }
                        "\x1b[B" => {
                            if pos + 1 < indices.len() { app.sel = indices[pos + 1]; }
                        }
                        "" => { app.focus = app::Focus::Editor; }
                        _ => {}
                    }
                } else {
                    // Editor mode
                    match raw.as_str() {
                        "\x1b[A" => { // Up
                            if app.cursor_row > 0 {
                                app.cursor_row -= 1;
                                app.clamp_cursor();
                                if app.cursor_row < app.scroll_ed { app.scroll_ed = app.cursor_row; }
                            }
                        }
                        "\x1b[B" => { // Down
                            app.cursor_row += 1;
                            app.clamp_cursor();
                            let editor_h = H - HEADER_H - STATUS_H;
                            let lines_vis = (editor_h - 32) / LINE_H;
                            if app.cursor_row >= app.scroll_ed + lines_vis as usize {
                                app.scroll_ed = app.cursor_row + 1 - lines_vis as usize;
                            }
                        }
                        "\x1b[C" => { // Right
                            if let Some(n) = app.cur_note() {
                                let row = app.cursor_row.min(n.body.len().saturating_sub(1));
                                if app.cursor_col < n.body.get(row).map_or(0, |l| l.len()) {
                                    app.cursor_col += 1;
                                } else if row + 1 < n.body.len() {
                                    app.cursor_row += 1;
                                    app.cursor_col = 0;
                                }
                            }
                        }
                        "\x1b[D" => { // Left
                            if app.cursor_col > 0 {
                                app.cursor_col -= 1;
                            } else if app.cursor_row > 0 {
                                app.cursor_row -= 1;
                                app.cursor_col = app.cur_note()
                                    .and_then(|n| n.body.get(app.cursor_row))
                                    .map_or(0, |l| l.len());
                            }
                        }
                        "\x7f" => { app.backspace(); }
                        "" => { app.newline(); }
                        s => {
                            if s.len() == 1 {
                                let b = s.as_bytes()[0];
                                if b >= 0x20 && b < 0x7f { app.insert_char(b as char); }
                            }
                        }
                    }
                }
            }
        }
        app::draw(&app);
    }
}

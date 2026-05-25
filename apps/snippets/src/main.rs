// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

use std::io::{self, BufRead, Write};

mod data;
mod app;

use app::{App, Mode, UserSnippet, draw};
use data::SNIPPETS;

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
    let mut app = App::new();

    println!("@supervisor: raise snippets");
    let _ = io::stdout().flush();
    draw(&app);

    for line in stdin.lock().lines() {
        let raw = match line { Ok(l) => l, Err(_) => break };

        // Search mode
        if app.mode == Mode::Search {
            match raw.as_str() {
                "\x1b" | "\x03" => {
                    app.search_buf.clear();
                    app.mode = Mode::Normal;
                }
                "\x7f" => { app.search_buf.pop(); }
                "" => { app.mode = Mode::Normal; }
                s if s.len() == 1 && s.chars().next().map_or(false, |c| !c.is_control()) => {
                    app.search_buf.push_str(s);
                    let vis = app.visible();
                    if let Some(&first) = vis.first() { app.sel = first; }
                }
                _ => {}
            }
            draw(&app);
            continue;
        }

        // New snippet modes
        if app.mode == Mode::NewTitle {
            match raw.as_str() {
                "\x03" => { app.mode = Mode::Normal; app.new_buf.clear(); app.new_snip = None; }
                "\x7f" => { app.new_buf.pop(); }
                "" => {
                    if !app.new_buf.is_empty() {
                        app.new_snip = Some(UserSnippet {
                            title: app.new_buf.clone(),
                            lang:  String::new(),
                            tags:  String::new(),
                            code:  Vec::new(),
                        });
                        app.new_buf.clear();
                        app.mode = Mode::NewLang;
                    }
                }
                s => { app.new_buf.push_str(s); }
            }
            draw(&app);
            continue;
        }

        if app.mode == Mode::NewLang {
            match raw.as_str() {
                "\x03" => { app.mode = Mode::Normal; app.new_buf.clear(); app.new_snip = None; }
                "\x7f" => { app.new_buf.pop(); }
                "" => {
                    let lang = app.new_buf.trim().to_lowercase();
                    let lang = if lang.is_empty() { "text".to_string() } else { lang };
                    if let Some(ref mut ns) = app.new_snip { ns.lang = lang; }
                    app.new_buf.clear();
                    app.mode = Mode::NewCode;
                }
                s => { app.new_buf.push_str(s); }
            }
            draw(&app);
            continue;
        }

        if app.mode == Mode::NewCode {
            match raw.as_str() {
                "\x03" => { app.mode = Mode::Normal; app.new_buf.clear(); app.new_snip = None; }
                "\x17" => {
                    if let Some(mut ns) = app.new_snip.take() {
                        if !app.new_buf.is_empty() {
                            ns.code.push(app.new_buf.clone());
                        }
                        app.status = format!("Saved '{}'.", ns.title);
                        app.user_snips.push(ns);
                        app.sel = SNIPPETS.len() + app.user_snips.len() - 1;
                    }
                    app.new_buf.clear();
                    app.mode = Mode::Normal;
                }
                "\x7f" => { app.new_buf.pop(); }
                "" => {
                    if let Some(ref mut ns) = app.new_snip {
                        ns.code.push(app.new_buf.clone());
                        app.new_buf.clear();
                    }
                }
                s => { app.new_buf.push_str(s); }
            }
            draw(&app);
            continue;
        }

        // Normal mode
        let vis = app.visible();
        let sel_pos = vis.iter().position(|&i| i == app.sel).unwrap_or(0);
        let list_vis = (app::CONTENT_H / app::LINE_H) as usize;

        match raw.as_str() {
            "\x03" => {
                fill(0, 0, app::W, app::H, app::C_BG);
                flush();
                std::process::exit(0);
            }
            "\x1b[A" => {
                if sel_pos > 0 {
                    app.sel = vis[sel_pos - 1];
                    if sel_pos - 1 < app.scroll { app.scroll = sel_pos - 1; }
                }
                app.code_scroll = 0;
            }
            "\x1b[B" => {
                if sel_pos + 1 < vis.len() {
                    app.sel = vis[sel_pos + 1];
                    if sel_pos + 1 >= app.scroll + list_vis {
                        app.scroll = sel_pos + 1 - list_vis + 1;
                    }
                }
                app.code_scroll = 0;
            }
            "\x1b[5~" => { app.code_scroll = app.code_scroll.saturating_sub(5); }
            "\x1b[6~" => { app.code_scroll += 5; }
            "/" => { app.mode = Mode::Search; }
            "\x1b" => { app.search_buf.clear(); }
            "\x0e" => {
                app.mode = Mode::NewTitle;
                app.new_buf.clear();
                app.new_snip = None;
                app.status = "Enter snippet title and press Enter".to_string();
            }
            "c" | "C" => {
                if app.sel < SNIPPETS.len() {
                    let code = SNIPPETS[app.sel].code.join("\\n");
                    let safe = code.replace('\n', " ");
                    println!("@supervisor: clipboard-set {}", safe);
                    let _ = io::stdout().flush();
                    app.status = format!("Copied '{}' to clipboard.", SNIPPETS[app.sel].title);
                } else {
                    let u = &app.user_snips[app.sel - SNIPPETS.len()];
                    let code = u.code.join(" ");
                    println!("@supervisor: clipboard-set {}", code);
                    let _ = io::stdout().flush();
                    app.status = format!("Copied '{}' to clipboard.", u.title);
                }
            }
            _ => {}
        }

        draw(&app);
    }
}

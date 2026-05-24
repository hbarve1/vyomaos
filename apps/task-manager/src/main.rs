// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

use std::io::{self, BufRead, Write};

const W: u32 = 640;
const H: u32 = 560;
const C_BG: u32     = 0x0D1117FF;
const C_ACCENT: u32 = 0x58A6FFFF;
const C_DIM: u32    = 0x8B949EFF;
const C_TITLE: u32  = 0xFFFFFFFF;
const C_BORDER: u32 = 0x30363DFF;
const C_SEL: u32    = 0x1F4068FF;
const C_OK: u32     = 0x3FB950FF;
const C_HINT: u32   = 0x6E7681FF;
const C_ERR: u32    = 0xFF7B72FF;
const C_DONE: u32   = 0x3FB950FF;
const C_FIELD: u32  = 0x21262DFF;

const TASKS_PATH: &str = "/data/tasks.toml";
const LIST_Y: u32      = 52;
const ROW_H: u32       = 26;
const VIS_ROWS: usize  = ((H - LIST_Y - 36) / ROW_H) as usize;

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
struct Task {
    title: String,
    done:  bool,
}

fn load_tasks() -> Vec<Task> {
    let Ok(s) = std::fs::read_to_string(TASKS_PATH) else { return Vec::new() };
    let mut tasks = Vec::new();
    let mut cur_title = String::new();
    let mut cur_done  = false;
    let mut in_task   = false;
    for line in s.lines() {
        let line = line.trim();
        if line == "[[task]]" {
            if in_task && !cur_title.is_empty() {
                tasks.push(Task { title: cur_title.clone(), done: cur_done });
            }
            cur_title.clear();
            cur_done  = false;
            in_task   = true;
        } else if let Some(rest) = line.strip_prefix("title = ") {
            cur_title = rest.trim_matches('"').to_string();
        } else if let Some(rest) = line.strip_prefix("done = ") {
            cur_done = rest.trim() == "true";
        }
    }
    if in_task && !cur_title.is_empty() {
        tasks.push(Task { title: cur_title, done: cur_done });
    }
    tasks
}

fn save_tasks(tasks: &[Task]) -> Result<(), String> {
    let mut out = String::new();
    for t in tasks {
        out.push_str("[[task]]\n");
        out.push_str(&format!("title = \"{}\"\n", t.title.replace('"', "\\\"")));
        out.push_str(&format!("done = {}\n\n", t.done));
    }
    std::fs::write(TASKS_PATH, out).map_err(|e| format!("{e}"))
}

fn done_count(tasks: &[Task]) -> usize {
    tasks.iter().filter(|t| t.done).count()
}

fn draw_list(tasks: &[Task], scroll: usize, cursor: usize, status: &str) {
    fill(0, 0, W, H, C_BG);
    let hdr = format!("Task Manager  —  {}/{} done", done_count(tasks), tasks.len());
    text(20, 14, C_ACCENT, &hdr);
    fill(0, 34, W, 1, C_BORDER);

    if tasks.is_empty() {
        text(20, LIST_Y + 10, C_HINT, "No tasks. Press 'a' to add.");
    } else {
        let end = (scroll + VIS_ROWS).min(tasks.len());
        for (vis_i, task) in tasks[scroll..end].iter().enumerate() {
            let ry = LIST_Y + vis_i as u32 * ROW_H;
            let is_sel = (scroll + vis_i) == cursor;
            if is_sel { fill(0, ry, W, ROW_H, C_SEL); }

            let check = if task.done { "[✓]" } else { "[ ]" };
            let tc     = if task.done { C_DONE } else if is_sel { C_TITLE } else { C_DIM };
            let check_color = if task.done { C_DONE } else { C_HINT };

            text(20, ry + 6, check_color, check);
            let title_disp: String = task.title.chars().take(60).collect();
            text(60, ry + 6, tc, &title_disp);
        }
    }

    fill(0, H - 30, W, 1, C_BORDER);
    if !status.is_empty() {
        let sc = if status.starts_with("Error") { C_ERR } else { C_OK };
        text(20, H - 18, sc, status);
    } else {
        text(20, H - 18, C_HINT, "↑↓: nav   Enter: toggle   a: add   d: delete   Ctrl+W: save   Ctrl+C: quit");
    }
    flush();
}

fn draw_add(input: &str) {
    fill(0, 0, W, H, C_BG);
    text(20, 14, C_ACCENT, "Task Manager  —  Add Task");
    fill(0, 34, W, 1, C_BORDER);
    text(20, 60, C_DIM, "Task title:");
    fill(20, 80, W - 40, 32, C_FIELD);
    border(20, 80, W - 40, 32, C_ACCENT);
    let display = if input.is_empty() { "_" } else { input };
    text(30, 88, C_TITLE, display);
    text(20, H - 18, C_HINT, "Enter: confirm   Esc: cancel");
    flush();
}

enum Mode {
    List { scroll: usize, cursor: usize, status: String },
    Add  { input: String },
}

fn main() {
    let stdin = io::stdin();
    let mut tasks = load_tasks();
    let mut mode  = Mode::List { scroll: 0, cursor: 0, status: String::new() };

    if let Mode::List { scroll, cursor, status } = &mode {
        draw_list(&tasks, *scroll, *cursor, status);
    }

    for line in stdin.lock().lines() {
        let raw = match line { Ok(l) => l, Err(_) => break };
        if raw.starts_with("REPLY:") { continue; }

        match &mut mode {
            Mode::List { scroll, cursor, status } => {
                let total = tasks.len();
                match raw.as_str() {
                    "\x03" => {
                        fill(0, 0, W, H, C_BG);
                        flush();
                        std::process::exit(0);
                    }
                    "\x17" => {
                        match save_tasks(&tasks) {
                            Ok(()) => *status = format!("Saved {} tasks", tasks.len()),
                            Err(e) => *status = format!("Error: {e}"),
                        }
                        draw_list(&tasks, *scroll, *cursor, status);
                    }
                    "\x1b[A" => {
                        if *cursor > 0 { *cursor -= 1; }
                        if *cursor < *scroll { *scroll = *cursor; }
                        status.clear();
                        draw_list(&tasks, *scroll, *cursor, status);
                    }
                    "\x1b[B" => {
                        if total > 0 && *cursor + 1 < total { *cursor += 1; }
                        if *cursor >= *scroll + VIS_ROWS { *scroll = *cursor + 1 - VIS_ROWS; }
                        status.clear();
                        draw_list(&tasks, *scroll, *cursor, status);
                    }
                    "" => {
                        if *cursor < tasks.len() {
                            tasks[*cursor].done = !tasks[*cursor].done;
                            status.clear();
                            draw_list(&tasks, *scroll, *cursor, status);
                        }
                    }
                    "a" => {
                        mode = Mode::Add { input: String::new() };
                        if let Mode::Add { input } = &mode {
                            draw_add(input);
                        }
                    }
                    "d" => {
                        if *cursor < tasks.len() {
                            tasks.remove(*cursor);
                            if *cursor > 0 && *cursor >= tasks.len() {
                                *cursor = tasks.len().saturating_sub(1);
                            }
                            *status = "Task deleted".to_string();
                            draw_list(&tasks, *scroll, *cursor, status);
                        }
                    }
                    _ => {}
                }
            }

            Mode::Add { input } => {
                match raw.as_str() {
                    "\x1b" => {
                        mode = Mode::List { scroll: 0, cursor: 0, status: String::new() };
                        if let Mode::List { scroll, cursor, status } = &mode {
                            draw_list(&tasks, *scroll, *cursor, status);
                        }
                    }
                    "" => {
                        if !input.is_empty() {
                            let title = input.clone();
                            tasks.push(Task { title, done: false });
                            let cur = tasks.len().saturating_sub(1);
                            mode = Mode::List { scroll: 0, cursor: cur, status: "Task added".to_string() };
                            if let Mode::List { scroll, cursor, status } = &mode {
                                draw_list(&tasks, *scroll, *cursor, status);
                            }
                        }
                    }
                    "\x7f" => {
                        input.pop();
                        let inp = input.clone();
                        draw_add(&inp);
                    }
                    ch if ch.len() == 1 => {
                        let c = ch.chars().next().unwrap();
                        if c.is_ascii_graphic() || c == ' ' {
                            input.push(c);
                            let inp = input.clone();
                            draw_add(&inp);
                        }
                    }
                    _ => {}
                }
            }
        }
    }
}

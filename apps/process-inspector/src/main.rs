use std::io::{self, BufRead, Write};
use std::time::{Duration, Instant};

const W: u32 = 840;
const H: u32 = 720;
const C_BG: u32     = 0x0D1117FF;
const C_HDR: u32    = 0x161B22FF;
const C_ACCENT: u32 = 0x58A6FFFF;
const C_DIM: u32    = 0x8B949EFF;
const C_TITLE: u32  = 0xFFFFFFFF;
const C_BORDER: u32 = 0x30363DFF;
const C_SEL: u32    = 0x1F4068FF;
const C_OK: u32     = 0x3FB950FF;
const C_HINT: u32   = 0x6E7681FF;
const C_ERR: u32    = 0xFF7B72FF;

const ROW_H: u32  = 20;
const LIST_Y: u32 = 72;
const HDR_Y: u32  = 48;
const VIS_ROWS: usize = ((H - LIST_Y - 36) / ROW_H) as usize;

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
struct Entry {
    name:     String,
    status:   String,
    uptime:   u64,
    restarts: u32,
}

fn parse_entries(reply: &str) -> Vec<Entry> {
    reply.split('|')
        .filter(|s| !s.is_empty())
        .filter_map(|part| {
            let fields: Vec<&str> = part.splitn(4, ':').collect();
            if fields.len() < 4 { return None; }
            Some(Entry {
                name:     fields[0].to_string(),
                status:   fields[1].to_string(),
                uptime:   fields[2].parse().unwrap_or(0),
                restarts: fields[3].parse().unwrap_or(0),
            })
        })
        .collect()
}

fn fmt_uptime(secs: u64) -> String {
    if secs < 60 { format!("{secs}s") }
    else if secs < 3600 { format!("{}m{}s", secs / 60, secs % 60) }
    else { format!("{}h{}m", secs / 3600, (secs % 3600) / 60) }
}

fn draw_list(entries: &[Entry], scroll: usize, cursor: usize) {
    fill(0, 0, W, H, C_BG);
    text(20, 14, C_ACCENT, "Process Inspector");
    fill(0, 36, W, 1, C_BORDER);

    // Header
    fill(0, HDR_Y, W, ROW_H, C_HDR);
    text(12,  HDR_Y + 3, C_DIM, "NAME");
    text(240, HDR_Y + 3, C_DIM, "STATUS");
    text(360, HDR_Y + 3, C_DIM, "UPTIME");
    text(500, HDR_Y + 3, C_DIM, "RESTARTS");
    fill(0, HDR_Y + ROW_H, W, 1, C_BORDER);

    let visible = entries.iter().skip(scroll).take(VIS_ROWS);
    for (i, entry) in visible.enumerate() {
        let ry = LIST_Y + i as u32 * ROW_H;
        let abs_idx = scroll + i;
        let is_sel = abs_idx == cursor;
        if is_sel {
            fill(0, ry, W, ROW_H, C_SEL);
        }
        let tc = if is_sel { C_TITLE } else { C_DIM };
        let sc = if entry.status == "run" { C_OK } else { C_ERR };
        let name = if entry.name.len() > 20 { &entry.name[..20] } else { &entry.name };
        text(12,  ry + 3, tc, name);
        text(240, ry + 3, sc, &entry.status);
        text(360, ry + 3, tc, &fmt_uptime(entry.uptime));
        text(500, ry + 3, tc, &entry.restarts.to_string());
    }

    let total = entries.len();
    text(12, H - 24, C_HINT,
        &format!("↑↓ navigate  Enter: detail  q/Ctrl+C: quit  ({total} apps)"));
    flush();
}

fn draw_detail(entry: &Entry, win_info: &Option<String>) {
    fill(0, 0, W, H, C_BG);
    text(20, 14, C_ACCENT, &format!("Process: {}", entry.name));
    fill(0, 36, W, 1, C_BORDER);

    let status_color = if entry.status == "run" { C_OK } else { C_ERR };

    // Info panel
    fill(20, 50, W - 40, 260, 0x161B22FF);
    border(20, 50, W - 40, 260, C_BORDER);

    let rows: &[(&str, String)] = &[
        ("Name",     entry.name.clone()),
        ("Status",   entry.status.clone()),
        ("Uptime",   fmt_uptime(entry.uptime)),
        ("Restarts", entry.restarts.to_string()),
        ("Window",   win_info.clone().unwrap_or_else(|| "querying…".into())),
    ];
    for (i, (label, val)) in rows.iter().enumerate() {
        let ry = 68 + i as u32 * 42;
        text(36, ry, C_DIM, label);
        let vc = if *label == "Status" { status_color } else { C_TITLE };
        text(200, ry, vc, val);
    }

    text(20, H - 24, C_HINT, "q / Ctrl+C: back to list");
    flush();
}

fn main() {
    let stdin = io::stdin();
    let mut entries: Vec<Entry> = Vec::new();
    let mut scroll:  usize = 0;
    let mut cursor:  usize = 0;
    let mut detail:  Option<(Entry, Option<String>)> = None;
    let mut last_poll = Instant::now() - Duration::from_secs(3);

    // Initial poll
    println!("@supervisor: ps-raw");
    let _ = io::stdout().flush();

    for line in stdin.lock().lines() {
        let raw = match line { Ok(l) => l, Err(_) => break };

        // Supervisor replies
        if let Some(rest) = raw.strip_prefix("REPLY:") {
            if let Some(info) = rest.strip_prefix("win-info ") {
                // "win-info <name> x,y,w,h" or "win-info <name> none"
                if let Some(detail_ref) = detail.as_mut() {
                    let coords = info.splitn(2, ' ').nth(1).unwrap_or("none");
                    detail_ref.1 = Some(coords.to_string());
                    draw_detail(&detail_ref.0, &detail_ref.1);
                }
            } else {
                // ps-raw reply
                let new_entries = parse_entries(rest);
                if !new_entries.is_empty() {
                    if cursor >= new_entries.len() { cursor = new_entries.len() - 1; }
                    entries = new_entries;
                }
                last_poll = Instant::now();
                if detail.is_none() {
                    draw_list(&entries, scroll, cursor);
                }
                // Sleep 2s before next poll
                std::thread::sleep(Duration::from_secs(2));
                println!("@supervisor: ps-raw");
                let _ = io::stdout().flush();
            }
            continue;
        }

        // Keyboard input
        match raw.as_str() {
            "\x03" | "q" => {
                if detail.is_some() {
                    detail = None;
                    draw_list(&entries, scroll, cursor);
                } else {
                    fill(0, 0, W, H, 0x0D1117FF);
                    flush();
                    std::process::exit(0);
                }
            }
            "\x1b[A" => {
                if detail.is_none() && cursor > 0 {
                    cursor -= 1;
                    if cursor < scroll { scroll = cursor; }
                    draw_list(&entries, scroll, cursor);
                }
            }
            "\x1b[B" => {
                if detail.is_none() && cursor + 1 < entries.len() {
                    cursor += 1;
                    if cursor >= scroll + VIS_ROWS { scroll = cursor + 1 - VIS_ROWS; }
                    draw_list(&entries, scroll, cursor);
                }
            }
            "" => {
                if detail.is_none() {
                    if let Some(entry) = entries.get(cursor).cloned() {
                        let name = entry.name.clone();
                        detail = Some((entry, None));
                        if let Some((ref e, ref wi)) = detail {
                            draw_detail(e, wi);
                        }
                        println!("@supervisor: win-info {name}");
                        let _ = io::stdout().flush();
                    }
                }
            }
            _ => {}
        }

        // Periodic re-poll if no reply arrived
        if last_poll.elapsed() > Duration::from_secs(4) {
            println!("@supervisor: ps-raw");
            let _ = io::stdout().flush();
            last_poll = Instant::now();
        }
    }
}

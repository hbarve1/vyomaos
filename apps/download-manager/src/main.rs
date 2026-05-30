// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

//! Download Manager — queue-based file downloader with progress UI.
//! Uses `@supervisor: download <url> <dest>` IPC for fetching files and
//! persists history to `/data/downloads.log`.

use std::io::{self, BufRead, Write};

// ── Layout constants ────────────────────────────────────────────────────────
const W: u32 = 1440;
const H: u32 = 900;
const HEADER_H: u32 = 56;
const LIST_Y: u32 = 72;
const ROW_H: u32 = 56;
const ROW_GAP: u32 = 4;
const ROW_X: u32 = 40;
const ROW_W: u32 = W - 80;

// ── Colors ──────────────────────────────────────────────────────────────────
const C_BG: u32      = 0x0D1117FF;
const C_HEADER: u32  = 0x161B22FF;
const C_TITLE: u32   = 0xFFFFFFFF;
const C_ACCENT: u32  = 0x58A6FFFF;
const C_ROW: u32     = 0x161B22FF;
const C_SEL: u32     = 0x1F6A4EFF;
const C_HINT: u32    = 0x8B949EFF;
const C_GREEN: u32   = 0x3FB950FF;
const C_YELLOW: u32  = 0xF0C000FF;
const C_ERR: u32     = 0xFF7B72FF;
const C_BORDER: u32  = 0x30363DFF;
const C_BAR_BG: u32  = 0x21262DFF;
const C_BAR_FG: u32  = 0x58A6FFFF;
const C_STATUS: u32  = 0xF0C000FF;

// ── Download model ──────────────────────────────────────────────────────────
#[derive(Clone, PartialEq)]
enum Status {
    Pending,
    Downloading,
    Complete,
    Failed(String),
}

#[derive(Clone)]
struct Download {
    url: String,
    filename: String,
    status: Status,
    size_bytes: usize,
}

fn status_label(s: &Status) -> &str {
    match s {
        Status::Pending     => "Pending",
        Status::Downloading => "Downloading...",
        Status::Complete    => "Complete",
        Status::Failed(_)   => "Failed",
    }
}

fn status_color(s: &Status) -> u32 {
    match s {
        Status::Pending     => C_HINT,
        Status::Downloading => C_YELLOW,
        Status::Complete    => C_GREEN,
        Status::Failed(_)   => C_ERR,
    }
}

// ── Draw helpers ────────────────────────────────────────────────────────────
fn fill(x: u32, y: u32, w: u32, h: u32, rgba: u32) {
    println!("VYOMA_DRAW:fill_rect:{x},{y},{w},{h},{rgba:#010x}");
}
fn text(x: u32, y: u32, rgba: u32, s: &str) {
    println!("VYOMA_DRAW:draw_text:{x},{y},{rgba:#010x},m,{s}");
}
fn text_s(x: u32, y: u32, rgba: u32, s: &str) {
    println!("VYOMA_DRAW:draw_text:{x},{y},{rgba:#010x},s,{s}");
}
fn flush() {
    println!("VYOMA_DRAW:flush");
    let _ = io::stdout().flush();
}

// ── Persistence ─────────────────────────────────────────────────────────────
const LOG_PATH: &str = "/data/downloads.log";
const DL_DIR: &str = "/data/downloads";

fn ensure_dirs() {
    let _ = std::fs::create_dir_all(DL_DIR);
}

/// Append a completed/failed entry to the download log.
fn append_log(dl: &Download) {
    use std::fs::OpenOptions;
    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(LOG_PATH) {
        let st = match &dl.status {
            Status::Complete => "complete".to_string(),
            Status::Failed(e) => format!("failed:{e}"),
            _ => "pending".to_string(),
        };
        let _ = writeln!(f, "{}|{}|{}|{}", dl.url, dl.filename, dl.size_bytes, st);
    }
}

/// Load download history from log file.
fn load_history() -> Vec<Download> {
    let content = std::fs::read_to_string(LOG_PATH).unwrap_or_default();
    content.lines().filter_map(|line| {
        let parts: Vec<&str> = line.splitn(4, '|').collect();
        if parts.len() < 4 { return None; }
        let status = if parts[3] == "complete" {
            Status::Complete
        } else if let Some(e) = parts[3].strip_prefix("failed:") {
            Status::Failed(e.to_string())
        } else {
            Status::Pending
        };
        Some(Download {
            url: parts[0].to_string(),
            filename: parts[1].to_string(),
            size_bytes: parts[2].parse().unwrap_or(0),
            status,
        })
    }).collect()
}

/// Rewrite the entire log from the current queue (after deletions).
fn save_all(queue: &[Download]) {
    let mut out = String::new();
    for dl in queue {
        let st = match &dl.status {
            Status::Complete => "complete".to_string(),
            Status::Failed(e) => format!("failed:{e}"),
            _ => "pending".to_string(),
        };
        out.push_str(&format!("{}|{}|{}|{}\n", dl.url, dl.filename, dl.size_bytes, st));
    }
    let _ = std::fs::write(LOG_PATH, out);
}

// ── Input mode ──────────────────────────────────────────────────────────────
#[derive(Clone, PartialEq)]
enum Mode {
    Normal,
    UrlInput(String),
}

// ── UI rendering ────────────────────────────────────────────────────────────
fn draw_ui(queue: &[Download], sel: usize, mode: &Mode, status_msg: &str) {
    fill(0, 0, W, H, C_BG);

    // Header
    fill(0, 0, W, HEADER_H, C_HEADER);
    text(40, 18, C_TITLE, "Downloads");
    let count_str = format!("{} items", queue.len());
    text(W - 200, 18, C_HINT, &count_str);
    fill(0, HEADER_H, W, 1, C_BORDER);

    // Download list
    let max_rows = ((H - LIST_Y - 80) / (ROW_H + ROW_GAP)) as usize;
    let scroll = if sel >= max_rows { sel - max_rows + 1 } else { 0 };

    if queue.is_empty() {
        text(W / 2 - 100, H / 2 - 8, C_HINT, "No downloads yet");
        text_s(W / 2 - 120, H / 2 + 16, C_HINT, "Press 'n' to start a new download");
    } else {
        for (vi, dl) in queue.iter().enumerate().skip(scroll).take(max_rows) {
            let ry = LIST_Y + ((vi - scroll) as u32) * (ROW_H + ROW_GAP);
            draw_row(ry, dl, vi == sel);
        }
    }

    // URL input overlay
    if let Mode::UrlInput(ref buf) = mode {
        draw_input_overlay(buf);
    }

    // Status bar
    if !status_msg.is_empty() {
        let sc = if status_msg.starts_with("Error") { C_ERR } else { C_STATUS };
        fill(ROW_X, H - 76, ROW_W, 24, C_HEADER);
        text(ROW_X + 12, H - 72, sc, status_msg);
    }

    // Footer
    let help = match mode {
        Mode::Normal => "n: new  Enter: retry  d: delete  o: open file  Up/Down: select  Esc: exit",
        Mode::UrlInput(_) => "Type URL then Enter to download  Esc: cancel",
    };
    text_s(ROW_X, H - 40, C_HINT, help);
    flush();
}

fn draw_row(ry: u32, dl: &Download, is_sel: bool) {
    let bg = if is_sel { C_SEL } else { C_ROW };
    fill(ROW_X, ry, ROW_W, ROW_H, bg);
    if is_sel {
        fill(ROW_X, ry, 3, ROW_H, C_ACCENT);
    }

    // Status icon square
    let icon_c = status_color(&dl.status);
    fill(ROW_X + 12, ry + 14, 28, 28, icon_c);
    let icon_ch = match &dl.status {
        Status::Pending     => "?",
        Status::Downloading => ">",
        Status::Complete    => "V",
        Status::Failed(_)   => "X",
    };
    text(ROW_X + 18, ry + 20, C_TITLE, icon_ch);

    // Filename
    let name_display: String = dl.filename.chars().take(50).collect();
    text(ROW_X + 54, ry + 8, C_TITLE, &name_display);

    // URL (truncated)
    let url_display: String = dl.url.chars().take(80).collect();
    text_s(ROW_X + 54, ry + 30, C_HINT, &url_display);

    // Status + size on right
    let st = status_label(&dl.status);
    text(ROW_X + ROW_W - 220, ry + 8, status_color(&dl.status), st);

    if dl.size_bytes > 0 {
        let size_str = format_size(dl.size_bytes);
        text_s(ROW_X + ROW_W - 220, ry + 30, C_HINT, &size_str);
    }

    // Progress bar for downloading items
    if dl.status == Status::Downloading {
        let bar_x = ROW_X + ROW_W - 160;
        let bar_w: u32 = 120;
        let bar_h: u32 = 8;
        let bar_y = ry + 36;
        fill(bar_x, bar_y, bar_w, bar_h, C_BAR_BG);
        // Indeterminate progress: show a pulsing bar
        fill(bar_x, bar_y, bar_w / 3, bar_h, C_BAR_FG);
    }
}

fn draw_input_overlay(buf: &str) {
    let ox = 100;
    let oy = H / 2 - 60;
    let ow = W - 200;
    let oh: u32 = 120;

    fill(ox, oy, ow, oh, 0x21262DFF);
    fill(ox, oy, ow, 1, C_ACCENT);
    fill(ox, oy + oh - 1, ow, 1, C_ACCENT);
    fill(ox, oy, 1, oh, C_ACCENT);
    fill(ox + ow - 1, oy, 1, oh, C_ACCENT);

    text(ox + 20, oy + 16, C_TITLE, "New Download — Enter URL:");

    // Input field
    fill(ox + 20, oy + 50, ow - 40, 28, 0x0D1117FF);
    let display_buf: String = if buf.len() > 100 {
        format!("...{}", &buf[buf.len() - 97..])
    } else {
        buf.to_string()
    };
    let cursor = format!("{}_", display_buf);
    text(ox + 28, oy + 56, C_ACCENT, &cursor);
}

fn format_size(bytes: usize) -> String {
    if bytes >= 1_048_576 {
        format!("{:.1} MB", bytes as f64 / 1_048_576.0)
    } else if bytes >= 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{} B", bytes)
    }
}

/// Extract a filename from a URL path.
fn filename_from_url(url: &str) -> String {
    // Strip query/fragment
    let path = url.split('?').next().unwrap_or(url);
    let path = path.split('#').next().unwrap_or(path);
    // Take last path segment
    if let Some(seg) = path.rsplit('/').next() {
        if !seg.is_empty() && seg.contains('.') {
            return seg.to_string();
        }
    }
    // Fallback: hash-like name
    let hash: u32 = url.bytes().fold(0u32, |acc, b| acc.wrapping_mul(31).wrapping_add(b as u32));
    format!("download-{hash:08x}")
}

/// Start a download by sending IPC to supervisor.
fn start_download(dl: &mut Download) {
    dl.status = Status::Downloading;
    let dest = format!("{}/{}", DL_DIR, dl.filename);
    println!("@supervisor: download {} {}", dl.url, dest);
    let _ = io::stdout().flush();
}

// ── Main event loop ─────────────────────────────────────────────────────────
fn main() {
    ensure_dirs();

    let stdin = io::stdin();
    let mut queue: Vec<Download> = load_history();
    let mut sel: usize = 0;
    let mut mode = Mode::Normal;
    let mut status_msg = String::new();

    draw_ui(&queue, sel, &mode, &status_msg);

    for line in stdin.lock().lines() {
        let raw = match line { Ok(l) => l, Err(_) => break };

        // ── IPC replies from supervisor ─────────────────────────────────
        if let Some(reply) = raw.strip_prefix("REPLY:") {
            handle_reply(reply, &mut queue, &mut status_msg);
            draw_ui(&queue, sel, &mode, &status_msg);
            continue;
        }

        match &mode {
            Mode::UrlInput(buf) => {
                let buf = buf.clone();
                match raw.as_str() {
                    "\x1b" | "\x1b[" => {
                        mode = Mode::Normal;
                        status_msg.clear();
                    }
                    // Enter — submit URL
                    "" => {
                        let url = buf.trim().to_string();
                        if !url.is_empty() {
                            let filename = filename_from_url(&url);
                            let mut dl = Download {
                                url,
                                filename,
                                status: Status::Pending,
                                size_bytes: 0,
                            };
                            start_download(&mut dl);
                            status_msg = format!("Downloading {}", dl.filename);
                            queue.push(dl);
                            sel = queue.len() - 1;
                        }
                        mode = Mode::Normal;
                    }
                    // Backspace
                    "\x7f" | "\x08" => {
                        let mut new_buf = buf;
                        new_buf.pop();
                        mode = Mode::UrlInput(new_buf);
                    }
                    other => {
                        // Append printable chars
                        if !other.starts_with('\x1b') {
                            let mut new_buf = buf;
                            new_buf.push_str(other);
                            mode = Mode::UrlInput(new_buf);
                        }
                    }
                }
                draw_ui(&queue, sel, &mode, &status_msg);
            }
            Mode::Normal => {
                match raw.as_str() {
                    "\x1b" | "\x1b[" | "\x03" => {
                        fill(0, 0, W, H, C_BG);
                        flush();
                        std::process::exit(0);
                    }
                    // Arrow Up
                    "\x1b[A" => {
                        if sel > 0 { sel -= 1; }
                        status_msg.clear();
                        draw_ui(&queue, sel, &mode, &status_msg);
                    }
                    // Arrow Down
                    "\x1b[B" => {
                        if !queue.is_empty() && sel + 1 < queue.len() { sel += 1; }
                        status_msg.clear();
                        draw_ui(&queue, sel, &mode, &status_msg);
                    }
                    // 'n' — new download
                    "n" => {
                        mode = Mode::UrlInput(String::new());
                        status_msg.clear();
                        draw_ui(&queue, sel, &mode, &status_msg);
                    }
                    // Enter — retry failed download
                    "" => {
                        if let Some(dl) = queue.get_mut(sel) {
                            match &dl.status {
                                Status::Failed(_) | Status::Pending => {
                                    start_download(dl);
                                    status_msg = format!("Retrying {}", dl.filename);
                                }
                                _ => {
                                    status_msg = format!("{}: {}", dl.filename, status_label(&dl.status));
                                }
                            }
                            draw_ui(&queue, sel, &mode, &status_msg);
                        }
                    }
                    // 'd' — delete from list
                    "d" => {
                        if !queue.is_empty() {
                            let name = queue[sel].filename.clone();
                            queue.remove(sel);
                            if sel >= queue.len() && sel > 0 { sel -= 1; }
                            save_all(&queue);
                            status_msg = format!("Removed {name}");
                            draw_ui(&queue, sel, &mode, &status_msg);
                        }
                    }
                    // 'o' — open downloaded file (focus file-manager)
                    "o" => {
                        if let Some(dl) = queue.get(sel) {
                            if dl.status == Status::Complete {
                                println!("@supervisor: focus file-manager");
                                let _ = io::stdout().flush();
                                status_msg = format!("Opened file-manager for {}", dl.filename);
                            } else {
                                status_msg = "Download not complete".to_string();
                            }
                            draw_ui(&queue, sel, &mode, &status_msg);
                        }
                    }
                    _ => {}
                }
            }
        }
    }
}

/// Handle IPC reply messages from supervisor download command.
fn handle_reply(reply: &str, queue: &mut Vec<Download>, status_msg: &mut String) {
    if let Some(rest) = reply.strip_prefix("download-done ") {
        let dest = rest.trim();
        if let Some(dl) = find_by_dest(queue, dest) {
            // Read file size
            let size = std::fs::metadata(dest).map(|m| m.len() as usize).unwrap_or(0);
            dl.size_bytes = size;
            dl.status = Status::Complete;
            append_log(dl);
            *status_msg = format!("Complete: {} ({})", dl.filename, format_size(size));
        }
    } else if let Some(rest) = reply.strip_prefix("download-error ") {
        // Format: "download-error <dest> <error>"
        let (dest, err) = rest.split_once(' ').unwrap_or((rest, "unknown error"));
        let dest = dest.trim();
        let err = err.trim();
        if let Some(dl) = find_by_dest(queue, dest) {
            dl.status = Status::Failed(err.to_string());
            append_log(dl);
            *status_msg = format!("Failed: {} — {}", dl.filename, err);
        } else {
            *status_msg = format!("Download error: {err}");
        }
    } else if let Some(rest) = reply.strip_prefix("download-progress ") {
        // Format: "download-progress <dest> <bytes>"
        let parts: Vec<&str> = rest.splitn(2, ' ').collect();
        if parts.len() == 2 {
            let dest = parts[0].trim();
            let bytes: usize = parts[1].trim().parse().unwrap_or(0);
            if let Some(dl) = find_by_dest(queue, dest) {
                dl.size_bytes = bytes;
                dl.status = Status::Downloading;
            }
        }
    }
}

/// Find a download entry by its destination path.
fn find_by_dest<'a>(queue: &'a mut [Download], dest: &str) -> Option<&'a mut Download> {
    // dest is /data/downloads/<filename>
    let fname = dest.rsplit('/').next().unwrap_or(dest);
    queue.iter_mut().find(|dl| dl.filename == fname)
}

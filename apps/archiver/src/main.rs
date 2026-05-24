// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

use std::io::{self, BufRead, Write};

const W: u32 = 1200;
const H: u32 = 760;
const HEADER_H: u32 = 48;
const STATUS_H: u32 = 32;
const CHAR_W: u32 = 8;
const LINE_H: u32 = 20;
const LEFT_W: u32 = 320;

const C_BG: u32     = 0x0D1117FF;
const C_HEADER: u32 = 0x21262DFF;
const C_BORDER: u32 = 0x30363DFF;
const C_TEXT: u32   = 0xE6EDF3FF;
const C_HINT: u32   = 0x6E7681FF;
const C_ORANGE: u32 = 0xFFA657FF;
const C_GREEN: u32  = 0x3FB950FF;
const C_SEL: u32    = 0x58A6FFFF;
const C_CARD: u32   = 0x161B22FF;
const C_SEL_BG: u32 = 0x1C2D4EFF;
const C_RED: u32    = 0xFF7B72FF;
const C_YELLOW: u32 = 0xD29922FF;

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

fn fmt_size(n: usize) -> String {
    if n < 1024 { format!("{}B", n) }
    else if n < 1024*1024 { format!("{}.{}KB", n/1024, (n%1024)*10/1024) }
    else { format!("{}.{}MB", n/(1024*1024), (n%(1024*1024))*10/(1024*1024)) }
}

#[derive(Clone)]
struct Entry {
    name: String,
    data: Vec<u8>,
}

#[derive(Clone)]
struct Archive {
    name:    String,
    entries: Vec<Entry>,
}

impl Archive {
    fn total_size(&self) -> usize { self.entries.iter().map(|e| e.data.len()).sum() }

    fn serialize(&self) -> Vec<u8> {
        let mut out = Vec::new();
        for e in &self.entries {
            let hdr = format!("FILE:{}:{}\n", e.name, e.data.len());
            out.extend_from_slice(hdr.as_bytes());
            out.extend_from_slice(&e.data);
            out.extend_from_slice(b"END\n");
        }
        out
    }

    fn parse(name: &str, raw: &[u8]) -> Self {
        let mut entries = Vec::new();
        let mut pos = 0usize;
        while pos < raw.len() {
            // Find "FILE:" header line
            if raw[pos..].starts_with(b"FILE:") {
                let line_end = raw[pos..].iter().position(|&b| b == b'\n').map(|i| pos+i).unwrap_or(raw.len());
                let line = std::str::from_utf8(&raw[pos..line_end]).unwrap_or("");
                let parts: Vec<&str> = line.splitn(3, ':').collect();
                if parts.len() == 3 {
                    let fname = parts[1].to_string();
                    let size: usize = parts[2].parse().unwrap_or(0);
                    let data_start = line_end + 1;
                    let data_end = (data_start + size).min(raw.len());
                    let data = raw[data_start..data_end].to_vec();
                    entries.push(Entry { name: fname, data });
                    // Skip past END\n
                    pos = data_end;
                    if raw[pos..].starts_with(b"END\n") { pos += 4; }
                    continue;
                }
            }
            pos += 1;
        }
        Archive { name: name.to_string(), entries }
    }

    fn save(&self) -> Result<(), String> {
        let path = format!("/data/{}", self.name);
        std::fs::write(&path, self.serialize()).map_err(|e| e.to_string())
    }

    fn load(name: &str) -> Result<Self, String> {
        let path = format!("/data/{}", name);
        let bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
        Ok(Self::parse(name, &bytes))
    }
}

fn demo_archive() -> Archive {
    Archive {
        name: "demo.tar".to_string(),
        entries: vec![
            Entry { name: "hello.txt".to_string(), data: b"Hello from VyomaOS!\nThis is a demo archive.".to_vec() },
            Entry { name: "notes.txt".to_string(), data: b"VyomaOS is a WASM-first OS.\nBuilt with Rust.".to_vec() },
            Entry { name: "config.toml".to_string(), data: b"[app]\nname = \"demo\"\nversion = \"0.1.0\"".to_vec() },
        ],
    }
}

enum Focus { Left, Right }

struct App {
    archives:     Vec<Archive>,
    arch_sel:     usize,
    entry_sel:    usize,
    focus:        Focus,
    naming:       bool,  // true = entering new archive name
    adding:       bool,  // true = entering filename to add
    input_buf:    String,
    status:       String,
}

impl App {
    fn new() -> Self {
        App {
            archives: vec![demo_archive()],
            arch_sel: 0,
            entry_sel: 0,
            focus: Focus::Left,
            naming: false,
            adding: false,
            input_buf: String::new(),
            status: String::new(),
        }
    }

    fn cur_archive(&self) -> Option<&Archive> { self.archives.get(self.arch_sel) }
    fn cur_archive_mut(&mut self) -> Option<&mut Archive> { self.archives.get_mut(self.arch_sel) }

    fn add_file_from_data(&mut self, filename: &str) -> Result<(), String> {
        let path = format!("/data/{}", filename);
        let data = std::fs::read(&path).map_err(|e| e.to_string())?;
        let arch = self.cur_archive_mut().ok_or("no archive selected")?;
        arch.entries.push(Entry { name: filename.to_string(), data });
        Ok(())
    }

    fn extract_entry(&self, entry_idx: usize) -> Result<(), String> {
        let arch = self.cur_archive().ok_or("no archive")?;
        let entry = arch.entries.get(entry_idx).ok_or("no entry")?;
        let path = format!("/data/{}", entry.name);
        std::fs::write(&path, &entry.data).map_err(|e| e.to_string())
    }
}

const CONTENT_H: u32 = H - HEADER_H - STATUS_H;
const RIGHT_X: u32 = LEFT_W + 4;
const RIGHT_W: u32 = W - RIGHT_X;

fn draw(app: &App) {
    fill(0, 0, W, H, C_BG);
    fill(0, 0, W, HEADER_H, C_HEADER);
    fill(0, HEADER_H - 1, W, 1, C_BORDER);
    text(16, 16, C_ORANGE, "File Archiver");
    text(210, 16, C_HINT, "Tab:panel  N:new  A:add  X:extract  Del:remove  Ctrl+W:save  Ctrl+C:exit");

    let lines_vis = (CONTENT_H / LINE_H) as usize;

    // Left panel — archive list
    let left_bg = if matches!(app.focus, Focus::Left) { C_CARD } else { C_BG };
    fill(0, HEADER_H, LEFT_W, CONTENT_H, left_bg);
    let lc = if matches!(app.focus, Focus::Left) { C_SEL } else { C_HINT };
    text(8, HEADER_H + 6, lc, &format!("Archives ({})", app.archives.len()));
    fill(0, HEADER_H + 18, LEFT_W, 1, C_BORDER);

    for (i, arch) in app.archives.iter().enumerate().take(lines_vis) {
        let ly = HEADER_H + 20 + i as u32 * LINE_H;
        let is_sel = i == app.arch_sel;
        if is_sel { fill(0, ly, LEFT_W, LINE_H, C_SEL_BG); }
        let nc = if is_sel { C_TEXT } else { C_HINT };
        let max_n = (LEFT_W - 80) as usize / CHAR_W as usize;
        let nd = if arch.name.len() > max_n { &arch.name[..max_n] } else { &arch.name };
        text(8, ly + 3, nc, nd);
        text(LEFT_W - 72, ly + 3, C_HINT, &fmt_size(arch.total_size()));
        if is_sel && matches!(app.focus, Focus::Left) {
            border(0, ly, LEFT_W, LINE_H, C_SEL);
        }
    }
    if matches!(app.focus, Focus::Left) { border(0, HEADER_H, LEFT_W, CONTENT_H, C_SEL); }

    // Divider
    fill(LEFT_W, HEADER_H, 4, CONTENT_H, C_BORDER);

    // Right panel — entries
    let right_bg = if matches!(app.focus, Focus::Right) { C_CARD } else { C_BG };
    fill(RIGHT_X, HEADER_H, RIGHT_W, CONTENT_H, right_bg);
    let rc = if matches!(app.focus, Focus::Right) { C_SEL } else { C_HINT };

    if let Some(arch) = app.cur_archive() {
        text(RIGHT_X + 8, HEADER_H + 6, rc, &format!("{}  ({} files, {})", arch.name, arch.entries.len(), fmt_size(arch.total_size())));
        fill(RIGHT_X, HEADER_H + 18, RIGHT_W, 1, C_BORDER);

        // Column headers
        let hy = HEADER_H + 20;
        text(RIGHT_X + 8,  hy, C_HINT, "Name");
        text(RIGHT_X + 560, hy, C_HINT, "Size");
        text(RIGHT_X + 640, hy, C_HINT, "Preview");
        fill(RIGHT_X, hy + LINE_H - 1, RIGHT_W, 1, C_BORDER);

        for (i, entry) in arch.entries.iter().enumerate().take(lines_vis - 2) {
            let ly = HEADER_H + 40 + i as u32 * LINE_H;
            let is_sel = i == app.entry_sel && matches!(app.focus, Focus::Right);
            if is_sel { fill(RIGHT_X, ly, RIGHT_W, LINE_H, C_SEL_BG); }
            let ec = if is_sel { C_TEXT } else { C_HINT };
            let max_n = 68usize;
            let nd = if entry.name.len() > max_n { &entry.name[..max_n] } else { &entry.name };
            text(RIGHT_X + 8,  ly + 3, ec, nd);
            text(RIGHT_X + 560, ly + 3, C_HINT, &fmt_size(entry.data.len()));
            // Preview (first 20 printable chars)
            let preview: String = entry.data.iter().take(20)
                .map(|&b| if b >= 0x20 && b < 0x7f { b as char } else { '.' })
                .collect();
            text(RIGHT_X + 640, ly + 3, C_HINT, &preview);
            if is_sel { border(RIGHT_X, ly, RIGHT_W, LINE_H, C_SEL); }
        }
    } else {
        text(RIGHT_X + 8, HEADER_H + 40, C_HINT, "No archive selected. Press N to create one.");
    }
    if matches!(app.focus, Focus::Right) { border(RIGHT_X, HEADER_H, RIGHT_W, CONTENT_H, C_SEL); }

    // Status bar
    let sb_y = H - STATUS_H;
    fill(0, sb_y, W, STATUS_H, C_HEADER);
    fill(0, sb_y, W, 1, C_BORDER);

    if app.naming {
        text(8, sb_y + 8, C_HINT, "Archive name (.tar): ");
        text(8 + 21 * CHAR_W, sb_y + 8, C_TEXT, &app.input_buf);
        fill(8 + (21 + app.input_buf.len() as u32) * CHAR_W, sb_y + 6, 2, STATUS_H - 12, C_SEL);
    } else if app.adding {
        text(8, sb_y + 8, C_HINT, "Add file from /data/: ");
        text(8 + 22 * CHAR_W, sb_y + 8, C_TEXT, &app.input_buf);
        fill(8 + (22 + app.input_buf.len() as u32) * CHAR_W, sb_y + 6, 2, STATUS_H - 12, C_SEL);
    } else if !app.status.is_empty() {
        text(8, sb_y + 8, C_HINT, &app.status);
    } else {
        let total: usize = app.archives.iter().map(|a| a.total_size()).sum();
        text(8, sb_y + 8, C_HINT, &format!("{} archives  total: {}  |  format: FILE:<name>:<size>\\n<data>END\\n", app.archives.len(), fmt_size(total)));
    }
    flush();
}

fn main() {
    let stdin = io::stdin();
    let mut app = App::new();

    println!("@supervisor: raise archiver");
    let _ = io::stdout().flush();
    draw(&app);

    for line in stdin.lock().lines() {
        let raw = match line { Ok(l) => l, Err(_) => break };
        if raw.starts_with("REPLY:") { continue; }
        app.status.clear();

        // Input prompts
        if app.naming || app.adding {
            match raw.as_str() {
                "\x03" | "\x1b" => { app.naming = false; app.adding = false; app.input_buf.clear(); }
                "\x7f" => { app.input_buf.pop(); }
                "" => {
                    let val = app.input_buf.trim().to_string();
                    if app.naming {
                        app.naming = false;
                        app.input_buf.clear();
                        if !val.is_empty() {
                            let name = if val.ends_with(".tar") { val.clone() } else { format!("{}.tar", val) };
                            app.archives.push(Archive { name: name.clone(), entries: vec![] });
                            app.arch_sel = app.archives.len() - 1;
                            app.status = format!("Created: {}", name);
                        }
                    } else {
                        app.adding = false;
                        app.input_buf.clear();
                        if !val.is_empty() {
                            match app.add_file_from_data(&val) {
                                Ok(()) => app.status = format!("Added: {}", val),
                                Err(e) => app.status = format!("Error: {}", e),
                            }
                        }
                    }
                }
                s if s.len() == 1 => {
                    let b = s.as_bytes()[0];
                    if b >= 0x20 && b < 0x7f { app.input_buf.push(b as char); }
                }
                _ => {}
            }
            draw(&app);
            continue;
        }

        match raw.as_str() {
            "\x03" => { fill(0, 0, W, H, C_BG); flush(); std::process::exit(0); }
            "\t" => { app.focus = if matches!(app.focus, Focus::Left) { Focus::Right } else { Focus::Left }; }
            "\x1b[A" => {
                match app.focus {
                    Focus::Left  => { if app.arch_sel > 0 { app.arch_sel -= 1; app.entry_sel = 0; } }
                    Focus::Right => { if app.entry_sel > 0 { app.entry_sel -= 1; } }
                }
            }
            "\x1b[B" => {
                match app.focus {
                    Focus::Left => {
                        if app.arch_sel + 1 < app.archives.len() { app.arch_sel += 1; app.entry_sel = 0; }
                    }
                    Focus::Right => {
                        let n = app.cur_archive().map_or(0, |a| a.entries.len());
                        if app.entry_sel + 1 < n { app.entry_sel += 1; }
                    }
                }
            }
            "" => {
                if matches!(app.focus, Focus::Left) {
                    app.focus = Focus::Right;
                } else {
                    // Extract current entry
                    let ei = app.entry_sel;
                    match app.extract_entry(ei) {
                        Ok(()) => {
                            let name = app.cur_archive().and_then(|a| a.entries.get(ei)).map(|e| e.name.clone()).unwrap_or_default();
                            app.status = format!("Extracted: /data/{}", name);
                        }
                        Err(e) => app.status = format!("Extract error: {}", e),
                    }
                }
            }
            "n" | "N" => { app.naming = true; app.input_buf.clear(); }
            "a" | "A" => { if app.cur_archive().is_some() { app.adding = true; app.input_buf.clear(); } }
            "x" | "X" => {
                if matches!(app.focus, Focus::Right) {
                    let ei = app.entry_sel;
                    match app.extract_entry(ei) {
                        Ok(()) => {
                            let name = app.cur_archive().and_then(|a| a.entries.get(ei)).map(|e| e.name.clone()).unwrap_or_default();
                            app.status = format!("Extracted: /data/{}", name);
                        }
                        Err(e) => app.status = format!("Error: {}", e),
                    }
                } else {
                    // Extract all entries of selected archive
                    if let Some(arch) = app.cur_archive() {
                        let count = arch.entries.len();
                        let names: Vec<_> = arch.entries.iter().map(|e| (e.name.clone(), e.data.clone())).collect();
                        for (name, data) in names {
                            let _ = std::fs::write(format!("/data/{}", name), data);
                        }
                        app.status = format!("Extracted {} files", count);
                    }
                }
            }
            "\x7f" => { // Del — remove selected entry
                if matches!(app.focus, Focus::Right) {
                    let sel = app.entry_sel;
                    let removed = if let Some(arch) = app.cur_archive_mut() {
                        if sel < arch.entries.len() {
                            let name = arch.entries[sel].name.clone();
                            arch.entries.remove(sel);
                            Some((name, arch.entries.len()))
                        } else { None }
                    } else { None };
                    if let Some((name, new_len)) = removed {
                        if app.entry_sel > 0 && app.entry_sel >= new_len { app.entry_sel -= 1; }
                        app.status = format!("Removed: {}", name);
                    }
                }
            }
            "\x17" => { // Ctrl+W — save archive
                if let Some(arch) = app.cur_archive() {
                    match arch.save() {
                        Ok(()) => app.status = format!("Saved /data/{}", arch.name),
                        Err(e) => app.status = format!("Save error: {}", e),
                    }
                }
            }
            "\x0c" => { // Ctrl+L — reload archive from disk
                if let Some(arch) = app.cur_archive() {
                    let name = arch.name.clone();
                    match Archive::load(&name) {
                        Ok(a) => {
                            app.archives[app.arch_sel] = a;
                            app.entry_sel = 0;
                            app.status = format!("Loaded /data/{}", name);
                        }
                        Err(e) => app.status = format!("Load error: {}", e),
                    }
                }
            }
            _ => {}
        }
        draw(&app);
    }
}

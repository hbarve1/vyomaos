// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

use std::io::{self, BufRead, Write};

const W: u32 = 1200;
const H: u32 = 760;
const HEADER_H: u32 = 40;
const LIST_W: u32 = 320;
const CONTENT_Y: u32 = HEADER_H;
const CONTENT_H: u32 = H - HEADER_H - 32;
const CODE_X: u32 = LIST_W + 1;
const CODE_W: u32 = W - CODE_X;
const LINE_H: u32 = 18;
const STATUS_H: u32 = 28;

const C_BG: u32     = 0x0D1117FF;
const C_HEADER: u32 = 0x21262DFF;
const C_BORDER: u32 = 0x30363DFF;
const C_TEXT: u32   = 0xE6EDF3FF;
const C_HINT: u32   = 0x6E7681FF;
const C_ORANGE: u32 = 0xFFA657FF;
const C_GREEN: u32  = 0x3FB950FF;
const C_SEL: u32    = 0x58A6FFFF;
const C_YELLOW: u32 = 0xD29922FF;
const C_PURPLE: u32 = 0xBC8CFFFF;
const C_CARD: u32   = 0x161B22FF;

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

struct Snippet {
    title:    &'static str,
    lang:     &'static str,
    tags:     &'static str,
    code:     &'static [&'static str],
}

const SNIPPETS: &[Snippet] = &[
    Snippet {
        title: "Hello World (Rust)",
        lang:  "rust",
        tags:  "basic hello",
        code:  &[
            "fn main() {",
            "    println!(\"Hello, World!\");",
            "}",
        ],
    },
    Snippet {
        title: "Read File Lines",
        lang:  "rust",
        tags:  "file io read",
        code:  &[
            "use std::fs::File;",
            "use std::io::{BufRead, BufReader};",
            "",
            "fn read_lines(path: &str) -> Vec<String> {",
            "    let file = File::open(path).unwrap();",
            "    BufReader::new(file).lines()",
            "        .filter_map(|l| l.ok())",
            "        .collect()",
            "}",
        ],
    },
    Snippet {
        title: "HashMap Usage",
        lang:  "rust",
        tags:  "collections map",
        code:  &[
            "use std::collections::HashMap;",
            "",
            "let mut map: HashMap<String, i32> = HashMap::new();",
            "map.insert(\"count\".to_string(), 0);",
            "*map.entry(\"count\".to_string()).or_insert(0) += 1;",
            "println!(\"{:?}\", map);",
        ],
    },
    Snippet {
        title: "Struct + Impl",
        lang:  "rust",
        tags:  "struct oop",
        code:  &[
            "struct Point { x: f64, y: f64 }",
            "",
            "impl Point {",
            "    fn new(x: f64, y: f64) -> Self { Point { x, y } }",
            "    fn dist(&self, o: &Point) -> f64 {",
            "        ((self.x-o.x).powi(2) + (self.y-o.y).powi(2)).sqrt()",
            "    }",
            "}",
        ],
    },
    Snippet {
        title: "Enum + Match",
        lang:  "rust",
        tags:  "enum match pattern",
        code:  &[
            "enum Color { Red, Green, Blue }",
            "",
            "fn hex(c: Color) -> &'static str {",
            "    match c {",
            "        Color::Red   => \"#FF0000\",",
            "        Color::Green => \"#00FF00\",",
            "        Color::Blue  => \"#0000FF\",",
            "    }",
            "}",
        ],
    },
    Snippet {
        title: "Spawn Thread",
        lang:  "rust",
        tags:  "thread concurrency",
        code:  &[
            "use std::thread;",
            "",
            "let handle = thread::spawn(|| {",
            "    println!(\"running in thread\");",
            "});",
            "handle.join().unwrap();",
        ],
    },
    Snippet {
        title: "HTTP GET (TCP)",
        lang:  "rust",
        tags:  "network http tcp",
        code:  &[
            "use std::io::{Read, Write};",
            "use std::net::TcpStream;",
            "",
            "let mut stream = TcpStream::connect(\"example.com:80\")?;",
            "stream.write_all(b\"GET / HTTP/1.0\\r\\nHost: example.com\\r\\n\\r\\n\")?;",
            "let mut body = String::new();",
            "stream.read_to_string(&mut body)?;",
        ],
    },
    Snippet {
        title: "Iterate Vec",
        lang:  "rust",
        tags:  "vec iterator",
        code:  &[
            "let nums = vec![1, 2, 3, 4, 5];",
            "let doubled: Vec<i32> = nums.iter().map(|n| n * 2).collect();",
            "let evens: Vec<&i32> = nums.iter().filter(|&&n| n % 2 == 0).collect();",
            "let sum: i32 = nums.iter().sum();",
        ],
    },
    Snippet {
        title: "Error Handling",
        lang:  "rust",
        tags:  "error result",
        code:  &[
            "use std::num::ParseIntError;",
            "",
            "fn parse(s: &str) -> Result<i32, ParseIntError> {",
            "    let n: i32 = s.trim().parse()?;",
            "    Ok(n * 2)",
            "}",
            "",
            "match parse(\"42\") {",
            "    Ok(v)  => println!(\"got {v}\"),",
            "    Err(e) => eprintln!(\"error: {e}\"),",
            "}",
        ],
    },
    Snippet {
        title: "Hello World (Python)",
        lang:  "python",
        tags:  "basic hello",
        code:  &[
            "print('Hello, World!')",
        ],
    },
    Snippet {
        title: "List Comprehension",
        lang:  "python",
        tags:  "list comprehension",
        code:  &[
            "nums = [1, 2, 3, 4, 5]",
            "doubled = [n * 2 for n in nums]",
            "evens = [n for n in nums if n % 2 == 0]",
            "squares = {n: n**2 for n in nums}",
        ],
    },
    Snippet {
        title: "Read/Write File",
        lang:  "python",
        tags:  "file io",
        code:  &[
            "# read",
            "with open('file.txt') as f:",
            "    lines = f.readlines()",
            "",
            "# write",
            "with open('out.txt', 'w') as f:",
            "    f.write('hello\\n')",
        ],
    },
    Snippet {
        title: "Dataclass",
        lang:  "python",
        tags:  "class struct",
        code:  &[
            "from dataclasses import dataclass",
            "",
            "@dataclass",
            "class Point:",
            "    x: float",
            "    y: float",
            "",
            "    def dist(self, other):",
            "        return ((self.x-other.x)**2 + (self.y-other.y)**2) ** 0.5",
        ],
    },
    Snippet {
        title: "HTTP Request",
        lang:  "python",
        tags:  "network http request",
        code:  &[
            "import urllib.request",
            "import json",
            "",
            "url = 'https://api.example.com/data'",
            "with urllib.request.urlopen(url) as r:",
            "    data = json.loads(r.read())",
        ],
    },
    Snippet {
        title: "Grep Files",
        lang:  "shell",
        tags:  "search grep file",
        code:  &[
            "# Search recursively for pattern",
            "grep -rn 'pattern' ./src/",
            "",
            "# Case-insensitive, show filenames only",
            "grep -rli 'TODO' .",
        ],
    },
    Snippet {
        title: "Find and Delete",
        lang:  "shell",
        tags:  "find delete clean",
        code:  &[
            "# Find and delete .tmp files",
            "find . -name '*.tmp' -delete",
            "",
            "# Find files older than 7 days",
            "find /tmp -mtime +7 -type f",
        ],
    },
    Snippet {
        title: "SSH Port Forward",
        lang:  "shell",
        tags:  "ssh tunnel network",
        code:  &[
            "# Local port forward: localhost:8080 -> remote:80",
            "ssh -L 8080:localhost:80 user@server",
            "",
            "# Keep alive in background",
            "ssh -fNL 8080:localhost:80 user@server",
        ],
    },
    Snippet {
        title: "Git Workflow",
        lang:  "shell",
        tags:  "git version control",
        code:  &[
            "git checkout -b feature/my-feature",
            "git add -p                  # interactive stage",
            "git commit -m 'feat: ...'",
            "git push -u origin HEAD",
            "git log --oneline --graph",
        ],
    },
    Snippet {
        title: "Docker Run",
        lang:  "shell",
        tags:  "docker container",
        code:  &[
            "# Run interactively",
            "docker run -it --rm ubuntu:22.04 bash",
            "",
            "# Run with port + volume",
            "docker run -d -p 8080:80 -v $(pwd):/app nginx",
        ],
    },
    Snippet {
        title: "VYOMA_DRAW Helpers",
        lang:  "rust",
        tags:  "vyomaos display draw",
        code:  &[
            "fn fill(x: u32, y: u32, w: u32, h: u32, rgba: u32) {",
            "    println!(\"VYOMA_DRAW:fill_rect:{x},{y},{w},{h},{rgba:#010x}\");",
            "}",
            "fn text(x: u32, y: u32, rgba: u32, s: &str) {",
            "    println!(\"VYOMA_DRAW:draw_text:{x},{y},{rgba:#010x},m,{s}\");",
            "}",
            "fn flush() {",
            "    println!(\"VYOMA_DRAW:flush\");",
            "    let _ = std::io::stdout().flush();",
            "}",
        ],
    },
];

// Very basic syntax highlighting — single-pass token scan
fn highlight_line<'a>(line: &'a str, lang: &str) -> Vec<(u32, &'a str)> {
    // Returns spans: (color, text_segment)
    // Simplified: just colour whole line based on first token
    let trimmed = line.trim_start();

    // Comment detection
    if lang == "rust" || lang == "shell" {
        if trimmed.starts_with("//") || trimmed.starts_with('#') {
            return vec![(C_HINT, line)];
        }
    }
    if lang == "python" && trimmed.starts_with('#') {
        return vec![(C_HINT, line)];
    }

    // String detection (line starts with quote content)
    if trimmed.starts_with('"') || trimmed.starts_with('\'') {
        return vec![(C_GREEN, line)];
    }

    // Rust keywords
    let rust_keywords = ["fn ", "let ", "use ", "pub ", "struct ", "enum ", "impl ", "match ",
                         "if ", "else", "for ", "while ", "return ", "mod ", "trait ", "type ",
                         "const ", "static ", "mut ", "ref ", "self", "Self", "true", "false"];
    let python_keywords = ["def ", "class ", "import ", "from ", "return ", "if ", "else",
                           "elif ", "for ", "while ", "with ", "as ", "print(", "True", "False"];
    let shell_keywords = ["git ", "docker ", "ssh ", "find ", "grep ", "echo ", "export ",
                          "cd ", "ls ", "mkdir ", "rm ", "cp ", "mv "];

    let keywords: &[&str] = match lang {
        "rust"   => &rust_keywords,
        "python" => &python_keywords,
        "shell"  => &shell_keywords,
        _        => &[],
    };

    for kw in keywords {
        if trimmed.starts_with(kw) {
            return vec![(C_ORANGE, line)];
        }
    }

    vec![(C_TEXT, line)]
}

fn lang_color(lang: &str) -> u32 {
    match lang {
        "rust"   => C_ORANGE,
        "python" => C_YELLOW,
        "shell"  => C_GREEN,
        _        => C_HINT,
    }
}

#[derive(PartialEq)]
enum Mode { Normal, Search, NewTitle, NewLang, NewCode }

struct UserSnippet {
    title: String,
    lang:  String,
    tags:  String,
    code:  Vec<String>,
}

struct App {
    sel:       usize,
    scroll:    usize,
    code_scroll: usize,
    mode:      Mode,
    search_buf: String,
    new_buf:   String,
    new_snip:  Option<UserSnippet>,
    user_snips: Vec<UserSnippet>,
    status:    String,
}

impl App {
    fn new() -> Self {
        App {
            sel: 0,
            scroll: 0,
            code_scroll: 0,
            mode: Mode::Normal,
            search_buf: String::new(),
            new_buf: String::new(),
            new_snip: None,
            user_snips: Vec::new(),
            status: String::from("↑↓=select  C=copy  /=search  Ctrl+N=new  Ctrl+C=quit"),
        }
    }

    fn matches(&self, i: usize) -> bool {
        if self.search_buf.is_empty() { return true; }
        let q = self.search_buf.to_lowercase();
        if i < SNIPPETS.len() {
            SNIPPETS[i].title.to_lowercase().contains(&q)
                || SNIPPETS[i].tags.to_lowercase().contains(&q)
                || SNIPPETS[i].lang.to_lowercase().contains(&q)
        } else {
            let u = &self.user_snips[i - SNIPPETS.len()];
            u.title.to_lowercase().contains(&q)
                || u.tags.to_lowercase().contains(&q)
                || u.lang.to_lowercase().contains(&q)
        }
    }

    fn visible(&self) -> Vec<usize> {
        let total = SNIPPETS.len() + self.user_snips.len();
        (0..total).filter(|&i| self.matches(i)).collect()
    }
}

fn draw(app: &App) {
    fill(0, 0, W, H, C_BG);

    // Header
    fill(0, 0, W, HEADER_H, C_HEADER);
    fill(0, HEADER_H - 1, W, 1, C_BORDER);
    text(16, 12, C_ORANGE, "Code Snippets");

    // Search box or mode label
    if app.mode == Mode::Search {
        text(200, 12, C_SEL, &format!("Search: {}_", app.search_buf));
    } else if !app.search_buf.is_empty() {
        text(200, 12, C_YELLOW, &format!("Filter: {}  (Esc=clear)", app.search_buf));
    } else {
        text(200, 12, C_HINT, "20 snippets · Rust · Python · Shell");
    }

    let vis = app.visible();
    let sel_in_vis = vis.iter().position(|&i| i == app.sel).unwrap_or(0);
    let list_vis = (CONTENT_H / LINE_H) as usize;
    let list_scroll = app.scroll.min(vis.len().saturating_sub(list_vis));

    // Left: snippet list
    fill(0, CONTENT_Y, LIST_W, CONTENT_H, C_BG);
    fill(0, CONTENT_Y, LIST_W, 1, C_BORDER);

    for (vi, &si) in vis[list_scroll..].iter().take(list_vis).enumerate() {
        let vy = CONTENT_Y + 4 + vi as u32 * LINE_H;
        let is_sel = si == app.sel;

        let (title, lang, _tags) = if si < SNIPPETS.len() {
            (SNIPPETS[si].title, SNIPPETS[si].lang, SNIPPETS[si].tags)
        } else {
            let u = &app.user_snips[si - SNIPPETS.len()];
            (u.title.as_str(), u.lang.as_str(), u.tags.as_str())
        };

        if is_sel {
            fill(0, vy - 2, LIST_W, LINE_H, C_CARD);
            fill(0, vy - 2, 2, LINE_H, C_SEL);
        }

        // Language badge
        let lc = lang_color(lang);
        let badge = &lang[..lang.len().min(2)].to_uppercase();
        let badge_w = badge.len() as u32 * 8 + 4;
        fill(4, vy, badge_w, 14, C_HEADER);
        text(6, vy, lc, badge);

        // Title (truncate)
        let max_title = ((LIST_W - badge_w - 16) / 8) as usize;
        let shown = if title.len() > max_title { &title[..max_title] } else { title };
        let title_col = if is_sel { C_TEXT } else { C_HINT };
        text(badge_w + 10, vy, title_col, shown);
    }

    // Divider
    fill(LIST_W, CONTENT_Y, 1, CONTENT_H, C_BORDER);

    // Right: code view
    fill(CODE_X, CONTENT_Y, CODE_W, CONTENT_H, C_CARD);

    if app.sel < SNIPPETS.len() + app.user_snips.len() {
        let (title, lang, tags, code_lines): (&str, &str, &str, Vec<&str>) = if app.sel < SNIPPETS.len() {
            let s = &SNIPPETS[app.sel];
            (s.title, s.lang, s.tags, s.code.to_vec())
        } else {
            let u = &app.user_snips[app.sel - SNIPPETS.len()];
            (u.title.as_str(), u.lang.as_str(), u.tags.as_str(),
             u.code.iter().map(|s| s.as_str()).collect())
        };

        let lc = lang_color(lang);
        text(CODE_X + 8, CONTENT_Y + 6, lc, &format!("[{}] {}", lang.to_uppercase(), title));
        text(CODE_X + 8, CONTENT_Y + 24, C_HINT, &format!("tags: {}", tags));
        fill(CODE_X, CONTENT_Y + 40, CODE_W, 1, C_BORDER);

        let code_vis = ((CONTENT_H - 50) / LINE_H) as usize;
        let cscroll = app.code_scroll.min(code_lines.len().saturating_sub(code_vis));

        for (li, &line) in code_lines[cscroll..].iter().take(code_vis).enumerate() {
            let ly = CONTENT_Y + 46 + li as u32 * LINE_H;
            // Gutter line number
            text(CODE_X + 4, ly, C_HINT, &format!("{:3}", li + 1 + cscroll));
            // Highlighted code
            let spans = highlight_line(line, lang);
            let mut cx = CODE_X + 32;
            for (col, seg) in spans {
                let max_chars = ((CODE_W - 36) / 8) as usize;
                let shown = if seg.len() > max_chars { &seg[..max_chars] } else { seg };
                text(cx, ly, col, shown);
                cx += shown.len() as u32 * 8;
            }
        }

        // Copy hint
        let copy_y = H - STATUS_H - 24;
        text(CODE_X + 8, copy_y, C_HINT, "C=copy snippet to clipboard");
    } else {
        text(CODE_X + 8, CONTENT_Y + 20, C_HINT, "No snippet selected");
    }

    // New snippet form overlay
    match &app.mode {
        Mode::NewTitle => {
            let oy = H / 2 - 50;
            fill(CODE_X + 8, oy, CODE_W - 16, 80, C_HEADER);
            border(CODE_X + 8, oy, CODE_W - 16, 80, C_SEL);
            text(CODE_X + 16, oy + 10, C_HINT, "New snippet — enter title:");
            text(CODE_X + 16, oy + 36, C_TEXT, &format!("{}_", app.new_buf));
        }
        Mode::NewLang => {
            let oy = H / 2 - 50;
            fill(CODE_X + 8, oy, CODE_W - 16, 80, C_HEADER);
            border(CODE_X + 8, oy, CODE_W - 16, 80, C_SEL);
            text(CODE_X + 16, oy + 10, C_HINT, "Language (rust/python/shell):");
            text(CODE_X + 16, oy + 36, C_TEXT, &format!("{}_", app.new_buf));
        }
        Mode::NewCode => {
            if let Some(ref ns) = app.new_snip {
                let oy = CONTENT_Y + 50;
                let oh = CONTENT_H - 60;
                fill(CODE_X + 8, oy, CODE_W - 16, oh, C_HEADER);
                border(CODE_X + 8, oy, CODE_W - 16, oh, C_SEL);
                text(CODE_X + 16, oy + 6, C_HINT, &format!("Code for '{}' (Enter=add line, Ctrl+W=save):", ns.title));
                fill(CODE_X + 16, oy + 22, CODE_W - 32, 1, C_BORDER);
                for (i, line) in ns.code.iter().enumerate() {
                    text(CODE_X + 16, oy + 28 + i as u32 * 16, C_TEXT, line);
                }
                let cur_y = oy + 28 + ns.code.len() as u32 * 16;
                if cur_y < oy + oh - 20 {
                    text(CODE_X + 16, cur_y, C_SEL, &format!("{}_", app.new_buf));
                }
            }
        }
        _ => {}
    }

    // Status bar
    let sb_y = H - STATUS_H;
    fill(0, sb_y, W, STATUS_H, C_HEADER);
    fill(0, sb_y, W, 1, C_BORDER);
    text(8, sb_y + 6, C_HINT, &app.status);

    flush();
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
                    // Reset sel to first match
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
                    // Ctrl+W — save
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
                    // Enter — add line
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
        let list_vis = (CONTENT_H / LINE_H) as usize;

        match raw.as_str() {
            "\x03" => { fill(0, 0, W, H, C_BG); flush(); std::process::exit(0); }

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

            // Code scroll
            "\x1b[5~" => { app.code_scroll = app.code_scroll.saturating_sub(5); }
            "\x1b[6~" => { app.code_scroll += 5; }

            "/" => {
                app.mode = Mode::Search;
            }
            "\x1b" => {
                app.search_buf.clear();
            }

            "\x0e" => {
                // Ctrl+N — new snippet
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
